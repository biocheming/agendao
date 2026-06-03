/// Orchestration Core - The single execution authority (AgenDao Constitution Article 1)
///
/// Phase 4: Real implementation with independent authorities
///
/// This is the "唯一执行内核" - all LLM loops (model → tool → model) are driven
/// by this core. Adapters (TUI/CLI/Web) must call this core, never build their own loops.
///
/// Generic over `S: SessionStore` — default is `agendao_session_core::SessionManager`;
/// `agendao_session::SessionManager` can be injected for unified session authority.

use crate::prompt_execution::{execute_prompt_with_session, execute_prompt_streaming_with_session};
use crate::OrchestratorError;
use agendao_session_core::{SessionAccess, SessionManager, SessionStore};
use std::sync::Arc;

/// Orchestration core with independent authorities
///
/// Phase 4: Extract authorities from ServerState
pub struct OrchestrationCore<S: SessionStore = SessionManager> {
    // Phase 4.1: ConfigStore shared authority.
    // Kept on the core so prompt execution can grow config-aware behavior
    // without re-threading another authority through every adapter path.
    #[allow(dead_code)]
    config_store: Arc<agendao_config::ConfigStore>,
    // Phase 4.2: SessionManager (generic over SessionStore impl)
    sessions: Arc<tokio::sync::Mutex<S>>,
    // Phase 4.3: ProviderRegistry
    providers: Arc<tokio::sync::RwLock<agendao_provider::ProviderRegistry>>,
    // Phase 6.2: ToolRegistry
    tools: Arc<tokio::sync::RwLock<agendao_tool::ToolRegistry>>,
}

impl OrchestrationCore<SessionManager> {
    /// Create orchestration core from config (default SessionManager).
    pub async fn new(config: &agendao_config::Config) -> Result<Self, OrchestratorError> {
        let sessions = Arc::new(tokio::sync::Mutex::new(SessionManager::new()));
        Self::new_with_sessions(config, sessions).await
    }
}

impl<S: SessionStore> OrchestrationCore<S> {
    /// Create orchestration core with an externally-provided SessionManager.
    ///
    /// Creates standalone ConfigStore, ProviderRegistry, and ToolRegistry.
    /// For shared authorities use `new_with_shared_authorities`.
    pub async fn new_with_sessions(
        config: &agendao_config::Config,
        sessions: Arc<tokio::sync::Mutex<S>>,
    ) -> Result<Self, OrchestratorError> {
        let config_store = Arc::new(agendao_config::ConfigStore::new(config.clone()));
        let providers = Arc::new(tokio::sync::RwLock::new(
            agendao_provider::ProviderRegistry::new(),
        ));
        let tools = Arc::new(tokio::sync::RwLock::new(agendao_tool::ToolRegistry::new()));

        Ok(Self {
            config_store,
            sessions,
            providers,
            tools,
        })
    }

    /// Create orchestration core with shared authorities from the server.
    ///
    /// Provider and config changes made through HTTP routes are
    /// immediately visible to the Unix socket prompt path — no restart needed.
    pub fn new_with_shared_authorities(
        config_store: Arc<agendao_config::ConfigStore>,
        sessions: Arc<tokio::sync::Mutex<S>>,
        providers: Arc<tokio::sync::RwLock<agendao_provider::ProviderRegistry>>,
        tools: Arc<tokio::sync::RwLock<agendao_tool::ToolRegistry>>,
    ) -> Self {
        Self {
            config_store,
            sessions,
            providers,
            tools,
        }
    }

    /// Access the shared SessionManager.
    pub fn sessions(&self) -> &Arc<tokio::sync::Mutex<S>> {
        &self.sessions
    }

    /// Execute prompt request (唯一入口)
    pub async fn execute_prompt(
        &self,
        session_id: &str,
        text: &str,
        options: PromptExecutionOptions,
    ) -> Result<PromptExecutionResult, OrchestratorError> {
        execute_prompt_with_session(
            &self.sessions,
            &self.providers,
            &self.tools,
            session_id,
            text,
            &options,
        )
        .await
    }

    /// Execute prompt with streaming output (Phase 6.3)
    pub async fn execute_prompt_streaming(
        &self,
        session_id: &str,
        text: &str,
        options: PromptExecutionOptions,
    ) -> Result<agendao_provider::StreamResult, OrchestratorError>
    where
        S: Send + 'static,
    {
        execute_prompt_streaming_with_session(
            &self.sessions,
            &self.providers,
            &self.tools,
            session_id,
            text,
            &options,
        )
        .await
    }

