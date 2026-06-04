mod capabilities;
mod config;
mod explain;
mod parts;
mod ports;
mod render;
mod types;

pub use capabilities::{
    ModalityKind, ModalityPreflightResult, ModalitySupportView, ModalityTransportResult,
    MultimodalCapabilitiesResponse, MultimodalCapabilityAuthority, MultimodalPolicyResponse,
    MultimodalPreflightRequest, MultimodalPreflightResponse, PreflightCapabilityView,
    PreflightInputPart,
};
pub use config::{MultimodalConfigView, ResolvedMultimodalConfig, ResolvedVoiceConfig};
pub use explain::{
    PersistedMultimodalAttachmentInfo, PersistedMultimodalExplain, RuntimeMultimodalExplain,
};
pub use parts::{MultimodalPart, SessionPartAdapter};
pub use ports::{AttachmentLoaderPort, CapturePort};
pub use render::MultimodalDisplaySummary;
pub use types::{CapturedAssetKind, CapturedAssetSource};

pub use agendao_config::{
    MultimodalAttachmentPolicyConfig, MultimodalConfig, MultimodalLimitsConfig,
};

use agendao_config::{Config, VoiceConfig};

#[derive(Debug, Clone)]
pub struct MultimodalAuthority {
    resolved: ResolvedMultimodalConfig,
}

impl MultimodalAuthority {
    pub fn from_config(config: &Config) -> Self {
        Self {
            resolved: ResolvedMultimodalConfig::from_config(config),
        }
    }

    pub fn resolved(&self) -> &ResolvedMultimodalConfig {
        &self.resolved
    }

    pub fn config_view(&self) -> MultimodalConfigView {
        MultimodalConfigView::from_resolved(&self.resolved)
    }

    pub fn voice_config(&self) -> &ResolvedVoiceConfig {
        &self.resolved.voice
    }

    pub fn merged_voice_config(config: &Config) -> VoiceConfig {
        ResolvedMultimodalConfig::merged_voice_config(config)
    }

    pub fn voice_part_from_data_url(
        &self,
        data_url: impl Into<String>,
        filename: impl Into<String>,
        mime: impl Into<String>,
        byte_len: usize,
    ) -> MultimodalPart {
        MultimodalPart::file(
            data_url,
            Some(filename.into()),
            Some(mime.into()),
            CapturedAssetKind::Audio,
            Some(byte_len),
            CapturedAssetSource::VoiceCapture,
        )
    }

    pub fn build_display_summary(
        &self,
        primary_text: Option<&str>,
        parts: &[MultimodalPart],
    ) -> MultimodalDisplaySummary {
        MultimodalDisplaySummary::from_parts(primary_text, parts)
    }

