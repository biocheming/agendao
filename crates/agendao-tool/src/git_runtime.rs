use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::{ToolContext, ToolError};

pub(crate) const DEFAULT_GIT_TIMEOUT_SECS: u64 = 120;

pub(crate) fn ensure_git_available() -> Result<(), ToolError> {
    which::which("git")
        .map(|_| ())
        .map_err(|e| ToolError::ExecutionError(format!("git executable not found: {}", e)))
}

pub(crate) async fn run_git_command(
    args: &[String],
    cwd: Option<&Path>,
    ctx: &ToolContext,
    timeout_secs: u64,
) -> Result<String, ToolError> {
    let mut command = tokio::process::Command::new("git");
    command.args(args.iter().map(|s| s.as_str()));
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let output_future = command.output();
    let output = tokio::select! {
        result = output_future => result.map_err(|e| ToolError::ExecutionError(format!("Failed to run git {:?}: {}", args, e)))?,
        _ = ctx.abort.cancelled() => return Err(ToolError::Cancelled),
        _ = tokio::time::sleep(Duration::from_secs(timeout_secs)) => {
            return Err(ToolError::Timeout(format!("git {:?} timed out after {} seconds", args, timeout_secs)));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(ToolError::ExecutionError(format!(
            "git {:?} failed: {}",
            args, detail
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
