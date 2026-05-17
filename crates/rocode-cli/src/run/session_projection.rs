use session_projection_usage::{cli_usage_snapshot_lines, format_token_count};
use session_projection_insights::cli_session_insights_lines;
#[cfg(test)]
use session_projection_layout::cli_render_retained_layout;
use session_projection_events::{
    cli_default_events_query_input, cli_event_lines, cli_events_filter_label,
    cli_events_offset_for_page, cli_events_page_for_offset, cli_events_page_size,
    cli_events_query, cli_events_window_label, cli_parse_events_command_input,
    CliEventsBrowserState, CliEventsCommandInput,
};

fn cli_is_terminal_stage_status(status: Option<&str>) -> bool {
    matches!(status, Some("done" | "blocked" | "cancelled"))
}

fn cli_set_root_server_session(runtime: &mut CliExecutionRuntime, session_id: String) {
    runtime.server_session_id = Some(session_id.clone());
    if let Ok(mut related) = runtime.related_session_ids.lock() {
        related.clear();
        related.insert(session_id);
    }
    if let Ok(mut root) = runtime.root_session_transcript.lock() {
        root.clear();
    }
    if let Ok(mut transcripts) = runtime.attached_session_transcripts.lock() {
        transcripts.clear();
    }
    if let Ok(mut accumulators) = runtime.stream_accumulators.lock() {
        accumulators.clear();
    }
    if let Ok(mut render_states) = runtime.render_states.lock() {
        render_states.clear();
    }
    if let Ok(mut focused) = runtime.focused_session_id.lock() {
        *focused = None;
    }
    if let Ok(mut projection) = runtime.frontend_projection.lock() {
        projection.session_runtime = None;
        projection.stage_summaries.clear();
        projection.telemetry_topology = None;
        projection.events_browser = None;
        projection.token_stats = CliSessionTokenStats::default();
        projection.model_catalog.clear();
        projection.current_model_label = Some(runtime.resolved_model_label.clone());
    }
    cli_set_view_label(runtime, None);
}

fn cli_render_session_block(
    runtime: &CliExecutionRuntime,
    session_id: &str,
    block: &OutputBlock,
    style: &CliStyle,
) -> String {
    let key = cli_canonical_session_id(runtime, session_id);
    let show_thinking = runtime.show_thinking.load(Ordering::SeqCst);
    let accumulators = match runtime.stream_accumulators.lock() {
        Ok(accumulators) => accumulators,
        Err(_) => return render_cli_block_rich(block, style),
    };
    let Some(accumulator) = accumulators.get(&key) else {
        return render_cli_block_rich(block, style);
    };
    let mut render_states = match runtime.render_states.lock() {
        Ok(states) => states,
        Err(_) => return render_cli_block_rich(block, style),
    };
    let state = render_states.entry(key).or_default();
    render_terminal_stream_block_semantic(state, accumulator, block, style, show_thinking)
}

fn cli_canonical_session_id(runtime: &CliExecutionRuntime, session_id: &str) -> String {
    if !session_id.is_empty() {
        return session_id.to_string();
    }

    runtime
        .server_session_id
        .clone()
        .unwrap_or_else(|| "__root__".to_string())
}

fn cli_observe_terminal_stream_block(
    runtime: &CliExecutionRuntime,
    session_id: &str,
    block_id: Option<&str>,
    block: &OutputBlock,
) {
    let key = cli_canonical_session_id(runtime, session_id);
    if let Ok(mut accumulators) = runtime.stream_accumulators.lock() {
        accumulators
            .entry(key)
            .or_insert_with(TerminalStreamAccumulator::new)
            .apply_output_block(block_id, block);
    }
}

fn cli_tracks_related_session(runtime: &CliExecutionRuntime, session_id: &str) -> bool {
    if session_id.is_empty() {
        return true;
    }
    runtime
        .related_session_ids
        .lock()
        .map(|related| related.contains(session_id))
        .unwrap_or(false)
}

fn cli_track_attached_session(
    runtime: &CliExecutionRuntime,
    parent_id: &str,
    attached_id: &str,
) -> bool {
    if parent_id.is_empty() || attached_id.is_empty() {
        return false;
    }
    let mut inserted = false;
    if let Ok(mut related) = runtime.related_session_ids.lock() {
        if related.contains(parent_id) {
            inserted = related.insert(attached_id.to_string());
        }
    }
    if inserted {
        if let Ok(mut transcripts) = runtime.attached_session_transcripts.lock() {
            transcripts.entry(attached_id.to_string()).or_default();
        }
    }
    inserted
}

fn cli_untrack_attached_session(
    runtime: &CliExecutionRuntime,
    parent_id: &str,
    attached_id: &str,
) -> bool {
    if parent_id.is_empty() || attached_id.is_empty() {
        return false;
    }
    runtime
        .related_session_ids
        .lock()
        .map(|mut related| related.contains(parent_id) && related.remove(attached_id))
        .unwrap_or(false)
}

fn cli_cache_attached_session_rendered(
    runtime: &CliExecutionRuntime,
    session_id: &str,
    rendered: &str,
) {
    if let Ok(mut transcripts) = runtime.attached_session_transcripts.lock() {
        transcripts
            .entry(session_id.to_string())
            .or_default()
            .append_rendered(rendered);
    }
}

fn cli_cache_root_session_block(
    runtime: &CliExecutionRuntime,
    block: &OutputBlock,
    style: &CliStyle,
) {
    let rendered = cli_render_session_block(runtime, "", block, style);
    cli_cache_root_session_rendered(runtime, &rendered);
}

fn cli_cache_root_session_rendered(runtime: &CliExecutionRuntime, rendered: &str) {
    if let Ok(mut transcript) = runtime.root_session_transcript.lock() {
        transcript.append_rendered(rendered);
    }
}

fn cli_capture_visible_root_transcript(runtime: &CliExecutionRuntime) {
    let snapshot = runtime
        .frontend_projection
        .lock()
        .ok()
        .map(|projection| projection.transcript.clone());
    if let Some(snapshot) = snapshot {
        if let Ok(mut root) = runtime.root_session_transcript.lock() {
            *root = snapshot;
        }
    }
}

fn cli_focused_session_id(runtime: &CliExecutionRuntime) -> Option<String> {
    runtime
        .focused_session_id
        .lock()
        .ok()
        .and_then(|focused| focused.clone())
}

fn cli_is_root_focused(runtime: &CliExecutionRuntime) -> bool {
    cli_focused_session_id(runtime).is_none()
}

fn cli_replace_visible_transcript(
    runtime: &CliExecutionRuntime,
    transcript: CliRetainedTranscript,
) -> io::Result<()> {
    if let Some(surface) = runtime.terminal_surface.as_ref() {
        surface.replace_transcript(transcript)
    } else {
        if let Ok(mut projection) = runtime.frontend_projection.lock() {
            projection.transcript = transcript;
            projection.scroll_offset = 0;
        }
        Ok(())
    }
}

fn cli_short_session_id(session_id: &str) -> &str {
    &session_id[..session_id.len().min(8)]
}

trait CliStageStatusLabel {
    fn as_ref_label(&self) -> &'static str;
}

impl CliStageStatusLabel for rocode_command::stage_protocol::StageStatus {
    fn as_ref_label(&self) -> &'static str {
        match self {
            rocode_command::stage_protocol::StageStatus::Running => "running",
            rocode_command::stage_protocol::StageStatus::Waiting => "waiting",
            rocode_command::stage_protocol::StageStatus::Done => "done",
            rocode_command::stage_protocol::StageStatus::Cancelled => "cancelled",
            rocode_command::stage_protocol::StageStatus::Cancelling => "cancelling",
            rocode_command::stage_protocol::StageStatus::Blocked => "blocked",
            rocode_command::stage_protocol::StageStatus::Retrying => "retrying",
        }
    }
}

trait CliRunStatusLabel {
    fn as_ref_label(&self) -> &'static str;
}

