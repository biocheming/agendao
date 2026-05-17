use async_trait::async_trait;
use std::collections::HashSet;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration};

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};
use rocode_core::process_registry::{global_registry, ProcessKind};
use rocode_permission::{BashArity, PermissionMatcherKind};
use rocode_plugin::{HookContext, HookEvent};

const DEFAULT_TIMEOUT_MS: u64 = 2 * 60 * 1000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

#[cfg(unix)]
async fn kill_process_tree(pid: u32) {
    let pid_str = pid.to_string();
    send_pkill_signal("-TERM", &pid_str).await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    if child_processes_exist(&pid_str).await {
        send_pkill_signal("-KILL", &pid_str).await;
    }
}

#[cfg(unix)]
async fn send_pkill_signal(signal: &str, pid_str: &str) {
    match tokio::process::Command::new("pkill")
        .arg(signal)
        .arg("-P")
        .arg(pid_str)
        .status()
        .await
    {
        Ok(status) if status.success() || status.code() == Some(1) => {}
        Ok(status) => {
            tracing::warn!(
                signal,
                pid = pid_str,
                status = ?status.code(),
                "pkill exited unsuccessfully while terminating bash child processes"
            );
        }
        Err(error) => {
            tracing::warn!(
                signal,
                pid = pid_str,
                %error,
                "failed to invoke pkill while terminating bash child processes"
            );
        }
    }
}

#[cfg(unix)]
async fn child_processes_exist(pid_str: &str) -> bool {
    match tokio::process::Command::new("pgrep")
        .arg("-P")
        .arg(pid_str)
        .status()
        .await
    {
        Ok(status) => status.success(),
        Err(error) => {
            tracing::warn!(
                pid = pid_str,
                %error,
                "failed to inspect bash child processes before escalating to SIGKILL"
            );
            true
        }
    }
}

fn should_inherit_shell_env(key: &str) -> bool {
    const SAFE_EXACT_KEYS: &[&str] = &[
        "APPDATA",
        "COLORTERM",
        "COMSPEC",
        "DISPLAY",
        "HOME",
        "HOMEDRIVE",
        "HOMEPATH",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "LANG",
        "LOCALAPPDATA",
        "LOGNAME",
        "NO_PROXY",
        "OLDPWD",
        "OS",
        "PATH",
        "PATHEXT",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PWD",
        "SHELL",
        "SSH_AUTH_SOCK",
        "SSH_AGENT_PID",
        "SYSTEMROOT",
        "TEMP",
        "TERM",
        "TMP",
        "TMPDIR",
        "USER",
        "USERNAME",
        "USERPROFILE",
        "VISUAL",
        "WAYLAND_DISPLAY",
        "WINDIR",
        "XAUTHORITY",
    ];
    const SAFE_PREFIXES: &[&str] = &["LC_", "XDG_"];

    let upper = key.to_ascii_uppercase();
    SAFE_EXACT_KEYS.contains(&upper.as_str())
        || SAFE_PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
}

fn inherited_shell_env() -> std::collections::HashMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| should_inherit_shell_env(key))
        .collect()
}

pub struct BashTool;

impl BashTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) async fn authorize_bash_command(
    command: &str,
    description: &str,
    ctx: &ToolContext,
) -> Result<(), ToolError> {
    let parsed = parse_bash_command(command);

    for path in &parsed.directories {
        if ctx.is_external_path(path) {
            let parent = std::path::Path::new(path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());

            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(format!("{}/*", parent))
                    .with_metadata("filepath", serde_json::json!(path))
                    .with_metadata("parentDir", serde_json::json!(parent)),
            )
            .await?;
        }
    }

    if !parsed.patterns.is_empty() {
        let scope_key = parsed.command_family_scope_key();
        let patterns: Vec<String> = parsed.patterns.into_iter().collect();
        let always: Vec<String> = parsed.always.into_iter().collect();
        let mut req = crate::PermissionRequest::new("bash")
            .with_patterns(patterns.clone())
            .with_metadata("description", serde_json::json!(description))
            .with_metadata("command", serde_json::json!(command))
            .with_risk_tag("dangerous_exec");
        if let Some(scope_key) = scope_key {
            req = req
                .with_scope_key(scope_key.clone())
                .with_matcher(PermissionMatcherKind::StructuredFamily, scope_key)
                .with_supported_lifetimes(crate::structured_dangerous_exec_lifetimes());
        } else {
            req = req.with_matcher(PermissionMatcherKind::ExactInput, command.to_string());
        }
        for a in always {
            req = req.with_always(a);
        }
        ctx.ask_permission(req).await?;
    }

    Ok(())
}

