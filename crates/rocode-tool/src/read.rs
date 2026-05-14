use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;
use walkdir::WalkDir;

use crate::path_guard::{resolve_user_path, RootPathFallbackPolicy};
use crate::tool_access::{
    self, read_block_message, read_warning_message, ToolAccessKey, ToolAccessOutcome,
};
use crate::{
    append_tool_repair_event_map, merge_tool_repair_telemetry, tool_repair_event, Metadata, Tool,
    ToolContext, ToolError, ToolResult,
};

const DEFAULT_READ_LIMIT: usize = 2000;
const MAX_LINE_LENGTH: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;
const DESCRIPTION: &str = include_str!("read.txt");

const INSTRUCTION_FILES: &[&str] = &[
    "AGENTS.md",
    "CONTEXT.md",
    "CONTEXT.txt",
    ".context",
    ".cursorrules",
    ".opencoderules",
];

const BASENAME_REPAIR_MAX_MATCHES: usize = 8;
const BASENAME_REPAIR_SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
];

pub struct ReadTool {
    directory: PathBuf,
}

impl ReadTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }

    pub fn with_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
}

impl Default for ReadTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn id(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Absolute path or project-relative path to the file or directory to read."
                },
                "offset": {
                    "type": "number",
                    "description": "The line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "number",
                    "description": "The maximum number of lines to read (defaults to 2000)"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let file_path: String = args
            .get("file_path")
            .or_else(|| args.get("filePath"))
            .or_else(|| args.get("filepath"))
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments(format!(
                    "file_path is required. Got args: {}. If you are unsure of the correct path, use glob first.",
                    serde_json::to_string(&args).unwrap_or_else(|_| format!("{:?}", args))
                ))
            })?
            .to_string();
        let file_path = file_path.trim().to_string();
        if file_path.is_empty() {
            return Err(ToolError::InvalidArguments(
                "file_path cannot be empty. If you do not know the path, call glob first (for example: pattern='**/*.html')."
                    .into(),
            ));
        }

        let offset: usize = args["offset"].as_u64().unwrap_or(1) as usize;

        let limit: usize = args["limit"].as_u64().unwrap_or(DEFAULT_READ_LIMIT as u64) as usize;

        if offset < 1 {
            return Err(ToolError::InvalidArguments("offset must be >= 1".into()));
        }

        let base_dir = if ctx.directory.is_empty() {
            &self.directory
        } else {
            Path::new(&ctx.directory)
        };

        let resolved = resolve_user_path(
            &file_path,
            base_dir,
            RootPathFallbackPolicy::ExistingFallbackOnly,
        );
        let path = resolved.resolved;
        if let Some(original) = resolved.corrected_from {
            tracing::warn!(
                from = %original.display(),
                to = %path.display(),
                session_dir = %base_dir.display(),
                "corrected suspicious root-level read path into session directory"
            );
        }

        let (path, repaired_from, basename_suggestions) =
            repair_missing_read_path(&file_path, path, base_dir, Path::new(&ctx.project_root));
        if let Some(ref original) = repaired_from {
            tracing::warn!(
                from = %original.display(),
                to = %path.display(),
                session_dir = %base_dir.display(),
                "auto-repaired basename-only read path into unique workspace match"
            );
        }

        let path_str = path.to_string_lossy().to_string();
        let mut repair_metadata = Metadata::new();
        if repaired_from.is_some() {
            let mut event = tool_repair_event("basename_auto_repair", "tool", "read");
            event.insert("from".to_string(), serde_json::json!(file_path));
            event.insert("to".to_string(), serde_json::json!(path_str.clone()));
            event.insert(
                "strategy".to_string(),
                serde_json::json!("unique_workspace_basename_match"),
            );
            append_tool_repair_event_map(&mut repair_metadata, event);
        }

        if ctx.is_external_path(&path_str) {
            let parent = path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| path_str.clone());

            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(format!("{}/*", parent))
                    .with_scope_key(crate::external_fs_scope_key(&parent))
                    .with_metadata("filepath", serde_json::json!(path_str))
                    .with_metadata("parentDir", serde_json::json!(parent)),
            )
            .await?;
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("read")
                .with_pattern(&path_str)
                .with_scope_key(crate::workspace_scope_key(&ctx.project_root, &path_str))
                .always_allow(),
        )
        .await?;

        let title = path
            .strip_prefix(&ctx.worktree)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let metadata = fs::metadata(&path).await.map_err(|_e| {
            let dir = path.parent().unwrap_or(Path::new("."));
            let base = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let sibling_suggestions = if let Ok(entries) = std::fs::read_dir(dir) {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        let target = base.to_lowercase();
                        name.contains(&target) || target.contains(&name)
                    })
                    .take(3)
                    .map(|e| e.path().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let suggestions = merge_suggestions(&basename_suggestions, &sibling_suggestions, 6);
            if !suggestions.is_empty() {
                ToolError::with_suggestions(
                    format!("File not found: {}", path.display()),
                    &suggestions,
                )
            } else {
                ToolError::FileNotFound(format!("File not found: {}", path.display()))
            }
        })?;

        if metadata.is_dir() {
            let outcome = tool_access::record_tool_access(
                &ctx.session_id,
                ToolAccessKey::Read {
                    path: path_str.clone(),
                    offset,
                    limit,
                },
            );
            if let ToolAccessOutcome::Block { consecutive } = outcome {
                return Err(ToolError::ExecutionError(read_block_message(consecutive)));
            }
            ctx.do_file_time_read(path_str.clone()).await?;
            ctx.do_lsp_touch_file(path_str.clone(), false).await?;
            let mut result = read_directory(&path, offset, limit, title)?;
            merge_tool_repair_telemetry(&mut result.metadata, &repair_metadata);
            return Ok(apply_repeated_access_feedback(result, outcome));
        }

        let content = fs::read(&path)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read file: {}", e)))?;

        let mime = detect_mime(&path);

        if is_image_mime(&mime) || mime == "application/pdf" {
            let outcome = tool_access::record_tool_access(
                &ctx.session_id,
                ToolAccessKey::Read {
                    path: path_str.clone(),
                    offset,
                    limit,
                },
            );
            if let ToolAccessOutcome::Block { consecutive } = outcome {
                return Err(ToolError::ExecutionError(read_block_message(consecutive)));
            }
            ctx.do_file_time_read(path_str.clone()).await?;
            ctx.do_lsp_touch_file(path_str.clone(), false).await?;
            let mut result = handle_binary_file(&path, &content, &mime, title)?;
            merge_tool_repair_telemetry(&mut result.metadata, &repair_metadata);
            return Ok(apply_repeated_access_feedback(result, outcome));
        }

        if is_binary(&content) {
            return Err(ToolError::BinaryFile(path.display().to_string()));
        }

        ctx.do_file_time_read(path_str.clone()).await?;
        ctx.do_lsp_touch_file(path_str.clone(), false).await?;
        let outcome = tool_access::record_tool_access(
            &ctx.session_id,
            ToolAccessKey::Read {
                path: path_str.clone(),
                offset,
                limit,
            },
        );
        if let ToolAccessOutcome::Block { consecutive } = outcome {
            return Err(ToolError::ExecutionError(read_block_message(consecutive)));
        }
        let mut result = read_file_content(
            &path,
            &path_str,
            &content,
            offset,
            limit,
            title,
            &ctx.project_root,
        )
        .await?;
        merge_tool_repair_telemetry(&mut result.metadata, &repair_metadata);
        Ok(apply_repeated_access_feedback(result, outcome))
    }
}

