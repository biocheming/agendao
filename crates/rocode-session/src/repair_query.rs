//! Session and model-level repair query snapshots (P1.3).
//!
//! This module takes the per-call repair events already recorded in
//! `toolRepairTelemetry` metadata and projects them into queryable
//! aggregate/sample snapshots, usable by server APIs and CLI tools.

use std::collections::BTreeMap;

use rocode_tool::structured_repair_events;
use rocode_types::{
    ModelRepairQuerySummary, RepairAggregateRow, RepairKind, RepairOutcomeKind, RepairQuery,
    RepairQueryResponse, RepairSample, SessionRepairQuerySnapshot, SessionRepairQuerySummary,
    ToolRepairCount,
};

use crate::session::Session;
use crate::telemetry::SessionTelemetryModelRef;
use crate::{MessageRole, PartType, ToolCallStatus, ToolState};

pub const SESSION_REPAIR_QUERY_SNAPSHOT_METADATA_KEY: &str = "repair_query_snapshot";

// ── Outcome classification ──────────────────────────────────────────────

pub fn classify_repair_outcome(
    status: &ToolCallStatus,
    state: Option<&ToolState>,
) -> Option<RepairOutcomeKind> {
    match status {
        ToolCallStatus::Completed => Some(RepairOutcomeKind::Success),
        ToolCallStatus::Error => {
            if let Some(ToolState::Error { error, .. }) = state {
                let lower = error.trim().to_ascii_lowercase();
                if lower.contains("permission denied") {
                    return Some(RepairOutcomeKind::PermissionDenied);
                }
                if lower.contains("invalid arguments") || lower.contains("validation error") {
                    return Some(RepairOutcomeKind::InvalidArguments);
                }
                if lower.contains("provider") || lower.contains("rejected") {
                    return Some(RepairOutcomeKind::ProviderRejected);
                }
                if lower.contains("cancel") {
                    return Some(RepairOutcomeKind::Canceled);
                }
            }
            Some(RepairOutcomeKind::ExecutionError)
        }
        _ => None,
    }
}

// ── Model ref helper ────────────────────────────────────────────────────

pub fn session_repair_query_model_ref(session: &Session) -> Option<SessionTelemetryModelRef> {
    crate::telemetry::session_telemetry_model_ref(session)
}

// ── Accumulators ────────────────────────────────────────────────────────

#[derive(Default)]
struct RepairAggregateAccumulator {
    rows: BTreeMap<(String, RepairKind, String), RepairAggregateRow>,
    samples: Vec<RepairSample>,
    total_events: u64,
    strict_would_fail_count: u64,
    injected_count: u64,
}

impl RepairAggregateAccumulator {
    fn record(
        &mut self,
        tool_name: &str,
        repair_kind: RepairKind,
        layer: &str,
        strict_mode_would_fail: bool,
        injected: bool,
        outcome: Option<RepairOutcomeKind>,
        sample: RepairSample,
    ) {
        self.total_events += 1;
        if strict_mode_would_fail {
            self.strict_would_fail_count += 1;
        }
        if injected {
            self.injected_count += 1;
        }

        let key = (tool_name.to_string(), repair_kind, layer.to_string());
        let row = self.rows.entry(key).or_insert_with(|| RepairAggregateRow {
            provider_id: sample.provider_id.clone(),
            model_id: sample.model_id.clone(),
            tool_name: tool_name.to_string(),
            repair_kind,
            layer: layer.to_string(),
            count: 0,
            strict_would_fail_count: 0,
            injected_count: 0,
            success_count: 0,
            error_count: 0,
            latest_at: None,
        });
        row.count += 1;
        if strict_mode_would_fail {
            row.strict_would_fail_count += 1;
        }
        if injected {
            row.injected_count += 1;
        }
        match outcome {
            Some(RepairOutcomeKind::Success) => row.success_count += 1,
            Some(
                RepairOutcomeKind::ExecutionError
                | RepairOutcomeKind::InvalidArguments
                | RepairOutcomeKind::PermissionDenied
                | RepairOutcomeKind::ProviderRejected
                | RepairOutcomeKind::Canceled,
            ) => row.error_count += 1,
            _ => {}
        }
        if sample.created_at > row.latest_at.unwrap_or(0) {
            row.latest_at = Some(sample.created_at);
        }

        self.samples.push(sample);
    }

