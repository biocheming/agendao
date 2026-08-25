//! Execution lifecycle: wait, cancellation, timeout, exit mapping.
//!
//! The handle is the only way to observe or steer a running sandbox
//! execution (plan §5.4's tail):
//!
//! ```text
//! wait / stream output
//!   -> cancellation or timeout => terminate tree (TERM -> grace -> KILL)
//!   -> reap child
//!   -> emit SandboxExited
//! ```
//!
//! Every terminal path emits exactly one `SandboxExited` — the handle
//! sets an internal flag so double-wait is an error, not a duplicate
//! event. Events come from the plan's identity (id + fingerprint), which
//! the caller can never rewrite.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::{BackendChild, BackendViolationToken};
use crate::launcher::{SandboxEvent, SandboxEventSink};
use crate::plan::SandboxPlan;
use crate::violation::{SandboxExecutionError, SandboxViolation};

/// How the execution ended, for the `SandboxExited` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStatus {
    /// The child exited on its own; nothing to clean up.
    NaturalExit,
    /// Cancelled: exited within the TERM grace period.
    TerminatedByRequest,
    /// Cancelled: SIGKILL escalation after the grace period expired.
    KilledAfterGrace,
    /// Deadline exceeded, then cancelled via the same escalation ladder.
    TimedOut,
}

/// Exit information surfaced to tools and events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SandboxExit {
    pub success: bool,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub cleanup: CleanupStatus,
}

/// A started sandbox execution. Produced only by
/// `PreparedSandboxExecution::start`.
pub struct SandboxExecutionHandle {
    child: Box<dyn BackendChild>,
    backend: String,
    plan: Arc<SandboxPlan>,
    sink: Arc<dyn SandboxEventSink>,
    violation_token: BackendViolationToken,
    finished: bool,
}

impl std::fmt::Debug for SandboxExecutionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxExecutionHandle")
            .field("execution_id", &self.plan.execution_id)
            .field("fingerprint", &self.plan.fingerprint)
            .field("backend", &self.backend)
            .field("finished", &self.finished)
            .finish()
    }
}

impl SandboxExecutionHandle {
    pub(crate) fn new(
        child: Box<dyn BackendChild>,
        backend: String,
        plan: Arc<SandboxPlan>,
        sink: Arc<dyn SandboxEventSink>,
        violation_token: BackendViolationToken,
    ) -> Self {
        Self {
            child,
            backend,
            plan,
            sink,
            violation_token,
            finished: false,
        }
    }

    /// The immutable plan this execution runs under (identity +
    /// fingerprint for auditing).
    pub fn plan(&self) -> &SandboxPlan {
        &self.plan
    }

    pub fn execution_id(&self) -> &str {
        &self.plan.execution_id
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.pid()
    }

