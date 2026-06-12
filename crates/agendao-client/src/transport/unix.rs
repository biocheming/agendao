use super::{PromptOptions, PromptResponse, SessionDetail};
use agendao_api::{AgentInfo, ExecutionModeInfo, FullProviderListResponse, SessionListItem};
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use anyhow::{Context, Result};
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
        let response: JsonRpcResponse<R> =
            serde_json::from_str(&response_line).context("Failed to parse JSON-RPC response")?;

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
            command: options.command,
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
        self.send_request("list_agents", serde_json::json!({}))
            .await
    }

    /// Subscribe to server events. When `session_id` is `Some`, the server may
    /// pre-filter by session. When `None`, the server streams the canonical
    /// frontend bus and the caller is expected to filter locally.
    pub async fn subscribe_events(
        &self,
        session_id: Option<&str>,
        tier: Option<&str>,
    ) -> Result<tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .context("Failed to connect to Unix socket for event subscription")?;

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "subscribe_events",
            params: serde_json::json!({
                "session_id": session_id,
                "tier": tier,
            }),
            id: 0,
        };
        let mut request_line = serde_json::to_string(&request)?;
        request_line.push('\n');
        stream.write_all(request_line.as_bytes()).await?;
        stream.flush().await?;

        // Read the subscribe ack response.
        let mut reader = BufReader::new(stream);
        let mut ack_line = String::new();
        reader.read_line(&mut ack_line).await?;
        let ack: JsonRpcResponse<serde_json::Value> =
            serde_json::from_str(&ack_line).context("Failed to parse subscribe_events ack")?;
        if ack.error.is_some() {
            anyhow::bail!("subscribe_events failed: {:?}", ack.error);
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                            if tx.send(value).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(rx)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{JsonRpcRequest, PromptRequest};

    #[test]
    fn prompt_request_serializes_command_scheduler_and_variant() {
        let request = PromptRequest {
            session_id: "ses_1".to_string(),
            text: "/run cargo test".to_string(),
            agent_id: Some("build".to_string()),
            scheduler_profile: Some("default".to_string()),
            model: Some("openai/gpt-5".to_string()),
            variant: Some("fast".to_string()),
            continue_last: false,
            command: Some("run".to_string()),
        };

        let value = serde_json::to_value(&request).expect("serialize unix prompt request");
        assert_eq!(value.get("command").and_then(|v| v.as_str()), Some("run"));
        assert_eq!(
            value.get("scheduler_profile").and_then(|v| v.as_str()),
            Some("default")
        );
        assert_eq!(value.get("variant").and_then(|v| v.as_str()), Some("fast"));
    }

    #[test]
    fn subscribe_events_request_serializes_tier() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "subscribe_events",
            params: serde_json::json!({
                "session_id": "ses_1",
                "tier": "tui",
            }),
            id: 0,
        };

        let value = serde_json::to_value(&request).expect("serialize subscribe_events");
        assert_eq!(value["method"], "subscribe_events");
        assert_eq!(value["params"]["session_id"], "ses_1");
        assert_eq!(value["params"]["tier"], "tui");
    }

    #[test]
    fn subscribe_events_request_allows_global_scope() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "subscribe_events",
            params: serde_json::json!({
                "session_id": serde_json::Value::Null,
                "tier": "tui",
            }),
            id: 0,
        };

        let value = serde_json::to_value(&request).expect("serialize global subscribe_events");
        assert!(value["params"]["session_id"].is_null());
        assert_eq!(value["params"]["tier"], "tui");
    }
}
