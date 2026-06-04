use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::oauth::ProviderAuth;
use crate::{ApiError, Result, ServerState};
use agendao_config::ModelConfig;
use agendao_provider::{
    provider_connection_descriptor_candidate_from_config_provider, AuthInfo, AuthMethodType,
    ConfigProvider, ProviderConnectionDescriptorCandidate, ProviderDescriptorError,
    ProviderProfileError,
};
use agendao_provider::{CatalogRefreshStatus, CatalogSnapshot, ModelsData, ModelsDevInfo};
use agendao_types::{
    ConfigPolicyValidationEffect, ConfigPolicyValidationItem, ConfigPolicyValidationOwner,
    ConfigPolicyValidationScope, ConfigPolicyValidationScopeKind, ConfigPolicyValidationSeverity,
};

pub(crate) fn provider_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_providers))
        .route("/refresh", post(refresh_provider_catalog))
        .route("/managed", get(list_managed_providers))
        .route("/known", get(list_known_providers))
        .route("/connect/schema", get(get_provider_connect_schema))
        .route("/connect/resolve", post(resolve_provider_connect))
        .route("/connect", post(connect_provider))
        .route("/register", post(register_custom_provider))
        .route("/auth", get(get_provider_auth))
        .route("/{id}/descriptor", get(get_provider_descriptor))
        .route("/{id}", put(update_provider).delete(delete_provider))
        .route("/{id}/oauth/authorize", post(oauth_authorize))
        .route("/{id}/oauth/callback", post(oauth_callback))
}

