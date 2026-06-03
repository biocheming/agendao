use ast_grep_config::Fixer;
use ast_grep_core::source::Edit;
use ast_grep_language::{LanguageExt, SupportLang};
use async_trait::async_trait;
use glob::Pattern;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use walkdir::WalkDir;

use crate::ast_grep_common::{
    compile_pattern, count_placeholders, display_path, resolve_any_path, should_visit,
    AstGrepLanguage, DEFAULT_GLOB, MAX_RESULTS_LIMIT, SUPPORTED_LANGUAGES,
};
use crate::{
    assert_external_directory, with_file_lock, ExternalDirectoryKind, ExternalDirectoryOptions,
    Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult,
};

const DESCRIPTION: &str = r#"Structural code replacement using the ast-grep engine.

Phase 1 supports Rust syntax only. This tool performs AST-aware rewrites instead of plain text substitution.

Safety model:
- default is preview-only
- set apply=true to write changes to disk
- if the result set exceeds maxReplacements, apply=true is rejected to avoid partial structural rewrites"#;

const DEFAULT_MAX_REPLACEMENTS: usize = 50;
const MAX_REPLACEMENT_PREVIEW_CHARS: usize = 240;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AstGrepReplaceInput {
    pattern: String,
    replacement: String,
    language: AstGrepLanguage,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default = "default_max_replacements", alias = "max_replacements")]
    max_replacements: usize,
    #[serde(default)]
    apply: bool,
}

fn default_max_replacements() -> usize {
    DEFAULT_MAX_REPLACEMENTS
}

#[derive(Debug, Clone, Serialize)]
struct ReplaceEditPreview {
    file: String,
    line: usize,
    column: usize,
    end_line: usize,
    end_column: usize,
    kind: String,
    matched: String,
    replacement: String,
}

#[derive(Debug, Clone, Serialize)]
struct FileChangePreview {
    file: String,
    replacements: usize,
    diff: String,
    edits: Vec<ReplaceEditPreview>,
}

#[derive(Debug, Clone)]
struct FileChange {
    absolute_path: PathBuf,
    display_path: String,
    original: String,
    updated: String,
    edits: Vec<ReplaceEditPreview>,
}

#[derive(Debug, Clone)]
struct ReplaceOutcome {
    changes: Vec<FileChange>,
    total_replacements: usize,
    truncated: bool,
}

pub struct AstGrepReplaceTool;

