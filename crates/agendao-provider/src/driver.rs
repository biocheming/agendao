//! Internalized driver abstractions (originally from ai-lib-rust).
//!
//! This module provides the core types and traits for protocol-agnostic
//! provider drivers: `ProviderDriver`, `DriverRequest`, `DriverResponse`,
//! `StreamingEvent`, `ApiStyle`, and a simple `DriverMessage` type.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// ApiStyle
// ---------------------------------------------------------------------------

/// Identifies the API wire format family a provider uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    OpenAiCompatible,
    EthnopicMessages,
    GeminiGenerate,
    Custom,
}

impl std::fmt::Display for ApiStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAiCompatible => write!(f, "closeai-compatible"),
            Self::EthnopicMessages => write!(f, "ethnopic-compatible"),
            Self::GeminiGenerate => write!(f, "gemini"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

// ---------------------------------------------------------------------------
// DriverMessage — multimodal message type for driver interface
// ---------------------------------------------------------------------------

/// A message for the driver interface with multimodal content support.
///
/// Supports text, images (base64/URL), audio (base64/URL), tool use, and
/// tool results — everything needed for `ProviderDriver::build_request()`
/// to construct API payloads for vision/voice-capable models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverMessage {
    pub role: DriverMessageRole,
    pub content: DriverMessageContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Message content — either plain text or structured blocks for multimodal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DriverMessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// A content block within a multimodal message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "audio")]
    Audio { source: AudioSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// base64-encoded image data or a URL.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// base64-encoded audio data or a URL.
    pub data: String,
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image_base64(data: String, media_type: Option<String>) -> Self {
        Self::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type,
                data,
            },
        }
    }

    pub fn image_url(url: String) -> Self {
        Self::Image {
            source: ImageSource {
                source_type: "url".to_string(),
                media_type: None,
                data: url,
            },
        }
    }

    pub fn audio_base64(data: String, media_type: Option<String>) -> Self {
        Self::Audio {
            source: AudioSource {
                source_type: "base64".to_string(),
                media_type,
                data,
            },
        }
    }

    pub fn tool_use(id: String, name: String, input: serde_json::Value) -> Self {
        Self::ToolUse { id, name, input }
    }

    pub fn tool_result(tool_use_id: String, content: serde_json::Value) -> Self {
        Self::ToolResult {
            tool_use_id,
            content,
        }
    }
}

impl DriverMessageContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn blocks(blocks: Vec<ContentBlock>) -> Self {
        Self::Blocks(blocks)
    }
}

