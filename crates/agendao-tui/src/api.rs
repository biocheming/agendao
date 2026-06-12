use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, RwLock};
use std::thread;

use agendao_stage_protocol::StageEvent;

pub use agendao_client::*;

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

fn local_workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

struct RuntimeApiClient {
    runtime: tokio::runtime::Runtime,
    client: agendao_client::AsyncApiClient,
    /// Optional transport selected via TransportSelector (Unix/HTTP fallback).
    transport: Option<agendao_client::FrontendTransport>,
    /// In-process server runtime for `--local`. This is the authoritative
    /// local execution path because it reuses the server's prompt/session
    /// ingress pipeline instead of the older text-only DirectTransport.
    local_server: Option<std::sync::Arc<crate::local_server_bridge::LocalServerState>>,
}

impl RuntimeApiClient {
    fn build_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start TUI API gateway runtime")
    }

    fn get_messages_after(
        &self,
        session_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            let after = after.map(str::to_string);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_messages(state, &session_id, after, limit)
                    .await
            });
        }
        self.block_on(self.client.get_messages_after(session_id, after, limit))
    }

    fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageInfo>> {
        self.get_messages_after(session_id, None, None)
    }

    fn new_local_for_workspace(workspace_root: PathBuf) -> Self {
        let runtime = Self::build_runtime();

        let local_server = Some(
            runtime
                .block_on(async {
                    crate::local_server_bridge::new_local_server_for_workspace(workspace_root).await
                })
                .expect("failed to initialize in-process server state for --local"),
        );

        let client = agendao_client::AsyncApiClient::new_with_password(
            "http://127.0.0.1:0".to_string(),
            None,
        );

        Self {
            runtime,
            client,
            transport: None,
            local_server,
        }
    }

    fn new_local_with_server(
        local_server: std::sync::Arc<crate::local_server_bridge::LocalServerState>,
    ) -> Self {
        let runtime = Self::build_runtime();
        let client = agendao_client::AsyncApiClient::new_with_password(
            "http://127.0.0.1:0".to_string(),
            None,
        );
        Self {
            runtime,
            client,
            transport: None,
            local_server: Some(local_server),
        }
    }

    /// Constructor for local (in-process server) mode — no server process, no IPC.
    fn new_local() -> Self {
        Self::new_local_for_workspace(local_workspace_root())
    }

    fn new_with_password(
        base_url: String,
        server_password: Option<String>,
        unix_socket_path: Option<String>,
    ) -> anyhow::Result<Self> {
        let runtime = Self::build_runtime();
        let transport = if let Some(path) = unix_socket_path {
            let selector = agendao_client::transport::TransportSelector::new(
                Some(path),
                base_url.clone(),
                server_password.clone(),
            );
            Some(runtime.block_on(async { selector.select_unix_required().await })?)
        } else {
            None
        };

        Ok(Self {
            runtime,
            client: agendao_client::AsyncApiClient::new_with_password(base_url, server_password),
            transport,
            local_server: None,
        })
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
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_create_session(
                    state,
                    CreateSessionRequest {
                        scheduler_profile,
                        directory,
                        project_id: None,
                        title: None,
                    },
                )
                .await
            });
        }
        self.block_on(self.client.create_session(scheduler_profile, directory))
    }

    fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_get_session(state, &session_id).await
            });
        }
        self.block_on(self.client.get_session(session_id))
    }

    fn get_session_status(&self) -> anyhow::Result<HashMap<String, SessionStatusInfo>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                let status = crate::local_server_bridge::local_get_session_status(state).await?;
                Ok(status
                    .into_iter()
                    .map(|(id, value)| {
                        (
                            id,
                            SessionStatusInfo {
                                status: value.status,
                                idle: value.idle,
                                busy: value.busy,
                                attempt: value.attempt,
                                message: value.message,
                                next: value.next,
                            },
                        )
                    })
                    .collect())
            });
        }
        self.block_on(self.client.get_session_status())
    }

    fn get_session_runtime(&self, session_id: &str) -> anyhow::Result<SessionRuntimeState> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                let runtime: SessionRuntimeState =
                    crate::local_server_bridge::local_get_session_runtime(state, &session_id)
                        .await?;
                serde_json::from_value(serde_json::to_value(runtime)?)
                    .map_err(anyhow::Error::from)
            });
        }
        self.block_on(self.client.get_session_runtime(session_id))
    }

    fn get_session_telemetry(&self, session_id: &str) -> anyhow::Result<SessionTelemetrySnapshot> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                let snapshot: SessionTelemetrySnapshot =
                    crate::local_server_bridge::local_get_session_telemetry(state, &session_id)
                        .await?;
                serde_json::from_value(serde_json::to_value(snapshot)?)
                    .map_err(anyhow::Error::from)
            });
        }
        self.block_on(self.client.get_session_telemetry(session_id))
    }

    fn get_session_todos(&self, session_id: &str) -> anyhow::Result<Vec<ApiTodoItem>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                let todos =
                    crate::local_server_bridge::local_get_session_todos(state, &session_id).await?;
                serde_json::from_value(serde_json::to_value(todos)?)
                    .map_err(anyhow::Error::from)
            });
        }
        self.block_on(self.client.get_session_todos(session_id))
    }

    fn get_session_diff(&self, session_id: &str) -> anyhow::Result<Vec<ApiDiffEntry>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                let diffs =
                    crate::local_server_bridge::local_get_session_diff(state, &session_id).await?;
                serde_json::from_value(serde_json::to_value(diffs)?)
                    .map_err(anyhow::Error::from)
            });
        }
        self.block_on(self.client.get_session_diff(session_id))
    }

    fn send_prompt(
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
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_prompt(
                    state,
                    &session_id,
                    PromptRequest {
                        message: (!content.trim().is_empty()).then_some(content),
                        parts,
                        idempotency_key,
                        ingress_source: ingress_source.or(Some("tui".to_string())),
                        source_origin,
                        source_surface,
                        agent,
                        scheduler_profile,
                        model,
                        variant,
                        command: None,
                        arguments: None,
                    },
                )
                .await
            });
        }
        self.block_on(self.client.send_prompt(
            session_id,
            content,
            parts,
            agent,
            scheduler_profile,
            model,
            variant,
            ingress_source,
            idempotency_key,
            source_origin,
            source_surface,
            None,
        ))
    }

    fn send_command_prompt(
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
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_prompt(
                    state,
                    &session_id,
                    PromptRequest {
                        message: None,
                        parts: None,
                        idempotency_key,
                        ingress_source: ingress_source.or(Some("tui".to_string())),
                        source_origin,
                        source_surface,
                        agent: None,
                        scheduler_profile: None,
                        model,
                        variant,
                        command: Some(command),
                        arguments,
                    },
                )
                .await
            });
        }
        self.block_on(self.client.send_command_prompt(
            session_id,
            command,
            arguments,
            model,
            variant,
            ingress_source,
            idempotency_key,
            source_origin,
            source_surface,
        ))
    }

    fn list_sessions(&self) -> anyhow::Result<Vec<SessionListItem>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_sessions(state, None, None).await
            });
        }
        // Unix socket / HTTP fallback via FrontendTransport.
        if let Some(ref transport) = self.transport {
            if let Ok(items) = self.block_on(transport.list_sessions()) {
                return Ok(items);
            }
        }
        self.block_on(self.client.list_sessions(None, None))
    }

    fn list_sessions_filtered(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let search = search.map(str::to_string);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_sessions(state, search, limit).await
            });
        }
        if search.is_none() {
            if let Some(ref transport) = self.transport {
                if let Ok(items) = self.block_on(transport.list_sessions()) {
                    return Ok(items);
                }
            }
        }
        self.block_on(self.client.list_sessions(search, limit))
    }

    fn connect_provider(&self, request: &ConnectProviderRequest) -> anyhow::Result<()> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let request = request.clone();
            return self.block_on(async move {
                crate::local_server_bridge::local_connect_provider(state, request).await
            });
        }
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
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let provider_id = provider_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_get_provider_descriptor(state, &provider_id).await
            });
        }
        self.block_on(self.client.get_provider_descriptor(provider_id))
    }

    fn list_questions(&self) -> anyhow::Result<Vec<QuestionInfo>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_questions(state).await
            });
        }
        self.block_on(self.client.list_questions())
    }

    fn reply_question(&self, question_id: &str, answers: Vec<Vec<String>>) -> anyhow::Result<()> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let question_id = question_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_reply_question(state, &question_id, answers).await
            });
        }
        self.block_on(self.client.reply_question(question_id, answers))
    }

    fn reject_question(&self, question_id: &str) -> anyhow::Result<()> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let question_id = question_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_reject_question(state, &question_id).await
            });
        }
        self.block_on(self.client.reject_question(question_id))
    }

    fn list_permissions(&self) -> anyhow::Result<Vec<PermissionRequestInfo>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_permissions(state).await
            });
        }
        self.block_on(self.client.list_permissions())
    }

    fn reply_permission(
        &self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let permission_id = permission_id.to_string();
            let reply = reply.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_reply_permission(
                    state,
                    &permission_id,
                    reply,
                    message,
                )
                .await
            });
        }
        self.block_on(self.client.reply_permission(permission_id, reply, message))
    }

    fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let session_id = session_id.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_delete_session(state, &session_id).await
            });
        }
        self.block_on(self.client.delete_session(session_id))
    }

    fn update_skill_proposal_status(
        &self,
        id: &str,
        status: &str,
    ) -> anyhow::Result<SkillEvolutionProposal> {
        self.block_on(self.client.update_skill_proposal_status(id, status))
    }

    sync_api_methods! {
        fn get_session_executions(&self, session_id: &str) -> SessionExecutionTopology;
        fn get_session_insights(&self, session_id: &str) -> SessionInsightsResponse;
        fn get_session_events(&self, session_id: &str, query: &SessionEventsQuery) -> Vec<StageEvent>;
        fn get_session_recovery(&self, session_id: &str) -> SessionRecoveryProtocol;
        fn execute_session_recovery(&self, session_id: &str, action: RecoveryActionKind, target_id: Option<String>) -> serde_json::Value;
        fn update_session_title(&self, session_id: &str, title: &str) -> SessionInfo;
        fn execute_shell(&self, session_id: &str, command: String, workdir: Option<String>) -> serde_json::Value;
        fn abort_session(&self, session_id: &str) -> serde_json::Value;
        fn cancel_tool_call(&self, session_id: &str, tool_call_id: &str) -> serde_json::Value;
        fn patch_config(&self, patch: &serde_json::Value) -> agendao_config::Config;
        fn put_provider_model_config(&self, provider_id: &str, model_key: &str, model: &agendao_config::ModelConfig) -> agendao_config::Config;
        fn delete_provider_model_config(&self, provider_id: &str, model_key: &str) -> agendao_config::Config;
        fn set_auth(&self, provider_id: &str, api_key: &str) -> ();
        fn register_custom_provider(&self, provider_id: &str, base_url: &str, protocol: &str, api_key: &str) -> ();
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
        fn get_lsp_servers(&self) -> Vec<String>;
        fn get_formatters(&self) -> Vec<String>;
        fn share_session(&self, session_id: &str) -> ShareResponse;
        fn unshare_session(&self, session_id: &str) -> bool;
        fn compact_session(&self, session_id: &str, focus: Option<&str>) -> CompactResponse;
        fn revert_session(&self, session_id: &str, message_id: &str) -> RevertResponse;
        fn fork_session(&self, session_id: &str, message_id: Option<&str>) -> SessionInfo;
    }

    fn get_config_providers(&self) -> anyhow::Result<ProviderListResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_config_providers(state).await
            });
        }
        self.block_on(self.client.get_config_providers())
    }

    fn get_config(&self) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_config(state).await
            });
        }
        self.block_on(self.client.get_config())
    }

    fn get_config_validation(&self) -> anyhow::Result<ConfigPolicyValidationSnapshot> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_config_validation(state).await
            });
        }
        self.block_on(self.client.get_config_validation())
    }

    fn list_agents(&self) -> anyhow::Result<Vec<AgentInfo>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_agents(state).await
            });
        }
        if let Some(ref transport) = self.transport {
            if let Ok(items) = self.block_on(transport.list_agents()) {
                return Ok(items);
            }
        }
        self.block_on(self.client.list_agents())
    }

    fn list_execution_modes(&self) -> anyhow::Result<Vec<ExecutionModeInfo>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_list_execution_modes(state).await
            });
        }
        if let Some(ref transport) = self.transport {
            if let Ok(items) = self.block_on(transport.list_execution_modes()) {
                return Ok(items);
            }
        }
        self.block_on(self.client.list_execution_modes())
    }

    fn get_workspace_context(
        &self,
    ) -> anyhow::Result<agendao_runtime_context::ResolvedWorkspaceContext> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_workspace_context(state).await
            });
        }
        if let Some(ref transport) = self.transport {
            if let Ok(context) = self.block_on(transport.get_workspace_context()) {
                return Ok(context);
            }
        }
        self.block_on(self.client.get_workspace_context())
    }

    fn get_multimodal_policy(&self) -> anyhow::Result<MultimodalPolicyResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_multimodal_policy(state).await
            });
        }
        self.block_on(self.client.get_multimodal_policy())
    }

    fn get_multimodal_capabilities(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<MultimodalCapabilitiesResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let model = model.map(str::to_string);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_multimodal_capabilities(state, model).await
            });
        }
        self.block_on(self.client.get_multimodal_capabilities(model))
    }

    fn preflight_multimodal(
        &self,
        request: &MultimodalPreflightRequest,
    ) -> anyhow::Result<MultimodalPreflightResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let request = request.clone();
            return self.block_on(async move {
                crate::local_server_bridge::local_preflight_multimodal(state, request).await
            });
        }
        self.block_on(self.client.preflight_multimodal(request))
    }

    fn get_recent_models(&self) -> anyhow::Result<Vec<agendao_state::RecentModelEntry>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_recent_models(state).await
            });
        }
        if let Some(ref transport) = self.transport {
            if let Ok(items) = self.block_on(transport.get_recent_models()) {
                return Ok(items);
            }
        }
        self.block_on(self.client.get_recent_models())
    }

    fn put_recent_models(
        &self,
        recent_models: &[agendao_state::RecentModelEntry],
    ) -> anyhow::Result<Vec<agendao_state::RecentModelEntry>> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let recent_models = recent_models.to_vec();
            return self.block_on(async move {
                crate::local_server_bridge::local_put_recent_models(state, recent_models).await
            });
        }
        if let Some(ref transport) = self.transport {
            if let Ok(items) = self.block_on(transport.put_recent_models(recent_models)) {
                return Ok(items);
            }
        }
        self.block_on(self.client.put_recent_models(recent_models))
    }

    fn get_all_providers(&self) -> anyhow::Result<FullProviderListResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_all_providers(state).await
            });
        }
        if let Some(ref transport) = self.transport {
            if let Ok(response) = self.block_on(transport.get_all_providers()) {
                return Ok(response);
            }
        }
        self.block_on(self.client.get_all_providers())
    }

    fn get_known_providers(&self) -> anyhow::Result<KnownProvidersResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_known_providers(state).await
            });
        }
        self.block_on(self.client.get_known_providers())
    }

    fn get_provider_connect_schema(&self) -> anyhow::Result<ProviderConnectSchemaResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_get_provider_connect_schema(state).await
            });
        }
        self.block_on(self.client.get_provider_connect_schema())
    }

    fn resolve_provider_connect(
        &self,
        query: &str,
    ) -> anyhow::Result<ResolveProviderConnectResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let query = query.to_string();
            return self.block_on(async move {
                crate::local_server_bridge::local_resolve_provider_connect(state, query).await
            });
        }
        self.block_on(self.client.resolve_provider_connect(query))
    }

    fn refresh_provider_catalog(&self) -> anyhow::Result<RefreshProviderCatalogResponse> {
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            return self.block_on(async move {
                crate::local_server_bridge::local_refresh_provider_catalog(state).await
            });
        }
        self.block_on(self.client.refresh_provider_catalog())
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
    local_server: Option<std::sync::Arc<crate::local_server_bridge::LocalServerState>>,
    current_session: RwLock<Option<SessionInfo>>,
}

