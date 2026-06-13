use super::*;
use agendao_server_core::frontend_events::FrontendEvent;
use crate::bridge::UiBridge;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Url;
use reqwest_eventsource::{Event as SseEvent, EventSource};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::sync::watch;

pub(super) fn env_var_enabled(name: &str) -> bool {
    let Ok(value) = std::env::var(name) else {
        return false;
    };
    let normalized = value.trim().to_ascii_lowercase();
    !normalized.is_empty() && !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
}

pub(super) fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub(super) fn resolve_tui_base_url(base_url_override: Option<&str>) -> String {
    if let Some(value) = base_url_override {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(value) = env_var("AGENDAO_TUI_BASE_URL") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // Prefer a live backend endpoint over a hardcoded default. This avoids
    // accidental 404s when localhost:3000 is occupied by a non-agendao service.
    let candidates = [
        "http://127.0.0.1:3000",
        "http://localhost:3000",
        "http://127.0.0.1:4096",
        "http://localhost:4096",
    ];
    for base in candidates {
        if endpoint_accepts_tcp(base) {
            return base.to_string();
        }
    }

    "http://localhost:3000".to_string()
}

fn endpoint_accepts_tcp(base: &str) -> bool {
    let Ok(url) = Url::parse(base) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let Some(port) = url.port_or_known_default() else {
        return false;
    };
    let addrs: Vec<SocketAddr> = match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(_) => return false,
    };
    addrs
        .iter()
        .any(|addr| TcpStream::connect_timeout(addr, Duration::from_millis(300)).is_ok())
}

/// Shared session filter. Updated by the app when the active session changes.
/// The SSE listener task reads this on each reconnect to build the URL.
pub(super) type SessionFilter = watch::Sender<Option<String>>;

pub(super) fn spawn_server_event_listener_task(
    ui_bridge: UiBridge,
    base_url: String,
    server_password: Option<String>,
    session_filter: SessionFilter,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut session_filter_rx = session_filter.subscribe();
        let mut headers = HeaderMap::new();
        if let Some(password) = server_password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {password}")) {
                headers.insert(AUTHORIZATION, value);
            }
        }

        let client = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .default_headers(headers)
            .build()
        {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(%err, "failed to initialize server event stream client");
                return;
            }
        };

        let base_event_url = format!("{}/event", base_url.trim_end_matches('/'));
        let mut recovery_sync_pending = false;
        loop {
            let connected_filter = current_session_filter(&session_filter_rx);
            if connected_filter
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                recovery_sync_pending = false;
                if session_filter_rx.changed().await.is_err() {
                    break;
                }
                continue;
            }
            let event_url = build_event_url(&base_event_url, connected_filter.as_deref());
            let mut source = match EventSource::new(client.get(event_url.clone())) {
                Ok(source) => source,
                Err(err) => {
                    tracing::warn!(
                        %err,
                        url = %event_url,
                        "failed to initialize server event source"
                    );
                    emit_reconnecting_state(&ui_bridge, connected_filter.as_deref());
                    recovery_sync_pending = true;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    continue;
                }
            };

            let reconnect_due_to_filter_change = consume_server_event_stream(
                &mut source,
                &ui_bridge,
                &mut session_filter_rx,
                &connected_filter,
                &mut recovery_sync_pending,
            )
            .await;
            if reconnect_due_to_filter_change {
                recovery_sync_pending = false;
                continue;
            }

            if current_session_filter(&session_filter_rx) == connected_filter {
                emit_reconnecting_state(&ui_bridge, connected_filter.as_deref());
                recovery_sync_pending = true;
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    })
}

