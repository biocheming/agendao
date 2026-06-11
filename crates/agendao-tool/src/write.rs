use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::path_guard::{resolve_user_path, RootPathFallbackPolicy};
use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

#[cfg(feature = "lsp")]
const MAX_DIAGNOSTICS_PER_FILE: usize = 20;
#[cfg(feature = "lsp")]
const MAX_PROJECT_DIAGNOSTICS_FILES: usize = 5;

pub struct WriteTool {
    directory: PathBuf,
}

impl WriteTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }
}

impl Default for WriteTool {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_write_file_path(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("file_path")
        .or_else(|| args.get("filePath"))
        .or_else(|| args.get("filepath"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidArguments("file_path (or filePath/filepath) is required".into())
        })
        .map(|value| value.trim().to_string())
}

#[async_trait]
impl Tool for WriteTool {
    fn id(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Writes content to a file on the local filesystem. Creates the file if it doesn't exist, overwrites if it does."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path or project-relative path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let file_path = extract_write_file_path(&args)?;

        let content: String = args["content"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("content is required".into()))?
            .to_string();

        let base_dir = if ctx.directory.is_empty() {
            &self.directory
        } else {
            Path::new(&ctx.directory)
        };

        let resolved = resolve_user_path(
            &file_path,
            base_dir,
            RootPathFallbackPolicy::PreferSessionDirWhenMissing,
        );
        let path = resolved.resolved;
        if let Some(original) = resolved.corrected_from {
            tracing::warn!(
                from = %original.display(),
                to = %path.display(),
                session_dir = %base_dir.display(),
                "corrected suspicious root-level write path into session directory"
            );
        }

        let path_str = path.to_string_lossy().to_string();

        if ctx.is_external_path(&path_str) {
            let parent = path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| path_str.clone());

            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(format!("{}/*", parent))
                    .with_scope_key(crate::external_fs_scope_key(&parent))
                    .with_metadata("filepath", serde_json::json!(&path_str))
                    .with_metadata("parentDir", serde_json::json!(parent)),
            )
            .await?;
        }

        let exists = match fs::metadata(&path).await {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(ToolError::ExecutionError(format!(
                    "Failed to inspect existing file: {}",
                    error
                )));
            }
        };

        let old_content = if exists {
            fs::read_to_string(&path).await.map_err(|error| {
                ToolError::ExecutionError(format!("Failed to read existing file: {}", error))
            })?
        } else {
            String::new()
        };

        if exists {
            ctx.do_file_time_assert(path_str.clone()).await?;
        }

        let diff = create_diff(&path_str, &old_content, &content);

        ctx.ask_permission(
            crate::PermissionRequest::new("edit")
                .with_pattern(&path_str)
                .with_scope_key(crate::workspace_scope_key(&ctx.project_root, &path_str))
                .with_metadata("diff", serde_json::json!(diff))
                .always_allow(),
        )
        .await?;

        let title = path
            .strip_prefix(&ctx.worktree)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionError(format!("Failed to create directory: {}", e))
            })?;
        }

        fs::write(&path, &content)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to write file: {}", e)))?;

        ctx.do_publish_bus(
            "file.edited",
            serde_json::json!({
                "file": path_str
            }),
        )
        .await;

        ctx.do_publish_bus(
            "file_watcher.updated",
            serde_json::json!({
                "file": path_str,
                "event": if exists { "change" } else { "add" }
            }),
        )
        .await;

        ctx.do_lsp_touch_file(path_str.clone(), true).await?;
        ctx.do_file_time_read(path_str.clone()).await?;

        let line_count = content.lines().count();
        let byte_count = content.len();

        let diagnostics_output = get_lsp_diagnostics(&path, &ctx).await;

        let output = if diagnostics_output.is_empty() {
            format!(
                "Successfully wrote {} bytes ({} lines) to {}",
                byte_count,
                line_count,
                path.display()
            )
        } else {
            format!(
                "Successfully wrote {} bytes ({} lines) to {}\n\n{}",
                byte_count,
                line_count,
                path.display(),
                diagnostics_output
            )
        };

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("bytes".into(), serde_json::json!(byte_count));
                m.insert("lines".into(), serde_json::json!(line_count));
                m.insert("filepath".into(), serde_json::json!(path_str));
                m.insert("exists".into(), serde_json::json!(exists));
                m.insert("diff".into(), serde_json::json!(diff));
                m
            },
            truncated: false,
        })
    }
}

