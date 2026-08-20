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

impl ProviderProfileError {
    /// Field name when the error is `MissingField`; used by callers to
    /// distinguish "no profile declared at all" from a partial declaration.
    pub fn missing_field(&self) -> Option<&str> {
        match self {
            Self::MissingField(field) => Some(field),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderApiFamily {
    OpenAiCompatible,
    AnthropicMessages,
}

impl ProviderApiFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::AnthropicMessages => "anthropic-compatible",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderApiShape {
    ChatCompletions,
    Responses,
    AnthropicMessages,
}

impl ProviderApiShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat-completions",
            Self::Responses => "responses",
            Self::AnthropicMessages => "anthropic-messages",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderTransportKind {
    Bearer,
    OAuth,
}

impl ProviderTransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::OAuth => "oauth",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderUsageShape {
    OpenAiCachedTokens,
    AnthropicReadWrite,
    Unknown,
}

impl ProviderUsageShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCachedTokens => "openai-cached-tokens",
            Self::AnthropicReadWrite => "anthropic-read-write",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderQuirk {
    NonStreamingSse,
    RawJsonLines,
    RequiresThinkingReplay,
    IgnoresUnknownFields,
}

impl ProviderQuirk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NonStreamingSse => "non-streaming-sse",
            Self::RawJsonLines => "raw-json-lines",
            Self::RequiresThinkingReplay => "requires-thinking-replay",
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
        Self::try_resolve_with_npm(provider_id, npm, &options).or_else(|error| {
            legacy_default_profile_fallback(
                provider_id,
                npm,
                &options,
                config_provider_has_credentials(provider),
                error,
            )
        })
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
        let npm = option_string(options, "npm")
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
        let mut profile =
            if let Some(profile) = custom_profile_from_options(provider_id, npm, options)? {
                validate_supported_npm(npm)?;
                profile
            } else {
                let provider_key = provider_id.trim().to_ascii_lowercase();
                match provider_key.as_str() {
                    "openai" => builtin_profile(
                        provider_id,
                        npm,
                        "@ai-sdk/openai",
                        ProviderApiFamily::OpenAiCompatible,
                        ProviderApiShape::Responses,
                        ProviderUsageShape::OpenAiCachedTokens,
                        CacheProtocolFamily::OpenAiCompatible,
                    )?,
                    "anthropic" => builtin_profile(
                        provider_id,
                        npm,
                        "@ai-sdk/anthropic",
                        ProviderApiFamily::AnthropicMessages,
                        ProviderApiShape::AnthropicMessages,
                        ProviderUsageShape::AnthropicReadWrite,
                        CacheProtocolFamily::AnthropicCompatible,
                    )?,
                    "deepseek" => builtin_profile(
                        provider_id,
                        npm,
                        "@ai-sdk/openai-compatible",
                        ProviderApiFamily::OpenAiCompatible,
                        ProviderApiShape::ChatCompletions,
                        ProviderUsageShape::OpenAiCachedTokens,
                        CacheProtocolFamily::OpenAiCompatible,
                    )?,
                    _ => {
                        return Err(ProviderProfileError::MissingField(
                            "provider_profile".to_string(),
                        ))
                    }
                }
            };

        // DeepSeek reasoning models require assistant `reasoning_content` on
        // tool-call continuations. This is a provider contract, not an
        // optional user preference, so a custom profile cannot erase it.
        if provider_id.trim().eq_ignore_ascii_case("deepseek") {
            profile.quirks.insert(ProviderQuirk::RequiresThinkingReplay);
        }
        Ok(profile)
    }
}

fn validate_supported_npm(npm: &str) -> Result<(), ProviderProfileError> {
    match npm.trim().to_ascii_lowercase().as_str() {
        "@ai-sdk/openai" | "@ai-sdk/openai-compatible" | "@ai-sdk/anthropic" => Ok(()),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "npm".to_string(),
            value: npm.to_string(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn builtin_profile(
    provider_id: &str,
    npm: &str,
    expected_npm: &str,
    api_family: ProviderApiFamily,
    api_shape: ProviderApiShape,
    usage_shape: ProviderUsageShape,
    cache_family: CacheProtocolFamily,
) -> Result<ProviderProfile, ProviderProfileError> {
    if !npm.trim().eq_ignore_ascii_case(expected_npm) {
        return Err(ProviderProfileError::UnsupportedValue {
            field: "npm".to_string(),
            value: npm.to_string(),
        });
    }
    Ok(ProviderProfile {
        provider_id: provider_id.to_string(),
        npm: expected_npm.to_string(),
        api_family,
        api_shape,
        transport: ProviderTransportKind::Bearer,
        usage_shape,
        cache_family,
        quirks: ProviderQuirks::default(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomProviderProfileConfig {
    api_style: String,
    api_shape: String,
    transport: String,
    usage_shape: String,
    #[serde(default)]
    quirks: Vec<String>,
}

fn custom_profile_from_options(
    provider_id: &str,
    npm: &str,
    options: &HashMap<String, Value>,
) -> Result<Option<ProviderProfile>, ProviderProfileError> {
    let Some(value) = options.get("provider_profile") else {
        return Ok(None);
    };

    let config: CustomProviderProfileConfig = serde_json::from_value(value.clone())
        .map_err(|error| ProviderProfileError::InvalidConfig(error.to_string()))?;
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
            "provider_profile".to_string(),
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
            ProviderApiFamily::OpenAiCompatible => CacheProtocolFamily::OpenAiCompatible,
            ProviderApiFamily::AnthropicMessages => CacheProtocolFamily::AnthropicCompatible,
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
        "anthropic" => "@ai-sdk/anthropic",
        "openai" => "@ai-sdk/openai",
        _ => "@ai-sdk/openai-compatible",
    }
}

/// Default wire profile for a supported SDK npm. Mirrors the table the
/// connect flow writes (`connect_protocol_profile` in agendao-server): the
/// npm package already pins the protocol family, so the profile values are
/// a faithful mapping, not a guess.
pub(crate) fn default_profile_for_npm(npm: &str) -> Option<Value> {
    let (api_style, api_shape, usage_shape) = match npm.trim() {
        "@ai-sdk/openai" => (
            "openai-compatible",
            "responses",
            "openai-cached-tokens",
        ),
        "@ai-sdk/openai-compatible" => (
            "openai-compatible",
            "chat-completions",
            "openai-cached-tokens",
        ),
        "@ai-sdk/anthropic" => (
            "anthropic-compatible",
            "messages",
            "anthropic-read-write",
        ),
        _ => return None,
    };
    Some(serde_json::json!({
        "api_style": api_style,
        "api_shape": api_shape,
        "transport": "bearer",
        "usage_shape": usage_shape,
    }))
}

/// Retry resolution with an npm-derived default profile for credentialed
/// legacy providers that predate the explicit `provider_profile` requirement
/// (opencode-style `options.baseURL`/`options.apiKey` configs). Keyless
/// unknown providers keep the original fail-closed behavior.
pub(crate) fn legacy_default_profile_fallback(
    provider_id: &str,
    npm: &str,
    options: &HashMap<String, Value>,
    has_credentials: bool,
    error: ProviderProfileError,
) -> Result<ProviderProfile, ProviderProfileError> {
    if error.missing_field() != Some("provider_profile") || !has_credentials {
        return Err(error);
    }
    let default = default_profile_for_npm(npm).ok_or_else(|| error.clone())?;
    let mut options = options.clone();
    options.insert("provider_profile".to_string(), default);
    let resolved = ProviderProfileResolver::try_resolve_with_npm(provider_id, npm, &options);
    if resolved.is_ok() {
        tracing::info!(
            provider = provider_id,
            npm = %npm,
            "applied npm-derived default provider profile (legacy config)"
        );
    }
    resolved
}

fn config_provider_has_credentials(provider: &ConfigProvider) -> bool {
    provider
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
        || provider.options.as_ref().is_some_and(|options| {
            ["apiKey", "apikey", "api_key"]
                .iter()
                .any(|key| option_string(options, key).is_some())
        })
}

pub(crate) fn resolve_npm_for_provider(provider_id: &str, provider: &ProviderState) -> String {
    if let Some(npm) = option_string(&provider.options, "npm") {
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

fn parse_api_family(value: &str) -> Result<ProviderApiFamily, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "openai-compatible" => Ok(ProviderApiFamily::OpenAiCompatible),
        "anthropic-messages" | "anthropic-compatible" => Ok(ProviderApiFamily::AnthropicMessages),
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
        "messages" => Ok(ProviderApiShape::AnthropicMessages),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "api_shape".to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_transport(value: &str) -> Result<ProviderTransportKind, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "bearer" => Ok(ProviderTransportKind::Bearer),
        "oauth" => Ok(ProviderTransportKind::OAuth),
        _ => Err(ProviderProfileError::UnsupportedValue {
            field: "transport".to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_usage_shape(value: &str) -> Result<ProviderUsageShape, ProviderProfileError> {
    match normalize_profile_value(value).as_str() {
        "openai-cached-tokens" => Ok(ProviderUsageShape::OpenAiCachedTokens),
        "anthropic-read-write" => Ok(ProviderUsageShape::AnthropicReadWrite),
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
        ProviderApiFamily::OpenAiCompatible => matches!(
            api_shape,
            ProviderApiShape::ChatCompletions | ProviderApiShape::Responses
        ),
        ProviderApiFamily::AnthropicMessages => api_shape == ProviderApiShape::AnthropicMessages,
    };
    if !shape_ok {
        return Err(ProviderProfileError::InvalidCombination(format!(
            "{api_family:?} cannot use {api_shape:?}"
        )));
    }

    let usage_ok = match api_family {
        ProviderApiFamily::OpenAiCompatible => {
            usage_shape == ProviderUsageShape::OpenAiCachedTokens
        }
        ProviderApiFamily::AnthropicMessages => {
            usage_shape == ProviderUsageShape::AnthropicReadWrite
        }
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

fn option_string(options: &HashMap<String, Value>, key: &str) -> Option<String> {
    match options.get(key)? {
        Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_options() -> HashMap<String, Value> {
        HashMap::new()
    }

    #[test]
    fn projects_builtin_openai_responses_profile() {
        let profile =
            ProviderProfileResolver::resolve_with_npm("openai", "@ai-sdk/openai", &empty_options());

        assert_eq!(profile.api_family, ProviderApiFamily::OpenAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::Responses);
        assert_eq!(profile.transport, ProviderTransportKind::Bearer);
        assert_eq!(profile.usage_shape, ProviderUsageShape::OpenAiCachedTokens);
        assert_eq!(profile.cache_family, CacheProtocolFamily::OpenAiCompatible);
        assert!(profile.quirks.as_slice().is_empty());
    }

    #[test]
    fn projects_anthropic_messages_profiles() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "anthropic",
            "@ai-sdk/anthropic",
            &empty_options(),
        );

        assert_eq!(profile.api_family, ProviderApiFamily::AnthropicMessages);
        assert_eq!(profile.api_shape, ProviderApiShape::AnthropicMessages);
        assert_eq!(profile.transport, ProviderTransportKind::Bearer);
        assert_eq!(profile.usage_shape, ProviderUsageShape::AnthropicReadWrite);
        assert_eq!(
            profile.cache_family,
            CacheProtocolFamily::AnthropicCompatible
        );
    }

    #[test]
    fn projects_deepseek_chat_profile_with_required_thinking_replay() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "deepseek",
            "@ai-sdk/openai-compatible",
            &empty_options(),
        );

        assert_eq!(profile.api_family, ProviderApiFamily::OpenAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert!(profile
            .quirks
            .contains(ProviderQuirk::RequiresThinkingReplay));
    }

    #[test]
    fn deepseek_custom_profile_cannot_drop_required_thinking_replay() {
        let options = HashMap::from([(
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "openai-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens",
                "quirks": []
            }),
        )]);

        let profile = ProviderProfileResolver::resolve_with_npm(
            "deepseek",
            "@ai-sdk/openai-compatible",
            &options,
        );
        assert!(profile
            .quirks
            .contains(ProviderQuirk::RequiresThinkingReplay));
    }

    #[test]
    fn custom_provider_without_profile_fails_closed() {
        let error = ProviderProfileResolver::try_resolve_with_npm(
            "custom",
            "@ai-sdk/openai-compatible",
            &empty_options(),
        )
        .expect_err("custom provider must declare a complete profile");

        assert_eq!(
            error,
            ProviderProfileError::MissingField("provider_profile".to_string())
        );
    }

    #[test]
    fn legacy_response_flag_does_not_select_protocol() {
        let mut options = empty_options();
        options.insert("useResponsesApi".to_string(), Value::Bool(true));

        let error = ProviderProfileResolver::try_resolve_with_npm(
            "custom",
            "@ai-sdk/openai-compatible",
            &options,
        )
        .expect_err("legacy flag must not infer a custom profile");
        assert!(matches!(error, ProviderProfileError::MissingField(_)));
    }

    #[test]
    fn profile_does_not_override_unsupported_sdk_shape() {
        let options = HashMap::from([(
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "openai-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens"
            }),
        )]);
        let error = ProviderProfileResolver::try_resolve_with_npm(
            "custom-provider",
            "@custom/unknown-provider",
            &options,
        )
        .expect_err("unsupported SDK shape must fail closed");
        assert!(matches!(
            error,
            ProviderProfileError::UnsupportedValue { ref field, .. } if field == "npm"
        ));
    }

    #[test]
    fn resolves_custom_openai_profile_from_strict_object() {
        let options = HashMap::from([(
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "openai-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens",
                "quirks": ["non-streaming-sse"]
            }),
        )]);

        let profile =
            ProviderProfileResolver::try_resolve_with_options("my-custom", &options).unwrap();

        assert_eq!(profile.api_family, ProviderApiFamily::OpenAiCompatible);
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);
        assert_eq!(profile.transport, ProviderTransportKind::Bearer);
        assert_eq!(profile.usage_shape, ProviderUsageShape::OpenAiCachedTokens);
        assert_eq!(profile.cache_family, CacheProtocolFamily::OpenAiCompatible);
        assert!(profile.quirks.contains(ProviderQuirk::NonStreamingSse));
    }

    #[test]
    fn custom_profile_rejects_flat_fields_and_legacy_object_key() {
        let options = HashMap::from([
            (
                "api_style".to_string(),
                Value::String("anthropic-compatible".to_string()),
            ),
            (
                "api_shape".to_string(),
                Value::String("messages".to_string()),
            ),
            ("transport".to_string(), Value::String("bearer".to_string())),
            (
                "usage_shape".to_string(),
                Value::String("anthropic-read-write".to_string()),
            ),
        ]);

        let error = ProviderProfileResolver::try_resolve_with_options("my-messages", &options)
            .expect_err("flat profile fields must not be accepted");
        assert_eq!(
            error,
            ProviderProfileError::MissingField("provider_profile".to_string())
        );

        let legacy = HashMap::from([(
            "providerProfile".to_string(),
            serde_json::json!({
                "api_style": "openai-compatible",
                "api_shape": "responses",
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens"
            }),
        )]);
        let error = ProviderProfileResolver::try_resolve_with_options("legacy", &legacy)
            .expect_err("legacy profile key must not be accepted");
        assert_eq!(
            error,
            ProviderProfileError::MissingField("provider_profile".to_string())
        );
    }

    #[test]
    fn custom_profile_rejects_unknown_nested_fields() {
        let options = HashMap::from([(
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "openai-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens",
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
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "made-up",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "openai-cached-tokens"
            }),
        )]);

        let error =
            ProviderProfileResolver::try_resolve_with_options("bad", &invalid_value).unwrap_err();
        assert!(matches!(
            error,
            ProviderProfileError::UnsupportedValue { .. }
        ));

        let invalid_combination = HashMap::from([(
            "provider_profile".to_string(),
            serde_json::json!({
                "api_style": "anthropic-compatible",
                "api_shape": "chat-completions",
                "transport": "bearer",
                "usage_shape": "anthropic-read-write"
            }),
        )]);

        let error = ProviderProfileResolver::try_resolve_with_options("bad", &invalid_combination)
            .unwrap_err();
        assert!(matches!(error, ProviderProfileError::InvalidCombination(_)));
    }

    #[test]
    fn legacy_config_provider_resolves_with_npm_derived_default_profile() {
        // opencode-era ConfigProvider: credentials inline in options, no
        // profile fields — try_resolve_config_provider (descriptor/artifact
        // paths) must apply the same legacy fallback as bootstrap.
        let provider = ConfigProvider {
            options: Some(HashMap::from([
                (
                    "baseURL".to_string(),
                    serde_json::json!("https://open.bigmodel.cn/api/coding/paas/v4"),
                ),
                ("apiKey".to_string(), serde_json::json!("legacy-key")),
            ])),
            ..Default::default()
        };

        let profile =
            ProviderProfileResolver::try_resolve_config_provider("zhipuai-coding-plan", &provider)
                .expect("legacy config provider should resolve via fallback");
        assert_eq!(profile.api_shape, ProviderApiShape::ChatCompletions);

        // Keyless config provider without profile keeps failing closed.
        let keyless = ConfigProvider::default();
        assert!(ProviderProfileResolver::try_resolve_config_provider("mystery", &keyless).is_err());
    }
}
