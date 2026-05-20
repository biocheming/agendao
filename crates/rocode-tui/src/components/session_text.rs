use std::collections::HashMap;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use rocode_command::output_blocks::SchedulerStageBlock;
use rocode_orchestrator::parse_execution_gate_decision;
use serde_json::Value;

use super::markdown::MarkdownRenderer;
use crate::{context::Message, theme::Theme};

pub const ASSISTANT_MARKER: &str = "▶ ";
const LEGACY_SYSTEM_REMINDER_PREFIX: &str = "System Reminder Sent:";
const LOADED_INSTRUCTION_FILES_PREFIX: &str = "Loaded instruction files:";

pub struct MessageTextRender {
    pub lines: Vec<Line<'static>>,
    pub allow_semantic_highlighting: bool,
    pub source_len: usize,
}

#[cfg(test)]
pub fn render_message_text_part(
    message: &Message,
    text: &str,
    theme: &Theme,
    marker_color: Color,
) -> MessageTextRender {
    render_message_text_part_with_width(message, text, theme, marker_color, None)
}

pub(super) fn render_message_text_part_with_width(
    message: &Message,
    text: &str,
    theme: &Theme,
    marker_color: Color,
    max_width: Option<u16>,
) -> MessageTextRender {
    let metadata = message.metadata.as_ref();

    if let Some(stage) = scheduler_stage(metadata) {
        if let Some(lines) = render_decision_stage_part(text, stage, metadata, theme, marker_color, max_width)
        {
            return MessageTextRender {
                lines,
                allow_semantic_highlighting: false,
                source_len: text.len(),
            };
        }

        let lines = render_scheduler_stage_part(text, stage, metadata, theme, marker_color, max_width);
        return MessageTextRender {
            lines,
            allow_semantic_highlighting: false,
            source_len: text.len(),
        };
    }

    MessageTextRender {
        lines: render_text_part_with_width(text, theme, marker_color, max_width),
        allow_semantic_highlighting: true,
        source_len: text.len(),
    }
}

#[cfg(test)]
pub fn render_text_part(text: &str, theme: &Theme, marker_color: Color) -> Vec<Line<'static>> {
    render_text_part_with_width(text, theme, marker_color, None)
}

fn render_text_part_with_width(
    text: &str,
    theme: &Theme,
    marker_color: Color,
    max_width: Option<u16>,
) -> Vec<Line<'static>> {
    let cleaned = strip_think_tags(text);
    let renderer = MarkdownRenderer::new(theme.clone());
    let mut lines = Vec::new();
    let mut markdown_buffer = Vec::new();

    for raw_line in cleaned.lines() {
        if is_system_reminder_display_line(raw_line) {
            flush_markdown_buffer(&renderer, &mut lines, &mut markdown_buffer, max_width);
            if !lines.last().is_none_or(line_is_blank) {
                lines.push(Line::from(""));
            }
            lines.push(render_system_reminder_line(raw_line, theme));
        } else {
            markdown_buffer.push(raw_line.to_string());
        }
    }

    flush_markdown_buffer(&renderer, &mut lines, &mut markdown_buffer, max_width);
    apply_assistant_marker(lines, marker_color)
}

pub struct ReasoningRender {
    pub lines: Vec<Line<'static>>,
    pub collapsible: bool,
}

pub(super) fn render_reasoning_part_with_width(
    text: &str,
    theme: &Theme,
    collapsed: bool,
    preview_lines: usize,
    max_width: Option<u16>,
) -> ReasoningRender {
    let cleaned = strip_think_tags(&text.replace("[REDACTED]", ""))
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return ReasoningRender {
            lines: Vec::new(),
            collapsible: false,
        };
    }

    let mut lines = Vec::new();
    let renderer = MarkdownRenderer::new(theme.clone()).with_concealed(true);
    let content_lines = renderer.to_lines(&cleaned, max_width);
    let total_content_lines = content_lines.len();
    let collapsible = total_content_lines > preview_lines;

    let header_style = Style::default()
        .fg(theme.text_muted)
        .add_modifier(Modifier::BOLD | Modifier::ITALIC);
    let body_prefix_style = Style::default().fg(theme.border_subtle);
    let body_style = Style::default()
        .fg(theme.text_muted)
        .add_modifier(Modifier::ITALIC);

    if collapsible && collapsed {
        lines.push(Line::from(Span::styled(
            format!("▶ reasoning · {} lines", total_content_lines),
            header_style,
        )));
        return ReasoningRender { lines, collapsible };
    }

    lines.push(Line::from(Span::styled("▼ reasoning", header_style)));

    let visible_count = if collapsible && collapsed {
        preview_lines
    } else {
        total_content_lines
    };

    for line in content_lines.into_iter().take(visible_count) {
        let mut spans = vec![Span::styled("┆ ", body_prefix_style)];
        spans.extend(line.spans.into_iter().map(|span| {
            Span::styled(
                span.content,
                span.style
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::ITALIC),
            )
        }));
        lines.push(Line::from(spans));
    }

    if collapsible {
        lines.push(Line::from(Span::styled("┆ collapse", body_style)));
    }

    ReasoningRender { lines, collapsible }
}

// ---------------------------------------------------------------------------
// Stage card header (shared by all non-route scheduler stages)
// ---------------------------------------------------------------------------

struct StageDecoration {
    icon: &'static str,
    label: &'static str,
    color_fn: fn(&Theme) -> Color,
}

fn stage_decoration(stage: &str) -> StageDecoration {
    match stage {
        "route" => StageDecoration {
            icon: "◈",
            label: "Route",
            color_fn: |t| t.info,
        },
        "interview" => StageDecoration {
            icon: "❓",
            label: "Interview",
            color_fn: |t| t.warning,
        },
        "plan" => StageDecoration {
            icon: "📋",
            label: "Plan",
            color_fn: |t| t.info,
        },
        "delegation" => StageDecoration {
            icon: "📤",
            label: "Delegation",
            color_fn: |t| t.secondary,
        },
        "review" => StageDecoration {
            icon: "🔍",
            label: "Review",
            color_fn: |t| t.warning,
        },
        "execution-orchestration" => StageDecoration {
            icon: "⚡",
            label: "Execution",
            color_fn: |t| t.primary,
        },
        "coordination-verification" => StageDecoration {
            icon: "🧪",
            label: "Coordination Verification",
            color_fn: |t| t.warning,
        },
        "coordination-gate" => StageDecoration {
            icon: "🚦",
            label: "Coordination Gate",
            color_fn: |t| t.info,
        },
        "coordination-retry" => StageDecoration {
            icon: "↺",
            label: "Coordination Retry",
            color_fn: |t| t.secondary,
        },
        "autonomous-verification" => StageDecoration {
            icon: "🧪",
            label: "Autonomous Verification",
            color_fn: |t| t.warning,
        },
        "autonomous-gate" => StageDecoration {
            icon: "🚦",
            label: "Autonomous Gate",
            color_fn: |t| t.info,
        },
        "autonomous-retry" => StageDecoration {
            icon: "↺",
            label: "Autonomous Retry",
            color_fn: |t| t.secondary,
        },
        "synthesis" => StageDecoration {
            icon: "✦",
            label: "Synthesis",
            color_fn: |t| t.success,
        },
        "handoff" => StageDecoration {
            icon: "📎",
            label: "Handoff",
            color_fn: |t| t.secondary,
        },
        _ => StageDecoration {
            icon: "◈",
            label: "Stage",
            color_fn: |t| t.primary,
        },
    }
}

