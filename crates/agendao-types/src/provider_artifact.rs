use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactVersion {
    #[serde(rename = "agendao-rust/provider/v1")]
    AgendaoRustProviderV1,
}

impl ProviderArtifactVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgendaoRustProviderV1 => "agendao-rust/provider/v1",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactApiFamily {
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "anthropic-compatible")]
    AnthropicCompatible,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactApiShape {
    #[serde(rename = "chat-completions")]
    ChatCompletions,
    #[serde(rename = "responses")]
    Responses,
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactTransport {
    #[serde(rename = "bearer")]
    Bearer,
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactUsageShape {
    #[serde(rename = "openai-cached-tokens")]
    OpenAiCachedTokens,
    #[serde(rename = "anthropic-read-write")]
    AnthropicReadWrite,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderArtifactCacheFamily {
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "anthropic-compatible")]
    AnthropicCompatible,
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
            version: ProviderArtifactVersion::AgendaoRustProviderV1,
            exported_at,
            providers,
        }
    }

    pub fn new_now(providers: Vec<ProviderArtifactEntry>) -> Self {
        Self::new(Utc::now().timestamp_millis(), providers)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderArtifactApiFamily, ProviderArtifactApiShape, ProviderArtifactBundle,
        ProviderArtifactCacheFamily, ProviderArtifactEntry, ProviderArtifactProfile,
        ProviderArtifactQuirk, ProviderArtifactTransport, ProviderArtifactUsageShape,
        ProviderArtifactVersion,
    };

    fn sample_entry() -> ProviderArtifactEntry {
        ProviderArtifactEntry {
            provider_id: "custom-openai".to_string(),
            name: Some("Custom OpenAI endpoint".to_string()),
            base_url: Some("https://models.example/v1".to_string()),
            env: vec!["CUSTOM_OPENAI_API_KEY".to_string()],
            profile: ProviderArtifactProfile {
                npm: "@ai-sdk/openai-compatible".to_string(),
                api_family: ProviderArtifactApiFamily::OpenAiCompatible,
                api_shape: ProviderArtifactApiShape::ChatCompletions,
                transport: ProviderArtifactTransport::Bearer,
                usage_shape: ProviderArtifactUsageShape::OpenAiCachedTokens,
                cache_family: ProviderArtifactCacheFamily::OpenAiCompatible,
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
            serde_json::json!(ProviderArtifactVersion::AgendaoRustProviderV1.as_str())
        );
        assert_eq!(value["exported_at"], serde_json::json!(123));
        assert_eq!(value["providers"].as_array().map(Vec::len), Some(1));
        assert!(value.get("managed").is_none());
    }

    #[test]
    fn bundle_roundtrips_through_current_schema() {
        let bundle = ProviderArtifactBundle::new(123, vec![sample_entry()]);

        let payload = serde_json::to_string(&bundle).expect("bundle should serialize");
        let parsed: ProviderArtifactBundle =
            serde_json::from_str(&payload).expect("bundle should parse");
        assert_eq!(parsed.exported_at, 123);
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].provider_id, "custom-openai");
        assert_eq!(parsed.providers[0].profile.npm, "@ai-sdk/openai-compatible");
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_version() {
        let payload = serde_json::json!({
            "version": "agendao-rust/provider/v999",
            "exported_at": 123,
            "providers": [sample_entry()]
        });

        let error = serde_json::from_value::<ProviderArtifactBundle>(payload)
            .expect_err("unknown version should fail closed");
        assert!(
            error.to_string().contains("did not match any variant")
                || error.to_string().contains("unknown variant")
        );
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_fields() {
        let payload = serde_json::json!({
            "version": "agendao-rust/provider/v1",
            "exported_at": 123,
            "providers": [sample_entry()],
            "extra": true
        });

        let error = serde_json::from_value::<ProviderArtifactBundle>(payload)
            .expect_err("unknown top-level field should fail closed");
        assert!(
            error.to_string().contains("unknown field")
                || error.to_string().contains("did not match any variant")
        );
    }

    #[test]
    fn import_envelope_rejects_unknown_profile_fields() {
        let payload = serde_json::json!({
            "version": "agendao-rust/provider/v1",
            "exported_at": 123,
            "providers": [{
                "provider_id": "openai",
                "profile": {
                    "npm": "@ai-sdk/openai",
                    "api_family": "openai-compatible",
                    "api_shape": "chat-completions",
                    "transport": "bearer",
                    "usage_shape": "openai-cached-tokens",
                    "cache_family": "openai-compatible",
                    "prompt_cache_key": "must-not-be-accepted"
                }
            }]
        });

        let error = serde_json::from_value::<ProviderArtifactBundle>(payload)
            .expect_err("unknown nested profile field should fail closed");
        assert!(
            error.to_string().contains("unknown field")
                || error.to_string().contains("did not match any variant")
        );
    }
}
