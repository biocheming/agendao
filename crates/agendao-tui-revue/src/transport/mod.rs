//! 火 — Transport layer: protocol-agnostic event sources.
//!
//! Three modes (priority order):
//!   1. Unix socket: AGENDAO_UNIX_SOCKET env var
//!   2. HTTP SSE: AGENDAO_TUI_BASE_URL env var
//!   3. Local-direct (default): in-process event bus

#[cfg(feature = "local-server")]
pub mod local;
#[cfg(not(feature = "local-server"))]
pub mod noop;

pub mod unix;
pub mod http_sse;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use agendao_server_core::frontend_events::FrontendEvent;

/// Spawn event source using a pre-created LocalServerState (local-direct mode).
#[cfg(feature = "local-server")]
pub fn spawn_local_event_source(
    tx: UnboundedSender<FrontendEvent>,
    state: Arc<agendao_server_local::LocalServerState>,
    handle: &tokio::runtime::Handle,
    session_filter: watch::Receiver<Option<String>>,
) -> Option<JoinHandle<()>> {
    local::spawn_source_from_state(tx, state, handle, session_filter)
}

/// Spawn the appropriate event source based on config.
///
/// Priority: unix socket > http sse > local-direct > noop.
pub fn spawn_event_source(
    tx: UnboundedSender<FrontendEvent>,
    workspace_root: PathBuf,
    handle: &tokio::runtime::Handle,
    session_filter: watch::Receiver<Option<String>>,
    unix_socket: Option<String>,
    base_url: Option<String>,
) -> Option<JoinHandle<()>> {
    // 1. Unix socket
    if let Some(ref path) = unix_socket {
        return unix::spawn_unix_event_source(tx, path.clone(), handle, session_filter);
    }
    // 2. HTTP SSE
    if let Some(ref url) = base_url {
        #[cfg(feature = "local-server")]
        return http_sse::spawn_http_event_source(tx, url.clone(), None, handle, session_filter);
        #[cfg(not(feature = "local-server"))]
        return http_sse::spawn_http_event_source(tx, url.clone(), None, handle, session_filter);
    }
    // 3. Local-direct
    #[cfg(feature = "local-server")]
    {
        return local::spawn_event_source(tx, workspace_root, handle, session_filter);
    }
    #[cfg(not(feature = "local-server"))]
    {
        return noop::spawn_event_source(tx, workspace_root, handle, session_filter);
    }
}
