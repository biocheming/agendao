use async_trait::async_trait;
use rocode_mcp::{McpClientRegistry, McpTool};
use rocode_tool::{Tool, ToolContext, ToolError, ToolResult, ToolSchemaSourceKind};
use std::sync::Arc;

/// A bridge tool that wraps an MCP tool and makes it executable through the
/// standard `ToolRegistry`. When the LLM calls an MCP tool, this bridge
/// delegates to `McpClient::call_tool()` on the appropriate server.
pub struct McpBridgeTool {
    tool: McpTool,
    clients: Arc<McpClientRegistry>,
}

impl McpBridgeTool {
    pub fn new(tool: McpTool, clients: Arc<McpClientRegistry>) -> Self {
        Self { tool, clients }
    }
}

#[async_trait]
impl Tool for McpBridgeTool {
    fn id(&self) -> &str {
        &self.tool.full_name
    }

    fn description(&self) -> &str {
        self.tool.description.as_deref().unwrap_or("MCP tool")
    }

    fn parameters(&self) -> serde_json::Value {
        self.tool.input_schema.clone()
    }

    fn source_kind(&self) -> ToolSchemaSourceKind {
        ToolSchemaSourceKind::Mcp
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let client = self
            .clients
            .get(&self.tool.server_name)
            .await
            .ok_or_else(|| {
                ToolError::ExecutionError(format!(
                    "MCP server '{}' is not connected",
                    self.tool.server_name
                ))
            })?;

        let result = client
            .call_tool(&self.tool.name, Some(args))
            .await
            .map_err(|e| ToolError::ExecutionError(format!("MCP call_tool failed: {}", e)))?;

        if result.is_error == Some(true) {
            let error_text = result
                .content
                .iter()
                .filter_map(|block| block.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(ToolError::ExecutionError(error_text));
        }

        let output = result
            .content
            .iter()
            .filter_map(|block| block.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult::simple(
            format!("MCP: {}", self.tool.full_name),
            output,
        ))
    }
}

/// Register all MCP tools from the `McpClientRegistry` into the main
/// `ToolRegistry` as executable bridge tools.
pub async fn register_mcp_tools(
    tool_registry: &rocode_tool::ToolRegistry,
    mcp_clients: &Arc<McpClientRegistry>,
) {
    let mcp_tool_registry = mcp_clients.tool_registry();
    let mcp_tools = mcp_tool_registry.list().await;

    for mcp_tool in mcp_tools {
        let bridge = McpBridgeTool::new(mcp_tool, mcp_clients.clone());
        tool_registry.register(bridge).await;
    }
}
