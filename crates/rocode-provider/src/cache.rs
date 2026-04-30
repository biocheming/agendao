use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::{Message, ToolDefinition, Usage};

pub const CACHE_REQUEST_FINGERPRINT_METADATA_KEY: &str = "cache_request_fingerprint";
pub const CACHE_BUST_INSPECTION_METADATA_KEY: &str = "cache_bust_inspection";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheBreakpointBudget {
    pub max_breakpoints: usize,
    pub used_by_system: usize,
    pub used_by_tools: usize,
    pub used_by_messages: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointCandidateKind {
    SystemBlock,
    ToolSchema,
    MessageBoundary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakpointCandidate {
    pub message_index: usize,
    pub kind: BreakpointCandidateKind,
    pub stable_score: f32,
    pub token_count: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheBreakpointPlan {
    pub candidates: Vec<BreakpointCandidate>,
    pub budget: CacheBreakpointBudget,
}

impl CacheBreakpointPlan {
    pub fn message_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.candidates
            .iter()
            .map(|candidate| candidate.message_index)
    }
}

pub fn plan_ethnopic_message_breakpoints(messages: &[Message]) -> CacheBreakpointPlan {
    let max_breakpoints = 4;
    let mut candidates = Vec::new();

    for index in messages
        .iter()
        .enumerate()
        .filter(|(_, message)| matches!(message.role, crate::Role::System))
        .map(|(index, _)| index)
        .take(2)
    {
        candidates.push(BreakpointCandidate {
            message_index: index,
            kind: BreakpointCandidateKind::SystemBlock,
            stable_score: 1.0,
            token_count: 0,
            reason: "stable system prompt".to_string(),
        });
    }

    if let Some(index) = stable_conversation_cache_boundary_index(messages) {
        if candidates.len() < max_breakpoints
            && !candidates
                .iter()
                .any(|candidate| candidate.message_index == index)
        {
            candidates.push(BreakpointCandidate {
                message_index: index,
                kind: BreakpointCandidateKind::MessageBoundary,
                stable_score: 0.8,
                token_count: 0,
                reason: "last stable conversation message before dynamic suffix".to_string(),
            });
        }
    }

    let used_by_system = candidates
        .iter()
        .filter(|candidate| candidate.kind == BreakpointCandidateKind::SystemBlock)
        .count();
    let used_by_messages = candidates
        .iter()
        .filter(|candidate| candidate.kind == BreakpointCandidateKind::MessageBoundary)
        .count();

    CacheBreakpointPlan {
        candidates,
        budget: CacheBreakpointBudget {
            max_breakpoints,
            used_by_system,
            used_by_tools: 0,
            used_by_messages,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseAiPromptCacheKeyField {
    CamelCase,
    SnakeCase,
}

impl CloseAiPromptCacheKeyField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CamelCase => "promptCacheKey",
            Self::SnakeCase => "prompt_cache_key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCacheKeyContext<'a> {
    pub session_id: &'a str,
    pub stage: &'a str,
    pub preset_hash: Option<&'a str>,
    pub repo_hash: Option<&'a str>,
}

pub fn build_prompt_cache_key(ctx: PromptCacheKeyContext<'_>) -> String {
    let session_hash = short_hash(ctx.session_id);
    let stage = stable_key_segment(ctx.stage).unwrap_or("chat");
    let preset = ctx
        .preset_hash
        .and_then(stable_key_segment)
        .unwrap_or("default");
    let repo = ctx
        .repo_hash
        .and_then(stable_key_segment)
        .unwrap_or("no-repo");
    format!("rocode:{session_hash}:{stage}:{preset}:{repo}")
}

pub fn closeai_prompt_cache_key_field(
    provider_id: &str,
    npm: &str,
    provider_options: &serde_json::Map<String, Value>,
) -> Option<CloseAiPromptCacheKeyField> {
    let provider_id = provider_id.trim().to_ascii_lowercase();
    let npm = npm.trim();
    let explicit = provider_options
        .get("setCacheKey")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if explicit
        || provider_id == "openai"
        || npm == "@ai-sdk/openai"
        || provider_id.starts_with("opencode")
        || provider_id == "venice"
    {
        return Some(CloseAiPromptCacheKeyField::CamelCase);
    }

    if provider_id == "openrouter" || npm == "@openrouter/ai-sdk-provider" {
        return Some(CloseAiPromptCacheKeyField::SnakeCase);
    }

    None
}

fn stable_conversation_cache_boundary_index(messages: &[Message]) -> Option<usize> {
    let last_index = messages.len().checked_sub(1)?;
    let boundary = if matches!(messages[last_index].role, crate::Role::User) {
        last_index.checked_sub(1)?
    } else {
        last_index
    };
    (!matches!(messages[boundary].role, crate::Role::System)).then_some(boundary)
}

fn push_diff<T: PartialEq>(
    changes: &mut Vec<CacheFingerprintDiff>,
    field: &str,
    previous: T,
    current: T,
    severity: CacheBustSeverity,
    reason: &str,
) {
    if previous != current {
        changes.push(CacheFingerprintDiff {
            field: field.to_string(),
            severity,
            reason: reason.to_string(),
        });
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseAiCacheFingerprint {
    pub prompt_cache_key: Option<String>,
    pub prompt_cache_retention: Option<String>,
    pub previous_response_id_used: bool,
    pub incremental_input_used: bool,
    pub cached_tokens_observed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthnopicCacheFingerprint {
    pub cache_control_hash: String,
    pub breakpoint_placement: Vec<usize>,
    pub ttl: Option<String>,
    pub scope: Option<String>,
    pub cache_read_observed: u64,
    pub cache_write_observed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRequestFingerprint {
    pub family: CacheProtocolFamily,
    pub surface: PromptSurfaceFingerprint,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closeai: Option<CloseAiCacheFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ethnopic: Option<EthnopicCacheFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CacheBustSeverity {
    Stable,
    SoftDegradation,
    LikelyBust,
    HardBust,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheFingerprintDiff {
    pub field: String,
    pub severity: CacheBustSeverity,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheBustInspection {
    pub status: String,
    pub severity: CacheBustSeverity,
    pub primary_cause: Option<String>,
    pub changes: Vec<CacheFingerprintDiff>,
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

pub fn inspect_cache_fingerprint_change(
    previous: Option<&CacheRequestFingerprint>,
    current: &CacheRequestFingerprint,
) -> CacheBustInspection {
    let Some(previous) = previous else {
        return CacheBustInspection {
            status: "cold_start".to_string(),
            severity: CacheBustSeverity::SoftDegradation,
            primary_cause: Some("no previous cache fingerprint".to_string()),
            changes: vec![CacheFingerprintDiff {
                field: "previousFingerprint".to_string(),
                severity: CacheBustSeverity::SoftDegradation,
                reason: "no previous request fingerprint is available for comparison".to_string(),
            }],
        };
    };

    let mut changes = Vec::new();
    push_diff(
        &mut changes,
        "family",
        previous.family,
        current.family,
        CacheBustSeverity::HardBust,
        "protocol family changed",
    );
    push_diff(
        &mut changes,
        "model",
        previous.surface.model.as_str(),
        current.surface.model.as_str(),
        CacheBustSeverity::HardBust,
        "model changed",
    );
    push_diff(
        &mut changes,
        "systemHash",
        previous.surface.system_hash.as_str(),
        current.surface.system_hash.as_str(),
        CacheBustSeverity::HardBust,
        "system prompt changed",
    );
    push_diff(
        &mut changes,
        "toolsHash",
        previous.surface.tools_hash.as_str(),
        current.surface.tools_hash.as_str(),
        CacheBustSeverity::HardBust,
        "tool schema or order changed",
    );
    push_diff(
        &mut changes,
        "apiParamsHash",
        previous.surface.api_params_hash.as_str(),
        current.surface.api_params_hash.as_str(),
        CacheBustSeverity::HardBust,
        "cache-key-sensitive API params changed",
    );
    push_diff(
        &mut changes,
        "messagePrefixHash",
        previous.surface.message_prefix_hash.as_str(),
        current.surface.message_prefix_hash.as_str(),
        CacheBustSeverity::LikelyBust,
        "message prefix changed before the stable boundary",
    );
    inspect_closeai_fingerprint(previous, current, &mut changes);
    inspect_ethnopic_fingerprint(previous, current, &mut changes);

    let severity = changes
        .iter()
        .map(|change| change.severity)
        .max()
        .unwrap_or(CacheBustSeverity::Stable);
    let primary_cause = changes
        .iter()
        .max_by_key(|change| change.severity)
        .map(|change| format!("{} changed: {}", change.field, change.reason));

    CacheBustInspection {
        status: if severity == CacheBustSeverity::Stable {
            "stable".to_string()
        } else {
            "degraded".to_string()
        },
        severity,
        primary_cause,
        changes,
    }
}

fn inspect_closeai_fingerprint(
    previous: &CacheRequestFingerprint,
    current: &CacheRequestFingerprint,
    changes: &mut Vec<CacheFingerprintDiff>,
) {
    let (Some(previous), Some(current)) = (previous.closeai.as_ref(), current.closeai.as_ref())
    else {
        return;
    };
    push_diff(
        changes,
        "promptCacheKey",
        previous.prompt_cache_key.as_deref(),
        current.prompt_cache_key.as_deref(),
        CacheBustSeverity::LikelyBust,
        "prompt cache affinity key changed",
    );
    push_diff(
        changes,
        "promptCacheRetention",
        previous.prompt_cache_retention.as_deref(),
        current.prompt_cache_retention.as_deref(),
        CacheBustSeverity::SoftDegradation,
        "prompt cache retention changed",
    );
    if previous.previous_response_id_used && !current.previous_response_id_used {
        changes.push(CacheFingerprintDiff {
            field: "previousResponseIdUsed".to_string(),
            severity: CacheBustSeverity::LikelyBust,
            reason: "Responses continuation was not used on the current request".to_string(),
        });
    }
    if previous.incremental_input_used && !current.incremental_input_used {
        changes.push(CacheFingerprintDiff {
            field: "incrementalInputUsed".to_string(),
            severity: CacheBustSeverity::SoftDegradation,
            reason: "incremental Responses input was not used on the current request".to_string(),
        });
    }
}

fn inspect_ethnopic_fingerprint(
    previous: &CacheRequestFingerprint,
    current: &CacheRequestFingerprint,
    changes: &mut Vec<CacheFingerprintDiff>,
) {
    let (Some(previous), Some(current)) = (previous.ethnopic.as_ref(), current.ethnopic.as_ref())
    else {
        return;
    };
    push_diff(
        changes,
        "cacheControlHash",
        previous.cache_control_hash.as_str(),
        current.cache_control_hash.as_str(),
        CacheBustSeverity::HardBust,
        "cache_control shape changed",
    );
    push_diff(
        changes,
        "breakpointPlacement",
        previous.breakpoint_placement.as_slice(),
        current.breakpoint_placement.as_slice(),
        CacheBustSeverity::LikelyBust,
        "cache breakpoint placement changed",
    );
    push_diff(
        changes,
        "ttl",
        previous.ttl.as_deref(),
        current.ttl.as_deref(),
        CacheBustSeverity::SoftDegradation,
        "cache ttl changed",
    );
    push_diff(
        changes,
        "scope",
        previous.scope.as_deref(),
        current.scope.as_deref(),
        CacheBustSeverity::SoftDegradation,
        "cache scope changed",
    );
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

fn short_hash(value: &str) -> String {
    text_fingerprint(value).chars().take(16).collect()
}

fn stable_key_segment(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        .then_some(trimmed)
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
    fn prompt_cache_key_hashes_session_identity() {
        let key = build_prompt_cache_key(PromptCacheKeyContext {
            session_id: "session-with-local-detail",
            stage: "exec",
            preset_hash: Some("preset_123"),
            repo_hash: Some("repo_456"),
        });

        assert!(key.starts_with("rocode:"));
        assert!(key.contains(":exec:preset_123:repo_456"));
        assert!(!key.contains("session-with-local-detail"));
    }

    #[test]
    fn prompt_cache_key_field_is_capability_gated() {
        assert_eq!(
            closeai_prompt_cache_key_field("openai", "@ai-sdk/openai", &serde_json::Map::new()),
            Some(CloseAiPromptCacheKeyField::CamelCase)
        );
        assert_eq!(
            closeai_prompt_cache_key_field(
                "openrouter",
                "@openrouter/ai-sdk-provider",
                &serde_json::Map::new()
            ),
            Some(CloseAiPromptCacheKeyField::SnakeCase)
        );
        assert_eq!(
            closeai_prompt_cache_key_field(
                "deepseek",
                "@ai-sdk/openai-compatible",
                &serde_json::Map::new()
            ),
            None
        );
    }

    #[test]
    fn ethnopic_breakpoint_plan_uses_system_and_stable_boundary() {
        let messages = vec![
            Message::system("system"),
            Message::user("first"),
            Message::assistant("answer"),
            Message::user("current"),
        ];

        let plan = plan_ethnopic_message_breakpoints(&messages);
        let indices = plan.message_indices().collect::<Vec<_>>();

        assert_eq!(indices, vec![0, 2]);
        assert_eq!(plan.budget.used_by_system, 1);
        assert_eq!(plan.budget.used_by_messages, 1);
        assert!(plan.budget.max_breakpoints >= indices.len());
    }

    #[test]
    fn ethnopic_breakpoint_plan_does_not_mark_current_user() {
        let messages = vec![Message::system("system"), Message::user("current")];

        let plan = plan_ethnopic_message_breakpoints(&messages);
        let indices = plan.message_indices().collect::<Vec<_>>();

        assert_eq!(indices, vec![0]);
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

    #[test]
    fn cache_bust_inspector_marks_tools_change_as_hard_bust() {
        let previous = test_cache_request_fingerprint("tools-a", "messages-a");
        let mut current = previous.clone();
        current.surface.tools_hash = "tools-b".to_string();

        let inspection = inspect_cache_fingerprint_change(Some(&previous), &current);

        assert_eq!(inspection.status, "degraded");
        assert_eq!(inspection.severity, CacheBustSeverity::HardBust);
        assert!(inspection
            .primary_cause
            .as_deref()
            .is_some_and(|cause| cause.contains("toolsHash")));
    }

    #[test]
    fn cache_bust_inspector_marks_message_change_as_likely_bust() {
        let previous = test_cache_request_fingerprint("tools-a", "messages-a");
        let mut current = previous.clone();
        current.surface.message_prefix_hash = "messages-b".to_string();

        let inspection = inspect_cache_fingerprint_change(Some(&previous), &current);

        assert_eq!(inspection.status, "degraded");
        assert_eq!(inspection.severity, CacheBustSeverity::LikelyBust);
        assert_eq!(inspection.changes[0].field, "messagePrefixHash");
    }

    #[test]
    fn cache_bust_inspector_marks_closeai_key_change_as_likely_bust() {
        let previous = test_closeai_cache_request_fingerprint(Some("key-a"));
        let current = test_closeai_cache_request_fingerprint(Some("key-b"));

        let inspection = inspect_cache_fingerprint_change(Some(&previous), &current);

        assert_eq!(inspection.severity, CacheBustSeverity::LikelyBust);
        assert!(inspection
            .changes
            .iter()
            .any(|change| change.field == "promptCacheKey"));
    }

    #[test]
    fn cache_bust_inspector_marks_ethnopic_cache_control_change_as_hard_bust() {
        let previous = test_ethnopic_cache_request_fingerprint("cache-a", vec![0, 2]);
        let current = test_ethnopic_cache_request_fingerprint("cache-b", vec![0, 2]);

        let inspection = inspect_cache_fingerprint_change(Some(&previous), &current);

        assert_eq!(inspection.severity, CacheBustSeverity::HardBust);
        assert!(inspection
            .changes
            .iter()
            .any(|change| change.field == "cacheControlHash"));
    }

    #[test]
    fn cache_bust_inspector_reports_cold_start() {
        let current = test_cache_request_fingerprint("tools-a", "messages-a");

        let inspection = inspect_cache_fingerprint_change(None, &current);

        assert_eq!(inspection.status, "cold_start");
        assert_eq!(inspection.severity, CacheBustSeverity::SoftDegradation);
    }

    fn test_cache_request_fingerprint(
        tools_hash: &str,
        message_prefix_hash: &str,
    ) -> CacheRequestFingerprint {
        CacheRequestFingerprint {
            family: CacheProtocolFamily::CloseAiCompatible,
            surface: PromptSurfaceFingerprint {
                model: "model-a".to_string(),
                system_hash: "system-a".to_string(),
                tools_hash: tools_hash.to_string(),
                message_prefix_hash: message_prefix_hash.to_string(),
                api_params_hash: "params-a".to_string(),
            },
            closeai: None,
            ethnopic: None,
        }
    }

    fn test_closeai_cache_request_fingerprint(
        prompt_cache_key: Option<&str>,
    ) -> CacheRequestFingerprint {
        let mut fingerprint = test_cache_request_fingerprint("tools-a", "messages-a");
        fingerprint.closeai = Some(CloseAiCacheFingerprint {
            prompt_cache_key: prompt_cache_key.map(ToString::to_string),
            prompt_cache_retention: Some("in_memory".to_string()),
            previous_response_id_used: true,
            incremental_input_used: true,
            cached_tokens_observed: 1024,
        });
        fingerprint
    }

    fn test_ethnopic_cache_request_fingerprint(
        cache_control_hash: &str,
        breakpoint_placement: Vec<usize>,
    ) -> CacheRequestFingerprint {
        let mut fingerprint = test_cache_request_fingerprint("tools-a", "messages-a");
        fingerprint.family = CacheProtocolFamily::EthnopicCompatible;
        fingerprint.ethnopic = Some(EthnopicCacheFingerprint {
            cache_control_hash: cache_control_hash.to_string(),
            breakpoint_placement,
            ttl: None,
            scope: None,
            cache_read_observed: 1024,
            cache_write_observed: 128,
        });
        fingerprint
    }
}
