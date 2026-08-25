//! Bounded terminal cleanup for tool-owned sandbox executions.

use std::time::Duration;

use agendao_sandbox::SandboxExecutionHandle;

use crate::ToolError;

/// The normal cancellation ladder has its own short TERM grace.  This outer
/// bound prevents a faulty backend wait from turning an abort/deadline into an
/// indefinitely stuck tool request.
const CLEANUP_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupCause {
    Abort,
    Deadline,
}

/// Terminate, reap, and audit a child without allowing cleanup itself to hang
/// the caller forever. Deadline cleanup is deliberately distinct so its
/// terminal event remains `TimedOut`; user aborts remain request cancellation.
pub async fn cleanup_execution(
    handle: &mut SandboxExecutionHandle,
    cause: CleanupCause,
    label: &str,
) -> Result<(), ToolError> {
    let cancellation = async {
        match cause {
            CleanupCause::Abort => handle.cancel().await,
            CleanupCause::Deadline => handle.cancel_timeout().await,
        }
    };
    match tokio::time::timeout(CLEANUP_DEADLINE, cancellation).await {
        Ok(Ok(_)) | Ok(Err(agendao_sandbox::SandboxExecutionError::AlreadyFinished)) => Ok(()),
        Ok(Err(error)) => Err(ToolError::ExecutionError(format!(
            "{label} cleanup failed: {error}"
        ))),
        Err(_) => Err(ToolError::ExecutionError(format!(
            "{label} cleanup exceeded {:?}; execution may require host-side inspection",
            CLEANUP_DEADLINE
        ))),
    }
}
