use rocode_runtime_context::ResolvedWorkspaceContext;

use crate::common::{
    build_connect_provider_request, build_session_list_params, http_error, server_url, HTTP_TIMEOUT,
};
use crate::{
    CompactResponse, CreateSessionRequest, ExecutionModeInfo, FullProviderListResponse,
    McpStatusInfo, MemoryConflictResponse, MemoryConsolidationRequest, MemoryConsolidationResponse,
    MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery, MemoryDetailView,
    MemoryListQuery, MemoryListResponse, MemoryRetrievalPreviewResponse, MemoryRetrievalQuery,
    MemoryRuleHitListResponse, MemoryRuleHitQuery, MemoryRulePackListResponse,
    MemoryValidationReportResponse, MultimodalPreflightRequest, MultimodalPreflightResponse,
    PromptPart, PromptRequest, PromptResponse, ProviderConnectSchemaResponse, QuestionInfo,
    RefreshProviderCatalogResponse, ResolveProviderConnectRequest, ResolveProviderConnectResponse,
    SessionEventsQuery, SessionInfo, SessionInsightsResponse, SessionListItem, SessionListResponse,
    SessionTelemetrySnapshot, ShareResponse, SkillCatalogEntry, SkillCatalogQuery,
    SkillDetailQuery, SkillDetailResponse, SkillHubArtifactCacheResponse, SkillHubAuditResponse,
    SkillHubDistributionResponse, SkillHubGuardRunRequest, SkillHubGuardRunResponse,
    SkillHubIndexRefreshRequest, SkillHubIndexRefreshResponse, SkillHubIndexResponse,
    SkillHubLifecycleResponse, SkillHubManagedDetachRequest, SkillHubManagedDetachResponse,
    SkillHubManagedRemoveRequest, SkillHubManagedRemoveResponse, SkillHubManagedResponse,
    SkillHubPolicyResponse, SkillHubRemoteInstallApplyRequest, SkillHubRemoteInstallPlanRequest,
    SkillHubRemoteUpdateApplyRequest, SkillHubRemoteUpdatePlanRequest, SkillHubSyncApplyRequest,
    SkillHubSyncPlanRequest, SkillHubSyncPlanResponse, SkillHubTimelineQuery,
    SkillHubTimelineResponse, SkillRemoteInstallPlan, SkillRemoteInstallResponse,
    UpdateSessionRequest,
};

pub struct AsyncApiClient {
    client: reqwest::Client,
    base_url: String,
}

impl AsyncApiClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn create_session(
        &self,
        parent_id: Option<String>,
        scheduler_profile: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        let url = server_url(&self.base_url, "/session");
        let req = CreateSessionRequest {
            parent_id,
            scheduler_profile,
        };
        let resp = self.client.post(&url).json(&req).send().await?;
        Self::json_ok(resp, "create session").await
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

    pub async fn send_prompt(
        &self,
        session_id: &str,
        content: String,
        parts: Option<Vec<PromptPart>>,
        agent: Option<String>,
        scheduler_profile: Option<String>,
        model: Option<String>,
        variant: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/prompt", session_id));
        let req = PromptRequest {
            message: (!content.trim().is_empty()).then_some(content),
            parts,
            agent,
            scheduler_profile,
            model,
            variant,
            command: None,
            arguments: None,
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
    ) -> anyhow::Result<PromptResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/prompt", session_id));
        let req = PromptRequest {
            message: None,
            parts: None,
            agent: None,
            scheduler_profile: None,
            model,
            variant,
            command: Some(command),
            arguments,
        };
        let resp = self.client.post(&url).json(&req).send().await?;
        Self::json_ok(resp, "send command prompt").await
    }

    pub async fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        let url = server_url(&self.base_url, &format!("/session/{}/abort", session_id));
        let resp = self.client.post(&url).send().await?;
        Self::json_ok(resp, "abort session").await
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
    ) -> anyhow::Result<Vec<rocode_command::stage_protocol::StageEvent>> {
        let url = server_url(&self.base_url, &format!("/session/{}/events", session_id));
        let resp = self.client.get(&url).query(query).send().await?;
        Self::json_ok(resp, "get session events").await
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

    pub async fn preflight_multimodal(
        &self,
        request: &MultimodalPreflightRequest,
    ) -> anyhow::Result<MultimodalPreflightResponse> {
        let url = server_url(&self.base_url, "/multimodal/preflight");
        let resp = self.client.post(&url).json(request).send().await?;
        Self::json_ok(resp, "post multimodal preflight").await
    }

    pub async fn get_all_providers(&self) -> anyhow::Result<FullProviderListResponse> {
        let url = server_url(&self.base_url, "/provider");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get all providers").await
    }

    pub async fn get_provider_connect_schema(
        &self,
    ) -> anyhow::Result<ProviderConnectSchemaResponse> {
        let url = server_url(&self.base_url, "/provider/connect/schema");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get provider connect schema").await
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

    pub async fn set_auth(&self, provider_id: &str, api_key: &str) -> anyhow::Result<()> {
        self.connect_provider(provider_id, api_key, None, None)
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

    pub async fn list_skill_hub_managed(&self) -> anyhow::Result<SkillHubManagedResponse> {
        let url = server_url(&self.base_url, "/skill/hub/managed");
        let resp = self.client.get(&url).send().await?;
        Self::json_ok(resp, "get skill hub managed").await
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
        let map: std::collections::HashMap<String, McpStatusInfo> =
            Self::json_ok(resp, "get MCP status").await?;
        let mut servers: Vec<McpStatusInfo> = map.into_values().collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub async fn get_lsp_servers(&self) -> anyhow::Result<Vec<String>> {
        let url = server_url(&self.base_url, "/lsp");
        let resp = self.client.get(&url).send().await?;
        let v: serde_json::Value = Self::json_ok(resp, "get LSP status").await?;
        Ok(v.get("servers")
            .and_then(|s| serde_json::from_value::<Vec<String>>(s.clone()).ok())
            .unwrap_or_default())
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

    pub async fn compact_session(&self, session_id: &str) -> anyhow::Result<CompactResponse> {
        let url = server_url(&self.base_url, &format!("/session/{}/compact", session_id));
        let resp = self.client.post(&url).send().await?;
        Self::json_ok(resp, &format!("compact session `{}`", session_id)).await
    }

    pub async fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<SessionInfo> {
        let url = server_url(&self.base_url, &format!("/session/{}/fork", session_id));
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(msg_id) = message_id {
            params.push(("message_id", msg_id.to_string()));
        }
        let req = if params.is_empty() {
            self.client.post(&url)
        } else {
            self.client.post(&url).query(&params)
        };
        let resp = req.send().await?;
        Self::json_ok(resp, &format!("fork session `{}`", session_id)).await
    }

    async fn expect_success(resp: reqwest::Response, action: &str) -> anyhow::Result<Vec<u8>> {
        let status = resp.status();
        if status.is_success() {
            Ok(resp.bytes().await?.to_vec())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(http_error(action, status, text))
        }
    }

    async fn json_ok<T: serde::de::DeserializeOwned>(
        resp: reqwest::Response,
        action: &str,
    ) -> anyhow::Result<T> {
        let bytes = Self::expect_success(resp, action).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
