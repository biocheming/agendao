//! Shared config-aware execution resolver authority.
//!
//! This module translates adapter-supplied resolution context into the stable
//! execution pipeline:
//! `ExecutionResolutionContext -> ResolvedExecutionSpec -> CompiledExecutionRequest`.
//!
//! Provider catalog lookup, config model overrides, capability merge, request
//! option merge, and default thinking behavior are centralized here so adapter
//! layers do not re-implement policy.

use std::collections::HashMap;
use std::time::Duration;

use agendao_config::Config as AppConfig;
use agendao_execution_types::CompiledExecutionRequest;
use agendao_provider::models::ModelProvider;
use agendao_provider::ReasoningEffort;

use crate::model_request::{
    ExecutionCapabilities, ExecutionModelSpec, ExecutionTuningSpec, ResolvedExecutionSpec,
};

#[derive(Debug, Clone, Default)]
pub struct ExecutionResolutionContext {
    pub session_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub variant: Option<String>,
}

fn config_provider_entry<'a>(
    config: &'a AppConfig,
    provider_id: &str,
) -> Option<&'a agendao_config::ProviderConfig> {
    config.provider.as_ref()?.get(provider_id)
}

fn config_model_entry<'a>(
    provider: &'a agendao_config::ProviderConfig,
    model_id: &str,
) -> Option<&'a agendao_config::ModelConfig> {
    provider
        .models
        .as_ref()?
        .iter()
        .find_map(|(configured_id, model)| {
            let resolved_id = model.model.as_deref().unwrap_or(configured_id.as_str());
            (configured_id == model_id || resolved_id == model_id).then_some(model)
        })
}

fn model_provider_from_config(
    provider: Option<&agendao_config::ProviderConfig>,
    model: Option<&agendao_config::ModelConfig>,
) -> ModelProvider {
    let api = model
        .and_then(|entry| entry.provider.as_ref())
        .and_then(|entry| entry.api.clone())
        .or_else(|| provider.and_then(|entry| entry.base_url.clone()));
    let npm = model
        .and_then(|entry| entry.provider.as_ref())
        .and_then(|entry| entry.npm.clone())
        .or_else(|| provider.and_then(|entry| entry.npm.clone()));
    ModelProvider { api, npm }
}

fn capabilities_from_config(model: &agendao_config::ModelConfig) -> ExecutionCapabilities {
    ExecutionCapabilities {
        reasoning: model.reasoning.unwrap_or(false),
        attachment: model.attachment.unwrap_or(false),
        temperature: model.temperature.unwrap_or(false),
        tool_call: model.tool_call.unwrap_or(false),
    }
}

fn capabilities_from_catalog(model: &agendao_provider::ModelsDevInfo) -> ExecutionCapabilities {
    ExecutionCapabilities {
        reasoning: model.reasoning,
        attachment: model.attachment,
        temperature: model.temperature,
        tool_call: model.tool_call,
    }
}

/// Parse a per-model `reasoning_effort` config string. Invalid values are
/// ignored with a warning so a typo cannot break request assembly.
fn parse_reasoning_effort(
    raw: Option<&str>,
    provider_id: &str,
    model_id: &str,
) -> Option<ReasoningEffort> {
    raw.and_then(|value| match value.parse::<ReasoningEffort>() {
        Ok(effort) => Some(effort),
        Err(_) => {
            tracing::warn!(
                provider = %provider_id,
                model = %model_id,
                value = %value,
                "ignoring invalid model reasoning_effort (expected none/minimal/low/medium/high)"
            );
            None
        }
    })
}

/// Convert config model variants into option tables, skipping disabled ones.
fn config_model_variants(
    model: &agendao_config::ModelConfig,
) -> Option<HashMap<String, HashMap<String, serde_json::Value>>> {
    let variants = model.variants.as_ref()?;
    let tables: HashMap<_, _> = variants
        .iter()
        .filter(|(_, variant)| !variant.disabled.unwrap_or(false))
        .map(|(name, variant)| (name.clone(), variant.extra.clone()))
        .collect();
    (!tables.is_empty()).then_some(tables)
}

