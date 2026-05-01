use async_trait::async_trait;
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::VecDeque;

use crate::runtime::runtime_pipeline_enabled;
use crate::{
    ChatRequest, ChatResponse, Choice, Content, ContentPart, Message, ProviderAdapter,
    ProviderConfig, ProviderError, Role, StreamEvent, StreamResult, ToolUse, Usage,
};

const VERTEX_API_BASE: &str = "https://aiplatform.googleapis.com/v1";

pub struct VertexGeminiAdapter;

impl Default for VertexGeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl VertexGeminiAdapter {
    pub fn new() -> Self {
        Self
    }

    fn extract_config(config: &ProviderConfig) -> Result<VertexExtractedConfig, ProviderError> {
        let access_token = if config.api_key.trim().is_empty() {
            config
                .option_string(&["access_token", "accessToken", "token"])
                .ok_or_else(|| {
                    ProviderError::ConfigError(
                        "vertex requires api_key or access_token option".to_string(),
                    )
                })?
        } else {
            config.api_key.clone()
        };
        let project_id = config
            .option_string(&["project", "project_id", "projectId"])
            .ok_or_else(|| ProviderError::ConfigError("vertex requires project id".to_string()))?;
        let location = config
            .option_string(&["location"])
            .unwrap_or_else(|| "us-east5".to_string());
        let base_url = (!config.base_url.trim().is_empty()).then(|| config.base_url.clone());

        Ok(VertexExtractedConfig {
            access_token,
            project_id,
            location,
            base_url,
        })
    }

    fn build_url(vertex_config: &VertexExtractedConfig, model: &str, method: &str) -> String {
        let base = vertex_config.base_url.as_deref().unwrap_or(VERTEX_API_BASE);
        format!(
            "{}/projects/{}/locations/{}/publishers/google/models/{}:{}",
            base, vertex_config.project_id, vertex_config.location, model, method
        )
    }

    fn convert_request(request: ChatRequest) -> VertexRequest {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for msg in request.messages {
            match msg.role {
                Role::System => {
                    if let Content::Text(text) = msg.content {
                        system_instruction = Some(VertexContent {
                            parts: vec![VertexPart::text(&text)],
                            role: "user".to_string(),
                        });
                    }
                }
                Role::User => {
                    let parts = content_to_parts(&msg.content);
                    contents.push(VertexContent {
                        parts,
                        role: "user".to_string(),
                    });
                }
                Role::Assistant => {
                    let parts = content_to_parts(&msg.content);
                    contents.push(VertexContent {
                        parts,
                        role: "model".to_string(),
                    });
                }
                Role::Tool => {
                    if let Content::Parts(parts) = msg.content {
                        let vertex_parts: Vec<VertexPart> = parts
                            .into_iter()
                            .filter_map(|p| {
                                p.tool_result.map(|tr| {
                                    VertexPart::function_response(&tr.tool_use_id, &tr.content)
                                })
                            })
                            .collect();
                        if !vertex_parts.is_empty() {
                            contents.push(VertexContent {
                                parts: vertex_parts,
                                role: "user".to_string(),
                            });
                        }
                    }
                }
            }
        }

        let generation_config = VertexGenerationConfig {
            max_output_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
        };

        VertexRequest {
            contents,
            system_instruction,
            generation_config: Some(generation_config),
            tools: None,
        }
    }
}

#[async_trait]
impl ProviderAdapter for VertexGeminiAdapter {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let vertex_config = Self::extract_config(config)?;
        let url = Self::build_url(&vertex_config, &request.model, "generateContent");
        let vertex_request = Self::convert_request(request);

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_bearer_auth(
                crate::transport::apply_json_content_type(client.post(&url)),
                &vertex_config.access_token,
            ),
            config,
        );

        let response = req_builder
            .json(&vertex_request)
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

        let vertex_response: VertexResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_vertex_response(vertex_response))
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let use_pipeline = runtime_pipeline_enabled(config);
        let vertex_config = Self::extract_config(config)?;
        let url = Self::build_url(&vertex_config, &request.model, "streamGenerateContent");
        let vertex_request = Self::convert_request(request);

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_sse_accept(crate::transport::apply_bearer_auth(
                crate::transport::apply_json_content_type(client.post(&url)),
                &vertex_config.access_token,
            )),
            config,
        );

        let response = req_builder
            .json(&vertex_request)
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
            let pipeline = crate::runtime::pipeline::Pipeline::vertex_default();
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
                            pending.extend(drain_vertex_sse_events(&mut buffer, false));
                        }
                        Some(Err(e)) => return Err(ProviderError::StreamError(e.to_string())),
                        None => {
                            exhausted = true;
                            pending.extend(drain_vertex_sse_events(&mut buffer, true));
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}

