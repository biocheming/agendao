use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("Model not found: provider={provider_id} model={model_id}")]
    ModelNotFound {
        provider_id: String,
        model_id: String,
        suggestions: Vec<String>,
    },

    #[error("Provider initialization failed: {provider_id}")]
    InitError {
        provider_id: String,
        #[source]
        cause: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

/// Map of bundled SDK package names to their provider identifiers.
/// Mirrors the TS `BUNDLED_PROVIDERS` record.
pub static BUNDLED_PROVIDERS: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert("@ai-sdk/amazon-bedrock", "amazon-bedrock");
    map.insert("@ai-sdk/anthropic", "ethnopic");
    map.insert("@ai-sdk/azure", "azure");
    map.insert("@ai-sdk/google", "google");
    map.insert("@ai-sdk/google-vertex", "google-vertex");
    map.insert("@ai-sdk/google-vertex/anthropic", "google-vertex-ethnopic");
    map.insert("@ai-sdk/openai", "openai");
    map.insert("@ai-sdk/openai-compatible", "openai-compatible");
    map.insert("@openrouter/ai-sdk-provider", "openrouter");
    map.insert("@ai-sdk/xai", "xai");
    map.insert("@ai-sdk/mistral", "mistral");
    map.insert("@ai-sdk/groq", "groq");
    map.insert("@ai-sdk/deepinfra", "deepinfra");
    map.insert("@ai-sdk/cerebras", "cerebras");
    map.insert("@ai-sdk/cohere", "cohere");
    map.insert("@ai-sdk/gateway", "gateway");
    map.insert("@ai-sdk/togetherai", "togetherai");
    map.insert("@ai-sdk/perplexity", "perplexity");
    map.insert("@ai-sdk/vercel", "vercel");
    map.insert("@gitlab/gitlab-ai-provider", "gitlab");
    map.insert("@ai-sdk/github-copilot", "github-copilot");
    map
});

/// Check if a model ID represents GPT-5 or later.
pub fn is_gpt5_or_later(model_id: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^gpt-(\d+)").unwrap());
    if let Some(caps) = RE.captures(model_id) {
        if let Some(num) = caps.get(1) {
            if let Ok(n) = num.as_str().parse::<u32>() {
                return n >= 5;
            }
        }
    }
    false
}

/// Determine whether to use the Copilot responses API for a given model.
pub fn should_use_copilot_responses_api(model_id: &str) -> bool {
    is_gpt5_or_later(model_id) && !model_id.starts_with("gpt-5-mini")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub temperature: bool,
    pub reasoning: bool,
    pub attachment: bool,
    pub toolcall: bool,
    pub input: ModalitySet,
    pub output: ModalitySet,
    pub interleaved: InterleavedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModalitySet {
    pub text: bool,
    pub audio: bool,
    pub image: bool,
    pub video: bool,
    pub pdf: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InterleavedConfig {
    Bool(bool),
    Field { field: String },
}

impl Default for InterleavedConfig {
    fn default() -> Self {
        Self::Bool(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostCache {
    pub read: f64,
    pub write: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostOver200K {
    pub input: f64,
    pub output: f64,
    pub cache: ModelCostCache,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelCost {
    pub input: f64,
    pub output: f64,
    pub cache: ModelCostCache,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_over_200k: Option<ModelCostOver200K>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelLimit {
    pub context: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelApi {
    pub id: String,
    pub url: String,
    pub npm: String,
}

/// Runtime model type matching TS `Provider.Model`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub provider_id: String,
    pub api: ProviderModelApi,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    pub capabilities: ModelCapabilities,
    pub cost: ProviderModelCost,
    pub limit: ProviderModelLimit,
    pub status: String,
    pub options: HashMap<String, serde_json::Value>,
    pub headers: HashMap<String, String>,
    pub release_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

/// Runtime provider type matching TS `Provider.Info`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderState {
    pub id: String,
    pub name: String,
    pub source: String,
    pub env: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub options: HashMap<String, serde_json::Value>,
    pub models: HashMap<String, ProviderModel>,
}

/// Configuration for a single model from the config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModel {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub temperature: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub attachment: Option<bool>,
    #[serde(default)]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub interleaved: Option<InterleavedConfig>,
    #[serde(default)]
    pub cost: Option<ConfigModelCost>,
    #[serde(default)]
    pub limit: Option<ConfigModelLimit>,
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub modalities: Option<ConfigModalities>,
    #[serde(default)]
    pub provider: Option<ConfigModelProvider>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModelCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModelLimit {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModalities {
    #[serde(default)]
    pub input: Option<Vec<String>>,
    #[serde(default)]
    pub output: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModelProvider {
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
}

/// Configuration for a single provider from the config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigProvider {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub env: Option<Vec<String>>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default, alias = "apiStyle", alias = "api_family", alias = "apiFamily")]
    pub api_style: Option<String>,
    #[serde(default, alias = "apiShape")]
    pub api_shape: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default, alias = "usageShape")]
    pub usage_shape: Option<String>,
    #[serde(default)]
    pub quirks: Option<Vec<String>>,
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub models: Option<HashMap<String, ConfigModel>>,
    #[serde(default)]
    pub blacklist: Option<Vec<String>>,
    #[serde(default)]
    pub whitelist: Option<Vec<String>>,
}

/// Top-level bootstrap configuration.
#[derive(Debug, Clone, Default)]
pub struct BootstrapConfig {
    pub providers: HashMap<String, ConfigProvider>,
    pub disabled_providers: HashSet<String>,
    pub enabled_providers: Option<HashSet<String>>,
    pub enable_experimental: bool,
    pub model: Option<String>,
    pub small_model: Option<String>,
}
