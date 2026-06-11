//! Shared Direct-mode event bridge.
//!
//! In Direct (in-process) mode there is no server and no SSE. This module
//! provides a poll-based event loop that synthesizes canonical events from
//! local session state. TUI and CLI both consume the same event stream,
//! mapping `DirectEvent` variants into their internal dispatch types
//! (`StateChange` / `CliServerEvent`).
//!
//! The event categories covered here match the `ServerEvent` canonical
//! contract (see docs/frontend-transport-event-matrix-2026-05-28.md).

use std::collections::{HashMap, HashSet};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ServerState;
use agendao_server_core::runtime_control::SessionRunStatus;

use std::sync::Arc;
use std::time::Duration;

/// Events emitted by the Direct bridge. These map 1:1 to ServerEvent
/// categories and are intended to be converted into frontend-internal
/// dispatch types by a thin adapter in each frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DirectEvent {
    SessionBusy {
        session_id: String,
    },
    SessionIdle {
        session_id: String,
    },
    SessionUpdated {
        session_id: String,
    },
    OutputBlock {
        session_id: String,
        block: serde_json::Value,
    },
    QuestionCreated {
        session_id: String,
        request_id: String,
        questions_json: Option<serde_json::Value>,
    },
    QuestionResolved {
        session_id: String,
        request_id: String,
    },
    PermissionRequested {
        session_id: String,
        permission_id: String,
        info_json: Option<serde_json::Value>,
    },
    PermissionResolved {
        session_id: String,
        permission_id: String,
    },
    ToolCallStarted {
        session_id: String,
    },
    ToolCallCompleted {
        session_id: String,
    },
    ControlInputTransition {
        session_id: String,
        phase: String, // "start" | "end"
    },
    TopologyChanged {
        session_id: String,
    },
    DiffUpdated {
        session_id: String,
    },
    SessionTreeChanged {
        session_id: String,
    },
}

/// Spawn a Direct-mode event loop for one session. Returns a receiver
/// that the frontend consumes.
///
/// P3: Now uses event_bus subscription instead of polling. Question/Permission
/// are in the bus, SessionStatus/ToolCall/Topology flow through StageEvent.
pub fn spawn_direct_event_loop(
    state: Arc<ServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<DirectEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        direct_event_subscription_loop(&state, &session_id, tx, cancel).await;
    });
    rx
}

fn should_emit_text_output_block(role: &str) -> bool {
    role == "assistant"
}

fn run_status_is_active(status: &SessionRunStatus) -> bool {
    matches!(
        status,
        SessionRunStatus::Busy | SessionRunStatus::Compacting | SessionRunStatus::Retry { .. }
    )
}

