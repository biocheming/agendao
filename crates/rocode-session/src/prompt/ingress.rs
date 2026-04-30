use std::collections::HashSet;

use serde::{Deserialize, Serialize};

const DEFAULT_WEB_BATCH_WINDOW_MS: i64 = 250;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressAttachmentRef {
    pub id: String,
    pub kind: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressStabilizationMetadata {
    pub batch_count: usize,
    pub dedupe_keys: Vec<String>,
    pub ordering_key: String,
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
    pub received_at_ms: i64,
    pub stabilized_at_ms: i64,
    pub user_intent_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<IngressAttachmentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quoted_context_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
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
            stabilization: IngressStabilizationMetadata::single(turn_id, "none"),
        }
    }
}

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
    target.stabilization.policy = "same_session_context_batch".to_string();
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
