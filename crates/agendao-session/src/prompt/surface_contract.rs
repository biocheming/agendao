use std::collections::HashMap;

use agendao_orchestrator::output_projection::{
    ContextProjectionPolicy, SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY,
    SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY,
};
use serde_json::Value;

use crate::{MessageRole, PartType, SessionMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HiddenRuntimeHint {
    ProposalNotice,
    SkillSaveSuggestion,
    /// Steering preview messages written at enqueue time for UI feedback.
    /// Hidden from model-visible replay so the model never sees the
    /// "will be applied at next tool boundary" meta-notice as a user message.
    SteeringPreview,
}

impl HiddenRuntimeHint {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ProposalNotice => "proposal_notice",
            Self::SkillSaveSuggestion => "skill_save_suggestion",
            Self::SteeringPreview => "steering_preview",
        }
    }
}

pub(super) fn parse_hidden_runtime_hint(value: &str) -> Option<HiddenRuntimeHint> {
    match value {
        "proposal_notice" => Some(HiddenRuntimeHint::ProposalNotice),
        "skill_save_suggestion" => Some(HiddenRuntimeHint::SkillSaveSuggestion),
        "steering_preview" => Some(HiddenRuntimeHint::SteeringPreview),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SanctionedModelContextProjectionPath {
    SchedulerOutputSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelContextProjection<'a> {
    pub path: SanctionedModelContextProjectionPath,
    pub summary: &'a str,
    pub policy: ContextProjectionPolicy,
}

pub(super) fn sanctioned_model_context_projection(
    metadata: &HashMap<String, Value>,
) -> Option<ModelContextProjection<'_>> {
    let summary = metadata
        .get(SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|summary| !summary.is_empty())?;

    let policy = metadata
        .get(SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY)
        .and_then(|value| serde_json::from_value::<ContextProjectionPolicy>(value.clone()).ok())?;

    if matches!(
        policy,
        ContextProjectionPolicy::Full | ContextProjectionPolicy::Hidden
    ) {
        return None;
    }

    Some(ModelContextProjection {
        path: SanctionedModelContextProjectionPath::SchedulerOutputSummary,
        summary,
        policy,
    })
}

pub(super) fn sanctioned_model_context_projection_for_message(
    message: &SessionMessage,
) -> Option<ModelContextProjection<'_>> {
    if !matches!(message.role, MessageRole::Assistant) {
        return None;
    }

    if message
        .metadata
        .get("runtime_hint")
        .and_then(Value::as_str)
        .and_then(parse_hidden_runtime_hint)
        .is_some()
    {
        return None;
    }

    if message.parts.iter().any(|part| {
        matches!(
            part.part_type,
            PartType::ToolCall { .. } | PartType::ToolResult { .. } | PartType::Reasoning { .. }
        )
    }) {
        return None;
    }

    sanctioned_model_context_projection(&message.metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanctioned_model_context_projection_reads_policy_backed_summary() {
        let metadata = HashMap::from([
            (
                SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
                serde_json::to_value(ContextProjectionPolicy::OnDemandArtifact)
                    .expect("policy should serialize"),
            ),
            (
                SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
                serde_json::json!("artifact-backed summary"),
            ),
        ]);

        let projection =
            sanctioned_model_context_projection(&metadata).expect("projection should load");

        assert_eq!(
            projection.path,
            SanctionedModelContextProjectionPath::SchedulerOutputSummary
        );
        assert_eq!(projection.summary, "artifact-backed summary");
        assert_eq!(projection.policy, ContextProjectionPolicy::OnDemandArtifact);
    }

    #[test]
    fn sanctioned_model_context_projection_requires_policy() {
        let metadata = HashMap::from([(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("summary without policy"),
        )]);

        assert!(sanctioned_model_context_projection(&metadata).is_none());
    }

    #[test]
    fn sanctioned_model_context_projection_rejects_full_policy() {
        let metadata = HashMap::from([
            (
                SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
                serde_json::to_value(ContextProjectionPolicy::Full)
                    .expect("policy should serialize"),
            ),
            (
                SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
                serde_json::json!("should not project"),
            ),
        ]);

        assert!(sanctioned_model_context_projection(&metadata).is_none());
    }

    #[test]
    fn hidden_runtime_hint_registry_recognizes_known_hints() {
        assert_eq!(
            parse_hidden_runtime_hint("proposal_notice"),
            Some(HiddenRuntimeHint::ProposalNotice)
        );
        assert_eq!(
            parse_hidden_runtime_hint("skill_save_suggestion"),
            Some(HiddenRuntimeHint::SkillSaveSuggestion)
        );
        assert_eq!(
            parse_hidden_runtime_hint("steering_preview"),
            Some(HiddenRuntimeHint::SteeringPreview)
        );
        assert!(parse_hidden_runtime_hint("unknown").is_none());
    }

    // P2: reasoning-only assistant must not be projected as a text summary.
    #[test]
    fn reasoning_only_assistant_is_not_projected_as_summary() {
        let mut msg = SessionMessage::assistant("s");
        msg.add_reasoning("hidden chain of thought");

        let projection = sanctioned_model_context_projection_for_message(&msg);
        assert!(
            projection.is_none(),
            "reasoning-only assistant must not be projected as summary"
        );
    }

    // ── P1.1: stable / volatile boundary regression ──────────────────────
}