impl CliRunStatusLabel for crate::api_client::SessionRunStatusKind {
    fn as_ref_label(&self) -> &'static str {
        match self {
            crate::api_client::SessionRunStatusKind::Idle => "idle",
            crate::api_client::SessionRunStatusKind::Running => "running",
            crate::api_client::SessionRunStatusKind::Compacting => "compacting",
            crate::api_client::SessionRunStatusKind::WaitingOnTool => "waiting_on_tool",
            crate::api_client::SessionRunStatusKind::WaitingOnUser => "waiting_on_user",
            crate::api_client::SessionRunStatusKind::Cancelling => "cancelling",
        }
    }
}

fn cli_current_observed_session_id(runtime: &CliExecutionRuntime) -> Option<String> {
    cli_focused_session_id(runtime).or_else(|| runtime.server_session_id.clone())
}

fn cli_set_view_label(runtime: &CliExecutionRuntime, label: Option<String>) {
    if let Ok(mut projection) = runtime.frontend_projection.lock() {
        projection.view_label = label;
    }
    cli_refresh_prompt(runtime);
}

fn cli_ordered_attached_session_ids(runtime: &CliExecutionRuntime) -> Vec<String> {
    let root_session_id = runtime.server_session_id.as_deref();
    let attached_ids = runtime
        .related_session_ids
        .lock()
        .map(|ids| ids.clone())
        .unwrap_or_default();
    let transcripts = runtime
        .attached_session_transcripts
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();

    let mut child_ids = BTreeSet::new();
    for session_id in &attached_ids {
        if root_session_id != Some(session_id.as_str()) {
            child_ids.insert(session_id.clone());
        }
    }
    for session_id in transcripts.keys() {
        child_ids.insert(session_id.clone());
    }

    child_ids.into_iter().collect()
}

fn cli_list_attached_sessions(runtime: &CliExecutionRuntime) {
    let style = CliStyle::detect();
    let attached_ids = runtime
        .related_session_ids
        .lock()
        .map(|ids| ids.clone())
        .unwrap_or_default();
    let transcripts = runtime
        .attached_session_transcripts
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();
    let focused = cli_focused_session_id(runtime);

    let mut lines = Vec::new();
    let child_ids = cli_ordered_attached_session_ids(runtime);
    if child_ids.is_empty() {
        lines.push("No attached sessions have been observed for this run yet.".to_string());
        lines.push("When scheduler agents fork, they will appear here.".to_string());
    } else {
        for session_id in child_ids {
            let transcript = transcripts.get(&session_id);
            let attached = attached_ids.contains(&session_id);
            let focus_marker = if focused.as_deref() == Some(session_id.as_str()) {
                "● focused"
            } else {
                "○ cached"
            };
            let status = if attached { "attached" } else { "detached" };
            let line_count = transcript.map(|item| item.line_count()).unwrap_or(0);
            lines.push(format!(
                "{}  {}  [{} · {} lines]",
                focus_marker, session_id, status, line_count
            ));
            if let Some(summary) = transcript
                .and_then(|item| item.last_line())
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                lines.push(format!("    {}", truncate_text(summary, 88)));
            }
        }
    }

    let footer = match focused {
        Some(attached_id) => format!(
            "/attached next · /attached prev · /attached focus <id> · /attached back · now viewing {}",
            attached_id
        ),
        None => "/attached next · /attached prev · /attached focus <id> · /attached back"
            .to_string(),
    };
    let _ = print_cli_list_on_surface(
        Some(runtime),
        "Attached Sessions",
        Some(&footer),
        &lines,
        &style,
    );
}

fn cli_format_stage_summary_brief(stage: &rocode_command::stage_protocol::StageSummary) -> String {
    let mut parts = vec![format!(
        "{} [{}]",
        stage.stage_name,
        stage.status.as_ref_label()
    )];
    if let (Some(index), Some(total)) = (stage.index, stage.total) {
        parts.push(format!("{}/{}", index, total));
    }
    if let (Some(step), Some(step_total)) = (stage.step, stage.step_total) {
        parts.push(format!("step {}/{}", step, step_total));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        parts.push(format!("waiting {}", waiting_on));
    }
    parts.join(" · ")
}

fn cli_stage_runtime_line(stage: &rocode_command::stage_protocol::StageSummary) -> String {
    let mut parts = vec![format!(
        "{} [{}]",
        stage.stage_name,
        stage.status.as_ref_label()
    )];
    if let (Some(index), Some(total)) = (stage.index, stage.total) {
        parts.push(format!("{}/{}", index, total));
    }
    if let (Some(step), Some(step_total)) = (stage.step, stage.step_total) {
        parts.push(format!("step {}/{}", step, step_total));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        parts.push(format!("waiting {}", waiting_on));
    }
    if let Some(retry_attempt) = stage.retry_attempt {
        parts.push(format!("retry {}", retry_attempt));
    }
    if stage.active_agent_count > 0 {
        parts.push(format!("agents {}", stage.active_agent_count));
    }
    if stage.active_tool_count > 0 {
        parts.push(format!("tools {}", stage.active_tool_count));
    }
    if stage.attached_session_count > 0 {
        parts.push(format!("attached {}", stage.attached_session_count));
    }
    if let Some(budget) = stage.skill_tree_budget {
        let truncated = if stage.skill_tree_truncated.unwrap_or(false) {
            " truncated"
        } else {
            ""
        };
        parts.push(format!(
            "budget {}{}",
            format_token_count(budget),
            truncated
        ));
    }
    if let Some(context_tokens) = stage.context_tokens.or(stage.estimated_context_tokens) {
        parts.push(format!("ctx {}", format_token_count(context_tokens)));
    }
    parts.join(" · ")
}

fn cli_stage_usage_line(stage: &rocode_command::stage_protocol::StageSummary) -> String {
    let mut parts = vec![format!(
        "{} [{}]",
        stage.stage_name,
        stage.status.as_ref_label()
    )];
    if let Some(prompt_tokens) = stage.prompt_tokens {
        parts.push(format!("in {}", format_token_count(prompt_tokens)));
    }
    if let Some(completion_tokens) = stage.completion_tokens {
        parts.push(format!("out {}", format_token_count(completion_tokens)));
    }
    if let Some(reasoning_tokens) = stage.reasoning_tokens.filter(|value| *value > 0) {
        parts.push(format!("reason {}", format_token_count(reasoning_tokens)));
    }
    if let Some(cache_read_tokens) = stage.cache_read_tokens.filter(|value| *value > 0) {
        parts.push(format!("cache-r {}", format_token_count(cache_read_tokens)));
    }
    if let Some(cache_miss_tokens) = stage.cache_miss_tokens.filter(|value| *value > 0) {
        parts.push(format!("cache-m {}", format_token_count(cache_miss_tokens)));
    }
    if let Some(cache_write_tokens) = stage.cache_write_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-w {}",
            format_token_count(cache_write_tokens)
        ));
    }
    if let Some(budget) = stage.skill_tree_budget {
        let truncated = if stage.skill_tree_truncated.unwrap_or(false) {
            " truncated"
        } else {
            ""
        };
        parts.push(format!(
            "budget {}{}",
            format_token_count(budget),
            truncated
        ));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        parts.push(format!("waiting {}", waiting_on));
    }
    if let Some(retry_attempt) = stage.retry_attempt {
        parts.push(format!("retry {}", retry_attempt));
    }
    parts.join(" · ")
}

fn cli_active_stage_summary<'a>(
    telemetry: &'a crate::api_client::SessionTelemetrySnapshot,
) -> Option<&'a rocode_command::stage_protocol::StageSummary> {
    if let Some(active_stage_id) = telemetry.runtime.active_stage_id.as_deref() {
        return telemetry
            .stages
            .iter()
            .find(|stage| stage.stage_id == active_stage_id);
    }

    telemetry.stages.iter().find(|stage| {
        matches!(
            stage.status,
            rocode_command::stage_protocol::StageStatus::Running
                | rocode_command::stage_protocol::StageStatus::Waiting
                | rocode_command::stage_protocol::StageStatus::Retrying
                | rocode_command::stage_protocol::StageStatus::Blocked
                | rocode_command::stage_protocol::StageStatus::Cancelling
        )
    })
}

