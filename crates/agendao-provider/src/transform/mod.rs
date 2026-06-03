mod model_config;
mod normalize;
mod options;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// Re-export all public items to maintain backward compatibility
pub use model_config::{
    apply_caching_per_part, ensure_noop_tool_if_needed, max_output_tokens,
    normalize_interleaved_thinking, sdk_key, variants,
};
pub use normalize::{
    apply_caching, apply_caching_with_policy, apply_interleaved_thinking, dedup_messages,
    extract_reasoning_from_response, mime_to_modality, normalize_messages,
    normalize_messages_for_caching, normalize_messages_with_interleaved_field,
    temperature_for_model, top_k_for_model, top_p_for_model, transform_messages, unsupported_parts,
    Modality, ProviderType, ReasoningContent, OUTPUT_TOKEN_MAX,
};
pub use options::{options, provider_options_map, schema, small_options};

#[cfg(test)]
use normalize::{normalize_tool_call_id, normalize_tool_call_id_mistral};
