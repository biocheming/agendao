use std::collections::HashMap;

use crate::cache::plan_anthropic_message_breakpoints;
use crate::models;
use crate::{CacheControl, Content, ContentPart, Message};

use super::normalize::{ProviderType, OUTPUT_TOKEN_MAX, WIDELY_SUPPORTED_EFFORTS};

macro_rules! hashmap {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map = HashMap::new();
        $(map.insert($key.to_string(), $value);)*
        map
    }};
}

/// Remap providerOptions keys from the stored `provider_id` to the expected SDK key.
/// Matches the TS logic that remaps `providerOptions[providerID]` -> `providerOptions[sdkKey]`.
pub(super) fn remap_provider_options(messages: &mut [Message], npm: &str, provider_id: &str) {
    let key = match sdk_key(npm) {
        Some(k) => k,
        None => return,
    };

    // Skip if the key already matches the provider_id.
    if key == provider_id {
        return;
    }

    let remap = |opts: &mut Option<HashMap<String, serde_json::Value>>| {
        let map = match opts.as_mut() {
            Some(m) => m,
            None => return,
        };
        if let Some(val) = map.remove(provider_id) {
            map.insert(key.to_string(), val);
        }
    };

    for msg in messages.iter_mut() {
        remap(&mut msg.provider_options);
        if let Content::Parts(parts) = &mut msg.content {
            for part in parts.iter_mut() {
                remap(&mut part.provider_options);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// normalize_interleaved_thinking
// ---------------------------------------------------------------------------

/// Normalize interleaved thinking content in messages.
/// For providers that don't support interleaved thinking, strip thinking blocks
/// from all but the last assistant message.
pub fn normalize_interleaved_thinking(
    messages: &mut [Message],
    _provider_type: &ProviderType,
    supports_interleaved: bool,
) {
    if supports_interleaved {
        return;
    }

    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, crate::Role::Assistant));

    for (idx, message) in messages.iter_mut().enumerate() {
        if !matches!(message.role, crate::Role::Assistant) {
            continue;
        }
        if Some(idx) == last_assistant_idx {
            continue;
        }

        if let Content::Parts(ref mut parts) = message.content {
            parts
                .retain(|part| part.content_type != "thinking" && part.content_type != "reasoning");

            if parts.is_empty() {
                parts.push(ContentPart {
                    content_type: "text".to_string(),
                    text: Some("[thinking]".to_string()),
                    ..Default::default()
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// apply_caching_per_part
// ---------------------------------------------------------------------------

/// Apply cache control markers at the part level.
pub fn apply_caching_per_part(messages: &mut [Message], provider_type: &ProviderType) {
    if let ProviderType::Anthropic = provider_type {
        let plan = plan_anthropic_message_breakpoints(messages);
        for boundary_index in plan.message_indices() {
            let Some(boundary) = messages.get_mut(boundary_index) else {
                continue;
            };
            if let Content::Parts(ref mut parts) = boundary.content {
                if let Some(last_part) = parts.last_mut() {
                    last_part.cache_control = Some(CacheControl::ephemeral());
                }
            }
            boundary.cache_control = Some(CacheControl::ephemeral());
        }
    }
}

// ---------------------------------------------------------------------------
// max_output_tokens
// ---------------------------------------------------------------------------

/// Get the maximum output tokens for a model, capped at OUTPUT_TOKEN_MAX.
pub fn max_output_tokens(model: &models::ModelInfo) -> u64 {
    let capped = model.limit.output.min(OUTPUT_TOKEN_MAX);
    if capped == 0 {
        OUTPUT_TOKEN_MAX
    } else {
        capped
    }
}

// ---------------------------------------------------------------------------
// sdk_key
// ---------------------------------------------------------------------------

/// Map npm package name to SDK key.
pub fn sdk_key(npm: &str) -> Option<&'static str> {
    match npm {
        "@ai-sdk/openai" | "@ai-sdk/openai-compatible" => Some("openai"),
        "@ai-sdk/anthropic" => Some("anthropic"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// variants
// ---------------------------------------------------------------------------

/// Generate reasoning/thinking configuration variants for a model.
/// Returns a map of variant_name -> config options.
pub fn variants(model: &models::ModelInfo) -> HashMap<String, HashMap<String, serde_json::Value>> {
    use serde_json::json;

    if !model.reasoning {
        return HashMap::new();
    }

    let id = model.id.to_lowercase();

    let npm = model
        .provider
        .as_ref()
        .and_then(|p| p.npm.as_deref())
        .unwrap_or("");
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");

    match npm {
        "@ai-sdk/openai-compatible" => WIDELY_SUPPORTED_EFFORTS
            .iter()
            .map(|e| (e.to_string(), hashmap! {"reasoningEffort" => json!(*e)}))
            .collect(),

        "@ai-sdk/openai" => {
            if id == "gpt-5-pro" {
                return HashMap::new();
            }
            let efforts: Vec<&str> = if id.contains("codex") {
                if id.contains("5.2") || id.contains("5.3") {
                    vec!["low", "medium", "high", "xhigh"]
                } else {
                    vec!["low", "medium", "high"]
                }
            } else {
                let mut arr: Vec<&str> = vec!["low", "medium", "high"];
                if id.contains("gpt-5-") || id == "gpt-5" {
                    arr.insert(0, "minimal");
                }
                // Check release_date for additional efforts
                let release_date = model.release_date.as_deref().unwrap_or("");
                if release_date >= "2025-11-13" {
                    arr.insert(0, "none");
                }
                if release_date >= "2025-12-04" {
                    arr.push("xhigh");
                }
                arr
            };
            efforts
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {
                            "reasoningEffort" => json!(*e),
                            "reasoningSummary" => json!("auto"),
                            "include" => json!(["reasoning.encrypted_content"])
                        },
                    )
                })
                .collect()
        }

        "@ai-sdk/anthropic" => {
            if api_id.contains("opus-4-6") || api_id.contains("opus-4.6") {
                return ["low", "medium", "high", "max"]
                    .iter()
                    .map(|e| {
                        (
                            e.to_string(),
                            hashmap! {
                                "thinking" => json!({"type": "adaptive"}),
                                "effort" => json!(*e)
                            },
                        )
                    })
                    .collect();
            }
            let budget_high = 16_000u64.min(model.limit.output / 2 - 1);
            let budget_max = 31_999u64.min(model.limit.output - 1);
            [
                (
                    "high".into(),
                    hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": budget_high})},
                ),
                (
                    "max".into(),
                    hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": budget_max})},
                ),
            ]
            .into_iter()
            .collect()
        }

        _ => HashMap::new(),
    }
}
