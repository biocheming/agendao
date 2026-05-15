use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::request_sanitizer::{sanitize_messages_for_protocol, SanitizerOptions};
use super::thinking_continuation::request_effectively_enables_thinking;
use crate::runtime::runtime_pipeline_enabled;
use crate::{
    ChatRequest, ChatResponse, Choice, Message, ProviderAdapter, ProviderConfig, ProviderError,
    Role, StreamResult, Usage,
};

// Default URL for the Ethnopic-compatible messages wire shape. Users override
// this via `base_url` in their provider configuration.
const ETHNOPIC_DEFAULT_URL: &str = "https://api.anthropic.com/v1/messages";

/// Build the Ethnopic-compatible endpoint URL from a user-supplied base URL.
/// The generic ethnopic-family transport appends `/messages` when the base URL
/// points at the provider root. The built-in default still targets the
/// public `/v1/messages` endpoint.
fn ethnopic_url(base_url: &str) -> String {
    let base = base_url.trim();
    if base.is_empty() {
        return ETHNOPIC_DEFAULT_URL.to_string();
    }
    if base.ends_with("/messages") {
        return base.to_string();
    }
    let base = base.trim_end_matches('/');
    format!("{base}/messages")
}

pub struct EthnopicAdapter;

impl Default for EthnopicAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl EthnopicAdapter {
    pub fn new() -> Self {
        Self
    }

    fn convert_request(request: ChatRequest) -> MessagesRequest {
        let max_tokens = request.max_tokens.unwrap_or(16_000);
        let thinking_enabled = request_effectively_enables_thinking(&request, true);
        let mut messages = Vec::new();
        let mut system = request.system;
        let sanitized = sanitize_messages_for_protocol(
            &request.messages,
            SanitizerOptions {
                drop_thinking_only_assistant: true,
                ..Default::default()
            },
        );

        for msg in sanitized {
            match msg.role {
                crate::Role::System => {
                    if let crate::Content::Text(text) = msg.content {
                        system = Some(text);
                    }
                }
                crate::Role::Assistant => {
                    let Some(message) = assistant_message_to_ethnopic(msg) else {
                        continue;
                    };
                    messages.push(message);
                }
                crate::Role::User | crate::Role::Tool => {
                    let content = prompt_message_to_ethnopic_user_blocks(&msg);
                    if content.is_empty() {
                        continue;
                    }
                    push_or_merge_user_message(&mut messages, content);
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
            thinking: messages_thinking_config(
                request.variant.as_deref(),
                max_tokens,
                thinking_enabled,
            ),
        }
    }
}

fn push_or_merge_user_message(messages: &mut Vec<MessagesMessage>, content: Vec<MessagesContent>) {
    if content.is_empty() {
        return;
    }

    if let Some(last) = messages.last_mut() {
        if last.role == "user" {
            last.content.extend(content);
            return;
        }
    }

    messages.push(MessagesMessage {
        role: "user".to_string(),
        content,
    });
}

fn assistant_message_to_ethnopic(message: Message) -> Option<MessagesMessage> {
    match message.content {
        crate::Content::Text(text) => {
            if text.is_empty() {
                None
            } else {
                Some(MessagesMessage {
                    role: "assistant".to_string(),
                    content: vec![MessagesContent::Text { text }],
                })
            }
        }
        crate::Content::Parts(parts) => {
            let mut thinking = String::new();
            let mut text = String::new();
            let mut tool_uses = Vec::new();

            for part in parts {
                match part.content_type.as_str() {
                    "reasoning" | "thinking" => {
                        if let Some(part_text) = part.text {
                            if !part_text.is_empty() {
                                thinking.push_str(&part_text);
                            }
                        }
                    }
                    "tool_use" => {
                        if let Some(tool_use) = part.tool_use {
                            tool_uses.push(MessagesContent::ToolUse {
                                id: tool_use.id,
                                name: tool_use.name,
                                input: tool_use.input,
                            });
                        }
                    }
                    _ => {
                        if let Some(part_text) = part.text {
                            if !part_text.is_empty() {
                                text.push_str(&part_text);
                            }
                        }
                    }
                }
            }

            let mut content = Vec::new();
            if !thinking.is_empty() {
                content.push(MessagesContent::Thinking { thinking });
            }
            if !text.is_empty() {
                content.push(MessagesContent::Text { text });
            }
            content.extend(tool_uses);

            if content.is_empty() || content.iter().all(MessagesContent::is_thinking) {
                None
            } else {
                Some(MessagesMessage {
                    role: "assistant".to_string(),
                    content,
                })
            }
        }
    }
}

fn prompt_message_to_ethnopic_user_blocks(message: &Message) -> Vec<MessagesContent> {
    if matches!(message.role, Role::User) {
        return user_textual_content_to_ethnopic(&message.content);
    }

    match &message.content {
        crate::Content::Text(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![MessagesContent::Text { text: text.clone() }]
            }
        }
        crate::Content::Parts(parts) => {
            let mut content = Vec::new();
            for part in parts {
                if let Some(tool_result) = &part.tool_result {
                    content.push(MessagesContent::ToolResult {
                        tool_use_id: tool_result.tool_use_id.clone(),
                        content: tool_result.content.clone(),
                        is_error: tool_result.is_error,
                    });
                    continue;
                }

                if let Some(text) = &part.text {
                    if !text.is_empty() {
                        content.push(MessagesContent::Text { text: text.clone() });
                    }
                }
            }
            content
        }
    }
}

fn user_textual_content_to_ethnopic(content: &crate::Content) -> Vec<MessagesContent> {
    match content {
        crate::Content::Text(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![MessagesContent::Text { text: text.clone() }]
            }
        }
        crate::Content::Parts(parts) => {
            if parts.len() == 1 && parts[0].content_type == "text" && parts[0].text.is_some() {
                return vec![MessagesContent::Text {
                    text: parts[0].text.clone().unwrap_or_default(),
                }];
            }

            let mut content = Vec::new();
            for part in parts {
                if let Some(text) = &part.text {
                    if !text.is_empty() {
                        content.push(MessagesContent::Text { text: text.clone() });
                    }
                }
            }
            content
        }
    }
}

