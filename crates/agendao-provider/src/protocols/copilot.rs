use async_trait::async_trait;
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use tracing;

use super::request_sanitizer::{
    content_visible_text_lossy, sanitize_messages_for_protocol,
    sanitize_messages_for_text_protocol, SanitizerOptions,
};
use crate::bootstrap::should_use_copilot_responses_api;
use crate::custom_fetch::get_custom_fetch_proxy;
use crate::responses::{
    FinishReason, GenerateOptions, OpenAIResponsesConfig, OpenAIResponsesLanguageModel,
    ResponsesProviderOptions, StreamOptions,
};
use crate::runtime::runtime_pipeline_enabled;
use crate::tools::InputTool;
use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ProviderAdapter, ProviderConfig,
    ProviderError, Role, StreamEvent, StreamResult, Usage,
};

const COPILOT_API_URL: &str = "https://api.githubcopilot.com/chat/completions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopilotRoute {
    Responses,
    ChatCompletions,
}

fn select_copilot_route(model_id: &str) -> CopilotRoute {
    if should_use_copilot_responses_api(model_id) {
        CopilotRoute::Responses
    } else {
        CopilotRoute::ChatCompletions
    }
}

async fn resolve_with_fallback<T, PFut, FFut, F>(
    primary: PFut,
    fallback: F,
) -> Result<T, ProviderError>
where
    PFut: Future<Output = Result<T, ProviderError>>,
    F: FnOnce(ProviderError) -> FFut,
    FFut: Future<Output = Result<T, ProviderError>>,
{
    match primary.await {
        Ok(value) => Ok(value),
        Err(err) => fallback(err).await,
    }
}

pub struct GitHubCopilotCloseAiAdapter;

impl Default for GitHubCopilotCloseAiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubCopilotCloseAiAdapter {
    pub fn new() -> Self {
        Self
    }

    fn get_api_url(config: &ProviderConfig) -> &str {
        if config.base_url.trim().is_empty() {
            COPILOT_API_URL
        } else {
            &config.base_url
        }
    }

    fn convert_request(request: ChatRequest) -> CopilotRequest {
        let messages: Vec<CopilotMessage> = sanitize_messages_for_text_protocol(&request.messages)
            .into_iter()
            .map(|msg| CopilotMessage {
                role: match msg.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::Tool => "user".to_string(),
                },
                content: match msg.content {
                    Content::Text(t) => CopilotContent::Text(t),
                    Content::Parts(parts) => {
                        let visible_text = content_visible_text_lossy(&Content::Parts(parts));
                        let contents: Vec<CopilotContentPart> = if visible_text.is_empty() {
                            Vec::new()
                        } else {
                            vec![CopilotContentPart {
                                content_type: "text".to_string(),
                                text: Some(visible_text),
                                image_url: None,
                            }]
                        };
                        CopilotContent::Parts(contents)
                    }
                },
            })
            .collect();

