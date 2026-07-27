use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCatalogMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subfamily: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

/// Return the model-authoritative replay input for a historical tool call.
///
/// Contract:
/// - Prefer the original raw model bytes when available and parseable.
/// - Fall back to the normalized stored input only when raw replay is absent
///   or cannot be reconstructed as JSON.
pub fn tool_call_replay_input(input: &Value, raw: Option<&str>) -> Value {
    tool_call_non_empty_raw(raw)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| input.clone())
}

/// Return the exact textual payload that should be considered authoritative for
/// replay/caching math.
///
/// Contract:
/// - Prefer the original raw model bytes when present.
/// - Otherwise fall back to the normalized observable serialization.
pub fn tool_call_replay_text(input: &Value, raw: Option<&str>) -> Option<String> {
    tool_call_non_empty_raw(raw)
        .map(ToOwned::to_owned)
        .or_else(|| tool_call_observable_arguments(input))
}

/// Byte length of the replay text for a historical tool call, computed without
/// materializing the payload.
///
/// Exactly matches `tool_call_replay_text(input, raw).map_or(0, |text| text.len())`:
/// callers that only need the length (token/context estimation) can avoid the
/// per-call JSON serialization heap allocation.
pub fn tool_call_replay_text_len(input: &Value, raw: Option<&str>) -> usize {
    if let Some(raw) = tool_call_non_empty_raw(raw) {
        return raw.len();
    }
    tool_call_observable_arguments_len(input).unwrap_or(0)
}

/// Byte length of `tool_call_observable_arguments(input)` without allocation.
fn tool_call_observable_arguments_len(input: &Value) -> Option<usize> {
    match input {
        Value::Null => None,
        Value::Object(object) if object.is_empty() => None,
        Value::Array(array) if array.is_empty() => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.len())
        }
        other => Some(json_serialized_byte_len(other)),
    }
}

/// Byte length of `value.to_string()` (compact JSON serialization) computed via a
/// counting writer, so no `String` is materialized.
fn json_serialized_byte_len(value: &Value) -> usize {
    struct ByteCounter(usize);
    impl std::io::Write for ByteCounter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = ByteCounter(0);
    // Serializing a `Value` never fails; `to_string` is `to_writer` into a `String`,
    // so the counted bytes match `value.to_string().len()` exactly.
    let _ = serde_json::to_writer(&mut counter, value);
    counter.0
}

/// Return the human-visible tool-call arguments for transcript / UI surfaces.
///
/// Contract:
/// - Uses normalized/stored input only.
/// - Never prefers the raw model bytes, because raw is replay/debug authority,
///   not display authority.
pub fn tool_call_observable_arguments(input: &Value) -> Option<String> {
    match input {
        Value::Null => None,
        Value::Object(object) if object.is_empty() => None,
        Value::Array(array) if array.is_empty() => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        other => Some(other.to_string()),
    }
}

fn tool_call_non_empty_raw(raw: Option<&str>) -> Option<&str> {
    raw.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replay_input_prefers_parseable_raw_shape() {
        let replay = tool_call_replay_input(
            &json!({"path":"normalized.txt"}),
            Some("{\"path\":\"raw.txt\"}"),
        );

        assert_eq!(replay, json!({"path":"raw.txt"}));
    }

    #[test]
    fn replay_input_falls_back_to_normalized_when_raw_is_not_json() {
        let replay = tool_call_replay_input(&json!({"path":"normalized.txt"}), Some("oops"));

        assert_eq!(replay, json!({"path":"normalized.txt"}));
    }

    #[test]
    fn replay_text_prefers_raw_bytes_while_observable_uses_normalized() {
        let input = json!({"path":"normalized.txt"});

        assert_eq!(
            tool_call_replay_text(&input, Some("{\"path\":\"raw.txt\"}")),
            Some("{\"path\":\"raw.txt\"}".to_string())
        );
        assert_eq!(
            tool_call_observable_arguments(&input),
            Some("{\"path\":\"normalized.txt\"}".to_string())
        );
    }

    #[test]
    fn replay_text_len_matches_materialized_replay_text_len() {
        let cases: Vec<(Value, Option<&str>)> = vec![
            (json!({"path":"normalized.txt"}), Some("{\"path\":\"raw.txt\"}")),
            (json!({"path":"normalized.txt"}), None),
            (json!({"path":"normalized.txt"}), Some("   ")),
            (json!(null), None),
            (json!({}), None),
            (json!([]), None),
            (json!("  padded  "), None),
            (json!("   "), None),
            (json!({"nested":{"list":[1,2,3],"unicode":"中文"}}), None),
            (json!(42), None),
        ];
        for (input, raw) in cases {
            assert_eq!(
                tool_call_replay_text_len(&input, raw),
                tool_call_replay_text(&input, raw).map_or(0, |text| text.len()),
                "input={input:?} raw={raw:?}"
            );
        }
    }
}
