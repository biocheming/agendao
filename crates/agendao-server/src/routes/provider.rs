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
        .route("/{id}/disabled", put(set_provider_disabled))
        .route("/{id}/test", post(test_provider_connection))
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
    /// Provider HTTP endpoint(从 config.provider[id].base_url 读)。
    /// `None` = 配置未设(常见于 SDK-managed 或 models.dev catalog provider)。
    /// 阴面记账(土律):server 唯一权威,TUI/web 只读消费。api_key 永不下发。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Provider wire protocol（OpenAI Responses / Chat Completions / Anthropic Messages）。
    /// 从完整 config profile 优先，catalog 的受支持 SDK shape 兜底。
    /// `None` 表示协议未声明或不受支持。
    /// 与 base_url 配对决定 HTTP 实际打哪条契约;TUI Settings 展示给用户验证。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// 是否被用户禁用(`config.disabled_providers` 成员)。与 connected(有 auth)
    /// 是两个独立维度:disabled 的 provider 不出现在运行时 registry,但配置保留。
    #[serde(default)]
    pub disabled: bool,
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
    /// 是否被用户禁用(`config.disabled_providers` 成员)。status 维度之外:
    /// disabled 的 provider 不进运行时 registry,但配置保留可再启用。
    #[serde(default)]
    pub disabled: bool,
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
    /// True only when this exact provider/model pair exists in the live
    /// runtime registry. Catalogue visibility and provider authentication do
    /// not by themselves make every advertised model callable.
    pub available: bool,
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
        available: false,
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
        available: true,
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
        available: false,
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
    existing.available = true;
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
    ("openai-responses", "OpenAI Responses"),
    ("openai-chat", "OpenAI Chat Completions"),
    ("anthropic", "Anthropic Messages"),
];

#[derive(Clone, Copy)]
struct ConnectProtocolProfile {
    npm: &'static str,
    api_style: &'static str,
    api_shape: &'static str,
    transport: &'static str,
    usage_shape: &'static str,
}

fn connect_protocol_profile(protocol: &str) -> Option<ConnectProtocolProfile> {
    match protocol {
        "openai-responses" => Some(ConnectProtocolProfile {
            npm: "@ai-sdk/openai",
            api_style: "openai-compatible",
            api_shape: "responses",
            transport: "bearer",
            usage_shape: "openai-cached-tokens",
        }),
        "openai-chat" => Some(ConnectProtocolProfile {
            npm: "@ai-sdk/openai-compatible",
            api_style: "openai-compatible",
            api_shape: "chat-completions",
            transport: "bearer",
            usage_shape: "openai-cached-tokens",
        }),
        "anthropic" => Some(ConnectProtocolProfile {
            npm: "@ai-sdk/anthropic",
            api_style: "anthropic-compatible",
            api_shape: "messages",
            transport: "bearer",
            usage_shape: "anthropic-read-write",
        }),
        _ => None,
    }
}

pub(crate) fn npm_to_protocol(npm: &str) -> Option<&'static str> {
    match npm.trim().to_ascii_lowercase().as_str() {
        "@ai-sdk/openai" => Some("openai-responses"),
        "@ai-sdk/openai-compatible" => Some("openai-chat"),
        "@ai-sdk/anthropic" => Some("anthropic"),
        _ => None,
    }
}

fn profile_to_wire_protocol(profile: &agendao_provider::ProviderProfile) -> &'static str {
    match profile.api_shape {
        agendao_provider::ProviderApiShape::Responses => "openai-responses",
        agendao_provider::ProviderApiShape::ChatCompletions => "openai-chat",
        agendao_provider::ProviderApiShape::AnthropicMessages => "anthropic",
    }
}

fn catalog_wire_protocol(npm: &str) -> Option<&'static str> {
    npm_to_protocol(npm)
}

pub(crate) fn configured_wire_protocol(
    provider_id: &str,
    provider: &agendao_config::ProviderConfig,
) -> Option<&'static str> {
    let bootstrap_provider = configured_provider_to_bootstrap_provider(provider);
    let profile = agendao_provider::ProviderProfileResolver::try_resolve_config_provider(
        provider_id,
        &bootstrap_provider,
    )
    .ok()?;
    Some(profile_to_wire_protocol(&profile))
}

