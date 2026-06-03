use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn parse_tool_call_input(tool_name: &str, args_str: &str) -> Value {
    let strict = serde_json::from_str::<Value>(args_str);
    if let Ok(parsed @ Value::Object(_)) = &strict {
        return parsed.clone();
    }

    if let Some(parsed_object) = agendao_util::json::try_parse_json_object_robust(args_str) {
        increment_tool_args_recovered(tool_name, "parse", args_str.len());
        return parsed_object;
    }

    if let Some(recovered) =
        agendao_util::json::recover_tool_arguments_from_jsonish(tool_name, args_str)
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
pub(super) struct NormalizedHistoricalToolCall {
    pub(super) tool_name: String,
    pub(super) arguments: String,
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

pub(super) fn normalize_tool_call_arguments_for_request(
    tool_name: &str,
    tool_call_id: &str,
    input: &Value,
) -> NormalizedHistoricalToolCall {
    match input {
        Value::Object(obj) => {
            let is_legacy_unrecoverable = obj
                .get("_agendao_unrecoverable_tool_args")
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
            if let Some(parsed_object) = agendao_util::json::try_parse_json_object_robust(raw) {
                increment_tool_args_recovered(tool_name, "history", raw.len());
                return NormalizedHistoricalToolCall {
                    tool_name: tool_name.to_string(),
                    arguments: parsed_object.to_string(),
                };
            }
            if let Some(recovered) =
                agendao_util::json::recover_tool_arguments_from_jsonish(tool_name, raw)
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
