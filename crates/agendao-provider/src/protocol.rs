use crate::{
    ChatRequest, ChatResponse, ProviderApiFamily, ProviderError, ProviderProfile,
    ProviderTransportKind, StreamResult,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;

/// Runtime adapter selector derived from a typed `ProviderProfile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderRuntimeAdapter {
    /// closeai-compatible chat completions family
    CloseAiCompatible,
    /// Ethnopic-compatible provider family
    Ethnopic,
    /// Google Gemini generateContent API
    Gemini,
    /// AWS Bedrock converse API (SigV4 auth)
    BedrockConverse,
    /// Google Vertex AI (Bearer token, Gemini SSE parsing)
    VertexGemini,
    /// GitHub Copilot adapter (OAuth + hybrid routing)
    GitHubCopilotCloseAi,
    /// GitLab AI Gateway adapter (PRIVATE-TOKEN)
    GitLabCloseAi,
}

impl ProviderRuntimeAdapter {
    pub fn from_profile(profile: &ProviderProfile) -> Self {
        match profile.api_family {
            ProviderApiFamily::EthnopicMessages => ProviderRuntimeAdapter::Ethnopic,
            ProviderApiFamily::GeminiGenerate => match profile.transport {
                ProviderTransportKind::VertexBearer => ProviderRuntimeAdapter::VertexGemini,
                _ => ProviderRuntimeAdapter::Gemini,
            },
            ProviderApiFamily::BedrockConverse => ProviderRuntimeAdapter::BedrockConverse,
            ProviderApiFamily::CloseAiCompatible | ProviderApiFamily::Custom => {
                match profile.transport {
                    ProviderTransportKind::PrivateToken => ProviderRuntimeAdapter::GitLabCloseAi,
                    ProviderTransportKind::OAuth => ProviderRuntimeAdapter::GitHubCopilotCloseAi,
                    _ => ProviderRuntimeAdapter::CloseAiCompatible,
                }
            }
        }
    }
}

impl fmt::Display for ProviderRuntimeAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderRuntimeAdapter::CloseAiCompatible => write!(f, "closeai-compatible"),
            ProviderRuntimeAdapter::Ethnopic => write!(f, "ethnopic"),
            ProviderRuntimeAdapter::Gemini => write!(f, "gemini"),
            ProviderRuntimeAdapter::BedrockConverse => write!(f, "bedrock-converse"),
            ProviderRuntimeAdapter::VertexGemini => write!(f, "vertex-gemini"),
            ProviderRuntimeAdapter::GitHubCopilotCloseAi => write!(f, "github-copilot-closeai"),
            ProviderRuntimeAdapter::GitLabCloseAi => write!(f, "gitlab-closeai"),
        }
    }
}

#[cfg(feature = "http-transport")]
pub type ProviderHttpClient = reqwest::Client;

#[cfg(not(feature = "http-transport"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct ProviderHttpClient;

#[cfg(feature = "http-transport")]
pub type ProviderHttpStreamError = reqwest::Error;

#[cfg(not(feature = "http-transport"))]
pub type ProviderHttpStreamError = std::io::Error;

/// Configuration for a provider instance.
/// Passed to ProviderAdapter methods for request construction.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Unique identifier for this provider (e.g., "deepseek", "openrouter")
    pub provider_id: String,
    /// Base URL for API requests
    pub base_url: String,
    /// API key or token
    pub api_key: String,
    /// Additional headers to include in requests
    pub headers: HashMap<String, String>,
    /// Provider-specific options (e.g., endpoint_path, thinking params)
    pub options: HashMap<String, serde_json::Value>,
}

impl ProviderConfig {
    pub fn new(
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            headers: HashMap::new(),
            options: HashMap::new(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn with_option(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.options.insert(key.into(), value);
        self
    }

    pub fn option_string(&self, keys: &[&str]) -> Option<String> {
        for key in keys {
            let Some(value) = self.options.get(*key) else {
                continue;
            };
            match value {
                serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
                serde_json::Value::Number(n) => return Some(n.to_string()),
                serde_json::Value::Bool(b) => return Some(b.to_string()),
                _ => {}
            }
        }
        None
    }

    pub fn option_bool(&self, keys: &[&str]) -> Option<bool> {
        for key in keys {
            let Some(value) = self.options.get(*key) else {
                continue;
            };
            match value {
                serde_json::Value::Bool(b) => return Some(*b),
                serde_json::Value::Number(n) => return Some(n.as_i64().unwrap_or(0) != 0),
                serde_json::Value::String(s) => {
                    let lower = s.trim().to_ascii_lowercase();
                    if matches!(lower.as_str(), "1" | "true" | "yes" | "on") {
                        return Some(true);
                    }
                    if matches!(lower.as_str(), "0" | "false" | "no" | "off") {
                        return Some(false);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// Runtime adapter for a provider profile.
/// Implementations bridge a typed provider profile to HTTP request construction,
/// response normalization, and SSE parsing.
/// Model lists, API keys, and retry logic are handled by ProviderInstance.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Send a non-streaming chat request.
    async fn chat(
        &self,
        client: &ProviderHttpClient,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError>;

    /// Send a streaming chat request.
    /// Returns a stream of StreamEvent items.
    async fn chat_stream(
        &self,
        client: &ProviderHttpClient,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError>;
}