    /// Piped stream handles, present when the launch's `StdioPlan`
    /// asked for `Piped`. Each may be taken once; tools use these to
    /// stream output while the sandboxed child runs.
    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.take_stdin()
    }

    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.take_stdout()
    }

    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.take_stderr()
    }

    /// Wait for natural exit and emit `SandboxExited`.
    pub async fn wait(&mut self) -> Result<SandboxExit, SandboxExecutionError> {
        self.ensure_running()?;
        let exit = self.child.wait().await?;
        Ok(self.finish(exit, CleanupStatus::NaturalExit))
    }

    /// Cancel the execution: TERM to the process group, wait out the
    /// plan's grace period, escalate to KILL, reap, emit `SandboxExited`.
    pub async fn cancel(&mut self) -> Result<SandboxExit, SandboxExecutionError> {
        self.ensure_running()?;
        let exit = self.terminate(CleanupStatus::TerminatedByRequest).await?;
        Ok(exit)
    }

    /// Cancel because a caller-side deadline expired: the same
    /// TERM → grace → KILL ladder, but the exit is audited as `TimedOut`
    /// instead of user-requested — distinct termination semantics in
    /// the event contract even when the cleanup mechanics are shared.
    pub async fn cancel_timeout(&mut self) -> Result<SandboxExit, SandboxExecutionError> {
        self.ensure_running()?;
        let exit = self.terminate(CleanupStatus::TimedOut).await?;
        Ok(exit)
    }

    /// Wait with a deadline; on expiry run the full cancellation ladder.
    pub async fn wait_with_timeout(
        &mut self,
        limit: Duration,
    ) -> Result<SandboxExit, SandboxExecutionError> {
        self.ensure_running()?;
        let waited = tokio::time::timeout(limit, self.child.wait()).await;
        match waited {
            Ok(exit) => Ok(self.finish(exit?, CleanupStatus::NaturalExit)),
            Err(_elapsed) => {
                let exit = self.terminate(CleanupStatus::TimedOut).await?;
                Ok(exit)
            }
        }
    }

    /// TERM → grace → KILL → reap. Shared by cancel and timeout paths so
    /// no path leaks an unreaped child.
    async fn terminate(
        &mut self,
        timeout_status: CleanupStatus,
    ) -> Result<SandboxExit, SandboxExecutionError> {
        self.child.signal_term().await?;
        let grace = Duration::from_secs(self.plan.process.term_grace_secs);
        let within_grace = tokio::time::timeout(grace, self.child.wait()).await;
        match within_grace {
            Ok(exit) => Ok(self.finish(exit?, timeout_status)),
            Err(_grace_expired) => {
                self.child.signal_kill().await?;
                let exit = self.child.wait().await?;
                let cleanup = if timeout_status == CleanupStatus::TimedOut {
                    CleanupStatus::TimedOut
                } else {
                    CleanupStatus::KilledAfterGrace
                };
                Ok(self.finish(exit, cleanup))
            }
        }
    }

    fn ensure_running(&self) -> Result<(), SandboxExecutionError> {
        if self.finished {
            return Err(SandboxExecutionError::AlreadyFinished);
        }
        Ok(())
    }

    /// Emit the terminal event exactly once.
    fn finish(&mut self, exit: crate::backend::BackendExit, cleanup: CleanupStatus) -> SandboxExit {
        self.record_exit_violation(exit.signal, exit.code);
        self.record_verified_violation();
        self.finished = true;
        let status = SandboxExit {
            success: exit.success,
            code: exit.code,
            signal: exit.signal,
            cleanup,
        };
        self.sink.record(SandboxEvent::Exited {
            execution_id: self.plan.execution_id.clone(),
            session_origin: self.plan.session_origin.clone(),
            status,
            backend: self.backend.clone(),
        });
        status
    }

    /// A SIGSYS termination is kernel evidence that the contained process hit
    /// the seccomp boundary. It is intentionally best-effort: the signal
    /// proves a denied syscall, not its exact syscall or endpoint.
    fn record_exit_violation(&self, _signal: Option<i32>, _code: Option<i32>) {
        #[cfg(unix)]
        if (_signal == Some(libc::SIGSYS) || _code == Some(128 + libc::SIGSYS))
            && self.backend != "native"
        {
            self.sink.record(SandboxEvent::Violation {
                violation: SandboxViolation {
                    execution_id: self.plan.execution_id.clone(),
                    plan_fingerprint: self.plan.fingerprint.clone(),
                    session_origin: self.plan.session_origin.clone(),
                    kind: crate::violation::SandboxViolationKind::SyscallDenied,
                    path_or_endpoint: None,
                    attribution: crate::violation::Attribution::BestEffort,
                    backend: self.backend.clone(),
                },
            });
        }
    }

    /// A backend may report only opaque-token evidence. Reject stale or
    /// cross-execution evidence, and mint every observable identity field
    /// from this handle's immutable plan/backend rather than backend input.
    fn record_verified_violation(&mut self) {
        let Some(report) = self.child.take_violation_report() else {
            return;
        };
        if report.token() != &self.violation_token {
            tracing::warn!(
                execution_id = %self.plan.execution_id,
                backend = %self.backend,
                "rejected sandbox violation evidence with a mismatched execution token"
            );
            return;
        }
        self.sink.record(SandboxEvent::Violation {
            violation: SandboxViolation {
                execution_id: self.plan.execution_id.clone(),
                plan_fingerprint: self.plan.fingerprint.clone(),
                session_origin: self.plan.session_origin.clone(),
                kind: report.kind,
                path_or_endpoint: report.path_or_endpoint,
                attribution: report.attribution,
                backend: self.backend.clone(),
            },
        });
    }
}