fn merge_model_spec(
    mut base: ExecutionModelSpec,
    provider: Option<&agendao_config::ProviderConfig>,
    model: Option<&agendao_config::ModelConfig>,
) -> ExecutionModelSpec {
    if let Some(model_cfg) = model {
        if let Some(name) = model_cfg.name.clone() {
            base.display_name = name;
        }
        let override_caps = capabilities_from_config(model_cfg);
        if model_cfg.reasoning.is_some() {
            base.capabilities.reasoning = override_caps.reasoning;
        }
        if model_cfg.attachment.is_some() {
            base.capabilities.attachment = override_caps.attachment;
        }
        if model_cfg.temperature.is_some() {
            base.capabilities.temperature = override_caps.temperature;
        }
        if model_cfg.tool_call.is_some() {
            base.capabilities.tool_call = override_caps.tool_call;
        }
        if model_cfg.reasoning_effort.is_some() {
            base.reasoning_effort = parse_reasoning_effort(
                model_cfg.reasoning_effort.as_deref(),
                &base.provider_id,
                &base.model_id,
            );
        }
        if model_cfg.timeout_secs.is_some() {
            base.timeout_secs = model_cfg.timeout_secs;
        }
        if model_cfg.stream_stall_timeout_secs.is_some() {
            base.stream_stall_timeout_secs = model_cfg.stream_stall_timeout_secs;
        }
        if let Some(variant_cfgs) = &model_cfg.variants {
            // Config variants overlay catalog variants per key; a disabled
            // config variant removes a catalog variant of the same name.
            let base_variants = base.variants.get_or_insert_with(HashMap::new);
            for (name, variant_cfg) in variant_cfgs {
                if variant_cfg.disabled.unwrap_or(false) {
                    base_variants.remove(name);
                } else {
                    base_variants.insert(name.clone(), variant_cfg.extra.clone());
                }
            }
        }
        if let Some(options) = model_cfg.options.clone() {
            base.options.extend(options);
        }
        base.provider = model_provider_from_config(provider, model);
    } else if base.provider.api.is_none() && base.provider.npm.is_none() {
        base.provider = model_provider_from_config(provider, None);
    }
    base
}

fn spec_from_catalog(
    model: agendao_provider::ModelsDevInfo,
    provider_id: &str,
) -> ExecutionModelSpec {
    ExecutionModelSpec {
        provider_id: provider_id.to_string(),
        model_id: model.id.clone(),
        display_name: model.name.clone(),
        capabilities: capabilities_from_catalog(&model),
        provider: model.provider.clone().unwrap_or(ModelProvider {
            api: None,
            npm: None,
        }),
        options: model.options.clone(),
        reasoning_effort: None,
        timeout_secs: None,
        stream_stall_timeout_secs: None,
        variants: model.variants.clone(),
    }
}

fn spec_from_config(
    provider: Option<&agendao_config::ProviderConfig>,
    model: &agendao_config::ModelConfig,
    provider_id: &str,
    model_id: &str,
) -> ExecutionModelSpec {
    ExecutionModelSpec {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        display_name: model.name.clone().unwrap_or_else(|| model_id.to_string()),
        capabilities: capabilities_from_config(model),
        provider: model_provider_from_config(provider, Some(model)),
        options: model.options.clone().unwrap_or_default(),
        reasoning_effort: parse_reasoning_effort(
            model.reasoning_effort.as_deref(),
            provider_id,
            model_id,
        ),
        timeout_secs: model.timeout_secs,
        stream_stall_timeout_secs: model.stream_stall_timeout_secs,
        variants: config_model_variants(model),
    }
}

async fn load_catalog_model(
    provider_id: &str,
    model_id: &str,
) -> Option<agendao_provider::ModelsDevInfo> {
    let registry = agendao_provider::ModelsRegistry::default();
    match tokio::time::timeout(Duration::from_millis(250), registry.get()).await {
        Ok(data) => data
            .get(provider_id)
            .and_then(|provider| provider.models.get(model_id))
            .cloned(),
        Err(_) => None,
    }
}

