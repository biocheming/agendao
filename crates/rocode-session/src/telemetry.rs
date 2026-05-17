use std::collections::BTreeMap;

use crate::{MessageRole, PartType, Session, ToolState};
use rocode_provider::provider_diagnostic_from_metadata;
use rocode_types::{
    ModelToolRepairTelemetrySummary, RepairKind, SessionTelemetrySnapshot,
    SessionToolRepairTelemetrySummary, ToolRepairCount, ToolRepairToolSummary,
    ToolResultGovernanceSummary, ToolTrajectoryQualityBand, ToolTrajectoryQualityPenalty,
    ToolTrajectoryQualitySummary,
};

pub const SESSION_TELEMETRY_METADATA_KEY: &str = "telemetry";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTelemetryModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Default)]
struct ToolRepairToolAccumulator {
    call_count: u64,
    repaired_call_count: u64,
    error_call_count: u64,
    repair_event_count: u64,
    event_kinds: BTreeMap<String, u64>,
    failure_kinds: BTreeMap<String, u64>,
}

#[derive(Default)]
struct ToolRepairAccumulator {
    total_tool_calls: u64,
    repaired_tool_call_count: u64,
    error_tool_call_count: u64,
    repair_event_count: u64,
    failure_kinds: BTreeMap<String, u64>,
    provider_diagnostic_count: u64,
    provider_diagnostic_kinds: BTreeMap<String, u64>,
    event_kinds: BTreeMap<String, u64>,
    event_layers: BTreeMap<String, u64>,
    tools: BTreeMap<String, ToolRepairToolAccumulator>,
}

impl ToolRepairAccumulator {
    fn record_repair_events(
        &mut self,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> bool {
        let events = rocode_tool::structured_repair_events(metadata);
        if events.is_empty() {
            return false;
        }

        for event in &events {
            self.repair_event_count += 1;
            *self
                .event_kinds
                .entry(event.repair_kind.clone())
                .or_default() += 1;
            *self.event_layers.entry(event.layer.clone()).or_default() += 1;
        }

        true
    }

    fn record_tool_call(
        &mut self,
        tool_name: &str,
        is_error: bool,
        error_text: Option<&str>,
        metadata: Option<&std::collections::HashMap<String, serde_json::Value>>,
    ) {
        self.total_tool_calls += 1;
        if is_error {
            self.error_tool_call_count += 1;
        }

        let mut per_tool_failure_kind: Option<String> = None;
        {
            let tool_entry = self.tools.entry(tool_name.to_string()).or_default();
            tool_entry.call_count += 1;
            if is_error {
                tool_entry.error_call_count += 1;
                let kind = classify_tool_failure_kind(error_text).to_string();
                per_tool_failure_kind = Some(kind.clone());
                *tool_entry.failure_kinds.entry(kind).or_default() += 1;
            }
        }

        if let Some(kind) = per_tool_failure_kind {
            *self.failure_kinds.entry(kind).or_default() += 1;
        }

        let Some(metadata) = metadata else {
            return;
        };
        if !self.record_repair_events(metadata) {
            return;
        }

        self.repaired_tool_call_count += 1;
        let events = rocode_tool::structured_repair_events(metadata);
        {
            let tool_entry = self.tools.entry(tool_name.to_string()).or_default();
            tool_entry.repaired_call_count += 1;
            tool_entry.repair_event_count += events.len() as u64;
            for event in &events {
                *tool_entry
                    .event_kinds
                    .entry(event.repair_kind.clone())
                    .or_default() += 1;
            }
        }
    }

    fn record_provider_diagnostic(&mut self, code: &str) {
        self.provider_diagnostic_count += 1;
        *self
            .provider_diagnostic_kinds
            .entry(code.to_string())
            .or_default() += 1;
    }

    fn build_session_summary(self) -> Option<SessionToolRepairTelemetrySummary> {
        if self.total_tool_calls == 0
            && self.provider_diagnostic_count == 0
            && self.repair_event_count == 0
        {
            return None;
        }

        Some(SessionToolRepairTelemetrySummary {
            total_tool_calls: self.total_tool_calls,
            repaired_tool_call_count: self.repaired_tool_call_count,
            error_tool_call_count: self.error_tool_call_count,
            repair_event_count: self.repair_event_count,
            failure_kinds: counts_to_vec(self.failure_kinds),
            provider_diagnostic_count: self.provider_diagnostic_count,
            provider_diagnostic_kinds: counts_to_vec(self.provider_diagnostic_kinds),
            event_kinds: counts_to_vec(self.event_kinds),
            event_layers: counts_to_vec(self.event_layers),
            tools: self
                .tools
                .into_iter()
                .map(|(tool_name, tool)| ToolRepairToolSummary {
                    tool_name,
                    call_count: tool.call_count,
                    repaired_call_count: tool.repaired_call_count,
                    error_call_count: tool.error_call_count,
                    repair_event_count: tool.repair_event_count,
                    event_kinds: counts_to_vec(tool.event_kinds),
                    failure_kinds: counts_to_vec(tool.failure_kinds),
                })
                .collect(),
        })
    }
}

pub fn session_telemetry_model_ref(session: &Session) -> Option<SessionTelemetryModelRef> {
    let provider_id = session
        .metadata
        .get("model_provider")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let model_id = session
        .metadata
        .get("model_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    Some(SessionTelemetryModelRef {
        provider_id,
        model_id,
    })
}

