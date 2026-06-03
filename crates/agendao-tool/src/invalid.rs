use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidParams {
    #[serde(alias = "tool_name")]
    #[serde(alias = "toolName")]
    pub tool: String,
    #[serde(alias = "error_message")]
    #[serde(alias = "errorMessage")]
    pub error: String,
    #[serde(alias = "receivedArgs")]
    pub received_args: Option<serde_json::Value>,
}

pub struct InvalidTool;

#[async_trait]
impl Tool for InvalidTool {
    fn id(&self) -> &str {
        "invalid"
    }

    fn description(&self) -> &str {
        "Do not use"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "The invalid or unknown tool name"
                },
                "error": {
                    "type": "string",
                    "description": "Description of why the tool call is invalid"
                },
            },
            "required": ["tool", "error"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let params: InvalidParams = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid parameters: {}", e)))?;

        let output = format!(
            "The arguments provided to the tool are invalid: {}",
            params.error
        );

        let mut metadata = Metadata::new();
        metadata.insert("tool_name".to_string(), serde_json::json!(params.tool));
        metadata.insert("error_message".to_string(), serde_json::json!(params.error));

        Ok(ToolResult {
            output,
            title: "Invalid Tool".to_string(),
            metadata,
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_tool_accepts_ts_shape() {
        let tool = InvalidTool;
        let ctx = ToolContext::new("s".to_string(), "m".to_string(), ".".to_string());
        let out = tool
            .execute(
                serde_json::json!({
                    "tool": "read_html",
                    "error": "unknown tool"
                }),
                ctx,
            )
            .await
            .expect("invalid tool should accept ts shape");
        assert_eq!(out.title, "Invalid Tool");
        assert!(out.output.contains("unknown tool"));
    }

    #[tokio::test]
    async fn invalid_tool_accepts_legacy_shape() {
        let tool = InvalidTool;
        let ctx = ToolContext::new("s".to_string(), "m".to_string(), ".".to_string());
        let out = tool
            .execute(
                serde_json::json!({
                    "toolName": "read_html",
                    "errorMessage": "unknown tool"
                }),
                ctx,
            )
            .await
            .expect("invalid tool should accept legacy shape");
        assert_eq!(out.title, "Invalid Tool");
        assert!(out.output.contains("unknown tool"));
    }
}
