use crate::auth::AuthInfo;
use crate::models::{ModelInfo, ModelInterleaved, ModelsData, ProviderInfo as ModelsProviderInfo};
use std::collections::{HashMap, HashSet};

use super::{from_models_dev_provider, get_custom_loader, BootstrapConfig, BootstrapError};
use super::{
    ConfigModel, ConfigProvider, InterleavedConfig, ModalitySet, ModelCapabilities, ModelCostCache,
    ProviderModel, ProviderModelApi, ProviderModelCost, ProviderModelLimit, ProviderState,
};

/// The initialized provider state, analogous to the TS `state()` return value.
pub struct ProviderBootstrapState {
    pub providers: HashMap<String, ProviderState>,
    pub model_loaders: HashSet<String>,
}

impl ProviderBootstrapState {
    /// Initialize the provider bootstrap state from models.dev data and config.
    pub fn init(
        models_dev: &ModelsData,
        config: &BootstrapConfig,
        auth_store: &HashMap<String, AuthInfo>,
    ) -> Self {
        let mut database: HashMap<String, ProviderState> = models_dev
            .iter()
            .map(|(id, provider)| (id.clone(), from_models_dev_provider(provider)))
            .collect();

        let disabled = &config.disabled_providers;
        let enabled = &config.enabled_providers;

        let mut providers: HashMap<String, ProviderState> = HashMap::new();
        let mut model_loaders: HashSet<String> = HashSet::new();

        if let Some(github_copilot) = database.get("github-copilot").cloned() {
            let mut enterprise = github_copilot.clone();
            enterprise.id = "github-copilot-enterprise".to_string();
            enterprise.name = "GitHub Copilot Enterprise".to_string();
            for model in enterprise.models.values_mut() {
                model.provider_id = "github-copilot-enterprise".to_string();
            }
            database.insert("github-copilot-enterprise".to_string(), enterprise);
        }

        let merge_provider = |providers: &mut HashMap<String, ProviderState>,
                              database: &HashMap<String, ProviderState>,
                              provider_id: &str,
                              patch: ProviderPatch| {
            if let Some(existing) = providers.get_mut(provider_id) {
                apply_patch(existing, patch);
            } else if let Some(base) = database.get(provider_id) {
                let mut merged = base.clone();
                apply_patch(&mut merged, patch);
                providers.insert(provider_id.to_string(), merged);
            }
        };

        for (provider_id, cfg_provider) in &config.providers {
            let existing = database.get(provider_id);
            let mut parsed = ProviderState {
                id: provider_id.clone(),
                name: cfg_provider
                    .name
                    .clone()
                    .or_else(|| existing.map(|provider| provider.name.clone()))
                    .unwrap_or_else(|| provider_id.clone()),
                env: cfg_provider
                    .env
                    .clone()
                    .or_else(|| existing.map(|provider| provider.env.clone()))
                    .unwrap_or_default(),
                options: merge_json_maps(
                    existing
                        .map(|provider| &provider.options)
                        .unwrap_or(&HashMap::new()),
                    cfg_provider.options.as_ref().unwrap_or(&HashMap::new()),
                ),
                source: "config".to_string(),
                key: None,
                models: existing
                    .map(|provider| provider.models.clone())
                    .unwrap_or_default(),
            };
            apply_config_provider_profile_options(&mut parsed.options, cfg_provider);

            if let Some(cfg_models) = &cfg_provider.models {
                for (model_id, cfg_model) in cfg_models {
                    let existing_model = parsed
                        .models
                        .get(&cfg_model.id.clone().unwrap_or_else(|| model_id.clone()));
                    let provider_model = config_to_provider_model(
                        provider_id,
                        model_id,
                        cfg_model,
                        existing_model,
                        cfg_provider,
                        models_dev.get(provider_id),
                    );
                    parsed.models.insert(model_id.clone(), provider_model);
                }
            }

            database.insert(provider_id.clone(), parsed);
        }

        for (provider_id, provider) in &database {
            if disabled.contains(provider_id) {
                continue;
            }
            let api_key = provider
                .env
                .iter()
                .find_map(|name| std::env::var(name).ok());
            if let Some(_key) = api_key {
                let key_val = if provider.env.len() == 1 {
                    std::env::var(&provider.env[0]).ok()
                } else {
                    None
                };
                merge_provider(
                    &mut providers,
                    &database,
                    provider_id,
                    ProviderPatch {
                        source: Some("env".to_string()),
                        key: key_val,
                        ..Default::default()
                    },
                );
            }
        }

        for (provider_id, auth) in auth_store {
            if disabled.contains(provider_id) {
                continue;
            }
            let maybe_key = match auth {
                AuthInfo::Api { key } => Some(key.clone()),
                AuthInfo::OAuth { access, .. } => Some(access.clone()),
                AuthInfo::WellKnown { token, .. } => Some(token.clone()),
            };
            if let Some(key) = maybe_key {
                merge_provider(
                    &mut providers,
                    &database,
                    provider_id,
                    ProviderPatch {
                        source: Some("api".to_string()),
                        key: Some(key),
                        ..Default::default()
                    },
                );
            }
        }

        for (provider_id, data) in &database {
            if disabled.contains(provider_id) {
                continue;
            }
            if let Some(loader) = get_custom_loader(provider_id) {
                let models_provider = to_models_provider_info(data, models_dev.get(provider_id));
                let result = loader.load(&models_provider, Some(data));

                if result.autoload || providers.contains_key(provider_id) {
                    if result.has_custom_get_model {
                        model_loaders.insert(provider_id.clone());
                    }

                    let patch = ProviderPatch {
                        source: if providers.contains_key(provider_id) {
                            None
                        } else {
                            Some("custom".to_string())
                        },
                        options: if result.options.is_empty() {
                            None
                        } else {
                            Some(result.options)
                        },
                        ..Default::default()
                    };
                    merge_provider(&mut providers, &database, provider_id, patch);

                    if !result.headers.is_empty() {
                        if let Some(provider) = providers.get_mut(provider_id) {
                            for model in provider.models.values_mut() {
                                for (key, value) in &result.headers {
                                    model.headers.insert(key.clone(), value.clone());
                                }
                            }
                        }
                    }

                    if !result.blacklist.is_empty() {
                        if let Some(provider) = providers.get_mut(provider_id) {
                            provider.models.retain(|model_id, _| {
                                let lower = model_id.to_lowercase();
                                !result
                                    .blacklist
                                    .iter()
                                    .any(|pattern| lower.contains(pattern))
                            });
                        }
                    }
                }
            }
        }

        for (provider_id, cfg_provider) in &config.providers {
            let mut patch = ProviderPatch {
                source: Some("config".to_string()),
                ..Default::default()
            };
            if let Some(env) = &cfg_provider.env {
                patch.env = Some(env.clone());
            }
            if let Some(name) = &cfg_provider.name {
                patch.name = Some(name.clone());
            }
            if let Some(options) = &cfg_provider.options {
                patch.options = Some(options.clone());
            }
            merge_provider(&mut providers, &database, provider_id, patch);
        }

        let is_provider_allowed = |provider_id: &str| -> bool {
            if let Some(enabled) = enabled {
                if !enabled.contains(provider_id) {
                    return false;
                }
            }
            !disabled.contains(provider_id)
        };

        let provider_ids: Vec<String> = providers.keys().cloned().collect();
        for provider_id in provider_ids {
            if !is_provider_allowed(&provider_id) {
                providers.remove(&provider_id);
                continue;
            }

            let cfg_provider = config.providers.get(&provider_id);

            if let Some(provider) = providers.get_mut(&provider_id) {
                let model_ids: Vec<String> = provider.models.keys().cloned().collect();
                for model_id in model_ids {
                    let should_remove = {
                        let model = &provider.models[&model_id];

                        let blocked_by_status = model_id == "gpt-5-chat-latest"
                            || (provider_id == "openrouter" && model_id == "openai/gpt-5-chat")
                            || (model.status == "alpha" && !config.enable_experimental)
                            || model.status == "deprecated";

                        if blocked_by_status {
                            true
                        } else if let Some(cfg_provider) = cfg_provider {
                            if let Some(blacklist) = &cfg_provider.blacklist {
                                if blacklist.contains(&model_id) {
                                    true
                                } else if let Some(whitelist) = &cfg_provider.whitelist {
                                    !whitelist.contains(&model_id)
                                } else {
                                    false
                                }
                            } else if let Some(whitelist) = &cfg_provider.whitelist {
                                !whitelist.contains(&model_id)
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    };

                    if should_remove {
                        provider.models.remove(&model_id);
                    }
                }

                if provider.models.is_empty() {
                    providers.remove(&provider_id);
                }
            }
        }

        Self {
            providers,
            model_loaders,
        }
    }

    pub fn list(&self) -> &HashMap<String, ProviderState> {
        &self.providers
    }

    pub fn get_provider(&self, provider_id: &str) -> Option<&ProviderState> {
        self.providers.get(provider_id)
    }

    pub fn get_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<&ProviderModel, BootstrapError> {
        let provider = self.providers.get(provider_id).ok_or_else(|| {
            let available: Vec<String> = self.providers.keys().cloned().collect();
            let suggestions = fuzzy_match(provider_id, &available, 3);
            BootstrapError::ModelNotFound {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                suggestions,
            }
        })?;

        provider.models.get(model_id).ok_or_else(|| {
            let available: Vec<String> = provider.models.keys().cloned().collect();
            let suggestions = fuzzy_match(model_id, &available, 3);
            BootstrapError::ModelNotFound {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                suggestions,
            }
        })
    }

    pub fn closest(&self, provider_id: &str, queries: &[&str]) -> Option<(String, String)> {
        let provider = self.providers.get(provider_id)?;
        for query in queries {
            for model_id in provider.models.keys() {
                if model_id.contains(query) {
                    return Some((provider_id.to_string(), model_id.clone()));
                }
            }
        }
        None
    }

    pub fn get_small_model(
        &self,
        provider_id: &str,
        config_small_model: Option<&str>,
    ) -> Option<ProviderModel> {
        if let Some(model_str) = config_small_model {
            let parsed = parse_model(model_str);
            return self
                .get_model(&parsed.provider_id, &parsed.model_id)
                .ok()
                .cloned();
        }

        if let Some(provider) = self.providers.get(provider_id) {
            let mut priority: Vec<&str> = vec!["gemini-3-flash", "gemini-2.5-flash", "gpt-5-nano"];

            if provider_id.starts_with("opencode") {
                priority = vec!["gpt-5-nano"];
            }
            if provider_id.starts_with("github-copilot") {
                priority = vec!["gpt-5-mini"];
                priority.extend_from_slice(&["gemini-3-flash", "gemini-2.5-flash", "gpt-5-nano"]);
            }

            for item in &priority {
                if provider_id == "amazon-bedrock" {
                    let cross_region_prefixes = ["global.", "us.", "eu."];
                    let candidates: Vec<&String> = provider
                        .models
                        .keys()
                        .filter(|model_id| model_id.contains(item))
                        .collect();

                    if let Some(global_match) = candidates
                        .iter()
                        .find(|model_id| model_id.starts_with("global."))
                    {
                        return provider.models.get(*global_match).cloned();
                    }

                    let region = provider
                        .options
                        .get("region")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let region_prefix = region.split('-').next().unwrap_or("");
                    if region_prefix == "us" || region_prefix == "eu" {
                        if let Some(regional) = candidates
                            .iter()
                            .find(|model_id| model_id.starts_with(&format!("{}.", region_prefix)))
                        {
                            return provider.models.get(*regional).cloned();
                        }
                    }

                    if let Some(unprefixed) = candidates.iter().find(|model_id| {
                        !cross_region_prefixes
                            .iter()
                            .any(|prefix| model_id.starts_with(prefix))
                    }) {
                        return provider.models.get(*unprefixed).cloned();
                    }
                } else {
                    for model_id in provider.models.keys() {
                        if model_id.contains(item) {
                            return provider.models.get(model_id).cloned();
                        }
                    }
                }
            }
        }

        if let Some(provider) = self.providers.get("opencode") {
            if let Some(model) = provider.models.get("gpt-5-nano") {
                return Some(model.clone());
            }
        }

        None
    }

    pub fn sort_models(models: &mut [ProviderModel]) {
        let priority_list = ["gpt-5", "big-pickle", "gemini-3-pro"];

        models.sort_by(|a, b| {
            let a_priority = priority_list
                .iter()
                .position(|priority| a.id.contains(priority))
                .map(|index| -(index as i64))
                .unwrap_or(i64::MAX);
            let b_priority = priority_list
                .iter()
                .position(|priority| b.id.contains(priority))
                .map(|index| -(index as i64))
                .unwrap_or(i64::MAX);

            a_priority
                .cmp(&b_priority)
                .then_with(|| {
                    let a_latest = if a.id.contains("latest") { 0 } else { 1 };
                    let b_latest = if b.id.contains("latest") { 0 } else { 1 };
                    a_latest.cmp(&b_latest)
                })
                .then_with(|| b.id.cmp(&a.id))
        });
    }

    pub fn default_model(
        &self,
        config_model: Option<&str>,
        recent: &[(String, String)],
    ) -> Option<ParsedModel> {
        if let Some(model_str) = config_model {
            return Some(parse_model(model_str));
        }

        for (provider_id, model_id) in recent {
            if let Some(provider) = self.providers.get(provider_id) {
                if provider.models.contains_key(model_id) {
                    return Some(ParsedModel {
                        provider_id: provider_id.clone(),
                        model_id: model_id.clone(),
                    });
                }
            }
        }

        let provider = self.providers.values().next()?;
        let mut models: Vec<ProviderModel> = provider.models.values().cloned().collect();
        Self::sort_models(&mut models);
        let model = models.first()?;
        Some(ParsedModel {
            provider_id: provider.id.clone(),
            model_id: model.id.clone(),
        })
    }
}

fn apply_config_provider_profile_options(
    options: &mut HashMap<String, serde_json::Value>,
    cfg_provider: &ConfigProvider,
) {
    let mut profile = serde_json::Map::new();
    if let Some(value) = &cfg_provider.api_style {
        profile.insert(
            "api_style".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &cfg_provider.api_shape {
        profile.insert(
            "api_shape".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &cfg_provider.transport {
        profile.insert(
            "transport".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &cfg_provider.usage_shape {
        profile.insert(
            "usage_shape".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(values) = &cfg_provider.quirks {
        profile.insert(
            "quirks".to_string(),
            serde_json::Value::Array(
                values
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }

    if !profile.is_empty() {
        options.insert(
            "providerProfile".to_string(),
            serde_json::Value::Object(profile),
        );
    }
}

#[derive(Debug, Clone)]
pub struct ParsedModel {
    pub provider_id: String,
    pub model_id: String,
}

pub fn parse_model(model_str: &str) -> ParsedModel {
    if let Some(pos) = model_str.find('/') {
        ParsedModel {
            provider_id: model_str[..pos].to_string(),
            model_id: model_str[pos + 1..].to_string(),
        }
    } else {
        ParsedModel {
            provider_id: model_str.to_string(),
            model_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderPatch {
    pub source: Option<String>,
    pub name: Option<String>,
    pub env: Option<Vec<String>>,
    pub key: Option<String>,
    pub options: Option<HashMap<String, serde_json::Value>>,
}

fn apply_patch(state: &mut ProviderState, patch: ProviderPatch) {
    if let Some(source) = patch.source {
        state.source = source;
    }
    if let Some(name) = patch.name {
        state.name = name;
    }
    if let Some(env) = patch.env {
        state.env = env;
    }
    if let Some(key) = patch.key {
        state.key = Some(key);
    }
    if let Some(options) = patch.options {
        for (key, value) in options {
            state.options.insert(key, value);
        }
    }
}

fn merge_json_maps(
    base: &HashMap<String, serde_json::Value>,
    overlay: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    let mut result = base.clone();
    for (key, value) in overlay {
        result.insert(key.clone(), value.clone());
    }
    result
}

fn config_to_provider_model(
    provider_id: &str,
    model_id: &str,
    cfg: &ConfigModel,
    existing: Option<&ProviderModel>,
    cfg_provider: &ConfigProvider,
    models_provider: Option<&ModelsProviderInfo>,
) -> ProviderModel {
    let api_model_id = cfg.id.clone().unwrap_or_else(|| model_id.to_string());

    let default_npm = cfg_provider
        .npm
        .clone()
        .or_else(|| models_provider.and_then(|provider| provider.npm.clone()))
        .unwrap_or_else(|| "@ai-sdk/openai-compatible".to_string());
    let default_api = cfg_provider
        .api
        .clone()
        .or_else(|| models_provider.and_then(|provider| provider.api.clone()))
        .unwrap_or_default();

    let base_cost = existing.map(|provider| &provider.cost);
    let base_limit = existing.map(|provider| &provider.limit);
    let base_caps = existing.map(|provider| &provider.capabilities);

    let cost = {
        let cfg_cost = cfg.cost.as_ref();
        ProviderModelCost {
            input: cfg_cost
                .and_then(|cost| cost.input)
                .or_else(|| base_cost.map(|cost| cost.input))
                .unwrap_or(0.0),
            output: cfg_cost
                .and_then(|cost| cost.output)
                .or_else(|| base_cost.map(|cost| cost.output))
                .unwrap_or(0.0),
            cache: ModelCostCache {
                read: cfg_cost
                    .and_then(|cost| cost.cache_read)
                    .or_else(|| base_cost.map(|cost| cost.cache.read))
                    .unwrap_or(0.0),
                write: cfg_cost
                    .and_then(|cost| cost.cache_write)
                    .or_else(|| base_cost.map(|cost| cost.cache.write))
                    .unwrap_or(0.0),
            },
            experimental_over_200k: base_cost.and_then(|cost| cost.experimental_over_200k.clone()),
        }
    };

    let limit = ProviderModelLimit {
        context: cfg
            .limit
            .as_ref()
            .and_then(|limit| limit.context)
            .or_else(|| base_limit.map(|limit| limit.context))
            .unwrap_or(128000),
        input: base_limit.and_then(|limit| limit.input),
        output: cfg
            .limit
            .as_ref()
            .and_then(|limit| limit.output)
            .or_else(|| base_limit.map(|limit| limit.output))
            .unwrap_or(4096),
    };

    let modalities_input = cfg
        .modalities
        .as_ref()
        .and_then(|modalities| modalities.input.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if base_caps
                .map(|capabilities| capabilities.input.text)
                .unwrap_or(true)
            {
                vec!["text".to_string()]
            } else {
                vec![]
            }
        });
    let modalities_output = cfg
        .modalities
        .as_ref()
        .and_then(|modalities| modalities.output.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if base_caps
                .map(|capabilities| capabilities.output.text)
                .unwrap_or(true)
            {
                vec!["text".to_string()]
            } else {
                vec![]
            }
        });

    let interleaved = match cfg.interleaved.as_ref() {
        Some(value) => value.clone(),
        None => existing
            .map(|provider| provider.capabilities.interleaved.clone())
            .unwrap_or_default(),
    };

    let options = merge_json_maps(
        &existing
            .map(|provider| provider.options.clone())
            .unwrap_or_default(),
        cfg.options.as_ref().unwrap_or(&HashMap::new()),
    );
    let headers = merge_string_maps(
        &existing
            .map(|provider| provider.headers.clone())
            .unwrap_or_default(),
        cfg.headers.as_ref().unwrap_or(&HashMap::new()),
    );

    ProviderModel {
        id: model_id.to_string(),
        provider_id: provider_id.to_string(),
        name: cfg.name.clone().unwrap_or_else(|| {
            if cfg.id.as_deref().is_some_and(|id| id != model_id) {
                model_id.to_string()
            } else {
                existing
                    .map(|provider| provider.name.clone())
                    .unwrap_or_else(|| model_id.to_string())
            }
        }),
        family: cfg
            .family
            .clone()
            .or_else(|| existing.and_then(|provider| provider.family.clone())),
        api: ProviderModelApi {
            id: api_model_id,
            url: cfg
                .provider
                .as_ref()
                .and_then(|provider| provider.api.clone())
                .or_else(|| existing.map(|provider| provider.api.url.clone()))
                .unwrap_or_else(|| default_api.clone()),
            npm: cfg
                .provider
                .as_ref()
                .and_then(|provider| provider.npm.clone())
                .or_else(|| existing.map(|provider| provider.api.npm.clone()))
                .unwrap_or_else(|| default_npm.clone()),
        },
        status: cfg
            .status
            .clone()
            .or_else(|| existing.map(|provider| provider.status.clone()))
            .unwrap_or_else(|| "active".to_string()),
        cost,
        limit,
        capabilities: ModelCapabilities {
            temperature: cfg
                .temperature
                .or_else(|| base_caps.map(|capabilities| capabilities.temperature))
                .unwrap_or(true),
            reasoning: cfg
                .reasoning
                .or_else(|| base_caps.map(|capabilities| capabilities.reasoning))
                .unwrap_or(false),
            attachment: cfg
                .attachment
                .or_else(|| base_caps.map(|capabilities| capabilities.attachment))
                .unwrap_or(false),
            toolcall: cfg
                .tool_call
                .or_else(|| base_caps.map(|capabilities| capabilities.toolcall))
                .unwrap_or(true),
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
        options,
        headers,
        release_date: cfg
            .release_date
            .clone()
            .or_else(|| existing.map(|provider| provider.release_date.clone()))
            .unwrap_or_default(),
        variants: cfg
            .variants
            .clone()
            .or_else(|| existing.and_then(|provider| provider.variants.clone())),
    }
}

fn merge_string_maps(
    base: &HashMap<String, String>,
    overlay: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut result = base.clone();
    for (key, value) in overlay {
        result.insert(key.clone(), value.clone());
    }
    result
}

fn to_models_provider_info(
    state: &ProviderState,
    original: Option<&ModelsProviderInfo>,
) -> ModelsProviderInfo {
    if let Some(original) = original {
        return original.clone();
    }

    let models = state
        .models
        .iter()
        .map(|(id, provider_model)| {
            let model_info = ModelInfo {
                id: provider_model.id.clone(),
                name: provider_model.name.clone(),
                family: provider_model.family.clone(),
                release_date: Some(provider_model.release_date.clone()),
                attachment: provider_model.capabilities.attachment,
                reasoning: provider_model.capabilities.reasoning,
                temperature: provider_model.capabilities.temperature,
                tool_call: provider_model.capabilities.toolcall,
                interleaved: match &provider_model.capabilities.interleaved {
                    InterleavedConfig::Bool(value) => Some(ModelInterleaved::Bool(*value)),
                    InterleavedConfig::Field { field } => Some(ModelInterleaved::Field {
                        field: field.clone(),
                    }),
                },
                cost: Some(crate::models::ModelCost {
                    input: provider_model.cost.input,
                    output: provider_model.cost.output,
                    cache_read: Some(provider_model.cost.cache.read),
                    cache_write: Some(provider_model.cost.cache.write),
                    context_over_200k: None,
                }),
                limit: crate::models::ModelLimit {
                    context: provider_model.limit.context,
                    input: provider_model.limit.input,
                    output: provider_model.limit.output,
                },
                modalities: None,
                experimental: None,
                status: Some(provider_model.status.clone()),
                options: provider_model.options.clone(),
                headers: if provider_model.headers.is_empty() {
                    None
                } else {
                    Some(provider_model.headers.clone())
                },
                provider: Some(crate::models::ModelProvider {
                    npm: Some(provider_model.api.npm.clone()),
                    api: Some(provider_model.api.url.clone()),
                }),
                variants: provider_model.variants.clone(),
            };
            (id.clone(), model_info)
        })
        .collect();

    ModelsProviderInfo {
        id: state.id.clone(),
        name: state.name.clone(),
        env: state.env.clone(),
        api: None,
        npm: None,
        models,
    }
}

fn fuzzy_match(query: &str, options: &[String], max: usize) -> Vec<String> {
    let query_lower = query.to_lowercase();
    let mut scored: Vec<(usize, &String)> = options
        .iter()
        .filter_map(|option| {
            let option_lower = option.to_lowercase();
            let score = longest_common_substring_len(&query_lower, &option_lower);
            if score >= 2
                || option_lower.contains(&query_lower)
                || query_lower.contains(&option_lower)
            {
                Some((score, option))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(max)
        .map(|(_, value)| value.clone())
        .collect()
}

fn longest_common_substring_len(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut max_len = 0;
    let mut prev = vec![0usize; b_bytes.len() + 1];
    for i in 1..=a_bytes.len() {
        let mut curr = vec![0usize; b_bytes.len() + 1];
        for j in 1..=b_bytes.len() {
            if a_bytes[i - 1] == b_bytes[j - 1] {
                curr[j] = prev[j - 1] + 1;
                if curr[j] > max_len {
                    max_len = curr[j];
                }
            }
        }
        prev = curr;
    }
    max_len
}
