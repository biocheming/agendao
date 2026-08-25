use std::collections::HashMap;
use std::sync::Arc;

#[cfg(unix)]
use std::collections::BTreeMap;

#[cfg(unix)]
use agendao_sandbox::{
    PrepareOptions, ProfileKind, PtyDimensions, SandboxExecutionRequest, SpawnSpec, TrustClass,
};
use agendao_sandbox::{SandboxExecutionHandle, SandboxHandleDriver};

use crate::{ToolContext, ToolError};

pub(super) struct ShellSpawn {
    pub(super) handle: SandboxExecutionHandle,
    pub(super) reader: Box<dyn std::io::Read + Send>,
    pub(super) writer: Box<dyn std::io::Write + Send>,
    pub(super) pid: u32,
    pub(super) command: String,
    pub(super) cwd: String,
}

/// Launch only through the host's sandbox execution authority.
pub(super) async fn start_sandboxed_shell(
    command: &str,
    args: &[String],
    cwd: &str,
    env: &HashMap<String, String>,
    cols: u16,
    rows: u16,
    ctx: &ToolContext,
) -> Result<ShellSpawn, ToolError> {
    #[cfg(not(unix))]
    {
        let _ = (command, args, cwd, env, cols, rows, ctx);
        return Err(ToolError::ExecutionError(
            "shell_session requires the sandbox pty backend, which is unix-only until Phase 7 \
             lands the Windows terminal backend"
                .into(),
        ));
    }
    #[cfg(unix)]
    {
        let boundary = ctx.sandbox_execution.clone().ok_or_else(|| {
            ToolError::ExecutionError(
                "shell_session tool requires a sandbox execution authority; \
                 no SandboxExecutionBoundary is installed in this host"
                    .into(),
            )
        })?;
        let spec = SpawnSpec {
            program: command.to_string(),
            args: args.to_vec(),
            cwd: Some(std::path::PathBuf::from(cwd)),
            env_overrides: env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        };
        let profile_kind = if ctx.sandbox_native_allowed {
            ProfileKind::Native
        } else {
            ProfileKind::InteractiveShell
        };
        let request = SandboxExecutionRequest::new(
            TrustClass::ModelReachable,
            profile_kind,
            spec,
            &ctx.directory,
        )
        .with_session_origin(ctx.session_id.clone());
        let prepared = boundary
            .prepare(request, PrepareOptions::default())
            .await
            .map_err(|error| {
                ToolError::ExecutionError(format!("sandbox denied the shell session: {error}"))
            })?;
        let (handle, master) = prepared
            .start_pty(PtyDimensions { rows, cols })
            .await
            .map_err(|error| {
                ToolError::ExecutionError(format!("sandbox shell launch failed: {error}"))
            })?;
        let pid = handle.pid().ok_or_else(|| {
            ToolError::ExecutionError("shell session did not expose a process id".to_string())
        })?;
        let reader = master.try_clone_reader().map_err(|error| {
            ToolError::ExecutionError(format!("failed to clone PTY reader: {error}"))
        })?;
        let writer = master.try_clone_writer().map_err(|error| {
            ToolError::ExecutionError(format!("failed to open PTY writer: {error}"))
        })?;
        Ok(ShellSpawn {
            handle,
            reader: Box::new(reader),
            writer: Box::new(writer),
            pid,
            command: command.to_string(),
            cwd: cwd.to_string(),
        })
    }
}

/// Registry shutdown uses the same cancellation ladder as explicit terminate.
pub(super) fn request_shutdown_cancel(control: &Arc<SandboxHandleDriver>) {
    let control = control.clone();
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move {
                if let Err(error) = control.terminate().await {
                    tracing::debug!(%error, "shell session already finished at shutdown");
                }
            });
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "no tokio runtime at shutdown; contained shell relies on die-with-parent"
            );
        }
    }
}