fn apply_repeated_access_feedback(
    mut result: ToolResult,
    outcome: ToolAccessOutcome,
) -> ToolResult {
    if let ToolAccessOutcome::Warn { consecutive } = outcome {
        let warning = read_warning_message(consecutive);
        result.output = format!("[Repeated read warning]\n{}\n\n{}", warning, result.output);
        result.metadata.insert(
            "toolAccessGuard".into(),
            serde_json::json!({
                "kind": "read",
                "status": "warning",
                "count": consecutive,
                "message": warning,
            }),
        );
    }
    result
}

fn repair_missing_read_path(
    raw_path: &str,
    path: PathBuf,
    base_dir: &Path,
    project_root: &Path,
) -> (PathBuf, Option<PathBuf>, Vec<String>) {
    if path.exists() || !is_basename_only_path(raw_path) {
        return (path, None, Vec::new());
    }

    let matches = find_workspace_basename_matches(raw_path, base_dir, project_root);
    if matches.len() == 1 {
        return (matches[0].clone(), Some(path), Vec::new());
    }

    let suggestions = matches
        .into_iter()
        .map(|candidate| candidate.to_string_lossy().to_string())
        .collect();
    (path, None, suggestions)
}

fn is_basename_only_path(raw_path: &str) -> bool {
    let path = Path::new(raw_path.trim());
    !path.is_absolute()
        && matches!(
            (path.components().next(), path.components().nth(1)),
            (Some(std::path::Component::Normal(_)), None)
        )
}

