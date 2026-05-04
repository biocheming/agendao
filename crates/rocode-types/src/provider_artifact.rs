use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactVersion {
    #[serde(rename = "rocode-rust/provider/v1")]
    RocodeRustProviderV1,
}

impl ProviderArtifactVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RocodeRustProviderV1 => "rocode-rust/provider/v1",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactApiFamily {
    #[serde(rename = "closeai-compatible")]
    CloseAiCompatible,
    #[serde(rename = "ethnopic-compatible")]
    EthnopicCompatible,
    #[serde(rename = "gemini-generate")]
    GeminiGenerate,
    #[serde(rename = "bedrock-converse")]
    BedrockConverse,
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactApiShape {
    #[serde(rename = "chat-completions")]
    ChatCompletions,
    #[serde(rename = "responses")]
    Responses,
    #[serde(rename = "ethnopic-messages")]
    EthnopicMessages,
    #[serde(rename = "gemini-generate-content")]
    GeminiGenerateContent,
    #[serde(rename = "bedrock-converse")]
    BedrockConverse,
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactTransport {
    #[serde(rename = "bearer")]
    Bearer,
    #[serde(rename = "vertex-bearer")]
    VertexBearer,
    #[serde(rename = "sigv4")]
    SigV4,
    #[serde(rename = "oauth")]
    OAuth,
    #[serde(rename = "private-token")]
    PrivateToken,
    #[serde(rename = "header-set")]
    HeaderSet,
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactUsageShape {
    #[serde(rename = "closeai-cached-tokens")]
    CloseAiCachedTokens,
    #[serde(rename = "ethnopic-read-write")]
    EthnopicReadWrite,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "bedrock")]
    Bedrock,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactCacheFamily {
    #[serde(rename = "closeai-compatible")]
    CloseAiCompatible,
    #[serde(rename = "ethnopic-compatible")]
    EthnopicCompatible,
    #[serde(rename = "disabled")]
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactQuirk {
    #[serde(rename = "non-streaming-sse")]
    NonStreamingSse,
    #[serde(rename = "raw-json-lines")]
    RawJsonLines,
    #[serde(rename = "requires-thinking-replay")]
    RequiresThinkingReplay,
    #[serde(rename = "responses-fallback-to-chat")]
    ResponsesFallbackToChat,
    #[serde(rename = "ignores-unknown-fields")]
    IgnoresUnknownFields,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderArtifactProfile {
    pub npm: String,
    pub api_family: ProviderArtifactApiFamily,
    pub api_shape: ProviderArtifactApiShape,
    pub transport: ProviderArtifactTransport,
    pub usage_shape: ProviderArtifactUsageShape,
    pub cache_family: ProviderArtifactCacheFamily,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<ProviderArtifactQuirk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderArtifactEntry {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    pub profile: ProviderArtifactProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderArtifactBundle {
    pub version: ProviderArtifactVersion,
    pub exported_at: i64,
    #[serde(default)]
    pub providers: Vec<ProviderArtifactEntry>,
}

impl ProviderArtifactBundle {
    pub fn new(exported_at: i64, providers: Vec<ProviderArtifactEntry>) -> Self {
        Self {
            version: ProviderArtifactVersion::RocodeRustProviderV1,
            exported_at,
            providers,
        }
    }

    pub fn new_now(providers: Vec<ProviderArtifactEntry>) -> Self {
        Self::new(Utc::now().timestamp_millis(), providers)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProviderArtifactImportEnvelope {
    Bundle(ProviderArtifactBundle),
    Legacy(ProviderArtifactLegacyPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderArtifactLegacyPayload {
    pub legacy_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderArtifactApiFamily, ProviderArtifactApiShape, ProviderArtifactBundle,
        ProviderArtifactCacheFamily, ProviderArtifactEntry, ProviderArtifactImportEnvelope,
        ProviderArtifactProfile, ProviderArtifactQuirk, ProviderArtifactTransport,
        ProviderArtifactUsageShape, ProviderArtifactVersion,
    };

    fn sample_entry() -> ProviderArtifactEntry {
        ProviderArtifactEntry {
            provider_id: "openrouter".to_string(),
            name: Some("OpenRouter".to_string()),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            env: vec!["OPENROUTER_API_KEY".to_string()],
            profile: ProviderArtifactProfile {
                npm: "@openrouter/ai-sdk-provider".to_string(),
                api_family: ProviderArtifactApiFamily::CloseAiCompatible,
                api_shape: ProviderArtifactApiShape::ChatCompletions,
                transport: ProviderArtifactTransport::Bearer,
                usage_shape: ProviderArtifactUsageShape::CloseAiCachedTokens,
                cache_family: ProviderArtifactCacheFamily::CloseAiCompatible,
                quirks: vec![ProviderArtifactQuirk::NonStreamingSse],
            },
        }
    }

    #[test]
    fn bundle_serializes_with_stable_version_and_entries() {
        let bundle = ProviderArtifactBundle::new(123, vec![sample_entry()]);

        let value = serde_json::to_value(&bundle).expect("bundle should serialize");

        assert_eq!(
            value["version"],
            serde_json::json!(ProviderArtifactVersion::RocodeRustProviderV1.as_str())
        );
        assert_eq!(value["exported_at"], serde_json::json!(123));
        assert_eq!(value["providers"].as_array().map(Vec::len), Some(1));
        assert!(value.get("managed").is_none());
    }

    #[test]
    fn bundle_roundtrips_through_import_envelope() {
        let bundle = ProviderArtifactBundle::new(123, vec![sample_entry()]);

        let payload = serde_json::to_string(&bundle).expect("bundle should serialize");
        let envelope: ProviderArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("bundle should parse");

        match envelope {
            ProviderArtifactImportEnvelope::Bundle(parsed) => {
                assert_eq!(parsed.exported_at, 123);
                assert_eq!(parsed.providers.len(), 1);
                assert_eq!(parsed.providers[0].provider_id, "openrouter");
                assert_eq!(
                    parsed.providers[0].profile.npm,
                    "@openrouter/ai-sdk-provider"
                );
            }
            ProviderArtifactImportEnvelope::Legacy(_) => panic!("expected bundle envelope"),
        }
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_version() {
        let payload = serde_json::json!({
            "version": "rocode-rust/provider/v999",
            "exported_at": 123,
            "providers": [sample_entry()]
        });

        let error = serde_json::from_value::<ProviderArtifactImportEnvelope>(payload)
            .expect_err("unknown version should fail closed");
        assert!(
            error.to_string().contains("did not match any variant")
                || error.to_string().contains("unknown variant")
        );
    }

    #[test]
    fn import_envelope_accepts_only_explicit_legacy_shape() {
        let payload = serde_json::json!({
            "legacy_format": "provider-alpha",
            "payload": {
                "providers": [{"provider_id": "legacy-openai"}]
            }
        });

        let envelope: ProviderArtifactImportEnvelope =
            serde_json::from_value(payload).expect("explicit legacy shape should parse");

        match envelope {
            ProviderArtifactImportEnvelope::Legacy(legacy) => {
                assert_eq!(legacy.legacy_format, "provider-alpha");
                assert!(legacy.payload.is_some());
            }
            ProviderArtifactImportEnvelope::Bundle(_) => panic!("expected legacy envelope"),
        }
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_fields() {
        let payload = serde_json::json!({
            "version": "rocode-rust/provider/v1",
            "exported_at": 123,
            "providers": [sample_entry()],
            "extra": true
        });

        let error = serde_json::from_value::<ProviderArtifactImportEnvelope>(payload)
            .expect_err("unknown top-level field should fail closed");
        assert!(
            error.to_string().contains("unknown field")
                || error.to_string().contains("did not match any variant")
        );
    }

    #[test]
    fn import_envelope_rejects_unknown_profile_fields() {
        let payload = serde_json::json!({
            "version": "rocode-rust/provider/v1",
            "exported_at": 123,
            "providers": [{
                "provider_id": "openai",
                "profile": {
                    "npm": "@ai-sdk/openai",
                    "api_family": "closeai-compatible",
                    "api_shape": "chat-completions",
                    "transport": "bearer",
                    "usage_shape": "closeai-cached-tokens",
                    "cache_family": "closeai-compatible",
                    "prompt_cache_key": "must-not-be-accepted"
                }
            }]
        });

        let error = serde_json::from_value::<ProviderArtifactImportEnvelope>(payload)
            .expect_err("unknown nested profile field should fail closed");
        assert!(
            error.to_string().contains("unknown field")
                || error.to_string().contains("did not match any variant")
        );
    }
}