fn structured_tool_timeout_hint(command: &str) -> Option<&'static str> {
    let first = command
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches(|c| c == '"' || c == '\''))
        .unwrap_or_default()
        .to_ascii_lowercase();
    match first.as_str() {
        "cat" | "head" | "tail" | "sed" | "awk" | "less" | "more" => {
            Some("If you were only inspecting a file, prefer `read` instead of `bash`.")
        }
        "grep" | "rg" | "ag" => {
            Some("If you were only searching text, prefer `grep` instead of `bash`.")
        }
        "find" | "fd" | "tree" | "ls" => {
            Some("If you were only discovering files, prefer `glob` or `ls` instead of `bash`.")
        }
        "python" | "python3" | "node" | "npm" | "pnpm" | "yarn" | "cargo" | "make" => Some(
            "If this work is long-running or multi-step, prefer delegating it with `task_flow` instead of holding the main turn in `bash`.",
        ),
        _ => None,
    }
}

fn bash_timeout_error(command: &str, timeout_ms: u64) -> ToolError {
    let mut message = format!("Command timed out after {}ms", timeout_ms);
    if let Some(hint) = structured_tool_timeout_hint(command) {
        message.push_str(". ");
        message.push_str(hint);
    }
    ToolError::Timeout(message)
}

#[async_trait]
impl Tool for BashTool {
    fn id(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the specified working directory. Prefer structured tools such as read, glob, grep, edit, write, or task_flow when they can complete the job more directly. Use bash as a last resort for commands that genuinely require the shell, build tools, package managers, or external CLIs."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds"
                },
                "workdir": {
                    "type": "string",
                    "description": "The working directory to run the command in"
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does"
                }
            },
            "required": ["command", "description"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let command: String = args["command"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("command is required".into()))?
            .to_string();

        let timeout_ms: u64 = args["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT_MS);

        let workdir: String = args["workdir"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| ctx.directory.clone());

        let description: String = args["description"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("description is required".into()))?
            .to_string();

        let title = description.clone();

        let mut env_vars = inherited_shell_env();
        if let Some(extra_env) = ctx.extra.get("env") {
            if let Some(env_obj) = extra_env.as_object() {
                for (key, value) in env_obj {
                    if let Some(val_str) = value.as_str() {
                        env_vars.insert(key.clone(), val_str.to_string());
                    }
                }
            }
        }

        // Plugin hook: shell.env — let plugins inject environment variables
        let mut hook_ctx = HookContext::new(HookEvent::ShellEnv)
            .with_session(&ctx.session_id)
            .with_data("cwd", serde_json::json!(&workdir));
        if let Some(call_id) = &ctx.call_id {
            hook_ctx = hook_ctx.with_data("call_id", serde_json::json!(call_id));
        }
        let env_hook_outputs = rocode_plugin::trigger_collect(hook_ctx).await;
        for output in env_hook_outputs {
            let Some(payload) = output.payload.as_ref() else {
                continue;
            };
            let Some(object) = payload
                .get("output")
                .and_then(|value| value.as_object())
                .or_else(|| payload.as_object())
            else {
                continue;
            };
            let Some(env) = object.get("env").and_then(|value| value.as_object()) else {
                continue;
            };
            for (key, value) in env {
                if let Some(value_str) = value.as_str() {
                    env_vars.insert(key.clone(), value_str.to_string());
                }
            }
        }

        authorize_bash_command(&command, &description, &ctx).await?;

        let shell = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "bash"
        };
        let flag = if cfg!(target_os = "windows") {
            "/C"
        } else {
            "-c"
        };

        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg(flag).arg(&command);
        cmd.current_dir(&workdir);
        for (key, value) in &env_vars {
            cmd.env(key, value);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionError(format!("Failed to spawn process: {}", e)))?;

        let child_pid = child.id();

        // Register in global process registry
        let _process_guard = if let Some(pid) = child_pid {
            let label = command
                .split_whitespace()
                .next()
                .unwrap_or("bash")
                .to_string();
            Some(global_registry().register(pid, format!("bash: {}", label), ProcessKind::Bash))
        } else {
            None
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut output = String::new();
        let mut truncated = false;

        let abort_token = ctx.abort.clone();

        let result = timeout(Duration::from_millis(timeout_ms), async {
            loop {
                tokio::select! {
                    _ = abort_token.cancelled() => {
                        #[cfg(unix)]
                        {
                            if let Some(pid) = child_pid {
                                kill_process_tree(pid).await;
                            }
                        }
                        if let Err(error) = child.kill().await {
                            tracing::debug!(
                                error = %error,
                                "Failed to kill bash child process after cancellation"
                            );
                        }
                        return Err(ToolError::Cancelled);
                    }
                    line = stdout_reader.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if output.len() + l.len() + 1 > MAX_OUTPUT_BYTES {
                                    truncated = true;
                                } else {
                                    output.push_str(&l);
                                    output.push('\n');
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                output.push_str(&format!("Error reading stdout: {}\n", e));
                            }
                        }
                    }
                    line = stderr_reader.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if output.len() + l.len() + 1 > MAX_OUTPUT_BYTES {
                                    truncated = true;
                                } else {
                                    output.push_str(&l);
                                    output.push('\n');
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                output.push_str(&format!("Error reading stderr: {}\n", e));
                            }
                        }
                    }
                }
            }
            Ok::<_, ToolError>(())
        })
        .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                #[cfg(unix)]
                {
                    if let Some(pid) = child_pid {
                        kill_process_tree(pid).await;
                    }
                }
                if let Err(error) = child.kill().await {
                    tracing::debug!(
                        error = %error,
                        "Failed to kill bash child process after timeout"
                    );
                }
                return Err(bash_timeout_error(&command, timeout_ms));
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to wait for process: {}", e)))?;

        // Guard auto-unregisters from process registry when dropped (RAII).

        let exit_code = status.code().unwrap_or(-1);

        if !status.success() {
            output.push_str(&format!("\nCommand exited with code: {}", exit_code));
        }

        if truncated {
            output.push_str(&format!(
                "\n\n(Output truncated at {} bytes)",
                MAX_OUTPUT_BYTES
            ));
        }

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("exit_code".into(), serde_json::json!(exit_code));
                m.insert("truncated".into(), serde_json::json!(truncated));
                m
            },
            truncated,
        })
    }
}