fn render_stage_header(
    profile: &str,
    stage: &str,
    stage_index: Option<u64>,
    stage_total: Option<u64>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let decoration = stage_decoration(stage);
    let accent = (decoration.color_fn)(theme);

    let mut title_spans = vec![
        Span::styled(format!("{} ", decoration.icon), Style::default().fg(accent)),
        Span::styled(
            format!("{} · {}", prettify_token(profile), decoration.label),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
    ];

    if let (Some(index), Some(total)) = (stage_index, stage_total) {
        title_spans.push(Span::styled(
            format!("  [{}/{}]", index, total),
            Style::default().fg(theme.text_muted),
        ));
    }

    let separator = "─".repeat(40);

    vec![
        Line::from(title_spans),
        Line::from(Span::styled(
            separator,
            Style::default().fg(theme.border_subtle),
        )),
    ]
}

fn render_scheduler_stage_part(
    text: &str,
    _stage: &str,
    metadata: Option<&HashMap<String, Value>>,
    theme: &Theme,
    marker_color: Color,
    max_width: Option<u16>,
) -> Vec<Line<'static>> {
    let block = metadata.and_then(|m| SchedulerStageBlock::from_metadata(text, m));
    let profile = block
        .as_ref()
        .and_then(|b| b.profile.as_deref())
        .unwrap_or("scheduler");
    let stage_index = block.as_ref().and_then(|b| b.stage_index);
    let stage_total = block.as_ref().and_then(|b| b.stage_total);
    let stage_name = block.as_ref().map(|b| b.stage.as_str()).unwrap_or(_stage);

    let mut lines = render_stage_header(profile, stage_name, stage_index, stage_total, theme);
    if let Some(ref blk) = block {
        lines.extend(render_stage_runtime_lines(blk, theme));
    }

    let body = block.as_ref().map(|b| b.text.as_str()).unwrap_or(text);
    if let Some(card) = generic_stage_card_from_text(body) {
        lines.push(Line::from(vec![
            Span::styled("◈ ", Style::default().fg(theme.info)),
            Span::styled(
                card.title.clone(),
                Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
            ),
        ]));
        append_decision_card_body(&mut lines, card, theme, max_width);
        return apply_assistant_marker(lines, marker_color);
    }

    let cleaned = strip_think_tags(body);
    let renderer = MarkdownRenderer::new(theme.clone());
    let body_lines = renderer.to_lines(&cleaned, max_width);
    lines.extend(body_lines);

    apply_assistant_marker(lines, marker_color)
}

struct DecisionField {
    label: String,
    value: String,
    tone: Option<String>,
}

struct DecisionSection {
    title: String,
    body: String,
}

struct DecisionCard {
    title: String,
    spec: DecisionRenderSpec,
    fields: Vec<DecisionField>,
    sections: Vec<DecisionSection>,
}

struct DecisionRenderSpec {
    show_header_divider: bool,
    field_label_emphasis: String,
    section_spacing: String,
}

fn render_decision_stage_part(
    text: &str,
    stage: &str,
    metadata: Option<&HashMap<String, Value>>,
    theme: &Theme,
    marker_color: Color,
    max_width: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    let decision = decision_card_from_message(stage, text, metadata?)?;

    let block = metadata.and_then(|m| SchedulerStageBlock::from_metadata(text, m));
    let profile = block
        .as_ref()
        .and_then(|b| b.profile.as_deref())
        .unwrap_or("scheduler");
    let stage_index = block.as_ref().and_then(|b| b.stage_index);
    let stage_total = block.as_ref().and_then(|b| b.stage_total);

    let mut lines = render_stage_header(profile, stage, stage_index, stage_total, theme);
    if !decision.spec.show_header_divider && lines.len() > 1 {
        lines.pop();
    }
    if let Some(ref blk) = block {
        lines.extend(render_stage_runtime_lines(blk, theme));
    }

    lines.push(Line::from(vec![
        Span::styled("◈ ", Style::default().fg(theme.info)),
        Span::styled(
            decision.title.clone(),
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
    ]));

    append_decision_card_body(&mut lines, decision, theme, max_width);

    Some(apply_assistant_marker(lines, marker_color))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn route_field_line(
    label: &str,
    value: &str,
    theme: &Theme,
    spec: &DecisionRenderSpec,
    value_style: Style,
) -> Line<'static> {
    let label_style = if spec.field_label_emphasis == "bold" {
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.primary)
    };
    Line::from(vec![
        Span::styled("• ", Style::default().fg(theme.border_active)),
        Span::styled(format!("{label}: "), label_style),
        Span::styled(value.to_string(), value_style),
    ])
}

