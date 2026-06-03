use std::collections::HashMap;

use serde_json::Value;

use crate::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};
use crate::terminal_segment_display::{
    extract_string_key, format_preview_line, normalize_tool_name, tool_argument_preview,
    tool_glyph, TerminalSegmentDisplayLine, TerminalSegmentTone,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolWriteSummary {
    pub size_bytes: Option<usize>,
    pub total_lines: Option<usize>,
    pub path: Option<String>,
    pub verb: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalBlockItem {
    Line(TerminalSegmentDisplayLine),
    Markdown {
        content: String,
    },
    Diff {
        label: Option<TerminalSegmentDisplayLine>,
        content: String,
    },
}

pub type TerminalToolBlockItem = TerminalBlockItem;

pub fn build_display_hint_items(
    info: &TerminalToolResultInfo,
) -> Option<Vec<TerminalToolBlockItem>> {
    let metadata = info.metadata.as_ref()?;
    let has_fields = metadata.contains_key("display.fields");
    let has_summary = metadata.contains_key("display.summary");
    let has_preview = metadata.contains_key("display.preview");

    if !has_fields && !has_summary && !has_preview {
        return None;
    }

    let mut items = Vec::new();
    if let Some(summary) = metadata.get("display.summary").and_then(|v| v.as_str()) {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format_preview_line(summary, 96),
                TerminalSegmentTone::Muted,
            ),
        ));
    }

    if let Some(fields) = metadata.get("display.fields").and_then(|v| v.as_array()) {
        for field in fields {
            let key = field
                .get("label")
                .or_else(|| field.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let value = field.get("value").and_then(|v| v.as_str()).unwrap_or("");
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("{}: {}", key, format_preview_line(value, 88 - key.len())),
                    TerminalSegmentTone::Primary,
                ),
            ));
        }
    }

    if let Some(preview) = metadata
        .get("display.preview")
        .and_then(|value| value.as_object())
    {
        let preview_kind = preview
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("text");
        let preview_text = preview
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if !preview_text.is_empty() {
            match preview_kind {
                "diff" => items.push(TerminalToolBlockItem::Diff {
                    label: None,
                    content: preview_text.to_string(),
                }),
                _ => {
                    for line in preview_text.lines().take(4) {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        items.push(TerminalToolBlockItem::Line(
                            TerminalSegmentDisplayLine::new(
                                format_preview_line(trimmed, 96),
                                TerminalSegmentTone::Muted,
                            ),
                        ));
                    }
                    let truncated = preview
                        .get("truncated")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    if truncated {
                        items.push(TerminalToolBlockItem::Line(
                            TerminalSegmentDisplayLine::new(
                                "Preview truncated.",
                                TerminalSegmentTone::Muted,
                            ),
                        ));
                    }
                }
            }
        }
    }

    Some(items)
}

pub fn build_file_items(path: &str, mime: &str) -> Vec<TerminalBlockItem> {
    let mut items = vec![TerminalBlockItem::Line(TerminalSegmentDisplayLine::new(
        format!("[file] {}", path),
        TerminalSegmentTone::Info,
    ))];
    if !mime.trim().is_empty() {
        items.push(TerminalBlockItem::Line(TerminalSegmentDisplayLine::new(
            format!("type: {}", mime.trim()),
            TerminalSegmentTone::Muted,
        )));
    }
    items
}

pub fn build_image_items(url: &str) -> Vec<TerminalBlockItem> {
    let trimmed = url.trim();
    let mut items = Vec::new();
    if let Some((mime, payload)) = parse_data_url(trimmed) {
        items.push(TerminalBlockItem::Line(TerminalSegmentDisplayLine::new(
            "[image] inline image",
            TerminalSegmentTone::Info,
        )));
        items.push(TerminalBlockItem::Line(TerminalSegmentDisplayLine::new(
            format!("type: {}", mime),
            TerminalSegmentTone::Muted,
        )));
        if let Some(size_bytes) = estimate_data_url_bytes(payload) {
            items.push(TerminalBlockItem::Line(TerminalSegmentDisplayLine::new(
                format!("size: {}", format_bytes(size_bytes)),
                TerminalSegmentTone::Muted,
            )));
        }
        return items;
    }

    items.push(TerminalBlockItem::Line(TerminalSegmentDisplayLine::new(
        format!("[image] {}", trimmed),
        TerminalSegmentTone::Info,
    )));
    items
}