// ---------------------------------------------------------------------------
// Tree-sitter based bash command parsing
// ---------------------------------------------------------------------------

/// Result of parsing a bash command with tree-sitter.
pub(crate) struct ParsedCommand {
    /// Full command text for each individual command (for permission patterns).
    patterns: HashSet<String>,
    /// BashArity-derived prefix patterns with wildcard (for "always allow").
    always: HashSet<String>,
    /// External directory paths found in path-manipulating commands.
    directories: Vec<String>,
}

pub(crate) fn command_family_scope_key(command: &str) -> Option<String> {
    parse_bash_command(command).command_family_scope_key()
}

impl ParsedCommand {
    pub(crate) fn command_family_scope_key(&self) -> Option<String> {
        let mut family = self
            .patterns
            .iter()
            .filter_map(|pattern| {
                let head = pattern.split_whitespace().next()?.trim();
                if head.is_empty() {
                    None
                } else {
                    Some(head.to_ascii_lowercase())
                }
            })
            .collect::<Vec<_>>();
        family.sort();
        family.dedup();

        if family.is_empty() {
            None
        } else {
            Some(format!("cmd:{}", family.join("+")))
        }
    }
}

const PATH_COMMANDS: &[&str] = &[
    "cd", "rm", "cp", "mv", "mkdir", "touch", "chmod", "chown", "cat",
];

