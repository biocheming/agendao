//! Shared Direct-mode event bridge.
//!
//! In Direct (in-process) mode there is no HTTP server and no SSE transport.
//! Frontends subscribe to the canonical `FrontendEvent` bus directly.

use std::collections::HashMap;
use std::sync::Arc;

use agendao_server_core::frontend_events::FrontendEvent;
use agendao_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ServerState;

/// Spawn a Direct-mode event loop for one session. Returns a receiver that the
/// frontend consumes.
pub fn spawn_direct_event_loop(
    state: Arc<ServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<FrontendEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        direct_event_subscription_loop(&state, &session_id, tx, cancel).await;
    });
    rx
}

/// Spawn a Direct-mode event loop for the full canonical frontend bus.
///
/// Consumers that need dynamic client-side routing should subscribe once here
/// and apply their own session filter locally. This avoids the "unsubscribe old
/// session / subscribe new session" race where early FrontendEvents for a newly
/// selected session can be dropped before the new subscription is live.
pub fn spawn_direct_event_bus(
    state: Arc<ServerState>,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<FrontendEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        direct_event_bus_loop(&state, tx, cancel).await;
    });
    rx
}

async fn direct_event_subscription_loop(
    state: &Arc<ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();
    let mut live_output_accum = HashMap::<String, String>::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            recv = event_rx.recv() => {
                let Ok(event_json) = recv else {
                    break;
                };
                let Ok(frontend_event) = serde_json::from_str::<FrontendEvent>(&event_json) else {
                    continue;
                };
                if frontend_event_session_id(&frontend_event) == Some(session_id) {
                    let frontend_event = coalesce_live_output_block(frontend_event, &mut live_output_accum);
                    let _ = tx.send(frontend_event);
                }
            }
        }
    }
}

async fn direct_event_bus_loop(
    state: &Arc<ServerState>,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();
    let mut live_output_accum = HashMap::<String, String>::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            recv = event_rx.recv() => {
                let Ok(event_json) = recv else {
                    break;
                };
                let Ok(frontend_event) = serde_json::from_str::<FrontendEvent>(&event_json) else {
                    continue;
                };
                let frontend_event = coalesce_live_output_block(frontend_event, &mut live_output_accum);
                let _ = tx.send(frontend_event);
            }
        }
    }
}

fn frontend_event_session_id(event: &FrontendEvent) -> Option<&str> {
    match event {
        FrontendEvent::SessionRuntimeReplaced { session_id, .. }
        | FrontendEvent::SessionProjectionReplaced { session_id, .. }
        | FrontendEvent::QuestionUpsert { session_id, .. }
        | FrontendEvent::QuestionRemoved { session_id, .. }
        | FrontendEvent::PermissionUpsert { session_id, .. }
        | FrontendEvent::PermissionRemoved { session_id, .. }
        | FrontendEvent::ToolCallUpsert { session_id, .. }
        | FrontendEvent::DiffReplaced { session_id, .. }
        | FrontendEvent::OutputBlockAppended { session_id, .. } => Some(session_id.as_str()),
    }
}

fn coalesce_live_output_block(
    event: FrontendEvent,
    accum: &mut HashMap<String, String>,
) -> FrontendEvent {
    let FrontendEvent::OutputBlockAppended {
        session_id,
        mut block,
        id,
        live_identity,
    } = event
    else {
        return event;
    };

    let Some(identity) = live_identity.as_ref() else {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    };

    let Some(text_field) = coalesced_text_field(identity) else {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    };

    let key = format!(
        "{}:{}:{}",
        session_id, identity.message_id, identity.part_key
    );

    if identity.phase == LivePartPhase::End {
        accum.remove(&key);
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    }

    if !matches!(identity.phase, LivePartPhase::Append | LivePartPhase::Snapshot) {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    }

    let text = block
        .get(text_field)
        .and_then(|value| value.as_str())
        .unwrap_or("");

    let accumulated = if identity.phase == LivePartPhase::Append {
        accum.entry(key.clone()).or_default().push_str(text);
        accum.get(&key).cloned().unwrap_or_default()
    } else {
        let merged = merge_snapshot_text(accum.get(&key).map(String::as_str), text);
        accum.insert(key, merged.clone());
        merged
    };

    if let Some(obj) = block.as_object_mut() {
        obj.insert(text_field.to_string(), serde_json::json!(accumulated));
        obj.insert("phase".to_string(), serde_json::json!("full"));
    }

    FrontendEvent::OutputBlockAppended {
        session_id,
        block,
        id,
        live_identity: Some(LiveMessagePartIdentity {
            phase: LivePartPhase::Snapshot,
            ..identity.clone()
        }),
    }
}

