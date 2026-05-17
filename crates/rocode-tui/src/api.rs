use std::collections::HashMap;
use std::sync::{mpsc, RwLock};
use std::thread;

pub use rocode_client::*;

type ApiJob = Box<dyn FnOnce(&RuntimeApiClient) + Send + 'static>;

macro_rules! sync_api_methods {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $ret:ty;)+) => {
        $(
            fn $name(&self $(, $arg: $ty)*) -> anyhow::Result<$ret> {
                self.block_on(self.client.$name($($arg),*))
            }
        )+
    };
}

struct RuntimeApiClient {
    runtime: tokio::runtime::Runtime,
    client: rocode_client::AsyncApiClient,
}

impl RuntimeApiClient {
    fn new_with_password(base_url: String, server_password: Option<String>) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to start TUI API gateway runtime");
        Self {
            runtime,
            client: rocode_client::AsyncApiClient::new_with_password(base_url, server_password),
        }
    }

    fn block_on<R>(
        &self,
        future: impl std::future::Future<Output = anyhow::Result<R>>,
    ) -> anyhow::Result<R> {
        self.runtime.block_on(future)
    }

    fn create_session(
        &self,
        scheduler_profile: Option<String>,
        directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        self.block_on(self.client.create_session(scheduler_profile, directory))
    }

    fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        self.block_on(self.client.get_session(session_id))
    }

    fn list_sessions(&self) -> anyhow::Result<Vec<SessionListItem>> {
        self.block_on(self.client.list_sessions(None, None))
    }

    fn list_sessions_filtered(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        self.block_on(self.client.list_sessions(search, limit))
    }

    fn connect_provider(&self, request: &ConnectProviderRequest) -> anyhow::Result<()> {
        self.block_on(self.client.connect_provider(
            &request.provider_id,
            &request.api_key,
            request.base_url.clone(),
            request.protocol.clone(),
        ))
    }

    fn list_skill_proposals(&self, status: &str) -> anyhow::Result<Vec<SkillEvolutionProposal>> {
        self.block_on(self.client.list_skill_proposals(status))
    }

    fn get_provider_descriptor(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<ProviderDescriptorResponse> {
        self.block_on(self.client.get_provider_descriptor(provider_id))
    }

    fn update_skill_proposal_status(
        &self,
        id: &str,
        status: &str,
    ) -> anyhow::Result<SkillEvolutionProposal> {
        self.block_on(self.client.update_skill_proposal_status(id, status))
    }

    sync_api_methods! {
        fn get_session_status(&self) -> HashMap<String, SessionStatusInfo>;
        fn get_session_executions(&self, session_id: &str) -> SessionExecutionTopology;
        fn get_session_runtime(&self, session_id: &str) -> SessionRuntimeState;
        fn get_session_telemetry(&self, session_id: &str) -> SessionTelemetrySnapshot;
        fn get_session_insights(&self, session_id: &str) -> SessionInsightsResponse;
        fn get_session_events(&self, session_id: &str, query: &SessionEventsQuery) -> Vec<rocode_command::stage_protocol::StageEvent>;
        fn get_session_todos(&self, session_id: &str) -> Vec<ApiTodoItem>;
        fn get_session_diff(&self, session_id: &str) -> Vec<ApiDiffEntry>;
        fn get_session_recovery(&self, session_id: &str) -> SessionRecoveryProtocol;
        fn execute_session_recovery(&self, session_id: &str, action: RecoveryActionKind, target_id: Option<String>) -> serde_json::Value;
        fn list_questions(&self) -> Vec<QuestionInfo>;
        fn reply_question(&self, question_id: &str, answers: Vec<Vec<String>>) -> ();
        fn reject_question(&self, question_id: &str) -> ();
        fn list_permissions(&self) -> Vec<PermissionRequestInfo>;
        fn reply_permission(&self, permission_id: &str, reply: &str, message: Option<String>) -> ();
        fn update_session_title(&self, session_id: &str, title: &str) -> SessionInfo;
        fn delete_session(&self, session_id: &str) -> bool;
        fn send_prompt(&self, session_id: &str, content: String, parts: Option<Vec<PromptPart>>, agent: Option<String>, scheduler_profile: Option<String>, model: Option<String>, variant: Option<String>, ingress_source: Option<String>, idempotency_key: Option<String>) -> PromptResponse;
        fn send_command_prompt(&self, session_id: &str, command: String, arguments: Option<String>, model: Option<String>, variant: Option<String>, ingress_source: Option<String>, idempotency_key: Option<String>) -> PromptResponse;
        fn execute_shell(&self, session_id: &str, command: String, workdir: Option<String>) -> serde_json::Value;
        fn abort_session(&self, session_id: &str) -> serde_json::Value;
        fn cancel_tool_call(&self, session_id: &str, tool_call_id: &str) -> serde_json::Value;
        fn get_config_providers(&self) -> ProviderListResponse;
        fn get_config(&self) -> rocode_config::Config;
        fn get_config_validation(&self) -> ConfigPolicyValidationSnapshot;
        fn get_workspace_context(&self) -> rocode_runtime_context::ResolvedWorkspaceContext;
        fn get_multimodal_policy(&self) -> MultimodalPolicyResponse;
        fn get_multimodal_capabilities(&self, model: Option<&str>) -> MultimodalCapabilitiesResponse;
        fn preflight_multimodal(&self, request: &MultimodalPreflightRequest) -> MultimodalPreflightResponse;
        fn get_recent_models(&self) -> Vec<rocode_state::RecentModelEntry>;
        fn put_recent_models(&self, recent_models: &[rocode_state::RecentModelEntry]) -> Vec<rocode_state::RecentModelEntry>;
        fn patch_config(&self, patch: &serde_json::Value) -> rocode_config::Config;
        fn put_provider_model_config(&self, provider_id: &str, model_key: &str, model: &rocode_config::ModelConfig) -> rocode_config::Config;
        fn delete_provider_model_config(&self, provider_id: &str, model_key: &str) -> rocode_config::Config;
        fn get_all_providers(&self) -> FullProviderListResponse;
        fn get_known_providers(&self) -> KnownProvidersResponse;
        fn get_provider_connect_schema(&self) -> ProviderConnectSchemaResponse;
        fn resolve_provider_connect(&self, query: &str) -> ResolveProviderConnectResponse;
        fn refresh_provider_catalog(&self) -> RefreshProviderCatalogResponse;
        fn set_auth(&self, provider_id: &str, api_key: &str) -> ();
        fn register_custom_provider(&self, provider_id: &str, base_url: &str, protocol: &str, api_key: &str) -> ();
        fn list_agents(&self) -> Vec<AgentInfo>;
        fn list_execution_modes(&self) -> Vec<ExecutionModeInfo>;
        fn list_skills(&self, query: Option<&SkillCatalogQuery>) -> Vec<SkillCatalogEntry>;
        fn get_skill_detail(&self, query: &SkillDetailQuery) -> SkillDetailResponse;
        fn manage_skill(&self, req: &SkillManageRequest) -> SkillManageResponse;
        fn list_memory(&self, query: Option<&MemoryListQuery>) -> MemoryListResponse;
        fn search_memory(&self, query: Option<&MemoryListQuery>) -> MemoryListResponse;
        fn get_memory_retrieval_preview(&self, query: &MemoryRetrievalQuery) -> MemoryRetrievalPreviewResponse;
        fn get_memory_detail(&self, id: &str) -> MemoryDetailView;
        fn get_memory_validation_report(&self, id: &str) -> MemoryValidationReportResponse;
        fn get_memory_conflicts(&self, id: &str) -> MemoryConflictResponse;
        fn list_memory_rule_packs(&self) -> MemoryRulePackListResponse;
        fn list_memory_rule_hits(&self, query: Option<&MemoryRuleHitQuery>) -> MemoryRuleHitListResponse;
        fn list_memory_consolidation_runs(&self, query: Option<&MemoryConsolidationRunQuery>) -> MemoryConsolidationRunListResponse;
        fn run_memory_consolidation(&self, request: &MemoryConsolidationRequest) -> MemoryConsolidationResponse;
        fn list_skill_hub_managed(&self) -> SkillHubManagedResponse;
        fn list_skill_hub_usage(&self) -> SkillHubUsageLedgerResponse;
        fn list_skill_hub_index(&self) -> SkillHubIndexResponse;
        fn list_skill_hub_distributions(&self) -> SkillHubDistributionResponse;
        fn list_skill_hub_artifact_cache(&self) -> SkillHubArtifactCacheResponse;
        fn list_skill_hub_policy(&self) -> SkillHubPolicyResponse;
        fn list_skill_hub_lifecycle(&self) -> SkillHubLifecycleResponse;
        fn refresh_skill_hub_index(&self, req: &SkillHubIndexRefreshRequest) -> SkillHubIndexRefreshResponse;
        fn list_skill_hub_audit(&self) -> SkillHubAuditResponse;
        fn list_skill_hub_timeline(&self, query: &SkillHubTimelineQuery) -> SkillHubTimelineResponse;
        fn run_skill_hub_guard(&self, req: &SkillHubGuardRunRequest) -> SkillHubGuardRunResponse;
        fn plan_skill_hub_sync(&self, req: &SkillHubSyncPlanRequest) -> SkillHubSyncPlanResponse;
        fn apply_skill_hub_sync(&self, req: &SkillHubSyncApplyRequest) -> SkillHubSyncPlanResponse;
        fn plan_skill_hub_remote_install(&self, req: &SkillHubRemoteInstallPlanRequest) -> SkillRemoteInstallPlan;
        fn apply_skill_hub_remote_install(&self, req: &SkillHubRemoteInstallApplyRequest) -> SkillRemoteInstallResponse;
        fn plan_skill_hub_remote_update(&self, req: &SkillHubRemoteUpdatePlanRequest) -> SkillRemoteInstallPlan;
        fn apply_skill_hub_remote_update(&self, req: &SkillHubRemoteUpdateApplyRequest) -> SkillRemoteInstallResponse;
        fn detach_skill_hub_managed(&self, req: &SkillHubManagedDetachRequest) -> SkillHubManagedDetachResponse;
        fn remove_skill_hub_managed(&self, req: &SkillHubManagedRemoveRequest) -> SkillHubManagedRemoveResponse;
        fn get_mcp_status(&self) -> Vec<McpStatusInfo>;
        fn start_mcp_auth(&self, name: &str) -> McpAuthStartInfo;
        fn authenticate_mcp(&self, name: &str) -> McpStatusInfo;
        fn remove_mcp_auth(&self, name: &str) -> bool;
        fn connect_mcp(&self, name: &str) -> bool;
        fn disconnect_mcp(&self, name: &str) -> bool;
        fn get_messages(&self, session_id: &str) -> Vec<MessageInfo>;
        fn get_messages_after(&self, session_id: &str, after: Option<&str>, limit: Option<usize>) -> Vec<MessageInfo>;
        fn get_lsp_servers(&self) -> Vec<String>;
        fn get_formatters(&self) -> Vec<String>;
        fn share_session(&self, session_id: &str) -> ShareResponse;
        fn unshare_session(&self, session_id: &str) -> bool;
        fn compact_session(&self, session_id: &str, focus: Option<&str>) -> CompactResponse;
        fn revert_session(&self, session_id: &str, message_id: &str) -> RevertResponse;
        fn fork_session(&self, session_id: &str, message_id: Option<&str>) -> SessionInfo;
    }
}