fn append_decision_card_body(
    lines: &mut Vec<Line<'static>>,
    decision: DecisionCard,
    theme: &Theme,
    max_width: Option<u16>,
) {
    for field in decision.fields {
        lines.push(route_field_line(
            &field.label,
            &field.value,
            theme,
            &decision.spec,
            decision_field_style(field.tone.as_deref(), &field.value, theme),
        ));
    }
    for section in decision.sections {
        if decision.spec.section_spacing == "loose" {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled("✦ ", Style::default().fg(theme.secondary)),
            Span::styled(
                section.title,
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        let renderer = MarkdownRenderer::new(theme.clone());
        for line in renderer.to_lines(&section.body, max_width) {
            let mut spans = vec![Span::styled("  ", Style::default().fg(theme.text_muted))];
            spans.extend(line.spans);
            lines.push(Line::from(spans));
        }
    }
}

fn decision_field_style(tone: Option<&str>, value: &str, theme: &Theme) -> Style {
    match tone {
        Some("success") => Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD),
        Some("warning") => Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
        Some("error") => Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
        Some("info") => Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        Some("muted") => Style::default().fg(theme.text_muted),
        Some("status") => match value.to_ascii_lowercase().as_str() {
            "done" => Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
            "blocked" => Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
            _ => Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        },
        _ => Style::default().fg(theme.text),
    }
}

fn render_stage_runtime_lines(block: &SchedulerStageBlock, theme: &Theme) -> Vec<Line<'static>> {
    let status = block.status.as_deref().unwrap_or("running");
    let step = block.step;
    let step_limit = block.loop_budget.as_deref().and_then(parse_step_limit);
    let waiting_on = block.waiting_on.as_deref().unwrap_or("none");
    let focus = block.focus.as_deref().filter(|v| !v.trim().is_empty());
    let last_event = block.last_event.as_deref().filter(|v| !v.trim().is_empty());
    let activity = block.activity.as_deref().filter(|v| !v.trim().is_empty());

    let status_style = match status {
        "done" => Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD),
        "cancelled" => Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
        "cancelling" => Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
        "waiting" => Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
        "blocked" => Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
        "retrying" => Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
    };
    let label_style = Style::default().fg(theme.text_muted);
    let value_style = Style::default().fg(theme.text);
    let status_icon = match status {
        "done" => "+",
        "cancelled" => "x",
        "cancelling" => "~",
        "waiting" => "?",
        "blocked" => "!",
        _ => "@",
    };

    let step_label = match (step, step_limit) {
        (Some(current), Some(limit)) => format!("{current}/{limit}"),
        (Some(current), None) => current.to_string(),
        (None, _) => "starting".to_string(),
    };
    let prompt_tokens = block.prompt_tokens;
    let completion_tokens = block.completion_tokens;
    let reasoning_tokens = block.reasoning_tokens;
    let cache_read_tokens = block.cache_read_tokens;
    let cache_miss_tokens = block.cache_miss_tokens;
    let cache_write_tokens = block.cache_write_tokens;

    let mut status_row = vec![
        Span::styled("  Status ", label_style),
        Span::styled(
            format!("{status_icon} {}", prettify_token(status)),
            status_style,
        ),
        Span::styled("   Step ", label_style),
        Span::styled(step_label, value_style),
        Span::styled("   Waiting ", label_style),
        Span::styled(prettify_token(waiting_on), value_style),
    ];
    status_row.push(Span::styled("   Tokens ", label_style));
    status_row.push(Span::styled(
        format!(
            "{}/{}",
            prompt_tokens
                .map(format_compact_number)
                .unwrap_or_else(|| "—".to_string()),
            completion_tokens
                .map(format_compact_number)
                .unwrap_or_else(|| "—".to_string())
        ),
        value_style,
    ));

    let mut lines = vec![Line::from(status_row)];

    let mut usage_parts = Vec::new();
    if let Some(reasoning_tokens) = reasoning_tokens {
        usage_parts.push(format!(
            "reasoning {}",
            format_compact_number(reasoning_tokens)
        ));
    }
    if let Some(cache_read_tokens) = cache_read_tokens {
        usage_parts.push(format!(
            "cache read {}",
            format_compact_number(cache_read_tokens)
        ));
    }
    if let Some(cache_miss_tokens) = cache_miss_tokens {
        usage_parts.push(format!(
            "cache miss {}",
            format_compact_number(cache_miss_tokens)
        ));
    }
    if let Some(cache_write_tokens) = cache_write_tokens {
        usage_parts.push(format!(
            "cache write {}",
            format_compact_number(cache_write_tokens)
        ));
    }
    if !usage_parts.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  Usage ", label_style),
            Span::styled(usage_parts.join(" · "), value_style.fg(theme.text_muted)),
        ]));
    }

    if let Some(focus) = focus {
        lines.push(Line::from(vec![
            Span::styled("  Focus ", label_style),
            Span::styled(focus.to_string(), value_style),
        ]));
    }

    if let Some(last_event) = last_event {
        lines.push(Line::from(vec![
            Span::styled("  Last ", label_style),
            Span::styled(last_event.to_string(), value_style),
        ]));
    }

    if let Some(activity) = activity {
        lines.push(Line::from(vec![
            Span::styled("  Activity ", label_style),
            Span::styled(
                activity.lines().next().unwrap_or_default().to_string(),
                value_style,
            ),
        ]));
        for line in activity.lines().skip(1) {
            lines.push(Line::from(vec![
                Span::styled("           ", label_style),
                Span::styled(line.to_string(), value_style),
            ]));
        }
    }

    let attached_session_id = block
        .attached_session_id
        .as_deref()
        .filter(|v| !v.trim().is_empty());
    if let Some(attached_id) = attached_session_id {
        lines.push(Line::from(vec![
            Span::styled("  → Session ", label_style),
            Span::styled(
                attached_id.to_string(),
                Style::default()
                    .fg(theme.info)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled("  (Ctrl+J to view)", Style::default().fg(theme.text_muted)),
        ]));
    }

    let available_skill_count = block.available_skill_count;
    let available_agent_count = block.available_agent_count;
    let available_category_count = block.available_category_count;
    let active_skills = if block.active_skills.is_empty() {
        None
    } else {
        Some(&block.active_skills)
    };
    let active_agents = if block.active_agents.is_empty() {
        None
    } else {
        Some(&block.active_agents)
    };
    let active_categories = if block.active_categories.is_empty() {
        None
    } else {
        Some(&block.active_categories)
    };

    if available_skill_count.is_some()
        || available_agent_count.is_some()
        || available_category_count.is_some()
        || active_skills.is_some()
        || active_agents.is_some()
        || active_categories.is_some()
    {
        let cap_label_style = Style::default().fg(theme.text_muted);
        let cap_value_style = Style::default().fg(theme.info);

        let mut available_parts = Vec::new();
        if let Some(count) = available_skill_count {
            available_parts.push(format!("skills {count}"));
        }
        if let Some(count) = available_agent_count {
            available_parts.push(format!("agents {count}"));
        }
        if let Some(count) = available_category_count {
            available_parts.push(format!("categories {count}"));
        }
        if !available_parts.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Available ", cap_label_style),
                Span::styled(available_parts.join(" · "), cap_value_style),
            ]));
        }

        if let Some(skills) = active_skills {
            lines.push(Line::from(vec![
                Span::styled("  Active Skills ", cap_label_style),
                Span::styled(skills.join(", "), cap_value_style),
            ]));
        }
        if let Some(agents) = active_agents {
            lines.push(Line::from(vec![
                Span::styled("  Active Agents ", cap_label_style),
                Span::styled(agents.join(", "), cap_value_style),
            ]));
        }
        if let Some(categories) = active_categories {
            lines.push(Line::from(vec![
                Span::styled("  Active Categories ", cap_label_style),
                Span::styled(categories.join(", "), cap_value_style),
            ]));
        }
    }

    if block.total_agent_count > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  Agents [{}/{}]",
                block.done_agent_count, block.total_agent_count
            ),
            Style::default().fg(theme.info),
        )]));
    }

    lines.push(Line::from(""));
    lines
}

