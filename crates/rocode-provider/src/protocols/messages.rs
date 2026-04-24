use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Message, ProtocolImpl, ProviderConfig, ProviderError,
    StreamResult, Usage,
};

// Protocol-level default URL for the Messages API.
// This is the technical endpoint for the messages protocol (analogous to
// OPENAI_API_URL for the OpenAI chat-completions protocol).  Users override
// this via `base_url` in their provider configuration.
const MESSAGES_DEFAULT_URL: &str = "https://api.anthropic.com/v1/messages";

fn runtime_pipeline_enabled(config: &ProviderConfig) -> bool {
    config
        .option_bool(&["runtime_pipeline"])
        .unwrap_or_else(|| {
            std::env::var("ROCODE_RUNTIME_PIPELINE")
                .ok()
                .and_then(|v| {
                    let lower = v.trim().to_ascii_lowercase();
                    if matches!(lower.as_str(), "1" | "true" | "yes" | "on") {
                        Some(true)
                    } else if matches!(lower.as_str(), "0" | "false" | "no" | "off") {
                        Some(false)
                    } else {
                        None
                    }
                })
                .unwrap_or(true)
        })
}

/// Build the messages endpoint URL from a user-supplied base URL.
/// The generic messages-family transport appends `/messages` when the base URL
/// points at the provider root. The built-in default still targets the
/// public `/v1/messages` endpoint.
fn messages_url(base_url: &str) -> String {
    let base = base_url.trim();
    if base.is_empty() {
        return MESSAGES_DEFAULT_URL.to_string();
    }
    if base.ends_with("/messages") {
        return base.to_string();
    }
    let base = base.trim_end_matches('/');
    format!("{base}/messages")
}

pub struct MessagesProtocol;

impl Default for MessagesProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl MessagesProtocol {
    pub fn new() -> Self {
        Self
    }

    fn convert_request(request: ChatRequest) -> MessagesRequest {
        let max_tokens = request.max_tokens.unwrap_or(16_000);
        let mut messages = Vec::new();
        let mut system = request.system;

        for msg in request.messages {
            match msg.role {
                crate::Role::System => {
                    if let crate::Content::Text(text) = msg.content {
                        system = Some(text);
                    }
                }
                _ => {
                    let mut content = Vec::new();
                    match msg.content {
                        crate::Content::Text(text) => {
                            if !text.is_empty() {
                                content.push(MessagesContent::Text { text });
                            }
                        }
                        crate::Content::Parts(parts) => {
                            for part in parts {
                                if part.content_type == "reasoning" {
                                    if let Some(text) = part.text {
                                        if !text.is_empty() {
                                            content
                                                .push(MessagesContent::Thinking { thinking: text });
                                        }
                                    }
                                } else if let Some(text) = part.text {
                                    if !text.is_empty() {
                                        content.push(MessagesContent::Text { text });
                                    }
                                }
                                if let Some(tool_use) = part.tool_use {
                                    content.push(MessagesContent::ToolUse {
                                        id: tool_use.id,
                                        name: tool_use.name,
                                        input: tool_use.input,
                                    });
                                }
                                if let Some(tool_result) = part.tool_result {
                                    content.push(MessagesContent::ToolResult {
                                        tool_use_id: tool_result.tool_use_id,
                                        content: tool_result.content,
                                        is_error: tool_result.is_error,
                                    });
                                }
                            }
                        }
                    }

                    if content.is_empty() {
                        continue;
                    }

                    messages.push(MessagesMessage {
                        role: match msg.role {
                            crate::Role::User => "user".to_string(),
                            crate::Role::Assistant => "assistant".to_string(),
                            crate::Role::Tool => "user".to_string(),
                            crate::Role::System => "user".to_string(),
                        },
                        content,
                    });
                }
            }
        }

        let tools = request.tools.and_then(|tools| {
            if tools.is_empty() {
                None
            } else {
                Some(
                    tools
                        .into_iter()
                        .map(|tool| MessagesTool {
                            name: tool.name,
                            description: tool.description,
                            input_schema: tool.parameters,
                        })
                        .collect(),
                )
            }
        });

        MessagesRequest {
            model: request.model,
            max_tokens,
            messages,
            system,
            tools,
            stream: request.stream,
            thinking: messages_thinking_config(request.variant.as_deref(), max_tokens),
        }
    }
}

#[async_trait]
impl ProtocolImpl for MessagesProtocol {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let url = messages_url(&config.base_url);
        tracing::debug!(url = %url, model = %request.model, "messages protocol chat request");

        let messages_request = Self::convert_request(request);

