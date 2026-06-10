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
}
