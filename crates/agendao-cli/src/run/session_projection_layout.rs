#[cfg(test)]
use super::{
    cli_active_stage_context_lines, CliFrontendPhase, CliFrontendProjection,
    CliObservedExecutionTopology, CliStyle, CliVisibleTranscript, SchedulerStageBlock,
};
#[cfg(test)]
use crate::run::session_projection_usage::format_token_count;
#[cfg(test)]
use crate::util::truncate_text;
#[cfg(test)]
use agendao_command_render::cli_panel::{display_width, pad_right_display, truncate_display};

#[cfg(test)]
pub(super) fn cli_fit_lines(
    lines: &[String],
    width: usize,
    rows: usize,
    tail: bool,
) -> Vec<String> {
    let mut wrapped = Vec::new();
    for line in lines {
        super::extend_wrapped_lines(&mut wrapped, line, width);
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
pub(super) fn cli_sidebar_lines(
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

    let ts = &projection.token_stats;
    let model_info = super::session_projection_usage::cli_lookup_model_catalog_entry(projection);
    let current_context_tokens =
        super::session_projection_usage::cli_current_context_tokens(projection);
    let last_turn = &projection.last_turn_tokens;
    if current_context_tokens.is_some()
        || ts.total_tokens > 0
        || model_info.is_some()
        || last_turn.input_tokens > 0
        || last_turn.output_tokens > 0
        || projection.cache_diagnostic.is_some()
        || projection.ingress_diagnostic.is_some()
        || projection.provider_diagnostic.is_some()
    {
        lines.push(String::new());
        lines.push("─ Usage ─".to_string());
        if let Some(current_tokens) = current_context_tokens {
            if let Some(model) = model_info.filter(|model| model.context_window.unwrap_or(0) > 0) {
                let limit = model.context_window.unwrap_or(0);
                let percent = super::session_projection_usage::cli_context_usage_percent(
                    current_tokens,
                    limit,
                );
                lines.push(format!(
                    "Current: {}",
                    super::session_projection_usage::cli_format_context_meter(
                        current_tokens,
                        Some(limit),
                    )
                ));
                if let Some(note) = agendao_types::context_pressure_label(percent) {
                    lines.push(format!("State:   {} ({})", note, percent.unwrap_or(0)));
                }
            } else {
                lines.push(format!("Current: {}", format_token_count(current_tokens)));
            }
        }
        if ts.total_tokens > 0 {
            lines.push(format!(
                "Workflow: {} cumulative",
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
        if ts.cache_read_tokens > 0 || ts.cache_miss_tokens > 0 || ts.cache_write_tokens > 0 {
            lines.push(format!(
                "Cache:   read {} · miss {} · write {}",
                format_token_count(ts.cache_read_tokens),
                format_token_count(ts.cache_miss_tokens),
                format_token_count(ts.cache_write_tokens)
            ));
        }
        if let Some(cache_diagnostic) = projection.cache_diagnostic.as_deref() {
            lines.push(format!("Cache:   {}", truncate_text(cache_diagnostic, 96)));
        }
        if let Some(ingress_diagnostic) = projection.ingress_diagnostic.as_deref() {
            lines.push(format!(
                "Ingress: {}",
                truncate_text(ingress_diagnostic, 96)
            ));
        }
        if let Some(provider_diagnostic) = projection.provider_diagnostic.as_deref() {
            lines.push(format!(
                "Provider: {}",
                truncate_text(provider_diagnostic, 96)
            ));
        }
        #[cfg(test)]
        if let Some(model) = model_info {
            if let (Some(input_price), Some(output_price)) =
                (model.cost_per_million_input, model.cost_per_million_output)
            {
                lines.push(format!(
                    "Price:   {}",
                    super::session_projection_usage::cli_format_price_pair(
                        input_price,
                        output_price,
                    )
                ));
            }
        }
        lines.push(format!("Cost:    ${:.4}", ts.total_cost));
    }

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

    if !projection.lsp_servers.is_empty() {
        lines.push(String::new());
        lines.push(format!("─ LSP ({}) ─", projection.lsp_servers.len()));
        for server in &projection.lsp_servers {
            lines.push(format!("● {}", server));
        }
    }

    lines.push(String::new());
    lines.push("/help · /model · /preset".to_string());
    lines.push("/runtime · /usage · /insights · /validation".to_string());
    lines.push("/events".to_string());
    lines.push("/events next · /events prev · /events page <n>".to_string());
    lines.push("/events first · /events clear".to_string());
    lines.push("/attached · /abort · /status".to_string());
    lines
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
    if let Some(ref attached_id) = stage.attached_session_id {
        lines.push(format!("→ Attached session: {}", attached_id));
    }
    lines
}

#[cfg(test)]
fn cli_messages_footer(
    transcript: &CliVisibleTranscript,
    width: usize,
    max_rows: usize,
    scroll_offset: usize,
) -> String {
    let total = transcript.total_rows(width);
    if total <= max_rows {
        return "visible transcript".to_string();
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
pub(super) fn cli_render_retained_layout(
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
    let header_box = cli_render_box(
        crate::branding::APP_SHORT_NAME,
        None,
        &header_lines,
        total_width,
        style,
    );

    let active_inner_width = total_width.saturating_sub(4).max(1);
    let (active_content_lines, active_chrome) = if projection.active_collapsed {
        (Vec::new(), 3usize)
    } else {
        let raw_lines = cli_active_stage_panel_lines(projection.active_stage.as_ref(), style);
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
        (raw_lines, 2 + natural_rows.clamp(2, 12))
    };

    let prompt_overhead = 8usize;
    let total_chrome = 3 + 2 + active_chrome + prompt_overhead;
    let sidebar_overhead = if projection.sidebar_collapsed { 3 } else { 0 };
    let body_rows = terminal_rows.saturating_sub(total_chrome).max(4) + sidebar_overhead;

    let mut screen = Vec::new();
    screen.extend(header_box);

    if projection.sidebar_collapsed {
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
        let active_rows = active_chrome.saturating_sub(2);
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
mod tests {
    use super::{cli_sidebar_lines, CliFrontendProjection, CliObservedExecutionTopology};

    #[test]
    fn sidebar_surfaces_cache_diagnostic_without_token_totals() {
        let mut projection = CliFrontendProjection::default();
        projection.cache_diagnostic = Some("cache explained".to_string());
        let topology = CliObservedExecutionTopology {
            active: false,
            root_id: None,
            scheduler_id: None,
            active_stage_id: None,
            stage_order: Vec::new(),
            nodes: Default::default(),
        };

        let lines = cli_sidebar_lines(&projection, &topology);

        assert!(lines
            .iter()
            .any(|line| line.contains("Cache:") && line.contains("cache explained")));
    }

    #[test]
    fn sidebar_surfaces_provider_diagnostic_without_token_totals() {
        let mut projection = CliFrontendProjection::default();
        projection.provider_diagnostic = Some("thinking replay rejected".to_string());
        let topology = CliObservedExecutionTopology {
            active: false,
            root_id: None,
            scheduler_id: None,
            active_stage_id: None,
            stage_order: Vec::new(),
            nodes: Default::default(),
        };

        let lines = cli_sidebar_lines(&projection, &topology);

        assert!(lines
            .iter()
            .any(|line| line.contains("Provider:") && line.contains("thinking replay rejected")));
    }
}