fn find_workspace_basename_matches(
    raw_path: &str,
    base_dir: &Path,
    project_root: &Path,
) -> Vec<PathBuf> {
    let basename = raw_path.trim();
    if basename.is_empty() {
        return Vec::new();
    }

    let mut roots = Vec::new();
    if base_dir.exists() {
        roots.push(base_dir.to_path_buf());
    }
    if project_root.exists() && !roots.iter().any(|root| root == project_root) {
        roots.push(project_root.to_path_buf());
    }

    let mut matches = Vec::new();
    for root in roots {
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| should_visit_basename_repair(entry.path()))
            .filter_map(Result::ok)
        {
            if entry.file_name().to_string_lossy() != basename {
                continue;
            }
            let candidate = entry.path().to_path_buf();
            if !matches.iter().any(|existing| existing == &candidate) {
                matches.push(candidate);
            }
            if matches.len() >= BASENAME_REPAIR_MAX_MATCHES {
                return matches;
            }
        }
    }
    matches
}

fn should_visit_basename_repair(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| !BASENAME_REPAIR_SKIP_DIRS.contains(&name))
        .unwrap_or(true)
}

fn merge_suggestions(primary: &[String], secondary: &[String], limit: usize) -> Vec<String> {
    let mut merged = Vec::new();
    for candidate in primary.iter().chain(secondary.iter()) {
        if merged.iter().any(|existing| existing == candidate) {
            continue;
        }
        merged.push(candidate.clone());
        if merged.len() >= limit {
            break;
        }
    }
    merged
}

fn detect_mime(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "ts" => "application/typescript",
        "md" => "text/markdown",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml" && mime != "image/vnd.fastbidsheet"
}

fn handle_binary_file(
    path: &Path,
    content: &[u8],
    mime: &str,
    title: String,
) -> Result<ToolResult, ToolError> {
    let base64_content =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, content);
    let data_url = format!("data:{};base64,{}", mime, base64_content);

    let file_type = if mime.starts_with("image/") {
        "Image"
    } else {
        "PDF"
    };
    let msg = format!("{} read successfully ({} bytes)", file_type, content.len());

    let output = format!(
        "<path>{}</path>\n<type>binary</type>\n<mime>{}</mime>\n<size>{}</size>\n<total-lines>0</total-lines>\n<content>\n{}\n</content>",
        path.display(),
        mime,
        content.len(),
        msg
    );

    let mut attachment = serde_json::Map::new();
    attachment.insert("type".to_string(), serde_json::json!("file"));
    attachment.insert("mime".to_string(), serde_json::json!(mime));
    attachment.insert("url".to_string(), serde_json::json!(data_url));
    if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
        attachment.insert("filename".to_string(), serde_json::json!(filename));
    }
    let attachment_value = serde_json::Value::Object(attachment);

    Ok(ToolResult {
        title,
        output,
        metadata: {
            let mut m = Metadata::new();
            m.insert("preview".into(), serde_json::json!(msg));
            m.insert("truncated".into(), serde_json::json!(false));
            m.insert("mime".into(), serde_json::json!(mime));
            m.insert("size".into(), serde_json::json!(content.len()));
            m.insert("attachment".into(), attachment_value.clone());
            m.insert("attachments".into(), serde_json::json!([attachment_value]));
            m
        },
        truncated: false,
    })
}

