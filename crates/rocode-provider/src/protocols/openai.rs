use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing;

use crate::custom_fetch::get_custom_fetch_proxy;
use crate::responses::*;
use crate::tools::InputTool;
use crate::{
    ChatRequest, ChatResponse, Choice, Message, ProtocolImpl, ProviderConfig, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
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

fn is_legacy_only(config: &ProviderConfig) -> bool {
    config
        .options
        .get("legacy_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

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

fn legacy_base_url(config: &ProviderConfig) -> Result<Option<&str>, ProviderError> {
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

// ===========================================================================
// Layer 1 — Wire Types
// ===========================================================================

#[derive(Debug, Deserialize)]
struct RawChatResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<RawChoice>,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
struct RawChoice {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    message: Option<RawMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<RawToolCall>>,
    #[serde(default, rename = "reasoning_text")]
    _reasoning_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<RawFunction>,
}

#[derive(Debug, Deserialize)]
struct RawFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

impl RawChatResponse {
    /// Convert the lenient wire format into our internal `ChatResponse`.
    fn into_chat_response(self) -> ChatResponse {
        let choices = self
            .choices
            .into_iter()
            .map(|c| {
                let raw_msg = c.message.unwrap_or(RawMessage {
                    role: None,
                    content: None,
                    tool_calls: None,
                    _reasoning_text: None,
                });

                // Build content parts from the raw message.
                let mut parts: Vec<crate::ContentPart> = Vec::new();

                // Text content
                if let Some(text) = &raw_msg.content {
                    if !text.is_empty() {
                        parts.push(crate::ContentPart {
                            content_type: "text".to_string(),
                            text: Some(text.clone()),
                            ..Default::default()
                        });
                    }
                }

                if let Some(reasoning) = &raw_msg._reasoning_text {
                    if !reasoning.is_empty() {
                        parts.insert(
                            0,
                            crate::ContentPart {
                                content_type: "reasoning".to_string(),
                                text: Some(reasoning.clone()),
                                ..Default::default()
                            },
                        );
                    }
                }

                // Tool calls → ContentPart with tool_use
                if let Some(tool_calls) = &raw_msg.tool_calls {
                    for tc in tool_calls {
                        let func = tc.function.as_ref();
                        let name = func.and_then(|f| f.name.as_deref()).unwrap_or("");
                        let args_str = func.and_then(|f| f.arguments.as_deref()).unwrap_or("{}");
                        let input = parse_tool_call_input(name, args_str);
                        let id = tc
                            .id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        parts.push(crate::ContentPart {
                            content_type: "tool_use".to_string(),
                            tool_use: Some(crate::ToolUse {
                                id,
                                name: name.to_string(),
                                input,
                            }),
                            ..Default::default()
                        });
                    }
                }

                let content = if parts.is_empty() {
                    crate::Content::Text(raw_msg.content.unwrap_or_default())
                } else if parts.len() == 1 && parts[0].content_type == "text" {
                    crate::Content::Text(parts.remove(0).text.unwrap_or_default())
                } else {
                    crate::Content::Parts(parts)
                };

                Choice {
                    index: c.index.unwrap_or(0),
                    message: Message {
                        role: match raw_msg.role.as_deref() {
                            Some("assistant") | None => Role::Assistant,
                            Some("system") => Role::System,
                            Some("user") => Role::User,
                            Some("tool") => Role::Tool,
                            _ => Role::Assistant,
                        },
                        content,
                        cache_control: None,
                        provider_options: None,
                    },
                    finish_reason: c.finish_reason,
                }
            })
            .collect();

        let usage = self.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
            cache_read_input_tokens: u.cache_read_input_tokens,
            cache_creation_input_tokens: u.cache_creation_input_tokens,
        });

        ChatResponse {
            id: self.id.unwrap_or_default(),
            model: self.model.unwrap_or_default(),
            choices,
            usage,
        }
    }
}

// ===========================================================================
// Layer 2 — Tool Call Recovery
// ===========================================================================

fn parse_tool_call_input(tool_name: &str, args_str: &str) -> Value {
    let strict = serde_json::from_str::<Value>(args_str);
    if let Ok(parsed @ Value::Object(_)) = &strict {
        return parsed.clone();
    }

    if let Some(parsed_object) = rocode_util::json::try_parse_json_object_robust(args_str) {
        increment_tool_args_recovered(tool_name, "parse", args_str.len());
        return parsed_object;
    }

    if let Some(recovered) =
        rocode_util::json::recover_tool_arguments_from_jsonish(tool_name, args_str)
    {
        tracing::info!(
            tool = tool_name,
            args_len = args_str.len(),
            "recovered malformed tool call arguments from JSON-ish payload"
        );
        increment_tool_args_recovered(tool_name, "parse", args_str.len());
        return recovered;
    }

    match strict {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                error = %error,
                args_len = args_str.len(),
                "failed to parse OpenAI tool call arguments as JSON, preserving raw string"
            );
            Value::String(args_str.to_string())
        }
    }
}

