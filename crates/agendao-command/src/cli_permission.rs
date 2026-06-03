//! CLI interactive permission approval UI.
//!
//! Displays permission requests from tool execution and lets the user
//! choose `Allow Once`, `Allow Turn`, `Allow Session`, or `Deny` via the
//! interactive selector.
//!
//! Turn/session grants remember the permission type + pattern so subsequent
//! identical requests are auto-approved within the same scope.

use crate::cli_select::{interactive_select_with_prelude, SelectOption, SelectResult};
use crate::cli_spinner::SpinnerGuard;
use crate::cli_style::CliStyle;
use agendao_permission::{PermissionClass, PermissionLifetime};
use std::collections::HashSet;
use std::io;
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
        patterns.iter().all(|p| {
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

fn default_lifetimes_for_class(
    permission_class: Option<PermissionClass>,
) -> Vec<PermissionLifetime> {
    match permission_class {
        Some(PermissionClass::InspectRead) => vec![PermissionLifetime::Once],
        Some(PermissionClass::WorkspaceWrite | PermissionClass::ExternalAccess) => vec![
            PermissionLifetime::Once,
            PermissionLifetime::Turn,
            PermissionLifetime::Session,
        ],
        Some(PermissionClass::DangerousExec) => vec![PermissionLifetime::Once],
        None => vec![PermissionLifetime::Once],
    }
}

fn lifetime_hint(scope: Option<&str>, lifetimes: &[PermissionLifetime]) -> Option<String> {
    if lifetimes.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    parts.push("once = this request".to_string());
    if lifetimes.contains(&PermissionLifetime::Turn) {
        parts.push(match scope {
            Some(scope) => format!("turn = current turn for {scope}"),
            None => "turn = current turn".to_string(),
        });
    }
    if lifetimes.contains(&PermissionLifetime::Session) {
        parts.push(match scope {
            Some(scope) => format!("session = this session for {scope}"),
            None => "session = this session".to_string(),
        });
    }

    Some(parts.join("  |  "))
}

fn display_scope<'a>(scope_key: Option<&'a str>, scope_label: Option<&'a str>) -> Option<&'a str> {
    scope_label.or(scope_key)
}

fn truncate_permission_preview(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    let mut chars = trimmed.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn push_permission_pattern_lines(
    lines: &mut Vec<String>,
    permission: &str,
    patterns: &[String],
    style: &CliStyle,
) {
    if patterns.is_empty() {
        return;
    }

    let preview_limit = if permission == "bash" { 1 } else { 2 };
    let preview_len = if permission == "bash" { 96 } else { 112 };
    let label = if permission == "bash" {
        "commands:"
    } else {
        "targets:"
    };
    lines.push(format!(
        "    {} {}",
        style.dim(label),
        if patterns.len() == 1 {
            "1 request".to_string()
        } else {
            format!("{} requests", patterns.len())
        }
    ));

    for pattern in patterns.iter().take(preview_limit) {
        lines.push(format!(
            "    {} {}",
            style.dim("→"),
            truncate_permission_preview(pattern, preview_len)
        ));
    }

    if patterns.len() > preview_limit {
        lines.push(format!(
            "    {} +{} more",
            style.dim("…"),
            patterns.len() - preview_limit
        ));
    }
}

/// Format a permission request into a human-readable summary block for the terminal.
fn format_permission_summary(
    permission: &str,
    permission_class: Option<&str>,
    scope_key: Option<&str>,
    scope_label: Option<&str>,
    matcher_label: Option<&str>,
    grant_target_summary: Option<&str>,
    patterns: &[String],
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    lifetimes: &[PermissionLifetime],
    risk_tags: &[String],
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

    if let Some(permission_class) = permission_class {
        lines.push(format!("    {} {}", style.dim("class:"), permission_class));
    }

    if let Some(scope) = display_scope(scope_key, scope_label) {
        lines.push(format!("    {} {}", style.dim("scope:"), scope));
    }
    if let Some(target) = grant_target_summary {
        lines.push(format!("    {} {}", style.dim("target:"), target));
    }
    if let Some(matcher) = matcher_label {
        lines.push(format!("    {} {}", style.dim("match:"), matcher));
    }
    let hint_scope = grant_target_summary.or_else(|| display_scope(scope_key, scope_label));
    if let Some(hint) = lifetime_hint(hint_scope, lifetimes) {
        lines.push(format!("    {} {}", style.dim("grant:"), hint));
    }
    if !risk_tags.is_empty() {
        lines.push(format!(
            "    {} {}",
            style.dim("risk:"),
            risk_tags.join(", ")
        ));
    }

    push_permission_pattern_lines(&mut lines, permission, patterns, style);

    // Show relevant metadata
    if let Some(command) = metadata.get("command").and_then(|v| v.as_str()) {
        let display = truncate_permission_preview(command, 108);
        lines.push(format!("    {} {}", style.dim("cmd:"), display));
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

fn permission_summary_prelude_lines(
    permission: &str,
    permission_class: Option<&str>,
    scope_key: Option<&str>,
    scope_label: Option<&str>,
    matcher_label: Option<&str>,
    grant_target_summary: Option<&str>,
    patterns: &[String],
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    lifetimes: &[PermissionLifetime],
    risk_tags: &[String],
) -> Vec<String> {
    let summary_style = CliStyle::plain();
    format_permission_summary(
        permission,
        permission_class,
        scope_key,
        scope_label,
        matcher_label,
        grant_target_summary,
        patterns,
        metadata,
        lifetimes,
        risk_tags,
        &summary_style,
    )
    .lines()
    .map(str::to_string)
    .collect()
}

fn permission_select_options(lifetimes: &[PermissionLifetime]) -> Vec<SelectOption> {
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
    options
}

/// Present a permission approval prompt to the user.
///
/// Returns the user's decision.
pub fn prompt_permission(
    permission: &str,
    permission_class: Option<&str>,
    scope_key: Option<&str>,
    scope_label: Option<&str>,
    matcher_label: Option<&str>,
    grant_target_summary: Option<&str>,
    patterns: &[String],
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    lifetimes: &[PermissionLifetime],
    risk_tags: &[String],
    style: &CliStyle,
) -> io::Result<PermissionDecision> {
    let summary_lines = permission_summary_prelude_lines(
        permission,
        permission_class,
        scope_key,
        scope_label,
        matcher_label,
        grant_target_summary,
        patterns,
        metadata,
        lifetimes,
        risk_tags,
    );
    let options = permission_select_options(lifetimes);

    let result = interactive_select_with_prelude(
        "Permission required",
        Some("Permission"),
        &summary_lines,
        &options,
        style,
    )?;

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
    agendao_tool::PermissionRequest,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), agendao_tool::ToolError>> + Send>,
> + Send
       + Sync
       + 'static {
    let memory = Arc::new(Mutex::new(PermissionMemory::new()));

    move |request: agendao_tool::PermissionRequest| {
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
                default_lifetimes_for_class(request.permission_class)
            } else {
                request.supported_lifetimes.clone()
            };

            let decision = tokio::task::spawn_blocking(move || {
                let style = CliStyle::detect();
                let permission_class = request.permission_class.map(|class| match class {
                    agendao_permission::PermissionClass::InspectRead => "Inspect read",
                    agendao_permission::PermissionClass::WorkspaceWrite => "Workspace write",
                    agendao_permission::PermissionClass::ExternalAccess => "External access",
                    agendao_permission::PermissionClass::DangerousExec => "Dangerous execution",
                });
                prompt_permission(
                    &permission,
                    permission_class,
                    request.scope_key.as_deref(),
                    None,
                    None,
                    None,
                    &patterns,
                    &metadata,
                    &lifetimes,
                    &request.risk_tags,
                    &style,
                )
            })
            .await
            .map_err(|e| {
                guard.resume();
                agendao_tool::ToolError::ExecutionError(format!("Permission prompt failed: {}", e))
            })?
            .map_err(|e| {
                guard.resume();
                agendao_tool::ToolError::ExecutionError(format!("Permission prompt IO error: {}", e))
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
                PermissionDecision::Deny => Err(agendao_tool::ToolError::PermissionDenied(format!(
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

        let summary = format_permission_summary(
            "bash",
            None,
            None,
            None,
            None,
            None,
            &["cargo test --all".to_string()],
            &metadata,
            &[PermissionLifetime::Once],
            &[],
            &style,
        );

        assert!(summary.contains("Execute Command"));
        assert!(summary.contains("cargo test --all"));
    }

    #[test]
    fn format_summary_bash_compresses_multiple_long_patterns() {
        let style = CliStyle::plain();
        let summary = format_permission_summary(
            "bash",
            Some("Dangerous execution"),
            None,
            None,
            None,
            None,
            &[
                "python3 /tmp/pubmed_xu.py".to_string(),
                "cat << 'PYEOF' > /tmp/pubmed_xu.py".to_string(),
            ],
            &std::collections::HashMap::new(),
            &[PermissionLifetime::Once],
            &["dangerous_exec".to_string()],
            &style,
        );

        assert!(summary.contains("commands: 2 requests"), "{summary}");
        assert!(summary.contains("python3 /tmp/pubmed_xu.py"), "{summary}");
        assert!(summary.contains("+1 more"), "{summary}");
        assert!(
            !summary.contains("cat << 'PYEOF' > /tmp/pubmed_xu.py"),
            "{summary}"
        );
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

        let summary = format_permission_summary(
            "edit",
            None,
            None,
            None,
            None,
            None,
            &["src/main.rs".to_string()],
            &metadata,
            &[PermissionLifetime::Once, PermissionLifetime::Session],
            &[],
            &style,
        );

        assert!(summary.contains("Edit File"));
        assert!(summary.contains("-old line"));
        assert!(summary.contains("+new line"));
    }

    #[test]
    fn permission_summary_prelude_lines_are_plain_text_for_selector_panel() {
        let lines = permission_summary_prelude_lines(
            "websearch",
            Some("External access"),
            Some("web"),
            Some("Web search"),
            None,
            None,
            &["example research query".to_string()],
            &std::collections::HashMap::new(),
            &[PermissionLifetime::Once, PermissionLifetime::Session],
            &[],
        );

        assert!(!lines.is_empty());
        assert!(lines.iter().all(|line| !line.contains('\u{1b}')));
        assert!(lines.join("\n").contains("Web Search"));
        assert!(lines.join("\n").contains("Web search"));
    }
}