    pub fn capability_authority(&self) -> MultimodalCapabilityAuthority<'_> {
        MultimodalCapabilityAuthority::new(&self.resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_config::{VoiceCommandConfig, VoiceConfig};
    use agendao_provider::bootstrap::{
        InterleavedConfig, ModalitySet, ModelCapabilities, ModelCostCache, ProviderModel,
        ProviderModelApi, ProviderModelCost, ProviderModelLimit,
    };
    use std::collections::HashMap;

    #[test]
    fn resolved_voice_config_prefers_multimodal_over_legacy_voice() {
        let config = Config {
            voice: Some(VoiceConfig {
                duration_seconds: Some(12),
                attach_audio: Some(true),
                mime: Some("audio/wav".to_string()),
                language: Some("zh".to_string()),
                record: Some(VoiceCommandConfig {
                    command: vec!["ffmpeg".to_string(), "{file}".to_string()],
                    env: HashMap::new(),
                }),
                transcribe: None,
            }),
            multimodal: Some(MultimodalConfig {
                voice: Some(VoiceConfig {
                    duration_seconds: Some(30),
                    attach_audio: Some(false),
                    mime: None,
                    language: Some("en".to_string()),
                    record: None,
                    transcribe: Some(VoiceCommandConfig {
                        command: vec!["whisper".to_string(), "{file}".to_string()],
                        env: HashMap::new(),
                    }),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let resolved = ResolvedMultimodalConfig::from_config(&config);
        assert_eq!(resolved.voice.duration_seconds, 30);
        assert!(!resolved.voice.attach_audio);
        assert_eq!(resolved.voice.language.as_deref(), Some("en"));
        assert_eq!(
            resolved
                .voice
                .record
                .as_ref()
                .map(|value| value.command.clone()),
            Some(vec!["ffmpeg".to_string(), "{file}".to_string()])
        );
        assert_eq!(
            resolved
                .voice
                .transcribe
                .as_ref()
                .map(|value| value.command.clone()),
            Some(vec!["whisper".to_string(), "{file}".to_string()])
        );
    }

    #[test]
    fn authority_builds_audio_display_summary() {
        let authority = MultimodalAuthority::from_config(&Config::default());
        let parts = vec![authority.voice_part_from_data_url(
            "data:audio/wav;base64,UklGRg==",
            "voice.wav",
            "audio/wav",
            8,
        )];

        let summary = authority.build_display_summary(None, &parts);
        assert_eq!(summary.attachment_count, 1);
        assert_eq!(summary.compact_label, "[audio input]");
        assert_eq!(summary.badges, vec!["audio".to_string()]);
    }

    #[test]
    fn session_part_adapter_classifies_audio_files_for_preflight() {
        let session_parts = vec![agendao_types::PromptPart::File {
            url: "data:audio/wav;base64,UklGRg==".to_string(),
            filename: Some("voice.wav".to_string()),
            mime: Some("audio/wav".to_string()),
        }];

        let multimodal_parts = SessionPartAdapter::from_session_parts(&session_parts);
        assert_eq!(multimodal_parts.len(), 1);

        let summary = MultimodalDisplaySummary::from_parts(None, &multimodal_parts);
        assert_eq!(summary.compact_label, "[audio input]");

        let preflight = SessionPartAdapter::preflight_parts_from_session_parts(&session_parts);
        assert_eq!(preflight.len(), 1);
        assert_eq!(preflight[0].kind, Some(ModalityKind::Audio));
        assert_eq!(preflight[0].label.as_deref(), Some("voice.wav"));
    }

    #[test]
    fn preflight_warns_for_unsupported_audio_input() {
        let authority = MultimodalAuthority::from_config(&Config::default());
        let model = ProviderModel {
            id: "text-only".to_string(),
            provider_id: "openai".to_string(),
            api: ProviderModelApi {
                id: "text-only".to_string(),
                url: "https://example.invalid".to_string(),
                npm: "@ai-sdk/openai".to_string(),
            },
            name: "Text Only".to_string(),
            family: None,
            capabilities: ModelCapabilities {
                temperature: true,
                reasoning: false,
                attachment: true,
                toolcall: false,
                input: ModalitySet {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
                output: ModalitySet {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
                interleaved: InterleavedConfig::Bool(false),
            },
            cost: ProviderModelCost {
                input: 0.0,
                output: 0.0,
                cache: ModelCostCache {
                    read: 0.0,
                    write: 0.0,
                },
                experimental_over_200k: None,
            },
            limit: ProviderModelLimit {
                context: 128000,
                input: None,
                output: 4096,
            },
            status: "stable".to_string(),
            options: HashMap::new(),
            headers: HashMap::new(),
            release_date: "2026-01-01".to_string(),
            variants: None,
        };

        let capability = authority
            .capability_authority()
            .capability_view("openai", &model);
        let result = authority.capability_authority().preflight(
            &capability,
            &[PreflightInputPart {
                kind: Some(ModalityKind::Audio),
                mime: Some("audio/wav".to_string()),
                byte_len: Some(1024),
                label: Some("voice.wav".to_string()),
            }],
        );

        assert!(!result.hard_block);
        assert_eq!(result.unsupported_parts, vec!["voice.wav".to_string()]);
        assert!(result
            .recommended_downgrade
            .as_deref()
            .is_some_and(|value| value.contains("switch to a model")));
    }

    #[test]
    fn transport_explain_reports_replaced_audio_attachment() {
        let authority = MultimodalAuthority::from_config(&Config::default());
        let model = ProviderModel {
            id: "text-only".to_string(),
            provider_id: "openai".to_string(),
            api: ProviderModelApi {
                id: "text-only".to_string(),
                url: "https://example.invalid".to_string(),
                npm: "@ai-sdk/openai".to_string(),
            },
            name: "Text Only".to_string(),
            family: None,
            capabilities: ModelCapabilities {
                temperature: true,
                reasoning: false,
                attachment: true,
                toolcall: false,
                input: ModalitySet {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
                output: ModalitySet {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
                interleaved: InterleavedConfig::Bool(false),
            },
            cost: ProviderModelCost {
                input: 0.0,
                output: 0.0,
                cache: ModelCostCache {
                    read: 0.0,
                    write: 0.0,
                },
                experimental_over_200k: None,
            },
            limit: ProviderModelLimit {
                context: 128000,
                input: None,
                output: 4096,
            },
            status: "stable".to_string(),
            options: HashMap::new(),
            headers: HashMap::new(),
            release_date: "2026-01-01".to_string(),
            variants: None,
        };
        let capability = authority
            .capability_authority()
            .capability_view("openai", &model);
        let session_parts = vec![agendao_types::PromptPart::File {
            url: "data:audio/wav;base64,UklGRg==".to_string(),
            filename: Some("voice.wav".to_string()),
            mime: Some("audio/wav".to_string()),
        }];

        let transport =
            authority
                .capability_authority()
                .transport_explain(&capability, &model, &session_parts);

        assert_eq!(transport.replaced_parts, vec!["voice.wav".to_string()]);
        assert_eq!(transport.warnings.len(), 1);
        assert!(transport.warnings[0].contains("does not support audio input"));
    }
}