#[async_trait]
impl ProviderAdapter for EthnopicAdapter {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let url = ethnopic_url(&config.base_url);
        tracing::debug!(url = %url, model = %request.model, "ethnopic adapter chat request");

        let messages_request = Self::convert_request(request);

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_json_content_type(
                crate::transport::apply_messages_api_headers(client.post(&url), config),
            ),
            config,
        );

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
            tracing::error!(url = %url, status = %status, "ethnopic adapter chat error");
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
        let url = ethnopic_url(&config.base_url);
        tracing::debug!(
            url = %url,
            model = %request.model,
            "ethnopic adapter chat_stream request"
        );

        let mut messages_request = Self::convert_request(request);
        messages_request.stream = Some(true);

        tracing::debug!(
            model = %messages_request.model,
            thinking_enabled = ?messages_request.thinking,
            "ethnopic adapter chat_stream request"
        );

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_sse_accept(crate::transport::apply_json_content_type(
                crate::transport::apply_messages_api_headers(client.post(&url), config),
            )),
            config,
        );

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
                "ethnopic adapter chat_stream error"
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
                            tracing::trace!(event = ?e, "ethnopic adapter sse event");
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

impl MessagesContent {
    fn is_thinking(&self) -> bool {
        matches!(self, Self::Thinking { .. })
    }
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

fn messages_thinking_config(
    variant: Option<&str>,
    max_tokens: u64,
    thinking_enabled: bool,
) -> Option<MessagesThinking> {
    if !thinking_enabled {
        return None;
    }

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
            cache_miss_input_tokens: None,
            cache_creation_input_tokens: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Content, ContentPart, Role};
    use serde_json::json;

    #[test]
    fn ethnopic_url_empty_falls_back_to_default() {
        assert_eq!(ethnopic_url(""), ETHNOPIC_DEFAULT_URL);
        assert_eq!(ethnopic_url("  "), ETHNOPIC_DEFAULT_URL);
    }

    #[test]
    fn ethnopic_url_appends_messages_path() {
        assert_eq!(
            ethnopic_url("https://coding.dashscope.aliyuncs.com/apps/anthropic/v1"),
            "https://coding.dashscope.aliyuncs.com/apps/anthropic/v1/messages"
        );
    }

    #[test]
    fn ethnopic_url_no_double_append() {
        assert_eq!(
            ethnopic_url("https://example.com/v1/messages"),
            "https://example.com/v1/messages"
        );
    }

    #[test]
    fn ethnopic_url_strips_trailing_slash() {
        assert_eq!(
            ethnopic_url("https://example.com/v1/"),
            "https://example.com/v1/messages"
        );
    }