#[derive(Debug, Serialize)]
pub struct ProviderListResponse {
    pub all: Vec<ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
    pub connected: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
pub struct ManagedProvidersResponse {
    pub providers: Vec<ManagedProviderInfo>,
}

#[derive(Debug, Serialize)]
pub struct ManagedProviderInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub connected: bool,
    pub has_auth: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    pub configured: bool,
    pub known: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    pub known_model_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor_candidate: Option<ProviderConnectionDescriptorCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor_candidate_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_overrides: Vec<ManagedModelOverrideInfo>,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
pub struct ProviderDescriptorResponse {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor_candidate: Option<ProviderConnectionDescriptorCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor_candidate_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ManagedModelOverrideInfo {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<agendao_config::ModelInterleavedConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_million_input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_million_output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ModelCapabilityInfo>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelCapabilityInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,
    #[serde(default, skip_serializing_if = "ModelModalityInfo::is_empty")]
    pub input: ModelModalityInfo,
    #[serde(default, skip_serializing_if = "ModelModalityInfo::is_empty")]
    pub output: ModelModalityInfo,
}

impl ModelCapabilityInfo {
    fn is_empty(&self) -> bool {
        self.attachment.is_none()
            && self.tool_call.is_none()
            && self.reasoning.is_none()
            && self.temperature.is_none()
            && self.input.is_empty()
            && self.output.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelModalityInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf: Option<bool>,
}

impl ModelModalityInfo {
    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.audio.is_none()
            && self.image.is_none()
            && self.video.is_none()
            && self.pdf.is_none()
    }
}

fn modality_contains(values: &[String], needle: &str) -> bool {
    values.iter().any(|value| value == needle)
}

fn modality_info_from_values(values: Option<&[String]>, fallback_text: bool) -> ModelModalityInfo {
    let Some(values) = values else {
        return ModelModalityInfo {
            text: fallback_text.then_some(true),
            ..Default::default()
        };
    };

    ModelModalityInfo {
        text: Some(fallback_text || modality_contains(values, "text")),
        audio: Some(modality_contains(values, "audio")),
        image: Some(modality_contains(values, "image")),
        video: Some(modality_contains(values, "video")),
        pdf: Some(modality_contains(values, "pdf")),
    }
}

fn capability_info_from_catalog(model: &ModelsDevInfo) -> Option<ModelCapabilityInfo> {
    let input = modality_info_from_values(
        model
            .modalities
            .as_ref()
            .map(|modalities| modalities.input.as_ref()),
        true,
    );
    let output = modality_info_from_values(
        model
            .modalities
            .as_ref()
            .map(|modalities| modalities.output.as_ref()),
        true,
    );
    let capability = ModelCapabilityInfo {
        attachment: Some(model.attachment),
        tool_call: Some(model.tool_call),
        reasoning: Some(model.reasoning),
        temperature: Some(model.temperature),
        input,
        output,
    };
    (!capability.is_empty()).then_some(capability)
}

fn capability_info_from_runtime(
    model: &agendao_provider::ModelInfo,
) -> Option<ModelCapabilityInfo> {
    let capability = ModelCapabilityInfo {
        tool_call: Some(model.supports_tools),
        input: ModelModalityInfo {
            text: Some(true),
            image: Some(model.supports_vision),
            ..Default::default()
        },
        output: ModelModalityInfo {
            text: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    (!capability.is_empty()).then_some(capability)
}

fn capability_info_from_config(configured_model: &ModelConfig) -> Option<ModelCapabilityInfo> {
    let input = modality_info_from_values(
        configured_model
            .modalities
            .as_ref()
            .and_then(|modalities| modalities.input.as_deref()),
        false,
    );
    let output = modality_info_from_values(
        configured_model
            .modalities
            .as_ref()
            .and_then(|modalities| modalities.output.as_deref()),
        false,
    );
    let capability = ModelCapabilityInfo {
        attachment: configured_model.attachment,
        tool_call: configured_model.tool_call,
        reasoning: configured_model.reasoning,
        temperature: configured_model.temperature,
        input,
        output,
    };
    (!capability.is_empty()).then_some(capability)
}

fn fill_missing_bool(target: &mut Option<bool>, incoming: Option<bool>) {
    if target.is_none() {
        *target = incoming;
    }
}

fn override_bool(target: &mut Option<bool>, incoming: Option<bool>) {
    if incoming.is_some() {
        *target = incoming;
    }
}

fn merge_fill_missing_modalities(existing: &mut ModelModalityInfo, incoming: &ModelModalityInfo) {
    fill_missing_bool(&mut existing.text, incoming.text);
    fill_missing_bool(&mut existing.audio, incoming.audio);
    fill_missing_bool(&mut existing.image, incoming.image);
    fill_missing_bool(&mut existing.video, incoming.video);
    fill_missing_bool(&mut existing.pdf, incoming.pdf);
}

fn merge_override_modalities(existing: &mut ModelModalityInfo, incoming: &ModelModalityInfo) {
    override_bool(&mut existing.text, incoming.text);
    override_bool(&mut existing.audio, incoming.audio);
    override_bool(&mut existing.image, incoming.image);
    override_bool(&mut existing.video, incoming.video);
    override_bool(&mut existing.pdf, incoming.pdf);
}

fn merge_fill_missing_capabilities(
    existing: &mut ModelCapabilityInfo,
    incoming: &ModelCapabilityInfo,
) {
    fill_missing_bool(&mut existing.attachment, incoming.attachment);
    fill_missing_bool(&mut existing.tool_call, incoming.tool_call);
    fill_missing_bool(&mut existing.reasoning, incoming.reasoning);
    fill_missing_bool(&mut existing.temperature, incoming.temperature);
    merge_fill_missing_modalities(&mut existing.input, &incoming.input);
    merge_fill_missing_modalities(&mut existing.output, &incoming.output);
}

fn merge_override_capabilities(existing: &mut ModelCapabilityInfo, incoming: &ModelCapabilityInfo) {
    override_bool(&mut existing.attachment, incoming.attachment);
    override_bool(&mut existing.tool_call, incoming.tool_call);
    override_bool(&mut existing.reasoning, incoming.reasoning);
    override_bool(&mut existing.temperature, incoming.temperature);
    merge_override_modalities(&mut existing.input, &incoming.input);
    merge_override_modalities(&mut existing.output, &incoming.output);
}

fn config_context_window(configured_model: &ModelConfig) -> Option<u64> {
    configured_model
        .limit
        .as_ref()
        .and_then(|limit| limit.context)
}

fn config_max_output_tokens(configured_model: &ModelConfig) -> Option<u64> {
    configured_model
        .limit
        .as_ref()
        .and_then(|limit| limit.output)
}

fn config_input_price(configured_model: &ModelConfig) -> Option<f64> {
    configured_model.cost.as_ref().and_then(|cost| cost.input)
}

fn config_output_price(configured_model: &ModelConfig) -> Option<f64> {
    configured_model.cost.as_ref().and_then(|cost| cost.output)
}

pub(crate) fn catalog_model_info(
    provider_id: &str,
    model: &ModelsDevInfo,
    variants: Vec<String>,
) -> ModelInfo {
    ModelInfo {
        id: model.id.clone(),
        name: model.name.clone(),
        provider: provider_id.to_string(),
        variants,
        context_window: Some(model.limit.context),
        max_output_tokens: Some(model.limit.output),
        cost_per_million_input: model.cost.as_ref().map(|cost| cost.input),
        cost_per_million_output: model.cost.as_ref().map(|cost| cost.output),
        capabilities: capability_info_from_catalog(model),
    }
}

pub(crate) fn runtime_model_info(
    model: &agendao_provider::ModelInfo,
    variants: Vec<String>,
) -> ModelInfo {
    ModelInfo {
        id: model.id.clone(),
        name: model.name.clone(),
        provider: model.provider.clone(),
        variants,
        context_window: Some(model.context_window),
        max_output_tokens: Some(model.max_output_tokens),
        cost_per_million_input: Some(model.cost_per_million_input),
        cost_per_million_output: Some(model.cost_per_million_output),
        capabilities: capability_info_from_runtime(model),
    }
}

pub(crate) fn configured_model_info(
    provider_id: &str,
    model_id: String,
    configured_model: &ModelConfig,
    variants: Vec<String>,
) -> ModelInfo {
    ModelInfo {
        id: model_id.clone(),
        name: configured_model
            .name
            .clone()
            .unwrap_or_else(|| model_id.clone()),
        provider: provider_id.to_string(),
        variants,
        context_window: config_context_window(configured_model),
        max_output_tokens: config_max_output_tokens(configured_model),
        cost_per_million_input: config_input_price(configured_model),
        cost_per_million_output: config_output_price(configured_model),
        capabilities: capability_info_from_config(configured_model),
    }
}

fn merge_catalog_model_info(existing: &mut ModelInfo, incoming: ModelInfo) {
    if existing.name.trim().is_empty() {
        existing.name = incoming.name;
    }
    if existing.variants.is_empty() && !incoming.variants.is_empty() {
        existing.variants = incoming.variants;
    }
    if existing.context_window.is_none() {
        existing.context_window = incoming.context_window;
    }
    if existing.max_output_tokens.is_none() {
        existing.max_output_tokens = incoming.max_output_tokens;
    }
    if existing.cost_per_million_input.is_none() {
        existing.cost_per_million_input = incoming.cost_per_million_input;
    }
    if existing.cost_per_million_output.is_none() {
        existing.cost_per_million_output = incoming.cost_per_million_output;
    }
    if let Some(incoming_capabilities) = incoming.capabilities {
        if let Some(existing_capabilities) = existing.capabilities.as_mut() {
            merge_fill_missing_capabilities(existing_capabilities, &incoming_capabilities);
        } else {
            existing.capabilities = Some(incoming_capabilities);
        }
    }
}

fn merge_runtime_model_info(existing: &mut ModelInfo, incoming: ModelInfo) {
    existing.name = incoming.name;
    if !incoming.variants.is_empty() {
        existing.variants = incoming.variants;
    }
    if existing.context_window.is_none() {
        existing.context_window = incoming.context_window;
    }
    if existing.max_output_tokens.is_none() {
        existing.max_output_tokens = incoming.max_output_tokens;
    }
    if existing.cost_per_million_input.is_none() {
        existing.cost_per_million_input = incoming.cost_per_million_input;
    }
    if existing.cost_per_million_output.is_none() {
        existing.cost_per_million_output = incoming.cost_per_million_output;
    }
    if let Some(incoming_capabilities) = incoming.capabilities {
        if let Some(existing_capabilities) = existing.capabilities.as_mut() {
            merge_fill_missing_capabilities(existing_capabilities, &incoming_capabilities);
        } else {
            existing.capabilities = Some(incoming_capabilities);
        }
    }
}

fn merge_config_model_info(existing: &mut ModelInfo, incoming: ModelInfo) {
    existing.name = incoming.name;
    if !incoming.variants.is_empty() {
        existing.variants = incoming.variants;
    }
    if incoming.context_window.is_some() {
        existing.context_window = incoming.context_window;
    }
    if incoming.max_output_tokens.is_some() {
        existing.max_output_tokens = incoming.max_output_tokens;
    }
    if incoming.cost_per_million_input.is_some() {
        existing.cost_per_million_input = incoming.cost_per_million_input;
    }
    if incoming.cost_per_million_output.is_some() {
        existing.cost_per_million_output = incoming.cost_per_million_output;
    }
    if let Some(incoming_capabilities) = incoming.capabilities {
        if let Some(existing_capabilities) = existing.capabilities.as_mut() {
            merge_override_capabilities(existing_capabilities, &incoming_capabilities);
        } else {
            existing.capabilities = Some(incoming_capabilities);
        }
    }
}

fn upsert_catalog_model_info(
    model_map: &mut HashMap<String, HashMap<String, ModelInfo>>,
    provider_id: &str,
    model: ModelInfo,
) {
    match model_map
        .entry(provider_id.to_string())
        .or_default()
        .entry(model.id.clone())
    {
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            merge_catalog_model_info(entry.get_mut(), model);
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(model);
        }
    }
}

pub(crate) fn upsert_runtime_model_info(
    model_map: &mut HashMap<String, HashMap<String, ModelInfo>>,
    provider_id: &str,
    model: ModelInfo,
) {
    match model_map
        .entry(provider_id.to_string())
        .or_default()
        .entry(model.id.clone())
    {
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            merge_runtime_model_info(entry.get_mut(), model);
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(model);
        }
    }
}

pub(crate) fn upsert_config_model_info(
    model_map: &mut HashMap<String, HashMap<String, ModelInfo>>,
    provider_id: &str,
    model: ModelInfo,
) {
    match model_map
        .entry(provider_id.to_string())
        .or_default()
        .entry(model.id.clone())
    {
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            merge_config_model_info(entry.get_mut(), model);
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(model);
        }
    }
}

const CONNECT_PROTOCOL_OPTIONS: &[(&str, &str)] = &[
    ("openai", "OpenAI"),
    ("openrouter", "OpenRouter"),
    ("perplexity", "Perplexity"),
    ("anthropic", "Ethnopic"),
    ("google", "Google"),
    ("bedrock", "Bedrock"),
    ("vertex", "Vertex"),
    ("github-copilot", "GitHub Copilot"),
    ("gitlab", "GitLab"),
];

fn protocol_to_npm(protocol: &str) -> Option<&'static str> {
    match protocol {
        "openai" => Some("@ai-sdk/openai-compatible"),
        "openrouter" => Some("@openrouter/ai-sdk-provider"),
        "perplexity" => Some("@ai-sdk/perplexity"),
        "anthropic" => Some("@ai-sdk/anthropic"),
        "google" => Some("@ai-sdk/google"),
        "bedrock" => Some("@ai-sdk/amazon-bedrock"),
        "vertex" => Some("@ai-sdk/google-vertex"),
        "github-copilot" => Some("@ai-sdk/github-copilot"),
        "gitlab" => Some("@ai-sdk/gitlab"),
        _ => None,
    }
}

