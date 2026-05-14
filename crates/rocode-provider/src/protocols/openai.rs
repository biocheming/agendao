use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tracing;

use super::openai_request_body::{build_request_body, openai_reasoning_effort};
use super::openai_response::{reassemble_sse_chunks, RawChatResponse};
use super::request_sanitizer::{sanitize_messages_for_protocol, SanitizerOptions};
use super::thinking_continuation::{
    request_effectively_enables_thinking, request_explicitly_disables_thinking,
    request_explicitly_enables_thinking,
    request_has_tool_call_continuation_missing_reasoning_replay,
    strip_reasoning_provider_options_for_new_continuation,
};
use crate::custom_fetch::get_custom_fetch_proxy;
use crate::responses::*;
use crate::runtime::runtime_pipeline_enabled;
use crate::tools::InputTool;
use crate::{
    ChatRequest, ChatResponse, Choice, Message, ProviderAdapter, ProviderApiShape, ProviderConfig,
    ProviderError, ProviderProfileResolver, ProviderQuirk, Role, StreamEvent, StreamResult, Usage,
};

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

// ===========================================================================
// Config helpers
// ===========================================================================

fn organization_from_config(config: &ProviderConfig) -> Option<String> {
    config
        .options
        .get("organization")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn provider_api_shape(config: &ProviderConfig) -> Result<ProviderApiShape, ProviderError> {
    ProviderProfileResolver::try_resolve_with_options(&config.provider_id, &config.options)
        .map(|profile| profile.api_shape)
        .map_err(|error| {
            ProviderError::ConfigError(format!(
                "provider `{}` profile resolution failed while selecting api_shape: {error}",
                config.provider_id
            ))
        })
}

fn uses_chat_completions_shape(config: &ProviderConfig) -> Result<bool, ProviderError> {
    Ok(provider_api_shape(config)? == ProviderApiShape::ChatCompletions)
}

fn chat_completions_base_url(config: &ProviderConfig) -> Result<Option<&str>, ProviderError> {
    let base = config.base_url.trim();
    if base.is_empty() {
        if config.provider_id != "openai" {
            return Err(ProviderError::ConfigError(format!(
                "provider `{}` requires `base_url` for closeai-compatible routing",
                config.provider_id
            )));
        }
        Ok(None)
    } else {
        Ok(Some(base))
    }
}

fn provider_requires_thinking_replay(config: &ProviderConfig) -> bool {
    match ProviderProfileResolver::try_resolve_with_options(&config.provider_id, &config.options) {
        Ok(profile) => profile
            .quirks
            .contains(ProviderQuirk::RequiresThinkingReplay),
        Err(error) => {
            tracing::debug!(
                provider_id = %config.provider_id,
                error = %error,
                "failed to resolve provider profile for thinking replay validation"
            );
            false
        }
    }
}

fn provider_defaults_thinking_enabled(config: &ProviderConfig) -> bool {
    config.provider_id.eq_ignore_ascii_case("deepseek")
}

fn start_fresh_non_thinking_continuation_boundary(request: &mut ChatRequest) -> Vec<String> {
    let (provider_options, removed) = strip_reasoning_provider_options_for_new_continuation(
        request.provider_options.take(),
        Some(("thinking", serde_json::json!({"type": "disabled"}))),
    );
    request.provider_options = provider_options;
    removed
}

fn maybe_start_new_continuation_boundary(config: &ProviderConfig, request: &mut ChatRequest) {
    if !provider_requires_thinking_replay(config) {
        return;
    }
    if !provider_defaults_thinking_enabled(config) {
        return;
    }
    if request_explicitly_enables_thinking(request) || request_explicitly_disables_thinking(request)
    {
        return;
    }
    if !request_has_tool_call_continuation_missing_reasoning_replay(&request.messages) {
        return;
    }

    let removed = start_fresh_non_thinking_continuation_boundary(request);
    tracing::warn!(
        provider_id = %config.provider_id,
        removed_keys = ?removed,
        "starting a fresh non-thinking continuation boundary because prior assistant tool-call history lacks required reasoning replay"
    );
}

fn validate_thinking_replay_request(
    config: &ProviderConfig,
    request: &ChatRequest,
) -> Result<(), ProviderError> {
    if !provider_requires_thinking_replay(config) {
        return Ok(());
    }
    if !request_effectively_enables_thinking(request, provider_defaults_thinking_enabled(config)) {
        return Ok(());
    }
    if !request_has_tool_call_continuation_missing_reasoning_replay(&request.messages) {
        return Ok(());
    }

    Err(ProviderError::InvalidRequest(format!(
        "provider `{}` requires assistant reasoning replay in thinking mode for each prior assistant tool-call continuation, but at least one assistant tool-call turn in request history lacks typed reasoning replay; preserve reasoning as typed `reasoning`/`thinking` parts or `providerOptions.openaiCompatible.reasoning_content`, or start a new continuation boundary before switching mode/provider",
        config.provider_id
    )))
}

fn is_missing_reasoning_replay_api_error(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("reasoning_content")
        && lower.contains("thinking mode")
        && lower.contains("passed back")
}

fn map_reasoning_replay_api_error(
    config: &ProviderConfig,
    status: reqwest::StatusCode,
    body: &str,
) -> Option<ProviderError> {
    if !provider_requires_thinking_replay(config) || status.as_u16() != 400 {
        return None;
    }
    if !is_missing_reasoning_replay_api_error(body) {
        return None;
    }

    Some(ProviderError::InvalidRequest(format!(
        "provider `{}` rejected the request because thinking-mode reasoning replay was missing or incompatible: {}; preserve prior assistant reasoning as typed `reasoning`/`thinking` parts or `providerOptions.openaiCompatible.reasoning_content`, or start a new continuation boundary before switching mode/provider",
        config.provider_id,
        body.trim()
    )))
}

// ===========================================================================
// Layer 5 — Request Building & URL
// ===========================================================================

fn chat_completions_url(base_url: Option<&str>) -> String {
    match base_url {
        None => OPENAI_API_URL.to_string(),
        Some(base) => {
            if base.ends_with("/chat/completions") {
                return base.to_string();
            }
            if base.ends_with('/') {
                format!("{base}chat/completions")
            } else {
                format!("{base}/chat/completions")
            }
        }
    }
}

fn responses_url(base_url: Option<&str>, path: &str) -> String {
    let path = path.trim_start_matches('/');
    match base_url {
        None => format!("https://api.openai.com/v1/{}", path),
        Some(base) => {
            if base.ends_with("/chat/completions") {
                return format!("{}/{}", base.trim_end_matches("/chat/completions"), path);
            }
            if base.ends_with("/v1") {
                return format!("{}/{}", base.trim_end_matches('/'), path);
            }
            if base.ends_with('/') {
                format!("{}{}", base, path)
            } else {
                format!("{}/{}", base, path)
            }
        }
    }
}

// ===========================================================================
// Layer 6 — Responses API Helpers
// ===========================================================================

fn extract_responses_provider_options(
    provider_options: Option<&HashMap<String, serde_json::Value>>,
) -> ResponsesProviderOptions {
    let Some(options) = provider_options else {
        return ResponsesProviderOptions::default();
    };

    for key in ["openai", "responses"] {
        if let Some(value) = options.get(key) {
            if let Ok(parsed) = serde_json::from_value::<ResponsesProviderOptions>(value.clone()) {
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
            finish_reason: Some(finish_reason_to_string(result.finish_reason)),
        }],
        usage: Some(usage),
    }
}

fn responses_generate_options(_config: &ProviderConfig, request: &ChatRequest) -> GenerateOptions {
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
        extract_responses_provider_options(request.provider_options.as_ref());
    if provider_options.reasoning_effort.is_none() {
        provider_options.reasoning_effort =
            openai_reasoning_effort(&request.model, request.variant.as_deref())
                .map(ToString::to_string);
    }
    if provider_options.reasoning_summary.is_none() && provider_options.reasoning_effort.is_some() {
        provider_options.reasoning_summary = Some("auto".to_string());
    }

    GenerateOptions {
        prompt,
        tools: tools_to_input_tools(request.tools.as_ref()),
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
    client: &Client,
    config: &ProviderConfig,
    model_id: &str,
) -> OpenAIResponsesLanguageModel {
    let api_key = config.api_key.clone();
    let org = organization_from_config(config);
    let base_url_opt = if config.base_url.is_empty() {
        None
    } else {
        Some(config.base_url.clone())
    };
    let client = client.clone();

    OpenAIResponsesLanguageModel::new(
        model_id.to_string(),
        OpenAIResponsesConfig {
            provider: "openai".to_string(),
            url: Arc::new(move |path, _model| responses_url(base_url_opt.as_deref(), path)),
            headers: Arc::new(move || {
                let mut h = HashMap::new();
                h.insert("Authorization".to_string(), format!("Bearer {}", api_key));
                if let Some(org) = &org {
                    h.insert("OpenAI-Organization".to_string(), org.clone());
                }
                h
            }),
            client: Some(client),
            file_id_prefixes: Some(vec!["file-".to_string()]),
            generate_id: None,
            metadata_extractor: None,
        },
    )
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

// ===========================================================================
// Layer 7a — chat/completions HTTP path
// ===========================================================================

async fn chat_completions_chat(
    client: &Client,
    config: &ProviderConfig,
    mut request: ChatRequest,
) -> Result<ChatResponse, ProviderError> {
    maybe_start_new_continuation_boundary(config, &mut request);
    validate_thinking_replay_request(config, &request)?;
    let base = chat_completions_base_url(config)?;
    let url = chat_completions_url(base);
    let mut request_body = build_request_body(&request)?;

    // Ensure stream is disabled for non-streaming path. The caller may have
    // set stream=true on the ChatRequest (e.g. prompt loop), but this path
    // expects a single JSON response, not SSE chunks.
    if let Value::Object(obj) = &mut request_body {
        obj.remove("stream");
        obj.remove("stream_options");
    }

    let mut req_builder = crate::transport::apply_bearer_auth(
        crate::transport::apply_json_content_type(client.post(&url)),
        &config.api_key,
    );

    if let Some(org) = organization_from_config(config) {
        req_builder = req_builder.header("OpenAI-Organization", org);
    }

    let response = req_builder
        .json(&request_body)
        .send()
        .await
        .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| format!("<body read failed: {}>", error));
        if let Some(mapped) = map_reasoning_replay_api_error(config, status, &body) {
            return Err(mapped);
        }
        return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
    }

    let body = response.text().await.map_err(|e| {
        let mut msg = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(cause) = source {
            msg.push_str(": ");
            msg.push_str(&cause.to_string());
            source = cause.source();
        }
        ProviderError::ApiError(msg)
    })?;

    // Some closeai-compatible providers (e.g. ZhipuAI) return SSE-formatted
    // streaming data even for non-streaming requests. Detect and reassemble.
    let raw: RawChatResponse = if body.trim_start().starts_with("data:") {
        reassemble_sse_chunks(&body)?
    } else {
        serde_json::from_str(&body).map_err(|e| {
            let preview = if body.chars().count() > 500 {
                format!("{}...", body.chars().take(500).collect::<String>())
            } else {
                body.clone()
            };
            ProviderError::ApiError(format!(
                "failed to decode response: {}\nBody: {}",
                e, preview
            ))
        })?
    };
    Ok(raw.into_chat_response())
}

async fn chat_stream_openai_compatible(
    client: &Client,
    config: &ProviderConfig,
    mut request: ChatRequest,
    use_pipeline: bool,
) -> Result<StreamResult, ProviderError> {
    maybe_start_new_continuation_boundary(config, &mut request);
    validate_thinking_replay_request(config, &request)?;
    let base = chat_completions_base_url(config)?;
    let url = chat_completions_url(base);
    request.stream = Some(true);
    let mut request_body = build_request_body(&request)?;

    // Match TS SDK: include stream_options for usage tracking
    if let Value::Object(obj) = &mut request_body {
        obj.insert(
            "stream_options".to_string(),
            serde_json::json!({"include_usage": true}),
        );
    }

    let mut req_builder = crate::transport::apply_sse_accept(crate::transport::apply_bearer_auth(
        crate::transport::apply_json_content_type(client.post(&url)),
        &config.api_key,
    ));

    if let Some(org) = organization_from_config(config) {
        req_builder = req_builder.header("OpenAI-Organization", org);
    }

    let response = req_builder
        .json(&request_body)
        .send()
        .await
        .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| format!("<body read failed: {}>", error));
        if let Some(mapped) = map_reasoning_replay_api_error(config, status, &body) {
            return Err(mapped);
        }
        return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
    }

    if use_pipeline {
        let pipeline = crate::runtime::pipeline::Pipeline::openai_default();
        let pipeline_stream = pipeline.process_stream(Box::pin(response.bytes_stream()));
        return Ok(crate::stream::pipeline_to_stream_result(pipeline_stream));
    }

    let json_stream = crate::stream::decode_sse_stream(response.bytes_stream()).await?;

    let stream = json_stream.flat_map(|result| {
        let events: Vec<Result<StreamEvent, ProviderError>> = match result {
            Ok(value) => crate::stream::parse_openai_value(value)
                .into_iter()
                .map(Ok)
                .collect(),
            Err(e) => vec![Err(e)],
        };
        futures::stream::iter(events)
    });

    Ok(crate::stream::assemble_tool_calls(Box::pin(stream)))
}

