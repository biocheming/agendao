//! Model-reachable one-shot queries through the sandbox boundary.
//!
//! `run_contained_query` is the shared launch path for short-lived,
//! collect-the-whole-output subprocesses a tool needs (git log/diff,
//! external catalog tool entries): the same fail-loudly contract,
//! request shape, cancellation ladder, and stdout/stderr draining the
//! bash tool uses (sandbox plan §4.4 — the boundary is the only launch
//! path for model-reachable execution; there is no direct-spawn
//! fallback).
//!
//! Process-registry visibility is deliberately left to callers: this
//! helper runs seconds-scale queries whose observability value is the
//! tool result itself; long-lived interactive surfaces (bash, PTY)
//! register with the global registry themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::Instant;

use agendao_sandbox::{
    PrepareOptions, ProfileKind, SandboxExecutionRequest, SpawnSpec, StdioPlan, TrustClass,
};

use crate::{
    cleanup_execution, drain_piped_output, CleanupCause, ToolContext, ToolError,
    MAX_CAPTURED_OUTPUT_BYTES,
};

/// Run a git command through the sandbox boundary and return its stdout.
///
/// This is the shared git launch path for every tool crate (repo history,
/// github research), so the profile choice and output unwrapping live in
/// one place instead of being re-derived per consumer (治理 KPI: 语义重复
/// 点数 0).
///
/// Git is not purely read-only — `status`/`diff` refresh the index and
/// `clone`/`fetch` write the work tree — so the profile is `WorkspaceWrite`
/// by default (workspace writable, network denied). A host that has
/// authorized native execution for the session gets `Native` instead,
/// exactly like bash (sandbox plan §4.4 — the boundary stays the only
/// launch path, whether it resolves to contained or native).
pub async fn run_git_command(
    args: &[String],
    cwd: Option<&Path>,
    ctx: &ToolContext,
    timeout: Duration,
) -> Result<String, ToolError> {
    let label = format!("git {}", args.first().map(String::as_str).unwrap_or(""));
    let profile = if ctx.sandbox_native_allowed {
        ProfileKind::Native
    } else {
        ProfileKind::WorkspaceWrite
    };
    let output = run_contained_query(
        ContainedQuerySpec {
            program: "git".into(),
            args: args.to_vec(),
            cwd: cwd.map(Path::to_path_buf),
            env_overrides: Default::default(),
            label: label.clone(),
        },
        ctx,
        timeout,
        profile,
    )
    .await?;

    if output.truncated() {
        return Err(ToolError::ExecutionError(format!(
            "git {:?} produced more than {} bytes of combined output; refusing to use a partial result",
            args, MAX_CAPTURED_OUTPUT_BYTES
        )));
    }

    if !output.success {
        let stderr = output.stderr.trim().to_string();
        let stdout = output.stdout.trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(ToolError::ExecutionError(format!(
            "git {:?} failed: {}",
            args, detail
        )));
    }
    Ok(output.stdout)
}

/// What to run and how to name it in errors.
pub struct ContainedQuerySpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env_overrides: BTreeMap<String, String>,
    /// Human label for denial/timeout messages (`"git log"`, the
    /// catalog tool name, …).
    pub label: String,
}

/// The collected result of one query.
pub struct ContainedQueryOutput {
    pub success: bool,
    /// Exit code when observable (mirrors `BackendExit::code`).
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// At least one stream exceeded its retained output budget.  Callers
    /// that parse output as data must reject it rather than treating a prefix
    /// as a complete answer.
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl ContainedQueryOutput {
    pub fn truncated(&self) -> bool {
        self.stdout_truncated || self.stderr_truncated
    }
}

/// Run one model-reachable subprocess through the installed boundary
/// and collect its output. Fails loudly when no authority is
/// installed — the same contract as the bash tool.
pub async fn run_contained_query(
    spec: ContainedQuerySpec,
    ctx: &ToolContext,
    timeout: Duration,
    profile: ProfileKind,
) -> Result<ContainedQueryOutput, ToolError> {
    let boundary = ctx.sandbox_execution.clone().ok_or_else(|| {
        ToolError::ExecutionError(format!(
            "{} requires a sandbox execution authority; no SandboxExecutionBoundary is installed in this host",
            spec.label
        ))
    })?;

    let request = SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        profile,
        SpawnSpec {
            program: spec.program,
            args: spec.args,
            cwd: spec.cwd,
            env_overrides: spec.env_overrides,
        },
        &ctx.directory,
    )
    .with_session_origin(ctx.session_id.clone());
    let prepared = boundary
        .prepare(
            request,
            PrepareOptions {
                stdio: StdioPlan::piped_output(),
                term_grace: Some(Duration::from_millis(300)),
                ..Default::default()
            },
        )
        .await
        // Governance denials (policy/backend) are not process
        // failures; the label tells the model which query was refused.
        .map_err(|e| ToolError::ExecutionError(format!("sandbox denied {}: {}", spec.label, e)))?;
    let mut handle = prepared.start().await.map_err(|e| {
        ToolError::ExecutionError(format!("{} process spawn failed: {}", spec.label, e))
    })?;

    let stdout = handle
        .take_stdout()
        .ok_or_else(|| ToolError::ExecutionError("piped stdout missing".into()))?;
    let stderr = handle
        .take_stderr()
        .ok_or_else(|| ToolError::ExecutionError("piped stderr missing".into()))?;

    // One absolute deadline governs *both* pipe draining and reaping.  A
    // child can close both pipes and then sleep; supervising only the read
    // side would otherwise leave wait() unbounded.
    let deadline = Instant::now() + timeout;
    let collected = tokio::select! {
        _ = ctx.abort.cancelled() => {
            // The boundary's TERM → grace → KILL ladder replaces any
            // local process-tree cleanup.
            cleanup_execution(&mut handle, CleanupCause::Abort, &spec.label).await?;
            return Err(ToolError::Cancelled);
        }
        _ = tokio::time::sleep_until(deadline) => {
            cleanup_execution(&mut handle, CleanupCause::Deadline, &spec.label).await?;
            return Err(ToolError::Timeout(format!(
                "{} timed out after {:?}",
                spec.label, timeout
            )));
        }
        result = drain_piped_output(stdout, stderr) => result,
    };
    let collected = match collected {
        Ok(output) => output,
        Err(error) => {
            cleanup_execution(&mut handle, CleanupCause::Abort, &spec.label).await?;
            return Err(ToolError::ExecutionError(format!(
                "{} output drain failed: {}",
                spec.label, error
            )));
        }
    };

    let exit = tokio::select! {
        _ = ctx.abort.cancelled() => {
            cleanup_execution(&mut handle, CleanupCause::Abort, &spec.label).await?;
            return Err(ToolError::Cancelled);
        }
        _ = tokio::time::sleep_until(deadline) => {
            cleanup_execution(&mut handle, CleanupCause::Deadline, &spec.label).await?;
            return Err(ToolError::Timeout(format!(
                "{} timed out after {:?}", spec.label, timeout
            )));
        }
        result = handle.wait() => result,
    }
    .map_err(|e| ToolError::ExecutionError(format!("{} wait failed: {}", spec.label, e)))?;

    Ok(ContainedQueryOutput {
        success: exit.success,
        code: exit.code,
        stdout: String::from_utf8_lossy(&collected.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&collected.stderr).into_owned(),
        stdout_truncated: collected.stdout_truncated,
        stderr_truncated: collected.stderr_truncated,
    })
}
