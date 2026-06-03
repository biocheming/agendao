use crate::{MessageRole, SessionMessage};

pub(crate) fn session_message_hook_payload(message: &SessionMessage) -> serde_json::Value {
    let mut payload = serde_json::to_value(message).unwrap_or_else(|_| serde_json::json!({}));
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    object.insert(
        "info".to_string(),
        serde_json::json!({
            "id": message.id,
            "sessionID": message.session_id,
            "role": hook_message_role(&message.role),
            "time": { "created": message.created_at.timestamp_millis() },
        }),
    );

    payload
}

pub(crate) fn hook_message_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

pub(crate) fn apply_chat_messages_hook_outputs(
    messages: &mut Vec<SessionMessage>,
    hook_outputs: Vec<agendao_plugin::HookOutput>,
) {
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = agendao_plugin::hook_payload_object(payload) else {
            continue;
        };
        let Some(next_messages) = object.get("messages").and_then(|value| value.as_array()) else {
            continue;
        };
        let parsed = serde_json::from_value::<Vec<SessionMessage>>(serde_json::Value::Array(
            next_messages.clone(),
        ));
        if let Ok(next) = parsed {
            *messages = next;
        }
    }
}

pub(crate) fn apply_chat_message_hook_outputs(
    message: &mut SessionMessage,
    hook_outputs: Vec<agendao_plugin::HookOutput>,
) {
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = agendao_plugin::hook_payload_object(payload) else {
            continue;
        };
        if let Some(next_message) = object.get("message") {
            if let Ok(parsed) = serde_json::from_value::<SessionMessage>(next_message.clone()) {
                *message = parsed;
            }
        }
        if let Some(next_parts) = object.get("parts").and_then(|value| value.as_array()) {
            let parsed = serde_json::from_value::<Vec<crate::MessagePart>>(
                serde_json::Value::Array(next_parts.clone()),
            );
            if let Ok(parts) = parsed {
                message.parts = parts;
            }
        }
    }
}