impl ApiClient {
    pub fn new(base_url: String) -> anyhow::Result<Self> {
        Self::new_with_password(base_url, None, None)
    }

    /// Direct (in-process) mode — no server, no IPC.
    /// Uses OrchestrationCore<agendao_session::SessionManager> for unified
    /// session authority across all operations.
    pub fn new_local() -> Self {
        Self::new_local_with_server(None)
    }

    pub fn new_local_with_server(
        local_server: Option<std::sync::Arc<crate::local_server_bridge::LocalServerState>>,
    ) -> Self {
        let (jobs, receiver) = mpsc::channel::<ApiJob>();
        let gateway_local_server = local_server.clone();
        thread::Builder::new()
            .name("agendao-tui-api-gateway".to_string())
            .spawn(move || {
                let client = if let Some(local_server) = gateway_local_server {
                    RuntimeApiClient::new_local_with_server(local_server)
                } else {
                    RuntimeApiClient::new_local()
                };
                while let Ok(job) = receiver.recv() {
                    job(&client);
                }
            })
            .expect("failed to start TUI API gateway thread");

        Self {
            priority_client: BlockingApiClient::new_with_password(
                "http://localhost:0".to_string(),
                None,
            ),
            base_url: "direct://local".to_string(),
            jobs,
            local_server,
            current_session: RwLock::new(None),
        }
    }

