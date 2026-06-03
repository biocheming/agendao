use std::collections::{HashMap, HashSet};

use agendao_types::{
    ProviderArtifactApiFamily, ProviderArtifactApiShape, ProviderArtifactBundle,
    ProviderArtifactCacheFamily, ProviderArtifactEntry, ProviderArtifactImportEnvelope,
    ProviderArtifactLegacyPayload, ProviderArtifactProfile, ProviderArtifactQuirk,
    ProviderArtifactTransport, ProviderArtifactUsageShape,
};

use crate::bootstrap::ConfigProvider;
use crate::cache::{CacheProtocolFamily, ProviderProfileFingerprint};
use crate::profile::{
    ProviderApiFamily, ProviderApiShape, ProviderProfile, ProviderProfileError,
    ProviderProfileResolver, ProviderQuirk, ProviderQuirks, ProviderTransportKind,
    ProviderUsageShape,
};

pub trait ProviderArtifactLegacyAdapter {
    fn legacy_format(&self) -> &'static str;

    fn import_entries(
        &self,
        payload: &ProviderArtifactLegacyPayload,
    ) -> Result<Vec<ProviderArtifactEntry>, ProviderArtifactError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderArtifactError {
    #[error("provider_id is required in canonical provider artifact")]
    MissingProviderId,
    #[error("provider `{provider_id}` is missing required profile field `{field}`")]
    MissingProfileField { provider_id: String, field: String },
    #[error("duplicate provider id in canonical provider artifact: {provider_id}")]
    DuplicateProviderId { provider_id: String },
    #[error(
        "provider `{provider_id}` has unsupported non-core field `{field}`; provider artifact v1 does not own it"
    )]
    UnsupportedNonCoreField { provider_id: String, field: String },
    #[error(
        "provider `{provider_id}` declared profile does not match resolved profile after import"
    )]
    ProfileParityMismatch { provider_id: String },
    #[error("provider `{provider_id}` fingerprint parity mismatch after import")]
    FingerprintParityMismatch { provider_id: String },
    #[error(
        "unsupported legacy provider artifact format: {legacy_format} (explicit legacy adapter required)"
    )]
    UnsupportedLegacyFormat { legacy_format: String },
    #[error(transparent)]
    InvalidProfile(#[from] ProviderProfileError),
}

pub fn export_provider_artifact_bundle(
    configured: &HashMap<String, ConfigProvider>,
) -> Result<ProviderArtifactBundle, ProviderArtifactError> {
    let mut provider_ids = configured.keys().cloned().collect::<Vec<_>>();
    provider_ids.sort_by(|left, right| {
        left.to_ascii_lowercase()
            .cmp(&right.to_ascii_lowercase())
            .then_with(|| left.cmp(right))
    });

    let mut providers = Vec::with_capacity(provider_ids.len());
    for provider_id in provider_ids {
        let provider = configured
            .get(&provider_id)
            .expect("provider id should exist while exporting bundle");
        providers.push(export_provider_artifact_entry(&provider_id, provider)?);
    }

    Ok(ProviderArtifactBundle::new_now(providers))
}

pub fn export_provider_artifact_entry(
    provider_id: &str,
    provider: &ConfigProvider,
) -> Result<ProviderArtifactEntry, ProviderArtifactError> {
    let provider_id = canonical_provider_id(provider_id)?;
    validate_exportable_provider(&provider_id, provider)?;
    let profile = ProviderProfileResolver::try_resolve_config_provider(&provider_id, provider)?;

    Ok(ProviderArtifactEntry {
        provider_id,
        name: trimmed_option(provider.name.as_deref()),
        base_url: trimmed_option(provider.api.as_deref()),
        env: sanitize_env_refs(provider.env.as_ref()),
        profile: provider_profile_to_artifact(&profile),
    })
}

pub fn import_provider_artifact_bundle(
    payload: ProviderArtifactImportEnvelope,
) -> Result<HashMap<String, ConfigProvider>, ProviderArtifactError> {
    import_provider_artifact_bundle_with_legacy_adapter(payload, None)
}

