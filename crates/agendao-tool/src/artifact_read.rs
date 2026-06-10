use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::read::ReadTool;
use crate::{Tool, ToolContext, ToolError, ToolResult};

pub struct ArtifactReadTool {
    reader: ReadTool,
}

#[derive(Debug, Deserialize)]
struct ArtifactReadInput {
    artifact_path: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
}

impl ArtifactReadTool {
    pub fn new() -> Self {
        Self {
            reader: ReadTool::new(),
        }
    }

    fn is_governed_artifact_path(path: &Path) -> bool {
        path.components().any(|component| component.as_os_str() == "session-artifacts")
            && path
                .components()
                .any(|component| component.as_os_str() == "tool-results")
    }

    fn normalize_artifact_path(&self, artifact_path: &str, ctx: &ToolContext) -> Result<PathBuf, ToolError> {
        let trimmed = artifact_path.trim();
        if trimmed.is_empty() {
            return Err(ToolError::InvalidArguments(
                "artifact_path cannot be empty".to_string(),
            ));
        }

        let path = PathBuf::from(trimmed);
        let resolved = if path.is_absolute() {
            path
        } else {
            Path::new(&ctx.directory).join(path)
        };

        if !Self::is_governed_artifact_path(&resolved) {
            return Err(ToolError::InvalidArguments(
                "artifact_path must point to a governed tool-result artifact under .agendao/session-artifacts/.../tool-results/".to_string(),
            ));
        }

        Ok(resolved)
    }
}

impl Default for ArtifactReadTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ArtifactReadTool {
    fn id(&self) -> &str {
        "artifact_read"
    }

    fn description(&self) -> &str {
        "Read a governed local tool-result artifact produced after large output truncation. Use this for `artifact:` paths from '[tool result governed: output too large]'. Do not pass artifact paths to webfetch or browser_session."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "artifact_path": {
                    "type": "string",
                    "description": "Absolute or session-relative path copied from a governed tool result `artifact:` line."
                },
                "offset": {
                    "type": "integer",
                    "description": "Optional 1-indexed line offset for partial reads."
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional max line count for partial reads."
                }
            },
            "required": ["artifact_path"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: ArtifactReadInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let artifact_path = self.normalize_artifact_path(&input.artifact_path, &ctx)?;

        let mut delegated_args = serde_json::json!({
            "file_path": artifact_path.to_string_lossy().to_string()
        });
        if let Some(offset) = input.offset {
            delegated_args["offset"] = serde_json::json!(offset);
        }
        if let Some(limit) = input.limit {
            delegated_args["limit"] = serde_json::json!(limit);
        }

        let mut result = self.reader.execute(delegated_args, ctx).await?;
        result.title = format!("Artifact read: {}", result.title);
        result
            .metadata
            .insert("artifact_read".to_string(), serde_json::json!(true));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn artifact_read_rejects_non_governed_paths() {
        let tool = ArtifactReadTool::new();
        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into());
        let err = tool
            .normalize_artifact_path("/tmp/random.txt", &ctx)
            .expect_err("should reject arbitrary paths");
        assert!(err
            .to_string()
            .contains("artifact_path must point to a governed tool-result artifact"));
    }

    #[tokio::test]
    async fn artifact_read_delegates_to_local_reader_for_governed_artifacts() {
        let temp = tempdir().unwrap();
        let artifact = temp
            .path()
            .join(".agendao/session-artifacts/ses-1/tool-results/result.txt");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, "line1\nline2\n").unwrap();

        let tool = ArtifactReadTool {
            reader: ReadTool::with_directory(temp.path()),
        };
        let ctx = ToolContext::new(
            "session-1".into(),
            "message-1".into(),
            temp.path().to_string_lossy().to_string(),
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "artifact_path": artifact.to_string_lossy().to_string()
                }),
                ctx,
            )
            .await
            .expect("artifact read should succeed");

        assert!(result.title.contains("Artifact read:"));
        assert!(result.output.contains("line1"));
        assert_eq!(
            result.metadata.get("artifact_read"),
            Some(&serde_json::json!(true))
        );
    }
}
