//! Tool operation dispatch over the session manager's single state authority.

use async_trait::async_trait;
use std::io::Write as _;
use std::sync::{Mutex, MutexGuard};

use crate::bash::authorize_bash_command;
use crate::{Tool, ToolContext, ToolError, ToolResult};

use super::manager::shell_session_manager;
use super::schema::{
    required_session_id, shell_metadata, shell_session_schema, validate_input, ShellSessionInput,
    ShellSessionOperation, DEFAULT_WAIT_MS, DESCRIPTION, MAX_WAIT_MS,
};

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
        Ok(ToolResult {
            title: "Shell Session Started".to_string(),
            output: format!(
                "Started shell session {} in {} using `{}` (pid {}).",
                session.id, session.cwd, session.command, session.pid
            ),
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
            .unwrap_or("Send input to persistent shell session");
        let mut data = input.input.unwrap_or_default();
        if input.append_newline && !data.ends_with('\n') {
            data.push('\n');
        }
        authorize_bash_command(&data, description, &ctx).await?;
        let bytes = data.into_bytes();
        let byte_len = bytes.len();
        let writer = session.writer.clone();
        tokio::task::spawn_blocking(move || {
            let mut writer = lock_mutex(&writer, "shell session writer")?;
            writer.write_all(&bytes).map_err(|error| {
                ToolError::ExecutionError(format!("failed to write to shell session: {error}"))
            })?;
            writer.flush().map_err(|error| {
                ToolError::ExecutionError(format!("failed to flush shell session: {error}"))
            })
        })
        .await
        .map_err(|error| {
            ToolError::ExecutionError(format!("failed to join shell write task: {error}"))
        })??;
        let session_view = session.view().await;
        let mut metadata = shell_metadata("write", &session_view);
        metadata.insert("bytes".to_string(), serde_json::json!(byte_len));
        Ok(ToolResult {
            title: "Shell Session Write".to_string(),
            output: format!("Sent {byte_len} bytes to shell session {session_id}."),
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
        let (buffer, cursor, buffer_start) = {
            let output = lock_mutex(&session.output_buffer, "shell session output buffer")?;
            let cursor = *lock_mutex(&session.cursor, "shell session cursor")?;
            let buffer_start = cursor.saturating_sub(output.len());
            (output.clone(), cursor, buffer_start)
        };
        let start_cursor = requested_cursor.max(buffer_start);
        let offset = start_cursor.saturating_sub(buffer_start);
        let output = String::from_utf8_lossy(buffer.get(offset..).unwrap_or_default()).to_string();
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
        Ok(ToolResult {
            title: "Shell Session Status".to_string(),
            output: format!(
                "Shell session {} is {:?} in {} (pid {}).",
                session_view.id, session_view.state, session_view.cwd, session_view.pid
            ),
            metadata: shell_metadata("status", &session_view),
            truncated: false,
        })
    }

    async fn terminate(&self, input: ShellSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = shell_session_manager().get_session(&session_id).await?;
        session.control.terminate().await.map_err(|error| {
            ToolError::ExecutionError(format!("failed to terminate shell session: {error}"))
        })?;
        let session_view = session.view().await;
        Ok(ToolResult {
            title: "Shell Session Terminating".to_string(),
            output: format!("Termination requested for shell session {session_id}."),
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
        shell_session_schema()
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input = serde_json::from_value(args)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        self.execute_impl(input, ctx).await
    }
}

fn lock_mutex<'a, T>(mutex: &'a Mutex<T>, resource: &str) -> Result<MutexGuard<'a, T>, ToolError> {
    mutex
        .lock()
        .map_err(|error| ToolError::ExecutionError(format!("{resource} poisoned: {error}")))
}
