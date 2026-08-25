use async_trait::async_trait;
use std::collections::HashSet;
use tokio::time::{Duration, Instant};

use crate::{
    cleanup_execution, drain_piped_output, CleanupCause, Metadata, Tool, ToolContext, ToolError,
    ToolResult, MAX_CAPTURED_OUTPUT_BYTES,
};
use agendao_core::process_registry::{global_registry, ProcessKind};
use agendao_permission::{BashArity, PermissionMatcherKind};
use agendao_plugin::{HookContext, HookEvent};
use agendao_sandbox::{
    PrepareOptions, ProfileKind, SandboxExecutionRequest, SpawnSpec, StdioPlan, TrustClass,
};

const DEFAULT_TIMEOUT_MS: u64 = 2 * 60 * 1000;
/// Compatibility name for the user-visible bash output contract.  The bound
/// applies to stdout + stderr together; the shared collector also imposes the
/// same 50KiB ceiling on each stream before continuing to discard-drain it.
const MAX_OUTPUT_BYTES: usize = MAX_CAPTURED_OUTPUT_BYTES;
const TRUNCATION_NOTICE: &str =
    "\n\n(Output truncated at 51200 bytes; stdout and stderr continue draining)";

/// Explicit environment intent for this launch only: caller-supplied
/// `ctx.extra["env"]` plus plugin `shell.env` hook output. Host-environment
/// inheritance is *not* collected here — the sandbox authority owns that
/// (native profiles inherit the filtered host environment; contained
/// profiles clear and reinject core names), so there is exactly one
/// environment allowlist in the system instead of two drifting ones.
fn explicit_shell_env() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::new()
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
        let scope_key = parsed.command_prefix_scope_key();
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
            "If this work is long-running, split it into bounded commands and inspect each result before continuing.",
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
        "Execute a shell command in the specified working directory. Prefer structured tools such as read, glob, grep, edit, or write when they can complete the job more directly. Use bash for commands that genuinely require the shell, build tools, package managers, or external CLIs."
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

        let mut env_vars = explicit_shell_env();
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
        let env_hook_outputs = agendao_plugin::trigger_collect(hook_ctx).await;
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

        // The sandbox boundary is the only launch path for model-reachable
        // process execution. No installed authority means no process runs:
        // failing loudly here is the contract — never a direct spawn
        // fallback (sandbox plan §4.4).
        let boundary = ctx.sandbox_execution.clone().ok_or_else(|| {
            ToolError::ExecutionError(
                "bash tool requires a sandbox execution authority; \
                 no SandboxExecutionBoundary is installed in this host"
                    .into(),
            )
        })?;

        let spec = SpawnSpec {
            program: shell.to_string(),
            args: vec![flag.to_string(), command.clone()],
            cwd: Some(std::path::PathBuf::from(&workdir)),
            env_overrides: env_vars,
        };
        // The profile kind is the tool's declared ceiling: contained
        // workspace execution by default; the native kind only when the
        // host declared this session permits it. The authority still
        // verifies the request against policy either way.
        let profile_kind = if ctx.sandbox_native_allowed {
            ProfileKind::Native
        } else {
            ProfileKind::WorkspaceWrite
        };
        let request = SandboxExecutionRequest::new(
            TrustClass::ModelReachable,
            profile_kind,
            spec,
            &ctx.directory,
        )
        .with_session_origin(ctx.session_id.clone());
        let options = PrepareOptions {
            // Pipes for streaming output; io shaping is not policy, so it
            // lives here rather than in the plan fingerprint.
            stdio: StdioPlan::piped_output(),
            // A cancelled/timed-out command gets a short grace window
            // before KILL — the ladder lives in the boundary now.
            term_grace: Some(Duration::from_millis(300)),
            ..Default::default()
        };
        let prepared = boundary
            .prepare(request, options)
            .await
            // 治理拒绝与进程失败在模型可见错误中必须分开:prepare 阶段
            // 的失败全是"不允许跑"(策略/环境/后端),不是进程问题。
            .map_err(|e| ToolError::ExecutionError(format!("sandbox denied the command: {e}")))?;
        let mut handle = prepared
            .start()
            .await
            // start 阶段失败是"允许跑了但进程起不来"。
            .map_err(|e| ToolError::ExecutionError(format!("process spawn failed: {e}")))?;

        let child_pid = handle.pid();

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

        let stdout = handle
            .take_stdout()
            .expect("piped stdout was requested in the launch options");
        let stderr = handle
            .take_stderr()
            .expect("piped stderr was requested in the launch options");

        // The deadline covers both output drain and the final wait.  Closing
        // both pipes before sleeping must not bypass the command timeout.
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let captured = tokio::select! {
            _ = ctx.abort.cancelled() => {
                cleanup_execution(&mut handle, CleanupCause::Abort, "bash command").await?;
                return Err(ToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline) => {
                cleanup_execution(&mut handle, CleanupCause::Deadline, "bash command").await?;
                return Err(bash_timeout_error(&command, timeout_ms));
            }
            result = drain_piped_output(stdout, stderr) => result,
        };
        let captured = match captured {
            Ok(output) => output,
            Err(error) => {
                cleanup_execution(&mut handle, CleanupCause::Abort, "bash command").await?;
                return Err(ToolError::ExecutionError(format!(
                    "failed to drain bash command output: {error}"
                )));
            }
        };

        let exit = tokio::select! {
            _ = ctx.abort.cancelled() => {
                cleanup_execution(&mut handle, CleanupCause::Abort, "bash command").await?;
                return Err(ToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline) => {
                cleanup_execution(&mut handle, CleanupCause::Deadline, "bash command").await?;
                return Err(bash_timeout_error(&command, timeout_ms));
            }
            result = handle.wait() => result,
        }
        .map_err(|e| ToolError::ExecutionError(format!("Failed to wait for process: {}", e)))?;

        // Guard auto-unregisters from process registry when dropped (RAII).

        let exit_code = exit.code.unwrap_or(-1);

        let mut output = String::new();
        let mut truncated = captured.truncated();
        truncated |= append_lossy_output(&mut output, &captured.stdout);
        truncated |= append_lossy_output(&mut output, &captured.stderr);
        if !exit.success {
            truncated |= append_output_text(
                &mut output,
                &format!("\nCommand exited with code: {exit_code}"),
            );
        }
        if truncated {
            append_truncation_notice(&mut output);
        }

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("exit_code".into(), serde_json::json!(exit_code));
                m.insert("truncated".into(), serde_json::json!(truncated));
                m.insert(
                    "stdout_truncated".into(),
                    serde_json::json!(captured.stdout_truncated),
                );
                m.insert(
                    "stderr_truncated".into(),
                    serde_json::json!(captured.stderr_truncated),
                );
                m.insert(
                    "output_limit_bytes".into(),
                    serde_json::json!(MAX_OUTPUT_BYTES),
                );
                m
            },
            truncated,
        })
    }
}

