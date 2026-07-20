//! Shared execution request authority.
//!
//! Pipeline:
//! `ExecutionResolutionContext -> ResolvedExecutionSpec -> CompiledExecutionRequest -> ChatRequest`
//!
//! This module owns the normalized runtime request contract used by root session,
//! scheduler, agent, subtask, compaction, and title-generation paths. Adapters
//! may provide context and defaults, but they should not duplicate request
//! assembly semantics once data reaches this layer.

use std::collections::HashMap;

pub use agendao_execution_types::{
    agent_generation_request, compaction_request, inline_subtask_request_defaults,
    message_title_request, session_runtime_request_defaults, session_title_request,
    CompiledExecutionRequest, ExecutionModelRef, ExecutionRequestContext, ExecutionRequestDefaults,
};
use agendao_provider::models::{ModelLimit, ModelProvider};
use agendao_provider::{ModelsDevInfo, ReasoningEffort};

#[derive(Debug, Clone, Default)]
pub struct ExecutionCapabilities {
    pub reasoning: bool,
    pub attachment: bool,
    pub temperature: bool,
    pub tool_call: bool,
}

#[derive(Debug, Clone)]
pub struct ExecutionModelSpec {
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: ExecutionCapabilities,
    pub provider: ModelProvider,
    pub options: HashMap<String, serde_json::Value>,
    /// Per-model typed reasoning effort from config (already validated).
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Per-model request-level timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// Per-model streaming stall timeout in seconds.
    pub stream_stall_timeout_secs: Option<u64>,
    /// Explicit variant option tables (config model variants overlaid on catalog
    /// variants). Used at compile time to merge the selected variant's options.
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionTuningSpec {
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedExecutionSpec {
    pub session_id: String,
    pub model: ExecutionModelSpec,
    pub tuning: ExecutionTuningSpec,
    pub request_options: HashMap<String, serde_json::Value>,
}

impl ResolvedExecutionSpec {
    pub fn compile(&self) -> CompiledExecutionRequest {
        let mut compiled_options = self.request_options.clone();
        let thinking_disabled = thinking_explicitly_disabled(&compiled_options);

        if self.model.capabilities.reasoning && !thinking_disabled {
            let generated = agendao_provider::options(
                &self.model.provider_id,
                &self.to_models_dev_info(),
                &self.session_id,
                &compiled_options,
            );
            for (key, value) in generated {
                compiled_options.entry(key).or_insert(value);
            }
        }

        // Variant semantics: the selected variant name maps to a reasoning effort
        // and (when a variant options table exists) contributes provider options.
        // Existing request/model option keys always win over variant-contributed keys.
        let variant = self.tuning.variant.as_deref();
        if let Some(variant_options) = variant.and_then(|name| self.variant_options(name)) {
            for (key, value) in variant_options {
                compiled_options.entry(key).or_insert(value);
            }
        }
        let reasoning_effort = self
            .model
            .reasoning_effort
            .or_else(|| variant.and_then(|name| name.parse::<ReasoningEffort>().ok()));

        CompiledExecutionRequest {
            model_id: self.model.model_id.clone(),
            max_tokens: self.tuning.max_tokens,
            temperature: self.tuning.temperature,
            top_p: self.tuning.top_p,
            variant: self.tuning.variant.clone(),
            provider_options: (!compiled_options.is_empty()).then_some(compiled_options),
            reasoning_effort,
            timeout_secs: self.model.timeout_secs,
            stream_stall_timeout_secs: self.model.stream_stall_timeout_secs,
        }
    }

    /// Look up the options table for a variant name: explicit (config/catalog)
    /// tables take precedence over generated reasoning variants.
    fn variant_options(&self, name: &str) -> Option<HashMap<String, serde_json::Value>> {
        if let Some(explicit) = self
            .model
            .variants
            .as_ref()
            .and_then(|variants| variants.get(name))
        {
            return Some(explicit.clone());
        }
        agendao_provider::transform::variants(&self.to_models_dev_info())
            .remove(name)
    }

    fn to_models_dev_info(&self) -> ModelsDevInfo {
        ModelsDevInfo {
            id: self.model.model_id.clone(),
            name: self.model.display_name.clone(),
            family: None,
            release_date: None,
            attachment: self.model.capabilities.attachment,
            reasoning: self.model.capabilities.reasoning,
            temperature: self.model.capabilities.temperature,
            tool_call: self.model.capabilities.tool_call,
            interleaved: None,
            cost: None,
            limit: ModelLimit {
                context: 131_072,
                input: None,
                output: 32_768,
            },
            modalities: None,
            experimental: None,
            status: None,
            options: self.model.options.clone(),
            headers: None,
            provider: Some(self.model.provider.clone()),
            variants: self.model.variants.clone(),
        }
    }
}

fn thinking_flag_disabled_string(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no" | "none" | "disabled"
    )
}

fn thinking_flag_disabled_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(false) => true,
        serde_json::Value::String(text) => thinking_flag_disabled_string(text),
        serde_json::Value::Object(map) => {
            map.get("enabled").is_some_and(thinking_flag_disabled_value)
                || map
                    .get("includeThoughts")
                    .is_some_and(thinking_flag_disabled_value)
                || map.get("type").is_some_and(thinking_flag_disabled_value)
        }
        _ => false,
    }
}