pub fn summarize_block_items_inline(items: &[TerminalBlockItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            TerminalBlockItem::Line(line) => Some(line.text.trim().to_string()),
            TerminalBlockItem::Markdown { content } => content
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| format_preview_line(line, 96)),
            TerminalBlockItem::Diff { label, content } => label
                .as_ref()
                .map(|label| label.text.trim().to_string())
                .or_else(|| {
                    content
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(|line| format_preview_line(line, 96))
                }),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn build_tool_body_items(
    name: &str,
    arguments: &str,
    state: TerminalToolState,
    result: Option<&TerminalToolResultInfo>,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let normalized = normalize_tool_name(name);

    let Some(info) = result else {
        return if normalized == "task"
            && matches!(
                state,
                TerminalToolState::Pending | TerminalToolState::Running
            ) {
            build_task_running_items(arguments)
        } else {
            Vec::new()
        };
    };

    if info.is_error {
        let mut items = Vec::new();
        let mut iter = info.output.lines().filter(|line| !line.trim().is_empty());
        if let Some(first_line) = iter.next() {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("Error: {}", format_preview_line(first_line, 96)),
                    TerminalSegmentTone::Error,
                ),
            ));
        }
        let extra_error_lines = if show_tool_details { 4 } else { 2 };
        for line in iter.take(extra_error_lines) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(line, 96),
                    TerminalSegmentTone::Error,
                ),
            ));
        }
        return items;
    }

    if !prefers_specialized_block_body(&normalized) {
        if let Some(items) = build_display_hint_items(info) {
            return items;
        }
    }

    match normalized.as_str() {
        "task" => build_task_result_items(
            &info.output,
            arguments,
            info.metadata.as_ref(),
            show_tool_details,
        ),
        "todowrite" | "todo_write" => build_todowrite_result_items(&info.output, show_tool_details),
        "batch" => build_batch_result_items(&info.output, arguments, show_tool_details),
        "question" => build_question_result_items(&info.output, arguments),
        value if is_write_tool(value) => build_write_result_items(
            &info.output,
            arguments,
            info.metadata.as_ref(),
            show_tool_details,
        ),
        value if is_edit_tool(value) => build_edit_result_items(
            &info.output,
            arguments,
            info.metadata.as_ref(),
            show_tool_details,
        ),
        value if is_patch_tool(value) => {
            build_patch_result_items(&info.output, info.metadata.as_ref(), show_tool_details)
        }
        value if is_read_tool(value) => Vec::new(),
        _ => build_generic_result_items(&info.output, show_tool_details),
    }
}

fn prefers_specialized_block_body(normalized_name: &str) -> bool {
    matches!(
        normalized_name,
        "task" | "todowrite" | "todo_write" | "batch" | "question"
    ) || is_write_tool(normalized_name)
        || is_edit_tool(normalized_name)
        || is_patch_tool(normalized_name)
}

pub fn build_batch_result_items(
    result_text: &str,
    arguments: &str,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let arg_parsed = serde_json::from_str::<Value>(arguments).ok();
    let calls = arg_parsed
        .as_ref()
        .and_then(|v| v.get("toolCalls").or_else(|| v.get("tool_calls")))
        .and_then(|v| v.as_array());

    let json_text = result_text
        .find("Results:\n")
        .map(|pos| &result_text[pos + "Results:\n".len()..])
        .unwrap_or(result_text);
    let result_parsed = serde_json::from_str::<Value>(json_text).ok();
    let result_array = result_parsed.as_ref().and_then(|v| {
        v.as_array()
            .or_else(|| v.get("results").and_then(|r| r.as_array()))
    });

    let mut items = Vec::new();
    if let Some(results) = result_array {
        let total = results.len();
        let ok_count = results
            .iter()
            .filter(|r| r.get("success").and_then(|v| v.as_bool()).unwrap_or(true))
            .count();
        let fail_count = total - ok_count;
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                if fail_count == 0 {
                    format!("{} tools: all ok", total)
                } else {
                    format!("{} tools: {} ok, {} failed", total, ok_count, fail_count)
                },
                if fail_count == 0 {
                    TerminalSegmentTone::Muted
                } else {
                    TerminalSegmentTone::Warning
                },
            ),
        ));

        if !show_tool_details {
            return items;
        }

        for (i, result_entry) in results.iter().enumerate() {
            let sub_name = calls
                .and_then(|c| c.get(i))
                .and_then(|c| {
                    c.get("tool")
                        .or_else(|| c.get("name"))
                        .or_else(|| c.get("tool_name"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("?");
            let is_ok = result_entry
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let sub_result = result_entry
                .get("output")
                .or_else(|| result_entry.get("result"))
                .or_else(|| result_entry.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let first_line = sub_result
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            let line_count = sub_result.lines().count();

            let mut row = format!(
                "{} {} {}",
                if is_ok { "●" } else { "✗" },
                tool_glyph(sub_name),
                sub_name
            );
            if let Some(call_args) = calls.and_then(|c| c.get(i)) {
                if let Some(args_json) = call_args.get("parameters").map(|v| v.to_string()) {
                    let sub_normalized = normalize_tool_name(sub_name);
                    if let Some(preview) = tool_argument_preview(&sub_normalized, &args_json) {
                        row.push_str(&format!("  {}", format_preview_line(&preview, 40)));
                    }
                }
            }
            if !is_ok {
                let err_preview = format_preview_line(first_line, 48);
                if !err_preview.is_empty() {
                    row.push_str(&format!("  {}", err_preview));
                }
            } else if line_count > 1 {
                row.push_str(&format!("  (+{} lines)", line_count));
            }
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    row,
                    if is_ok {
                        TerminalSegmentTone::Muted
                    } else {
                        TerminalSegmentTone::Error
                    },
                ),
            ));
        }

        return items;
    }

    if show_tool_details {
        let output_lines: Vec<&str> = result_text.lines().collect();
        let line_count = output_lines.len();
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("({} lines of output)", line_count),
                TerminalSegmentTone::Muted,
            ),
        ));
        for line in output_lines.iter().take(8) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(line, 96),
                    TerminalSegmentTone::Primary,
                ),
            ));
        }
        if line_count > 8 {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("… ({} more lines)", line_count - 8),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    }

    items
}