async fn resolve_request_execution_spec(
    config: &AppConfig,
    context: &ExecutionResolutionContext,
) -> ResolvedExecutionSpec {
    let provider_cfg = config_provider_entry(config, &context.provider_id);
    let model_cfg =
        provider_cfg.and_then(|provider| config_model_entry(provider, &context.model_id));

    let base_options = provider_cfg
        .and_then(|provider| provider.options.clone())
        .unwrap_or_default();

    let model_spec = match (
        load_catalog_model(&context.provider_id, &context.model_id).await,
        model_cfg,
    ) {
        (Some(catalog), maybe_model_cfg) => {
            let spec = spec_from_catalog(catalog, &context.provider_id);
            merge_model_spec(spec, provider_cfg, maybe_model_cfg)
        }
        (None, Some(model)) => {
            spec_from_config(provider_cfg, model, &context.provider_id, &context.model_id)
        }
        (None, None) => {
            return ResolvedExecutionSpec {
                session_id: context.session_id.clone(),
                model: ExecutionModelSpec {
                    provider_id: context.provider_id.clone(),
                    model_id: context.model_id.clone(),
                    display_name: context.model_id.clone(),
                    capabilities: ExecutionCapabilities::default(),
                    provider: model_provider_from_config(provider_cfg, None),
                    options: HashMap::new(),
                    reasoning_effort: None,
                    timeout_secs: None,
                    stream_stall_timeout_secs: None,
                    variants: None,
                },
                tuning: ExecutionTuningSpec {
                    max_tokens: context.max_tokens,
                    temperature: context.temperature,
                    top_p: context.top_p,
                    variant: context.variant.clone(),
                },
                request_options: base_options,
            };
        }
    };

    let mut request_options = base_options;
    request_options.extend(model_spec.options.clone());

    ResolvedExecutionSpec {
        session_id: context.session_id.clone(),
        model: model_spec,
        tuning: ExecutionTuningSpec {
            max_tokens: context.max_tokens,
            temperature: context.temperature,
            top_p: context.top_p,
            variant: context.variant.clone(),
        },
        request_options,
    }
}

