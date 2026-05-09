//! CLI interactive permission approval UI.
//!
//! Displays permission requests from tool execution and lets the user
//! choose `Allow Once`, `Allow Turn`, `Allow Session`, or `Deny` via the
//! interactive selector.
//!
//! Turn/session grants remember the permission type + pattern so subsequent
//! identical requests are auto-approved within the same scope.

use crate::cli_select::{interactive_select, SelectOption, SelectResult};
use crate::cli_spinner::SpinnerGuard;
use crate::cli_style::CliStyle;
use rocode_permission::PermissionLifetime;
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Stores permission grants that were approved for a turn or session.
///
/// Key format: `"{permission}:{pattern}"` (e.g. `"bash:ls"`, `"edit:src/main.rs"`).
/// A wildcard key `"{permission}:*"` means the entire permission type was blanket-approved.
#[derive(Debug, Clone, Default)]
pub struct PermissionMemory {
    turn_granted: HashSet<String>,
    session_granted: HashSet<String>,
}

impl PermissionMemory {
    pub fn new() -> Self {
        Self::default()
    }

    fn keys_for(permission: &str, patterns: &[String]) -> Vec<String> {
        if patterns.is_empty() {
            vec![format!("{}:*", permission)]
        } else {
            patterns
                .iter()
                .map(|pattern| format!("{}:{}", permission, pattern))
                .collect()
        }
    }

    pub fn grant_turn(&mut self, permission: &str, patterns: &[String]) {
        for key in Self::keys_for(permission, patterns) {
            self.turn_granted.insert(key);
        }
    }

    pub fn grant_session(&mut self, permission: &str, patterns: &[String]) {
        for key in Self::keys_for(permission, patterns) {
            self.session_granted.insert(key);
        }
    }

    pub fn clear_turn(&mut self) {
        self.turn_granted.clear();
    }

    /// Check whether the permission request is already auto-approved.
    pub fn is_granted(&self, permission: &str, patterns: &[String]) -> bool {
        // Blanket wildcard grant
        if self.session_granted.contains(&format!("{}:*", permission))
            || self.turn_granted.contains(&format!("{}:*", permission))
        {
            return true;
        }
        // Check each pattern
        if patterns.is_empty() {
            return false;
        }
        patterns
            .iter()
            .all(|p| {
                let key = format!("{}:{}", permission, p);
                self.session_granted.contains(&key) || self.turn_granted.contains(&key)
            })
    }
}

/// The possible user decisions for a permission request.
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    Allow,
    AllowTurn,
    AllowSession,
    Deny,
}

/// Format a permission request into a human-readable summary block for the terminal.
fn format_permission_summary(
    permission: &str,
    patterns: &[String],
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    style: &CliStyle,
) -> String {
    let mut lines = Vec::new();

    // Permission type icon + label
    let (icon, label) = match permission {
        "bash" => ("⚡", "Execute Command"),
        "edit" => ("✏️ ", "Edit File"),
        "write" => ("📝", "Write File"),
        "read" => ("📖", "Read File"),
        "grep" => ("🔍", "Search Files"),
        "glob" => ("📂", "Find Files"),
        "list" => ("📂", "List Directory"),
        "external_directory" => ("⚠️ ", "Access External Directory"),
        "websearch" => ("🌐", "Web Search"),
        "network" => ("🌐", "Network Request"),
        "browser" => ("🌐", "Browser Session"),
        "context_docs" => ("📚", "Context Docs"),
        "media" | "media_inspect" => ("🖼️ ", "Media Inspect"),
        "task" | "task_flow" => ("📋", "Task Management"),
        _ => ("🔧", permission),
    };

    lines.push(format!(
        "  {} {} {}",
        icon,
        style.bold(label),
        style.dim(&format!("({})", permission))
    ));

    // Show patterns (file paths, commands, etc.)
    if !patterns.is_empty() {
        for pattern in patterns {
            lines.push(format!("    {} {}", style.dim("→"), pattern));
        }
    }

    // Show relevant metadata
    if let Some(command) = metadata.get("command").and_then(|v| v.as_str()) {
        let display = if command.len() > 120 {
            format!("{}…", &command[..117])
        } else {
            command.to_string()
        };
        lines.push(format!("    {} {}", style.dim("$"), display));
    }

    if let Some(filepath) = metadata.get("filepath").and_then(|v| v.as_str()) {
        if patterns.is_empty() || !patterns.iter().any(|p| p == filepath) {
            lines.push(format!("    {} {}", style.dim("file:"), filepath));
        }
    }

    if let Some(diff) = metadata.get("diff").and_then(|v| v.as_str()) {
        // Show first few lines of the diff
        let diff_lines: Vec<&str> = diff.lines().take(8).collect();
        if !diff_lines.is_empty() {
            lines.push(format!("    {}", style.dim("diff:")));
            for dline in &diff_lines {
                let colored = if dline.starts_with('+') {
                    style.bold_green(dline)
                } else if dline.starts_with('-') {
                    style.bold_red(dline)
                } else {
                    style.dim(dline)
                };
                lines.push(format!("    {}", colored));
            }
            let total_diff_lines = diff.lines().count();
            if total_diff_lines > 8 {
                lines.push(format!(
                    "    {}",
                    style.dim(&format!("... ({} more lines)", total_diff_lines - 8))
                ));
            }
        }
    }

    if let Some(query) = metadata.get("query").and_then(|v| v.as_str()) {
        lines.push(format!("    {} {}", style.dim("query:"), query));
    }

    lines.join("\n")
}