fn coalesced_text_field(identity: &LiveMessagePartIdentity) -> Option<&'static str> {
    match identity.part_kind {
        LiveMessagePartKind::AssistantText | LiveMessagePartKind::AssistantReasoning => {
            Some("text")
        }
        LiveMessagePartKind::ToolCall => Some("detail"),
        _ => None,
    }
}

fn merge_snapshot_text(existing: Option<&str>, incoming: &str) -> String {
    let Some(existing) = existing.filter(|value| !value.is_empty()) else {
        return incoming.to_string();
    };
    if incoming.is_empty() {
        return existing.to_string();
    }
    if incoming.starts_with(existing) {
        return incoming.to_string();
    }
    if existing.starts_with(incoming) {
        return existing.to_string();
    }

    let overlap = suffix_prefix_overlap(existing, incoming);
    if overlap > 0 {
        let mut merged = String::with_capacity(existing.len() + incoming.len() - overlap);
        merged.push_str(existing);
        merged.push_str(&incoming[overlap..]);
        return merged;
    }

    let mut merged = String::with_capacity(existing.len() + incoming.len());
    merged.push_str(existing);
    merged.push_str(incoming);
    merged
}

fn suffix_prefix_overlap(existing: &str, incoming: &str) -> usize {
    let max = existing.len().min(incoming.len());
    for size in (1..=max).rev() {
        if existing.is_char_boundary(existing.len() - size)
            && incoming.is_char_boundary(size)
            && existing[existing.len() - size..] == incoming[..size]
        {
            return size;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use agendao_server_core::frontend_events::FrontendEvent;
    use agendao_server_core::runtime_events::ToolCallPhase;
    use agendao_types::{
        LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase,
        ASSISTANT_TEXT_MAIN_PART_KEY,
    };

    #[test]
    fn frontend_event_session_id_extracts_session_scoped_variants() {
        let event = FrontendEvent::ToolCallUpsert {
            session_id: "ses_direct".to_string(),
            tool_call_id: "tool_1".to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Start,
        };

        assert_eq!(
            super::frontend_event_session_id(&event),
            Some("ses_direct")
        );
    }

    #[test]
    fn coalesces_live_message_append_into_full_snapshot() {
        let mut accum = HashMap::new();
        let identity = LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
            legacy_block_id: None,
        };

        let first = super::coalesce_live_output_block(
            FrontendEvent::OutputBlockAppended {
                session_id: "ses_direct".to_string(),
                block: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "text": "hel"
                }),
                id: Some("msg_1".to_string()),
                live_identity: Some(identity.clone()),
            },
            &mut accum,
        );
        let second = super::coalesce_live_output_block(
            FrontendEvent::OutputBlockAppended {
                session_id: "ses_direct".to_string(),
                block: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "text": "lo"
                }),
                id: Some("msg_1".to_string()),
                live_identity: Some(identity),
            },
            &mut accum,
        );

        let FrontendEvent::OutputBlockAppended { block, .. } = first else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "hel");
        assert_eq!(block["phase"], "full");

        let FrontendEvent::OutputBlockAppended { block, .. } = second else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "hello");
        assert_eq!(block["phase"], "full");
    }

    #[test]
    fn snapshot_fragments_do_not_collapse_to_last_token() {
        let mut accum = HashMap::new();
        let identity = LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: None,
        };

        let fragments = ["你", "好", "，世界"];
        let mut last = None;
        for fragment in fragments {
            last = Some(super::coalesce_live_output_block(
                FrontendEvent::OutputBlockAppended {
                    session_id: "ses_direct".to_string(),
                    block: serde_json::json!({
                        "kind": "message",
                        "phase": "full",
                        "text": fragment
                    }),
                    id: Some("msg_1".to_string()),
                    live_identity: Some(identity.clone()),
                },
                &mut accum,
            ));
        }

        let FrontendEvent::OutputBlockAppended { block, .. } =
            last.expect("coalesced event")
        else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "你好，世界");
        assert_eq!(block["phase"], "full");
    }
}
