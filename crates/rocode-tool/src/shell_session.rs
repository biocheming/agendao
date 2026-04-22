use async_trait::async_trait;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tokio::sync::{Notify, RwLock};

use crate::bash::authorize_bash_command;
use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};
use rocode_core::process_registry::{global_registry, ProcessKind};

const BUFFER_LIMIT: usize = 2 * 1024 * 1024;
const DEFAULT_WAIT_MS: u64 = 250;
const MAX_WAIT_MS: u64 = 5_000;

const DESCRIPTION: &str = r#"Persistent interactive shell session.

Phase 1 operations:
- start: create a long-lived PTY-backed shell session
- write: send line-oriented input to the session
- read: read buffered output since a cursor
- status: inspect session state
- terminate: stop the session

This tool is the structured authority for interactive shell state.
It complements the one-shot `bash` tool rather than replacing it."#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ShellSessionOperation {
    Start,
    Write,
    Read,
    Status,
    Terminate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ShellSessionState {
    Running,
    Exited,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShellSessionInput {
    operation: ShellSessionOperation,
    #[serde(default, alias = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default, alias = "appendNewline")]
    append_newline: bool,
    #[serde(default)]
    cursor: Option<u64>,
    #[serde(default, alias = "waitMs")]
    wait_ms: Option<u64>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ShellSessionView {
    id: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    pid: u32,
    created_at: i64,
    state: ShellSessionState,
    exit_code: Option<u32>,
    error: Option<String>,
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

struct ShellSessionRecord {
    id: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    pid: u32,
    created_at: i64,
    writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
    output_buffer: Arc<Mutex<Vec<u8>>>,
    cursor: Arc<Mutex<usize>>,
    notify: Arc<Notify>,
    lifecycle: Arc<RwLock<ShellLifecycle>>,
}

impl ShellSessionRecord {
    async fn view(&self) -> ShellSessionView {
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

    async fn is_running(&self) -> bool {
        self.lifecycle.read().await.state == ShellSessionState::Running
    }
}

struct ShellSpawn {
    child: Box<dyn Child + Send + Sync>,
    reader: Box<dyn std::io::Read + Send>,
    writer: Box<dyn std::io::Write + Send>,
    pid: u32,
    killer: Box<dyn ChildKiller + Send + Sync>,
    command: String,
    cwd: String,
}

struct ShellSessionManager {
    sessions: Arc<RwLock<HashMap<String, Arc<ShellSessionRecord>>>>,
}

impl ShellSessionManager {
    fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn start_session(
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
        let args = if input.command.is_some() {
            input.args.clone()
        } else {
            default_shell_args(input)
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
        let env = input.env.clone();
        let spawn =
            tokio::task::spawn_blocking(move || spawn_shell(command, args, cwd, env, cols, rows))
                .await
                .map_err(|e| {
                    ToolError::ExecutionError(format!("failed to join shell start task: {}", e))
                })??;

        let ShellSpawn {
            mut child,
            reader,
            writer,
            pid,
            killer,
            command,
            cwd,
        } = spawn;

        let killer = Arc::new(Mutex::new(killer));
        let session_args = if input.command.is_some() {
            input.args.clone()
        } else {
            default_shell_args(input)
        };
        let shutdown_killer = killer.clone();
        let process_guard = global_registry().register_with_shutdown(
            pid,
            format!("shell_session: {}", command),
            ProcessKind::Bash,
            Arc::new(move || {
                request_shutdown_kill(&shutdown_killer);
            }),
        );

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let cursor = Arc::new(Mutex::new(0usize));
        let notify = Arc::new(Notify::new());
        let lifecycle = Arc::new(RwLock::new(ShellLifecycle::default()));

        let record = Arc::new(ShellSessionRecord {
            id: id.clone(),
            command,
            args: session_args,
            cwd,
            pid,
            created_at: chrono::Utc::now().timestamp(),
            writer: Arc::new(Mutex::new(writer)),
            killer,
            output_buffer: output_buffer.clone(),
            cursor: cursor.clone(),
            notify: notify.clone(),
            lifecycle: lifecycle.clone(),
        });

        self.sessions.write().await.insert(id, record.clone());

        let read_notify = notify.clone();
        tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        read_notify.notify_waiters();
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
                                read_notify.notify_waiters();
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
                                read_notify.notify_waiters();
                                break;
                            }
                        };
                        output.extend_from_slice(chunk);
                        *total += n;
                        if output.len() > BUFFER_LIMIT {
                            let excess = output.len() - BUFFER_LIMIT;
                            output.drain(..excess);
                        }
                        read_notify.notify_waiters();
                    }
                    Err(_) => {
                        read_notify.notify_waiters();
                        break;
                    }
                }
            }
        });

        let wait_notify = notify.clone();
        tokio::task::spawn_blocking(move || {
            let lifecycle = lifecycle;
            let status = child.wait();
            {
                let mut guard = lifecycle.blocking_write();
                match status {
                    Ok(status) => {
                        guard.state = ShellSessionState::Exited;
                        guard.exit_code = Some(status.exit_code());
                    }
                    Err(err) => {
                        guard.state = ShellSessionState::Error;
                        guard.error = Some(err.to_string());
                    }
                }
            }
            drop(process_guard);
            wait_notify.notify_waiters();
        });

        Ok(record.view().await)
    }

    async fn get_session(&self, session_id: &str) -> Result<Arc<ShellSessionRecord>, ToolError> {
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

static SHELL_SESSION_MANAGER: OnceLock<ShellSessionManager> = OnceLock::new();

fn shell_session_manager() -> &'static ShellSessionManager {
    SHELL_SESSION_MANAGER.get_or_init(ShellSessionManager::new)
}