pub fn build_question_result_items(
    result_text: &str,
    arguments: &str,
) -> Vec<TerminalToolBlockItem> {
    let arg_parsed = serde_json::from_str::<Value>(arguments).ok();
    let questions = arg_parsed
        .as_ref()
        .and_then(|v| v.get("questions"))
        .and_then(|v| v.as_array());
    let result_parsed = serde_json::from_str::<Value>(result_text).ok();
    let answers = result_parsed
        .as_ref()
        .and_then(|v| v.get("answers"))
        .and_then(|v| v.as_array());

    let mut items = Vec::new();
    if let Some(qs) = questions {
        for (i, q) in qs.iter().enumerate() {
            let q_text = q.get("question").and_then(|v| v.as_str()).unwrap_or("?");
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("Q: {}", format_preview_line(q_text, 88)),
                    TerminalSegmentTone::Info,
                ),
            ));
            if let Some(opts) = q.get("options").and_then(|v| v.as_array()) {
                for opt in opts {
                    let label = opt.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = opt.get("description").and_then(|v| v.as_str());
                    items.push(TerminalToolBlockItem::Line(
                        TerminalSegmentDisplayLine::new(
                            match desc {
                                Some(d) => {
                                    format!("  · {} — {}", label, format_preview_line(d, 64))
                                }
                                None => format!("  · {}", label),
                            },
                            TerminalSegmentTone::Muted,
                        ),
                    ));
                }
            }
            let answer = answers
                .and_then(|a| a.get(i))
                .and_then(|v| v.as_str())
                .unwrap_or("(no answer)");
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("A: {}", format_preview_line(answer, 88)),
                    TerminalSegmentTone::Success,
                ),
            ));
        }
        return items;
    }

    let first_line = result_text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(result_text);
    items.push(TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new(
            format_preview_line(first_line, 88),
            TerminalSegmentTone::Primary,
        ),
    ));
    items
}

pub fn build_task_running_items(arguments: &str) -> Vec<TerminalToolBlockItem> {
    let summary = parse_task_argument_summary(arguments);
    let mut items = vec![TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new(
            "Delegating task to subagent…",
            TerminalSegmentTone::Warning,
        ),
    )];

    let subagent = summary
        .category
        .as_deref()
        .or(summary.subagent_type.as_deref());
    if let Some(name) = subagent {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Subagent: {}", name),
                TerminalSegmentTone::Info,
            ),
        ));
    }

    if let Some(prompt) = summary.prompt_preview.as_deref() {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Task: {}", prompt),
                TerminalSegmentTone::Muted,
            ),
        ));
    } else if let Some(description) = summary.description.as_deref() {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Task: {}", format_preview_line(description, 88)),
                TerminalSegmentTone::Muted,
            ),
        ));
    }

    if let Some(skill_count) = summary.skill_count {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "Skills: {}",
                    if skill_count == 0 {
                        "none".to_string()
                    } else {
                        skill_count.to_string()
                    }
                ),
                TerminalSegmentTone::Muted,
            ),
        ));
    }

    if !summary.checklist.is_empty() {
        let total = summary.checklist.len();
        let preview_limit = total.min(4);
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Checklist ({} items):", total),
                TerminalSegmentTone::Info,
            ),
        ));
        for item in summary.checklist.iter().take(preview_limit) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("[ ] {}", format_preview_line(item, 88)),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
        if total > preview_limit {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("… ({} more items)", total - preview_limit),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    }

    items
}

pub fn build_task_result_items(
    result_text: &str,
    arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let arg_summary = parse_task_argument_summary(arguments);
    let mut items = Vec::new();

    let subagent = arg_summary
        .category
        .as_deref()
        .or(arg_summary.subagent_type.as_deref());
    if let Some(name) = subagent {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Subagent: {}", name),
                TerminalSegmentTone::Info,
            ),
        ));
    }
    if let Some(prompt) = arg_summary.prompt_preview.as_deref() {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Task: {}", prompt),
                TerminalSegmentTone::Muted,
            ),
        ));
    }

    let summary = parse_task_result_summary(result_text);
    if let Some(task_id) = summary.task_id.as_deref() {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Task ID: {}", task_id),
                TerminalSegmentTone::Info,
            ),
        ));
    }
    if let Some(task_status) = summary.task_status.as_deref() {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("Status: {}", task_status),
                if task_status.eq_ignore_ascii_case("completed") {
                    TerminalSegmentTone::Success
                } else {
                    TerminalSegmentTone::Info
                },
            ),
        ));
    }
    if let Some(meta) = metadata {
        if let Some(has_text_output) = meta.get("hasTextOutput").and_then(|v| v.as_bool()) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!(
                        "Text Output: {}",
                        if has_text_output { "yes" } else { "no" }
                    ),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
        if let Some(model) = extract_task_model_label(meta) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("Model: {}", model),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    }

    if summary.body.trim().is_empty() {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                "Subagent finished with no textual output",
                TerminalSegmentTone::Muted,
            ),
        ));
        if !arg_summary.checklist.is_empty() {
            let completed = summary
                .task_status
                .as_deref()
                .is_some_and(|status| status.eq_ignore_ascii_case("completed"));
            append_checklist_items(
                &mut items,
                &arg_summary.checklist,
                show_tool_details,
                completed,
            );
        }
        return items;
    }

    let mut checklist = parse_markdown_checklist(&summary.body);
    if checklist.is_empty() && !arg_summary.checklist.is_empty() {
        let completed = summary
            .task_status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("completed"));
        checklist = arg_summary
            .checklist
            .iter()
            .map(|item| ChecklistItem {
                checked: completed,
                text: item.clone(),
            })
            .collect();
    }
    if !checklist.is_empty() {
        append_checklist_entries(&mut items, &checklist, show_tool_details);
    }

    let body_lines: Vec<&str> = summary.body.lines().collect();
    let total = body_lines.len();
    let preview_limit = if show_tool_details {
        total
    } else {
        total.min(5)
    };

    if total > 1 {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("({} lines)", total),
                TerminalSegmentTone::Muted,
            ),
        ));
    }
    if preview_limit > 0 {
        let preview_content = body_lines[..preview_limit].join("\n");
        if !preview_content.trim().is_empty() {
            items.push(TerminalToolBlockItem::Markdown {
                content: preview_content,
            });
        }
    }
    if total > preview_limit {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "… ({} more lines, toggle Tool Details to expand)",
                    total - preview_limit
                ),
                TerminalSegmentTone::Muted,
            ),
        ));
    }

    items
}

