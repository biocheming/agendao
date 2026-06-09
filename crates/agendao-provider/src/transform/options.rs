use std::collections::HashMap;

use crate::cache::{build_prompt_cache_key, closeai_prompt_cache_key_field, PromptCacheKeyContext};
use crate::models;

use super::model_config::sdk_key;
use super::normalize::{is_ethnopic_compatible_npm, slug_override};

// ── Pipeline ───────────────────────────────────────────────────────────
//
// options() assembles provider request options in this order:
//
// 1. Provider-family defaults   (driven by npm / provider_id)
// 2. Prompt cache key           (session + preset + repo → cache key)
// 3. Model-family tuning        (api_id patterns: gpt-5, kimi-k2.5, etc.)
// 4. Runtime flags              (per-model: reasoning, template args)
//
// P3.1: provider-family defaults (step 1) are candidates for moving into
// ProviderProfile.request_defaults.  Model-family tuning (step 3) and
// runtime flags (step 4) depend on api_id/model fields and are harder to
// declarativize without per-model config entries.

pub fn options(
    provider_id: &str,
    model: &models::ModelInfo,
    session_id: &str,
    provider_options: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    use serde_json::json;
    let mut result = HashMap::new();

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
    let provider_id = provider_id.to_ascii_lowercase();

    // ── Step 1: Provider-family defaults ────────────────────────────
    // These are driven by npm / provider_id only, not model-specific.
    // Candidates for ProviderProfile.request_defaults in a future refactor.

    apply_openai_family_defaults(&provider_id, npm, &mut result);
    apply_openrouter_family_defaults(npm, &mut result);
    apply_baseten_opencode_defaults(&provider_id, api_id, &mut result);
    apply_zhipuai_thinking_defaults(&provider_id, npm, &mut result);

    // ── Step 2: Prompt cache key ────────────────────────────────────
    // Session + preset + repo → cache key.  Provider-family gating is
    // handled by closeai_prompt_cache_key_field().

    let provider_options_object = provider_options
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    let prompt_cache_key = build_prompt_cache_key(PromptCacheKeyContext {
        session_id,
        stage: provider_options
            .get("cacheStage")
            .and_then(|value| value.as_str())
            .unwrap_or("chat"),
        preset_hash: provider_options
            .get("cachePresetHash")
            .and_then(|value| value.as_str()),
        repo_hash: provider_options
            .get("cacheRepoHash")
            .and_then(|value| value.as_str()),
    });
    if let Some(field) = closeai_prompt_cache_key_field(&provider_id, npm, &provider_options_object)
    {
        result.insert(field.as_str().to_string(), json!(prompt_cache_key.clone()));
    }

    // ── Step 3: Model-family tuning ─────────────────────────────────
    // These match api_id patterns (gpt-5, kimi-k2.5, gemini-3) and
    // depend on the specific model, not just the provider.

    apply_google_thinking_config(npm, api_id, &mut result);
    apply_openrouter_gemini3_reasoning(npm, api_id, &mut result);
    apply_kimi_k2p5_ethnopic_thinking(npm, api_id, model, &mut result);
    apply_gpt5_reasoning_config(&provider_id, api_id, &mut result);

    // ── Step 4: Runtime flags ───────────────────────────────────────
    // Individual model-level flags that don't fit a family pattern.

    apply_alibaba_thinking_flag(&provider_id, model, npm, api_id, &mut result);
    apply_gateway_caching(npm, &mut result);

    result
}

// ── Provider-family helpers ─────────────────────────────────────────────

fn apply_openai_family_defaults(
    provider_id: &str,
    npm: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if provider_id == "openai" || npm == "@ai-sdk/openai" || npm == "@ai-sdk/github-copilot" {
        result.insert("store".to_string(), serde_json::json!(false));
    }
}

fn apply_openrouter_family_defaults(
    npm: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if npm == "@openrouter/ai-sdk-provider" {
        result.insert("usage".to_string(), serde_json::json!({"include": true}));
    }
}

fn apply_baseten_opencode_defaults(
    provider_id: &str,
    api_id: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if provider_id == "baseten"
        || (provider_id.starts_with("opencode")
            && (api_id == "kimi-k2-thinking" || api_id == "glm-4.6"))
    {
        result.insert(
            "chat_template_args".to_string(),
            serde_json::json!({"enable_thinking": true}),
        );
    }
}