fn cli_runtime_snapshot_lines(
    session_id: &str,
    telemetry: &crate::api_client::SessionTelemetrySnapshot,
) -> Vec<String> {
    let runtime = &telemetry.runtime;
    let topology = &telemetry.topology;
    let mut lines = vec![
        format!("Session: {}", session_id),
        format!("Run status: {}", runtime.run_status.as_ref_label()),
        format!(
            "Topology: active {} · running {} · waiting {} · cancelling {} · retry {} · done {}",
            topology.active_count,
            topology.running_count,
            topology.waiting_count,
            topology.cancelling_count,
            topology.retry_count,
            topology.done_count
        ),
        format!("Stages observed: {}", telemetry.stages.len()),
    ];

    if let Some(current_message_id) = runtime.current_message_id.as_deref() {
        lines.push(format!("Current message: {}", current_message_id));
    }

    if let Some(stage) = cli_active_stage_summary(telemetry) {
        lines.push(String::new());
        lines.push(format!(
            "Active stage: {}",
            cli_format_stage_summary_brief(stage)
        ));
        if let Some(last_event) = stage.last_event.as_deref() {
            lines.push(format!("Last event: {}", last_event));
        }
        if let Some(activity) = stage.activity.as_deref().filter(|value| !value.is_empty()) {
            lines.push(format!("Activity: {}", activity.replace('\n', " · ")));
        }
        if let Some(focus) = stage.focus.as_deref() {
            lines.push(format!("Focus: {}", focus));
        }
        if let Some(context_tokens) = stage.context_tokens.or(stage.estimated_context_tokens) {
            lines.push(format!("Context: {}", format_token_count(context_tokens)));
        }
        if let Some(strategy) = stage.skill_tree_truncation_strategy.as_deref() {
            let truncated = if stage.skill_tree_truncated.unwrap_or(false) {
                "yes"
            } else {
                "no"
            };
            lines.push(format!(
                "Skill tree truncation: {} ({})",
                strategy, truncated
            ));
        }
    }

    if !telemetry.stages.is_empty() {
        lines.push(String::new());
        lines.push(format!("Stage summaries ({})", telemetry.stages.len()));
        for stage in &telemetry.stages {
            lines.push(format!("  {}", cli_stage_runtime_line(stage)));
            if let Some(last_event) = stage.last_event.as_deref() {
                lines.push(format!("    last-event {}", last_event));
            }
            if let Some(activity) = stage.activity.as_deref().filter(|value| !value.is_empty()) {
                lines.push(format!(
                    "    activity {}",
                    activity.replace('\n', " · ")
                ));
            }
            if let Some(focus) = stage.focus.as_deref() {
                lines.push(format!("    focus {}", focus));
            }
        }
    }

    lines.push(String::new());
    if runtime.active_tools.is_empty() {
        lines.push("Active tools: none".to_string());
    } else {
        lines.push(format!("Active tools ({})", runtime.active_tools.len()));
        for tool in &runtime.active_tools {
            lines.push(format!("  {} · {}", tool.tool_name, tool.tool_call_id));
        }
    }

    if let Some(question) = runtime.pending_question.as_ref() {
        lines.push(String::new());
        lines.push(format!("Pending question: {}", question.request_id));
    }
    if let Some(permission) = runtime.pending_permission.as_ref() {
        lines.push(format!("Pending permission: {}", permission.permission_id));
    }
    if telemetry.granted_by_turn_count + telemetry.granted_by_session_count > 0
        || telemetry.last_permission_miss_count > 0
        || !telemetry.granted_by_matcher_kind.is_empty()
    {
        lines.push(String::new());
        lines.push("Permission Authority:".to_string());
        lines.push(format!(
            "  Turn grants: {} · Session grants: {} · Pending: {} · Misses: {}",
            telemetry.granted_by_turn_count,
            telemetry.granted_by_session_count,
            telemetry.pending_permission_count,
            telemetry.last_permission_miss_count,
        ));
        if let Some(ref kind) = telemetry.last_permission_matcher_kind {
            lines.push(format!("  Last grant: {kind}"));
        }
        if !telemetry.granted_by_matcher_kind.is_empty() {
            let matchers: Vec<String> = telemetry
                .granted_by_matcher_kind
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            lines.push(format!("  Matchers: {}", matchers.join(", ")));
        }
    }

    if runtime.attached_sessions.is_empty() {
        lines.push(String::new());
        lines.push("Attached sessions: none".to_string());
    } else {
        lines.push(String::new());
        lines.push(format!(
            "Attached sessions ({})",
            runtime.attached_sessions.len()
        ));
        for child in &runtime.attached_sessions {
            let kind = child
                .context_kind
                .map(cli_session_context_kind_label)
                .unwrap_or("attached session");
            lines.push(format!(
                "  {} · {} ← {}",
                kind, child.attached_id, child.parent_id
            ));
        }
    }

    if let Some(memory) = telemetry.memory.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Memory runtime: {} · {}",
            memory.workspace_mode,
            truncate_text(&memory.workspace_key, 72)
        ));
        lines.push(format!(
            "  Frozen snapshot: {} items{}",
            memory.frozen_snapshot_items,
            cli_optional_generated_at(memory.frozen_snapshot_generated_at)
        ));
        lines.push(format!(
            "  Last prefetch: {} items{}",
            memory.last_prefetch_items,
            cli_optional_generated_at(memory.last_prefetch_generated_at)
        ));
        lines.push(format!(
            "  Session records: candidate {} · validated {} · rejected {}",
            memory.candidate_count, memory.validated_count, memory.rejected_count
        ));
        lines.push(format!(
            "  Validation pressure: warnings {} · methodology {} · skill targets {}",
            memory.warning_count,
            memory.methodology_candidate_count,
            memory.derived_skill_candidate_count
        ));
        lines.push(format!(
            "  Skill linkage: linked {} · feedback lessons {}",
            memory.linked_skill_count, memory.skill_feedback_lesson_count
        ));
        lines.push(format!(
            "  Retrieval: runs {} · hits {} · used {}",
            memory.retrieval_run_count, memory.retrieval_hit_count, memory.retrieval_use_count
        ));
        if let Some(query) = memory.last_prefetch_query.as_deref() {
            lines.push(format!("  Prefetch query: {}", truncate_text(query, 120)));
        }
        if let Some(run) = memory.latest_consolidation_run.as_ref() {
            lines.push(format!(
                "  Latest consolidation: {} · merged {} · promoted {} · conflicts {}",
                run.run_id, run.merged_count, run.promoted_count, run.conflict_count
            ));
        }
        if memory.recent_rule_hits.is_empty() {
            lines.push("  Recent rule hits: none".to_string());
        } else {
            lines.push(format!(
                "  Recent rule hits ({})",
                memory.recent_rule_hits.len()
            ));
            for hit in &memory.recent_rule_hits {
                let detail = hit.detail.as_deref().unwrap_or("no detail");
                let memory_ref = hit
                    .memory_id
                    .as_ref()
                    .map(|id| id.0.as_str())
                    .unwrap_or("workspace");
                lines.push(format!(
                    "    {} · {} · {}",
                    hit.hit_kind,
                    memory_ref,
                    truncate_text(detail, 100)
                ));
            }
        }
    }

    lines
}

fn cli_session_context_kind_label(kind: crate::api_client::SessionContextKind) -> &'static str {
    kind.label()
}

fn cli_session_handoff_mode_label(mode: rocode_types::SessionHandoffMode) -> &'static str {
    match mode {
        rocode_types::SessionHandoffMode::SelfContinuity => "self continuity",
        rocode_types::SessionHandoffMode::BoundedHandoff => "bounded handoff",
        rocode_types::SessionHandoffMode::StageOutputSink => "stage output sink",
        rocode_types::SessionHandoffMode::FullHistoryFork => "full-history fork",
    }
}

fn cli_context_closure_prefix_status_label(
    prefix: &rocode_types::SessionPrefixStabilityContract,
) -> &'static str {
    prefix.status_label()
}