pub fn import_provider_artifact_bundle_with_legacy_adapter(
    payload: ProviderArtifactImportEnvelope,
    legacy_adapter: Option<&dyn ProviderArtifactLegacyAdapter>,
) -> Result<HashMap<String, ConfigProvider>, ProviderArtifactError> {
    let entries = resolve_entries_from_artifact(payload, legacy_adapter)?;
    validate_unique_provider_ids(&entries)?;

    let mut providers = HashMap::with_capacity(entries.len());
    for entry in entries {
        let provider_id = canonical_provider_id(&entry.provider_id)?;
        let provider = config_provider_from_artifact_entry(&provider_id, &entry)?;
        providers.insert(provider_id, provider);
    }

    Ok(providers)
}

fn resolve_entries_from_artifact(
    payload: ProviderArtifactImportEnvelope,
    legacy_adapter: Option<&dyn ProviderArtifactLegacyAdapter>,
) -> Result<Vec<ProviderArtifactEntry>, ProviderArtifactError> {
    match payload {
        ProviderArtifactImportEnvelope::Bundle(bundle) => Ok(bundle.providers),
        ProviderArtifactImportEnvelope::Legacy(legacy) => match legacy_adapter {
            Some(adapter) if adapter.legacy_format() == legacy.legacy_format => {
                adapter.import_entries(&legacy)
            }
            _ => Err(ProviderArtifactError::UnsupportedLegacyFormat {
                legacy_format: legacy.legacy_format,
            }),
        },
    }
}

fn validate_unique_provider_ids(
    entries: &[ProviderArtifactEntry],
) -> Result<(), ProviderArtifactError> {
    let mut seen = HashSet::new();
    for entry in entries {
        let provider_id = canonical_provider_id(&entry.provider_id)?;
        let key = provider_id.to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(ProviderArtifactError::DuplicateProviderId { provider_id });
        }
    }
    Ok(())
}

fn validate_exportable_provider(
    provider_id: &str,
    provider: &ConfigProvider,
) -> Result<(), ProviderArtifactError> {
    if provider
        .options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
    {
        return Err(ProviderArtifactError::UnsupportedNonCoreField {
            provider_id: provider_id.to_string(),
            field: "options".to_string(),
        });
    }
    if provider
        .models
        .as_ref()
        .is_some_and(|models| !models.is_empty())
    {
        return Err(ProviderArtifactError::UnsupportedNonCoreField {
            provider_id: provider_id.to_string(),
            field: "models".to_string(),
        });
    }
    if provider
        .whitelist
        .as_ref()
        .is_some_and(|whitelist| !whitelist.is_empty())
    {
        return Err(ProviderArtifactError::UnsupportedNonCoreField {
            provider_id: provider_id.to_string(),
            field: "whitelist".to_string(),
        });
    }
    if provider
        .blacklist
        .as_ref()
        .is_some_and(|blacklist| !blacklist.is_empty())
    {
        return Err(ProviderArtifactError::UnsupportedNonCoreField {
            provider_id: provider_id.to_string(),
            field: "blacklist".to_string(),
        });
    }
    Ok(())
}

