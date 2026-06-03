use std::borrow::Cow;
use std::collections::HashMap;

use crate::{
    ModalityPreflightResult, ModalityTransportResult, MultimodalDisplaySummary,
    PreflightCapabilityView,
};
use agendao_session::{MessageRole, PartType, SessionMessage};
use serde::{Deserialize, Serialize};

const RESOLVED_MODEL_KEY: &str = "multimodal_resolved_model";
const ATTACHMENT_COUNT_KEY: &str = "multimodal_attachment_count";
const BADGES_KEY: &str = "multimodal_badges";
const KINDS_KEY: &str = "multimodal_kinds";
const COMPACT_LABEL_KEY: &str = "multimodal_compact_label";
const PREFLIGHT_KEY: &str = "multimodal_preflight";
const TRANSPORT_KEY: &str = "multimodal_transport";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMultimodalExplain {
    pub summary: MultimodalDisplaySummary,
    pub capability: PreflightCapabilityView,
    pub result: ModalityPreflightResult,
    pub transport: ModalityTransportResult,
    pub resolved_model: String,
}

impl RuntimeMultimodalExplain {
    pub fn persist_into_message_metadata(&self, message: &mut SessionMessage) {
        message.metadata.insert(
            RESOLVED_MODEL_KEY.to_string(),
            serde_json::json!(self.resolved_model),
        );
        message.metadata.insert(
            ATTACHMENT_COUNT_KEY.to_string(),
            serde_json::json!(self.summary.attachment_count),
        );
        message.metadata.insert(
            BADGES_KEY.to_string(),
            serde_json::json!(self.summary.badges.clone()),
        );
        message.metadata.insert(
            KINDS_KEY.to_string(),
            serde_json::json!(self.summary.kinds.clone()),
        );
        if !self.summary.compact_label.trim().is_empty() {
            message.metadata.insert(
                COMPACT_LABEL_KEY.to_string(),
                serde_json::json!(self.summary.compact_label.clone()),
            );
        }
        message.metadata.insert(
            PREFLIGHT_KEY.to_string(),
            serde_json::json!({
                "warnings": self.result.warnings.clone(),
                "unsupported_parts": self.result.unsupported_parts.clone(),
                "recommended_downgrade": self.result.recommended_downgrade.clone(),
                "hard_block": self.result.hard_block,
                "capability": self.capability.clone(),
            }),
        );
        message.metadata.insert(
            TRANSPORT_KEY.to_string(),
            serde_json::json!({
                "replaced_parts": self.transport.replaced_parts.clone(),
                "warnings": self.transport.warnings.clone(),
            }),
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedMultimodalAttachmentInfo {
    pub filename: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedMultimodalExplain {
    pub attachment_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_downgrade: Option<String>,
    #[serde(default)]
    pub hard_block: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_replaced_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<PersistedMultimodalAttachmentInfo>,
}

impl PersistedMultimodalExplain {
    pub fn display_label(&self) -> Cow<'_, str> {
        self.compact_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| {
                Cow::Owned(default_compact_label(
                    self.attachment_count.max(self.attachments.len() as u32) as usize,
                ))
            })
    }

    pub fn summary_line(&self) -> String {
        let mut fields = vec![self.display_label().into_owned()];
        if self.attachment_count > 0 {
            fields.push(format!("attachments {}", self.attachment_count));
        }
        let kinds = if self.kinds.is_empty() {
            &self.badges
        } else {
            &self.kinds
        };
        if !kinds.is_empty() {
            fields.push(kinds.join(", "));
        }
        if self.hard_block {
            fields.push("hard block".to_string());
        }
        fields.join(" · ")
    }

    pub fn combined_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for warning in self
            .warnings
            .iter()
            .chain(self.transport_warnings.iter())
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !warnings.iter().any(|existing| existing == warning) {
                warnings.push(warning.to_string());
            }
        }
        warnings
    }

    pub fn has_message_signal(message: &SessionMessage) -> bool {
        message
            .parts
            .iter()
            .any(|part| matches!(part.part_type, PartType::File { .. }))
            || message.metadata.contains_key(PREFLIGHT_KEY)
    }