fn cli_context_closure_boundary_status_label(
    boundary: &rocode_types::SessionCompactionBoundaryContract,
) -> &'static str {
    boundary.status_label()
}

fn cli_context_closure_cache_status_label(
    cache: &rocode_types::SessionCacheExplainabilityContract,
) -> &'static str {
    cache.status_label()
}

pub(crate) fn cli_context_closure_cache_diagnostic_label(
    contract: Option<&rocode_types::SessionContextClosureContract>,
) -> Option<String> {
    contract?.coarse_diagnostic_label()
}

fn cli_context_closure_isolation_status_label(
    isolation: &rocode_types::SessionChildHistoryIsolationContract,
) -> &'static str {
    isolation.status_label()
}

pub(crate) fn cli_cache_evidence_status_label(
    summary: &rocode_provider::cache::CacheEvidenceSummary,
) -> Option<String> {
    if !summary.should_surface() {
        return None;
    }

    let has_cause = summary
        .primary_cause
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    Some(
        if has_cause {
            rocode_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: true,
                source: rocode_types::SessionCacheExplainabilitySource::None,
                severity: None,
                explanation: None,
            }
            .status_label()
        } else {
            rocode_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: false,
                source: rocode_types::SessionCacheExplainabilitySource::None,
                severity: None,
                explanation: None,
            }
            .status_label()
        }
        .to_string(),
    )
}

fn cli_context_closure_evidence_impact_label(
    severity: rocode_types::SessionCacheSeverity,
) -> &'static str {
    severity.label()
}

fn cli_context_closure_evidence_source_label(
    source: rocode_types::SessionCacheExplainabilitySource,
) -> &'static str {
    source.label()
}

fn cli_context_closure_evidence_detail_label(detail: &str) -> String {
    let normalized = detail.trim();
    if normalized.is_empty() {
        return "--".to_string();
    }

    if normalized.contains("boundary recorded · prefix changed") {
        return "boundary recorded · prefix changed".to_string();
    }
    if normalized.contains("prefix changed before the stable boundary") {
        return "prefix changed before the stable boundary".to_string();
    }
    if normalized.contains("tool surface changed") {
        return "tool surface changed".to_string();
    }
    if normalized.contains("request shape changed") {
        return "request shape changed".to_string();
    }
    if normalized.contains("systemHash changed: system prompt changed") {
        return "system prompt changed".to_string();
    }
    if normalized.contains("family changed: protocol family changed") {
        return "request family changed".to_string();
    }
    if normalized.contains("surface changed:") {
        let field = normalized
            .split_once(':')
            .map(|(_, suffix)| suffix.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("runtime fields");
        return format!("surface changed · {}", field);
    }

    normalized.to_string()
}

fn cli_prompt_surface_evidence_label(fields: &[String]) -> String {
    if fields.is_empty() {
        "surface changed".to_string()
    } else {
        format!("surface {}", fields.join(", "))
    }
}

fn cli_yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn cli_optional_generated_at(ts: Option<i64>) -> String {
    ts.and_then(|value| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value))
        .map(|value| value.with_timezone(&chrono::Local))
        .map(|value| format!(" @ {}", value.format("%Y-%m-%d %H:%M:%S")))
        .unwrap_or_default()
}

