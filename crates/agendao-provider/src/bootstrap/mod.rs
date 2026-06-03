mod custom_loaders;
mod provider_factory;
mod state;
#[cfg(test)]
mod tests;
mod types;

use crate::auth::AuthInfo;
use crate::models::{ModelInfo, ModelInterleaved, ModelsData, ProviderInfo as ModelsProviderInfo};
use crate::provider::ProviderRegistry;
use std::collections::HashMap;

use self::custom_loaders::get_custom_loader;
#[cfg(test)]
use self::custom_loaders::AmazonBedrockLoader;
use self::provider_factory::{
    create_concrete_provider, load_models_dev_cache, register_fallback_env_providers,
    wrap_provider_for_state,
};

pub use self::custom_loaders::{CustomLoader, CustomLoaderResult};
pub use self::state::{parse_model, ParsedModel, ProviderBootstrapState, ProviderPatch};
pub use self::types::{
    is_gpt5_or_later, should_use_copilot_responses_api, BootstrapConfig, BootstrapError,
    ConfigModalities, ConfigModel, ConfigModelCost, ConfigModelLimit, ConfigModelProvider,
    ConfigProvider, InterleavedConfig, ModalitySet, ModelCapabilities, ModelCostCache,
    ModelCostOver200K, ProviderModel, ProviderModelApi, ProviderModelCost, ProviderModelLimit,
    ProviderState, BUNDLED_PROVIDERS,
};

/// Transform a models.dev model into a runtime ProviderModel.
pub fn from_models_dev_model(provider: &ModelsProviderInfo, model: &ModelInfo) -> ProviderModel {
    let modalities_input = model
        .modalities
        .as_ref()
        .map(|m| &m.input)
        .cloned()
        .unwrap_or_default();
    let modalities_output = model
        .modalities
        .as_ref()
        .map(|m| &m.output)
        .cloned()
        .unwrap_or_default();

    let interleaved = match model.interleaved.as_ref() {
        Some(ModelInterleaved::Bool(value)) => InterleavedConfig::Bool(*value),
        Some(ModelInterleaved::Field { field }) => InterleavedConfig::Field {
            field: field.clone(),
        },
        None => InterleavedConfig::Bool(false),
    };

    let cost = model.cost.as_ref();
    let over_200k = cost.and_then(|c| c.context_over_200k.as_ref());

    let mut variants = crate::transform::variants(model);
    if let Some(explicit_variants) = &model.variants {
        for (variant_name, options) in explicit_variants {
            variants.insert(variant_name.clone(), options.clone());
        }
    }

    ProviderModel {
        id: model.id.clone(),
        provider_id: provider.id.clone(),
        name: model.name.clone(),
        family: model.family.clone(),
        api: ProviderModelApi {
            id: model.id.clone(),
            url: model
                .provider
                .as_ref()
                .and_then(|p| p.api.clone())
                .or_else(|| provider.api.clone())
                .unwrap_or_default(),
            npm: model
                .provider
                .as_ref()
                .and_then(|p| p.npm.clone())
                .or_else(|| provider.npm.clone())
                .unwrap_or_else(|| "@ai-sdk/openai-compatible".to_string()),
        },
        status: model.status.clone().unwrap_or_else(|| "active".to_string()),
        headers: model.headers.clone().unwrap_or_default(),
        options: model.options.clone(),
        cost: ProviderModelCost {
            input: cost.map(|c| c.input).unwrap_or(0.0),
            output: cost.map(|c| c.output).unwrap_or(0.0),
            cache: ModelCostCache {
                read: cost.and_then(|c| c.cache_read).unwrap_or(0.0),
                write: cost.and_then(|c| c.cache_write).unwrap_or(0.0),
            },
            experimental_over_200k: over_200k.map(|o| ModelCostOver200K {
                input: o.input,
                output: o.output,
                cache: ModelCostCache {
                    read: o.cache_read.unwrap_or(0.0),
                    write: o.cache_write.unwrap_or(0.0),
                },
            }),
        },
        limit: ProviderModelLimit {
            context: model.limit.context,
            input: model.limit.input,
            output: model.limit.output,
        },
        capabilities: ModelCapabilities {
            temperature: model.temperature,
            reasoning: model.reasoning,
            attachment: model.attachment,
            toolcall: model.tool_call,
            input: ModalitySet {
                text: modalities_input.contains(&"text".to_string()),
                audio: modalities_input.contains(&"audio".to_string()),
                image: modalities_input.contains(&"image".to_string()),
                video: modalities_input.contains(&"video".to_string()),
                pdf: modalities_input.contains(&"pdf".to_string()),
            },
            output: ModalitySet {
                text: modalities_output.contains(&"text".to_string()),
                audio: modalities_output.contains(&"audio".to_string()),
                image: modalities_output.contains(&"image".to_string()),
                video: modalities_output.contains(&"video".to_string()),
                pdf: modalities_output.contains(&"pdf".to_string()),
            },
            interleaved,
        },
        release_date: model.release_date.clone().unwrap_or_default(),
        variants: if variants.is_empty() {
            None
        } else {
            Some(variants)
        },
    }
}