fn apply_connect_protocol_profile(
    provider: &mut agendao_config::ProviderConfig,
    profile: ConnectProtocolProfile,
) {
    provider.npm = Some(profile.npm.to_string());
    provider.api_style = Some(profile.api_style.to_string());
    provider.api_shape = Some(profile.api_shape.to_string());
    provider.transport = Some(profile.transport.to_string());
    provider.usage_shape = Some(profile.usage_shape.to_string());
}

fn provider_display_name(provider_id: &str, name: &str) -> String {
    if provider_id.eq_ignore_ascii_case("anthropic") || name.eq_ignore_ascii_case("anthropic") {
        "Anthropic".to_string()
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
        protocol: Some("openai-chat".to_string()),
        env: Vec::new(),
        connected: false,
        model_count: 0,
        supports_api_key_connect: true,
    }
}

async fn load_catalog_snapshot(state: &ServerState) -> std::sync::Arc<CatalogSnapshot> {
    state.catalog_authority.snapshot().await
}

fn build_model_variant_lookup(data: &ModelsData) -> HashMap<String, HashMap<String, Vec<String>>> {
    data.iter()
        .map(|(provider_id, provider)| {
            let model_map = provider
                .models
                .iter()
                .map(|(model_id, model)| {
                    let mut variants = model
                        .variants
                        .as_ref()
                        .map(|items| items.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    if variants.is_empty() {
                        variants = synthetic_variant_names(provider_id, model);
                    }
                    variants.sort();
                    (model_id.clone(), variants)
                })
                .collect::<HashMap<_, _>>();
            (provider_id.clone(), model_map)
        })
        .collect()
}

/// Detect whether a provider+model pair uses the Anthropic-compatible protocol family.
///
/// This is a **protocol compatibility check**, not a brand reference.  When users
/// configure an Anthropic-compatible provider (directly or via Bedrock/Vertex),
/// the thinking variant surface is `["high", "max"]` rather than the OpenAI-style
/// `["low", "medium", "high"]`.
fn is_anthropic_protocol_family(provider_id: &str) -> bool {
    let provider = provider_id.to_ascii_lowercase();
    provider.contains("anthropic")
}

fn synthetic_variant_names(provider_id: &str, model: &ModelsDevInfo) -> Vec<String> {
    if !model.reasoning {
        return Vec::new();
    }

    if is_anthropic_protocol_family(provider_id) {
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
    build_model_variant_lookup(&snapshot.data)
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
    // One snapshot per response: the variant lookup and the catalogue
    // iteration must see the same generation.
    let catalog_snapshot = load_catalog_snapshot(state.as_ref()).await;
    let variant_lookup = build_model_variant_lookup(&catalog_snapshot.data);

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
    // base_url 映射:config.provider[id].base_url 显式配置优先(由 step 2 用
    // `insert` 覆盖),catalog api URL 兜底(由 step 1 `or_insert_with` 填)——
    // 目录 provider 的默认端点对用户可见/可编辑,不再是 "(not set)" 黑洞。
    let mut provider_base_urls: HashMap<String, String> = HashMap::new();
    // protocol 映射:从 config profile + catalog npm 解析为三种 wire protocol。
    // 阴面同 base_url;configured 优先,catalog 兜底(由 step 1/2 两端各自填,
    // `or_insert_with` 保证 config 先写后不被 catalog 覆盖)。
    let mut provider_protocols: HashMap<String, String> = HashMap::new();

    // 1) models.dev full provider catalogue.
    for (provider_id, provider) in &catalog_snapshot.data {
        let Some(protocol) = provider.npm.as_deref().and_then(catalog_wire_protocol) else {
            continue;
        };
        provider_names
            .entry(provider_id.clone())
            .or_insert_with(|| provider_display_name(provider_id, &provider.name));
        // base_url 从 catalog info.api 兜底填入(显示/编辑预填用);step 2 的
        // config 显式配置会用 `insert` 覆盖此值。
        if let Some(api) = provider
            .api
            .as_deref()
            .filter(|url| !url.trim().is_empty())
        {
            provider_base_urls
                .entry(provider_id.clone())
                .or_insert_with(|| api.to_string());
        }
        // protocol 从 catalog info.npm 反推填入(catalog 作为兜底,step 2 的
        // config 端再覆盖优先级更高的);`or_insert_with` 保证幂等。
        provider_protocols
            .entry(provider_id.clone())
            .or_insert_with(|| protocol.to_string());
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
            // 显式配置的 name（rename）优先于 catalog 默认名——config 是用户权威
            // （与 protocol 的 config-override 语义一致,此前 catalog 用 or_insert_with
            // 抢在 config 前面,rename 在列表里不生效）。
            if let Some(name) = provider.name.as_deref().filter(|n| !n.trim().is_empty()) {
                provider_names.insert(
                    provider_id.clone(),
                    provider_display_name(provider_id, name),
                );
            } else {
                provider_names
                    .entry(provider_id.clone())
                    .or_insert_with(|| provider_display_name(provider_id, provider_id));
            }
            // base_url 填入(土律单点):config 显式配置 `insert` 覆盖 step 1 的
            // catalog api 兜底——用户配置的端点优先于目录默认值。
            if let Some(base) = provider.base_url.as_ref().filter(|s| !s.is_empty()) {
                provider_base_urls.insert(provider_id.clone(), base.clone());
            }
            // protocol 填入(土律单点·config override):用户/管理面显式配的 npm 优先,
            // 用 `insert` 直接覆盖 step 1 的 catalog 兜底值——与 KnownProviderEntry
            // 的 `configured.npm.or(info.npm)` 一致语义(provider.rs:1334)。
            if let Some(proto) = configured_wire_protocol(provider_id, provider) {
                provider_protocols.insert(provider_id.clone(), proto.to_string());
            }
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
                base_url: provider_base_urls.get(&id).cloned(),
                protocol: provider_protocols.get(&id).cloned(),
                disabled: config.disabled_providers.contains(&id),
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
                .iter()
                .find(|model| model.available)
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
        api_key: provider.api_key.clone(),
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
    // One snapshot per response: the variant lookup and the catalogue
    // lookups must see the same generation.
    let catalog_snapshot = load_catalog_snapshot(state.as_ref()).await;
    let variant_lookup = build_model_variant_lookup(&catalog_snapshot.data);
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
            let known = catalog_snapshot.data.get(&id);
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
                disabled: config.disabled_providers.contains(&id),
                env: known
                    .map(|provider| provider.env.clone())
                    .unwrap_or_default(),
                known_model_count: known.map(|provider| provider.models.len()).unwrap_or(0),
                base_url: configured.and_then(|provider| provider.base_url.clone()),
                protocol: configured
                    .and_then(|provider| configured_wire_protocol(&id, provider))
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
    let catalog_snapshot = load_catalog_snapshot(state).await;
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

    let mut providers: Vec<KnownProviderEntry> = catalog_snapshot
        .data
        .iter()
        .filter_map(|(id, info)| {
            let configured = configured_providers.get(id);
            let npm = configured
                .and_then(|provider| provider.npm.clone())
                .or(info.npm.clone());
            let protocol = configured
                .and_then(|provider| configured_wire_protocol(id, provider))
                .or_else(|| npm.as_deref().and_then(catalog_wire_protocol))?;
            let base_url = configured
                .and_then(|provider| provider.base_url.clone())
                .or(info.api.clone());
            Some(KnownProviderEntry {
                connected: connected_ids.contains(id),
                model_count: info.models.len(),
                env: info.env.clone(),
                name: provider_display_name(id, &info.name),
                id: id.clone(),
                base_url,
                protocol: Some(protocol.to_string()),
                npm,
                supports_api_key_connect: true,
            })
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
        let protocol_profile = connect_protocol_profile(protocol)
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
                apply_connect_protocol_profile(provider, protocol_profile);
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

pub(crate) async fn update_provider(
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
                let protocol_profile = connect_protocol_profile(protocol)
                    .ok_or_else(|| anyhow::anyhow!("Invalid protocol: {}", protocol))?;
                provider.id = Some(provider_id.to_string());
                provider.base_url = Some(base_url.to_string());
                apply_connect_protocol_profile(provider, protocol_profile);
            }

            Ok(())
        })
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    drop(updated);

    state.rebuild_providers().await;
    crate::session_runtime::events::broadcast_config_updated(state.as_ref());
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct SetProviderDisabledRequest {
    pub disabled: bool,
}

/// Enable/disable provider（config.disabled_providers 的单一写入口）。
///
/// 用 `replace_with` 直接改写而非 PATCH merge——merge 的
/// `merge_vec_replace_if_non_empty` 语义下空数组无法清除 disabled 列表，
/// re-enable 会失效。disabled 的 provider 不进运行时 registry（bootstrap 消费），
/// 但 provider 配置与 auth 全部保留，可随时再启用。
pub(crate) async fn set_provider_disabled(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetProviderDisabledRequest>,
) -> Result<Json<bool>> {
    let provider_id = id.trim();
    if provider_id.is_empty() {
        return Err(ApiError::BadRequest("provider id is required".to_string()));
    }

    let updated = state
        .config_store
        .replace_with(|config| {
            if req.disabled {
                if !config.disabled_providers.iter().any(|p| p == provider_id) {
                    config.disabled_providers.push(provider_id.to_string());
                }
            } else {
                config.disabled_providers.retain(|p| p != provider_id);
            }
            Ok(())
        })
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    drop(updated);

    state.rebuild_providers().await;
    crate::session_runtime::events::broadcast_config_updated(state.as_ref());
    Ok(Json(true))
}

#[derive(Debug, Serialize)]
pub struct TestProviderConnectionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    pub latency_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 测试连接（金律·可观测性）：用存储的 auth 对 provider 的 models 端点发一个
/// 轻量 GET，回报 ok/status/延迟/错误。不产生任何副作用（只读探测）。
///
/// 覆盖 OpenAI Responses / Chat Completions（Bearer）与 Anthropic Messages
///（x-api-key）。
pub(crate) async fn test_provider_connection(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<TestProviderConnectionResponse>> {
    fn fail_fast(error: impl Into<String>) -> TestProviderConnectionResponse {
        TestProviderConnectionResponse {
            ok: false,
            status: None,
            latency_ms: 0,
            error: Some(error.into()),
        }
    }

    let provider_id = id.trim();
    if provider_id.is_empty() {
        return Err(ApiError::BadRequest("provider id is required".to_string()));
    }

    let config = state.config_store.config();
    let configured = config.provider.as_ref().and_then(|m| m.get(provider_id));
    // base_url：config 显式配置优先，catalog(models.dev)默认端点兜底——
    // 多数 connected provider 并不在 config 里写 base_url(土律·兜底不假装)。
    let (catalog_api, catalog_npm) = state
        .catalog_authority
        .map_snapshot(|snapshot| {
            snapshot
                .data
                .get(provider_id)
                .map(|provider| (provider.api.clone(), provider.npm.clone()))
                .unwrap_or_default()
        })
        .await;
    let base_url = configured
        .and_then(|p| p.base_url.clone())
        .filter(|s| !s.trim().is_empty())
        .or(catalog_api);
    let Some(base_url) = base_url else {
        return Ok(Json(fail_fast("no base_url configured for this provider")));
    };
    let protocol = configured
        .and_then(|provider| configured_wire_protocol(provider_id, provider))
        .or_else(|| catalog_npm.as_deref().and_then(catalog_wire_protocol))
        .ok_or_else(|| {
            ApiError::BadRequest(
                "provider protocol is missing or unsupported; choose openai-responses, openai-chat, or anthropic"
                    .to_string(),
            )
        })?
        .to_string();
    let api_key = state.auth_manager.get_api_key(provider_id).await;

    let outcome =
        agendao_provider::transport::connection_test(&base_url, &protocol, api_key.as_deref())
            .await;
    Ok(Json(TestProviderConnectionResponse {
        ok: outcome.ok,
        status: outcome.status,
        latency_ms: outcome.latency_ms,
        error: outcome.error,
    }))
}

pub(crate) async fn delete_provider(
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
        connect_draft_from_known_provider, connect_protocol_profile, npm_to_protocol,
        provider_descriptor_response, resolve_known_provider_matches, KnownProviderEntry,
        ModelInfo, ProviderConnectDraftMode, CONNECT_PROTOCOL_OPTIONS,
    };
    use agendao_config::ProviderConfig;

    #[tokio::test]
    async fn list_providers_fills_base_url_from_catalog_with_config_override() {
        use axum::extract::State;
        use std::collections::HashMap;

        // 最小目录：一个带 config 覆盖的 provider + 一个纯目录 provider。
        let catalog = serde_json::json!({
            "zhipuai-coding-plan": {
                "id": "zhipuai-coding-plan",
                "name": "Zhipu AI Coding Plan",
                "api": "https://open.bigmodel.cn/api/coding/paas/v4",
                "npm": "@ai-sdk/openai-compatible",
                "env": [],
                "models": {}
            },
            "acme-cat": {
                "id": "acme-cat",
                "name": "Acme Catalog",
                "api": "https://api.acme.dev/v1",
                "npm": "@ai-sdk/openai-compatible",
                "env": [],
                "models": {}
            }
        });
        let dir = std::env::temp_dir().join(format!(
            "agendao-test-cat-{}.json",
            std::process::id()
        ));
        std::fs::write(&dir, catalog.to_string()).expect("write test catalog");

        let mut state = crate::ServerState::new();
        state.catalog_authority = std::sync::Arc::new(
            agendao_provider::ModelCatalogAuthority::with_snapshot_path(dir.clone()),
        );
        // config 显式 base_url 必须覆盖目录 api 兜底。
        state.config_store = std::sync::Arc::new(agendao_config::ConfigStore::new(
            agendao_config::Config {
                provider: Some(HashMap::from([(
                    "zhipuai-coding-plan".to_string(),
                    ProviderConfig {
                        base_url: Some("https://config-override.example/v1".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        ));
        let state = std::sync::Arc::new(state);

        let axum::Json(resp) =
            super::list_providers(State(state)).await;
        let find = |id: &str| {
            resp.all
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("provider {id} missing"))
        };
        assert_eq!(
            find("acme-cat").base_url.as_deref(),
            Some("https://api.acme.dev/v1"),
            "catalog api URL 应作为 base_url 兜底下发"
        );
        assert_eq!(
            find("zhipuai-coding-plan").base_url.as_deref(),
            Some("https://config-override.example/v1"),
            "config 显式 base_url 必须覆盖目录兜底"
        );
        let _ = std::fs::remove_file(&dir);
    }

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
    fn connect_schema_lists_exactly_three_wire_protocols() {
        assert_eq!(
            CONNECT_PROTOCOL_OPTIONS
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec!["openai-responses", "openai-chat", "anthropic"]
        );
    }

    #[test]
    fn protocol_mapping_accepts_only_supported_protocols_and_sdk_shapes() {
        assert!(connect_protocol_profile("openrouter").is_none());
        assert!(connect_protocol_profile("openai").is_none());
        assert_eq!(
            connect_protocol_profile("openai-responses").map(|profile| profile.npm),
            Some("@ai-sdk/openai")
        );
        assert_eq!(
            npm_to_protocol("@ai-sdk/openai-compatible"),
            Some("openai-chat")
        );
        assert_eq!(npm_to_protocol("@openrouter/ai-sdk-provider"), None);
        assert_eq!(npm_to_protocol("@ai-sdk/perplexity"), None);
    }

    #[tokio::test]
    async fn set_provider_disabled_roundtrip() {
        use super::{set_provider_disabled, SetProviderDisabledRequest};
        use axum::{extract::Path, extract::State, Json};
        use std::sync::Arc;

        let state = Arc::new(crate::ServerState::new());
        // disable → 进 disabled_providers
        let Json(ok) = set_provider_disabled(
            State(state.clone()),
            Path("deepseek".to_string()),
            Json(SetProviderDisabledRequest { disabled: true }),
        )
        .await
        .expect("disable should succeed");
        assert!(ok);
        assert!(state
            .config_store
            .config()
            .disabled_providers
            .iter()
            .any(|p| p == "deepseek"));
        // re-enable → 必须能清出列表（PATCH merge 的空数组不可清除语义正是
        // 为什么这个端点走 replace_with 直写——回归守卫）。
        let Json(ok) = set_provider_disabled(
            State(state.clone()),
            Path("deepseek".to_string()),
            Json(SetProviderDisabledRequest { disabled: false }),
        )
        .await
        .expect("re-enable should succeed");
        assert!(ok);
        assert!(!state
            .config_store
            .config()
            .disabled_providers
            .iter()
            .any(|p| p == "deepseek"));
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
            provider("anthropic", "Anthropic", &["OPENROUTER_TOKEN"], None, None),
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
    fn custom_query_draft_defaults_to_openai_chat_protocol() {
        let draft = connect_draft_from_custom_query("  my-provider  ");
        assert_eq!(draft.mode, ProviderConnectDraftMode::Custom);
        assert_eq!(draft.provider_id, "my-provider");
        assert_eq!(draft.protocol.as_deref(), Some("openai-chat"));
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
            Some("openai-chat"),
        );

        let draft = connect_draft_from_known_provider(&provider);
        assert_eq!(draft.mode, ProviderConnectDraftMode::Known);
        assert_eq!(draft.provider_id, "openrouter");
        assert_eq!(draft.known_provider_id.as_deref(), Some("openrouter"));
        assert_eq!(
            draft.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(draft.protocol.as_deref(), Some("openai-chat"));
        assert_eq!(draft.env, vec!["OPENROUTER_API_KEY".to_string()]);
    }

    #[test]
    fn configured_provider_descriptor_candidate_exposes_non_secret_projection() {
        let configured = ProviderConfig {
            name: Some(" OpenRouter ".to_string()),
            api_key: Some("secret-123".to_string()),
            base_url: Some(" https://openrouter.ai/api/v1 ".to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            api_style: Some("openai-compatible".to_string()),
            api_shape: Some("chat-completions".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("openai-cached-tokens".to_string()),
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
            api_style: Some("openai-compatible".to_string()),
            api_shape: Some("messages".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("openai-cached-tokens".to_string()),
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
            api_style: Some("openai-compatible".to_string()),
            api_shape: Some("messages".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("openai-cached-tokens".to_string()),
            ..Default::default()
        };

        let response = provider_descriptor_response("broken", &configured);

        assert!(response.descriptor_candidate.is_none());
        assert!(response
            .descriptor_candidate_error
            .as_deref()
            .is_some_and(|error| error.contains("invalid provider profile combination")));
    }

    #[test]
    fn runtime_model_merge_marks_only_the_exact_model_available() {
        let model = |id: &str, available: bool| ModelInfo {
            id: id.to_string(),
            name: id.to_string(),
            provider: "deepseek".to_string(),
            available,
            variants: Vec::new(),
            context_window: None,
            max_output_tokens: None,
            cost_per_million_input: None,
            cost_per_million_output: None,
            capabilities: None,
        };
        let mut models = std::collections::HashMap::from([(
            "deepseek".to_string(),
            std::collections::HashMap::from([
                (
                    "deepseek-v4-flash".to_string(),
                    model("deepseek-v4-flash", false),
                ),
                (
                    "deepseek-v4-pro".to_string(),
                    model("deepseek-v4-pro", false),
                ),
            ]),
        )]);

        super::upsert_runtime_model_info(&mut models, "deepseek", model("deepseek-v4-flash", true));

        let deepseek = &models["deepseek"];
        assert!(deepseek["deepseek-v4-flash"].available);
        assert!(!deepseek["deepseek-v4-pro"].available);
    }
}
