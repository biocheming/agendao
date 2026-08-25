//! Shared ownership of a sandbox handle.
//!
//! `SandboxExecutionHandle` takes `&mut self` for both `wait` and
//! `cancel`, so a session that needs a natural-exit observer *and* a
//! terminate path cannot simply park the handle behind a mutex: the
//! observer would hold the lock across `wait().await` for the child's
//! whole lifetime and starve `cancel()` forever — a deadlock. The driver
//! is the fix: exactly one task owns the handle and selects between the
//! child's exit and terminate commands, which serializes the two paths
//! on a single owner instead of a contended lock.

use tokio::sync::{mpsc, oneshot};

use crate::lifecycle::SandboxExecutionHandle;
use crate::violation::SandboxExecutionError;

/// What the child did when it ended. `AlreadyFinished` never reaches the
/// callback: it means someone else already booked this exit.
pub type ExitStatus = Result<crate::lifecycle::SandboxExit, SandboxExecutionError>;

enum Command {
    Terminate {
        ack: oneshot::Sender<Result<(), String>>,
    },
}

/// The mailbox to the one task that owns a sandbox handle. Clone freely;
/// every clone can request the cancellation ladder.
#[derive(Clone)]
pub struct SandboxHandleDriver {
    tx: mpsc::Sender<Command>,
}

impl SandboxHandleDriver {
    /// Spawn the single owning task for `handle`. `on_exit` runs exactly
    /// once, when the child either exits naturally or through the
    /// cancellation ladder. It runs on the driver's task, so async work
    /// must be spawned inside the callback.
    pub fn spawn(
        handle: SandboxExecutionHandle,
        on_exit: impl FnOnce(ExitStatus) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Command>(8);
        tokio::task::spawn(async move {
            let mut handle = handle;
            let mut mailbox = Some(rx);
            let status = loop {
                let next_command = async {
                    match mailbox.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    status = handle.wait() => break status,
                    command = next_command => {
                        match command {
                            Some(Command::Terminate { ack }) => {
                                let status = handle.cancel().await;
                                let reply = match &status {
                                    Ok(_)
                                    | Err(SandboxExecutionError::AlreadyFinished) => Ok(()),
                                    Err(err) => Err(err.to_string()),
                                };
                                let _ = ack.send(reply);
                                break status;
                            }
                            // Every mailbox clone is gone; keep observing
                            // the natural exit until the child ends.
                            None => mailbox = None,
                        }
                    }
                }
            };
            on_exit(status);
        });
        Self { tx }
    }

    /// Run the cancellation ladder (TERM → grace → KILL) and wait for it
    /// to finish. A driver that already exited means the session already
    /// ended — success, the terminate is idempotent.
    pub async fn terminate(&self) -> Result<(), String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(Command::Terminate { ack: ack_tx })
            .await
            .map_err(|_| "sandbox driver already exited".to_string())?;
        match ack_rx.await {
            Ok(result) => result,
            // The driver dropped the ack while booking the exit.
            Err(_) => Ok(()),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::backend::BackendRegistry;
    use crate::launcher::{PrepareOptions, SandboxLauncher};
    use crate::model::TrustClass;
    use crate::native::NativeBackend;
    use crate::policy::PolicyInputs;
    use crate::request::{ProfileKind, SandboxExecutionRequest, SpawnSpec};
    use std::sync::Arc;

    async fn native_sleep_handle(args: &[&str], grace_ms: u64) -> SandboxExecutionHandle {
        let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
        let launcher = SandboxLauncher::new(registry, Arc::new(crate::EventLog::default()));
        let root = std::env::temp_dir();
        let request = SandboxExecutionRequest::new(
            TrustClass::ModelReachable,
            ProfileKind::Native,
            SpawnSpec::new("sleep").with_args(args.iter().map(|a| a.to_string()).collect()),
            &root,
        );
        let prepared = launcher
            .prepare(
                request,
                &PolicyInputs::baseline(agendao_types::SessionPermissionMode::UnsandboxedYolo),
                &PrepareOptions {
                    term_grace: Some(std::time::Duration::from_millis(grace_ms)),
                    ..Default::default()
                },
            )
            .expect("native prepare");
        prepared.start().await.expect("native start")
    }

    #[tokio::test]
    async fn natural_exit_reaches_the_observer() {
        let handle = native_sleep_handle(&["0.1"], 500).await;
        let (seen_tx, seen_rx) = oneshot::channel();
        let driver = SandboxHandleDriver::spawn(handle, move |status| {
            let _ = seen_tx.send(status.map(|exit| exit.code));
        });
        assert_eq!(seen_rx.await.unwrap().unwrap(), Some(0));
        // The driver is gone after the exit; terminate reports it.
        assert!(driver.terminate().await.is_err());
    }

    #[tokio::test]
    async fn terminate_runs_the_ladder_before_natural_exit() {
        let handle = native_sleep_handle(&["30"], 300).await;
        let (seen_tx, seen_rx) = oneshot::channel();
        let driver = SandboxHandleDriver::spawn(handle, move |status| {
            let _ = seen_tx.send(status.map(|exit| exit.success));
        });
        driver.terminate().await.expect("ladder runs on demand");
        assert!(!seen_rx.await.unwrap().unwrap());
        assert!(driver.terminate().await.is_err());
    }
}
