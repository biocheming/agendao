use super::{PromptOptions, PromptResponse, SessionDetail};
use agendao_api::{
    AgentInfo, ApiTodoItem, CompactResponse, ConnectProviderRequest, CreateSessionRequest,
    DisabledConfigUpdate, ExecutionModeInfo, FullProviderListResponse, McpAuthStartInfo,
    McpStatusInfo, MessageInfo, PermissionRequestInfo, PluginListEntry, ProviderDescriptorResponse,
    QuestionInfo, RecoveryActionKind, RefreshProviderCatalogResponse, SessionInfo, SessionListItem,
    SessionRecoveryProtocol, SessionRuntimeState, SkillCatalogEntry, SkillCatalogQuery,
    SkillDetailQuery, SkillDetailResponse, SkillEvolutionProposal, SkillManageRequest,
    SkillManageResponse, TestProviderConnectionResponse, ToolListEntry,
};
use agendao_config::Config;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use agendao_types::task_ledger::{SessionTaskLedger, SessionTaskLedgerView, TaskLedgerOp};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Unix Socket transport for the server's JSON-RPC interface.
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
            scheduler: options.scheduler,
            model: options.model,
            variant: options.variant,
            reasoning_effort: options.reasoning_effort,
            continue_last: options.continue_last,
            command: options.command,
        };

        self.send_request("prompt", params).await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionListItem>> {
        self.list_sessions_filtered(None, None, Some(100)).await
    }

    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionInfo> {
        self.send_request("create_session", request).await
    }

    pub async fn list_sessions_filtered(
        &self,
        directory: Option<&str>,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SessionListItem>> {
        self.send_request(
            "list_sessions",
            serde_json::json!({
                "directory": directory,
                "search": search,
                "limit": limit,
            }),
        )
        .await
    }

    pub async fn list_messages(&self, session_id: &str) -> Result<Vec<MessageInfo>> {
        self.send_request(
            "list_messages",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn get_session_info(&self, session_id: &str) -> Result<SessionInfo> {
        self.send_request(
            "get_session_info",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn get_session_runtime(&self, session_id: &str) -> Result<SessionRuntimeState> {
        self.send_request(
            "get_session_runtime",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn submit_input(
        &self,
        session_id: &str,
        command: &agendao_types::submission::SubmitInputCommand,
    ) -> Result<agendao_types::submission::SubmissionDisposition> {
        self.send_request(
            "submit_input",
            serde_json::json!({ "session_id": session_id, "command": command }),
        )
        .await
    }

    pub async fn interrupt(
        &self,
        session_id: &str,
        command: &agendao_types::submission::InterruptCommand,
    ) -> Result<agendao_types::submission::InterruptDisposition> {
        self.send_request(
            "interrupt",
            serde_json::json!({ "session_id": session_id, "command": command }),
        )
        .await
    }

    pub async fn delete_queued_input(
        &self,
        session_id: &str,
        item_id: &str,
        request: &agendao_types::submission::QueueMutationRequest,
    ) -> Result<agendao_types::submission::QueueMutationDisposition> {
        self.send_request(
            "delete_queued_input",
            serde_json::json!({ "session_id": session_id, "item_id": item_id, "request": request }),
        )
        .await
    }

    pub async fn edit_queued_input(
        &self,
        session_id: &str,
        item_id: &str,
        request: &agendao_types::submission::QueueEditRequest,
    ) -> Result<agendao_types::submission::QueueMutationDisposition> {
        self.send_request(
            "edit_queued_input",
            serde_json::json!({ "session_id": session_id, "item_id": item_id, "request": request }),
        )
        .await
    }

    pub async fn reorder_queued_input(
        &self,
        session_id: &str,
        item_id: &str,
        request: &agendao_types::submission::QueueReorderRequest,
    ) -> Result<agendao_types::submission::QueueMutationDisposition> {
        self.send_request(
            "reorder_queued_input",
            serde_json::json!({ "session_id": session_id, "item_id": item_id, "request": request }),
        )
        .await
    }

    pub async fn get_task_ledger(&self, session_id: &str) -> Result<SessionTaskLedger> {
        self.get_task_ledger_view(session_id)
            .await
            .map(|view| view.ledger)
    }

    pub async fn get_task_ledger_view(&self, session_id: &str) -> Result<SessionTaskLedgerView> {
        self.send_request(
            "get_task_ledger",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn apply_task_ledger_op(
        &self,
        session_id: &str,
        expected_revision: u64,
        op: TaskLedgerOp,
    ) -> Result<SessionTaskLedger> {
        self.apply_task_ledger_op_view(session_id, expected_revision, op)
            .await
            .map(|view| view.ledger)
    }

    pub async fn apply_task_ledger_op_view(
        &self,
        session_id: &str,
        expected_revision: u64,
        op: TaskLedgerOp,
    ) -> Result<SessionTaskLedgerView> {
        #[derive(Deserialize)]
        struct WriteResult {
            ledger: SessionTaskLedgerView,
        }

        let result: WriteResult = self
            .send_request(
                "apply_task_ledger_op",
                serde_json::json!({
                    "session_id": session_id,
                    "expected_revision": expected_revision,
                    "op": op,
                }),
            )
            .await?;
        Ok(result.ledger)
    }

    pub async fn get_session_todos(&self, session_id: &str) -> Result<Vec<ApiTodoItem>> {
        self.send_request(
            "get_session_todos",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn get_config(&self) -> Result<Config> {
        self.send_request("get_config", serde_json::json!({})).await
    }

    pub async fn patch_config(&self, patch: &serde_json::Value) -> Result<Config> {
        self.send_request("patch_config", patch).await
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

    pub async fn get_provider_descriptor(
        &self,
        provider_id: &str,
    ) -> Result<ProviderDescriptorResponse> {
        self.send_request(
            "get_provider_descriptor",
            serde_json::json!({ "provider_id": provider_id }),
        )
        .await
    }

    pub async fn connect_provider(&self, request: &ConnectProviderRequest) -> Result<()> {
        let _: bool = self.send_request("connect_provider", request).await?;
        Ok(())
    }

    pub async fn update_provider(
        &self,
        provider_id: &str,
        name: Option<&str>,
        base_url: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<bool> {
        self.send_request(
            "update_provider",
            serde_json::json!({
                "provider_id": provider_id,
                "name": name,
                "base_url": base_url,
                "protocol": protocol,
            }),
        )
        .await
    }

    pub async fn delete_provider(&self, provider_id: &str) -> Result<bool> {
        self.send_request(
            "delete_provider",
            serde_json::json!({ "provider_id": provider_id }),
        )
        .await
    }

    pub async fn get_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> Result<agendao_config::ModelConfig> {
        self.send_request(
            "get_provider_model_config",
            serde_json::json!({ "provider_id": provider_id, "model_key": model_key }),
        )
        .await
    }

    pub async fn put_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
        model: &agendao_config::ModelConfig,
    ) -> Result<Config> {
        self.send_request(
            "put_provider_model_config",
            serde_json::json!({
                "provider_id": provider_id,
                "model_key": model_key,
                "model": model,
            }),
        )
        .await
    }

    pub async fn delete_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> Result<Config> {
        self.send_request(
            "delete_provider_model_config",
            serde_json::json!({ "provider_id": provider_id, "model_key": model_key }),
        )
        .await
    }

    pub async fn set_provider_disabled(&self, provider_id: &str, disabled: bool) -> Result<bool> {
        self.send_request(
            "set_provider_disabled",
            serde_json::json!({ "provider_id": provider_id, "disabled": disabled }),
        )
        .await
    }

    pub async fn test_provider_connection(
        &self,
        provider_id: &str,
    ) -> Result<TestProviderConnectionResponse> {
        self.send_request(
            "test_provider_connection",
            serde_json::json!({ "provider_id": provider_id }),
        )
        .await
    }

    pub async fn refresh_provider_catalog(&self) -> Result<RefreshProviderCatalogResponse> {
        self.send_request("refresh_provider_catalog", serde_json::json!({}))
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

    pub async fn list_tools(&self) -> Result<Vec<ToolListEntry>> {
        self.send_request("list_tools", serde_json::json!({})).await
    }

    pub async fn list_skills(&self, query: &SkillCatalogQuery) -> Result<Vec<SkillCatalogEntry>> {
        self.send_request("list_skills", query).await
    }

    pub async fn get_skill_detail(&self, query: &SkillDetailQuery) -> Result<SkillDetailResponse> {
        self.send_request("get_skill_detail", query).await
    }

    pub async fn manage_skill(&self, request: &SkillManageRequest) -> Result<SkillManageResponse> {
        self.send_request("manage_skill", request).await
    }

    pub async fn list_skill_proposals(&self, status: &str) -> Result<Vec<SkillEvolutionProposal>> {
        self.send_request(
            "list_skill_proposals",
            serde_json::json!({ "status": status }),
        )
        .await
    }

    pub async fn update_skill_proposal_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<SkillEvolutionProposal> {
        self.send_request(
            "update_skill_proposal_status",
            serde_json::json!({ "id": id, "status": status }),
        )
        .await
    }

    pub async fn get_mcp_status(&self) -> Result<Vec<McpStatusInfo>> {
        self.send_request("get_mcp_status", serde_json::json!({}))
            .await
    }

    pub async fn connect_mcp(&self, name: &str) -> Result<bool> {
        self.send_request("connect_mcp", serde_json::json!({ "name": name }))
            .await
    }

    pub async fn disconnect_mcp(&self, name: &str) -> Result<bool> {
        self.send_request("disconnect_mcp", serde_json::json!({ "name": name }))
            .await
    }

    pub async fn start_mcp_auth(&self, name: &str) -> Result<McpAuthStartInfo> {
        self.send_request("start_mcp_auth", serde_json::json!({ "name": name }))
            .await
    }

    pub async fn authenticate_mcp(&self, name: &str) -> Result<McpStatusInfo> {
        self.send_request("authenticate_mcp", serde_json::json!({ "name": name }))
            .await
    }

    pub async fn remove_mcp_auth(&self, name: &str) -> Result<bool> {
        self.send_request("remove_mcp_auth", serde_json::json!({ "name": name }))
            .await
    }

    pub async fn put_mcp_config(
        &self,
        key: &str,
        mcp: &agendao_config::McpServerConfig,
    ) -> Result<Config> {
        self.send_request(
            "put_mcp_config",
            serde_json::json!({ "key": key, "mcp": mcp }),
        )
        .await
    }

    pub async fn delete_mcp_config(&self, key: &str) -> Result<Config> {
        self.send_request("delete_mcp_config", serde_json::json!({ "key": key }))
            .await
    }

    pub async fn list_plugins(&self) -> Result<Vec<PluginListEntry>> {
        self.send_request("list_plugins", serde_json::json!({}))
            .await
    }

    pub async fn put_plugin_config(
        &self,
        key: &str,
        plugin: &agendao_config::PluginConfig,
    ) -> Result<Config> {
        self.send_request(
            "put_plugin_config",
            serde_json::json!({ "key": key, "plugin": plugin }),
        )
        .await
    }

    pub async fn delete_plugin_config(&self, key: &str) -> Result<Config> {
        self.send_request("delete_plugin_config", serde_json::json!({ "key": key }))
            .await
    }

    pub async fn cancel_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> Result<serde_json::Value> {
        self.send_request(
            "cancel_tool_call",
            serde_json::json!({ "session_id": session_id, "tool_call_id": tool_call_id }),
        )
        .await
    }

    pub async fn execute_shell(
        &self,
        session_id: &str,
        command: &str,
        workdir: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.send_request(
            "execute_shell",
            serde_json::json!({
                "session_id": session_id,
                "command": command,
                "workdir": workdir,
            }),
        )
        .await
    }

    pub async fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> Result<SessionInfo> {
        self.send_request(
            "fork_session",
            serde_json::json!({ "session_id": session_id, "message_id": message_id }),
        )
        .await
    }

    pub async fn execute_session_recovery(
        &self,
        session_id: &str,
        action: RecoveryActionKind,
    ) -> Result<serde_json::Value> {
        self.send_request(
            "execute_session_recovery",
            serde_json::json!({ "session_id": session_id, "action": action }),
        )
        .await
    }

    pub async fn get_session_recovery(&self, session_id: &str) -> Result<SessionRecoveryProtocol> {
        self.send_request(
            "get_session_recovery",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn list_questions(&self) -> Result<Vec<QuestionInfo>> {
        self.send_request("list_questions", serde_json::json!({}))
            .await
    }

    pub async fn reply_question(&self, question_id: &str, answers: Vec<Vec<String>>) -> Result<()> {
        let _: bool = self
            .send_request(
                "reply_question",
                serde_json::json!({ "question_id": question_id, "answers": answers }),
            )
            .await?;
        Ok(())
    }

    pub async fn reject_question(&self, question_id: &str) -> Result<()> {
        let _: bool = self
            .send_request("reject_question", serde_json::json!({ "id": question_id }))
            .await?;
        Ok(())
    }

    pub async fn abort_session(&self, session_id: &str) -> Result<serde_json::Value> {
        self.send_request(
            "abort_session",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn compact_session(
        &self,
        session_id: &str,
        focus: Option<&str>,
    ) -> Result<CompactResponse> {
        self.send_request(
            "compact_session",
            serde_json::json!({ "session_id": session_id, "focus": focus }),
        )
        .await
    }

    pub async fn update_session_title(&self, session_id: &str, title: &str) -> Result<SessionInfo> {
        self.send_request(
            "update_session_title",
            serde_json::json!({ "session_id": session_id, "title": title }),
        )
        .await
    }

    pub async fn set_session_permission_mode(
        &self,
        session_id: &str,
        mode: crate::SessionPermissionMode,
    ) -> Result<SessionInfo> {
        self.send_request(
            "set_session_permission_mode",
            serde_json::json!({ "session_id": session_id, "mode": mode }),
        )
        .await
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<bool> {
        self.send_request(
            "delete_session",
            serde_json::json!({ "session_id": session_id }),
        )
        .await
    }

    pub async fn put_disabled_config(&self, update: &DisabledConfigUpdate) -> Result<Config> {
        self.send_request("put_disabled_config", update).await
    }

    pub async fn list_permissions(&self) -> Result<Vec<PermissionRequestInfo>> {
        self.send_request("list_permissions", serde_json::json!({}))
            .await
    }

    pub async fn reply_permission(
        &self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) -> Result<()> {
        let _: bool = self
            .send_request(
                "reply_permission",
                serde_json::json!({
                    "permission_id": permission_id,
                    "reply": reply,
                    "message": message,
                }),
            )
            .await?;
        Ok(())
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
    scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    continue_last: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{JsonRpcRequest, PromptRequest};
    use agendao_orchestrator::selector::SchedulerChoice;

    #[test]
    fn prompt_request_serializes_command_scheduler_and_variant() {
        let request = PromptRequest {
            session_id: "ses_1".to_string(),
            text: "/run cargo test".to_string(),
            agent_id: Some("build".to_string()),
            scheduler: Some(SchedulerChoice::Auto),
            model: Some("openai/gpt-5".to_string()),
            variant: Some("fast".to_string()),
            reasoning_effort: Some("high".to_string()),
            continue_last: false,
            command: Some("run".to_string()),
        };

        let value = serde_json::to_value(&request).expect("serialize unix prompt request");
        assert_eq!(value.get("command").and_then(|v| v.as_str()), Some("run"));
        assert_eq!(value["scheduler"]["kind"], "auto");
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