    fn into_snapshot(self, now: i64) -> SessionRepairQuerySnapshot {
        let mut rows: Vec<RepairAggregateRow> = self.rows.into_values().collect();
        rows.sort_by(|a, b| b.count.cmp(&a.count));

        // Derive summary
        let distinct_tools: std::collections::HashSet<&str> =
            rows.iter().map(|r| r.tool_name.as_str()).collect();
        let distinct_kinds: std::collections::HashSet<RepairKind> =
            rows.iter().map(|r| r.repair_kind).collect();

        let mut kind_counts: BTreeMap<String, u64> = BTreeMap::new();
        let mut tool_counts: BTreeMap<String, u64> = BTreeMap::new();
        for row in &rows {
            *kind_counts
                .entry(row.repair_kind.as_str().to_string())
                .or_default() += row.count;
            *tool_counts.entry(row.tool_name.clone()).or_default() += row.count;
        }
        let top_repairs = top_n(kind_counts, 10);
        let top_tools = top_n(tool_counts, 10);

        SessionRepairQuerySnapshot {
            summary: SessionRepairQuerySummary {
                total_events: self.total_events,
                distinct_tools: distinct_tools.len() as u64,
                distinct_repair_kinds: distinct_kinds.len() as u64,
                strict_would_fail_count: self.strict_would_fail_count,
                injected_count: self.injected_count,
                top_repairs,
                top_tools,
            },
            rows,
            samples: self.samples,
            updated_at: now,
        }
    }
}

fn top_n(counts: BTreeMap<String, u64>, n: usize) -> Vec<ToolRepairCount> {
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries
        .into_iter()
        .take(n)
        .map(|(key, count)| ToolRepairCount { key, count })
        .collect()
}

// ── Builders ─────────────────────────────────────────────────────────────

pub fn build_session_repair_query_snapshot(
    session: &Session,
) -> Option<SessionRepairQuerySnapshot> {
    let model_ref = session_repair_query_model_ref(session);
    let now = chrono::Utc::now().timestamp_millis();
    let mut acc = RepairAggregateAccumulator::default();

    for message in &session.messages {
        if !matches!(message.role, MessageRole::Assistant) {
            continue;
        }

        for part in &message.parts {
            let (tool_call_id, tool_name, status, state) = match &part.part_type {
                PartType::ToolCall {
                    id,
                    name,
                    status,
                    state,
                    ..
                } => (id.clone(), name.clone(), status.clone(), state.clone()),
                _ => continue,
            };

            let outcome = classify_repair_outcome(&status, state.as_ref());
            let metadata = state.as_ref().and_then(|s| match s {
                ToolState::Completed { metadata, .. } => Some(metadata),
                ToolState::Error {
                    metadata: Some(metadata),
                    ..
                } => Some(metadata),
                _ => None,
            });

            let Some(metadata) = metadata else {
                continue;
            };

            let events = structured_repair_events(metadata);
            for event in &events {
                let Some(repair_kind) = event.normalized_kind() else {
                    continue;
                };

                let sample = RepairSample {
                    message_id: Some(message.id.clone()),
                    tool_call_id: Some(tool_call_id.clone()),
                    provider_id: model_ref.as_ref().map(|m| m.provider_id.clone()),
                    model_id: model_ref.as_ref().map(|m| m.model_id.clone()),
                    tool_name: tool_name.clone(),
                    repair_kind,
                    layer: event.layer.clone(),
                    reason: event.reason.clone(),
                    raw_shape: event.raw_shape.clone(),
                    normalized_shape: event.normalized_shape.clone(),
                    strict_mode_would_fail: event.strict_mode_would_fail,
                    injected_into_model_context: event.injected_into_model_context,
                    outcome,
                    created_at: now,
                };

                acc.record(
                    &tool_name,
                    repair_kind,
                    &event.layer,
                    event.strict_mode_would_fail,
                    event.injected_into_model_context,
                    outcome,
                    sample,
                );
            }
        }
    }

    if acc.total_events == 0 {
        return None;
    }

    Some(acc.into_snapshot(now))
}