    pub fn new_with_password(
        base_url: String,
        server_password: Option<String>,
        unix_socket_path: Option<String>,
    ) -> anyhow::Result<Self> {
        let bootstrap_client = RuntimeApiClient::new_with_password(
            base_url.clone(),
            server_password.clone(),
            unix_socket_path.clone(),
        )?;
        let (jobs, receiver) = mpsc::channel::<ApiJob>();
        thread::Builder::new()
            .name("agendao-tui-api-gateway".to_string())
            .spawn(move || {
                let client = bootstrap_client;
                while let Ok(job) = receiver.recv() {
                    job(&client);
                }
            })
            .expect("failed to start TUI API gateway thread");

        Ok(Self {
            priority_client: BlockingApiClient::new_with_password(
                base_url.clone(),
                server_password,
            ),
            base_url,
            jobs,
            local_server: None,
            current_session: RwLock::new(None),
        })
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
    ) -> anyhow::Result<Vec<StageEvent>> {
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
        self.call("list questions", |client| {
            if let Some(ref state) = client.local_server {
                let state = std::sync::Arc::clone(state);
                return client.block_on(async move {
                    crate::local_server_bridge::local_list_questions(state).await
                });
            }
            client.list_questions()
        })
    }

    pub fn reply_question(
        &self,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        let question_id = question_id.to_string();
        self.call("reply question", move |client| {
            if let Some(ref state) = client.local_server {
                let state = std::sync::Arc::clone(state);
                return client.block_on(async move {
                    crate::local_server_bridge::local_reply_question(state, &question_id, answers)
                        .await
                });
            }
            client.reply_question(&question_id, answers)
        })
    }

