use crate::models::ModelsData;
use crate::profile::ProviderProfile;
use crate::protocol::{ProviderConfig, ProviderRuntimeAdapter};
use crate::provider::{
    ModelInfo as RuntimeModelInfo, Provider as RuntimeProvider, ProviderRegistry,
};
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
use crate::protocol_loader::{ProtocolLoader, ProtocolManifest};
#[cfg(feature = "http-transport")]
use crate::protocol_validator::ProtocolValidator;
#[cfg(feature = "http-transport")]
use crate::protocols::create_provider_adapter_for_profile;
#[cfg(feature = "http-transport")]
use crate::runtime::{Pipeline, ProtocolSource, ProviderRuntime, RuntimeContext};
#[cfg(feature = "http-transport")]
use std::time::Instant;

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

fn provider_option_string(provider: &ProviderState, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = options_get_insensitive(&provider.options, key) else {
            continue;
        };
        match value {
            serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
            serde_json::Value::Number(n) => return Some(n.to_string()),
            serde_json::Value::Bool(b) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}

pub(super) fn options_get_insensitive<'a>(
    options: &'a HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(value) = options.get(key) {
        return Some(value);
    }
    let key_lower = key.to_lowercase();
    options
        .iter()
        .find_map(|(name, value)| (name.to_lowercase() == key_lower).then_some(value))
}

fn provider_secret(provider: &ProviderState, fallback_env: &[&str]) -> Option<String> {
    provider_option_string(provider, &["apiKey", "api_key", "apikey"])
        .or_else(|| provider.key.clone().filter(|key| !key.trim().is_empty()))
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
    provider_option_string(provider, &["baseURL", "baseUrl", "url", "api"])
        .or_else(|| {
            provider
                .models
                .values()
                .find_map(|model| (!model.api.url.trim().is_empty()).then(|| model.api.url.clone()))
        })
        .or_else(|| {
            if provider.id == "zhipuai-coding-plan" {
                Some("https://open.bigmodel.cn/api/coding/paas/v4".to_string())
            } else {
                None
            }
        })
}