fn apply_zhipuai_thinking_defaults(
    provider_id: &str,
    npm: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if (provider_id == "zai" || provider_id == "zhipuai")
        && matches!(
            npm,
            "@ai-sdk/openai-compatible" | "openai-compatible" | "closeai-compatible"
        )
    {
        result.insert(
            "thinking".to_string(),
            serde_json::json!({"type": "enabled", "clear_thinking": false}),
        );
    }
}

// ── Model-family helpers ────────────────────────────────────────────────

fn apply_google_thinking_config(
    npm: &str,
    api_id: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if npm != "@ai-sdk/google" && npm != "@ai-sdk/google-vertex" {
        return;
    }
    let mut thinking = serde_json::json!({"includeThoughts": true});
    if api_id.contains("gemini-3") {
        thinking["thinkingLevel"] = serde_json::json!("high");
    }
    result.insert("thinkingConfig".to_string(), thinking);
}

fn apply_openrouter_gemini3_reasoning(
    npm: &str,
    api_id: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if npm == "@openrouter/ai-sdk-provider" && api_id.contains("gemini-3") {
        result.insert("reasoning".to_string(), serde_json::json!({"effort": "high"}));
    }
}

fn apply_kimi_k2p5_ethnopic_thinking(
    npm: &str,
    api_id: &str,
    model: &models::ModelInfo,
    result: &mut HashMap<String, serde_json::Value>,
) {
    let api_id_lower = api_id.to_lowercase();
    if !is_ethnopic_compatible_npm(npm)
        || !(api_id_lower.contains("k2p5")
            || api_id_lower.contains("kimi-k2.5")
            || api_id_lower.contains("kimi-k2p5"))
    {
        return;
    }
    let budget = 16_000u64.min(model.limit.output / 2 - 1);
    result.insert(
        "thinking".to_string(),
        serde_json::json!({"type": "enabled", "budgetTokens": budget}),
    );
}

fn apply_gpt5_reasoning_config(
    provider_id: &str,
    api_id: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if !api_id.contains("gpt-5") || api_id.contains("gpt-5-chat") {
        return;
    }
    if !api_id.contains("gpt-5-pro") {
        result.insert("reasoningEffort".to_string(), serde_json::json!("medium"));
        result.insert("reasoningSummary".to_string(), serde_json::json!("auto"));
    }
    if api_id.contains("gpt-5.") && !api_id.contains("codex") && !api_id.contains("-chat") {
        result.insert("textVerbosity".to_string(), serde_json::json!("low"));
    }
    if provider_id.starts_with("opencode") {
        result.insert(
            "include".to_string(),
            serde_json::json!(["reasoning.encrypted_content"]),
        );
        result.insert("reasoningSummary".to_string(), serde_json::json!("auto"));
    }
}

// ── Runtime flags ───────────────────────────────────────────────────────

fn apply_alibaba_thinking_flag(
    provider_id: &str,
    model: &models::ModelInfo,
    npm: &str,
    api_id: &str,
    result: &mut HashMap<String, serde_json::Value>,
) {
    if provider_id == "alibaba-cn"
        && model.reasoning
        && npm == "@ai-sdk/openai-compatible"
        && !api_id.to_lowercase().contains("kimi-k2-thinking")
    {
        result.insert("enable_thinking".to_string(), serde_json::json!(true));
    }
}

fn apply_gateway_caching(npm: &str, result: &mut HashMap<String, serde_json::Value>) {
    if npm == "@ai-sdk/gateway" {
        result.insert("gateway".to_string(), serde_json::json!({"caching": "auto"}));
    }
}

// ── small_options ───────────────────────────────────────────────────────
// (unchanged from original; model-specific branching is inherent here)

/// Generate small model options (reduced reasoning effort).
pub fn small_options(model: &models::ModelInfo) -> HashMap<String, serde_json::Value> {
    use serde_json::json;
    let mut result = HashMap::new();

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
    let provider_id = model.id.to_lowercase();

    if provider_id == "openai" || npm == "@ai-sdk/openai" || npm == "@ai-sdk/github-copilot" {
        result.insert("store".to_string(), json!(false));
        if api_id.contains("gpt-5") {
            if api_id.contains("5.") {
                result.insert("reasoningEffort".to_string(), json!("low"));
            } else {
                result.insert("reasoningEffort".to_string(), json!("minimal"));
            }
        }
        return result;
    }

    if provider_id == "google" {
        if api_id.contains("gemini-3") {
            result.insert(
                "thinkingConfig".to_string(),
                json!({"thinkingLevel": "minimal"}),
            );
        } else {
            result.insert("thinkingConfig".to_string(), json!({"thinkingBudget": 0}));
        }
        return result;
    }

    if provider_id == "openrouter" {
        if api_id.contains("google") {
            result.insert("reasoning".to_string(), json!({"enabled": false}));
        } else {
            result.insert("reasoningEffort".to_string(), json!("minimal"));
        }
        return result;
    }

    result
}

