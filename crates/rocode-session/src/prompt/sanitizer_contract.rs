//! Shared sanitizer contract — the single entry point for message cleanup
//! across all four lifecycle paths: pre-request, fallback retry, session resume,
//! and post-compaction.
//!
//! Every sanitize action records uniform repair telemetry so that no path can
//! silently diverge from the others.
//!
//! ## Governance (ROCode Constitution §9)
//! - This is an orchestration-layer contract. Only `rocode-session` owns it.
//! - Adapters (tools, providers) inject state; they don't run sanitizer logic.
//! - All four paths funnel through the same stage dispatch.

use rocode_provider::protocols::request_sanitizer::{
    sanitize_messages_for_protocol_with_actions, SanitizerOptions,
};
use rocode_provider::Message as ProviderMessage;
use rocode_tool::{append_structured_repair_event, repair_event_builder, Metadata};
use rocode_types::{RepairEvent, RepairPolicy, SanitizerAction, SanitizerStage};

// ── Contract ────────────────────────────────────────────────────────────

/// Accumulator for sanitizer telemetry — records each action taken during
/// a sanitization pass.
#[derive(Debug, Clone, Default)]
pub struct SanitizerTelemetry {
    pub stage: Option<SanitizerStage>,
    pub actions: Vec<SanitizerAction>,
    pub message_count_before: usize,
    pub message_count_after: usize,
}

impl SanitizerTelemetry {
    pub fn new(stage: SanitizerStage) -> Self {
        Self {
            stage: Some(stage),
            actions: Vec::new(),
            message_count_before: 0,
            message_count_after: 0,
        }
    }

    /// Record an action and emit a structured repair event into the given metadata.
    pub fn record(&mut self, action: SanitizerAction, repair_metadata: &mut Metadata) {
        let stable_kind = action.repair_kind();
        tracing::debug!(
            stage = %self.stage.map(|s| s.label()).unwrap_or("unknown"),
            action = %stable_kind.as_str(),
            detail = %action.description(),
            "sanitizer action"
        );

        let event = repair_event_builder(stable_kind.as_str(), "sanitizer", "")
            .reason(action.description())
            .build();

        append_structured_repair_event(repair_metadata, &event);
        self.actions.push(action);
    }

    /// Convert accumulated actions to structured RepairEvents for a summary.
    pub fn to_repair_events(&self) -> Vec<RepairEvent> {
        self.actions
            .iter()
            .map(|action| {
                repair_event_builder(action.repair_kind().as_str(), "sanitizer", "")
                    .reason(action.description())
                    .build()
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

// ── Unified entry points ────────────────────────────────────────────────

/// Run the shared sanitizer contract over a set of provider messages.
///
/// This is the single entry point that all four lifecycle paths should use
/// instead of calling `sanitize_messages_for_protocol` directly.
///
/// In `Permissive` mode, synthetic repairs (interrupted tool placeholders) are
/// injected to keep the conversation flowing. In `Strict` mode, those synthetic
/// repairs are skipped but the corresponding repair events are still recorded
/// with `strict_mode_would_fail = true`.
pub fn sanitize_with_contract(
    messages: &[ProviderMessage],
    stage: SanitizerStage,
    policy: RepairPolicy,
    repair_metadata: &mut Metadata,
) -> (Vec<ProviderMessage>, SanitizerTelemetry) {
    let count_before = messages.len();
    let mut telemetry = SanitizerTelemetry::new(stage);
    telemetry.message_count_before = count_before;

    let options = SanitizerOptions {
        drop_thinking_only_assistant: true,
        skip_synthetic_repair: matches!(policy, RepairPolicy::Strict),
    };

    let mut actions = Vec::new();
    let sanitized =
        sanitize_messages_for_protocol_with_actions(messages, options, Some(&mut actions));

    telemetry.message_count_after = sanitized.len();

    // Convert every sanitizer action into a repair event.
    // In permissive mode, mark strict_mode_would_fail for synthetic actions.
    for action in &actions {
        let mut event = repair_event_builder(action.repair_kind().as_str(), "sanitizer", "")
            .reason(action.description())
            .build();
        if matches!(policy, RepairPolicy::Permissive) {
            event.strict_mode_would_fail = is_synthetic_repair_action(action);
        }
        append_structured_repair_event(repair_metadata, &event);
        telemetry.actions.push(action.clone());
    }

    (sanitized, telemetry)
}

/// Synthetic repairs are those that inject placeholder content rather than
/// fixing actual protocol violations.
fn is_synthetic_repair_action(action: &SanitizerAction) -> bool {
    matches!(action, SanitizerAction::OrphanedToolResult { .. })
}

/// Run the shared sanitizer for text-protocol (non-tool) projections.
pub fn sanitize_text_with_contract(
    messages: &[ProviderMessage],
    stage: SanitizerStage,
    policy: RepairPolicy,
    repair_metadata: &mut Metadata,
) -> (Vec<ProviderMessage>, SanitizerTelemetry) {
    sanitize_with_contract(messages, stage, policy, repair_metadata)
}

/// Convenience: run the sanitizer contract and return only the cleaned messages,
/// discarding telemetry.
pub fn sanitize_with_contract_quiet(
    messages: &[ProviderMessage],
    stage: SanitizerStage,
    policy: RepairPolicy,
) -> Vec<ProviderMessage> {
    let count_before = messages.len();
    let options = SanitizerOptions {
        drop_thinking_only_assistant: true,
        skip_synthetic_repair: matches!(policy, RepairPolicy::Strict),
    };
    let mut actions = Vec::new();
    let sanitized =
        sanitize_messages_for_protocol_with_actions(messages, options, Some(&mut actions));

    if !actions.is_empty() {
        tracing::debug!(
            stage = %stage,
            before = count_before,
            after = sanitized.len(),
            actions = actions.len(),
            "sanitizer contract (quiet): {} actions", actions.len()
        );
    }

    sanitized
}
