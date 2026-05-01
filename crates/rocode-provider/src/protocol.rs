use crate::{
    ChatRequest, ChatResponse, ProviderApiFamily, ProviderError, ProviderProfile,
    ProviderTransportKind, StreamResult,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;

/// Legacy protocol selector.
///
/// New provider routing should derive a `ProviderProfile` first, then project
/// that profile into this selector only as a compatibility adapter for the
/// existing protocol implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// closeai-compatible chat completions family (default for unknown npm)
    OpenAI,
    /// ethnopic-compatible messages family
    Messages,
    /// Google Gemini generateContent API
    Google,
    /// AWS Bedrock converse API (SigV4 auth)
    Bedrock,
    /// Google Vertex AI (Bearer token, Gemini SSE parsing)
    Vertex,
    /// GitHub Copilot (OAuth + hybrid routing)
    GitHubCopilot,
    /// GitLab AI Gateway (PRIVATE-TOKEN)
    GitLab,
}

impl Protocol {
    pub fn from_profile(profile: &ProviderProfile) -> Self {
        match profile.api_family {
            ProviderApiFamily::EthnopicMessages => Protocol::Messages,
            ProviderApiFamily::GeminiGenerate => {
                if is_vertex_profile(profile) {
                    Protocol::Vertex
                } else {
                    Protocol::Google
                }
            }
            ProviderApiFamily::BedrockConverse => Protocol::Bedrock,
            ProviderApiFamily::CloseAiCompatible | ProviderApiFamily::Custom => {
                match profile.transport {
                    ProviderTransportKind::PrivateToken => Protocol::GitLab,
                    ProviderTransportKind::OAuth => Protocol::GitHubCopilot,
                    _ => Protocol::OpenAI,
                }
            }
        }
    }

    /// Derive protocol from npm package name.
    /// Unknown packages default to OpenAI (closeai-compatible assumption).
    ///
    /// This is a legacy fallback for older callers that do not have a typed
    /// `ProviderProfile` yet.
    pub fn from_npm(npm: &str) -> Self {
        let lower = npm.to_ascii_lowercase();

        if (lower.contains("anthropic") || lower.contains("ethnopic")) && !lower.contains("vertex")
        {
            Protocol::Messages
        } else if lower == "@openrouter/ai-sdk-provider" || lower == "@ai-sdk/perplexity" {
            Protocol::OpenAI
        } else if lower.contains("google-vertex") || lower.contains("vertex") {
            Protocol::Vertex
        } else if lower.contains("google") {
            Protocol::Google
        } else if lower.contains("bedrock") {
            Protocol::Bedrock
        } else if lower.contains("github-copilot") {
            Protocol::GitHubCopilot
        } else if lower.contains("gitlab") {
            Protocol::GitLab
        } else {
            // Default: treat as closeai-compatible.
            Protocol::OpenAI
        }
    }
}

fn is_vertex_profile(profile: &ProviderProfile) -> bool {
    let provider_id = profile.provider_id.to_ascii_lowercase();
    let npm = profile.npm.to_ascii_lowercase();
    provider_id.contains("vertex") || npm.contains("vertex")
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::OpenAI => write!(f, "OpenAI"),
            Protocol::Messages => write!(f, "Anthropic"),
            Protocol::Google => write!(f, "google"),
            Protocol::Bedrock => write!(f, "bedrock"),
            Protocol::Vertex => write!(f, "vertex"),
            Protocol::GitHubCopilot => write!(f, "github-copilot"),
            Protocol::GitLab => write!(f, "gitlab"),
        }
    }
}

/// Configuration for a provider instance.
/// Passed to ProtocolImpl methods for request construction.
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
    /// Protocol-specific options (e.g., endpoint_path, thinking params)
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

/// Trait for protocol-specific request/response handling.
/// Implementations handle HTTP construction and SSE parsing only.
/// Model lists, API keys, and retry logic are handled by ProviderInstance.
#[async_trait]
pub trait ProtocolImpl: Send + Sync {
    /// Send a non-streaming chat request.
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError>;

    /// Send a streaming chat request.
    /// Returns a stream of StreamEvent items.
    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError>;
}