fn read_directory(
    path: &Path,
    offset: usize,
    limit: usize,
    title: String,
) -> Result<ToolResult, ToolError> {
    let mut entries: Vec<String> = Vec::new();

    for entry in WalkDir::new(path)
        .max_depth(1)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path() == path {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().is_dir() {
            entries.push(format!("{}/", name));
        } else {
            entries.push(name);
        }
    }

    entries.sort();

    let start = offset.saturating_sub(1);
    let sliced: Vec<&str> = entries
        .iter()
        .skip(start)
        .take(limit)
        .map(|s| s.as_str())
        .collect();
    let truncated = start + sliced.len() < entries.len();

    let output = format!(
        "<path>{}</path>\n<type>directory</type>\n<entries>\n{}\n{}{}\n</entries>",
        path.display(),
        sliced.join("\n"),
        if truncated {
            format!(
                "\n(Showing {} of {} entries. Use 'offset' parameter to read beyond entry {})",
                sliced.len(),
                entries.len(),
                offset + sliced.len()
            )
        } else {
            format!("\n({} entries)", entries.len())
        },
        ""
    );

    let preview = sliced
        .iter()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    Ok(ToolResult {
        title,
        output,
        metadata: {
            let mut m = Metadata::new();
            m.insert("preview".into(), serde_json::json!(preview));
            m.insert("truncated".into(), serde_json::json!(truncated));
            m
        },
        truncated,
    })
}

async fn read_file_content(
    path: &Path,
    path_str: &str,
    content: &[u8],
    offset: usize,
    limit: usize,
    title: String,
    project_root: &str,
) -> Result<ToolResult, ToolError> {
    let text = String::from_utf8_lossy(content);
    let lines: Vec<&str> = text.lines().collect();

    if offset > lines.len() {
        return Err(ToolError::InvalidArguments(format!(
            "Offset {} is out of range (file has {} lines)",
            offset,
            lines.len()
        )));
    }

    let start = offset.saturating_sub(1);
    let mut result_lines: Vec<String> = Vec::new();
    let mut bytes = 0;
    let mut truncated_by_bytes = false;
    let mut truncated_by_line = false;

    for (i, line_text) in lines
        .iter()
        .enumerate()
        .take(std::cmp::min(lines.len(), start + limit))
        .skip(start)
    {
        let (line, line_was_truncated) = truncate_line_for_output(line_text, MAX_LINE_LENGTH);
        truncated_by_line |= line_was_truncated;

        let size = line.len() + if result_lines.is_empty() { 0 } else { 1 };
        if bytes + size > MAX_BYTES {
            truncated_by_bytes = true;
            break;
        }

        result_lines.push(format!("{}: {}", i + 1, line));
        bytes += size;
    }

    let preview = result_lines
        .iter()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let total_lines = lines.len();
    let last_read_line = start + result_lines.len();
    let has_more_lines = total_lines > last_read_line;
    let truncated = has_more_lines || truncated_by_bytes || truncated_by_line;

    let truncation_msg = if truncated_by_bytes {
        format!(
            "\n\n(Output truncated at {} bytes. Use 'offset' parameter to read beyond line {})",
            MAX_BYTES, last_read_line
        )
    } else if truncated_by_line {
        format!(
            "\n\n(Some lines were truncated at {} characters for display.)",
            MAX_LINE_LENGTH
        )
    } else if has_more_lines {
        format!(
            "\n\n(File has more lines. Use 'offset' parameter to read beyond line {})",
            last_read_line
        )
    } else {
        format!("\n\n(End of file - total {} lines)", total_lines)
    };

    let mut output = format!(
        "<path>{}</path>\n<type>file</type>\n<size>{}</size>\n<total-lines>{}</total-lines>\n<content>\n{}{}\n</content>",
        path.display(),
        content.len(),
        total_lines,
        result_lines.join("\n"),
        truncation_msg
    );

    let project_root_path = PathBuf::from(project_root);
    let instructions = resolve_instruction_prompts(path, &project_root_path).await;

    let mut loaded_files = vec![path_str.to_string()];

    if !instructions.is_empty() {
        let instruction_content: Vec<String> = instructions
            .iter()
            .map(|i| {
                loaded_files.push(i.filepath.clone());
                i.content.clone()
            })
            .collect();

        output.push_str("\n\n<system-reminder>\n");
        output.push_str(&instruction_content.join("\n\n"));
        output.push_str("\n</system-reminder>");
    }

    Ok(ToolResult {
        title,
        output,
        metadata: {
            let mut m = Metadata::new();
            m.insert("preview".into(), serde_json::json!(preview));
            m.insert("truncated".into(), serde_json::json!(truncated));
            m.insert("filepath".into(), serde_json::json!(path_str));
            m.insert("loaded".into(), serde_json::json!(loaded_files));
            m.insert("size".into(), serde_json::json!(content.len()));
            m.insert("total_lines".into(), serde_json::json!(total_lines));
            m
        },
        truncated,
    })
}