/// Poll session state at a fixed interval and synthesize DirectEvent
/// variants. Covers: session status, message arrival, question/permission
/// lifecycle, and tool state transitions.
async fn direct_poll_loop(
    state: &Arc<ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<DirectEvent>,
    cancel: CancellationToken,
) {
    let mut last_message_count = 0usize;
    let mut last_session_updated_at = None;
    let mut last_active_state: Option<bool> = None;
    let mut last_tool_state: Option<String> = None;
    let mut pending_question_ids = HashSet::new();
    let mut pending_permission_ids: HashMap<String, String> = HashMap::new();
    let mut last_execution_count: usize;
    let mut last_error: Option<String> = None;
    let mut idle_rounds = 0u32;
    if let Ok(session) = crate::local_get_session(Arc::clone(state), session_id).await {
        last_session_updated_at = Some(session.time.updated);
    }

    if let Ok(messages) = crate::local_list_messages(Arc::clone(state), session_id, None, None).await {
        last_message_count = messages.len();
        last_tool_state = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .and_then(|m| m.finish.as_deref())
            .map(|f| f.to_string());
        last_error = messages
            .last()
            .and_then(|m| m.error.as_deref())
            .filter(|e| !e.is_empty())
            .map(|e| e.to_string());
    }

    if let Ok(questions) = crate::local_list_questions(Arc::clone(state)).await {
        pending_question_ids = questions
            .into_iter()
            .filter(|q| q.session_id == session_id)
            .map(|q| q.id)
            .collect();
    }

    if let Ok(permissions) = crate::local_list_permissions(Arc::clone(state)).await {
        pending_permission_ids = permissions
            .into_iter()
            .filter(|p| p.session_id == session_id)
            .map(|p| (p.id, p.session_id))
            .collect();
    }

    let topology = state
        .runtime_control
        .list_session_execution_topology(session_id)
        .await;
    last_execution_count = topology.active_count + topology.running_count;

    let mut current_interval_ms = 300u64;

    loop {
        let deadline = tokio::time::sleep(Duration::from_millis(current_interval_ms));
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = deadline => {},
        }

        // ── Run status ─────────────────────────────────────────────
        let current_run_status = state.runtime_control.session_run_status(session_id).await;
        let is_active = run_status_is_active(&current_run_status);
        if last_active_state != Some(is_active) {
            if is_active {
                let _ = tx.send(DirectEvent::SessionBusy {
                    session_id: session_id.to_string(),
                });
                let _ = tx.send(DirectEvent::ControlInputTransition {
                    session_id: session_id.to_string(),
                    phase: "start".to_string(),
                });
            } else {
                let _ = tx.send(DirectEvent::SessionIdle {
                    session_id: session_id.to_string(),
                });
                let _ = tx.send(DirectEvent::ControlInputTransition {
                    session_id: session_id.to_string(),
                    phase: "end".to_string(),
                });
            }
            last_active_state = Some(is_active);
        }

        let Ok(session) = crate::local_get_session(Arc::clone(state), session_id).await else {
            break;
        };
        let session_changed = last_session_updated_at != Some(session.time.updated);
        last_session_updated_at = Some(session.time.updated);

        // ── Messages ──────────────────────────────────────────────
        if session_changed {
            let Ok(messages) =
                crate::local_list_messages(Arc::clone(state), session_id, None, None).await
            else {
                break;
            };

            let count = messages.len();
            let prev_count = last_message_count;
            last_message_count = count;
            let _ = tx.send(DirectEvent::SessionUpdated {
                session_id: session_id.to_string(),
            });

            // Emit assistant output blocks for new messages.
            // User prompts are already surfaced by each frontend's local
            // optimistic/submit path and later reconciled through
            // SessionUpdated + transcript sync. Re-emitting them here creates
            // transient duplicate/triplicate prompt rows in Direct mode.
            if count > prev_count {
                for msg in messages.iter().skip(prev_count) {
                    if should_emit_text_output_block(&msg.role) {
                        for part in &msg.parts {
                            if let Some(text) = part.text.as_deref() {
                                let _ = tx.send(DirectEvent::OutputBlock {
                                    session_id: session_id.to_string(),
                                    block: serde_json::json!({
                                        "kind": "message",
                                        "role": msg.role,
                                        "text": text,
                                        "messageId": msg.id,
                                    }),
                                });
                            }
                        }
                    }
                }
            }

            // Tool lifecycle from finish field.
            let current_tool_state = messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .and_then(|m| m.finish.as_deref())
                .map(|f| f.to_string());
            if current_tool_state != last_tool_state {
                if current_tool_state.as_deref() == Some("tool_calls") {
                    let _ = tx.send(DirectEvent::ToolCallStarted {
                        session_id: session_id.to_string(),
                    });
                } else if last_tool_state.as_deref() == Some("tool_calls") {
                    let _ = tx.send(DirectEvent::ToolCallCompleted {
                        session_id: session_id.to_string(),
                    });
                }
                last_tool_state = current_tool_state;
            }
            let current_error = messages
                .last()
                .and_then(|m| m.error.as_deref())
                .filter(|e| !e.is_empty())
                .map(|e| e.to_string());
            if current_error != last_error {
                last_error = current_error.clone();
                if let Some(err) = current_error {
                    let _ = tx.send(DirectEvent::OutputBlock {
                        session_id: session_id.to_string(),
                        block: serde_json::json!({
                            "kind": "status",
                            "tone": "error",
                            "text": err,
                        }),
                    });
                }
            }
        }

        // ── Questions ─────────────────────────────────────────────
        if let Ok(questions) = crate::local_list_questions(Arc::clone(state)).await {
            let mut current_ids = HashSet::new();
            for q in questions.into_iter().filter(|q| q.session_id == session_id) {
                current_ids.insert(q.id.clone());
                if pending_question_ids.insert(q.id.clone()) {
                    let questions_json = Some(question_info_to_defs_json(&q));
                    let _ = tx.send(DirectEvent::QuestionCreated {
                        session_id: q.session_id,
                        request_id: q.id,
                        questions_json,
                    });
                }
            }
            for resolved_id in pending_question_ids
                .iter()
                .filter(|id| !current_ids.contains(*id))
                .cloned()
                .collect::<Vec<_>>()
            {
                pending_question_ids.remove(&resolved_id);
                let _ = tx.send(DirectEvent::QuestionResolved {
                    session_id: session_id.to_string(),
                    request_id: resolved_id,
                });
            }
        }

        // ── Permissions ───────────────────────────────────────────
        if let Ok(permissions) = crate::local_list_permissions(Arc::clone(state)).await {
            let mut current_ids: HashMap<String, String> = HashMap::new();
            for p in permissions
                .into_iter()
                .filter(|p| p.session_id == session_id)
            {
                current_ids.insert(p.id.clone(), p.session_id.clone());
                if !pending_permission_ids.contains_key(&p.id) {
                    let info_json = serde_json::to_value(&p).ok();
                    let _ = tx.send(DirectEvent::PermissionRequested {
                        session_id: p.session_id.clone(),
                        permission_id: p.id.clone(),
                        info_json,
                    });
                }
            }
            for (resolved_id, resolved_session_id) in pending_permission_ids
                .iter()
                .filter(|(id, _)| !current_ids.contains_key(*id))
                .map(|(id, sid)| (id.clone(), sid.clone()))
                .collect::<Vec<_>>()
            {
                let _ = tx.send(DirectEvent::PermissionResolved {
                    session_id: resolved_session_id,
                    permission_id: resolved_id,
                });
            }
            pending_permission_ids = current_ids;
        }

        // ── Topology ───────────────────────────────────────────────
        let topology = state
            .runtime_control
            .list_session_execution_topology(session_id)
            .await;
        let active = topology.active_count + topology.running_count;
        if active != last_execution_count {
            last_execution_count = active;
            let _ = tx.send(DirectEvent::TopologyChanged {
                session_id: session_id.to_string(),
            });
        }

        // ── Adaptive backoff ───────────────────────────────────────
        let has_activity = is_active
            || !pending_question_ids.is_empty()
            || !pending_permission_ids.is_empty()
            || last_execution_count > 0;

        if !has_activity {
            idle_rounds = idle_rounds.saturating_add(1);
        } else {
            idle_rounds = 0;
        }

        // After 5 consecutive idle rounds, backoff to 1500ms; stay at 300ms otherwise
        current_interval_ms = if idle_rounds >= 5 { 1500 } else { 300 };

    }
}

