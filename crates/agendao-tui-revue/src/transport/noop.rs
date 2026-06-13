//! No-op transport — used when local-server feature is disabled.

use tokio::sync::mpsc::UnboundedSender;
use agendao_server_core::frontend_events::FrontendEvent;
use tokio::task::JoinHandle;

/// Spawn a no-op event source that never produces events.
pub fn spawn_event_source(
    _tx: UnboundedSender<FrontendEvent>,
    _workspace_root: std::path::PathBuf,
    _handle: &tokio::runtime::Handle,
    _session_filter: tokio::sync::watch::Receiver<Option<String>>,
) -> Option<JoinHandle<()>> {
    tracing::info!("local-server feature disabled — no event transport");
    None
}