fn parse_step_limit(loop_budget: &str) -> Option<u64> {
    loop_budget
        .strip_prefix("step-limit:")
        .and_then(|value| value.parse::<u64>().ok())
}

fn apply_assistant_marker(lines: Vec<Line<'static>>, marker_color: Color) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let marker = if idx == 0 { ASSISTANT_MARKER } else { "  " };
            let mut spans = vec![Span::styled(marker, Style::default().fg(marker_color))];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

fn flush_markdown_buffer(
    renderer: &MarkdownRenderer,
    lines: &mut Vec<Line<'static>>,
    markdown_buffer: &mut Vec<String>,
    max_width: Option<u16>,
) {
    if markdown_buffer.is_empty() {
        return;
    }

    let markdown = markdown_buffer.join("\n");
    markdown_buffer.clear();
    if markdown.trim().is_empty() {
        return;
    }

    lines.extend(renderer.to_lines(&markdown, max_width));
}

fn is_system_reminder_display_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with(LEGACY_SYSTEM_REMINDER_PREFIX)
        || line.starts_with(LOADED_INSTRUCTION_FILES_PREFIX)
}

fn render_system_reminder_line(line: &str, theme: &Theme) -> Line<'static> {
    let reminder = line.trim();
    let detail = reminder
        .strip_prefix(LOADED_INSTRUCTION_FILES_PREFIX)
        .or_else(|| reminder.strip_prefix(LEGACY_SYSTEM_REMINDER_PREFIX))
        .map(str::trim)
        .unwrap_or_default();

    let mut spans = vec![
        Span::styled("↳ ", Style::default().fg(theme.border_subtle)),
        Span::styled(
            "Instruction Files",
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
    ];
    if !detail.is_empty() {
        spans.push(Span::styled("  ", Style::default().fg(theme.border_subtle)));
        spans.push(Span::styled(
            detail.to_string(),
            Style::default().fg(theme.text_muted),
        ));
    }
    Line::from(spans)
}

fn line_is_blank(line: &Line<'static>) -> bool {
    line.spans.iter().all(|span| span.content.trim().is_empty())
}

fn scheduler_stage(metadata: Option<&HashMap<String, Value>>) -> Option<&str> {
    metadata
        .and_then(|m| m.get("scheduler_stage"))
        .and_then(Value::as_str)
}

fn split_stage_heading(text: &str) -> (Option<&str>, &str) {
    if let Some(rest) = text.strip_prefix("## ") {
        if let Some((title, body)) = rest.split_once("\n\n") {
            return (Some(title.trim()), body);
        }
        if let Some((title, body)) = rest.split_once('\n') {
            return (Some(title.trim()), body);
        }
    }

    (None, text)
}

fn parse_route_decision_value(text: &str) -> Option<Value> {
    let candidate = extract_json_candidate(text)?;
    serde_json::from_str(candidate).ok()
}

fn extract_json_candidate(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(json_block) = extract_fenced_json_block(trimmed) {
        return Some(json_block);
    }

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }

    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

fn extract_fenced_json_block(text: &str) -> Option<&str> {
    for fence in ["```json", "```JSON", "```"] {
        if let Some(start) = text.find(fence) {
            let rest = &text[start + fence.len()..];
            if let Some(end) = rest.find("```") {
                return Some(rest[..end].trim());
            }
        }
    }
    None
}

fn route_string_field<'a>(decision: &'a Value, key: &str) -> Option<&'a str> {
    decision.get(key).and_then(Value::as_str)
}

fn decision_card_from_message(
    stage: &str,
    text: &str,
    metadata: &HashMap<String, Value>,
) -> Option<DecisionCard> {
    decision_card_from_metadata(metadata).or_else(|| decision_card_from_text(stage, text, metadata))
}

