use std::collections::HashSet;

use rocode_types::{
    ExternalAdapterEvent, ExternalAdapterIngressRef, ExternalAdapterValidationError,
};
use serde::{Deserialize, Serialize};

const DEFAULT_WEB_BATCH_WINDOW_MS: i64 = 250;
pub const INGRESS_POLICY_UNSPECIFIED: &str = "none";
pub const INGRESS_POLICY_ENTRY_METADATA_ONLY: &str = "entry_metadata_only";
pub const INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY: &str = "external_adapter_metadata_only";
pub const INGRESS_POLICY_SCHEDULER_METADATA_ONLY: &str = "scheduler_metadata_only";
pub const INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH: &str = "same_session_context_batch";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressSource {
    Cli,
    Tui,
    Web,
    Api,
    Scheduler,
    Other(String),
}

/// Canonical normalization for ingress source labels supplied by clients/routes.
/// Missing or blank values default to `Api`: ingress source represents caller
/// identity, not the HTTP transport shape used to deliver the turn.
pub fn normalize_ingress_source(value: Option<&str>) -> IngressSource {
    match value.unwrap_or("api").trim().to_ascii_lowercase().as_str() {
        "cli" => IngressSource::Cli,
        "tui" => IngressSource::Tui,
        "web" => IngressSource::Web,
        "api" => IngressSource::Api,
        "scheduler" => IngressSource::Scheduler,
        other if !other.is_empty() => IngressSource::Other(other.to_string()),
        _ => IngressSource::Api,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressAttachmentRef {
    pub id: String,
    pub kind: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressStabilizationMetadata {
    /// Live field: exposed as diagnostics/telemetry and incremented by the
    /// constrained web batching path when multiple turns are merged.
    pub batch_count: usize,
    /// Helper/reserved field: populated by stabilization helpers when duplicate
    /// ingress items are collapsed, but not currently consumed by the prompt
    /// authority pipeline.
    pub dedupe_keys: Vec<String>,
    /// Helper/reserved field: preserved for ordering-aware stabilization work.
    pub ordering_key: String,
    /// Live field: records the ingress handling policy that produced the
    /// stabilized turn and may participate in prompt-surface diagnostics.
    pub policy: String,
}

impl IngressStabilizationMetadata {
    pub fn single(ordering_key: impl Into<String>, policy: impl Into<String>) -> Self {
        Self {
            batch_count: 1,
            dedupe_keys: Vec::new(),
            ordering_key: ordering_key.into(),
            policy: policy.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressTurnEnvelope {
    pub session_id: String,
    pub source: IngressSource,
    pub turn_id: String,
    /// Helper/runtime timestamp for stabilization logic; not part of prompt
    /// authority.
    pub received_at_ms: i64,
    /// Helper/runtime timestamp for stabilization logic; not part of prompt
    /// authority.
    pub stabilized_at_ms: i64,
    /// Non-authoritative shadow text. `PromptInput.parts` remains the sole
    /// model-visible user input authority.
    pub user_intent_text: String,
    /// Reserved field for future ingress-native attachment descriptors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<IngressAttachmentRef>,
    /// Reserved field for future quoted-context references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quoted_context_refs: Vec<String>,
    /// Helper/control-turn marker used by stabilization logic to avoid merging
    /// command/reply turns into ordinary user prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Live runtime metadata field. This may steer ingress-local handling but
    /// must not become prompt authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_key: Option<String>,
    /// Live runtime metadata field for scheduler-originated turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_stage_id: Option<String>,
    /// Live runtime metadata field for scoped dedupe across repeated ingress
    /// submissions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Typed audit reference for externally sourced turns. This metadata is
    /// preserved for delivery/audit consumers and must not become prompt
    /// authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_adapter: Option<ExternalAdapterIngressRef>,
    pub stabilization: IngressStabilizationMetadata,
}

impl IngressTurnEnvelope {
    pub fn new_text(
        session_id: impl Into<String>,
        source: IngressSource,
        turn_id: impl Into<String>,
        received_at_ms: i64,
        text: impl Into<String>,
    ) -> Self {
        let turn_id = turn_id.into();
        Self {
            session_id: session_id.into(),
            source,
            turn_id: turn_id.clone(),
            received_at_ms,
            stabilized_at_ms: received_at_ms,
            user_intent_text: text.into(),
            attachments: Vec::new(),
            quoted_context_refs: Vec::new(),
            command: None,
            context_key: None,
            scheduler_stage_id: None,
            idempotency_key: None,
            external_adapter: None,
            stabilization: IngressStabilizationMetadata::single(
                turn_id,
                INGRESS_POLICY_UNSPECIFIED,
            ),
        }
    }
}

pub fn external_adapter_event_to_ingress_turn(
    session_id: impl Into<String>,
    event: &ExternalAdapterEvent,
) -> Result<IngressTurnEnvelope, ExternalAdapterIngressMappingError> {
    let session_id = session_id.into();
    if session_id.trim().is_empty() {
        return Err(ExternalAdapterIngressMappingError::MissingSessionBinding);
    }

    event
        .validate()
        .map_err(ExternalAdapterIngressMappingError::InvalidEvent)?;

    let turn_id = format!(
        "external:{}:{}:{}",
        event.source.as_str(),
        event.adapter_id.trim(),
        event.external_event_id.trim()
    );
    let mut turn = IngressTurnEnvelope::new_text(
        session_id.trim().to_string(),
        IngressSource::Other(
            event
                .ingress_source_label()
                .map_err(ExternalAdapterIngressMappingError::InvalidEvent)?,
        ),
        turn_id,
        event.received_at_ms,
        event.text.clone(),
    );

    turn.attachments = event
        .attachments
        .iter()
        .map(|attachment| IngressAttachmentRef {
            id: attachment.id.trim().to_string(),
            kind: attachment.kind.trim().to_string(),
            uri: attachment.uri.trim().to_string(),
        })
        .collect();
    turn.idempotency_key = Some(
        event
            .stable_idempotency_key()
            .map_err(ExternalAdapterIngressMappingError::InvalidEvent)?,
    );
    turn.external_adapter = Some(ExternalAdapterIngressRef::from(event));
    turn.stabilization.policy = INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY.to_string();

    Ok(turn)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExternalAdapterIngressMappingError {
    #[error("external adapter event has no resolved ROCode session binding")]
    MissingSessionBinding,
    #[error(transparent)]
    InvalidEvent(ExternalAdapterValidationError),
}

/// Shared stabilization helper for ingress merge/dedupe semantics.
/// Live prompt execution currently uses this helper only for the constrained
/// `session_prompt` Web burst-batching path; outside that narrow route it
/// remains helper/test infrastructure rather than a global ingress pipeline.
pub fn stabilize_ingress_turns(mut turns: Vec<IngressTurnEnvelope>) -> Vec<IngressTurnEnvelope> {
    turns.sort_by(|a, b| {
        a.received_at_ms
            .cmp(&b.received_at_ms)
            .then_with(|| a.turn_id.cmp(&b.turn_id))
    });

    let mut seen_idempotency_keys = HashSet::new();
    let mut stabilized: Vec<IngressTurnEnvelope> = Vec::new();

    for turn in turns {
        if let Some(key) = turn.idempotency_key.as_deref() {
            let scoped_key = format!("{}:{:?}:{}", turn.session_id, turn.source, key);
            if !seen_idempotency_keys.insert(scoped_key) {
                if let Some(last) = stabilized.last_mut() {
                    last.stabilization.dedupe_keys.push(key.to_string());
                }
                continue;
            }
        }

        if let Some(last) = stabilized.last_mut() {
            if can_merge_ingress_turns(last, &turn) {
                merge_ingress_turn(last, turn);
                continue;
            }
        }

        stabilized.push(turn);
    }

    stabilized
}

fn can_merge_ingress_turns(left: &IngressTurnEnvelope, right: &IngressTurnEnvelope) -> bool {
    left.session_id == right.session_id
        && left.source == right.source
        && left.context_key == right.context_key
        && left.scheduler_stage_id == right.scheduler_stage_id
        && matches!(left.source, IngressSource::Web)
        && right.received_at_ms.saturating_sub(left.stabilized_at_ms) <= DEFAULT_WEB_BATCH_WINDOW_MS
        && !is_control_or_reply_turn(left)
        && !is_control_or_reply_turn(right)
}

fn is_control_or_reply_turn(turn: &IngressTurnEnvelope) -> bool {
    let Some(command) = turn.command.as_deref() else {
        return false;
    };
    matches!(
        command.trim().trim_start_matches('/'),
        "stop" | "new" | "reset" | "permission_reply" | "question_reply"
    )
}

fn merge_ingress_turn(target: &mut IngressTurnEnvelope, next: IngressTurnEnvelope) {
    // This only merges ingress shadow text and ingress-local metadata. Live
    // callers must separately rebuild authoritative `PromptInput.parts`.
    if !target.user_intent_text.is_empty() && !next.user_intent_text.is_empty() {
        target.user_intent_text.push('\n');
    }
    target.user_intent_text.push_str(&next.user_intent_text);
    target.attachments.extend(next.attachments);
    for quoted_ref in next.quoted_context_refs {
        if !target.quoted_context_refs.contains(&quoted_ref) {
            target.quoted_context_refs.push(quoted_ref);
        }
    }
    target.stabilized_at_ms = target.stabilized_at_ms.max(next.received_at_ms);
    target.stabilization.batch_count += next.stabilization.batch_count.max(1);
    target.stabilization.policy = INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH.to_string();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper-only: these tests cover ingress.rs stabilization semantics, not
    // the live session prompt execution pipeline.

    #[test]
    fn normalize_ingress_source_defaults_to_api_and_preserves_known_sources() {
        assert_eq!(normalize_ingress_source(None), IngressSource::Api);
        assert_eq!(normalize_ingress_source(Some("")), IngressSource::Api);
        assert_eq!(normalize_ingress_source(Some("cli")), IngressSource::Cli);
        assert_eq!(normalize_ingress_source(Some("TUI")), IngressSource::Tui);
        assert_eq!(normalize_ingress_source(Some("web")), IngressSource::Web);
        assert_eq!(
            normalize_ingress_source(Some("scheduler")),
            IngressSource::Scheduler
        );
        assert_eq!(
            normalize_ingress_source(Some("feishu")),
            IngressSource::Other("feishu".to_string())
        );
    }

    #[test]
    fn stabilizes_same_session_text_burst_into_one_turn() {
        let first =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Web, "turn_1", 100, "first");
        let second =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Web, "turn_2", 101, "second");

        let stabilized = stabilize_ingress_turns(vec![second, first]);

        assert_eq!(stabilized.len(), 1);
        assert_eq!(stabilized[0].user_intent_text, "first\nsecond");
        assert_eq!(stabilized[0].stabilization.batch_count, 2);
    }

    #[test]
    fn does_not_merge_control_commands() {
        let first =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Web, "turn_1", 100, "work");
        let mut stop =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Web, "turn_2", 101, "/stop");
        stop.command = Some("stop".to_string());

        let stabilized = stabilize_ingress_turns(vec![first, stop]);

        assert_eq!(stabilized.len(), 2);
    }

    #[test]
    fn api_turns_do_not_batch_without_idempotency_dedupe() {
        let first =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Api, "turn_1", 100, "first");
        let second =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Api, "turn_2", 101, "second");

        let stabilized = stabilize_ingress_turns(vec![first, second]);

        assert_eq!(stabilized.len(), 2);
    }

    #[test]
    fn web_turns_do_not_batch_after_default_window() {
        let first =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Web, "turn_1", 100, "first");
        let second =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Web, "turn_2", 500, "second");

        let stabilized = stabilize_ingress_turns(vec![first, second]);

        assert_eq!(stabilized.len(), 2);
    }

    #[test]
    fn dedupes_idempotent_resubmits() {
        let mut first =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Api, "turn_1", 100, "same");
        first.idempotency_key = Some("idem_1".to_string());
        let mut duplicate =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Api, "turn_2", 101, "same");
        duplicate.idempotency_key = Some("idem_1".to_string());

        let stabilized = stabilize_ingress_turns(vec![first, duplicate]);

        assert_eq!(stabilized.len(), 1);
        assert_eq!(stabilized[0].user_intent_text, "same");
        assert_eq!(stabilized[0].stabilization.dedupe_keys, vec!["idem_1"]);
    }

    #[test]
    fn idempotency_dedupe_is_scoped_to_session_and_source() {
        let mut first =
            IngressTurnEnvelope::new_text("ses_a", IngressSource::Api, "turn_1", 100, "same");
        first.idempotency_key = Some("idem_1".to_string());
        let mut second =
            IngressTurnEnvelope::new_text("ses_b", IngressSource::Api, "turn_2", 101, "same");
        second.idempotency_key = Some("idem_1".to_string());

        let stabilized = stabilize_ingress_turns(vec![first, second]);

        assert_eq!(stabilized.len(), 2);
    }

    #[test]
    fn scheduler_turns_do_not_batch_across_stage_boundary() {
        let mut first = IngressTurnEnvelope::new_text(
            "ses_a",
            IngressSource::Scheduler,
            "turn_1",
            100,
            "stage a",
        );
        first.scheduler_stage_id = Some("stage_a".to_string());
        let mut second = IngressTurnEnvelope::new_text(
            "ses_a",
            IngressSource::Scheduler,
            "turn_2",
            101,
            "stage b",
        );
        second.scheduler_stage_id = Some("stage_b".to_string());

        let stabilized = stabilize_ingress_turns(vec![first, second]);

        assert_eq!(stabilized.len(), 2);
    }

    fn sample_external_event() -> ExternalAdapterEvent {
        ExternalAdapterEvent {
            adapter_id: "generic".to_string(),
            source: rocode_types::ExternalAdapterSource::GenericWebhook,
            external_event_id: "evt_1".to_string(),
            external_user_id: "user_1".to_string(),
            external_conversation_id: "chat_1".to_string(),
            external_thread_id: Some("thread_1".to_string()),
            received_at_ms: 1_714_000_000_000,
            text: "show status".to_string(),
            attachments: vec![rocode_types::ExternalAdapterAttachmentRef {
                id: "file_1".to_string(),
                kind: "image".to_string(),
                uri: "rocode://external/generic/file_1".to_string(),
            }],
            idempotency_key: None,
            reply_target: Some(rocode_types::ExternalAdapterReplyTarget {
                target_type: "chat".to_string(),
                target_id: "chat_1".to_string(),
                thread_id: Some("thread_1".to_string()),
            }),
            raw_event_ref: Some(rocode_types::ExternalAdapterRawEventRef {
                kind: "object-ref".to_string(),
                uri: "rocode://external/generic/evt_1".to_string(),
                checksum: None,
            }),
        }
    }

    #[test]
    fn maps_external_adapter_event_after_session_binding() {
        let event = sample_external_event();

        let turn = external_adapter_event_to_ingress_turn("ses_external", &event).unwrap();

        assert_eq!(turn.session_id, "ses_external");
        assert_eq!(
            turn.source,
            IngressSource::Other("external:generic-webhook:generic".to_string())
        );
        assert_eq!(turn.turn_id, "external:generic-webhook:generic:evt_1");
        assert_eq!(turn.user_intent_text, "show status");
        assert_eq!(
            turn.idempotency_key,
            Some("external:generic:generic-webhook:evt_1".to_string())
        );
        assert_eq!(
            turn.stabilization.policy,
            INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY
        );
    }

    #[test]
    fn external_adapter_mapping_requires_resolved_session_binding() {
        let event = sample_external_event();

        assert_eq!(
            external_adapter_event_to_ingress_turn(" ", &event),
            Err(ExternalAdapterIngressMappingError::MissingSessionBinding)
        );
    }

    #[test]
    fn external_adapter_metadata_does_not_enter_shadow_prompt_text() {
        let event = sample_external_event();

        let turn = external_adapter_event_to_ingress_turn("ses_external", &event).unwrap();

        assert_eq!(turn.user_intent_text, "show status");
        assert!(!turn.user_intent_text.contains("user_1"));
        assert!(!turn.user_intent_text.contains("chat_1"));
        let external = turn.external_adapter.unwrap();
        assert_eq!(external.external_user_id, "user_1");
        assert_eq!(external.external_conversation_id, "chat_1");
        assert_eq!(
            external.raw_event_ref.unwrap().uri,
            "rocode://external/generic/evt_1"
        );
    }

    #[test]
    fn external_adapter_attachments_remain_refs() {
        let event = sample_external_event();

        let turn = external_adapter_event_to_ingress_turn("ses_external", &event).unwrap();

        assert_eq!(turn.attachments.len(), 1);
        assert_eq!(turn.attachments[0].id, "file_1");
        assert_eq!(turn.attachments[0].kind, "image");
        assert_eq!(turn.attachments[0].uri, "rocode://external/generic/file_1");
        assert!(!turn.user_intent_text.contains("file_1"));
    }

    #[test]
    fn external_adapter_dedupe_uses_stable_scoped_idempotency_key() {
        let first =
            external_adapter_event_to_ingress_turn("ses_external", &sample_external_event())
                .unwrap();
        let mut second_event = sample_external_event();
        second_event.text = "retry should not run twice".to_string();
        let second = external_adapter_event_to_ingress_turn("ses_external", &second_event).unwrap();

        let stabilized = stabilize_ingress_turns(vec![first, second]);

        assert_eq!(stabilized.len(), 1);
        assert_eq!(
            stabilized[0].stabilization.dedupe_keys,
            vec!["external:generic:generic-webhook:evt_1"]
        );
    }
}