// ---- Internal config ----

struct VertexExtractedConfig {
    access_token: String,
    project_id: String,
    location: String,
    base_url: Option<String>,
}

// ---- Request/Response types ----

#[derive(Debug, Serialize)]
struct VertexRequest {
    contents: Vec<VertexContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<VertexContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<VertexGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<VertexTool>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexContent {
    parts: Vec<VertexPart>,
    role: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<VertexFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<VertexFunctionResponse>,
}

impl VertexPart {
    fn text(t: &str) -> Self {
        Self {
            text: Some(t.to_string()),
            function_call: None,
            function_response: None,
        }
    }

    fn function_call(name: &str, args: serde_json::Value) -> Self {
        Self {
            text: None,
            function_call: Some(VertexFunctionCall {
                name: name.to_string(),
                args,
            }),
            function_response: None,
        }
    }

    fn function_response(name: &str, response: &str) -> Self {
        Self {
            text: None,
            function_call: None,
            function_response: Some(VertexFunctionResponse {
                name: name.to_string(),
                response: serde_json::json!({ "content": response }),
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct VertexGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Debug, Serialize)]
struct VertexTool {
    function_declarations: Vec<VertexFunctionDeclaration>,
}

#[derive(Debug, Serialize)]
struct VertexFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct VertexResponse {
    candidates: Vec<VertexCandidate>,
    #[serde(default)]
    usage_metadata: Option<VertexUsage>,
}

#[derive(Debug, Deserialize)]
struct VertexCandidate {
    content: VertexContent,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default, rename = "safetyRatings")]
    _safety_ratings: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct VertexUsage {
    prompt_token_count: u64,
    candidates_token_count: u64,
    total_token_count: u64,
}

// ---- Helpers ----

fn content_to_parts(content: &Content) -> Vec<VertexPart> {
    match content {
        Content::Text(text) => vec![VertexPart::text(text)],
        Content::Parts(parts) => parts
            .iter()
            .filter_map(|p| {
                if let Some(text) = &p.text {
                    Some(VertexPart::text(text))
                } else {
                    p.tool_use.as_ref().map(|tool_use| {
                        VertexPart::function_call(&tool_use.name, tool_use.input.clone())
                    })
                }
            })
            .collect(),
    }
}

fn convert_vertex_response(response: VertexResponse) -> ChatResponse {
    let candidate = response.candidates.first();

    let content = candidate
        .and_then(|c| c.content.parts.first())
        .map(|p| {
            if let Some(text) = &p.text {
                Content::Text(text.clone())
            } else if let Some(fc) = &p.function_call {
                Content::Parts(vec![ContentPart {
                    content_type: "tool_use".to_string(),
                    text: None,
                    image_url: None,
                    tool_use: Some(ToolUse {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: fc.name.clone(),
                        input: fc.args.clone(),
                    }),
                    tool_result: None,
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                }])
            } else {
                Content::Text(String::new())
            }
        })
        .unwrap_or(Content::Text(String::new()));

    let usage = response.usage_metadata.map(|u| Usage {
        prompt_tokens: u.prompt_token_count,
        completion_tokens: u.candidates_token_count,
        total_tokens: u.total_token_count,
        cache_read_input_tokens: None,
        cache_miss_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    ChatResponse {
        id: format!("vertex_{}", uuid::Uuid::new_v4()),
        model: "google-vertex".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content,
                cache_control: None,
                provider_options: None,
            },
            finish_reason: candidate.and_then(|c| c.finish_reason.clone()),
        }],
        usage,
    }
}

fn parse_vertex_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }
    if data == "[DONE]" {
        return Some(StreamEvent::Done);
    }

    let response: VertexResponse = serde_json::from_str(data).ok()?;

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

fn parse_vertex_sse_line(line: &str) -> Option<StreamEvent> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return None;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        return parse_vertex_sse(data);
    }
    // Vertex may return raw JSON lines without the "data: " prefix.
    parse_vertex_sse(line)
}

fn drain_vertex_sse_events(buffer: &mut String, flush: bool) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    while let Some(newline_idx) = buffer.find('\n') {
        let line = buffer[..newline_idx].trim_end_matches('\r').to_string();
        buffer.drain(..=newline_idx);
        if let Some(event) = parse_vertex_sse_line(&line) {
            events.push(event);
        }
    }

    if flush {
        let line = buffer.trim();
        if let Some(event) = parse_vertex_sse_line(line) {
            events.push(event);
        }
        buffer.clear();
    }

    events
}