fn decision_card_from_metadata(metadata: &HashMap<String, Value>) -> Option<DecisionCard> {
    let title = metadata
        .get("scheduler_decision_title")
        .and_then(Value::as_str)?
        .to_string();
    Some(DecisionCard {
        title,
        spec: decision_spec_from_metadata(metadata).unwrap_or_else(default_decision_render_spec),
        fields: metadata
            .get("scheduler_decision_fields")
            .and_then(Value::as_array)
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| {
                        Some(DecisionField {
                            label: field.get("label")?.as_str()?.to_string(),
                            value: field.get("value")?.as_str()?.to_string(),
                            tone: field
                                .get("tone")
                                .and_then(Value::as_str)
                                .map(|value| value.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sections: metadata
            .get("scheduler_decision_sections")
            .and_then(Value::as_array)
            .map(|sections| {
                sections
                    .iter()
                    .filter_map(|section| {
                        Some(DecisionSection {
                            title: section.get("title")?.as_str()?.to_string(),
                            body: section.get("body")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn decision_card_from_text(
    stage: &str,
    text: &str,
    metadata: &HashMap<String, Value>,
) -> Option<DecisionCard> {
    let (_title, body) = split_stage_heading(text);
    match stage {
        "route" => {
            let decision = parse_route_decision_value(body.trim())?;
            let outcome_label = route_outcome_label_from_value(&decision);
            let mut fields = vec![DecisionField {
                label: "Outcome".to_string(),
                value: outcome_label,
                tone: Some(match route_string_field(&decision, "mode") {
                    Some("direct") => "warning".to_string(),
                    Some("orchestrate") => "success".to_string(),
                    _ => "info".to_string(),
                }),
            }];
            if let Some(preset) = route_string_field(&decision, "preset") {
                fields.push(DecisionField {
                    label: "Preset".to_string(),
                    value: prettify_token(preset),
                    tone: Some("info".to_string()),
                });
            }
            if let Some(review_mode) = route_string_field(&decision, "review_mode") {
                fields.push(DecisionField {
                    label: "Review".to_string(),
                    value: prettify_token(review_mode),
                    tone: Some(if review_mode == "skip" {
                        "warning".to_string()
                    } else {
                        "success".to_string()
                    }),
                });
            }
            if let Some(insert_plan_stage) =
                decision.get("insert_plan_stage").and_then(Value::as_bool)
            {
                fields.push(DecisionField {
                    label: "Plan Stage".to_string(),
                    value: if insert_plan_stage { "Yes" } else { "No" }.to_string(),
                    tone: Some(if insert_plan_stage {
                        "success".to_string()
                    } else {
                        "muted".to_string()
                    }),
                });
            }
            if let Some(reason) = route_string_field(&decision, "rationale_summary")
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                fields.push(DecisionField {
                    label: "Why".to_string(),
                    value: reason.to_string(),
                    tone: None,
                });
            }
            let mut sections = Vec::new();
            if let Some(context) = route_string_field(&decision, "context_append")
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sections.push(DecisionSection {
                    title: "Appended Context".to_string(),
                    body: context.to_string(),
                });
            }
            if let Some(response) = route_string_field(&decision, "direct_response")
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sections.push(DecisionSection {
                    title: "Response".to_string(),
                    body: response.to_string(),
                });
            }
            Some(DecisionCard {
                title: "Decision".to_string(),
                spec: default_decision_render_spec(),
                fields,
                sections,
            })
        }
        "coordination-gate" | "autonomous-gate" => {
            let decision = metadata
                .get("scheduler_gate_status")
                .and_then(Value::as_str)
                .map(
                    |status| rocode_orchestrator::SchedulerExecutionGateDecision {
                        status: match status {
                            "done" => rocode_orchestrator::SchedulerExecutionGateStatus::Done,
                            "continue" => {
                                rocode_orchestrator::SchedulerExecutionGateStatus::Continue
                            }
                            _ => rocode_orchestrator::SchedulerExecutionGateStatus::Blocked,
                        },
                        summary: metadata
                            .get("scheduler_gate_summary")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        next_input: metadata
                            .get("scheduler_gate_next_input")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        final_response: metadata
                            .get("scheduler_gate_final_response")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    },
                )
                .or_else(|| parse_execution_gate_decision(body.trim()))?;
            let status = format!("{:?}", decision.status).to_ascii_lowercase();
            let mut fields = vec![DecisionField {
                label: "Outcome".to_string(),
                value: gate_outcome_label_from_status(&status),
                tone: Some("status".to_string()),
            }];
            if !decision.summary.is_empty() {
                fields.push(DecisionField {
                    label: "Why".to_string(),
                    value: decision.summary,
                    tone: None,
                });
            }
            if let Some(next_input) = decision.next_input.filter(|value| !value.is_empty()) {
                fields.push(DecisionField {
                    label: "Next Action".to_string(),
                    value: next_input,
                    tone: Some("warning".to_string()),
                });
            }
            Some(DecisionCard {
                title: "Decision".to_string(),
                spec: default_decision_render_spec(),
                fields,
                sections: Vec::new(),
            })
        }
        _ => None,
    }
}

fn generic_stage_card_from_text(text: &str) -> Option<DecisionCard> {
    let (_title, body) = split_stage_heading(text);
    let value = parse_route_decision_value(body.trim())?;
    let object = value.as_object()?;
    if object.is_empty() {
        return None;
    }

    let mut fields = Vec::new();
    let mut sections = Vec::new();

    for (key, value) in object {
        if value.is_null() {
            continue;
        }

        let label = prettify_token(key);
        if let Some(field_value) = json_value_as_field(key, value) {
            fields.push(DecisionField {
                label,
                value: field_value,
                tone: generic_json_field_tone(key, value),
            });
            continue;
        }

        let body = json_value_as_markdown(value);
        if body.trim().is_empty() {
            continue;
        }
        sections.push(DecisionSection { title: label, body });
    }

    if fields.is_empty() && sections.is_empty() {
        return None;
    }

    Some(DecisionCard {
        title: "Details".to_string(),
        spec: default_decision_render_spec(),
        fields,
        sections,
    })
}

fn json_value_as_field(key: &str, value: &Value) -> Option<String> {
    match value {
        Value::Bool(boolean) => Some(if *boolean { "Yes" } else { "No" }.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.contains('\n') || trimmed.chars().count() > 100 {
                None
            } else {
                Some(render_scalar_field_value(key, trimmed))
            }
        }
        Value::Array(items) if !items.is_empty() && items.iter().all(is_inline_json_value) => {
            let rendered = items
                .iter()
                .map(render_inline_json_value)
                .collect::<Vec<_>>()
                .join(", ");
            (rendered.chars().count() <= 120).then_some(rendered)
        }
        _ => None,
    }
}

fn is_inline_json_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Bool(_) | Value::Null | Value::Number(_) | Value::String(_)
    )
}

fn render_inline_json_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(boolean) => {
            if *boolean {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.trim().to_string(),
        _ => value.to_string(),
    }
}

fn render_scalar_field_value(key: &str, value: &str) -> String {
    if should_prettify_scalar_value(key, value) {
        prettify_token(value)
    } else {
        value.to_string()
    }
}

fn should_prettify_scalar_value(key: &str, value: &str) -> bool {
    (key == "status"
        || key.ends_with("_status")
        || key == "mode"
        || key.ends_with("_mode")
        || key.ends_with("_strategy"))
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch == '-' || ch == '_')
}

fn generic_json_field_tone(key: &str, value: &Value) -> Option<String> {
    if key == "status" || key.ends_with("_status") {
        return Some("status".to_string());
    }
    if key.contains("error") {
        return Some("error".to_string());
    }
    if key.contains("warning") {
        return Some("warning".to_string());
    }
    if matches!(value, Value::Bool(true)) {
        return Some("success".to_string());
    }
    None
}

fn json_value_as_markdown(value: &Value) -> String {
    let mut lines = Vec::new();
    append_json_markdown_lines(value, &mut lines, 0);
    lines.join("\n")
}

fn append_json_markdown_lines(value: &Value, lines: &mut Vec<String>, indent: usize) {
    let prefix = " ".repeat(indent);
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                let label = prettify_token(key);
                if let Some(field_value) = json_value_as_field(key, nested) {
                    lines.push(format!("{prefix}- {label}: {field_value}"));
                } else {
                    lines.push(format!("{prefix}- {label}"));
                    append_json_markdown_lines(nested, lines, indent + 2);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if is_inline_json_value(item) {
                    lines.push(format!("{prefix}- {}", render_inline_json_value(item)));
                } else {
                    lines.push(format!("{prefix}-"));
                    append_json_markdown_lines(item, lines, indent + 2);
                }
            }
        }
        Value::String(text) => {
            for line in text.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    lines.push(format!("{prefix}{trimmed}"));
                }
            }
        }
        _ => lines.push(format!("{prefix}{}", render_inline_json_value(value))),
    }
}

fn prettify_token(raw: &str) -> String {
    raw.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        let compact = value as f64 / 1_000_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}M")
        } else {
            format!("{compact:.1}M")
        };
    }
    if value >= 1_000 {
        let compact = value as f64 / 1_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}K")
        } else {
            format!("{compact:.1}K")
        };
    }
    value.to_string()
}

