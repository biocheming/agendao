use std::path::Path;
use std::time::Duration;

use agendao_tool_core::run_git_command as run_git_contained;

use crate::{ToolContext, ToolError};

pub(crate) const DEFAULT_GIT_TIMEOUT_SECS: u64 = 120;

pub(crate) fn ensure_git_available() -> Result<(), ToolError> {
    which::which("git")
        .map(|_| ())
        .map_err(|e| ToolError::ExecutionError(format!("git executable not found: {}", e)))
}

/// Run one git command through the sandbox boundary. Delegates the
/// profile choice, request shape, and output unwrapping to the single
/// shared implementation in `agendao-tool-core` (semantic duplication 0).
pub(crate) async fn run_git_command(
    args: &[String],
    cwd: Option<&Path>,
    ctx: &ToolContext,
    timeout_secs: u64,
) -> Result<String, ToolError> {
    run_git_contained(args, cwd, ctx, Duration::from_secs(timeout_secs)).await
}