pub fn build_todowrite_result_items(
    result_text: &str,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let entries = parse_todowrite_entries(result_text);
    let mut items = Vec::new();

    if entries.is_empty() {
        let output_lines: Vec<&str> = result_text.lines().collect();
        let line_count = output_lines.len();
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("({} lines of output)", line_count),
                TerminalSegmentTone::Muted,
            ),
        ));
        for line in output_lines.iter().take(8) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(line, 96),
                    TerminalSegmentTone::Primary,
                ),
            ));
        }
        if line_count > 8 {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("… ({} more lines)", line_count - 8),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
        return items;
    }

    let total = entries.len();
    let preview_limit = if show_tool_details {
        total
    } else {
        total.min(5)
    };
    items.push(TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new(
            format!("Todo List ({} items)", total),
            TerminalSegmentTone::Info,
        ),
    ));

    for entry in entries.iter().take(preview_limit) {
        let mut row = format!(
            "[{}] {}",
            entry.status,
            format_preview_line(&entry.text, 72)
        );
        if let Some(priority) = entry.priority.as_deref() {
            row.push_str(&format!("  [{}]", priority));
        }
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(row, todo_status_tone(&entry.status)),
        ));
    }

    if total > preview_limit {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "… ({} more todos, toggle Tool Details to expand)",
                    total - preview_limit
                ),
                TerminalSegmentTone::Muted,
            ),
        ));
    }

    items
}

pub fn parse_write_summary(result_text: &str) -> Option<ToolWriteSummary> {
    let first_line = result_text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first_line.is_empty() {
        return None;
    }

    let lower = first_line.to_ascii_lowercase();
    let verb = if lower.contains("wrote") || lower.contains("written") {
        Some("wrote")
    } else if lower.contains("updated") {
        Some("updated")
    } else if lower.contains("created") {
        Some("created")
    } else if lower.contains("saved") {
        Some("saved")
    } else {
        None
    };

    let mut summary = ToolWriteSummary {
        size_bytes: None,
        total_lines: None,
        path: None,
        verb,
    };

    let tokens: Vec<&str> = first_line.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if token.starts_with("bytes") && index > 0 {
            summary.size_bytes = parse_numeric_token(tokens[index - 1]);
        }
        if token.starts_with("lines") && index > 0 {
            summary.total_lines = parse_numeric_token(tokens[index - 1]);
        }
    }

    summary.path = first_line
        .rsplit_once(" to ")
        .map(|(_, path)| sanitize_path_token(path))
        .or_else(|| {
            first_line
                .rsplit_once(" into ")
                .map(|(_, path)| sanitize_path_token(path))
        });

    if summary.verb.is_none()
        && summary.size_bytes.is_none()
        && summary.total_lines.is_none()
        && summary.path.is_none()
    {
        None
    } else {
        Some(summary)
    }
}

pub fn build_write_result_items(
    result_text: &str,
    arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let args_parsed = serde_json::from_str::<Value>(arguments).ok();
    let write_summary = parse_write_summary(result_text);
    let write_path = args_parsed
        .as_ref()
        .and_then(crate::terminal_segment_display::extract_path)
        .or_else(|| crate::terminal_segment_display::extract_jsonish_path_from_raw(arguments))
        .or_else(|| {
            write_summary
                .as_ref()
                .and_then(|summary| summary.path.clone())
        });

    let mut items = vec![TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new("✦ Write Complete", TerminalSegmentTone::Success),
    )];
    if let Some(path) = write_path {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(format!("File: {}", path), TerminalSegmentTone::Info),
        ));
    }
    if let Some(summary) = write_summary.as_ref() {
        let mut stats = Vec::new();
        if let Some(size_bytes) = summary.size_bytes {
            stats.push(format!("Size {}", format_bytes(size_bytes)));
        }
        if let Some(total_lines) = summary.total_lines {
            stats.push(format!("Lines {}", total_lines));
        }
        if !stats.is_empty() {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(stats.join("  ·  "), TerminalSegmentTone::Muted),
            ));
        }
    }
    if show_tool_details {
        if let Some(diff_str) = metadata
            .and_then(|m| m.get("diff"))
            .and_then(|v| v.as_str())
            .filter(|d| !d.is_empty())
        {
            items.push(TerminalToolBlockItem::Diff {
                label: None,
                content: diff_str.to_string(),
            });
        } else if let Some(first_line) = result_text.lines().find(|line| !line.trim().is_empty()) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(first_line, 96),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    }
    items
}