async fn cli_print_runtime_snapshot(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
) {
    let Some(session_id) = cli_current_observed_session_id(runtime) else {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning(
                "No active session available for /runtime.",
            )),
            style,
        );
        return;
    };

    match api_client.get_session_telemetry(&session_id).await {
        Ok(telemetry) => {
            let lines = cli_runtime_snapshot_lines(&session_id, &telemetry);
            let footer =
                "Source: /session/{id}/telemetry · use /events [stage=<id>] for raw event log";
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Runtime Telemetry",
                Some(footer),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load runtime telemetry: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_usage_snapshot(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
) {
    let Some(session_id) = cli_current_observed_session_id(runtime) else {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning(
                "No active session available for /usage.",
            )),
            style,
        );
        return;
    };

    match api_client.get_session_telemetry(&session_id).await {
        Ok(telemetry) => {
            let projection = runtime
                .frontend_projection
                .lock()
                .ok()
                .map(|projection| projection.clone());
            let lines = cli_usage_snapshot_lines(&session_id, &telemetry, projection.as_ref());
            let footer =
                "Source: /session/{id}/telemetry · stage totals come from authority summaries";
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Session Usage",
                Some(footer),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load session usage: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_session_insights(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
) {
    let Some(session_id) = cli_current_observed_session_id(runtime) else {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning(
                "No active session available for /insights.",
            )),
            style,
        );
        return;
    };

    match api_client.get_session_insights(&session_id).await {
        Ok(insights) => {
            let lines = cli_session_insights_lines(&session_id, &insights);
            let footer = "Source: /session/{id}/insights · includes persisted telemetry, multimodal explain, memory explain, and effective policy";
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Session Insights",
                Some(footer),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load session insights: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_session_events(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    raw_filter: Option<&str>,
) {
    let Some(session_id) = cli_current_observed_session_id(runtime) else {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning(
                "No active session available for /events.",
            )),
            style,
        );
        return;
    };

    let command = cli_parse_events_command_input(raw_filter);
    let remembered = runtime
        .frontend_projection
        .lock()
        .ok()
        .and_then(|projection| projection.events_browser.clone())
        .filter(|state| state.session_id == session_id);

    let (filter, offset, preserve_previous_state, empty_page_message) = match command {
        CliEventsCommandInput::ShowCurrent => {
            if let Some(state) = remembered.as_ref() {
                (state.filter.clone(), state.offset, false, None)
            } else {
                (cli_default_events_query_input(), 0, false, None)
            }
        }
        CliEventsCommandInput::ShowFiltered { filter, page } => (
            filter.clone(),
            cli_events_offset_for_page(&filter, page),
            false,
            (page > 1).then(|| {
                format!(
                    "Requested page {} has no events for the current filter. Use /events first, /events prev, or reduce page.",
                    page
                )
            }),
        ),
        CliEventsCommandInput::JumpPage(page) => {
            let filter = remembered
                .as_ref()
                .map(|state| state.filter.clone())
                .unwrap_or_else(cli_default_events_query_input);
            (
                filter.clone(),
                cli_events_offset_for_page(&filter, page),
                false,
                (page > 1).then(|| {
                    format!(
                        "Requested page {} has no events for the current filter. Use /events first, /events prev, or change filters.",
                        page
                    )
                }),
            )
        }
        CliEventsCommandInput::NextPage => {
            if let Some(state) = remembered.as_ref() {
                let next_offset = state.offset.saturating_add(cli_events_page_size(&state.filter));
                (state.filter.clone(), next_offset, true, None)
            } else {
                (cli_default_events_query_input(), 0, false, None)
            }
        }
        CliEventsCommandInput::PreviousPage => {
            if let Some(state) = remembered.as_ref() {
                let step = cli_events_page_size(&state.filter);
                (
                    state.filter.clone(),
                    state.offset.saturating_sub(step),
                    false,
                    None,
                )
            } else {
                (cli_default_events_query_input(), 0, false, None)
            }
        }
        CliEventsCommandInput::FirstPage => {
            if let Some(state) = remembered.as_ref() {
                (state.filter.clone(), 0, false, None)
            } else {
                (cli_default_events_query_input(), 0, false, None)
            }
        }
        CliEventsCommandInput::Clear => (cli_default_events_query_input(), 0, false, None),
    };

    let query = cli_events_query(&filter, offset);
    match api_client.get_session_events(&session_id, &query).await {
        Ok(events) => {
            if events.is_empty() && offset > 0 {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(empty_page_message.unwrap_or_else(
                        || {
                            if preserve_previous_state {
                                "No more events for the current filter. Use /events prev or change filters."
                                    .to_string()
                            } else {
                                "That event page is empty for the current filter. Use /events first, /events prev, or adjust filters."
                                    .to_string()
                            }
                        },
                    ))),
                    style,
                );
                return;
            }

            let page_size = cli_events_page_size(&filter);
            let page_index = cli_events_page_for_offset(&filter, offset);
            let can_go_prev = offset > 0;
            let can_go_next = events.len() >= page_size;
            let mut lines = vec![format!("Session: {}", session_id)];
            lines.extend(cli_event_lines(&events, style));
            let footer = format!(
                "Page {} · {} · {} · {}{}{}{}{}",
                page_index,
                cli_events_window_label(offset, events.len()),
                cli_events_filter_label(&filter),
                if can_go_prev {
                    "/events prev"
                } else {
                    "first page"
                },
                if can_go_next { " · /events next" } else { "" },
                " · /events page <n>",
                " · /events clear",
                if page_index > 1 {
                    " · /events first"
                } else {
                    ""
                }
            );

            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.events_browser = Some(CliEventsBrowserState {
                    session_id: session_id.clone(),
                    filter: filter.clone(),
                    offset,
                });
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Session Events",
                Some(&footer),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load session events: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_list(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    search: Option<&str>,
) {
    let query = crate::api_client::MemoryListQuery {
        search: search
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        limit: Some(50),
        source_session_id: cli_current_observed_session_id(runtime),
        ..Default::default()
    };

    let response_result = if query.search.is_some() {
        api_client.search_memory(Some(&query)).await
    } else {
        api_client.list_memory(Some(&query)).await
    };

    match response_result {
        Ok(response) => {
            let mut lines = Vec::new();
            if let Some(session_id) = query.source_session_id.as_deref() {
                lines.push(format!("Session filter: {}", session_id));
            } else {
                lines.push("Scope: current workspace authority".to_string());
            }
            if let Some(search) = query.search.as_deref() {
                lines.push(format!("Search: {}", search));
            }
            lines.push(format!("Total: {}", response.items.len()));
            lines.push(String::new());
            if response.items.is_empty() {
                lines.push(style.dim("No memory records matched the current query."));
            } else {
                for item in &response.items {
                    lines.push(format!(
                        "{} · {:?} · {:?} · {:?}",
                        item.id.0, item.kind, item.status, item.validation_status
                    ));
                    if item.linked_skill_name.is_some() || item.derived_skill_name.is_some() {
                        lines.push(format!(
                            "  skills: linked={} · target={}",
                            item.linked_skill_name.as_deref().unwrap_or("--"),
                            item.derived_skill_name.as_deref().unwrap_or("--")
                        ));
                    }
                    lines.push(format!("  {}", item.title));
                    lines.push(format!("  {}", item.summary));
                }
            }
            let footer = format!(
                "Source: {} · search fields: {} · detail: /memory show <id>",
                if query.search.is_some() {
                    "/memory/search"
                } else {
                    "/memory/list"
                },
                response.contract.search_fields.join(", ")
            );
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Records",
                Some(&footer),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to list memory records: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_retrieval_preview(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    query_text: Option<&str>,
) {
    let query = crate::api_client::MemoryRetrievalQuery {
        query: query_text
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        stage: None,
        limit: Some(6),
        kinds: Vec::new(),
        scopes: Vec::new(),
        session_id: cli_current_observed_session_id(runtime),
    };

    match api_client.get_memory_retrieval_preview(&query).await {
        Ok(response) => {
            let packet = response.packet;
            let mut lines = Vec::new();
            if let Some(session_id) = query.session_id.as_deref() {
                lines.push(format!("Session filter: {}", session_id));
            }
            if let Some(search) = packet.query.as_deref() {
                lines.push(format!("Query: {}", search));
            }
            lines.push(format!(
                "Items: {} · Budget: {}",
                packet.items.len(),
                packet
                    .budget_limit
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "--".to_string())
            ));
            lines.push(format!("Contract: {}", response.contract.note));
            lines.push(String::new());
            if packet.items.is_empty() {
                lines.push("No memory records would be injected for this turn.".to_string());
            } else {
                for item in packet.items {
                    lines.push(format!(
                        "{} · {:?} · {:?}",
                        item.card.id.0, item.card.kind, item.card.validation_status
                    ));
                    lines.push(format!("  {}", item.card.title));
                    lines.push(format!("  why: {}", item.why_recalled));
                    lines.push(format!("  summary: {}", item.card.summary));
                    if let Some(evidence) = item.evidence_summary.as_deref() {
                        lines.push(format!("  evidence: {}", evidence));
                    }
                }
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Retrieval Preview",
                Some("Source: /memory/retrieval-preview"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory retrieval preview: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_detail(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    record_id: &str,
) {
    match api_client.get_memory_detail(record_id).await {
        Ok(detail) => {
            let record = detail.record;
            let mut lines = vec![
                format!("Id: {}", record.id.0),
                format!(
                    "Kind: {:?} · Scope: {:?} · Status: {:?} · Validation: {:?}",
                    record.kind, record.scope, record.status, record.validation_status
                ),
                format!("Title: {}", record.title),
                format!("Summary: {}", record.summary),
            ];
            if !record.trigger_conditions.is_empty() {
                lines.push("Triggers:".to_string());
                lines.extend(
                    record
                        .trigger_conditions
                        .iter()
                        .map(|value| format!("  - {}", value)),
                );
            }
            if !record.normalized_facts.is_empty() {
                lines.push("Facts:".to_string());
                lines.extend(
                    record
                        .normalized_facts
                        .iter()
                        .map(|value| format!("  - {}", value)),
                );
            }
            if !record.boundaries.is_empty() {
                lines.push("Boundaries:".to_string());
                lines.extend(
                    record
                        .boundaries
                        .iter()
                        .map(|value| format!("  - {}", value)),
                );
            }
            if !record.evidence_refs.is_empty() {
                lines.push("Evidence:".to_string());
                lines.extend(record.evidence_refs.iter().map(|evidence| {
                    format!(
                        "  - session={} message={} tool={} stage={} {}",
                        evidence.session_id.as_deref().unwrap_or("--"),
                        evidence.message_id.as_deref().unwrap_or("--"),
                        evidence.tool_call_id.as_deref().unwrap_or("--"),
                        evidence.stage_id.as_deref().unwrap_or("--"),
                        evidence.note.as_deref().unwrap_or("")
                    )
                }));
            }
            let footer = "Source: /memory/{id} · validation: /memory validation <id> · conflicts: /memory conflicts <id>";
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Detail",
                Some(footer),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory detail `{}`: {}",
                    record_id, error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_validation_report(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    record_id: &str,
) {
    match api_client.get_memory_validation_report(record_id).await {
        Ok(response) => {
            let mut lines = vec![format!("Record: {}", response.record_id.0)];
            if let Some(report) = response.latest {
                lines.push(format!("Status: {:?}", report.status));
                lines.push(format!("Checked at: {}", report.checked_at));
                if report.issues.is_empty() {
                    lines.push("Issues: none".to_string());
                } else {
                    lines.push("Issues:".to_string());
                    lines.extend(
                        report
                            .issues
                            .into_iter()
                            .map(|issue| format!("  - {}", issue)),
                    );
                }
            } else {
                lines.push("No validation report recorded yet.".to_string());
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Validation",
                Some("Source: /memory/{id}/validation-report"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory validation report `{}`: {}",
                    record_id, error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_config_validation(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
) {
    match api_client.get_config_validation().await {
        Ok(snapshot) => {
            let lines = crate::config_cmd::config_validation_lines(&snapshot);
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Config Validation",
                Some("Source: /config/validation"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load config validation snapshot: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_conflicts(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    record_id: &str,
) {
    match api_client.get_memory_conflicts(record_id).await {
        Ok(response) => {
            let mut lines = vec![format!("Record: {}", response.record_id.0)];
            if response.conflicts.is_empty() {
                lines.push("No duplicate or contradiction conflicts recorded.".to_string());
            } else {
                for conflict in response.conflicts {
                    lines.push(format!(
                        "{} · {} · other {}",
                        conflict.id, conflict.conflict_kind, conflict.other_record_id.0
                    ));
                    lines.push(format!("  {}", conflict.detail));
                }
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Conflicts",
                Some("Source: /memory/{id}/conflicts"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory conflicts `{}`: {}",
                    record_id, error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_rule_packs(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
) {
    match api_client.list_memory_rule_packs().await {
        Ok(response) => {
            let mut lines = Vec::new();
            if response.items.is_empty() {
                lines.push("No memory rule packs registered.".to_string());
            } else {
                for pack in response.items {
                    lines.push(format!(
                        "{} · {:?} · version {}",
                        pack.id, pack.rule_pack_kind, pack.version
                    ));
                    if pack.rules.is_empty() {
                        lines.push("  rules: none".to_string());
                    } else {
                        for rule in pack.rules {
                            lines.push(format!("  - {}: {}", rule.id, rule.description));
                        }
                    }
                }
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Rule Packs",
                Some("Source: /memory/rule-packs"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory rule packs: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_rule_hits(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    raw_query: Option<&str>,
) {
    let parsed = rocode_command::interactive::parse_memory_rule_hit_query(raw_query);
    let query = crate::api_client::MemoryRuleHitQuery {
        run_id: parsed.run_id.clone(),
        memory_id: parsed.record_id.map(rocode_types::MemoryRecordId),
        limit: parsed.limit.map(|value| value as u32),
    };

    match api_client.list_memory_rule_hits(Some(&query)).await {
        Ok(response) => {
            let mut lines = Vec::new();
            if let Some(run_id) = query.run_id.as_deref() {
                lines.push(format!("Run filter: {}", run_id));
            }
            if let Some(memory_id) = query.memory_id.as_ref() {
                lines.push(format!("Record filter: {}", memory_id.0));
            }
            lines.push(format!("Total: {}", response.items.len()));
            lines.push(String::new());
            if response.items.is_empty() {
                lines.push("No matching memory rule hits were found.".to_string());
            } else {
                for hit in response.items {
                    lines.push(format!(
                        "{} · {} · run={} · memory={}",
                        hit.id,
                        hit.hit_kind,
                        hit.run_id.as_deref().unwrap_or("--"),
                        hit.memory_id
                            .as_ref()
                            .map(|id| id.0.as_str())
                            .unwrap_or("--")
                    ));
                    if let Some(pack_id) = hit.rule_pack_id.as_deref() {
                        lines.push(format!("  pack: {}", pack_id));
                    }
                    if let Some(detail) = hit.detail.as_deref() {
                        lines.push(format!("  {}", detail));
                    }
                }
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Rule Hits",
                Some("Source: /memory/rule-hits"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory rule hits: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_print_memory_consolidation_runs(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
) {
    match api_client
        .list_memory_consolidation_runs(Some(&crate::api_client::MemoryConsolidationRunQuery {
            limit: Some(20),
        }))
        .await
    {
        Ok(response) => {
            let mut lines = Vec::new();
            if response.items.is_empty() {
                lines.push("No consolidation runs recorded yet.".to_string());
            } else {
                for run in response.items {
                    lines.push(format!(
                        "{} · merged {} · promoted {} · conflicts {}",
                        run.run_id, run.merged_count, run.promoted_count, run.conflict_count
                    ));
                    lines.push(format!(
                        "  started={} finished={}",
                        run.started_at,
                        run.finished_at
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "--".to_string())
                    ));
                }
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Consolidation Runs",
                Some("Source: /memory/consolidation/runs"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to load memory consolidation runs: {}",
                    error
                ))),
                style,
            );
        }
    }
}

async fn cli_run_memory_consolidation(
    runtime: &CliExecutionRuntime,
    api_client: &CliApiClient,
    style: &CliStyle,
    raw_request: Option<&str>,
) {
    let parsed = rocode_command::interactive::parse_memory_consolidation_request(raw_request);
    let request = crate::api_client::MemoryConsolidationRequest {
        limit: parsed.limit.map(|value| value as u32),
        include_candidates: parsed.include_candidates,
    };

    match api_client.run_memory_consolidation(&request).await {
        Ok(response) => {
            let mut lines = vec![
                format!("Run: {}", response.run.run_id),
                format!(
                    "Merged: {} · Promoted: {} · Conflicts: {}",
                    response.run.merged_count,
                    response.run.promoted_count,
                    response.run.conflict_count
                ),
            ];
            if !response.promoted_record_ids.is_empty() {
                lines.push("Promoted records:".to_string());
                lines.extend(
                    response
                        .promoted_record_ids
                        .iter()
                        .map(|id| format!("  - {}", id.0)),
                );
            }
            if !response.reflection_notes.is_empty() {
                lines.push("Reflection:".to_string());
                lines.extend(
                    response
                        .reflection_notes
                        .iter()
                        .map(|note| format!("  - {}", note)),
                );
            }
            if !response.rule_hits.is_empty() {
                lines.push("Rule hits:".to_string());
                lines.extend(
                    response
                        .rule_hits
                        .iter()
                        .take(8)
                        .map(|hit| format!("  - {} ({})", hit.hit_kind, hit.id)),
                );
            }
            let _ = print_cli_list_on_surface(
                Some(runtime),
                "Memory Consolidation",
                Some("Source: POST /memory/consolidate · inspect: /memory runs · /memory hits"),
                &lines,
                style,
            );
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to run memory consolidation: {}",
                    error
                ))),
                style,
            );
        }
    }
}

fn cli_focus_attached_session(
    runtime: &CliExecutionRuntime,
    requested_id: &str,
) -> io::Result<bool> {
    let requested_id = requested_id.trim();
    if requested_id.is_empty() {
        return Ok(false);
    }

    let transcripts = runtime
        .attached_session_transcripts
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();
    let related = runtime
        .related_session_ids
        .lock()
        .map(|ids| ids.clone())
        .unwrap_or_default();
    let root_session_id = runtime.server_session_id.as_deref();

    let mut candidates = BTreeSet::new();
    for session_id in related {
        if root_session_id != Some(session_id.as_str()) {
            candidates.insert(session_id);
        }
    }
    for session_id in transcripts.keys() {
        candidates.insert(session_id.clone());
    }

    let target = if candidates.contains(requested_id) {
        Some(requested_id.to_string())
    } else {
        let mut prefix_matches = candidates
            .into_iter()
            .filter(|candidate| candidate.starts_with(requested_id))
            .collect::<Vec<_>>();
        if prefix_matches.len() == 1 {
            prefix_matches.pop()
        } else {
            None
        }
    };

    let Some(target_id) = target else {
        return Ok(false);
    };

    let Some(transcript) = transcripts.get(&target_id).cloned() else {
        return Ok(false);
    };

    if cli_is_root_focused(runtime) {
        cli_capture_visible_root_transcript(runtime);
    }
    if let Ok(mut focused) = runtime.focused_session_id.lock() {
        *focused = Some(target_id.clone());
    }
    cli_set_view_label(
        runtime,
        Some(format!(
            "view attached {}",
            cli_short_session_id(&target_id)
        )),
    );
    cli_replace_visible_transcript(runtime, transcript)?;
    Ok(true)
}

fn cli_cycle_attached_session(
    runtime: &CliExecutionRuntime,
    forward: bool,
) -> io::Result<Option<(String, usize, usize)>> {
    let child_ids = cli_ordered_attached_session_ids(runtime);
    if child_ids.is_empty() {
        return Ok(None);
    }

    let focused = cli_focused_session_id(runtime);
    let next_index = match focused
        .as_deref()
        .and_then(|current| child_ids.iter().position(|id| id == current))
    {
        Some(index) if forward => (index + 1) % child_ids.len(),
        Some(index) => (index + child_ids.len() - 1) % child_ids.len(),
        None if forward => 0,
        None => child_ids.len() - 1,
    };
    let target_id = child_ids[next_index].clone();
    if !cli_focus_attached_session(runtime, &target_id)? {
        return Ok(None);
    }
    Ok(Some((target_id, next_index + 1, child_ids.len())))
}

fn cli_focus_root_session(runtime: &CliExecutionRuntime) -> io::Result<bool> {
    if cli_is_root_focused(runtime) {
        return Ok(false);
    }
    let transcript = runtime
        .root_session_transcript
        .lock()
        .map(|item| item.clone())
        .unwrap_or_default();
    if let Ok(mut focused) = runtime.focused_session_id.lock() {
        *focused = None;
    }
    cli_set_view_label(runtime, None);
    cli_replace_visible_transcript(runtime, transcript)?;
    Ok(true)
}

fn cli_session_update_requires_refresh(source: Option<&str>) -> bool {
    matches!(
        source,
        Some(
            "prompt.final"
                | "stream.final"
                | "prompt.completed"
                | "session.title.set"
                | "prompt.done"
                | "prompt.scheduler.stage.step"
                | "prompt.scheduler.stage.usage"
                | "prompt.scheduler.stage.tool.start"
                | "prompt.scheduler.stage.tool.end"
                | "prompt.scheduler.stage.step_checkpoint.compact"
                | "prompt.scheduler.stage.step_checkpoint.compacted"
                | "prompt.scheduler.stage.step_checkpoint.blocked"
                | "prompt.scheduler.snapshot"
        )
    )
}

#[cfg(test)]
mod session_update_refresh_tests {
    use super::cli_session_update_requires_refresh;

    #[test]
    fn cli_refreshes_for_scheduler_stage_summary_sources() {
        assert!(cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.step"
        )));
        assert!(cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.usage"
        )));
        assert!(cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.tool.start"
        )));
        assert!(cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.tool.end"
        )));
        assert!(!cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.reasoning"
        )));
        assert!(!cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.content"
        )));
    }
}

#[cfg(test)]
fn extend_wrapped_lines(out: &mut Vec<String>, text: &str, width: usize) {
    if text.is_empty() {
        out.push(String::new());
        return;
    }
    let wrapped = wrap_display_text(text, width.max(1));
    if wrapped.is_empty() {
        out.push(String::new());
    } else {
        out.extend(wrapped);
    }
}

#[cfg(test)]
fn cli_active_stage_context_lines(
    stage: Option<&SchedulerStageBlock>,
    style: &CliStyle,
) -> Vec<String> {
    let Some(stage) = stage else {
        return Vec::new();
    };

    let max_width = usize::from(style.width).saturating_sub(8).clamp(24, 96);
    let header = if let (Some(index), Some(total)) = (stage.stage_index, stage.stage_total) {
        format!("Stage: {} [{}/{}]", stage.title, index, total)
    } else {
        format!("Stage: {}", stage.title)
    };

    let mut summary = Vec::new();
    if let Some(step) = stage.step {
        summary.push(format!("step {step}"));
    }
    if let Some(status) = stage.status.as_deref().filter(|value| !value.is_empty()) {
        summary.push(status.to_string());
    }
    if let Some(waiting_on) = stage
        .waiting_on
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        summary.push(format!("waiting on {waiting_on}"));
    }
    summary.push(format!(
        "tokens {}/{}",
        stage
            .prompt_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
        stage
            .completion_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string())
    ));

    let mut lines = vec![
        truncate_display(&header, max_width),
        truncate_display(&format!("Status: {}", summary.join(" · ")), max_width),
    ];
    if let Some(focus) = stage.focus.as_deref().filter(|value| !value.is_empty()) {
        lines.push(truncate_display(&format!("Focus: {focus}"), max_width));
    }
    if let Some(last_event) = stage
        .last_event
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(truncate_display(&format!("Last: {last_event}"), max_width));
    }
    if let Some(ref attached_id) = stage.attached_session_id {
        lines.push(truncate_display(
            &format!("Child: {attached_id}"),
            max_width,
        ));
    }
    lines
}

fn cli_attach_interactive_handles(
    runtime: &mut CliExecutionRuntime,
    handles: CliInteractiveHandles,
) {
    runtime.terminal_surface = Some(handles.terminal_surface);
    runtime.prompt_chrome = Some(handles.prompt_chrome.clone());
    runtime.prompt_session = Some(handles.prompt_session.clone());
    if let Ok(mut slot) = runtime.prompt_session_slot.lock() {
        *slot = Some(handles.prompt_session.clone());
    }
    runtime.queued_inputs = handles.queued_inputs;
    runtime.busy_flag = handles.busy_flag;
    runtime.exit_requested = handles.exit_requested;
    runtime.active_abort = handles.active_abort;
    handles.prompt_chrome.update_from_runtime(runtime);
    cli_refresh_prompt(runtime);
}

async fn cli_trigger_abort(handle: CliActiveAbortHandle) -> bool {
    match handle {
        CliActiveAbortHandle::Server {
            api_client,
            session_id,
        } => match api_client.abort_session(&session_id).await {
            Ok(result) => result
                .get("aborted")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            Err(e) => {
                tracing::error!("Failed to abort server session: {}", e);
                false
            }
        },
    }
}

async fn cli_execute_new_session_action(
    runtime: &mut CliExecutionRuntime,
    api_client: &CliApiClient,
    repl_style: &CliStyle,
) {
    match api_client
        .create_session(
            runtime.scheduler_profile_name.clone(),
            Some(cli_session_directory(&runtime.working_dir)),
        )
        .await
    {
        Ok(new_session) => {
            let new_sid = new_session.id.clone();
            cli_set_root_server_session(runtime, new_sid.clone());

            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::title(format!(
                    "New session created: {}",
                    &new_sid[..new_sid.len().min(8)]
                ))),
                repl_style,
            );

            cli_refresh_server_info(api_client, &runtime.frontend_projection, Some(&new_sid)).await;
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to create new session: {}",
                    error
                ))),
                repl_style,
            );
        }
    }
}

async fn cli_execute_fork_session_action(
    runtime: &mut CliExecutionRuntime,
    api_client: &CliApiClient,
    repl_style: &CliStyle,
) {
    let Some(session_id) = runtime.server_session_id.clone() else {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning("No active server session to fork.")),
            repl_style,
        );
        return;
    };

    match api_client.fork_session(&session_id, None).await {
        Ok(forked) => {
            cli_set_root_server_session(runtime, forked.id.clone());
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::title(format!("Forked session: {}", forked.id))),
                repl_style,
            );
            cli_refresh_server_info(api_client, &runtime.frontend_projection, Some(&forked.id))
                .await;
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to fork session: {}",
                    error
                ))),
                repl_style,
            );
        }
    }
}

async fn cli_execute_compact_session_action(
    runtime: &mut CliExecutionRuntime,
    api_client: &CliApiClient,
    repl_style: &CliStyle,
    focus: Option<&str>,
) {
    let Some(session_id) = runtime.server_session_id.clone() else {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning("No server session to compact.")),
            repl_style,
        );
        return;
    };

    match api_client.compact_session(&session_id, focus).await {
        Ok(response) => {
            let response_message = response.message.trim();
            let block = if response.success {
                let label = if response_message.is_empty() {
                    focus
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| format!("Session compacted around focus: {value}"))
                        .unwrap_or_else(|| "Session compacted successfully.".to_string())
                } else {
                    response_message.to_string()
                };
                StatusBlock::title(label)
            } else {
                let label = if response_message.is_empty() {
                    "Nothing to compact yet.".to_string()
                } else {
                    response_message.to_string()
                };
                StatusBlock::warning(label)
            };
            let _ = print_block(Some(runtime), OutputBlock::Status(block), repl_style);
            if let Ok(mut proj) = runtime.frontend_projection.lock() {
                proj.session_runtime = None;
                proj.stage_summaries.clear();
                proj.telemetry_topology = None;
                proj.events_browser = None;
                proj.token_stats = CliSessionTokenStats::default();
            }
            cli_refresh_server_info(api_client, &runtime.frontend_projection, Some(&session_id))
                .await;
        }
        Err(error) => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to compact session: {}",
                    error
                ))),
                repl_style,
            );
        }
    }
}