#[derive(Debug, Clone)]
struct NormalizedHistoricalToolCall {
    tool_name: String,
    arguments: String,
}

fn invalid_tool_payload_for_history(
    tool_name: &str,
    tool_call_id: &str,
    error: &str,
    received_args: Value,
) -> Value {
    json!({
        "tool": tool_name,
        "toolCallId": tool_call_id,
        "error": error,
        "receivedArgs": received_args,
    })
}

fn normalize_tool_call_arguments_for_request(
    tool_name: &str,
    tool_call_id: &str,
    input: &Value,
) -> NormalizedHistoricalToolCall {
    match input {
        Value::Object(obj) => {
            let is_legacy_unrecoverable = obj
                .get("_rocode_unrecoverable_tool_args")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if !is_legacy_unrecoverable {
                return NormalizedHistoricalToolCall {
                    tool_name: tool_name.to_string(),
                    arguments: input.to_string(),
                };
            }

            let received_args = json!({
                "type": "object",
                "source": "legacy-unrecoverable-sentinel",
                "raw_len": obj.get("raw_len").and_then(Value::as_u64),
                "preview": obj.get("raw_preview").and_then(Value::as_str),
            });
            let payload = invalid_tool_payload_for_history(
                tool_name,
                tool_call_id,
                "Historical tool arguments were previously marked unrecoverable.",
                received_args,
            );
            tracing::debug!(
                tool = tool_name,
                tool_call_id = tool_call_id,
                "routing legacy unrecoverable historical tool_call input to invalid tool"
            );
            increment_tool_args_invalid(tool_name, "history", 0);
            NormalizedHistoricalToolCall {
                tool_name: "invalid".to_string(),
                arguments: payload.to_string(),
            }
        }
        Value::String(raw) => {
            if let Some(parsed_object) = rocode_util::json::try_parse_json_object_robust(raw) {
                increment_tool_args_recovered(tool_name, "history", raw.len());
                return NormalizedHistoricalToolCall {
                    tool_name: tool_name.to_string(),
                    arguments: parsed_object.to_string(),
                };
            }
            if let Some(recovered) =
                rocode_util::json::recover_tool_arguments_from_jsonish(tool_name, raw)
            {
                tracing::info!(
                    tool = tool_name,
                    tool_call_id = tool_call_id,
                    raw_len = raw.len(),
                    "recovered historical tool call input from JSON-ish payload"
                );
                increment_tool_args_recovered(tool_name, "history", raw.len());
                return NormalizedHistoricalToolCall {
                    tool_name: tool_name.to_string(),
                    arguments: recovered.to_string(),
                };
            }
            let payload = invalid_tool_payload_for_history(
                tool_name,
                tool_call_id,
                "Historical tool arguments are malformed/truncated and cannot be replayed safely.",
                json!({
                    "type": "string",
                    "raw_len": raw.len(),
                    "preview": raw.chars().take(240).collect::<String>(),
                }),
            );
            tracing::debug!(
                tool = tool_name,
                tool_call_id = tool_call_id,
                raw_len = raw.len(),
                "routing unrecoverable historical tool_call input to invalid tool"
            );
            increment_tool_args_invalid(tool_name, "history", raw.len());
            NormalizedHistoricalToolCall {
                tool_name: "invalid".to_string(),
                arguments: payload.to_string(),
            }
        }
        other => {
            let input_type = match other {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
                Value::String(_) => "string",
            };
            let payload = invalid_tool_payload_for_history(
                tool_name,
                tool_call_id,
                "Historical tool arguments are non-object and cannot be replayed safely.",
                json!({
                    "type": input_type,
                }),
            );
            tracing::debug!(
                tool = tool_name,
                tool_call_id = tool_call_id,
                input_type = input_type,
                "routing non-object historical tool_call input to invalid tool"
            );
            increment_tool_args_invalid(tool_name, "history", 0);
            NormalizedHistoricalToolCall {
                tool_name: "invalid".to_string(),
                arguments: payload.to_string(),
            }
        }
    }
}