impl AstGrepReplaceTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AstGrepReplaceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for AstGrepReplaceTool {
    fn id(&self) -> &str {
        "ast_grep_replace"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "AST pattern to replace"
                },
                "replacement": {
                    "type": "string",
                    "description": "Replacement template that can reference ast-grep metavariables"
                },
                "language": {
                    "type": "string",
                    "enum": SUPPORTED_LANGUAGES,
                    "description": "Language hint for the parser (Phase 1 currently supports rust only)"
                },
                "path": {
                    "type": "string",
                    "description": "Optional file or directory path. Defaults to current session directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional file glob filter when path is a directory"
                },
                "maxReplacements": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "default": 50,
                    "description": "Maximum number of structural replacements to preview or apply"
                },
                "max_replacements": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "default": 50,
                    "description": "Maximum number of structural replacements to preview or apply (snake_case alias)"
                },
                "apply": {
                    "type": "boolean",
                    "default": false,
                    "description": "When false, return a preview only. When true, write the transformed files to disk."
                }
            },
            "required": ["pattern", "replacement", "language"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: AstGrepReplaceInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        validate_input(&input)?;

        let target_path = resolve_any_path(&ctx, input.path.as_deref())?;
        let external_kind = if target_path.is_dir() {
            ExternalDirectoryKind::Directory
        } else {
            ExternalDirectoryKind::File
        };
        assert_external_directory(
            &ctx,
            Some(&target_path.to_string_lossy()),
            ExternalDirectoryOptions {
                bypass: false,
                kind: external_kind,
            },
        )
        .await?;

        ctx.ask_permission(
            PermissionRequest::new("ast_grep_replace")
                .with_pattern(&input.pattern)
                .with_metadata("language", serde_json::json!(input.language.as_str()))
                .with_metadata(
                    "path",
                    serde_json::json!(target_path.to_string_lossy().to_string()),
                )
                .with_metadata("apply", serde_json::json!(input.apply))
                .always_allow(),
        )
        .await?;

        let matcher = compile_pattern(&input.pattern, &input.language)?;
        let fixer = Fixer::from_str(input.replacement.trim(), &SupportLang::Rust).map_err(|e| {
            ToolError::InvalidArguments(format!("Invalid Rust ast-grep replacement: {}", e))
        })?;
        let glob_pattern = Pattern::new(input.glob.as_deref().unwrap_or(DEFAULT_GLOB))
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid glob pattern: {}", e)))?;

        let outcome = collect_replacements(
            &target_path,
            &glob_pattern,
            &matcher,
            &fixer,
            input.max_replacements,
        )?;

        if input.apply && outcome.truncated {
            return Err(ToolError::ExecutionError(format!(
                "Refusing to apply a truncated structural rewrite. Narrow the scope or increase maxReplacements above {}.",
                input.max_replacements
            )));
        }

        if input.apply && !outcome.changes.is_empty() {
            let diff_summary = build_permission_diff_summary(&outcome.changes);
            let permission_pattern = if target_path.is_dir() {
                format!("{}/*", target_path.to_string_lossy())
            } else {
                target_path.to_string_lossy().to_string()
            };
            ctx.ask_permission(
                PermissionRequest::new("edit")
                    .with_pattern(permission_pattern)
                    .with_metadata("diff", serde_json::json!(diff_summary))
                    .always_allow(),
            )
            .await?;

            apply_changes(&ctx, &outcome.changes).await?;
        }

        let previews: Vec<FileChangePreview> = outcome
            .changes
            .iter()
            .map(|change| FileChangePreview {
                file: change.display_path.clone(),
                replacements: change.edits.len(),
                diff: create_diff(&change.display_path, &change.original, &change.updated),
                edits: change.edits.clone(),
            })
            .collect();

        let output = render_replace_output(
            &input,
            &previews,
            outcome.total_replacements,
            outcome.truncated,
        );

        let mut metadata = Metadata::new();
        metadata.insert("pattern".into(), serde_json::json!(input.pattern));
        metadata.insert("replacement".into(), serde_json::json!(input.replacement));
        metadata.insert(
            "placeholder_count".into(),
            serde_json::json!(count_placeholders(&input.pattern)),
        );
        metadata.insert(
            "language".into(),
            serde_json::json!(input.language.as_str()),
        );
        metadata.insert(
            "path".into(),
            serde_json::json!(target_path.to_string_lossy().to_string()),
        );
        metadata.insert("glob".into(), serde_json::json!(input.glob));
        metadata.insert("apply".into(), serde_json::json!(input.apply));
        metadata.insert(
            "count".into(),
            serde_json::json!(outcome.total_replacements),
        );
        metadata.insert("fileCount".into(), serde_json::json!(previews.len()));
        metadata.insert("truncated".into(), serde_json::json!(outcome.truncated));
        metadata.insert(
            "changes".into(),
            serde_json::to_value(&previews).unwrap_or_else(|_| serde_json::json!([])),
        );
        metadata.insert("implemented".into(), serde_json::json!(true));

        Ok(ToolResult {
            title: if input.apply {
                format!("Applied ast_grep_replace '{}'", input.pattern.trim())
            } else {
                format!("Preview ast_grep_replace '{}'", input.pattern.trim())
            },
            output,
            metadata,
            truncated: outcome.truncated,
        })
    }
}

fn validate_input(input: &AstGrepReplaceInput) -> Result<(), ToolError> {
    if input.pattern.trim().is_empty() {
        return Err(ToolError::InvalidArguments(
            "pattern cannot be empty".to_string(),
        ));
    }
    if input.replacement.trim().is_empty() {
        return Err(ToolError::InvalidArguments(
            "replacement cannot be empty".to_string(),
        ));
    }
    if input.max_replacements == 0 || input.max_replacements > MAX_RESULTS_LIMIT {
        return Err(ToolError::InvalidArguments(format!(
            "maxReplacements must be between 1 and {}",
            MAX_RESULTS_LIMIT
        )));
    }
    Ok(())
}

fn collect_replacements(
    target_path: &Path,
    glob_pattern: &Pattern,
    matcher: &ast_grep_core::Pattern,
    fixer: &Fixer,
    max_replacements: usize,
) -> Result<ReplaceOutcome, ToolError> {
    let mut changes = Vec::new();
    let mut total_replacements = 0usize;
    let mut truncated = false;

    if target_path.is_file() {
        if let Some(change) = collect_file_change(
            target_path.parent().unwrap_or_else(|| Path::new(".")),
            target_path,
            matcher,
            fixer,
        )? {
            total_replacements += change.edits.len();
            if total_replacements > max_replacements {
                truncated = true;
                return Ok(ReplaceOutcome {
                    changes: Vec::new(),
                    total_replacements: max_replacements,
                    truncated,
                });
            }
            changes.push(change);
        }
        return Ok(ReplaceOutcome {
            changes,
            total_replacements,
            truncated,
        });
    }

    for entry in WalkDir::new(target_path)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| should_visit(entry, target_path))
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(target_path).unwrap_or(path);
        if !glob_pattern.matches_path(rel) {
            continue;
        }
        let Some(change) = collect_file_change(target_path, path, matcher, fixer)? else {
            continue;
        };
        if total_replacements + change.edits.len() > max_replacements {
            truncated = true;
            break;
        }
        total_replacements += change.edits.len();
        changes.push(change);
    }

    Ok(ReplaceOutcome {
        changes,
        total_replacements,
        truncated,
    })
}

