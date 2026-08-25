//! Persistent shell-session state, lifecycle, and PTY output management.

use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, RwLock};

use agendao_core::process_registry::{global_registry, ProcessKind};
use agendao_sandbox::SandboxHandleDriver;

use crate::bash::authorize_bash_command;
use crate::{ToolContext, ToolError};

use super::launcher::{request_shutdown_cancel, start_sandboxed_shell, ShellSpawn};
use super::schema::BUFFER_LIMIT;
use super::schema::{
    authorize_cwd, default_shell_args, default_shell_command, format_command_line, resolve_cwd,
    ShellSessionInput,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ShellSessionState {
    Running,
    Exited,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ShellSessionView {
    pub(super) id: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) cwd: String,
    pub(super) pid: u32,
    pub(super) created_at: i64,
    pub(super) state: ShellSessionState,
    pub(super) exit_code: Option<u32>,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone)]
struct ShellLifecycle {
    state: ShellSessionState,
    exit_code: Option<u32>,
    error: Option<String>,
}

impl Default for ShellLifecycle {
    fn default() -> Self {
        Self {
            state: ShellSessionState::Running,
            exit_code: None,
            error: None,
        }
    }
}

pub(super) struct ShellSessionRecord {
    id: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    pid: u32,
    created_at: i64,
    pub(super) writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    pub(super) control: Arc<SandboxHandleDriver>,
    pub(super) output_buffer: Arc<Mutex<Vec<u8>>>,
    pub(super) cursor: Arc<Mutex<usize>>,
    pub(super) notify: Arc<Notify>,
    lifecycle: Arc<RwLock<ShellLifecycle>>,
}

impl ShellSessionRecord {
    pub(super) async fn view(&self) -> ShellSessionView {
        let lifecycle = self.lifecycle.read().await;
        ShellSessionView {
            id: self.id.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            cwd: self.cwd.clone(),
            pid: self.pid,
            created_at: self.created_at,
            state: lifecycle.state.clone(),
            exit_code: lifecycle.exit_code,
            error: lifecycle.error.clone(),
        }
    }

    pub(super) async fn is_running(&self) -> bool {
        self.lifecycle.read().await.state == ShellSessionState::Running
    }
}

/// Book the terminal state exactly once. `AlreadyFinished` means another
/// writer already recorded the exit — there is nothing to add.
async fn book_session_exit(
    status: Result<agendao_sandbox::SandboxExit, agendao_sandbox::SandboxExecutionError>,
    lifecycle: &Arc<RwLock<ShellLifecycle>>,
    notify: &Arc<Notify>,
) {
    match status {
        Ok(exit) => {
            let mut guard = lifecycle.write().await;
            guard.state = ShellSessionState::Exited;
            guard.exit_code = exit.code.map(|code| code.unsigned_abs());
        }
        Err(agendao_sandbox::SandboxExecutionError::AlreadyFinished) => {}
        Err(err) => {
            let mut guard = lifecycle.write().await;
            guard.state = ShellSessionState::Error;
            guard.error = Some(err.to_string());
        }
    }
    notify.notify_waiters();
}

pub(super) struct ShellSessionManager {
    sessions: Arc<RwLock<HashMap<String, Arc<ShellSessionRecord>>>>,
}

