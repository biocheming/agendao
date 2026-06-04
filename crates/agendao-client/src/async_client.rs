use std::collections::HashMap;

use agendao_config::{Config as AppConfig, ModelConfig};
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Serialize;

use crate::common::{
    build_connect_provider_request, build_session_list_params, http_error, server_url,
    FormatterStatusResponse, LspStatusResponse, RecentModelsPayload, HTTP_TIMEOUT,
};
use crate::{
    AgentInfo, ApiDiffEntry, ApiTodoItem, CompactRequest, CompactResponse,
    ConfigPolicyValidationSnapshot, CreateSessionRequest, ExecuteRecoveryRequest,
    ExecuteShellRequest, ExecutionModeInfo, FullProviderListResponse, KnownProvidersResponse,
    McpAuthStartInfo, McpStatusInfo, MemoryConflictResponse, MemoryConsolidationRequest,
    MemoryConsolidationResponse, MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery,
    MemoryDetailView, MemoryListQuery, MemoryListResponse, MemoryRetrievalPreviewResponse,
    MemoryRetrievalQuery, MemoryRuleHitListResponse, MemoryRuleHitQuery,
    MemoryRulePackListResponse, MemoryValidationReportResponse, MessageInfo,
    MultimodalCapabilitiesResponse, MultimodalPolicyResponse, MultimodalPreflightRequest,
    MultimodalPreflightResponse, PermissionRequestInfo, PromptPart, PromptRequest, PromptResponse,
    ProviderConnectSchemaResponse, ProviderDescriptorResponse, ProviderListResponse,
    ProvisionExternalAdapterSessionRequest, ProvisionExternalAdapterSessionResponse, QuestionInfo,
    RecoveryActionKind, RefreshProviderCatalogResponse, RepairQuery, RepairQueryResponse,
    ResolveProviderConnectRequest, ResolveProviderConnectResponse, RevertRequest, RevertResponse,
    SessionEventsQuery, SessionExecutionTopology, SessionInfo, SessionInsightsResponse,
    SessionListItem, SessionListResponse, SessionRecoveryProtocol, SessionRepairSummaryResponse,
    SessionRuntimeState, SessionStatusInfo, SessionTelemetrySnapshot, ShareResponse,
    SkillCatalogEntry, SkillCatalogQuery, SkillDetailQuery, SkillDetailResponse,
    SkillEvolutionProposal, SkillHubArtifactCacheResponse, SkillHubAuditResponse,
    SkillHubDistributionResponse, SkillHubGuardRunRequest, SkillHubGuardRunResponse,
    SkillHubIndexRefreshRequest, SkillHubIndexRefreshResponse, SkillHubIndexResponse,
    SkillHubLifecycleResponse, SkillHubManagedDetachRequest, SkillHubManagedDetachResponse,
    SkillHubManagedRemoveRequest, SkillHubManagedRemoveResponse, SkillHubManagedResponse,
    SkillHubNegativeEntropyResponse, SkillHubPolicyResponse, SkillHubRemoteInstallApplyRequest,
    SkillHubRemoteInstallPlanRequest, SkillHubRemoteUpdateApplyRequest,
    SkillHubRemoteUpdatePlanRequest, SkillHubReviewCandidatesSyncRequest,
    SkillHubReviewCandidatesSyncResponse, SkillHubSemanticConflictResponse,
    SkillHubSyncApplyRequest, SkillHubSyncPlanRequest, SkillHubSyncPlanResponse,
    SkillHubTimelineQuery, SkillHubTimelineResponse, SkillHubUsageLedgerResponse,
    SkillHubVitalityUpdateRequest, SkillHubVitalityUpdateResponse, SkillManageRequest,
    SkillManageResponse, SkillRemoteInstallPlan, SkillRemoteInstallResponse, UpdateSessionRequest,
};

#[derive(Clone)]
pub struct AsyncApiClient {
    client: reqwest::Client,
    base_url: String,
}

impl AsyncApiClient {
    pub fn new(base_url: String) -> Self {
        Self::new_with_password(base_url, None)
    }