/// Append lossy text without allowing invalid UTF-8 expansion to exceed the
/// public 50KiB result limit. Returns true when the text itself was clipped.
fn append_lossy_output(output: &mut String, bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    append_output_text(output, text.as_ref())
}

fn append_output_text(output: &mut String, text: &str) -> bool {
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(output.len());
    if text.len() <= remaining {
        output.push_str(text);
        return false;
    }
    let mut boundary = remaining;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.push_str(&text[..boundary]);
    true
}

fn append_truncation_notice(output: &mut String) {
    if output.len().saturating_add(TRUNCATION_NOTICE.len()) > MAX_OUTPUT_BYTES {
        let mut boundary = MAX_OUTPUT_BYTES.saturating_sub(TRUNCATION_NOTICE.len());
        while boundary > 0 && !output.is_char_boundary(boundary) {
            boundary -= 1;
        }
        output.truncate(boundary);
    }
    output.push_str(TRUNCATION_NOTICE);
}

/// Result of parsing a bash command with lightweight shell tokenization.
pub(crate) struct ParsedCommand {
    /// Full command text for each individual command (for permission patterns).
    patterns: HashSet<String>,
    /// BashArity-derived prefix patterns with wildcard (for "always allow").
    always: HashSet<String>,
    /// External directory paths found in path-manipulating commands.
    directories: Vec<String>,
}