// ── schema (Gemini schema sanitization) ─────────────────────────────────
// (unchanged)

/// Sanitize a JSON schema for Gemini/Google models.
/// - Convert integer enums to string enums
/// - Recursive sanitization of nested objects/arrays
/// - Filter required array to only include fields in properties
/// - Remove properties/required from non-object types
/// - Handle empty array items
pub fn schema(model: &models::ModelInfo, input_schema: serde_json::Value) -> serde_json::Value {
    let provider_id = model.id.to_lowercase();
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");

    if provider_id == "google" || api_id.contains("gemini") {
        sanitize_gemini(input_schema)
    } else {
        input_schema
    }
}

fn sanitize_gemini(obj: serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Map, Value};

    match obj {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => obj,
        Value::Array(arr) => Value::Array(arr.into_iter().map(sanitize_gemini).collect()),
        Value::Object(map) => {
            let mut result = Map::new();

            for (key, value) in map {
                if key == "enum" {
                    if let Value::Array(ref enum_vals) = value {
                        let string_vals: Vec<Value> = enum_vals
                            .iter()
                            .map(|v| match v {
                                Value::String(s) => Value::String(s.clone()),
                                other => Value::String(other.to_string()),
                            })
                            .collect();
                        result.insert(key, Value::Array(string_vals));

                        if let Some(Value::String(t)) = result.get("type") {
                            if t == "integer" || t == "number" {
                                result.insert(
                                    "type".to_string(),
                                    Value::String("string".to_string()),
                                );
                            }
                        }
                    } else {
                        result.insert(key, value);
                    }
                } else if value.is_object() || value.is_array() {
                    result.insert(key, sanitize_gemini(value));
                } else {
                    result.insert(key, value);
                }
            }

            if let Some(Value::Array(ref enum_vals)) = result.get("enum") {
                if !enum_vals.is_empty() {
                    if let Some(Value::String(t)) = result.get("type") {
                        if t == "integer" || t == "number" {
                            result.insert("type".to_string(), Value::String("string".to_string()));
                        }
                    }
                }
            }

            if result.get("type") == Some(&json!("object")) {
                if let (Some(Value::Object(ref props)), Some(Value::Array(ref required))) =
                    (result.get("properties"), result.get("required"))
                {
                    let filtered: Vec<Value> = required
                        .iter()
                        .filter(|r| {
                            if let Value::String(field) = r {
                                props.contains_key(field)
                            } else {
                                false
                            }
                        })
                        .cloned()
                        .collect();
                    result.insert("required".to_string(), Value::Array(filtered));
                }
            }

            if result.get("type") == Some(&json!("array")) {
                if !result.contains_key("items") || result.get("items") == Some(&Value::Null) {
                    result.insert("items".to_string(), json!({}));
                }
                if let Some(Value::Object(ref mut items)) = result.get_mut("items") {
                    if !items.contains_key("type") {
                        items.insert("type".to_string(), Value::String("string".to_string()));
                    }
                }
            }

            if let Some(Value::String(ref t)) = result.get("type") {
                if t != "object" {
                    result.remove("properties");
                    result.remove("required");
                }
            }

            Value::Object(result)
        }
    }
}

// ── provider_options_map ────────────────────────────────────────────────
// (unchanged)

