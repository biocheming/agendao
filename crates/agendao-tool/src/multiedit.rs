use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

use crate::path_guard::{authorize_external_file_path, resolve_user_path, RootPathFallbackPolicy};
use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

pub struct MultiEditTool;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiEditInput {
    edits: Vec<FileEdit>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEdit {
    file_path: String,
    edits: Vec<EditOperation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditOperation {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for MultiEditTool {
    fn id(&self) -> &str {
        "multiedit"
    }

    fn description(&self) -> &str {
        "Apply multiple string replacements across multiple files in a single atomic operation. Each file can have multiple edits applied in sequence."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "The path to the file to edit"
                            },
                            "edits": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "old_string": {
                                            "type": "string",
                                            "description": "The text to search for"
                                        },
                                        "new_string": {
                                            "type": "string",
                                            "description": "The text to replace it with"
                                        },
                                        "replace_all": {
                                            "type": "boolean",
                                            "default": false,
                                            "description": "Replace all occurrences"
                                        }
                                    },
                                    "required": ["old_string", "new_string"]
                                },
                                "description": "List of edits to apply to this file"
                            }
                        },
                        "required": ["file_path", "edits"]
                    },
                    "description": "List of files with their edits"
                }
            },
            "required": ["edits"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: MultiEditInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let fallback_base_dir = std::env::current_dir().unwrap_or_default();
        let base_path = if ctx.directory.is_empty() {
            fallback_base_dir.as_path()
        } else {
            Path::new(&ctx.directory)
        };
        let mut results: Vec<String> = Vec::new();
        let mut total_edits = 0;
        let mut total_files = 0;
        let mut metadata = Metadata::new();

        for file_edit in input.edits {
            let resolved = resolve_user_path(
                &file_edit.file_path,
                base_path,
                RootPathFallbackPolicy::ExistingFallbackOnly,
            );
            let requested_path = resolved.resolved;
            if let Some(original) = resolved.corrected_from {
                tracing::warn!(
                    from = %original.display(),
                    to = %requested_path.display(),
                    session_dir = %base_path.display(),
                    "corrected suspicious root-level multi-edit path into session directory"
                );
            }
            let authorized_path = ctx.resolve_existing_file_path(&requested_path)?;
            authorize_external_file_path(&ctx, &authorized_path).await?;
            let file_path = authorized_path.operation_path().to_path_buf();
            let file_path_str = authorized_path.display_path();

            let content =
                fs::read_to_string(&file_path)
                    .await
                    .map_err(|error| match error.kind() {
                        std::io::ErrorKind::NotFound => ToolError::FileNotFound(format!(
                            "File not found: {}",
                            file_path.display()
                        )),
                        _ => ToolError::ExecutionError(format!("Failed to read file: {}", error)),
                    })?;

            let mut new_content = content.clone();
            let mut file_edits = 0;

            for edit in file_edit.edits {
                let count = if edit.replace_all {
                    let count = new_content.matches(&edit.old_string).count();
                    new_content = new_content.replace(&edit.old_string, &edit.new_string);
                    count
                } else {
                    if !new_content.contains(&edit.old_string) {
                        return Err(ToolError::ExecutionError(format!(
                            "Could not find '{}' in file {}",
                            edit.old_string, file_edit.file_path
                        )));
                    }
                    if let Some(pos) = new_content.find(&edit.old_string) {
                        let before = &new_content[..pos];
                        let after = &new_content[pos + edit.old_string.len()..];
                        new_content = format!("{}{}{}", before, edit.new_string, after);
                        1
                    } else {
                        0
                    }
                };
                file_edits += count;
            }

            if file_edits > 0 {
                ctx.do_file_time_assert(file_path_str.clone()).await?;

                let diff = create_diff(&file_path_str, &content, &new_content);
                ctx.ask_permission(
                    PermissionRequest::new("edit")
                        .with_pattern(&file_path_str)
                        .with_scope_key(authorized_path.permission_scope_key())
                        .with_metadata("diff", serde_json::json!(diff))
                        .always_allow(),
                )
                .await?;

                let revalidated = ctx.resolve_existing_file_path(&file_path)?;
                if revalidated != authorized_path {
                    return Err(ToolError::ExecutionError(format!(
                        "multi-edit target changed after authorization: {}",
                        file_path.display()
                    )));
                }
                fs::write(&file_path, &new_content).await.map_err(|error| {
                    ToolError::ExecutionError(format!("Failed to write file: {}", error))
                })?;

                ctx.do_publish_bus(
                    "file.edited",
                    serde_json::json!({
                        "file": file_path_str
                    }),
                )
                .await;

                ctx.do_publish_bus(
                    "file_watcher.updated",
                    serde_json::json!({
                        "file": file_path_str,
                        "event": "change"
                    }),
                )
                .await;

                ctx.do_lsp_touch_file(file_path_str.clone(), true).await?;
                ctx.do_file_time_read(file_path_str.clone()).await?;

                let title = file_path
                    .strip_prefix(&ctx.worktree)
                    .unwrap_or(&file_path)
                    .to_string_lossy()
                    .to_string();
                results.push(format!("- {}: {} edit(s)", title, file_edits));
                metadata.insert(
                    title,
                    serde_json::json!({
                        "filepath": file_path_str,
                        "edits": file_edits,
                    }),
                );
                total_edits += file_edits;
                total_files += 1;
            }
        }

        let output = if results.is_empty() {
            "No edits applied.".to_string()
        } else {
            format!(
                "Applied {} edit(s) across {} file(s):\n{}",
                total_edits,
                total_files,
                results.join("\n")
            )
        };

        Ok(ToolResult {
            title: format!("Multi-edit: {} edits in {} files", total_edits, total_files),
            output,
            metadata,
            truncated: false,
        })
    }
}

impl Default for MultiEditTool {
    fn default() -> Self {
        Self
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

#[cfg(test)]
mod tests {
    use super::MultiEditInput;

    #[test]
    fn multiedit_rejects_filepath_alias() {
        let error = serde_json::from_value::<MultiEditInput>(serde_json::json!({
            "edits": [
                {
                    "filepath": "src/main.rs",
                    "edits": [
                        {
                            "old_string": "a",
                            "new_string": "b"
                        }
                    ]
                }
            ]
        }))
        .expect_err("filepath alias should be rejected");
        assert!(error.to_string().contains("unknown field"));
    }
}