pub fn build_session_tool_repair_telemetry(
    session: &Session,
) -> Option<SessionToolRepairTelemetrySummary> {
    let mut accumulator = ToolRepairAccumulator::default();

    let _ = accumulator.record_repair_events(&session.metadata);

    for message in &session.messages {
        if !matches!(message.role, MessageRole::Assistant) {
            continue;
        }

        if let Some(summary) = provider_diagnostic_from_metadata(&message.metadata) {
            accumulator.record_provider_diagnostic(&summary.code);
        }

        for part in &message.parts {
            let PartType::ToolCall {
                name,
                status,
                state,
                ..
            } = &part.part_type
            else {
                continue;
            };

            let Some((is_error, error_text, metadata)) =
                finalized_tool_call_metadata(status, state.as_ref())
            else {
                continue;
            };
            accumulator.record_tool_call(name, is_error, error_text, metadata);
        }
    }

    accumulator.build_session_summary()
}

pub fn build_session_tool_result_governance_summary(
    session: &Session,
) -> Option<ToolResultGovernanceSummary> {
    let mut single_result_governed_count = 0u64;
    let mut batch_governed_count = 0u64;
    let mut transcript_fallback_count = 0u64;

    for message in &session.messages {
        for part in &message.parts {
            let PartType::ToolResult { metadata, .. } = &part.part_type else {
                continue;
            };
            let Some(metadata) = metadata.as_ref() else {
                continue;
            };

            if metadata
                .get("tool_result_governed")
                .and_then(|value| value.as_bool())
                == Some(true)
            {
                single_result_governed_count += 1;
            }
            if metadata
                .get("tool_result_batch_governed")
                .and_then(|value| value.as_bool())
                == Some(true)
            {
                batch_governed_count += 1;
            }
            if metadata
                .get("tool_result_transcript_fallback_truncated")
                .and_then(|value| value.as_bool())
                == Some(true)
            {
                transcript_fallback_count += 1;
            }
        }
    }

    if single_result_governed_count == 0
        && batch_governed_count == 0
        && transcript_fallback_count == 0
    {
        return None;
    }

    Some(ToolResultGovernanceSummary {
        single_result_governed_count,
        batch_governed_count,
        transcript_fallback_count,
    })
}

pub fn aggregate_model_tool_repair_telemetry<'a>(
    sessions: impl IntoIterator<Item = &'a Session>,
    provider_id: &str,
    model_id: &str,
) -> Option<ModelToolRepairTelemetrySummary> {
    let mut session_count = 0u64;
    let mut repaired_session_count = 0u64;
    let mut error_session_count = 0u64;
    let mut provider_diagnostic_session_count = 0u64;
    let mut aggregate = ToolRepairAccumulator::default();

    for session in sessions {
        let Some(model_ref) = session_telemetry_model_ref(session) else {
            continue;
        };
        if model_ref.provider_id != provider_id || model_ref.model_id != model_id {
            continue;
        }

        let Some(summary) = build_session_tool_repair_telemetry(session) else {
            continue;
        };

        session_count += 1;
        if summary.repaired_tool_call_count > 0 || summary.repair_event_count > 0 {
            repaired_session_count += 1;
        }
        if summary.error_tool_call_count > 0 {
            error_session_count += 1;
        }
        if summary.provider_diagnostic_count > 0 {
            provider_diagnostic_session_count += 1;
        }

        aggregate.total_tool_calls += summary.total_tool_calls;
        aggregate.repaired_tool_call_count += summary.repaired_tool_call_count;
        aggregate.error_tool_call_count += summary.error_tool_call_count;
        aggregate.repair_event_count += summary.repair_event_count;
        aggregate.provider_diagnostic_count += summary.provider_diagnostic_count;

        for count in summary.failure_kinds {
            *aggregate.failure_kinds.entry(count.key).or_default() += count.count;
        }
        for count in summary.provider_diagnostic_kinds {
            *aggregate
                .provider_diagnostic_kinds
                .entry(count.key)
                .or_default() += count.count;
        }
        for count in summary.event_kinds {
            *aggregate.event_kinds.entry(count.key).or_default() += count.count;
        }
        for count in summary.event_layers {
            *aggregate.event_layers.entry(count.key).or_default() += count.count;
        }
        for tool in summary.tools {
            let entry = aggregate.tools.entry(tool.tool_name).or_default();
            entry.call_count += tool.call_count;
            entry.repaired_call_count += tool.repaired_call_count;
            entry.error_call_count += tool.error_call_count;
            entry.repair_event_count += tool.repair_event_count;
            for count in tool.event_kinds {
                *entry.event_kinds.entry(count.key).or_default() += count.count;
            }
            for count in tool.failure_kinds {
                *entry.failure_kinds.entry(count.key).or_default() += count.count;
            }
        }
    }

    if session_count == 0 {
        return None;
    }

    Some(ModelToolRepairTelemetrySummary {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        session_count,
        repaired_session_count,
        error_session_count,
        provider_diagnostic_session_count,
        total_tool_calls: aggregate.total_tool_calls,
        repaired_tool_call_count: aggregate.repaired_tool_call_count,
        error_tool_call_count: aggregate.error_tool_call_count,
        repair_event_count: aggregate.repair_event_count,
        failure_kinds: counts_to_vec(aggregate.failure_kinds),
        provider_diagnostic_count: aggregate.provider_diagnostic_count,
        provider_diagnostic_kinds: counts_to_vec(aggregate.provider_diagnostic_kinds),
        event_kinds: counts_to_vec(aggregate.event_kinds),
        event_layers: counts_to_vec(aggregate.event_layers),
        tools: aggregate
            .tools
            .into_iter()
            .map(|(tool_name, tool)| ToolRepairToolSummary {
                tool_name,
                call_count: tool.call_count,
                repaired_call_count: tool.repaired_call_count,
                error_call_count: tool.error_call_count,
                repair_event_count: tool.repair_event_count,
                event_kinds: counts_to_vec(tool.event_kinds),
                failure_kinds: counts_to_vec(tool.failure_kinds),
            })
            .collect(),
    })
}

