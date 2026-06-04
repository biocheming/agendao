use std::collections::HashMap;

use agendao_command_render::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};
use agendao_command_render::terminal_segment_display::{
    format_preview_line, is_denied_result, normalize_tool_name, tool_argument_preview, tool_glyph,
    TerminalSegmentTone,
};
use agendao_command_render::terminal_tool_block_display::{
    build_batch_result_items, build_display_hint_items, build_edit_result_items,
    build_patch_result_items, build_question_result_items, build_task_result_items,
    build_task_running_items, build_todowrite_result_items, build_write_result_items,
    summarize_block_items_inline, TerminalToolBlockItem, ToolWriteSummary,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::markdown::MarkdownRenderer;
use crate::theme::Theme;

/// Threshold: tool results longer than this are "block" tools with expandable output
const BLOCK_RESULT_THRESHOLD: usize = 3;

#[derive(Debug, Clone, Default)]
struct ReadSummary {
    size_bytes: Option<usize>,
    total_lines: Option<usize>,
}
type WriteSummary = ToolWriteSummary;

/// Returns true if this tool typically produces block-level output
fn is_block_tool(
    name: &str,
    state: TerminalToolState,
    result: Option<&TerminalToolResultInfo>,
    show_tool_details: bool,
) -> bool {
    // Check display.mode override from metadata
    if let Some(info) = result {
        if let Some(mode) = info
            .metadata
            .as_ref()
            .and_then(|m| m.get("display.mode"))
            .and_then(|v| v.as_str())
        {
            return mode == "block";
        }
    }

    let normalized = normalize_tool_name(name);
    match normalized.as_str() {
        "task" | "question" | "todowrite" | "todo_write" => {
            return matches!(state, TerminalToolState::Completed)
                && show_tool_details
                && result.is_some();
        }
        "bash" | "shell" => return result.is_some(),
        "apply_patch" | "applypatch" | "batch" => {
            return matches!(state, TerminalToolState::Completed)
                && show_tool_details
                && result.is_some();
        }
        "skill" | "skills_list" | "skill_view" => return false,
        _ => {}
    }

    // edit/write tools with diff metadata are block-level
    if is_write_tool(&normalized) || is_edit_tool(&normalized) {
        if let Some(info) = result {
            if info
                .metadata
                .as_ref()
                .and_then(|m| m.get("diff"))
                .and_then(|v| v.as_str())
                .is_some_and(|d| !d.is_empty())
            {
                return true;
            }
        }
    }
    // Otherwise, check result length
    if let Some(info) = result {
        info.output.lines().count() > BLOCK_RESULT_THRESHOLD
    } else {
        false
    }
}

fn is_read_tool(normalized_name: &str) -> bool {
    matches!(normalized_name, "read" | "readfile" | "read_file")
}

fn is_list_tool(normalized_name: &str) -> bool {
    matches!(
        normalized_name,
        "ls" | "list" | "listdir" | "list_dir" | "list_directory"
    )
}

fn is_write_tool(normalized_name: &str) -> bool {
    matches!(normalized_name, "write" | "writefile" | "write_file")
}

fn is_edit_tool(normalized_name: &str) -> bool {
    matches!(normalized_name, "edit" | "editfile" | "edit_file")
}

fn is_patch_tool(normalized_name: &str) -> bool {
    matches!(normalized_name, "apply_patch" | "applypatch")
}

fn prefers_specialized_block_body(normalized_name: &str) -> bool {
    matches!(
        normalized_name,
        "task" | "todowrite" | "todo_write" | "batch" | "question"
    ) || is_write_tool(normalized_name)
        || is_edit_tool(normalized_name)
        || is_patch_tool(normalized_name)
}

fn split_list_output<'a>(lines: &'a [&'a str]) -> (Option<&'a str>, Vec<&'a str>) {
    if lines.is_empty() {
        return (None, Vec::new());
    }
    let first = lines[0].trim();
    if first.starts_with('/') && first.ends_with('/') {
        (Some(first), lines[1..].to_vec())
    } else {
        (None, lines.to_vec())
    }
}

fn display_summary(info: &TerminalToolResultInfo) -> Option<&str> {
    info.metadata
        .as_ref()
        .and_then(|m| m.get("display.summary"))
        .and_then(|v| v.as_str())
        .filter(|summary| !summary.trim().is_empty())
}

fn result_title(info: &TerminalToolResultInfo) -> Option<&str> {
    info.title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
}

fn should_show_inline_argument_preview(
    normalized: &str,
    state: TerminalToolState,
    result: Option<&TerminalToolResultInfo>,
) -> bool {
    match state {
        TerminalToolState::Pending | TerminalToolState::Running => !matches!(
            normalized,
            "question" | "todowrite" | "todo_write" | "apply_patch" | "applypatch" | "batch"
        ),
        TerminalToolState::Completed if result.is_some() => !matches!(
            normalized,
            "question" | "todowrite" | "todo_write" | "apply_patch" | "applypatch"
        ),
        _ => true,
    }
}

/// Render a single tool call as lines (inline or block style)
pub fn render_tool_call(
    name: &str,
    arguments: &str,
    state: TerminalToolState,
    result: Option<&TerminalToolResultInfo>,
    show_tool_details: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let normalized = normalize_tool_name(name);
    if matches!(state, TerminalToolState::Completed)
        && !show_tool_details
        && !matches!(
            normalized.as_str(),
            "task"
                | "question"
                | "todowrite"
                | "todo_write"
                | "batch"
                | "apply_patch"
                | "applypatch"
        )
    {
        return Vec::new();
    }

    let block_mode = is_block_tool(name, state, result, show_tool_details);
    let read_summary = if is_read_tool(&normalized) {
        result.and_then(|info| {
            if info.is_error {
                None
            } else {
                Some(parse_read_summary(&info.output))
            }
        })
    } else {
        None
    };

    let glyph = tool_glyph(name);
    let is_denied = result.is_some_and(|info| info.is_error && is_denied_result(&info.output));

    let (state_icon, icon_style, name_style) = styles_for_state(state, is_denied, theme);

    let mut lines = Vec::new();

    if block_mode {
        let bg = theme.background_panel;
        let mut main_spans = vec![
            block_prefix(theme, bg),
            Span::styled(format!("{} ", state_icon), icon_style.bg(bg)),
            Span::styled(format!("{} ", glyph), icon_style.bg(bg)),
            Span::styled(name.to_string(), name_style.bg(bg)),
        ];

        let argument_preview = tool_argument_preview(&normalized, arguments);
        if let Some(ref preview) = argument_preview {
            main_spans.push(Span::styled(
                format!("  {}", preview),
                Style::default().fg(theme.text_muted).bg(bg),
            ));
        } else if let Some(title) = result.and_then(result_title) {
            main_spans.push(Span::styled(
                format!("  {}", format_preview_line(title, 60)),
                Style::default().fg(theme.text_muted).bg(bg),
            ));
        }
        if let Some(summary) = read_summary.as_ref() {
            if let Some(compact) = format_read_summary(summary) {
                main_spans.push(Span::styled(
                    format!("  [{}]", compact),
                    Style::default().fg(theme.text_muted).bg(bg),
                ));
            }
        }

        if is_denied {
            main_spans.push(Span::styled(
                "  denied",
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD)
                    .bg(bg),
            ));
        }

        lines.push(Line::from(main_spans));

        if let Some(info) = result {
            let result_text = &info.output;
            let is_error = info.is_error;

            if is_error {
                let mut iter = result_text.lines().filter(|line| !line.trim().is_empty());
                if let Some(first_line) = iter.next() {
                    lines.push(block_content_line(
                        format!("Error: {}", format_preview_line(first_line, 96)),
                        Style::default().fg(theme.error),
                        theme,
                        bg,
                    ));
                }

                let extra_error_lines = if show_tool_details { 4 } else { 2 };
                for line in iter.take(extra_error_lines) {
                    lines.push(block_content_line(
                        format_preview_line(line, 96),
                        Style::default().fg(theme.error),
                        theme,
                        bg,
                    ));
                }
            } else if !prefers_specialized_block_body(&normalized)
                && render_display_hints(info, theme, bg, &mut lines)
            {
                // Display hints handled the rendering
            } else if normalized == "task" {
                render_task_result_block(
                    result_text,
                    arguments,
                    info.metadata.as_ref(),
                    show_tool_details,
                    theme,
                    bg,
                    &mut lines,
                );
            } else if matches!(normalized.as_str(), "todowrite" | "todo_write") {
                render_todowrite_result_block(
                    result_text,
                    show_tool_details,
                    theme,
                    bg,
                    &mut lines,
                );
            } else if is_write_tool(&normalized) {
                render_write_result_block(
                    result_text,
                    arguments,
                    info.metadata.as_ref(),
                    show_tool_details,
                    theme,
                    bg,
                    &mut lines,
                );
            } else if is_edit_tool(&normalized) {
                render_edit_result_block(
                    result_text,
                    arguments,
                    info.metadata.as_ref(),
                    show_tool_details,
                    theme,
                    bg,
                    &mut lines,
                );
            } else if is_patch_tool(&normalized) {
                render_patch_result_block(
                    result_text,
                    arguments,
                    info.metadata.as_ref(),
                    show_tool_details,
                    theme,
                    bg,
                    &mut lines,
                );
            } else if is_read_tool(&normalized) {
                // Read output is very large and noisy; keep it summarized in the header only.
            } else if normalized == "batch" {
                render_batch_result_block(
                    result_text,
                    arguments,
                    show_tool_details,
                    theme,
                    bg,
                    &mut lines,
                );
            } else if normalized == "question" {
                render_question_result_block(result_text, arguments, theme, bg, &mut lines);
            } else if show_tool_details {
                if let Some(title) = result_title(info) {
                    lines.push(block_content_line(
                        format_preview_line(title, 96),
                        Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
                        theme,
                        bg,
                    ));
                }
                let output_lines = result_text.lines().collect::<Vec<_>>();
                let (list_root, list_entries) = if is_list_tool(&normalized) {
                    split_list_output(&output_lines)
                } else {
                    (None, output_lines.clone())
                };
                let line_count = list_entries.len();
                let mut preview_limit = if normalized == "bash" || normalized == "shell" {
                    10usize
                } else if is_list_tool(&normalized) {
                    40usize
                } else {
                    6usize
                };
                if line_count.saturating_sub(preview_limit) <= 2 {
                    preview_limit = line_count;
                }

                if let Some(root) = list_root {
                    lines.push(block_content_line(
                        format!("[Directory]: {}", root),
                        Style::default()
                            .fg(theme.info)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                        theme,
                        bg,
                    ));
                }

                lines.push(block_content_line(
                    if is_list_tool(&normalized) {
                        format!("({} files)", line_count)
                    } else {
                        format!("({} lines of output)", line_count)
                    },
                    Style::default().fg(theme.text_muted),
                    theme,
                    bg,
                ));

                for line in list_entries.iter().take(preview_limit) {
                    lines.push(block_content_line(
                        format_preview_line(line, 96),
                        Style::default().fg(theme.text),
                        theme,
                        bg,
                    ));
                }

                if line_count > preview_limit {
                    lines.push(block_content_line(
                        format!("… ({} more lines)", line_count - preview_limit),
                        Style::default().fg(theme.text_muted),
                        theme,
                        bg,
                    ));
                }
            }
        } else if normalized == "task"
            && matches!(
                state,
                TerminalToolState::Pending | TerminalToolState::Running
            )
        {
            render_task_running_block(arguments, theme, bg, &mut lines);
        } else if matches!(
            state,
            TerminalToolState::Pending | TerminalToolState::Running
        ) {
            render_pending_block_items(&normalized, arguments, theme, bg, &mut lines);
        }

        return lines;
    }

    // Inline mode
    let mut main_spans = vec![
        Span::styled(format!("{} ", state_icon), icon_style),
        Span::styled(format!("{} ", glyph), Style::default().fg(theme.tool_icon)),
        Span::styled(name.to_string(), name_style),
    ];

    // Argument preview on the same line as tool name (e.g. "◯ → ls → .")
    if should_show_inline_argument_preview(&normalized, state, result) {
        if let Some(argument_preview) = tool_argument_preview(&normalized, arguments) {
            main_spans.push(Span::styled(
                format!("  {}", argument_preview),
                Style::default().fg(theme.text_muted),
            ));
        }
    }

    // Inline result summary for completed non-block tools
    if let Some(info) = result {
        if info.is_error {
            main_spans.push(Span::styled(
                format!(
                    " — {}",
                    format_preview_line(
                        info.output.lines().next().unwrap_or(&info.output).trim(),
                        96
                    )
                ),
                Style::default().fg(theme.error),
            ));
            if is_denied {
                main_spans.push(Span::styled(
                    " (denied)",
                    Style::default()
                        .fg(theme.error)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        } else {
            if let Some(summary) = display_summary(info) {
                main_spans.push(Span::styled(
                    format!(" — {}", format_preview_line(summary, 80)),
                    Style::default().fg(theme.text_muted),
                ));
            } else if let Some(title) = result_title(info) {
                main_spans.push(Span::styled(
                    format!(" — {}", format_preview_line(title, 80)),
                    Style::default().fg(theme.text_muted),
                ));
            } else if normalized == "batch" {
                let summary = summarize_block_items_inline(&build_batch_result_items(
                    &info.output,
                    arguments,
                    false,
                ));
                if !summary.is_empty() {
                    main_spans.push(Span::styled(
                        format!(" — {}", format_preview_line(&summary, 80)),
                        Style::default().fg(theme.text_muted),
                    ));
                }
            } else if is_patch_tool(&normalized) {
                let summary = summarize_block_items_inline(&build_patch_result_items(
                    &info.output,
                    info.metadata.as_ref(),
                    false,
                ));
                if !summary.is_empty() {
                    main_spans.push(Span::styled(
                        format!(" — {}", format_preview_line(&summary, 80)),
                        Style::default().fg(theme.text_muted),
                    ));
                }
            } else {
                let result_text = &info.output;
                if is_write_tool(&normalized) {
                    if let Some(write_summary) = parse_write_summary(result_text) {
                        let mut summary_parts = Vec::new();
                        if let Some(size_bytes) = write_summary.size_bytes {
                            summary_parts.push(format_bytes(size_bytes));
                        }
                        if let Some(total_lines) = write_summary.total_lines {
                            summary_parts.push(format!("{} lines", total_lines));
                        }
                        let verb = write_summary.verb.unwrap_or("updated");
                        let summary_text = if summary_parts.is_empty() {
                            if let Some(path) = write_summary.path.as_deref() {
                                format!("{} {}", verb, path)
                            } else {
                                verb.to_string()
                            }
                        } else {
                            format!("{} · {}", verb, summary_parts.join(" · "))
                        };
                        main_spans.push(Span::styled(
                            format!(" — {}", summary_text),
                            Style::default().fg(theme.success),
                        ));
                    }
                } else {
                    let line_count = result_text.lines().count();
                    if line_count <= 1 {
                        let summary = result_text.trim();
                        if !summary.is_empty() && summary.len() <= 80 {
                            main_spans.push(Span::styled(
                                format!(" — {}", summary),
                                Style::default().fg(theme.text_muted),
                            ));
                        }
                    } else if let Some(first_line) =
                        result_text.lines().find(|line| !line.trim().is_empty())
                    {
                        main_spans.push(Span::styled(
                            format!(
                                " — {} (+{} lines)",
                                format_preview_line(first_line, 72),
                                line_count.saturating_sub(1)
                            ),
                            Style::default().fg(theme.text_muted),
                        ));
                    }
                }
            }
        }
    } else if matches!(
        state,
        TerminalToolState::Pending | TerminalToolState::Running
    ) {
        if let Some(status) = inline_pending_status(&normalized, arguments) {
            main_spans.push(Span::styled(
                format!(" — {}", status),
                Style::default().fg(theme.text_muted),
            ));
        }
    }

    lines.push(Line::from(main_spans));

    lines
}

fn style_for_segment_tone(tone: TerminalSegmentTone, theme: &Theme) -> Style {
    match tone {
        TerminalSegmentTone::Primary => Style::default().fg(theme.text),
        TerminalSegmentTone::Muted => Style::default().fg(theme.text_muted),
        TerminalSegmentTone::Success => Style::default().fg(theme.success),
        TerminalSegmentTone::Error => Style::default().fg(theme.error),
        TerminalSegmentTone::Info => Style::default().fg(theme.info),
        TerminalSegmentTone::Warning => Style::default().fg(theme.warning),
    }
}

fn render_shared_block_items(
    items: Vec<TerminalToolBlockItem>,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    for item in items {
        match item {
            TerminalToolBlockItem::Line(line) => lines.push(block_content_line(
                line.text,
                style_for_segment_tone(line.tone, theme),
                theme,
                bg,
            )),
            TerminalToolBlockItem::Markdown { content } => {
                for markdown_line in MarkdownRenderer::new(theme.clone()).to_lines(&content, None) {
                    lines.push(block_markdown_line(markdown_line, theme, bg));
                }
            }
            TerminalToolBlockItem::Diff { label, content } => {
                if let Some(label) = label {
                    lines.push(block_content_line(
                        label.text,
                        style_for_segment_tone(label.tone, theme).add_modifier(Modifier::BOLD),
                        theme,
                        bg,
                    ));
                }
                render_inline_diff(&content, theme, bg, lines);
            }
        }
    }
}

fn render_pending_block_items(
    normalized: &str,
    arguments: &str,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    let (status, detail) = match normalized {
        "bash" | "shell" => (
            "Executing command…",
            tool_argument_preview(normalized, arguments),
        ),
        "apply_patch" | "applypatch" => ("Preparing patch…", None),
        "batch" => (
            "Running batch tool calls…",
            tool_argument_preview(normalized, arguments),
        ),
        "question" => (
            "Waiting for answers…",
            tool_argument_preview(normalized, arguments),
        ),
        "todowrite" | "todo_write" => (
            "Updating todo list…",
            tool_argument_preview(normalized, arguments),
        ),
        _ => return,
    };

    lines.push(block_content_line(
        status,
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
        theme,
        bg,
    ));

    if let Some(detail) = detail.filter(|detail| !detail.trim().is_empty()) {
        lines.push(block_content_line(
            format_preview_line(&detail, 96),
            Style::default().fg(theme.text_muted),
            theme,
            bg,
        ));
    }
}

fn inline_pending_status(normalized: &str, arguments: &str) -> Option<String> {
    match normalized {
        "question" => tool_argument_preview(normalized, arguments).map(|preview| {
            if preview == "1 question" {
                "Asking 1 question…".to_string()
            } else {
                format!("Asking {}…", preview)
            }
        }),
        "todowrite" | "todo_write" => tool_argument_preview(normalized, arguments).map(|preview| {
            if preview == "1 todo" {
                "Updating 1 todo…".to_string()
            } else {
                format!("Updating {}…", preview)
            }
        }),
        "task" => Some("Delegating…".to_string()),
        "apply_patch" | "applypatch" => Some("Preparing patch…".to_string()),
        "batch" => tool_argument_preview(normalized, arguments)
            .map(|preview| format!("Running {}…", preview)),
        _ => None,
    }
}

/// Render structured display hints from tool metadata.
/// Returns true if any display hints were rendered, false to fall through to default rendering.
fn render_display_hints(
    info: &TerminalToolResultInfo,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) -> bool {
    let Some(items) = build_display_hint_items(info) else {
        return false;
    };
    render_shared_block_items(items, theme, bg, lines);
    true
}

/// Render batch tool results as a list of sub-tool entries instead of raw JSON.
fn render_batch_result_block(
    result_text: &str,
    arguments: &str,
    show_tool_details: bool,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_batch_result_items(result_text, arguments, show_tool_details),
        theme,
        bg,
        lines,
    );
}

/// Render question tool results: show each Q&A pair instead of raw JSON.
fn render_question_result_block(
    result_text: &str,
    arguments: &str,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_question_result_items(result_text, arguments),
        theme,
        bg,
        lines,
    );
}

fn render_write_result_block(
    result_text: &str,
    arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_write_result_items(result_text, arguments, metadata, show_tool_details),
        theme,
        bg,
        lines,
    );
}