pub struct ShellSessionTool;

impl ShellSessionTool {
    pub fn new() -> Self {
        Self
    }

    async fn execute_impl(
        &self,
        input: ShellSessionInput,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        validate_input(&input)?;
        match input.operation {
            ShellSessionOperation::Start => self.start(input, ctx).await,
            ShellSessionOperation::Write => self.write(input, ctx).await,
            ShellSessionOperation::Read => self.read(input).await,
            ShellSessionOperation::Status => self.status(input).await,
            ShellSessionOperation::Terminate => self.terminate(input).await,
        }
    }

    async fn start(
        &self,
        input: ShellSessionInput,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let session = shell_session_manager().start_session(&input, &ctx).await?;
        let output = format!(
            "Started shell session {} in {} using `{}` (pid {}).",
            session.id, session.cwd, session.command, session.pid
        );
        Ok(ToolResult {
            title: "Shell Session Started".to_string(),
            output,
            metadata: shell_metadata("start", &session),
            truncated: false,
        })
    }

    async fn write(
        &self,
        input: ShellSessionInput,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = shell_session_manager().get_session(&session_id).await?;
        let description = input
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Send input to persistent shell session")
            .to_string();
        let mut data = input.input.unwrap_or_default();
        if input.append_newline && !data.ends_with('\n') {
            data.push('\n');
        }
        authorize_bash_command(&data, &description, &ctx).await?;
        let writer = session.writer.clone();
        let bytes = data.into_bytes();
        let byte_len = bytes.len();
        tokio::task::spawn_blocking(move || {
            let mut writer = lock_mutex(&writer, "shell session writer")?;
            writer.write_all(&bytes).map_err(|e| {
                ToolError::ExecutionError(format!("failed to write to shell session: {}", e))
            })?;
            writer.flush().map_err(|e| {
                ToolError::ExecutionError(format!("failed to flush shell session: {}", e))
            })?;
            Ok::<(), ToolError>(())
        })
        .await
        .map_err(|e| {
            ToolError::ExecutionError(format!("failed to join shell write task: {}", e))
        })??;
        let session_view = session.view().await;
        let mut metadata = shell_metadata("write", &session_view);
        metadata.insert("bytes".to_string(), serde_json::json!(byte_len));
        Ok(ToolResult {
            title: "Shell Session Write".to_string(),
            output: format!("Sent {} bytes to shell session {}.", byte_len, session_id),
            metadata,
            truncated: false,
        })
    }