/// Transform a models.dev provider into a runtime ProviderState.
pub fn from_models_dev_provider(provider: &ModelsProviderInfo) -> ProviderState {
    let models = provider
        .models
        .iter()
        .map(|(id, model)| (id.clone(), from_models_dev_model(provider, model)))
        .collect();

    ProviderState {
        id: provider.id.clone(),
        source: "custom".to_string(),
        name: provider.name.clone(),
        env: provider.env.clone(),
        key: None,
        options: HashMap::new(),
        models,
    }
}

/// Create a ProviderRegistry populated from environment variables.
/// Scans known provider env vars and registers any that are configured.
pub fn create_registry_from_env() -> ProviderRegistry {
    let auth_store: HashMap<String, AuthInfo> = HashMap::new();
    create_registry_from_env_with_auth_store(&auth_store)
}

/// Create a ProviderRegistry populated from environment variables plus explicit
/// auth store entries (for example plugin-provided auth tokens).
pub fn create_registry_from_env_with_auth_store(
    auth_store: &HashMap<String, AuthInfo>,
) -> ProviderRegistry {
    bootstrap_registry(&BootstrapConfig::default(), auth_store)
}

/// Create a ProviderRegistry using the given bootstrap config and auth store.
/// This is the primary entry point when you have a loaded application config
/// whose provider/model fields have been converted into a `BootstrapConfig`.
pub fn create_registry_from_bootstrap_config(
    config: &BootstrapConfig,
    auth_store: &HashMap<String, AuthInfo>,
) -> ProviderRegistry {
    bootstrap_registry(config, auth_store)
}

fn bootstrap_registry(
    config: &BootstrapConfig,
    auth_store: &HashMap<String, AuthInfo>,
) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();

    let models_dev = load_models_dev_cache();
    let state = ProviderBootstrapState::init(&models_dev, config, auth_store);

    for (provider_id, provider_state) in &state.providers {
        if let Some(provider) = create_concrete_provider(provider_id, provider_state) {
            let provider = wrap_provider_for_state(provider_state, provider);
            let registered_id = provider.id().to_string();
            registry.register_arc(provider);
            if !provider_state.options.is_empty() {
                registry.merge_config(&registered_id, provider_state.options.clone());
            }
            tracing::debug!(
                provider = provider_id,
                concrete_provider = registered_id,
                "Registered provider from bootstrap state"
            );
        } else {
            tracing::debug!(
                provider = provider_id,
                "No concrete provider implementation for bootstrap provider"
            );
        }
    }

    if registry.list().is_empty() {
        tracing::debug!(
            "No providers registered from bootstrap state, falling back to direct env registration"
        );
        register_fallback_env_providers(&mut registry);
    }

    registry
}

/// Build a `BootstrapConfig` from the raw config fields typically found in
/// `agendao_config::Config`. This bridges the gap between the config loader
/// and the provider bootstrap system.
///
/// The `providers` map should be converted from `agendao_config::ProviderConfig`
/// to `ConfigProvider` by the caller (see `config_provider_to_bootstrap` helper).
pub fn bootstrap_config_from_raw(
    providers: HashMap<String, ConfigProvider>,
    disabled_providers: Vec<String>,
    enabled_providers: Vec<String>,
    model: Option<String>,
    small_model: Option<String>,
) -> BootstrapConfig {
    BootstrapConfig {
        providers,
        disabled_providers: disabled_providers.into_iter().collect(),
        enabled_providers: if enabled_providers.is_empty() {
            None
        } else {
            Some(enabled_providers.into_iter().collect())
        },
        enable_experimental: false,
        model,
        small_model,
    }
}

/// Apply custom loaders to models data, mutating it in place.
/// This runs each provider's custom loader and applies blacklists, headers,
/// and option overrides.
pub fn apply_custom_loaders(data: &mut ModelsData) {
    let provider_ids: Vec<String> = data.keys().cloned().collect();

    for provider_id in &provider_ids {
        if let Some(loader) = get_custom_loader(provider_id) {
            let provider_info = match data.get(provider_id) {
                Some(provider) => provider.clone(),
                None => continue,
            };
            let result = loader.load(&provider_info, None);

            if !result.blacklist.is_empty() {
                if let Some(provider) = data.get_mut(provider_id) {
                    provider.models.retain(|model_id, _| {
                        let lower = model_id.to_lowercase();
                        !result
                            .blacklist
                            .iter()
                            .any(|pattern| lower.contains(pattern))
                    });
                }
            }

            if !result.headers.is_empty() {
                if let Some(provider) = data.get_mut(provider_id) {
                    for model in provider.models.values_mut() {
                        let headers = model.headers.get_or_insert_with(HashMap::new);
                        for (key, value) in &result.headers {
                            headers.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
        }
    }
}

/// Filter models by status, removing deprecated models and optionally alpha models.
pub fn filter_models_by_status(data: &mut ModelsData, enable_experimental: bool) {
    for provider in data.values_mut() {
        provider.models.retain(|_model_id, model| {
            let status = model.status.as_deref().unwrap_or("active");
            if status == "deprecated" {
                return false;
            }
            if status == "alpha" && !enable_experimental {
                return false;
            }
            true
        });
    }

    data.retain(|_provider_id, provider| !provider.models.is_empty());
}