/// Convert provider options to the format expected by the SDK.
/// For gateway, splits options across gateway and upstream provider namespaces.
/// For other providers, wraps under the SDK key.
pub fn provider_options_map(
    model: &models::ModelInfo,
    opts: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    if opts.is_empty() {
        return opts;
    }

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
    let provider_id = model.id.to_lowercase();

    if npm == "@ai-sdk/gateway" {
        let i = api_id.find('/');
        let raw_slug = if let Some(pos) = i {
            if pos > 0 {
                Some(&api_id[..pos])
            } else {
                None
            }
        } else {
            None
        };
        let slug = raw_slug.map(|s| slug_override(s).unwrap_or(s));

        let gateway = opts.get("gateway").cloned();
        let rest: HashMap<String, serde_json::Value> =
            opts.into_iter().filter(|(k, _)| k != "gateway").collect();
        let has_rest = !rest.is_empty();

        let mut result: HashMap<String, serde_json::Value> = HashMap::new();
        if let Some(gw) = gateway.clone() {
            result.insert("gateway".to_string(), gw);
        }

        if has_rest {
            if let Some(slug) = slug {
                result.insert(
                    slug.to_string(),
                    serde_json::to_value(&rest).unwrap_or_default(),
                );
            } else if let Some(ref gw) = gateway {
                if gw.is_object() {
                    let mut merged = gw.clone();
                    if let Some(obj) = merged.as_object_mut() {
                        for (k, v) in &rest {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                    result.insert("gateway".to_string(), merged);
                } else {
                    result.insert(
                        "gateway".to_string(),
                        serde_json::to_value(&rest).unwrap_or_default(),
                    );
                }
            } else {
                result.insert(
                    "gateway".to_string(),
                    serde_json::to_value(&rest).unwrap_or_default(),
                );
            }
        }

        return result;
    }

    let key = sdk_key(npm)
        .map(|s: &str| s.to_string())
        .unwrap_or_else(|| provider_id.clone());
    let mut result = HashMap::new();
    result.insert(key, serde_json::json!(opts));
    result
}

// ── Tests ───────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModelInfo, ModelLimit, ModelProvider};
    use std::collections::HashMap;

    fn test_model(provider_id: &str, npm: &str, api_id: &str) -> ModelInfo {
        ModelInfo {
            id: provider_id.to_string(),
            name: format!("test-{}", provider_id),
            family: None,
            release_date: None,
            attachment: false,
            reasoning: false,
            temperature: false,
            tool_call: true,
            interleaved: None,
            cost: None,
            limit: ModelLimit {
                context: 128000,
                input: None,
                output: 4096,
            },
            modalities: None,
            experimental: None,
            status: None,
            options: HashMap::new(),
            headers: None,
            provider: Some(ModelProvider {
                npm: Some(npm.to_string()),
                api: Some(api_id.to_string()),
            }),
            variants: None,
        }
    }

    // ── P1.1: prompt cache key injection regression ────────────────────

    #[test]
    fn openai_injects_prompt_cache_key_in_camel_case() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openai", &model, "ses-1", &provider_opts);

        assert!(
            result.contains_key("promptCacheKey"),
            "OpenAI provider must inject promptCacheKey"
        );
        let key = result["promptCacheKey"].as_str().expect("promptCacheKey must be a string");
        assert!(key.starts_with("agendao:"), "promptCacheKey must start with agendao:");
        assert!(key.contains(":chat:default:no-repo"), "defaults: chat/default/no-repo");
    }

    #[test]
    fn openrouter_injects_prompt_cache_key_in_snake_case() {
        let model = test_model("openrouter", "@openrouter/ai-sdk-provider", "openai/gpt-4o");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openrouter", &model, "ses-2", &provider_opts);

        assert!(
            result.contains_key("prompt_cache_key"),
            "OpenRouter must inject prompt_cache_key"
        );
        let key = result["prompt_cache_key"].as_str().expect("prompt_cache_key must be a string");
        assert!(key.starts_with("agendao:"));
    }

    #[test]
    fn kimi_injects_prompt_cache_key_in_snake_case() {
        let model = test_model("kimi", "@ai-sdk/openai-compatible", "kimi-k2");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("kimi", &model, "ses-3", &provider_opts);

        assert!(
            result.contains_key("prompt_cache_key"),
            "kimi must inject prompt_cache_key"
        );
    }

    #[test]
    fn moonshot_injects_prompt_cache_key_in_snake_case() {
        let model = test_model("moonshot", "@ai-sdk/openai-compatible", "moonshot-v1");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("moonshot", &model, "ses-4", &provider_opts);

        assert!(
            result.contains_key("prompt_cache_key"),
            "moonshot must inject prompt_cache_key"
        );
    }

    #[test]
    fn deepseek_does_not_inject_prompt_cache_key() {
        let model = test_model("deepseek", "@ai-sdk/openai-compatible", "deepseek-chat");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("deepseek", &model, "ses-5", &provider_opts);

        assert!(
            !result.contains_key("promptCacheKey") && !result.contains_key("prompt_cache_key"),
            "deepseek must NOT inject any prompt cache key"
        );
    }

    #[test]
    fn cache_stage_defaults_to_chat_when_absent_from_provider_options() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openai", &model, "ses-6", &provider_opts);

        let key = result["promptCacheKey"].as_str().expect("promptCacheKey must be a string");
        assert!(
            key.contains(":chat:"),
            "cacheStage must default to 'chat' when not provided"
        );
    }

    #[test]
    fn cache_stage_reads_from_provider_options() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::from([(
            "cacheStage".to_string(),
            serde_json::json!("exec"),
        )]);
        let result = options("openai", &model, "ses-7", &provider_opts);

        let key = result["promptCacheKey"].as_str().expect("promptCacheKey must be a string");
        assert!(
            key.contains(":exec:"),
            "cacheStage must be read from provider_options"
        );
    }

    #[test]
    fn cache_preset_hash_and_repo_hash_flow_into_cache_key() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::from([
            ("cachePresetHash".to_string(), serde_json::json!("sisyphus_v3")),
            ("cacheRepoHash".to_string(), serde_json::json!("repo_abc")),
        ]);
        let result = options("openai", &model, "ses-8", &provider_opts);

        let key = result["promptCacheKey"].as_str().expect("promptCacheKey must be a string");
        assert!(
            key.contains(":sisyphus_v3:repo_abc"),
            "cachePresetHash and cacheRepoHash must appear in cache key, got: {}",
            key
        );
    }

    // ── P3.1: helper behavior coverage ───────────────────────────────

    fn test_model_with_reasoning(provider_id: &str, npm: &str, api_id: &str) -> ModelInfo {
        let mut model = test_model(provider_id, npm, api_id);
        model.reasoning = true;
        model
    }

    #[test]
    fn openrouter_gemini3_injects_reasoning_effort_high() {
        let model = test_model("openrouter", "@openrouter/ai-sdk-provider", "google/gemini-3-flash");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openrouter", &model, "ses", &provider_opts);

        assert!(result.contains_key("usage"), "OpenRouter must inject usage.include");
        let reasoning = result
            .get("reasoning")
            .expect("gemini-3 via OpenRouter must inject reasoning");
        assert_eq!(
            reasoning["effort"].as_str(),
            Some("high"),
            "gemini-3 reasoning effort must be high"
        );
    }

    #[test]
    fn openrouter_non_gemini_does_not_inject_reasoning() {
        let model = test_model("openrouter", "@openrouter/ai-sdk-provider", "openai/gpt-4o");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openrouter", &model, "ses", &provider_opts);

        assert!(result.contains_key("usage"));
        assert!(!result.contains_key("reasoning"), "non-gemini must not inject reasoning");
    }

    #[test]
    fn gpt5_opencode_injects_include_reasoning_encrypted_content() {
        let model = test_model("opencode", "@ai-sdk/openai", "gpt-5.1");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("opencode", &model, "ses", &provider_opts);

        let include = result
            .get("include")
            .expect("opencode gpt-5 must inject include field");
        assert!(
            include.as_array().is_some_and(|arr| arr
                .iter()
                .any(|v| v.as_str() == Some("reasoning.encrypted_content"))),
            "opencode gpt-5 must include reasoning.encrypted_content"
        );
    }

    #[test]
    fn gpt5_non_opencode_does_not_inject_include() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5.1");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openai", &model, "ses", &provider_opts);

        assert!(!result.contains_key("include"), "non-opencode must not inject include");
        assert_eq!(
            result.get("reasoningEffort").and_then(|v| v.as_str()),
            Some("medium"),
            "gpt-5 must inject reasoningEffort=medium"
        );
    }

    #[test]
    fn alibaba_cn_reasoning_model_injects_enable_thinking() {
        let model = test_model_with_reasoning(
            "alibaba-cn",
            "@ai-sdk/openai-compatible",
            "qwen3-coder",
        );
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("alibaba-cn", &model, "ses", &provider_opts);

        assert_eq!(
            result.get("enable_thinking").and_then(|v| v.as_bool()),
            Some(true),
            "alibaba-cn reasoning model must inject enable_thinking=true"
        );
    }

    #[test]
    fn alibaba_cn_non_reasoning_model_does_not_inject_enable_thinking() {
        let model = test_model("alibaba-cn", "@ai-sdk/openai-compatible", "qwen-turbo");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("alibaba-cn", &model, "ses", &provider_opts);

        assert!(
            !result.contains_key("enable_thinking"),
            "non-reasoning alibaba model must not inject enable_thinking"
        );
    }

    #[test]
    fn gateway_injects_caching_auto() {
        let model = test_model("vercel", "@ai-sdk/gateway", "openai/gpt-4o");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("vercel", &model, "ses", &provider_opts);

        let gw = result
            .get("gateway")
            .expect("gateway npm must inject gateway options");
        assert_eq!(
            gw["caching"].as_str(),
            Some("auto"),
            "gateway must inject caching=auto"
        );
    }
}