static TOOL_ARGS_RECOVERED_TOTAL: AtomicU64 = AtomicU64::new(0);
static TOOL_ARGS_INVALID_TOTAL: AtomicU64 = AtomicU64::new(0);

fn increment_tool_args_recovered(tool_name: &str, phase: &'static str, raw_len: usize) {
    let total = TOOL_ARGS_RECOVERED_TOTAL.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(
        metric = "tool_args_recovered_total",
        total,
        tool = tool_name,
        phase,
        raw_len,
        "tool arguments recovered"
    );
    if total.is_multiple_of(25) {
        tracing::info!(
            metric = "tool_args_recovered_total",
            total,
            "tool arguments recovered aggregate"
        );
    }
}

fn increment_tool_args_invalid(tool_name: &str, phase: &'static str, raw_len: usize) {
    let total = TOOL_ARGS_INVALID_TOTAL.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(
        metric = "tool_args_invalid_total",
        total,
        tool = tool_name,
        phase,
        raw_len,
        "tool arguments routed to invalid"
    );
    if total.is_multiple_of(25) {
        tracing::info!(
            metric = "tool_args_invalid_total",
            total,
            "tool arguments invalid aggregate"
        );
    }
}

// ===========================================================================
// Layer 3 — SSE Parsing
// ===========================================================================

// ===========================================================================
// Layer 4 — Message Conversion
// ===========================================================================

fn to_openai_compatible_chat_messages(messages: &[Message]) -> Vec<Value> {
    let mut converted = Vec::new();
    let mut assistant_tool_call_ids: HashSet<String> = HashSet::new();
    let historical_tool_result_ids: HashSet<String> = messages
        .iter()
        .filter(|m| matches!(m.role, Role::Tool))
        .flat_map(|message| match &message.content {
            crate::Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| {
                    part.tool_result
                        .as_ref()
                        .map(|tool_result| tool_result.tool_use_id.clone())
                })
                .collect::<Vec<_>>(),
            crate::Content::Text(_) => Vec::new(),
        })
        .collect();

    for message in messages {
        match message.role {
            Role::System => {
                converted.push(json!({
                    "role": "system",
                    "content": content_text_lossy(&message.content),
                }));
            }
            Role::User => {
                converted.push(json!({
                    "role": "user",
                    "content": user_content_to_openai(&message.content),
                }));
            }
            Role::Assistant => {
                let (assistant_msg, emitted_tool_calls) =
                    assistant_message_to_openai(&message.content);
                assistant_tool_call_ids.extend(emitted_tool_calls.iter().cloned());
                converted.push(assistant_msg);
                for tool_call_id in emitted_tool_calls {
                    if historical_tool_result_ids.contains(&tool_call_id) {
                        continue;
                    }
                    converted.push(interrupted_tool_result_to_openai(&tool_call_id));
                }
            }
            Role::Tool => {
                converted.extend(tool_messages_to_openai(
                    &message.content,
                    &assistant_tool_call_ids,
                ));
            }
        }
    }

    converted
}

fn interrupted_tool_result_to_openai(tool_call_id: &str) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": "[Tool execution was interrupted]",
    })
}