fn cli_frontend_set_phase(
    frontend_projection: &Arc<Mutex<CliFrontendProjection>>,
    phase: CliFrontendPhase,
    active_label: Option<String>,
) {
    if let Ok(mut projection) = frontend_projection.lock() {
        projection.phase = phase;
        if active_label.is_some() {
            projection.active_label = active_label;
        }
    }
}

fn cli_frontend_clear(runtime: &CliExecutionRuntime) {
    if let Ok(mut projection) = runtime.frontend_projection.lock() {
        projection.phase = CliFrontendPhase::Idle;
        projection.active_label = None;
        projection.active_stage = None;
    }
}

fn cli_frontend_observe_block(
    frontend_projection: &Arc<Mutex<CliFrontendProjection>>,
    block: &OutputBlock,
) {
    let Ok(mut projection) = frontend_projection.lock() else {
        return;
    };
    match block {
        OutputBlock::SchedulerStage(stage) => {
            projection.phase = match stage.status.as_deref() {
                Some("waiting") | Some("blocked") => CliFrontendPhase::Waiting,
                Some("cancelling") => CliFrontendPhase::Cancelling,
                Some("cancelled") | Some("done") => projection.phase,
                _ => CliFrontendPhase::Busy,
            };
            projection.active_label = Some(cli_stage_activity_label(stage));
        }
        OutputBlock::Tool(tool) => {
            projection.phase = CliFrontendPhase::Busy;
            projection.active_label = Some(format!("tool {}", tool.name));
        }
        OutputBlock::SessionEvent(event) if event.event == "question" => {
            projection.phase = CliFrontendPhase::Waiting;
            projection.active_label = Some("question".to_string());
        }
        OutputBlock::Message(message)
            if message.role == OutputMessageRole::Assistant
                && matches!(message.phase, MessagePhase::Start | MessagePhase::Delta) =>
        {
            projection.phase = CliFrontendPhase::Busy;
            projection.active_label = Some("assistant response".to_string());
        }
        _ => {}
    }
}

