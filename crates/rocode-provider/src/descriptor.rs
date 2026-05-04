use crate::bootstrap::{ConfigProvider, BUNDLED_PROVIDERS};
use crate::cache::CacheProtocolFamily;
use crate::profile::{
    ProviderApiFamily, ProviderApiShape, ProviderProfile, ProviderProfileError,
    ProviderProfileResolver, ProviderQuirk, ProviderTransportKind, ProviderUsageShape,
};
pub use rocode_types::{ProviderConnectionDescriptorCandidate, ProviderProfileDescriptorView};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderDescriptorError {
    #[error("provider_id is required")]
    MissingProviderId,
    #[error(transparent)]
    InvalidProfile(#[from] ProviderProfileError),
}

pub fn provider_connection_descriptor_candidate_from_config_provider(
    provider_id: &str,
    provider: &ConfigProvider,
) -> Result<ProviderConnectionDescriptorCandidate, ProviderDescriptorError> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Err(ProviderDescriptorError::MissingProviderId);
    }

    Ok(ProviderConnectionDescriptorCandidate {
        provider_id: provider_id.to_string(),
        name: trimmed_option(provider.name.as_deref()),
        base_url: trimmed_option(provider.api.as_deref()),
        env: sanitize_env_refs(provider.env.as_ref()),
        profile: resolve_profile_candidate(provider_id, provider)?
            .map(provider_profile_to_descriptor_view),
    })
}

fn resolve_profile_candidate(
    provider_id: &str,
    provider: &ConfigProvider,
) -> Result<Option<ProviderProfile>, ProviderDescriptorError> {
    if !should_project_profile(provider_id, provider) {
        return Ok(None);
    }

    let profile = ProviderProfileResolver::try_resolve_config_provider(provider_id, provider)?;
    Ok(Some(profile))
}

fn should_project_profile(provider_id: &str, provider: &ConfigProvider) -> bool {
    provider
        .npm
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || provider.api_style.is_some()
        || provider.api_shape.is_some()
        || provider.transport.is_some()
        || provider.usage_shape.is_some()
        || provider
            .quirks
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || is_bundled_provider_id(provider_id)
}

fn is_bundled_provider_id(provider_id: &str) -> bool {
    let provider_id = provider_id.trim();
    BUNDLED_PROVIDERS
        .values()
        .any(|known| known.eq_ignore_ascii_case(provider_id))
}

fn sanitize_env_refs(env: Option<&Vec<String>>) -> Vec<String> {
    let mut result = Vec::new();
    for value in env.into_iter().flatten() {
        if let Some(trimmed) = trimmed_option(Some(value)) {
            if !result.contains(&trimmed) {
                result.push(trimmed);
            }
        }
    }
    result
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn provider_profile_to_descriptor_view(profile: ProviderProfile) -> ProviderProfileDescriptorView {
    ProviderProfileDescriptorView {
        provider_id: profile.provider_id,
        npm: profile.npm,
        api_family: provider_api_family_label(profile.api_family).to_string(),
        api_shape: provider_api_shape_label(profile.api_shape).to_string(),
        transport: provider_transport_label(profile.transport).to_string(),
        usage_shape: provider_usage_shape_label(profile.usage_shape).to_string(),
        cache_family: cache_family_label(profile.cache_family).to_string(),
        quirks: profile
            .quirks
            .as_slice()
            .iter()
            .map(|quirk| provider_quirk_label(*quirk).to_string())
            .collect(),
    }
}

fn provider_api_family_label(value: ProviderApiFamily) -> &'static str {
    match value {
        ProviderApiFamily::CloseAiCompatible => "closeai-compatible",
        ProviderApiFamily::EthnopicMessages => "ethnopic-compatible",
        ProviderApiFamily::GeminiGenerate => "gemini-generate",
        ProviderApiFamily::BedrockConverse => "bedrock-converse",
        ProviderApiFamily::Custom => "custom",
    }
}

fn provider_api_shape_label(value: ProviderApiShape) -> &'static str {
    match value {
        ProviderApiShape::ChatCompletions => "chat-completions",
        ProviderApiShape::Responses => "responses",
        ProviderApiShape::EthnopicMessages => "ethnopic-messages",
        ProviderApiShape::GeminiGenerateContent => "gemini-generate-content",
        ProviderApiShape::BedrockConverse => "bedrock-converse",
        ProviderApiShape::Custom => "custom",
    }
}

fn provider_transport_label(value: ProviderTransportKind) -> &'static str {
    match value {
        ProviderTransportKind::Bearer => "bearer",
        ProviderTransportKind::VertexBearer => "vertex-bearer",
        ProviderTransportKind::SigV4 => "sigv4",
        ProviderTransportKind::OAuth => "oauth",
        ProviderTransportKind::PrivateToken => "private-token",
        ProviderTransportKind::HeaderSet => "header-set",
        ProviderTransportKind::Custom => "custom",
    }
}

