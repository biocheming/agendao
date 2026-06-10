use std::borrow::Cow;
use std::collections::HashMap;

use agendao_orchestrator::output_projection::{
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

pub(super) const DYNAMIC_CATALOG_SECTION_TITLES: &[&str] = &[
    "Capability Projection",
    "Available Capabilities",
    "System Capabilities",
    "Available Execution Resources",
    "Available Skills",
    "Available Categories",
    "Tool & Agent Selection",
    "Delegation Table",
];

pub(super) const STABLE_GOVERNANCE_SECTION_TITLES: &[&str] = &[
    "Preset Role Summary",
    "Tone Augment",
    "Task Management",
    "Constraints",
    "Routing Goal",
    "Planner Charter",
    "Interview Charter",
    "Review Charter",
    "Handoff Charter",
    "Execution Charter",
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
    if trimmed.starts_with("Current local time:") {
        let indent_len = line.len().saturating_sub(trimmed.len());
        let indent = &line[..indent_len];
        return Cow::Owned(format!("{indent}Current local time: <dynamic>"));
    }
    if trimmed.starts_with("Local timezone:") {
        let indent_len = line.len().saturating_sub(trimmed.len());
        let indent = &line[..indent_len];
        return Cow::Owned(format!("{indent}Local timezone: <dynamic>"));
    }
    Cow::Borrowed(line)
}

pub(super) fn looks_like_clock_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("Today's date:")
        || trimmed.starts_with("Today’s date:")
        || trimmed.starts_with("Current local time:")
        || trimmed.starts_with("Local timezone:")
}

pub(super) fn is_dynamic_catalog_header(title: &str) -> bool {
    DYNAMIC_CATALOG_SECTION_TITLES
        .iter()
        .any(|candidate| title.eq_ignore_ascii_case(candidate))
}