        CopilotRequest {
            model: request.model,
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            stream: false,
        }
    }

    fn responses_url(base_url: Option<&str>, path: &str) -> String {
        let path = path.trim_start_matches('/');
        match base_url {
            None => format!("https://api.githubcopilot.com/{}", path),
            Some(base) => {
                if base.ends_with("/chat/completions") {
                    return format!("{}/{}", base.trim_end_matches("/chat/completions"), path);
                }
                if base.ends_with('/') {
                    format!("{}{}", base, path)
                } else {
                    format!("{}/{}", base, path)
                }
            }
        }
    }

    fn extract_responses_provider_options(
        provider_options: Option<&HashMap<String, serde_json::Value>>,
    ) -> ResponsesProviderOptions {
        let Some(options) = provider_options else {
            return ResponsesProviderOptions::default();
        };

        for key in ["github-copilot", "openai", "responses"] {
            if let Some(value) = options.get(key) {
                if let Ok(parsed) =
                    serde_json::from_value::<ResponsesProviderOptions>(value.clone())
                {
                    return parsed;
                }
            }
        }

        serde_json::from_value::<ResponsesProviderOptions>(serde_json::json!(options))
            .unwrap_or_default()
    }

    fn tools_to_input_tools(tools: Option<&Vec<crate::ToolDefinition>>) -> Option<Vec<InputTool>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| InputTool::Function {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.parameters.clone(),
                })
                .collect(),
        )
    }

    fn responses_generate_options(
        request: &ChatRequest,
        _config: &ProviderConfig,
    ) -> GenerateOptions {
        let mut prompt = sanitize_messages_for_protocol(
            &request.messages,
            SanitizerOptions {
                drop_thinking_only_assistant: false,
                ..Default::default()
            },
        );
        if let Some(system) = &request.system {
            let has_system = prompt.iter().any(|m| matches!(m.role, Role::System));
            if !has_system {
                prompt.insert(0, Message::system(system.clone()));
            }
        }

        let mut provider_options =
            Self::extract_responses_provider_options(request.provider_options.as_ref());
        if provider_options.reasoning_effort.is_none() {
            provider_options.reasoning_effort =
                copilot_reasoning_effort(&request.model, request.variant.as_deref())
                    .map(ToString::to_string);
        }

        GenerateOptions {
            prompt,
            tools: Self::tools_to_input_tools(request.tools.as_ref()),
            tool_choice: None,
            max_output_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            top_k: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: None,
            provider_options: Some(provider_options),
            response_format: None,
        }
    }

    fn responses_model(
        model_id: &str,
        config: &ProviderConfig,
        client: &reqwest::Client,
    ) -> OpenAIResponsesLanguageModel {
        let token = config.api_key.clone();
        let base_url_opt = (!config.base_url.trim().is_empty()).then(|| config.base_url.clone());
        let client = client.clone();

        OpenAIResponsesLanguageModel::new(
            model_id.to_string(),
            OpenAIResponsesConfig {
                provider: "github-copilot".to_string(),
                url: Arc::new(move |path, _model| {
                    Self::responses_url(base_url_opt.as_deref(), path)
                }),
                headers: Arc::new(move || {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), format!("Bearer {}", token));
                    h.insert(
                        "Copilot-Integration-Id".to_string(),
                        "vscode-chat".to_string(),
                    );
                    h
                }),
                client: Some(client),
                file_id_prefixes: Some(vec!["file-".to_string()]),
                generate_id: None,
                metadata_extractor: None,
            },
        )
    }

    fn finish_reason_to_string(reason: FinishReason) -> String {
        match reason {
            FinishReason::Stop => "stop".to_string(),
            FinishReason::Length => "length".to_string(),
            FinishReason::ContentFilter => "content_filter".to_string(),
            FinishReason::ToolCalls => "tool-calls".to_string(),
            FinishReason::Error => "error".to_string(),
            FinishReason::Unknown => "unknown".to_string(),
        }
    }

    fn responses_chat_response(
        request: &ChatRequest,
        result: crate::responses::ResponsesGenerateResult,
    ) -> ChatResponse {
        let usage = Usage {
            prompt_tokens: result.usage.input_tokens,
            completion_tokens: result.usage.output_tokens,
            total_tokens: result.usage.input_tokens + result.usage.output_tokens,
            cache_read_input_tokens: result
                .usage
                .input_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            cache_miss_input_tokens: None,
            cache_creation_input_tokens: None,
        };

        ChatResponse {
            id: result
                .metadata
                .response_id
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            model: result
                .metadata
                .model_id
                .unwrap_or_else(|| request.model.clone()),
            choices: vec![Choice {
                index: 0,
                message: result.message,
                finish_reason: Some(Self::finish_reason_to_string(result.finish_reason)),
            }],
            usage: Some(usage),
        }
    }

    async fn chat_completions_chat(
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let url = Self::get_api_url(config);
        let copilot_request = Self::convert_request(request);

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_copilot_headers(
                crate::transport::apply_json_content_type(client.post(url)),
                config,
            ),
            config,
        );

        let response = req_builder
            .json(&copilot_request)
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

        let copilot_response: CopilotResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_copilot_response(copilot_response))
    }

    async fn chat_completions_stream(
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
        use_pipeline: bool,
    ) -> Result<StreamResult, ProviderError> {
        let url = Self::get_api_url(config).to_string();
        let mut copilot_request = Self::convert_request(request);
        copilot_request.stream = true;

        let req_builder = crate::transport::apply_config_headers(
            crate::transport::apply_sse_accept(crate::transport::apply_copilot_headers(
                crate::transport::apply_json_content_type(client.post(&url)),
                config,
            )),
            config,
        );

        let response = req_builder
            .json(&copilot_request)
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
                            pending.extend(drain_copilot_sse_events(&mut buffer, false));
                        }
                        Some(Err(e)) => return Err(ProviderError::StreamError(e.to_string())),
                        None => {
                            exhausted = true;
                            pending.extend(drain_copilot_sse_events(&mut buffer, true));
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl ProviderAdapter for GitHubCopilotCloseAiAdapter {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        if select_copilot_route(&request.model) == CopilotRoute::Responses {
            let response_model = Self::responses_model(&request.model, config, client);
            let options = Self::responses_generate_options(&request, config);
            let request_for_primary = request.clone();
            let model_for_log = request.model.clone();
            let client_for_fallback = client.clone();
            let config_for_fallback = config.clone();
            return resolve_with_fallback(
                async move {
                    response_model
                        .do_generate(options)
                        .await
                        .map(|result| Self::responses_chat_response(&request_for_primary, result))
                },
                move |err| async move {
                    if get_custom_fetch_proxy("github-copilot").is_some() {
                        tracing::warn!(
                            model = %model_for_log,
                            error = %err,
                            "Copilot responses generate failed while custom fetch proxy is active; skipping chat-completions fallback"
                        );
                        return Err(err);
                    }
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Copilot responses generate failed, falling back to chat completions"
                    );
                    Self::chat_completions_chat(
                        &client_for_fallback,
                        &config_for_fallback,
                        request,
                    )
                    .await
                },
            )
            .await;
        }

        Self::chat_completions_chat(client, config, request).await
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let use_pipeline = runtime_pipeline_enabled(config);
        if select_copilot_route(&request.model) == CopilotRoute::Responses {
            let response_model = Self::responses_model(&request.model, config, client);
            let options = StreamOptions {
                generate: Self::responses_generate_options(&request, config),
            };
            let model_for_log = request.model.clone();
            let client_for_fallback = client.clone();
            let config_for_fallback = config.clone();
            return resolve_with_fallback(
                async move { response_model.do_stream(options).await },
                move |err| async move {
                    if get_custom_fetch_proxy("github-copilot").is_some() {
                        tracing::warn!(
                            model = %model_for_log,
                            error = %err,
                            "Copilot responses stream failed while custom fetch proxy is active; skipping chat-completions fallback"
                        );
                        return Err(err);
                    }
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Copilot responses stream failed, falling back to chat completions"
                    );
                    Self::chat_completions_stream(
                        &client_for_fallback,
                        &config_for_fallback,
                        request,
                        use_pipeline,
                    )
                    .await
                },
            )
            .await;
        }

        Self::chat_completions_stream(client, config, request, use_pipeline).await
    }
}

