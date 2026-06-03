use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::bootstrap::{ConfigProvider, ProviderState};
use crate::cache::CacheProtocolFamily;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderProfileError {
    InvalidConfig(String),
    UnsupportedValue { field: String, value: String },
    InvalidCombination(String),
    MissingField(String),
}

impl std::fmt::Display for ProviderProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "invalid provider profile config: {message}"),
            Self::UnsupportedValue { field, value } => {
                write!(f, "unsupported provider profile {field}: {value}")
            }
            Self::InvalidCombination(message) => {
                write!(f, "invalid provider profile combination: {message}")
            }
            Self::MissingField(field) => write!(f, "missing provider profile field: {field}"),
        }
    }
}

impl std::error::Error for ProviderProfileError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderApiFamily {
    CloseAiCompatible,
    EthnopicMessages,
    GeminiGenerate,
    BedrockConverse,
    Custom,
}

impl ProviderApiFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloseAiCompatible => "closeai-compatible",
            Self::EthnopicMessages => "ethnopic-compatible",
            Self::GeminiGenerate => "gemini-generate",
            Self::BedrockConverse => "bedrock-converse",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderApiShape {
    ChatCompletions,
    Responses,
    EthnopicMessages,
    GeminiGenerateContent,
    BedrockConverse,
    Custom,
}

impl ProviderApiShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat-completions",
            Self::Responses => "responses",
            Self::EthnopicMessages => "ethnopic-messages",
            Self::GeminiGenerateContent => "gemini-generate-content",
            Self::BedrockConverse => "bedrock-converse",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderTransportKind {
    Bearer,
    VertexBearer,
    SigV4,
    OAuth,
    PrivateToken,
    HeaderSet,
    Custom,
}

impl ProviderTransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::VertexBearer => "vertex-bearer",
            Self::SigV4 => "sigv4",
            Self::OAuth => "oauth",
            Self::PrivateToken => "private-token",
            Self::HeaderSet => "header-set",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderUsageShape {
    CloseAiCachedTokens,
    EthnopicReadWrite,
    Gemini,
    Bedrock,
    Unknown,
}

impl ProviderUsageShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloseAiCachedTokens => "closeai-cached-tokens",
            Self::EthnopicReadWrite => "ethnopic-read-write",
            Self::Gemini => "gemini",
            Self::Bedrock => "bedrock",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderQuirk {
    NonStreamingSse,
    RawJsonLines,
    RequiresThinkingReplay,
    ResponsesFallbackToChat,
    IgnoresUnknownFields,
}

impl ProviderQuirk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NonStreamingSse => "non-streaming-sse",
            Self::RawJsonLines => "raw-json-lines",
            Self::RequiresThinkingReplay => "requires-thinking-replay",
            Self::ResponsesFallbackToChat => "responses-fallback-to-chat",
            Self::IgnoresUnknownFields => "ignores-unknown-fields",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderQuirks {
    quirks: Vec<ProviderQuirk>,
}

impl ProviderQuirks {
    pub fn new(quirks: impl IntoIterator<Item = ProviderQuirk>) -> Self {
        let mut result = Self::default();
        for quirk in quirks {
            result.insert(quirk);
        }
        result
    }

    pub fn contains(&self, quirk: ProviderQuirk) -> bool {
        self.quirks.contains(&quirk)
    }

    pub fn insert(&mut self, quirk: ProviderQuirk) {
        if !self.contains(quirk) {
            self.quirks.push(quirk);
        }
    }