fn finalized_tool_call_metadata<'a>(
    status: &crate::ToolCallStatus,
    state: Option<&'a ToolState>,
) -> Option<(
    bool,
    Option<&'a str>,
    Option<&'a std::collections::HashMap<String, serde_json::Value>>,
)> {
    match state {
        Some(ToolState::Completed { metadata, .. }) => Some((false, None, Some(metadata))),
        Some(ToolState::Error {
            error, metadata, ..
        }) => Some((true, Some(error.as_str()), metadata.as_ref())),
        _ if matches!(status, crate::ToolCallStatus::Completed) => Some((false, None, None)),
        _ if matches!(status, crate::ToolCallStatus::Error) => Some((true, None, None)),
        _ => None,
    }
}

fn classify_tool_failure_kind(error_text: Option<&str>) -> &'static str {
    let Some(error_text) = error_text else {
        return "error";
    };
    let lower = error_text.trim().to_ascii_lowercase();
    if lower.starts_with("permission denied:") || lower.contains("permission denied") {
        "permission_denied"
    } else if lower.starts_with("file not found:") || lower.contains("file not found") {
        "file_not_found"
    } else if lower.starts_with("timeout:")
        || lower.contains("timeout:")
        || lower.contains("timed out")
    {
        "timeout"
    } else if lower.starts_with("invalid arguments:")
        || lower.contains("invalid arguments:")
        || lower.starts_with("validation error:")
        || lower.contains("validation error:")
    {
        "invalid_arguments"
    } else if lower == "cancelled" || lower.contains("cancelled") || lower.contains("canceled") {
        "cancelled"
    } else {
        "execution_error"
    }
}

fn counts_to_vec(counts: BTreeMap<String, u64>) -> Vec<ToolRepairCount> {
    counts
        .into_iter()
        .map(|(key, count)| ToolRepairCount { key, count })
        .collect()
}

pub fn persist_session_telemetry_snapshot(
    session: &mut Session,
    snapshot: &SessionTelemetrySnapshot,
) -> anyhow::Result<()> {
    // Keep the session row in sync with the final end-of-turn telemetry
    // snapshot instead of writing usage incrementally on every token event.
    session.update_usage(snapshot.usage.clone());
    let value = serde_json::to_value(snapshot)?;
    session.insert_metadata(SESSION_TELEMETRY_METADATA_KEY.to_string(), value);
    Ok(())
}

pub fn load_session_telemetry_snapshot(session: &Session) -> Option<SessionTelemetrySnapshot> {
    session
        .metadata
        .get(SESSION_TELEMETRY_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

pub fn session_last_run_status_label(session: &Session) -> String {
    if let Some(label) = session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, MessageRole::Assistant))
        .and_then(|message| {
            message
                .finish
                .clone()
                .or_else(|| {
                    message
                        .metadata
                        .get("finish_reason")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned)
                })
                .map(|value| normalize_finish_reason(&value))
        })
    {
        return label;
    }

    match session.status {
        crate::SessionStatus::Completed => "completed".to_string(),
        crate::SessionStatus::Archived => "archived".to_string(),
        crate::SessionStatus::Compacting => "compacting".to_string(),
        crate::SessionStatus::Active => "active".to_string(),
    }
}

fn normalize_finish_reason(reason: &str) -> String {
    let trimmed = reason.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "stop" | "completed" => "completed".to_string(),
        "cancelled" | "canceled" | "abort" | "aborted" => "cancelled".to_string(),
        other => other.to_string(),
    }
}

// ── P2.1: Tool trajectory quality scoring ───────────────────────────────