fn config_provider_from_artifact_entry(
    provider_id: &str,
    entry: &ProviderArtifactEntry,
) -> Result<ConfigProvider, ProviderArtifactError> {
    let npm = trimmed_option(Some(entry.profile.npm.as_str())).ok_or_else(|| {
        ProviderArtifactError::MissingProfileField {
            provider_id: provider_id.to_string(),
            field: "npm".to_string(),
        }
    })?;

    let quirks = artifact_quirks_to_strings(&entry.profile.quirks);
    let provider = ConfigProvider {
        name: trimmed_option(entry.name.as_deref()),
        env: {
            let env = sanitize_env_refs(Some(&entry.env));
            (!env.is_empty()).then_some(env)
        },
        api: trimmed_option(entry.base_url.as_deref()),
        npm: Some(npm.clone()),
        api_style: Some(artifact_api_family_label(entry.profile.api_family).to_string()),
        api_shape: Some(artifact_api_shape_label(entry.profile.api_shape).to_string()),
        transport: Some(artifact_transport_label(entry.profile.transport).to_string()),
        usage_shape: Some(artifact_usage_shape_label(entry.profile.usage_shape).to_string()),
        quirks: (!quirks.is_empty()).then_some(quirks),
        options: None,
        models: None,
        blacklist: None,
        whitelist: None,
    };

    let declared_profile = artifact_profile_to_provider_profile(provider_id, &entry.profile);
    let resolved_profile =
        ProviderProfileResolver::try_resolve_config_provider(provider_id, &provider)?;
    if resolved_profile != declared_profile {
        return Err(ProviderArtifactError::ProfileParityMismatch {
            provider_id: provider_id.to_string(),
        });
    }

    let declared_fingerprint = ProviderProfileFingerprint::from_profile(&declared_profile);
    let resolved_fingerprint = ProviderProfileFingerprint::from_profile(&resolved_profile);
    if declared_fingerprint != resolved_fingerprint {
        return Err(ProviderArtifactError::FingerprintParityMismatch {
            provider_id: provider_id.to_string(),
        });
    }

    Ok(provider)
}

fn canonical_provider_id(provider_id: &str) -> Result<String, ProviderArtifactError> {
    trimmed_option(Some(provider_id)).ok_or(ProviderArtifactError::MissingProviderId)
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn sanitize_env_refs(env: Option<&Vec<String>>) -> Vec<String> {
    let mut sanitized = Vec::new();
    for value in env.into_iter().flatten() {
        let Some(value) = trimmed_option(Some(value)) else {
            continue;
        };
        if !sanitized.contains(&value) {
            sanitized.push(value);
        }
    }
    sanitized
}

fn provider_profile_to_artifact(profile: &ProviderProfile) -> ProviderArtifactProfile {
    ProviderArtifactProfile {
        npm: profile.npm.clone(),
        api_family: provider_api_family_to_artifact(profile.api_family),
        api_shape: provider_api_shape_to_artifact(profile.api_shape),
        transport: provider_transport_to_artifact(profile.transport),
        usage_shape: provider_usage_shape_to_artifact(profile.usage_shape),
        cache_family: cache_family_to_artifact(profile.cache_family),
        quirks: profile
            .quirks
            .as_slice()
            .iter()
            .copied()
            .map(provider_quirk_to_artifact)
            .collect(),
    }
}

fn artifact_profile_to_provider_profile(
    provider_id: &str,
    profile: &ProviderArtifactProfile,
) -> ProviderProfile {
    ProviderProfile {
        provider_id: provider_id.to_string(),
        npm: profile.npm.trim().to_string(),
        api_family: artifact_api_family_to_provider(profile.api_family),
        api_shape: artifact_api_shape_to_provider(profile.api_shape),
        transport: artifact_transport_to_provider(profile.transport),
        usage_shape: artifact_usage_shape_to_provider(profile.usage_shape),
        cache_family: artifact_cache_family_to_provider(profile.cache_family),
        quirks: ProviderQuirks::new(
            profile
                .quirks
                .iter()
                .copied()
                .map(artifact_quirk_to_provider),
        ),
    }
}

fn artifact_api_family_label(value: ProviderArtifactApiFamily) -> &'static str {
    match value {
        ProviderArtifactApiFamily::CloseAiCompatible => "closeai-compatible",
        ProviderArtifactApiFamily::EthnopicCompatible => "ethnopic-compatible",
        ProviderArtifactApiFamily::GeminiGenerate => "gemini-generate",
        ProviderArtifactApiFamily::BedrockConverse => "bedrock-converse",
        ProviderArtifactApiFamily::Custom => "custom",
    }
}