#[cfg(feature = "terminal-tools")]
pub(crate) fn command_family_scope_key(command: &str) -> Option<String> {
    parse_bash_command(command).command_prefix_scope_key()
}

impl ParsedCommand {
    pub(crate) fn command_prefix_scope_key(&self) -> Option<String> {
        let mut prefixes = self
            .patterns
            .iter()
            .filter_map(|pattern| {
                let words = split_shell_words(pattern);
                let executable = words.first()?.trim();
                if executable.is_empty() {
                    None
                } else {
                    let executable = std::path::Path::new(executable)
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or(executable)
                        .to_ascii_lowercase();
                    let mut prefix = executable;
                    if let Some(argument) = words.get(1).filter(|value| !value.trim().is_empty()) {
                        prefix.push('/');
                        prefix.push_str(&argument.to_ascii_lowercase());
                    }
                    Some(prefix)
                }
            })
            .collect::<Vec<_>>();
        prefixes.sort();
        prefixes.dedup();

        if prefixes.is_empty() {
            None
        } else {
            Some(format!("cmd:{}", prefixes.join("+")))
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
    for segment in split_shell_segments(command) {
        let tokens = split_shell_words(segment);
        if tokens.is_empty() {
            continue;
        }

        if PATH_COMMANDS.contains(&tokens[0].as_str()) {
            for arg in &tokens[1..] {
                if arg.starts_with('-') || (tokens[0] == "chmod" && arg.starts_with('+')) {
                    continue;
                }
                let path = if std::path::Path::new(arg).is_absolute() {
                    arg.clone()
                } else if arg.starts_with('~') {
                    if let Ok(home) = std::env::var("HOME") {
                        arg.replacen('~', &home, 1)
                    } else {
                        arg.clone()
                    }
                } else {
                    arg.clone()
                };
                result.directories.push(path);
            }
        }

        if tokens[0] != "cd" {
            result.patterns.insert(segment.trim().to_string());
            let prefix = BashArity::prefix(&tokens);
            result.always.insert(format!("{} *", prefix.join(" ")));
        }
    }

    if result.patterns.is_empty() && !command.trim().is_empty() {
        let tokens = split_shell_words(command);
        if !tokens.is_empty() {
            result.patterns.insert(command.trim().to_string());
            let prefix = BashArity::prefix(&tokens);
            result.always.insert(format!("{} *", prefix.join(" ")));
        }
    }

    result
}

fn split_shell_segments(command: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let chars: Vec<(usize, char)> = command.char_indices().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let (idx, ch) = chars[i];
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ';' | '|' | '&' if !in_single && !in_double => {
                let is_double = matches!(ch, '|' | '&')
                    && chars.get(i + 1).is_some_and(|(_, next)| *next == ch);
                let end = idx;
                if start < end {
                    segments.push(command[start..end].trim());
                }
                start = if is_double {
                    chars
                        .get(i + 1)
                        .map(|(next_idx, next)| next_idx + next.len_utf8())
                        .unwrap_or(command.len())
                } else {
                    idx + ch.len_utf8()
                };
                if is_double {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    if start < command.len() {
        segments.push(command[start..].trim());
    }

    segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn split_shell_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_prefix_scope_key_uses_executable_and_subcommand() {
        let parsed = parse_bash_command("cargo test && git status");
        assert_eq!(
            parsed.command_prefix_scope_key().as_deref(),
            Some("cmd:cargo/test+git/status")
        );
    }

    #[test]
    fn command_prefix_scope_distinguishes_destructive_subcommands() {
        assert_ne!(
            parse_bash_command("cargo test --workspace").command_prefix_scope_key(),
            parse_bash_command("cargo clean").command_prefix_scope_key(),
        );
        assert_ne!(
            parse_bash_command("git status --short").command_prefix_scope_key(),
            parse_bash_command("git reset --hard").command_prefix_scope_key(),
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
    fn structured_tool_timeout_hint_bounds_long_running_builds() {
        let err = bash_timeout_error("cargo test", 5000);
        let message = err.to_string();
        assert!(message.contains("split it into bounded commands"));
    }
}