/// Build a deterministic session-level quality score from existing telemetry
/// signals. This is a pure read-only function — it does not modify metadata.
pub fn build_session_tool_trajectory_quality(
    session: &Session,
) -> Option<ToolTrajectoryQualitySummary> {
    let repair_summary = build_session_tool_repair_telemetry(session)?;
    let query_snapshot = crate::repair_query::build_session_repair_query_snapshot(session);

    let total_tool_calls = repair_summary.total_tool_calls;
    let error_tool_call_count = repair_summary.error_tool_call_count;
    let repaired_tool_call_count = repair_summary.repaired_tool_call_count;
    let repair_event_count = repair_summary.repair_event_count;
    let provider_diagnostic_count = repair_summary.provider_diagnostic_count;

    let strict_would_fail_count = query_snapshot
        .as_ref()
        .map_or(0, |s| s.summary.strict_would_fail_count);
    let invalid_reroute_count = query_snapshot
        .as_ref()
        .map_or(0, |s| count_kind(&s.rows, RepairKind::InvalidToolReroute));
    let sanitizer_event_count = query_snapshot.as_ref().map_or(0, |s| {
        s.summary.total_events.saturating_sub(
            s.rows
                .iter()
                .filter(|r| r.layer != "sanitizer")
                .map(|r| r.count)
                .sum(),
        )
    });

    let orphan_tool_result_count = query_snapshot.as_ref().map_or(0, |s| {
        count_kind(&s.rows, RepairKind::SanitizerOrphanedToolResult)
    });
    let duplicate_tool_id_count = query_snapshot.as_ref().map_or(0, |s| {
        count_kind(&s.rows, RepairKind::SanitizerDuplicateToolId)
    });
    let malformed_placeholder_count = query_snapshot.as_ref().map_or(0, |s| {
        count_kind(&s.rows, RepairKind::SanitizerAssistantMalformedPlaceholder)
    });
    let trailing_invalid_thinking_count = query_snapshot.as_ref().map_or(0, |s| {
        count_kind(&s.rows, RepairKind::SanitizerTrailingInvalidThinkingBlock)
    });

    // Also count session-level sanitizer repair events recorded via
    // sanitize_with_contract into session.metadata (not tool-call metadata).
    let session_level_events = rocode_tool::structured_repair_events(&session.record().metadata);
    let session_sanitizer_count = session_level_events
        .iter()
        .filter(|e| e.layer == "sanitizer")
        .count() as u64;
    let session_orphan_count = session_level_events
        .iter()
        .filter(|e| e.normalized_kind() == Some(RepairKind::SanitizerOrphanedToolResult))
        .count() as u64;
    let session_duplicate_count = session_level_events
        .iter()
        .filter(|e| e.normalized_kind() == Some(RepairKind::SanitizerDuplicateToolId))
        .count() as u64;
    let session_malformed_count = session_level_events
        .iter()
        .filter(|e| e.normalized_kind() == Some(RepairKind::SanitizerAssistantMalformedPlaceholder))
        .count() as u64;
    let session_trailing_thinking_count = session_level_events
        .iter()
        .filter(|e| e.normalized_kind() == Some(RepairKind::SanitizerTrailingInvalidThinkingBlock))
        .count() as u64;

    let orphan_tool_result_count = orphan_tool_result_count + session_orphan_count;
    let duplicate_tool_id_count = duplicate_tool_id_count + session_duplicate_count;
    let malformed_placeholder_count = malformed_placeholder_count + session_malformed_count;
    let trailing_invalid_thinking_count =
        trailing_invalid_thinking_count + session_trailing_thinking_count;
    let sanitizer_event_count = sanitizer_event_count + session_sanitizer_count;

    let mut score: i32 = 100;
    let mut penalties: Vec<ToolTrajectoryQualityPenalty> = Vec::new();

    let mut penalize = |key: &str, count: u64, unit: i32| {
        if count > 0 {
            let points = (count as i32 * unit).min(score);
            score -= points;
            penalties.push(ToolTrajectoryQualityPenalty {
                key: key.to_string(),
                count,
                points: -points,
            });
        }
    };

    penalize("error_tool_calls", error_tool_call_count, 12);
    penalize("provider_diagnostics", provider_diagnostic_count, 10);
    penalize("orphan_tool_results", orphan_tool_result_count, 10);
    penalize("duplicate_tool_ids", duplicate_tool_id_count, 8);
    penalize("strict_would_fail", strict_would_fail_count, 8);
    penalize("malformed_placeholders", malformed_placeholder_count, 8);
    penalize("invalid_reroute", invalid_reroute_count, 6);
    penalize(
        "trailing_invalid_thinking",
        trailing_invalid_thinking_count,
        5,
    );
    penalize(
        "other_sanitizer_events",
        sanitizer_event_count.saturating_sub(
            orphan_tool_result_count
                + duplicate_tool_id_count
                + malformed_placeholder_count
                + trailing_invalid_thinking_count,
        ),
        2,
    );

    score = score.clamp(0, 100);

    let band = match score {
        90..=100 => ToolTrajectoryQualityBand::Clean,
        70..=89 => ToolTrajectoryQualityBand::Recoverable,
        45..=69 => ToolTrajectoryQualityBand::Degraded,
        _ => ToolTrajectoryQualityBand::Risky,
    };

    let mut notes: Vec<String> = Vec::new();
    if score >= 90 && error_tool_call_count == 0 && repair_event_count == 0 {
        notes.push("clean_success_path".to_string());
    }
    if error_tool_call_count == 0 && repair_event_count > 0 {
        notes.push("success_with_repairs".to_string());
    }
    if error_tool_call_count > 0 && (repair_event_count > 0 || provider_diagnostic_count > 0) {
        notes.push("blocked_and_noisy".to_string());
    }
    if sanitizer_event_count >= 3 {
        notes.push("sanitizer_heavy".to_string());
    }
    if provider_diagnostic_count > 0 {
        notes.push("provider_rejection_present".to_string());
    }

    Some(ToolTrajectoryQualitySummary {
        score: score as u8,
        band,
        total_tool_calls,
        repaired_tool_call_count,
        error_tool_call_count,
        repair_event_count,
        provider_diagnostic_count,
        strict_would_fail_count,
        invalid_reroute_count,
        sanitizer_event_count,
        orphan_tool_result_count,
        duplicate_tool_id_count,
        malformed_placeholder_count,
        trailing_invalid_thinking_count,
        penalties,
        notes,
    })
}