fn artifact_api_shape_label(value: ProviderArtifactApiShape) -> &'static str {
    match value {
        ProviderArtifactApiShape::ChatCompletions => "chat-completions",
        ProviderArtifactApiShape::Responses => "responses",
        ProviderArtifactApiShape::EthnopicMessages => "ethnopic-messages",
        ProviderArtifactApiShape::GeminiGenerateContent => "gemini-generate-content",
        ProviderArtifactApiShape::BedrockConverse => "bedrock-converse",
        ProviderArtifactApiShape::Custom => "custom",
    }
}

fn artifact_transport_label(value: ProviderArtifactTransport) -> &'static str {
    match value {
        ProviderArtifactTransport::Bearer => "bearer",
        ProviderArtifactTransport::VertexBearer => "vertex-bearer",
        ProviderArtifactTransport::SigV4 => "sigv4",
        ProviderArtifactTransport::OAuth => "oauth",
        ProviderArtifactTransport::PrivateToken => "private-token",
        ProviderArtifactTransport::HeaderSet => "header-set",
        ProviderArtifactTransport::Custom => "custom",
    }
}

fn artifact_usage_shape_label(value: ProviderArtifactUsageShape) -> &'static str {
    match value {
        ProviderArtifactUsageShape::CloseAiCachedTokens => "closeai-cached-tokens",
        ProviderArtifactUsageShape::EthnopicReadWrite => "ethnopic-read-write",
        ProviderArtifactUsageShape::Gemini => "gemini",
        ProviderArtifactUsageShape::Bedrock => "bedrock",
        ProviderArtifactUsageShape::Unknown => "unknown",
    }
}

fn artifact_quirks_to_strings(quirks: &[ProviderArtifactQuirk]) -> Vec<String> {
    quirks
        .iter()
        .map(|quirk| match quirk {
            ProviderArtifactQuirk::NonStreamingSse => "non-streaming-sse".to_string(),
            ProviderArtifactQuirk::RawJsonLines => "raw-json-lines".to_string(),
            ProviderArtifactQuirk::RequiresThinkingReplay => "requires-thinking-replay".to_string(),
            ProviderArtifactQuirk::ResponsesFallbackToChat => {
                "responses-fallback-to-chat".to_string()
            }
            ProviderArtifactQuirk::IgnoresUnknownFields => "ignores-unknown-fields".to_string(),
        })
        .collect()
}

fn provider_api_family_to_artifact(value: ProviderApiFamily) -> ProviderArtifactApiFamily {
    match value {
        ProviderApiFamily::CloseAiCompatible => ProviderArtifactApiFamily::CloseAiCompatible,
        ProviderApiFamily::EthnopicMessages => ProviderArtifactApiFamily::EthnopicCompatible,
        ProviderApiFamily::GeminiGenerate => ProviderArtifactApiFamily::GeminiGenerate,
        ProviderApiFamily::BedrockConverse => ProviderArtifactApiFamily::BedrockConverse,
        ProviderApiFamily::Custom => ProviderArtifactApiFamily::Custom,
    }
}

fn provider_api_shape_to_artifact(value: ProviderApiShape) -> ProviderArtifactApiShape {
    match value {
        ProviderApiShape::ChatCompletions => ProviderArtifactApiShape::ChatCompletions,
        ProviderApiShape::Responses => ProviderArtifactApiShape::Responses,
        ProviderApiShape::EthnopicMessages => ProviderArtifactApiShape::EthnopicMessages,
        ProviderApiShape::GeminiGenerateContent => ProviderArtifactApiShape::GeminiGenerateContent,
        ProviderApiShape::BedrockConverse => ProviderArtifactApiShape::BedrockConverse,
        ProviderApiShape::Custom => ProviderArtifactApiShape::Custom,
    }
}

