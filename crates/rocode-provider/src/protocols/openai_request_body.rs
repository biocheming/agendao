use serde_json::{json, Map, Value};
use std::collections::HashSet;

use super::openai_tool_recovery::normalize_tool_call_arguments_for_request;
use crate::{ChatRequest, Message, ProviderError, Role};

pub(super) fn to_openai_compatible_chat_messages(messages: &[Message]) -> Vec<Value> {
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

pub(super) fn build_request_body(request: &ChatRequest) -> Result<Value, ProviderError> {
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

        // Match TS SDK behavior: provider options are spread into the request
        // body, so provider-specific fields remain top-level keys.
        if let Some(Value::Object(opts)) = obj.remove("provider_options") {
            for (k, v) in opts {
                obj.entry(k).or_insert(v);
            }
        }

        if let Some(effort) = openai_reasoning_effort(&request.model, request.variant.as_deref()) {
            obj.insert(
                "reasoning_effort".to_string(),
                Value::String(effort.to_string()),
            );
        }

        // Models like deepseek-v4 automatically enter thinking mode based
        // on conversation complexity. Until reasoning_content round-trip
        // is fully supported, explicitly disable thinking when the user
        // has not opted in with a variant.
        if request.variant.is_none() && is_auto_thinking_model(&request.model) {
            if !obj.contains_key("thinking") {
                obj.insert(
                    "thinking".to_string(),
                    serde_json::json!({"type": "disabled"}),
                );
            }
        }
    }

    Ok(value)
}

fn is_auto_thinking_model(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    id.contains("deepseek") && id.contains("v4")
}

pub(super) fn openai_reasoning_effort(
    model_id: &str,
    variant: Option<&str>,
) -> Option<&'static str> {
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