// ── Query functions ──────────────────────────────────────────────────────

pub fn query_session_repair_snapshot(
    session: &Session,
    query: &RepairQuery,
) -> RepairQueryResponse {
    let snapshot = build_session_repair_query_snapshot(session);
    let Some(snapshot) = snapshot else {
        return RepairQueryResponse {
            summary: None,
            model_summary: None,
            rows: Vec::new(),
            samples: Vec::new(),
            truncated: false,
        };
    };

    let filtered_rows: Vec<RepairAggregateRow> = snapshot
        .rows
        .into_iter()
        .filter(|row| filter_row(row, query))
        .collect();

    let filtered_samples: Vec<RepairSample> = if query.include_samples.unwrap_or(false) {
        snapshot
            .samples
            .into_iter()
            .filter(|sample| filter_sample(sample, query))
            .collect()
    } else {
        Vec::new()
    };

    let limit = query.limit.unwrap_or(100);
    let (rows, truncated) = if filtered_rows.len() > limit {
        (filtered_rows.into_iter().take(limit).collect(), true)
    } else {
        (filtered_rows, false)
    };
    let samples = if filtered_samples.len() > limit {
        filtered_samples.into_iter().take(limit).collect()
    } else {
        filtered_samples
    };

    RepairQueryResponse {
        summary: Some(snapshot.summary),
        model_summary: None,
        rows,
        samples,
        truncated,
    }
}

pub fn query_model_repair_summary<'a>(
    sessions: impl IntoIterator<Item = &'a Session>,
    query: &RepairQuery,
) -> RepairQueryResponse {
    let sessions: Vec<&Session> = sessions.into_iter().collect();
    let mut total_events = 0u64;
    let mut strict_would_fail_count = 0u64;
    let mut session_count = 0u64;
    let mut kind_map: BTreeMap<RepairKind, u64> = BTreeMap::new();
    let mut tool_map: BTreeMap<String, u64> = BTreeMap::new();
    let mut rows: BTreeMap<(String, RepairKind, String), RepairAggregateRow> = BTreeMap::new();
    let mut last_model_ref: Option<SessionTelemetryModelRef> = None;

    for session in &sessions {
        let Some(model_ref) = session_repair_query_model_ref(session) else {
            continue;
        };
        if let Some(ref pid) = query.provider_id {
            if model_ref.provider_id != *pid {
                continue;
            }
        }
        if let Some(ref mid) = query.model_id {
            if model_ref.model_id != *mid {
                continue;
            }
        }

        let Some(snapshot) = build_session_repair_query_snapshot(session) else {
            continue;
        };

        session_count += 1;
        total_events += snapshot.summary.total_events;
        strict_would_fail_count += snapshot.summary.strict_would_fail_count;
        last_model_ref = Some(model_ref);

        for row in snapshot.rows {
            if !filter_row(&row, query) {
                continue;
            }
            let key = (row.tool_name.clone(), row.repair_kind, row.layer.clone());
            let entry = rows.entry(key).or_insert_with(|| RepairAggregateRow {
                provider_id: row.provider_id.clone(),
                model_id: row.model_id.clone(),
                tool_name: row.tool_name.clone(),
                repair_kind: row.repair_kind,
                layer: row.layer.clone(),
                count: 0,
                strict_would_fail_count: 0,
                injected_count: 0,
                success_count: 0,
                error_count: 0,
                latest_at: None,
            });
            entry.count += row.count;
            entry.strict_would_fail_count += row.strict_would_fail_count;
            entry.injected_count += row.injected_count;
            entry.success_count += row.success_count;
            entry.error_count += row.error_count;
            if row.latest_at > entry.latest_at {
                entry.latest_at = row.latest_at;
            }

            *kind_map.entry(row.repair_kind).or_default() += row.count;
            *tool_map.entry(row.tool_name.clone()).or_default() += row.count;
        }
    }

    if session_count == 0 {
        return RepairQueryResponse {
            summary: None,
            model_summary: None,
            rows: Vec::new(),
            samples: Vec::new(),
            truncated: false,
        };
    }

    let mut result_rows: Vec<RepairAggregateRow> = rows.into_values().collect();
    result_rows.sort_by(|a, b| b.count.cmp(&a.count));

    let limit = query.limit.unwrap_or(100);
    let truncated = result_rows.len() > limit;
    if truncated {
        result_rows.truncate(limit);
    }

    let model_ref = last_model_ref;

    RepairQueryResponse {
        summary: None,
        model_summary: Some(ModelRepairQuerySummary {
            provider_id: query
                .provider_id
                .clone()
                .or_else(|| model_ref.as_ref().map(|m| m.provider_id.clone()))
                .unwrap_or_default(),
            model_id: query
                .model_id
                .clone()
                .or_else(|| model_ref.as_ref().map(|m| m.model_id.clone()))
                .unwrap_or_default(),
            session_count,
            total_events,
            strict_would_fail_count,
            top_repairs: top_n(
                kind_map
                    .into_iter()
                    .map(|(k, v)| (k.as_str().to_string(), v))
                    .collect(),
                10,
            ),
            top_tools: top_n(tool_map.into_iter().map(|(k, v)| (k, v)).collect(), 10),
        }),
        rows: result_rows,
        samples: Vec::new(),
        truncated,
    }
}

