use serde::Deserialize;

use crate::Usage;

#[derive(Debug, Deserialize)]
pub(super) struct RawUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u64>,
    #[serde(default)]
    prompt_cache_miss_tokens: Option<u64>,
}

pub(super) fn raw_usage_to_usage(raw: RawUsage) -> Usage {
    crate::cache::usage_from_counts(
        raw.prompt_tokens.unwrap_or(0),
        raw.completion_tokens.unwrap_or(0),
        raw.total_tokens.unwrap_or(0),
        raw.cache_read_input_tokens.or(raw.prompt_cache_hit_tokens),
        raw.prompt_cache_miss_tokens,
        raw.cache_creation_input_tokens,
    )
}