fn collect_file_change(
    base_dir: &Path,
    path: &Path,
    matcher: &ast_grep_core::Pattern,
    fixer: &Fixer,
) -> Result<Option<FileChange>, ToolError> {
    let original = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            return Err(ToolError::ExecutionError(format!(
                "Failed to read {}: {}",
                path.display(),
                err
            )))
        }
    };

    let root = SupportLang::Rust.ast_grep(&original);
    let mut raw_edits: Vec<Edit<String>> = Vec::new();
    let mut previews = Vec::new();

    for matched in root.root().find_all(matcher) {
        let edit = matched.make_edit(matcher, fixer);
        if edit.deleted_length == 0 && edit.inserted_text.is_empty() {
            continue;
        }
        let start = edit.position;
        let end = edit.position + edit.deleted_length;
        let matched_text = original.get(start..end).unwrap_or_default().to_string();
        let replacement = String::from_utf8(edit.inserted_text.clone()).map_err(|err| {
            ToolError::ExecutionError(format!("Invalid replacement output: {}", err))
        })?;
        let (line, column) = byte_offset_to_line_col(&original, start);
        let (end_line, end_column) = byte_offset_to_line_col(&original, end);
        previews.push(ReplaceEditPreview {
            file: display_path(base_dir, path),
            line,
            column,
            end_line,
            end_column,
            kind: matched.kind().into_owned(),
            matched: truncate_chars(&matched_text, MAX_REPLACEMENT_PREVIEW_CHARS),
            replacement: truncate_chars(&replacement, MAX_REPLACEMENT_PREVIEW_CHARS),
        });
        raw_edits.push(edit);
    }

    if raw_edits.is_empty() {
        return Ok(None);
    }

    let updated = apply_edits(&original, &raw_edits)?;
    if updated == original {
        return Ok(None);
    }

    Ok(Some(FileChange {
        absolute_path: path.to_path_buf(),
        display_path: display_path(base_dir, path),
        original,
        updated,
        edits: previews,
    }))
}

fn apply_edits(source: &str, edits: &[Edit<String>]) -> Result<String, ToolError> {
    let mut out = Vec::with_capacity(source.len());
    let mut cursor = 0usize;

    for edit in edits {
        if edit.position < cursor {
            return Err(ToolError::ExecutionError(
                "ast-grep produced overlapping edits".to_string(),
            ));
        }
        if edit.position > source.len() {
            return Err(ToolError::ExecutionError(
                "ast-grep produced an out-of-bounds edit".to_string(),
            ));
        }
        let end = edit.position + edit.deleted_length;
        if end > source.len() {
            return Err(ToolError::ExecutionError(
                "ast-grep produced an out-of-bounds delete range".to_string(),
            ));
        }
        out.extend_from_slice(&source.as_bytes()[cursor..edit.position]);
        out.extend_from_slice(&edit.inserted_text);
        cursor = end;
    }

    out.extend_from_slice(&source.as_bytes()[cursor..]);
    String::from_utf8(out).map_err(|err| {
        ToolError::ExecutionError(format!("Failed to build rewritten file: {}", err))
    })
}

async fn apply_changes(ctx: &ToolContext, changes: &[FileChange]) -> Result<(), ToolError> {
    for change in changes {
        let path = change.absolute_path.clone();
        let path_str = path.to_string_lossy().to_string();
        ctx.do_file_time_assert(path_str.clone()).await?;

        with_file_lock(&path_str, || async {
            tokio_fs::write(&path, &change.updated).await
        })
        .await
        .map_err(|err| ToolError::ExecutionError(format!("Failed to write file: {}", err)))?;

        ctx.do_publish_bus(
            "file.edited",
            serde_json::json!({
                "file": path_str,
            }),
        )
        .await;
        ctx.do_publish_bus(
            "file_watcher.updated",
            serde_json::json!({
                "file": path.to_string_lossy().to_string(),
                "event": "change"
            }),
        )
        .await;
        ctx.do_lsp_touch_file(path.to_string_lossy().to_string(), true)
            .await?;
        ctx.do_file_time_read(path.to_string_lossy().to_string())
            .await?;
    }
    Ok(())
}

fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let bounded = offset.min(source.len());
    let prefix = &source[..bounded];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map(|(_, tail)| tail.chars().count() + 1)
        .unwrap_or_else(|| prefix.chars().count() + 1);
    (line, column)
}

fn build_permission_diff_summary(changes: &[FileChange]) -> String {
    let mut lines = Vec::new();
    for change in changes {
        lines.push(format!("--- {}", change.display_path));
        lines.push(create_diff(
            &change.display_path,
            &change.original,
            &change.updated,
        ));
    }
    lines.join("\n")
}

fn create_diff(filepath: &str, old_content: &str, new_content: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut diff = format!("--- {}\n+++ {}\n", filepath, filepath);
    let mut old_idx = 0usize;
    let mut new_idx = 0usize;

    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        if old_idx >= old_lines.len() {
            diff.push_str(&format!("+{}\n", new_lines[new_idx]));
            new_idx += 1;
        } else if new_idx >= new_lines.len() {
            diff.push_str(&format!("-{}\n", old_lines[old_idx]));
            old_idx += 1;
        } else if old_lines[old_idx] == new_lines[new_idx] {
            old_idx += 1;
            new_idx += 1;
        } else {
            diff.push_str(&format!("-{}\n", old_lines[old_idx]));
            diff.push_str(&format!("+{}\n", new_lines[new_idx]));
            old_idx += 1;
            new_idx += 1;
        }
    }

    diff
}

fn render_replace_output(
    input: &AstGrepReplaceInput,
    previews: &[FileChangePreview],
    total_replacements: usize,
    truncated: bool,
) -> String {
    if previews.is_empty() {
        return format!(
            "No structural replacements found for Rust ast-grep pattern: {}",
            input.pattern.trim()
        );
    }

    let mut lines = vec![format!(
        "{} {} structural replacement(s) across {} file(s) for pattern '{}'{}",
        if input.apply { "Applied" } else { "Previewed" },
        total_replacements,
        previews.len(),
        input.pattern.trim(),
        if truncated { " (truncated)" } else { "" }
    )];

    if !input.apply {
        lines.push("Preview only. Re-run with apply=true to write changes.".to_string());
    }

    for (index, change) in previews.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!(
            "{}. {} ({} replacement(s))",
            index + 1,
            change.file,
            change.replacements
        ));
        for edit in &change.edits {
            lines.push(format!(
                "   - {}:{}:{} {}",
                edit.file, edit.line, edit.column, edit.kind
            ));
            lines.push(format!(
                "     matched: {}",
                edit.matched.replace('\n', "\\n")
            ));
            lines.push(format!(
                "     replacement: {}",
                edit.replacement.replace('\n', "\\n")
            ));
        }
    }

    lines.join("\n")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn schema_exposes_preview_and_apply_fields() {
        let tool = AstGrepReplaceTool::new();
        let schema = tool.parameters();
        assert!(schema["properties"].get("replacement").is_some());
        assert!(schema["properties"].get("apply").is_some());
        assert!(schema["properties"].get("maxReplacements").is_some());
    }

    #[tokio::test]
    async fn preview_reports_structural_rewrite_without_writing() {
        let dir = tempdir().expect("tempdir should exist");
        let file = dir.path().join("sample.rs");
        fs::write(&file, "fn demo() {\n    foo(bar);\n}\n").expect("fixture should write");

        let tool = AstGrepReplaceTool::new();
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "foo($X)",
                    "replacement": "bar($X)",
                    "language": "rust"
                }),
                ctx,
            )
            .await
            .expect("preview should succeed");

        let current = fs::read_to_string(&file).expect("file should still exist");
        assert!(result.output.contains("Preview only"));
        assert!(result.output.contains("replacement: bar(bar)"));
        assert!(current.contains("foo(bar)"));
        assert_eq!(result.metadata["apply"], serde_json::json!(false));
        assert_eq!(result.metadata["count"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn apply_writes_transformed_file() {
        let dir = tempdir().expect("tempdir should exist");
        let file = dir.path().join("sample.rs");
        fs::write(&file, "fn demo() {\n    foo(bar);\n}\n").expect("fixture should write");

        let tool = AstGrepReplaceTool::new();
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "foo($X)",
                    "replacement": "bar($X)",
                    "language": "rust",
                    "apply": true
                }),
                ctx,
            )
            .await
            .expect("apply should succeed");

        let current = fs::read_to_string(&file).expect("file should still exist");
        assert!(result.output.contains("Applied 1 structural replacement"));
        assert!(current.contains("bar(bar)"));
        assert!(!current.contains("foo(bar)"));
        assert_eq!(result.metadata["apply"], serde_json::json!(true));
    }
}