/// Present a permission approval prompt to the user.
///
/// Returns the user's decision.
pub fn prompt_permission(
    permission: &str,
    patterns: &[String],
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    lifetimes: &[PermissionLifetime],
    style: &CliStyle,
) -> io::Result<PermissionDecision> {
    let summary = format_permission_summary(permission, patterns, metadata, style);

    // Print the summary block to stderr
    let mut stderr = io::stderr();
    write!(stderr, "{}\n", summary)?;
    stderr.flush()?;

    let mut options = vec![SelectOption {
        label: "Allow Once".to_string(),
        description: Some("Allow this action once".to_string()),
    }];
    if lifetimes.contains(&PermissionLifetime::Turn) {
        options.push(SelectOption {
            label: "Allow Turn".to_string(),
            description: Some("Allow this type for the current turn".to_string()),
        });
    }
    if lifetimes.contains(&PermissionLifetime::Session) {
        options.push(SelectOption {
            label: "Allow Session".to_string(),
            description: Some("Allow this type for the rest of the session".to_string()),
        });
    }
    options.push(SelectOption {
        label: "Deny".to_string(),
        description: Some("Block this action".to_string()),
    });

    let result = interactive_select("Permission required", None, &options, style)?;

    match result {
        SelectResult::Selected(choices) => {
            let choice = choices.first().map(|s| s.as_str()).unwrap_or("Deny");
            match choice {
                "Allow Once" => Ok(PermissionDecision::Allow),
                "Allow Turn" => Ok(PermissionDecision::AllowTurn),
                "Allow Session" => Ok(PermissionDecision::AllowSession),
                _ => Ok(PermissionDecision::Deny),
            }
        }
        SelectResult::Other(_) => Ok(PermissionDecision::Deny),
        SelectResult::Cancelled => Ok(PermissionDecision::Deny),
    }
}

/// Build a CLI permission callback that can be passed to `AgentExecutor::with_ask_permission()`.
///
/// Returns a closure that:
/// - Checks the scoped `PermissionMemory` for prior grants
/// - If not already granted, pauses the spinner, prompts the user interactively, then resumes
/// - Records turn/session decisions in memory for future auto-approval
pub fn build_cli_permission_callback(
    spinner_guard: Arc<std::sync::Mutex<SpinnerGuard>>,
) -> impl Fn(
    rocode_tool::PermissionRequest,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), rocode_tool::ToolError>> + Send>,