impl ShellSessionManager {
    fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub(super) async fn start_session(
        &self,
        input: &ShellSessionInput,
        ctx: &ToolContext,
    ) -> Result<ShellSessionView, ToolError> {
        let id = format!("shell_{}", uuid::Uuid::new_v4().simple());
        let command = input
            .command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(default_shell_command);
        let args = if input.args.is_empty() {
            default_shell_args(input)
        } else {
            input.args.clone()
        };
        let cwd = resolve_cwd(input.cwd.as_deref(), ctx);
        let description = input
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Start persistent shell session")
            .to_string();

        authorize_cwd(&cwd, ctx).await?;
        authorize_bash_command(&format_command_line(&command, &args), &description, ctx).await?;

        let cols = input.cols.unwrap_or(80).max(20);
        let rows = input.rows.unwrap_or(24).max(4);
        let spawn = start_sandboxed_shell(&command, &args, &cwd, &input.env, cols, rows, ctx)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("failed to start sandboxed shell: {}", e))
            })?;

        let ShellSpawn {
            handle,
            reader,
            writer,
            pid,
            command,
            cwd,
        } = spawn;

        let session_args = if input.args.is_empty() {
            default_shell_args(input)
        } else {
            input.args.clone()
        };
        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let cursor = Arc::new(Mutex::new(0usize));
        let notify = Arc::new(Notify::new());
        let lifecycle = Arc::new(RwLock::new(ShellLifecycle::default()));

        // The registry guard and the driver reference each other (the
        // guard's shutdown hook terminates through the driver; the
        // driver's exit callback drops the guard), so the hook binds to
        // a late slot that is filled synchronously right after spawn —
        // before any shutdown could fire.
        let late_control = Arc::new(std::sync::OnceLock::new());
        let hook_control = late_control.clone();
        let process_guard = global_registry().register_with_shutdown(
            pid,
            format!("shell_session: {}", command),
            ProcessKind::Bash,
            Arc::new(move || {
                if let Some(control) = hook_control.get() {
                    request_shutdown_cancel(control);
                }
            }),
        );

        // The session driver (agendao-sandbox): sole owner of the handle,
        // selecting between the child's natural exit and terminate
        // requests — a lock held across `wait().await` would starve
        // `cancel()` forever. Booking happens in the callback, the single
        // place the terminal state is written.
        let exit_lifecycle = lifecycle.clone();
        let exit_notify = notify.clone();
        let control = Arc::new(SandboxHandleDriver::spawn(handle, move |status| {
            let lifecycle = exit_lifecycle.clone();
            let notify = exit_notify.clone();
            let guard = process_guard;
            tokio::spawn(async move {
                book_session_exit(status, &lifecycle, &notify).await;
                drop(guard);
            });
        }));
        let _ = late_control.set(control.clone());

        // A persistent PTY outlives its start request, but it must not outlive
        // the session execution that owns that request. Route prompt abort
        // through the same driver as explicit terminate and natural exit, so
        // all three paths produce one terminal lifecycle booking.
        let abort = ctx.abort.clone();
        let abort_control = control.clone();
        tokio::spawn(async move {
            abort.cancelled().await;
            if let Err(error) = abort_control.terminate().await {
                tracing::debug!(%error, "shell session already ended before abort cleanup");
            }
        });

        let record = Arc::new(ShellSessionRecord {
            id: id.clone(),
            command,
            args: session_args,
            cwd,
            pid,
            created_at: chrono::Utc::now().timestamp(),
            writer: Arc::new(Mutex::new(writer)),
            control: control.clone(),
            output_buffer: output_buffer.clone(),
            cursor: cursor.clone(),
            notify: notify.clone(),
            lifecycle: lifecycle.clone(),
        });

        self.sessions.write().await.insert(id, record.clone());

        spawn_output_reader(reader, output_buffer, cursor, notify);

        Ok(record.view().await)
    }

    pub(super) async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<ShellSessionRecord>, ToolError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| {
                ToolError::ExecutionError(format!("shell session `{}` was not found", session_id))
            })
    }
}

fn spawn_output_reader(
    reader: Box<dyn std::io::Read + Send>,
    output_buffer: Arc<Mutex<Vec<u8>>>,
    cursor: Arc<Mutex<usize>>,
    notify: Arc<Notify>,
) {
    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    notify.notify_waiters();
                    break;
                }
                Ok(n) => {
                    let chunk = &buf[..n];
                    let mut output = match output_buffer.lock() {
                        Ok(guard) => guard,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "shell session output buffer poisoned; stopping PTY read loop"
                            );
                            notify.notify_waiters();
                            break;
                        }
                    };
                    let mut total = match cursor.lock() {
                        Ok(guard) => guard,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "shell session cursor poisoned; stopping PTY read loop"
                            );
                            notify.notify_waiters();
                            break;
                        }
                    };
                    append_session_output(&mut output, chunk);
                    *total += n;
                    notify.notify_waiters();
                }
                Err(_) => {
                    notify.notify_waiters();
                    break;
                }
            }
        }
    });
}

/// Keep the replay buffer at a strict bound even when a single PTY read is
/// larger than the buffer. The monotonic cursor remains the absolute byte
/// position; callers can therefore detect a replay gap via `bufferStart`.
pub(super) fn append_session_output(output: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= BUFFER_LIMIT {
        output.clear();
        output.extend_from_slice(&chunk[chunk.len() - BUFFER_LIMIT..]);
        return;
    }
    let excess = output
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(BUFFER_LIMIT);
    if excess > 0 {
        output.drain(..excess);
    }
    output.extend_from_slice(chunk);
}

static SHELL_SESSION_MANAGER: OnceLock<ShellSessionManager> = OnceLock::new();

pub(super) fn shell_session_manager() -> &'static ShellSessionManager {
    SHELL_SESSION_MANAGER.get_or_init(ShellSessionManager::new)
}
