use serde_json::Value;
use std::collections::HashMap;

use crate::{ChatRequest, Message, Role};

pub const CONTINUATION_BOUNDARY_REASONING_OPTION_KEYS: &[&str] = &[
    "reasoning",
    "reasoning_effort",
    "reasoningEffort",
    "reasoningSummary",
    "reasoning_summary",
    "thinking",
    "include_reasoning",
    "includeReasoning",
    "enable_thinking",
    "thinkingConfig",
];

pub fn thinking_value_enabled(value: &Value) -> bool {
    match value {
        Value::Bool(enabled) => *enabled,
        Value::String(text) => !matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "off" | "no" | "none" | "disabled"
        ),
        Value::Number(number) => number.as_i64().unwrap_or(0) != 0,
        Value::Object(map) => {
            if let Some(enabled) = map.get("enabled") {
                return thinking_value_enabled(enabled);
            }
            if let Some(enabled) = map.get("enable_thinking") {
                return thinking_value_enabled(enabled);
            }
            if let Some(value) = map.get("includeThoughts") {
                return thinking_value_enabled(value);
            }
            if let Some(value) = map.get("type") {
                return thinking_value_enabled(value);
            }
            if let Some(value) = map.get("effort") {
                return thinking_value_enabled(value);
            }
            !map.is_empty()
        }
        Value::Array(values) => !values.is_empty(),
        Value::Null => false,
    }
}

pub fn thinking_value_disabled(value: &Value) -> bool {
    match value {
        Value::Bool(enabled) => !enabled,
        Value::String(text) => matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "off" | "no" | "none" | "disabled"
        ),
        Value::Number(number) => number.as_i64().unwrap_or(0) == 0,
        Value::Object(map) => {
            if let Some(enabled) = map.get("enabled") {
                return thinking_value_disabled(enabled);
            }
            if let Some(enabled) = map.get("enable_thinking") {
                return thinking_value_disabled(enabled);
            }
            if let Some(value) = map.get("includeThoughts") {
                return thinking_value_disabled(value);
            }
            if let Some(value) = map.get("type") {
                return thinking_value_disabled(value);
            }
            if let Some(value) = map.get("effort") {
                return thinking_value_disabled(value);
            }
            false
        }
        Value::Array(values) => values.is_empty(),
        Value::Null => true,
    }
}

pub fn request_explicitly_enables_thinking(request: &ChatRequest) -> bool {
    let Some(options) = request.provider_options.as_ref() else {
        return false;
    };

    for key in [
        "thinking",
        "reasoning",
        "enable_thinking",
        "thinkingConfig",
        "reasoningEffort",
        "reasoning_effort",
    ] {
        if options.get(key).is_some_and(thinking_value_enabled) {
            return true;
        }
    }

    options
        .get("chat_template_args")
        .is_some_and(thinking_value_enabled)
}

pub fn request_explicitly_disables_thinking(request: &ChatRequest) -> bool {
    let Some(options) = request.provider_options.as_ref() else {
        return false;
    };

    for key in [
        "thinking",
        "reasoning",
        "enable_thinking",
        "thinkingConfig",
        "reasoningEffort",
        "reasoning_effort",
    ] {
        if options.get(key).is_some_and(thinking_value_disabled) {
            return true;
        }
    }

    options
        .get("chat_template_args")
        .is_some_and(thinking_value_disabled)
}

pub fn request_effectively_enables_thinking(
    request: &ChatRequest,
    default_thinking_enabled: bool,
) -> bool {
    request_explicitly_enables_thinking(request)
        || (default_thinking_enabled && !request_explicitly_disables_thinking(request))
}

pub fn message_has_reasoning_replay(message: &Message) -> bool {
    let has_wire_field = |provider_options: &Option<HashMap<String, Value>>| {
        provider_options
            .as_ref()
            .and_then(|options| options.get("openaiCompatible"))
            .and_then(Value::as_object)
            .is_some_and(|options| {
                options.contains_key("reasoning_content")
                    || options.contains_key("reasoning_details")
            })
    };

    if has_wire_field(&message.provider_options) {
        return true;
    }

    match &message.content {
        crate::Content::Text(_) => false,
        crate::Content::Parts(parts) => parts.iter().any(|part| {
            matches!(part.content_type.as_str(), "reasoning" | "thinking")
                && part
                    .text
                    .as_ref()
                    .is_some_and(|text| !text.trim().is_empty())
                || has_wire_field(&part.provider_options)
        }),
    }
}

pub fn message_has_tool_calls(message: &Message) -> bool {
    match &message.content {
        crate::Content::Text(_) => false,
        crate::Content::Parts(parts) => parts
            .iter()
            .any(|part| part.content_type == "tool_use" || part.tool_use.is_some()),
    }
}

pub fn request_has_tool_call_continuation_missing_reasoning_replay(messages: &[Message]) -> bool {
    messages.iter().enumerate().any(|(index, message)| {
        matches!(message.role, Role::Assistant)
            && index + 1 < messages.len()
            && message_has_tool_calls(message)
            && !message_has_reasoning_replay(message)
    })
}

pub fn strip_reasoning_provider_options_for_new_continuation(
    provider_options: Option<HashMap<String, Value>>,
    disable_marker: Option<(&str, Value)>,
) -> (Option<HashMap<String, Value>>, Vec<String>) {
    let mut removed = Vec::new();
    let mut provider_options = provider_options.unwrap_or_default();

    for key in CONTINUATION_BOUNDARY_REASONING_OPTION_KEYS {
        if provider_options.remove(*key).is_some() {
            removed.push((*key).to_string());
        }
    }
    if provider_options.remove("chat_template_args").is_some() {
        removed.push("chat_template_args".to_string());
    }

    if let Some((key, value)) = disable_marker {
        provider_options.insert(key.to_string(), value);
        if !removed.iter().any(|removed_key| removed_key == key) {
            removed.push(key.to_string());
        }
    }

    let provider_options = if provider_options.is_empty() {
        None
    } else {
        Some(provider_options)
    };

    (provider_options, removed)
}
