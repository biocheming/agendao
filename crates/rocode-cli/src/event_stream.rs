//! SSE (Server-Sent Events) subscription for the CLI.
//!
//! Connects to the server's `/event` endpoint and forwards parsed events
//! to the CLI's REPL loop via an unbounded channel.  Auto-reconnects on
//! disconnect with exponential backoff (capped at 5 s).

use std::time::Duration;

use tokio::sync::mpsc;

use crate::util::server_url;

/// Interactive CLI renders assistant/tool output incrementally, so it must
/// subscribe to a high-frequency tier instead of the low-frequency `cli`
/// tier, which is final-only and strips `output_block` events server-side.
const INTERACTIVE_CLI_SSE_TIER: &str = "tui";

// ── Event types ──────────────────────────────────────────────────────

/// Events the CLI cares about, parsed from the SSE stream.
#[derive(Debug, Clone)]
pub enum CliServerEvent {
    /// Global config changed — refetch `/config` for updated preferences.
    ConfigUpdated,
    /// Session state changed — fetch latest session to diff messages.
    SessionUpdated {
        session_id: String,
        source: Option<String>,
    },
    /// Session became busy (prompt running).
    SessionBusy { session_id: String },
    /// Session became idle (prompt finished).
    SessionIdle { session_id: String },
    /// Session is retrying.
    SessionRetrying { session_id: String },
    /// A question was created and needs user interaction.
    QuestionCreated {
        request_id: String,
        session_id: String,
        /// Raw question definitions from the server (Vec<QuestionDef> JSON).
        questions_json: serde_json::Value,
    },
    /// A question was resolved (answered, rejected, or cancelled).
    QuestionResolved { request_id: String },
    /// A permission request was created and needs user interaction.
    PermissionRequested {
        session_id: String,
        permission_id: String,
        info_json: serde_json::Value,
    },
    /// A permission request was resolved.
    PermissionResolved {
        session_id: String,
        permission_id: String,
    },
    /// A tool call started.
    ToolCallStarted {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
    },
    /// A tool call completed.
    ToolCallCompleted {
        session_id: String,
        tool_call_id: String,
    },
    /// An attached session was attached under a parent session in the active tree.
    AttachedSessionAttached {
        parent_id: String,
        attached_id: String,
    },
    /// An attached session was detached from a parent session in the active tree.
    AttachedSessionDetached {
        parent_id: String,
        attached_id: String,
    },
    /// An output block was emitted (message, tool result, etc.).
    OutputBlock {
        session_id: String,
        id: Option<String>,
        payload: serde_json::Value,
    },
    /// An error event from the server.
    Error {
        session_id: String,
        error: String,
        message_id: Option<String>,
        done: Option<bool>,
    },
    /// Token usage update.
    Usage {
        session_id: String,
        prompt_tokens: u64,
        completion_tokens: u64,
        message_id: Option<String>,
    },
    /// Unknown event type (logged but ignored).
    Unknown {
        event: String,
        data: serde_json::Value,
    },
}

// ── SSE subscriber ───────────────────────────────────────────────────

/// Spawn a background task that subscribes to the server's SSE stream
/// and forwards parsed events to `tx`.
///
/// The task runs until `tx` is closed (all receivers dropped) or the
/// cancellation token is cancelled.
pub fn spawn_sse_subscriber(
    base_url: String,
    session_id: String,
    tx: mpsc::UnboundedSender<CliServerEvent>,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Duration::from_millis(400);
        let max_backoff = Duration::from_secs(5);

        loop {
            if cancel.is_cancelled() {
                break;
            }

            match connect_and_consume(&base_url, &session_id, &tx, &cancel).await {
                Ok(()) => {
                    // Clean shutdown (cancel requested).
                    break;
                }
                Err(e) => {
                    if cancel.is_cancelled() {
                        break;
                    }
                    tracing::warn!("SSE connection lost: {}. Reconnecting in {:?}…", e, backoff);
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = cancel.cancelled() => break,
                    }
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }

        tracing::debug!("SSE subscriber exiting");
    })
}