fn thinking_explicitly_disabled(options: &HashMap<String, serde_json::Value>) -> bool {
    for key in [
        "thinking",
        "reasoning",
        "enable_thinking",
        "thinkingConfig",
        "reasoningEffort",
    ] {
        if options.get(key).is_some_and(thinking_flag_disabled_value) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inherit_missing_only_fills_absent_fields() {
        let request = CompiledExecutionRequest {
            model_id: "model-a".to_string(),
            max_tokens: Some(32),
            temperature: None,
            top_p: Some(0.7),
            variant: None,
            provider_options: None,
            reasoning_effort: None,
            timeout_secs: None,
            stream_stall_timeout_secs: None,
        };

        let inherited = request.inherit_missing(
            &ExecutionRequestDefaults::default()
                .with_max_tokens(Some(64))
                .with_temperature(Some(0.2))
                .with_top_p(Some(0.9))
                .with_variant(Some("fast".to_string()))
                .with_provider_options(Some(HashMap::from([(
                    "thinking".to_string(),
                    json!(true),
                )])))
                .with_reasoning_effort(Some(ReasoningEffort::Low))
                .with_timeout_secs(Some(120))
                .with_stream_stall_timeout_secs(Some(30)),
        );

        assert_eq!(inherited.max_tokens, Some(32));
        assert_eq!(inherited.temperature, Some(0.2));
        assert_eq!(inherited.top_p, Some(0.7));
        assert_eq!(inherited.variant.as_deref(), Some("fast"));
        assert_eq!(
            inherited
                .provider_options
                .as_ref()
                .and_then(|options| options.get("thinking")),
            Some(&json!(true))
        );
        assert_eq!(inherited.reasoning_effort, Some(ReasoningEffort::Low));
        assert_eq!(inherited.timeout_secs, Some(120));
        assert_eq!(inherited.stream_stall_timeout_secs, Some(30));
    }

    #[test]
    fn compile_with_model_and_defaults_respects_context_overrides() {
        let context = ExecutionRequestContext {
            model_id: Some("ctx-model".to_string()),
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            variant: Some("deep".to_string()),
            provider_options: None,
            provider_id: Some("provider".to_string()),
        };

        let compiled = context.compile_with_model_and_defaults(
            "override-model",
            &inline_subtask_request_defaults(Some("fast".to_string())),
        );

        assert_eq!(compiled.model_id, "override-model");
        assert_eq!(compiled.max_tokens, Some(512));
        assert_eq!(compiled.temperature, Some(0.2));
        assert_eq!(compiled.variant.as_deref(), Some("deep"));
    }

    #[test]
    fn session_runtime_defaults_include_variant_and_max_tokens() {
        let defaults = session_runtime_request_defaults(Some("fast".to_string()));
        assert_eq!(defaults.max_tokens, Some(8192));
        assert_eq!(defaults.temperature, None);
        assert_eq!(defaults.variant.as_deref(), Some("fast"));
    }

    #[test]
    fn inline_subtask_defaults_include_runtime_budget() {
        let defaults = inline_subtask_request_defaults(Some("deep".to_string()));
        assert_eq!(defaults.max_tokens, Some(2048));
        assert_eq!(defaults.temperature, Some(0.2));
        assert_eq!(defaults.top_p, None);
        assert_eq!(defaults.variant.as_deref(), Some("deep"));
    }

    #[test]
    fn compaction_request_uses_shared_policy() {
        let request = compaction_request("compact-model", Some("fast".to_string()));
        assert_eq!(request.model_id, "compact-model");
        assert_eq!(request.max_tokens, Some(4096));
        assert_eq!(request.temperature, Some(0.0));
        assert_eq!(request.variant.as_deref(), Some("fast"));
    }

    #[test]
    fn title_requests_use_shared_policy() {
        let session = session_title_request("title-model");
        assert_eq!(session.model_id, "title-model");
        assert_eq!(session.max_tokens, Some(100));
        assert_eq!(session.temperature, Some(0.0));

        let message = message_title_request("message-model");
        assert_eq!(message.model_id, "message-model");
        assert_eq!(message.max_tokens, Some(64));
        assert_eq!(message.temperature, Some(0.0));
    }

    #[test]
    fn agent_generation_request_uses_shared_policy() {
        let request = agent_generation_request("agent-model");
        assert_eq!(request.model_id, "agent-model");
        assert_eq!(request.temperature, Some(0.3));
        assert_eq!(request.max_tokens, None);
    }

    // ── Phase 6: Architecture regression tests ──
    //
    // These tests lock down the convergence invariants so that future changes
    // cannot silently re-introduce authority bypass or field drift.

    /// Invariant: CompiledExecutionRequest fields are a strict subset of
    /// ChatRequest tunable parameters. If a new tunable field is added to
    /// CompiledExecutionRequest, it must also appear in to_chat_request output.
    /// This test catches field drift between the two types.
    #[test]
    fn compiled_request_fields_propagate_to_chat_request() {
        let compiled = CompiledExecutionRequest {
            model_id: "regression-model".to_string(),
            max_tokens: Some(999),
            temperature: Some(0.42),
            top_p: Some(0.88),
            variant: Some("deep".to_string()),
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"enabled": true}),
            )])),
            reasoning_effort: Some(ReasoningEffort::Medium),
            timeout_secs: Some(300),
            stream_stall_timeout_secs: Some(45),
        };

        let chat = compiled.to_chat_request(vec![], vec![], true);

        assert_eq!(chat.model, "regression-model");
        assert_eq!(chat.max_tokens, Some(999));
        assert_eq!(chat.temperature, Some(0.42));
        assert_eq!(chat.top_p, Some(0.88));
        assert_eq!(chat.variant.as_deref(), Some("deep"));
        assert!(chat.provider_options.is_some());
        assert_eq!(
            chat.provider_options
                .as_ref()
                .and_then(|o| o.get("thinking")),
            Some(&json!({"enabled": true}))
        );
        assert_eq!(chat.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(chat.timeout_secs, Some(300));
        assert_eq!(chat.stream_stall_timeout_secs, Some(45));
    }

    /// Invariant: inherit_missing is the single merge point for defaults.
    /// All policy helpers must produce results consistent with inherit_missing
    /// semantics — explicit values win, absent values get filled.
    #[test]
    fn all_entry_points_use_inherit_missing_semantics() {
        // Build a compiled request with explicit max_tokens but no temperature.
        let base = CompiledExecutionRequest {
            model_id: "test".to_string(),
            max_tokens: Some(42),
            ..Default::default()
        };

        let defaults = ExecutionRequestDefaults::default()
            .with_max_tokens(Some(9999))
            .with_temperature(Some(0.5))
            .with_variant(Some("fast".to_string()));

        let merged = base.inherit_missing(&defaults);

        // Explicit max_tokens preserved (not overwritten by defaults).
        assert_eq!(merged.max_tokens, Some(42));
        // Absent temperature filled from defaults.
        assert_eq!(merged.temperature, Some(0.5));
        // Absent variant filled from defaults.
        assert_eq!(merged.variant.as_deref(), Some("fast"));
    }

    /// Invariant: every cross-entry-point helper goes through the shared
    /// pipeline (ExecutionRequestDefaults + inherit_missing). Verify that
    /// adding a new field to ExecutionRequestDefaults propagates to all
    /// helpers without per-helper changes.
    #[test]
    fn cross_entry_point_variant_propagation() {
        let variant = Some("extended".to_string());

        let session = session_runtime_request_defaults(variant.clone());
        let subtask = inline_subtask_request_defaults(variant.clone());
        let compact = compaction_request("m", variant.clone());

        // All three entry points carry the variant through the shared pipeline.
        assert_eq!(session.variant.as_deref(), Some("extended"));
        assert_eq!(subtask.variant.as_deref(), Some("extended"));
        assert_eq!(compact.variant.as_deref(), Some("extended"));
    }

    /// Invariant: ExecutionRequestDefaults field set matches
    /// CompiledExecutionRequest tunable field set (minus model_id).
    /// If a new tunable is added to CompiledExecutionRequest, it must also
    /// appear in ExecutionRequestDefaults so inherit_missing can fill it.
    #[test]
    fn defaults_field_set_covers_compiled_tunables() {
        // Construct defaults with every field set.
        let defaults = ExecutionRequestDefaults {
            max_tokens: Some(1),
            temperature: Some(0.1),
            top_p: Some(0.2),
            variant: Some("v".to_string()),
            provider_options: Some(HashMap::from([("k".to_string(), json!("v"))])),
            reasoning_effort: Some(ReasoningEffort::Minimal),
            timeout_secs: Some(60),
            stream_stall_timeout_secs: Some(15),
        };

        // Merge into an empty compiled request.
        let compiled = CompiledExecutionRequest {
            model_id: "m".to_string(),
            ..Default::default()
        }
        .inherit_missing(&defaults);

        // Every tunable field should be filled.
        assert!(compiled.max_tokens.is_some(), "max_tokens not propagated");
        assert!(compiled.temperature.is_some(), "temperature not propagated");
        assert!(compiled.top_p.is_some(), "top_p not propagated");
        assert!(compiled.variant.is_some(), "variant not propagated");
        assert!(
            compiled.provider_options.is_some(),
            "provider_options not propagated"
        );
        assert!(
            compiled.reasoning_effort.is_some(),
            "reasoning_effort not propagated"
        );
        assert!(compiled.timeout_secs.is_some(), "timeout_secs not propagated");
        assert!(
            compiled.stream_stall_timeout_secs.is_some(),
            "stream_stall_timeout_secs not propagated"
        );
    }

    /// Invariant: with_variant and with_model return new instances (struct
    /// update syntax), not in-place mutation. The original must be unchanged.
    #[test]
    fn compiled_request_immutable_builders() {
        let original = CompiledExecutionRequest {
            model_id: "original".to_string(),
            variant: Some("v1".to_string()),
            ..Default::default()
        };

        let changed_model = original.with_model("new-model");
        let changed_variant = original.with_variant(Some("v2".to_string()));

        // Original unchanged.
        assert_eq!(original.model_id, "original");
        assert_eq!(original.variant.as_deref(), Some("v1"));

        // New instances have the changes.
        assert_eq!(changed_model.model_id, "new-model");
        assert_eq!(changed_model.variant.as_deref(), Some("v1")); // variant preserved
        assert_eq!(changed_variant.model_id, "original"); // model preserved
        assert_eq!(changed_variant.variant.as_deref(), Some("v2"));
    }

    // ── Model-level tuning semantics (reasoning effort / timeouts / variants) ──

    fn model_spec(
        reasoning_effort: Option<ReasoningEffort>,
        variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
    ) -> ExecutionModelSpec {
        ExecutionModelSpec {
            provider_id: "openai".to_string(),
            model_id: "gpt-test".to_string(),
            display_name: "gpt-test".to_string(),
            capabilities: ExecutionCapabilities {
                reasoning: true,
                ..Default::default()
            },
            provider: ModelProvider {
                api: None,
                npm: None,
            },
            options: HashMap::new(),
            reasoning_effort,
            timeout_secs: Some(600),
            stream_stall_timeout_secs: Some(20),
            variants,
        }
    }

    fn spec_with_variant(
        model: ExecutionModelSpec,
        variant: Option<&str>,
        request_options: HashMap<String, serde_json::Value>,
    ) -> ResolvedExecutionSpec {
        ResolvedExecutionSpec {
            session_id: "s1".to_string(),
            model,
            tuning: ExecutionTuningSpec {
                variant: variant.map(str::to_string),
                ..Default::default()
            },
            request_options,
        }
    }

    #[test]
    fn compile_maps_variant_name_to_reasoning_effort() {
        for (variant, expected) in [
            ("low", ReasoningEffort::Low),
            ("medium", ReasoningEffort::Medium),
            ("high", ReasoningEffort::High),
        ] {
            let compiled =
                spec_with_variant(model_spec(None, None), Some(variant), HashMap::new()).compile();
            assert_eq!(compiled.reasoning_effort, Some(expected), "variant {variant}");
        }
    }

    #[test]
    fn compile_prefers_explicit_model_reasoning_effort_over_variant() {
        let compiled = spec_with_variant(
            model_spec(Some(ReasoningEffort::Minimal), None),
            Some("high"),
            HashMap::new(),
        )
        .compile();
        assert_eq!(compiled.reasoning_effort, Some(ReasoningEffort::Minimal));
    }

    #[test]
    fn compile_leaves_reasoning_effort_none_without_config_or_known_variant() {
        let compiled = spec_with_variant(model_spec(None, None), Some("deep"), HashMap::new())
            .compile();
        assert_eq!(compiled.reasoning_effort, None);

        let compiled = spec_with_variant(model_spec(None, None), None, HashMap::new()).compile();
        assert_eq!(compiled.reasoning_effort, None);
    }

    #[test]
    fn compile_propagates_model_timeouts() {
        let compiled = spec_with_variant(model_spec(None, None), None, HashMap::new()).compile();
        assert_eq!(compiled.timeout_secs, Some(600));
        assert_eq!(compiled.stream_stall_timeout_secs, Some(20));
    }

    #[test]
    fn compile_merges_variant_options_preserving_existing_keys() {
        let variants = Some(HashMap::from([(
            "high".to_string(),
            HashMap::from([
                ("reasoningEffort".to_string(), json!("high")),
                ("thinking".to_string(), json!({"budget": 16000})),
            ]),
        )]));
        let request_options = HashMap::from([("reasoningEffort".to_string(), json!("low"))]);

        let compiled = spec_with_variant(
            model_spec(Some(ReasoningEffort::Low), variants),
            Some("high"),
            request_options,
        )
        .compile();

        let options = compiled.provider_options.expect("provider options");
        // Existing request option key wins over the variant-contributed key.
        assert_eq!(options.get("reasoningEffort"), Some(&json!("low")));
        // Absent keys are contributed by the variant table.
        assert_eq!(options.get("thinking"), Some(&json!({"budget": 16000})));
    }

    #[test]
    fn compile_uses_generated_variant_options_when_no_explicit_table() {
        // gpt-test under @ai-sdk/openai generates low/high reasoningEffort variants.
        let mut model = model_spec(None, None);
        model.provider.npm = Some("@ai-sdk/openai".to_string());

        let compiled = spec_with_variant(model, Some("high"), HashMap::new()).compile();

        assert_eq!(compiled.reasoning_effort, Some(ReasoningEffort::High));
        let options = compiled.provider_options.expect("provider options");
        assert_eq!(options.get("reasoningEffort"), Some(&json!("high")));
    }
}