/// Maximum diff lines shown inline before truncation.
const INLINE_DIFF_MAX_LINES: usize = 12;

/// Render diff content inline in the message flow, with truncation.
fn render_inline_diff(
    diff_str: &str,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    use super::diff::DiffView;

    let diff_view = DiffView::new().with_content(diff_str);
    let diff_lines = diff_view.to_lines(theme);
    let total = diff_lines.len();

    for diff_line in diff_lines.into_iter().take(INLINE_DIFF_MAX_LINES) {
        // Wrap each diff line with the block prefix for consistent indentation
        let mut spans = vec![block_prefix(theme, bg)];
        spans.extend(
            diff_line
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content, s.style.bg(bg))),
        );
        lines.push(Line::from(spans));
    }

    if total > INLINE_DIFF_MAX_LINES {
        lines.push(block_content_line(
            format!("… (+{} more diff lines)", total - INLINE_DIFF_MAX_LINES),
            Style::default().fg(theme.text_muted),
            theme,
            bg,
        ));
    }
}

/// Render edit tool result block with inline diff.
fn render_edit_result_block(
    result_text: &str,
    arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_edit_result_items(result_text, arguments, metadata, show_tool_details),
        theme,
        bg,
        lines,
    );
}

/// Render apply_patch tool result block with inline diff.
fn render_patch_result_block(
    result_text: &str,
    _arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_patch_result_items(result_text, metadata, show_tool_details),
        theme,
        bg,
        lines,
    );
}

