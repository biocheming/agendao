use super::{PromptOptions, PromptResponse, SessionDetail};
use anyhow::{Context, Result};
use rocode_api::{AgentInfo, ExecutionModeInfo, FullProviderListResponse, SessionListItem};
use rocode_orchestrator::{CoreSessionManager, OrchestrationCore, SessionStore};
use rocode_runtime_context::ResolvedWorkspaceContext;
use rocode_state::RecentModelEntry;
use std::sync::Arc;

/// Direct transport - calls OrchestrationCore in-process.
///
/// Generic over `S: SessionStore` so it can accept a core backed by
/// `rocode_session::SessionManager` for unified authority.
pub struct DirectTransport<S: SessionStore = CoreSessionManager> {
    core: Arc<OrchestrationCore<S>>,
}

impl DirectTransport<CoreSessionManager> {
    /// Create DirectTransport with the default SessionManager (backward compat).
    pub async fn new(config: &rocode_config::Config) -> Result<Self> {
        let core = OrchestrationCore::new(config).await?;
        Ok(Self {
            core: Arc::new(core),
        })
    }
}

impl<S: SessionStore> DirectTransport<S> {
    /// Create DirectTransport with a pre-built OrchestrationCore.
    /// Allows injecting a core that shares the server's SessionManager
    /// for unified session authority across transport paths.
    pub fn new_with_core(core: Arc<OrchestrationCore<S>>) -> Self {
        Self { core }
    }

    /// Access the underlying orchestration core.
    pub fn core(&self) -> &Arc<OrchestrationCore<S>> {
        &self.core
    }

    pub async fn prompt(
        &self,
        session_id: &str,
        text: &str,
        options: PromptOptions,
    ) -> Result<PromptResponse> {
        let exec_options = rocode_orchestrator::PromptExecutionOptions {
            agent_id: options.agent_id,
            scheduler_profile: options.scheduler_profile,
            model: options.model,
            variant: options.variant,
            continue_last: options.continue_last,
            source_origin: options
                .source_origin
                .or(Some(rocode_types::MessageSourceOrigin::Operator)),
            source_surface: options
                .source_surface
                .or(Some(rocode_types::MessageSourceSurface::Direct)),
            ingress_source: options.ingress_source,
            idempotency_key: options.idempotency_key,
        };

        let result = self
            .core
            .execute_prompt(session_id, text, exec_options)
            .await
            .context("Direct prompt execution failed")?;

        Ok(PromptResponse {
            session_id: result.session_id,
            message_id: result.message_id,
            text: result.text,
        })
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionListItem>> {
        let sessions = self.core.list_sessions().await?;
        Ok(sessions
            .into_iter()
            .map(|s| {
                let id = s.id;
                SessionListItem {
                    slug: id.clone(),
                    project_id: String::new(),
                    directory: String::new(),
                    parent_id: None,
                    title: s.title.unwrap_or_default(),
                    version: String::new(),
                    time: rocode_types::SessionTime {
                        created: 0,
                        updated: 0,
                        compacting: None,
                        archived: None,
                    },
                    summary: None,
                    hints: None,
                    pending_command_invocation: None,
                    id,
                }
            })
            .collect())
    }

    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        let session = self.core.get_session(session_id).await?;
        Ok(SessionDetail {
            id: session.id,
            messages: session
                .messages
                .into_iter()
                .map(|m| super::SessionMessage {
                    id: m.id,
                    role: m.role,
                    content: m.content,
                })
                .collect(),
        })
    }

    pub async fn get_workspace_context(&self) -> Result<ResolvedWorkspaceContext> {
        anyhow::bail!("workspace context is not exposed through DirectTransport")
    }

    pub async fn get_recent_models(&self) -> Result<Vec<RecentModelEntry>> {
        anyhow::bail!("recent models are not exposed through DirectTransport")
    }

    pub async fn put_recent_models(
        &self,
        _recent_models: &[RecentModelEntry],
    ) -> Result<Vec<RecentModelEntry>> {
        anyhow::bail!("recent models are not exposed through DirectTransport")
    }

    pub async fn get_all_providers(&self) -> Result<FullProviderListResponse> {
        anyhow::bail!("provider catalogue is not exposed through DirectTransport")
    }

    pub async fn list_execution_modes(&self) -> Result<Vec<ExecutionModeInfo>> {
        anyhow::bail!("execution modes are not exposed through DirectTransport")
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        anyhow::bail!("agents are not exposed through DirectTransport")
    }
}
