use chrono::Local;

const PROMPT_ROCODE_HEADER: &str = include_str!("prompt_templates/rocode_header.txt");
const PROMPT_COMPATIBILITY_OVERLAY: &str =
    include_str!("prompt_templates/compatibility_overlay.txt");
const MAX_MCP_RESOURCE_CHARS: usize = 12_000;

pub struct SystemPrompt;

impl SystemPrompt {
    /// Returns the ROCode product header (used as base instructions).
    pub fn instructions() -> &'static str {
        PROMPT_ROCODE_HEADER.trim()
    }

    /// Wrap arbitrary text in a `<system-reminder>` block so it is treated as
    /// injected runtime context, not user-authored content.
    pub fn system_reminder(content: &str) -> String {
        format!("<system-reminder>\n{}\n</system-reminder>", content.trim())
    }

    /// Build a system-reminder block for MCP resource text content.
    pub fn mcp_resource_reminder(filename: &str, uri: &str, content: &str) -> String {
        let (content, truncated) = trim_for_prompt(content, MAX_MCP_RESOURCE_CHARS);
        let truncation_hint = if truncated {
            "\n\n[Content truncated for prompt safety.]"
        } else {
            ""
        };
        let body = format!(
            "MCP resource context from {} ({}):\n{}{}",
            filename, uri, content, truncation_hint
        );
        Self::system_reminder(&body)
    }

    /// Build the composed system prompt for the target model family.
    ///
    /// ROCode now uses a two-layer prompt surface:
    ///   1. a single product header shared by all models
    ///   2. a thin compatibility overlay for family-specific adaptation
    pub fn for_model(_model_api_id: &str) -> String {
        format!(
            "{}\n\n{}",
            PROMPT_ROCODE_HEADER.trim(),
            PROMPT_COMPATIBILITY_OVERLAY.trim()
        )
    }

    /// Build the environment context block.
    /// Produces a string like:
    /// ```text
    /// You are powered by the model named test-model-large. The exact model ID is ethnopic/test-model-large
    /// Here is some useful information about the environment you are running in:
    /// <env>
    ///   Working directory: /home/user/project
    ///   Is directory a git repo: yes
    ///   Platform: linux
    ///   Today's date: Wed Feb 19 2026
    ///   Current local time: 2026-02-19 14:03:07 +08:00
    ///   Local timezone: CST
    /// </env>
    /// ```
    pub fn environment(env: &EnvironmentContext) -> String {
        let now = Local::now();
        let mut lines = Vec::with_capacity(11);

        lines.push(format!(
            "You are powered by the model named {}. The exact model ID is {}/{}",
            env.model_api_id, env.provider_id, env.model_api_id
        ));
        lines.push(
            "Here is some useful information about the environment you are running in:".to_string(),
        );
        lines.push("<env>".to_string());
        lines.push(format!("  Working directory: {}", env.working_directory));
        lines.push(format!(
            "  Is directory a git repo: {}",
            if env.is_git_repo { "yes" } else { "no" }
        ));
        lines.push(format!("  Platform: {}", env.platform));
        lines.push(format!("  Today's date: {}", now.format("%a %b %d %Y")));
        lines.push(format!(
            "  Current local time: {}",
            now.format("%Y-%m-%d %H:%M:%S %:z")
        ));
        lines.push(format!("  Local timezone: {}", now.format("%Z")));
        lines.push("</env>".to_string());

        lines.join("\n")
    }
}

/// Context needed to build the environment block in the system prompt.
#[derive(Debug, Clone)]
pub struct EnvironmentContext {
    pub model_api_id: String,
    pub provider_id: String,
    pub working_directory: String,
    pub is_git_repo: bool,
    pub platform: String,
}

impl EnvironmentContext {
    /// Build from the current runtime environment.
    pub fn from_current(
        model_api_id: impl Into<String>,
        provider_id: impl Into<String>,
        working_directory: impl Into<String>,
    ) -> Self {
        let wd: String = working_directory.into();
        let is_git = std::path::Path::new(&wd).join(".git").exists();
        Self {
            model_api_id: model_api_id.into(),
            provider_id: provider_id.into(),
            working_directory: wd,
            is_git_repo: is_git,
            platform: std::env::consts::OS.to_string(),
        }
    }
}