fn provider_usage_shape_label(value: ProviderUsageShape) -> &'static str {
    match value {
        ProviderUsageShape::CloseAiCachedTokens => "closeai-cached-tokens",
        ProviderUsageShape::EthnopicReadWrite => "ethnopic-read-write",
        ProviderUsageShape::Gemini => "gemini",
        ProviderUsageShape::Bedrock => "bedrock",
        ProviderUsageShape::Unknown => "unknown",
    }
}

fn cache_family_label(value: CacheProtocolFamily) -> &'static str {
    match value {
        CacheProtocolFamily::CloseAiCompatible => "closeai-compatible",
        CacheProtocolFamily::EthnopicCompatible => "ethnopic-compatible",
        CacheProtocolFamily::Disabled => "disabled",
    }
}

fn provider_quirk_label(value: ProviderQuirk) -> &'static str {
    match value {
        ProviderQuirk::NonStreamingSse => "non-streaming-sse",
        ProviderQuirk::RawJsonLines => "raw-json-lines",
        ProviderQuirk::RequiresThinkingReplay => "requires-thinking-replay",
        ProviderQuirk::ResponsesFallbackToChat => "responses-fallback-to-chat",
        ProviderQuirk::IgnoresUnknownFields => "ignores-unknown-fields",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn projects_bundled_provider_profile_from_config_without_runtime_state() {
        let provider = ConfigProvider {
            name: Some(" OpenAI ".to_string()),
            env: Some(vec![" OPENAI_API_KEY ".to_string()]),
            api: Some(" https://api.openai.com/v1 ".to_string()),
            ..Default::default()
        };

        let descriptor =
            provider_connection_descriptor_candidate_from_config_provider("openai", &provider)
                .expect("descriptor should build");

        assert_eq!(descriptor.provider_id, "openai");
        assert_eq!(descriptor.name.as_deref(), Some("OpenAI"));
        assert_eq!(
            descriptor.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(descriptor.env, vec!["OPENAI_API_KEY".to_string()]);

        let profile = descriptor
            .profile
            .expect("bundled provider should project profile");
        assert_eq!(profile.provider_id, "openai");
        assert_eq!(profile.api_family, "closeai-compatible");
        assert_eq!(profile.api_shape, "chat-completions");
        assert_eq!(profile.transport, "bearer");
    }

    #[test]
    fn projects_explicit_custom_profile_without_leaking_runtime_only_fields() {
        let provider = ConfigProvider {
            api: Some("https://custom.example/v1".to_string()),
            api_style: Some("closeai-compatible".to_string()),
            api_shape: Some("responses".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("closeai-cached-tokens".to_string()),
            quirks: Some(vec!["raw-json-lines".to_string()]),
            options: Some(HashMap::from([(
                "apiKey".to_string(),
                serde_json::json!("secret-should-not-leak"),
            )])),
            models: Some(HashMap::new()),
            blacklist: Some(vec!["blocked".to_string()]),
            whitelist: Some(vec!["allowed".to_string()]),
            ..Default::default()
        };

        let descriptor =
            provider_connection_descriptor_candidate_from_config_provider("my-custom", &provider)
                .expect("descriptor should build");
        let value = serde_json::to_value(&descriptor).expect("descriptor should serialize");

        assert!(value.get("options").is_none());
        assert!(value.get("models").is_none());
        assert!(value.get("blacklist").is_none());
        assert!(value.get("whitelist").is_none());
        assert_eq!(
            value["base_url"],
            serde_json::json!("https://custom.example/v1")
        );
        assert_eq!(
            value["profile"]["api_shape"],
            serde_json::json!("responses")
        );
        assert_eq!(
            value["profile"]["quirks"],
            serde_json::json!(["raw-json-lines"])
        );
    }

    #[test]
    fn leaves_profile_absent_for_custom_provider_without_explicit_semantic_input() {
        let provider = ConfigProvider {
            api: Some("https://example.invalid/v1".to_string()),
            env: Some(vec![
                "CUSTOM_API_KEY".to_string(),
                "CUSTOM_API_KEY".to_string(),
            ]),
            ..Default::default()
        };

        let descriptor =
            provider_connection_descriptor_candidate_from_config_provider("my-custom", &provider)
                .expect("descriptor should build");

        assert_eq!(descriptor.env, vec!["CUSTOM_API_KEY".to_string()]);
        assert!(descriptor.profile.is_none());
    }

    #[test]
    fn rejects_missing_provider_id() {
        let provider = ConfigProvider::default();
        let error = provider_connection_descriptor_candidate_from_config_provider("   ", &provider)
            .expect_err("missing provider id should fail");

        assert!(matches!(error, ProviderDescriptorError::MissingProviderId));
    }
}