    pub fn as_slice(&self) -> &[ProviderQuirk] {
        &self.quirks
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub provider_id: String,
    pub npm: String,
    pub api_family: ProviderApiFamily,
    pub api_shape: ProviderApiShape,
    pub transport: ProviderTransportKind,
    pub usage_shape: ProviderUsageShape,
    pub cache_family: CacheProtocolFamily,
    pub quirks: ProviderQuirks,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderProfileResolver;

impl ProviderProfileResolver {
    pub fn resolve(provider_id: &str, provider: &ProviderState) -> ProviderProfile {
        Self::try_resolve(provider_id, provider).expect("provider profile should resolve")
    }

    pub fn try_resolve(
        provider_id: &str,
        provider: &ProviderState,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        let npm = resolve_npm_for_provider(provider_id, provider);
        Self::try_resolve_with_npm(provider_id, &npm, &provider.options)
    }

    pub fn try_resolve_config_provider(
        provider_id: &str,
        provider: &ConfigProvider,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        let options = explicit_profile_options_from_config_provider(provider);
        let npm = provider
            .npm
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_npm_for_provider_id(provider_id));
        Self::try_resolve_with_npm(provider_id, npm, &options)
    }

    pub fn resolve_with_options(
        provider_id: &str,
        options: &HashMap<String, Value>,
    ) -> ProviderProfile {
        Self::try_resolve_with_options(provider_id, options)
            .expect("provider profile should resolve")
    }

    pub fn try_resolve_with_options(
        provider_id: &str,
        options: &HashMap<String, Value>,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        let provider_key = provider_id.trim().to_ascii_lowercase();
        let npm = option_string(options, &["npm"])
            .unwrap_or_else(|| default_npm_for_provider_id(&provider_key).to_string());
        Self::try_resolve_with_npm(provider_id, &npm, options)
    }

    pub fn resolve_with_npm(
        provider_id: &str,
        npm: &str,
        options: &HashMap<String, Value>,
    ) -> ProviderProfile {
        Self::try_resolve_with_npm(provider_id, npm, options)
            .expect("provider profile should resolve")
    }

    pub fn try_resolve_with_npm(
        provider_id: &str,
        npm: &str,
        options: &HashMap<String, Value>,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        if let Some(profile) = custom_profile_from_options(provider_id, npm, options)? {
            return Ok(profile);
        }

        let provider_key = provider_id.trim().to_ascii_lowercase();
        let npm_key = npm.trim().to_ascii_lowercase();

        let (api_family, api_shape, transport, usage_shape, cache_family) =
            classify_provider(&provider_key, &npm_key, options);

        let mut quirks = ProviderQuirks::default();
        if provider_key.contains("zhipu") || provider_key.contains("bigmodel") {
            quirks.insert(ProviderQuirk::NonStreamingSse);
        }
        if provider_key.contains("deepseek") {
            quirks.insert(ProviderQuirk::RequiresThinkingReplay);
        }
        if provider_key.contains("github-copilot") {
            quirks.insert(ProviderQuirk::ResponsesFallbackToChat);
        }
        if option_bool(options, &["nonStreamingSse", "non_streaming_sse"]).unwrap_or(false) {
            quirks.insert(ProviderQuirk::NonStreamingSse);
        }
        if option_bool(options, &["rawJsonLines", "raw_json_lines"]).unwrap_or(false) {
            quirks.insert(ProviderQuirk::RawJsonLines);
        }

        Ok(ProviderProfile {
            provider_id: provider_id.to_string(),
            npm: npm.to_string(),
            api_family,
            api_shape,
            transport,
            usage_shape,
            cache_family,
            quirks,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomProviderProfileConfig {
    #[serde(alias = "apiStyle", alias = "api_family", alias = "apiFamily")]
    api_style: String,
    #[serde(alias = "apiShape")]
    api_shape: String,
    transport: String,
    #[serde(alias = "usageShape")]
    usage_shape: String,
    #[serde(default)]
    quirks: Vec<String>,
}

fn custom_profile_from_options(
    provider_id: &str,
    npm: &str,
    options: &HashMap<String, Value>,
) -> Result<Option<ProviderProfile>, ProviderProfileError> {
    if let Some(value) =
        options_get_insensitive_any(options, &["providerProfile", "provider_profile", "profile"])
    {
        let config: CustomProviderProfileConfig = serde_json::from_value(value.clone())
            .map_err(|error| ProviderProfileError::InvalidConfig(error.to_string()))?;
        return config.into_profile(provider_id, npm).map(Some);
    }

    if !custom_profile_flat_keys_present(options) {
        return Ok(None);
    }

    let config = CustomProviderProfileConfig {
        api_style: required_option_string(
            options,
            "api_style",
            &["api_style", "apiStyle", "api_family", "apiFamily"],
        )?,
        api_shape: required_option_string(options, "api_shape", &["api_shape", "apiShape"])?,
        transport: required_option_string(options, "transport", &["transport"])?,
        usage_shape: required_option_string(
            options,
            "usage_shape",
            &["usage_shape", "usageShape"],
        )?,
        quirks: option_string_list(options, &["quirks"])?,
    };
    config.into_profile(provider_id, npm).map(Some)
}

fn explicit_profile_options_from_config_provider(
    provider: &ConfigProvider,
) -> HashMap<String, serde_json::Value> {
    let mut options = HashMap::new();
    let mut profile = serde_json::Map::new();
    if let Some(value) = provider
        .api_style
        .as_ref()
        .and_then(|value| trimmed_option(Some(value)))
    {
        profile.insert("api_style".to_string(), serde_json::Value::String(value));
    }
    if let Some(value) = provider
        .api_shape
        .as_ref()
        .and_then(|value| trimmed_option(Some(value)))
    {
        profile.insert("api_shape".to_string(), serde_json::Value::String(value));
    }
    if let Some(value) = provider
        .transport
        .as_ref()
        .and_then(|value| trimmed_option(Some(value)))
    {
        profile.insert("transport".to_string(), serde_json::Value::String(value));
    }
    if let Some(value) = provider
        .usage_shape
        .as_ref()
        .and_then(|value| trimmed_option(Some(value)))
    {
        profile.insert("usage_shape".to_string(), serde_json::Value::String(value));
    }
    if let Some(quirks) = provider.quirks.as_ref() {
        let quirks = quirks
            .iter()
            .filter_map(|value| trimmed_option(Some(value)))
            .map(serde_json::Value::String)
            .collect::<Vec<_>>();
        if !quirks.is_empty() {
            profile.insert("quirks".to_string(), serde_json::Value::Array(quirks));
        }
    }

    if !profile.is_empty() {
        options.insert(
            "providerProfile".to_string(),
            serde_json::Value::Object(profile),
        );
    }
    options
}

impl CustomProviderProfileConfig {
    fn into_profile(
        self,
        provider_id: &str,
        npm: &str,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        let api_family = parse_api_family(&self.api_style)?;
        let api_shape = parse_api_shape(&self.api_shape)?;
        let transport = parse_transport(&self.transport)?;
        let usage_shape = parse_usage_shape(&self.usage_shape)?;
        let quirks = parse_quirks(&self.quirks)?;
        validate_profile_combination(api_family, api_shape, usage_shape)?;
        let cache_family = match api_family {
            ProviderApiFamily::CloseAiCompatible => CacheProtocolFamily::CloseAiCompatible,
            ProviderApiFamily::EthnopicMessages => CacheProtocolFamily::EthnopicCompatible,
            ProviderApiFamily::GeminiGenerate
            | ProviderApiFamily::BedrockConverse
            | ProviderApiFamily::Custom => CacheProtocolFamily::Disabled,
        };

        Ok(ProviderProfile {
            provider_id: provider_id.to_string(),
            npm: npm.to_string(),
            api_family,
            api_shape,
            transport,
            usage_shape,
            cache_family,
            quirks,
        })
    }
}

pub(crate) fn default_npm_for_provider_id(provider_id: &str) -> &'static str {
    let provider_id = provider_id.trim().to_ascii_lowercase();
    match provider_id.as_str() {
        "ethnopic" => "@ai-sdk/anthropic",
        "google" => "@ai-sdk/google",
        "google-vertex" | "google-vertex-ethnopic" => "@ai-sdk/google-vertex",
        "amazon-bedrock" => "@ai-sdk/amazon-bedrock",
        "github-copilot" | "github-copilot-enterprise" => "@ai-sdk/github-copilot",
        "gitlab" => "@gitlab/gitlab-ai-provider",
        "openai" => "@ai-sdk/openai",
        _ => "@ai-sdk/openai-compatible",
    }
}

pub(crate) fn resolve_npm_for_provider(provider_id: &str, provider: &ProviderState) -> String {
    if let Some(npm) = provider_option_string(provider, &["npm"]) {
        return npm;
    }

    if let Some(npm) = provider
        .models
        .values()
        .find_map(|model| (!model.api.npm.trim().is_empty()).then(|| model.api.npm.clone()))
    {
        return npm;
    }

    default_npm_for_provider_id(provider_id).to_string()
}

fn classify_provider(
    provider_id: &str,
    npm: &str,
    options: &HashMap<String, Value>,
) -> (
    ProviderApiFamily,
    ProviderApiShape,
    ProviderTransportKind,
    ProviderUsageShape,
    CacheProtocolFamily,
) {
    if provider_id.contains("gitlab") || npm.contains("gitlab") {
        return (
            ProviderApiFamily::CloseAiCompatible,
            ProviderApiShape::ChatCompletions,
            ProviderTransportKind::PrivateToken,
            ProviderUsageShape::CloseAiCachedTokens,
            CacheProtocolFamily::CloseAiCompatible,
        );
    }

    if provider_id.contains("github-copilot") || npm.contains("github-copilot") {
        return (
            ProviderApiFamily::CloseAiCompatible,
            ProviderApiShape::ChatCompletions,
            ProviderTransportKind::OAuth,
            ProviderUsageShape::CloseAiCachedTokens,
            CacheProtocolFamily::CloseAiCompatible,
        );
    }

    if provider_id.contains("google-vertex") || npm.contains("google-vertex") {
        return (
            ProviderApiFamily::GeminiGenerate,
            ProviderApiShape::GeminiGenerateContent,
            ProviderTransportKind::VertexBearer,
            ProviderUsageShape::Gemini,
            CacheProtocolFamily::Disabled,
        );
    }

    if provider_id.contains("google") || provider_id.contains("gemini") || npm.contains("google") {
        return (
            ProviderApiFamily::GeminiGenerate,
            ProviderApiShape::GeminiGenerateContent,
            ProviderTransportKind::Bearer,
            ProviderUsageShape::Gemini,
            CacheProtocolFamily::Disabled,
        );
    }

    if provider_id.contains("bedrock") || npm.contains("bedrock") {
        return (
            ProviderApiFamily::BedrockConverse,
            ProviderApiShape::BedrockConverse,
            ProviderTransportKind::SigV4,
            ProviderUsageShape::Bedrock,
            CacheProtocolFamily::Disabled,
        );
    }

    if ((provider_id.contains("anthropic") || provider_id.contains("ethnopic"))
        && !provider_id.contains("vertex"))
        || ((npm.contains("anthropic") || npm.contains("ethnopic")) && !npm.contains("vertex"))
    {
        return (
            ProviderApiFamily::EthnopicMessages,
            ProviderApiShape::EthnopicMessages,
            ProviderTransportKind::Bearer,
            ProviderUsageShape::EthnopicReadWrite,
            CacheProtocolFamily::EthnopicCompatible,
        );
    }

    let api_shape =
        if option_bool(options, &["useResponsesApi", "use_responses_api"]).unwrap_or(false) {
            ProviderApiShape::Responses
        } else {
            ProviderApiShape::ChatCompletions
        };

    (
        ProviderApiFamily::CloseAiCompatible,
        api_shape,
        ProviderTransportKind::Bearer,
        ProviderUsageShape::CloseAiCachedTokens,
        CacheProtocolFamily::CloseAiCompatible,
    )
}

fn parse_api_family(value: &str) -> Result<ProviderApiFamily, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "closeai-compatible" | "openai-compatible" => Ok(ProviderApiFamily::CloseAiCompatible),
        "ethnopic-messages" | "ethnopic-compatible" | "anthropic-messages" => {
            Ok(ProviderApiFamily::EthnopicMessages)
        }
        "gemini-generate" | "gemini" => Ok(ProviderApiFamily::GeminiGenerate),
        "bedrock-converse" | "bedrock" => Ok(ProviderApiFamily::BedrockConverse),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "api_style".to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_api_shape(value: &str) -> Result<ProviderApiShape, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "chat-completions" => Ok(ProviderApiShape::ChatCompletions),
        "responses" => Ok(ProviderApiShape::Responses),
        "messages" => Ok(ProviderApiShape::EthnopicMessages),
        "gemini-generate-content" | "generate-content" => {
            Ok(ProviderApiShape::GeminiGenerateContent)
        }
        "bedrock-converse" => Ok(ProviderApiShape::BedrockConverse),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "api_shape".to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_transport(value: &str) -> Result<ProviderTransportKind, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "bearer" => Ok(ProviderTransportKind::Bearer),
        "vertex-bearer" | "vertex" => Ok(ProviderTransportKind::VertexBearer),
        "sigv4" => Ok(ProviderTransportKind::SigV4),
        "oauth" => Ok(ProviderTransportKind::OAuth),
        "private-token" => Ok(ProviderTransportKind::PrivateToken),
        "header-set" => Ok(ProviderTransportKind::HeaderSet),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "transport".to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_usage_shape(value: &str) -> Result<ProviderUsageShape, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "closeai-cached-tokens" | "openai-cached-tokens" => {
            Ok(ProviderUsageShape::CloseAiCachedTokens)
        }
        "ethnopic-read-write" | "anthropic-read-write" => Ok(ProviderUsageShape::EthnopicReadWrite),
        "gemini" => Ok(ProviderUsageShape::Gemini),
        "bedrock" => Ok(ProviderUsageShape::Bedrock),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "usage_shape".to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_quirks(values: &[String]) -> Result<ProviderQuirks, ProviderProfileError> {
    let mut quirks = ProviderQuirks::default();
    for value in values {
        let quirk = match normalize_profile_value(value).as_str() {
            "non-streaming-sse" => ProviderQuirk::NonStreamingSse,
            "raw-json-lines" => ProviderQuirk::RawJsonLines,
            "requires-thinking-replay" => ProviderQuirk::RequiresThinkingReplay,
            "responses-fallback-to-chat" => ProviderQuirk::ResponsesFallbackToChat,
            "ignores-unknown-fields" => ProviderQuirk::IgnoresUnknownFields,
            _ => {
                return Err(ProviderProfileError::UnsupportedValue {
                    field: "quirks".to_string(),
                    value: value.clone(),
                })
            }
        };
        quirks.insert(quirk);
    }
    Ok(quirks)
}

fn validate_profile_combination(
    api_family: ProviderApiFamily,
    api_shape: ProviderApiShape,
    usage_shape: ProviderUsageShape,
) -> Result<(), ProviderProfileError> {
    let shape_ok = match api_family {
        ProviderApiFamily::CloseAiCompatible => matches!(
            api_shape,
            ProviderApiShape::ChatCompletions | ProviderApiShape::Responses
        ),
        ProviderApiFamily::EthnopicMessages => api_shape == ProviderApiShape::EthnopicMessages,
        ProviderApiFamily::GeminiGenerate => api_shape == ProviderApiShape::GeminiGenerateContent,
        ProviderApiFamily::BedrockConverse => api_shape == ProviderApiShape::BedrockConverse,
        ProviderApiFamily::Custom => false,
    };
    if !shape_ok {
        return Err(ProviderProfileError::InvalidCombination(format!(
            "{api_family:?} cannot use {api_shape:?}"
        )));
    }

    let usage_ok = match api_family {
        ProviderApiFamily::CloseAiCompatible => {
            usage_shape == ProviderUsageShape::CloseAiCachedTokens
        }
        ProviderApiFamily::EthnopicMessages => usage_shape == ProviderUsageShape::EthnopicReadWrite,
        ProviderApiFamily::GeminiGenerate => usage_shape == ProviderUsageShape::Gemini,
        ProviderApiFamily::BedrockConverse => usage_shape == ProviderUsageShape::Bedrock,
        ProviderApiFamily::Custom => false,
    };
    if !usage_ok {
        return Err(ProviderProfileError::InvalidCombination(format!(
            "{api_family:?} cannot use {usage_shape:?}"
        )));
    }

    Ok(())
}

fn normalize_profile_value(value: &str) -> String {
    value.trim().replace('_', "-").to_ascii_lowercase()
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn provider_option_string(provider: &ProviderState, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = options_get_insensitive(&provider.options, key) else {
            continue;
        };
        match value {
            Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
            Value::Number(n) => return Some(n.to_string()),
            Value::Bool(b) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}

fn options_get_insensitive<'a>(
    options: &'a HashMap<String, Value>,
    key: &str,
) -> Option<&'a Value> {
    if let Some(value) = options.get(key) {
        return Some(value);
    }
    let key_lower = key.to_lowercase();
    options
        .iter()
        .find_map(|(name, value)| (name.to_lowercase() == key_lower).then_some(value))
}

fn options_get_insensitive_any<'a>(
    options: &'a HashMap<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| options_get_insensitive(options, key))
}

fn custom_profile_flat_keys_present(options: &HashMap<String, Value>) -> bool {
    [
        "api_style",
        "apiStyle",
        "api_family",
        "apiFamily",
        "api_shape",
        "apiShape",
        "transport",
        "usage_shape",
        "usageShape",
        "quirks",
    ]
    .iter()
    .any(|key| options_get_insensitive(options, key).is_some())
}

fn required_option_string(
    options: &HashMap<String, Value>,
    field: &str,
    keys: &[&str],
) -> Result<String, ProviderProfileError> {
    option_string(options, keys)
        .ok_or_else(|| ProviderProfileError::MissingField(field.to_string()))
}

fn option_string_list(
    options: &HashMap<String, Value>,
    keys: &[&str],
) -> Result<Vec<String>, ProviderProfileError> {
    let Some(value) = options_get_insensitive_any(options, keys) else {
        return Ok(Vec::new());
    };

    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str().map(ToString::to_string).ok_or_else(|| {
                    ProviderProfileError::InvalidConfig("quirks must be string array".to_string())
                })
            })
            .collect(),
        _ => Err(ProviderProfileError::InvalidConfig(
            "quirks must be string array".to_string(),
        )),
    }
}

