//! Local-direct transport — in-process event bus.
//!
//! Uses `agendao_server_local::spawn_direct_event_bus()` to receive
//! `FrontendEvent` from the in-process orchestration core.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use agendao_server_core::frontend_events::FrontendEvent;

/// Spawn a background task that forwards local-direct events to `tx`.
///
/// Creates an in-process `LocalServerState` for the given workspace,
/// then spawns the direct event bus on the given runtime handle.
pub fn spawn_event_source(
    tx: UnboundedSender<FrontendEvent>,
    workspace_root: PathBuf,
    handle: &tokio::runtime::Handle,
) -> Option<JoinHandle<()>> {
    let handle = handle.spawn(async move {
        let state = match agendao_server_local::new_local_server_for_workspace(
            workspace_root.clone(),
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(?workspace_root, %e, "failed to init local server for event bus");
                return;
            }
        };

        let cancel = CancellationToken::new();
        let mut rx = agendao_server_local::spawn_direct_event_bus(
            Arc::clone(&state), cancel.clone(),
        );

        loop {
            match rx.recv().await {
                Some(event) => {
                    if tx.send(event).is_err() {
                        break; // receiver dropped
                    }
                }
                None => break,
            }
        }
    });

    Some(handle)
}