fn provider_transport_to_artifact(value: ProviderTransportKind) -> ProviderArtifactTransport {
    match value {
        ProviderTransportKind::Bearer => ProviderArtifactTransport::Bearer,
        ProviderTransportKind::VertexBearer => ProviderArtifactTransport::VertexBearer,
        ProviderTransportKind::SigV4 => ProviderArtifactTransport::SigV4,
        ProviderTransportKind::OAuth => ProviderArtifactTransport::OAuth,
        ProviderTransportKind::PrivateToken => ProviderArtifactTransport::PrivateToken,
        ProviderTransportKind::HeaderSet => ProviderArtifactTransport::HeaderSet,
        ProviderTransportKind::Custom => ProviderArtifactTransport::Custom,
    }
}

fn provider_usage_shape_to_artifact(value: ProviderUsageShape) -> ProviderArtifactUsageShape {
    match value {
        ProviderUsageShape::CloseAiCachedTokens => ProviderArtifactUsageShape::CloseAiCachedTokens,
        ProviderUsageShape::EthnopicReadWrite => ProviderArtifactUsageShape::EthnopicReadWrite,
        ProviderUsageShape::Gemini => ProviderArtifactUsageShape::Gemini,
        ProviderUsageShape::Bedrock => ProviderArtifactUsageShape::Bedrock,
        ProviderUsageShape::Unknown => ProviderArtifactUsageShape::Unknown,
    }
}

fn cache_family_to_artifact(value: CacheProtocolFamily) -> ProviderArtifactCacheFamily {
    match value {
        CacheProtocolFamily::CloseAiCompatible => ProviderArtifactCacheFamily::CloseAiCompatible,
        CacheProtocolFamily::EthnopicCompatible => ProviderArtifactCacheFamily::EthnopicCompatible,
        CacheProtocolFamily::Disabled => ProviderArtifactCacheFamily::Disabled,
    }
}

fn provider_quirk_to_artifact(value: ProviderQuirk) -> ProviderArtifactQuirk {
    match value {
        ProviderQuirk::NonStreamingSse => ProviderArtifactQuirk::NonStreamingSse,
        ProviderQuirk::RawJsonLines => ProviderArtifactQuirk::RawJsonLines,
        ProviderQuirk::RequiresThinkingReplay => ProviderArtifactQuirk::RequiresThinkingReplay,
        ProviderQuirk::ResponsesFallbackToChat => ProviderArtifactQuirk::ResponsesFallbackToChat,
        ProviderQuirk::IgnoresUnknownFields => ProviderArtifactQuirk::IgnoresUnknownFields,
    }
}

fn artifact_api_family_to_provider(value: ProviderArtifactApiFamily) -> ProviderApiFamily {
    match value {
        ProviderArtifactApiFamily::CloseAiCompatible => ProviderApiFamily::CloseAiCompatible,
        ProviderArtifactApiFamily::EthnopicCompatible => ProviderApiFamily::EthnopicMessages,
        ProviderArtifactApiFamily::GeminiGenerate => ProviderApiFamily::GeminiGenerate,
        ProviderArtifactApiFamily::BedrockConverse => ProviderApiFamily::BedrockConverse,
        ProviderArtifactApiFamily::Custom => ProviderApiFamily::Custom,
    }
}

fn artifact_api_shape_to_provider(value: ProviderArtifactApiShape) -> ProviderApiShape {
    match value {
        ProviderArtifactApiShape::ChatCompletions => ProviderApiShape::ChatCompletions,
        ProviderArtifactApiShape::Responses => ProviderApiShape::Responses,
        ProviderArtifactApiShape::EthnopicMessages => ProviderApiShape::EthnopicMessages,
        ProviderArtifactApiShape::GeminiGenerateContent => ProviderApiShape::GeminiGenerateContent,
        ProviderArtifactApiShape::BedrockConverse => ProviderApiShape::BedrockConverse,
        ProviderArtifactApiShape::Custom => ProviderApiShape::Custom,
    }
}

