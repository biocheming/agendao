use crate::{CapturedAssetKind, ResolvedMultimodalConfig};
use rocode_provider::{
    bootstrap::ProviderModel, mime_to_modality, transform_messages, Content, ContentPart, ImageUrl,
    Message, Modality, ProviderType, Role,
};
use rocode_session::prompt::PartInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModalityKind {
    Text,
    Audio,
    Image,
    Video,
    Pdf,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModalitySupportView {
    pub text: bool,
    pub audio: bool,
    pub image: bool,
    pub video: bool,
    pub pdf: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightCapabilityView {
    pub provider_id: String,
    pub model_id: String,
    pub attachment: bool,
    pub tool_call: bool,
    pub reasoning: bool,
    pub temperature: bool,
    pub input: ModalitySupportView,
    pub output: ModalitySupportView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightInputPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ModalityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModalityPreflightResult {
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub unsupported_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_downgrade: Option<String>,
    #[serde(default)]
    pub hard_block: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModalityTransportResult {
    #[serde(default)]
    pub replaced_parts: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalPolicyResponse {
    pub policy: crate::MultimodalConfigView,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MultimodalCapabilitiesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<PreflightCapabilityView>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MultimodalPreflightRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub parts: Vec<PreflightInputPart>,
    #[serde(default)]
    pub session_parts: Vec<PartInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalPreflightResponse {
    pub policy: crate::MultimodalConfigView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<PreflightCapabilityView>,
    pub result: ModalityPreflightResult,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub struct MultimodalCapabilityAuthority<'a> {
    policy: &'a ResolvedMultimodalConfig,
}

impl<'a> MultimodalCapabilityAuthority<'a> {
    pub fn new(policy: &'a ResolvedMultimodalConfig) -> Self {
        Self { policy }
    }

    pub fn capability_view(
        &self,
        provider_id: impl Into<String>,
        model: &ProviderModel,
    ) -> PreflightCapabilityView {
        PreflightCapabilityView {
            provider_id: provider_id.into(),
            model_id: model.id.clone(),
            attachment: model.capabilities.attachment,
            tool_call: model.capabilities.toolcall,
            reasoning: model.capabilities.reasoning,
            temperature: model.capabilities.temperature,
            input: ModalitySupportView {
                text: model.capabilities.input.text,
                audio: model.capabilities.input.audio,
                image: model.capabilities.input.image,
                video: model.capabilities.input.video,
                pdf: model.capabilities.input.pdf,
            },
            output: ModalitySupportView {
                text: model.capabilities.output.text,
                audio: model.capabilities.output.audio,
                image: model.capabilities.output.image,
                video: model.capabilities.output.video,
                pdf: model.capabilities.output.pdf,
            },
        }
    }

    pub fn preflight(
        &self,
        capability: &PreflightCapabilityView,
        parts: &[PreflightInputPart],
    ) -> ModalityPreflightResult {
        let mut result = ModalityPreflightResult::default();

        for (index, part) in parts.iter().enumerate() {
            let label = part
                .label
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("part {}", index + 1));

            if let Some(byte_len) = part.byte_len {
                if let Some(limit) = self.policy.max_input_bytes {
                    if byte_len > limit {
                        result.hard_block = true;
                        result.warnings.push(format!(
                            "{} exceeds multimodal maxInputBytes ({} > {}).",
                            label, byte_len, limit
                        ));
                    }
                }
            }

            let Some(kind) = resolve_part_kind(part) else {
                result.warnings.push(format!(
                    "{} could not be classified from kind/mime; transport will decide.",
                    label
                ));
                continue;
            };

            match kind {
                ModalityKind::Audio => {
                    if !self.policy.allow_audio_input {
                        result.hard_block = true;
                        result
                            .warnings
                            .push(format!("{} is blocked by multimodal policy.", label));
                    }
                    if !capability.input.audio {
                        result.unsupported_parts.push(label.clone());
                    }
                }
                ModalityKind::Image => {
                    if !self.policy.allow_image_input {
                        result.hard_block = true;
                        result
                            .warnings
                            .push(format!("{} is blocked by multimodal policy.", label));
                    }
                    if !capability.input.image {
                        result.unsupported_parts.push(label.clone());
                    }
                }
                ModalityKind::File => {
                    if !self.policy.allow_file_input {
                        result.hard_block = true;
                        result
                            .warnings
                            .push(format!("{} is blocked by multimodal policy.", label));
                    }
                    if !capability.attachment {
                        result.unsupported_parts.push(label.clone());
                    }
                }
                ModalityKind::Video => {
                    if !capability.input.video {
                        result.unsupported_parts.push(label.clone());
                    }
                }
                ModalityKind::Pdf => {
                    if !capability.input.pdf {
                        result.unsupported_parts.push(label.clone());
                    }
                }
                ModalityKind::Text => {}
            }

            if part.mime.is_some() && kind == ModalityKind::File && !capability.attachment {
                result.warnings.push(format!(
                    "{} is an attachment but the selected model does not advertise attachment support.",
                    label
                ));
            }
        }

        if !result.unsupported_parts.is_empty() {
            result.warnings.push(format!(
                "Provider capability metadata says the selected model does not support: {}.",
                result.unsupported_parts.join(", ")
            ));
            result.recommended_downgrade = Some(
                "Remove unsupported attachments or switch to a model that supports those modalities."
                    .to_string(),
            );
        }

        result
    }

    pub fn transport_explain(
        &self,
        capability: &PreflightCapabilityView,
        model: &ProviderModel,
        parts: &[PartInput],
    ) -> ModalityTransportResult {
        let original_parts = build_transport_message_parts(parts);
        if original_parts.is_empty() {
            return ModalityTransportResult::default();
        }

        let mut messages = vec![Message {
            role: Role::User,
            content: Content::Parts(original_parts.clone()),
            cache_control: None,
            provider_options: None,
        }];
        transform_messages(
            &mut messages,
            ProviderType::from_provider_id(&capability.provider_id),
            &model.id,
            &supported_modalities(capability),
            &model.api.npm,
            &capability.provider_id,
        );

        let Some(Message {
            content: Content::Parts(transformed_parts),
            ..
        }) = messages.into_iter().next()
        else {
            return ModalityTransportResult::default();
        };

        let mut result = ModalityTransportResult::default();
        for (original, transformed) in original_parts.iter().zip(transformed_parts.iter()) {
            if !matches!(original.content_type.as_str(), "file" | "image") {
                continue;
            }
            let Some(text) = transformed.text.as_deref() else {
                continue;
            };
            if transformed.content_type != "text" || !text.starts_with("ERROR:") {
                continue;
            }
            let label = content_part_label(original);
            result.replaced_parts.push(label);
            result.warnings.push(text.to_string());
        }

        result
    }
}

fn build_transport_message_parts(parts: &[PartInput]) -> Vec<ContentPart> {
    parts
        .iter()
        .filter_map(|part| match part {
            PartInput::Text { text } => Some(ContentPart {
                content_type: "text".to_string(),
                text: Some(text.clone()),
                ..Default::default()
            }),
            PartInput::File {
                url,
                filename,
                mime,
            } => Some(ContentPart {
                content_type: "file".to_string(),
                image_url: Some(ImageUrl { url: url.clone() }),
                media_type: mime.clone(),
                filename: filename.clone(),
                ..Default::default()
            }),
            PartInput::Agent { .. } | PartInput::Subtask { .. } => None,
        })
        .collect()
}

fn supported_modalities(capability: &PreflightCapabilityView) -> Vec<Modality> {
    let mut modalities = Vec::new();
    if capability.input.audio {
        modalities.push(Modality::Audio);
    }
    if capability.input.image {
        modalities.push(Modality::Image);
    }
    if capability.input.video {
        modalities.push(Modality::Video);
    }
    if capability.input.pdf {
        modalities.push(Modality::Pdf);
    }
    modalities
}

fn content_part_label(part: &ContentPart) -> String {
    part.filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            part.media_type
                .as_deref()
                .and_then(mime_to_modality)
                .map(|modality| modality.to_string())
        })
        .unwrap_or_else(|| "attachment".to_string())
}

fn resolve_part_kind(part: &PreflightInputPart) -> Option<ModalityKind> {
    if let Some(kind) = part.kind {
        return Some(kind);
    }

    let mime = part.mime.as_deref()?;
    match mime_to_modality(mime) {
        Some(Modality::Audio) => Some(ModalityKind::Audio),
        Some(Modality::Image) => Some(ModalityKind::Image),
        Some(Modality::Video) => Some(ModalityKind::Video),
        Some(Modality::Pdf) => Some(ModalityKind::Pdf),
        None => Some(if mime.starts_with("text/") {
            ModalityKind::Text
        } else if mime == "application/octet-stream" {
            ModalityKind::File
        } else {
            map_file_kind_from_mime(mime)
        }),
    }
}

fn map_file_kind_from_mime(mime: &str) -> ModalityKind {
    if mime.starts_with("audio/") {
        ModalityKind::Audio
    } else if mime.starts_with("image/") {
        ModalityKind::Image
    } else if mime.starts_with("video/") {
        ModalityKind::Video
    } else if mime == "application/pdf" {
        ModalityKind::Pdf
    } else {
        ModalityKind::File
    }
}

impl From<CapturedAssetKind> for ModalityKind {
    fn from(value: CapturedAssetKind) -> Self {
        match value {
            CapturedAssetKind::Audio => Self::Audio,
            CapturedAssetKind::Image => Self::Image,
            CapturedAssetKind::File => Self::File,
            CapturedAssetKind::Video => Self::Video,
            CapturedAssetKind::Pdf => Self::Pdf,
        }
    }
}