fn content_text_lossy(content: &crate::Content) -> String {
    match content {
        crate::Content::Text(text) => text.clone(),
        crate::Content::Parts(parts) => parts
            .iter()
            .filter_map(|part| part.text.clone())
            .collect::<Vec<_>>()
            .join(""),
    }
}

fn user_content_to_openai(content: &crate::Content) -> Value {
    match content {
        crate::Content::Text(text) => Value::String(text.clone()),
        crate::Content::Parts(parts) => {
            if parts.len() == 1 && parts[0].content_type == "text" && parts[0].text.is_some() {
                return Value::String(parts[0].text.clone().unwrap_or_default());
            }

            let mut converted_parts = Vec::new();
            for part in parts {
                if let Some(text) = &part.text {
                    converted_parts.push(json!({
                        "type": "text",
                        "text": text,
                    }));
                    continue;
                }

                if let Some(image) = &part.image_url {
                    converted_parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": image.url },
                    }));
                }
            }

            if converted_parts.is_empty() {
                Value::String(String::new())
            } else {
                Value::Array(converted_parts)
            }
        }
    }
}

fn assistant_message_to_openai(content: &crate::Content) -> (Value, Vec<String>) {
    match content {
        crate::Content::Text(text) => (
            json!({
                "role": "assistant",
                "content": text,
            }),
            Vec::new(),
        ),
        crate::Content::Parts(parts) => {
            let mut text = String::new();
            let mut reasoning_content = String::new();
            let mut tool_calls = Vec::new();

            for part in parts {
                match part.content_type.as_str() {
                    "text" => {
                        if let Some(part_text) = &part.text {
                            text.push_str(part_text);
                        }
                    }
                    "reasoning" => {
                        if let Some(part_text) = &part.text {
                            reasoning_content.push_str(part_text);
                        }
                    }
                    "tool_use" => {
                        if let Some(tool_use) = &part.tool_use {
                            let normalized = normalize_tool_call_arguments_for_request(
                                &tool_use.name,
                                &tool_use.id,
                                &tool_use.input,
                            );
                            tool_calls.push(json!({
                                "id": tool_use.id,
                                "type": "function",
                                "function": {
                                    "name": normalized.tool_name,
                                    "arguments": normalized.arguments,
                                }
                            }));
                        }
                    }
                    _ => {
                        if let Some(part_text) = &part.text {
                            text.push_str(part_text);
                        }
                    }
                }
            }

            let mut message = Map::new();
            message.insert("role".to_string(), Value::String("assistant".to_string()));
            if !reasoning_content.is_empty() {
                message.insert(
                    "reasoning_content".to_string(),
                    Value::String(reasoning_content),
                );
            }
            if tool_calls.is_empty() {
                message.insert("content".to_string(), Value::String(text));
            } else {
                message.insert(
                    "content".to_string(),
                    if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    },
                );
                message.insert("tool_calls".to_string(), Value::Array(tool_calls));
            }
            let ids = message
                .get("tool_calls")
                .and_then(|value| value.as_array())
                .map(|calls| {
                    calls
                        .iter()
                        .filter_map(|call| call.get("id").and_then(Value::as_str))
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            (Value::Object(message), ids)
        }
    }
}

fn tool_messages_to_openai(
    content: &crate::Content,
    assistant_tool_call_ids: &HashSet<String>,
) -> Vec<Value> {
    match content {
        crate::Content::Text(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "role": "user",
                    "content": text,
                })]
            }
        }
        crate::Content::Parts(parts) => {
            let mut messages = Vec::new();
            for part in parts {
                if let Some(tool_result) = &part.tool_result {
                    if !assistant_tool_call_ids.contains(&tool_result.tool_use_id) {
                        tracing::warn!(
                            tool_call_id = %tool_result.tool_use_id,
                            "dropping orphan historical tool message without matching assistant tool_call"
                        );
                        continue;
                    }
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_result.tool_use_id,
                        "content": tool_result.content,
                    }));
                } else if let Some(text) = &part.text {
                    if !text.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": text,
                        }));
                    }
                }
            }
            messages
        }
    }
}

