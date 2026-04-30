use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::{Message, ToolDefinition, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheProtocolFamily {
    CloseAiCompatible,
    EthnopicCompatible,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseAiCompatibleApiShape {
    ChatCompletions,
    Responses,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseAiCacheCapabilities {
    pub api_shape: CloseAiCompatibleApiShape,
    pub supports_prompt_cache_key: bool,
    pub supports_prompt_cache_retention: bool,
    pub supports_previous_response_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthnopicCacheCapabilities {
    pub supports_cache_control: bool,
    pub supports_cache_ttl: bool,
    pub supports_cache_scope: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCacheCapabilities {
    pub family: CacheProtocolFamily,
    pub closeai: Option<CloseAiCacheCapabilities>,
    pub ethnopic: Option<EthnopicCacheCapabilities>,
    pub override_: ProviderCacheOverrides,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCacheOverrides {
    pub usage_parser: Option<CacheUsageParserKind>,
    pub extra_headers: Vec<CacheHeaderCapability>,
    pub ignored_fields: Vec<String>,
    pub provider_notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheUsageParserKind {
    CloseAiCachedTokens,
    EthnopicReadWrite,
    PromptCacheHitMiss,
    AutoDetect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheHeaderCapability {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawCacheUsage {
    CloseAi { raw_json: Value },
    Ethnopic { raw_json: Value },
    Unknown { raw_json: Value },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizedCacheUsage {
    CloseAi {
        input_tokens: u64,
        cached_input_tokens: u64,
        non_cached_input_tokens: u64,
        output_tokens: u64,
    },
    Ethnopic {
        input_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
        output_tokens: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSurfaceFingerprint {
    pub model: String,
    pub system_hash: String,
    pub tools_hash: String,
    pub message_prefix_hash: String,
    pub api_params_hash: String,
}

impl PromptSurfaceFingerprint {
    pub fn new(
        model: impl Into<String>,
        system: Option<&str>,
        tools: &[ToolDefinition],
        messages: &[Message],
        api_params: &Value,
    ) -> Self {
        Self {
            model: model.into(),
            system_hash: text_fingerprint(system.unwrap_or_default()),
            tools_hash: tool_surface_fingerprint(tools),
            message_prefix_hash: message_prefix_fingerprint(messages),
            api_params_hash: json_fingerprint(api_params),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsageMetrics {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub reasoning_tokens: u64,
    pub context_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_miss_tokens: u64,
    pub cache_write_tokens: u64,
}

impl TokenUsageMetrics {
    pub fn from_value(value: &Value) -> Self {
        let input_tokens = read_u64_any(value, &["prompt_tokens", "input_tokens"]);
        let output_tokens = read_u64_any(value, &["completion_tokens", "output_tokens"]);
        let total_tokens = read_u64_any(value, &["total_tokens"]);
        let reasoning_tokens = read_u64_any(value, &["reasoning_tokens"]);
        let cache_read_tokens = read_u64_any(
            value,
            &[
                "cache_read_input_tokens",
                "cache_read_tokens",
                "prompt_cache_hit_tokens",
            ],
        )
        .max(read_nested_u64_any(
            value,
            &[
                &["prompt_tokens_details", "cached_tokens"],
                &["input_tokens_details", "cached_tokens"],
            ],
        ));
        let cache_miss_tokens = read_u64_any(
            value,
            &[
                "cache_miss_input_tokens",
                "cache_miss_tokens",
                "prompt_cache_miss_tokens",
            ],
        );
        let cache_write_tokens = read_u64_any(
            value,
            &["cache_creation_input_tokens", "cache_write_tokens"],
        );
        let context_tokens = input_tokens
            .max(cache_read_tokens.saturating_add(cache_miss_tokens))
            .max(cache_read_tokens.saturating_add(cache_write_tokens));

        Self {
            input_tokens,
            output_tokens,
            total_tokens,
            reasoning_tokens,
            context_tokens,
            cache_read_tokens,
            cache_miss_tokens,
            cache_write_tokens,
        }
    }

    pub fn to_usage_nonzero_cache_fields(&self) -> Usage {
        usage_from_counts(
            self.input_tokens,
            self.output_tokens,
            self.total_tokens,
            nonzero(self.cache_read_tokens),
            nonzero(self.cache_miss_tokens),
            nonzero(self.cache_write_tokens),
        )
    }
}

pub fn usage_from_counts(
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cache_read_input_tokens: Option<u64>,
    cache_miss_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
) -> Usage {
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_input_tokens,
        cache_miss_input_tokens,
        cache_creation_input_tokens,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalToolDefinition {
    name: String,
    description: Option<String>,
    parameters: Value,
}

pub fn canonical_tool_surface_json(tools: &[ToolDefinition]) -> String {
    let mut canonical = tools
        .iter()
        .map(|tool| CanonicalToolDefinition {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: stable_json_value(&tool.parameters),
        })
        .collect::<Vec<_>>();
    canonical.sort_by(|a, b| cache_tool_order_key(&a.name).cmp(&cache_tool_order_key(&b.name)));
    serde_json::to_string(&canonical).unwrap_or_else(|_| "[]".to_string())
}

pub fn tool_surface_fingerprint(tools: &[ToolDefinition]) -> String {
    let canonical = canonical_tool_surface_json(tools);
    sha256_hex(canonical.as_bytes())
}

pub fn message_prefix_fingerprint(messages: &[Message]) -> String {
    serializable_fingerprint(messages)
}

pub fn json_fingerprint(value: &Value) -> String {
    let canonical = stable_json_string(value);
    sha256_hex(canonical.as_bytes())
}

pub fn text_fingerprint(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

pub fn serializable_fingerprint<T: Serialize + ?Sized>(value: &T) -> String {
    let json = serde_json::to_value(value).unwrap_or(Value::Null);
    json_fingerprint(&json)
}

pub fn stable_json_string(value: &Value) -> String {
    serde_json::to_string(&stable_json_value(value)).unwrap_or_else(|_| "null".to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn cache_tool_order_key(name: &str) -> (u8, &str) {
    match name {
        "task_flow" => (0, name),
        "task" => (1, name),
        "skills_categories" => (2, name),
        "skills_list" => (3, name),
        "skill_view" => (4, name),
        "skill" => (5, name),
        "skill_manage" => (6, name),
        _ => (7, name),
    }
}

fn stable_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(stable_json_value).collect()),
        Value::Object(map) => {
            let sorted = map
                .iter()
                .map(|(key, value)| (key.clone(), stable_json_value(value)))
                .collect::<BTreeMap<_, _>>();
            let mut stable = serde_json::Map::new();
            for (key, value) in sorted {
                stable.insert(key, value);
            }
            Value::Object(stable)
        }
        _ => value.clone(),
    }
}

fn read_u64_any(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn read_nested_u64_any(value: &Value, paths: &[&[&str]]) -> u64 {
    paths
        .iter()
        .find_map(|path| {
            let mut current = value;
            for key in *path {
                current = current.get(*key)?;
            }
            current.as_u64()
        })
        .unwrap_or(0)
}

fn nonzero(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_closeai_cached_tokens_shape() {
        let metrics = TokenUsageMetrics::from_value(&serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 50,
            "prompt_tokens_details": {
                "cached_tokens": 900
            }
        }));

        assert_eq!(metrics.input_tokens, 1000);
        assert_eq!(metrics.output_tokens, 50);
        assert_eq!(metrics.cache_read_tokens, 900);
        assert_eq!(metrics.cache_miss_tokens, 0);
        assert_eq!(metrics.context_tokens, 1000);
    }

    #[test]
    fn extracts_closeai_responses_cached_tokens_shape() {
        let metrics = TokenUsageMetrics::from_value(&serde_json::json!({
            "input_tokens": 1000,
            "output_tokens": 50,
            "input_tokens_details": {
                "cached_tokens": 800
            }
        }));

        assert_eq!(metrics.input_tokens, 1000);
        assert_eq!(metrics.output_tokens, 50);
        assert_eq!(metrics.cache_read_tokens, 800);
        assert_eq!(metrics.cache_miss_tokens, 0);
        assert_eq!(metrics.context_tokens, 1000);
    }

    #[test]
    fn extracts_ethnopic_read_write_shape() {
        let metrics = TokenUsageMetrics::from_value(&serde_json::json!({
            "input_tokens": 1200,
            "output_tokens": 80,
            "cache_read_input_tokens": 1000,
            "cache_creation_input_tokens": 200
        }));

        assert_eq!(metrics.input_tokens, 1200);
        assert_eq!(metrics.output_tokens, 80);
        assert_eq!(metrics.cache_read_tokens, 1000);
        assert_eq!(metrics.cache_write_tokens, 200);
        assert_eq!(metrics.context_tokens, 1200);
    }

    #[test]
    fn canonical_tool_surface_sorts_tools_and_schema_keys() {
        let first = vec![
            ToolDefinition {
                name: "websearch".to_string(),
                description: Some("search".to_string()),
                parameters: serde_json::from_str(r#"{"z":1,"a":{"b":2,"a":1}}"#).unwrap(),
            },
            ToolDefinition {
                name: "task".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];
        let second = vec![
            ToolDefinition {
                name: "task".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "websearch".to_string(),
                description: Some("search".to_string()),
                parameters: serde_json::from_str(r#"{"a":{"a":1,"b":2},"z":1}"#).unwrap(),
            },
        ];

        assert_eq!(
            canonical_tool_surface_json(&first),
            canonical_tool_surface_json(&second)
        );
        assert_eq!(
            tool_surface_fingerprint(&first),
            tool_surface_fingerprint(&second)
        );
    }

    #[test]
    fn json_fingerprint_is_stable_for_object_key_order() {
        let first: Value = serde_json::from_str(r#"{"z":1,"a":{"y":2,"b":3}}"#).unwrap();
        let second: Value = serde_json::from_str(r#"{"a":{"b":3,"y":2},"z":1}"#).unwrap();

        assert_eq!(stable_json_string(&first), stable_json_string(&second));
        assert_eq!(json_fingerprint(&first), json_fingerprint(&second));
    }

    #[test]
    fn prompt_surface_fingerprint_tracks_message_prefix() {
        let tools = vec![ToolDefinition {
            name: "task".to_string(),
            description: None,
            parameters: serde_json::json!({}),
        }];
        let api_params = serde_json::json!({"temperature": 0});
        let first = PromptSurfaceFingerprint::new(
            "model-a",
            Some("system"),
            &tools,
            &[Message::user("hello")],
            &api_params,
        );
        let second = PromptSurfaceFingerprint::new(
            "model-a",
            Some("system"),
            &tools,
            &[Message::user("hello")],
            &api_params,
        );
        let changed = PromptSurfaceFingerprint::new(
            "model-a",
            Some("system"),
            &tools,
            &[Message::user("hello again")],
            &api_params,
        );

        assert_eq!(first, second);
        assert_ne!(first.message_prefix_hash, changed.message_prefix_hash);
    }
}