pub async fn resolve_compiled_execution_request(
    config: &AppConfig,
    context: &ExecutionResolutionContext,
) -> CompiledExecutionRequest {
    resolve_request_execution_spec(config, context)
        .await
        .compile()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config_with_model(
        provider_options: Option<HashMap<String, serde_json::Value>>,
        model_reasoning: Option<bool>,
        model_options: Option<HashMap<String, serde_json::Value>>,
    ) -> AppConfig {
        let model = agendao_config::ModelConfig {
            name: Some("Test Model".to_string()),
            reasoning: model_reasoning,
            options: model_options,
            provider: Some(agendao_config::ModelProviderConfig {
                api: Some("https://example.test".to_string()),
                npm: Some("@ai-sdk/openai-compatible".to_string()),
            }),
            ..Default::default()
        };
        let provider = agendao_config::ProviderConfig {
            name: Some("zhipuai".to_string()),
            options: provider_options,
            models: Some(HashMap::from([("glm-5".to_string(), model)])),
            ..Default::default()
        };
        AppConfig {
            provider: Some(HashMap::from([("zhipuai".to_string(), provider)])),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn reasoning_capability_does_not_invent_provider_specific_options() {
        let config = config_with_model(None, Some(true), None);
        let spec = resolve_request_execution_spec(
            &config,
            &ExecutionResolutionContext {
                session_id: "s1".to_string(),
                provider_id: "zhipuai".to_string(),
                model_id: "glm-5".to_string(),
                ..Default::default()
            },
        )
        .await;
        assert!(spec.model.capabilities.reasoning);
        assert!(spec.compile().provider_options.is_none());
    }

    #[tokio::test]
    async fn request_spec_respects_explicit_thinking_disable() {
        let config = config_with_model(
            Some(HashMap::from([("thinking".to_string(), json!(false))])),
            Some(true),
            None,
        );
        let compiled = resolve_compiled_execution_request(
            &config,
            &ExecutionResolutionContext {
                session_id: "s1".to_string(),
                provider_id: "zhipuai".to_string(),
                model_id: "glm-5".to_string(),
                ..Default::default()
            },
        )
        .await
        .provider_options
        .expect("compiled");
        assert_eq!(compiled.get("thinking"), Some(&json!(false)));
    }

    #[tokio::test]
    async fn request_spec_merges_provider_and_model_options() {
        let config = config_with_model(
            Some(HashMap::from([(
                "promptCacheKey".to_string(),
                json!("root"),
            )])),
            Some(false),
            Some(HashMap::from([(
                "temperature_mode".to_string(),
                json!("fixed"),
            )])),
        );
        let spec = resolve_request_execution_spec(
            &config,
            &ExecutionResolutionContext {
                session_id: "s1".to_string(),
                provider_id: "zhipuai".to_string(),
                model_id: "glm-5".to_string(),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            spec.request_options.get("promptCacheKey"),
            Some(&json!("root"))
        );
        assert_eq!(
            spec.request_options.get("temperature_mode"),
            Some(&json!("fixed"))
        );
    }

    fn config_with_model_tuning(
        reasoning_effort: Option<&str>,
        timeout_secs: Option<u64>,
        stream_stall_timeout_secs: Option<u64>,
        variants: Option<HashMap<String, agendao_config::ModelVariantConfig>>,
    ) -> AppConfig {
        let model = agendao_config::ModelConfig {
            name: Some("Test Model".to_string()),
            reasoning: Some(true),
            reasoning_effort: reasoning_effort.map(str::to_string),
            timeout_secs,
            stream_stall_timeout_secs,
            variants,
            provider: Some(agendao_config::ModelProviderConfig {
                api: Some("https://example.test".to_string()),
                npm: Some("@ai-sdk/openai-compatible".to_string()),
            }),
            ..Default::default()
        };
        let provider = agendao_config::ProviderConfig {
            name: Some("zhipuai".to_string()),
            models: Some(HashMap::from([("glm-5".to_string(), model)])),
            ..Default::default()
        };
        AppConfig {
            provider: Some(HashMap::from([("zhipuai".to_string(), provider)])),
            ..Default::default()
        }
    }

    fn test_context() -> ExecutionResolutionContext {
        ExecutionResolutionContext {
            session_id: "s1".to_string(),
            provider_id: "zhipuai".to_string(),
            model_id: "glm-5".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn request_spec_carries_model_level_tuning_fields() {
        let config = config_with_model_tuning(Some("high"), Some(900), Some(25), None);
        let spec = resolve_request_execution_spec(&config, &test_context()).await;
        assert_eq!(spec.model.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(spec.model.timeout_secs, Some(900));
        assert_eq!(spec.model.stream_stall_timeout_secs, Some(25));

        let compiled = spec.compile();
        assert_eq!(compiled.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(compiled.timeout_secs, Some(900));
        assert_eq!(compiled.stream_stall_timeout_secs, Some(25));
    }

    #[tokio::test]
    async fn request_spec_ignores_invalid_reasoning_effort() {
        let config = config_with_model_tuning(Some("extreme"), None, None, None);
        let spec = resolve_request_execution_spec(&config, &test_context()).await;
        assert_eq!(spec.model.reasoning_effort, None);
    }

    #[tokio::test]
    async fn request_spec_leaves_tuning_none_without_config() {
        let config = config_with_model_tuning(None, None, None, None);
        let spec = resolve_request_execution_spec(&config, &test_context()).await;
        assert_eq!(spec.model.reasoning_effort, None);
        assert_eq!(spec.model.timeout_secs, None);
        assert_eq!(spec.model.stream_stall_timeout_secs, None);
        let compiled = spec.compile();
        assert_eq!(compiled.reasoning_effort, None);
        assert_eq!(compiled.timeout_secs, None);
        assert_eq!(compiled.stream_stall_timeout_secs, None);
    }

    #[tokio::test]
    async fn request_spec_carries_config_variant_tables_and_skips_disabled() {
        let variants = HashMap::from([
            (
                "low".to_string(),
                agendao_config::ModelVariantConfig {
                    disabled: None,
                    extra: HashMap::from([("reasoningEffort".to_string(), json!("low"))]),
                },
            ),
            (
                "high".to_string(),
                agendao_config::ModelVariantConfig {
                    disabled: Some(true),
                    extra: HashMap::from([("reasoningEffort".to_string(), json!("high"))]),
                },
            ),
        ]);
        let config = config_with_model_tuning(None, None, None, Some(variants));
        let spec = resolve_request_execution_spec(
            &config,
            &ExecutionResolutionContext {
                variant: Some("low".to_string()),
                ..test_context()
            },
        )
        .await;

        let tables = spec.model.variants.as_ref().expect("variant tables");
        assert!(tables.contains_key("low"));
        assert!(!tables.contains_key("high"), "disabled variant skipped");

        let compiled = spec.compile();
        assert_eq!(compiled.reasoning_effort, Some(ReasoningEffort::Low));
        let options = compiled.provider_options.expect("provider options");
        assert_eq!(options.get("reasoningEffort"), Some(&json!("low")));
    }
}