fn route_outcome_label_from_value(decision: &Value) -> String {
    match route_string_field(decision, "mode") {
        Some("direct") => match route_string_field(decision, "direct_kind") {
            Some("reply") => "Direct Reply".to_string(),
            Some("clarify") => "Direct Clarification".to_string(),
            _ => "Direct".to_string(),
        },
        Some("orchestrate") => "Orchestrate".to_string(),
        _ => "Unknown".to_string(),
    }
}

fn gate_outcome_label_from_status(status: &str) -> String {
    match status {
        "continue" => "Continue".to_string(),
        "done" => "Done".to_string(),
        "blocked" => "Blocked".to_string(),
        other => prettify_token(other),
    }
}

fn decision_spec_from_metadata(metadata: &HashMap<String, Value>) -> Option<DecisionRenderSpec> {
    let spec = metadata.get("scheduler_decision_spec")?;
    Some(DecisionRenderSpec {
        show_header_divider: spec.get("show_header_divider")?.as_bool()?,
        field_label_emphasis: spec.get("field_label_emphasis")?.as_str()?.to_string(),
        section_spacing: spec.get("section_spacing")?.as_str()?.to_string(),
    })
}

fn default_decision_render_spec() -> DecisionRenderSpec {
    DecisionRenderSpec {
        show_header_divider: true,
        field_label_emphasis: "bold".to_string(),
        section_spacing: "loose".to_string(),
    }
}