    pub fn new_with_password(base_url: String, server_password: Option<String>) -> Self {
        let mut headers = HeaderMap::new();
        if let Some(password) = server_password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {password}")) {
                headers.insert(AUTHORIZATION, value);
            }
        }

        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .default_headers(headers)
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn create_session(
        &self,
        scheduler_profile: Option<String>,
        directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        let url = server_url(&self.base_url, "/session");
        let req = CreateSessionRequest {
            scheduler_profile,
            directory,
            project_id: None,
            title: None,
        };
        let resp = self.client.post(&url).json(&req).send().await?;
        Self::json_ok(resp, "create session").await
    }

    pub async fn provision_external_adapter_session(
        &self,
        request: &ProvisionExternalAdapterSessionRequest,
    ) -> anyhow::Result<ProvisionExternalAdapterSessionResponse> {
        self.post_json(
            "/external-adapter/session/provision",
            "provision external adapter session",
            request,
        )
        .await
    }

    pub async fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        let url = server_url(&self.base_url, &format!("/session/{}", session_id));
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get session").await
    }

    pub async fn list_sessions(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        let url = server_url(&self.base_url, "/session");
        let params = build_session_list_params(search, limit);
        let req = if params.is_empty() {
            self.client.get(&url)
        } else {
            self.client.get(&url).query(&params)
        };
        let resp = req.send().await?;
        let response: SessionListResponse = Self::json_ok(resp, "list sessions").await?;
        Ok(response.items)
    }

    pub async fn list_sessions_filtered(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        self.list_sessions(search, limit).await
    }

    pub async fn get_session_status(&self) -> anyhow::Result<HashMap<String, SessionStatusInfo>> {
        self.get_json("/session/status", "get session status").await
    }

    pub async fn get_session_executions(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionExecutionTopology> {
        self.get_json(
            &format!("/session/{}/executions", session_id),
            &format!("get session executions `{}`", session_id),
        )
        .await
    }

    pub async fn get_session_runtime(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionRuntimeState> {
        self.get_json(
            &format!("/session/{}/runtime", session_id),
            &format!("get session runtime `{}`", session_id),
        )
        .await
    }

    pub async fn update_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> anyhow::Result<SessionInfo> {
        let url = server_url(&self.base_url, &format!("/session/{}", session_id));
        let req = UpdateSessionRequest {
            title: Some(title.to_string()),
        };
        let resp = self.client.patch(&url).json(&req).send().await?;
        Self::json_ok(resp, "update session title").await
    }

    pub async fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let value: serde_json::Value = self
            .delete_json(
                &format!("/session/{}", session_id),
                &format!("delete session `{}`", session_id),
            )
            .await?;
        Ok(value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub async fn send_prompt(
        &self,
        session_id: &str,
        content: String,
        parts: Option<Vec<PromptPart>>,
        agent: Option<String>,
        scheduler_profile: Option<String>,
        model: Option<String>,
        variant: Option<String>,
        ingress_source: Option<String>,
        idempotency_key: Option<String>,
        source_origin: Option<agendao_types::MessageSourceOrigin>,
        source_surface: Option<agendao_types::MessageSourceSurface>,
    ) -> anyhow::Result<PromptResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/prompt", session_id));
        let req = PromptRequest {
            message: (!content.trim().is_empty()).then_some(content),
            parts,
            idempotency_key,
            ingress_source,
            agent,
            scheduler_profile,
            model,
            variant,
            command: None,
            arguments: None,
            source_origin,
            source_surface,
        };
        let resp = self.client.post(&url).json(&req).send().await?;
        Self::json_ok(resp, "send prompt").await
    }

    pub async fn send_command_prompt(
        &self,
        session_id: &str,
        command: String,
        arguments: Option<String>,
        model: Option<String>,
        variant: Option<String>,
        ingress_source: Option<String>,
        idempotency_key: Option<String>,
        source_origin: Option<agendao_types::MessageSourceOrigin>,
        source_surface: Option<agendao_types::MessageSourceSurface>,
    ) -> anyhow::Result<PromptResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/prompt", session_id));
        let req = PromptRequest {
            message: None,
            parts: None,
            idempotency_key,
            ingress_source,
            agent: None,
            scheduler_profile: None,
            model,
            variant,
            command: Some(command),
            arguments,
            source_origin,
            source_surface,
        };
        let resp = self.client.post(&url).json(&req).send().await?;
        Self::json_ok(resp, "send command prompt").await
    }

    pub async fn execute_shell(
        &self,
        session_id: &str,
        command: String,
        workdir: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let request = ExecuteShellRequest { command, workdir };
        self.post_json(
            &format!("/session/{}/shell", session_id),
            "execute shell command",
            &request,
        )
        .await
    }

    pub async fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        let url = server_url(&self.base_url, &format!("/session/{}/abort", session_id));
        let resp = self.client.post(&url).send().await?;
        Self::json_ok(resp, "abort session").await
    }

    pub async fn cancel_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = server_url(
            &self.base_url,
            &format!("/session/{}/tool/{}/cancel", session_id, tool_call_id),
        );
        let resp = self.client.post(&url).send().await?;
        Self::json_ok(resp, &format!("cancel tool call `{}`", tool_call_id)).await
    }

    pub async fn list_questions(&self) -> anyhow::Result<Vec<QuestionInfo>> {
        let url = server_url(&self.base_url, "/question");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "list questions").await
    }

    pub async fn reply_question(
        &self,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        let url = server_url(&self.base_url, &format!("/question/{}/reply", question_id));
        let body = serde_json::json!({ "answers": answers });
        let resp = self.client.post(&url).json(&body).send().await?;
        Self::expect_success(resp, &format!("reply question `{}`", question_id)).await?;
        Ok(())
    }

    pub async fn reject_question(&self, question_id: &str) -> anyhow::Result<()> {
        let url = server_url(&self.base_url, &format!("/question/{}/reject", question_id));
        let resp = self.client.post(&url).send().await?;
        Self::expect_success(resp, &format!("reject question `{}`", question_id)).await?;
        Ok(())
    }

    pub async fn list_permissions(&self) -> anyhow::Result<Vec<PermissionRequestInfo>> {
        self.get_json("/permission", "list permissions").await
    }

    pub async fn reply_permission(
        &self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        let url = server_url(
            &self.base_url,
            &format!("/permission/{}/reply", permission_id),
        );
        let body = serde_json::json!({
            "reply": reply,
            "message": message,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        Self::expect_success(resp, &format!("reply permission `{}`", permission_id)).await?;
        Ok(())
    }

    pub async fn get_session_telemetry(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionTelemetrySnapshot> {
        let url = server_url(
            &self.base_url,
            &format!("/session/{}/telemetry", session_id),
        );
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get session telemetry").await
    }

    pub async fn get_session_insights(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionInsightsResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/insights", session_id));
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get session insights").await
    }

    pub async fn get_session_events(
        &self,
        session_id: &str,
        query: &SessionEventsQuery,
    ) -> anyhow::Result<Vec<agendao_stage_protocol::StageEvent>> {
        let url = server_url(&self.base_url, &format!("/session/{}/events", session_id));
        let resp = self.client.get(&url).query(query).send().await?;
        Self::json_ok(resp, "get session events").await
    }

    pub async fn get_session_todos(&self, session_id: &str) -> anyhow::Result<Vec<ApiTodoItem>> {
        let url = server_url(&self.base_url, &format!("/session/{}/todo", session_id));
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        Ok(resp.json::<Vec<ApiTodoItem>>().await?)
    }

    pub async fn get_session_diff(&self, session_id: &str) -> anyhow::Result<Vec<ApiDiffEntry>> {
        let url = server_url(&self.base_url, &format!("/session/{}/diff", session_id));
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        Ok(resp.json::<Vec<ApiDiffEntry>>().await?)
    }

    pub async fn get_session_recovery(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionRecoveryProtocol> {
        self.get_json(
            &format!("/session/{}/recovery", session_id),
            &format!("get session recovery `{}`", session_id),
        )
        .await
    }

    pub async fn get_session_repair_summary(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionRepairSummaryResponse> {
        self.get_json(
            &format!("/session/{}/repair/summary", session_id),
            &format!("get session repair summary `{}`", session_id),
        )
        .await
    }

    pub async fn query_session_repair(
        &self,
        session_id: &str,
        query: &RepairQuery,
    ) -> anyhow::Result<RepairQueryResponse> {
        self.get_json_query(
            &format!("/session/{}/repair/query", session_id),
            query,
            &format!("query session repair `{}`", session_id),
        )
        .await
    }

    pub async fn query_global_repair(
        &self,
        query: &RepairQuery,
    ) -> anyhow::Result<RepairQueryResponse> {
        self.get_json_query("/global/repair/query", query, "query global repair")
            .await
    }

    pub async fn execute_session_recovery(
        &self,
        session_id: &str,
        action: RecoveryActionKind,
        target_id: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let request = ExecuteRecoveryRequest { action, target_id };
        self.post_json(
            &format!("/session/{}/recovery/execute", session_id),
            &format!("execute session recovery `{}`", session_id),
            &request,
        )
        .await
    }

    pub async fn list_memory(
        &self,
        query: Option<&MemoryListQuery>,
    ) -> anyhow::Result<MemoryListResponse> {
        let url = server_url(&self.base_url, "/memory/list");
        let req = match query {
            Some(query) => self.client.get(&url).query(query),
            None => self.client.get(&url),
        };
        let resp = req.send().await?;
        Self::json_ok(resp, "list memory").await
    }

    pub async fn search_memory(
        &self,
        query: Option<&MemoryListQuery>,
    ) -> anyhow::Result<MemoryListResponse> {
        let url = server_url(&self.base_url, "/memory/search");
        let req = match query {
            Some(query) => self.client.get(&url).query(query),
            None => self.client.get(&url),
        };
        let resp = req.send().await?;
        Self::json_ok(resp, "search memory").await
    }

    pub async fn get_memory_retrieval_preview(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> anyhow::Result<MemoryRetrievalPreviewResponse> {
        let url = server_url(&self.base_url, "/memory/retrieval-preview");
        let resp = self.client.get(&url).query(query).send().await?;
        Self::json_ok(resp, "get memory retrieval preview").await
    }

    pub async fn get_memory_detail(&self, record_id: &str) -> anyhow::Result<MemoryDetailView> {
        let url = server_url(&self.base_url, &format!("/memory/{}", record_id));
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get memory detail").await
    }

    pub async fn get_memory_validation_report(
        &self,
        record_id: &str,
    ) -> anyhow::Result<MemoryValidationReportResponse> {
        let url = server_url(
            &self.base_url,
            &format!("/memory/{}/validation-report", record_id),
        );
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get memory validation report").await
    }

    pub async fn get_memory_conflicts(
        &self,
        record_id: &str,
    ) -> anyhow::Result<MemoryConflictResponse> {
        let url = server_url(&self.base_url, &format!("/memory/{}/conflicts", record_id));
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get memory conflicts").await
    }

    pub async fn list_memory_rule_packs(&self) -> anyhow::Result<MemoryRulePackListResponse> {
        let url = server_url(&self.base_url, "/memory/rule-packs");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "list memory rule packs").await
    }

    pub async fn list_memory_rule_hits(
        &self,
        query: Option<&MemoryRuleHitQuery>,
    ) -> anyhow::Result<MemoryRuleHitListResponse> {
        let url = server_url(&self.base_url, "/memory/rule-hits");
        let req = match query {
            Some(query) => self.client.get(&url).query(query),
            None => self.client.get(&url),
        };
        let resp = req.send().await?;
        Self::json_ok(resp, "list memory rule hits").await
    }

    pub async fn list_memory_consolidation_runs(
        &self,
        query: Option<&MemoryConsolidationRunQuery>,
    ) -> anyhow::Result<MemoryConsolidationRunListResponse> {
        let url = server_url(&self.base_url, "/memory/consolidation/runs");
        let req = match query {
            Some(query) => self.client.get(&url).query(query),
            None => self.client.get(&url),
        };
        let resp = req.send().await?;
        Self::json_ok(resp, "list memory consolidation runs").await
    }

    pub async fn run_memory_consolidation(
        &self,
        request: &MemoryConsolidationRequest,
    ) -> anyhow::Result<MemoryConsolidationResponse> {
        let url = server_url(&self.base_url, "/memory/consolidate");
        let resp = self.client.post(&url).json(request).send().await?;
        Self::json_ok(resp, "run memory consolidation").await
    }

    pub async fn get_workspace_context(&self) -> anyhow::Result<ResolvedWorkspaceContext> {
        let url = server_url(&self.base_url, "/workspace/context");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get workspace context").await
    }

    pub async fn get_config(&self) -> anyhow::Result<AppConfig> {
        let url = server_url(&self.base_url, "/config");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get config").await
    }

    pub async fn get_config_validation(&self) -> anyhow::Result<ConfigPolicyValidationSnapshot> {
        self.get_json("/config/validation", "get config validation")
            .await
    }

    pub async fn get_config_providers(&self) -> anyhow::Result<ProviderListResponse> {
        self.get_json("/config/providers", "get config providers")
            .await
    }

    pub async fn get_multimodal_policy(&self) -> anyhow::Result<MultimodalPolicyResponse> {
        self.get_json("/multimodal/policy", "get multimodal policy")
            .await
    }

    pub async fn get_multimodal_capabilities(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<MultimodalCapabilitiesResponse> {
        let url = server_url(&self.base_url, "/multimodal/capabilities");
        let req = if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            self.client.get(&url).query(&[("model", model)])
        } else {
            self.client.get(&url)
        };
        let resp = req.send().await?;
        Self::json_ok(resp, "get multimodal capabilities").await
    }

    pub async fn preflight_multimodal(
        &self,
        request: &MultimodalPreflightRequest,
    ) -> anyhow::Result<MultimodalPreflightResponse> {
        let url = server_url(&self.base_url, "/multimodal/preflight");
        let resp = self.client.post(&url).json(request).send().await?;
        Self::json_ok(resp, "post multimodal preflight").await
    }

    pub async fn get_recent_models(&self) -> anyhow::Result<Vec<RecentModelEntry>> {
        let payload: RecentModelsPayload = self
            .get_json("/workspace/recent-models", "get recent models")
            .await?;
        Ok(payload.recent_models)
    }

    pub async fn put_recent_models(
        &self,
        recent_models: &[RecentModelEntry],
    ) -> anyhow::Result<Vec<RecentModelEntry>> {
        let payload: RecentModelsPayload = self
            .put_json(
                "/workspace/recent-models",
                "save recent models",
                &RecentModelsPayload {
                    recent_models: recent_models.to_vec(),
                },
            )
            .await?;
        Ok(payload.recent_models)
    }

    pub async fn patch_config(&self, patch: &serde_json::Value) -> anyhow::Result<AppConfig> {
        self.patch_json("/config", "patch config", patch).await
    }

    pub async fn get_all_providers(&self) -> anyhow::Result<FullProviderListResponse> {
        let url = server_url(&self.base_url, "/provider");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get all providers").await
    }

    pub async fn get_known_providers(&self) -> anyhow::Result<KnownProvidersResponse> {
        self.get_json("/provider/known", "get known providers")
            .await
    }

    pub async fn get_provider_connect_schema(
        &self,
    ) -> anyhow::Result<ProviderConnectSchemaResponse> {
        let url = server_url(&self.base_url, "/provider/connect/schema");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get provider connect schema").await
    }

    pub async fn get_provider_descriptor(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<ProviderDescriptorResponse> {
        self.get_json(
            &format!("/provider/{}/descriptor", urlencoding::encode(provider_id)),
            &format!("get provider descriptor `{provider_id}`"),
        )
        .await
    }

    pub async fn resolve_provider_connect(
        &self,
        query: &str,
    ) -> anyhow::Result<ResolveProviderConnectResponse> {
        let url = server_url(&self.base_url, "/provider/connect/resolve");
        let resp = self
            .client
            .post(&url)
            .json(&ResolveProviderConnectRequest {
                query: query.to_string(),
            })
            .send()
            .await?;
        Self::json_ok(resp, "resolve provider connect").await
    }

    pub async fn refresh_provider_catalog(&self) -> anyhow::Result<RefreshProviderCatalogResponse> {
        let url = server_url(&self.base_url, "/provider/refresh");
        let resp = self.client.post(&url).send().await?;
        Self::json_ok(resp, "refresh provider catalogue").await
    }

    pub async fn put_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
        model: &ModelConfig,
    ) -> anyhow::Result<AppConfig> {
        let url = server_url(
            &self.base_url,
            &format!(
                "/config/provider/{}/models/{}",
                urlencoding::encode(provider_id),
                urlencoding::encode(model_key)
            ),
        );
        let resp = self.client.put(&url).json(model).send().await?;
        Self::json_ok(
            resp,
            &format!("put provider model config `{provider_id}/{model_key}`"),
        )
        .await
    }

    pub async fn delete_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> anyhow::Result<AppConfig> {
        let url = server_url(
            &self.base_url,
            &format!(
                "/config/provider/{}/models/{}",
                urlencoding::encode(provider_id),
                urlencoding::encode(model_key)
            ),
        );
        let resp = self.client.delete(&url).send().await?;
        Self::json_ok(
            resp,
            &format!("delete provider model config `{provider_id}/{model_key}`"),
        )
        .await
    }

    pub async fn set_auth(&self, provider_id: &str, api_key: &str) -> anyhow::Result<()> {
        self.connect_provider(provider_id, api_key, None, None)
            .await
    }

    pub async fn register_custom_provider(
        &self,
        provider_id: &str,
        base_url: &str,
        protocol: &str,
        api_key: &str,
    ) -> anyhow::Result<()> {
        self.connect_provider(
            provider_id,
            api_key,
            Some(base_url.to_string()),
            Some(protocol.to_string()),
        )
        .await
    }

    pub async fn connect_provider(
        &self,
        provider_id: &str,
        api_key: &str,
        base_url: Option<String>,
        protocol: Option<String>,
    ) -> anyhow::Result<()> {
        let url = server_url(&self.base_url, "/provider/connect");
        let body = build_connect_provider_request(provider_id, api_key, base_url, protocol);
        let resp = self.client.post(&url).json(&body).send().await?;
        Self::expect_success(resp, &format!("connect provider `{}`", provider_id)).await?;
        Ok(())
    }

    pub async fn list_execution_modes(&self) -> anyhow::Result<Vec<ExecutionModeInfo>> {
        let url = server_url(&self.base_url, "/mode");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "list execution modes").await
    }

    pub async fn list_agents(&self) -> anyhow::Result<Vec<AgentInfo>> {
        self.get_json("/agent", "list agents").await
    }

    pub async fn list_skills(
        &self,
        query: Option<&SkillCatalogQuery>,
    ) -> anyhow::Result<Vec<SkillCatalogEntry>> {
        let url = server_url(&self.base_url, "/skill/catalog");
        let resp = match query {
            Some(query) => self.client.get(&url).query(query).send().await?,
            None => self.client.get(&url).send().await?,
        };
        Self::json_ok(resp, "list skills").await
    }

    pub async fn get_skill_detail(
        &self,
        query: &SkillDetailQuery,
    ) -> anyhow::Result<SkillDetailResponse> {
        let url = server_url(&self.base_url, "/skill/detail");
        let resp = self.client.get(&url).query(query).send().await?;
        Self::json_ok(resp, &format!("get skill detail `{}`", query.name)).await
    }

    pub async fn manage_skill(
        &self,
        req: &SkillManageRequest,
    ) -> anyhow::Result<SkillManageResponse> {
        let url = server_url(&self.base_url, "/skill/manage");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "manage skill").await
    }

    pub async fn list_skill_hub_managed(&self) -> anyhow::Result<SkillHubManagedResponse> {
        let url = server_url(&self.base_url, "/skill/hub/managed");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub managed").await
    }

    pub async fn list_skill_hub_usage(&self) -> anyhow::Result<SkillHubUsageLedgerResponse> {
        let url = server_url(&self.base_url, "/skill/hub/usage");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub usage ledger").await
    }

    pub async fn list_skill_hub_negative_entropy(
        &self,
    ) -> anyhow::Result<SkillHubNegativeEntropyResponse> {
        let url = server_url(&self.base_url, "/skill/hub/negative-entropy");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub negative entropy diagnostics").await
    }

    pub async fn sync_skill_hub_review_candidates(
        &self,
        req: &SkillHubReviewCandidatesSyncRequest,
    ) -> anyhow::Result<SkillHubReviewCandidatesSyncResponse> {
        self.post_json(
            "/skill/hub/review-candidates/sync",
            "sync skill hub review candidates",
            req,
        )
        .await
    }

    pub async fn list_skill_hub_semantic_conflicts(
        &self,
    ) -> anyhow::Result<SkillHubSemanticConflictResponse> {
        let url = server_url(&self.base_url, "/skill/hub/semantic-conflicts");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub semantic conflict diagnostics").await
    }

    pub async fn sync_skill_hub_semantic_conflict_review_candidates(
        &self,
        req: &SkillHubReviewCandidatesSyncRequest,
    ) -> anyhow::Result<SkillHubReviewCandidatesSyncResponse> {
        self.post_json(
            "/skill/hub/semantic-conflicts/review-candidates/sync",
            "sync skill hub semantic conflict review candidates",
            req,
        )
        .await
    }

    pub async fn update_skill_hub_vitality(
        &self,
        req: &SkillHubVitalityUpdateRequest,
    ) -> anyhow::Result<SkillHubVitalityUpdateResponse> {
        self.post_json("/skill/hub/vitality", "update skill hub vitality", req)
            .await
    }

    pub async fn list_skill_hub_index(&self) -> anyhow::Result<SkillHubIndexResponse> {
        let url = server_url(&self.base_url, "/skill/hub/index");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub index").await
    }

    pub async fn list_skill_hub_distributions(
        &self,
    ) -> anyhow::Result<SkillHubDistributionResponse> {
        let url = server_url(&self.base_url, "/skill/hub/distributions");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub distributions").await
    }

    pub async fn list_skill_hub_artifact_cache(
        &self,
    ) -> anyhow::Result<SkillHubArtifactCacheResponse> {
        let url = server_url(&self.base_url, "/skill/hub/artifact-cache");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub artifact cache").await
    }

    pub async fn list_skill_hub_policy(&self) -> anyhow::Result<SkillHubPolicyResponse> {
        let url = server_url(&self.base_url, "/skill/hub/policy");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub policy").await
    }

    pub async fn list_skill_hub_lifecycle(&self) -> anyhow::Result<SkillHubLifecycleResponse> {
        let url = server_url(&self.base_url, "/skill/hub/lifecycle");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub lifecycle").await
    }

    pub async fn refresh_skill_hub_index(
        &self,
        req: &SkillHubIndexRefreshRequest,
    ) -> anyhow::Result<SkillHubIndexRefreshResponse> {
        let url = server_url(&self.base_url, "/skill/hub/index/refresh");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "refresh skill hub index").await
    }

    pub async fn list_skill_hub_audit(&self) -> anyhow::Result<SkillHubAuditResponse> {
        let url = server_url(&self.base_url, "/skill/hub/audit");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub audit").await
    }

    pub async fn list_skill_hub_timeline(
        &self,
        query: &SkillHubTimelineQuery,
    ) -> anyhow::Result<SkillHubTimelineResponse> {
        let url = server_url(&self.base_url, "/skill/hub/timeline");
        let resp = self.client.get(&url).query(query).send().await?;
        Self::json_ok(resp, "get skill hub timeline").await
    }

    pub async fn run_skill_hub_guard(
        &self,
        req: &SkillHubGuardRunRequest,
    ) -> anyhow::Result<SkillHubGuardRunResponse> {
        let url = server_url(&self.base_url, "/skill/hub/guard/run");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "run skill hub guard").await
    }

    pub async fn plan_skill_hub_sync(
        &self,
        req: &SkillHubSyncPlanRequest,
    ) -> anyhow::Result<SkillHubSyncPlanResponse> {
        let url = server_url(&self.base_url, "/skill/hub/sync/plan");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "plan skill hub sync").await
    }

    pub async fn apply_skill_hub_sync(
        &self,
        req: &SkillHubSyncApplyRequest,
    ) -> anyhow::Result<SkillHubSyncPlanResponse> {
        let url = server_url(&self.base_url, "/skill/hub/sync/apply");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "apply skill hub sync").await
    }

    pub async fn plan_skill_hub_remote_install(
        &self,
        req: &SkillHubRemoteInstallPlanRequest,
    ) -> anyhow::Result<SkillRemoteInstallPlan> {
        let url = server_url(&self.base_url, "/skill/hub/install/plan");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "plan skill hub remote install").await
    }

    pub async fn apply_skill_hub_remote_install(
        &self,
        req: &SkillHubRemoteInstallApplyRequest,
    ) -> anyhow::Result<SkillRemoteInstallResponse> {
        let url = server_url(&self.base_url, "/skill/hub/install/apply");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "apply skill hub remote install").await
    }

    pub async fn plan_skill_hub_remote_update(
        &self,
        req: &SkillHubRemoteUpdatePlanRequest,
    ) -> anyhow::Result<SkillRemoteInstallPlan> {
        let url = server_url(&self.base_url, "/skill/hub/update/plan");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "plan skill hub remote update").await
    }

    pub async fn apply_skill_hub_remote_update(
        &self,
        req: &SkillHubRemoteUpdateApplyRequest,
    ) -> anyhow::Result<SkillRemoteInstallResponse> {
        let url = server_url(&self.base_url, "/skill/hub/update/apply");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "apply skill hub remote update").await
    }

    pub async fn detach_skill_hub_managed(
        &self,
        req: &SkillHubManagedDetachRequest,
    ) -> anyhow::Result<SkillHubManagedDetachResponse> {
        let url = server_url(&self.base_url, "/skill/hub/detach");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "detach skill hub managed skill").await
    }

    pub async fn remove_skill_hub_managed(
        &self,
        req: &SkillHubManagedRemoveRequest,
    ) -> anyhow::Result<SkillHubManagedRemoveResponse> {
        let url = server_url(&self.base_url, "/skill/hub/remove");
        let resp = self.client.post(&url).json(req).send().await?;
        Self::json_ok(resp, "remove skill hub managed skill").await
    }

    pub async fn get_mcp_status(&self) -> anyhow::Result<Vec<McpStatusInfo>> {
        let url = server_url(&self.base_url, "/mcp");
        let resp = self.client.get(&url).send().await?;
        let map: HashMap<String, McpStatusInfo> = Self::json_ok(resp, "get MCP status").await?;
        let mut servers: Vec<McpStatusInfo> = map.into_values().collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub async fn start_mcp_auth(&self, name: &str) -> anyhow::Result<McpAuthStartInfo> {
        self.post_json_no_body(
            &format!("/mcp/{}/auth/start", name),
            &format!("start MCP auth `{}`", name),
        )
        .await
    }

    pub async fn authenticate_mcp(&self, name: &str) -> anyhow::Result<McpStatusInfo> {
        self.post_json_no_body(
            &format!("/mcp/{}/auth", name),
            &format!("authenticate MCP `{}`", name),
        )
        .await
    }

    pub async fn remove_mcp_auth(&self, name: &str) -> anyhow::Result<bool> {
        let value: serde_json::Value = self
            .delete_json(
                &format!("/mcp/{}/auth", name),
                &format!("remove MCP auth `{}`", name),
            )
            .await?;
        Ok(value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub async fn connect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let resp = self
            .post_expect_success(
                &format!("/mcp/{}/connect", name),
                &format!("connect MCP `{}`", name),
                Option::<&serde_json::Value>::None,
            )
            .await?;
        Ok(serde_json::from_slice::<bool>(&resp).unwrap_or(true))
    }

    pub async fn disconnect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let resp = self
            .post_expect_success(
                &format!("/mcp/{}/disconnect", name),
                &format!("disconnect MCP `{}`", name),
                Option::<&serde_json::Value>::None,
            )
            .await?;
        Ok(serde_json::from_slice::<bool>(&resp).unwrap_or(true))
    }

    pub async fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageInfo>> {
        self.get_messages_after(session_id, None, None).await
    }

    pub async fn get_messages_after(
        &self,
        session_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        let url = server_url(&self.base_url, &format!("/session/{}/message", session_id));
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(after) = after.map(str::trim).filter(|value| !value.is_empty()) {
            params.push(("after", after.to_string()));
        }
        if let Some(limit) = limit.filter(|value| *value > 0) {
            params.push(("limit", limit.to_string()));
        }
        let req = if params.is_empty() {
            self.client.get(&url)
        } else {
            self.client.get(&url).query(&params)
        };
        let resp = req.send().await?;
        Self::json_ok(resp, "get messages").await
    }

    pub async fn get_lsp_servers(&self) -> anyhow::Result<Vec<String>> {
        let status: LspStatusResponse = self.get_json("/lsp", "get LSP status").await?;
        Ok(status.servers)
    }

    pub async fn get_formatters(&self) -> anyhow::Result<Vec<String>> {
        let status: FormatterStatusResponse =
            self.get_json("/formatter", "get formatter status").await?;
        Ok(status.formatters)
    }

    pub async fn share_session(&self, session_id: &str) -> anyhow::Result<ShareResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/share", session_id));
        let resp = self.client.post(&url).send().await?;
        Self::json_ok(resp, &format!("share session `{}`", session_id)).await
    }

    pub async fn unshare_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let url = server_url(&self.base_url, &format!("/session/{}/share", session_id));
        let resp = self.client.delete(&url).send().await?;
        let value: serde_json::Value =
            Self::json_ok(resp, &format!("unshare session `{}`", session_id)).await?;
        Ok(value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub async fn compact_session(
        &self,
        session_id: &str,
        focus: Option<&str>,
    ) -> anyhow::Result<CompactResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/compact", session_id));
        let req = if let Some(focus) = focus.map(str::trim).filter(|value| !value.is_empty()) {
            self.client.post(&url).query(&CompactRequest {
                focus: Some(focus.to_string()),
            })
        } else {
            self.client.post(&url)
        };
        let resp = req.send().await?;
        Self::json_ok(resp, &format!("compact session `{}`", session_id)).await
    }

    pub async fn revert_session(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> anyhow::Result<RevertResponse> {
        let request = RevertRequest {
            message_id: message_id.to_string(),
        };
        self.post_json(
            &format!("/session/{}/revert", session_id),
            &format!("revert session `{}`", session_id),
            &request,
        )
        .await
    }

    pub async fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<SessionInfo> {
        self.post_json(
            &format!("/session/{}/fork", session_id),
            &format!("fork session `{}`", session_id),
            &serde_json::json!({ "message_id": message_id }),
        )
        .await
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        action: &str,
    ) -> anyhow::Result<T> {
        let url = server_url(&self.base_url, path);
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, action).await
    }

    async fn get_json_query<T: serde::de::DeserializeOwned, Q: Serialize + ?Sized>(
        &self,
        path: &str,
        query: &Q,
        action: &str,
    ) -> anyhow::Result<T> {
        let url = server_url(&self.base_url, path);
        let resp = self.client.get(&url).query(query).send().await?;
        Self::json_ok(resp, action).await
    }

    async fn post_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let bytes = self.post_expect_success(path, action, Some(body)).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn post_json_no_body<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        action: &str,
    ) -> anyhow::Result<T> {
        let bytes = self
            .post_expect_success(path, action, Option::<&serde_json::Value>::None)
            .await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn post_expect_success<B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: Option<&B>,
    ) -> anyhow::Result<Vec<u8>> {
        let url = server_url(&self.base_url, path);
        let request = self.client.post(url);
        let resp = match body {
            Some(body) => request.json(body).send().await?,
            None => request.send().await?,
        };
        Self::expect_success(resp, action).await
    }

    async fn patch_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let url = server_url(&self.base_url, path);
        let resp = self.client.patch(&url).json(body).send().await?;
        Self::json_ok(resp, action).await
    }

    async fn put_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let url = server_url(&self.base_url, path);
        let resp = self.client.put(&url).json(body).send().await?;
        Self::json_ok(resp, action).await
    }

    async fn delete_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        action: &str,
    ) -> anyhow::Result<T> {
        let url = server_url(&self.base_url, path);
        let resp = self.client.delete(&url).send().await?;
        Self::json_ok(resp, action).await
    }

    async fn expect_success(resp: reqwest::Response, action: &str) -> anyhow::Result<Vec<u8>> {
        let status = resp.status();
        if status.is_success() {
            Ok(resp.bytes().await?.to_vec())
        } else {
            let text = resp
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            Err(http_error(action, status, text))
        }
    }

    pub async fn list_skill_proposals(
        &self,
        status: &str,
    ) -> anyhow::Result<Vec<SkillEvolutionProposal>> {
        let url = format!("/skill/proposal/?status={}", status);
        self.get_json(&url, "list skill proposals").await
    }

    pub async fn update_skill_proposal_status(
        &self,
        id: &str,
        status: &str,
    ) -> anyhow::Result<SkillEvolutionProposal> {
        let url = format!("/skill/proposal/{}/status", id);
        self.post_json(
            &url,
            "update skill proposal status",
            &serde_json::json!({"status": status}),
        )
        .await
    }

    /// Submit a mid-run steering message to the owner session.
    /// Constitution §9: client only submits; runtime consumes at tool boundary.
    pub async fn submit_steering(
        &self,
        session_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("/session/{}/steer", session_id);
        self.post_json(&url, "submit steering", &serde_json::json!({"text": text}))
            .await
    }

    async fn json_ok<T: serde::de::DeserializeOwned>(
        resp: reqwest::Response,
        action: &str,
    ) -> anyhow::Result<T> {
        let bytes = Self::expect_success(resp, action).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