    async fn read(&self, input: ShellSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = shell_session_manager().get_session(&session_id).await?;
        let requested_cursor = input.cursor.unwrap_or(0) as usize;
        let wait_ms = input.wait_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS);
        if session.is_running().await {
            let current_cursor = *lock_mutex(&session.cursor, "shell session cursor")?;
            if requested_cursor >= current_cursor && wait_ms > 0 {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(wait_ms),
                    session.notify.notified(),
                )
                .await;
            }
        }

        let (buffer_bytes, cursor, buffer_start) = {
            let output = lock_mutex(&session.output_buffer, "shell session output buffer")?;
            let cursor = *lock_mutex(&session.cursor, "shell session cursor")?;
            let buffer_start = cursor.saturating_sub(output.len());
            (output.clone(), cursor, buffer_start)
        };

        let start_cursor = requested_cursor.max(buffer_start);
        let offset = start_cursor.saturating_sub(buffer_start);
        let bytes = if offset < buffer_bytes.len() {
            buffer_bytes[offset..].to_vec()
        } else {
            Vec::new()
        };
        let output = String::from_utf8_lossy(&bytes).to_string();
        let session_view = session.view().await;
        let mut metadata = shell_metadata("read", &session_view);
        metadata.insert(
            "requestedCursor".to_string(),
            serde_json::json!(requested_cursor),
        );
        metadata.insert("bufferStart".to_string(), serde_json::json!(buffer_start));
        metadata.insert("startCursor".to_string(), serde_json::json!(start_cursor));
        metadata.insert("endCursor".to_string(), serde_json::json!(cursor));
        metadata.insert(
            "truncatedReplay".to_string(),
            serde_json::json!(requested_cursor < buffer_start),
        );
        Ok(ToolResult {
            title: "Shell Session Read".to_string(),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn status(&self, input: ShellSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = shell_session_manager().get_session(&session_id).await?;
        let session_view = session.view().await;
        let output = format!(
            "Shell session {} is {:?} in {} (pid {}).",
            session_view.id, session_view.state, session_view.cwd, session_view.pid
        );
        Ok(ToolResult {
            title: "Shell Session Status".to_string(),
            output,
            metadata: shell_metadata("status", &session_view),
            truncated: false,
        })
    }

    async fn terminate(&self, input: ShellSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = shell_session_manager().get_session(&session_id).await?;
        let killer = session.killer.clone();
        tokio::task::spawn_blocking(move || {
            let mut killer = lock_mutex(&killer, "shell session killer")?;
            killer.kill().map_err(|e| {
                ToolError::ExecutionError(format!("failed to terminate shell session: {}", e))
            })
        })
        .await
        .map_err(|e| {
            ToolError::ExecutionError(format!("failed to join shell terminate task: {}", e))
        })??;
        let session_view = session.view().await;
        Ok(ToolResult {
            title: "Shell Session Terminating".to_string(),
            output: format!("Termination requested for shell session {}.", session_id),
            metadata: shell_metadata("terminate", &session_view),
            truncated: false,
        })
    }
}

impl Default for ShellSessionTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShellSessionTool {
    fn id(&self) -> &str {
        "shell_session"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["start", "write", "read", "status", "terminate"],
                    "description": "Which shell session operation to execute"
                },
                "session_id": {
                    "type": "string",
                    "description": "Existing shell session id for write/read/status/terminate"
                },
                "command": {
                    "type": "string",
                    "description": "Program to start for the session. Defaults to the user's shell."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments passed to the session program during start"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the shell session"
                },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Extra environment variables for the shell session"
                },
                "input": {
                    "type": "string",
                    "description": "Line-oriented text to send to the shell session"
                },
                "append_newline": {
                    "type": "boolean",
                    "description": "Whether to append a trailing newline after `input`"
                },
                "cursor": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Read buffered output starting from this byte cursor"
                },
                "wait_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "How long read should wait for more output when already caught up"
                },
                "cols": {
                    "type": "integer",
                    "minimum": 20,
                    "description": "Initial terminal width for start"
                },
                "rows": {
                    "type": "integer",
                    "minimum": 4,
                    "description": "Initial terminal height for start"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description for permission review on start/write"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: ShellSessionInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        self.execute_impl(input, ctx).await
    }
}

fn default_shell_command() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "bash".to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("COMSPEC")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "cmd.exe".to_string())
    }
}

fn default_shell_args(input: &ShellSessionInput) -> Vec<String> {
    if !input.args.is_empty() {
        return input.args.clone();
    }
    #[cfg(unix)]
    {
        vec!["-i".to_string()]
    }
    #[cfg(windows)]
    {
        Vec::new()
    }
}

fn resolve_cwd(cwd: Option<&str>, ctx: &ToolContext) -> String {
    cwd.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let path = std::path::Path::new(value);
            if path.is_absolute() {
                path.to_string_lossy().to_string()
            } else {
                std::path::Path::new(&ctx.directory)
                    .join(path)
                    .to_string_lossy()
                    .to_string()
            }
        })
        .unwrap_or_else(|| ctx.directory.clone())
}

async fn authorize_cwd(cwd: &str, ctx: &ToolContext) -> Result<(), ToolError> {
    if !ctx.is_external_path(cwd) {
        return Ok(());
    }
    let parent = std::path::Path::new(cwd)
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string());
    ctx.ask_permission(
        PermissionRequest::new("external_directory")
            .with_pattern(format!("{}/*", parent))
            .with_metadata("filepath", serde_json::json!(cwd))
            .with_metadata("parentDir", serde_json::json!(parent)),
    )
    .await
}

