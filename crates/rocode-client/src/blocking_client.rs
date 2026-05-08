use std::collections::HashMap;
use std::sync::RwLock;

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use rocode_command::stage_protocol::StageEvent;
use rocode_config::{Config as AppConfig, ModelConfig};
use rocode_runtime_context::ResolvedWorkspaceContext;
use rocode_state::RecentModelEntry;
use serde::Serialize;

use crate::common::{
    build_connect_provider_request, build_session_list_params, http_error, server_url,
    FormatterStatusResponse, LspStatusResponse, RecentModelsPayload, HTTP_TIMEOUT,
};
use crate::{
    AgentInfo, ApiDiffEntry, ApiTodoItem, CompactRequest, CompactResponse,
    ConfigPolicyValidationSnapshot, ConnectProviderRequest, CreateSessionRequest,
    ExecuteRecoveryRequest, ExecuteShellRequest, ExecutionModeInfo, FullProviderListResponse,
    KnownProvidersResponse, McpAuthStartInfo, McpStatusInfo, MemoryConflictResponse,
    MemoryConsolidationRequest, MemoryConsolidationResponse, MemoryConsolidationRunListResponse,
    MemoryConsolidationRunQuery, MemoryDetailView, MemoryListQuery, MemoryListResponse,
    MemoryRetrievalPreviewResponse, MemoryRetrievalQuery, MemoryRuleHitListResponse,
    MemoryRuleHitQuery, MemoryRulePackListResponse, MemoryValidationReportResponse, MessageInfo,
    MultimodalCapabilitiesResponse, MultimodalPolicyResponse, MultimodalPreflightRequest,
    MultimodalPreflightResponse, PermissionRequestInfo, PromptPart, PromptRequest, PromptResponse,
    ProviderConnectSchemaResponse, ProviderDescriptorResponse, ProviderListResponse,
    ProvisionExternalAdapterSessionRequest, ProvisionExternalAdapterSessionResponse, QuestionInfo,
    RecoveryActionKind, RefreshProviderCatalogResponse, ResolveProviderConnectRequest,
    ResolveProviderConnectResponse, RevertRequest, RevertResponse, SessionEventsQuery,
    SessionExecutionTopology, SessionInfo, SessionInsightsResponse, SessionListItem,
    SessionListResponse, SessionRecoveryProtocol, SessionRuntimeState, SessionStatusInfo,
    SessionTelemetrySnapshot, ShareResponse, SkillCatalogEntry, SkillCatalogQuery,
    SkillDetailQuery, SkillDetailResponse, SkillHubArtifactCacheResponse, SkillHubAuditResponse,
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

pub struct BlockingApiClient {
    client: Client,
    base_url: String,
    current_session: RwLock<Option<SessionInfo>>,
}

impl BlockingApiClient {
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

        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .default_headers(headers)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url,
            current_session: RwLock::new(None),
        }
    }

    pub fn create_session(
        &self,
        scheduler_profile: Option<String>,
        directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        let request = CreateSessionRequest {
            scheduler_profile,
            directory,
            project_id: None,
            title: None,
        };
        self.post_json("/session", "create session", &request)
    }

    pub fn provision_external_adapter_session(
        &self,
        request: &ProvisionExternalAdapterSessionRequest,
    ) -> anyhow::Result<ProvisionExternalAdapterSessionResponse> {
        self.post_json(
            "/external-adapter/session/provision",
            "provision external adapter session",
            request,
        )
    }

    pub fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        self.get_json(
            &format!("/session/{}", session_id),
            &format!("get session `{}`", session_id),
        )
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionListItem>> {
        self.list_sessions_filtered(None, None)
    }

    pub fn list_sessions_filtered(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        let url = server_url(&self.base_url, "/session");
        let params = build_session_list_params(search, limit);
        let request = if params.is_empty() {
            self.client.get(&url)
        } else {
            self.client.get(&url).query(&params)
        };
        let response: SessionListResponse = Self::json_ok(request.send()?, "list sessions")?;
        Ok(response.items)
    }

    pub fn get_session_status(&self) -> anyhow::Result<HashMap<String, SessionStatusInfo>> {
        self.get_json("/session/status", "get session status")
    }

    pub fn get_session_executions(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionExecutionTopology> {
        self.get_json(
            &format!("/session/{}/executions", session_id),
            &format!("get session executions `{}`", session_id),
        )
    }

    pub fn get_session_runtime(&self, session_id: &str) -> anyhow::Result<SessionRuntimeState> {
        self.get_json(
            &format!("/session/{}/runtime", session_id),
            &format!("get session runtime `{}`", session_id),
        )
    }

    pub fn get_session_telemetry(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionTelemetrySnapshot> {
        self.get_json(
            &format!("/session/{}/telemetry", session_id),
            &format!("get session telemetry `{}`", session_id),
        )
    }

    pub fn get_session_insights(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionInsightsResponse> {
        self.get_json(
            &format!("/session/{}/insights", session_id),
            &format!("get session insights `{}`", session_id),
        )
    }

    pub fn get_session_events(
        &self,
        session_id: &str,
        query: &SessionEventsQuery,
    ) -> anyhow::Result<Vec<StageEvent>> {
        self.get_json_query(
            &format!("/session/{}/events", session_id),
            query,
            &format!("get session events `{}`", session_id),
        )
    }

    pub fn get_session_todos(&self, session_id: &str) -> anyhow::Result<Vec<ApiTodoItem>> {
        let response = self
            .client
            .get(server_url(
                &self.base_url,
                &format!("/session/{}/todo", session_id),
            ))
            .send()?;
        if !response.status().is_success() {
            return Ok(Vec::new());
        }
        Ok(response.json::<Vec<ApiTodoItem>>()?)
    }

    pub fn get_session_diff(&self, session_id: &str) -> anyhow::Result<Vec<ApiDiffEntry>> {
        let response = self
            .client
            .get(server_url(
                &self.base_url,
                &format!("/session/{}/diff", session_id),
            ))
            .send()?;
        if !response.status().is_success() {
            return Ok(Vec::new());
        }
        Ok(response.json::<Vec<ApiDiffEntry>>()?)
    }

    pub fn get_session_recovery(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionRecoveryProtocol> {
        self.get_json(
            &format!("/session/{}/recovery", session_id),
            &format!("get session recovery `{}`", session_id),
        )
    }

    pub fn execute_session_recovery(
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
    }

    pub fn list_questions(&self) -> anyhow::Result<Vec<QuestionInfo>> {
        self.get_json("/question", "list questions")
    }

    pub fn reply_question(
        &self,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        let body = serde_json::json!({ "answers": answers });
        self.post_unit(
            &format!("/question/{}/reply", question_id),
            &format!("reply question `{}`", question_id),
            Some(&body),
        )
    }

    pub fn reject_question(&self, question_id: &str) -> anyhow::Result<()> {
        self.post_unit(
            &format!("/question/{}/reject", question_id),
            &format!("reject question `{}`", question_id),
            Option::<&serde_json::Value>::None,
        )
    }

    pub fn list_permissions(&self) -> anyhow::Result<Vec<PermissionRequestInfo>> {
        self.get_json("/permission", "list permissions")
    }

    pub fn reply_permission(
        &self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "reply": reply,
            "message": message,
        });
        self.post_unit(
            &format!("/permission/{}/reply", permission_id),
            &format!("reply permission `{}`", permission_id),
            Some(&body),
        )
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> anyhow::Result<SessionInfo> {
        let request = UpdateSessionRequest {
            title: Some(title.to_string()),
        };
        self.patch_json(
            &format!("/session/{}", session_id),
            &format!("update session `{}` title", session_id),
            &request,
        )
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let response = self.delete_expect_success(
            &format!("/session/{}", session_id),
            &format!("delete session `{}`", session_id),
        )?;
        let value = response.json::<serde_json::Value>()?;
        Ok(value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn send_prompt(
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
    ) -> anyhow::Result<PromptResponse> {
        let request = PromptRequest {
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
        };
        self.post_json(
            &format!("/session/{}/prompt", session_id),
            &format!("send prompt to session `{}`", session_id),
            &request,
        )
    }

    pub fn send_command_prompt(
        &self,
        session_id: &str,
        command: String,
        arguments: Option<String>,
        model: Option<String>,
        variant: Option<String>,
        ingress_source: Option<String>,
        idempotency_key: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        let request = PromptRequest {
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
        };
        self.post_json(
            &format!("/session/{}/prompt", session_id),
            &format!("send command prompt to session `{}`", session_id),
            &request,
        )
    }

    pub fn execute_shell(
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
    }

    pub fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        self.post_json_no_body(
            &format!("/session/{}/abort", session_id),
            &format!("abort session `{}`", session_id),
        )
    }

    pub fn cancel_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.post_json_no_body(
            &format!("/session/{}/tool/{}/cancel", session_id, tool_call_id),
            &format!("cancel tool call `{}`", tool_call_id),
        )
    }

    pub fn get_config_providers(&self) -> anyhow::Result<ProviderListResponse> {
        self.get_json("/config/providers", "get config providers")
    }

    pub fn get_config(&self) -> anyhow::Result<AppConfig> {
        self.get_json("/config", "get config")
    }

    pub fn get_config_validation(&self) -> anyhow::Result<ConfigPolicyValidationSnapshot> {
        self.get_json("/config/validation", "get config validation")
    }

    pub fn get_workspace_context(&self) -> anyhow::Result<ResolvedWorkspaceContext> {
        self.get_json("/workspace/context", "get workspace context")
    }

    pub fn get_multimodal_policy(&self) -> anyhow::Result<MultimodalPolicyResponse> {
        self.get_json("/multimodal/policy", "get multimodal policy")
    }

    pub fn get_multimodal_capabilities(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<MultimodalCapabilitiesResponse> {
        let url = server_url(&self.base_url, "/multimodal/capabilities");
        let request = if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            self.client.get(&url).query(&[("model", model)])
        } else {
            self.client.get(&url)
        };
        Self::json_ok(request.send()?, "get multimodal capabilities")
    }

    pub fn preflight_multimodal(
        &self,
        request: &MultimodalPreflightRequest,
    ) -> anyhow::Result<MultimodalPreflightResponse> {
        self.post_json("/multimodal/preflight", "run multimodal preflight", request)
    }

    pub fn get_recent_models(&self) -> anyhow::Result<Vec<RecentModelEntry>> {
        let payload: RecentModelsPayload =
            self.get_json("/workspace/recent-models", "get recent models")?;
        Ok(payload.recent_models)
    }

    pub fn put_recent_models(
        &self,
        recent_models: &[RecentModelEntry],
    ) -> anyhow::Result<Vec<RecentModelEntry>> {
        let payload: RecentModelsPayload = self.put_json(
            "/workspace/recent-models",
            "save recent models",
            &RecentModelsPayload {
                recent_models: recent_models.to_vec(),
            },
        )?;
        Ok(payload.recent_models)
    }

    pub fn patch_config(&self, patch: &serde_json::Value) -> anyhow::Result<AppConfig> {
        self.patch_json("/config", "patch config", patch)
    }

    pub fn put_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
        model: &ModelConfig,
    ) -> anyhow::Result<AppConfig> {
        self.put_json(
            &format!(
                "/config/provider/{}/models/{}",
                urlencoding::encode(provider_id),
                urlencoding::encode(model_key)
            ),
            &format!("put provider model config `{provider_id}/{model_key}`"),
            model,
        )
    }

    pub fn delete_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> anyhow::Result<AppConfig> {
        let response = self.delete_expect_success(
            &format!(
                "/config/provider/{}/models/{}",
                urlencoding::encode(provider_id),
                urlencoding::encode(model_key)
            ),
            &format!("delete provider model config `{provider_id}/{model_key}`"),
        )?;
        Self::json_ok(response, "delete provider model config")
    }

    pub fn get_all_providers(&self) -> anyhow::Result<FullProviderListResponse> {
        self.get_json("/provider", "get all providers")
    }

    pub fn get_known_providers(&self) -> anyhow::Result<KnownProvidersResponse> {
        self.get_json("/provider/known", "get known providers")
    }

    pub fn get_provider_connect_schema(&self) -> anyhow::Result<ProviderConnectSchemaResponse> {
        self.get_json("/provider/connect/schema", "get provider connect schema")
    }

    pub fn get_provider_descriptor(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<ProviderDescriptorResponse> {
        self.get_json(
            &format!("/provider/{}/descriptor", urlencoding::encode(provider_id)),
            &format!("get provider descriptor `{provider_id}`"),
        )
    }

    pub fn resolve_provider_connect(
        &self,
        query: &str,
    ) -> anyhow::Result<ResolveProviderConnectResponse> {
        self.post_json(
            "/provider/connect/resolve",
            "resolve provider connect query",
            &ResolveProviderConnectRequest {
                query: query.to_string(),
            },
        )
    }

    pub fn refresh_provider_catalog(&self) -> anyhow::Result<RefreshProviderCatalogResponse> {
        self.post_json_no_body("/provider/refresh", "refresh provider catalogue")
    }

    pub fn set_auth(&self, provider_id: &str, api_key: &str) -> anyhow::Result<()> {
        self.connect_provider(&build_connect_provider_request(
            provider_id,
            api_key,
            None,
            None,
        ))
    }

    pub fn register_custom_provider(
        &self,
        provider_id: &str,
        base_url: &str,
        protocol: &str,
        api_key: &str,
    ) -> anyhow::Result<()> {
        self.connect_provider(&build_connect_provider_request(
            provider_id,
            api_key,
            Some(base_url.to_string()),
            Some(protocol.to_string()),
        ))
    }

    pub fn connect_provider(&self, request: &ConnectProviderRequest) -> anyhow::Result<()> {
        self.post_unit(
            "/provider/connect",
            &format!("connect provider `{}`", request.provider_id),
            Some(request),
        )
    }

    pub fn list_agents(&self) -> anyhow::Result<Vec<AgentInfo>> {
        self.get_json("/agent", "list agents")
    }

    pub fn list_execution_modes(&self) -> anyhow::Result<Vec<ExecutionModeInfo>> {
        self.get_json("/mode", "list execution modes")
    }

    pub fn list_skills(
        &self,
        query: Option<&SkillCatalogQuery>,
    ) -> anyhow::Result<Vec<SkillCatalogEntry>> {
        let url = server_url(&self.base_url, "/skill/catalog");
        let response = match query {
            Some(query) => self.client.get(&url).query(query).send()?,
            None => self.client.get(&url).send()?,
        };
        Self::json_ok(response, "list skills")
    }

    pub fn get_skill_detail(
        &self,
        query: &SkillDetailQuery,
    ) -> anyhow::Result<SkillDetailResponse> {
        self.get_json_query(
            "/skill/detail",
            query,
            &format!("fetch skill detail `{}`", query.name),
        )
    }

    pub fn manage_skill(&self, req: &SkillManageRequest) -> anyhow::Result<SkillManageResponse> {
        self.post_json("/skill/manage", "manage skill", req)
    }

    pub fn list_memory(
        &self,
        query: Option<&MemoryListQuery>,
    ) -> anyhow::Result<MemoryListResponse> {
        let url = server_url(&self.base_url, "/memory/list");
        let response = match query {
            Some(query) => self.client.get(&url).query(query).send()?,
            None => self.client.get(&url).send()?,
        };
        Self::json_ok(response, "list memory")
    }

    pub fn search_memory(
        &self,
        query: Option<&MemoryListQuery>,
    ) -> anyhow::Result<MemoryListResponse> {
        let url = server_url(&self.base_url, "/memory/search");
        let response = match query {
            Some(query) => self.client.get(&url).query(query).send()?,
            None => self.client.get(&url).send()?,
        };
        Self::json_ok(response, "search memory")
    }

    pub fn get_memory_retrieval_preview(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> anyhow::Result<MemoryRetrievalPreviewResponse> {
        self.get_json_query(
            "/memory/retrieval-preview",
            query,
            "fetch memory retrieval preview",
        )
    }

    pub fn get_memory_detail(&self, id: &str) -> anyhow::Result<MemoryDetailView> {
        self.get_json(
            &format!("/memory/{}", id),
            &format!("fetch memory detail `{}`", id),
        )
    }

    pub fn get_memory_validation_report(
        &self,
        id: &str,
    ) -> anyhow::Result<MemoryValidationReportResponse> {
        self.get_json(
            &format!("/memory/{}/validation-report", id),
            &format!("fetch memory validation report `{}`", id),
        )
    }

    pub fn get_memory_conflicts(&self, id: &str) -> anyhow::Result<MemoryConflictResponse> {
        self.get_json(
            &format!("/memory/{}/conflicts", id),
            &format!("fetch memory conflicts `{}`", id),
        )
    }

    pub fn list_memory_rule_packs(&self) -> anyhow::Result<MemoryRulePackListResponse> {
        self.get_json("/memory/rule-packs", "fetch memory rule packs")
    }

    pub fn list_memory_rule_hits(
        &self,
        query: Option<&MemoryRuleHitQuery>,
    ) -> anyhow::Result<MemoryRuleHitListResponse> {
        let url = server_url(&self.base_url, "/memory/rule-hits");
        let response = match query {
            Some(query) => self.client.get(&url).query(query).send()?,
            None => self.client.get(&url).send()?,
        };
        Self::json_ok(response, "fetch memory rule hits")
    }

    pub fn list_memory_consolidation_runs(
        &self,
        query: Option<&MemoryConsolidationRunQuery>,
    ) -> anyhow::Result<MemoryConsolidationRunListResponse> {
        let url = server_url(&self.base_url, "/memory/consolidation/runs");
        let response = match query {
            Some(query) => self.client.get(&url).query(query).send()?,
            None => self.client.get(&url).send()?,
        };
        Self::json_ok(response, "fetch memory consolidation runs")
    }

    pub fn run_memory_consolidation(
        &self,
        request: &MemoryConsolidationRequest,
    ) -> anyhow::Result<MemoryConsolidationResponse> {
        self.post_json("/memory/consolidate", "run memory consolidation", request)
    }

    pub fn list_skill_hub_managed(&self) -> anyhow::Result<SkillHubManagedResponse> {
        self.get_json("/skill/hub/managed", "fetch skill hub managed state")
    }

    pub fn list_skill_hub_usage(&self) -> anyhow::Result<SkillHubUsageLedgerResponse> {
        self.get_json("/skill/hub/usage", "fetch skill hub usage ledger")
    }

    pub fn list_skill_hub_negative_entropy(
        &self,
    ) -> anyhow::Result<SkillHubNegativeEntropyResponse> {
        self.get_json(
            "/skill/hub/negative-entropy",
            "fetch skill hub negative entropy diagnostics",
        )
    }

    pub fn sync_skill_hub_review_candidates(
        &self,
        req: &SkillHubReviewCandidatesSyncRequest,
    ) -> anyhow::Result<SkillHubReviewCandidatesSyncResponse> {
        self.post_json(
            "/skill/hub/review-candidates/sync",
            "sync skill hub review candidates",
            req,
        )
    }

    pub fn list_skill_hub_semantic_conflicts(
        &self,
    ) -> anyhow::Result<SkillHubSemanticConflictResponse> {
        self.get_json(
            "/skill/hub/semantic-conflicts",
            "fetch skill hub semantic conflict diagnostics",
        )
    }

    pub fn update_skill_hub_vitality(
        &self,
        req: &SkillHubVitalityUpdateRequest,
    ) -> anyhow::Result<SkillHubVitalityUpdateResponse> {
        self.post_json("/skill/hub/vitality", "update skill hub vitality", req)
    }

    pub fn list_skill_hub_index(&self) -> anyhow::Result<SkillHubIndexResponse> {
        self.get_json("/skill/hub/index", "fetch skill hub source index")
    }

    pub fn list_skill_hub_distributions(&self) -> anyhow::Result<SkillHubDistributionResponse> {
        self.get_json("/skill/hub/distributions", "fetch skill hub distributions")
    }

    pub fn list_skill_hub_artifact_cache(&self) -> anyhow::Result<SkillHubArtifactCacheResponse> {
        self.get_json(
            "/skill/hub/artifact-cache",
            "fetch skill hub artifact cache",
        )
    }

    pub fn list_skill_hub_policy(&self) -> anyhow::Result<SkillHubPolicyResponse> {
        self.get_json("/skill/hub/policy", "fetch skill hub policy")
    }

    pub fn list_skill_hub_lifecycle(&self) -> anyhow::Result<SkillHubLifecycleResponse> {
        self.get_json("/skill/hub/lifecycle", "fetch skill hub lifecycle")
    }

    pub fn refresh_skill_hub_index(
        &self,
        req: &SkillHubIndexRefreshRequest,
    ) -> anyhow::Result<SkillHubIndexRefreshResponse> {
        self.post_json("/skill/hub/index/refresh", "refresh skill hub index", req)
    }

    pub fn list_skill_hub_audit(&self) -> anyhow::Result<SkillHubAuditResponse> {
        self.get_json("/skill/hub/audit", "fetch skill hub audit")
    }

    pub fn list_skill_hub_timeline(
        &self,
        query: &SkillHubTimelineQuery,
    ) -> anyhow::Result<SkillHubTimelineResponse> {
        self.get_json_query(
            "/skill/hub/timeline",
            query,
            "fetch skill hub governance timeline",
        )
    }

    pub fn run_skill_hub_guard(
        &self,
        req: &SkillHubGuardRunRequest,
    ) -> anyhow::Result<SkillHubGuardRunResponse> {
        self.post_json("/skill/hub/guard/run", "run skill hub guard", req)
    }

    pub fn plan_skill_hub_sync(
        &self,
        req: &SkillHubSyncPlanRequest,
    ) -> anyhow::Result<SkillHubSyncPlanResponse> {
        self.post_json("/skill/hub/sync/plan", "plan skill hub sync", req)
    }

    pub fn apply_skill_hub_sync(
        &self,
        req: &SkillHubSyncApplyRequest,
    ) -> anyhow::Result<SkillHubSyncPlanResponse> {
        self.post_json("/skill/hub/sync/apply", "apply skill hub sync", req)
    }

    pub fn plan_skill_hub_remote_install(
        &self,
        req: &SkillHubRemoteInstallPlanRequest,
    ) -> anyhow::Result<SkillRemoteInstallPlan> {
        self.post_json(
            "/skill/hub/install/plan",
            "plan skill hub remote install",
            req,
        )
    }

    pub fn apply_skill_hub_remote_install(
        &self,
        req: &SkillHubRemoteInstallApplyRequest,
    ) -> anyhow::Result<SkillRemoteInstallResponse> {
        self.post_json(
            "/skill/hub/install/apply",
            "apply skill hub remote install",
            req,
        )
    }

    pub fn plan_skill_hub_remote_update(
        &self,
        req: &SkillHubRemoteUpdatePlanRequest,
    ) -> anyhow::Result<SkillRemoteInstallPlan> {
        self.post_json(
            "/skill/hub/update/plan",
            "plan skill hub remote update",
            req,
        )
    }

    pub fn apply_skill_hub_remote_update(
        &self,
        req: &SkillHubRemoteUpdateApplyRequest,
    ) -> anyhow::Result<SkillRemoteInstallResponse> {
        self.post_json(
            "/skill/hub/update/apply",
            "apply skill hub remote update",
            req,
        )
    }

    pub fn detach_skill_hub_managed(
        &self,
        req: &SkillHubManagedDetachRequest,
    ) -> anyhow::Result<SkillHubManagedDetachResponse> {
        self.post_json("/skill/hub/detach", "detach skill hub managed skill", req)
    }

    pub fn remove_skill_hub_managed(
        &self,
        req: &SkillHubManagedRemoveRequest,
    ) -> anyhow::Result<SkillHubManagedRemoveResponse> {
        self.post_json("/skill/hub/remove", "remove skill hub managed skill", req)
    }

    pub fn get_mcp_status(&self) -> anyhow::Result<Vec<McpStatusInfo>> {
        let mut servers: Vec<McpStatusInfo> = self
            .get_json::<HashMap<String, McpStatusInfo>>("/mcp", "fetch MCP status")?
            .into_values()
            .collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub fn start_mcp_auth(&self, name: &str) -> anyhow::Result<McpAuthStartInfo> {
        self.post_json_no_body(
            &format!("/mcp/{}/auth", name),
            &format!("start MCP auth `{}`", name),
        )
    }

    pub fn authenticate_mcp(&self, name: &str) -> anyhow::Result<McpStatusInfo> {
        self.post_json_no_body(
            &format!("/mcp/{}/auth/authenticate", name),
            &format!("authenticate MCP `{}`", name),
        )
    }

    pub fn remove_mcp_auth(&self, name: &str) -> anyhow::Result<bool> {
        let response = self.delete_expect_success(
            &format!("/mcp/{}/auth", name),
            &format!("remove MCP auth `{}`", name),
        )?;
        let value = response.json::<serde_json::Value>()?;
        Ok(value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn connect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let response = self.post_expect_success(
            &format!("/mcp/{}/connect", name),
            &format!("connect MCP `{}`", name),
            Option::<&serde_json::Value>::None,
        )?;
        Ok(response.json::<bool>().unwrap_or(true))
    }

    pub fn disconnect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let response = self.post_expect_success(
            &format!("/mcp/{}/disconnect", name),
            &format!("disconnect MCP `{}`", name),
            Option::<&serde_json::Value>::None,
        )?;
        Ok(response.json::<bool>().unwrap_or(true))
    }

    pub fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageInfo>> {
        self.get_messages_after(session_id, None, None)
    }

    pub fn get_messages_after(
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
        let request = if params.is_empty() {
            self.client.get(&url)
        } else {
            self.client.get(&url).query(&params)
        };
        Self::json_ok(request.send()?, "get messages")
    }

    pub fn get_lsp_servers(&self) -> anyhow::Result<Vec<String>> {
        let status: LspStatusResponse = self.get_json("/lsp", "get LSP status")?;
        Ok(status.servers)
    }

    pub fn get_formatters(&self) -> anyhow::Result<Vec<String>> {
        let status: FormatterStatusResponse =
            self.get_json("/formatter", "get formatter status")?;
        Ok(status.formatters)
    }

    pub fn share_session(&self, session_id: &str) -> anyhow::Result<ShareResponse> {
        self.post_json_no_body(
            &format!("/session/{}/share", session_id),
            &format!("share session `{}`", session_id),
        )
    }

    pub fn unshare_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let response = self.delete_expect_success(
            &format!("/session/{}/share", session_id),
            &format!("unshare session `{}`", session_id),
        )?;
        let value = response.json::<serde_json::Value>()?;
        Ok(value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn compact_session(
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
        let resp = req.send()?;
        Self::json_ok(resp, &format!("compact session `{}`", session_id))
    }

    pub fn revert_session(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> anyhow::Result<RevertResponse> {
        self.post_json(
            &format!("/session/{}/revert", session_id),
            &format!("revert session `{}`", session_id),
            &RevertRequest {
                message_id: message_id.to_string(),
            },
        )
    }

    pub fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<SessionInfo> {
        let url = server_url(&self.base_url, &format!("/session/{}/fork", session_id));
        let request = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "message_id": message_id }));
        Self::json_ok(request.send()?, &format!("fork session `{}`", session_id))
    }

    pub fn set_current_session(&self, session: SessionInfo) {
        if let Ok(mut current) = self.current_session.write() {
            *current = Some(session);
        }
    }

    pub fn get_current_session(&self) -> Option<SessionInfo> {
        self.current_session
            .read()
            .ok()
            .and_then(|current| current.clone())
    }

    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        action: &str,
    ) -> anyhow::Result<T> {
        let response = self.client.get(server_url(&self.base_url, path)).send()?;
        Self::json_ok(response, action)
    }

    fn get_json_query<T: serde::de::DeserializeOwned, Q: Serialize + ?Sized>(
        &self,
        path: &str,
        query: &Q,
        action: &str,
    ) -> anyhow::Result<T> {
        let response = self
            .client
            .get(server_url(&self.base_url, path))
            .query(query)
            .send()?;
        Self::json_ok(response, action)
    }

    fn post_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let response = self.post_expect_success(path, action, Some(body))?;
        Ok(response.json::<T>()?)
    }

    fn post_json_no_body<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        action: &str,
    ) -> anyhow::Result<T> {
        let response =
            self.post_expect_success(path, action, Option::<&serde_json::Value>::None)?;
        Ok(response.json::<T>()?)
    }

    fn post_unit<B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: Option<&B>,
    ) -> anyhow::Result<()> {
        let _ = self.post_expect_success(path, action, body)?;
        Ok(())
    }

    fn post_expect_success<B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: Option<&B>,
    ) -> anyhow::Result<Response> {
        let url = server_url(&self.base_url, path);
        let request = self.client.post(url);
        let response = match body {
            Some(body) => request.json(body).send()?,
            None => request.send()?,
        };
        Self::expect_success(response, action)
    }

    fn patch_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let response = self
            .client
            .patch(server_url(&self.base_url, path))
            .json(body)
            .send()?;
        Self::json_ok(response, action)
    }

    fn put_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        action: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let response = self
            .client
            .put(server_url(&self.base_url, path))
            .json(body)
            .send()?;
        Self::json_ok(response, action)
    }

    fn delete_expect_success(&self, path: &str, action: &str) -> anyhow::Result<Response> {
        let response = self
            .client
            .delete(server_url(&self.base_url, path))
            .send()?;
        Self::expect_success(response, action)
    }

    fn expect_success(response: Response, action: &str) -> anyhow::Result<Response> {
        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let text = response
                .text()
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            Err(http_error(action, status, text))
        }
    }

    fn json_ok<T: serde::de::DeserializeOwned>(
        response: Response,
        action: &str,
    ) -> anyhow::Result<T> {
        Ok(Self::expect_success(response, action)?.json::<T>()?)
    }
}