fn count_kind(rows: &[rocode_types::RepairAggregateRow], kind: RepairKind) -> u64 {
    rows.iter()
        .filter(|r| r.repair_kind == kind)
        .map(|r| r.count)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionMessage;
    use rocode_types::{
        PersistedStageTelemetrySummary, SessionTelemetrySnapshotVersion, SessionUsage,
    };

    fn sample_snapshot() -> SessionTelemetrySnapshot {
        SessionTelemetrySnapshot {
            version: SessionTelemetrySnapshotVersion::V4,
            usage: SessionUsage {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 3,
                cache_write_tokens: 4,
                cache_read_tokens: 5,
                cache_miss_tokens: 0,
                context_tokens: 10,
                total_cost: 0.25,
            },
            stage_summaries: vec![PersistedStageTelemetrySummary {
                stage_id: "stage-1".to_string(),
                stage_name: "Plan".to_string(),
                index: Some(1),
                total: Some(2),
                step: Some(1),
                step_total: Some(3),
                status: rocode_content::stage_protocol::StageStatus::Running,
                prompt_tokens: Some(11),
                completion_tokens: Some(7),
                reasoning_tokens: Some(5),
                cache_read_tokens: Some(2),
                cache_write_tokens: Some(1),
                focus: Some("inspect".to_string()),
                last_event: Some("scheduler.stage.started".to_string()),
                waiting_on: None,
                activity: Some("Inspecting scheduler state".to_string()),
                estimated_context_tokens: Some(99),
                skill_tree_budget: Some(512),
                skill_tree_truncation_strategy: Some("head".to_string()),
                skill_tree_truncated: Some(false),
                retry_attempt: None,
                active_agent_count: 1,
                active_tool_count: 2,
                attached_session_count: 0,
                primary_attached_session_id: None,
            }],
            tool_repair_summary: Some(SessionToolRepairTelemetrySummary {
                total_tool_calls: 3,
                repaired_tool_call_count: 2,
                error_tool_call_count: 1,
                repair_event_count: 4,
                failure_kinds: vec![ToolRepairCount {
                    key: "invalid_arguments".to_string(),
                    count: 1,
                }],
                provider_diagnostic_count: 1,
                provider_diagnostic_kinds: vec![ToolRepairCount {
                    key: "thinking_replay_rejected".to_string(),
                    count: 1,
                }],
                event_kinds: vec![ToolRepairCount {
                    key: "alias_normalization".to_string(),
                    count: 2,
                }],
                event_layers: vec![ToolRepairCount {
                    key: "tool".to_string(),
                    count: 4,
                }],
                tools: vec![ToolRepairToolSummary {
                    tool_name: "task_flow".to_string(),
                    call_count: 2,
                    repaired_call_count: 2,
                    error_call_count: 1,
                    repair_event_count: 4,
                    event_kinds: vec![ToolRepairCount {
                        key: "alias_normalization".to_string(),
                        count: 2,
                    }],
                    failure_kinds: vec![ToolRepairCount {
                        key: "invalid_arguments".to_string(),
                        count: 1,
                    }],
                }],
            }),
            memory: None,
            compaction_continuity: None,
            repair_query_snapshot: None,
            tool_trajectory_quality: None,
            tool_result_governance: None,
            pending_permission_count: 0,
            granted_by_turn_count: 0,
            granted_by_session_count: 0,
            granted_by_matcher_kind: std::collections::BTreeMap::new(),
            last_permission_matcher_kind: None,
            last_permission_grant_target: None,
            last_permission_miss_count: 0,
            pending_steering_count: 0,
            consumed_steering_count: 0,
            last_steering_injected_at: None,
            last_steering_source_session_id: None,
            last_run_status: "completed".to_string(),
            updated_at: 123,
        }
    }

    #[test]
    fn telemetry_snapshot_roundtrips_via_session_metadata() {
        let mut session = Session::new("proj", ".");
        let snapshot = sample_snapshot();
        persist_session_telemetry_snapshot(&mut session, &snapshot).expect("persist should work");

        let loaded = load_session_telemetry_snapshot(&session).expect("snapshot should load");
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn telemetry_snapshot_syncs_usage_into_session_record() {
        let mut session = Session::new("proj", ".");
        let snapshot = sample_snapshot();

        persist_session_telemetry_snapshot(&mut session, &snapshot).expect("persist should work");

        assert_eq!(session.record().usage.as_ref(), Some(&snapshot.usage));
    }

    #[test]
    fn telemetry_snapshot_load_tolerates_corrupted_metadata() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            SESSION_TELEMETRY_METADATA_KEY.to_string(),
            serde_json::json!({"usage": "bad"}),
        );

        assert!(load_session_telemetry_snapshot(&session).is_none());
    }

    #[test]
    fn session_last_run_status_prefers_latest_assistant_finish_reason() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.finish = Some("stop".to_string());
        session.messages_mut().push(assistant);

        assert_eq!(session_last_run_status_label(&session), "completed");
    }

    #[test]
    fn build_session_tool_repair_telemetry_aggregates_tool_states() {
        let mut session = Session::new("proj", ".");

        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.add_tool_call("call-1", "task_flow", serde_json::json!({}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            let mut metadata = rocode_tool::Metadata::new();
            rocode_tool::append_tool_repair_event_map(&mut metadata, {
                let mut event =
                    rocode_tool::tool_repair_event("alias_normalization", "tool", "task_flow");
                event.insert(
                    "aliases".to_string(),
                    serde_json::json!(["action->operation"]),
                );
                event
            });
            *state = Some(ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Task".to_string(),
                metadata,
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }

        assistant.add_tool_call("call-2", "read", serde_json::json!({}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Error;
            let mut metadata = rocode_tool::Metadata::new();
            rocode_tool::append_tool_repair_event_map(&mut metadata, {
                let mut event =
                    rocode_tool::tool_repair_event("basename_auto_repair", "tool", "read");
                event.insert("from".to_string(), serde_json::json!("Game.ts"));
                event.insert(
                    "to".to_string(),
                    serde_json::json!("/tmp/project/src/Game.ts"),
                );
                event
            });
            *state = Some(ToolState::Error {
                input: serde_json::json!({}),
                error: "boom".to_string(),
                metadata: Some(metadata),
                time: crate::ErrorTime { start: 3, end: 4 },
            });
        }
        session.push_message(assistant);

        let diag_message = session.add_assistant_message();
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("v4-flash".to_string()),
            message: "thinking replay rejected".to_string(),
        }
        .attach_to_metadata(&mut diag_message.metadata);

        let summary = build_session_tool_repair_telemetry(&session).expect("summary");
        assert_eq!(summary.total_tool_calls, 2);
        assert_eq!(summary.repaired_tool_call_count, 2);
        assert_eq!(summary.error_tool_call_count, 1);
        assert_eq!(summary.repair_event_count, 2);
        assert_eq!(summary.provider_diagnostic_count, 1);
        assert!(summary
            .failure_kinds
            .iter()
            .any(|count| count.key == "execution_error" && count.count == 1));
        assert!(summary
            .provider_diagnostic_kinds
            .iter()
            .any(|count| count.key == "thinking_replay_rejected" && count.count == 1));
        assert!(summary
            .event_kinds
            .iter()
            .any(|count| count.key == "alias_normalization" && count.count == 1));
        assert!(summary
            .event_kinds
            .iter()
            .any(|count| count.key == "basename_auto_repair" && count.count == 1));
        assert!(summary.tools.iter().any(|tool| {
            tool.tool_name == "read"
                && tool.error_call_count == 1
                && tool.repaired_call_count == 1
                && tool
                    .failure_kinds
                    .iter()
                    .any(|count| count.key == "execution_error" && count.count == 1)
        }));
    }

    #[test]
    fn build_session_tool_repair_telemetry_includes_session_level_sanitizer_repairs() {
        let mut session = Session::new("proj", ".");
        let mut metadata = rocode_tool::Metadata::new();
        let event = rocode_tool::repair_event_builder("thinking_only_assistant", "sanitizer", "")
            .reason("dropped assistant message with only thinking blocks")
            .build();
        rocode_tool::append_structured_repair_event(&mut metadata, &event);
        session.record_mut().metadata.extend(metadata);

        let summary = build_session_tool_repair_telemetry(&session).expect("summary");
        assert_eq!(summary.total_tool_calls, 0);
        assert_eq!(summary.repair_event_count, 1);
        assert!(summary
            .event_kinds
            .iter()
            .any(|count| count.key == "thinking_only_assistant" && count.count == 1));
        assert!(summary
            .event_layers
            .iter()
            .any(|count| count.key == "sanitizer" && count.count == 1));
    }

    #[test]
    fn aggregate_model_tool_repair_telemetry_groups_matching_sessions() {
        let mut first = Session::new("proj", ".");
        first.insert_metadata("model_provider".to_string(), serde_json::json!("deepseek"));
        first.insert_metadata("model_id".to_string(), serde_json::json!("v4-flash"));
        let mut assistant = SessionMessage::assistant(first.id.clone());
        assistant.add_tool_call("call-1", "task_flow", serde_json::json!({}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            let mut metadata = rocode_tool::Metadata::new();
            rocode_tool::append_tool_repair_event_map(
                &mut metadata,
                rocode_tool::tool_repair_event("alias_normalization", "tool", "task_flow"),
            );
            *state = Some(ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Task".to_string(),
                metadata,
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }
        first.push_message(assistant);

        let mut second = Session::new("proj", ".");
        second.insert_metadata("model_provider".to_string(), serde_json::json!("deepseek"));
        second.insert_metadata("model_id".to_string(), serde_json::json!("v4-flash"));
        let mut second_assistant = SessionMessage::assistant(second.id.clone());
        second_assistant.add_tool_call("call-2", "read", serde_json::json!({}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = second_assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            *state = Some(ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Read".to_string(),
                metadata: rocode_tool::Metadata::new(),
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }
        second.push_message(second_assistant);
        let diag = second.add_assistant_message();
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("v4-flash".to_string()),
            message: "thinking replay rejected".to_string(),
        }
        .attach_to_metadata(&mut diag.metadata);

        let mut third = Session::new("proj", ".");
        third.insert_metadata("model_provider".to_string(), serde_json::json!("openai"));
        third.insert_metadata("model_id".to_string(), serde_json::json!("gpt-4.1"));

        let summary = aggregate_model_tool_repair_telemetry(
            [&first, &second, &third],
            "deepseek",
            "v4-flash",
        )
        .expect("summary");

        assert_eq!(summary.session_count, 2);
        assert_eq!(summary.repaired_session_count, 1);
        assert_eq!(summary.error_session_count, 0);
        assert_eq!(summary.provider_diagnostic_session_count, 1);
        assert_eq!(summary.total_tool_calls, 2);
        assert_eq!(summary.repaired_tool_call_count, 1);
        assert_eq!(summary.repair_event_count, 1);
        assert_eq!(summary.provider_diagnostic_count, 1);
        assert!(summary
            .provider_diagnostic_kinds
            .iter()
            .any(|count| count.key == "thinking_replay_rejected" && count.count == 1));
        assert!(summary
            .tools
            .iter()
            .any(|tool| tool.tool_name == "task_flow" && tool.repaired_call_count == 1));
    }

    #[test]
    fn aggregate_model_tool_repair_telemetry_counts_session_level_repairs_as_repaired_sessions() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("deepseek"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("v4-flash"));
        let mut metadata = rocode_tool::Metadata::new();
        rocode_tool::append_structured_repair_event(
            &mut metadata,
            &rocode_tool::repair_event_builder("orphaned_tool_result", "sanitizer", "")
                .reason("orphaned tool_result without pending tool_use")
                .build(),
        );
        session.record_mut().metadata.extend(metadata);

        let summary = aggregate_model_tool_repair_telemetry([&session], "deepseek", "v4-flash")
            .expect("summary");
        assert_eq!(summary.session_count, 1);
        assert_eq!(summary.repaired_session_count, 1);
        assert_eq!(summary.total_tool_calls, 0);
        assert_eq!(summary.repair_event_count, 1);
    }

    #[test]
    fn build_session_tool_repair_telemetry_classifies_common_failure_kinds() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());

        for (idx, tool_name, error) in [
            ("1", "bash", "Permission denied: bash requires approval"),
            ("2", "read", "File not found: missing.txt"),
            ("3", "websearch", "Timeout: search request timed out"),
            (
                "4",
                "skill_manage",
                "Invalid arguments: create requires either `body` or `methodology`",
            ),
            ("5", "task_flow", "Cancelled"),
        ] {
            assistant.add_tool_call(format!("call-{idx}"), tool_name, serde_json::json!({}));
            if let Some(crate::MessagePart {
                part_type: PartType::ToolCall { status, state, .. },
                ..
            }) = assistant.parts.last_mut()
            {
                *status = crate::ToolCallStatus::Error;
                *state = Some(ToolState::Error {
                    input: serde_json::json!({}),
                    error: error.to_string(),
                    metadata: None,
                    time: crate::ErrorTime { start: 1, end: 2 },
                });
            }
        }
        session.push_message(assistant);

        let summary = build_session_tool_repair_telemetry(&session).expect("summary");
        assert_eq!(summary.error_tool_call_count, 5);
        for kind in [
            "permission_denied",
            "file_not_found",
            "timeout",
            "invalid_arguments",
            "cancelled",
        ] {
            assert!(summary
                .failure_kinds
                .iter()
                .any(|count| count.key == kind && count.count == 1));
        }
    }

    #[test]
    fn aggregate_model_tool_repair_telemetry_keeps_provider_only_sessions() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("deepseek"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("v4-flash"));
        let assistant = session.add_assistant_message();
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::RequestValidation,
            code: "thinking_replay_missing".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("v4-flash".to_string()),
            message: "thinking replay missing".to_string(),
        }
        .attach_to_metadata(&mut assistant.metadata);

        let summary = aggregate_model_tool_repair_telemetry([&session], "deepseek", "v4-flash")
            .expect("provider-only session should still aggregate");

        assert_eq!(summary.session_count, 1);
        assert_eq!(summary.total_tool_calls, 0);
        assert_eq!(summary.provider_diagnostic_session_count, 1);
        assert_eq!(summary.provider_diagnostic_count, 1);
        assert!(summary
            .provider_diagnostic_kinds
            .iter()
            .any(|count| count.key == "thinking_replay_missing" && count.count == 1));
    }

    // ── P2.1 quality scoring tests ────────────────────────────────────

    #[test]
    fn build_session_tool_trajectory_quality_scores_clean_success_high() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.add_tool_call("call-1", "read", serde_json::json!({"file_path":"a.txt"}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            *state = Some(ToolState::Completed {
                input: serde_json::json!({"file_path": "a.txt"}),
                output: "ok".to_string(),
                title: "Read".to_string(),
                metadata: rocode_tool::Metadata::new(),
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }
        assistant.add_tool_call("call-2", "echo", serde_json::json!({"value":"hi"}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            *state = Some(ToolState::Completed {
                input: serde_json::json!({"value": "hi"}),
                output: "hi".to_string(),
                title: "Echo".to_string(),
                metadata: rocode_tool::Metadata::new(),
                time: crate::CompletedTime {
                    start: 3,
                    end: 4,
                    compacted: None,
                },
                attachments: None,
            });
        }
        session.push_message(assistant);

        let quality = build_session_tool_trajectory_quality(&session).expect("should build");
        assert!(quality.score >= 90);
        assert_eq!(quality.band, ToolTrajectoryQualityBand::Clean);
        assert!(quality.notes.contains(&"clean_success_path".to_string()));
    }

    #[test]
    fn build_session_tool_trajectory_quality_penalizes_success_with_repairs() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.add_tool_call("call-1", "read", serde_json::json!({"file_path":"a.txt"}));
        let mut metadata = rocode_tool::Metadata::new();
        rocode_tool::append_tool_repair_event_map(
            &mut metadata,
            rocode_tool::tool_repair_event("alias_normalization", "tool", "read"),
        );
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            *state = Some(ToolState::Completed {
                input: serde_json::json!({"file_path": "a.txt"}),
                output: "ok".to_string(),
                title: "Read".to_string(),
                metadata,
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }
        session.push_message(assistant);

        let quality = build_session_tool_trajectory_quality(&session).expect("should build");
        // Repaired but no error: score should still be high but notes reflect repairs.
        assert!(quality.repaired_tool_call_count > 0);
        assert_eq!(quality.error_tool_call_count, 0);
        assert!(quality.notes.contains(&"success_with_repairs".to_string()));
    }

    #[test]
    fn build_session_tool_trajectory_quality_marks_blocked_noisy_session_low() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("test"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("test"));

        let mut assistant = SessionMessage::assistant(session.id.clone());
        // 4 errors to push score low enough for Degraded band.
        for i in 0..4 {
            let call_id = format!("call-{}", i);
            assistant.add_tool_call(&call_id, "fail_tool", serde_json::json!({}));
            let mut metadata = rocode_tool::Metadata::new();
            rocode_tool::append_tool_repair_event_map(
                &mut metadata,
                rocode_tool::tool_repair_event("fallback", "tool", "fail_tool"),
            );
            if let Some(crate::MessagePart {
                part_type: PartType::ToolCall { status, state, .. },
                ..
            }) = assistant.parts.last_mut()
            {
                *status = crate::ToolCallStatus::Error;
                *state = Some(ToolState::Error {
                    input: serde_json::json!({}),
                    error: "boom".to_string(),
                    metadata: Some(metadata),
                    time: crate::ErrorTime { start: 1, end: 2 },
                });
            }
        }
        session.push_message(assistant);

        let diag = session.add_assistant_message();
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "test".to_string(),
            model_id: Some("test".to_string()),
            message: "rejected".to_string(),
        }
        .attach_to_metadata(&mut diag.metadata);

        let quality = build_session_tool_trajectory_quality(&session).expect("should build");
        assert!(quality.score <= 60, "score={}", quality.score);
        assert!(matches!(
            quality.band,
            ToolTrajectoryQualityBand::Degraded | ToolTrajectoryQualityBand::Risky
        ));
        assert!(quality.notes.contains(&"blocked_and_noisy".to_string()));
        assert!(quality
            .notes
            .contains(&"provider_rejection_present".to_string()));
    }

    #[test]
    fn build_session_tool_trajectory_quality_penalizes_sanitizer_heavy_session() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("test"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("test"));

        let mut assistant = SessionMessage::assistant(session.id.clone());
        // Add 3 tool calls, each with a sanitizer-type repair event.
        for i in 0..3 {
            let call_id = format!("call-{}", i);
            assistant.add_tool_call(&call_id, "read", serde_json::json!({}));
            let mut metadata = rocode_tool::Metadata::new();
            rocode_tool::append_tool_repair_event_map(
                &mut metadata,
                rocode_tool::tool_repair_event(
                    RepairKind::SanitizerAssistantMalformedPlaceholder.as_str(),
                    "sanitizer",
                    "read",
                ),
            );
            if let Some(crate::MessagePart {
                part_type: PartType::ToolCall { status, state, .. },
                ..
            }) = assistant.parts.last_mut()
            {
                *status = crate::ToolCallStatus::Completed;
                *state = Some(ToolState::Completed {
                    input: serde_json::json!({}),
                    output: "ok".to_string(),
                    title: "Read".to_string(),
                    metadata,
                    time: crate::CompletedTime {
                        start: 1,
                        end: 2,
                        compacted: None,
                    },
                    attachments: None,
                });
            }
        }
        session.push_message(assistant);

        let quality = build_session_tool_trajectory_quality(&session).expect("should build");
        assert!(quality.sanitizer_event_count >= 3);
        assert!(quality.notes.contains(&"sanitizer_heavy".to_string()));
        assert!(quality.malformed_placeholder_count >= 3);
    }

    #[test]
    fn telemetry_snapshot_roundtrips_tool_trajectory_quality() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.add_tool_call("call-1", "read", serde_json::json!({"file_path":"a.txt"}));
        if let Some(crate::MessagePart {
            part_type: PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = crate::ToolCallStatus::Completed;
            *state = Some(ToolState::Completed {
                input: serde_json::json!({"file_path": "a.txt"}),
                output: "ok".to_string(),
                title: "Read".to_string(),
                metadata: rocode_tool::Metadata::new(),
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }
        session.push_message(assistant);

        let quality = build_session_tool_trajectory_quality(&session).expect("should build");
        let snapshot = SessionTelemetrySnapshot {
            version: rocode_types::SessionTelemetrySnapshotVersion::V6,
            usage: rocode_types::SessionUsage {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                context_tokens: 0,
                total_cost: 0.0,
            },
            stage_summaries: vec![],
            tool_repair_summary: Some(build_session_tool_repair_telemetry(&session).unwrap()),
            memory: None,
            compaction_continuity: None,
            repair_query_snapshot: None,
            tool_trajectory_quality: Some(quality),
            tool_result_governance: None,
            pending_permission_count: 1,
            granted_by_turn_count: 2,
            granted_by_session_count: 3,
            granted_by_matcher_kind: std::collections::BTreeMap::from([
                ("scope_only".to_string(), 4),
                ("structured_family".to_string(), 1),
            ]),
            last_permission_matcher_kind: Some("scope_only".to_string()),
            last_permission_grant_target: Some("Task flow: create task".to_string()),
            last_permission_miss_count: 5,
            pending_steering_count: 0,
            consumed_steering_count: 0,
            last_steering_injected_at: None,
            last_steering_source_session_id: None,
            last_run_status: "completed".to_string(),
            updated_at: 123,
        };

        persist_session_telemetry_snapshot(&mut session, &snapshot).expect("persist");
        let loaded = load_session_telemetry_snapshot(&session).expect("load");
        assert_eq!(
            loaded.version,
            rocode_types::SessionTelemetrySnapshotVersion::V6
        );
        assert_eq!(loaded.pending_permission_count, 1);
        assert_eq!(loaded.granted_by_turn_count, 2);
        assert_eq!(loaded.granted_by_session_count, 3);
        assert_eq!(
            loaded.granted_by_matcher_kind.get("scope_only"),
            Some(&4)
        );
        assert_eq!(
            loaded.last_permission_matcher_kind.as_deref(),
            Some("scope_only")
        );
        assert_eq!(
            loaded.last_permission_grant_target.as_deref(),
            Some("Task flow: create task")
        );
        let q = loaded
            .tool_trajectory_quality
            .expect("quality should survive");
        assert!(q.score >= 90);
        assert_eq!(q.band, ToolTrajectoryQualityBand::Clean);
    }

    #[test]
    fn tool_result_governance_summary_counts_single_batch_and_fallback_markers() {
        let mut session = Session::new("proj", ".");
        let mut tool = SessionMessage::tool(session.id.clone());
        tool.parts.push(crate::MessagePart {
            id: "prt_1".to_string(),
            part_type: PartType::ToolResult {
                tool_call_id: "call-1".to_string(),
                content: "preview".to_string(),
                is_error: false,
                title: None,
                metadata: Some(std::collections::HashMap::from([
                    ("tool_result_governed".to_string(), serde_json::json!(true)),
                    (
                        "tool_result_batch_governed".to_string(),
                        serde_json::json!(true),
                    ),
                ])),
                attachments: None,
            },
            created_at: chrono::Utc::now(),
            message_id: Some(tool.id.clone()),
        });
        tool.parts.push(crate::MessagePart {
            id: "prt_2".to_string(),
            part_type: PartType::ToolResult {
                tool_call_id: "call-2".to_string(),
                content: "fallback".to_string(),
                is_error: false,
                title: None,
                metadata: Some(std::collections::HashMap::from([(
                    "tool_result_transcript_fallback_truncated".to_string(),
                    serde_json::json!(true),
                )])),
                attachments: None,
            },
            created_at: chrono::Utc::now(),
            message_id: Some(tool.id.clone()),
        });
        session.push_message(tool);

        let summary =
            build_session_tool_result_governance_summary(&session).expect("summary should exist");
        assert_eq!(summary.single_result_governed_count, 1);
        assert_eq!(summary.batch_governed_count, 1);
        assert_eq!(summary.transcript_fallback_count, 1);
    }
}