fn format_command_line(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        return command.to_string();
    }
    format!("{} {}", command, args.join(" "))
}

fn required_session_id(input: &ShellSessionInput) -> Result<String, ToolError> {
    input
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .ok_or_else(|| {
            ToolError::InvalidArguments("session_id is required for this operation".to_string())
        })
}

fn validate_input(input: &ShellSessionInput) -> Result<(), ToolError> {
    match input.operation {
        ShellSessionOperation::Start => {
            if let Some(command) = input.command.as_deref() {
                if command.trim().is_empty() {
                    return Err(ToolError::InvalidArguments(
                        "command cannot be empty".to_string(),
                    ));
                }
            }
        }
        ShellSessionOperation::Write => {
            required_session_id(input)?;
            let payload = input.input.as_deref().unwrap_or_default();
            if payload.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "input is required for write".to_string(),
                ));
            }
            if payload
                .chars()
                .any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
            {
                return Err(ToolError::InvalidArguments(
                    "write only supports printable line-oriented shell input in Phase 1"
                        .to_string(),
                ));
            }
        }
        ShellSessionOperation::Read => {
            required_session_id(input)?;
        }
        ShellSessionOperation::Status | ShellSessionOperation::Terminate => {
            required_session_id(input)?;
        }
    }
    Ok(())
}

fn shell_metadata(operation: &str, session: &ShellSessionView) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert("operation".to_string(), serde_json::json!(operation));
    metadata.insert(
        "session".to_string(),
        shell_metadata_value("session", session),
    );
    metadata
}

fn lock_mutex<'a, T>(mutex: &'a Mutex<T>, resource: &str) -> Result<MutexGuard<'a, T>, ToolError> {
    mutex
        .lock()
        .map_err(|error| ToolError::ExecutionError(format!("{} poisoned: {}", resource, error)))
}

fn request_shutdown_kill(killer: &Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>) {
    match killer.lock() {
        Ok(mut guard) => {
            if let Err(error) = guard.kill() {
                tracing::warn!(%error, "failed to kill shell session during shutdown callback");
            }
        }
        Err(error) => {
            tracing::warn!(%error, "shell session killer poisoned during shutdown callback");
        }
    }
}

fn shell_metadata_value<T: Serialize>(key: &str, value: &T) -> serde_json::Value {
    match serde_json::to_value(value) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(metadata_key = key, %error, "failed to serialize shell session metadata");
            serde_json::json!({
                "serialization_error": error.to_string(),
            })
        }
    }
}