impl DriverMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: DriverMessageRole::System,
            content: DriverMessageContent::Text(text.into()),
            tool_call_id: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: DriverMessageRole::User,
            content: DriverMessageContent::Text(text.into()),
            tool_call_id: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: DriverMessageRole::Assistant,
            content: DriverMessageContent::Text(text.into()),
            tool_call_id: None,
        }
    }

    /// Create a tool result message with `tool_call_id`.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: DriverMessageRole::Tool,
            content: DriverMessageContent::Text(content.into()),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    /// Create a message with structured content blocks (multimodal).
    pub fn with_blocks(role: DriverMessageRole, blocks: Vec<ContentBlock>) -> Self {
        Self {
            role,
            content: DriverMessageContent::Blocks(blocks),
            tool_call_id: None,
        }
    }

    /// Returns true if any content block contains an image.
    pub fn contains_image(&self) -> bool {
        match &self.content {
            DriverMessageContent::Text(_) => false,
            DriverMessageContent::Blocks(bs) => {
                bs.iter().any(|b| matches!(b, ContentBlock::Image { .. }))
            }
        }
    }

    /// Returns true if any content block contains audio.
    pub fn contains_audio(&self) -> bool {
        match &self.content {
            DriverMessageContent::Text(_) => false,
            DriverMessageContent::Blocks(bs) => {
                bs.iter().any(|b| matches!(b, ContentBlock::Audio { .. }))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DriverRequest / DriverResponse / UsageInfo
// ---------------------------------------------------------------------------

/// Request produced by a `ProviderDriver` for sending to a provider API.
#[derive(Debug, Clone)]
pub struct DriverRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
    pub stream: bool,
}

/// Response parsed by a `ProviderDriver` from a provider API response.
#[derive(Debug, Clone)]
pub struct DriverResponse {
    pub content: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<UsageInfo>,
    pub tool_calls: Vec<serde_json::Value>,
    pub raw: serde_json::Value,
}

/// Token usage information.
#[derive(Debug, Clone, Default)]
pub struct UsageInfo {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// StreamingEvent
// ---------------------------------------------------------------------------

/// Unified streaming event type for provider-agnostic stream processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum StreamingEvent {
    PartialContentDelta {
        content: String,
        sequence_id: Option<u64>,
    },
    ThinkingDelta {
        thinking: String,
        tool_consideration: Option<String>,
    },
    ToolCallStarted {
        tool_call_id: String,
        tool_name: String,
        index: Option<u32>,
    },
    PartialToolCall {
        tool_call_id: String,
        arguments: String,
        index: Option<u32>,
        is_complete: Option<bool>,
    },
    ToolCallEnded {
        tool_call_id: String,
        index: Option<u32>,
    },
    Metadata {
        usage: Option<serde_json::Value>,
        finish_reason: Option<String>,
        stop_reason: Option<String>,
    },
    FinalCandidate {
        candidate_index: u32,
        finish_reason: String,
    },
    StreamEnd {
        finish_reason: Option<String>,
    },
    StreamError {
        error: serde_json::Value,
        event_id: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// ProviderDriver trait
// ---------------------------------------------------------------------------

/// Capability flags a driver may advertise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    FunctionCalling,
    Vision,
    Streaming,
    SystemPrompt,
    Thinking,
}

/// Error type for driver operations.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("request build error: {0}")]
    RequestBuild(String),
    #[error("response parse error: {0}")]
    ResponseParse(String),
    #[error("stream parse error: {0}")]
    StreamParse(String),
    #[error("{0}")]
    Other(String),
}

/// A protocol-agnostic provider driver.
///
/// Implementations know how to build requests and parse responses for a
/// specific API wire format family (closeai-compatible, ethnopic-compatible, Gemini, etc.).
#[async_trait]
pub trait ProviderDriver: Send + Sync + std::fmt::Debug {
    /// Unique identifier for this provider.
    fn provider_id(&self) -> &str;

    /// The API wire format this driver speaks.
    fn api_style(&self) -> ApiStyle;

    /// Build an HTTP request from high-level parameters.
    fn build_request(
        &self,
        messages: &[DriverMessage],
        model: &str,
        temperature: Option<f64>,
        max_tokens: Option<u32>,
        stream: bool,
        extra: Option<&serde_json::Value>,
    ) -> Result<DriverRequest, DriverError>;

    /// Parse a non-streaming response body.
    fn parse_response(&self, body: &serde_json::Value) -> Result<DriverResponse, DriverError>;

    /// Parse a single SSE data line into a streaming event.
    fn parse_stream_event(&self, data: &str) -> Result<Option<StreamingEvent>, DriverError>;

    /// Capabilities this driver supports.
    fn supported_capabilities(&self) -> &[Capability];

    /// Check whether an SSE data line signals stream completion.
    fn is_stream_done(&self, data: &str) -> bool;
}

#[cfg(test)]
mod tests {
    use super::ApiStyle;

    #[test]
    fn api_style_display_uses_neutral_protocol_family_labels() {
        assert_eq!(ApiStyle::OpenAiCompatible.to_string(), "closeai-compatible");
        assert_eq!(
            ApiStyle::EthnopicMessages.to_string(),
            "ethnopic-compatible"
        );
    }
}
