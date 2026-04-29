//! Bridge layer for protocol-agnostic provider driver integration.
//!
//! This module provides two categories of conversions:
//!
//! 1. **Streaming events**: `StreamingEvent` → `StreamEvent`
//!    (used by both existing protocols and DriverBasedProtocol)
//!
//! 2. **Non-streaming responses**: `DriverResponse` → `ChatResponse`
//!    (used by DriverBasedProtocol for simple providers)
//!
//! Additionally, [`DriverBasedProtocol`] implements `ProtocolImpl` by delegating
//! to a `ProviderDriver`, providing a zero-boilerplate path for adding new
//! closeai-compatible or ethnopic-compatible providers.

use crate::driver::{
    ApiStyle, AudioSource, ContentBlock, DriverMessage, DriverMessageContent, DriverMessageRole,
    DriverResponse, ProviderDriver, StreamingEvent,
};
use crate::message::{ChatResponse, Choice, Message, Usage};
use crate::protocol::{ProtocolImpl, ProviderConfig};
use crate::provider::ProviderError;
use crate::stream::{StreamEvent, StreamResult, StreamUsage};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;

fn uses_x_api_key_auth(api_style: ApiStyle) -> bool {
    matches!(api_style, ApiStyle::EthnopicMessages)
}

/// Convert a single `StreamingEvent` into zero or more rocode `StreamEvent`s.
///
/// Returns a Vec because some events (e.g. Metadata) may produce multiple
/// StreamEvents (Usage + FinishStep), while others (e.g. FinalCandidate)
/// produce none.
pub fn streaming_event_to_stream_events(event: StreamingEvent) -> Vec<StreamEvent> {
    match event {
        StreamingEvent::PartialContentDelta { content, .. } => {
            vec![StreamEvent::TextDelta(content)]
        }

        StreamingEvent::ThinkingDelta { thinking, .. } => {
            // StreamingEvent doesn't assign IDs to thinking blocks;
            // use a fixed sentinel so downstream consumers can track state.
            vec![StreamEvent::ReasoningDelta {
                id: "thinking".to_string(),
                text: thinking,
            }]
        }

        StreamingEvent::ToolCallStarted {
            tool_call_id,
            tool_name,
            index,
        } => {
            // Prefer index-based ID for consistency (same strategy as
            // openai_tool_call_id / messages_tool_call_id in stream.rs).
            let id = index
                .map(|i| format!("tool-call-{}", i))
                .unwrap_or(tool_call_id);
            vec![StreamEvent::ToolCallStart {
                id,
                name: tool_name,
            }]
        }

        StreamingEvent::PartialToolCall {
            tool_call_id,
            arguments,
            index,
            ..
        } => {
            let id = index
                .map(|i| format!("tool-call-{}", i))
                .unwrap_or(tool_call_id);
            vec![StreamEvent::ToolCallDelta {
                id,
                input: arguments,
            }]
        }

        StreamingEvent::ToolCallEnded { .. } => {
            // ToolCallEnd in rocode requires assembled input (serde_json::Value).
            // The assemble_tool_calls() wrapper handles this — we don't need
            // to emit anything here.
            vec![]
        }

        StreamingEvent::Metadata {
            usage,
            finish_reason,
            stop_reason,
        } => {
            let mut events = Vec::new();
            let usage_for_step = usage.as_ref().map(extract_usage).unwrap_or_default();

            if let Some(usage_val) = usage {
                let su = extract_usage(&usage_val);
                events.push(StreamEvent::Usage {
                    prompt_tokens: su.prompt_tokens,
                    completion_tokens: su.completion_tokens,
                    context_tokens: su.context_tokens.max(su.prompt_tokens),
                    cache_read_tokens: su.cache_read_tokens,
                    cache_write_tokens: su.cache_write_tokens,
                });
            }

            // finish_reason or stop_reason signals end of a step
            let reason = finish_reason.or(stop_reason);
            if let Some(ref r) = reason {
                let normalized = match r.as_str() {
                    "end_turn" | "stop" => "stop".to_string(),
                    "tool_use" | "tool_calls" => "tool-calls".to_string(),
                    other => other.to_string(),
                };
                events.push(StreamEvent::FinishStep {
                    finish_reason: Some(normalized),
                    usage: usage_for_step,
                    provider_metadata: None,
                });
            }

            events
        }

        StreamingEvent::FinalCandidate { .. } => {
            // Multi-candidate scenarios are not used in rocode.
            vec![]
        }

        StreamingEvent::StreamEnd { .. } => {
            vec![StreamEvent::Done]
        }

        StreamingEvent::StreamError { error, .. } => {
            let msg = if let Some(s) = error.as_str() {
                s.to_string()
            } else {
                error.to_string()
            };
            vec![StreamEvent::Error(msg)]
        }
    }
}