fn truncate_line_for_output(value: &str, max_chars: usize) -> (String, bool) {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        (format!("{truncated}..."), true)
    } else {
        (truncated, false)
    }
}

fn is_binary(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }

    let check_len = std::cmp::min(4096, content.len());
    let bytes = &content[..check_len];

    if bytes.contains(&0) {
        return true;
    }

    let non_printable = bytes
        .iter()
        .filter(|&&b| b < 9 || (b > 13 && b < 32))
        .count();

    non_printable as f32 / check_len as f32 > 0.3
}

struct InstructionPrompt {
    filepath: String,
    content: String,
}

async fn resolve_instruction_prompts(
    file_path: &Path,
    project_root: &Path,
) -> Vec<InstructionPrompt> {
    let mut results = Vec::new();

    let target = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    let mut current = target.parent().unwrap_or(&target).to_path_buf();

    while current.starts_with(&root) && current != root {
        if let Some(found) = find_instruction_file(&current).await {
            let canonical = found.canonicalize().unwrap_or_else(|_| found.clone());
            if canonical != target {
                if let Ok(content) = tokio::fs::read_to_string(&found).await {
                    if !content.is_empty() {
                        results.push(InstructionPrompt {
                            filepath: found.to_string_lossy().to_string(),
                            content: format!("Instructions from: {}\n{}", found.display(), content),
                        });
                    }
                }
            }
        }

        if !current.pop() {
            break;
        }
    }

    if let Some(found) = find_instruction_file(&root).await {
        let canonical = found.canonicalize().unwrap_or_else(|_| found.clone());
        if canonical != target {
            if let Ok(content) = tokio::fs::read_to_string(&found).await {
                if !content.is_empty() {
                    results.push(InstructionPrompt {
                        filepath: found.to_string_lossy().to_string(),
                        content: format!("Instructions from: {}\n{}", found.display(), content),
                    });
                }
            }
        }
    }

    results
}

