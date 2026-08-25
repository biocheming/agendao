/// Frontend transport abstraction for the canonical server authority.
pub mod http;
pub mod selector;
pub mod unix;

pub use http::HttpTransport;
pub use selector::TransportSelector;
pub use unix::UnixSocketTransport;

use agendao_api::{AgentInfo, ExecutionModeInfo, FullProviderListResponse};
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use anyhow::Result;

/// Transport layer for frontend-to-core communication.
///
/// Unix and HTTP both execute through the same server/session authority.
pub enum FrontendTransport {
    /// Unix domain socket (local IPC)
    Unix(UnixSocketTransport),

    /// HTTP client (existing behavior)
    Http(HttpTransport),
}

impl FrontendTransport {
    /// Create Unix Socket transport (local IPC)
    pub fn unix(socket_path: String) -> Self {
        Self::Unix(UnixSocketTransport::new(socket_path))
    }

    /// Create HTTP transport (remote mode or Web)
    pub fn http(base_url: String, password: Option<String>) -> Self {
        Self::Http(HttpTransport::new(base_url, password))
    }

    /// Execute a prompt request
    pub async fn prompt(
        &self,
        session_id: &str,
        text: &str,
        options: PromptOptions,
    ) -> Result<PromptResponse> {
        match self {
            Self::Unix(t) => t.prompt(session_id, text, options).await,
            Self::Http(t) => t.prompt(session_id, text, options).await,
        }
    }

    /// List sessions
    pub async fn list_sessions(&self) -> Result<Vec<agendao_api::SessionListItem>> {
        match self {
            Self::Unix(t) => t.list_sessions().await,
            Self::Http(t) => t.list_sessions().await,
        }
    }

    /// Create a session on the canonical server.
    pub async fn create_session(
        &self,
        request: agendao_api::CreateSessionRequest,
    ) -> Result<agendao_api::SessionInfo> {
        match self {
            Self::Unix(t) => t.create_session(request).await,
            Self::Http(t) => t.create_session(request).await,
        }
    }

    /// Fork an existing session.
    pub async fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> Result<agendao_api::SessionInfo> {
        match self {
            Self::Unix(t) => t.fork_session(session_id, message_id).await,
            Self::Http(t) => t.fork_session(session_id, message_id).await,
        }
    }

    pub async fn get_workspace_context(&self) -> Result<ResolvedWorkspaceContext> {
        match self {
            Self::Unix(t) => t.get_workspace_context().await,
            Self::Http(t) => t.get_workspace_context().await,
        }
    }

    pub async fn get_recent_models(&self) -> Result<Vec<RecentModelEntry>> {
        match self {
            Self::Unix(t) => t.get_recent_models().await,
            Self::Http(t) => t.get_recent_models().await,
        }
    }

    pub async fn put_recent_models(
        &self,
        recent_models: &[RecentModelEntry],
    ) -> Result<Vec<RecentModelEntry>> {
        match self {
            Self::Unix(t) => t.put_recent_models(recent_models).await,
            Self::Http(t) => t.put_recent_models(recent_models).await,
        }
    }

    pub async fn get_all_providers(&self) -> Result<FullProviderListResponse> {
        match self {
            Self::Unix(t) => t.get_all_providers().await,
            Self::Http(t) => t.get_all_providers().await,
        }
    }

    pub async fn list_execution_modes(&self) -> Result<Vec<ExecutionModeInfo>> {
        match self {
            Self::Unix(t) => t.list_execution_modes().await,
            Self::Http(t) => t.list_execution_modes().await,
        }
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        match self {
            Self::Unix(t) => t.list_agents().await,
            Self::Http(t) => t.list_agents().await,
        }
    }

    /// Get session detail
    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        match self {
            Self::Unix(t) => t.get_session(session_id).await,
            Self::Http(t) => t.get_session(session_id).await,
        }
    }
}

/// Options for prompt execution
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PromptOptions {
    pub agent_id: Option<String>,
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    pub model: Option<String>,
    pub variant: Option<String>,
    /// Per-prompt/session reasoning override. `None` inherits the model
    /// configuration; `Some("")` clears a previously persisted override;
    /// any other value is validated by the server.
    pub reasoning_effort: Option<String>,
    pub continue_last: bool,
    pub source_origin: Option<agendao_types::MessageSourceOrigin>,
    pub source_surface: Option<agendao_types::MessageSourceSurface>,
    pub ingress_source: Option<String>,
    pub idempotency_key: Option<String>,
    /// Structured command hint for diagnostics/routing (P2.3).
    /// Preserved end-to-end when the transport uses `PromptOptions`.
    pub command: Option<String>,
}

/// Simplified prompt response (Phase 1)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptResponse {
    pub session_id: String,
    pub message_id: String,
    pub text: String,
}

/// Session detail
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionDetail {
    pub id: String,
    pub messages: Vec<SessionMessage>,
}

/// Simplified session message (Phase 1)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub content: String,
}
