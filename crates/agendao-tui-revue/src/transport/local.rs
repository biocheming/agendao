//! Local-direct transport — in-process event bus.
//!
//! Old TUI: tokio::spawn + watch::channel session filter + UiBridge.
//! New: handle.spawn + watch::channel session filter + EventBus sender.

use agendao_server_core::frontend_events::FrontendEvent;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
        | FrontendEvent::TodoReplaced { session_id, .. }
        | FrontendEvent::TaskLedgerReplaced { session_id, .. }
        | FrontendEvent::SessionError { session_id, .. }
        | FrontendEvent::OutputBlockAppended { session_id, .. } => Some(session_id.as_str()),
        FrontendEvent::ConfigUpdated => None,
    }
}

pub async fn new_local_server_for_workspace(
    workspace_root: PathBuf,
) -> anyhow::Result<Arc<agendao_server::ServerState>> {
    let state = Arc::new(
        agendao_server::ServerState::new_with_storage_for_url_in_workspace(
            "http://127.0.0.1:0".to_string(),
            workspace_root,
        )
        .await?,
    );
    state.ensure_frontend_projector();
    state.ensure_catalog_refresh_loop();
    Ok(state)
}

/// Spawn event source from a pre-created server state.
pub fn spawn_source_from_state(
    tx: UnboundedSender<FrontendEvent>,
    state: Arc<agendao_server::ServerState>,
    handle: &tokio::runtime::Handle,
    session_filter: watch::Receiver<Option<String>>,
) -> Option<JoinHandle<()>> {
    let jh = handle.spawn(async move {
        let cancel = CancellationToken::new();
        let mut rx =
            agendao_server::spawn_local_frontend_events(Arc::clone(&state), cancel.clone());
        let mut filter_rx = session_filter;
        loop {
            tokio::select! {
                event = rx.recv() => {
                    let Some(fe) = event else { break };
                    // 全局事件（config.updated，无 session id）跨会话放行；
                    // 会话事件按当前 filter 匹配。
                    let pass = match event_session_id(&fe) {
                        None => true,
                        Some(sid) => filter_rx.borrow().as_deref() == Some(sid),
                    };
                    if pass && tx.send(fe).is_err() { break; }
                }
                changed = filter_rx.changed() => {
                    if changed.is_err() { cancel.cancel(); break; }
                }
            }
        }
    });
    Some(jh)
}

/// Spawn a background task that forwards local-direct events to `tx`.
///
/// Connect the local TUI to the canonical frontend event receiver:
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
        let state = match new_local_server_for_workspace(workspace_root.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(?workspace_root, %e, "failed to init local server");
                return;
            }
        };
        let cancel = CancellationToken::new();
        let mut rx =
            agendao_server::spawn_local_frontend_events(Arc::clone(&state), cancel.clone());
        let mut filter_rx = session_filter;
        loop {
            tokio::select! {
                event = rx.recv() => {
                    let Some(fe) = event else { break };
                    // 全局事件（config.updated）跨会话放行；会话事件按 filter 匹配。
                    let pass = match event_session_id(&fe) {
                        None => true,
                        Some(sid) => filter_rx.borrow().as_deref() == Some(sid),
                    };
                    if pass && tx.send(fe).is_err() { break; }
                }
                changed = filter_rx.changed() => {
                    if changed.is_err() { cancel.cancel(); break; }
                }
            }
        }
    });
    Some(jh)
}