async fn consume_server_event_stream(
    source: &mut EventSource,
    ui_bridge: &UiBridge,
    session_filter_rx: &mut watch::Receiver<Option<String>>,
    connected_filter: &Option<String>,
    recovery_sync_pending: &mut bool,
) -> bool {
    while let Some(event) = source.next().await {
        match event {
            Ok(SseEvent::Open) => {
                tracing::debug!(filter = ?connected_filter, "server event stream connected");
                if *recovery_sync_pending {
                    emit_reconnected_sync(ui_bridge, connected_filter.as_deref());
                    *recovery_sync_pending = false;
                }
            }
            Ok(SseEvent::Message(message)) => {
                forward_server_event_payload(&message.data, ui_bridge);

                // Match the previous behavior: reconnect after a complete
                // event if the active session filter changed.
                let current = current_session_filter(session_filter_rx);
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
    // P2-1: TUI defaults to high-frequency tier.
    url.query_pairs_mut().append_pair("tier", "tui");
    url
}

fn current_session_filter(session_filter_rx: &watch::Receiver<Option<String>>) -> Option<String> {
    session_filter_rx.borrow().clone()
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

fn emit_reconnecting_state(ui_bridge: &UiBridge, session_id: Option<&str>) {
    let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
        return;
    };
    let _ = ui_bridge.emit(Event::Custom(Box::new(
        CustomEvent::SessionStatusReconnecting {
            session_id: session_id.to_string(),
        },
    )));
}

fn emit_reconnected_sync(ui_bridge: &UiBridge, session_id: Option<&str>) {
    let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
        return;
    };
    let _ = ui_bridge.emit(Event::Custom(Box::new(CustomEvent::SessionUpdated {
            session_id: session_id.to_string(),
            source: Some("stream.reconnected".to_string()),
        })));
}

fn parse_server_event_payload(payload: &str) -> Option<Event> {
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }

    if let Ok(frontend) = serde_json::from_str::<FrontendEvent>(payload) {
        return Some(Event::Custom(Box::new(CustomEvent::FrontendEvent(Box::new(
            frontend,
        )))));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{build_event_url, emit_reconnected_sync, forward_server_event};
    use crate::bridge::UiBridge;
    use crate::event::CustomEvent;
    use crate::Event;
    use agendao_server_core::frontend_events::FrontendEvent;
    use agendao_server_core::runtime_events::ToolCallPhase;

    #[test]
    fn build_event_url_appends_session_filter() {
        let url = build_event_url("http://localhost:3000/event", Some("session-1"));
        assert_eq!(
            url.as_str(),
            "http://localhost:3000/event?session=session-1&tier=tui"
        );
    }

    #[test]
    fn build_event_url_leaves_unfiltered_stream_plain() {
        let url = build_event_url("http://localhost:3000/event", None);
        assert_eq!(url.as_str(), "http://localhost:3000/event?tier=tui");
    }

    #[test]
    fn blank_session_filter_is_treated_as_unsubscribed() {
        let filter = Some("   ".to_string());
        let active = filter
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        assert!(active.is_none());
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
        let CustomEvent::FrontendEvent(frontend) = *custom
        else {
            panic!("expected output block event");
        };
        let FrontendEvent::OutputBlockAppended {
            session_id,
            id,
            block,
            ..
        } = *frontend
        else {
            panic!("expected output block frontend event");
        };

        assert_eq!(session_id, "session-1");
        assert_eq!(id.as_deref(), Some("message-1"));
        assert_eq!(block["kind"], "reasoning");
        assert_eq!(block["phase"], "delta");
        assert_eq!(block["text"], "thinking");
    }

    #[test]
    fn permission_requested_event_is_forwarded() {
        let event = forward_server_event(&[serde_json::json!({
            "type": "permission.upsert",
            "sessionID": "session-1",
            "permission": {
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
        let CustomEvent::FrontendEvent(frontend) = *custom
        else {
            panic!("expected permission frontend event");
        };
        let FrontendEvent::PermissionUpsert {
            session_id,
            permission,
        } = *frontend
        else {
            panic!("expected permission upsert frontend event");
        };

        assert_eq!(session_id, "session-1");
        assert_eq!(permission.id, "permission-1");
        assert_eq!(permission.tool, "bash");
    }

    #[test]
    fn legacy_control_input_transition_event_is_ignored() {
        let event = forward_server_event(&[serde_json::json!({
            "type": "control_input.transition",
            "sessionID": "session-1",
            "kind": "steering",
            "phase": "queued",
            "at": 123
        })
        .to_string()]);

        assert!(event.is_none());
    }

    #[test]
    fn compacting_session_status_event_is_ignored() {
        let event = forward_server_event(&[serde_json::json!({
            "type": "session.status",
            "sessionID": "session-1",
            "status": {
                "type": "compacting"
            }
        })
        .to_string()]);

        assert!(event.is_none());
    }

    #[test]
    fn waiting_on_user_session_status_event_is_ignored() {
        let event = forward_server_event(&[serde_json::json!({
            "type": "session.status",
            "sessionID": "session-1",
            "status": {
                "type": "waiting_on_user"
            }
        })
        .to_string()]);

        assert!(event.is_none());
    }

    #[test]
    fn tool_call_frontend_event_is_forwarded() {
        let event = forward_server_event(&[serde_json::to_string(&FrontendEvent::ToolCallUpsert {
            session_id: "session-1".to_string(),
            tool_call_id: "tool-1".to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Start,
        })
        .expect("serialize frontend event")])
        .expect("tool call frontend event");

        let Event::Custom(custom) = event else {
            panic!("expected custom event");
        };
        let CustomEvent::FrontendEvent(frontend) = *custom else {
            panic!("expected frontend event");
        };
        let FrontendEvent::ToolCallUpsert {
            session_id,
            tool_call_id,
            tool_name,
            phase,
        } = *frontend
        else {
            panic!("expected tool call frontend event");
        };

        assert_eq!(session_id, "session-1");
        assert_eq!(tool_call_id, "tool-1");
        assert_eq!(tool_name, "bash");
        assert_eq!(phase, ToolCallPhase::Start);
    }

    #[test]
    fn reconnected_sync_event_targets_current_session() {
        let ui_bridge = UiBridge::new();

        emit_reconnected_sync(&ui_bridge, Some("session-1"));

        let event = ui_bridge
            .drain(1)
            .into_iter()
            .next()
            .expect("reconnected sync event");
        let Event::Custom(custom) = event else {
            panic!("expected custom event");
        };
        let CustomEvent::SessionUpdated { session_id, source } = *custom
        else {
            panic!("expected session updated state change");
        };

        assert_eq!(session_id, "session-1");
        assert_eq!(source.as_deref(), Some("stream.reconnected"));
    }
}