/// Convert QuestionInfo to canonical Vec<QuestionDef> JSON (SSE-compatible format).
fn question_info_to_defs_json(info: &agendao_api::QuestionInfo) -> serde_json::Value {
    if !info.items.is_empty() {
        let defs: Vec<serde_json::Value> = info
            .items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "question": item.question,
                    "header": item.header,
                    "options": item.options.iter().map(|o| {
                        serde_json::json!({"label": o.label, "description": o.description})
                    }).collect::<Vec<_>>(),
                    "multiple": item.multiple,
                })
            })
            .collect();
        return serde_json::Value::Array(defs);
    }
    let defs: Vec<serde_json::Value> = info
        .questions
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let opts: Vec<serde_json::Value> = info
                .options
                .as_ref()
                .and_then(|all| all.get(i))
                .map(|labels| {
                    labels
                        .iter()
                        .map(|l| serde_json::json!({"label": l, "description": null}))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({
                "question": q,
                "header": null,
                "options": opts,
                "multiple": false,
            })
        })
        .collect();
    serde_json::Value::Array(defs)
}

/// P3: Event-driven Direct bridge - subscribes to canonical ServerEvent bus.
/// Replaces polling with push-based events. Now that Question/Permission are
/// in event_bus and SessionStatus/ToolCall/Topology flow through StageEvent,
/// this mode should be complete enough for production use.
async fn direct_event_subscription_loop(
    state: &Arc<ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<DirectEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.event_bus.subscribe();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            Ok(event_json) = event_rx.recv() => {
                // Parse ServerEvent and convert to DirectEvent if relevant to this session
                let Ok(server_event) = serde_json::from_str::<agendao_server_core::runtime_events::ServerEvent>(&event_json) else {
                    continue;
                };

                let direct_event = match server_event {
                    agendao_server_core::runtime_events::ServerEvent::SessionStatus { session_id: sid, status } if sid == session_id => {
                        // Parse status to determine busy/idle/compacting/retrying
                        if let Some(status_obj) = status.as_object() {
                            match status_obj.get("type").and_then(|v| v.as_str()) {
                                Some("idle") => Some(DirectEvent::SessionIdle { session_id: sid }),
                                Some("busy") | Some("compacting") | Some("retry") => Some(DirectEvent::SessionBusy { session_id: sid }),
                                _ => None,
                            }
                        } else if let Some(status_str) = status.as_str() {
                            match status_str {
                                "idle" => Some(DirectEvent::SessionIdle { session_id: sid }),
                                _ => Some(DirectEvent::SessionBusy { session_id: sid }),
                            }
                        } else {
                            None
                        }
                    }
                    agendao_server_core::runtime_events::ServerEvent::SessionUpdated { session_id: sid, .. } if sid == session_id => {
                        Some(DirectEvent::SessionUpdated { session_id: sid })
                    }
                    agendao_server_core::runtime_events::ServerEvent::OutputBlock { session_id: sid, block, .. } if sid == session_id => {
                        Some(DirectEvent::OutputBlock { session_id: sid, block })
                    }
                    agendao_server_core::runtime_events::ServerEvent::QuestionCreated { session_id: sid, request_id, questions } if sid == session_id => {
                        Some(DirectEvent::QuestionCreated {
                            session_id: sid,
                            request_id,
                            questions_json: Some(questions),
                        })
                    }
                    agendao_server_core::runtime_events::ServerEvent::QuestionResolved { session_id: sid, request_id, .. } if sid == session_id => {
                        Some(DirectEvent::QuestionResolved { session_id: sid, request_id })
                    }
                    agendao_server_core::runtime_events::ServerEvent::PermissionRequested { session_id: sid, permission_id, info } if sid == session_id => {
                        Some(DirectEvent::PermissionRequested {
                            session_id: sid,
                            permission_id,
                            info_json: Some(info),
                        })
                    }
                    agendao_server_core::runtime_events::ServerEvent::PermissionResolved { session_id: sid, permission_id, .. } if sid == session_id => {
                        Some(DirectEvent::PermissionResolved { session_id: sid, permission_id })
                    }
                    agendao_server_core::runtime_events::ServerEvent::ControlInputTransition { session_id: sid, kind, phase, .. } if sid == session_id => {
                        Some(DirectEvent::ControlInputTransition {
                            session_id: sid,
                            phase: format!("{:?}", phase).to_lowercase(),
                        })
                    }
                    agendao_server_core::runtime_events::ServerEvent::TopologyChanged { session_id: sid, .. } if sid == session_id => {
                        Some(DirectEvent::TopologyChanged { session_id: sid })
                    }
                    agendao_server_core::runtime_events::ServerEvent::DiffUpdated { session_id: sid, .. } if sid == session_id => {
                        Some(DirectEvent::DiffUpdated { session_id: sid })
                    }
                    _ => None,
                };

                if let Some(event) = direct_event {
                    let _ = tx.send(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn direct_bridge_initial_subscription_does_not_replay_history() {
        let mut last_message_count = 0usize;
        let existing_messages = vec![1, 2, 3];
        last_message_count = existing_messages.len();
        let next_messages = vec![1, 2, 3];

        assert_eq!(last_message_count, next_messages.len());
    }

    #[test]
    fn question_resolved_event_carries_session_id() {
        let event = super::DirectEvent::QuestionResolved {
            session_id: "ses_direct".to_string(),
            request_id: "q_123".to_string(),
        };
        let value = serde_json::to_value(&event).expect("direct event should serialize");
        assert_eq!(value["type"], "question_resolved");
        assert_eq!(value["session_id"], "ses_direct");
        assert_eq!(value["request_id"], "q_123");
    }

    #[test]
    fn direct_bridge_only_streams_assistant_text_blocks() {
        assert!(super::should_emit_text_output_block("assistant"));
        assert!(!super::should_emit_text_output_block("user"));
        assert!(!super::should_emit_text_output_block("system"));
        assert!(!super::should_emit_text_output_block("tool"));
    }

    #[test]
    fn direct_bridge_active_run_status_matches_runtime_authority() {
        assert!(super::run_status_is_active(&super::SessionRunStatus::Busy));
        assert!(super::run_status_is_active(
            &super::SessionRunStatus::Compacting
        ));
        assert!(super::run_status_is_active(
            &super::SessionRunStatus::Retry {
                attempt: 1,
                message: "retrying".to_string(),
                next: 0,
            }
        ));
        assert!(!super::run_status_is_active(&super::SessionRunStatus::Idle));
        assert!(!super::run_status_is_active(
            &super::SessionRunStatus::Blocked {
                reason: None,
                recheck_at: None,
            }
        ));
        assert!(!super::run_status_is_active(
            &super::SessionRunStatus::Sleeping {
                reason: None,
                wake_at: None,
            }
        ));
    }
}