fn npm_to_protocol(npm: &str) -> Option<&'static str> {
    match npm {
        "@ai-sdk/openai-compatible" => Some("openai"),
        "@openrouter/ai-sdk-provider" => Some("openrouter"),
        "@ai-sdk/perplexity" => Some("perplexity"),
        "@ai-sdk/anthropic" => Some("anthropic"),
        "@ai-sdk/google" => Some("google"),
        "@ai-sdk/amazon-bedrock" => Some("bedrock"),
        "@ai-sdk/google-vertex" => Some("vertex"),
        "@ai-sdk/github-copilot" => Some("github-copilot"),
        "@ai-sdk/gitlab" => Some("gitlab"),
        _ => None,
    }
}

fn provider_display_name(provider_id: &str, name: &str) -> String {
    if provider_id.eq_ignore_ascii_case("anthropic") || name.eq_ignore_ascii_case("anthropic") {
        "Ethnopic".to_string()
    } else {
        name.to_string()
    }
}

fn search_text_matches(value: &str, query_lower: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    !value.is_empty() && value.contains(query_lower)
}

fn known_provider_match_score(provider: &KnownProviderEntry, query: &str) -> Option<u8> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let query_lower = query.to_ascii_lowercase();
    let id = provider.id.to_ascii_lowercase();
    let name = provider.name.to_ascii_lowercase();

    if id == query_lower {
        return Some(0);
    }
    if name == query_lower {
        return Some(1);
    }
    if id.starts_with(&query_lower) {
        return Some(2);
    }
    if name.starts_with(&query_lower) {
        return Some(3);
    }
    if search_text_matches(&provider.id, &query_lower) {
        return Some(4);
    }
    if search_text_matches(&provider.name, &query_lower) {
        return Some(5);
    }
    if provider
        .env
        .iter()
        .any(|value| search_text_matches(value, &query_lower))
    {
        return Some(6);
    }

    None
}

