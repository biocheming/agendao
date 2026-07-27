use async_trait::async_trait;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

pub struct GlobTool {
    directory: PathBuf,
}

impl GlobTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a glob pattern has no recursive `**` component,
/// meaning it should only match in the immediate directory.
fn is_shallow_pattern(pattern: &str) -> bool {
    !pattern.contains("**") && !pattern.contains('/')
}

#[async_trait]
impl Tool for GlobTool {
    fn id(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Fast file pattern matching tool. Supports glob patterns like '**/*.js' or 'src/**/*.ts'. Returns files sorted by modification time (most recent first)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. Defaults to current directory."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let pattern: String = args["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("pattern is required".into()))?
            .to_string();

        let search_path: String = args["path"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| ctx.directory.clone());

        let base_dir = if search_path.is_empty() {
            &self.directory
        } else {
            Path::new(&search_path)
        };

        let base_dir_str = base_dir.to_string_lossy().to_string();

        if ctx.is_external_path(&base_dir_str) {
            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(format!("{}/*", base_dir_str))
                    .with_scope_key(crate::external_fs_scope_key(&base_dir_str))
                    .with_metadata("path", serde_json::json!(&base_dir_str)),
            )
            .await?;
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("glob")
                .with_pattern(&pattern)
                .with_scope_key(crate::workspace_scope_key(&ctx.project_root, &base_dir_str))
                .with_metadata("path", serde_json::json!(&base_dir_str))
                .always_allow(),
        )
        .await?;

        // Validate the glob pattern early.
        let glob_pattern = glob::Pattern::new(&pattern)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid glob pattern: {}", e)))?;

        let shallow = is_shallow_pattern(&pattern);

        // Post-filter against the full glob pattern on relative paths.
        //
        // The walk is synchronous and can take seconds on large trees, so run
        // it on a blocking thread instead of the async worker. Results are
        // sorted by mtime, which requires collecting all matches first; the
        // main win is that WalkBuilder skips hidden/ignored directories
        // (.git, target, node_modules) during the traversal.
        let scan_base_dir = base_dir.to_path_buf();
        let scan = tokio::task::spawn_blocking(move || {
            let mut files_with_mtime: Vec<(String, SystemTime)> = Vec::new();
            let mut walker = WalkBuilder::new(&scan_base_dir);
            walker.hidden(true).git_ignore(true).follow_links(true);
            if shallow {
                walker.max_depth(Some(1));
            }
            for entry in walker.build().filter_map(|e| e.ok()) {
                let abs_path = entry.path();
                let is_file = entry
                    .file_type()
                    .map(|ft| ft.is_file())
                    .unwrap_or_else(|| abs_path.is_file());
                if !is_file {
                    continue;
                }
                let rel_path = abs_path.strip_prefix(&scan_base_dir).unwrap_or(abs_path);
                let rel_str = rel_path.to_string_lossy();
                if glob_pattern.matches(&rel_str) {
                    let mtime = entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    files_with_mtime.push((abs_path.to_string_lossy().to_string(), mtime));
                }
            }
            files_with_mtime
        });

        let mut files_with_mtime = scan
            .await
            .map_err(|e| ToolError::ExecutionError(format!("glob scan failed: {}", e)))?;

        files_with_mtime.sort_by(|a, b| b.1.cmp(&a.1));

        let total = files_with_mtime.len();
        let truncated = total > 100;
        let matches: Vec<&str> = files_with_mtime
            .iter()
            .take(100)
            .map(|(p, _)| p.as_str())
            .collect();

        let title = format!("glob '{}'", pattern);
        let output = if matches.is_empty() {
            format!("No files matching pattern '{}' found", pattern)
        } else {
            let mut result = matches.join("\n");
            if truncated {
                result.push_str(&format!("\n\n(Results are truncated: showing first 100 of {}. Consider using a more specific path or pattern.)", total));
            } else {
                result.push_str(&format!("\n\n({} files)", total));
            }
            result
        };

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("count".into(), serde_json::json!(total));
                m.insert("truncated".into(), serde_json::json!(truncated));
                m
            },
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn glob_skips_gitignored_and_hidden_files() {
        let dir = tempdir().expect("tempdir");
        // An (empty) `.git` directory marks the tempdir as a git repo so
        // WalkBuilder applies .gitignore rules.
        std::fs::create_dir(dir.path().join(".git")).expect("create .git");
        std::fs::write(dir.path().join(".gitignore"), "target/\nnode_modules/\n")
            .expect("write .gitignore");
        std::fs::create_dir_all(dir.path().join("target")).expect("create target");
        std::fs::create_dir_all(dir.path().join("node_modules")).expect("create node_modules");
        std::fs::create_dir_all(dir.path().join("src")).expect("create src");
        std::fs::write(dir.path().join("target/ignored.txt"), "x").expect("write target file");
        std::fs::write(dir.path().join("node_modules/ignored.txt"), "x")
            .expect("write node_modules file");
        std::fs::write(dir.path().join("src/kept.txt"), "x").expect("write kept file");
        std::fs::write(dir.path().join("src/.hidden.txt"), "x").expect("write hidden file");

        let tool = GlobTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "**/*.txt",
                    "path": dir.path().display().to_string()
                }),
                ToolContext::new(
                    "glob-tool-ignore-rules".to_string(),
                    "message-1".to_string(),
                    dir.path().display().to_string(),
                ),
            )
            .await
            .expect("glob should succeed");

        assert!(
            result.output.contains("kept.txt"),
            "kept.txt should be listed: {}",
            result.output
        );
        assert!(
            !result.output.contains("ignored.txt"),
            "gitignored files should be skipped: {}",
            result.output
        );
        assert!(
            !result.output.contains("hidden"),
            "hidden files should be skipped: {}",
            result.output
        );
        assert_eq!(
            result.metadata.get("count").and_then(|v| v.as_u64()),
            Some(1),
            "only src/kept.txt should match"
        );
        assert!(!result.truncated);
    }
}
