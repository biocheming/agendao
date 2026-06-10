use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;

// Legacy constants — prefer reading from agendao_config::RuntimeBudgetConfig.
// These exist for backward compatibility; new code should use budget_authority().
pub const SINGLE_TOOL_RESULT_MAX_CHARS: usize = 32_000;
pub const TOOL_RESULT_BATCH_MAX_CHARS: usize = 120_000;
const TOOL_RESULT_PREVIEW_CHARS: usize = 8_000;

/// Read tool result budget from the canonical authority.
/// Falls back to the legacy constants when no config store is available
/// (standalone / non-server contexts).
pub fn tool_result_budget(
    config: Option<&agendao_config::RuntimeBudgetConfig>,
) -> ToolResultBudget {
    config.map_or_else(ToolResultBudget::legacy, |b| ToolResultBudget {
        max_single_chars: b.tool_result_max_chars,
        max_batch_chars: b.tool_batch_aggregate_max_chars,
        preview_chars: b.tool_result_preview_chars,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct ToolResultBudget {
    pub max_single_chars: usize,
    pub max_batch_chars: usize,
    pub preview_chars: usize,
}

impl ToolResultBudget {
    pub fn legacy() -> Self {
        Self {
            max_single_chars: SINGLE_TOOL_RESULT_MAX_CHARS,
            max_batch_chars: TOOL_RESULT_BATCH_MAX_CHARS,
            preview_chars: TOOL_RESULT_PREVIEW_CHARS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GovernedToolResultOutput {
    pub output: String,
    pub degraded: bool,
    pub original_chars: usize,
    pub governed_chars: usize,
    pub artifact_path: Option<String>,
}

pub type GovernedStreamToolResultEntry = (
    String,
    String,
    bool,
    Option<String>,
    Option<HashMap<String, serde_json::Value>>,
    Option<Vec<serde_json::Value>>,
);

pub fn govern_tool_result_output(
    session_id: &str,
    tool_call_id: &str,
    output: String,
    metadata: &mut HashMap<String, serde_json::Value>,
    artifacts_root: &Path,
    budget: ToolResultBudget,
) -> GovernedToolResultOutput {
    let original_chars = output.chars().count();
    if original_chars <= budget.max_single_chars {
        return GovernedToolResultOutput {
            governed_chars: original_chars,
            output,
            degraded: false,
            original_chars,
            artifact_path: None,
        };
    }

    let artifact_path = persist_large_tool_result(
        artifacts_root,
        session_id,
        tool_call_id,
        &output,
        metadata
            .get("content_type")
            .and_then(|value| value.as_str())
            .unwrap_or("text/plain"),
    )
    .ok();
    let preview: String = output.chars().take(budget.preview_chars).collect();
    let governed = build_governed_preview(
        original_chars,
        preview.chars().count(),
        artifact_path.as_deref(),
        &preview,
    );
    let governed_chars = governed.chars().count();

    metadata.insert("tool_result_governed".to_string(), serde_json::json!(true));
    metadata.insert(
        "tool_result_preview_only".to_string(),
        serde_json::json!(true),
    );
    metadata.insert(
        "tool_result_original_chars".to_string(),
        serde_json::json!(original_chars),
    );
    metadata.insert(
        "tool_result_governed_chars".to_string(),
        serde_json::json!(governed_chars),
    );
    metadata.insert(
        "tool_result_governance_reason".to_string(),
        serde_json::json!("output_too_large"),
    );
    if let Some(path) = artifact_path.as_ref() {
        metadata.insert(
            "tool_result_artifact_path".to_string(),
            serde_json::json!(path),
        );
    } else {
        metadata.insert(
            "tool_result_artifact_persist_failed".to_string(),
            serde_json::json!(true),
        );
    }

    GovernedToolResultOutput {
        output: governed,
        degraded: true,
        original_chars,
        governed_chars,
        artifact_path,
    }
}

pub fn govern_tool_result_batch(
    session_id: &str,
    stream_tool_results: Vec<GovernedStreamToolResultEntry>,
    artifacts_root: &Path,
    budget: ToolResultBudget,
) -> Vec<GovernedStreamToolResultEntry> {
    let total_chars: usize = stream_tool_results
        .iter()
        .map(|(_, content, _, _, _, _)| content.chars().count())
        .sum();
    if total_chars <= budget.max_batch_chars {
        return stream_tool_results;
    }

    let mut indexed = stream_tool_results
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>();
    indexed.sort_by_key(|(_, (_, content, _, _, _, _))| std::cmp::Reverse(content.chars().count()));

    let mut current_total = total_chars;
    let mut rewritten: HashMap<usize, GovernedStreamToolResultEntry> = HashMap::new();
    for (original_index, (tool_call_id, content, is_error, title, metadata, attachments)) in indexed
    {
        if current_total <= budget.max_batch_chars {
            rewritten.insert(
                original_index,
                (
                    tool_call_id,
                    content,
                    is_error,
                    title,
                    metadata,
                    attachments,
                ),
            );
            continue;
        }

        let mut metadata_map = metadata.unwrap_or_default();
        metadata_map.insert(
            "tool_result_batch_governed".to_string(),
            serde_json::json!(true),
        );
        metadata_map.insert(
            "tool_result_batch_governance_reason".to_string(),
            serde_json::json!("aggregate_output_too_large"),
        );
        let governed = govern_tool_result_output(
            session_id,
            &tool_call_id,
            content.clone(),
            &mut metadata_map,
            artifacts_root,
            budget,
        );
        current_total = current_total
            .saturating_sub(content.chars().count())
            .saturating_add(governed.governed_chars);
        rewritten.insert(
            original_index,
            (
                tool_call_id,
                governed.output,
                is_error,
                title,
                Some(metadata_map),
                attachments,
            ),
        );
    }

    let mut out = rewritten.into_iter().collect::<Vec<_>>();
    out.sort_by_key(|(index, _)| *index);
    out.into_iter().map(|(_, entry)| entry).collect()
}

fn persist_large_tool_result(
    artifacts_root: &Path,
    session_id: &str,
    tool_call_id: &str,
    output: &str,
    content_type: &str,
) -> std::io::Result<String> {
    let dir = artifacts_root.join(session_id).join("tool-results");
    std::fs::create_dir_all(&dir)?;
    let ext = extension_for_content_type(content_type);
    let ts = timestamp_ms();
    let filename = format!("{tool_call_id}-{ts}.{ext}");
    let path = dir.join(filename);
    std::fs::write(&path, output)?;
    Ok(path.to_string_lossy().to_string())
}

fn build_governed_preview(
    original_chars: usize,
    preview_chars: usize,
    artifact_path: Option<&str>,
    preview: &str,
) -> String {
    let mut out = String::from("[tool result governed: output too large]\n");
    out.push_str(&format!("original_chars: {original_chars}\n"));
    out.push_str(&format!("preview_chars: {preview_chars}\n"));
    if let Some(path) = artifact_path {
        out.push_str(&format!("artifact: {path}\n"));
    } else {
        out.push_str("artifact: unavailable\n");
    }
    out.push_str(
        "artifact_read_hint: this artifact is a local file. Read it with `artifact_read(artifact_path)` or a local file reader such as `read(file_path)`. Do not pass it to `webfetch` or `browser_session`.\n",
    );
    out.push_str("\nPreview:\n");
    out.push_str(preview);
    out
}

fn extension_for_content_type(content_type: &str) -> &'static str {
    match content_type {
        "application/json" => "json",
        "text/markdown" => "md",
        _ => "txt",
    }
}

fn timestamp_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn default_tool_result_artifacts_root(worktree: &str) -> PathBuf {
    Path::new(worktree)
        .join(".agendao")
        .join("session-artifacts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_passes_through_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut metadata = HashMap::new();
        let governed = govern_tool_result_output(
            "session-1",
            "call-1",
            "short output".to_string(),
            &mut metadata,
            dir.path(),
            ToolResultBudget::legacy(),
        );
        assert!(!governed.degraded);
        assert_eq!(governed.output, "short output");
        assert!(governed.artifact_path.is_none());
        assert!(!metadata.contains_key("tool_result_governed"));
    }

    #[test]
    fn large_output_is_persisted_and_previewed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut metadata = HashMap::new();
        let large = "x".repeat(SINGLE_TOOL_RESULT_MAX_CHARS + 1024);
        let governed = govern_tool_result_output(
            "session-1",
            "call-1",
            large,
            &mut metadata,
            dir.path(),
            ToolResultBudget::legacy(),
        );
        assert!(governed.degraded);
        assert!(governed
            .output
            .contains("[tool result governed: output too large]"));
        let artifact = governed.artifact_path.expect("artifact path");
        assert!(std::path::Path::new(&artifact).exists());
        assert_eq!(
            metadata.get("tool_result_governed"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            metadata.get("tool_result_preview_only"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            metadata.get("tool_result_governance_reason"),
            Some(&serde_json::json!("output_too_large"))
        );
        assert!(governed
            .output
            .contains("artifact_read_hint: this artifact is a local file."));
        assert!(governed.output.contains("Do not pass it to `webfetch` or `browser_session`"));
    }

    #[test]
    fn large_batch_governs_largest_results_first_without_reordering() {
        let dir = tempfile::tempdir().expect("tempdir");
        let batch = vec![
            (
                "call-1".to_string(),
                "A".repeat(10_000),
                false,
                None,
                None,
                None,
            ),
            (
                "call-2".to_string(),
                "B".repeat(90_000),
                false,
                None,
                None,
                None,
            ),
            (
                "call-3".to_string(),
                "C".repeat(40_000),
                false,
                None,
                None,
                None,
            ),
        ];

        let governed =
            govern_tool_result_batch("session-1", batch, dir.path(), ToolResultBudget::legacy());
        assert_eq!(governed.len(), 3);
        assert_eq!(governed[0].0, "call-1");
        assert_eq!(governed[1].0, "call-2");
        assert_eq!(governed[2].0, "call-3");
        assert!(governed[1]
            .1
            .contains("[tool result governed: output too large]"));
        assert!(
            governed[1]
                .4
                .as_ref()
                .and_then(|m| m.get("tool_result_batch_governed"))
                == Some(&serde_json::json!(true))
        );
    }
}