// ===========================================================================
// Layer 5 — Request Building & URL
// ===========================================================================

fn build_request_body(request: &ChatRequest) -> Result<Value, ProviderError> {
    let mut value =
        serde_json::to_value(request).map_err(|e| ProviderError::InvalidRequest(e.to_string()))?;

    if let Value::Object(obj) = &mut value {
        let mut prompt = request.messages.clone();
        if let Some(system) = &request.system {
            let has_system = prompt.iter().any(|m| matches!(m.role, Role::System));
            if !has_system {
                prompt.insert(0, Message::system(system.clone()));
            }
        }
        obj.insert(
            "messages".to_string(),
            Value::Array(to_openai_compatible_chat_messages(&prompt)),
        );
        obj.remove("system");

        // Merge provider_options into the top-level body (matching TS SDK behavior).
        // The TS SDK spreads provider options directly into the request body so that
        // provider-specific fields like `thinking`, `enable_thinking`, etc. are sent
        // as top-level keys rather than nested under `provider_options`.
        if let Some(Value::Object(opts)) = obj.remove("provider_options") {
            for (k, v) in opts {
                obj.entry(k).or_insert(v);
            }
        }

        let effort = openai_reasoning_effort(&request.model, request.variant.as_deref());
        if let Some(effort) = effort {
            obj.insert(
                "reasoning_effort".to_string(),
                Value::String(effort.to_string()),
            );
        }
    }

    Ok(value)
}

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

fn openai_reasoning_effort(model_id: &str, variant: Option<&str>) -> Option<&'static str> {
    let variant = variant?.trim().to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();
    let supports_effort = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("gpt-5")
        || model.contains("codex");
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
    let mut prompt = request.messages.clone();
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
// Layer 7a — Legacy HTTP Path (chat/completions)
// ===========================================================================

async fn chat_legacy(
    client: &Client,
    config: &ProviderConfig,
    request: ChatRequest,
) -> Result<ChatResponse, ProviderError> {
    let base = legacy_base_url(config)?;
    let url = chat_completions_url(base);
    let mut request_body = build_request_body(&request)?;

    // Ensure stream is disabled for non-streaming path. The caller may have
    // set stream=true on the ChatRequest (e.g. prompt loop), but chat_legacy
    // expects a single JSON response, not SSE chunks.
    if let Value::Object(obj) = &mut request_body {
        obj.remove("stream");
        obj.remove("stream_options");
    }

    let mut req_builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");

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

/// Reassemble SSE `data:` chunks (streaming format) into a single `RawChatResponse`.
/// Some closeai-compatible providers return SSE even for non-streaming requests.
fn reassemble_sse_chunks(body: &str) -> Result<RawChatResponse, ProviderError> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<RawUsage> = None;
    // tool_calls keyed by index: (id, name, arguments)
    let mut tool_calls: HashMap<u32, (Option<String>, Option<String>, String)> = HashMap::new();

    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line[5..].trim();
        if data == "[DONE]" {
            break;
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(delta) = choice.get("delta") {
                    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                        content.push_str(text);
                    }
                    // ZhipuAI uses "reasoning_content"; OpenAI uses "reasoning_text"
                    let reasoning_val = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning_text"))
                        .and_then(|v| v.as_str());
                    if let Some(r) = reasoning_val {
                        reasoning.push_str(r);
                    }
                    if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let entry =
                                tool_calls.entry(idx).or_insert((None, None, String::new()));
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                entry.0 = Some(id.to_string());
                            }
                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                    entry.1 = Some(name.to_string());
                                }
                                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                    entry.2.push_str(args);
                                }
                            }
                        }
                    }
                }
                if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                    finish_reason = Some(fr.to_string());
                }
            }
        }
        if let Some(u) = chunk.get("usage") {
            usage = serde_json::from_value(u.clone()).ok();
        }
    }

    let raw_tool_calls: Option<Vec<RawToolCall>> = if tool_calls.is_empty() {
        None
    } else {
        let mut sorted: Vec<_> = tool_calls.into_iter().collect();
        sorted.sort_by_key(|(idx, _)| *idx);
        Some(
            sorted
                .into_iter()
                .map(|(_idx, (id, name, args))| RawToolCall {
                    id,
                    function: Some(RawFunction {
                        name,
                        arguments: Some(args),
                    }),
                })
                .collect(),
        )
    };

    Ok(RawChatResponse {
        id: None,
        model: None,
        choices: vec![RawChoice {
            index: Some(0),
            message: Some(RawMessage {
                role: Some("assistant".to_string()),
                content: if content.is_empty() {
                    None
                } else {
                    Some(content)
                },
                tool_calls: raw_tool_calls,
                _reasoning_text: if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                },
            }),
            finish_reason,
        }],
        usage,
    })
}