/// Convert a stream of `StreamingEvent`s into a rocode `StreamResult`.
///
/// This is the main entry point for Phase 2 ProviderDriver integration:
/// ```text
/// ProviderDriver::parse_stream_event() → StreamingEvent
///     → bridge_streaming_events() → StreamEvent
///     → assemble_tool_calls() → final StreamResult
/// ```
pub fn bridge_streaming_events(
    input: Pin<Box<dyn Stream<Item = Result<StreamingEvent, ProviderError>> + Send>>,
) -> StreamResult {
    let stream = input.flat_map(|result| {
        let events: Vec<Result<StreamEvent, ProviderError>> = match result {
            Ok(event) => streaming_event_to_stream_events(event)
                .into_iter()
                .map(Ok)
                .collect(),
            Err(e) => vec![Err(e)],
        };
        futures::stream::iter(events)
    });

    crate::stream::assemble_tool_calls(Box::pin(stream))
}

// ---- Phase 2: DriverResponse → ChatResponse conversion ----

/// Convert a `DriverResponse` into a rocode `ChatResponse`.
///
/// This handles the structural mapping between the two response formats:
/// - `DriverResponse.content` → single `Choice` with assistant message
/// - `DriverResponse.finish_reason` → `Choice.finish_reason`
/// - `DriverResponse.usage` → `ChatResponse.usage`
///
/// The `id` and `model` fields are populated from the raw response JSON
/// if available (OpenAI format), or default to empty strings.
pub fn driver_response_to_chat_response(resp: DriverResponse) -> ChatResponse {
    let id = resp
        .raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let model = resp
        .raw
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let content = resp.content.unwrap_or_default();

    let raw_usage = resp.raw.get("usage").map(extract_usage);
    let usage = resp.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cache_read_input_tokens: raw_usage
            .as_ref()
            .and_then(|usage| (usage.cache_read_tokens > 0).then_some(usage.cache_read_tokens)),
        cache_creation_input_tokens: raw_usage
            .as_ref()
            .and_then(|usage| (usage.cache_write_tokens > 0).then_some(usage.cache_write_tokens)),
    });

    ChatResponse {
        id,
        model,
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: resp.finish_reason,
        }],
        usage,
    }
}

// ---- Phase 2: DriverBasedProtocol ----

/// A generic `ProtocolImpl` backed by a `ProviderDriver`.
///
/// This provides a zero-boilerplate way to add new providers that follow
/// standard OpenAI or Anthropic API formats. For providers with custom
/// needs (thinking config, beta headers, Responses API, etc.), use a
/// dedicated protocol implementation instead.
///
/// # Usage
///
/// ```ignore
/// use rocode_provider::driver::{ApiStyle, DriverMessage};
///
/// // Implement ProviderDriver for your provider, then:
/// let protocol = DriverBasedProtocol::new(driver);
/// ```
pub struct DriverBasedProtocol {
    driver: std::sync::Arc<dyn ProviderDriver>,
}

impl DriverBasedProtocol {
    pub fn new(driver: Box<dyn ProviderDriver>) -> Self {
        Self {
            driver: std::sync::Arc::from(driver),
        }
    }
}

#[async_trait]
impl ProtocolImpl for DriverBasedProtocol {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: crate::ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        // Build the URL from config
        let url = if config.base_url.trim().is_empty() {
            return Err(ProviderError::ConfigError(
                "base_url is required for driver-based protocol".to_string(),
            ));
        } else {
            config.base_url.trim_end_matches('/').to_string()
        };

        // Convert rocode ChatRequest → driver params
        let ai_messages = chat_request_to_driver_messages(&request);
        let temperature = request.temperature.map(|t| t as f64);
        let max_tokens = request.max_tokens.map(|t| t as u32);

        let driver_req = self
            .driver
            .build_request(
                &ai_messages,
                &request.model,
                temperature,
                max_tokens,
                false,
                None,
            )
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        // Merge driver headers with config headers
        let mut req_builder = client.post(&url).header("Content-Type", "application/json");