pub fn build_edit_result_items(
    result_text: &str,
    arguments: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let args_parsed = serde_json::from_str::<Value>(arguments).ok();
    let edit_path = args_parsed
        .as_ref()
        .and_then(crate::terminal_segment_display::extract_path)
        .or_else(|| crate::terminal_segment_display::extract_jsonish_path_from_raw(arguments));

    let mut items = vec![TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new("✦ Edit Complete", TerminalSegmentTone::Success),
    )];
    if let Some(path) = edit_path {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(format!("File: {}", path), TerminalSegmentTone::Info),
        ));
    }
    if let Some(replacements) = metadata
        .and_then(|m| m.get("replacements"))
        .and_then(|v| v.as_u64())
    {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("{} replacement(s)", replacements),
                TerminalSegmentTone::Muted,
            ),
        ));
    }
    if let Some(diags) = metadata
        .and_then(|m| m.get("diagnostics"))
        .and_then(|v| v.as_array())
    {
        if !diags.is_empty() {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("⚠ {} diagnostic(s)", diags.len()),
                    TerminalSegmentTone::Warning,
                ),
            ));
        }
    }
    if show_tool_details {
        if let Some(diff_str) = metadata
            .and_then(|m| m.get("diff"))
            .and_then(|v| v.as_str())
            .filter(|d| !d.is_empty())
        {
            items.push(TerminalToolBlockItem::Diff {
                label: None,
                content: diff_str.to_string(),
            });
        } else if let Some(first_line) = result_text.lines().find(|line| !line.trim().is_empty()) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(first_line, 96),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    }
    items
}

pub fn build_patch_result_items(
    result_text: &str,
    metadata: Option<&HashMap<String, serde_json::Value>>,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let files: Vec<String> = metadata
        .and_then(|m| m.get("files"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    f.as_str()
                        .map(String::from)
                        .or_else(|| f.get("path").and_then(|p| p.as_str()).map(String::from))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut items = vec![TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new(
            format!("✦ Patch Applied — {} file(s)", files.len().max(1)),
            TerminalSegmentTone::Success,
        ),
    )];
    for file in files.iter().take(8) {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(format!("  {}", file), TerminalSegmentTone::Info),
        ));
    }
    if files.len() > 8 {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!("  … (+{} more files)", files.len() - 8),
                TerminalSegmentTone::Muted,
            ),
        ));
    }
    if let Some(diags) = metadata
        .and_then(|m| m.get("diagnostics"))
        .and_then(|v| v.as_array())
    {
        if !diags.is_empty() {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("⚠ {} diagnostic(s)", diags.len()),
                    TerminalSegmentTone::Warning,
                ),
            ));
        }
    }
    if show_tool_details {
        let per_file_diffs: Vec<(String, String, String)> = metadata
            .and_then(|m| m.get("files"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        let path = f
                            .get("relativePath")
                            .or_else(|| f.get("path"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                            .to_string();
                        let change_type = f
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("update")
                            .to_string();
                        let diff = f.get("diff").and_then(|v| v.as_str()).unwrap_or("");
                        if diff.is_empty() {
                            None
                        } else {
                            Some((path, change_type, diff.to_string()))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        if !per_file_diffs.is_empty() {
            for (path, change_type, diff_str) in per_file_diffs {
                let label = match change_type.as_str() {
                    "add" => format!("# Created {}", path),
                    "delete" => format!("# Deleted {}", path),
                    "move" => format!("# Moved {}", path),
                    _ => format!("← Patched {}", path),
                };
                items.push(TerminalToolBlockItem::Diff {
                    label: Some(TerminalSegmentDisplayLine::new(
                        label,
                        TerminalSegmentTone::Info,
                    )),
                    content: diff_str,
                });
            }
        } else if let Some(diff_str) = metadata
            .and_then(|m| m.get("diff"))
            .and_then(|v| v.as_str())
            .filter(|d| !d.is_empty())
        {
            items.push(TerminalToolBlockItem::Diff {
                label: None,
                content: diff_str.to_string(),
            });
        } else if let Some(first_line) = result_text.lines().find(|line| !line.trim().is_empty()) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(first_line, 96),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    }
    items
}

fn is_read_tool(normalized_name: &str) -> bool {
    matches!(normalized_name, "read" | "readfile" | "read_file")
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

fn build_generic_result_items(
    result_text: &str,
    show_tool_details: bool,
) -> Vec<TerminalToolBlockItem> {
    let output_lines: Vec<&str> = result_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if output_lines.is_empty() {
        return Vec::new();
    }

    let mut items = Vec::new();
    if show_tool_details {
        let preview_limit = output_lines.len().min(5);
        for line in output_lines.iter().take(preview_limit) {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format_preview_line(line, 96),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
        if output_lines.len() > preview_limit {
            items.push(TerminalToolBlockItem::Line(
                TerminalSegmentDisplayLine::new(
                    format!("… ({} more lines)", output_lines.len() - preview_limit),
                    TerminalSegmentTone::Muted,
                ),
            ));
        }
    } else {
        let first_line = format_preview_line(output_lines[0], 96);
        let suffix = if output_lines.len() > 1 {
            format!("{first_line} (+{} lines)", output_lines.len() - 1)
        } else {
            first_line
        };
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(suffix, TerminalSegmentTone::Muted),
        ));
    }

    items
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TaskResultSummary {
    task_id: Option<String>,
    task_status: Option<String>,
    body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TaskArgumentSummary {
    category: Option<String>,
    subagent_type: Option<String>,
    description: Option<String>,
    prompt_preview: Option<String>,
    skill_count: Option<usize>,
    checklist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChecklistItem {
    checked: bool,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoPreviewEntry {
    status: String,
    text: String,
    priority: Option<String>,
}

fn parse_task_result_summary(result_text: &str) -> TaskResultSummary {
    let mut summary = TaskResultSummary::default();
    for line in result_text.lines() {
        let trimmed = line.trim();
        if let Some(raw) = trimmed.strip_prefix("task_id:") {
            let id = raw
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !id.is_empty() {
                summary.task_id = Some(id);
            }
            continue;
        }
        if let Some(raw) = trimmed.strip_prefix("task_status:") {
            let status = raw.trim().to_string();
            if !status.is_empty() {
                summary.task_status = Some(status);
            }
        }
    }

    if let (Some(start), Some(end)) = (
        result_text.find("<task_result>"),
        result_text.find("</task_result>"),
    ) {
        if end > start {
            let body = &result_text[start + "<task_result>".len()..end];
            summary.body = body.trim().to_string();
            return summary;
        }
    }

    summary.body = result_text.trim().to_string();
    summary
}

fn parse_markdown_checklist(text: &str) -> Vec<ChecklistItem> {
    fn parse_line(line: &str) -> Option<ChecklistItem> {
        let trimmed = line.trim();
        let (checked, rest) = if let Some(rest) = trimmed.strip_prefix("- [x]") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("- [X]") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("- [ ]") {
            (false, rest)
        } else {
            return None;
        };
        let text = rest.trim();
        if text.is_empty() {
            return None;
        }
        Some(ChecklistItem {
            checked,
            text: text.to_string(),
        })
    }

    text.lines().filter_map(parse_line).collect()
}

fn parse_task_argument_summary(arguments: &str) -> TaskArgumentSummary {
    let mut summary = TaskArgumentSummary::default();
    let raw = arguments.trim();
    if raw.is_empty() {
        return summary;
    }

    let parsed = parse_json_object_relaxed(raw);
    let Some(value) = parsed.as_ref() else {
        return summary;
    };

    summary.category = extract_string_key(value, &["category"]);
    summary.subagent_type = extract_string_key(value, &["subagent_type", "subagentType"]);
    summary.description = extract_string_key(value, &["description"]);
    let prompt = extract_string_key(value, &["prompt"]);
    if let Some(prompt_text) = prompt.as_deref() {
        summary.checklist = parse_markdown_checklist(prompt_text)
            .into_iter()
            .map(|item| item.text)
            .collect();
    }
    summary.prompt_preview = prompt.and_then(|prompt| {
        prompt
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("- ["))
            .or_else(|| prompt.lines().map(str::trim).find(|line| !line.is_empty()))
            .map(|line| format_preview_line(line, 88))
    });
    summary.skill_count = value
        .get("load_skills")
        .or_else(|| value.get("loadSkills"))
        .and_then(|v| v.as_array())
        .map(Vec::len);

    summary
}

fn parse_json_object_relaxed(raw: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw).ok().or_else(|| {
        let start = raw.find('{')?;
        let end = raw.rfind('}')?;
        (end > start)
            .then(|| &raw[start..=end])
            .and_then(|slice| serde_json::from_str::<Value>(slice).ok())
    })
}

fn append_checklist_items(
    items: &mut Vec<TerminalToolBlockItem>,
    checklist: &[String],
    show_tool_details: bool,
    completed: bool,
) {
    items.push(TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new(
            format!("Checklist ({} items):", checklist.len()),
            TerminalSegmentTone::Info,
        ),
    ));
    let preview_limit = if show_tool_details {
        checklist.len()
    } else {
        checklist.len().min(5)
    };
    for item in checklist.iter().take(preview_limit) {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "{} {}",
                    if completed { "[x]" } else { "[ ]" },
                    format_preview_line(item, 88)
                ),
                if completed {
                    TerminalSegmentTone::Success
                } else {
                    TerminalSegmentTone::Muted
                },
            ),
        ));
    }
    if checklist.len() > preview_limit {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "… ({} more items, toggle Tool Details to expand)",
                    checklist.len() - preview_limit
                ),
                TerminalSegmentTone::Muted,
            ),
        ));
    }
}

