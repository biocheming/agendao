use crate::schema::{AgentConfig, CommandConfig};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

/// Parse a Markdown file as a command definition.
pub(super) fn parse_markdown_command(
    path: &Path,
    base_dir: &Path,
) -> Result<(String, CommandConfig)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read command definition {}", path.display()))?;
    let (frontmatter, body) = split_frontmatter(&content)
        .with_context(|| format!("invalid command definition {}", path.display()))?;
    let name = derive_name_from_path(path, base_dir, &["command", "commands"]);
    let mut config = match frontmatter {
        Some(frontmatter) => serde_yaml::from_str::<CommandConfig>(&frontmatter)
            .with_context(|| format!("invalid command frontmatter in {}", path.display()))?,
        None => CommandConfig::default(),
    };
    config.template = Some(body.trim().to_string());
    Ok((name, config))
}

/// Parse a Markdown file as an agent definition.
pub(super) fn parse_markdown_agent(path: &Path, base_dir: &Path) -> Result<(String, AgentConfig)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read agent definition {}", path.display()))?;
    let (frontmatter, body) = split_frontmatter(&content)
        .with_context(|| format!("invalid agent definition {}", path.display()))?;
    let name = derive_name_from_path(path, base_dir, &["agent", "agents"]);
    let mut config = match frontmatter {
        Some(frontmatter) => serde_yaml::from_str::<AgentConfig>(&frontmatter)
            .with_context(|| format!("invalid agent frontmatter in {}", path.display()))?,
        None => AgentConfig::default(),
    };
    config.prompt = Some(body.trim().to_string());
    Ok((name, config))
}

/// Split optional YAML frontmatter from the Markdown body.
pub(super) fn split_frontmatter(content: &str) -> Result<(Option<String>, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Ok((None, content.to_string()));
    }

    let after_first = &trimmed[3..];
    let Some(end_idx) = after_first.find("\n---") else {
        bail!("frontmatter opening delimiter has no closing delimiter");
    };
    let frontmatter = after_first[..end_idx].trim().to_string();
    let body_start = end_idx + 4;
    let body = if body_start < after_first.len() {
        after_first[body_start..].to_string()
    } else {
        String::new()
    };
    Ok((Some(frontmatter), body))
}

fn derive_name_from_path(path: &Path, base_dir: &Path, strip_prefixes: &[&str]) -> String {
    let relative = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let mut name = relative.as_str();
    for prefix in strip_prefixes {
        let direct = format!("{prefix}/");
        if let Some(stripped) = name.strip_prefix(&direct) {
            name = stripped;
            break;
        }
        let nested = format!(".agendao/{prefix}/");
        if let Some(stripped) = name.strip_prefix(&nested) {
            name = stripped;
            break;
        }
    }
    name.strip_suffix(".md").unwrap_or(name).to_string()
}
