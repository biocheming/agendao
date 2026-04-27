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
    if let Ok(mut transcripts) = runtime.child_session_transcripts.lock() {
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

fn cli_track_child_session(runtime: &CliExecutionRuntime, parent_id: &str, child_id: &str) -> bool {
    if parent_id.is_empty() || child_id.is_empty() {
        return false;
    }
    let mut inserted = false;
    if let Ok(mut related) = runtime.related_session_ids.lock() {
        if related.contains(parent_id) {
            inserted = related.insert(child_id.to_string());
        }
    }
    if inserted {
        if let Ok(mut transcripts) = runtime.child_session_transcripts.lock() {
            transcripts.entry(child_id.to_string()).or_default();
        }
    }
    inserted
}

fn cli_untrack_child_session(
    runtime: &CliExecutionRuntime,
    parent_id: &str,
    child_id: &str,
) -> bool {
    if parent_id.is_empty() || child_id.is_empty() {
        return false;
    }
    runtime
        .related_session_ids
        .lock()
        .map(|mut related| related.contains(parent_id) && related.remove(child_id))
        .unwrap_or(false)
}

fn cli_cache_child_session_rendered(
    runtime: &CliExecutionRuntime,
    session_id: &str,
    rendered: &str,
) {
    if let Ok(mut transcripts) = runtime.child_session_transcripts.lock() {
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

fn cli_ordered_child_session_ids(runtime: &CliExecutionRuntime) -> Vec<String> {
    let root_session_id = runtime.server_session_id.as_deref();
    let attached_ids = runtime
        .related_session_ids
        .lock()
        .map(|ids| ids.clone())
        .unwrap_or_default();
    let transcripts = runtime
        .child_session_transcripts
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

fn cli_list_child_sessions(runtime: &CliExecutionRuntime) {
    let style = CliStyle::detect();
    let attached_ids = runtime
        .related_session_ids
        .lock()
        .map(|ids| ids.clone())
        .unwrap_or_default();
    let transcripts = runtime
        .child_session_transcripts
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();
    let focused = cli_focused_session_id(runtime);

    let mut lines = Vec::new();
    let child_ids = cli_ordered_child_session_ids(runtime);
    if child_ids.is_empty() {
        lines.push("No child sessions have been observed for this run yet.".to_string());
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
        Some(child_id) => format!(
            "/child next · /child prev · /child focus <id> · /child back · now viewing {}",
            child_id
        ),
        None => "/child next · /child prev · /child focus <id> · /child back".to_string(),
    };
    let _ = print_cli_list_on_surface(
        Some(runtime),
        "Child Sessions",
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
    if stage.child_session_count > 0 {
        parts.push(format!("child {}", stage.child_session_count));
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

#[cfg_attr(not(test), allow(dead_code))]
fn cli_current_context_tokens(projection: &CliFrontendProjection) -> Option<u64> {
    let usage_context_tokens = (projection.token_stats.context_tokens > 0)
        .then_some(projection.token_stats.context_tokens);
    let active_stage_id = projection
        .session_runtime
        .as_ref()
        .and_then(|runtime| runtime.active_stage_id.as_deref());
    let active_stage_context_tokens = active_stage_id.and_then(|active_stage_id| {
        projection
            .stage_summaries
            .iter()
            .find(|stage| stage.stage_id == active_stage_id)
            .and_then(|stage| stage.estimated_context_tokens)
    });

    rocode_types::current_context_tokens_from_sources(
        usage_context_tokens,
        active_stage_context_tokens,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_context_pressure(percent: Option<u64>) -> Option<rocode_types::ContextPressure> {
    let pressure = rocode_types::context_pressure_for_percent(percent);
    pressure.is_warning_or_higher().then_some(pressure)
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_context_pressure_note(percent: Option<u64>) -> Option<&'static str> {
    rocode_types::context_pressure_label(percent)
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
        if let Some(focus) = stage.focus.as_deref() {
            lines.push(format!("Focus: {}", focus));
        }
        if let Some(context_tokens) = stage.context_tokens.or(stage.estimated_context_tokens) {
            lines.push(format!(
                "Context: {}",
                format_token_count(context_tokens)
            ));
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

    if runtime.child_sessions.is_empty() {
        lines.push(String::new());
        lines.push("Child sessions: none".to_string());
    } else {
        lines.push(String::new());
        lines.push(format!("Child sessions ({})", runtime.child_sessions.len()));
        for child in &runtime.child_sessions {
            lines.push(format!("  {} ← {}", child.child_id, child.parent_id));
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

fn cli_usage_snapshot_lines(
    session_id: &str,
    telemetry: &crate::api_client::SessionTelemetrySnapshot,
    projection: Option<&CliFrontendProjection>,
) -> Vec<String> {
    let usage = &telemetry.usage;
    let mut lines = vec![format!("Session: {}", session_id)];

    if let Some(projection) = projection {
        if let Some(current_tokens) = projection.current_context_tokens() {
            lines.push(String::new());
            lines.push("Current context".to_string());
            if let Some(model) = cli_lookup_model_catalog_entry(projection)
                .filter(|model| model.context_window.unwrap_or(0) > 0)
            {
                let limit = model.context_window.unwrap_or(0);
                let percent = cli_context_usage_percent(current_tokens, limit);
                lines.push(format!(
                    "  Pressure: {}",
                    cli_format_context_meter(current_tokens, Some(limit))
                ));
                if let Some(note) = cli_context_pressure_note(percent) {
                    lines.push(format!("  State: {}", note));
                }
            } else {
                lines.push(format!(
                    "  Pressure: {}",
                    format_token_count(current_tokens)
                ));
            }
        }

        let last_turn = &projection.last_turn_tokens;
        if last_turn.input_tokens > 0 || last_turn.output_tokens > 0 {
            lines.push(String::new());
            lines.push("Last turn".to_string());
            lines.push(format!(
                "  Input {} · Output {}",
                format_token_count(last_turn.input_tokens),
                format_token_count(last_turn.output_tokens)
            ));
        }
    }

    lines.push(String::new());
    lines.push("Session cumulative".to_string());
    lines.push(format!(
        "  Total: {}",
        format_token_count(
            usage.input_tokens
                + usage.output_tokens
                + usage.reasoning_tokens
                + usage.cache_read_tokens
                + usage.cache_write_tokens
        )
    ));
    lines.push(format!(
        "  Input: {}",
        format_token_count(usage.input_tokens)
    ));
    lines.push(format!(
        "  Output: {}",
        format_token_count(usage.output_tokens)
    ));
    lines.push(format!(
        "  Reasoning: {}",
        format_token_count(usage.reasoning_tokens)
    ));
    lines.push(format!(
        "  Cache read: {}",
        format_token_count(usage.cache_read_tokens)
    ));
    lines.push(format!(
        "  Cache write: {}",
        format_token_count(usage.cache_write_tokens)
    ));
    lines.push(format!("  Cost: ${:.4}", usage.total_cost));

    if !telemetry.stages.is_empty() {
        lines.push(String::new());
        lines.push(format!("Stage totals ({})", telemetry.stages.len()));
        for stage in &telemetry.stages {
            lines.push(format!("  {}", cli_stage_usage_line(stage)));
        }
    }

    lines
}

fn cli_session_insights_lines(
    session_id: &str,
    insights: &crate::api_client::SessionInsightsResponse,
) -> Vec<String> {
    let mut lines = vec![
        format!("Session: {}", session_id),
        format!("Title: {}", insights.title),
        format!("Directory: {}", insights.directory),
        format!(
            "Updated: {}",
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(insights.updated)
                .map(|value| value.with_timezone(&chrono::Local))
                .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| insights.updated.to_string())
        ),
    ];

    if let Some(telemetry) = insights.telemetry.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Persisted telemetry: version {:?} · status {} · updated {}",
            telemetry.version, telemetry.last_run_status, telemetry.updated_at
        ));
        lines.push(format!(
            "  Session cumulative: total {} · input {} · output {} · reasoning {}",
            format_token_count(
                telemetry.usage.input_tokens
                    + telemetry.usage.output_tokens
                    + telemetry.usage.reasoning_tokens
                    + telemetry.usage.cache_read_tokens
                    + telemetry.usage.cache_write_tokens
            ),
            format_token_count(telemetry.usage.input_tokens),
            format_token_count(telemetry.usage.output_tokens),
            format_token_count(telemetry.usage.reasoning_tokens),
        ));
        lines.push(format!(
            "  Cache read {} · cache write {} · cost ${:.4}",
            format_token_count(telemetry.usage.cache_read_tokens),
            format_token_count(telemetry.usage.cache_write_tokens),
            telemetry.usage.total_cost
        ));
        lines.push(format!(
            "  Persisted stages: {}",
            telemetry.stage_summaries.len()
        ));
    }

    if let Some(memory) = insights.memory.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Memory explain: {} · {}",
            memory.summary.workspace_mode,
            truncate_text(&memory.summary.workspace_key, 88)
        ));
        lines.push(format!(
            "  Frozen snapshot packet: {} items{}",
            memory.summary.frozen_snapshot_items,
            cli_optional_generated_at(memory.summary.frozen_snapshot_generated_at)
        ));
        lines.push(format!(
            "  Last prefetch packet: {} items{}",
            memory.summary.last_prefetch_items,
            cli_optional_generated_at(memory.summary.last_prefetch_generated_at)
        ));
        if let Some(query) = memory.summary.last_prefetch_query.as_deref() {
            lines.push(format!("  Prefetch query: {}", truncate_text(query, 120)));
        }
        lines.push(format!(
            "  Validation pressure: warnings {} · methodology {} · skill targets {}",
            memory.summary.warning_count,
            memory.summary.methodology_candidate_count,
            memory.summary.derived_skill_candidate_count
        ));
        if let Some(run) = memory.summary.latest_consolidation_run.as_ref() {
            lines.push(format!(
                "  Latest consolidation: {} · merged {} · promoted {} · conflicts {}",
                run.run_id, run.merged_count, run.promoted_count, run.conflict_count
            ));
        }
        if !memory.summary.recent_rule_hits.is_empty() {
            lines.push(format!(
                "  Recent rule hits ({})",
                memory.summary.recent_rule_hits.len()
            ));
            for hit in &memory.summary.recent_rule_hits {
                let detail = hit.detail.as_deref().unwrap_or("no detail");
                lines.push(format!(
                    "    {} · {}",
                    hit.hit_kind,
                    truncate_text(detail, 96)
                ));
            }
        }
        if let Some(packet) = memory.frozen_snapshot.as_ref() {
            lines.push(format!(
                "  Frozen snapshot scopes: {}",
                packet
                    .scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(packet) = memory.last_prefetch_packet.as_ref() {
            lines.push(format!(
                "  Last prefetch scopes: {}",
                packet
                    .scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let skill_linked = memory
            .recent_session_records
            .iter()
            .filter(|item| item.linked_skill_name.is_some() || item.derived_skill_name.is_some())
            .take(3)
            .collect::<Vec<_>>();
        if !skill_linked.is_empty() {
            lines.push("  Skill-linked recent records:".to_string());
            for item in skill_linked {
                lines.push(format!(
                    "    {} · linked={} · target={}",
                    truncate_text(&item.title, 72),
                    item.linked_skill_name.as_deref().unwrap_or("--"),
                    item.derived_skill_name.as_deref().unwrap_or("--")
                ));
            }
        }
        let suggested_ids = memory
            .summary
            .recent_rule_hits
            .iter()
            .filter_map(|hit| hit.memory_id.as_ref().map(|id| id.0.as_str()))
            .chain(
                memory
                    .last_prefetch_packet
                    .iter()
                    .flat_map(|packet| packet.items.iter().map(|item| item.card.id.0.as_str())),
            )
            .take(3)
            .collect::<Vec<_>>();
        if !suggested_ids.is_empty() {
            lines.push("  Follow-up commands:".to_string());
            for record_id in suggested_ids {
                lines.push(format!("    /memory show {}", record_id));
                lines.push(format!("    /memory hits record={}", record_id));
            }
        }
        if let Some(run) = memory.summary.latest_consolidation_run.as_ref() {
            lines.push(format!("    /memory hits run={}", run.run_id));
        }
    }

    if let Some(multimodal) = insights.multimodal.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Multimodal explain: {}",
            multimodal.display_label()
        ));
        lines.push(format!(
            "  Message: {} · attachments {} · hard block {}",
            multimodal.user_message_id,
            multimodal.attachment_count,
            if multimodal.hard_block { "yes" } else { "no" }
        ));
        lines.push(format!(
            "  Resolved model: {}",
            multimodal.resolved_model.as_deref().unwrap_or("--")
        ));
        lines.push(format!(
            "  Kinds: {}",
            if multimodal.kinds.is_empty() {
                "--".to_string()
            } else {
                multimodal.kinds.join(", ")
            }
        ));
        lines.push(format!(
            "  Badges: {}",
            if multimodal.badges.is_empty() {
                "--".to_string()
            } else {
                multimodal.badges.join(", ")
            }
        ));
        lines.push(format!(
            "  Unsupported parts: {}",
            if multimodal.unsupported_parts.is_empty() {
                "none".to_string()
            } else {
                multimodal.unsupported_parts.join(", ")
            }
        ));
        lines.push(format!(
            "  Recommended downgrade: {}",
            multimodal
                .recommended_downgrade
                .as_deref()
                .unwrap_or("none")
        ));
        lines.push(format!(
            "  Transport replaced parts: {}",
            if multimodal.transport_replaced_parts.is_empty() {
                "none".to_string()
            } else {
                multimodal.transport_replaced_parts.join(", ")
            }
        ));
        if !multimodal.attachments.is_empty() {
            lines.push("  Attachments:".to_string());
            for attachment in &multimodal.attachments {
                lines.push(format!(
                    "    {} ({})",
                    truncate_text(&attachment.filename, 72),
                    attachment.mime
                ));
            }
        }
        let combined_warnings = multimodal.combined_warnings();
        if !combined_warnings.is_empty() {
            lines.push("  Warnings:".to_string());
            for warning in &combined_warnings {
                lines.push(format!("    {}", truncate_text(warning, 108)));
            }
        }
    }

    lines
}

type CliEventsQueryInput = rocode_command::interactive::InteractiveEventsQuery;
type CliEventsCommandInput = rocode_command::interactive::InteractiveEventsCommand;

#[cfg(test)]
const CLI_EVENTS_DEFAULT_PAGE_SIZE: usize =
    rocode_command::interactive::EVENTS_BROWSER_DEFAULT_PAGE_SIZE;

fn cli_default_events_query_input() -> CliEventsQueryInput {
    rocode_command::interactive::default_events_browser_query()
}

#[derive(Debug, Clone, Default)]
struct CliEventsBrowserState {
    session_id: String,
    filter: CliEventsQueryInput,
    offset: usize,
}

fn cli_parse_events_command_input(raw: Option<&str>) -> CliEventsCommandInput {
    rocode_command::interactive::parse_events_browser_command(raw)
}

#[cfg(test)]
fn cli_parse_events_query_input(raw: Option<&str>) -> CliEventsQueryInput {
    rocode_command::interactive::parse_events_browser_query(raw)
}

fn cli_events_query(
    input: &CliEventsQueryInput,
    offset: usize,
) -> crate::api_client::SessionEventsQuery {
    crate::api_client::SessionEventsQuery {
        stage_id: input.stage_id.clone(),
        execution_id: input.execution_id.clone(),
        event_type: input.event_type.clone(),
        since: input.since,
        limit: input.limit,
        offset: Some(offset),
    }
}

fn cli_events_page_size(input: &CliEventsQueryInput) -> usize {
    rocode_command::interactive::events_browser_page_size(input)
}

fn cli_events_offset_for_page(input: &CliEventsQueryInput, page: usize) -> usize {
    rocode_command::interactive::events_browser_offset_for_page(input, page)
}

fn cli_events_page_for_offset(input: &CliEventsQueryInput, offset: usize) -> usize {
    rocode_command::interactive::events_browser_page_for_offset(input, offset)
}

fn cli_events_filter_label(input: &CliEventsQueryInput) -> String {
    let mut parts = Vec::new();
    if let Some(stage_id) = input.stage_id.as_deref() {
        parts.push(format!("stage={stage_id}"));
    }
    if let Some(execution_id) = input.execution_id.as_deref() {
        parts.push(format!("exec={execution_id}"));
    }
    if let Some(event_type) = input.event_type.as_deref() {
        parts.push(format!("type={event_type}"));
    }
    if let Some(since) = input.since {
        parts.push(format!("since={since}"));
    }
    parts.push(format!("limit={}", cli_events_page_size(input)));
    parts.join(" · ")
}

fn cli_events_window_label(offset: usize, count: usize) -> String {
    if count == 0 {
        return "items 0".to_string();
    }
    format!("items {}-{}", offset + 1, offset + count)
}

fn cli_event_payload_summary(payload: &serde_json::Value) -> Option<String> {
    match payload {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.trim().to_string()),
        value => serde_json::to_string(value).ok(),
    }
    .filter(|text| !text.is_empty())
    .map(|text| truncate_text(&text.replace('\n', " "), 120))
}

fn cli_optional_generated_at(ts: Option<i64>) -> String {
    ts.and_then(|value| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value))
        .map(|value| value.with_timezone(&chrono::Local))
        .map(|value| format!(" @ {}", value.format("%Y-%m-%d %H:%M:%S")))
        .unwrap_or_default()
}

fn cli_event_lines(
    events: &[rocode_command::stage_protocol::StageEvent],
    style: &CliStyle,
) -> Vec<String> {
    if events.is_empty() {
        return vec![style.dim("no matching events")];
    }

    let mut lines = Vec::new();
    for event in events {
        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(event.ts)
            .map(|value| value.with_timezone(&chrono::Local))
            .map(|value| value.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| event.ts.to_string());
        let mut headline = format!("{} · {} · {:?}", ts, event.event_type, event.scope);
        if let Some(stage_id) = event.stage_id.as_deref() {
            headline.push_str(&format!(" · stage {}", stage_id));
        }
        if let Some(execution_id) = event.execution_id.as_deref() {
            headline.push_str(&format!(" · exec {}", execution_id));
        }
        lines.push(headline);
        if let Some(payload) = cli_event_payload_summary(&event.payload) {
            lines.push(format!("  {}", payload));
        }
    }
    lines
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
            let footer = "Source: /session/{id}/insights · includes persisted telemetry, multimodal explain, and memory explain";
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

fn cli_focus_child_session(runtime: &CliExecutionRuntime, requested_id: &str) -> io::Result<bool> {
    let requested_id = requested_id.trim();
    if requested_id.is_empty() {
        return Ok(false);
    }

    let transcripts = runtime
        .child_session_transcripts
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
        Some(format!("view child {}", cli_short_session_id(&target_id))),
    );
    cli_replace_visible_transcript(runtime, transcript)?;
    Ok(true)
}

fn cli_cycle_child_session(
    runtime: &CliExecutionRuntime,
    forward: bool,
) -> io::Result<Option<(String, usize, usize)>> {
    let child_ids = cli_ordered_child_session_ids(runtime);
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
    if !cli_focus_child_session(runtime, &target_id)? {
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
        )
    )
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
    if let Some(ref child_id) = stage.child_session_id {
        lines.push(truncate_display(&format!("Child: {child_id}"), max_width));
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
        .create_session(None, runtime.resolved_scheduler_profile_name.clone())
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
            let block = if response.success {
                let label = focus
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!("Session compacted around focus: {value}"))
                    .unwrap_or_else(|| "Session compacted successfully.".to_string());
                StatusBlock::title(label)
            } else {
                StatusBlock::warning("Nothing to compact yet.")
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
fn cli_fit_lines(lines: &[String], width: usize, rows: usize, tail: bool) -> Vec<String> {
    let mut wrapped = Vec::new();
    for line in lines {
        extend_wrapped_lines(&mut wrapped, line, width);
    }
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }
    if wrapped.len() > rows {
        if tail {
            wrapped.split_off(wrapped.len().saturating_sub(rows))
        } else {
            wrapped.truncate(rows);
            wrapped
        }
    } else {
        wrapped.resize(rows, String::new());
        wrapped
    }
}

#[cfg(test)]
fn cli_box_line(text: &str, inner_width: usize, style: &CliStyle) -> String {
    let content = pad_right_display(text, inner_width, ' ');
    if style.color {
        format!("{} {} {}", style.cyan("│"), content, style.cyan("│"))
    } else {
        format!("│ {} │", content)
    }
}

#[cfg(test)]
fn cli_render_box(
    title: &str,
    footer: Option<&str>,
    lines: &[String],
    outer_width: usize,
    style: &CliStyle,
) -> Vec<String> {
    let inner_width = outer_width.saturating_sub(4).max(1);
    let chrome_width = inner_width + 2;
    let header_content = pad_right_display(
        &truncate_display(&format!(" {} ", title.trim()), chrome_width),
        chrome_width,
        '─',
    );
    let header = if style.color {
        format!(
            "{}{}{}",
            style.cyan("╭"),
            style.bold_cyan(&header_content),
            style.cyan("╮")
        )
    } else {
        format!("╭{}╮", header_content)
    };

    let footer_text = footer.unwrap_or("");
    let footer_content = if footer_text.is_empty() {
        "─".repeat(chrome_width)
    } else {
        pad_right_display(
            &truncate_display(&format!(" {} ", footer_text.trim()), chrome_width),
            chrome_width,
            '─',
        )
    };
    let footer = if style.color {
        format!(
            "{}{}{}",
            style.cyan("╰"),
            style.dim(&footer_content),
            style.cyan("╯")
        )
    } else {
        format!("╰{}╯", footer_content)
    };

    let mut rendered = Vec::with_capacity(lines.len() + 2);
    rendered.push(header);
    rendered.extend(
        lines
            .iter()
            .map(|line| cli_box_line(line, inner_width, style)),
    );
    rendered.push(footer);
    rendered
}

#[cfg(test)]
fn cli_join_columns(
    left: &[String],
    left_width: usize,
    right: &[String],
    right_width: usize,
    gap: usize,
) -> Vec<String> {
    let blank_left = " ".repeat(left_width);
    let blank_right = " ".repeat(right_width);
    let height = left.len().max(right.len());
    let mut rows = Vec::with_capacity(height);
    for index in 0..height {
        let left_line = left.get(index).map(String::as_str).unwrap_or(&blank_left);
        let right_line = right.get(index).map(String::as_str).unwrap_or(&blank_right);
        rows.push(format!("{}{}{}", left_line, " ".repeat(gap), right_line));
    }
    rows
}

#[cfg(test)]
fn cli_terminal_rows() -> usize {
    crossterm::terminal::size()
        .map(|(_, rows)| usize::from(rows))
        .unwrap_or(28)
}

#[cfg(test)]
fn cli_sidebar_lines(
    projection: &CliFrontendProjection,
    topology: &CliObservedExecutionTopology,
) -> Vec<String> {
    let phase = match projection.phase {
        CliFrontendPhase::Idle => "idle",
        CliFrontendPhase::Busy => "busy",
        CliFrontendPhase::Waiting => "waiting",
        CliFrontendPhase::Cancelling => "cancelling",
        CliFrontendPhase::Failed => "error",
    };
    let mut lines = vec![
        format!("Phase: {}", phase),
        format!(
            "Queue: {}",
            if projection.queue_len == 0 {
                "empty".to_string()
            } else {
                projection.queue_len.to_string()
            }
        ),
    ];
    if let Some(active) = projection
        .active_label
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Activity: {active}"));
    }
    if topology.active {
        lines.push("Execution: active".to_string());
    } else {
        lines.push("Execution: idle".to_string());
    }
    if let Some(active_stage_id) = topology.active_stage_id.as_deref() {
        if let Some(node) = topology.nodes.get(active_stage_id) {
            lines.push(format!("Node: {}", node.label));
            lines.push(format!("Status: {}", node.status));
            if let Some(waiting_on) = node.waiting_on.as_deref() {
                lines.push(format!("Waiting: {waiting_on}"));
            }
            if let Some(recent_event) = node.recent_event.as_deref() {
                lines.push(format!("Last: {recent_event}"));
            }
        }
    }

    // ── Usage (current context + session totals + last turn) ────
    let ts = &projection.token_stats;
    let model_info = cli_lookup_model_catalog_entry(projection);
    let current_context_tokens = cli_current_context_tokens(projection);
    let last_turn = &projection.last_turn_tokens;
    if current_context_tokens.is_some()
        || ts.total_tokens > 0
        || model_info.is_some()
        || last_turn.input_tokens > 0
        || last_turn.output_tokens > 0
    {
        lines.push(String::new());
        lines.push("─ Usage ─".to_string());
        if let Some(current_tokens) = current_context_tokens {
            if let Some(model) = model_info.filter(|model| model.context_window.unwrap_or(0) > 0) {
                let limit = model.context_window.unwrap_or(0);
                let percent = cli_context_usage_percent(current_tokens, limit);
                lines.push(format!(
                    "Current: {}",
                    cli_format_context_meter(current_tokens, Some(limit))
                ));
                if let Some(note) = cli_context_pressure_note(percent) {
                    lines.push(format!("State:   {} ({})", note, percent.unwrap_or(0)));
                }
            } else {
                lines.push(format!("Current: {}", format_token_count(current_tokens)));
            }
        }
        if ts.total_tokens > 0 {
            lines.push(format!(
                "Session: {} cumulative",
                format_token_count(ts.total_tokens)
            ));
        }
        if last_turn.input_tokens > 0 || last_turn.output_tokens > 0 {
            lines.push(format!(
                "Turn:    ↑{}  ↓{}",
                format_token_count(last_turn.input_tokens),
                format_token_count(last_turn.output_tokens)
            ));
        }
        if ts.reasoning_tokens > 0 {
            lines.push(format!(
                "Reason:  {}",
                format_token_count(ts.reasoning_tokens)
            ));
        }
        if ts.cache_read_tokens > 0 || ts.cache_write_tokens > 0 {
            lines.push(format!(
                "Cache:   read {} · write {}",
                format_token_count(ts.cache_read_tokens),
                format_token_count(ts.cache_write_tokens)
            ));
        }
        if let Some(model) = model_info {
            if let (Some(input_price), Some(output_price)) =
                (model.cost_per_million_input, model.cost_per_million_output)
            {
                lines.push(format!(
                    "Price:   {}",
                    cli_format_price_pair(input_price, output_price)
                ));
            }
        }
        lines.push(format!("Cost:    ${:.4}", ts.total_cost));
    }

    // ── MCP servers ─────────────────────────────────────────────
    if !projection.mcp_servers.is_empty() {
        let connected = projection
            .mcp_servers
            .iter()
            .filter(|s| s.status == "connected")
            .count();
        let errored = projection
            .mcp_servers
            .iter()
            .filter(|s| s.status == "failed" || s.status == "error")
            .count();
        lines.push(String::new());
        lines.push(format!("─ MCP ({} active, {} err) ─", connected, errored));
        for server in &projection.mcp_servers {
            let indicator = match server.status.as_str() {
                "connected" => "●",
                "failed" | "error" => "✗",
                "needs_auth" | "needs auth" => "?",
                _ => "○",
            };
            lines.push(format!("{} {} [{}]", indicator, server.name, server.status));
            if let Some(ref err) = server.error {
                lines.push(format!("  ↳ {}", err));
            }
        }
    }

    // ── LSP servers ─────────────────────────────────────────────
    if !projection.lsp_servers.is_empty() {
        lines.push(String::new());
        lines.push(format!("─ LSP ({}) ─", projection.lsp_servers.len()));
        for server in &projection.lsp_servers {
            lines.push(format!("● {}", server));
        }
    }

    lines.push(String::new());
    lines.push("/help · /model · /preset".to_string());
    lines.push("/runtime · /usage · /insights · /events".to_string());
    lines.push("/events next · /events prev · /events page <n>".to_string());
    lines.push("/events first · /events clear".to_string());
    lines.push("/child · /abort · /status".to_string());
    lines
}

/// Format a token count for display (e.g., 1234 → "1,234", 1234567 → "1.2M").
fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        let compact = n as f64 / 1_000_000.0;
        if compact.fract() == 0.0 {
            format!("{compact:.0}M")
        } else {
            format!("{compact:.1}M")
        }
    } else if n >= 1_000 {
        let compact = n as f64 / 1_000.0;
        if compact.fract() == 0.0 {
            format!("{compact:.0}K")
        } else {
            format!("{compact:.1}K")
        }
    } else {
        n.to_string()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    rocode_types::context_usage_percent(used, limit)
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_context_usage_bar(percent: Option<u64>, width: usize) -> String {
    let safe_percent = percent.unwrap_or(0).min(100) as usize;
    let filled = ((safe_percent * width) + 50) / 100;
    format!(
        "[{}{}]",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_format_context_meter(used: u64, limit: Option<u64>) -> String {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return format_token_count(used);
    };
    let percent = cli_context_usage_percent(used, limit);
    format!(
        "{}/{} {} {}%",
        format_token_count(used),
        format_token_count(limit),
        cli_context_usage_bar(percent, 10),
        percent.unwrap_or(0)
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_lookup_model_catalog_entry(
    projection: &CliFrontendProjection,
) -> Option<&CliModelCatalogEntry> {
    let model_label = projection
        .current_model_label
        .as_deref()
        .filter(|value| !value.trim().is_empty() && *value != "auto")?;
    projection.model_catalog.get(model_label).or_else(|| {
        projection
            .model_catalog
            .iter()
            .find(|(candidate, _)| {
                candidate.as_str() == model_label
                    || candidate
                        .rsplit_once('/')
                        .map(|(_, suffix)| suffix == model_label)
                        .unwrap_or(false)
            })
            .map(|(_, model)| model)
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_format_price_pair(input: f64, output: f64) -> String {
    format!(
        "${}/{} /1M",
        cli_format_price(input),
        cli_format_price(output)
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn cli_format_price(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else if value >= 0.1 {
        format!("{value:.3}")
    } else {
        format!("{value:.4}")
    }
}

#[cfg(test)]
fn cli_active_stage_panel_lines(
    stage: Option<&SchedulerStageBlock>,
    style: &CliStyle,
) -> Vec<String> {
    let Some(stage) = stage else {
        return vec![
            "No active stage. Running work will appear here in-place.".to_string(),
            "Transcript stays on the left; live execution stays here.".to_string(),
            String::new(),
            "Queued prompts remain editable in the input box below.".to_string(),
            "Use /abort to stop the active execution boundary.".to_string(),
        ];
    };

    let mut lines = cli_active_stage_context_lines(Some(stage), style);
    if let Some(activity) = stage.activity.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("Activity: {}", activity.replace('\n', " · ")));
    }
    let mut available = Vec::new();
    if let Some(count) = stage.available_skill_count {
        available.push(format!("skills {}", count));
    }
    if let Some(count) = stage.available_agent_count {
        available.push(format!("agents {}", count));
    }
    if let Some(count) = stage.available_category_count {
        available.push(format!("categories {}", count));
    }
    if !available.is_empty() {
        lines.push(format!("Available: {}", available.join(" · ")));
    }
    if !stage.active_skills.is_empty() {
        lines.push(format!("Active skills: {}", stage.active_skills.join(", ")));
    }
    if stage.total_agent_count > 0 {
        lines.push(format!(
            "Agents: [{}/{}]",
            stage.done_agent_count, stage.total_agent_count
        ));
    }
    if let Some(ref child_id) = stage.child_session_id {
        lines.push(format!("→ Child session: {}", child_id));
    }
    lines
}

#[cfg(test)]
fn cli_messages_footer(
    transcript: &CliRetainedTranscript,
    width: usize,
    max_rows: usize,
    scroll_offset: usize,
) -> String {
    let total = transcript.total_rows(width);
    if total <= max_rows {
        return "retained transcript".to_string();
    }
    if scroll_offset == 0 {
        format!("↑ /up to scroll · {} lines total", total)
    } else {
        let max_offset = total.saturating_sub(max_rows);
        let clamped = scroll_offset.min(max_offset);
        let position = max_offset.saturating_sub(clamped);
        format!("line {}/{} · /up /down /bottom", position + 1, total,)
    }
}

#[cfg(test)]
fn cli_render_retained_layout(
    mode: &str,
    model: &str,
    directory: &str,
    projection: &CliFrontendProjection,
    topology: &CliObservedExecutionTopology,
    style: &CliStyle,
) -> Vec<String> {
    let total_width = usize::from(style.width.saturating_sub(1)).clamp(60, 160);
    let terminal_rows = cli_terminal_rows().max(20);
    let gap = 1usize;

    // Session header — compact single-line with session title
    let session_title = projection
        .session_title
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(untitled)");
    let mut header_parts = vec![
        truncate_display(session_title, 32),
        mode.to_string(),
        model.to_string(),
    ];
    if let Some(view_label) = projection
        .view_label
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        header_parts.push(view_label.to_string());
    }
    header_parts.push(truncate_display(directory, 24));
    let header_lines = vec![format!("> {}", header_parts.join(" · "))];
    let header_box = cli_render_box("ROCode", None, &header_lines, total_width, style);

    // ── Adaptive layout: compute actual content sizes, then allocate rows ──
    //
    // Fixed chrome overhead (lines consumed by box borders):
    //   header_box:   3 lines (top border + 1 content + bottom border)
    //   messages_box: 2 lines (top border + bottom border)
    //   active_box:   2 lines (top border + bottom border) when expanded, 3 when collapsed
    //   prompt:       ~8 lines (rendered separately by PromptFrame, not counted in screen_lines)
    //
    // Remaining rows after chrome are split between messages content and active content,
    // with active getting exactly what it needs (clamped) and messages getting the rest.

    let active_inner_width = total_width.saturating_sub(4).max(1);

    // Compute active panel's natural content height
    let (active_content_lines, active_chrome) = if projection.active_collapsed {
        // Collapsed: single label line + 2 chrome = 3 total
        (Vec::new(), 3usize) // content handled separately below
    } else {
        let raw_lines = cli_active_stage_panel_lines(projection.active_stage.as_ref(), style);
        // Wrap lines to actual width to get true row count
        let mut wrapped_count = 0usize;
        for line in &raw_lines {
            wrapped_count += 1.max(
                (display_width(line) + active_inner_width.saturating_sub(1))
                    / active_inner_width.max(1),
            );
        }
        let natural_rows = if raw_lines.is_empty() {
            1
        } else {
            wrapped_count
        };
        (raw_lines, 2 + natural_rows.clamp(2, 12)) // chrome(2) + content(2..12)
    };

    // Total chrome = header(3) + messages chrome(2) + active chrome + prompt overhead(~8)
    let prompt_overhead = 8usize; // header + ~6 visible rows + footer
    let total_chrome = 3 + 2 + active_chrome + prompt_overhead;
    let sidebar_overhead = if projection.sidebar_collapsed { 3 } else { 0 };

    // Messages get whatever remains after chrome and active content
    let body_rows = terminal_rows.saturating_sub(total_chrome).max(4) + sidebar_overhead;

    let mut screen = Vec::new();
    screen.extend(header_box);

    if projection.sidebar_collapsed {
        // Full-width Messages only, no sidebar column
        let messages_inner = total_width.saturating_sub(4).max(1);
        let transcript_lines = projection.transcript.viewport_lines(
            messages_inner,
            body_rows,
            projection.scroll_offset,
        );
        let messages_footer = cli_messages_footer(
            &projection.transcript,
            messages_inner,
            body_rows,
            projection.scroll_offset,
        );
        let messages_box = cli_render_box(
            "Messages",
            Some(&messages_footer),
            &transcript_lines,
            total_width,
            style,
        );
        screen.extend(messages_box);
    } else {
        let right_width = (if total_width >= 128 { 38 } else { 32 })
            .min(total_width.saturating_sub(29 + gap))
            .max(24);
        let left_width = total_width.saturating_sub(right_width + gap);
        let left_inner = left_width.saturating_sub(4).max(1);
        let right_inner = right_width.saturating_sub(4).max(1);
        let transcript_lines =
            projection
                .transcript
                .viewport_lines(left_inner, body_rows, projection.scroll_offset);
        let messages_footer = cli_messages_footer(
            &projection.transcript,
            left_inner,
            body_rows,
            projection.scroll_offset,
        );
        let sidebar_lines = cli_fit_lines(
            &cli_sidebar_lines(projection, topology),
            right_inner,
            body_rows,
            false,
        );
        let messages_box = cli_render_box(
            "Messages",
            Some(&messages_footer),
            &transcript_lines,
            left_width,
            style,
        );
        let sidebar_box = cli_render_box("Sidebar", None, &sidebar_lines, right_width, style);
        let body = cli_join_columns(&messages_box, left_width, &sidebar_box, right_width, gap);
        screen.extend(body);
    }

    if projection.active_collapsed {
        // Single collapsed bar
        let collapsed_label = if let Some(stage) = projection.active_stage.as_ref() {
            format!(
                "▸ {} (collapsed — /active to expand)",
                truncate_display(&stage.title, total_width.saturating_sub(48).max(12)),
            )
        } else {
            "▸ No active stage (/active to expand)".to_string()
        };
        let active_box = cli_render_box("Active", None, &[collapsed_label], total_width, style);
        screen.extend(active_box);
    } else {
        // Use actual content height (already clamped 2..12 during budget computation)
        let active_rows = active_chrome.saturating_sub(2); // remove chrome to get content rows
        let active_lines = cli_fit_lines(
            &active_content_lines,
            active_inner_width,
            active_rows,
            false,
        );
        let active_box = cli_render_box("Active", None, &active_lines, total_width, style);
        screen.extend(active_box);
    }

    screen
}

#[cfg(test)]
mod session_projection_tests {
    use super::{
        cli_context_usage_bar, cli_current_context_tokens, cli_default_events_query_input,
        cli_format_context_meter, cli_parse_events_command_input, cli_parse_events_query_input,
        CliEventsCommandInput, CliEventsQueryInput, CliFrontendProjection, CLI_EVENTS_DEFAULT_PAGE_SIZE,
    };

    #[test]
    fn parses_default_events_query_input() {
        assert_eq!(
            cli_parse_events_query_input(None),
            cli_default_events_query_input()
        );
    }

    #[test]
    fn parses_stage_alias_events_query_input() {
        assert_eq!(
            cli_parse_events_query_input(Some("stg_123")),
            CliEventsQueryInput {
                stage_id: Some("stg_123".to_string()),
                limit: Some(CLI_EVENTS_DEFAULT_PAGE_SIZE),
                ..Default::default()
            }
        );
    }

    #[test]
    fn parses_structured_events_query_input() {
        assert_eq!(
            cli_parse_events_query_input(Some(
                "stage=stg_1 exec=exe_2 type=session.updated limit=10 since=42"
            )),
            CliEventsQueryInput {
                stage_id: Some("stg_1".to_string()),
                execution_id: Some("exe_2".to_string()),
                event_type: Some("session.updated".to_string()),
                since: Some(42),
                limit: Some(10),
            }
        );
    }

    #[test]
    fn parses_events_navigation_commands() {
        assert_eq!(
            cli_parse_events_command_input(Some("next")),
            CliEventsCommandInput::NextPage
        );
        assert_eq!(
            cli_parse_events_command_input(Some("prev")),
            CliEventsCommandInput::PreviousPage
        );
        assert_eq!(
            cli_parse_events_command_input(Some("clear")),
            CliEventsCommandInput::Clear
        );
        assert_eq!(
            cli_parse_events_command_input(Some("first")),
            CliEventsCommandInput::FirstPage
        );
        assert_eq!(
            cli_parse_events_command_input(Some("page 3")),
            CliEventsCommandInput::JumpPage(3)
        );
        assert_eq!(
            cli_parse_events_command_input(Some("stage=stg_1 limit=10")),
            CliEventsCommandInput::ShowFiltered {
                filter: CliEventsQueryInput {
                    stage_id: Some("stg_1".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
                page: 1,
            }
        );
        assert_eq!(
            cli_parse_events_command_input(Some("stage=stg_1 limit=10 page=2")),
            CliEventsCommandInput::ShowFiltered {
                filter: CliEventsQueryInput {
                    stage_id: Some("stg_1".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
                page: 2,
            }
        );
    }

    #[test]
    fn formats_compact_context_meter() {
        assert_eq!(
            cli_format_context_meter(12_450, Some(200_000)),
            "12.4K/200K [█░░░░░░░░░] 6%"
        );
    }

    #[test]
    fn builds_context_bar_with_clamped_width() {
        assert_eq!(cli_context_usage_bar(Some(0), 5), "[░░░░░]");
        assert_eq!(cli_context_usage_bar(Some(50), 5), "[███░░]");
        assert_eq!(cli_context_usage_bar(Some(140), 5), "[█████]");
    }

    #[test]
    fn current_context_does_not_fall_back_to_last_turn_input() {
        let mut projection = CliFrontendProjection::default();
        projection.last_turn_tokens.input_tokens = 1_388_907;

        assert_eq!(cli_current_context_tokens(&projection), None);
    }
}