fn strip_think_tags(text: &str) -> String {
    text.replace("<think>", "")
        .replace("</think>", "")
        .replace("<think/>", "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{MessageRole, TokenUsage};
    use chrono::Utc;
    use rocode_command::governance_fixtures::canonical_scheduler_stage_fixture;
    use serde_json::json;

    struct StageRuntimeMeta<'a> {
        step: Option<u64>,
        status: Option<&'a str>,
        focus: Option<&'a str>,
        last_event: Option<&'a str>,
        waiting_on: Option<&'a str>,
        activity: Option<&'a str>,
        loop_budget: Option<&'a str>,
    }

    fn message_with_stage(stage: &str) -> Message {
        message_with_stage_meta(stage, None, None, None)
    }

    fn message_with_stage_meta(
        stage: &str,
        profile: Option<&str>,
        index: Option<u64>,
        total: Option<u64>,
    ) -> Message {
        let mut metadata = HashMap::new();
        metadata.insert("scheduler_stage".to_string(), json!(stage));
        if let Some(p) = profile {
            metadata.insert("scheduler_profile".to_string(), json!(p));
        }
        if let Some(i) = index {
            metadata.insert("scheduler_stage_index".to_string(), json!(i));
        }
        if let Some(t) = total {
            metadata.insert("scheduler_stage_total".to_string(), json!(t));
        }
        Message {
            id: "m1".to_string(),
            role: MessageRole::Assistant,
            content: String::new(),
            created_at: Utc::now(),
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            metadata: Some(metadata),
            multimodal: None,
            parts: Vec::new(),
        }
    }

    fn message_with_stage_runtime_meta(
        stage: &str,
        profile: Option<&str>,
        index: Option<u64>,
        total: Option<u64>,
        runtime: StageRuntimeMeta<'_>,
    ) -> Message {
        let mut message = message_with_stage_meta(stage, profile, index, total);
        let metadata = message.metadata.as_mut().expect("metadata should exist");
        if let Some(step) = runtime.step {
            metadata.insert("scheduler_stage_step".to_string(), json!(step));
        }
        if let Some(status) = runtime.status {
            metadata.insert("scheduler_stage_status".to_string(), json!(status));
        }
        if let Some(focus) = runtime.focus {
            metadata.insert("scheduler_stage_focus".to_string(), json!(focus));
        }
        if let Some(last_event) = runtime.last_event {
            metadata.insert("scheduler_stage_last_event".to_string(), json!(last_event));
        }
        if let Some(waiting_on) = runtime.waiting_on {
            metadata.insert("scheduler_stage_waiting_on".to_string(), json!(waiting_on));
        }
        if let Some(activity) = runtime.activity {
            metadata.insert("scheduler_stage_activity".to_string(), json!(activity));
        }
        if let Some(loop_budget) = runtime.loop_budget {
            metadata.insert(
                "scheduler_stage_loop_budget".to_string(),
                json!(loop_budget),
            );
        }
        metadata.insert("scheduler_stage_prompt_tokens".to_string(), json!(1200_u64));
        metadata.insert(
            "scheduler_stage_completion_tokens".to_string(),
            json!(320_u64),
        );
        metadata.insert(
            "scheduler_stage_reasoning_tokens".to_string(),
            json!(40_u64),
        );
        metadata.insert(
            "scheduler_stage_cache_read_tokens".to_string(),
            json!(2_u64),
        );
        metadata.insert(
            "scheduler_stage_cache_miss_tokens".to_string(),
            json!(3_u64),
        );
        metadata.insert(
            "scheduler_stage_cache_write_tokens".to_string(),
            json!(1_u64),
        );
        message.tokens = TokenUsage {
            input: 1200,
            output: 320,
            reasoning: 40,
            cache_read: 0,
            cache_miss: 0,
            cache_write: 0,
        };
        message
    }

    fn message_with_gate_meta(
        stage: &str,
        status: &str,
        summary: &str,
        next_input: Option<&str>,
    ) -> Message {
        let mut message = message_with_stage_runtime_meta(
            stage,
            Some("atlas"),
            Some(2),
            Some(3),
            StageRuntimeMeta {
                step: Some(1),
                status: Some("running"),
                focus: Some("Decide whether the coordination loop can finish."),
                last_event: Some("Stage completed"),
                waiting_on: Some("none"),
                activity: None,
                loop_budget: Some("step-limit:3"),
            },
        );
        let metadata = message.metadata.as_mut().expect("metadata should exist");
        metadata.insert("scheduler_gate_status".to_string(), json!(status));
        metadata.insert("scheduler_gate_summary".to_string(), json!(summary));
        if let Some(next_input) = next_input {
            metadata.insert("scheduler_gate_next_input".to_string(), json!(next_input));
        }
        message
    }

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn first_assistant_line_uses_larger_marker() {
        let lines = render_text_part("hello", &Theme::default(), Color::Blue);
        assert!(line_text(&lines[0]).starts_with(ASSISTANT_MARKER));
    }

    #[test]
    fn render_message_text_part_styles_route_orchestration_decision() {
        let theme = Theme::default();
        let message = message_with_stage_meta("route", Some("prometheus"), Some(1), Some(4));
        let rendered = render_message_text_part(
            &message,
            "## prometheus · Route\n\n```json\n{\n  \"mode\": \"orchestrate\",\n  \"preset\": \"prometheus\",\n  \"review_mode\": \"normal\",\n  \"insert_plan_stage\": true,\n  \"rationale_summary\": \"Needs upfront planning.\"\n}\n```",
            &theme,
            Color::Blue,
        );

        assert!(!rendered.allow_semantic_highlighting);
        // Stage header should be present
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Prometheus · Route"));
        assert!(all_text.contains("[1/4]"));

        let outcome_line = rendered
            .lines
            .iter()
            .find(|line| line_text(line).contains("Outcome: Orchestrate"))
            .expect("outcome line should exist");
        assert_eq!(outcome_line.spans[2].style.fg, Some(theme.primary));
        assert!(outcome_line.spans[2]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(outcome_line.spans[3].style.fg, Some(theme.success));
        assert!(outcome_line.spans[3]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(rendered
            .lines
            .iter()
            .all(|line| !line_text(line).contains("\"mode\"")));
    }

    #[test]
    fn render_message_text_part_styles_route_direct_response_section() {
        let theme = Theme::default();
        let message = message_with_stage("route");
        let rendered = render_message_text_part(
            &message,
            "## router · Route\n\n{\"mode\":\"direct\",\"direct_kind\":\"reply\",\"direct_response\":\"Hi there!\",\"rationale_summary\":\"Greeting\"}",
            &theme,
            Color::Blue,
        );

        let heading = rendered
            .lines
            .iter()
            .find(|line| line_text(line).contains("Response"))
            .expect("response heading should exist");
        assert_eq!(heading.spans[2].style.fg, Some(theme.secondary));
        assert!(heading.spans[2].style.add_modifier.contains(Modifier::BOLD));
        assert!(rendered
            .lines
            .iter()
            .any(|line| line_text(line).contains("Hi there!")));
    }

    #[test]
    fn render_message_text_part_styles_route_context_as_markdown() {
        let theme = Theme::default();
        let message = message_with_stage("route");
        let rendered = render_message_text_part(
            &message,
            "## router · Route\n\n{\"mode\":\"orchestrate\",\"preset\":\"sisyphus\",\"context_append\":\"**important** context\",\"rationale_summary\":\"test\"}",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Appended Context"));
        // The markdown should be rendered, not shown as raw **bold**
        assert!(all_text.contains("important"));
        assert!(!all_text.contains("**important**"));
    }

    #[test]
    fn render_message_text_part_styles_route_orchestration_decision_without_persisted_heading() {
        let theme = Theme::default();
        let message = message_with_stage_meta("route", Some("prometheus"), Some(1), Some(4));
        let rendered = render_message_text_part(
            &message,
            "{\"mode\":\"orchestrate\",\"preset\":\"prometheus\",\"review_mode\":\"normal\",\"insert_plan_stage\":true,\"rationale_summary\":\"Needs upfront planning.\"}",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Prometheus · Route"));
        assert!(all_text.contains("[1/4]"));
        assert!(all_text.contains("Outcome: Orchestrate"));
        assert!(!all_text.contains("\"mode\""));
    }

    #[test]
    fn scheduler_stage_interview_gets_stage_header() {
        let theme = Theme::default();
        let message = message_with_stage_meta("interview", Some("prometheus"), Some(1), Some(4));
        let rendered = render_message_text_part(
            &message,
            "## prometheus · Interview\n\nWhat scope do you want?",
            &theme,
            Color::Blue,
        );

        assert!(!rendered.allow_semantic_highlighting);
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Prometheus · Interview"));
        assert!(all_text.contains("[1/4]"));
        assert!(all_text.contains("What scope do you want?"));
    }

    #[test]
    fn scheduler_stage_plan_gets_stage_header() {
        let theme = Theme::default();
        let message = message_with_stage_meta("plan", Some("prometheus"), Some(2), Some(4));
        let rendered = render_message_text_part(
            &message,
            "## prometheus · Plan\n\n### Step 1\nMigrate schema",
            &theme,
            Color::Blue,
        );

        assert!(!rendered.allow_semantic_highlighting);
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Prometheus · Plan"));
        assert!(all_text.contains("[2/4]"));
        assert!(all_text.contains("Migrate schema"));
    }

    #[test]
    fn scheduler_stage_plan_without_persisted_heading_still_renders_header() {
        let theme = Theme::default();
        let message = message_with_stage_meta("plan", Some("prometheus"), Some(2), Some(4));
        let rendered =
            render_message_text_part(&message, "### Step 1\nMigrate schema", &theme, Color::Blue);

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Prometheus · Plan"));
        assert!(all_text.contains("[2/4]"));
        assert!(all_text.contains("Migrate schema"));
    }

    #[test]
    fn scheduler_stage_synthesis_gets_success_accent() {
        let theme = Theme::default();
        let message = message_with_stage_meta("synthesis", Some("sisyphus"), Some(2), Some(2));
        let rendered = render_message_text_part(
            &message,
            "## sisyphus · Synthesis\n\nAll tasks completed.",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Sisyphus · Synthesis"));
        assert!(all_text.contains("[2/2]"));
        // Synthesis header should use success color
        let header_line = &rendered.lines[0];
        let accent_span = &header_line.spans[2]; // icon span is [1], title span is [2]
        assert_eq!(accent_span.style.fg, Some(theme.success));
    }

    #[test]
    fn scheduler_stage_execution_orchestration_gets_header() {
        let theme = Theme::default();
        let message = message_with_stage_meta(
            "execution-orchestration",
            Some("hephaestus"),
            Some(1),
            Some(1),
        );
        let rendered = render_message_text_part(
            &message,
            "## hephaestus · Execution Orchestration\n\nFixed the bug.",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Hephaestus · Execution"));
        assert!(all_text.contains("[1/1]"));
    }

    #[test]
    fn scheduler_internal_stage_gets_specific_header() {
        let theme = Theme::default();
        let message =
            message_with_stage_meta("coordination-verification", Some("atlas"), Some(1), Some(3));
        let rendered = render_message_text_part(
            &message,
            "## atlas · Coordination Verification\n\nMissing proof for task B.",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Atlas · Coordination Verification"));
        assert!(all_text.contains("[1/3]"));
        assert!(all_text.contains("Missing proof for task B."));
    }

    #[test]
    fn scheduler_stage_renders_runtime_metadata_summary() {
        let theme = Theme::default();
        let message = message_with_stage_runtime_meta(
            "plan",
            Some("prometheus"),
            Some(3),
            Some(5),
            StageRuntimeMeta {
                step: Some(2),
                status: Some("running"),
                focus: Some("Draft the executable plan and its guardrails."),
                last_event: Some("Tool finished: Read"),
                waiting_on: Some("model"),
                activity: Some("Task → build\n- label: Schema migration"),
                loop_budget: Some("step-limit:6"),
            },
        );
        let rendered = render_message_text_part(
            &message,
            "## prometheus · Plan\n\n### Step 1\nMigrate schema",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Status @ Running"));
        assert!(all_text.contains("Step 2/6"));
        assert!(all_text.contains("Waiting Model"));
        assert!(all_text.contains("Tokens 1.2K/320"));
        assert!(
            all_text.contains("Usage reasoning 40 · cache read 2 · cache miss 3 · cache write 1")
        );
        assert!(all_text.contains("Focus Draft the executable plan and its guardrails."));
        assert!(all_text.contains("Last Tool finished: Read"));
        assert!(all_text.contains("Activity Task → build"));
    }

    #[test]
    fn scheduler_stage_generic_json_renders_as_structured_card() {
        let theme = Theme::default();
        let message = message_with_stage_meta("delegation", Some("atlas"), Some(2), Some(4));
        let rendered = render_message_text_part(
            &message,
            "## atlas · Delegation\n\n```json\n{\"status\":\"running\",\"estimated_context_tokens\":256,\"active_agents\":[\"planner\",\"tester\"],\"task_status\":{\"task_a\":\"done - verified\",\"task_b\":\"running\"}}\n```",
            &theme,
            Color::Blue,
        );

        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Atlas · Delegation"));
        assert!(all_text.contains("Details"));
        assert!(all_text.contains("Status: Running"));
        assert!(all_text.contains("Estimated Context Tokens: 256"));
        assert!(all_text.contains("Active Agents: planner, tester"));
        assert!(all_text.contains("Task Status"));
        assert!(!all_text.contains("\"estimated_context_tokens\""));
    }

    #[test]
    fn non_scheduler_message_renders_plain_markdown() {
        let message = Message {
            id: "m1".to_string(),
            role: MessageRole::Assistant,
            content: String::new(),
            created_at: Utc::now(),
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            metadata: None,
            multimodal: None,
            parts: Vec::new(),
        };
        let rendered = render_message_text_part(&message, "hello", &Theme::default(), Color::Blue);
        assert!(rendered.allow_semantic_highlighting);
        assert!(line_text(&rendered.lines[0]).contains("hello"));
    }

    #[test]
    fn system_reminder_display_renders_on_its_own_styled_line() {
        let rendered = render_text_part(
            "Primary answer.\n\nLoaded instruction files: /tmp/project/AGENTS.md",
            &Theme::default(),
            Color::Blue,
        );

        let all_text: Vec<String> = rendered.iter().map(line_text).collect();
        assert!(all_text.iter().any(|line| line.contains("Primary answer.")));
        assert!(all_text
            .iter()
            .any(|line| line.contains("Instruction Files")));
        assert!(all_text
            .iter()
            .all(|line| !line.contains("Primary answer.Loaded instruction files")));
    }

    #[test]
    fn gate_stage_renders_structured_decision_card() {
        let theme = Theme::default();
        let message = message_with_gate_meta(
            "coordination-gate",
            "continue",
            "Task B still lacks evidence.",
            Some("Run one more worker round on task B."),
        );
        let rendered = render_message_text_part(
            &message,
            "## atlas · Coordination Gate\n\n{\"status\":\"continue\"}",
            &theme,
            Color::Blue,
        );
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Decision"));
        assert!(all_text.contains("Outcome: Continue"));
        assert!(all_text.contains("Why: Task B still lacks evidence."));
        assert!(all_text.contains("Next Action: Run one more worker round on task B."));
    }

    #[test]
    fn legacy_gate_json_renders_structured_decision_card_without_metadata_projection() {
        let theme = Theme::default();
        let message = message_with_stage_meta("coordination-gate", Some("atlas"), Some(2), Some(3));
        let rendered = render_message_text_part(
            &message,
            "## atlas · Coordination Gate\n\n```json\n{\"gate_decision\":\"done\",\"reasoning\":\"All delegated work verified complete.\",\"next_actions\":[\"No further execution needed\"],\"task_status\":{\"task_1\":\"done - verified\"}}\n```",
            &theme,
            Color::Blue,
        );
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Decision"));
        assert!(all_text.contains("Outcome: Done"));
        assert!(all_text.contains("Why: All delegated work verified complete."));
        assert!(all_text.contains("Next Action: - No further execution needed"));
    }

    #[test]
    fn canonical_scheduler_stage_fixture_renders_consistently() {
        let fixture = canonical_scheduler_stage_fixture();
        let mut message =
            message_with_stage_meta("coordination-gate", Some("atlas"), Some(2), Some(3));
        message.metadata = Some(fixture.metadata);
        let rendered = render_message_text_part(
            &message,
            &fixture.message_text,
            &Theme::default(),
            Color::Blue,
        );
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(all_text.contains("Atlas · Coordination Gate"));
        assert!(all_text.contains("[2/3]"));
        assert!(all_text.contains("Status ? Waiting"));
        assert!(all_text.contains("Step 4"));
        assert!(all_text.contains("Waiting User"));
        assert!(all_text.contains("Tokens 1.2K/320"));
        assert!(all_text.contains("Active Agents oracle"));
        assert!(all_text.contains("Active Skills debug, qa"));
        assert!(all_text.contains("Decision pending on the unresolved task ledger."));
    }

    #[test]
    fn stage_runtime_lines_render_attached_session_id() {
        let mut message = message_with_stage_runtime_meta(
            "execution",
            Some("prometheus"),
            Some(1),
            Some(5),
            StageRuntimeMeta {
                step: Some(2),
                status: Some("running"),
                focus: None,
                last_event: None,
                waiting_on: Some("model"),
                activity: None,
                loop_budget: None,
            },
        );
        message.metadata.as_mut().unwrap().insert(
            "scheduler_stage_attached_session_id".to_string(),
            json!("attached-session-abc-123"),
        );
        let rendered =
            render_message_text_part(&message, "stage content", &Theme::default(), Color::Blue);
        let all_text: String = rendered.lines.iter().map(line_text).collect();
        assert!(
            all_text.contains("→ Session"),
            "should show navigation arrow"
        );
        assert!(
            all_text.contains("attached-session-abc-123"),
            "should show attached session ID"
        );
        assert!(
            all_text.contains("Ctrl+J to view"),
            "should show keybind hint"
        );
    }
}
