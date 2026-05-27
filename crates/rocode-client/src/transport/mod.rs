/// Frontend transport abstraction - supports Direct, Unix Socket, and HTTP modes.
///
/// Phase 1: Only Direct + Http
/// Phase 2: Add Unix socket ✓
/// Phase 3: Smart fallback logic ✓

pub mod direct;
pub mod http;
pub mod unix;
pub mod selector;

pub use direct::DirectTransport;
pub use http::HttpTransport;
pub use unix::UnixSocketTransport;
pub use selector::TransportSelector;

use anyhow::Result;

/// Transport layer for frontend-to-core communication.
///
/// Architecture note (ROCode Constitution Article 1 & 9):
/// - Direct mode: TUI/CLI directly call OrchestrationCore (zero network overhead)
/// - Unix mode: Local IPC via Unix domain socket (minimal overhead)
/// - Http mode: Web frontend or remote connections use HTTP client
/// - All transports execute through the same OrchestrationCore authority
pub enum FrontendTransport {
    /// Direct in-process call to OrchestrationCore
    Direct(DirectTransport),

    /// Unix domain socket (local IPC)
    Unix(UnixSocketTransport),

    /// HTTP client (existing behavior)
    Http(HttpTransport),
}

impl FrontendTransport {
    /// Create Direct transport (local mode, default SessionManager).
    pub async fn direct(config: &rocode_config::Config) -> Result<Self> {
        Ok(Self::Direct(DirectTransport::new(config).await?))
    }

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
            Self::Direct(t) => t.prompt(session_id, text, options).await,
            Self::Unix(t) => t.prompt(session_id, text, options).await,
            Self::Http(t) => t.prompt(session_id, text, options).await,
        }
    }

    /// List sessions
    pub async fn list_sessions(&self) -> Result<Vec<rocode_api::SessionListItem>> {
        match self {
            Self::Direct(t) => t.list_sessions().await,
            Self::Unix(t) => t.list_sessions().await,
            Self::Http(t) => t.list_sessions().await,
        }
    }

    /// Get session detail
    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        match self {
            Self::Direct(t) => t.get_session(session_id).await,
            Self::Unix(t) => t.get_session(session_id).await,
            Self::Http(t) => t.get_session(session_id).await,
        }
    }
}

/// Options for prompt execution
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PromptOptions {
    pub agent_id: Option<String>,
    pub scheduler_profile: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub continue_last: bool,
    pub source_origin: Option<rocode_types::MessageSourceOrigin>,
    pub source_surface: Option<rocode_types::MessageSourceSurface>,
    pub ingress_source: Option<String>,
    pub idempotency_key: Option<String>,
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