fn artifact_transport_to_provider(value: ProviderArtifactTransport) -> ProviderTransportKind {
    match value {
        ProviderArtifactTransport::Bearer => ProviderTransportKind::Bearer,
        ProviderArtifactTransport::VertexBearer => ProviderTransportKind::VertexBearer,
        ProviderArtifactTransport::SigV4 => ProviderTransportKind::SigV4,
        ProviderArtifactTransport::OAuth => ProviderTransportKind::OAuth,
        ProviderArtifactTransport::PrivateToken => ProviderTransportKind::PrivateToken,
        ProviderArtifactTransport::HeaderSet => ProviderTransportKind::HeaderSet,
        ProviderArtifactTransport::Custom => ProviderTransportKind::Custom,
    }
}

fn artifact_usage_shape_to_provider(value: ProviderArtifactUsageShape) -> ProviderUsageShape {
    match value {
        ProviderArtifactUsageShape::CloseAiCachedTokens => ProviderUsageShape::CloseAiCachedTokens,
        ProviderArtifactUsageShape::EthnopicReadWrite => ProviderUsageShape::EthnopicReadWrite,
        ProviderArtifactUsageShape::Gemini => ProviderUsageShape::Gemini,
        ProviderArtifactUsageShape::Bedrock => ProviderUsageShape::Bedrock,
        ProviderArtifactUsageShape::Unknown => ProviderUsageShape::Unknown,
    }
}

fn artifact_cache_family_to_provider(value: ProviderArtifactCacheFamily) -> CacheProtocolFamily {
    match value {
        ProviderArtifactCacheFamily::CloseAiCompatible => CacheProtocolFamily::CloseAiCompatible,
        ProviderArtifactCacheFamily::EthnopicCompatible => CacheProtocolFamily::EthnopicCompatible,
        ProviderArtifactCacheFamily::Disabled => CacheProtocolFamily::Disabled,
    }
}

