use super::*;
use crate::bridge::UiBridge;
use futures::StreamExt;
use reqwest::Url;
use reqwest_eventsource::{Event as SseEvent, EventSource};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::task::JoinHandle;

pub(super) fn env_var_enabled(name: &str) -> bool {
    let Ok(value) = std::env::var(name) else {
        return false;
    };
    let normalized = value.trim().to_ascii_lowercase();
    !normalized.is_empty() && !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
}

pub(super) fn env_var_with_fallback(primary: &str, fallback: &str) -> Option<String> {
    std::env::var(primary)
        .ok()
        .or_else(|| std::env::var(fallback).ok())
}

pub(super) fn resolve_tui_base_url() -> String {
    if let Some(value) = env_var_with_fallback("ROCODE_TUI_BASE_URL", "OPENCODE_TUI_BASE_URL") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // Prefer a live backend endpoint over a hardcoded default. This avoids
    // accidental 404s when localhost:3000 is occupied by a non-opencode service.
    let candidates = [
        "http://127.0.0.1:3000",
        "http://localhost:3000",
        "http://127.0.0.1:4096",
        "http://localhost:4096",
    ];
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
    {
        Ok(client) => client,
        Err(_) => return "http://localhost:3000".to_string(),
    };

    for base in candidates {
        let health_url = format!("{}/health", base);
        if let Ok(response) = client.get(&health_url).send() {
            if response.status().is_success() {
                return base.to_string();
            }
        }
    }

    "http://localhost:3000".to_string()
}

/// Shared session filter. Updated by the app when the active session changes.
/// The SSE listener task reads this on each reconnect to build the URL.
pub(super) type SessionFilter = Arc<StdMutex<Option<String>>>;

pub(super) fn spawn_server_event_listener_task(
    ui_bridge: UiBridge,
    base_url: String,
    session_filter: SessionFilter,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .build()
        {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(%err, "failed to initialize server event stream client");
                return;
            }
        };

        let base_event_url = format!("{}/event", base_url.trim_end_matches('/'));
        loop {
            let connected_filter = read_session_filter(&session_filter);
            let event_url = build_event_url(&base_event_url, connected_filter.as_deref());
            let mut source = match EventSource::new(client.get(event_url.clone())) {
                Ok(source) => source,
                Err(err) => {
                    tracing::warn!(
                        %err,
                        url = %event_url,
                        "failed to initialize server event source"
                    );
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    continue;
                }
            };

            let reconnect_due_to_filter_change = consume_server_event_stream(
                &mut source,
                &ui_bridge,
                &session_filter,
                &connected_filter,
            )
            .await;
            if reconnect_due_to_filter_change {
                continue;
            }

            if read_session_filter(&session_filter) == connected_filter {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    })
}

async fn consume_server_event_stream(
    source: &mut EventSource,
    ui_bridge: &UiBridge,
    session_filter: &SessionFilter,
    connected_filter: &Option<String>,
) -> bool {
    while let Some(event) = source.next().await {
        match event {
            Ok(SseEvent::Open) => {
                tracing::debug!(filter = ?connected_filter, "server event stream connected");
            }
            Ok(SseEvent::Message(message)) => {
                forward_server_event_payload(&message.data, ui_bridge);

                // Match the previous behavior: reconnect after a complete
                // event if the active session filter changed.
                let current = read_session_filter(session_filter);
                if current != *connected_filter {
                    tracing::debug!(
                        old = ?connected_filter,
                        new = ?current,
                        "session filter changed, reconnecting SSE"
                    );
                    source.close();
                    return true;
                }
            }
            Err(err) => {
                tracing::debug!(%err, "server event stream disconnected");
                return false;
            }
        }
    }

    false
}

fn build_event_url(base_event_url: &str, session_id: Option<&str>) -> Url {
    let mut url = Url::parse(base_event_url)
        .expect("resolved TUI base URL should always produce a valid event URL");
    if let Some(session_id) = session_id {
        url.query_pairs_mut().append_pair("session", session_id);
    }
    url
}

fn read_session_filter(session_filter: &SessionFilter) -> Option<String> {
    session_filter.lock().ok().and_then(|guard| guard.clone())
}

#[cfg(test)]
fn forward_server_event(data_lines: &[String]) -> Option<Event> {
    if data_lines.is_empty() {
        return None;
    }
    let payload = data_lines.join("\n");
    parse_server_event_payload(&payload)
}

fn forward_server_event_payload(payload: &str, ui_bridge: &UiBridge) {
    if let Some(event) = parse_server_event_payload(payload) {
        let _ = ui_bridge.emit(event);
    }
}

