use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterEvent {
    pub adapter_id: String,
    pub source: ExternalAdapterSource,
    pub external_event_id: String,
    pub external_user_id: String,
    pub external_conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    pub received_at_ms: i64,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ExternalAdapterAttachmentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_target: Option<ExternalAdapterReplyTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_event_ref: Option<ExternalAdapterRawEventRef>,
}

impl ExternalAdapterEvent {
    pub fn validate(&self) -> Result<(), ExternalAdapterValidationError> {
        require_non_blank("adapter_id", &self.adapter_id)?;
        require_non_blank("external_event_id", &self.external_event_id)?;
        require_non_blank("external_user_id", &self.external_user_id)?;
        require_non_blank("external_conversation_id", &self.external_conversation_id)?;
        if let Some(thread_id) = self.external_thread_id.as_deref() {
            require_non_blank("external_thread_id", thread_id)?;
        }
        if self.received_at_ms <= 0 {
            return Err(ExternalAdapterValidationError::InvalidTimestamp {
                field: "received_at_ms",
            });
        }
        if let Some(idempotency_key) = self.idempotency_key.as_deref() {
            require_non_blank("idempotency_key", idempotency_key)?;
        }
        for attachment in &self.attachments {
            attachment.validate()?;
        }
        if let Some(reply_target) = &self.reply_target {
            reply_target.validate()?;
        }
        if let Some(raw_event_ref) = &self.raw_event_ref {
            raw_event_ref.validate()?;
        }
        Ok(())
    }

    pub fn stable_idempotency_key(&self) -> Result<String, ExternalAdapterValidationError> {
        self.validate()?;
        if let Some(idempotency_key) = self.idempotency_key.as_deref() {
            return Ok(idempotency_key.trim().to_string());
        }

        Ok(format!(
            "external:{}:{}:{}",
            self.adapter_id.trim(),
            self.source.as_str(),
            self.external_event_id.trim()
        ))
    }

