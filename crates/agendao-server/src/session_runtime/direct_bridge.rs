//! Shared Direct-mode event bridge.
//!
//! In Direct (in-process) mode there is no HTTP server and no SSE transport.
//! Frontends subscribe to the canonical `FrontendEvent` bus directly.

use std::sync::Arc;

use agendao_server_core::frontend_events::FrontendEvent;
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

async fn direct_event_subscription_loop(
    state: &Arc<ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();

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
                    let _ = tx.send(frontend_event);
                }
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

#[cfg(test)]
mod tests {
    use agendao_server_core::frontend_events::FrontendEvent;
    use agendao_server_core::runtime_events::ToolCallPhase;

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
}