        if !config.api_key.is_empty() {
            // Use x-api-key for ethnopic-compatible/messages-style APIs,
            // Bearer auth for the other families.
            if uses_x_api_key_auth(self.driver.api_style()) {
                req_builder = req_builder.header("x-api-key", &config.api_key);
            } else {
                req_builder =
                    req_builder.header("Authorization", format!("Bearer {}", config.api_key));
            }
        }

        for (key, value) in &driver_req.headers {
            req_builder = req_builder.header(key, value);
        }
        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .json(&driver_req.body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let driver_resp = self
            .driver
            .parse_response(&body)
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(driver_response_to_chat_response(driver_resp))
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: crate::ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let url = if config.base_url.trim().is_empty() {
            return Err(ProviderError::ConfigError(
                "base_url is required for driver-based protocol".to_string(),
            ));
        } else {
            config.base_url.trim_end_matches('/').to_string()
        };

        let ai_messages = chat_request_to_driver_messages(&request);
        let temperature = request.temperature.map(|t| t as f64);
        let max_tokens = request.max_tokens.map(|t| t as u32);

        let driver_req = self
            .driver
            .build_request(
                &ai_messages,
                &request.model,
                temperature,
                max_tokens,
                true,
                None,
            )
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let mut req_builder = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        if !config.api_key.is_empty() {
            // Same auth split as the non-streaming path above.
            if uses_x_api_key_auth(self.driver.api_style()) {
                req_builder = req_builder.header("x-api-key", &config.api_key);
            } else {
                req_builder =
                    req_builder.header("Authorization", format!("Bearer {}", config.api_key));
            }
        }

        for (key, value) in &driver_req.headers {
            req_builder = req_builder.header(key, value);
        }
        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .json(&driver_req.body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        // SSE decode → JSON values (Phase 1 infrastructure)
        let json_stream = crate::stream::decode_sse_stream(response.bytes_stream()).await?;

        // Use driver to parse JSON → StreamingEvent, then bridge → StreamEvent.
        // NOTE: driver.parse_stream_event() takes &str, so we serialize the Value.
        // This is acceptable for the generic path; specialized protocol-family
        // implementations use their own more efficient Value-based parsers.
        let driver = self.driver.clone();
        let stream = json_stream.filter_map(move |result| {
            let driver = driver.clone();
            async move {
                match result {
                    Ok(value) => {
                        let data = match serde_json::to_string(&value) {
                            Ok(data) => data,
                            Err(error) => {
                                tracing::error!(%error, value = ?value, "failed to serialize provider stream event");
                                return Some(Err(ProviderError::StreamError(format!(
                                    "failed to serialize provider stream event: {}",
                                    error
                                ))));
                            }
                        };
                        match driver.parse_stream_event(&data) {
                            Ok(Some(streaming_event)) => {
                                let events = streaming_event_to_stream_events(streaming_event);
                                Some(Ok(events))
                            }
                            Ok(None) => None,
                            Err(e) => Some(Err(ProviderError::StreamError(e.to_string()))),
                        }
                    }
                    Err(e) => Some(Err(e)),
                }
            }
        });

        // Flatten Vec<StreamEvent> into individual events
        let flat_stream = stream.flat_map(|result| match result {
            Ok(events) => {
                let items: Vec<Result<StreamEvent, ProviderError>> =
                    events.into_iter().map(Ok).collect();
                futures::stream::iter(items)
            }
            Err(e) => futures::stream::iter(vec![Err(e)]),
        });

        Ok(crate::stream::assemble_tool_calls(Box::pin(flat_stream)))
    }
}

/// Convert rocode `ChatRequest` messages into `DriverMessage` format.
///
/// Handles multimodal content: text, images (via `image_url`), tool use, and
/// tool results are all properly converted to `ContentBlock`s.
fn chat_request_to_driver_messages(request: &crate::ChatRequest) -> Vec<DriverMessage> {
    let mut messages = Vec::new();

    // Add system message if present
    if let Some(ref system) = request.system {
        messages.push(DriverMessage::system(system));
    }

    for msg in &request.messages {
        let role = match msg.role {
            crate::message::Role::System => DriverMessageRole::System,
            crate::message::Role::User => DriverMessageRole::User,
            crate::message::Role::Assistant => DriverMessageRole::Assistant,
            crate::message::Role::Tool => DriverMessageRole::Tool,
        };

        let driver_msg = match &msg.content {
            crate::message::Content::Text(text) => {
                if role == DriverMessageRole::Tool {
                    // Tool results need tool_call_id — use empty string as fallback
                    DriverMessage::tool("", text)
                } else {
                    DriverMessage {
                        role,
                        content: DriverMessageContent::Text(text.clone()),
                        tool_call_id: None,
                    }
                }
            }
            crate::message::Content::Parts(parts) => {
                let blocks: Vec<ContentBlock> = parts
                    .iter()
                    .filter_map(|p| match p.content_type.as_str() {
                        "text" => p.text.as_ref().map(ContentBlock::text),
                        "image_url" | "image" => p
                            .image_url
                            .as_ref()
                            .map(|img| ContentBlock::image_url(img.url.clone())),
                        "file"
                            if p.media_type
                                .as_deref()
                                .is_some_and(|mime| mime.starts_with("audio/")) =>
                        {
                            p.image_url
                                .as_ref()
                                .map(|audio| driver_audio_block(&audio.url, p.media_type.clone()))
                        }
                        "tool_use" => p.tool_use.as_ref().map(|tu| {
                            ContentBlock::tool_use(tu.id.clone(), tu.name.clone(), tu.input.clone())
                        }),
                        "tool_result" => p.tool_result.as_ref().map(|tr| {
                            ContentBlock::tool_result(
                                tr.tool_use_id.clone(),
                                serde_json::Value::String(tr.content.clone()),
                            )
                        }),
                        _ => p.text.as_ref().map(ContentBlock::text),
                    })
                    .collect();

                if blocks.is_empty() {
                    continue;
                }

                // If all blocks are text, flatten to simple text content
                let all_text = blocks
                    .iter()
                    .all(|b| matches!(b, ContentBlock::Text { .. }));
                if all_text {
                    let text: String = blocks
                        .iter()
                        .map(|b| match b {
                            ContentBlock::Text { text } => text.as_str(),
                            _ => "",
                        })
                        .collect();
                    DriverMessage {
                        role,
                        content: DriverMessageContent::Text(text),
                        tool_call_id: None,
                    }
                } else {
                    DriverMessage::with_blocks(role, blocks)
                }
            }
        };
        messages.push(driver_msg);
    }

    messages
}

fn driver_audio_block(url: &str, media_type: Option<String>) -> ContentBlock {
    if let Some((resolved_media_type, data)) = decode_data_url(url) {
        return ContentBlock::audio_base64(data, Some(resolved_media_type));
    }

    ContentBlock::Audio {
        source: AudioSource {
            source_type: "url".to_string(),
            media_type,
            data: url.to_string(),
        },
    }
}

fn decode_data_url(url: &str) -> Option<(String, String)> {
    if !url.starts_with("data:") {
        return None;
    }

    let (metadata, payload) = url.split_once(',')?;
    let media_type = metadata
        .trim_start_matches("data:")
        .split(';')
        .next()
        .filter(|value| !value.is_empty())?
        .to_string();
    let data = if metadata.contains(";base64") {
        payload.to_string()
    } else {
        urlencoding::decode(payload).ok()?.into_owned()
    };

    Some((media_type, data))
}

/// Extract usage information from a raw JSON value.
///
/// Handles both OpenAI format (`prompt_tokens`/`completion_tokens`)
/// and Anthropic format (`input_tokens`/`output_tokens`).
fn extract_usage(value: &serde_json::Value) -> StreamUsage {
    let prompt = value
        .get("prompt_tokens")
        .or_else(|| value.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let completion = value
        .get("completion_tokens")
        .or_else(|| value.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let reasoning = value
        .get("reasoning_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let cache_read = value
        .get("cache_read_input_tokens")
        .or_else(|| value.get("cache_read_tokens"))
        .or_else(|| value.get("prompt_cache_hit_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let cache_write = value
        .get("cache_creation_input_tokens")
        .or_else(|| value.get("cache_write_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    StreamUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        context_tokens: prompt.max(cache_read.saturating_add(cache_write)),
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::UsageInfo;
    use futures::StreamExt;
    use serde_json::json;

    #[test]
    fn partial_content_delta_maps_to_text_delta() {
        let event = StreamingEvent::PartialContentDelta {
            content: "hello".to_string(),
            sequence_id: Some(1),
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], StreamEvent::TextDelta(s) if s == "hello"));
    }

    #[test]
    fn thinking_delta_maps_to_reasoning_delta() {
        let event = StreamingEvent::ThinkingDelta {
            thinking: "let me think...".to_string(),
            tool_consideration: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::ReasoningDelta { id, text } => {
                assert_eq!(id, "thinking");
                assert_eq!(text, "let me think...");
            }
            other => panic!("expected ReasoningDelta, got: {:?}", other),
        }
    }

    #[test]
    fn tool_call_started_uses_index_based_id() {
        let event = StreamingEvent::ToolCallStarted {
            tool_call_id: "call_abc123".to_string(),
            tool_name: "read".to_string(),
            index: Some(2),
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "tool-call-2");
                assert_eq!(name, "read");
            }
            other => panic!("expected ToolCallStart, got: {:?}", other),
        }
    }

    #[test]
    fn tool_call_started_falls_back_to_tool_call_id() {
        let event = StreamingEvent::ToolCallStarted {
            tool_call_id: "call_xyz".to_string(),
            tool_name: "write".to_string(),
            index: None,
        };
        let result = streaming_event_to_stream_events(event);
        match &result[0] {
            StreamEvent::ToolCallStart { id, .. } => assert_eq!(id, "call_xyz"),
            other => panic!("expected ToolCallStart, got: {:?}", other),
        }
    }

    #[test]
    fn partial_tool_call_maps_to_tool_call_delta() {
        let event = StreamingEvent::PartialToolCall {
            tool_call_id: "tc".to_string(),
            arguments: r#"{"path":"#.to_string(),
            index: Some(0),
            is_complete: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::ToolCallDelta { id, input } => {
                assert_eq!(id, "tool-call-0");
                assert_eq!(input, r#"{"path":"#);
            }
            other => panic!("expected ToolCallDelta, got: {:?}", other),
        }
    }

    #[test]
    fn tool_call_ended_produces_no_events() {
        let event = StreamingEvent::ToolCallEnded {
            tool_call_id: "tc".to_string(),
            index: Some(0),
        };
        let result = streaming_event_to_stream_events(event);
        assert!(result.is_empty());
    }

    #[test]
    fn metadata_with_openai_usage() {
        let event = StreamingEvent::Metadata {
            usage: Some(json!({
                "prompt_tokens": 100,
                "completion_tokens": 50
            })),
            finish_reason: None,
            stop_reason: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
                context_tokens,
                ..
            } => {
                assert_eq!(*prompt_tokens, 100);
                assert_eq!(*completion_tokens, 50);
                assert_eq!(*context_tokens, 100);
            }
            other => panic!("expected Usage, got: {:?}", other),
        }
    }

    #[test]
    fn metadata_with_ethnopic_usage() {
        let event = StreamingEvent::Metadata {
            usage: Some(json!({
                "input_tokens": 200,
                "output_tokens": 80
            })),
            finish_reason: None,
            stop_reason: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
                context_tokens,
                ..
            } => {
                assert_eq!(*prompt_tokens, 200);
                assert_eq!(*completion_tokens, 80);
                assert_eq!(*context_tokens, 200);
            }
            other => panic!("expected Usage, got: {:?}", other),
        }
    }

    #[test]
    fn metadata_with_finish_reason_produces_usage_and_finish_step() {
        let event = StreamingEvent::Metadata {
            usage: Some(json!({"prompt_tokens": 10, "completion_tokens": 5})),
            finish_reason: Some("stop".to_string()),
            stop_reason: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], StreamEvent::Usage { .. }));
        match &result[1] {
            StreamEvent::FinishStep {
                finish_reason: Some(r),
                ..
            } => assert_eq!(r, "stop"),
            other => panic!("expected FinishStep, got: {:?}", other),
        }
    }

    #[test]
    fn metadata_normalizes_ethnopic_stop_reasons() {
        let event = StreamingEvent::Metadata {
            usage: None,
            finish_reason: None,
            stop_reason: Some("end_turn".to_string()),
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::FinishStep {
                finish_reason: Some(r),
                ..
            } => assert_eq!(r, "stop"),
            other => panic!("expected FinishStep, got: {:?}", other),
        }
    }

    #[test]
    fn metadata_normalizes_tool_use_reason() {
        let event = StreamingEvent::Metadata {
            usage: None,
            finish_reason: Some("tool_use".to_string()),
            stop_reason: None,
        };
        let result = streaming_event_to_stream_events(event);
        match &result[0] {
            StreamEvent::FinishStep {
                finish_reason: Some(r),
                ..
            } => assert_eq!(r, "tool-calls"),
            other => panic!("expected FinishStep, got: {:?}", other),
        }
    }

    #[test]
    fn stream_end_maps_to_done() {
        let event = StreamingEvent::StreamEnd {
            finish_reason: Some("stop".to_string()),
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], StreamEvent::Done));
    }

    #[test]
    fn stream_error_maps_to_error() {
        let event = StreamingEvent::StreamError {
            error: json!("rate limit exceeded"),
            event_id: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], StreamEvent::Error(msg) if msg == "rate limit exceeded"));
    }

    #[test]
    fn stream_error_handles_object_error() {
        let event = StreamingEvent::StreamError {
            error: json!({"type": "overloaded_error", "message": "server busy"}),
            event_id: None,
        };
        let result = streaming_event_to_stream_events(event);
        assert_eq!(result.len(), 1);
        match &result[0] {
            StreamEvent::Error(msg) => {
                assert!(msg.contains("overloaded_error"));
            }
            other => panic!("expected Error, got: {:?}", other),
        }
    }

    #[test]
    fn final_candidate_produces_no_events() {
        let event = StreamingEvent::FinalCandidate {
            candidate_index: 0,
            finish_reason: "stop".to_string(),
        };
        let result = streaming_event_to_stream_events(event);
        assert!(result.is_empty());
    }

    #[test]
    fn extract_usage_handles_cache_tokens() {
        let val = json!({
            "input_tokens": 500,
            "output_tokens": 100,
            "cache_read_input_tokens": 400,
            "cache_creation_input_tokens": 50
        });
        let usage = extract_usage(&val);
        assert_eq!(usage.prompt_tokens, 500);
        assert_eq!(usage.completion_tokens, 100);
        assert_eq!(usage.cache_read_tokens, 400);
        assert_eq!(usage.cache_write_tokens, 50);
    }

    #[test]
    fn extract_usage_maps_deepseek_prompt_cache_hit_tokens() {
        let val = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "prompt_cache_hit_tokens": 850,
            "prompt_cache_miss_tokens": 150
        });
        let usage = extract_usage(&val);
        assert_eq!(usage.prompt_tokens, 1000);
        assert_eq!(usage.completion_tokens, 100);
        assert_eq!(usage.cache_read_tokens, 850);
        assert_eq!(usage.cache_write_tokens, 0);
    }

    // ---- Phase 2: DriverResponse → ChatResponse tests ----

    #[test]
    fn driver_response_to_chat_response_openai_format() {
        let resp = DriverResponse {
            content: Some("Hello world".to_string()),
            finish_reason: Some("stop".to_string()),
            usage: Some(UsageInfo {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            tool_calls: vec![],
            raw: json!({
                "id": "chatcmpl-123",
                "model": "gpt-4",
                "choices": [{"message": {"content": "Hello world"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }),
        };

        let chat_resp = driver_response_to_chat_response(resp);
        assert_eq!(chat_resp.id, "chatcmpl-123");
        assert_eq!(chat_resp.model, "gpt-4");
        assert_eq!(chat_resp.choices.len(), 1);
        assert_eq!(chat_resp.choices[0].finish_reason.as_deref(), Some("stop"));
        let usage = chat_resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn driver_response_to_chat_response_preserves_deepseek_cache_usage() {
        let resp = DriverResponse {
            content: Some("cached".to_string()),
            finish_reason: Some("stop".to_string()),
            usage: Some(UsageInfo {
                prompt_tokens: 1000,
                completion_tokens: 25,
                total_tokens: 1025,
            }),
            tool_calls: vec![],
            raw: json!({
                "id": "chatcmpl-cache",
                "model": "deepseek-v4-flash",
                "usage": {
                    "prompt_tokens": 1000,
                    "completion_tokens": 25,
                    "total_tokens": 1025,
                    "prompt_cache_hit_tokens": 900,
                    "prompt_cache_miss_tokens": 100
                }
            }),
        };

        let chat_resp = driver_response_to_chat_response(resp);
        let usage = chat_resp.usage.expect("usage should be present");
        assert_eq!(usage.cache_read_input_tokens, Some(900));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn driver_response_to_chat_response_ethnopic_format() {
        let resp = DriverResponse {
            content: Some("Bonjour!".to_string()),
            finish_reason: Some("stop".to_string()),
            usage: Some(UsageInfo {
                prompt_tokens: 20,
                completion_tokens: 8,
                total_tokens: 28,
            }),
            tool_calls: vec![],
            raw: json!({
                "id": "msg_abc",
                "model": "ethnopic-compatible-model",
                "content": [{"type": "text", "text": "Bonjour!"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 20, "output_tokens": 8}
            }),
        };

        let chat_resp = driver_response_to_chat_response(resp);
        assert_eq!(chat_resp.id, "msg_abc");
        assert_eq!(chat_resp.model, "ethnopic-compatible-model");
        assert_eq!(chat_resp.choices.len(), 1);
        assert_eq!(chat_resp.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn driver_response_to_chat_response_no_content() {
        let resp = DriverResponse {
            content: None,
            finish_reason: Some("tool_calls".to_string()),
            usage: None,
            tool_calls: vec![json!({"type": "tool_use", "name": "bash"})],
            raw: json!({}),
        };

        let chat_resp = driver_response_to_chat_response(resp);
        assert_eq!(chat_resp.id, "");
        assert_eq!(chat_resp.model, "");
        assert!(chat_resp.usage.is_none());
    }

    #[test]
    fn chat_request_to_driver_messages_basic() {
        let request = crate::ChatRequest::new(
            "gpt-4",
            vec![
                crate::Message::system("You are helpful"),
                crate::Message::user("Hi"),
                crate::Message::assistant("Hello!"),
            ],
        )
        .with_system("System prompt");

        let messages = chat_request_to_driver_messages(&request);
        // system prompt + system msg + user + assistant = 4
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn chat_request_to_driver_messages_maps_audio_file_to_audio_block() {
        let request = crate::ChatRequest::new(
            "gpt-4o-audio",
            vec![crate::Message {
                role: crate::message::Role::User,
                content: crate::message::Content::Parts(vec![crate::message::ContentPart {
                    content_type: "file".to_string(),
                    text: None,
                    image_url: Some(crate::message::ImageUrl {
                        url: "data:audio/wav;base64,UklGRg==".to_string(),
                    }),
                    tool_use: None,
                    tool_result: None,
                    cache_control: None,
                    filename: Some("voice.wav".to_string()),
                    media_type: Some("audio/wav".to_string()),
                    provider_options: None,
                }]),
                cache_control: None,
                provider_options: None,
            }],
        );

        let messages = chat_request_to_driver_messages(&request);
        assert_eq!(messages.len(), 1);
        let DriverMessageContent::Blocks(blocks) = &messages[0].content else {
            panic!("expected multimodal blocks");
        };
        assert!(matches!(
            blocks.first(),
            Some(ContentBlock::Audio { source })
                if source.source_type == "base64"
                    && source.media_type.as_deref() == Some("audio/wav")
                    && source.data == "UklGRg=="
        ));
    }

    #[tokio::test]
    async fn bridge_streaming_events_produces_correct_stream() {
        let input_events = vec![
            Ok(StreamingEvent::PartialContentDelta {
                content: "Hello".to_string(),
                sequence_id: None,
            }),
            Ok(StreamingEvent::ToolCallStarted {
                tool_call_id: "tc-0".to_string(),
                tool_name: "read".to_string(),
                index: Some(0),
            }),
            Ok(StreamingEvent::PartialToolCall {
                tool_call_id: "tc-0".to_string(),
                arguments: r#"{"path":"/tmp/a"}"#.to_string(),
                index: Some(0),
                is_complete: Some(true),
            }),
            Ok(StreamingEvent::StreamEnd {
                finish_reason: Some("tool_use".to_string()),
            }),
        ];

        let input: Pin<Box<dyn Stream<Item = Result<StreamingEvent, ProviderError>> + Send>> =
            Box::pin(futures::stream::iter(input_events));

        let output: Vec<StreamEvent> = bridge_streaming_events(input)
            .map(|r| r.expect("should be Ok"))
            .collect()
            .await;

        // Should have: TextDelta, ToolCallStart, ToolCallDelta, ToolCallEnd (from assembler), Done
        assert!(output
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(s) if s == "Hello")));
        assert!(output
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolCallStart { name, .. } if name == "read")));
        assert!(output
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolCallEnd { .. })));
        assert!(output.iter().any(|e| matches!(e, StreamEvent::Done)));
    }
}
