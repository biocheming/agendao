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

use std::sync::Arc;
use std::time::Duration;

/// Events emitted by the Direct bridge. These map 1:1 to ServerEvent
/// categories and are intended to be converted into frontend-internal
/// dispatch types by a thin adapter in each frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DirectEvent {
    SessionBusy { session_id: String },
    SessionIdle { session_id: String },
    SessionUpdated { session_id: String },
    OutputBlock { session_id: String, block: serde_json::Value },
    QuestionCreated { session_id: String, request_id: String, questions_json: Option<serde_json::Value> },
    QuestionResolved { request_id: String },
    PermissionRequested { session_id: String, permission_id: String, info_json: Option<serde_json::Value> },
    PermissionResolved { session_id: String, permission_id: String },
    ToolCallStarted { session_id: String },
    ToolCallCompleted { session_id: String },
    ConfigUpdated,
    ControlInputTransition {
        session_id: String,
        phase: String, // "start" | "end"
    },
    TopologyChanged { session_id: String },
    DiffUpdated { session_id: String },
    SessionTreeChanged { session_id: String },
}

/// Spawn a Direct-mode event loop for one session. Returns a receiver
/// that the frontend consumes.
pub fn spawn_direct_event_loop(
    state: Arc<ServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<DirectEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        direct_poll_loop(&state, &session_id, tx, cancel).await;
    });
    rx
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
    let mut stale_ticks = 0u32;
    let mut was_idle = false;
    let mut last_tool_state: Option<String> = None;
    let mut pending_question_ids = HashSet::new();
    let mut pending_permission_ids: HashMap<String, String> = HashMap::new();
    let mut last_execution_count = 0usize;
    let mut last_config_snapshot: Option<String> = None;
    let mut last_error: Option<String> = None;
    let mut interval = tokio::time::interval(Duration::from_millis(300));

    let _ = tx.send(DirectEvent::SessionBusy {
        session_id: session_id.to_string(),
    });
    let _ = tx.send(DirectEvent::ControlInputTransition {
        session_id: session_id.to_string(),
        phase: "start".to_string(),
    });

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {},
        }

        // ── Messages ──────────────────────────────────────────────
        let Ok(messages) = crate::local_list_messages(
            Arc::clone(state),
            session_id,
            None,
            None,
        )
        .await
        else {
            break;
        };

        let count = messages.len();
        if count > last_message_count {
            let prev_count = last_message_count;
            last_message_count = count;
            stale_ticks = 0;
            was_idle = false;
            let _ = tx.send(DirectEvent::SessionUpdated {
                session_id: session_id.to_string(),
            });

            // Emit output blocks for new messages.
            for msg in messages.iter().skip(prev_count) {
                if msg.role == "assistant" || msg.role == "user" {
                    for part in &msg.parts {
                        if let Some(text) = part.text.as_deref() {
                            let _ = tx.send(DirectEvent::OutputBlock {
                                session_id: session_id.to_string(),
                                block: serde_json::json!({
                                    "kind": if msg.role == "user" { "message" } else { "message" },
                                    "role": msg.role,
                                    "text": text,
                                    "messageId": msg.id,
                                }),
                            });
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
        } else {
            // Idle detection.
            let has_terminal_finish = messages
                .last()
                .filter(|m| m.role == "assistant")
                .and_then(|m| m.finish.as_deref())
                .map(|f| f != "tool_calls" && f != "unknown")
                .unwrap_or(false);

            if !has_terminal_finish {
                stale_ticks = 0;
                if was_idle {
                    was_idle = false;
                    let _ = tx.send(DirectEvent::SessionBusy {
                        session_id: session_id.to_string(),
                    });
                }
            } else {
                stale_ticks += 1;
                if stale_ticks >= 10 && !was_idle {
                    was_idle = true;
                    let _ = tx.send(DirectEvent::SessionIdle {
                        session_id: session_id.to_string(),
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
                    request_id: resolved_id,
                });
            }
        }

        // ── Permissions ───────────────────────────────────────────
        if let Ok(permissions) = crate::local_list_permissions(Arc::clone(state)).await {
            let mut current_ids: HashMap<String, String> = HashMap::new();
            for p in permissions.into_iter().filter(|p| p.session_id == session_id) {
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

        // ── Config ────────────────────────────────────────────────
        if let Ok(config) = crate::local_get_config(Arc::clone(state)).await {
            let snapshot = format!("{:?}", config);
            if last_config_snapshot.as_deref() != Some(&snapshot) {
                last_config_snapshot = Some(snapshot);
                let _ = tx.send(DirectEvent::ConfigUpdated);
            }
        }

        // ── Error ──────────────────────────────────────────────────
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
}

/// Convert QuestionInfo to canonical Vec<QuestionDef> JSON (SSE-compatible format).
fn question_info_to_defs_json(info: &rocode_api::QuestionInfo) -> serde_json::Value {
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