fn render_task_running_block(
    arguments: &str,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(build_task_running_items(arguments), theme, bg, lines);
}

fn render_task_result_block(
    result_text: &str,
    arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_task_result_items(result_text, arguments, metadata, show_tool_details),
        theme,
        bg,
        lines,
    );
}

fn render_todowrite_result_block(
    result_text: &str,
    show_tool_details: bool,
    theme: &Theme,
    bg: ratatui::style::Color,
    lines: &mut Vec<Line<'static>>,
) {
    render_shared_block_items(
        build_todowrite_result_items(result_text, show_tool_details),
        theme,
        bg,
        lines,
    );
}

fn block_prefix(theme: &Theme, background: ratatui::style::Color) -> Span<'static> {
    Span::styled(
        "│ ",
        Style::default().fg(theme.border_subtle).bg(background),
    )
}

fn block_content_line(
    content: impl Into<String>,
    style: Style,
    theme: &Theme,
    background: ratatui::style::Color,
) -> Line<'static> {
    Line::from(vec![
        block_prefix(theme, background),
        Span::styled(format!("  {}", content.into()), style.bg(background)),
    ])
}

fn block_markdown_line(
    content: Line<'static>,
    theme: &Theme,
    background: ratatui::style::Color,
) -> Line<'static> {
    let mut spans = Vec::with_capacity(content.spans.len() + 2);
    spans.push(block_prefix(theme, background));
    spans.push(Span::styled("  ", Style::default().bg(background)));
    for span in content.spans {
        spans.push(Span::styled(span.content, span.style.bg(background)));
    }
    Line::from(spans)
}

