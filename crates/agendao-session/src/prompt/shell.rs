use std::path::Path;

use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::Session;

use super::ModelRef;

/// Input for the `shell()` function.
#[derive(Debug, Clone)]
pub struct ShellInput {
    pub session_id: String,
    pub command_str: String,
    pub agent: Option<String>,
    pub model: Option<ModelRef>,
    pub abort: Option<CancellationToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellInvocation {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

pub(crate) fn resolve_shell_invocation(shell_env: Option<&str>, command: &str) -> ShellInvocation {
    let shell = shell_env
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("/bin/bash")
        .to_string();
    let shell_name = Path::new(&shell)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    let escaped = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_string());

    let args = match shell_name.as_str() {
        "nu" | "fish" => vec!["-c".to_string(), command.to_string()],
        "zsh" => vec![
            "-c".to_string(),
            "-l".to_string(),
            format!(
                "[[ -f ~/.zshenv ]] && source ~/.zshenv >/dev/null 2>&1 || true\n\
                 [[ -f \"${{ZDOTDIR:-$HOME}}/.zshrc\" ]] && source \"${{ZDOTDIR:-$HOME}}/.zshrc\" >/dev/null 2>&1 || true\n\
                 eval {}",
                escaped
            ),
        ],
        "bash" => vec![
            "-c".to_string(),
            "-l".to_string(),
            format!(
                "shopt -s expand_aliases\n\
                 [[ -f ~/.bashrc ]] && source ~/.bashrc >/dev/null 2>&1 || true\n\
                 eval {}",
                escaped
            ),
        ],
        "cmd" => vec!["/c".to_string(), command.to_string()],
        "powershell" | "pwsh" => vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ],
        _ => vec!["-c".to_string(), command.to_string()],
    };

    ShellInvocation {
        program: shell,
        args,
    }
}

/// Execute a shell command in the session context.
///
/// Creates a user message + assistant message with a tool call part recording
/// the shell execution and its output. The command is provided by the user
/// through the session UI and is intentionally executed as-is.
pub async fn shell_exec(input: &ShellInput, session: &mut Session) -> anyhow::Result<String> {
    // Create synthetic user message
    let _user_msg = session.add_user_message("The following tool was executed by the user");

    // Create assistant message with tool call
    let assistant_msg = session.add_assistant_message();
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    assistant_msg.add_tool_call(
        &call_id,
        "bash",
        serde_json::json!({ "command": input.command_str }),
    );
    let invocation =
        resolve_shell_invocation(std::env::var("SHELL").ok().as_deref(), &input.command_str);
    let abort = input.abort.clone().unwrap_or_default();

    let mut command = tokio::process::Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = command.spawn()?;
    let mut stdout = String::new();
    let mut stderr = String::new();

    let stdout_task = child.stdout.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Err(error) = pipe.read_to_end(&mut buf).await {
                tracing::debug!(
                    error = %error,
                    "Failed to read full stdout from shell child process"
                );
            }
            String::from_utf8_lossy(&buf).to_string()
        })
    });
    let stderr_task = child.stderr.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Err(error) = pipe.read_to_end(&mut buf).await {
                tracing::debug!(
                    error = %error,
                    "Failed to read full stderr from shell child process"
                );
            }
            String::from_utf8_lossy(&buf).to_string()
        })
    });

    let mut aborted = false;
    tokio::select! {
        _ = child.wait() => {}
        _ = abort.cancelled() => {
            aborted = true;
            if let Err(error) = child.kill().await {
                tracing::debug!(
                    error = %error,
                    "Failed to kill shell child process after cancellation"
                );
            }
            if let Err(error) = child.wait().await {
                tracing::debug!(
                    error = %error,
                    "Failed to wait for shell child process after cancellation"
                );
            }
        }
    }

    if let Some(task) = stdout_task {
        if let Ok(out) = task.await {
            stdout = out;
        }
    }
    if let Some(task) = stderr_task {
        if let Ok(out) = task.await {
            stderr = out;
        }
    }

    let mut result = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{}\n{}", stdout, stderr)
    };
    if aborted {
        result.push_str("\n\n<metadata>\nUser aborted the command\n</metadata>");
    }

    // Record the tool result
    assistant_msg.add_tool_result(&call_id, &result, aborted);

    Ok(result)
}

/// Input for the `command()` function.
#[derive(Debug, Clone)]
pub struct CommandInput {
    pub session_id: String,
    pub command: String,
    pub arguments: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub message_id: Option<String>,
    pub variant: Option<String>,
}

/// Resolve a command template with arguments.
///
/// Replaces `$1`, `$2`, etc. placeholders with positional arguments,
/// and `$ARGUMENTS` with the full argument string.
pub fn resolve_command_template(template: &str, arguments: &str) -> String {
    let args: Vec<&str> = arguments.split_whitespace().collect();

    // Find the highest placeholder index
    let mut max_index = 0u32;
    let placeholder_re = regex::Regex::new(r"\$(\d+)").unwrap();
    for cap in placeholder_re.captures_iter(template) {
        if let Ok(idx) = cap[1].parse::<u32>() {
            if idx > max_index {
                max_index = idx;
            }
        }
    }

    let has_arguments_placeholder = template.contains("$ARGUMENTS");

    // Replace $N placeholders
    let mut result = placeholder_re
        .replace_all(template, |caps: &regex::Captures| {
            let idx: usize = caps[1].parse().unwrap_or(0);
            if idx == 0 || idx > args.len() {
                return String::new();
            }
            let arg_idx = idx - 1;
            // Last placeholder swallows remaining args
            if idx as u32 == max_index {
                args[arg_idx..].join(" ")
            } else {
                args.get(arg_idx).unwrap_or(&"").to_string()
            }
        })
        .to_string();

    // Replace $ARGUMENTS
    result = result.replace("$ARGUMENTS", arguments);

    // If no placeholders and user provided arguments, append them
    if max_index == 0 && !has_arguments_placeholder && !arguments.trim().is_empty() {
        result = format!("{}\n\n{}", result, arguments);
    }

    result.trim().to_string()
}
