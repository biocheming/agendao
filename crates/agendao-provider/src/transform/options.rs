use std::collections::HashMap;

use crate::cache::{build_prompt_cache_key, openai_prompt_cache_key_field, PromptCacheKeyContext};
use crate::models;

// ── Pipeline ───────────────────────────────────────────────────────────
//
// options() assembles provider request options in this order:
//
// 1. Protocol defaults          (driven by the explicit SDK shape)
// 2. Prompt cache key           (session + stage + repo → cache key)
// 3. OpenAI model tuning        (only for models that declare reasoning)

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

    apply_openai_protocol_defaults(npm, &mut result);

    // ── Step 2: Prompt cache key ────────────────────────────────────
    // Session + stage + repo → cache key. Provider-family gating is
    // handled by openai_prompt_cache_key_field().

    let prompt_cache_key = build_prompt_cache_key(PromptCacheKeyContext {
        session_id,
        stage: provider_options
            .get("cacheStage")
            .and_then(|value| value.as_str())
            .unwrap_or("chat"),
        repo_hash: provider_options
            .get("cacheRepoHash")
            .and_then(|value| value.as_str()),
    });
    if let Some(field) = openai_prompt_cache_key_field(&provider_id, npm) {
        result.insert(field.as_str().to_string(), json!(prompt_cache_key));
    }

    if model.reasoning && npm == "@ai-sdk/openai" {
        apply_gpt5_reasoning_config(api_id, &mut result);
    }

    result
}

// ── Provider-family helpers ─────────────────────────────────────────────

fn apply_openai_protocol_defaults(npm: &str, result: &mut HashMap<String, serde_json::Value>) {
    if npm == "@ai-sdk/openai" {
        result.insert("store".to_string(), serde_json::json!(false));
    }
}

fn apply_gpt5_reasoning_config(api_id: &str, result: &mut HashMap<String, serde_json::Value>) {
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
        let key = result["promptCacheKey"]
            .as_str()
            .expect("promptCacheKey must be a string");
        assert!(
            key.starts_with("agendao:"),
            "promptCacheKey must start with agendao:"
        );
        assert!(
            key.contains(":chat:no-repo"),
            "defaults: chat/default/no-repo"
        );
        assert_eq!(
            result.get("store").and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn openai_non_reasoning_request_keeps_protocol_defaults() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-4.1");
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openai", &model, "ses-2", &provider_opts);

        assert!(result.contains_key("promptCacheKey"));
        assert_eq!(
            result.get("store").and_then(|value| value.as_bool()),
            Some(false)
        );
        assert!(!result.contains_key("reasoningEffort"));
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

        let key = result["promptCacheKey"]
            .as_str()
            .expect("promptCacheKey must be a string");
        assert!(
            key.contains(":chat:"),
            "cacheStage must default to 'chat' when not provided"
        );
    }

    #[test]
    fn cache_stage_reads_from_provider_options() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let provider_opts: HashMap<String, serde_json::Value> =
            HashMap::from([("cacheStage".to_string(), serde_json::json!("exec"))]);
        let result = options("openai", &model, "ses-7", &provider_opts);

        let key = result["promptCacheKey"]
            .as_str()
            .expect("promptCacheKey must be a string");
        assert!(
            key.contains(":exec:"),
            "cacheStage must be read from provider_options"
        );
    }

    #[test]
    fn cache_repo_hash_flows_into_cache_key() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let provider_opts: HashMap<String, serde_json::Value> =
            HashMap::from([("cacheRepoHash".to_string(), serde_json::json!("repo_abc"))]);
        let result = options("openai", &model, "ses-8", &provider_opts);

        let key = result["promptCacheKey"]
            .as_str()
            .expect("promptCacheKey must be a string");
        assert!(
            key.contains(":repo_abc"),
            "cacheRepoHash must appear in cache key, got: {}",
            key
        );
    }

    #[test]
    fn prompt_cache_key_ignores_overlay_and_diagnostic_provider_options() {
        let model = test_model("openai", "@ai-sdk/openai", "gpt-5");
        let baseline_provider_opts: HashMap<String, serde_json::Value> = HashMap::from([
            ("cacheStage".to_string(), serde_json::json!("chat")),
            ("cacheRepoHash".to_string(), serde_json::json!("repo_abc")),
        ]);
        let noisy_provider_opts: HashMap<String, serde_json::Value> = HashMap::from([
            ("cacheStage".to_string(), serde_json::json!("chat")),
            ("cacheRepoHash".to_string(), serde_json::json!("repo_abc")),
            (
                "promptSurfaceVolatility".to_string(),
                serde_json::json!(["clock", "repo_status"]),
            ),
            (
                "dynamicOverlayReasons".to_string(),
                serde_json::json!(["request_boundary_hygiene", "tool_compaction"]),
            ),
            (
                "requestBoundaryHygieneSummary".to_string(),
                serde_json::json!({
                    "dropped_orphan_tool_results": 1,
                    "compressed_tool_results": 1
                }),
            ),
            (
                "providerDiagnostic".to_string(),
                serde_json::json!({
                    "kind": "transient",
                    "message": "not part of cache identity"
                }),
            ),
        ]);

        let baseline_key = options("openai", &model, "ses-cache", &baseline_provider_opts)
            .get("promptCacheKey")
            .and_then(|value| value.as_str())
            .expect("baseline promptCacheKey must be present")
            .to_string();
        let noisy_key = options("openai", &model, "ses-cache", &noisy_provider_opts)
            .get("promptCacheKey")
            .and_then(|value| value.as_str())
            .expect("noisy promptCacheKey must be present")
            .to_string();

        assert_eq!(
            baseline_key, noisy_key,
            "diagnostics and dynamic overlays must not perturb prompt cache identity"
        );
    }

    #[test]
    fn gpt5_non_opencode_does_not_inject_include() {
        let mut model = test_model("openai", "@ai-sdk/openai", "gpt-5.1");
        model.reasoning = true;
        let provider_opts: HashMap<String, serde_json::Value> = HashMap::new();
        let result = options("openai", &model, "ses", &provider_opts);

        assert!(
            !result.contains_key("include"),
            "non-opencode must not inject include"
        );
        assert_eq!(
            result.get("reasoningEffort").and_then(|v| v.as_str()),
            Some("medium"),
            "gpt-5 must inject reasoningEffort=medium"
        );
    }
}