/// TUI-owned API gateway.
///
/// The TUI can be driven from async/reactive code, so it must not perform HTTP
/// I/O directly from render/event paths. This gateway owns the async HTTP client
/// and runtime on one dedicated thread while preserving the existing synchronous
/// call surface until call sites are converted to commands and read-model
/// projections.
pub struct ApiClient {
    base_url: String,
    jobs: mpsc::Sender<ApiJob>,
    priority_client: BlockingApiClient,
    current_session: RwLock<Option<SessionInfo>>,
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        Self::new_with_password(base_url, None)
    }

    pub fn new_with_password(base_url: String, server_password: Option<String>) -> Self {
        let (jobs, receiver) = mpsc::channel::<ApiJob>();
        let thread_base_url = base_url.clone();
        let thread_server_password = server_password.clone();
        thread::Builder::new()
            .name("rocode-tui-api-gateway".to_string())
            .spawn(move || {
                let client =
                    RuntimeApiClient::new_with_password(thread_base_url, thread_server_password);
                while let Ok(job) = receiver.recv() {
                    job(&client);
                }
            })
            .expect("failed to start TUI API gateway thread");

        Self {
            priority_client: BlockingApiClient::new_with_password(
                base_url.clone(),
                server_password,
            ),
            base_url,
            jobs,
            current_session: RwLock::new(None),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn call<R, F>(&self, action: &'static str, f: F) -> anyhow::Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&RuntimeApiClient) -> anyhow::Result<R> + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.jobs
            .send(Box::new(move |client| {
                let _ = reply_tx.send(f(client));
            }))
            .map_err(|_| anyhow::anyhow!("TUI API gateway stopped while trying to {action}"))?;
        reply_rx.recv().map_err(|_| {
            anyhow::anyhow!("TUI API gateway dropped response while trying to {action}")
        })?
    }

    pub fn create_session(
        &self,
        scheduler_profile: Option<String>,
        directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        self.call("create session", move |client| {
            client.create_session(scheduler_profile, directory)
        })
    }

    pub fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        let session_id = session_id.to_string();
        self.call("get session", move |client| client.get_session(&session_id))
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionListItem>> {
        self.call("list sessions", |client| client.list_sessions())
    }

    pub fn list_sessions_filtered(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        let search = search.map(str::to_string);
        self.call("list sessions", move |client| {
            client.list_sessions_filtered(search.as_deref(), limit)
        })
    }

    pub fn get_session_status(&self) -> anyhow::Result<HashMap<String, SessionStatusInfo>> {
        self.call("get session status", |client| client.get_session_status())
    }

    pub fn get_session_executions(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionExecutionTopology> {
        let session_id = session_id.to_string();
        self.call("get session executions", move |client| {
            client.get_session_executions(&session_id)
        })
    }

    pub fn get_session_runtime(&self, session_id: &str) -> anyhow::Result<SessionRuntimeState> {
        let session_id = session_id.to_string();
        self.call("get session runtime", move |client| {
            client.get_session_runtime(&session_id)
        })
    }

    pub fn get_session_telemetry(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionTelemetrySnapshot> {
        let session_id = session_id.to_string();
        self.call("get session telemetry", move |client| {
            client.get_session_telemetry(&session_id)
        })
    }

    pub fn get_session_insights(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionInsightsResponse> {
        let session_id = session_id.to_string();
        self.call("get session insights", move |client| {
            client.get_session_insights(&session_id)
        })
    }

    pub fn get_session_events(
        &self,
        session_id: &str,
        query: &SessionEventsQuery,
    ) -> anyhow::Result<Vec<rocode_command::stage_protocol::StageEvent>> {
        let session_id = session_id.to_string();
        let query = query.clone();
        self.call("get session events", move |client| {
            client.get_session_events(&session_id, &query)
        })
    }

    pub fn get_session_todos(&self, session_id: &str) -> anyhow::Result<Vec<ApiTodoItem>> {
        let session_id = session_id.to_string();
        self.call("get session todos", move |client| {
            client.get_session_todos(&session_id)
        })
    }

    pub fn get_session_diff(&self, session_id: &str) -> anyhow::Result<Vec<ApiDiffEntry>> {
        let session_id = session_id.to_string();
        self.call("get session diff", move |client| {
            client.get_session_diff(&session_id)
        })
    }

    pub fn get_session_recovery(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionRecoveryProtocol> {
        let session_id = session_id.to_string();
        self.call("get session recovery", move |client| {
            client.get_session_recovery(&session_id)
        })
    }

    pub fn execute_session_recovery(
        &self,
        session_id: &str,
        action: RecoveryActionKind,
        target_id: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let session_id = session_id.to_string();
        self.call("execute session recovery", move |client| {
            client.execute_session_recovery(&session_id, action, target_id)
        })
    }

    pub fn list_questions(&self) -> anyhow::Result<Vec<QuestionInfo>> {
        self.call("list questions", |client| client.list_questions())
    }

    pub fn reply_question(
        &self,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        let question_id = question_id.to_string();
        self.call("reply question", move |client| {
            client.reply_question(&question_id, answers)
        })
    }

    pub fn reject_question(&self, question_id: &str) -> anyhow::Result<()> {
        let question_id = question_id.to_string();
        self.call("reject question", move |client| {
            client.reject_question(&question_id)
        })
    }

    pub fn list_permissions(&self) -> anyhow::Result<Vec<PermissionRequestInfo>> {
        self.call("list permissions", |client| client.list_permissions())
    }

    pub fn reply_permission(
        &self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        let permission_id = permission_id.to_string();
        let reply = reply.to_string();
        self.call("reply permission", move |client| {
            client.reply_permission(&permission_id, &reply, message)
        })
    }

    /// Submit permission replies outside the shared gateway queue so interactive
    /// confirmations are not head-of-line blocked by telemetry or sync reads.
    pub fn reply_permission_priority(
        &self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        self.priority_client
            .reply_permission(permission_id, reply, message)
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> anyhow::Result<SessionInfo> {
        let session_id = session_id.to_string();
        let title = title.to_string();
        self.call("update session title", move |client| {
            client.update_session_title(&session_id, &title)
        })
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let session_id = session_id.to_string();
        self.call("delete session", move |client| {
            client.delete_session(&session_id)
        })
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
        idempotency_key: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        let session_id = session_id.to_string();
        self.call("send prompt", move |client| {
            client.send_prompt(
                &session_id,
                content,
                parts,
                agent,
                scheduler_profile,
                model,
                variant,
                Some("tui".to_string()),
                idempotency_key,
            )
        })
    }

    pub fn send_command_prompt(
        &self,
        session_id: &str,
        command: String,
        arguments: Option<String>,
        model: Option<String>,
        variant: Option<String>,
        idempotency_key: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        let session_id = session_id.to_string();
        self.call("send command prompt", move |client| {
            client.send_command_prompt(
                &session_id,
                command,
                arguments,
                model,
                variant,
                Some("tui".to_string()),
                idempotency_key,
            )
        })
    }

    pub fn execute_shell(
        &self,
        session_id: &str,
        command: String,
        workdir: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let session_id = session_id.to_string();
        self.call("execute shell", move |client| {
            client.execute_shell(&session_id, command, workdir)
        })
    }

    pub fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        let session_id = session_id.to_string();
        self.call("abort session", move |client| {
            client.abort_session(&session_id)
        })
    }

    pub fn cancel_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let session_id = session_id.to_string();
        let tool_call_id = tool_call_id.to_string();
        self.call("cancel tool call", move |client| {
            client.cancel_tool_call(&session_id, &tool_call_id)
        })
    }

    pub fn get_config_providers(&self) -> anyhow::Result<ProviderListResponse> {
        self.call("get config providers", |client| {
            client.get_config_providers()
        })
    }

    pub fn get_config(&self) -> anyhow::Result<rocode_config::Config> {
        self.call("get config", |client| client.get_config())
    }

    pub fn get_config_validation(&self) -> anyhow::Result<ConfigPolicyValidationSnapshot> {
        self.call("get config validation", |client| {
            client.get_config_validation()
        })
    }

    pub fn get_workspace_context(
        &self,
    ) -> anyhow::Result<rocode_runtime_context::ResolvedWorkspaceContext> {
        self.call("get workspace context", |client| {
            client.get_workspace_context()
        })
    }

    pub fn get_multimodal_policy(&self) -> anyhow::Result<MultimodalPolicyResponse> {
        self.call("get multimodal policy", |client| {
            client.get_multimodal_policy()
        })
    }

    pub fn get_multimodal_capabilities(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<MultimodalCapabilitiesResponse> {
        let model = model.map(str::to_string);
        self.call("get multimodal capabilities", move |client| {
            client.get_multimodal_capabilities(model.as_deref())
        })
    }

    pub fn preflight_multimodal(
        &self,
        request: &MultimodalPreflightRequest,
    ) -> anyhow::Result<MultimodalPreflightResponse> {
        let request = request.clone();
        self.call("preflight multimodal", move |client| {
            client.preflight_multimodal(&request)
        })
    }

    pub fn get_recent_models(&self) -> anyhow::Result<Vec<rocode_state::RecentModelEntry>> {
        self.call("get recent models", |client| client.get_recent_models())
    }

    pub fn put_recent_models(
        &self,
        recent_models: &[rocode_state::RecentModelEntry],
    ) -> anyhow::Result<Vec<rocode_state::RecentModelEntry>> {
        let recent_models = recent_models.to_vec();
        self.call("put recent models", move |client| {
            client.put_recent_models(&recent_models)
        })
    }

    pub fn patch_config(&self, patch: &serde_json::Value) -> anyhow::Result<rocode_config::Config> {
        let patch = patch.clone();
        self.call("patch config", move |client| client.patch_config(&patch))
    }

    pub fn put_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
        model: &rocode_config::ModelConfig,
    ) -> anyhow::Result<rocode_config::Config> {
        let provider_id = provider_id.to_string();
        let model_key = model_key.to_string();
        let model = model.clone();
        self.call("put provider model config", move |client| {
            client.put_provider_model_config(&provider_id, &model_key, &model)
        })
    }

    pub fn delete_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> anyhow::Result<rocode_config::Config> {
        let provider_id = provider_id.to_string();
        let model_key = model_key.to_string();
        self.call("delete provider model config", move |client| {
            client.delete_provider_model_config(&provider_id, &model_key)
        })
    }

    pub fn get_all_providers(&self) -> anyhow::Result<FullProviderListResponse> {
        self.call("get all providers", |client| client.get_all_providers())
    }

    pub fn get_known_providers(&self) -> anyhow::Result<KnownProvidersResponse> {
        self.call("get known providers", |client| client.get_known_providers())
    }

    pub fn get_provider_connect_schema(&self) -> anyhow::Result<ProviderConnectSchemaResponse> {
        self.call("get provider connect schema", |client| {
            client.get_provider_connect_schema()
        })
    }

    pub fn get_provider_descriptor(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<ProviderDescriptorResponse> {
        let provider_id = provider_id.to_string();
        self.call("get provider descriptor", move |client| {
            client.get_provider_descriptor(&provider_id)
        })
    }

    pub fn resolve_provider_connect(
        &self,
        query: &str,
    ) -> anyhow::Result<ResolveProviderConnectResponse> {
        let query = query.to_string();
        self.call("resolve provider connect", move |client| {
            client.resolve_provider_connect(&query)
        })
    }

    pub fn refresh_provider_catalog(&self) -> anyhow::Result<RefreshProviderCatalogResponse> {
        self.call("refresh provider catalog", |client| {
            client.refresh_provider_catalog()
        })
    }

    pub fn set_auth(&self, provider_id: &str, api_key: &str) -> anyhow::Result<()> {
        let provider_id = provider_id.to_string();
        let api_key = api_key.to_string();
        self.call("set auth", move |client| {
            client.set_auth(&provider_id, &api_key)
        })
    }

    pub fn register_custom_provider(
        &self,
        provider_id: &str,
        base_url: &str,
        protocol: &str,
        api_key: &str,
    ) -> anyhow::Result<()> {
        let provider_id = provider_id.to_string();
        let base_url = base_url.to_string();
        let protocol = protocol.to_string();
        let api_key = api_key.to_string();
        self.call("register custom provider", move |client| {
            client.register_custom_provider(&provider_id, &base_url, &protocol, &api_key)
        })
    }

    pub fn connect_provider(&self, request: &ConnectProviderRequest) -> anyhow::Result<()> {
        let request = request.clone();
        self.call("connect provider", move |client| {
            client.connect_provider(&request)
        })
    }

    pub fn list_agents(&self) -> anyhow::Result<Vec<AgentInfo>> {
        self.call("list agents", |client| client.list_agents())
    }

    pub fn list_execution_modes(&self) -> anyhow::Result<Vec<ExecutionModeInfo>> {
        self.call("list execution modes", |client| {
            client.list_execution_modes()
        })
    }

    pub fn list_skills(
        &self,
        query: Option<&SkillCatalogQuery>,
    ) -> anyhow::Result<Vec<SkillCatalogEntry>> {
        let query = query.cloned();
        self.call("list skills", move |client| {
            client.list_skills(query.as_ref())
        })
    }

    pub fn get_skill_detail(
        &self,
        query: &SkillDetailQuery,
    ) -> anyhow::Result<SkillDetailResponse> {
        let query = query.clone();
        self.call("get skill detail", move |client| {
            client.get_skill_detail(&query)
        })
    }

    pub fn manage_skill(&self, req: &SkillManageRequest) -> anyhow::Result<SkillManageResponse> {
        let req = req.clone();
        self.call("manage skill", move |client| client.manage_skill(&req))
    }

    pub fn list_skill_proposals(
        &self,
        status: &str,
    ) -> anyhow::Result<Vec<SkillEvolutionProposal>> {
        let status = status.to_string();
        self.call("list skill proposals", move |client| {
            client.list_skill_proposals(&status)
        })
    }

    pub fn update_skill_proposal_status(
        &self,
        id: &str,
        status: &str,
    ) -> anyhow::Result<SkillEvolutionProposal> {
        let id = id.to_string();
        let status = status.to_string();
        self.call("update skill proposal status", move |client| {
            client.update_skill_proposal_status(&id, &status)
        })
    }

    pub fn list_memory(
        &self,
        query: Option<&MemoryListQuery>,
    ) -> anyhow::Result<MemoryListResponse> {
        let query = query.cloned();
        self.call("list memory", move |client| {
            client.list_memory(query.as_ref())
        })
    }

    pub fn search_memory(
        &self,
        query: Option<&MemoryListQuery>,
    ) -> anyhow::Result<MemoryListResponse> {
        let query = query.cloned();
        self.call("search memory", move |client| {
            client.search_memory(query.as_ref())
        })
    }

    pub fn get_memory_retrieval_preview(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> anyhow::Result<MemoryRetrievalPreviewResponse> {
        let query = query.clone();
        self.call("get memory retrieval preview", move |client| {
            client.get_memory_retrieval_preview(&query)
        })
    }

    pub fn get_memory_detail(&self, id: &str) -> anyhow::Result<MemoryDetailView> {
        let id = id.to_string();
        self.call("get memory detail", move |client| {
            client.get_memory_detail(&id)
        })
    }

    pub fn get_memory_validation_report(
        &self,
        id: &str,
    ) -> anyhow::Result<MemoryValidationReportResponse> {
        let id = id.to_string();
        self.call("get memory validation report", move |client| {
            client.get_memory_validation_report(&id)
        })
    }

    pub fn get_memory_conflicts(&self, id: &str) -> anyhow::Result<MemoryConflictResponse> {
        let id = id.to_string();
        self.call("get memory conflicts", move |client| {
            client.get_memory_conflicts(&id)
        })
    }

    pub fn list_memory_rule_packs(&self) -> anyhow::Result<MemoryRulePackListResponse> {
        self.call("list memory rule packs", |client| {
            client.list_memory_rule_packs()
        })
    }

    pub fn list_memory_rule_hits(
        &self,
        query: Option<&MemoryRuleHitQuery>,
    ) -> anyhow::Result<MemoryRuleHitListResponse> {
        let query = query.cloned();
        self.call("list memory rule hits", move |client| {
            client.list_memory_rule_hits(query.as_ref())
        })
    }

    pub fn list_memory_consolidation_runs(
        &self,
        query: Option<&MemoryConsolidationRunQuery>,
    ) -> anyhow::Result<MemoryConsolidationRunListResponse> {
        let query = query.cloned();
        self.call("list memory consolidation runs", move |client| {
            client.list_memory_consolidation_runs(query.as_ref())
        })
    }

    pub fn run_memory_consolidation(
        &self,
        request: &MemoryConsolidationRequest,
    ) -> anyhow::Result<MemoryConsolidationResponse> {
        let request = request.clone();
        self.call("run memory consolidation", move |client| {
            client.run_memory_consolidation(&request)
        })
    }

    pub fn list_skill_hub_managed(&self) -> anyhow::Result<SkillHubManagedResponse> {
        self.call("list skill hub managed", |client| {
            client.list_skill_hub_managed()
        })
    }

    pub fn list_skill_hub_usage(&self) -> anyhow::Result<SkillHubUsageLedgerResponse> {
        self.call("list skill hub usage", |client| {
            client.list_skill_hub_usage()
        })
    }

    pub fn list_skill_hub_index(&self) -> anyhow::Result<SkillHubIndexResponse> {
        self.call("list skill hub index", |client| {
            client.list_skill_hub_index()
        })
    }

    pub fn list_skill_hub_distributions(&self) -> anyhow::Result<SkillHubDistributionResponse> {
        self.call("list skill hub distributions", |client| {
            client.list_skill_hub_distributions()
        })
    }

    pub fn list_skill_hub_artifact_cache(&self) -> anyhow::Result<SkillHubArtifactCacheResponse> {
        self.call("list skill hub artifact cache", |client| {
            client.list_skill_hub_artifact_cache()
        })
    }

    pub fn list_skill_hub_policy(&self) -> anyhow::Result<SkillHubPolicyResponse> {
        self.call("list skill hub policy", |client| {
            client.list_skill_hub_policy()
        })
    }

    pub fn list_skill_hub_lifecycle(&self) -> anyhow::Result<SkillHubLifecycleResponse> {
        self.call("list skill hub lifecycle", |client| {
            client.list_skill_hub_lifecycle()
        })
    }

    pub fn refresh_skill_hub_index(
        &self,
        req: &SkillHubIndexRefreshRequest,
    ) -> anyhow::Result<SkillHubIndexRefreshResponse> {
        let req = req.clone();
        self.call("refresh skill hub index", move |client| {
            client.refresh_skill_hub_index(&req)
        })
    }

    pub fn list_skill_hub_audit(&self) -> anyhow::Result<SkillHubAuditResponse> {
        self.call("list skill hub audit", |client| {
            client.list_skill_hub_audit()
        })
    }

    pub fn list_skill_hub_timeline(
        &self,
        query: &SkillHubTimelineQuery,
    ) -> anyhow::Result<SkillHubTimelineResponse> {
        let query = query.clone();
        self.call("list skill hub timeline", move |client| {
            client.list_skill_hub_timeline(&query)
        })
    }

    pub fn run_skill_hub_guard(
        &self,
        req: &SkillHubGuardRunRequest,
    ) -> anyhow::Result<SkillHubGuardRunResponse> {
        let req = req.clone();
        self.call("run skill hub guard", move |client| {
            client.run_skill_hub_guard(&req)
        })
    }

    pub fn plan_skill_hub_sync(
        &self,
        req: &SkillHubSyncPlanRequest,
    ) -> anyhow::Result<SkillHubSyncPlanResponse> {
        let req = req.clone();
        self.call("plan skill hub sync", move |client| {
            client.plan_skill_hub_sync(&req)
        })
    }

    pub fn apply_skill_hub_sync(
        &self,
        req: &SkillHubSyncApplyRequest,
    ) -> anyhow::Result<SkillHubSyncPlanResponse> {
        let req = req.clone();
        self.call("apply skill hub sync", move |client| {
            client.apply_skill_hub_sync(&req)
        })
    }

    pub fn plan_skill_hub_remote_install(
        &self,
        req: &SkillHubRemoteInstallPlanRequest,
    ) -> anyhow::Result<SkillRemoteInstallPlan> {
        let req = req.clone();
        self.call("plan skill hub remote install", move |client| {
            client.plan_skill_hub_remote_install(&req)
        })
    }

    pub fn apply_skill_hub_remote_install(
        &self,
        req: &SkillHubRemoteInstallApplyRequest,
    ) -> anyhow::Result<SkillRemoteInstallResponse> {
        let req = req.clone();
        self.call("apply skill hub remote install", move |client| {
            client.apply_skill_hub_remote_install(&req)
        })
    }

    pub fn plan_skill_hub_remote_update(
        &self,
        req: &SkillHubRemoteUpdatePlanRequest,
    ) -> anyhow::Result<SkillRemoteInstallPlan> {
        let req = req.clone();
        self.call("plan skill hub remote update", move |client| {
            client.plan_skill_hub_remote_update(&req)
        })
    }

    pub fn apply_skill_hub_remote_update(
        &self,
        req: &SkillHubRemoteUpdateApplyRequest,
    ) -> anyhow::Result<SkillRemoteInstallResponse> {
        let req = req.clone();
        self.call("apply skill hub remote update", move |client| {
            client.apply_skill_hub_remote_update(&req)
        })
    }

    pub fn detach_skill_hub_managed(
        &self,
        req: &SkillHubManagedDetachRequest,
    ) -> anyhow::Result<SkillHubManagedDetachResponse> {
        let req = req.clone();
        self.call("detach skill hub managed", move |client| {
            client.detach_skill_hub_managed(&req)
        })
    }

    pub fn remove_skill_hub_managed(
        &self,
        req: &SkillHubManagedRemoveRequest,
    ) -> anyhow::Result<SkillHubManagedRemoveResponse> {
        let req = req.clone();
        self.call("remove skill hub managed", move |client| {
            client.remove_skill_hub_managed(&req)
        })
    }

    pub fn get_mcp_status(&self) -> anyhow::Result<Vec<McpStatusInfo>> {
        self.call("get MCP status", |client| client.get_mcp_status())
    }

    pub fn start_mcp_auth(&self, name: &str) -> anyhow::Result<McpAuthStartInfo> {
        let name = name.to_string();
        self.call("start MCP auth", move |client| client.start_mcp_auth(&name))
    }

    pub fn authenticate_mcp(&self, name: &str) -> anyhow::Result<McpStatusInfo> {
        let name = name.to_string();
        self.call("authenticate MCP", move |client| {
            client.authenticate_mcp(&name)
        })
    }

    pub fn remove_mcp_auth(&self, name: &str) -> anyhow::Result<bool> {
        let name = name.to_string();
        self.call("remove MCP auth", move |client| {
            client.remove_mcp_auth(&name)
        })
    }

    pub fn connect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let name = name.to_string();
        self.call("connect MCP", move |client| client.connect_mcp(&name))
    }

    pub fn disconnect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let name = name.to_string();
        self.call("disconnect MCP", move |client| client.disconnect_mcp(&name))
    }

    pub fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageInfo>> {
        let session_id = session_id.to_string();
        self.call("get messages", move |client| {
            client.get_messages(&session_id)
        })
    }

    pub fn get_messages_after(
        &self,
        session_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        let session_id = session_id.to_string();
        let after = after.map(str::to_string);
        self.call("get messages after", move |client| {
            client.get_messages_after(&session_id, after.as_deref(), limit)
        })
    }

    pub fn get_lsp_servers(&self) -> anyhow::Result<Vec<String>> {
        self.call("get LSP servers", |client| client.get_lsp_servers())
    }

    pub fn get_formatters(&self) -> anyhow::Result<Vec<String>> {
        self.call("get formatters", |client| client.get_formatters())
    }

    pub fn share_session(&self, session_id: &str) -> anyhow::Result<ShareResponse> {
        let session_id = session_id.to_string();
        self.call("share session", move |client| {
            client.share_session(&session_id)
        })
    }

    pub fn unshare_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let session_id = session_id.to_string();
        self.call("unshare session", move |client| {
            client.unshare_session(&session_id)
        })
    }

    pub fn compact_session(
        &self,
        session_id: &str,
        focus: Option<&str>,
    ) -> anyhow::Result<CompactResponse> {
        let session_id = session_id.to_string();
        let focus = focus.map(str::to_string);
        self.call("compact session", move |client| {
            client.compact_session(&session_id, focus.as_deref())
        })
    }

    pub fn revert_session(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> anyhow::Result<RevertResponse> {
        let session_id = session_id.to_string();
        let message_id = message_id.to_string();
        self.call("revert session", move |client| {
            client.revert_session(&session_id, &message_id)
        })
    }

    pub fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<SessionInfo> {
        let session_id = session_id.to_string();
        let message_id = message_id.map(str::to_string);
        self.call("fork session", move |client| {
            client.fork_session(&session_id, message_id.as_deref())
        })
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
}
