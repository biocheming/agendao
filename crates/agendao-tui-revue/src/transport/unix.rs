//! Unix socket transport — local IPC via Unix domain socket.
//!
//! Old TUI: socket_event_subscriber() async loop.
//! New: same agendao_client::UnixSocketTransport, events → EventBus sender.

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use agendao_client::transport::UnixSocketTransport;
use agendao_server_core::frontend_events::FrontendEvent;

/// Spawn a background task that subscribes to Unix socket events.
/// Mirrors old TUI's socket_event_subscriber().
pub fn spawn_unix_event_source(
    tx: UnboundedSender<FrontendEvent>,
    socket_path: String,
    handle: &tokio::runtime::Handle,
    session_filter: watch::Receiver<Option<String>>,
) -> Option<JoinHandle<()>> {
    let jh = handle.spawn(async move {
        let transport = UnixSocketTransport::new(socket_path);
        let mut filter_rx = session_filter;
        let cancel = CancellationToken::new();
        loop {
            let Ok(mut json_rx) = transport.subscribe_events(None, Some("tui")).await else {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                continue;
            };
            loop {
                tokio::select! {
                    event = json_rx.recv() => {
                        match event {
                            Some(json) => {
                                if let Ok(fe) = serde_json::from_value::<FrontendEvent>(json) {
                                    let sid = event_session_id_short(&fe);
                                    if filter_rx.borrow().as_deref() == sid {
                                        if tx.send(fe).is_err() { return; }
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                    changed = filter_rx.changed() => {
                        if changed.is_err() { cancel.cancel(); return; }
                    }
                }
            }
        }
    });
    Some(jh)
}

fn event_session_id_short(event: &FrontendEvent) -> Option<&str> {
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