        let mut req_builder = client
            .post(&url)
            // Messages API protocol headers — required by the API specification,
            // not brand-specific.  Analogous to OpenAI's "Authorization: Bearer".
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .json(&messages_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            tracing::error!(url = %url, status = %status, "messages protocol chat error");
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let messages_response: MessagesResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_response(messages_response))
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let use_pipeline = runtime_pipeline_enabled(config);
        let url = messages_url(&config.base_url);
        tracing::debug!(
            url = %url,
            model = %request.model,
            "messages protocol chat_stream request"
        );

        let mut messages_request = Self::convert_request(request);
        messages_request.stream = Some(true);

        tracing::debug!(
            model = %messages_request.model,
            thinking_enabled = ?messages_request.thinking,
            "messages protocol chat_stream request"
        );

        let mut req_builder = client
            .post(&url)
            // Messages API protocol headers (see non-streaming chat() for rationale).
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");

        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .json(&messages_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            tracing::error!(
                url = %url,
                status = %status,
                "messages protocol chat_stream error"
            );
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        if use_pipeline {
            let pipeline = crate::runtime::pipeline::Pipeline::ethnopic_default();
            let streaming_events = pipeline.process_stream(Box::pin(response.bytes_stream()));
            return Ok(crate::stream::pipeline_to_stream_result(streaming_events));
        }

        let json_stream = crate::stream::decode_sse_stream(response.bytes_stream()).await?;

        let stream = futures::stream::unfold(
            (json_stream, std::collections::HashMap::<u32, String>::new()),
            |(mut json_stream, mut block_types)| async move {
                match json_stream.next().await {
                    Some(Ok(value)) => {
                        let event =
                            crate::stream::parse_ethnopic_value_stateful(value, &mut block_types);
                        if let Some(ref e) = event {
                            tracing::trace!(event = ?e, "messages protocol sse event");
                        }
                        Some((event.map(Ok), (json_stream, block_types)))
                    }
                    Some(Err(e)) => Some((Some(Err(e)), (json_stream, block_types))),
                    None => None,
                }
            },
        )
        .filter_map(|x| async { x });

        Ok(crate::stream::assemble_tool_calls(Box::pin(stream)))
    }
}

// ---- Request/Response types ----

#[derive(Debug, Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u64,
    messages: Vec<MessagesMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<MessagesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<MessagesThinking>,
}

#[derive(Debug, Serialize)]
struct MessagesMessage {
    role: String,
    content: Vec<MessagesContent>,
}

#[derive(Debug, Serialize)]
struct MessagesTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(rename = "input_schema")]
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum MessagesContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum MessagesThinking {
    #[serde(rename = "enabled")]
    Enabled {
        #[serde(rename = "budget_tokens")]
        budget_tokens: u64,
    },
}

fn messages_thinking_config(variant: Option<&str>, max_tokens: u64) -> Option<MessagesThinking> {
    let target = if let Some(v) = variant {
        let v = v.trim().to_ascii_lowercase();
        match v.as_str() {
            "low" => 4_000,
            "medium" => 8_000,
            "high" => 16_000,
            "max" | "xhigh" => 31_999,
            _ => 16_000, // Default to high if unrecognized
        }
    } else {
        16_000 // Default to high if no variant specified
    };

    let ceiling = max_tokens.saturating_sub(1);
    let budget_tokens = target.min(ceiling);
    if budget_tokens == 0 {
        return None;
    }
    Some(MessagesThinking::Enabled { budget_tokens })
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    id: String,
    model: String,
    content: Vec<MessagesResponseContent>,
    usage: MessagesResponseUsage,
}

#[derive(Debug, Deserialize)]
struct MessagesResponseContent {
    #[serde(rename = "type")]
    _content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessagesResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
}

// ---- Helpers ----

fn convert_response(response: MessagesResponse) -> ChatResponse {
    let content = response
        .content
        .iter()
        .filter_map(|c| c.text.clone())
        .collect::<Vec<_>>()
        .join("");

    ChatResponse {
        id: response.id,
        model: response.model,
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(Usage {
            prompt_tokens: response.usage.input_tokens,
            completion_tokens: response.usage.output_tokens,
            total_tokens: response.usage.input_tokens + response.usage.output_tokens,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_url_empty_falls_back_to_default() {
        assert_eq!(messages_url(""), MESSAGES_DEFAULT_URL);
        assert_eq!(messages_url("  "), MESSAGES_DEFAULT_URL);
    }

    #[test]
    fn messages_url_appends_messages_path() {
        assert_eq!(
            messages_url("https://coding.dashscope.aliyuncs.com/apps/anthropic/v1"),
            "https://coding.dashscope.aliyuncs.com/apps/anthropic/v1/messages"
        );
    }

    #[test]
    fn messages_url_no_double_append() {
        assert_eq!(
            messages_url("https://example.com/v1/messages"),
            "https://example.com/v1/messages"
        );
    }

    #[test]
    fn messages_url_strips_trailing_slash() {
        assert_eq!(
            messages_url("https://example.com/v1/"),
            "https://example.com/v1/messages"
        );
    }
}