async fn find_instruction_file(dir: &Path) -> Option<PathBuf> {
    for name in INSTRUCTION_FILES {
        let path = dir.join(name);
        if path.exists() && path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_access::clear_tool_access_tracker;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn read_rejects_empty_file_path() {
        let tool = ReadTool::new();
        let ctx = ToolContext::new(
            "session-1".to_string(),
            "message-1".to_string(),
            ".".to_string(),
        );
        let err = tool
            .execute(serde_json::json!({ "file_path": "   " }), ctx)
            .await
            .expect_err("empty file_path should be rejected");

        match err {
            ToolError::InvalidArguments(msg) => {
                assert!(msg.contains("cannot be empty"));
            }
            other => panic!("unexpected error: {}", other),
        }
    }

    #[test]
    fn binary_read_keeps_output_compact_and_moves_payload_to_metadata_attachments() {
        let path = Path::new("/tmp/sample.pdf");
        let content = vec![0u8, 1u8, 2u8, 3u8];
        let result = handle_binary_file(path, &content, "application/pdf", "sample.pdf".into())
            .expect("binary read should succeed");

        assert!(
            !result.output.contains("data:application/pdf;base64"),
            "output should not inline base64 data"
        );
        assert!(result.output.contains("PDF read successfully"));

        let attachments = result
            .metadata
            .get("attachments")
            .and_then(|v| v.as_array())
            .expect("attachments should exist");
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0].get("mime").and_then(|v| v.as_str()),
            Some("application/pdf")
        );
        assert!(
            attachments[0]
                .get("url")
                .and_then(|v| v.as_str())
                .map(|v| v.starts_with("data:application/pdf;base64,"))
                .unwrap_or(false),
            "attachment url should contain data-url"
        );
    }

    #[tokio::test]
    async fn read_file_content_truncates_multibyte_lines_without_panicking() {
        let path = Path::new("/tmp/utf8-long-line.md");
        let long_line = "教".repeat(MAX_LINE_LENGTH + 50);
        let content = long_line.into_bytes();

        let result = read_file_content(
            path,
            &path.display().to_string(),
            &content,
            1,
            10,
            "utf8-long-line.md".to_string(),
            "/tmp",
        )
        .await
        .expect("utf8 truncation should succeed");

        assert!(result.output.contains("1: "));
        assert!(result.output.contains("..."));
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn repeated_reads_warn_on_third_and_block_on_fourth() {
        let session_id = "read-tool-repeated-reads";
        clear_tool_access_tracker(session_id);
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("demo.txt");
        fs::write(&file_path, "line1\nline2\n")
            .await
            .expect("write demo file");
        let tool = ReadTool::with_directory(dir.path());

        for idx in 0..2 {
            let result = tool
                .execute(
                    serde_json::json!({ "file_path": "demo.txt" }),
                    ToolContext::new(
                        session_id.to_string(),
                        format!("message-{idx}"),
                        dir.path().display().to_string(),
                    ),
                )
                .await
                .expect("read should succeed");
            assert!(!result.output.contains("[Repeated read warning]"));
        }

        let warning = tool
            .execute(
                serde_json::json!({ "file_path": "demo.txt" }),
                ToolContext::new(
                    session_id.to_string(),
                    "message-3".to_string(),
                    dir.path().display().to_string(),
                ),
            )
            .await
            .expect("third read should warn");
        assert!(warning.output.contains("[Repeated read warning]"));
        assert_eq!(
            warning
                .metadata
                .get("toolAccessGuard")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str()),
            Some("warning")
        );

        let err = tool
            .execute(
                serde_json::json!({ "file_path": "demo.txt" }),
                ToolContext::new(
                    session_id.to_string(),
                    "message-4".to_string(),
                    dir.path().display().to_string(),
                ),
            )
            .await
            .expect_err("fourth read should be blocked");
        match err {
            ToolError::ExecutionError(message) => assert!(message.contains("BLOCKED")),
            other => panic!("unexpected error: {other}"),
        }
        clear_tool_access_tracker(session_id);
    }

    #[tokio::test]
    async fn read_auto_repairs_unique_basename_match_from_workspace_root() {
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("voicecraft/src/Game.ts");
        fs::create_dir_all(nested.parent().expect("nested parent"))
            .await
            .expect("create nested dir");
        fs::write(&nested, "export const GAME = true;\n")
            .await
            .expect("write nested file");

        let tool = ReadTool::with_directory(dir.path());
        let result = tool
            .execute(
                serde_json::json!({ "file_path": "Game.ts" }),
                ToolContext::new(
                    "session".to_string(),
                    "message".to_string(),
                    dir.path().display().to_string(),
                ),
            )
            .await
            .expect("unique basename should auto-repair");

        assert!(result.output.contains("voicecraft/src/Game.ts"));
        assert!(result.output.contains("export const GAME = true;"));
        let repair_events = crate::tool_repair_events(&result.metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("basename_auto_repair")
                && event
                    .get("to")
                    .and_then(|value| value.as_str())
                    .is_some_and(|path| path.ends_with("voicecraft/src/Game.ts"))
        }));
    }

    #[tokio::test]
    async fn read_ambiguous_basename_returns_candidate_suggestions() {
        let dir = tempdir().expect("tempdir");
        let first = dir.path().join("voicecraft/src/Game.ts");
        let second = dir.path().join("demo/src/Game.ts");
        fs::create_dir_all(first.parent().expect("first parent"))
            .await
            .expect("create first dir");
        fs::create_dir_all(second.parent().expect("second parent"))
            .await
            .expect("create second dir");
        fs::write(&first, "export const GAME = 1;\n")
            .await
            .expect("write first file");
        fs::write(&second, "export const GAME = 2;\n")
            .await
            .expect("write second file");

        let tool = ReadTool::with_directory(dir.path());
        let err = tool
            .execute(
                serde_json::json!({ "file_path": "Game.ts" }),
                ToolContext::new(
                    "session".to_string(),
                    "message".to_string(),
                    dir.path().display().to_string(),
                ),
            )
            .await
            .expect_err("ambiguous basename should return suggestions");

        let message = err.to_string();
        assert!(message.contains("Did you mean one of these?"));
        assert!(message.contains("voicecraft/src/Game.ts"));
        assert!(message.contains("demo/src/Game.ts"));
    }
}
