use serde_json::{json, Value};

pub(super) fn parse_tool_call_input(args: &str) -> Value {
    match serde_json::from_str(args) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                %error,
                args_len = args.len(),
                "failed to decode OpenAI tool call arguments"
            );
            Value::String(args.to_string())
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct HistoricalToolCall {
    pub(super) tool_name: String,
    pub(super) arguments: String,
}

pub(super) fn serialize_historical_tool_call(
    tool_name: &str,
    tool_call_id: &str,
    input: &Value,
) -> HistoricalToolCall {
    if input.is_object() {
        return HistoricalToolCall {
            tool_name: tool_name.to_string(),
            arguments: input.to_string(),
        };
    }

    let input_type = match input {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::String(_) => "string",
    };
    HistoricalToolCall {
        tool_name: "invalid".to_string(),
        arguments: json!({
            "tool": tool_name,
            "toolCallId": tool_call_id,
            "error": "Historical tool arguments are non-object and cannot be replayed.",
            "receivedArgs": { "type": input_type },
        })
        .to_string(),
    }
}
