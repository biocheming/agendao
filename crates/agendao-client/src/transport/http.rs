use super::{PromptOptions, PromptResponse, SessionDetail};
use agendao_api::{
    AgentInfo, ExecutionModeInfo, FullProviderListResponse, PromptRequest, SessionListItem,
};
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use anyhow::Result;

/// HTTP transport - wraps existing AsyncApiClient
pub struct HttpTransport {
    client: crate::AsyncApiClient,
}

impl HttpTransport {
    pub fn new(base_url: String, password: Option<String>) -> Self {
        let client = if let Some(pwd) = password {
            crate::AsyncApiClient::new_with_password(base_url, Some(pwd))
        } else {
            crate::AsyncApiClient::new(base_url)
        };
        Self { client }
    }

    pub async fn prompt(
        &self,
        session_id: &str,
        text: &str,
        options: PromptOptions,
    ) -> Result<PromptResponse> {
        let response = self
            .client
            .send_prompt(
                session_id,
                PromptRequest {
                    message: Some(text.to_string()),
                    parts: None,
                    agent: options.agent_id,
                    scheduler: options.scheduler,
                    model: options.model,
                    variant: options.variant,
                    reasoning_effort: options.reasoning_effort,
                    ingress_source: options.ingress_source,
                    idempotency_key: options.idempotency_key,
                    source_origin: options.source_origin,
                    source_surface: options.source_surface,
                    command: options.command,
                    arguments: None,
                },
            )
            .await?;

        Ok(PromptResponse {
            session_id: session_id.to_string(),
            message_id: response
                .session_id
                .unwrap_or_else(|| session_id.to_string()),
            text: response.status,
        })
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionListItem>> {
        self.client.list_sessions(None, Some(100)).await
    }

    pub async fn create_session(
        &self,
        request: agendao_api::CreateSessionRequest,
    ) -> Result<agendao_api::SessionInfo> {
        self.client
            .create_session(request.scheduler, request.directory)
            .await
    }

    pub async fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> Result<agendao_api::SessionInfo> {
        self.client.fork_session(session_id, message_id).await
    }

    pub async fn get_workspace_context(&self) -> Result<ResolvedWorkspaceContext> {
        self.client.get_workspace_context().await
    }

    pub async fn get_recent_models(&self) -> Result<Vec<RecentModelEntry>> {
        self.client.get_recent_models().await
    }

    pub async fn put_recent_models(
        &self,
        recent_models: &[RecentModelEntry],
    ) -> Result<Vec<RecentModelEntry>> {
        self.client.put_recent_models(recent_models).await
    }

    pub async fn get_all_providers(&self) -> Result<FullProviderListResponse> {
        self.client.get_all_providers().await
    }

    pub async fn list_execution_modes(&self) -> Result<Vec<ExecutionModeInfo>> {
        self.client.list_execution_modes().await
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        self.client.list_agents().await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        let session = self.client.get_session(session_id).await?;
        Ok(SessionDetail {
            id: session.id,
            messages: vec![],
        })
    }
}