    /// Get provider registry for external registration
    pub fn providers(&self) -> &Arc<tokio::sync::RwLock<agendao_provider::ProviderRegistry>> {
        &self.providers
    }

    /// Get tool registry for external registration
    pub fn tools(&self) -> &Arc<tokio::sync::RwLock<agendao_tool::ToolRegistry>> {
        &self.tools
    }

    /// List all sessions
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, OrchestratorError> {
        let sessions = self.sessions.lock().await;
        let store = &*sessions;
        let summaries = store
            .list()
            .iter()
            .map(|session| {
                let record = session.record();
                SessionSummary {
                    id: record.id.clone(),
                    title: if record.title.is_empty() {
                        None
                    } else {
                        Some(record.title.clone())
                    },
                    created_at: record.created_at.to_rfc3339(),
                    last_message_at: Some(
                        chrono::DateTime::from_timestamp_millis(record.time.updated)
                            .unwrap_or_else(|| chrono::Utc::now())
                            .to_rfc3339(),
                    ),
                }
            })
            .collect();
        Ok(summaries)
    }

    /// Get session detail
    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<SessionDetail, OrchestratorError> {
        let sessions = self.sessions.lock().await;
        let session = sessions.get(session_id).ok_or_else(|| {
            OrchestratorError::Other(format!("Session not found: {}", session_id))
        })?;

        let record = session.record();
        Ok(SessionDetail {
            id: record.id.clone(),
            title: if record.title.is_empty() {
                None
            } else {
                Some(record.title.clone())
            },
            messages: record
                .messages
                .iter()
                .map(|msg| {
                    let content = msg
                        .parts
                        .iter()
                        .filter_map(|part| {
                            if let agendao_types::PartType::Text { text, .. } = &part.part_type {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    MessageSummary {
                        id: msg.id.clone(),
                        role: format!("{:?}", msg.role),
                        content,
                        created_at: msg.created_at.to_rfc3339(),
                    }
                })
                .collect(),
        })
    }

    /// Execute tool call (唯一入口)
    pub async fn execute_tool(
        &self,
        _session_id: &str,
        _tool_call: ToolCallRequest,
    ) -> Result<ToolCallResult, OrchestratorError> {
        Err(OrchestratorError::Other(
            "OrchestrationCore::execute_tool not yet implemented (Phase 3)".to_string(),
        ))
    }
}

/// Options for prompt execution
#[derive(Debug, Clone)]
pub struct PromptExecutionOptions {
    pub agent_id: Option<String>,
    pub scheduler_profile: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    /// When true and text is empty, skip adding a new user message.
    /// The LLM continues from the last assistant response.
    pub continue_last: bool,
    /// Canonical source metadata — who originated this prompt.
    pub source_origin: Option<agendao_types::MessageSourceOrigin>,
    /// Which surface/transport the prompt arrived through.
    pub source_surface: Option<agendao_types::MessageSourceSurface>,
    /// Best-effort ingress/source label for session metadata parity.
    pub ingress_source: Option<String>,
    /// Best-effort idempotency marker for session/message metadata parity.
    pub idempotency_key: Option<String>,
}

impl Default for PromptExecutionOptions {
    fn default() -> Self {
        Self {
            agent_id: None,
            scheduler_profile: None,
            model: None,
            variant: None,
            continue_last: false,
            source_origin: None,
            source_surface: None,
            ingress_source: None,
            idempotency_key: None,
        }
    }
}

/// Result of prompt execution
#[derive(Debug, Clone)]
pub struct PromptExecutionResult {
    pub session_id: String,
    pub message_id: String,
    pub text: String,
    pub usage: Option<UsageInfo>,
}

/// Session summary for listing
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub created_at: String,
    pub last_message_at: Option<String>,
}

/// Session detail
#[derive(Debug, Clone)]
pub struct SessionDetail {
    pub id: String,
    pub title: Option<String>,
    pub messages: Vec<MessageSummary>,
}

/// Message summary
#[derive(Debug, Clone)]
pub struct MessageSummary {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

/// Tool call request
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Tool call result
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub output: String,
    pub error: Option<String>,
}

/// Usage information
#[derive(Debug, Clone)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}