fn default_secret_env_for_provider(
    provider_id: &str,
    adapter: ProviderRuntimeAdapter,
) -> Vec<&'static str> {
    match adapter {
        ProviderRuntimeAdapter::Ethnopic => vec!["ANTHROPIC_API_KEY"],
        ProviderRuntimeAdapter::Gemini => vec!["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"],
        ProviderRuntimeAdapter::BedrockConverse => vec!["AWS_ACCESS_KEY_ID"],
        ProviderRuntimeAdapter::VertexGemini => vec![
            "GOOGLE_VERTEX_ACCESS_TOKEN",
            "GOOGLE_CLOUD_ACCESS_TOKEN",
            "GOOGLE_OAUTH_ACCESS_TOKEN",
            "GCP_ACCESS_TOKEN",
        ],
        ProviderRuntimeAdapter::GitHubCopilotCloseAi => vec!["GITHUB_COPILOT_TOKEN"],
        ProviderRuntimeAdapter::GitLabCloseAi => vec!["GITLAB_TOKEN"],
        ProviderRuntimeAdapter::CloseAiCompatible => match provider_id {
            "openai" => vec!["OPENAI_API_KEY"],
            "opencode" => vec!["AGENDAO_API_KEY"],
            "openrouter" => vec!["OPENROUTER_API_KEY"],
            "mistral" => vec!["MISTRAL_API_KEY"],
            "groq" => vec!["GROQ_API_KEY"],
            "deepinfra" => vec!["DEEPINFRA_API_KEY"],
            "deepseek" => vec!["DEEPSEEK_API_KEY"],
            "xai" => vec!["XAI_API_KEY"],
            "cerebras" => vec!["CEREBRAS_API_KEY"],
            "cohere" => vec!["COHERE_API_KEY"],
            "together" | "togetherai" => vec!["TOGETHER_API_KEY", "TOGETHERAI_API_KEY"],
            "perplexity" => vec!["PERPLEXITY_API_KEY"],
            "vercel" => vec!["VERCEL_API_KEY"],
            _ => vec![],
        },
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

fn option_string(options: &HashMap<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = options.get(*key) else {
            continue;
        };
        match value {
            serde_json::Value::String(v) if !v.trim().is_empty() => return Some(v.clone()),
            serde_json::Value::Number(v) => return Some(v.to_string()),
            serde_json::Value::Bool(v) => return Some(v.to_string()),
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

fn env_string(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            if !raw.trim().is_empty() {
                return Some(raw);
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
        pipeline_enabled: option_bool(options, &["runtime_pipeline", "pipeline_enabled"])
            .or_else(|| env_bool(&["AGENDAO_RUNTIME_PIPELINE"]))
            .unwrap_or(defaults.pipeline_enabled),
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
        protocol_path: option_string(options, &["protocol_path", "runtime_protocol_path"])
            .or_else(|| env_string(&["AGENDAO_RUNTIME_PROTOCOL_PATH"])),
        protocol_version: option_string(options, &["protocol_version", "runtime_protocol_version"])
            .or_else(|| env_string(&["AGENDAO_RUNTIME_PROTOCOL_VERSION"])),
        hot_reload: option_bool(options, &["hot_reload", "runtime_hot_reload"])
            .or_else(|| env_bool(&["AGENDAO_RUNTIME_HOT_RELOAD"]))
            .unwrap_or(defaults.hot_reload),
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

    let api_key = match adapter {
        ProviderRuntimeAdapter::BedrockConverse => {
            let access_key_id = provider_option_string(provider, &["accessKeyId", "access_key_id"])
                .or_else(|| env_any(&["AWS_ACCESS_KEY_ID"]))
                .or_else(|| provider_secret(provider, &fallback_env))?;
            let secret =
                provider_option_string(provider, &["secretAccessKey", "secret_access_key"])
                    .or_else(|| env_any(&["AWS_SECRET_ACCESS_KEY"]))?;
            let region = provider_option_string(provider, &["region"])
                .or_else(|| env_any(&["AWS_REGION"]))
                .unwrap_or_else(|| "us-east-1".to_string());
            options.insert(
                "access_key_id".to_string(),
                serde_json::Value::String(access_key_id.clone()),
            );
            options.insert(
                "secret_access_key".to_string(),
                serde_json::Value::String(secret),
            );
            options.insert("region".to_string(), serde_json::Value::String(region));
            if let Some(session_token) =
                provider_option_string(provider, &["sessionToken", "session_token"])
                    .or_else(|| env_any(&["AWS_SESSION_TOKEN"]))
            {
                options.insert(
                    "session_token".to_string(),
                    serde_json::Value::String(session_token),
                );
            }
            access_key_id
        }
        ProviderRuntimeAdapter::VertexGemini => {
            let token = provider_option_string(provider, &["accessToken", "access_token", "token"])
                .or_else(|| provider_secret(provider, &fallback_env))?;
            let project = provider_option_string(provider, &["project", "projectId", "project_id"])
                .or_else(|| env_any(&["GOOGLE_CLOUD_PROJECT", "GCP_PROJECT", "GCLOUD_PROJECT"]))?;
            let location = provider_option_string(provider, &["location"])
                .or_else(|| env_any(&["GOOGLE_CLOUD_LOCATION", "VERTEX_LOCATION"]))
                .unwrap_or_else(|| "us-east5".to_string());
            options.insert("project".to_string(), serde_json::Value::String(project));
            options.insert("location".to_string(), serde_json::Value::String(location));
            token
        }
        _ => provider_secret(provider, &fallback_env)?,
    };

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

    let manifest: Option<ProtocolManifest> = ProtocolLoader::new()
        .try_load_provider(provider_id, &config.options)
        .and_then(|manifest| match ProtocolValidator::validate(&manifest) {
            Ok(()) => Some(manifest),
            Err(err) => {
                tracing::warn!(
                    provider = provider_id,
                    error = %err,
                    "protocol manifest validation failed, using built-in adapter routing"
                );
                None
            }
        });

    if let Some(manifest) = &manifest {
        if config.base_url.trim().is_empty() && !manifest.endpoint.base_url.trim().is_empty() {
            config.base_url = manifest.endpoint.base_url.clone();
        }
        config.options.insert(
            "runtime_manifest_id".to_string(),
            serde_json::Value::String(manifest.id.clone()),
        );
        config.options.insert(
            "runtime_manifest_version".to_string(),
            serde_json::Value::String(manifest.protocol_version.clone()),
        );
    }

    let mut runtime_config = build_runtime_config(&config.options);
    if runtime_config.protocol_version.is_none() {
        if let Some(manifest) = &manifest {
            runtime_config.protocol_version = Some(manifest.protocol_version.clone());
        }
    }
    config.options.insert(
        "runtime_enabled".to_string(),
        serde_json::Value::Bool(runtime_config.enabled),
    );
    config.options.insert(
        "runtime_preflight".to_string(),
        serde_json::Value::Bool(runtime_config.preflight_enabled),
    );
    config.options.insert(
        "runtime_pipeline".to_string(),
        serde_json::Value::Bool(runtime_config.pipeline_enabled),
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
    ));

    if runtime_config.enabled {
        let protocol_source = if let Some(manifest) = &manifest {
            ProtocolSource::Manifest {
                path: runtime_config
                    .protocol_path
                    .clone()
                    .unwrap_or_else(|| "env/auto".to_string()),
                version: runtime_config
                    .protocol_version
                    .clone()
                    .unwrap_or_else(|| manifest.protocol_version.clone()),
            }
        } else {
            ProtocolSource::BuiltinAdapter { npm: npm.clone() }
        };

        let context = RuntimeContext {
            protocol_source,
            provider_id: provider_id.to_string(),
            created_at: Instant::now(),
        };
        let mut runtime = ProviderRuntime::new(runtime_config.clone(), context);
        if runtime.is_pipeline_enabled() {
            let pipeline = match manifest.as_ref() {
                Some(manifest) => Pipeline::from_manifest(manifest).unwrap_or_else(|err| {
                    tracing::error!(
                        provider = provider_id,
                        protocol_path = runtime_config.protocol_path.as_deref().unwrap_or("env/auto"),
                        protocol_version = runtime_config
                            .protocol_version
                            .as_deref()
                            .unwrap_or(manifest.protocol_version.as_str()),
                        error = %err,
                        "failed to build runtime pipeline from manifest, using provider defaults"
                    );
                    Pipeline::for_profile(&provider_profile)
                }),
                None => Pipeline::for_profile(&provider_profile),
            };
            runtime.set_pipeline(Arc::new(pipeline));
        }
        instance = instance.with_runtime(runtime);
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

#[cfg(feature = "http-transport")]
pub(super) fn register_fallback_env_providers(registry: &mut ProviderRegistry) {
    let fallback: Vec<(&str, Vec<&str>)> = vec![
        ("ethnopic", vec!["ANTHROPIC_API_KEY"]),
        ("openai", vec!["OPENAI_API_KEY"]),
        (
            "google",
            vec!["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"],
        ),
        (
            "amazon-bedrock",
            vec!["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"],
        ),
        ("openrouter", vec!["OPENROUTER_API_KEY"]),
        ("mistral", vec!["MISTRAL_API_KEY"]),
        ("groq", vec!["GROQ_API_KEY"]),
        ("deepseek", vec!["DEEPSEEK_API_KEY"]),
        ("xai", vec!["XAI_API_KEY"]),
        ("cerebras", vec!["CEREBRAS_API_KEY"]),
        ("cohere", vec!["COHERE_API_KEY"]),
        ("deepinfra", vec!["DEEPINFRA_API_KEY"]),
        ("together", vec!["TOGETHER_API_KEY", "TOGETHERAI_API_KEY"]),
        ("perplexity", vec!["PERPLEXITY_API_KEY"]),
        ("vercel", vec!["VERCEL_API_KEY"]),
        ("gitlab", vec!["GITLAB_TOKEN"]),
        ("github-copilot", vec!["GITHUB_COPILOT_TOKEN"]),
        (
            "google-vertex",
            vec![
                "GOOGLE_VERTEX_ACCESS_TOKEN",
                "GOOGLE_CLOUD_ACCESS_TOKEN",
                "GOOGLE_OAUTH_ACCESS_TOKEN",
                "GCP_ACCESS_TOKEN",
            ],
        ),
    ];

    for (provider_id, env_keys) in fallback {
        let state = ProviderState {
            id: provider_id.to_string(),
            name: provider_id.to_string(),
            source: "env".to_string(),
            env: env_keys.into_iter().map(|key| key.to_string()).collect(),
            key: None,
            options: HashMap::new(),
            models: HashMap::new(),
        };
        if let Some(provider) = create_concrete_provider(provider_id, &state) {
            registry.register_arc(provider);
        }
    }
}

#[cfg(not(feature = "http-transport"))]
pub(super) fn register_fallback_env_providers(_registry: &mut ProviderRegistry) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderApiShape;

    fn provider_state_with_profile(api_shape: &str) -> ProviderState {
        let mut options = HashMap::new();
        options.insert(
            "providerProfile".to_string(),
            serde_json::json!({
                "api_style": "closeai-compatible",
                "api_shape": api_shape,
                "transport": "bearer",
                "usage_shape": "closeai-cached-tokens"
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
    fn closeai_responses_profile_keeps_declared_api_shape() {
        let provider = provider_state_with_profile("responses");
        let profile = ProviderProfileResolver::try_resolve("my-custom", &provider)
            .expect("profile should resolve");
        let adapter = ProviderRuntimeAdapter::from_profile(&profile);

        let config = provider_config_for_adapter("my-custom", &provider, &profile, adapter)
            .expect("config should resolve");

        assert_eq!(profile.api_shape, ProviderApiShape::Responses);
        assert_eq!(adapter, ProviderRuntimeAdapter::CloseAiCompatible);
        assert_eq!(
            config
                .options
                .get("npm")
                .and_then(serde_json::Value::as_str),
            Some("@ai-sdk/openai-compatible")
        );
    }

    #[test]
    fn closeai_chat_completions_profile_keeps_declared_api_shape() {
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
            Some("closeai-compatible")
        );
    }
}