/// Connect to the SSE endpoint and consume events until disconnect.
async fn connect_and_consume(
    base_url: &str,
    session_id: &str,
    tx: &mpsc::UnboundedSender<CliServerEvent>,
    cancel: &tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    // Subscribe with server-side session filter so we only receive events
    // relevant to our session (plus global events like config.updated).
    // This replaces most client-side is_my_session filtering, though we
    // keep the client-side checks as a defense-in-depth measure.
    let url = interactive_cli_sse_url(base_url, session_id);
    let client = reqwest::Client::new();

    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("SSE connect failed: {}", resp.status());
    }

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    use futures::StreamExt;

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete SSE frames (separated by blank lines).
                        while let Some(pos) = buffer.find("\n\n") {
                            let frame = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();

                            current_event.clear();
                            current_data.clear();

                            for line in frame.lines() {
                                if let Some(value) = line.strip_prefix("event: ") {
                                    current_event = value.trim().to_string();
                                } else if let Some(value) = line.strip_prefix("data: ") {
                                    if !current_data.is_empty() {
                                        current_data.push('\n');
                                    }
                                    current_data.push_str(value);
                                } else if line.starts_with("data:") {
                                    // data: with no space — value is empty
                                    if !current_data.is_empty() {
                                        current_data.push('\n');
                                    }
                                }
                            }

                            if !current_data.is_empty() {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&current_data) {
                                    if let Some(evt) = parse_event(&current_event, &json, session_id) {
                                        if tx.send(evt).is_err() {
                                            // Receiver dropped.
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        return Err(e.into());
                    }
                    None => {
                        // Stream ended.
                        anyhow::bail!("SSE stream ended");
                    }
                }
            }
            _ = cancel.cancelled() => {
                return Ok(());
            }
        }
    }
}

fn interactive_cli_sse_url(base_url: &str, session_id: &str) -> String {
    format!(
        "{}?session={}&tier={}",
        server_url(base_url, "/event"),
        session_id,
        INTERACTIVE_CLI_SSE_TIER,
    )
}