fn artifact_quirk_to_provider(value: ProviderArtifactQuirk) -> ProviderQuirk {
    match value {
        ProviderArtifactQuirk::NonStreamingSse => ProviderQuirk::NonStreamingSse,
        ProviderArtifactQuirk::RawJsonLines => ProviderQuirk::RawJsonLines,
        ProviderArtifactQuirk::RequiresThinkingReplay => ProviderQuirk::RequiresThinkingReplay,
        ProviderArtifactQuirk::ResponsesFallbackToChat => ProviderQuirk::ResponsesFallbackToChat,
        ProviderArtifactQuirk::IgnoresUnknownFields => ProviderQuirk::IgnoresUnknownFields,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        export_provider_artifact_bundle, export_provider_artifact_entry,
        import_provider_artifact_bundle, import_provider_artifact_bundle_with_legacy_adapter,
        ProviderArtifactError, ProviderArtifactLegacyAdapter,
    };
    use crate::bootstrap::ConfigProvider;
    use crate::cache::ProviderProfileFingerprint;
    use crate::profile::{ProviderApiShape, ProviderProfileResolver, ProviderQuirk};
    use agendao_types::{
        ProviderArtifactBundle, ProviderArtifactCacheFamily, ProviderArtifactEntry,
        ProviderArtifactImportEnvelope, ProviderArtifactLegacyPayload, ProviderArtifactProfile,
        ProviderArtifactQuirk,
    };
    use std::collections::HashMap;

    struct AlphaLegacyAdapter;

    impl ProviderArtifactLegacyAdapter for AlphaLegacyAdapter {
        fn legacy_format(&self) -> &'static str {
            "provider-alpha"
        }

        fn import_entries(
            &self,
            payload: &ProviderArtifactLegacyPayload,
        ) -> Result<Vec<ProviderArtifactEntry>, ProviderArtifactError> {
            #[derive(serde::Deserialize)]
            struct LegacyProvider {
                provider_id: String,
                npm: String,
            }

            #[derive(serde::Deserialize)]
            struct LegacyPayload {
                providers: Vec<LegacyProvider>,
            }

            let raw =
                payload
                    .payload
                    .clone()
                    .ok_or(ProviderArtifactError::MissingProfileField {
                        provider_id: "legacy".to_string(),
                        field: "payload".to_string(),
                    })?;
            let parsed: LegacyPayload = serde_json::from_value(raw).map_err(|error| {
                ProviderArtifactError::MissingProfileField {
                    provider_id: "legacy".to_string(),
                    field: error.to_string(),
                }
            })?;
            Ok(parsed
                .providers
                .into_iter()
                .map(|provider| sample_custom_entry(&provider.provider_id, &provider.npm))
                .collect())
        }
    }

    fn sample_custom_provider() -> ConfigProvider {
        ConfigProvider {
            name: Some("My Custom".to_string()),
            env: Some(vec![
                "CUSTOM_API_KEY".to_string(),
                "CUSTOM_API_KEY".to_string(),
            ]),
            api: Some(" https://custom.example/v1 ".to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            api_style: Some("closeai-compatible".to_string()),
            api_shape: Some("responses".to_string()),
            transport: Some("bearer".to_string()),
            usage_shape: Some("closeai-cached-tokens".to_string()),
            quirks: Some(vec![
                "non-streaming-sse".to_string(),
                "raw-json-lines".to_string(),
            ]),
            options: None,
            models: None,
            whitelist: None,
            blacklist: None,
        }
    }

    fn sample_custom_entry(provider_id: &str, npm: &str) -> ProviderArtifactEntry {
        ProviderArtifactEntry {
            provider_id: provider_id.to_string(),
            name: Some("My Custom".to_string()),
            base_url: Some("https://custom.example/v1".to_string()),
            env: vec!["CUSTOM_API_KEY".to_string()],
            profile: ProviderArtifactProfile {
                npm: npm.to_string(),
                api_family: agendao_types::ProviderArtifactApiFamily::CloseAiCompatible,
                api_shape: agendao_types::ProviderArtifactApiShape::Responses,
                transport: agendao_types::ProviderArtifactTransport::Bearer,
                usage_shape: agendao_types::ProviderArtifactUsageShape::CloseAiCachedTokens,
                cache_family: ProviderArtifactCacheFamily::CloseAiCompatible,
                quirks: vec![
                    ProviderArtifactQuirk::NonStreamingSse,
                    ProviderArtifactQuirk::RawJsonLines,
                ],
            },
        }
    }

    #[test]
    fn export_provider_bundle_sorts_ids_and_sanitizes_descriptor_fields() {
        let providers = HashMap::from([
            ("zeta".to_string(), sample_custom_provider()),
            (
                "alpha".to_string(),
                ConfigProvider {
                    name: Some(" OpenAI ".to_string()),
                    env: Some(vec![" OPENAI_API_KEY ".to_string()]),
                    api: Some(" https://api.openai.com/v1 ".to_string()),
                    ..Default::default()
                },
            ),
        ]);

        let bundle = export_provider_artifact_bundle(&providers).expect("export should succeed");

        assert_eq!(bundle.providers.len(), 2);
        assert_eq!(bundle.providers[0].provider_id, "alpha");
        assert_eq!(bundle.providers[0].name.as_deref(), Some("OpenAI"));
        assert_eq!(
            bundle.providers[0].base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(bundle.providers[0].env, vec!["OPENAI_API_KEY".to_string()]);
        assert_eq!(bundle.providers[1].provider_id, "zeta");
        assert_eq!(bundle.providers[1].env, vec!["CUSTOM_API_KEY".to_string()]);
        assert_eq!(
            bundle.providers[1].profile.api_shape,
            agendao_types::ProviderArtifactApiShape::Responses
        );
    }

    #[test]
    fn export_provider_entry_rejects_non_core_fields() {
        let provider = ConfigProvider {
            options: Some(HashMap::from([(
                "apiKey".to_string(),
                serde_json::json!("secret"),
            )])),
            ..Default::default()
        };

        let error =
            export_provider_artifact_entry("openai", &provider).expect_err("options should fail");
        assert!(matches!(
            error,
            ProviderArtifactError::UnsupportedNonCoreField { .. }
        ));
    }

    #[test]
    fn import_provider_bundle_preserves_profile_and_fingerprint_parity() {
        let exported = export_provider_artifact_bundle(&HashMap::from([(
            "my-custom".to_string(),
            sample_custom_provider(),
        )]))
        .expect("export");
        let payload = serde_json::to_string(&exported).expect("serialize");
        let parsed: ProviderArtifactImportEnvelope = serde_json::from_str(&payload).expect("parse");

        let imported = import_provider_artifact_bundle(parsed).expect("import should succeed");
        let provider = imported.get("my-custom").expect("provider");
        let resolved = ProviderProfileResolver::try_resolve_config_provider("my-custom", provider)
            .expect("profile should resolve");
        let replayed = export_provider_artifact_bundle(&imported).expect("re-export");

        assert_eq!(replayed.version, exported.version);
        assert_eq!(replayed.providers, exported.providers);
        assert_eq!(resolved.api_shape, ProviderApiShape::Responses);
        assert!(resolved.quirks.contains(ProviderQuirk::NonStreamingSse));
        assert!(resolved.quirks.contains(ProviderQuirk::RawJsonLines));
        assert_eq!(
            ProviderProfileFingerprint::from_profile(&resolved),
            ProviderProfileFingerprint::from_profile(
                &ProviderProfileResolver::try_resolve_config_provider(
                    "my-custom",
                    &sample_custom_provider()
                )
                .expect("source profile")
            )
        );
    }

    #[test]
    fn import_provider_bundle_rejects_profile_parity_mismatch() {
        let mut entry = sample_custom_entry("broken", "@ai-sdk/openai-compatible");
        entry.profile.cache_family = ProviderArtifactCacheFamily::Disabled;

        let error = import_provider_artifact_bundle(ProviderArtifactImportEnvelope::Bundle(
            ProviderArtifactBundle::new(123, vec![entry]),
        ))
        .expect_err("cache family mismatch should fail");

        assert!(matches!(
            error,
            ProviderArtifactError::ProfileParityMismatch { .. }
                | ProviderArtifactError::FingerprintParityMismatch { .. }
        ));
    }

    #[test]
    fn import_provider_bundle_rejects_duplicate_provider_ids() {
        let first = sample_custom_entry("dup", "@ai-sdk/openai-compatible");
        let second = sample_custom_entry(" DUP ", "@ai-sdk/openai-compatible");

        let error = import_provider_artifact_bundle(ProviderArtifactImportEnvelope::Bundle(
            ProviderArtifactBundle::new(123, vec![first, second]),
        ))
        .expect_err("duplicate provider ids should fail");

        assert!(matches!(
            error,
            ProviderArtifactError::DuplicateProviderId { .. }
        ));
    }

    #[test]
    fn import_provider_bundle_rejects_legacy_payload_without_explicit_adapter() {
        let envelope =
            ProviderArtifactImportEnvelope::Legacy(agendao_types::ProviderArtifactLegacyPayload {
                legacy_format: "provider-alpha".to_string(),
                payload: Some(serde_json::json!({"providers": []})),
            });

        let error =
            import_provider_artifact_bundle(envelope).expect_err("legacy should fail closed");
        assert!(matches!(
            error,
            ProviderArtifactError::UnsupportedLegacyFormat { .. }
        ));
    }

    #[test]
    fn import_provider_bundle_accepts_matching_explicit_legacy_adapter() {
        let envelope =
            ProviderArtifactImportEnvelope::Legacy(agendao_types::ProviderArtifactLegacyPayload {
                legacy_format: "provider-alpha".to_string(),
                payload: Some(serde_json::json!({
                    "providers": [{
                        "provider_id": "legacy-openai",
                        "npm": "@ai-sdk/openai-compatible"
                    }]
                })),
            });

        let imported = import_provider_artifact_bundle_with_legacy_adapter(
            envelope,
            Some(&AlphaLegacyAdapter),
        )
        .expect("legacy adapter should be accepted");

        assert!(imported.contains_key("legacy-openai"));
    }
}
