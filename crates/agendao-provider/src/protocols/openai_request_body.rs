use serde_json::{json, Map, Value};

use super::openai_tool_arguments::serialize_historical_tool_call;
use super::request_sanitizer::{
    interrupted_tool_result_text, sanitize_messages_for_protocol, SanitizerOptions,
};
use crate::{clamp_effort, openai_compatible_efforts, ChatRequest, Message, ProviderError, Role};

pub(super) fn to_openai_compatible_chat_messages(messages: &[Message]) -> Vec<Value> {
    let sanitized = sanitize_messages_for_protocol(messages, SanitizerOptions::default());
    let mut converted = Vec::new();

    for message in &sanitized {
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
                let (assistant_msg, _emitted_tool_calls) = assistant_message_to_openai(message);
                converted.push(assistant_msg);
            }
            Role::Tool => {
                converted.extend(tool_messages_to_openai(&message.content));
            }
        }
    }
    converted
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

                match part.content_type.as_str() {
                    "image" | "image_url" => {
                        if let Some(image) = &part.image_url {
                            converted_parts.push(json!({
                                "type": "image_url",
                                "image_url": { "url": image.url },
                            }));
                        }
                    }
                    "file" => converted_file_part_to_openai(part, &mut converted_parts),
                    _ => {}
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

fn converted_file_part_to_openai(part: &crate::ContentPart, converted_parts: &mut Vec<Value>) {
    let Some(url) = part.image_url.as_ref().map(|image| image.url.as_str()) else {
        return;
    };
    let mime = part
        .media_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    if mime.starts_with("image/") {
        converted_parts.push(json!({
            "type": "image_url",
            "image_url": { "url": url },
        }));
        return;
    }

    if mime.starts_with("audio/") {
        if let Some((format, data)) = data_url_audio(url) {
            converted_parts.push(json!({
                "type": "input_audio",
                "input_audio": { "data": data, "format": format },
            }));
        } else {
            let label = part.filename.as_deref().unwrap_or("audio attachment");
            converted_parts.push(json!({
                "type": "text",
                "text": format!("[Audio attachment `{label}` could not be inlined; provide a data URL or a model/provider that accepts audio URLs.]"),
            }));
        }
        return;
    }

    // The base Chat Completions protocol has no portable video/document part.
    // Do not emit a relay-specific `video_url` field here: without a provider
    // capability declaration that would turn a valid request into a 400.
    if mime.starts_with("video/") {
        converted_parts.push(json!({
            "type": "text",
            "text": format!(
                "[Video attachment `{}` is not supported by the Chat Completions wire shape.]",
                part.filename.as_deref().unwrap_or("video attachment")
            ),
        }));
        return;
    }

    let label = part.filename.as_deref().unwrap_or("file attachment");
    converted_parts.push(json!({
        "type": "text",
        "text": format!("[File attachment `{label}` ({mime}) is not supported by the Chat Completions wire shape.]"),
    }));
}

fn data_url_audio(url: &str) -> Option<(&str, &str)> {
    let (header, data) = url.strip_prefix("data:")?.split_once(',')?;
    if !header.to_ascii_lowercase().contains(";base64") {
        return None;
    }
    let mime = header.split(';').next()?.trim();
    let format = match mime {
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "audio/webm" => "webm",
        _ => mime.strip_prefix("audio/")?,
    };
    Some((format, data))
}

fn assistant_reasoning_wire_fields(
    message: &Message,
    parts: &[crate::ContentPart],
) -> Map<String, Value> {
    let mut fields = Map::new();

    apply_reasoning_wire_fields(message.provider_options.as_ref(), &mut fields);
    for part in parts {
        apply_reasoning_wire_fields(part.provider_options.as_ref(), &mut fields);
    }

    fields
}

fn apply_reasoning_wire_fields(
    provider_options: Option<&std::collections::HashMap<String, Value>>,
    fields: &mut Map<String, Value>,
) {
    let Some(provider_options) = provider_options else {
        return;
    };

    let Some(Value::Object(openai_compatible)) = provider_options.get("openaiCompatible") else {
        return;
    };

    for field in ["reasoning_content", "reasoning_details"] {
        if let Some(value) = openai_compatible.get(field) {
            fields.insert(field.to_string(), value.clone());
        }
    }
}

fn assistant_message_to_openai(message: &Message) -> (Value, Vec<String>) {
    match &message.content {
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
                    "reasoning" | "thinking" => {
                        if let Some(part_text) = &part.text {
                            reasoning_content.push_str(part_text);
                        }
                    }
                    "tool_use" => {
                        if let Some(tool_use) = &part.tool_use {
                            let normalized = serialize_historical_tool_call(
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

            let mut assistant_obj = Map::new();
            assistant_obj.insert("role".to_string(), Value::String("assistant".to_string()));
            let reasoning_wire_fields = assistant_reasoning_wire_fields(message, parts);
            if !reasoning_wire_fields.is_empty() {
                for (field, value) in reasoning_wire_fields {
                    assistant_obj.insert(field, value);
                }
            } else if !reasoning_content.is_empty() {
                assistant_obj.insert(
                    "reasoning_content".to_string(),
                    Value::String(reasoning_content),
                );
            }
            if tool_calls.is_empty() {
                assistant_obj.insert("content".to_string(), Value::String(text));
            } else {
                assistant_obj.insert(
                    "content".to_string(),
                    if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    },
                );
                assistant_obj.insert("tool_calls".to_string(), Value::Array(tool_calls));
            }
            let ids = assistant_obj
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

            (Value::Object(assistant_obj), ids)
        }
    }
}

fn tool_messages_to_openai(content: &crate::Content) -> Vec<Value> {
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
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_result.tool_use_id,
                        "content": if tool_result.content == interrupted_tool_result_text() {
                            interrupted_tool_result_text().to_string()
                        } else {
                            tool_result.content.clone()
                        },
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
    // Build the top-level object field by field: `messages` is converted
    // exactly once, and every other field is serialized exactly once (the
    // previous implementation ran `to_value(request)` on the whole
    // ChatRequest, cloned the message list, then overwrote "messages").
    fn field_value<T: serde::Serialize>(value: &T) -> Result<Value, ProviderError> {
        serde_json::to_value(value).map_err(|e| ProviderError::InvalidRequest(e.to_string()))
    }

    let mut obj = Map::new();
    obj.insert("model".to_string(), field_value(&request.model)?);

    // Only the rare "prepend a system prompt" path needs an owned copy of the
    // message list; otherwise convert the borrowed slice directly.
    let needs_system_prepend = request.system.is_some()
        && !request
            .messages
            .iter()
            .any(|m| matches!(m.role, Role::System));
    let messages = if needs_system_prepend {
        let mut prompt = Vec::with_capacity(request.messages.len() + 1);
        prompt.push(Message::system(
            request.system.clone().expect("system checked above"),
        ));
        prompt.extend(request.messages.iter().cloned());
        to_openai_compatible_chat_messages(&prompt)
    } else {
        to_openai_compatible_chat_messages(&request.messages)
    };
    obj.insert("messages".to_string(), Value::Array(messages));

    if let Some(max_tokens) = request.max_tokens {
        obj.insert("max_tokens".to_string(), Value::from(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        obj.insert("temperature".to_string(), field_value(&temperature)?);
    }
    if let Some(top_p) = request.top_p {
        obj.insert("top_p".to_string(), field_value(&top_p)?);
    }
    // `system` is folded into `messages` above and never sent as its own key.
    if let Some(tools) = &request.tools {
        obj.insert("tools".to_string(), field_value(tools)?);
    }
    if let Some(stream) = request.stream {
        obj.insert("stream".to_string(), Value::Bool(stream));
    }

    // Match TS SDK behavior: provider options are spread into the request
    // body, so provider-specific fields remain top-level keys.
    if let Some(opts) = &request.provider_options {
        for (k, v) in opts {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    // Typed `reasoning_effort` (per-model config) takes priority over the
    // variant-string fallback. Explicit `none` is sent only for wires that
    // advertise it; providers without a disable level keep their default.
    let requested = request.reasoning_effort.or_else(|| {
        openai_reasoning_effort(&request.model, request.variant.as_deref())
            .and_then(|value| value.parse().ok())
    });
    if let Some(requested) = requested {
        if let Some(effort) = clamp_effort(requested, openai_compatible_efforts(&request.model)) {
            obj.insert(
                "reasoning_effort".to_string(),
                Value::String(effort.to_string()),
            );
        }
    } else if let Some(effort) = openai_reasoning_effort(&request.model, request.variant.as_deref())
    {
        obj.insert(
            "reasoning_effort".to_string(),
            Value::String(effort.to_string()),
        );
    }

    Ok(Value::Object(obj))
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
        "max" => Some("max"),
        "xhigh" => Some("xhigh"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReasoningEffort;

    fn body_for(request: ChatRequest) -> Value {
        build_request_body(&request).expect("request body should build")
    }

    #[test]
    fn typed_reasoning_effort_maps_to_wire_string() {
        for (effort, expected) in [
            (ReasoningEffort::Minimal, "low"),
            (ReasoningEffort::Low, "low"),
            (ReasoningEffort::Medium, "medium"),
            (ReasoningEffort::High, "high"),
            (ReasoningEffort::XHigh, "xhigh"),
            (ReasoningEffort::Max, "xhigh"),
            (ReasoningEffort::Ultra, "xhigh"),
        ] {
            let mut request = ChatRequest::new("gpt-5-codex", vec![Message::user("hi")]);
            request.reasoning_effort = Some(effort);
            let body = body_for(request);
            assert_eq!(
                body.get("reasoning_effort").and_then(Value::as_str),
                Some(expected),
                "effort {effort} should map to {expected}"
            );
        }
    }

    #[test]
    fn typed_reasoning_effort_none_is_explicitly_disabled_when_supported() {
        let mut request = ChatRequest::new("gpt-5-codex", vec![Message::user("hi")]);
        request.reasoning_effort = Some(ReasoningEffort::None);
        let body = body_for(request);
        assert_eq!(
            body.get("reasoning_effort").and_then(Value::as_str),
            Some("none")
        );
    }

    #[test]
    fn typed_reasoning_effort_overrides_variant() {
        let mut request = ChatRequest::new("gpt-5-codex", vec![Message::user("hi")]);
        request.variant = Some("high".to_string());
        request.reasoning_effort = Some(ReasoningEffort::Low);
        let body = body_for(request);
        assert_eq!(
            body.get("reasoning_effort").and_then(Value::as_str),
            Some("low")
        );

        // Typed None also suppresses the variant-derived value.
        let mut request = ChatRequest::new("gpt-5-codex", vec![Message::user("hi")]);
        request.variant = Some("high".to_string());
        request.reasoning_effort = Some(ReasoningEffort::None);
        let body = body_for(request);
        assert_eq!(
            body.get("reasoning_effort").and_then(Value::as_str),
            Some("none")
        );
    }

    #[test]
    fn variant_whitelist_still_applies_without_typed_effort() {
        let mut request = ChatRequest::new("gpt-5-codex", vec![Message::user("hi")]);
        request.variant = Some("high".to_string());
        let body = body_for(request);
        assert_eq!(
            body.get("reasoning_effort").and_then(Value::as_str),
            Some("high")
        );

        let request = ChatRequest::new("gpt-5-codex", vec![Message::user("hi")]);
        let body = body_for(request);
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn maps_image_and_audio_attachments_to_openai_parts() {
        let request = ChatRequest::new(
            "gpt-4.1",
            vec![Message {
                role: Role::User,
                content: crate::Content::Parts(vec![
                    crate::ContentPart::image_url(
                        "data:image/png;base64,AAAA",
                        Some("diagram.png".to_string()),
                        Some("image/png".to_string()),
                    ),
                    crate::ContentPart::file(
                        "data:audio/wav;base64,UklGRg==",
                        Some("voice.wav".to_string()),
                        Some("audio/wav".to_string()),
                    ),
                ]),
                cache_control: None,
                provider_options: None,
            }],
        );
        let body = body_for(request);
        let parts = body["messages"][0]["content"].as_array().expect("parts");
        assert_eq!(parts[0]["type"], "image_url");
        assert_eq!(parts[0]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(parts[1]["type"], "input_audio");
        assert_eq!(parts[1]["input_audio"]["format"], "wav");
        assert_eq!(parts[1]["input_audio"]["data"], "UklGRg==");
    }
}