fn option_bool(options: &HashMap<String, Value>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        let Some(value) = options_get_insensitive(options, key) else {
            continue;
        };
        match value {
            Value::Bool(v) => return Some(*v),
            Value::Number(n) => return Some(n.as_i64().unwrap_or(0) != 0),
            Value::String(s) => {
                let lower = s.trim().to_ascii_lowercase();
                if matches!(lower.as_str(), "1" | "true" | "yes" | "on") {
                    return Some(true);
                }
                if matches!(lower.as_str(), "0" | "false" | "no" | "off") {
                    return Some(false);
                }
            }
            _ => {}
        }
    }
    None
}

fn option_string(options: &HashMap<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = options_get_insensitive(options, key) else {
            continue;
        };
        match value {
            Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
            Value::Number(n) => return Some(n.to_string()),
            Value::Bool(b) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_options() -> HashMap<String, Value> {
        HashMap::new()
    }

    #[test]
    fn projects_closeai_compatible_profiles() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "deepseek",
            "@ai-sdk/openai-compatible",
            &empty_options(),
        );

        assert_eq!(profile.api_family, ProviderApiFamily::CloseAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert_eq!(profile.transport, ProviderTransportKind::Bearer);
        assert_eq!(profile.usage_shape, ProviderUsageShape::CloseAiCachedTokens);
        assert_eq!(profile.cache_family, CacheProtocolFamily::CloseAiCompatible);
        assert!(profile
            .quirks
            .contains(ProviderQuirk::RequiresThinkingReplay));
    }

    #[test]
    fn projects_ethnopic_messages_profiles() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "ethnopic",
            "@ai-sdk/anthropic",
            &empty_options(),
        );

        assert_eq!(profile.api_family, ProviderApiFamily::EthnopicMessages);
        assert_eq!(profile.api_shape, ProviderApiShape::EthnopicMessages);
        assert_eq!(profile.transport, ProviderTransportKind::Bearer);
        assert_eq!(profile.usage_shape, ProviderUsageShape::EthnopicReadWrite);
        assert_eq!(
            profile.cache_family,
            CacheProtocolFamily::EthnopicCompatible
        );
    }

    #[test]
    fn projects_gitlab_as_private_token_closeai() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "gitlab",
            "@gitlab/gitlab-ai-provider",
            &empty_options(),
        );

        assert_eq!(profile.api_family, ProviderApiFamily::CloseAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert_eq!(profile.transport, ProviderTransportKind::PrivateToken);
        assert_eq!(profile.cache_family, CacheProtocolFamily::CloseAiCompatible);
    }

    #[test]
    fn projects_copilot_as_oauth_closeai_with_route_quirk() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "github-copilot",
            "@ai-sdk/github-copilot",
            &empty_options(),
        );

        assert_eq!(profile.api_family, ProviderApiFamily::CloseAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert_eq!(profile.transport, ProviderTransportKind::OAuth);
        assert!(profile
            .quirks
            .contains(ProviderQuirk::ResponsesFallbackToChat));
    }

    #[test]
    fn projects_vertex_and_bedrock_as_non_cache_families() {
        let vertex = ProviderProfileResolver::resolve_with_npm(
            "google-vertex",
            "@ai-sdk/google-vertex",
            &empty_options(),
        );
        let bedrock = ProviderProfileResolver::resolve_with_npm(
            "amazon-bedrock",
            "@ai-sdk/amazon-bedrock",
            &empty_options(),
        );

        assert_eq!(vertex.api_family, ProviderApiFamily::GeminiGenerate);
        assert_eq!(vertex.api_shape, ProviderApiShape::GeminiGenerateContent);
        assert_eq!(vertex.transport, ProviderTransportKind::VertexBearer);
        assert_eq!(vertex.cache_family, CacheProtocolFamily::Disabled);
        assert_eq!(bedrock.api_family, ProviderApiFamily::BedrockConverse);
        assert_eq!(bedrock.transport, ProviderTransportKind::SigV4);
        assert_eq!(bedrock.cache_family, CacheProtocolFamily::Disabled);
    }

    #[test]
    fn response_shape_is_explicit_capability_not_family_change() {
        let mut options = empty_options();
        options.insert("useResponsesApi".to_string(), Value::Bool(true));

        let profile =
            ProviderProfileResolver::resolve_with_npm("openai", "@ai-sdk/openai", &options);

        assert_eq!(profile.api_family, ProviderApiFamily::CloseAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::Responses);
        assert_eq!(profile.cache_family, CacheProtocolFamily::CloseAiCompatible);
    }

    #[test]
    fn resolve_with_options_uses_npm_override_before_default() {
        let mut options = empty_options();
        options.insert(
            "npm".to_string(),
            Value::String("@ai-sdk/anthropic".to_string()),
        );

        let profile = ProviderProfileResolver::resolve_with_options("custom-provider", &options);

        assert_eq!(profile.npm, "@ai-sdk/anthropic");
        assert_eq!(profile.api_family, ProviderApiFamily::EthnopicMessages);
    }

    #[test]
    fn resolves_custom_closeai_profile_from_strict_object() {
        let options = HashMap::from([(
            "providerProfile".to_string(),
            serde_json::json!({
                "api_style": "closeai-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "closeai-cached-tokens",
                "quirks": ["non-streaming-sse"]
            }),
        )]);

        let profile =
            ProviderProfileResolver::try_resolve_with_options("my-custom", &options).unwrap();

        assert_eq!(profile.api_family, ProviderApiFamily::CloseAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert_eq!(profile.transport, ProviderTransportKind::Bearer);
        assert_eq!(profile.usage_shape, ProviderUsageShape::CloseAiCachedTokens);
        assert_eq!(profile.cache_family, CacheProtocolFamily::CloseAiCompatible);
        assert!(profile.quirks.contains(ProviderQuirk::NonStreamingSse));
    }

    #[test]
    fn resolves_custom_messages_profile_from_flat_fields() {
        let options = HashMap::from([
            (
                "api_style".to_string(),
                Value::String("ethnopic-compatible".to_string()),
            ),
            (
                "api_shape".to_string(),
                Value::String("messages".to_string()),
            ),
            ("transport".to_string(), Value::String("bearer".to_string())),
            (
                "usage_shape".to_string(),
                Value::String("ethnopic-read-write".to_string()),
            ),
        ]);

        let profile =
            ProviderProfileResolver::try_resolve_with_options("my-messages", &options).unwrap();

        assert_eq!(profile.api_family, ProviderApiFamily::EthnopicMessages);
        assert_eq!(profile.api_shape, ProviderApiShape::EthnopicMessages);
        assert_eq!(
            profile.cache_family,
            CacheProtocolFamily::EthnopicCompatible
        );
    }

    #[test]
    fn custom_profile_rejects_unknown_nested_fields() {
        let options = HashMap::from([(
            "providerProfile".to_string(),
            serde_json::json!({
                "api_style": "closeai-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "closeai-cached-tokens",
                "prompt_cache_key": "must-not-be-accepted"
            }),
        )]);

        let error =
            ProviderProfileResolver::try_resolve_with_options("my-custom", &options).unwrap_err();

        assert!(matches!(error, ProviderProfileError::InvalidConfig(_)));
    }

    #[test]
    fn custom_profile_rejects_invalid_values_and_combinations() {
        let invalid_value = HashMap::from([(
            "providerProfile".to_string(),
            serde_json::json!({
                "api_style": "made-up",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "closeai-cached-tokens"
            }),
        )]);

        let error =
            ProviderProfileResolver::try_resolve_with_options("bad", &invalid_value).unwrap_err();
        assert!(matches!(
            error,
            ProviderProfileError::UnsupportedValue { .. }
        ));

        let invalid_combination = HashMap::from([(
            "providerProfile".to_string(),
            serde_json::json!({
                "api_style": "ethnopic-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "ethnopic-read-write"
            }),
        )]);

        let error = ProviderProfileResolver::try_resolve_with_options("bad", &invalid_combination)
            .unwrap_err();
        assert!(matches!(error, ProviderProfileError::InvalidCombination(_)));
    }
}
