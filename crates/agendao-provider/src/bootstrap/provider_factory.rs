use crate::models::ModelsData;
use crate::profile::ProviderProfile;
use crate::protocol::{ProviderConfig, ProviderRuntimeAdapter};
use crate::provider::{ModelInfo as RuntimeModelInfo, Provider as RuntimeProvider};
use crate::runtime::RuntimeConfig;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use super::{ProviderModel, ProviderState};

#[cfg(feature = "http-transport")]
use crate::catalog::load_default_catalog_data_sync;
#[cfg(feature = "http-transport")]
use crate::instance::ProviderInstance;
#[cfg(feature = "http-transport")]
use crate::profile::{resolve_npm_for_provider, ProviderProfileResolver};
#[cfg(feature = "http-transport")]
use crate::protocols::create_provider_adapter_for_profile;
#[cfg(feature = "http-transport")]
use crate::runtime::ProviderRuntime;

pub(super) fn env_any(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn provider_secret(provider: &ProviderState, fallback_env: &[&str]) -> Option<String> {
    provider
        .key
        .clone()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| {
            provider
                .env
                .iter()
                .find_map(|name| std::env::var(name).ok())
                .filter(|key| !key.trim().is_empty())
        })
        .or_else(|| env_any(fallback_env))
}

fn provider_base_url(provider: &ProviderState) -> Option<String> {
    provider
        .models
        .values()
        .find_map(|model| (!model.api.url.trim().is_empty()).then(|| model.api.url.clone()))
}

fn default_secret_env_for_provider(
    provider_id: &str,
    adapter: ProviderRuntimeAdapter,
) -> Vec<&'static str> {
    match adapter {
        ProviderRuntimeAdapter::Anthropic => vec!["ANTHROPIC_API_KEY"],
        ProviderRuntimeAdapter::OpenAiCompatible if provider_id == "openai" => {
            vec!["OPENAI_API_KEY"]
        }
        ProviderRuntimeAdapter::OpenAiCompatible => vec![],
    }
}

fn collect_provider_headers(provider: &ProviderState) -> HashMap<String, String> {
    let mut headers = HashMap::new();

    for model in provider.models.values() {
        headers.extend(model.headers.clone());
    }

    if let Some(serde_json::Value::Object(map)) = provider.options.get("headers") {
        for (key, value) in map {
            if let Some(value) = value.as_str() {
                headers.insert(key.clone(), value.to_string());
            }
        }
    }

    headers
}

fn parse_bool_text(raw: &str) -> Option<bool> {
    let lower = raw.trim().to_ascii_lowercase();
    if matches!(lower.as_str(), "1" | "true" | "yes" | "on") {
        return Some(true);
    }
    if matches!(lower.as_str(), "0" | "false" | "no" | "off") {
        return Some(false);
    }
    None
}