fn styles_for_state(
    state: TerminalToolState,
    is_denied: bool,
    theme: &Theme,
) -> (&'static str, Style, Style) {
    match state {
        TerminalToolState::Pending => (
            "◯",
            Style::default().fg(theme.warning),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ),
        TerminalToolState::Running => (
            super::spinner::progress_circle_icon(),
            Style::default().fg(theme.warning),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        TerminalToolState::Completed => (
            "●",
            Style::default().fg(theme.success),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        TerminalToolState::Failed => {
            let mut name_style = Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD);
            if is_denied {
                name_style = name_style.add_modifier(Modifier::CROSSED_OUT);
            }
            ("✗", Style::default().fg(theme.error), name_style)
        }
    }
}

fn parse_write_summary(result_text: &str) -> Option<WriteSummary> {
    agendao_command_render::terminal_tool_block_display::parse_write_summary(result_text)
}

fn parse_read_summary(result_text: &str) -> ReadSummary {
    let mut summary = ReadSummary::default();
    for line in result_text.lines() {
        if summary.size_bytes.is_none() {
            summary.size_bytes = extract_tag_value(line, "size").and_then(|v| v.parse().ok());
        }
        if summary.total_lines.is_none() {
            summary.total_lines =
                extract_tag_value(line, "total-lines").and_then(|v| v.parse().ok());
        }
        if summary.size_bytes.is_some() && summary.total_lines.is_some() {
            break;
        }
    }
    summary
}

fn format_read_summary(summary: &ReadSummary) -> Option<String> {
    match (summary.size_bytes, summary.total_lines) {
        (Some(size), Some(lines)) => Some(format!("{}, {} lines", format_bytes(size), lines)),
        (Some(size), None) => Some(format_bytes(size)),
        (None, Some(lines)) => Some(format!("{} lines", lines)),
        (None, None) => None,
    }
}

fn extract_tag_value<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{}>", tag);
    let end_tag = format!("</{}>", tag);
    let content = line.strip_prefix(start_tag.as_str())?;
    content.strip_suffix(end_tag.as_str())
}