> + Send
       + Sync
       + 'static {
    let memory = Arc::new(Mutex::new(PermissionMemory::new()));

    move |request: rocode_tool::PermissionRequest| {
        let memory = memory.clone();
        let spinner_guard = spinner_guard.clone();
        Box::pin(async move {
            // Check if already granted
            {
                let mem = memory.lock().await;
                if mem.is_granted(&request.permission, &request.patterns) {
                    return Ok(());
                }
            }

            // Pause spinner so it doesn't trample the permission prompt
            let guard = spinner_guard
                .lock()
                .map(|g| g.clone())
                .unwrap_or_else(|_| SpinnerGuard::noop());
            guard.pause();

            // Prompt user on a blocking task (crossterm raw mode needs real terminal)
            let permission = request.permission.clone();
            let patterns = request.patterns.clone();
            let metadata = request.metadata.clone();
            let lifetimes = if request.supported_lifetimes.is_empty() {
                vec![PermissionLifetime::Once, PermissionLifetime::Session]
            } else {
                request.supported_lifetimes.clone()
            };

            let decision = tokio::task::spawn_blocking(move || {
                let style = CliStyle::detect();
                prompt_permission(&permission, &patterns, &metadata, &lifetimes, &style)
            })
            .await
            .map_err(|e| {
                guard.resume();
                rocode_tool::ToolError::ExecutionError(format!("Permission prompt failed: {}", e))
            })?
            .map_err(|e| {
                guard.resume();
                rocode_tool::ToolError::ExecutionError(format!("Permission prompt IO error: {}", e))
            })?;

            guard.resume();

            match decision {
                PermissionDecision::Allow => Ok(()),
                PermissionDecision::AllowTurn => {
                    let mut mem = memory.lock().await;
                    mem.grant_turn(&request.permission, &request.patterns);
                    Ok(())
                }
                PermissionDecision::AllowSession => {
                    let mut mem = memory.lock().await;
                    mem.grant_session(&request.permission, &request.patterns);
                    Ok(())
                }
                PermissionDecision::Deny => Err(rocode_tool::ToolError::PermissionDenied(format!(
                    "User denied permission: {} [{}]",
                    request.permission,
                    request.patterns.join(", ")
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_memory_grant_and_check() {
        let mut mem = PermissionMemory::new();

        assert!(!mem.is_granted("bash", &["ls".to_string()]));

        mem.grant_session("bash", &["ls".to_string()]);
        assert!(mem.is_granted("bash", &["ls".to_string()]));
        assert!(!mem.is_granted("bash", &["rm -rf /".to_string()]));
    }

    #[test]
    fn permission_memory_wildcard_grant() {
        let mut mem = PermissionMemory::new();

        mem.grant_session("edit", &[]);
        assert!(mem.is_granted("edit", &["any-file.rs".to_string()]));
        assert!(mem.is_granted("edit", &["another.rs".to_string()]));
    }

    #[test]
    fn permission_memory_multiple_patterns() {
        let mut mem = PermissionMemory::new();

        let patterns = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
        mem.grant_session("edit", &patterns);

        assert!(mem.is_granted("edit", &["src/a.rs".to_string()]));
        assert!(mem.is_granted("edit", &["src/b.rs".to_string()]));
        assert!(mem.is_granted("edit", &patterns));
        assert!(!mem.is_granted("edit", &["src/c.rs".to_string()]));
    }

    #[test]
    fn permission_memory_empty_patterns_not_granted_without_wildcard() {
        let mem = PermissionMemory::new();
        assert!(!mem.is_granted("bash", &[]));
    }

    #[test]
    fn format_summary_bash_command() {
        let style = CliStyle::plain();
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("command".to_string(), serde_json::json!("cargo test --all"));

        let summary =
            format_permission_summary("bash", &["cargo test --all".to_string()], &metadata, &style);

        assert!(summary.contains("Execute Command"));
        assert!(summary.contains("cargo test --all"));
    }

    #[test]
    fn format_summary_edit_with_diff() {
        let style = CliStyle::plain();
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "diff".to_string(),
            serde_json::json!("-old line\n+new line"),
        );
        metadata.insert("filepath".to_string(), serde_json::json!("src/main.rs"));

        let summary =
            format_permission_summary("edit", &["src/main.rs".to_string()], &metadata, &style);

        assert!(summary.contains("Edit File"));
        assert!(summary.contains("-old line"));
        assert!(summary.contains("+new line"));
    }
}