fn parse_server_event_payload(payload: &str) -> Option<Event> {
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return None;
    };
    parse_server_event_value(value)
}

fn parse_server_event_value(value: serde_json::Value) -> Option<Event> {
    let event_type = value.get("type").and_then(|item| item.as_str());
    let session_id = value
        .get("sessionID")
        .and_then(|item| item.as_str())
        .or_else(|| value.get("sessionId").and_then(|item| item.as_str()));

    match event_type {
        Some("session.updated") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let source = value
                .get("source")
                .and_then(|item| item.as_str())
                .map(str::to_string);
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::SessionUpdated {
                    session_id: session_id.to_string(),
                    source,
                },
            ))))
        }
        Some("config.updated") => Some(Event::Custom(Box::new(CustomEvent::StateChanged(
            StateChange::ConfigUpdated,
        )))),
        Some("session.status") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let status_type = value
                .get("status")
                .and_then(|status| status.get("type"))
                .and_then(|item| item.as_str())
                .or_else(|| value.get("status").and_then(|item| item.as_str()));
            match status_type {
                Some("busy") => Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                    StateChange::SessionStatusBusy(session_id.to_string()),
                )))),
                Some("idle") => Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                    StateChange::SessionStatusIdle(session_id.to_string()),
                )))),
                Some("retry") => {
                    let attempt = value
                        .get("status")
                        .and_then(|status| status.get("attempt"))
                        .and_then(|item| item.as_u64())
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(0);
                    let message = value
                        .get("status")
                        .and_then(|status| status.get("message"))
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let next = value
                        .get("status")
                        .and_then(|status| status.get("next"))
                        .and_then(|item| item.as_i64())
                        .unwrap_or_default();
                    Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                        StateChange::SessionStatusRetrying {
                            session_id: session_id.to_string(),
                            attempt,
                            message,
                            next,
                        },
                    ))))
                }
                _ => None,
            }
        }
        Some("question.created") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(request_id) = value
                .get("requestID")
                .and_then(|item| item.as_str())
                .or_else(|| value.get("requestId").and_then(|item| item.as_str()))
            else {
                return None;
            };
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::QuestionCreated {
                    session_id: session_id.to_string(),
                    request_id: request_id.to_string(),
                },
            ))))
        }
        Some("question.resolved") | Some("question.replied") | Some("question.rejected") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(request_id) = value
                .get("requestID")
                .and_then(|item| item.as_str())
                .or_else(|| value.get("requestId").and_then(|item| item.as_str()))
            else {
                return None;
            };
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::QuestionResolved {
                    session_id: session_id.to_string(),
                    request_id: request_id.to_string(),
                },
            ))))
        }
        Some("permission.requested") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(info) = value.get("info").cloned() else {
                return None;
            };
            let Ok(permission) = serde_json::from_value::<crate::api::PermissionRequestInfo>(info)
            else {
                return None;
            };
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::PermissionRequested {
                    session_id: session_id.to_string(),
                    permission,
                },
            ))))
        }
        Some("permission.resolved") | Some("permission.replied") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(permission_id) = value
                .get("permissionID")
                .and_then(|item| item.as_str())
                .or_else(|| value.get("permissionId").and_then(|item| item.as_str()))
                .or_else(|| value.get("requestID").and_then(|item| item.as_str()))
                .or_else(|| value.get("requestId").and_then(|item| item.as_str()))
            else {
                return None;
            };
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::PermissionResolved {
                    session_id: session_id.to_string(),
                    permission_id: permission_id.to_string(),
                },
            ))))
        }
        Some("tool_call.lifecycle") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(tool_call_id) = value.get("toolCallId").and_then(|item| item.as_str()) else {
                tracing::warn!("tool_call.lifecycle missing toolCallId");
                return None;
            };
            match value.get("phase").and_then(|item| item.as_str()) {
                Some("start") => {
                    let Some(tool_name) = value.get("toolName").and_then(|item| item.as_str())
                    else {
                        tracing::warn!("tool_call.lifecycle start missing toolName");
                        return None;
                    };
                    Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                        StateChange::ToolCallStarted {
                            session_id: session_id.to_string(),
                            tool_call_id: tool_call_id.to_string(),
                            tool_name: tool_name.to_string(),
                        },
                    ))))
                }
                Some("complete") => Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                    StateChange::ToolCallCompleted {
                        session_id: session_id.to_string(),
                        tool_call_id: tool_call_id.to_string(),
                    },
                )))),
                Some(other) => {
                    tracing::debug!(phase = other, "ignoring unknown tool_call.lifecycle phase");
                    None
                }
                None => {
                    tracing::warn!("tool_call.lifecycle missing phase");
                    None
                }
            }
        }
        Some("tool_call.start") => {
            tracing::info!("Received tool_call.start event");
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(tool_call_id) = value.get("toolCallId").and_then(|item| item.as_str()) else {
                tracing::warn!("tool_call.start missing toolCallId");
                return None;
            };
            let Some(tool_name) = value.get("toolName").and_then(|item| item.as_str()) else {
                tracing::warn!("tool_call.start missing toolName");
                return None;
            };
            tracing::info!(
                "Sending ToolCallStarted: id={}, name={}",
                tool_call_id,
                tool_name
            );
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::ToolCallStarted {
                    session_id: session_id.to_string(),
                    tool_call_id: tool_call_id.to_string(),
                    tool_name: tool_name.to_string(),
                },
            ))))
        }
        Some("tool_call.complete") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(tool_call_id) = value.get("toolCallId").and_then(|item| item.as_str()) else {
                return None;
            };
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::ToolCallCompleted {
                    session_id: session_id.to_string(),
                    tool_call_id: tool_call_id.to_string(),
                },
            ))))
        }
        Some("execution.topology.changed") => {
            let Some(session_id) = session_id else {
                return None;
            };
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::TopologyChanged {
                    session_id: session_id.to_string(),
                },
            ))))
        }
        Some("diff.updated") | Some("session.diff") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let diffs = value
                .get("diff")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|entry| {
                            let path = entry.get("path")?.as_str()?;
                            let additions = entry.get("additions")?.as_u64().unwrap_or(0);
                            let deletions = entry.get("deletions")?.as_u64().unwrap_or(0);
                            Some(crate::context::DiffEntry {
                                file: path.to_string(),
                                additions: additions as u32,
                                deletions: deletions as u32,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::DiffUpdated {
                    session_id: session_id.to_string(),
                    diffs,
                },
            ))))
        }
        Some("output_block") => {
            let Some(session_id) = session_id else {
                return None;
            };
            let Some(block) = value.get("block") else {
                return None;
            };
            let id = value
                .get("id")
                .and_then(|item| item.as_str())
                .map(str::to_string);
            Some(Event::Custom(Box::new(CustomEvent::StateChanged(
                StateChange::OutputBlock {
                    session_id: session_id.to_string(),
                    id,
                    payload: block.clone(),
                },
            ))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{build_event_url, forward_server_event};
    use crate::event::{CustomEvent, StateChange};
    use crate::Event;

    #[test]
    fn build_event_url_appends_session_filter() {
        let url = build_event_url("http://localhost:3000/event", Some("session-1"));
        assert_eq!(
            url.as_str(),
            "http://localhost:3000/event?session=session-1"
        );
    }

    #[test]
    fn build_event_url_leaves_unfiltered_stream_plain() {
        let url = build_event_url("http://localhost:3000/event", None);
        assert_eq!(url.as_str(), "http://localhost:3000/event");
    }

    #[test]
    fn output_block_forwarded_with_wrapper_id() {
        let event = forward_server_event(&[serde_json::json!({
            "type": "output_block",
            "sessionID": "session-1",
            "id": "message-1",
            "block": {
                "kind": "reasoning",
                "phase": "delta",
                "text": "thinking",
            }
        })
        .to_string()])
        .expect("reasoning event");

        let Event::Custom(custom) = event else {
            panic!("expected custom event");
        };
        let CustomEvent::StateChanged(StateChange::OutputBlock {
            session_id,
            id,
            payload,
        }) = *custom
        else {
            panic!("expected output block event");
        };

        assert_eq!(session_id, "session-1");
        assert_eq!(id.as_deref(), Some("message-1"));
        assert_eq!(payload["kind"], "reasoning");
        assert_eq!(payload["phase"], "delta");
        assert_eq!(payload["text"], "thinking");
    }

    #[test]
    fn permission_requested_event_is_forwarded() {
        let event = forward_server_event(&[serde_json::json!({
            "type": "permission.requested",
            "sessionID": "session-1",
            "permissionID": "permission-1",
            "info": {
                "id": "permission-1",
                "session_id": "session-1",
                "tool": "bash",
                "input": {
                    "permission": "bash",
                    "patterns": ["cargo test"],
                    "metadata": {"command": "cargo test"}
                },
                "message": "Permission required"
            }
        })
        .to_string()])
        .expect("permission event");

        let Event::Custom(custom) = event else {
            panic!("expected custom event");
        };
        let CustomEvent::StateChanged(StateChange::PermissionRequested {
            session_id,
            permission,
        }) = *custom
        else {
            panic!("expected permission state change");
        };

        assert_eq!(session_id, "session-1");
        assert_eq!(permission.id, "permission-1");
        assert_eq!(permission.tool, "bash");
    }
}