async fn chat_completions_stream(
    client: &Client,
    config: &ProviderConfig,
    request: ChatRequest,
) -> Result<StreamResult, ProviderError> {
    chat_stream_openai_compatible(client, config, request, false).await
}

async fn chat_stream_runtime_pipeline(
    client: &Client,
    config: &ProviderConfig,
    request: ChatRequest,
) -> Result<StreamResult, ProviderError> {
    chat_stream_openai_compatible(client, config, request, true).await
}

// ===========================================================================
// CloseAiCompatibleAdapter struct + ProviderAdapter
// ===========================================================================

pub struct CloseAiCompatibleAdapter;

impl Default for CloseAiCompatibleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CloseAiCompatibleAdapter {
    pub fn new() -> Self {
        Self
    }
}

// Phase 3: Full dual routing — Responses API with chat-completions fallback.
#[async_trait]
impl ProviderAdapter for CloseAiCompatibleAdapter {
    async fn chat(
        &self,
        client: &Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        if uses_chat_completions_shape(config)? {
            return chat_completions_chat(client, config, request).await;
        }

        let response_model = responses_model(client, config, &request.model);
        let options = responses_generate_options(config, &request);
        let request_for_primary = request.clone();
        let model_for_log = request.model.clone();
        let client_for_fallback = client.clone();
        let config_for_fallback = config.clone();
        resolve_with_fallback(
            async move {
                response_model
                    .do_generate(options)
                    .await
                    .map(|result| responses_chat_response(&request_for_primary, result))
            },
            move |err| async move {
                if get_custom_fetch_proxy("openai").is_some() {
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Responses generate failed while custom fetch proxy is active; skipping chat-completions fallback"
                    );
                    return Err(err);
                }
                tracing::warn!(
                    model = %model_for_log,
                    error = %err,
                    "Responses generate failed, falling back to chat-completions"
                );
                chat_completions_chat(&client_for_fallback, &config_for_fallback, request).await
            },
        )
        .await
    }

    async fn chat_stream(
        &self,
        client: &Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let use_pipeline = runtime_pipeline_enabled(config);
        if uses_chat_completions_shape(config)? {
            return if use_pipeline {
                chat_stream_runtime_pipeline(client, config, request).await
            } else {
                chat_completions_stream(client, config, request).await
            };
        }

        let response_model = responses_model(client, config, &request.model);
        let options = StreamOptions {
            generate: responses_generate_options(config, &request),
        };
        let model_for_log = request.model.clone();
        let client_for_fallback = client.clone();
        let config_for_fallback = config.clone();
        resolve_with_fallback(
            async move { response_model.do_stream(options).await },
            move |err| async move {
                if get_custom_fetch_proxy("openai").is_some() {
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Responses stream failed while custom fetch proxy is active; skipping chat-completions fallback"
                    );
                    return Err(err);
                }
                tracing::warn!(
                    model = %model_for_log,
                    error = %err,
                    "Responses stream failed, falling back to chat-completions stream"
                );
                if use_pipeline {
                    chat_stream_runtime_pipeline(&client_for_fallback, &config_for_fallback, request)
                        .await
                } else {
                    chat_completions_stream(&client_for_fallback, &config_for_fallback, request)
                        .await
                }
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::super::openai_request_body::to_openai_compatible_chat_messages;
    use super::super::openai_response::{
        RawChatResponse, RawChoice, RawFunction, RawMessage, RawToolCall,
    };
    use super::super::openai_tool_recovery::{
        normalize_tool_call_arguments_for_request, parse_tool_call_input,
    };
    use super::*;
    use crate::custom_fetch::{
        register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
        CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    };
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

    #[tokio::test]
    async fn resolve_with_fallback_returns_primary_when_successful() {
        let result =
            resolve_with_fallback(async { Ok::<_, ProviderError>(7usize) }, |_err| async {
                Ok::<_, ProviderError>(0usize)
            })
            .await
            .expect("primary result should be returned");
        assert_eq!(result, 7);
    }

    #[tokio::test]
    async fn resolve_with_fallback_calls_fallback_on_error() {
        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |_err| async { Ok::<_, ProviderError>(9usize) },
        )
        .await
        .expect("fallback should handle primary error");
        assert_eq!(result, 9);
    }

    #[tokio::test]
    async fn resolve_with_fallback_skips_chat_completions_when_custom_fetch_active() {
        register_custom_fetch_proxy("openai", Arc::new(NoopProxy));

        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |e| async move {
                if get_custom_fetch_proxy("openai").is_some() {
                    return Err(e);
                }
                Ok::<_, ProviderError>(9usize)
            },
        )
        .await;

        unregister_custom_fetch_proxy("openai");
        assert!(result.is_err());
    }

    #[test]
    fn openai_native_provider_defaults_to_chat_completions_shape() {
        let config = ProviderConfig::new("openai", "https://example.com/v1", "test-key")
            .with_option("npm", serde_json::json!("@ai-sdk/openai"));
        assert!(uses_chat_completions_shape(&config).unwrap());
    }

    #[test]
    fn responses_shape_routes_away_from_chat_completions_path() {
        let config = ProviderConfig::new("deepseek", "https://example.com/v1", "test-key")
            .with_option("npm", serde_json::json!("@ai-sdk/openai-compatible"))
            .with_option("useResponsesApi", serde_json::json!(true));
        assert!(!uses_chat_completions_shape(&config).unwrap());
    }

    #[test]
    fn chat_completions_base_url_allows_empty_for_openai_provider() {
        let config = ProviderConfig::new("openai", "   ", "test-key");
        assert!(chat_completions_base_url(&config).unwrap().is_none());
    }

    #[test]
    fn chat_completions_base_url_rejects_empty_for_openai_compatible_provider() {
        let config = ProviderConfig::new("deepseek", "   ", "test-key");
        let err = chat_completions_base_url(&config).unwrap_err();
        assert!(matches!(
            err,
            ProviderError::ConfigError(msg)
                if msg.contains("requires `base_url` for closeai-compatible routing")
        ));
    }

    #[test]
    fn validate_thinking_replay_request_requires_replay_for_deepseek_thinking_continuation() {
        let request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message::assistant_parts(vec![crate::ContentPart::tool_use(
                    "call_1",
                    "bash",
                    json!({ "command": "ls" }),
                )]),
                Message::user("follow up"),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"type": "enabled"}),
            )])),
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        let err = validate_thinking_replay_request(&config, &request).unwrap_err();
        assert!(
            matches!(err, ProviderError::InvalidRequest(message) if message.contains("requires assistant reasoning replay in thinking mode for each prior assistant tool-call continuation"))
        );
    }

    #[test]
    fn validate_thinking_replay_request_allows_replayed_reasoning_parts() {
        let request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message {
                    role: Role::Assistant,
                    content: crate::Content::Parts(vec![
                        crate::ContentPart::reasoning("hidden trace"),
                        crate::ContentPart::text("assistant answer"),
                    ]),
                    cache_control: None,
                    provider_options: None,
                },
                Message::user("follow up"),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"type": "enabled"}),
            )])),
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        assert!(validate_thinking_replay_request(&config, &request).is_ok());
    }

    #[test]
    fn validate_thinking_replay_request_rejects_latest_tool_call_turn_without_replay() {
        let request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message::assistant_parts(vec![
                    crate::ContentPart::reasoning("hidden trace"),
                    crate::ContentPart::tool_use(
                        "call_1",
                        "bash",
                        json!({ "command": "npm install" }),
                    ),
                ]),
                Message::tool_parts(vec![crate::ContentPart::tool_result(
                    "call_1",
                    "ok",
                    Some(false),
                )]),
                Message::assistant_parts(vec![crate::ContentPart::tool_use(
                    "call_2",
                    "bash",
                    json!({ "command": "npx tsc --noEmit" }),
                )]),
                Message::tool_parts(vec![crate::ContentPart::tool_result(
                    "call_2",
                    "build failed",
                    Some(false),
                )]),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"type": "enabled"}),
            )])),
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        let err = validate_thinking_replay_request(&config, &request).unwrap_err();
        assert!(
            matches!(err, ProviderError::InvalidRequest(message) if message.contains("tool-call continuation"))
        );
    }

    #[test]
    fn validate_thinking_replay_request_ignores_non_tool_call_assistant_continuation() {
        let request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message::assistant("assistant without replay"),
                Message::user("follow up"),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"type": "enabled"}),
            )])),
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        assert!(validate_thinking_replay_request(&config, &request).is_ok());
    }

    #[test]
    fn validate_thinking_replay_request_ignores_non_thinking_requests() {
        let request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message::assistant_parts(vec![crate::ContentPart::tool_use(
                    "call_1",
                    "bash",
                    json!({ "command": "ls" }),
                )]),
                Message::user("follow up"),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        let err = validate_thinking_replay_request(&config, &request).unwrap_err();
        assert!(
            matches!(err, ProviderError::InvalidRequest(message) if message.contains("requires assistant reasoning replay in thinking mode"))
        );
    }

    #[test]
    fn maybe_start_new_continuation_boundary_disables_implicit_deepseek_thinking() {
        let mut request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message::assistant_parts(vec![crate::ContentPart::tool_use(
                    "call_1",
                    "bash",
                    json!({ "command": "ls" }),
                )]),
                Message::tool_parts(vec![crate::ContentPart::tool_result(
                    "call_1",
                    "ok",
                    Some(false),
                )]),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        maybe_start_new_continuation_boundary(&config, &mut request);

        assert_eq!(
            request
                .provider_options
                .as_ref()
                .and_then(|options| options.get("thinking")),
            Some(&json!({"type": "disabled"}))
        );
        assert!(validate_thinking_replay_request(&config, &request).is_ok());
    }

    #[test]
    fn maybe_start_new_continuation_boundary_preserves_explicit_thinking_requests() {
        let mut request = ChatRequest {
            model: "deepseek-v4".to_string(),
            messages: vec![
                Message::user("first turn"),
                Message::assistant_parts(vec![crate::ContentPart::tool_use(
                    "call_1",
                    "bash",
                    json!({ "command": "ls" }),
                )]),
                Message::tool_parts(vec![crate::ContentPart::tool_result(
                    "call_1",
                    "ok",
                    Some(false),
                )]),
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: None,
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"type": "enabled"}),
            )])),
            variant: None,
        };

        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));

        maybe_start_new_continuation_boundary(&config, &mut request);

        assert_eq!(
            request
                .provider_options
                .as_ref()
                .and_then(|options| options.get("thinking")),
            Some(&json!({"type": "enabled"}))
        );
        let err = validate_thinking_replay_request(&config, &request).unwrap_err();
        assert!(
            matches!(err, ProviderError::InvalidRequest(message) if message.contains("requires assistant reasoning replay in thinking mode"))
        );
    }

    #[test]
    fn map_reasoning_replay_api_error_rewrites_deepseek_400() {
        let config = ProviderConfig::new("deepseek", "https://api.deepseek.com/v1", "test-key")
            .with_option("npm", json!("@ai-sdk/openai-compatible"));
        let body = r#"{"error":{"message":"The reasoning_content in the thinking mode must be passed back to the API.","type":"invalid_request_error","code":"invalid_request_error"}}"#;

        let err = map_reasoning_replay_api_error(&config, reqwest::StatusCode::BAD_REQUEST, body)
            .expect("error should be rewritten");

        assert!(
            matches!(err, ProviderError::InvalidRequest(message) if message.contains("thinking-mode reasoning replay was missing or incompatible"))
        );
    }

    #[test]
    fn converts_tool_roundtrip_messages_to_openai_compatible_shape() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![
                crate::ContentPart {
                    content_type: "text".to_string(),
                    text: Some("Running command".to_string()),
                    ..Default::default()
                },
                crate::ContentPart {
                    content_type: "tool_use".to_string(),
                    tool_use: Some(crate::ToolUse {
                        id: "call_1".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({ "cmd": "ls" }),
                    }),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        };

        let tool_result = Message {
            role: Role::Tool,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "tool_result".to_string(),
                tool_result: Some(crate::ToolResult {
                    tool_use_id: "call_1".to_string(),
                    content: "ok".to_string(),
                    is_error: Some(false),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant, tool_result]);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[0]["tool_calls"][0]["type"], "function");
        assert_eq!(converted[0]["tool_calls"][0]["function"]["name"], "bash");
        assert_eq!(converted[1]["role"], "tool");
        assert_eq!(converted[1]["tool_call_id"], "call_1");
        assert_eq!(converted[1]["content"], "ok");
    }

    #[test]
    fn routes_unrecoverable_historical_tool_call_to_invalid_and_keeps_tool_message() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![
                crate::ContentPart {
                    content_type: "text".to_string(),
                    text: Some("Attempting tool call".to_string()),
                    ..Default::default()
                },
                crate::ContentPart {
                    content_type: "tool_use".to_string(),
                    tool_use: Some(crate::ToolUse {
                        id: "call_bad".to_string(),
                        name: "write".to_string(),
                        input: Value::String("not-json".to_string()),
                    }),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        };

        let tool_result = Message {
            role: Role::Tool,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "tool_result".to_string(),
                tool_result: Some(crate::ToolResult {
                    tool_use_id: "call_bad".to_string(),
                    content: "ok".to_string(),
                    is_error: Some(false),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant, tool_result]);
        assert_eq!(
            converted.len(),
            2,
            "unrecoverable args should be routed to invalid while keeping tool/result pair"
        );
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[0]["tool_calls"][0]["function"]["name"], "invalid");
        let args = converted[0]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .expect("arguments must be JSON string");
        let parsed_args: Value = serde_json::from_str(args).expect("valid invalid payload");
        assert_eq!(parsed_args["tool"], "write");
        assert_eq!(parsed_args["toolCallId"], "call_bad");
        assert_eq!(parsed_args["receivedArgs"]["type"], "string");
        assert_eq!(converted[1]["role"], "tool");
    }

    #[test]
    fn injects_interrupted_tool_result_when_historical_tool_result_is_missing() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "tool_use".to_string(),
                tool_use: Some(crate::ToolUse {
                    id: "call_missing".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({ "file_path": "t2.html" }),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[1]["role"], "tool");
        assert_eq!(converted[1]["tool_call_id"], "call_missing");
        assert_eq!(converted[1]["content"], "[Tool execution was interrupted]");
    }

    #[test]
    fn injects_interrupted_tool_result_per_assistant_segment_even_with_reused_call_id() {
        let first_assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "tool_use".to_string(),
                tool_use: Some(crate::ToolUse {
                    id: "tool-call-0".to_string(),
                    name: "invalid".to_string(),
                    input: serde_json::json!({ "tool": "skill_manage" }),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let user = Message::user("follow up");

        let second_assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "tool_use".to_string(),
                tool_use: Some(crate::ToolUse {
                    id: "tool-call-0".to_string(),
                    name: "skills_list".to_string(),
                    input: serde_json::json!({}),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let second_tool_result = Message {
            role: Role::Tool,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "tool_result".to_string(),
                tool_result: Some(crate::ToolResult {
                    tool_use_id: "tool-call-0".to_string(),
                    content: "<available_skills />".to_string(),
                    is_error: Some(false),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[
            first_assistant,
            user,
            second_assistant,
            second_tool_result,
        ]);

        assert_eq!(converted.len(), 5);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[1]["role"], "tool");
        assert_eq!(converted[1]["tool_call_id"], "tool-call-0");
        assert_eq!(converted[1]["content"], "[Tool execution was interrupted]");
        assert_eq!(converted[2]["role"], "user");
        assert_eq!(converted[3]["role"], "assistant");
        assert_eq!(converted[4]["role"], "tool");
        assert_eq!(converted[4]["tool_call_id"], "tool-call-0");
        assert_eq!(converted[4]["content"], "<available_skills />");
    }

    #[test]
    fn raw_chat_response_parses_valid_tool_arguments_as_object() {
        let raw = RawChatResponse {
            id: Some("resp_1".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![RawChoice {
                index: Some(0),
                message: Some(RawMessage {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: Some(vec![RawToolCall {
                        id: Some("call_1".to_string()),
                        function: Some(RawFunction {
                            name: Some("write".to_string()),
                            arguments: Some(
                                r#"{"file_path":"t2.html","content":"line1\nline2"}"#.to_string(),
                            ),
                        }),
                    }]),
                    _reasoning_text: None,
                }),
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let chat = raw.into_chat_response();
        let choice = &chat.choices[0];
        let crate::Content::Parts(parts) = &choice.message.content else {
            panic!("expected parts content");
        };
        let input = parts[0]
            .tool_use
            .as_ref()
            .expect("missing tool_use")
            .input
            .clone();
        assert!(input.is_object(), "valid JSON args should remain an object");
        assert_eq!(input["file_path"], "t2.html");
    }

    #[test]
    fn raw_chat_response_maps_deepseek_prompt_cache_hits() {
        let raw: RawChatResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_cache",
            "model": "deepseek-v4-flash",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "ok"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "total_tokens": 1050,
                "prompt_cache_hit_tokens": 900,
                "prompt_cache_miss_tokens": 100
            }
        }))
        .expect("raw chat response should deserialize");

        let chat = raw.into_chat_response();
        let usage = chat.usage.expect("usage should be present");
        assert_eq!(usage.prompt_tokens, 1000);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.cache_read_input_tokens, Some(900));
        assert_eq!(usage.cache_miss_input_tokens, Some(100));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn raw_chat_response_preserves_reasoning_text_as_part() {
        let raw = RawChatResponse {
            id: Some("resp_reasoning".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![RawChoice {
                index: Some(0),
                message: Some(RawMessage {
                    role: Some("assistant".to_string()),
                    content: Some("final answer".to_string()),
                    tool_calls: None,
                    _reasoning_text: Some("thinking trace".to_string()),
                }),
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        };

        let chat = raw.into_chat_response();
        let crate::Content::Parts(parts) = &chat.choices[0].message.content else {
            panic!("expected parts content");
        };
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].content_type, "reasoning");
        assert_eq!(parts[0].text.as_deref(), Some("thinking trace"));
        assert_eq!(parts[1].content_type, "text");
        assert_eq!(parts[1].text.as_deref(), Some("final answer"));
    }

    #[test]
    fn assistant_reasoning_parts_round_trip_to_reasoning_content() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![
                crate::ContentPart {
                    content_type: "reasoning".to_string(),
                    text: Some("internal trace".to_string()),
                    ..Default::default()
                },
                crate::ContentPart {
                    content_type: "text".to_string(),
                    text: Some("final answer".to_string()),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[0]["reasoning_content"], "internal trace");
        assert_eq!(converted[0]["content"], "final answer");
    }

    #[test]
    fn assistant_reasoning_provider_options_round_trip_to_reasoning_content() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "text".to_string(),
                text: Some("final answer".to_string()),
                provider_options: Some(HashMap::from([(
                    "openaiCompatible".to_string(),
                    json!({ "reasoning_content": "wire replay" }),
                )])),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[0]["reasoning_content"], "wire replay");
        assert_eq!(converted[0]["content"], "final answer");
    }

    #[test]
    fn assistant_thinking_parts_round_trip_to_reasoning_content() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![
                crate::ContentPart {
                    content_type: "thinking".to_string(),
                    text: Some("thinking trace".to_string()),
                    ..Default::default()
                },
                crate::ContentPart {
                    content_type: "text".to_string(),
                    text: Some("final answer".to_string()),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["reasoning_content"], "thinking trace");
        assert_eq!(converted[0]["content"], "final answer");
    }

    #[test]
    fn assistant_reasoning_provider_options_override_typed_reasoning_text() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![
                crate::ContentPart {
                    content_type: "reasoning".to_string(),
                    text: Some("typed reasoning".to_string()),
                    ..Default::default()
                },
                crate::ContentPart {
                    content_type: "text".to_string(),
                    text: Some("final answer".to_string()),
                    provider_options: Some(HashMap::from([(
                        "openaiCompatible".to_string(),
                        json!({ "reasoning_content": "wire replay" }),
                    )])),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["reasoning_content"], "wire replay");
        assert_eq!(converted[0]["content"], "final answer");
    }

    #[test]
    fn assistant_reasoning_provider_options_round_trip_reasoning_details() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![crate::ContentPart {
                content_type: "text".to_string(),
                text: Some("final answer".to_string()),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: Some(HashMap::from([(
                "openaiCompatible".to_string(),
                json!({
                    "reasoning_details": [
                        { "type": "summary", "text": "step one" }
                    ]
                }),
            )])),
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 1);
        assert_eq!(
            converted[0]["reasoning_details"],
            json!([{ "type": "summary", "text": "step one" }])
        );
        assert_eq!(converted[0]["content"], "final answer");
    }

    #[test]
    fn assistant_reasoning_survives_alongside_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: crate::Content::Parts(vec![
                crate::ContentPart {
                    content_type: "reasoning".to_string(),
                    text: Some("need to inspect a file first".to_string()),
                    ..Default::default()
                },
                crate::ContentPart {
                    content_type: "tool_use".to_string(),
                    tool_use: Some(crate::ToolUse {
                        id: "call_reasoning".to_string(),
                        name: "read".to_string(),
                        input: serde_json::json!({ "file_path": "README.md" }),
                    }),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        };

        let converted = to_openai_compatible_chat_messages(&[assistant]);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(
            converted[0]["reasoning_content"],
            "need to inspect a file first"
        );
        assert_eq!(converted[0]["tool_calls"][0]["function"]["name"], "read");
        assert_eq!(converted[1]["role"], "tool");
        assert_eq!(converted[1]["content"], "[Tool execution was interrupted]");
    }

    #[test]
    fn responses_generate_options_defaults_reasoning_summary_to_auto() {
        let request = ChatRequest {
            model: "gpt-5".to_string(),
            variant: Some("medium".to_string()),
            messages: vec![Message::user("hello".to_string())],
            system: None,
            tools: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stream: None,
            provider_options: None,
        };

        let options = responses_generate_options(&ProviderConfig::new("test", "", ""), &request);
        let provider_options = options.provider_options.expect("provider options");
        assert_eq!(provider_options.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(provider_options.reasoning_summary.as_deref(), Some("auto"));
    }

    #[test]
    fn responses_generate_options_preserve_tool_continuity_stub() {
        let request = ChatRequest {
            model: "gpt-5".to_string(),
            messages: vec![Message::assistant_parts(vec![
                crate::ContentPart::tool_use("call-1", "ls", json!({ "path": "." })),
            ])],
            system: None,
            tools: None,
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            stream: None,
            provider_options: None,
            variant: None,
        };

        let options = responses_generate_options(&ProviderConfig::new("openai", "", ""), &request);
        assert_eq!(options.prompt.len(), 2);
        assert!(matches!(options.prompt[0].role, Role::Assistant));
        assert!(matches!(options.prompt[1].role, Role::Tool));
    }

    #[test]
    fn raw_chat_response_recovers_truncated_write_arguments_into_object() {
        let truncated_json = "{\"file_path\":\"t2.html\",\"content\":\"line1";
        let raw = RawChatResponse {
            id: Some("resp_2".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![RawChoice {
                index: Some(0),
                message: Some(RawMessage {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: Some(vec![RawToolCall {
                        id: Some("call_1".to_string()),
                        function: Some(RawFunction {
                            name: Some("write".to_string()),
                            arguments: Some(truncated_json.to_string()),
                        }),
                    }]),
                    _reasoning_text: None,
                }),
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let chat = raw.into_chat_response();
        let choice = &chat.choices[0];
        let crate::Content::Parts(parts) = &choice.message.content else {
            panic!("expected parts content");
        };
        let input = parts[0]
            .tool_use
            .as_ref()
            .expect("missing tool_use")
            .input
            .clone();
        assert!(
            input.is_object(),
            "truncated write arguments should be recovered into object"
        );
        assert_eq!(input["file_path"], "t2.html");
        assert_eq!(input["content"], "line1");
    }

    #[test]
    fn raw_chat_response_recovers_truncated_unknown_tool_arguments() {
        // Truncated JSON like {"foo":"bar is now recoverable by the robust
        // repair pipeline, so we expect it to be parsed into an object.
        let truncated_json = "{\"foo\":\"bar";
        let raw = RawChatResponse {
            id: Some("resp_2b".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![RawChoice {
                index: Some(0),
                message: Some(RawMessage {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: Some(vec![RawToolCall {
                        id: Some("call_1".to_string()),
                        function: Some(RawFunction {
                            name: Some("unknown_tool".to_string()),
                            arguments: Some(truncated_json.to_string()),
                        }),
                    }]),
                    _reasoning_text: None,
                }),
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let chat = raw.into_chat_response();
        let choice = &chat.choices[0];
        let crate::Content::Parts(parts) = &choice.message.content else {
            panic!("expected parts content");
        };
        let input = parts[0]
            .tool_use
            .as_ref()
            .expect("missing tool_use")
            .input
            .clone();
        assert!(
            input.is_object(),
            "truncated JSON should be recovered into an object"
        );
        assert_eq!(input["foo"], "bar");
    }

    #[test]
    fn raw_chat_response_recovers_literal_control_characters_into_object() {
        let raw = RawChatResponse {
            id: Some("resp_3".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![RawChoice {
                index: Some(0),
                message: Some(RawMessage {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: Some(vec![RawToolCall {
                        id: Some("call_1".to_string()),
                        function: Some(RawFunction {
                            name: Some("write".to_string()),
                            arguments: Some(
                                "{\"file_path\":\"t2.html\",\"content\":\"line1\nline2\"}"
                                    .to_string(),
                            ),
                        }),
                    }]),
                    _reasoning_text: None,
                }),
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let chat = raw.into_chat_response();
        let choice = &chat.choices[0];
        let crate::Content::Parts(parts) = &choice.message.content else {
            panic!("expected parts content");
        };
        let input = parts[0]
            .tool_use
            .as_ref()
            .expect("missing tool_use")
            .input
            .clone();
        assert!(
            input.is_object(),
            "literal control characters should be recovered into JSON object"
        );
        assert_eq!(input["file_path"], "t2.html");
    }

    #[test]
    fn normalize_tool_call_arguments_recovers_json_object_from_raw_string() {
        let input = Value::String("{\"file_path\":\"t2.html\",\"content\":\"ok\"}".to_string());
        let normalized = normalize_tool_call_arguments_for_request("write", "call_1", &input);
        let parsed: Value =
            serde_json::from_str(&normalized.arguments).expect("normalized must be valid JSON");
        assert_eq!(normalized.tool_name, "write");
        assert!(
            parsed.is_object(),
            "normalized args should be a JSON object"
        );
        assert_eq!(parsed["file_path"], "t2.html");
    }

    #[test]
    fn normalize_tool_call_arguments_routes_unrecoverable_non_json_string_to_invalid() {
        let input = Value::String("not-json".to_string());
        let normalized = normalize_tool_call_arguments_for_request("write", "call_1", &input);
        let parsed: Value =
            serde_json::from_str(&normalized.arguments).expect("normalized must be valid JSON");
        assert_eq!(normalized.tool_name, "invalid");
        assert_eq!(parsed["tool"], "write");
        assert_eq!(parsed["toolCallId"], "call_1");
        assert_eq!(parsed["receivedArgs"]["type"], "string");
        assert!(parsed["error"]
            .as_str()
            .unwrap_or_default()
            .contains("malformed/truncated"));
    }

    #[test]
    fn normalize_tool_call_arguments_routes_legacy_sentinel_object_to_invalid() {
        let input = json!({
            "_rocode_unrecoverable_tool_args": true,
            "tool": "write",
            "raw_len": 128,
            "raw_preview": "{\"content\":\"<html>"
        });
        let normalized = normalize_tool_call_arguments_for_request("write", "call_legacy", &input);
        assert_eq!(normalized.tool_name, "invalid");
        let parsed: Value =
            serde_json::from_str(&normalized.arguments).expect("normalized must be valid JSON");
        assert_eq!(parsed["tool"], "write");
        assert_eq!(parsed["toolCallId"], "call_legacy");
        assert_eq!(
            parsed["receivedArgs"]["source"],
            "legacy-unrecoverable-sentinel"
        );
    }

    #[test]
    fn parse_tool_call_input_recovers_truncated_write_jsonish_payload() {
        let truncated = "{\"file_path\":\"t2.html\",\"content\":\"<html><body>hello";
        let parsed = parse_tool_call_input("write", truncated);
        assert!(
            parsed.is_object(),
            "truncated write payload should be recovered"
        );
        assert_eq!(parsed["file_path"], "t2.html");
        assert_eq!(parsed["content"], "<html><body>hello");
    }
}