fn create_diff(filepath: &str, old_content: &str, new_content: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut diff = format!("--- {}\n+++ {}\n", filepath, filepath);

    let mut old_idx = 0;
    let mut new_idx = 0;

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

async fn get_lsp_diagnostics(path: &Path, ctx: &ToolContext) -> String {
    #[cfg(feature = "lsp")]
    {
        use agendao_lsp::detect_language;

        if let Some(lsp_registry) = &ctx.lsp_registry {
            return get_lsp_diagnostics_impl(path, lsp_registry.clone()).await;
        }
    }

    #[cfg(not(feature = "lsp"))]
    {
        let _ = (path, ctx);
    }

    String::new()
}

#[cfg(feature = "lsp")]
async fn get_lsp_diagnostics_impl(
    path: &Path,
    lsp_registry: Arc<agendao_lsp::LspClientRegistry>,
) -> String {
    use agendao_lsp::detect_language;
    use std::collections::HashMap;

    let language = detect_language(path);
    let clients = lsp_registry.list().await;

    let client = clients
        .iter()
        .find(|(id, _)| id.contains(language))
        .map(|(_, c)| c.clone());

    let Some(client) = client else {
        return String::new();
    };

    // Open/refresh the written file so the LSP re-publishes diagnostics
    if let Ok(content) = tokio::fs::read_to_string(path).await {
        if let Err(error) = client.open_document(path, &content, language).await {
            tracing::debug!(
                path = %path.display(),
                error = %error,
                "Failed to sync written file to LSP before diagnostics"
            );
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Collect all diagnostics from all files the LSP knows about
    let all_diagnostics = client.get_all_diagnostics().await;

    let normalized_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut output = String::new();
    let mut project_diagnostics_count = 0;

    // First pass: the written file itself
    for (file, diagnostics) in &all_diagnostics {
        let file_canonical = file.canonicalize().unwrap_or_else(|_| file.clone());
        if file_canonical != normalized_path {
            continue;
        }

        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == Some(lsp_types::DiagnosticSeverity::ERROR))
            .collect();

        if errors.is_empty() {
            continue;
        }

        let total = errors.len();
        let limited: Vec<_> = errors.into_iter().take(MAX_DIAGNOSTICS_PER_FILE).collect();
        let suffix = if limited.len() < total {
            format!("\n... and {} more", total - limited.len())
        } else {
            String::new()
        };

        let lines: Vec<String> = limited
            .iter()
            .map(|d| {
                let line = d.range.start.line + 1;
                format!("  Line {}: {}", line, d.message)
            })
            .collect();

        output.push_str(&format!(
            "LSP errors detected in this file, please fix:\n<diagnostics file=\"{}\">\n{}{}\n</diagnostics>",
            path.display(),
            lines.join("\n"),
            suffix
        ));
    }

    // Second pass: up to MAX_PROJECT_DIAGNOSTICS_FILES other files
    for (file, diagnostics) in &all_diagnostics {
        if project_diagnostics_count >= MAX_PROJECT_DIAGNOSTICS_FILES {
            break;
        }

        let file_canonical = file.canonicalize().unwrap_or_else(|_| file.clone());
        if file_canonical == normalized_path {
            continue;
        }

        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == Some(lsp_types::DiagnosticSeverity::ERROR))
            .collect();

        if errors.is_empty() {
            continue;
        }

        project_diagnostics_count += 1;

        let total = errors.len();
        let limited: Vec<_> = errors.into_iter().take(MAX_DIAGNOSTICS_PER_FILE).collect();
        let suffix = if limited.len() < total {
            format!("\n... and {} more", total - limited.len())
        } else {
            String::new()
        };

        let lines: Vec<String> = limited
            .iter()
            .map(|d| {
                let line = d.range.start.line + 1;
                format!("  Line {}: {}", line, d.message)
            })
            .collect();

        output.push_str(&format!(
            "\n\nLSP errors detected in other files:\n<diagnostics file=\"{}\">\n{}{}\n</diagnostics>",
            file.display(),
            lines.join("\n"),
            suffix
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::extract_write_file_path;

    #[test]
    fn extract_write_file_path_accepts_filepath_alias() {
        let args = serde_json::json!({
            "filepath": "src/main.ts",
            "content": "hello"
        });
        let path = extract_write_file_path(&args).expect("filepath alias should be accepted");
        assert_eq!(path, "src/main.ts");
    }
}