fn trim_for_prompt(input: &str, max_chars: usize) -> (&str, bool) {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max_chars {
        return (trimmed, false);
    }

    let end = trimmed
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());
    (&trimmed[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_for_model_fallback() {
        let prompt = SystemPrompt::for_model("some-unknown-model");
        assert!(prompt.contains("You are ROCode."));
        assert!(prompt.contains("Compatibility overlay"));
    }

    #[test]
    fn test_for_model_gpt4() {
        let prompt = SystemPrompt::for_model("gpt-4o");
        assert!(prompt.contains("You are ROCode."));
        assert!(prompt.contains("Compatibility overlay"));
    }

    #[test]
    fn test_for_model_gpt5() {
        let prompt = SystemPrompt::for_model("gpt-5-turbo");
        assert!(prompt.contains("You are ROCode."));
        assert!(prompt.contains("Compatibility overlay"));
    }

    #[test]
    fn test_for_model_gemini() {
        let prompt = SystemPrompt::for_model("gemini-2.0-flash");
        assert!(prompt.contains("You are ROCode."));
        assert!(prompt.contains("Compatibility overlay"));
    }

    #[test]
    fn test_for_model_trinity() {
        let prompt = SystemPrompt::for_model("Trinity-Large");
        assert!(prompt.contains("You are ROCode."));
        assert!(prompt.contains("Compatibility overlay"));
    }

    #[test]
    fn test_environment_output() {
        let ctx = EnvironmentContext {
            model_api_id: "test-model-large".to_string(),
            provider_id: "ethnopic".to_string(),
            working_directory: "/tmp/test".to_string(),
            is_git_repo: true,
            platform: "linux".to_string(),
        };
        let env = SystemPrompt::environment(&ctx);
        assert!(env.contains("test-model-large"));
        assert!(env.contains("ethnopic/test-model-large"));
        assert!(env.contains("/tmp/test"));
        assert!(env.contains("Is directory a git repo: yes"));
        assert!(env.contains("Platform: linux"));
        assert!(env.contains("Current local time: "));
        assert!(env.contains("Local timezone: "));
        assert!(env.contains("<env>"));
        assert!(env.contains("</env>"));
    }

    #[test]
    fn test_environment_no_git() {
        let ctx = EnvironmentContext {
            model_api_id: "gpt-4o".to_string(),
            provider_id: "openai".to_string(),
            working_directory: "/tmp/no-git".to_string(),
            is_git_repo: false,
            platform: "macos".to_string(),
        };
        let env = SystemPrompt::environment(&ctx);
        assert!(env.contains("Is directory a git repo: no"));
    }

    #[test]
    fn test_instructions() {
        let inst = SystemPrompt::instructions();
        assert!(!inst.is_empty());
        assert!(inst.starts_with("You are ROCode"));
    }

    #[test]
    fn test_system_reminder_wraps_content() {
        let wrapped = SystemPrompt::system_reminder("hello");
        assert!(wrapped.starts_with("<system-reminder>"));
        assert!(wrapped.contains("hello"));
        assert!(wrapped.ends_with("</system-reminder>"));
    }

    #[test]
    fn test_mcp_resource_reminder_includes_filename_uri_and_content() {
        let wrapped = SystemPrompt::mcp_resource_reminder("rules.md", "repo/rules", "line1\nline2");
        assert!(wrapped.contains("MCP resource context from rules.md (repo/rules):"));
        assert!(wrapped.contains("line1"));
        assert!(wrapped.contains("<system-reminder>"));
    }

    #[test]
    fn test_mcp_resource_reminder_truncates_very_large_content() {
        let content = "a".repeat(20_000);
        let wrapped = SystemPrompt::mcp_resource_reminder("big.txt", "repo/big", &content);
        assert!(wrapped.contains("MCP resource context from big.txt (repo/big):"));
        assert!(wrapped.contains("Content truncated for prompt safety."));
        // sanity check: output should be significantly smaller than full payload.
        assert!(wrapped.len() < 15_000);
    }
}
