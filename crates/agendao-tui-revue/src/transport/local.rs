//! Local-direct transport — in-process event bus.
//!
//! Old TUI: tokio::spawn + watch::channel session filter + UiBridge.
//! New: handle.spawn + watch::channel session filter + EventBus sender.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use agendao_server_core::frontend_events::FrontendEvent;

/// Helper: extract session_id from any FrontendEvent variant.
fn event_session_id(event: &FrontendEvent) -> Option<&str> {
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

/// Spawn a background task that forwards local-direct events to `tx`.
///
/// Mirrors old TUI's spawn_tui_direct_event_bridge():
/// - Creates LocalServerState for the workspace
/// - Filters events by session_id via watch::channel
/// - Forwards matching events to tx
pub fn spawn_event_source(
    tx: UnboundedSender<FrontendEvent>,
    workspace_root: PathBuf,
    handle: &tokio::runtime::Handle,
    session_filter: watch::Receiver<Option<String>>,
) -> Option<JoinHandle<()>> {
    let jh = handle.spawn(async move {
        let state = match agendao_server_local::new_local_server_for_workspace(
            workspace_root.clone(),
        ).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(?workspace_root, %e, "failed to init local server");
                return;
            }
        };
        let cancel = CancellationToken::new();
        let mut rx = agendao_server_local::spawn_direct_event_bus(
            Arc::clone(&state), cancel.clone(),
        );
        let mut filter_rx = session_filter;
        loop {
            tokio::select! {
                event = rx.recv() => {
                    let Some(fe) = event else { break };
                    let Some(sid) = event_session_id(&fe) else { continue };
                    // Only forward if matches current session filter
                    if filter_rx.borrow().as_deref() == Some(sid) {
                        if tx.send(fe).is_err() { break; }
                    }
                }
                changed = filter_rx.changed() => {
                    if changed.is_err() { cancel.cancel(); break; }
                }
            }
        }
    });
    Some(jh)
}
