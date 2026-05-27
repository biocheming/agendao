use super::{PromptOptions, PromptResponse, SessionDetail};
use anyhow::Result;
use rocode_api::SessionListItem;

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
                text.to_string(),
                None, // parts
                options.agent_id,
                options.scheduler_profile,
                options.model,
                options.variant,
                options.ingress_source,
                options.idempotency_key,
                options.source_origin,
                options.source_surface,
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

    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        let session = self.client.get_session(session_id).await?;
        Ok(SessionDetail {
            id: session.id,
            messages: vec![],
        })
    }
}