    pub fn reject_question(&self, question_id: &str) -> anyhow::Result<()> {
        let question_id = question_id.to_string();
        self.call("reject question", move |client| {
            if let Some(ref state) = client.local_server {
                let state = std::sync::Arc::clone(state);
                return client.block_on(async move {
                    crate::local_server_bridge::local_reject_question(state, &question_id).await
                });
            }
            client.reject_question(&question_id)
        })
    }

    pub fn list_permissions(&self) -> anyhow::Result<Vec<PermissionRequestInfo>> {
        self.call("list permissions", |client| {
            if let Some(ref state) = client.local_server {
                let state = std::sync::Arc::clone(state);
                return client.block_on(async move {
                    crate::local_server_bridge::local_list_permissions(state).await
                });
            }
            client.list_permissions()
        })
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
            if let Some(ref state) = client.local_server {
                let state = std::sync::Arc::clone(state);
                return client.block_on(async move {
                    crate::local_server_bridge::local_reply_permission(
                        state,
                        &permission_id,
                        reply,
                        message,
                    )
                    .await
                });
            }
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
        if let Some(ref state) = self.local_server {
            let state = std::sync::Arc::clone(state);
            let permission_id = permission_id.to_string();
            let reply = reply.to_string();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    anyhow::anyhow!(
                        "failed to initialize direct permission reply runtime: {}",
                        error
                    )
                })?;
            return runtime.block_on(async move {
                crate::local_server_bridge::local_reply_permission(
                    state,
                    &permission_id,
                    reply,
                    message,
                )
                .await
            });
        }
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
                Some(agendao_types::MessageSourceOrigin::Operator),
                Some(agendao_types::MessageSourceSurface::Tui),
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
                Some(agendao_types::MessageSourceOrigin::Operator),
                Some(agendao_types::MessageSourceSurface::Tui),
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

    pub fn get_config(&self) -> anyhow::Result<agendao_config::Config> {
        self.call("get config", |client| client.get_config())
    }

    pub fn get_config_validation(&self) -> anyhow::Result<ConfigPolicyValidationSnapshot> {
        self.call("get config validation", |client| {
            client.get_config_validation()
        })
    }

    pub fn get_workspace_context(
        &self,
    ) -> anyhow::Result<agendao_runtime_context::ResolvedWorkspaceContext> {
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

    pub fn get_recent_models(&self) -> anyhow::Result<Vec<agendao_state::RecentModelEntry>> {
        self.call("get recent models", |client| client.get_recent_models())
    }

    pub fn put_recent_models(
        &self,
        recent_models: &[agendao_state::RecentModelEntry],
    ) -> anyhow::Result<Vec<agendao_state::RecentModelEntry>> {
        let recent_models = recent_models.to_vec();
        self.call("put recent models", move |client| {
            client.put_recent_models(&recent_models)
        })
    }

    pub fn patch_config(
        &self,
        patch: &serde_json::Value,
    ) -> anyhow::Result<agendao_config::Config> {
        let patch = patch.clone();
        self.call("patch config", move |client| client.patch_config(&patch))
    }

    pub fn put_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
        model: &agendao_config::ModelConfig,
    ) -> anyhow::Result<agendao_config::Config> {
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
    ) -> anyhow::Result<agendao_config::Config> {
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
        if self.base_url == "direct://local" {
            return Ok(Vec::new());
        }
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
        if self.base_url == "direct://local" {
            return Ok(Vec::new());
        }
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

#[cfg(test)]
mod tests {
    use super::ApiClient;
    #[cfg(feature = "local-server")]
    use super::{MessageInfo, PromptPart, RuntimeApiClient};
    #[cfg(feature = "local-server")]
    use once_cell::sync::Lazy;
    #[cfg(feature = "local-server")]
    use std::path::PathBuf;
    #[cfg(feature = "local-server")]
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(feature = "local-server")]
    use std::sync::Arc;
    #[cfg(feature = "local-server")]
    use std::sync::Mutex;
    #[cfg(feature = "local-server")]
    use std::time::{Duration, Instant};

    #[cfg(feature = "local-server")]
    static LOCAL_ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[cfg(feature = "local-server")]
    struct LocalTestPaths {
        workspace_root: PathBuf,
        data_root: PathBuf,
    }

    #[cfg(feature = "local-server")]
    struct LocalEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        old_agendao_data_dir: Option<String>,
        old_xdg_data_home: Option<String>,
    }

    #[cfg(feature = "local-server")]
    impl Drop for LocalEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.old_agendao_data_dir {
                    Some(value) => std::env::set_var("AGENDAO_DATA_DIR", value),
                    None => std::env::remove_var("AGENDAO_DATA_DIR"),
                }
                match &self.old_xdg_data_home {
                    Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                    None => std::env::remove_var("XDG_DATA_HOME"),
                }
            }
        }
    }

    #[cfg(feature = "local-server")]
    fn test_local_paths() -> LocalTestPaths {
        let root = std::env::temp_dir().join(format!("agendao-tui-local-{}", uuid::Uuid::new_v4()));
        let workspace_root = root.join("workspace");
        let data_root = root.join("data");
        std::fs::create_dir_all(&workspace_root).expect("create temp workspace root");
        std::fs::create_dir_all(&data_root).expect("create temp data root");
        LocalTestPaths {
            workspace_root,
            data_root,
        }
    }

    #[cfg(feature = "local-server")]
    fn install_local_test_env(data_root: &PathBuf) -> LocalEnvGuard {
        let lock = LOCAL_ENV_LOCK.lock().expect("lock local env");
        let old_agendao_data_dir = std::env::var("AGENDAO_DATA_DIR").ok();
        let old_xdg_data_home = std::env::var("XDG_DATA_HOME").ok();
        unsafe {
            std::env::set_var("AGENDAO_DATA_DIR", data_root);
            std::env::set_var("XDG_DATA_HOME", data_root);
        }
        LocalEnvGuard {
            _lock: lock,
            old_agendao_data_dir,
            old_xdg_data_home,
        }
    }

    #[cfg(feature = "local-server")]
    fn wait_for_messages<F>(
        client: &RuntimeApiClient,
        session_id: &str,
        predicate: F,
    ) -> Vec<MessageInfo>
    where
        F: Fn(&[MessageInfo]) -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let messages = client
                .get_messages(session_id)
                .expect("read local messages while waiting");
            if predicate(&messages) {
                return messages;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for expected local messages: {:?}",
                messages
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[cfg(feature = "local-server")]
    struct MockLocalProvider {
        call_count: AtomicUsize,
        model: agendao_provider::ModelInfo,
    }

    #[cfg(feature = "local-server")]
    impl MockLocalProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                model: agendao_provider::ModelInfo {
                    id: "mock-model".to_string(),
                    name: "Mock Model".to_string(),
                    provider: "mock-local".to_string(),
                    context_window: 8192,
                    max_input_tokens: None,
                    max_output_tokens: 4096,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.0,
                    cost_per_million_output: 0.0,
                    cost_per_million_cache_read: None,
                    cost_per_million_cache_write: None,
                },
            }
        }
    }

    #[cfg(feature = "local-server")]
    #[async_trait::async_trait]
    impl agendao_provider::Provider for MockLocalProvider {
        fn id(&self) -> &str {
            "mock-local"
        }

        fn name(&self) -> &str {
            "Mock Local Provider"
        }

        fn models(&self) -> Vec<agendao_provider::ModelInfo> {
            vec![self.model.clone()]
        }

        fn get_model(&self, id: &str) -> Option<&agendao_provider::ModelInfo> {
            (id == self.model.id).then_some(&self.model)
        }

        async fn chat(
            &self,
            _request: agendao_provider::ChatRequest,
        ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(agendao_provider::ChatResponse {
                id: format!("mock-local-{idx}"),
                model: "mock-model".to_string(),
                choices: vec![agendao_provider::Choice {
                    index: 0,
                    message: agendao_provider::Message::assistant("hello from local"),
                    finish_reason: Some("stop".to_string()),
                }],
                usage: Some(agendao_provider::Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                    cache_read_input_tokens: Some(0),
                    cache_miss_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                }),
            })
        }

        async fn chat_stream(
            &self,
            _request: agendao_provider::ChatRequest,
        ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
            Err(agendao_provider::ProviderError::ApiError(
                "streaming not implemented".into(),
            ))
        }
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn local_runtime_get_messages_reads_shared_authority() {
        let paths = test_local_paths();
        let _env = install_local_test_env(&paths.data_root);
        let client = RuntimeApiClient::new_local_for_workspace(paths.workspace_root);
        let state = Arc::clone(client.local_server.as_ref().expect("local server state"));
        client
            .block_on(async {
                crate::local_server_bridge::local_register_provider(
                    &state,
                    Arc::new(MockLocalProvider::new()),
                )
                .await;
                Ok::<(), anyhow::Error>(())
            })
            .expect("register mock local provider");
        let session = client
            .create_session(None, Some(".".to_string()))
            .expect("create local session");

        client
            .send_prompt(
                &session.id,
                "hello local".to_string(),
                None,
                None,
                None,
                Some("mock-local/mock-model".to_string()),
                Some("fast".to_string()),
                Some("tui".to_string()),
                Some("tui_local_1".to_string()),
                Some(agendao_types::MessageSourceOrigin::Operator),
                Some(agendao_types::MessageSourceSurface::Tui),
            )
            .expect("send local prompt");

        let messages = wait_for_messages(&client, &session.id, |messages| {
            messages.iter().any(|message| {
                message.role == "user"
                    && message
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("ingress_idempotency_key"))
                        .and_then(|value| value.as_str())
                        == Some("tui_local_1")
            })
        });
        assert!(
            messages.iter().any(|message| {
                message.role == "user"
                    && message
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("ingress_idempotency_key"))
                        .and_then(|value| value.as_str())
                        == Some("tui_local_1")
            }),
            "local message read should come from shared server authority"
        );

        let first_message_id = messages
            .first()
            .map(|message| message.id.clone())
            .expect("at least one message");
        let incremental = client
            .get_messages_after(&session.id, Some(first_message_id.as_str()), Some(16))
            .expect("read incremental local messages");
        assert!(
            incremental
                .iter()
                .all(|message| message.id != first_message_id),
            "incremental local read should honor the after anchor"
        );
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn local_runtime_accepts_structured_prompt_parts() {
        let paths = test_local_paths();
        let _env = install_local_test_env(&paths.data_root);
        let client = RuntimeApiClient::new_local_for_workspace(paths.workspace_root);
        let state = Arc::clone(client.local_server.as_ref().expect("local server state"));
        client
            .block_on(async {
                crate::local_server_bridge::local_register_provider(
                    &state,
                    Arc::new(MockLocalProvider::new()),
                )
                .await;
                Ok::<(), anyhow::Error>(())
            })
            .expect("register mock local provider");
        let session = client
            .create_session(None, Some(".".to_string()))
            .expect("create local session");

        client
            .send_prompt(
                &session.id,
                "delegate locally".to_string(),
                Some(vec![PromptPart::Agent {
                    name: "explore".to_string(),
                }]),
                None,
                None,
                Some("mock-local/mock-model".to_string()),
                None,
                Some("tui".to_string()),
                Some("multipart_local_1".to_string()),
                Some(agendao_types::MessageSourceOrigin::Operator),
                Some(agendao_types::MessageSourceSurface::Tui),
            )
            .expect("multipart prompt should succeed in local mode");

        let messages = wait_for_messages(&client, &session.id, |messages| {
            messages.iter().any(|message| {
                message.role == "user"
                    && message
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("ingress_idempotency_key"))
                        .and_then(|value| value.as_str())
                        == Some("multipart_local_1")
                    && message.parts.iter().any(|part| part.part_type == "agent")
            })
        });
        assert!(
            messages.iter().any(|message| {
                message.role == "user"
                    && message
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("ingress_idempotency_key"))
                        .and_then(|value| value.as_str())
                        == Some("multipart_local_1")
                    && message.parts.iter().any(|part| part.part_type == "agent")
            }),
            "multipart prompt should round-trip through local server authority"
        );
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn local_runtime_lists_modes_and_agents_without_http() {
        let paths = test_local_paths();
        let _env = install_local_test_env(&paths.data_root);
        let client = RuntimeApiClient::new_local_for_workspace(paths.workspace_root);

        let modes = client
            .list_execution_modes()
            .expect("list local execution modes");
        assert!(
            !modes.is_empty(),
            "local execution mode listing should use local server authority"
        );

        let agents = client.list_agents().expect("list local agents");
        assert!(
            !agents.is_empty(),
            "local agent listing should use local server authority"
        );
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn local_runtime_delete_session_removes_it_from_local_listing() {
        let paths = test_local_paths();
        let _env = install_local_test_env(&paths.data_root);
        let client = RuntimeApiClient::new_local_for_workspace(paths.workspace_root);

        let session = client
            .create_session(None, Some(".".to_string()))
            .expect("create local session");
        let before = client
            .list_sessions()
            .expect("list sessions before local delete");
        assert!(
            before.iter().any(|item| item.id == session.id),
            "new local session should appear in local listing"
        );

        let deleted = client
            .delete_session(&session.id)
            .expect("delete local session");
        assert!(deleted, "local session delete should report success");

        let after = client
            .list_sessions()
            .expect("list sessions after local delete");
        assert!(
            after.iter().all(|item| item.id != session.id),
            "deleted local session should disappear from local listing"
        );
    }

    #[test]
    fn direct_api_client_returns_empty_mcp_status() {
        let client = ApiClient::new_local();
        let status = client
            .get_mcp_status()
            .expect("direct local MCP status should not hit HTTP");
        assert!(status.is_empty());
    }

    #[test]
    fn direct_api_client_returns_empty_lsp_servers() {
        let client = ApiClient::new_local();
        let servers = client
            .get_lsp_servers()
            .expect("direct local LSP status should not hit HTTP");
        assert!(servers.is_empty());
    }
}