fn cli_stage_activity_label(stage: &SchedulerStageBlock) -> String {
    let mut parts = Vec::new();
    if let (Some(index), Some(total)) = (stage.stage_index, stage.stage_total) {
        parts.push(format!("stage {index}/{total}"));
    } else {
        parts.push("stage".to_string());
    }
    parts.push(stage.stage.clone());
    if let Some(step) = stage.step {
        parts.push(format!("step {step}"));
    }
    parts.join(" · ")
}

fn cli_scheduler_stage_snapshot_key(stage: &SchedulerStageBlock) -> String {
    let decision_title = stage
        .decision
        .as_ref()
        .map(|decision| decision.title.clone())
        .unwrap_or_default();
    format!(
        "{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}",
        stage.stage_index.unwrap_or_default(),
        stage.stage,
        stage.status,
        stage.step,
        stage.waiting_on,
        stage.last_event,
        stage.prompt_tokens,
        stage.completion_tokens,
        decision_title,
        stage.activity.as_deref().unwrap_or_default()
    )
}

fn cli_should_emit_scheduler_stage_block(
    snapshots: &Arc<Mutex<HashMap<String, String>>>,
    stage: &SchedulerStageBlock,
) -> bool {
    let stage_id = stage.stage_id.clone().unwrap_or_else(|| {
        format!(
            "{}:{}",
            stage.stage_index.unwrap_or_default(),
            stage.stage.as_str()
        )
    });
    let snapshot = cli_scheduler_stage_snapshot_key(stage);
    let Ok(mut cache) = snapshots.lock() else {
        return true;
    };
    match cache.get(&stage_id) {
        Some(existing) if existing == &snapshot => false,
        _ => {
            cache.insert(stage_id, snapshot);
            true
        }
    }
}

