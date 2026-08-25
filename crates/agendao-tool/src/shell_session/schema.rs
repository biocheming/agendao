//! Wire schema, input validation, and metadata projection for shell sessions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::bash::command_family_scope_key;
use crate::{Metadata, PermissionRequest, ToolContext, ToolError};

use super::manager::ShellSessionView;

pub(super) const BUFFER_LIMIT: usize = 2 * 1024 * 1024;
pub(super) const DEFAULT_WAIT_MS: u64 = 250;
pub(super) const MAX_WAIT_MS: u64 = 5_000;
pub(super) const DESCRIPTION: &str = r#"Persistent interactive shell session.

Phase 1 operations:
- start: create a long-lived PTY-backed shell session
- write: send line-oriented input to the session
- read: read buffered output since a cursor
- status: inspect session state
- terminate: stop the session

This tool is the structured authority for interactive shell state.
It complements the one-shot `bash` tool rather than replacing it."#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ShellSessionOperation {
    Start,
    Write,
    Read,
    Status,
    Terminate,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShellSessionInput {
    pub(super) operation: ShellSessionOperation,
    #[serde(default)]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) command: Option<String>,
    #[serde(default)]
    pub(super) args: Vec<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) env: HashMap<String, String>,
    #[serde(default)]
    pub(super) input: Option<String>,
    #[serde(default)]
    pub(super) append_newline: bool,
    #[serde(default)]
    pub(super) cursor: Option<u64>,
    #[serde(default)]
    pub(super) wait_ms: Option<u64>,
    #[serde(default)]
    pub(super) cols: Option<u16>,
    #[serde(default)]
    pub(super) rows: Option<u16>,
    #[serde(default)]
    pub(super) description: Option<String>,
}

pub(super) fn shell_session_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "operation": { "type": "string", "enum": ["start", "write", "read", "status", "terminate"], "description": "Which shell session operation to execute" },
            "session_id": { "type": "string", "description": "Existing shell session id for write/read/status/terminate" },
            "command": { "type": "string", "description": "Program to start for the session. Defaults to the user's shell." },
            "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments passed to the session program during start" },
            "cwd": { "type": "string", "description": "Working directory for the shell session" },
            "env": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Extra environment variables for the shell session" },
            "input": { "type": "string", "description": "Line-oriented text to send to the shell session" },
            "append_newline": { "type": "boolean", "description": "Whether to append a trailing newline after `input`" },
            "cursor": { "type": "integer", "minimum": 0, "description": "Read buffered output starting from this byte cursor" },
            "wait_ms": { "type": "integer", "minimum": 0, "description": "How long read should wait for more output when already caught up" },
            "cols": { "type": "integer", "minimum": 20, "description": "Initial terminal width for start" },
            "rows": { "type": "integer", "minimum": 4, "description": "Initial terminal height for start" },
            "description": { "type": "string", "description": "Human-readable description for permission review on start/write" }
        },
        "required": ["operation"]
    })
}

pub(super) fn validate_input(input: &ShellSessionInput) -> Result<(), ToolError> {
    match input.operation {
        ShellSessionOperation::Start => {
            if input
                .command
                .as_deref()
                .is_some_and(|command| command.trim().is_empty())
            {
                return Err(ToolError::InvalidArguments(
                    "command cannot be empty".to_string(),
                ));
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
        ShellSessionOperation::Read
        | ShellSessionOperation::Status
        | ShellSessionOperation::Terminate => {
            required_session_id(input)?;
        }
    }
    Ok(())
}

pub(super) fn required_session_id(input: &ShellSessionInput) -> Result<String, ToolError> {
    input
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ToolError::InvalidArguments("session_id is required for this operation".to_string())
        })
}

pub(super) fn default_shell_command() -> String {
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

pub(super) fn default_shell_args(input: &ShellSessionInput) -> Vec<String> {
    if input.args.is_empty() {
        #[cfg(unix)]
        {
            return vec!["-i".to_string()];
        }
        #[cfg(windows)]
        {
            return Vec::new();
        }
    }
    input.args.clone()
}

pub(super) fn resolve_cwd(cwd: Option<&str>, ctx: &ToolContext) -> String {
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

pub(super) async fn authorize_cwd(cwd: &str, ctx: &ToolContext) -> Result<(), ToolError> {
    if !ctx.is_external_path(cwd) {
        return Ok(());
    }
    let parent = std::path::Path::new(cwd)
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string());
    ctx.ask_permission(
        PermissionRequest::new("external_directory")
            .with_pattern(format!("{parent}/*"))
            .with_scope_key(crate::external_fs_scope_key(&parent))
            .with_metadata("filepath", serde_json::json!(cwd))
            .with_metadata("parentDir", serde_json::json!(parent)),
    )
    .await
}

pub(super) fn format_command_line(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    }
}

pub(super) fn shell_metadata(operation: &str, session: &ShellSessionView) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert("operation".to_string(), serde_json::json!(operation));
    metadata.insert(
        "scope_key".to_string(),
        serde_json::json!(shell_session_scope(operation, Some(&session.command))),
    );
    metadata.insert(
        "session".to_string(),
        shell_metadata_value("session", session),
    );
    metadata
}

fn shell_session_scope(operation: &str, command: Option<&str>) -> String {
    command
        .and_then(command_family_scope_key)
        .unwrap_or_else(|| format!("shell_session:{operation}"))
}

fn shell_metadata_value<T: Serialize>(key: &str, value: &T) -> serde_json::Value {
    match serde_json::to_value(value) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(metadata_key = key, %error, "failed to serialize shell session metadata");
            serde_json::json!({ "serialization_error": error.to_string() })
        }
    }
}
