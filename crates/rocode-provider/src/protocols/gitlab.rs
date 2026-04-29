use async_trait::async_trait;
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ProtocolImpl, ProviderConfig,
    ProviderError, Role, StreamEvent, StreamResult, Usage,
};

const GITLAB_API_URL: &str = "https://gitlab.com/api/v4/ai/chat/completions";

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

pub struct GitLabProtocol;

impl Default for GitLabProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl GitLabProtocol {
    pub fn new() -> Self {
        Self
    }

    fn get_api_url(config: &ProviderConfig) -> String {
        if config.base_url.trim().is_empty() {
            GITLAB_API_URL.to_string()
        } else {
            let base = config.base_url.trim_end_matches('/');
            format!("{}/api/v4/ai/chat/completions", base)
        }
    }

    fn convert_request(request: ChatRequest) -> GitLabRequest {
        let messages: Vec<GitLabMessage> = request
            .messages
            .into_iter()
            .map(|msg| GitLabMessage {
                role: match msg.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::Tool => "user".to_string(),
                },
                content: match msg.content {
                    Content::Text(t) => GitLabContent::Text(t),
                    Content::Parts(parts) => {
                        let text = parts
                            .iter()
                            .filter_map(|p| p.text.as_ref())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");
                        GitLabContent::Text(text)
                    }
                },
            })
            .collect();

        GitLabRequest {
            model: request.model,
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: false,
        }
    }
}

#[async_trait]
impl ProtocolImpl for GitLabProtocol {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let url = Self::get_api_url(config);
        let gitlab_request = Self::convert_request(request);

        let mut req_builder = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("PRIVATE-TOKEN", &config.api_key);

        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .json(&gitlab_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            return Err(ProviderError::api_error_with_status(
                format!("{}: {}", status, body),
                status.as_u16(),
            ));
        }

        let gitlab_response: GitLabResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_gitlab_response(gitlab_response))
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let use_pipeline = runtime_pipeline_enabled(config);
        let url = Self::get_api_url(config);
        let mut gitlab_request = Self::convert_request(request);
        gitlab_request.stream = true;

        let mut req_builder = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("PRIVATE-TOKEN", &config.api_key)
            .header("Accept", "text/event-stream");

        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .json(&gitlab_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            return Err(ProviderError::api_error_with_status(
                format!("{}: {}", status, body),
                status.as_u16(),
            ));
        }

        if use_pipeline {
            let pipeline = crate::runtime::pipeline::Pipeline::openai_default();
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
                            pending.extend(drain_gitlab_sse_events(&mut buffer, false));
                        }
                        Some(Err(e)) => return Err(ProviderError::StreamError(e.to_string())),
                        None => {
                            exhausted = true;
                            pending.extend(drain_gitlab_sse_events(&mut buffer, true));
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
struct GitLabRequest {
    model: String,
    messages: Vec<GitLabMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct GitLabMessage {
    role: String,
    content: GitLabContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum GitLabContent {
    Text(String),
}

#[derive(Debug, Deserialize)]
struct GitLabResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<GitLabChoice>,
    usage: Option<GitLabUsage>,
}

#[derive(Debug, Deserialize)]
struct GitLabChoice {
    _index: u32,
    message: GitLabResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabResponseMessage {
    _role: String,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct GitLabStreamResponse {
    choices: Vec<GitLabStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct GitLabStreamChoice {
    delta: GitLabDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabDelta {
    content: Option<String>,
}

// ---- Helpers ----

fn convert_gitlab_response(response: GitLabResponse) -> ChatResponse {
    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();

    let usage = response.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cache_read_input_tokens: None,
        cache_miss_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    ChatResponse {
        id: response
            .id
            .unwrap_or_else(|| format!("gitlab_{}", uuid::Uuid::new_v4())),
        model: response.model.unwrap_or_else(|| "gitlab".to_string()),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: Content::Text(content),
                cache_control: None,
                provider_options: None,
            },
            finish_reason: response
                .choices
                .first()
                .and_then(|c| c.finish_reason.clone()),
        }],
        usage,
    }
}

fn parse_gitlab_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }
    if data == "[DONE]" {
        return Some(StreamEvent::Done);
    }

    let response: GitLabStreamResponse = serde_json::from_str(data).ok()?;
    let choice = response.choices.first()?;

    if let Some(content) = &choice.delta.content {
        if !content.is_empty() {
            return Some(StreamEvent::TextDelta(content.clone()));
        }
    }

    if let Some(reason) = &choice.finish_reason {
        if reason == "tool_calls" {
            return Some(StreamEvent::Done);
        }
    }

    None
}

fn drain_gitlab_sse_events(buffer: &mut String, flush: bool) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    while let Some(newline_idx) = buffer.find('\n') {
        let line = buffer[..newline_idx]
            .trim_end_matches('\r')
            .trim()
            .to_string();
        buffer.drain(..=newline_idx);
        if let Some(data) = line.strip_prefix("data: ") {
            if let Some(event) = parse_gitlab_sse(data) {
                events.push(event);
            }
        }
    }

    if flush {
        let line = buffer.trim();
        if let Some(data) = line.strip_prefix("data: ") {
            if let Some(event) = parse_gitlab_sse(data) {
                events.push(event);
            }
        }
        buffer.clear();
    }

    events
}
