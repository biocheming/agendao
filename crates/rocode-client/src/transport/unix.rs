use super::{PromptOptions, PromptResponse, SessionDetail};
use anyhow::{Context, Result};
use rocode_api::{
    AgentInfo, ExecutionModeInfo, FullProviderListResponse, SessionListItem,
};
use rocode_runtime_context::ResolvedWorkspaceContext;
use rocode_state::RecentModelEntry;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Unix Socket transport - communicates with OrchestrationCore via Unix domain socket
///
/// Protocol: JSON-RPC over Unix socket
/// Each request/response is a single JSON line terminated by \n
pub struct UnixSocketTransport {
    socket_path: String,
}

impl UnixSocketTransport {
    pub fn new(socket_path: String) -> Self {
        Self { socket_path }
    }

    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: T,
    ) -> Result<R> {
        // Connect to Unix socket
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .context("Failed to connect to Unix socket")?;

        // Build JSON-RPC request
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };

        // Serialize and send
        let request_json = serde_json::to_string(&request)?;
        stream.write_all(request_json.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        // Read response
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;

        // Parse JSON-RPC response
        let response: JsonRpcResponse<R> = serde_json::from_str(&response_line)
            .context("Failed to parse JSON-RPC response")?;

        if let Some(error) = response.error {
            anyhow::bail!("RPC error {}: {}", error.code, error.message);
        }

        response
            .result
            .ok_or_else(|| anyhow::anyhow!("Missing result in response"))
    }

    pub async fn prompt(
        &self,
        session_id: &str,
        text: &str,
        options: PromptOptions,
    ) -> Result<PromptResponse> {
        let params = PromptRequest {
            session_id: session_id.to_string(),
            text: text.to_string(),
            agent_id: options.agent_id,
            scheduler_profile: options.scheduler_profile,
            model: options.model,
            variant: options.variant,
            continue_last: options.continue_last,
        };

        self.send_request("prompt", params).await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionListItem>> {
        self.send_request("list_sessions", serde_json::json!({}))
            .await
    }

    pub async fn get_recent_models(&self) -> Result<Vec<RecentModelEntry>> {
        self.send_request("get_recent_models", serde_json::json!({}))
            .await
    }

    pub async fn get_workspace_context(&self) -> Result<ResolvedWorkspaceContext> {
        self.send_request("get_workspace_context", serde_json::json!({}))
            .await
    }

    pub async fn put_recent_models(
        &self,
        recent_models: &[RecentModelEntry],
    ) -> Result<Vec<RecentModelEntry>> {
        self.send_request(
            "put_recent_models",
            serde_json::json!({ "recent_models": recent_models }),
        )
        .await
    }

    pub async fn get_all_providers(&self) -> Result<FullProviderListResponse> {
        self.send_request("get_all_providers", serde_json::json!({}))
            .await
    }

    pub async fn list_execution_modes(&self) -> Result<Vec<ExecutionModeInfo>> {
        self.send_request("list_execution_modes", serde_json::json!({}))
            .await
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        self.send_request("list_agents", serde_json::json!({})).await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        let params = serde_json::json!({ "session_id": session_id });
        self.send_request("get_session", params).await
    }
}

// ============================================================================
// JSON-RPC Protocol Types
// ============================================================================

#[derive(Debug, Serialize)]
struct JsonRpcRequest<T> {
    jsonrpc: &'static str,
    method: &'static str,
    params: T,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    #[serde(rename = "jsonrpc")]
    _jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    #[serde(rename = "id")]
    _id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Serialize)]
struct PromptRequest {
    session_id: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scheduler_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    continue_last: bool,
}