fn option_bool(options: &HashMap<String, serde_json::Value>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        let Some(value) = options.get(*key) else {
            continue;
        };
        match value {
            serde_json::Value::Bool(v) => return Some(*v),
            serde_json::Value::Number(n) => return Some(n.as_i64().unwrap_or(0) != 0),
            serde_json::Value::String(s) => {
                if let Some(value) = parse_bool_text(s) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn option_u32(options: &HashMap<String, serde_json::Value>, keys: &[&str]) -> Option<u32> {
    for key in keys {
        let Some(value) = options.get(*key) else {
            continue;
        };
        match value {
            serde_json::Value::Number(n) => {
                if let Some(value) = n.as_u64() {
                    return Some(value as u32);
                }
                if let Some(value) = n.as_i64() {
                    return Some(value.max(0) as u32);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(value) = s.parse::<u32>() {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn option_u64(options: &HashMap<String, serde_json::Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(value) = options.get(*key) else {
            continue;
        };
        match value {
            serde_json::Value::Number(n) => {
                if let Some(value) = n.as_u64() {
                    return Some(value);
                }
                if let Some(value) = n.as_i64() {
                    return Some(value.max(0) as u64);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(value) = s.parse::<u64>() {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn option_f64(options: &HashMap<String, serde_json::Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        let Some(value) = options.get(*key) else {
            continue;
        };
        match value {
            serde_json::Value::Number(n) => {
                if let Some(value) = n.as_f64() {
                    return Some(value);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(value) = s.parse::<f64>() {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn env_bool(keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            if let Some(value) = parse_bool_text(&raw) {
                return Some(value);
            }
        }
    }
    None
}

fn env_u32(keys: &[&str]) -> Option<u32> {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            if let Ok(value) = raw.parse::<u32>() {
                return Some(value);
            }
        }
    }
    None
}

fn env_u64(keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            if let Ok(value) = raw.parse::<u64>() {
                return Some(value);
            }
        }
    }
    None
}

fn env_f64(keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            if let Ok(value) = raw.parse::<f64>() {
                return Some(value);
            }
        }
    }
    None
}

fn build_runtime_config(options: &HashMap<String, serde_json::Value>) -> RuntimeConfig {
    let defaults = RuntimeConfig::default();
    RuntimeConfig {
        enabled: option_bool(options, &["runtime_enabled"])
            .or_else(|| env_bool(&["AGENDAO_RUNTIME_ENABLED"]))
            .unwrap_or(defaults.enabled),
        preflight_enabled: option_bool(options, &["runtime_preflight", "preflight_enabled"])
            .or_else(|| env_bool(&["AGENDAO_RUNTIME_PREFLIGHT"]))
            .unwrap_or(defaults.preflight_enabled),
        circuit_breaker_threshold: option_u32(
            options,
            &[
                "circuit_breaker_threshold",
                "runtime_circuit_breaker_threshold",
            ],
        )
        .or_else(|| env_u32(&["AGENDAO_RUNTIME_CIRCUIT_BREAKER_THRESHOLD"]))
        .unwrap_or(defaults.circuit_breaker_threshold),
        circuit_breaker_cooldown_secs: option_u64(
            options,
            &[
                "circuit_breaker_cooldown_secs",
                "runtime_circuit_breaker_cooldown_secs",
            ],
        )
        .or_else(|| env_u64(&["AGENDAO_RUNTIME_CIRCUIT_BREAKER_COOLDOWN_SECS"]))
        .unwrap_or(defaults.circuit_breaker_cooldown_secs),
        rate_limit_rps: option_f64(options, &["rate_limit_rps", "runtime_rate_limit_rps"])
            .or_else(|| env_f64(&["AGENDAO_RUNTIME_RATE_LIMIT_RPS"]))
            .unwrap_or(defaults.rate_limit_rps),
        max_inflight: option_u32(options, &["max_inflight", "runtime_max_inflight"])
            .or_else(|| env_u32(&["AGENDAO_RUNTIME_MAX_INFLIGHT"]))
            .unwrap_or(defaults.max_inflight),
    }
}

fn provider_config_for_adapter(
    provider_id: &str,
    provider: &ProviderState,
    profile: &ProviderProfile,
    adapter: ProviderRuntimeAdapter,
) -> Option<ProviderConfig> {
    let fallback_env = default_secret_env_for_provider(provider_id, adapter);
    let headers = collect_provider_headers(provider);
    let mut options = provider.options.clone();
    options.insert(
        "npm".to_string(),
        serde_json::Value::String(profile.npm.clone()),
    );
    options.insert(
        "runtime_adapter".to_string(),
        serde_json::Value::String(adapter.to_string()),
    );

    let base_url = provider_base_url(provider).unwrap_or_default();

    let api_key = provider_secret(provider, &fallback_env)?;

    Some(ProviderConfig {
        provider_id: provider_id.to_string(),
        base_url,
        api_key,
        headers,
        options,
    })
}

#[cfg(feature = "http-transport")]
fn create_protocol_provider(
    provider_id: &str,
    provider: &ProviderState,
) -> Option<Arc<dyn RuntimeProvider>> {
    let npm = resolve_npm_for_provider(provider_id, provider);
    let provider_profile =
        match ProviderProfileResolver::try_resolve_with_npm(provider_id, &npm, &provider.options) {
            Ok(profile) => profile,
            Err(error) => {
                tracing::warn!(
                    provider = provider_id,
                    error = %error,
                    "provider profile validation failed, skipping provider"
                );
                return None;
            }
        };
    let adapter = ProviderRuntimeAdapter::from_profile(&provider_profile);
    let mut config =
        provider_config_for_adapter(provider_id, provider, &provider_profile, adapter)?;

    let runtime_config = build_runtime_config(&config.options);
    config.options.insert(
        "runtime_enabled".to_string(),
        serde_json::Value::Bool(runtime_config.enabled),
    );
    config.options.insert(
        "runtime_preflight".to_string(),
        serde_json::Value::Bool(runtime_config.preflight_enabled),
    );

    let provider_adapter = create_provider_adapter_for_profile(&provider_profile);
    let models: HashMap<String, RuntimeModelInfo> = provider
        .models
        .values()
        .map(|model| (model.id.clone(), state_model_to_runtime(provider_id, model)))
        .collect();

    let mut instance = ProviderInstance::new(
        provider_id.to_string(),
        provider.name.clone(),
        config,
        provider_adapter,
        models,
    )
    .with_provider_profile_fingerprint(crate::cache::ProviderProfileFingerprint::from_profile(
        &provider_profile,
    ))
    .with_api_shape(provider_profile.api_shape);

    if runtime_config.enabled {
        instance = instance.with_runtime(ProviderRuntime::new(runtime_config));
    }

    Some(Arc::new(instance))
}

#[cfg(not(feature = "http-transport"))]
fn create_protocol_provider(
    _provider_id: &str,
    _provider: &ProviderState,
) -> Option<Arc<dyn RuntimeProvider>> {
    None
}

pub(super) fn create_concrete_provider(
    provider_id: &str,
    provider: &ProviderState,
) -> Option<Arc<dyn RuntimeProvider>> {
    create_protocol_provider(provider_id, provider)
}

struct AliasedProvider {
    id: String,
    name: String,
    inner: Arc<dyn RuntimeProvider>,
    models: Vec<RuntimeModelInfo>,
    model_index: HashMap<String, RuntimeModelInfo>,
}

impl AliasedProvider {
    fn new(
        id: String,
        name: String,
        inner: Arc<dyn RuntimeProvider>,
        models: Vec<RuntimeModelInfo>,
    ) -> Self {
        let model_index = models
            .iter()
            .map(|model| (model.id.clone(), model.clone()))
            .collect();
        Self {
            id,
            name,
            inner,
            models,
            model_index,
        }
    }
}

#[async_trait]
impl RuntimeProvider for AliasedProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn provider_profile_fingerprint(&self) -> Option<crate::cache::ProviderProfileFingerprint> {
        self.inner.provider_profile_fingerprint()
    }

    fn api_shape(&self) -> Option<crate::ProviderApiShape> {
        self.inner.api_shape()
    }

    fn models(&self) -> Vec<RuntimeModelInfo> {
        self.models.clone()
    }

    fn get_model(&self, id: &str) -> Option<&RuntimeModelInfo> {
        self.model_index.get(id)
    }

    async fn chat(
        &self,
        request: crate::ChatRequest,
    ) -> Result<crate::ChatResponse, crate::ProviderError> {
        self.inner.chat(request).await
    }

    async fn chat_stream(
        &self,
        request: crate::ChatRequest,
    ) -> Result<crate::StreamResult, crate::ProviderError> {
        self.inner.chat_stream(request).await
    }
}

fn state_model_to_runtime(provider_id: &str, model: &ProviderModel) -> RuntimeModelInfo {
    RuntimeModelInfo {
        id: model.id.clone(),
        name: model.name.clone(),
        provider: provider_id.to_string(),
        context_window: model.limit.context,
        max_input_tokens: model.limit.input,
        max_output_tokens: model.limit.output,
        supports_vision: model.capabilities.input.image
            || model.capabilities.output.image
            || model.capabilities.input.video
            || model.capabilities.output.video,
        supports_tools: model.capabilities.toolcall,
        cost_per_million_input: model.cost.input,
        cost_per_million_output: model.cost.output,
        cost_per_million_cache_read: Some(model.cost.cache.read),
        cost_per_million_cache_write: Some(model.cost.cache.write),
    }
}

pub(super) fn wrap_provider_for_state(
    provider_state: &ProviderState,
    provider: Arc<dyn RuntimeProvider>,
) -> Arc<dyn RuntimeProvider> {
    let should_wrap = provider_state.id != provider.id()
        || provider_state.name != provider.name()
        || !provider_state.models.is_empty();

    if !should_wrap {
        return provider;
    }

    let models = if provider_state.models.is_empty() {
        provider.models()
    } else {
        provider_state
            .models
            .values()
            .map(|model| state_model_to_runtime(&provider_state.id, model))
            .collect()
    };

    Arc::new(AliasedProvider::new(
        provider_state.id.clone(),
        provider_state.name.clone(),
        provider,
        models,
    ))
}

#[cfg(feature = "http-transport")]
pub(super) fn load_models_dev_cache() -> ModelsData {
    load_default_catalog_data_sync()
}

#[cfg(not(feature = "http-transport"))]
pub(super) fn load_models_dev_cache() -> ModelsData {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderApiShape;

    fn provider_state_with_profile(api_shape: &str) -> ProviderState {
        let mut options = HashMap::new();
        options.insert(
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "openai-compatible",
                "api_shape": api_shape,
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens"
            }),
        );

        ProviderState {
            id: "my-custom".to_string(),
            name: "my-custom".to_string(),
            source: "config".to_string(),
            env: Vec::new(),
            key: Some("test-key".to_string()),
            options,
            models: HashMap::new(),
        }
    }

    #[test]
    fn openai_responses_profile_keeps_declared_api_shape() {
        let provider = provider_state_with_profile("responses");
        let profile = ProviderProfileResolver::try_resolve("my-custom", &provider)
            .expect("profile should resolve");
        let adapter = ProviderRuntimeAdapter::from_profile(&profile);

        let config = provider_config_for_adapter("my-custom", &provider, &profile, adapter)
            .expect("config should resolve");

        assert_eq!(profile.api_shape, ProviderApiShape::Responses);
        assert_eq!(adapter, ProviderRuntimeAdapter::OpenAiCompatible);
        assert_eq!(
            config
                .options
                .get("npm")
                .and_then(serde_json::Value::as_str),
            Some("@ai-sdk/openai-compatible")
        );
    }

    #[test]
    fn openai_chat_completions_profile_keeps_declared_api_shape() {
        let provider = provider_state_with_profile("chat-completions");
        let profile = ProviderProfileResolver::try_resolve("my-custom", &provider)
            .expect("profile should resolve");
        let adapter = ProviderRuntimeAdapter::from_profile(&profile);

        let config = provider_config_for_adapter("my-custom", &provider, &profile, adapter)
            .expect("config should resolve");

        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert_eq!(
            config
                .options
                .get("runtime_adapter")
                .and_then(serde_json::Value::as_str),
            Some("openai-compatible")
        );
    }
}