#[cfg(test)]
mod session_projection_tests {
    use super::{
        cli_cache_evidence_status_label, cli_context_closure_cache_diagnostic_label,
        cli_runtime_snapshot_lines,
    };

    #[test]
    fn context_closure_cache_diagnostic_prefers_narrow_status_words() {
        let contract = rocode_types::SessionContextClosureContract {
            prefix_stability: rocode_types::SessionPrefixStabilityContract {
                basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
                tracked_on_api_view: true,
                api_view_messages: 8,
                trimmed_model_visible_messages: 3,
                prefix_change_detected: true,
                explanation: None,
            },
            compaction_boundary: rocode_types::SessionCompactionBoundaryContract {
                boundary_recorded: true,
                phase: None,
                trigger: None,
                reason: None,
                lifecycle_status: None,
                governance_status: None,
                request_pressure_percent: None,
                live_pressure_percent: None,
                compaction_attempted: true,
                compaction_succeeded: true,
                blocking: false,
                installed: None,
            },
            cache_explainability: rocode_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: true,
                source: rocode_types::SessionCacheExplainabilitySource::BoundaryEvidence,
                severity: Some(rocode_types::SessionCacheSeverity::MediumChange),
                explanation: None,
            },
            child_history_isolation: rocode_types::SessionChildHistoryIsolationContract {
                attached_subtree_session_count: 0,
                owner_session_cumulative_tokens: 0,
                workflow_cumulative_tokens: 0,
                attached_subtree_cumulative_tokens: 0,
                owner_live_context_tokens: Some(0),
                owner_local_live_prefix: true,
                child_history_in_live_prefix_detected: false,
                explanation: "isolated".to_string(),
            },
        };

        assert_eq!(
            cli_context_closure_cache_diagnostic_label(Some(&contract)).as_deref(),
            Some("cache explained · prefix changed")
        );
    }

    #[test]
    fn cache_evidence_status_label_uses_narrow_status_words() {
        let summary = rocode_provider::cache::CacheEvidenceSummary {
            status: "degraded".to_string(),
            severity: rocode_provider::cache::CacheEvidenceSeverity::MediumChange,
            primary_cause: Some("tool surface changed".to_string()),
            change_count: 1,
        };

        assert_eq!(
            cli_cache_evidence_status_label(&summary).as_deref(),
            Some("cache explained")
        );
    }

    #[test]
    fn runtime_snapshot_shows_permission_authority_for_miss_only() {
        let telemetry = crate::api_client::SessionTelemetrySnapshot {
            runtime: crate::api_client::SessionRuntimeState {
                session_id: "sess_123".to_string(),
                run_status: crate::api_client::SessionRunStatusKind::Idle,
                current_message_id: None,
                usage: None,
                active_stage_id: None,
                active_stage_count: 0,
                active_tools: Vec::new(),
                pending_question: None,
                pending_permission: None,
                attached_sessions: Vec::new(),
            },
            stages: Vec::new(),
            topology: crate::api_client::SessionExecutionTopology {
                session_id: "sess_123".to_string(),
                active_count: 0,
                done_count: 0,
                running_count: 0,
                waiting_count: 0,
                cancelling_count: 0,
                retry_count: 0,
                updated_at: None,
                roots: Vec::new(),
            },
            usage: rocode_session::SessionUsage::default(),
            usage_books: rocode_types::SessionUsageBooks::default(),
            tool_repair_summary: None,
            model_tool_repair_summary: None,
            repair_query_snapshot: None,
            tool_trajectory_quality: None,
            tool_result_governance: None,
            pending_permission_count: 0,
            granted_by_turn_count: 0,
            granted_by_session_count: 0,
            granted_by_matcher_kind: Default::default(),
            last_permission_matcher_kind: None,
            last_permission_grant_target: None,
            last_permission_miss_count: 2,
            memory: None,
            cache_evidence: None,
            context_explain: None,
            ownership: None,
            context_compaction_summary: None,
            compaction_continuity: None,
            context_compaction_lifecycle_summary: None,
            context_pressure_governance_summary: None,
            cache_semantics: None,
            context_closure_contract: None,
            prompt_surface_evidence: None,
            ingress_stabilization: None,
            execution_preflight_summary: None,
            provider_diagnostic_summary: None,
        };

        let lines = cli_runtime_snapshot_lines("sess_123", &telemetry);
        let rendered = lines.join("\n");
        assert!(rendered.contains("Permission Authority:"));
        assert!(rendered.contains("Misses: 2"));
    }

}
