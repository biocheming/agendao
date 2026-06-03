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
use agendao_provider::models::ModelProvider;

use crate::{
    CompiledExecutionRequest, ExecutionCapabilities, ExecutionModelSpec, ExecutionTuningSpec,
    ResolvedExecutionSpec,
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

pub async fn resolve_request_execution_spec(
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
    async fn request_spec_enables_default_thinking_when_reasoning_supported() {
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
        let compiled = spec.compile().provider_options.expect("compiled");
        assert_eq!(
            compiled.get("thinking"),
            Some(&json!({"type": "enabled", "clear_thinking": false}))
        );
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
}
