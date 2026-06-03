use async_trait::async_trait;
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use super::request_sanitizer::{content_visible_text_lossy, sanitize_messages_for_text_protocol};
use crate::runtime::runtime_pipeline_enabled;
use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ProviderAdapter, ProviderConfig,
    ProviderError, Role, StreamEvent, StreamResult, Usage,
};

const GOOGLE_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

pub struct GeminiAdapter;

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiAdapter {
    pub fn new() -> Self {
        Self
    }

    fn convert_request(request: ChatRequest) -> GoogleRequest {
        let mut contents = Vec::new();
        let mut system_instruction = None;
        let sanitized = sanitize_messages_for_text_protocol(&request.messages);

        for msg in sanitized {
            match msg.role {
                Role::System => {
                    if let Content::Text(text) = msg.content {
                        system_instruction = Some(GoogleContent {
                            parts: vec![GooglePart::text(&text)],
                            role: "user".to_string(),
                        });
                    }
                }
                Role::User => {
                    let text_content = content_visible_text_lossy(&msg.content);
                    contents.push(GoogleContent {
                        parts: vec![GooglePart::text(&text_content)],
                        role: "user".to_string(),
                    });
                }
                Role::Assistant => {
                    let text_content = content_visible_text_lossy(&msg.content);
                    contents.push(GoogleContent {
                        parts: vec![GooglePart::text(&text_content)],
                        role: "model".to_string(),
                    });
                }
                Role::Tool => {}
            }
        }

        GoogleRequest {
            contents,
            system_instruction,
            generation_config: Some(GenerationConfig {
                max_output_tokens: request.max_tokens,
                temperature: request.temperature,
            }),
        }
    }
}

#[async_trait]
impl ProviderAdapter for GeminiAdapter {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let base_url = if config.base_url.trim().is_empty() {
            GOOGLE_API_URL
        } else {
            config.base_url.trim()
        };
        let url = format!(
            "{}/{}:generateContent?key={}",
            base_url, request.model, config.api_key
        );

        let google_request = Self::convert_request(request);

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_json_content_type(client.post(&url)),
            config,
        );

        let response = req_builder
            .json(&google_request)
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

        let google_response: GoogleResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_google_response(google_response))
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let use_pipeline = runtime_pipeline_enabled(config);
        let base_url = if config.base_url.trim().is_empty() {
            GOOGLE_API_URL
        } else {
            config.base_url.trim()
        };
        let url = format!(
            "{}/{}:streamGenerateContent?key={}&alt=sse",
            base_url, request.model, config.api_key
        );

        let google_request = Self::convert_request(request);

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_sse_accept(crate::transport::apply_json_content_type(
                client.post(&url),
            )),
            config,
        );

        let response = req_builder
            .json(&google_request)
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

        if use_pipeline {
            let pipeline = crate::runtime::pipeline::Pipeline::google_default();
            let streaming_events = pipeline.process_stream(Box::pin(response.bytes_stream()));
            return Ok(crate::stream::pipeline_to_stream_result(streaming_events));
        }

        let stream = stream::try_unfold(
            (
                response.bytes_stream(),
                String::new(),
                VecDeque::<StreamEvent>::new(),
                false,
            ),
            |(mut chunks, mut buffer, mut pending, mut exhausted)| async move {
                loop {
                    if let Some(event) = pending.pop_front() {
                        return Ok(Some((event, (chunks, buffer, pending, exhausted))));
                    }
                    if exhausted {
                        return Ok(None);
                    }

                    match chunks.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                            pending.extend(drain_google_sse_events(&mut buffer, false));
                        }
                        Some(Err(e)) => return Err(ProviderError::StreamError(e.to_string())),
                        None => {
                            exhausted = true;
                            pending.extend(drain_google_sse_events(&mut buffer, true));
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}

// ---- Request/Response types ----

#[derive(Debug, Serialize)]
struct GoogleRequest {
    contents: Vec<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GoogleContent {
    parts: Vec<GooglePart>,
    role: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GooglePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

impl GooglePart {
    fn text(t: &str) -> Self {
        Self {
            text: Some(t.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct GoogleResponse {
    candidates: Vec<GoogleCandidate>,
    usage_metadata: Option<GoogleUsage>,
}

#[derive(Debug, Deserialize)]
struct GoogleCandidate {
    content: GoogleContent,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUsage {
    prompt_token_count: u64,
    candidates_token_count: u64,
    total_token_count: u64,
}

// ---- Helpers ----

fn convert_google_response(response: GoogleResponse) -> ChatResponse {
    let content = response
        .candidates
        .first()
        .and_then(|c| c.content.parts.first())
        .and_then(|p| p.text.clone())
        .unwrap_or_default();

    let usage = response.usage_metadata.map(|u| Usage {
        prompt_tokens: u.prompt_token_count,
        completion_tokens: u.candidates_token_count,
        total_tokens: u.total_token_count,
        cache_read_input_tokens: None,
        cache_miss_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    ChatResponse {
        id: format!("google_{}", uuid::Uuid::new_v4()),
        model: "google".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: response
                .candidates
                .first()
                .and_then(|c| c.finish_reason.clone()),
        }],
        usage,
    }
}

fn parse_google_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }
    if data == "[DONE]" {
        return Some(StreamEvent::Done);
    }

    let response: GoogleResponse = serde_json::from_str(data).ok()?;

    let text = response
        .candidates
        .first()?
        .content
        .parts
        .first()?
        .text
        .clone()?;

    Some(StreamEvent::TextDelta(text))
}

fn drain_google_sse_events(buffer: &mut String, flush: bool) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    while let Some(newline_idx) = buffer.find('\n') {
        let line = buffer[..newline_idx]
            .trim_end_matches('\r')
            .trim()
            .to_string();
        buffer.drain(..=newline_idx);
        if let Some(data) = line.strip_prefix("data: ") {
            if let Some(event) = parse_google_sse(data) {
                events.push(event);
            }
        }
    }

    if flush {
        let line = buffer.trim();
        if !line.is_empty() {
            if let Some(data) = line.strip_prefix("data: ") {
                if let Some(event) = parse_google_sse(data) {
                    events.push(event);
                }
            } else if let Some(event) = parse_google_sse(line) {
                events.push(event);
            }
        }
        buffer.clear();
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContentPart;
    use serde_json::json;

    #[test]
    fn convert_request_drops_tool_only_history_from_text_protocol() {
        let request = ChatRequest {
            model: "gemini-test".to_string(),
            messages: vec![
                Message::user("before"),
                Message::assistant_parts(vec![ContentPart::tool_use("call-1", "ls", json!({}))]),
                Message::tool_parts(vec![ContentPart::tool_result("call-1", "ok", None)]),
                Message::user("after"),
            ],
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let converted = GeminiAdapter::convert_request(request);
        assert_eq!(converted.contents.len(), 1);
        assert_eq!(converted.contents[0].role, "user");
        assert_eq!(
            converted.contents[0].parts[0].text.as_deref(),
            Some("before\n\nafter")
        );
    }

    #[test]
    fn convert_request_drops_thinking_only_assistant_from_text_protocol() {
        let request = ChatRequest {
            model: "gemini-test".to_string(),
            messages: vec![
                Message::user("first"),
                Message::assistant_parts(vec![ContentPart::reasoning("hidden")]),
                Message::user("second"),
            ],
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let converted = GeminiAdapter::convert_request(request);
        assert_eq!(converted.contents.len(), 1);
        assert_eq!(
            converted.contents[0].parts[0].text.as_deref(),
            Some("first\n\nsecond")
        );
    }
}