async fn chat_stream_openai_compatible(
    client: &Client,
    config: &ProviderConfig,
    mut request: ChatRequest,
    use_pipeline: bool,
) -> Result<StreamResult, ProviderError> {
    let base = legacy_base_url(config)?;
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

    let mut req_builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream");

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

async fn chat_stream_legacy(
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
// OpenAIProtocol struct + ProtocolImpl
// ===========================================================================

pub struct OpenAIProtocol;

impl Default for OpenAIProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAIProtocol {
    pub fn new() -> Self {
        Self
    }
}

// Phase 3: Full dual routing — Responses API with Legacy fallback.
#[async_trait]
impl ProtocolImpl for OpenAIProtocol {
    async fn chat(
        &self,
        client: &Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        if is_legacy_only(config) {
            return chat_legacy(client, config, request).await;
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
                        "Responses generate failed while custom fetch proxy is active; skipping legacy fallback"
                    );
                    return Err(err);
                }
                tracing::warn!(
                    model = %model_for_log,
                    error = %err,
                    "Responses generate failed, falling back to chat completions"
                );
                chat_legacy(&client_for_fallback, &config_for_fallback, request).await
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
        if is_legacy_only(config) {
            return if use_pipeline {
                chat_stream_runtime_pipeline(client, config, request).await
            } else {
                chat_stream_legacy(client, config, request).await
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
                        "Responses stream failed while custom fetch proxy is active; skipping legacy fallback"
                    );
                    return Err(err);
                }
                tracing::warn!(
                    model = %model_for_log,
                    error = %err,
                    "Responses stream failed, falling back to chat completions stream"
                );
                if use_pipeline {
                    chat_stream_runtime_pipeline(&client_for_fallback, &config_for_fallback, request)
                        .await
                } else {
                    chat_stream_legacy(&client_for_fallback, &config_for_fallback, request).await
                }
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fetch::{
        register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
        CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    };
    use async_trait::async_trait;
    use futures::stream;
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
    async fn resolve_with_fallback_skips_legacy_when_custom_fetch_active() {
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
    fn openai_native_provider_does_not_use_legacy_only() {
        let config = ProviderConfig::new("openai", "https://example.com/v1", "test-key");
        assert!(!is_legacy_only(&config));
    }

    #[test]
    fn openai_compatible_provider_uses_legacy_only() {
        let config = ProviderConfig::new("deepseek", "", "test-key")
            .with_option("legacy_only", serde_json::json!(true));
        assert!(is_legacy_only(&config));
    }

    #[test]
    fn legacy_base_url_allows_empty_for_openai_provider() {
        let config = ProviderConfig::new("openai", "   ", "test-key");
        assert!(legacy_base_url(&config).unwrap().is_none());
    }

    #[test]
    fn legacy_base_url_rejects_empty_for_openai_compatible_provider() {
        let config = ProviderConfig::new("deepseek", "   ", "test-key");
        let err = legacy_base_url(&config).unwrap_err();
        assert!(matches!(
            err,
            ProviderError::ConfigError(msg)
                if msg.contains("requires `base_url` for closeai-compatible routing")
        ));
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
        assert_eq!(
            converted[1]["content"],
            "[Tool execution was interrupted]"
        );
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