/// Parse an SSE event JSON payload into a `CliServerEvent`.
///
/// Returns `None` for events that belong to other sessions (session
/// filtering) or events we don't care about.
fn parse_event(
    event_name: &str,
    json: &serde_json::Value,
    my_session_id: &str,
) -> Option<CliServerEvent> {
    // Helper to extract session_id from various field names.
    let event_session_id = json
        .get("sessionID")
        .or_else(|| json.get("sessionId"))
        .or_else(|| json.get("session_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Determine event type from SSE event name or payload's "type" field.
    let event_type = if !event_name.is_empty() {
        event_name.to_string()
    } else {
        json.get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    // Global events (no session filter).
    if event_type.as_str() == "config.updated" {
        return Some(CliServerEvent::ConfigUpdated);
    }

    // Session-scoped events — filter by session_id.
    let is_my_session = event_session_id.is_empty() || event_session_id == my_session_id;

    match event_type.as_str() {
        "session.updated" => {
            if !is_my_session {
                return None;
            }
            let source = json
                .get("source")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(CliServerEvent::SessionUpdated {
                session_id: event_session_id.to_string(),
                source,
            })
        }
        "session.status" => {
            if !is_my_session {
                return None;
            }
            // Support both plain string "idle" and tagged object {"type": "idle"}
            // formats. The server serializes SessionRunStatus with
            // #[serde(tag = "type")] so the value is an object, but we also
            // accept a plain string for forward compatibility.
            let status = json
                .get("status")
                .and_then(|v| {
                    v.as_str()
                        .or_else(|| v.get("type").and_then(|t| t.as_str()))
                })
                .unwrap_or("");
            match status {
                "busy" => Some(CliServerEvent::SessionBusy {
                    session_id: event_session_id.to_string(),
                }),
                "idle" => Some(CliServerEvent::SessionIdle {
                    session_id: event_session_id.to_string(),
                }),
                "retry" => Some(CliServerEvent::SessionRetrying {
                    session_id: event_session_id.to_string(),
                }),
                _ => None,
            }
        }
        "question.created" => {
            // Questions may come from child/subsessions — always handle them
            // so the CLI user can answer regardless of which session asked.
            let request_id = json
                .get("requestID")
                .or_else(|| json.get("requestId"))
                .or_else(|| json.get("request_id"))
                .or_else(|| json.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let questions_json = json
                .get("questions")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            Some(CliServerEvent::QuestionCreated {
                request_id,
                session_id: event_session_id.to_string(),
                questions_json,
            })
        }
        "question.resolved" | "question.replied" | "question.rejected" => {
            let request_id = json
                .get("requestID")
                .or_else(|| json.get("requestId"))
                .or_else(|| json.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(CliServerEvent::QuestionResolved { request_id })
        }
        "permission.requested" => {
            let permission_id = json
                .get("permissionID")
                .or_else(|| json.get("permissionId"))
                .or_else(|| json.get("requestID"))
                .or_else(|| json.get("requestId"))
                .or_else(|| json.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let info_json = json.get("info").cloned().unwrap_or(serde_json::Value::Null);
            Some(CliServerEvent::PermissionRequested {
                session_id: event_session_id.to_string(),
                permission_id,
                info_json,
            })
        }
        "permission.resolved" | "permission.replied" => {
            let session_id = event_session_id.to_string();
            let permission_id = json
                .get("permissionID")
                .or_else(|| json.get("permissionId"))
                .or_else(|| json.get("requestID"))
                .or_else(|| json.get("requestId"))
                .or_else(|| json.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(CliServerEvent::PermissionResolved {
                session_id,
                permission_id,
            })
        }
        "tool_call.lifecycle" => {
            let tool_call_id = json
                .get("toolCallId")
                .or_else(|| json.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match json.get("phase").and_then(|v| v.as_str()).unwrap_or("") {
                "start" => {
                    let tool_name = json
                        .get("toolName")
                        .or_else(|| json.get("tool_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(CliServerEvent::ToolCallStarted {
                        session_id: event_session_id.to_string(),
                        tool_call_id,
                        tool_name,
                    })
                }
                "complete" => Some(CliServerEvent::ToolCallCompleted {
                    session_id: event_session_id.to_string(),
                    tool_call_id,
                }),
                _ => None,
            }
        }
        "tool_call.start" => {
            let tool_call_id = json
                .get("toolCallId")
                .or_else(|| json.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_name = json
                .get("toolName")
                .or_else(|| json.get("tool_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(CliServerEvent::ToolCallStarted {
                session_id: event_session_id.to_string(),
                tool_call_id,
                tool_name,
            })
        }
        "tool_call.complete" => {
            let tool_call_id = json
                .get("toolCallId")
                .or_else(|| json.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(CliServerEvent::ToolCallCompleted {
                session_id: event_session_id.to_string(),
                tool_call_id,
            })
        }
        "attached_session.attached" => {
            let parent_id = json
                .get("parentID")
                .or_else(|| json.get("parentId"))
                .or_else(|| json.get("parent_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let attached_id = json
                .get("attachedID")
                .or_else(|| json.get("childId"))
                .or_else(|| json.get("attached_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(CliServerEvent::AttachedSessionAttached {
                parent_id,
                attached_id,
            })
        }
        "attached_session.detached" => {
            let parent_id = json
                .get("parentID")
                .or_else(|| json.get("parentId"))
                .or_else(|| json.get("parent_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let attached_id = json
                .get("attachedID")
                .or_else(|| json.get("childId"))
                .or_else(|| json.get("attached_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(CliServerEvent::AttachedSessionDetached {
                parent_id,
                attached_id,
            })
        }
        "output_block" => {
            // Output blocks may or may not carry a session_id.
            let id = json.get("id").and_then(|v| v.as_str()).map(String::from);
            Some(CliServerEvent::OutputBlock {
                session_id: event_session_id.to_string(),
                id,
                payload: json.clone(),
            })
        }
        "error" => {
            let error = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            let message_id = json
                .get("message_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let done = json.get("done").and_then(|v| v.as_bool());
            Some(CliServerEvent::Error {
                session_id: event_session_id.to_string(),
                error,
                message_id,
                done,
            })
        }
        "usage" => {
            let prompt_tokens = json
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let completion_tokens = json
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let message_id = json
                .get("message_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(CliServerEvent::Usage {
                session_id: event_session_id.to_string(),
                prompt_tokens,
                completion_tokens,
                message_id,
            })
        }
        _ => {
            tracing::trace!("Unknown SSE event: {}", event_type);
            Some(CliServerEvent::Unknown {
                event: event_type,
                data: json.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{interactive_cli_sse_url, parse_event, CliServerEvent};

    #[test]
    fn output_block_is_filtered_to_current_session() {
        let mine = serde_json::json!({
            "type": "output_block",
            "sessionID": "session-1",
            "block": {
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "hi"
            }
        });
        let other = serde_json::json!({
            "type": "output_block",
            "sessionID": "session-2",
            "block": {
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "nope"
            }
        });

        let mine_event = parse_event("", &mine, "session-1");
        let other_event = parse_event("", &other, "session-1");

        assert!(matches!(
            mine_event,
            Some(CliServerEvent::OutputBlock { session_id, .. }) if session_id == "session-1"
        ));
        assert!(matches!(
            other_event,
            Some(CliServerEvent::OutputBlock { session_id, .. }) if session_id == "session-2"
        ));
    }

    #[test]
    fn interactive_cli_subscribes_to_high_frequency_sse_tier() {
        let url = interactive_cli_sse_url("http://127.0.0.1:8757", "session-1");
        assert_eq!(url, "http://127.0.0.1:8757/event?session=session-1&tier=tui");
    }

    #[test]
    fn config_updated_is_parsed_as_global_event() {
        let payload = serde_json::json!({
            "type": "config.updated"
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(event, Some(CliServerEvent::ConfigUpdated)));
    }

    #[test]
    fn question_created_from_attached_session_is_not_filtered() {
        let payload = serde_json::json!({
            "type": "question.created",
            "sessionID": "attached-session-1",
            "requestID": "question-1",
            "questions": [{
                "header": "Scope",
                "question": "Proceed?",
                "options": [{"label": "Yes"}]
            }]
        });

        let event = parse_event("", &payload, "parent-session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::QuestionCreated { session_id, request_id, .. })
                if session_id == "attached-session-1" && request_id == "question-1"
        ));
    }

    #[test]
    fn attached_session_attach_event_is_parsed() {
        let payload = serde_json::json!({
            "type": "attached_session.attached",
            "parentID": "parent-session-1",
            "attachedID": "attached-session-1"
        });

        let event = parse_event("", &payload, "parent-session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::AttachedSessionAttached { parent_id, attached_id })
                if parent_id == "parent-session-1" && attached_id == "attached-session-1"
        ));
    }

    #[test]
    fn canonical_question_resolved_event_is_parsed() {
        let payload = serde_json::json!({
            "type": "question.resolved",
            "sessionID": "session-1",
            "requestID": "question-1",
            "resolution": "answered",
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::QuestionResolved { request_id }) if request_id == "question-1"
        ));
    }

    #[test]
    fn canonical_tool_call_lifecycle_complete_event_is_parsed() {
        let payload = serde_json::json!({
            "type": "tool_call.lifecycle",
            "sessionID": "session-1",
            "toolCallId": "tool-1",
            "phase": "complete",
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::ToolCallCompleted { session_id, tool_call_id })
                if session_id == "session-1" && tool_call_id == "tool-1"
        ));
    }

    #[test]
    fn canonical_permission_requested_event_is_parsed() {
        let payload = serde_json::json!({
            "type": "permission.requested",
            "sessionID": "session-1",
            "permissionID": "permission-1",
            "info": {
                "id": "permission-1",
                "session_id": "session-1",
                "tool": "bash",
                "input": {"permission": "bash", "patterns": ["cargo test"]},
                "message": "Permission required"
            }
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::PermissionRequested { session_id, permission_id, .. })
                if session_id == "session-1" && permission_id == "permission-1"
        ));
    }

    #[test]
    fn session_status_idle_tagged_object_is_parsed() {
        // The server serializes SessionRunStatus with #[serde(tag = "type")]
        // so idle becomes {"type": "idle"} rather than a plain string.
        let payload = serde_json::json!({
            "type": "session.status",
            "sessionID": "session-1",
            "status": {"type": "idle"}
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::SessionIdle { session_id }) if session_id == "session-1"
        ));
    }

    #[test]
    fn session_status_busy_tagged_object_is_parsed() {
        let payload = serde_json::json!({
            "type": "session.status",
            "sessionID": "session-1",
            "status": {"type": "busy"}
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::SessionBusy { session_id }) if session_id == "session-1"
        ));
    }

    #[test]
    fn session_status_plain_string_is_parsed() {
        // Forward-compatible: also accept plain string format.
        let payload = serde_json::json!({
            "type": "session.status",
            "sessionID": "session-1",
            "status": "idle"
        });

        let event = parse_event("", &payload, "session-1");

        assert!(matches!(
            event,
            Some(CliServerEvent::SessionIdle { session_id }) if session_id == "session-1"
        ));
    }
}
