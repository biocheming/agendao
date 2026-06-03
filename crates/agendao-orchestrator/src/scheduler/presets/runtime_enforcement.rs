use crate::ExecutionContext;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeArtifactPolicy {
    Disabled,
    MarkdownUnder(&'static str),
}

pub fn validate_runtime_orchestration_tool(
    preset_name: &str,
    tool_name: &str,
    allowed_tools: &[&str],
) -> Result<(), String> {
    if allowed_tools
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(tool_name))
    {
        return Ok(());
    }

    Err(format!(
        "{preset_name} runtime may only invoke orchestration tools [{}]; got `{tool_name}`",
        allowed_tools.join(", ")
    ))
}

pub fn validate_runtime_artifact_path(
    preset_name: &str,
    raw_path: &str,
    exec_ctx: &ExecutionContext,
    policy: RuntimeArtifactPolicy,
) -> Result<(), String> {
    match policy {
        RuntimeArtifactPolicy::Disabled => Err(format!(
            "{preset_name} runtime does not manage scheduler artifacts: {raw_path}"
        )),
        RuntimeArtifactPolicy::MarkdownUnder(prefix) => {
            validate_markdown_under_prefix(preset_name, raw_path, exec_ctx, prefix)
        }
    }
}

fn validate_markdown_under_prefix(
    preset_name: &str,
    raw_path: &str,
    exec_ctx: &ExecutionContext,
    prefix: &str,
) -> Result<(), String> {
    let workdir = Path::new(&exec_ctx.workdir);
    let candidate = Path::new(raw_path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workdir.join(candidate)
    };

    let normalized = normalize_path(&resolved);
    let normalized_workdir = normalize_path(workdir);

    if !normalized.starts_with(&normalized_workdir) {
        return Err(format!(
            "{preset_name} runtime may only reference artifacts inside the session workdir: {raw_path}"
        ));
    }

    let relative = normalized
        .strip_prefix(&normalized_workdir)
        .ok()
        .unwrap_or(&normalized);
    let normalized_prefix = normalize_path(Path::new(prefix));

    if !relative.starts_with(&normalized_prefix) {
        return Err(format!(
            "{preset_name} runtime may only reference markdown artifacts under {prefix}: {raw_path}"
        ));
    }

    let is_markdown = normalized
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false);
    if !is_markdown {
        return Err(format!(
            "{preset_name} runtime may only reference markdown artifacts (*.md): {raw_path}"
        ));
    }

    Ok(())
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