// ── Filters ──────────────────────────────────────────────────────────────

fn filter_row(row: &RepairAggregateRow, query: &RepairQuery) -> bool {
    if let Some(ref tool) = query.tool_name {
        if row.tool_name != *tool {
            return false;
        }
    }
    if let Some(ref kind) = query.repair_kind {
        if row.repair_kind != *kind {
            return false;
        }
    }
    if let Some(ref layer) = query.layer {
        if row.layer != *layer {
            return false;
        }
    }
    if query.strict_only.unwrap_or(false) && row.strict_would_fail_count == 0 {
        return false;
    }
    true
}

fn filter_sample(sample: &RepairSample, query: &RepairQuery) -> bool {
    if let Some(ref tool) = query.tool_name {
        if sample.tool_name != *tool {
            return false;
        }
    }
    if let Some(ref kind) = query.repair_kind {
        if sample.repair_kind != *kind {
            return false;
        }
    }
    if let Some(ref layer) = query.layer {
        if sample.layer != *layer {
            return false;
        }
    }
    if query.strict_only.unwrap_or(false) && !sample.strict_mode_would_fail {
        return false;
    }
    true
}

// ── Persistence helpers ──────────────────────────────────────────────────

pub fn persist_session_repair_query_snapshot(
    session: &mut Session,
    snapshot: &SessionRepairQuerySnapshot,
) -> anyhow::Result<()> {
    let value = serde_json::to_value(snapshot)?;
    session.insert_metadata(
        SESSION_REPAIR_QUERY_SNAPSHOT_METADATA_KEY.to_string(),
        value,
    );
    Ok(())
}