fn append_checklist_entries(
    items: &mut Vec<TerminalToolBlockItem>,
    checklist: &[ChecklistItem],
    show_tool_details: bool,
) {
    let total = checklist.len();
    let preview_limit = if show_tool_details {
        total
    } else {
        total.min(5)
    };
    items.push(TerminalToolBlockItem::Line(
        TerminalSegmentDisplayLine::new(
            format!("Checklist ({} items):", total),
            TerminalSegmentTone::Info,
        ),
    ));
    for item in checklist.iter().take(preview_limit) {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "{} {}",
                    if item.checked { "[x]" } else { "[ ]" },
                    format_preview_line(&item.text, 88)
                ),
                if item.checked {
                    TerminalSegmentTone::Success
                } else {
                    TerminalSegmentTone::Muted
                },
            ),
        ));
    }
    if total > preview_limit {
        items.push(TerminalToolBlockItem::Line(
            TerminalSegmentDisplayLine::new(
                format!(
                    "… ({} more items, toggle Tool Details to expand)",
                    total - preview_limit
                ),
                TerminalSegmentTone::Muted,
            ),
        ));
    }
}

fn extract_task_model_label(metadata: &HashMap<String, serde_json::Value>) -> Option<String> {
    let model = metadata.get("model")?.as_object()?;
    let provider = model
        .get("providerID")
        .or_else(|| model.get("provider_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let model_id = model
        .get("modelID")
        .or_else(|| model.get("model_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if provider.is_empty() && model_id.is_empty() {
        None
    } else if !provider.is_empty() && !model_id.is_empty() {
        Some(format!("{provider}:{model_id}"))
    } else {
        Some(format!("{provider}{model_id}"))
    }
}

fn is_ascii_todo_marker_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- [ ]")
        || trimmed.starts_with("- [x]")
        || trimmed.starts_with("- [X]")
        || trimmed.starts_with("- [~]")
    {
        return true;
    }

    let mut parts = trimmed.split_whitespace();
    let first = parts.next().unwrap_or_default();
    let second = parts.next().unwrap_or_default();
    first.starts_with("todo_")
        || second.starts_with("todo_")
        || first.starts_with("TODO_")
        || second.starts_with("TODO_")
}

fn is_legacy_todo_marker_line(line: &str) -> bool {
    line.starts_with("✅")
        || line.starts_with("🔄")
        || line.starts_with("⏳")
        || line.starts_with("☐")
        || line.starts_with("☑")
        || line.starts_with("❌")
}

fn is_todo_marker_line(line: &str) -> bool {
    is_ascii_todo_marker_line(line) || is_legacy_todo_marker_line(line)
}

fn detect_todo_status(line: &str) -> String {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("- [x]") {
        return "done".to_string();
    }
    if lower.starts_with("- [~]") {
        return "in progress".to_string();
    }
    if lower.starts_with("- [ ]") {
        return "pending".to_string();
    }
    if lower.contains("[done]") {
        return "done".to_string();
    }
    if lower.contains("[in_progress]") || lower.contains("[in-progress]") {
        return "in progress".to_string();
    }
    if lower.contains("[cancelled]") || lower.contains("[canceled]") {
        return "cancelled".to_string();
    }
    if lower.contains("[pending]") {
        return "pending".to_string();
    }

    if trimmed.starts_with("✅") || trimmed.starts_with("☑") {
        return "done".to_string();
    }
    if trimmed.starts_with("🔄") {
        return "in progress".to_string();
    }
    if trimmed.starts_with("❌") {
        return "cancelled".to_string();
    }
    if trimmed.starts_with("⏳") || trimmed.starts_with("☐") {
        return "pending".to_string();
    }

    "todo".to_string()
}

fn parse_todowrite_entries(result_text: &str) -> Vec<TodoPreviewEntry> {
    let raw_lines: Vec<&str> = result_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut entries = Vec::new();
    let mut idx = 0usize;

    while idx < raw_lines.len() {
        let line = raw_lines[idx];
        if line.starts_with('#') {
            idx += 1;
            continue;
        }

        if !is_todo_marker_line(line) {
            idx += 1;
            continue;
        }

        let status = detect_todo_status(line);
        let priority = ["high", "medium", "low"].iter().find_map(|p| {
            let tag = format!("[{}]", p);
            line.contains(&tag).then(|| p.to_string())
        });

        let mut text = line.to_string();
        if idx + 1 < raw_lines.len() {
            let next = raw_lines[idx + 1];
            let next_is_marker = next.starts_with('#') || is_todo_marker_line(next);
            if !next_is_marker {
                text = next.to_string();
                idx += 1;
            }
        }

        entries.push(TodoPreviewEntry {
            status,
            text,
            priority,
        });
        idx += 1;
    }

    entries
}

fn todo_status_tone(status: &str) -> TerminalSegmentTone {
    match status {
        "done" => TerminalSegmentTone::Success,
        "in progress" => TerminalSegmentTone::Warning,
        "cancelled" => TerminalSegmentTone::Error,
        _ => TerminalSegmentTone::Muted,
    }
}

fn parse_data_url(url: &str) -> Option<(&str, &str)> {
    let body = url.strip_prefix("data:")?;
    let (meta, payload) = body.split_once(',')?;
    let mime = meta
        .split(';')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("application/octet-stream");
    Some((mime, payload))
}

fn estimate_data_url_bytes(payload: &str) -> Option<usize> {
    let clean_len = payload
        .chars()
        .filter(|ch| !matches!(ch, '\n' | '\r' | ' ' | '\t'))
        .count();
    if clean_len == 0 {
        return None;
    }

    let padding = payload.chars().rev().take_while(|ch| *ch == '=').count();
    Some((clean_len.saturating_mul(3) / 4).saturating_sub(padding))
}

fn parse_numeric_token(token: &str) -> Option<usize> {
    let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<usize>().ok()
    }
}

fn sanitize_path_token(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .trim_matches('`')
        .trim_end_matches('.')
        .trim_end_matches(',')
        .to_string()
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
        build_display_hint_items, build_edit_result_items, build_file_items, build_image_items,
        build_patch_result_items, build_task_result_items, build_task_running_items,
        build_todowrite_result_items, build_tool_body_items, build_write_result_items,
        parse_write_summary, summarize_block_items_inline, TerminalToolBlockItem,
    };
    use crate::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};
    use std::collections::HashMap;

    #[test]
    fn parse_write_summary_from_success_message() {
        let summary =
            parse_write_summary("Successfully wrote 30199 bytes (725 lines) to ./t2.html")
                .expect("summary");
        assert_eq!(summary.size_bytes, Some(30199));
        assert_eq!(summary.total_lines, Some(725));
        assert_eq!(summary.path.as_deref(), Some("./t2.html"));
        assert_eq!(summary.verb, Some("wrote"));
    }

    #[test]
    fn write_items_include_diff_block() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "diff".to_string(),
            serde_json::json!("@@ -1 +1 @@\n-old\n+new"),
        );
        let items = build_write_result_items(
            "Successfully wrote 10 bytes (2 lines) to ./new_file.txt",
            r#"{"file_path":"./new_file.txt"}"#,
            Some(&metadata),
            true,
        );
        assert!(items
            .iter()
            .any(|item| matches!(item, TerminalToolBlockItem::Diff { .. })));
    }

    #[test]
    fn edit_items_include_diff_block() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "diff".to_string(),
            serde_json::json!("@@ -1 +1 @@\n-old\n+new"),
        );
        let items = build_edit_result_items(
            "Edit completed",
            r#"{"file_path":"test.rs"}"#,
            Some(&metadata),
            true,
        );
        assert!(items
            .iter()
            .any(|item| matches!(item, TerminalToolBlockItem::Diff { .. })));
    }

    #[test]
    fn patch_items_include_per_file_diff_labels() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "files".to_string(),
            serde_json::json!([
                {
                    "relativePath": "foo.rs",
                    "type": "update",
                    "diff": "@@ -1 +1 @@\n-old\n+new"
                }
            ]),
        );
        let items = build_patch_result_items("Patch applied", Some(&metadata), true);
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Diff { label: Some(label), .. }
                if label.text.contains("Patched foo.rs")
        )));
    }

    #[test]
    fn task_running_items_include_summary_and_checklist() {
        let items = build_task_running_items(
            r###"{"category":"visual-engineering","load_skills":["frontend-ui-ux","theme-factory"],"prompt":"## 1. TASK\nRedesign page\n- [ ] 修改 t2.html\n- [ ] 增强视觉冲击力"}"###,
        );

        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Subagent: visual-engineering"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Skills: 2"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text.contains("[ ] 修改 t2.html")
        )));
    }

    #[test]
    fn task_result_items_include_checklist_and_markdown_preview() {
        let mut metadata = HashMap::new();
        metadata.insert("hasTextOutput".to_string(), serde_json::json!(true));
        metadata.insert(
            "model".to_string(),
            serde_json::json!({
                "providerID": "zhipuai",
                "modelID": "glm-5",
            }),
        );
        let items = build_task_result_items(
            "task_id: abc123\ntask_status: completed\n<task_result>\n- [x] 修改 t2.html\n- [x] 增强视觉冲击力\n\n## Summary\nDone.\n</task_result>",
            r###"{"category":"visual-engineering","prompt":"## 1. TASK\nRedesign page"}"###,
            Some(&metadata),
            false,
        );

        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Task ID: abc123"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Model: zhipuai:glm-5"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Markdown { content }
                if content.contains("## Summary") && content.contains("Done.")
        )));
    }

    #[test]
    fn build_tool_body_items_prefers_task_renderer_over_display_hints() {
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

        let items = build_tool_body_items(
            "task",
            r###"{"category":"analysis","description":"Inspect migration status"}"###,
            TerminalToolState::Completed,
            Some(&result),
            false,
        );

        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Task ID: child-1"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Markdown { content } if content.contains("## Summary")
        )));
    }

    #[test]
    fn build_tool_body_items_prefers_question_renderer_over_display_hints() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("1 question answered"),
        );
        metadata.insert(
            "display.fields".to_string(),
            serde_json::json!([{ "label": "Scope", "value": "Proceed" }]),
        );
        let result = TerminalToolResultInfo {
            output: r#"{"answers":["Proceed"]}"#.to_string(),
            is_error: false,
            title: Some("User response received".to_string()),
            metadata: Some(metadata),
        };

        let items = build_tool_body_items(
            "question",
            r#"{"questions":[{"question":"Choose rollout scope","options":[{"label":"Proceed"}]}]}"#,
            TerminalToolState::Completed,
            Some(&result),
            true,
        );

        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line)
                if line.text.contains("Q: Choose rollout scope")
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text.contains("A: Proceed")
        )));
    }

    #[test]
    fn build_display_hint_items_include_preview_contract() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("api.semanticscholar.org · application/json"),
        );
        metadata.insert(
            "display.preview".to_string(),
            serde_json::json!({
                "kind": "text",
                "text": "{\"total\":2,\"data\":[{\"title\":\"Paper A\"}]}",
                "truncated": true
            }),
        );
        let info = TerminalToolResultInfo {
            output: String::new(),
            is_error: false,
            title: Some("WebFetch".to_string()),
            metadata: Some(metadata),
        };

        let items = build_display_hint_items(&info).expect("display hints");
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line)
                if line.text.contains("api.semanticscholar.org")
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line)
                if line.text.contains("{\"total\":2")
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Preview truncated."
        )));
    }

    #[test]
    fn todowrite_items_parse_ascii_markers_and_priority() {
        let items = build_todowrite_result_items(
            "## Todos\n- [x] [high]\n完成首页改版\n- [~] [medium]\n接线 settings panel\n- [ ] [low]\n补测试",
            false,
        );

        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "Todo List (3 items)"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line)
                if line.text.contains("[done] 完成首页改版") && line.text.contains("[high]")
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line)
                if line.text.contains("[in progress] 接线 settings panel")
        )));
    }

    #[test]
    fn file_items_include_path_and_mime() {
        let items = build_file_items("/tmp/demo.png", "image/png");
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "[file] /tmp/demo.png"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "type: image/png"
        )));
    }

    #[test]
    fn image_items_summarize_inline_data_url() {
        let items = build_image_items("data:image/png;base64,QUJDRA==");
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "[image] inline image"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "type: image/png"
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            TerminalToolBlockItem::Line(line) if line.text == "size: 4 B"
        )));
    }

    #[test]
    fn inline_summary_flattens_file_items() {
        let summary = summarize_block_items_inline(&build_file_items("/tmp/demo.png", "image/png"));
        assert_eq!(summary, "[file] /tmp/demo.png · type: image/png");
    }

    #[test]
    fn inline_summary_flattens_inline_image_items() {
        let summary =
            summarize_block_items_inline(&build_image_items("data:image/png;base64,QUJDRA=="));
        assert_eq!(
            summary,
            "[image] inline image · type: image/png · size: 4 B"
        );
    }
}