pub(crate) fn parse_bash_command(command: &str) -> ParsedCommand {
    let mut result = ParsedCommand {
        patterns: HashSet::new(),
        always: HashSet::new(),
        directories: Vec::new(),
    };

    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_bash::LANGUAGE;
    if parser.set_language(&language.into()).is_err() {
        // Fallback: treat entire command as a single pattern
        let tokens: Vec<String> = command.split_whitespace().map(String::from).collect();
        result.patterns.insert(command.to_string());
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
        return result;
    }

    let Some(tree) = parser.parse(command, None) else {
        let tokens: Vec<String> = command.split_whitespace().map(String::from).collect();
        result.patterns.insert(command.to_string());
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
        return result;
    };

    let root = tree.root_node();
    collect_commands(root, command.as_bytes(), &mut result);

    // If tree-sitter found no commands (e.g. variable assignment only), use full command
    if result.patterns.is_empty() && !command.trim().is_empty() {
        let tokens: Vec<String> = command.split_whitespace().map(String::from).collect();
        result.patterns.insert(command.to_string());
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
    }

    result
}

fn collect_commands(node: tree_sitter::Node, source: &[u8], result: &mut ParsedCommand) {
    if node.kind() == "command" {
        process_command_node(node, source, result);
        return;
    }

    // Recurse into children to find command nodes inside pipelines, lists, etc.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_commands(child, source, result);
    }
}

fn process_command_node(node: tree_sitter::Node, source: &[u8], result: &mut ParsedCommand) {
    // Get full command text, including redirects if parent is redirected_statement
    let command_text = if node.parent().map(|p| p.kind()) == Some("redirected_statement") {
        node.parent()
            .unwrap()
            .utf8_text(source)
            .unwrap_or_default()
            .to_string()
    } else {
        node.utf8_text(source).unwrap_or_default().to_string()
    };

    // Extract tokens: command_name + word/string/raw_string/concatenation children
    let mut tokens: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "command_name" | "word" | "string" | "raw_string" | "concatenation" => {
                let text = child.utf8_text(source).unwrap_or_default().to_string();
                tokens.push(text);
            }
            _ => {}
        }
    }

    if tokens.is_empty() {
        return;
    }

    // Check for path-manipulating commands and extract external paths
    if PATH_COMMANDS.contains(&tokens[0].as_str()) {
        for arg in &tokens[1..] {
            if arg.starts_with('-') || (tokens[0] == "chmod" && arg.starts_with('+')) {
                continue;
            }
            // Resolve path
            let path = if std::path::Path::new(arg).is_absolute() {
                arg.clone()
            } else if arg.starts_with('~') {
                if let Ok(home) = std::env::var("HOME") {
                    arg.replacen('~', &home, 1)
                } else {
                    arg.clone()
                }
            } else {
                // Relative path — can't resolve without cwd context here,
                // but the caller checks is_external_path which handles this
                arg.clone()
            };
            result.directories.push(path);
        }
    }

    // Skip "cd" from patterns (covered by directory check above)
    if tokens[0] != "cd" {
        result.patterns.insert(command_text);
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_family_scope_key_uses_command_heads() {
        let parsed = parse_bash_command("cargo test && git status");
        assert_eq!(
            parsed.command_family_scope_key().as_deref(),
            Some("cmd:cargo+git")
        );
    }

    #[test]
    fn structured_tool_timeout_hint_prefers_read_for_file_inspection() {
        let err = bash_timeout_error("cat src/lib.rs", 5000);
        let message = err.to_string();
        assert!(message.contains("Command timed out after 5000ms"));
        assert!(message.contains("prefer `read`"));
    }

    #[test]
    fn structured_tool_timeout_hint_prefers_task_flow_for_long_running_builds() {
        let err = bash_timeout_error("cargo test", 5000);
        let message = err.to_string();
        assert!(message.contains("prefer delegating it with `task_flow`"));
    }
}