    pub fn from_message(message: &SessionMessage) -> Option<Self> {
        if !matches!(message.role, MessageRole::User) || !Self::has_message_signal(message) {
            return None;
        }

        let attachments = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::File { filename, mime, .. } => Some(PersistedMultimodalAttachmentInfo {
                    filename: filename.clone(),
                    mime: mime.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        let preflight = message
            .metadata
            .get(PREFLIGHT_KEY)
            .and_then(|value| value.as_object());
        let transport = message
            .metadata
            .get(TRANSPORT_KEY)
            .and_then(|value| value.as_object());

        let mut kinds = metadata_string_list(&message.metadata, KINDS_KEY);
        if kinds.is_empty() {
            kinds = infer_kinds_from_attachments(&attachments);
        }

        let mut compact_label = message
            .metadata
            .get(COMPACT_LABEL_KEY)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if compact_label.is_none() && !attachments.is_empty() {
            compact_label = Some(default_compact_label(attachments.len()));
        }

        Some(Self {
            attachment_count: message
                .metadata
                .get(ATTACHMENT_COUNT_KEY)
                .and_then(|value| value.as_u64())
                .map(|value| value as u32)
                .unwrap_or(attachments.len() as u32),
            kinds,
            badges: metadata_string_list(&message.metadata, BADGES_KEY),
            compact_label,
            resolved_model: message
                .metadata
                .get(RESOLVED_MODEL_KEY)
                .and_then(|value| value.as_str())
                .map(str::to_string),
            warnings: preflight
                .and_then(|value| value.get("warnings"))
                .map(json_string_list)
                .unwrap_or_default(),
            unsupported_parts: preflight
                .and_then(|value| value.get("unsupported_parts"))
                .map(json_string_list)
                .unwrap_or_default(),
            recommended_downgrade: preflight
                .and_then(|value| value.get("recommended_downgrade"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
            hard_block: preflight
                .and_then(|value| value.get("hard_block"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            transport_replaced_parts: transport
                .and_then(|value| value.get("replaced_parts"))
                .map(json_string_list)
                .unwrap_or_default(),
            transport_warnings: transport
                .and_then(|value| value.get("warnings"))
                .map(json_string_list)
                .unwrap_or_default(),
            attachments,
        })
    }
}

fn metadata_string_list(metadata: &HashMap<String, serde_json::Value>, key: &str) -> Vec<String> {
    metadata.get(key).map(json_string_list).unwrap_or_default()
}

fn json_string_list(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn infer_kinds_from_attachments(attachments: &[PersistedMultimodalAttachmentInfo]) -> Vec<String> {
    let mut kinds = Vec::new();
    for attachment in attachments {
        let kind = if attachment.mime.starts_with("audio/") {
            "audio"
        } else if attachment.mime.starts_with("image/") {
            "image"
        } else if attachment.mime.starts_with("video/") {
            "video"
        } else if attachment.mime == "application/pdf" {
            "pdf"
        } else if attachment.mime.starts_with("text/") {
            "text"
        } else {
            "file"
        };
        if !kinds.iter().any(|existing| existing == kind) {
            kinds.push(kind.to_string());
        }
    }
    kinds
}

fn default_compact_label(attachment_count: usize) -> String {
    if attachment_count == 1 {
        "[1 attachment]".to_string()
    } else {
        format!("[{} attachments]", attachment_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModalitySupportView, MultimodalDisplaySummary};

    #[test]
    fn persisted_explain_reads_message_metadata_and_attachment_fallbacks() {
        let mut message = SessionMessage::user("session-1", "");
        message.parts.clear();
        message.parts.push(agendao_session::MessagePart {
            id: "part-1".to_string(),
            part_type: PartType::File {
                url: "data:audio/wav;base64,UklGRg==".to_string(),
                filename: "voice.wav".to_string(),
                mime: "audio/wav".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: Some(message.id.clone()),
        });
        message.metadata.insert(
            PREFLIGHT_KEY.to_string(),
            serde_json::json!({
                "warnings": ["audio not supported"],
                "unsupported_parts": ["voice.wav"],
                "recommended_downgrade": "switch model",
                "hard_block": false
            }),
        );
        message.metadata.insert(
            TRANSPORT_KEY.to_string(),
            serde_json::json!({
                "replaced_parts": ["voice.wav"],
                "warnings": ["ERROR: Cannot read voice.wav"]
            }),
        );

        let explain = PersistedMultimodalExplain::from_message(&message)
            .expect("persisted explain should exist");
        assert_eq!(explain.attachment_count, 1);
        assert_eq!(explain.kinds, vec!["audio".to_string()]);
        assert_eq!(explain.compact_label.as_deref(), Some("[1 attachment]"));
        assert_eq!(
            explain.transport_replaced_parts,
            vec!["voice.wav".to_string()]
        );
        assert_eq!(explain.transport_warnings.len(), 1);
    }

    #[test]
    fn persisted_explain_helpers_provide_summary_and_deduped_warnings() {
        let explain = PersistedMultimodalExplain {
            attachment_count: 2,
            kinds: vec!["audio".to_string(), "image".to_string()],
            badges: vec!["audio".to_string()],
            compact_label: None,
            resolved_model: None,
            warnings: vec![
                "audio transcription recommended".to_string(),
                "audio transcription recommended".to_string(),
            ],
            unsupported_parts: Vec::new(),
            recommended_downgrade: None,
            hard_block: true,
            transport_replaced_parts: Vec::new(),
            transport_warnings: vec![
                "provider downgraded audio".to_string(),
                "audio transcription recommended".to_string(),
            ],
            attachments: Vec::new(),
        };

        assert_eq!(
            explain.summary_line(),
            "[2 attachments] · attachments 2 · audio, image · hard block"
        );
        assert_eq!(
            explain.combined_warnings(),
            vec![
                "audio transcription recommended".to_string(),
                "provider downgraded audio".to_string(),
            ]
        );
    }

    #[test]
    fn runtime_explain_persists_metadata_contract() {
        let mut message = SessionMessage::user("session-1", "");
        let explain = RuntimeMultimodalExplain {
            summary: MultimodalDisplaySummary {
                primary_text: String::new(),
                attachment_count: 1,
                badges: vec!["audio".to_string()],
                compact_label: "[audio input]".to_string(),
                kinds: vec!["audio".to_string()],
            },
            capability: PreflightCapabilityView {
                provider_id: "openai".to_string(),
                model_id: "gpt-audio".to_string(),
                attachment: true,
                tool_call: true,
                reasoning: false,
                temperature: true,
                input: ModalitySupportView {
                    text: true,
                    audio: true,
                    image: false,
                    video: false,
                    pdf: false,
                },
                output: ModalitySupportView {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
            },
            result: ModalityPreflightResult::default(),
            transport: ModalityTransportResult::default(),
            resolved_model: "openai/gpt-audio".to_string(),
        };

        explain.persist_into_message_metadata(&mut message);
        assert_eq!(
            message
                .metadata
                .get(RESOLVED_MODEL_KEY)
                .and_then(|value| value.as_str()),
            Some("openai/gpt-audio")
        );
        assert!(message.metadata.contains_key(PREFLIGHT_KEY));
        assert!(message.metadata.contains_key(TRANSPORT_KEY));
    }
}