pub fn load_session_repair_query_snapshot(session: &Session) -> Option<SessionRepairQuerySnapshot> {
    session
        .record()
        .metadata
        .get(SESSION_REPAIR_QUERY_SNAPSHOT_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionMessage;
    use rocode_tool::Metadata;

    fn build_tool_call_part(
        call_id: &str,
        tool_name: &str,
        status: ToolCallStatus,
        state: ToolState,
    ) -> crate::MessagePart {
        crate::MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::ToolCall {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                input: serde_json::json!({}),
                status,
                raw: None,
                state: Some(state),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        }
    }

    fn repair_metadata_with_kind(kind: &str) -> Metadata {
        let mut metadata = Metadata::new();
        rocode_tool::append_tool_repair_event_map(
            &mut metadata,
            rocode_tool::tool_repair_event(kind, "tool", "test_tool"),
        );
        metadata
    }

    #[test]
    fn session_repair_snapshot_groups_by_tool_and_kind() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("mock"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("mock-model"));

        let mut assistant = SessionMessage::assistant(session.id.clone());
        // Two tool calls on the same tool, both with tool_name_repair events
        assistant.parts.push(build_tool_call_part(
            "call-1",
            "echo",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Echo".to_string(),
                metadata: repair_metadata_with_kind("tool_name_repair"),
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        assistant.parts.push(build_tool_call_part(
            "call-2",
            "read",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Read".to_string(),
                metadata: repair_metadata_with_kind("basename_auto_repair"),
                time: crate::CompletedTime {
                    start: 3,
                    end: 4,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        session.push_message(assistant);

        let snapshot =
            build_session_repair_query_snapshot(&session).expect("should build snapshot");
        assert_eq!(snapshot.summary.total_events, 2);
        assert_eq!(snapshot.summary.distinct_tools, 2);
        assert_eq!(snapshot.summary.distinct_repair_kinds, 2);
        assert_eq!(snapshot.rows.len(), 2);
    }

    #[test]
    fn session_repair_snapshot_counts_strict_failures() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());
        let mut metadata = Metadata::new();
        let mut event = rocode_tool::tool_repair_event(
            RepairKind::ArgumentNormalization.as_str(),
            "session_prompt",
            "write",
        );
        event.insert(
            "strict_mode_would_fail".to_string(),
            serde_json::json!(true),
        );
        rocode_tool::append_tool_repair_event_map(&mut metadata, event);

        assistant.parts.push(build_tool_call_part(
            "call-1",
            "write",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Write".to_string(),
                metadata,
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        session.push_message(assistant);

        let snapshot =
            build_session_repair_query_snapshot(&session).expect("should build snapshot");
        assert_eq!(snapshot.summary.strict_would_fail_count, 1);
        assert_eq!(snapshot.rows[0].strict_would_fail_count, 1);
    }

    #[test]
    fn session_repair_query_filters_rows_and_samples() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("mock"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("mock-model"));
        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.parts.push(build_tool_call_part(
            "call-1",
            "echo",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Echo".to_string(),
                metadata: repair_metadata_with_kind("tool_name_repair"),
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        assistant.parts.push(build_tool_call_part(
            "call-2",
            "read",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Read".to_string(),
                metadata: repair_metadata_with_kind("basename_auto_repair"),
                time: crate::CompletedTime {
                    start: 3,
                    end: 4,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        session.push_message(assistant);

        let query = RepairQuery {
            tool_name: Some("echo".to_string()),
            ..Default::default()
        };
        let response = query_session_repair_snapshot(&session, &query);
        assert_eq!(response.rows.len(), 1);
        assert_eq!(response.rows[0].tool_name, "echo");
        assert_eq!(response.rows[0].repair_kind, RepairKind::ToolNameRepair);
    }

    #[test]
    fn model_repair_query_aggregates_across_sessions() {
        let mut s1 = Session::new("proj", ".");
        s1.insert_metadata("model_provider".to_string(), serde_json::json!("mock"));
        s1.insert_metadata("model_id".to_string(), serde_json::json!("mock-model"));
        let mut a1 = SessionMessage::assistant(s1.id.clone());
        a1.parts.push(build_tool_call_part(
            "c1",
            "echo",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Echo".to_string(),
                metadata: repair_metadata_with_kind("tool_name_repair"),
                time: crate::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        s1.push_message(a1);

        let mut s2 = Session::new("proj2", ".");
        s2.insert_metadata("model_provider".to_string(), serde_json::json!("mock"));
        s2.insert_metadata("model_id".to_string(), serde_json::json!("mock-model"));
        let mut a2 = SessionMessage::assistant(s2.id.clone());
        a2.parts.push(build_tool_call_part(
            "c2",
            "echo",
            ToolCallStatus::Completed,
            ToolState::Completed {
                input: serde_json::json!({}),
                output: "ok".to_string(),
                title: "Echo".to_string(),
                metadata: repair_metadata_with_kind("tool_name_repair"),
                time: crate::CompletedTime {
                    start: 3,
                    end: 4,
                    compacted: None,
                },
                attachments: None,
            },
        ));
        s2.push_message(a2);

        let response = query_model_repair_summary(
            [&s1, &s2],
            &RepairQuery {
                provider_id: Some("mock".to_string()),
                model_id: Some("mock-model".to_string()),
                ..Default::default()
            },
        );
        let ms = response.model_summary.expect("should have model summary");
        assert_eq!(ms.session_count, 2);
        assert_eq!(ms.total_events, 2);
    }
}