fn spawn_shell(
    command: String,
    args: Vec<String>,
    cwd: String,
    env: HashMap<String, String>,
    cols: u16,
    rows: u16,
) -> Result<ShellSpawn, ToolError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| ToolError::ExecutionError(format!("failed to create PTY: {}", e)))?;

    let mut builder = CommandBuilder::new(&command);
    for arg in &args {
        builder.arg(arg);
    }
    builder.cwd(&cwd);
    for (key, value) in &env {
        builder.env(key, value);
    }

    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|e| ToolError::ExecutionError(format!("failed to spawn shell session: {}", e)))?;
    let pid = child.process_id().ok_or_else(|| {
        ToolError::ExecutionError("shell session did not expose a process id".to_string())
    })?;
    let killer = child.clone_killer();
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| ToolError::ExecutionError(format!("failed to clone PTY reader: {}", e)))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| ToolError::ExecutionError(format!("failed to open PTY writer: {}", e)))?;

    Ok(ShellSpawn {
        child,
        reader,
        writer,
        pid,
        killer,
        command,
        cwd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::sync::Mutex as AsyncMutex;

    const SHELL_TEST_TIMEOUT: Duration = Duration::from_secs(10);

    fn should_skip_pty_test(err: &ToolError) -> bool {
        matches!(err, ToolError::ExecutionError(message) if message.contains("failed to create PTY") || message.contains("failed to openpty"))
    }

    async fn run_shell_test<F>(name: &str, future: F)
    where
        F: Future<Output = ()>,
    {
        match tokio::time::timeout(SHELL_TEST_TIMEOUT, future).await {
            Ok(()) => {}
            Err(_) => panic!(
                "shell session test `{}` exceeded {:?}; PTY tests must fail fast instead of hanging",
                name, SHELL_TEST_TIMEOUT
            ),
        }
    }

    fn test_exit_script(marker: &str) -> String {
        #[cfg(windows)]
        {
            format!("echo {marker}\r\nexit\r\n")
        }
        #[cfg(not(windows))]
        {
            format!("printf '{marker}\\n'\nexit\n")
        }
    }

    async fn wait_for_session_state(
        ctx: ToolContext,
        session_id: &serde_json::Value,
        expected: &str,
    ) -> ToolResult {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let status = ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "status",
                        "session_id": session_id
                    }),
                    ctx.clone(),
                )
                .await
                .expect("status should succeed");
            if status.metadata["session"]["state"] == serde_json::json!(expected) {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "shell session did not reach `{}` before timeout; last state was {}",
                expected,
                status.metadata["session"]["state"]
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[test]
    fn schema_exposes_shell_session_operations() {
        let schema = ShellSessionTool::new().parameters();
        let operations = schema["properties"]["operation"]["enum"]
            .as_array()
            .expect("operation enum");
        assert!(operations.iter().any(|value| value == "start"));
        assert!(operations.iter().any(|value| value == "write"));
        assert!(operations.iter().any(|value| value == "read"));
        assert!(operations.iter().any(|value| value == "status"));
        assert!(operations.iter().any(|value| value == "terminate"));
    }

    #[tokio::test]
    async fn shell_session_roundtrip_start_write_read_status() {
        run_shell_test("shell_session_roundtrip_start_write_read_status", async {
            let dir = tempdir().expect("tempdir");
            let permissions = Arc::new(AsyncMutex::new(Vec::<String>::new()));
            let permissions_clone = permissions.clone();
            let ctx = ToolContext::new(
                "session-1".into(),
                "message-1".into(),
                dir.path().to_string_lossy().to_string(),
            )
            .with_ask(move |req| {
                let permissions_clone = permissions_clone.clone();
                async move {
                    permissions_clone.lock().await.push(req.permission);
                    Ok(())
                }
            });

            let start = match ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "start",
                        "description": "Start shell for structured tool testing"
                    }),
                    ctx.clone(),
                )
                .await
            {
                Ok(result) => result,
                Err(err) if should_skip_pty_test(&err) => {
                    eprintln!(
                        "skipping PTY integration test in current environment: {}",
                        err
                    );
                    return;
                }
                Err(err) => panic!("shell session start should succeed: {}", err),
            };
            let session_id = start.metadata["session"]["id"]
                .as_str()
                .expect("session id")
                .to_string();

            let marker = "hello-shell";
            ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "write",
                        "session_id": session_id,
                        "input": test_exit_script(marker),
                        "description": "Emit a marker and exit"
                    }),
                    ctx.clone(),
                )
                .await
                .expect("write should succeed");

            let read = ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "read",
                        "session_id": start.metadata["session"]["id"],
                        "cursor": 0,
                        "wait_ms": 2_000
                    }),
                    ctx.clone(),
                )
                .await
                .expect("read should succeed");
            assert!(read.output.contains(marker), "output was: {}", read.output);

            let status =
                wait_for_session_state(ctx, &start.metadata["session"]["id"], "exited").await;
            assert_eq!(
                status.metadata["session"]["state"],
                serde_json::json!("exited")
            );

            let permissions = permissions.lock().await;
            assert!(
                permissions
                    .iter()
                    .filter(|item| item.as_str() == "bash")
                    .count()
                    >= 2
            );
        })
        .await;
    }

    #[tokio::test]
    async fn shell_session_terminate_stops_running_process() {
        run_shell_test("shell_session_terminate_stops_running_process", async {
            let dir = tempdir().expect("tempdir");
            let ctx = ToolContext::new(
                "session-2".into(),
                "message-2".into(),
                dir.path().to_string_lossy().to_string(),
            );
            let start = match ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "start"
                    }),
                    ctx.clone(),
                )
                .await
            {
                Ok(result) => result,
                Err(err) if should_skip_pty_test(&err) => {
                    eprintln!(
                        "skipping PTY integration test in current environment: {}",
                        err
                    );
                    return;
                }
                Err(err) => panic!("start should succeed: {}", err),
            };
            let session_id = start.metadata["session"]["id"]
                .as_str()
                .expect("session id")
                .to_string();

            ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "terminate",
                        "session_id": session_id
                    }),
                    ctx.clone(),
                )
                .await
                .expect("terminate should succeed");

            tokio::time::sleep(Duration::from_millis(250)).await;
            let status = ShellSessionTool::new()
                .execute(
                    serde_json::json!({
                        "operation": "status",
                        "session_id": start.metadata["session"]["id"]
                    }),
                    ctx,
                )
                .await
                .expect("status should succeed");
            assert_ne!(
                status.metadata["session"]["state"],
                serde_json::json!("running")
            );
        })
        .await;
    }
}