fn resolve_known_provider_matches(
    providers: &[KnownProviderEntry],
    query: &str,
) -> Vec<KnownProviderEntry> {
    let mut scored = providers
        .iter()
        .filter_map(|provider| {
            known_provider_match_score(provider, query).map(|score| (score, provider.clone()))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(score_a, provider_a), (score_b, provider_b)| {
        score_a
            .cmp(score_b)
            .then_with(|| provider_a.id.cmp(&provider_b.id))
    });

    scored.into_iter().map(|(_, provider)| provider).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConnectDraftMode {
    Known,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnectDraft {
    pub mode: ProviderConnectDraftMode,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub model_count: usize,
    #[serde(default)]
    pub supports_api_key_connect: bool,
}

fn connect_draft_from_known_provider(provider: &KnownProviderEntry) -> ProviderConnectDraft {
    ProviderConnectDraft {
        mode: ProviderConnectDraftMode::Known,
        provider_id: provider.id.clone(),
        known_provider_id: Some(provider.id.clone()),
        name: Some(provider.name.clone()),
        base_url: provider.base_url.clone(),
        protocol: provider.protocol.clone(),
        env: provider.env.clone(),
        connected: provider.connected,
        model_count: provider.model_count,
        supports_api_key_connect: provider.supports_api_key_connect,
    }
}

fn connect_draft_from_custom_query(query: &str) -> ProviderConnectDraft {
    ProviderConnectDraft {
        mode: ProviderConnectDraftMode::Custom,
        provider_id: query.trim().to_string(),
        known_provider_id: None,
        name: None,
        base_url: None,
        protocol: Some("openai".to_string()),
        env: Vec::new(),
        connected: false,
        model_count: 0,
        supports_api_key_connect: true,
    }
}

async fn load_catalog_snapshot(state: &ServerState) -> CatalogSnapshot {
    state.catalog_authority.snapshot().await
}

fn build_model_variant_lookup(data: ModelsData) -> HashMap<String, HashMap<String, Vec<String>>> {
    data.into_iter()
        .map(|(provider_id, provider)| {
            let model_map = provider
                .models
                .into_iter()
                .map(|(model_id, model)| {
                    let mut variants = model
                        .variants
                        .as_ref()
                        .map(|items| items.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    if variants.is_empty() {
                        variants = synthetic_variant_names(&provider_id, &model);
                    }
                    variants.sort();
                    (model_id, variants)
                })
                .collect::<HashMap<_, _>>();
            (provider_id, model_map)
        })
        .collect()
}

/// Detect whether a provider+model pair uses the Ethnopic-compatible protocol family.
///
/// This is a **protocol compatibility check**, not a brand reference.  When users
/// configure an Ethnopic-compatible provider (directly or via Bedrock/Vertex),
/// the thinking variant surface is `["high", "max"]` rather than the OpenAI-style
/// `["low", "medium", "high"]`.
fn is_ethnopic_protocol_family(provider_id: &str) -> bool {
    let provider = provider_id.to_ascii_lowercase();
    provider.contains("anthropic") || provider.contains("ethnopic")
}

fn synthetic_variant_names(provider_id: &str, model: &ModelsDevInfo) -> Vec<String> {
    if !model.reasoning {
        return Vec::new();
    }

    if is_ethnopic_protocol_family(provider_id) {
        return vec!["high".to_string(), "max".to_string()];
    }

    let provider = provider_id.to_ascii_lowercase();
    let model_id = model.id.to_ascii_lowercase();

    let is_google =
        provider.contains("google") || provider.contains("vertex") || model_id.contains("gemini");
    if is_google {
        return vec!["high".to_string(), "max".to_string()];
    }

    vec!["low".to_string(), "medium".to_string(), "high".to_string()]
}

pub(crate) async fn get_model_variant_lookup(
    state: &ServerState,
) -> HashMap<String, HashMap<String, Vec<String>>> {
    let snapshot = load_catalog_snapshot(state).await;
    build_model_variant_lookup(snapshot.data)
}

pub(crate) fn variants_for_model(
    lookup: &HashMap<String, HashMap<String, Vec<String>>>,
    provider_id: &str,
    model_id: &str,
) -> Vec<String> {
    lookup
        .get(provider_id)
        .and_then(|models| models.get(model_id))
        .cloned()
        .unwrap_or_default()
}

pub(crate) async fn list_providers(
    State(state): State<Arc<ServerState>>,
) -> Json<ProviderListResponse> {
    let variant_lookup = get_model_variant_lookup(state.as_ref()).await;
    let models_data = load_catalog_snapshot(state.as_ref()).await.data;

    let providers_guard = state.providers.read().await;
    let connected: std::collections::HashSet<String> = providers_guard
        .list()
        .into_iter()
        .map(|provider| provider.id().to_string())
        .collect();
    let connected_models = providers_guard.list_models();
    drop(providers_guard);

    let mut provider_names: HashMap<String, String> = HashMap::new();
    let mut provider_models: HashMap<String, HashMap<String, ModelInfo>> = HashMap::new();

    // 1) models.dev full provider catalogue.
    for (provider_id, provider) in &models_data {
        provider_names
            .entry(provider_id.clone())
            .or_insert_with(|| provider_display_name(provider_id, &provider.name));
        for model in provider.models.values() {
            let variants = variants_for_model(&variant_lookup, provider_id, &model.id);
            upsert_catalog_model_info(
                &mut provider_models,
                provider_id,
                catalog_model_info(provider_id, model, variants),
            );
        }
    }

    // 2) Config-defined providers/models (even if absent from models.dev).
    let config = state.config_store.config();
    if let Some(configured_providers) = &config.provider {
        for (provider_id, provider) in configured_providers {
            provider_names
                .entry(provider_id.clone())
                .or_insert_with(|| {
                    provider
                        .name
                        .as_deref()
                        .map(|name| provider_display_name(provider_id, name))
                        .unwrap_or_else(|| provider_display_name(provider_id, provider_id))
                });
            if let Some(models) = &provider.models {
                for (configured_model_id, configured) in models {
                    let model_id = configured
                        .model
                        .clone()
                        .unwrap_or_else(|| configured_model_id.clone());
                    let mut variants = configured
                        .variants
                        .as_ref()
                        .map(|items| items.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    if variants.is_empty() {
                        variants = variants_for_model(&variant_lookup, provider_id, &model_id);
                    } else {
                        variants.sort();
                    }
                    upsert_config_model_info(
                        &mut provider_models,
                        provider_id,
                        configured_model_info(provider_id, model_id, configured, variants),
                    );
                }
            }
        }
    }

    // 3) Connected runtime models override names/capabilities-derived variants.
    for model in connected_models {
        let provider_id = model.provider.clone();
        provider_names
            .entry(provider_id.clone())
            .or_insert_with(|| provider_id.clone());
        let variants = variants_for_model(&variant_lookup, &provider_id, &model.id);
        upsert_runtime_model_info(
            &mut provider_models,
            &provider_id,
            runtime_model_info(&model, variants),
        );
    }

    for provider_id in provider_names.keys() {
        provider_models.entry(provider_id.clone()).or_default();
    }

    let mut all: Vec<ProviderInfo> = provider_models
        .into_iter()
        .map(|(id, model_map)| {
            let mut models: Vec<ModelInfo> = model_map.into_values().collect();
            models.sort_by(|a, b| a.id.cmp(&b.id));
            ProviderInfo {
                name: provider_names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| id.clone()),
                id,
                models,
            }
        })
        .collect();
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let mut connected: Vec<String> = connected.into_iter().collect();
    connected.sort();

    let default_model: HashMap<String, String> = all
        .iter()
        .filter_map(|provider| {
            provider
                .models
                .first()
                .map(|model| (provider.id.clone(), model.id.clone()))
        })
        .collect();

    Json(ProviderListResponse {
        all,
        default_model,
        connected,
    })
}

fn managed_provider_status(connected: bool, configured: bool, has_auth: bool) -> &'static str {
    if connected {
        "connected"
    } else if configured && !has_auth {
        "needs-auth"
    } else if has_auth {
        "saved"
    } else {
        "configured"
    }
}

fn managed_provider_auth_type(auth: Option<&AuthInfo>) -> Option<String> {
    match auth {
        Some(AuthInfo::Api { .. }) => Some("api".to_string()),
        Some(AuthInfo::OAuth { .. }) => Some("oauth".to_string()),
        Some(AuthInfo::WellKnown { .. }) => Some("wellknown".to_string()),
        None => None,
    }
}

fn configured_provider_to_bootstrap_provider(
    provider: &agendao_config::ProviderConfig,
) -> ConfigProvider {
    ConfigProvider {
        name: provider.name.clone(),
        env: provider.env.clone(),
        api: provider.base_url.clone(),
        npm: provider.npm.clone(),
        api_style: provider.api_style.clone(),
        api_shape: provider.api_shape.clone(),
        transport: provider.transport.clone(),
        usage_shape: provider.usage_shape.clone(),
        quirks: (!provider.quirks.is_empty()).then_some(provider.quirks.clone()),
        ..Default::default()
    }
}

fn configured_provider_to_descriptor_candidate_result(
    provider_id: &str,
    provider: &agendao_config::ProviderConfig,
) -> std::result::Result<ProviderConnectionDescriptorCandidate, ProviderDescriptorError> {
    let bootstrap_provider = configured_provider_to_bootstrap_provider(provider);
    provider_connection_descriptor_candidate_from_config_provider(provider_id, &bootstrap_provider)
}

fn configured_provider_to_descriptor_candidate(
    provider_id: &str,
    provider: &agendao_config::ProviderConfig,
) -> std::result::Result<ProviderConnectionDescriptorCandidate, String> {
    configured_provider_to_descriptor_candidate_result(provider_id, provider)
        .map_err(|error| error.to_string())
}

fn provider_descriptor_projection(
    provider_id: &str,
    provider: &agendao_config::ProviderConfig,
) -> (
    Option<ProviderConnectionDescriptorCandidate>,
    Option<String>,
) {
    match configured_provider_to_descriptor_candidate(provider_id, provider) {
        Ok(candidate) => (Some(candidate), None),
        Err(error) => (None, Some(error)),
    }
}

pub(crate) fn collect_provider_profile_validation(
    config: &agendao_config::Config,
) -> Vec<ConfigPolicyValidationItem> {
    let Some(providers) = config.provider.as_ref() else {
        return Vec::new();
    };

    let mut provider_ids = providers.keys().cloned().collect::<Vec<_>>();
    provider_ids.sort();

    provider_ids
        .into_iter()
        .filter_map(|provider_id| {
            let provider = providers.get(&provider_id)?;
            let error =
                configured_provider_to_descriptor_candidate_result(&provider_id, provider).err()?;
            Some(provider_profile_validation_item(&provider_id, &error))
        })
        .collect()
}

fn provider_profile_validation_item(
    provider_id: &str,
    error: &ProviderDescriptorError,
) -> ConfigPolicyValidationItem {
    ConfigPolicyValidationItem {
        owner: ConfigPolicyValidationOwner::ProviderProfile,
        scope: ConfigPolicyValidationScope {
            kind: ConfigPolicyValidationScopeKind::Provider,
            subject_id: normalized_subject_id(provider_id),
        },
        path: provider_profile_validation_path(provider_id, error),
        severity: ConfigPolicyValidationSeverity::Error,
        effect: ConfigPolicyValidationEffect::FailClosedBootstrap,
        code: "provider_profile_invalid".to_string(),
        message: error.to_string(),
        fallback: None,
    }
}

fn provider_profile_validation_path(provider_id: &str, error: &ProviderDescriptorError) -> String {
    let provider_id = provider_id.trim();
    let root = if provider_id.is_empty() {
        "provider".to_string()
    } else {
        format!("provider.{provider_id}")
    };

    match error {
        ProviderDescriptorError::MissingProviderId => root,
        ProviderDescriptorError::InvalidProfile(profile_error) => {
            match provider_profile_error_field(profile_error) {
                Some(field) => format!("{root}.{field}"),
                None => root,
            }
        }
    }
}

fn provider_profile_error_field(error: &ProviderProfileError) -> Option<&'static str> {
    match error {
        ProviderProfileError::UnsupportedValue { field, .. }
        | ProviderProfileError::MissingField(field) => match field.as_str() {
            "api_style" => Some("api_style"),
            "api_shape" => Some("api_shape"),
            "transport" => Some("transport"),
            "usage_shape" => Some("usage_shape"),
            "quirks" => Some("quirks"),
            _ => None,
        },
        ProviderProfileError::InvalidConfig(_) | ProviderProfileError::InvalidCombination(_) => {
            None
        }
    }
}

fn normalized_subject_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn provider_descriptor_response(
    provider_id: &str,
    provider: &agendao_config::ProviderConfig,
) -> ProviderDescriptorResponse {
    let (descriptor_candidate, descriptor_candidate_error) =
        provider_descriptor_projection(provider_id, provider);

    ProviderDescriptorResponse {
        provider_id: provider_id.to_string(),
        descriptor_candidate,
        descriptor_candidate_error,
    }
}

pub(crate) async fn get_provider_descriptor(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderDescriptorResponse>> {
    let config = state.config_store.config();
    let provider_id = id.trim();
    let provider = config
        .provider
        .as_ref()
        .and_then(|providers| providers.get(provider_id))
        .ok_or_else(|| {
            ApiError::NotFound(format!("Configured provider not found: {provider_id}"))
        })?;

    Ok(Json(provider_descriptor_response(provider_id, provider)))
}

async fn list_managed_providers(
    State(state): State<Arc<ServerState>>,
) -> Json<ManagedProvidersResponse> {
    let variant_lookup = get_model_variant_lookup(state.as_ref()).await;
    let models_data = load_catalog_snapshot(state.as_ref()).await.data;
    let auth_store = state.auth_manager.list().await;
    let config = state.config_store.config();

    let providers_guard = state.providers.read().await;
    let runtime_provider_ids: std::collections::HashSet<String> = providers_guard
        .list()
        .into_iter()
        .map(|provider| provider.id().to_string())
        .collect();
    let runtime_models = providers_guard.list_models();
    drop(providers_guard);

    let mut provider_ids: std::collections::HashSet<String> = auth_store.keys().cloned().collect();
    if let Some(configured_providers) = &config.provider {
        provider_ids.extend(configured_providers.keys().cloned());
    }

    let mut providers = provider_ids
        .into_iter()
        .map(|id| {
            let known = models_data.get(&id);
            let configured = config
                .provider
                .as_ref()
                .and_then(|provider_map| provider_map.get(&id));
            let mut model_map: HashMap<String, ModelInfo> = HashMap::new();

            if let Some(configured_models) =
                configured.and_then(|provider| provider.models.as_ref())
            {
                for (configured_model_id, configured_model) in configured_models {
                    let model_id = configured_model
                        .model
                        .clone()
                        .unwrap_or_else(|| configured_model_id.clone());
                    let mut variants = configured_model
                        .variants
                        .as_ref()
                        .map(|items| items.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    if variants.is_empty() {
                        variants = variants_for_model(&variant_lookup, &id, &model_id);
                    } else {
                        variants.sort();
                    }
                    model_map.insert(
                        model_id.clone(),
                        configured_model_info(&id, model_id.clone(), configured_model, variants),
                    );
                }
            }

            for runtime_model in runtime_models.iter().filter(|model| model.provider == id) {
                let variants = variants_for_model(&variant_lookup, &id, &runtime_model.id);
                match model_map.entry(runtime_model.id.clone()) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        merge_runtime_model_info(
                            entry.get_mut(),
                            runtime_model_info(runtime_model, variants),
                        );
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(runtime_model_info(runtime_model, variants));
                    }
                }
            }

            let mut models: Vec<ModelInfo> = model_map.into_values().collect();
            models.sort_by(|a, b| a.id.cmp(&b.id));
            let mut model_overrides = configured
                .and_then(|provider| provider.models.as_ref())
                .map(|configured_models| {
                    configured_models
                        .iter()
                        .map(|(key, configured_model)| ManagedModelOverrideInfo {
                            key: key.clone(),
                            name: configured_model.name.clone(),
                            model: configured_model.model.clone(),
                            base_url: configured_model.base_url.clone(),
                            family: configured_model.family.clone(),
                            reasoning: configured_model.reasoning,
                            tool_call: configured_model.tool_call,
                            headers: configured_model.headers.clone(),
                            options: configured_model
                                .options
                                .as_ref()
                                .map(|value| serde_json::to_value(value).unwrap_or_default()),
                            variants: configured_model
                                .variants
                                .as_ref()
                                .map(|value| serde_json::to_value(value).unwrap_or_default()),
                            modalities: configured_model
                                .modalities
                                .as_ref()
                                .map(|value| serde_json::to_value(value).unwrap_or_default()),
                            interleaved: configured_model.interleaved.clone(),
                            cost: configured_model
                                .cost
                                .as_ref()
                                .map(|value| serde_json::to_value(value).unwrap_or_default()),
                            limit: configured_model
                                .limit
                                .as_ref()
                                .map(|value| serde_json::to_value(value).unwrap_or_default()),
                            attachment: configured_model.attachment,
                            temperature: configured_model.temperature,
                            status: configured_model.status.clone(),
                            release_date: configured_model.release_date.clone(),
                            experimental: configured_model.experimental,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            model_overrides.sort_by(|a, b| a.key.cmp(&b.key));

            let connected = runtime_provider_ids.contains(&id);
            let auth = auth_store.get(&id);
            let has_auth = auth.is_some();
            let configured_flag = configured.is_some();
            let (descriptor_candidate, descriptor_candidate_error) = configured
                .map(|provider| provider_descriptor_projection(&id, provider))
                .unwrap_or((None, None));

            ManagedProviderInfo {
                id: id.clone(),
                name: configured
                    .and_then(|provider| provider.name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| known.map(|provider| provider.name.clone()))
                    .unwrap_or_else(|| id.clone()),
                status: managed_provider_status(connected, configured_flag, has_auth).to_string(),
                connected,
                has_auth,
                auth_type: managed_provider_auth_type(auth),
                configured: configured_flag,
                known: known.is_some(),
                env: known
                    .map(|provider| provider.env.clone())
                    .unwrap_or_default(),
                known_model_count: known.map(|provider| provider.models.len()).unwrap_or(0),
                base_url: configured.and_then(|provider| provider.base_url.clone()),
                protocol: configured
                    .and_then(|provider| provider.npm.as_deref())
                    .and_then(npm_to_protocol)
                    .map(str::to_string),
                descriptor_candidate,
                descriptor_candidate_error,
                model_overrides,
                models,
            }
        })
        .collect::<Vec<_>>();

    providers.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| b.has_auth.cmp(&a.has_auth))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Json(ManagedProvidersResponse { providers })
}

async fn known_provider_entries(state: &ServerState) -> Vec<KnownProviderEntry> {
    let models_data = load_catalog_snapshot(state).await.data;
    let config = state.config_store.config();
    let configured_providers = config.provider.clone().unwrap_or_default();
    let connected_ids: std::collections::HashSet<String> = state
        .providers
        .read()
        .await
        .list_models()
        .into_iter()
        .map(|m| m.provider)
        .collect();

    let mut providers: Vec<KnownProviderEntry> = models_data
        .into_iter()
        .map(|(id, info)| {
            let configured = configured_providers.get(&id);
            let npm = configured
                .and_then(|provider| provider.npm.clone())
                .or(info.npm.clone());
            let base_url = configured
                .and_then(|provider| provider.base_url.clone())
                .or(info.api.clone());
            KnownProviderEntry {
                connected: connected_ids.contains(&id),
                model_count: info.models.len(),
                env: info.env,
                name: provider_display_name(&id, &info.name),
                id,
                base_url,
                protocol: npm.as_deref().and_then(npm_to_protocol).map(str::to_string),
                npm,
                supports_api_key_connect: true,
            }
        })
        .collect();
    providers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    providers
}

#[derive(Debug, Serialize)]
pub struct RefreshProviderCatalogResponse {
    pub generation_before: u64,
    pub generation_after: u64,
    pub changed: bool,
    pub status: CatalogRefreshStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub(crate) async fn refresh_provider_catalog(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<RefreshProviderCatalogResponse>> {
    let before = state.catalog_authority.snapshot().await;
    let after = state.catalog_authority.refresh_with_result(true).await;
    if after.snapshot.generation != before.generation {
        state.rebuild_providers().await;
        crate::session_runtime::events::broadcast_config_updated(state.as_ref());
    }

    Ok(Json(RefreshProviderCatalogResponse {
        generation_before: before.generation,
        generation_after: after.snapshot.generation,
        changed: after.snapshot.generation != before.generation,
        status: after.status,
        error_message: after.error_message,
    }))
}

/// A lightweight provider entry for the "known providers" catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct KnownProviderEntry {
    pub id: String,
    pub name: String,
    pub env: Vec<String>,
    pub model_count: usize,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    #[serde(default)]
    pub supports_api_key_connect: bool,
}

#[derive(Debug, Serialize)]
pub struct KnownProvidersResponse {
    pub providers: Vec<KnownProviderEntry>,
}

/// Returns all providers known to `models.dev`, regardless of whether they are
/// currently connected.  Each entry includes the primary env var(s) and a flag
/// indicating whether the provider is already connected.
pub(crate) async fn list_known_providers(
    State(state): State<Arc<ServerState>>,
) -> Json<KnownProvidersResponse> {
    let providers = known_provider_entries(state.as_ref()).await;
    Json(KnownProvidersResponse { providers })
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectProtocolOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderConnectSchemaResponse {
    pub providers: Vec<KnownProviderEntry>,
    pub protocols: Vec<ConnectProtocolOption>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveProviderConnectRequest {
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct ResolveProviderConnectResponse {
    pub query: String,
    pub suggested_mode: ProviderConnectDraftMode,
    pub exact_match: bool,
    pub matches: Vec<KnownProviderEntry>,
    pub draft: ProviderConnectDraft,
    pub custom_draft: ProviderConnectDraft,
}

pub(crate) async fn get_provider_connect_schema(
    State(state): State<Arc<ServerState>>,
) -> Json<ProviderConnectSchemaResponse> {
    let providers = known_provider_entries(state.as_ref()).await;
    let protocols = CONNECT_PROTOCOL_OPTIONS
        .iter()
        .map(|(id, name)| ConnectProtocolOption {
            id: (*id).to_string(),
            name: (*name).to_string(),
        })
        .collect();
    Json(ProviderConnectSchemaResponse {
        providers,
        protocols,
    })
}

pub(crate) async fn resolve_provider_connect(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ResolveProviderConnectRequest>,
) -> Json<ResolveProviderConnectResponse> {
    let query = req.query.trim().to_string();
    let matches =
        resolve_known_provider_matches(&known_provider_entries(state.as_ref()).await, &query);
    let exact_match = matches
        .first()
        .map(|provider| provider.id.eq_ignore_ascii_case(&query))
        .unwrap_or(false);
    let draft = matches
        .first()
        .map(connect_draft_from_known_provider)
        .unwrap_or_else(|| connect_draft_from_custom_query(&query));

    Json(ResolveProviderConnectResponse {
        query: query.clone(),
        suggested_mode: draft.mode.clone(),
        exact_match,
        matches,
        draft,
        custom_draft: connect_draft_from_custom_query(&query),
    })
}

#[derive(Debug, Serialize)]
pub struct AuthMethodInfo {
    pub name: String,
    pub description: String,
}

async fn get_provider_auth(
    State(state): State<Arc<ServerState>>,
) -> Json<HashMap<String, Vec<AuthMethodInfo>>> {
    if let Err(error) = super::plugin_auth::ensure_plugin_loader_active(&state).await {
        tracing::warn!(%error, "failed to warm plugin loader for provider auth list");
    }
    let Some(loader) = super::get_plugin_loader() else {
        return Json(HashMap::new());
    };
    let methods = ProviderAuth::methods(loader).await;
    let result = methods
        .into_iter()
        .map(|(provider, values)| {
            let mapped = values
                .into_iter()
                .map(|method| AuthMethodInfo {
                    name: method.label,
                    description: method.method_type,
                })
                .collect::<Vec<_>>();
            (provider, mapped)
        })
        .collect::<HashMap<_, _>>();
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeRequest {
    pub method: usize,
}

#[derive(Debug, Serialize)]
pub struct OAuthAuthorizeResponse {
    pub url: String,
    #[serde(rename = "method")]
    pub method_type: String,
    pub instructions: String,
}

async fn oauth_authorize(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<OAuthAuthorizeRequest>,
) -> Result<Json<OAuthAuthorizeResponse>> {
    let _ = super::plugin_auth::ensure_plugin_loader_active(&state).await?;
    let loader = super::get_plugin_loader()
        .ok_or_else(|| ApiError::NotFound("no plugin loader initialized".to_string()))?;
    let authorization = ProviderAuth::authorize(loader, &id, req.method, None)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(OAuthAuthorizeResponse {
        url: authorization.url,
        method_type: match authorization.method {
            AuthMethodType::Auto => "auto".to_string(),
            AuthMethodType::Code => "code".to_string(),
        },
        instructions: authorization.instructions,
    }))
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackRequest {
    pub method: usize,
    pub code: Option<String>,
}

async fn oauth_callback(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<OAuthCallbackRequest>,
) -> Result<Json<bool>> {
    let _ = super::plugin_auth::ensure_plugin_loader_active(&state).await?;
    let loader = super::get_plugin_loader()
        .ok_or_else(|| ApiError::NotFound("no plugin loader initialized".to_string()))?;
    ProviderAuth::new(state.auth_manager.clone())
        .callback(loader, &id, req.code.as_deref())
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Refresh auth loader state after callback and apply custom-fetch proxy changes immediately.
    if let Some(bridge) = loader.auth_bridge(&id).await {
        match bridge.load().await {
            Ok(load_result) => {
                crate::server::sync_custom_fetch_proxy(
                    &id,
                    bridge,
                    loader,
                    load_result.has_custom_fetch,
                );
            }
            Err(error) => {
                crate::server::sync_custom_fetch_proxy(&id, bridge, loader, false);
                tracing::warn!(
                    provider = %id,
                    %error,
                    "failed to refresh plugin auth loader after oauth callback"
                );
            }
        }
    }

    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct ConnectProviderRequest {
    pub provider_id: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
}

pub(crate) async fn connect_provider(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ConnectProviderRequest>,
) -> Result<Json<bool>> {
    let provider_id = req.provider_id.trim();
    let api_key = req.api_key.trim();
    if provider_id.is_empty() {
        return Err(ApiError::BadRequest("provider_id is required".to_string()));
    }
    if api_key.is_empty() {
        return Err(ApiError::BadRequest("api_key is required".to_string()));
    }

    match (&req.base_url, &req.protocol) {
        (Some(_), None) | (None, Some(_)) => {
            return Err(ApiError::BadRequest(
                "base_url and protocol must be provided together".to_string(),
            ));
        }
        _ => {}
    }

    if let (Some(base_url), Some(protocol)) = (&req.base_url, &req.protocol) {
        let base_url = base_url.trim();
        let protocol = protocol.trim();
        if base_url.is_empty() {
            return Err(ApiError::BadRequest("base_url is required".to_string()));
        }
        let npm = protocol_to_npm(protocol)
            .ok_or_else(|| ApiError::BadRequest(format!("Invalid protocol: {}", protocol)))?;

        let updated = state
            .config_store
            .replace_with(|config| {
                let providers = config.provider.get_or_insert_with(HashMap::new);
                let provider = providers
                    .entry(provider_id.to_string())
                    .or_insert_with(agendao_config::ProviderConfig::default);
                if provider
                    .name
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
                {
                    provider.name = Some(provider_id.to_string());
                }
                provider.id = Some(provider_id.to_string());
                provider.base_url = Some(base_url.to_string());
                provider.npm = Some(npm.to_string());
                Ok(())
            })
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        drop(updated);
    }

    state
        .auth_manager
        .set(
            provider_id,
            agendao_provider::AuthInfo::Api {
                key: api_key.to_string(),
            },
        )
        .await;
    state.rebuild_providers().await;
    crate::session_runtime::events::broadcast_config_updated(state.as_ref());

    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
}

async fn update_provider(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<bool>> {
    let provider_id = id.trim();
    if provider_id.is_empty() {
        return Err(ApiError::BadRequest("provider id is required".to_string()));
    }

    match (&req.base_url, &req.protocol) {
        (Some(_), None) | (None, Some(_)) => {
            return Err(ApiError::BadRequest(
                "base_url and protocol must be provided together".to_string(),
            ));
        }
        _ => {}
    }

    let updated = state
        .config_store
        .replace_with(|config| {
            let providers = config.provider.get_or_insert_with(HashMap::new);
            let provider = providers
                .entry(provider_id.to_string())
                .or_insert_with(agendao_config::ProviderConfig::default);

            if let Some(name) = &req.name {
                let trimmed = name.trim();
                provider.name = (!trimmed.is_empty()).then_some(trimmed.to_string());
            }

            if let (Some(base_url), Some(protocol)) = (&req.base_url, &req.protocol) {
                let base_url = base_url.trim();
                let protocol = protocol.trim();
                if base_url.is_empty() {
                    return Err(anyhow::anyhow!("base_url is required"));
                }
                let npm = protocol_to_npm(protocol)
                    .ok_or_else(|| anyhow::anyhow!("Invalid protocol: {}", protocol))?;
                provider.id = Some(provider_id.to_string());
                provider.base_url = Some(base_url.to_string());
                provider.npm = Some(npm.to_string());
            }

            Ok(())
        })
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    drop(updated);

    state.rebuild_providers().await;
    crate::session_runtime::events::broadcast_config_updated(state.as_ref());
    Ok(Json(true))
}

async fn delete_provider(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<bool>> {
    let provider_id = id.trim();
    if provider_id.is_empty() {
        return Err(ApiError::BadRequest("provider id is required".to_string()));
    }

    let updated = state
        .config_store
        .replace_with(|config| {
            if let Some(providers) = config.provider.as_mut() {
                providers.remove(provider_id);
            }
            Ok(())
        })
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    drop(updated);

    state.auth_manager.remove(provider_id).await;
    state.rebuild_providers().await;
    crate::session_runtime::events::broadcast_config_updated(state.as_ref());
    Ok(Json(true))
}

async fn register_custom_provider(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ConnectProviderRequest>,
) -> Result<Json<bool>> {
    connect_provider(State(state), Json(req)).await
}

#[cfg(test)]
mod tests {
    use super::{
        configured_provider_to_descriptor_candidate, connect_draft_from_custom_query,
        connect_draft_from_known_provider, npm_to_protocol, protocol_to_npm,
        provider_descriptor_response, resolve_known_provider_matches, KnownProviderEntry,
        ProviderConnectDraftMode, CONNECT_PROTOCOL_OPTIONS,
    };
    use agendao_config::ProviderConfig;

    fn provider(
        id: &str,
        name: &str,
        env: &[&str],
        base_url: Option<&str>,
        protocol: Option<&str>,
    ) -> KnownProviderEntry {
        KnownProviderEntry {
            id: id.to_string(),
            name: name.to_string(),
            env: env.iter().map(|value| (*value).to_string()).collect(),
            model_count: 0,
            connected: false,
            base_url: base_url.map(str::to_string),
            protocol: protocol.map(str::to_string),
            npm: None,
            supports_api_key_connect: true,
        }
    }

    #[test]
    fn connect_schema_lists_openrouter_and_perplexity() {
        assert!(CONNECT_PROTOCOL_OPTIONS
            .iter()
            .any(|(id, _)| *id == "openrouter"));
        assert!(CONNECT_PROTOCOL_OPTIONS
            .iter()
            .any(|(id, _)| *id == "perplexity"));
    }

    #[test]
    fn protocol_mapping_supports_openrouter_and_perplexity() {
        assert_eq!(
            protocol_to_npm("openrouter"),
            Some("@openrouter/ai-sdk-provider")
        );
        assert_eq!(protocol_to_npm("perplexity"), Some("@ai-sdk/perplexity"));
        assert_eq!(
            npm_to_protocol("@openrouter/ai-sdk-provider"),
            Some("openrouter")
        );
        assert_eq!(npm_to_protocol("@ai-sdk/perplexity"), Some("perplexity"));
    }

    #[test]
    fn resolve_matches_prioritize_exact_then_prefix_then_contains_then_env() {
        let providers = vec![
            provider(
                "openrouter",
                "OpenRouter",
                &["OPENROUTER_API_KEY"],
                None,
                None,
            ),
            provider("openai", "OpenAI", &["OPENAI_API_KEY"], None, None),
            provider(
                "routerstack",
                "Router Stack",
                &["ROUTERSTACK_KEY"],
                None,
                None,
            ),
            provider("anthropic", "Ethnopic", &["OPENROUTER_TOKEN"], None, None),
        ];

        let matches = resolve_known_provider_matches(&providers, "openrouter");
        assert_eq!(
            matches
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["openrouter", "anthropic"]
        );

        let matches = resolve_known_provider_matches(&providers, "open");
        assert_eq!(
            matches
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["openai", "openrouter", "anthropic"]
        );

        let matches = resolve_known_provider_matches(&providers, "router");
        assert_eq!(
            matches
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["routerstack", "openrouter", "anthropic"]
        );
    }

    #[test]
    fn custom_query_draft_defaults_to_openai_protocol() {
        let draft = connect_draft_from_custom_query("  my-provider  ");
        assert_eq!(draft.mode, ProviderConnectDraftMode::Custom);
        assert_eq!(draft.provider_id, "my-provider");
        assert_eq!(draft.protocol.as_deref(), Some("openai"));
        assert!(draft.base_url.is_none());
        assert!(draft.known_provider_id.is_none());
    }

    #[test]
    fn known_provider_draft_preserves_overlay_fields() {
        let provider = provider(
            "openrouter",
            "OpenRouter",
            &["OPENROUTER_API_KEY"],
            Some("https://openrouter.ai/api/v1"),
            Some("openrouter"),
        );

        let draft = connect_draft_from_known_provider(&provider);
        assert_eq!(draft.mode, ProviderConnectDraftMode::Known);
        assert_eq!(draft.provider_id, "openrouter");
        assert_eq!(draft.known_provider_id.as_deref(), Some("openrouter"));
        assert_eq!(
            draft.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(draft.protocol.as_deref(), Some("openrouter"));
        assert_eq!(draft.env, vec!["OPENROUTER_API_KEY".to_string()]);
    }

    #[test]
    fn configured_provider_descriptor_candidate_exposes_non_secret_projection() {
        let configured = ProviderConfig {
            name: Some(" OpenRouter ".to_string()),
            api_key: Some("secret-123".to_string()),
            base_url: Some(" https://openrouter.ai/api/v1 ".to_string()),
            npm: Some("@openrouter/ai-sdk-provider".to_string()),
            env: Some(vec![" OPENROUTER_API_KEY ".to_string()]),
            ..Default::default()
        };

        let candidate = configured_provider_to_descriptor_candidate("openrouter", &configured)
            .expect("candidate should build");
        let value = serde_json::to_value(&candidate).expect("candidate should serialize");

        assert_eq!(candidate.provider_id, "openrouter");
        assert_eq!(candidate.name.as_deref(), Some("OpenRouter"));
        assert_eq!(
            candidate.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(candidate.env, vec!["OPENROUTER_API_KEY".to_string()]);
        assert!(value.get("api_key").is_none());
        assert!(value.get("options").is_none());
        assert!(candidate.profile.is_some());
    }

    #[test]
    fn configured_provider_descriptor_candidate_reports_invalid_profile() {
        let configured = ProviderConfig {
            api_style: Some("closeai-compatible".to_string()),
            api_shape: Some("messages".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("closeai-cached-tokens".to_string()),
            ..Default::default()
        };

        let error = configured_provider_to_descriptor_candidate("broken", &configured)
            .expect_err("invalid config should surface as inspection error");

        assert!(error.contains("invalid provider profile combination"));
    }

    #[test]
    fn provider_descriptor_response_uses_shared_candidate_field() {
        let configured = ProviderConfig {
            name: Some(" OpenAI ".to_string()),
            base_url: Some(" https://api.openai.com/v1 ".to_string()),
            env: Some(vec![" OPENAI_API_KEY ".to_string()]),
            ..Default::default()
        };

        let response = provider_descriptor_response("openai", &configured);
        let value = serde_json::to_value(&response).expect("response should serialize");

        assert_eq!(value["provider_id"], serde_json::json!("openai"));
        assert_eq!(
            value["descriptor_candidate"]["name"],
            serde_json::json!("OpenAI")
        );
        assert!(value.get("descriptor_candidate_error").is_none());
    }

    #[test]
    fn provider_descriptor_response_preserves_projection_error_without_candidate() {
        let configured = ProviderConfig {
            api_style: Some("closeai-compatible".to_string()),
            api_shape: Some("messages".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("closeai-cached-tokens".to_string()),
            ..Default::default()
        };

        let response = provider_descriptor_response("broken", &configured);

        assert!(response.descriptor_candidate.is_none());
        assert!(response
            .descriptor_candidate_error
            .as_deref()
            .is_some_and(|error| error.contains("invalid provider profile combination")));
    }
}
