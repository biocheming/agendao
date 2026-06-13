//! Synchronous API bridge.
//!
//! Wraps `agendao_client::AsyncApiClient` using a background tokio runtime.
//! `ApiBridge` is Clone so it can be shared across views.

use agendao_client::{AsyncApiClient, SessionInfo};
use std::sync::Arc;
use tokio::runtime::Runtime;

pub use agendao_client::PromptResponse;

#[derive(Clone)]
pub struct ApiBridge {
    client: Arc<AsyncApiClient>,
    runtime: Arc<Runtime>,
}

impl ApiBridge {
    pub fn new(base_url: &str) -> anyhow::Result<Self> {
        let client = Arc::new(AsyncApiClient::new(base_url.to_string()));
        let runtime = Arc::new(
            Runtime::new()
                .map_err(|e| anyhow::anyhow!("tokio runtime: {}", e))?
        );
        Ok(Self { client, runtime })
    }

    pub fn create_session(
        &self,
        profile: Option<String>,
        directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        self.runtime.block_on(
            self.client.create_session(profile, directory)
        )
    }

    pub fn send_prompt(
        &self,
        session_id: &str,
        content: String,
    ) -> anyhow::Result<PromptResponse> {
        let c = Arc::clone(&self.client);
        self.runtime.block_on(c.send_prompt(
            session_id,
            content,
            None, None, None, None, None, None, None, None, None, None,
        ))
    }

    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }
}
