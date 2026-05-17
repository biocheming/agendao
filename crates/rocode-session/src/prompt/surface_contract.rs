use std::borrow::Cow;
use std::collections::HashMap;

use rocode_orchestrator::output_projection::{
    ContextProjectionPolicy, SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY,
    SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY,
};
use serde_json::{Map, Value};

use crate::{MessageRole, PartType, SessionMessage};

pub(super) const VOLATILE_SYSTEM_SECTION_TITLES: &[&str] = &[
    "Exact Recent Tail",
    "Working Ledger",
    "Latest Compaction Summary",
];

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
pub(super) enum PromptSurfaceProviderOptionGroup {
    ReasoningMode,
    ToolPolicy,
}

impl PromptSurfaceProviderOptionGroup {
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::ReasoningMode => &[
                "reasoning",
                "reasoning_effort",
                "reasoningEffort",
                "thinking",
                "include_reasoning",
                "includeReasoning",
            ],
            Self::ToolPolicy => &[
                "allowed_tools",
                "allowedTools",
                "tool_choice",
                "toolChoice",
                "allowed_tool_names",
                "allowedToolNames",
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SanctionedModelContextProjectionPath {
    SchedulerOutputSummary,
}

impl SanctionedModelContextProjectionPath {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SchedulerOutputSummary => "scheduler_output_summary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelContextProjection<'a> {
    pub path: SanctionedModelContextProjectionPath,
    pub summary: &'a str,
    pub policy: Option<ContextProjectionPolicy>,
    pub legacy_without_policy: bool,
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
        .map(|value| serde_json::from_value::<ContextProjectionPolicy>(value.clone()))
        .transpose()
        .ok()?;

    if matches!(
        policy,
        Some(ContextProjectionPolicy::Full | ContextProjectionPolicy::Hidden)
    ) {
        return None;
    }

    Some(ModelContextProjection {
        path: SanctionedModelContextProjectionPath::SchedulerOutputSummary,
        summary,
        policy,
        legacy_without_policy: policy.is_none(),
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

pub(super) fn is_volatile_system_section(title: &str) -> bool {
    VOLATILE_SYSTEM_SECTION_TITLES
        .iter()
        .any(|volatile| title.eq_ignore_ascii_case(volatile))
}

pub(super) fn normalize_stable_system_line<'a>(line: &'a str) -> Cow<'a, str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("Today's date:") || trimmed.starts_with("Today’s date:") {
        let indent_len = line.len().saturating_sub(trimmed.len());
        let indent = &line[..indent_len];
        return Cow::Owned(format!("{indent}Today's date: <dynamic>"));
    }
    Cow::Borrowed(line)
}

pub(super) fn collect_prompt_surface_provider_options(
    provider_options: &HashMap<String, Value>,
    group: PromptSurfaceProviderOptionGroup,
) -> Map<String, Value> {
    let mut relevant = Map::new();
    for key in group.keys() {
        if let Some(value) = provider_options.get(*key) {
            relevant.insert((*key).to_string(), value.clone());
        }
    }
    relevant
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
        assert_eq!(
            projection.policy,
            Some(ContextProjectionPolicy::OnDemandArtifact)
        );
        assert!(!projection.legacy_without_policy);
    }

    #[test]
    fn sanctioned_model_context_projection_allows_legacy_summary_without_policy() {
        let metadata = HashMap::from([(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("legacy summary"),
        )]);

        let projection =
            sanctioned_model_context_projection(&metadata).expect("projection should load");

        assert_eq!(projection.summary, "legacy summary");
        assert!(projection.policy.is_none());
        assert!(projection.legacy_without_policy);
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

    #[test]
    fn volatile_system_section_registry_matches_case_insensitively() {
        assert!(is_volatile_system_section("Exact Recent Tail"));
        assert!(is_volatile_system_section("working ledger"));
        assert!(!is_volatile_system_section("Repository Digest"));
    }

    #[test]
    fn normalize_stable_system_line_rewrites_dynamic_date() {
        assert_eq!(
            normalize_stable_system_line("  Today's date: Fri May 01 2026"),
            "  Today's date: <dynamic>"
        );
        assert_eq!(normalize_stable_system_line("static line"), "static line");
    }

    #[test]
    fn provider_option_registry_collects_reasoning_and_tool_policy_keys() {
        let provider_options = HashMap::from([
            (
                "thinking".to_string(),
                serde_json::json!({"type": "enabled"}),
            ),
            (
                "tool_choice".to_string(),
                serde_json::json!({"type": "function", "function": {"name": "read"}}),
            ),
            ("irrelevant".to_string(), serde_json::json!(true)),
        ]);

        let reasoning = collect_prompt_surface_provider_options(
            &provider_options,
            PromptSurfaceProviderOptionGroup::ReasoningMode,
        );
        let tool_policy = collect_prompt_surface_provider_options(
            &provider_options,
            PromptSurfaceProviderOptionGroup::ToolPolicy,
        );

        assert_eq!(reasoning.len(), 1);
        assert!(reasoning.contains_key("thinking"));
        assert_eq!(tool_policy.len(), 1);
        assert!(tool_policy.contains_key("tool_choice"));
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
}