pub(super) fn is_stable_governance_header(title: &str) -> bool {
    STABLE_GOVERNANCE_SECTION_TITLES
        .iter()
        .any(|candidate| title.eq_ignore_ascii_case(candidate))
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
        assert_eq!(
            normalize_stable_system_line("  Current local time: 2026-05-17 11:22:33 +08:00"),
            "  Current local time: <dynamic>"
        );
        assert_eq!(
            normalize_stable_system_line("  Local timezone: CST"),
            "  Local timezone: <dynamic>"
        );
        assert_eq!(normalize_stable_system_line("static line"), "static line");
    }

    #[test]
    fn clock_line_detector_matches_known_dynamic_fields() {
        assert!(looks_like_clock_line("  Today's date: Fri May 01 2026"));
        assert!(looks_like_clock_line(
            "  Current local time: 2026-05-17 11:22:33 +08:00"
        ));
        assert!(looks_like_clock_line("  Local timezone: CST"));
        assert!(!looks_like_clock_line("  Working directory: /repo"));
    }

    #[test]
    fn stable_governance_headers_are_not_marked_dynamic() {
        assert!(is_stable_governance_header("Planner Charter"));
        assert!(is_stable_governance_header("constraints"));
        assert!(!is_dynamic_catalog_header("Planner Charter"));
        assert!(!is_dynamic_catalog_header("Preset Role Summary"));
    }

    #[test]
    fn dynamic_catalog_headers_are_detected() {
        assert!(is_dynamic_catalog_header("Capability Projection"));
        assert!(is_dynamic_catalog_header("available execution resources"));
        assert!(is_dynamic_catalog_header("Delegation Table"));
        assert!(!is_stable_governance_header("Delegation Table"));
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

    // ── P1.1: stable / volatile boundary regression ──────────────────────

    /// The stable projection must exclude all volatile sections even when their
    /// content changes. The raw fingerprint differs (volatile content present),
    /// but the stripped text must match the version without volatile sections.
    #[test]
    fn stable_projection_excludes_volatile_sections_entirely() {
        let prompt_with_volatile = "\
## Repository Digest
stable line one

## Exact Recent Tail
user: what is the capital
assistant: Paris

## Working Ledger
step 1 complete

## Useful Commands
stable line two
";

        let expected_stable = "\
## Repository Digest
stable line one

## Useful Commands
stable line two
";

        // Verify that each volatile title is recognized.
        assert!(is_volatile_system_section("Exact Recent Tail"));
        assert!(is_volatile_system_section("Working Ledger"));
        assert!(is_volatile_system_section("Latest Compaction Summary"));

        // Raw fingerprint differs because of volatile content.
        let hash_raw = agendao_provider::cache::text_fingerprint(prompt_with_volatile);
        let hash_stable = agendao_provider::cache::text_fingerprint(expected_stable);
        assert_ne!(hash_raw, hash_stable);

        // After stripping volatile sections, the text must match.
        let stripped = strip_volatile_sections(prompt_with_volatile);

        // The stripped text must match expected (modulo trailing newline).
        // Normalize trailing whitespace before comparing.
        assert_eq!(
            stripped.trim(),
            expected_stable.trim(),
            "stripped=\n---\n{}\n---\nexpected=\n---\n{}\n---",
            stripped,
            expected_stable,
        );

        // Hashes must match after stripping.
        assert_eq!(
            agendao_provider::cache::text_fingerprint(stripped.trim()),
            agendao_provider::cache::text_fingerprint(expected_stable.trim()),
            "fingerprint mismatch: stripped.len={} expected.len={}",
            stripped.trim().len(),
            expected_stable.trim().len(),
        );
    }

    /// Dynamic environment fields (date/time/timezone) that vary with every
    /// invocation must normalize to <dynamic> so the stable hash is invariant.
    #[test]
    fn dynamic_time_fields_normalize_to_stable_form() {
        // Different dates → same normalized output.
        assert_eq!(
            normalize_stable_system_line("  Today's date: Mon Jan 01 2026"),
            "  Today's date: <dynamic>"
        );
        assert_eq!(
            normalize_stable_system_line("  Today's date: Tue Dec 25 2030"),
            "  Today's date: <dynamic>"
        );

        // Different times → same normalized output.
        assert_eq!(
            normalize_stable_system_line("  Current local time: 2026-01-01 00:00:00 UTC"),
            "  Current local time: <dynamic>"
        );
        assert_eq!(
            normalize_stable_system_line("  Current local time: 2030-12-25 23:59:59 +08:00"),
            "  Current local time: <dynamic>"
        );

        // Different timezones → same normalized output.
        assert_eq!(
            normalize_stable_system_line("  Local timezone: UTC"),
            "  Local timezone: <dynamic>"
        );
        assert_eq!(
            normalize_stable_system_line("  Local timezone: CST"),
            "  Local timezone: <dynamic>"
        );
    }

    /// A full system prompt with dynamic fields in stable sections must produce
    /// the same stable hash regardless of the actual date/time/timezone values.
    #[test]
    fn stable_system_surface_hash_invariant_to_dynamic_env_fields() {
        let prompt_jan = "\
## Environment
  Today's date: Mon Jan 01 2026
  Current local time: 2026-01-01 09:00:00 UTC
  Local timezone: UTC
## Repository Digest
stable content
";

        let prompt_jun = "\
## Environment
  Today's date: Tue Jun 09 2026
  Current local time: 2026-06-09 23:59:59 +08:00
  Local timezone: CST
## Repository Digest
stable content
";

        // The stable projection must be identical.
        let stripped_jan = strip_volatile_sections(prompt_jan);
        let stripped_jun = strip_volatile_sections(prompt_jun);

        let hash_jan =
            agendao_provider::cache::text_fingerprint(&normalize_all_lines(&stripped_jan));
        let hash_jun =
            agendao_provider::cache::text_fingerprint(&normalize_all_lines(&stripped_jun));

        assert_eq!(
            hash_jan, hash_jun,
            "stable system surface hash must be invariant to dynamic date/time/timezone values"
        );
    }

    /// Helper: strip lines belonging to volatile sections.
    fn strip_volatile_sections(text: &str) -> String {
        let mut result = Vec::new();
        let mut skipping = false;
        for line in text.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                skipping = is_volatile_system_section(header.trim());
                if skipping {
                    continue;
                }
            }
            if skipping {
                continue;
            }
            result.push(line.to_string());
        }
        result.join("\n")
    }

    /// Helper: apply normalize_stable_system_line to every line.
    fn normalize_all_lines(text: &str) -> String {
        text.lines()
            .map(|line| normalize_stable_system_line(line).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