    pub fn ingress_source_label(&self) -> Result<String, ExternalAdapterValidationError> {
        self.validate()?;
        Ok(format!(
            "external:{}:{}",
            self.source.as_str(),
            self.adapter_id.trim()
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterResolvedBinding {
    pub session_id: String,
    pub actor_id: String,
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_id: Option<String>,
}

impl ExternalAdapterResolvedBinding {
    pub fn validate(&self) -> Result<(), ExternalAdapterValidationError> {
        require_non_blank("binding.session_id", &self.session_id)?;
        require_non_blank("binding.actor_id", &self.actor_id)?;
        require_non_blank("binding.workspace_id", &self.workspace_id)?;
        if let Some(route_policy_id) = self.route_policy_id.as_deref() {
            require_non_blank("binding.route_policy_id", route_policy_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalAdapterSource {
    #[serde(rename = "generic-webhook")]
    GenericWebhook,
    #[serde(rename = "cron")]
    Cron,
    #[serde(rename = "feishu-lark")]
    FeishuLark,
    #[serde(rename = "wechat")]
    WeChat,
    #[serde(rename = "custom")]
    Custom,
}

impl ExternalAdapterSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GenericWebhook => "generic-webhook",
            Self::Cron => "cron",
            Self::FeishuLark => "feishu-lark",
            Self::WeChat => "wechat",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterAttachmentRef {
    pub id: String,
    pub kind: String,
    pub uri: String,
}

impl ExternalAdapterAttachmentRef {
    pub fn validate(&self) -> Result<(), ExternalAdapterValidationError> {
        require_non_blank("attachment.id", &self.id)?;
        require_non_blank("attachment.kind", &self.kind)?;
        require_non_blank("attachment.uri", &self.uri)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterReplyTarget {
    pub target_type: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl ExternalAdapterReplyTarget {
    pub fn validate(&self) -> Result<(), ExternalAdapterValidationError> {
        require_non_blank("reply_target.target_type", &self.target_type)?;
        require_non_blank("reply_target.target_id", &self.target_id)?;
        if let Some(thread_id) = self.thread_id.as_deref() {
            require_non_blank("reply_target.thread_id", thread_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterRawEventRef {
    pub kind: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

impl ExternalAdapterRawEventRef {
    pub fn validate(&self) -> Result<(), ExternalAdapterValidationError> {
        require_non_blank("raw_event_ref.kind", &self.kind)?;
        require_non_blank("raw_event_ref.uri", &self.uri)?;
        if let Some(checksum) = self.checksum.as_deref() {
            require_non_blank("raw_event_ref.checksum", checksum)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterIngressRef {
    pub adapter_id: String,
    pub source: ExternalAdapterSource,
    pub external_event_id: String,
    pub external_user_id: String,
    pub external_conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_target: Option<ExternalAdapterReplyTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_event_ref: Option<ExternalAdapterRawEventRef>,
}

impl From<&ExternalAdapterEvent> for ExternalAdapterIngressRef {
    fn from(event: &ExternalAdapterEvent) -> Self {
        Self {
            adapter_id: event.adapter_id.trim().to_string(),
            source: event.source,
            external_event_id: event.external_event_id.trim().to_string(),
            external_user_id: event.external_user_id.trim().to_string(),
            external_conversation_id: event.external_conversation_id.trim().to_string(),
            external_thread_id: event
                .external_thread_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            reply_target: event.reply_target.clone(),
            raw_event_ref: event.raw_event_ref.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExternalAdapterValidationError {
    #[error("missing required external adapter field: {field}")]
    MissingRequired { field: &'static str },
    #[error("invalid external adapter timestamp: {field}")]
    InvalidTimestamp { field: &'static str },
}

fn require_non_blank(
    field: &'static str,
    value: &str,
) -> Result<(), ExternalAdapterValidationError> {
    if value.trim().is_empty() {
        Err(ExternalAdapterValidationError::MissingRequired { field })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event() -> ExternalAdapterEvent {
        ExternalAdapterEvent {
            adapter_id: "generic".to_string(),
            source: ExternalAdapterSource::GenericWebhook,
            external_event_id: "evt_1".to_string(),
            external_user_id: "user_1".to_string(),
            external_conversation_id: "chat_1".to_string(),
            external_thread_id: None,
            received_at_ms: 1_714_000_000_000,
            text: "hello".to_string(),
            attachments: Vec::new(),
            idempotency_key: None,
            reply_target: Some(ExternalAdapterReplyTarget {
                target_type: "chat".to_string(),
                target_id: "chat_1".to_string(),
                thread_id: None,
            }),
            raw_event_ref: Some(ExternalAdapterRawEventRef {
                kind: "object-ref".to_string(),
                uri: "rocode://external/generic/evt_1".to_string(),
                checksum: None,
            }),
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let payload = serde_json::json!({
            "adapter_id": "generic",
            "source": "generic-webhook",
            "external_event_id": "evt_1",
            "external_user_id": "user_1",
            "external_conversation_id": "chat_1",
            "received_at_ms": 1,
            "text": "hello",
            "unexpected": true
        });

        let result = serde_json::from_value::<ExternalAdapterEvent>(payload);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_source_values() {
        let payload = serde_json::json!({
            "adapter_id": "generic",
            "source": "surprise-chat-product",
            "external_event_id": "evt_1",
            "external_user_id": "user_1",
            "external_conversation_id": "chat_1",
            "received_at_ms": 1,
            "text": "hello"
        });

        let result = serde_json::from_value::<ExternalAdapterEvent>(payload);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_blank_required_identity_fields() {
        let mut event = sample_event();
        event.external_user_id = " ".to_string();

        assert_eq!(
            event.validate(),
            Err(ExternalAdapterValidationError::MissingRequired {
                field: "external_user_id"
            })
        );
    }

    #[test]
    fn derives_stable_idempotency_key_when_adapter_does_not_supply_one() {
        let event = sample_event();

        assert_eq!(
            event.stable_idempotency_key().unwrap(),
            "external:generic:generic-webhook:evt_1"
        );
    }

    #[test]
    fn preserves_adapter_supplied_idempotency_key_after_trimming() {
        let mut event = sample_event();
        event.idempotency_key = Some(" idem_1 ".to_string());

        assert_eq!(event.stable_idempotency_key().unwrap(), "idem_1");
    }

    #[test]
    fn attachment_refs_are_refs_not_inline_content() {
        let mut event = sample_event();
        event.attachments.push(ExternalAdapterAttachmentRef {
            id: "file_1".to_string(),
            kind: "image".to_string(),
            uri: "rocode://external/generic/file_1".to_string(),
        });

        event.validate().unwrap();
        assert_eq!(event.attachments[0].uri, "rocode://external/generic/file_1");
    }

    #[test]
    fn resolved_binding_requires_explicit_session_actor_and_workspace() {
        let binding = ExternalAdapterResolvedBinding {
            session_id: "ses_1".to_string(),
            actor_id: "".to_string(),
            workspace_id: "ws_1".to_string(),
            route_policy_id: None,
        };

        assert_eq!(
            binding.validate(),
            Err(ExternalAdapterValidationError::MissingRequired {
                field: "binding.actor_id"
            })
        );
    }
}
