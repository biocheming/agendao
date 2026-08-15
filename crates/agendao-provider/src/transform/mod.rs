mod model_config;
mod normalize;
mod options;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use model_config::{
    apply_caching_per_part, max_output_tokens, normalize_interleaved_thinking, sdk_key, variants,
};
pub use normalize::{
    apply_caching, apply_caching_with_policy, apply_interleaved_thinking, dedup_messages,
    extract_reasoning_from_response, mime_to_modality, normalize_messages,
    normalize_messages_for_caching, normalize_messages_with_interleaved_field, transform_messages,
    unsupported_parts, Modality, ProviderType, ReasoningContent, OUTPUT_TOKEN_MAX,
};
pub use options::options;

#[cfg(test)]
use normalize::normalize_tool_call_id;