// ---- Helpers ----

fn copilot_reasoning_effort(model_id: &str, variant: Option<&str>) -> Option<&'static str> {
    let variant = variant?.trim().to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();
    let supports_effort = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("gpt-5");
    if !supports_effort {
        return None;
    }

    match variant.as_str() {
        "none" => Some("none"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "max" | "xhigh" => Some("high"),
        _ => None,
    }
}

// ---- Request/Response types ----

#[derive(Debug, Serialize)]
struct CopilotRequest {
    model: String,
    messages: Vec<CopilotMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct CopilotMessage {
    role: String,
    content: CopilotContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum CopilotContent {
    Text(String),
    Parts(Vec<CopilotContentPart>),
}

#[derive(Debug, Serialize)]
struct CopilotContentPart {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<CopilotImageUrl>,
}

#[derive(Debug, Serialize)]
struct CopilotImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct CopilotResponse {
    id: String,
    model: String,
    choices: Vec<CopilotChoice>,
    usage: Option<CopilotUsage>,
}

#[derive(Debug, Deserialize)]
struct CopilotChoice {
    _index: u32,
    message: CopilotResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotResponseMessage {
    _role: String,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct CopilotStreamResponse {
    choices: Vec<CopilotStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct CopilotStreamChoice {
    delta: CopilotDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotDelta {
    content: Option<String>,
}

fn convert_copilot_response(response: CopilotResponse) -> ChatResponse {
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
        id: response.id,
        model: response.model,
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

fn parse_copilot_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }
    if data == "[DONE]" {
        return Some(StreamEvent::Done);
    }

    let response: CopilotStreamResponse = serde_json::from_str(data).ok()?;

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

fn drain_copilot_sse_events(buffer: &mut String, flush: bool) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    while let Some(newline_idx) = buffer.find('\n') {
        let line = buffer[..newline_idx]
            .trim_end_matches('\r')
            .trim()
            .to_string();
        buffer.drain(..=newline_idx);
        if let Some(data) = line.strip_prefix("data: ") {
            if let Some(event) = parse_copilot_sse(data) {
                events.push(event);
            }
        }
    }

    if flush {
        let line = buffer.trim();
        if let Some(data) = line.strip_prefix("data: ") {
            if let Some(event) = parse_copilot_sse(data) {
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
    use crate::custom_fetch::{
        register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
        CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    };
    use crate::ContentPart;
    use async_trait::async_trait;
    use futures::stream;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct NoopProxy;

    #[async_trait]
    impl CustomFetchProxy for NoopProxy {
        async fn fetch(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchResponse, ProviderError> {
            Ok(CustomFetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: String::new(),
            })
        }

        async fn fetch_stream(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchStreamResponse, ProviderError> {
            Ok(CustomFetchStreamResponse {
                status: 200,
                headers: HashMap::new(),
                stream: Box::pin(stream::empty()),
            })
        }
    }

    #[test]
    fn select_route_uses_responses_for_gpt5_models() {
        assert_eq!(select_copilot_route("gpt-5"), CopilotRoute::Responses);
        assert_eq!(select_copilot_route("gpt-5-codex"), CopilotRoute::Responses);
    }

    #[test]
    fn convert_request_drops_tool_only_history_from_text_protocol() {
        let request = ChatRequest {
            model: "copilot-test".to_string(),
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

        let converted = GitHubCopilotCloseAiAdapter::convert_request(request);
        assert_eq!(converted.messages.len(), 1);
        assert_eq!(converted.messages[0].role, "user");
        match &converted.messages[0].content {
            CopilotContent::Text(text) => assert_eq!(text, "before\n\nafter"),
            CopilotContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].text.as_deref(), Some("before\n\nafter"));
            }
        }
    }

    #[test]
    fn responses_generate_options_preserve_tool_continuity_stub() {
        let request = ChatRequest {
            model: "gpt-5".to_string(),
            messages: vec![Message::assistant_parts(vec![ContentPart::tool_use(
                "call-1",
                "ls",
                json!({ "path": "." }),
            )])],
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let options = GitHubCopilotCloseAiAdapter::responses_generate_options(
            &request,
            &ProviderConfig::new("github-copilot", "", "test-key"),
        );

        assert_eq!(options.prompt.len(), 2);
        assert!(matches!(options.prompt[0].role, Role::Assistant));
        assert!(matches!(options.prompt[1].role, Role::Tool));
    }

    #[test]
    fn select_route_keeps_chat_completions_for_non_gpt5_models() {
        assert_eq!(
            select_copilot_route("gpt-4o"),
            CopilotRoute::ChatCompletions
        );
        assert_eq!(
            select_copilot_route("test-model-v2"),
            CopilotRoute::ChatCompletions
        );
    }

    #[tokio::test]
    async fn resolve_with_fallback_calls_fallback_on_primary_error() {
        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |_err| async { Ok::<_, ProviderError>(42usize) },
        )
        .await
        .expect("fallback should handle responses error");
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn resolve_with_fallback_skips_chat_completions_when_custom_fetch_active() {
        register_custom_fetch_proxy("github-copilot", Arc::new(NoopProxy));

        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |e| async move {
                if get_custom_fetch_proxy("github-copilot").is_some() {
                    return Err(e);
                }
                Ok::<_, ProviderError>(42usize)
            },
        )
        .await;

        unregister_custom_fetch_proxy("github-copilot");
        assert!(result.is_err());
    }
}