    #[test]
    fn drops_thinking_only_assistant_messages_from_wire_copy() {
        let request = ChatRequest {
            model: "claude-test".to_string(),
            messages: vec![
                Message::user("first"),
                Message::assistant_parts(vec![ContentPart::reasoning("hidden chain")]),
                Message::user("second"),
            ],
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let converted = EthnopicAdapter::convert_request(request);
        assert_eq!(converted.messages.len(), 1);
        assert_eq!(converted.messages[0].role, "user");
        assert_eq!(converted.messages[0].content.len(), 2);
        assert!(matches!(
            &converted.messages[0].content[0],
            MessagesContent::Text { text } if text == "first"
        ));
        assert!(matches!(
            &converted.messages[0].content[1],
            MessagesContent::Text { text } if text == "second"
        ));
    }

    #[test]
    fn respects_explicit_thinking_disable_in_provider_options() {
        let request = ChatRequest {
            model: "claude-test".to_string(),
            messages: vec![Message::user("first")],
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: Some(std::collections::HashMap::from([(
                "thinking".to_string(),
                json!({"type": "disabled"}),
            )])),
            variant: Some("high".to_string()),
        };

        let converted = EthnopicAdapter::convert_request(request);
        assert!(converted.thinking.is_none());
    }

    #[test]
    // P2.3: reused call_id history must be sanitized before Ethnopic (messages) transport.
    fn injects_interrupted_tool_result_per_assistant_segment_even_with_reused_call_id() {
        let request = ChatRequest {
            model: "claude-test".to_string(),
            messages: vec![
                Message::assistant_parts(vec![ContentPart::tool_use(
                    "tool-call-0",
                    "invalid",
                    json!({ "tool": "skill_manage" }),
                )]),
                Message::user("follow up"),
                Message::assistant_parts(vec![ContentPart::tool_use(
                    "tool-call-0",
                    "skills_list",
                    json!({}),
                )]),
                Message::tool_parts(vec![ContentPart::tool_result(
                    "tool-call-0",
                    "{\"skills\":[]}",
                    None,
                )]),
            ],
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let converted = EthnopicAdapter::convert_request(request);
        assert_eq!(converted.messages.len(), 4);

        assert_eq!(converted.messages[0].role, "assistant");
        assert!(matches!(
            &converted.messages[0].content[0],
            MessagesContent::ToolUse { id, name, .. }
            if id == "tool-call-0" && name == "invalid"
        ));

        assert_eq!(converted.messages[1].role, "user");
        assert_eq!(converted.messages[1].content.len(), 2);
        assert!(matches!(
            &converted.messages[1].content[0],
            MessagesContent::ToolResult {
                tool_use_id,
                content,
                is_error: Some(true)
            } if tool_use_id == "tool-call-0" && content == "[Tool execution was interrupted]"
        ));
        assert!(matches!(
            &converted.messages[1].content[1],
            MessagesContent::Text { text } if text == "follow up"
        ));

        assert_eq!(converted.messages[2].role, "assistant");
        assert!(matches!(
            &converted.messages[2].content[0],
            MessagesContent::ToolUse { id, name, .. }
            if id == "tool-call-0--dedup-1" && name == "skills_list"
        ));

        assert_eq!(converted.messages[3].role, "user");
        assert!(matches!(
            &converted.messages[3].content[0],
            MessagesContent::ToolResult {
                tool_use_id,
                content,
                is_error: None
            } if tool_use_id == "tool-call-0--dedup-1" && content == "{\"skills\":[]}"
        ));
    }

    #[test]
    fn drops_orphan_tool_results_without_matching_pending_tool_use() {
        let request = ChatRequest {
            model: "claude-test".to_string(),
            messages: vec![Message::tool_parts(vec![ContentPart::tool_result(
                "orphan-call",
                "ignored",
                Some(true),
            )])],
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let converted = EthnopicAdapter::convert_request(request);
        assert!(converted.messages.is_empty());
    }

    #[test]
    fn preserves_visible_text_while_replaying_reasoning_before_tool_use() {
        let request = ChatRequest {
            model: "claude-test".to_string(),
            messages: vec![Message {
                role: Role::Assistant,
                content: Content::Parts(vec![
                    ContentPart::reasoning("plan"),
                    ContentPart::text("visible"),
                    ContentPart::tool_use("call-1", "ls", json!({ "path": "." })),
                ]),
                cache_control: None,
                provider_options: None,
            }],
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let converted = EthnopicAdapter::convert_request(request);
        assert_eq!(converted.messages.len(), 2);
        assert_eq!(converted.messages[0].role, "assistant");
        assert_eq!(converted.messages[0].content.len(), 3);
        assert!(matches!(
            &converted.messages[0].content[0],
            MessagesContent::Thinking { thinking } if thinking == "plan"
        ));
        assert!(matches!(
            &converted.messages[0].content[1],
            MessagesContent::Text { text } if text == "visible"
        ));
        assert!(matches!(
            &converted.messages[0].content[2],
            MessagesContent::ToolUse { id, name, .. } if id == "call-1" && name == "ls"
        ));
        assert!(matches!(
            &converted.messages[1].content[0],
            MessagesContent::ToolResult {
                tool_use_id,
                content,
                is_error: Some(true)
            } if tool_use_id == "call-1" && content == "[Tool execution was interrupted]"
        ));
    }
}