fn format_bytes(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else if bytes as f64 >= KB {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_read_summary, parse_read_summary, parse_write_summary, tool_argument_preview,
    };
    use std::collections::HashMap;

    #[test]
    fn list_tool_preview_shows_path() {
        let preview = tool_argument_preview("ls", r#"{"path":"."}"#);
        assert_eq!(preview.as_deref(), Some("→ ."));
    }

    #[test]
    fn read_tool_preview_supports_file_path_keys() {
        let preview = tool_argument_preview("read", r#"{"file_path":"/tmp/a.txt"}"#);
        assert_eq!(preview.as_deref(), Some("→ /tmp/a.txt"));
    }

    #[test]
    fn generic_preview_compacts_json_to_key_values() {
        let preview = tool_argument_preview("unknown", r#"{"path":".","recursive":true}"#);
        assert_eq!(preview.as_deref(), Some("[path=., recursive=true]"));
    }

    #[test]
    fn apply_patch_preview_hides_patch_body() {
        let preview = tool_argument_preview("apply_patch", "*** Begin Patch\n...");
        assert_eq!(preview.as_deref(), Some("Patch"));
    }

    #[test]
    fn parse_read_summary_from_tool_output_tags() {
        let output = "<path>/tmp/a.txt</path>\n<size>4096</size>\n<total-lines>256</total-lines>\n<content>...</content>";
        let summary = parse_read_summary(output);
        assert_eq!(
            format_read_summary(&summary).as_deref(),
            Some("4.0 KB, 256 lines")
        );
    }

    #[test]
    fn batch_preview_shows_tool_count_and_names() {
        let args = r#"{"toolCalls":[{"tool":"read","parameters":{"file_path":"/tmp/a.txt"}},{"tool":"edit","parameters":{"file_path":"/tmp/b.txt"}},{"tool":"read","parameters":{"file_path":"/tmp/c.txt"}}]}"#;
        let preview = tool_argument_preview("batch", args);
        assert_eq!(preview.as_deref(), Some("3 tools (read, edit)"));
    }

    #[test]
    fn batch_preview_with_no_names_shows_count_only() {
        let args = r#"{"toolCalls":[{},{}]}"#;
        let preview = tool_argument_preview("batch", args);
        assert_eq!(preview.as_deref(), Some("2 tools"));
    }

    #[test]
    fn write_preview_recovers_path_from_jsonish_arguments() {
        let args = "{\"file_path\":\"t2.html\",\"content\":\"<!DOCTYPE html>\n<html";
        let preview = tool_argument_preview("write", args);
        assert_eq!(preview.as_deref(), Some("← t2.html"));
    }

    #[test]
    fn task_preview_uses_prompt_when_description_missing() {
        let args = r###"{"category":"quick","prompt":"## 1. TASK\nRedesign t2.html with stronger visual impact."}"###;
        let preview = tool_argument_preview("task", args);
        assert_eq!(
            preview.as_deref(),
            Some("quick task Redesign t2.html with stronger visual impact.")
        );
    }

    #[test]
    fn parse_write_summary_from_success_message() {
        let output = "Successfully wrote 30199 bytes (725 lines) to ./t2.html";
        let summary = parse_write_summary(output).expect("write summary should parse");
        assert_eq!(summary.size_bytes, Some(30199));
        assert_eq!(summary.total_lines, Some(725));
        assert_eq!(summary.path.as_deref(), Some("./t2.html"));
        assert_eq!(summary.verb, Some("wrote"));
    }

    #[test]
    fn render_edit_result_block_shows_diff_when_metadata_has_diff() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };
        use std::collections::HashMap;

        let theme = crate::theme::Theme::dark();
        let diff_content = "--- a/test.rs\n+++ b/test.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n }";
        let mut metadata = HashMap::new();
        metadata.insert("diff".to_string(), serde_json::json!(diff_content));
        metadata.insert("filepath".to_string(), serde_json::json!("test.rs"));
        let result = TerminalToolResultInfo {
            output: "Edit completed".to_string(),
            is_error: false,
            title: None,
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "edit",
            r#"{"file_path":"test.rs","old_string":"old","new_string":"new"}"#,
            TerminalToolState::Completed,
            Some(&result),
            true, // show_tool_details = true to trigger diff rendering
            &theme,
        );

        // Should have more than just the header — diff lines should be present
        assert!(
            lines.len() > 3,
            "Expected diff lines, got {} lines",
            lines.len()
        );

        // Flatten all spans to text and check for diff markers
        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(
            full_text.contains("Edit Complete"),
            "Should show edit complete header"
        );
        assert!(
            full_text.contains("new") || full_text.contains("+"),
            "Should contain diff addition marker or content"
        );
    }

    #[test]
    fn render_patch_result_block_shows_per_file_diffs() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };
        use std::collections::HashMap;

        let theme = crate::theme::Theme::dark();
        let file1_diff = "--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-old\n+new";
        let file2_diff = "--- a/bar.rs\n+++ b/bar.rs\n@@ -1 +1 @@\n-x\n+y";
        let mut metadata = HashMap::new();
        metadata.insert(
            "diff".to_string(),
            serde_json::json!(format!("{}\n{}", file1_diff, file2_diff)),
        );
        metadata.insert(
            "files".to_string(),
            serde_json::json!([
                {
                    "relativePath": "foo.rs",
                    "type": "update",
                    "diff": file1_diff,
                },
                {
                    "relativePath": "bar.rs",
                    "type": "add",
                    "diff": file2_diff,
                }
            ]),
        );
        let result = TerminalToolResultInfo {
            output: "Patch applied".to_string(),
            is_error: false,
            title: None,
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "apply_patch",
            "",
            TerminalToolState::Completed,
            Some(&result),
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();

        // Should show per-file headers
        assert!(
            full_text.contains("Patched foo.rs"),
            "Should show per-file header for foo.rs, got: {}",
            full_text
        );
        assert!(
            full_text.contains("Created bar.rs"),
            "Should show 'Created' header for added file bar.rs, got: {}",
            full_text
        );
    }

    #[test]
    fn render_write_result_block_shows_diff_from_metadata() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };
        use std::collections::HashMap;

        let theme = crate::theme::Theme::dark();
        let diff_content = "--- /dev/null\n+++ b/new_file.txt\n@@ -0,0 +1,2 @@\n+line1\n+line2";
        let mut metadata = HashMap::new();
        metadata.insert("diff".to_string(), serde_json::json!(diff_content));
        let result = TerminalToolResultInfo {
            output: "Successfully wrote 10 bytes (2 lines) to ./new_file.txt".to_string(),
            is_error: false,
            title: None,
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "write",
            r#"{"file_path":"./new_file.txt","content":"line1\nline2"}"#,
            TerminalToolState::Completed,
            Some(&result),
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(
            full_text.contains("Write Complete"),
            "Should show write header"
        );
        assert!(
            full_text.contains("line1") || full_text.contains("+"),
            "Should render diff content"
        );
    }

    #[test]
    fn inline_tool_prefers_result_title_as_summary() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let result = TerminalToolResultInfo {
            output: "raw body line 1\nraw body line 2".to_string(),
            is_error: false,
            title: Some("Loaded skill: planner".to_string()),
            metadata: None,
        };

        let lines = render_tool_call(
            "skill_view",
            r#"{"name":"planner"}"#,
            TerminalToolState::Completed,
            Some(&result),
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("Loaded skill: planner"));
        assert!(!full_text.contains("raw body line 1 (+1 lines)"));
    }

    #[test]
    fn block_tool_details_show_result_title_line() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let result = TerminalToolResultInfo {
            output: "first output line\nsecond output line\nthird output line\nfourth output line"
                .to_string(),
            is_error: false,
            title: Some("Repository Status".to_string()),
            metadata: Some(HashMap::new()),
        };

        let lines = render_tool_call(
            "repo_status",
            r#"{"path":"."}"#,
            TerminalToolState::Completed,
            Some(&result),
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("Repository Status"));
        assert!(full_text.contains("(4 lines of output)"));
    }

    #[test]
    fn running_bash_without_output_stays_inline() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::TerminalToolState;

        let theme = crate::theme::Theme::dark();
        let lines = render_tool_call(
            "bash",
            r#"{"command":"cargo test -p agendao-tui","description":"Run tests"}"#,
            TerminalToolState::Running,
            None,
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("$ cargo test -p agendao-tui"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn pending_apply_patch_stays_inline() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::TerminalToolState;

        let theme = crate::theme::Theme::dark();
        let lines = render_tool_call(
            "apply_patch",
            "*** Begin Patch\n*** End Patch",
            TerminalToolState::Pending,
            None,
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("Preparing patch"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn question_result_uses_structured_q_and_a_over_display_hints() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("1 question answered"),
        );
        metadata.insert(
            "display.fields".to_string(),
            serde_json::json!([{ "key": "Scope", "value": "Proceed" }]),
        );
        let result = TerminalToolResultInfo {
            output: r#"{"answers":["Proceed"]}"#.to_string(),
            is_error: false,
            title: Some("User response received".to_string()),
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "question",
            r#"{"questions":[{"question":"Choose rollout scope","options":[{"label":"Proceed"}]}]}"#,
            TerminalToolState::Completed,
            Some(&result),
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("Q: Choose rollout scope"));
        assert!(full_text.contains("A: Proceed"));
    }

    #[test]
    fn pending_question_stays_inline_with_status_summary() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::TerminalToolState;

        let theme = crate::theme::Theme::dark();
        let lines = render_tool_call(
            "question",
            r#"{"questions":[{"question":"Choose rollout scope"}]}"#,
            TerminalToolState::Pending,
            None,
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("1 question"));
        assert!(full_text.contains("Asking 1 question"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn completed_question_without_details_stays_visible_inline() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("1 question answered"),
        );
        let result = TerminalToolResultInfo {
            output: r#"{"answers":["Proceed"]}"#.to_string(),
            is_error: false,
            title: Some("User response received".to_string()),
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "question",
            r#"{"questions":[{"question":"Choose rollout scope"}]}"#,
            TerminalToolState::Completed,
            Some(&result),
            false,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("1 question answered"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn pending_todowrite_stays_inline_with_status_summary() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::TerminalToolState;

        let theme = crate::theme::Theme::dark();
        let lines = render_tool_call(
            "todowrite",
            r#"{"todos":[{"content":"Add tests"},{"content":"Refine TUI"}]}"#,
            TerminalToolState::Running,
            None,
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("2 todos"));
        assert!(full_text.contains("Updating 2 todos"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn task_result_without_details_stays_visible_inline() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("Delegated inspect task via session child-1"),
        );
        metadata.insert("hasTextOutput".to_string(), serde_json::json!(true));
        let result = TerminalToolResultInfo {
            output: "task_id: child-1\ntask_status: completed\n<task_result>\n## Summary\nDone.\n</task_result>"
                .to_string(),
            is_error: false,
            title: Some("Completed Task child-1".to_string()),
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "task",
            r###"{"category":"analysis","description":"Inspect migration status"}"###,
            TerminalToolState::Completed,
            Some(&result),
            false,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("Delegated inspect task via session child-1"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn task_result_with_details_uses_structured_block_body() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("Delegated inspect task via session child-1"),
        );
        metadata.insert("hasTextOutput".to_string(), serde_json::json!(true));
        let result = TerminalToolResultInfo {
            output: "task_id: child-1\ntask_status: completed\n<task_result>\n## Summary\nDone.\n</task_result>"
                .to_string(),
            is_error: false,
            title: Some("Completed Task child-1".to_string()),
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "task",
            r###"{"category":"analysis","description":"Inspect migration status"}"###,
            TerminalToolState::Completed,
            Some(&result),
            true,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("Task ID: child-1"));
        assert!(full_text.contains("Status: completed"));
        assert!(full_text.contains("Summary"));
        assert!(full_text.contains("│"));
    }

    #[test]
    fn completed_batch_without_details_stays_visible_inline() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let result = TerminalToolResultInfo {
            output:
                r#"{"results":[{"success":true,"output":"ok"},{"success":false,"error":"boom"}]}"#
                    .to_string(),
            is_error: false,
            title: None,
            metadata: Some(HashMap::new()),
        };

        let lines = render_tool_call(
            "batch",
            r#"{"toolCalls":[{"tool":"read","parameters":{"file_path":"a.txt"}},{"tool":"edit","parameters":{"file_path":"b.txt"}}]}"#,
            TerminalToolState::Completed,
            Some(&result),
            false,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("batch"));
        assert!(full_text.contains("2 tools (read, edit)"));
        assert!(full_text.contains("2 tools: 1 ok, 1 failed"));
        assert!(!full_text.contains("│"));
    }

    #[test]
    fn completed_apply_patch_without_details_stays_visible_inline() {
        use super::render_tool_call;
        use agendao_command_render::terminal_presentation::{
            TerminalToolResultInfo, TerminalToolState,
        };

        let theme = crate::theme::Theme::dark();
        let mut metadata = HashMap::new();
        metadata.insert(
            "files".to_string(),
            serde_json::json!([
                { "path": "src/a.rs" },
                { "path": "src/b.rs" }
            ]),
        );
        let result = TerminalToolResultInfo {
            output: "Patch applied".to_string(),
            is_error: false,
            title: None,
            metadata: Some(metadata),
        };

        let lines = render_tool_call(
            "apply_patch",
            "*** Begin Patch\n*** End Patch",
            TerminalToolState::Completed,
            Some(&result),
            false,
            &theme,
        );

        let full_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(full_text.contains("apply_patch"));
        assert!(full_text.contains("Patch"));
        assert!(full_text.contains("Patch Applied"));
        assert!(!full_text.contains("│"));
    }
}
