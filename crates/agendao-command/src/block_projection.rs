use crate::output_blocks::{
    classify_tool_result_display, format_tool_header, BlockTone, MessageBlock, MessagePhase,
    MessageRole, OutputBlock, QueueItemBlock, ReasoningBlock, SessionEventBlock, SessionEventField,
    StatusBlock, ToolBlock, ToolPhase, ToolStructuredDetail, ToolWebField, ToolWebPreview,
};
use agendao_types::tool_call_observable_arguments;
use serde_json::json;
use std::collections::HashMap;

fn tool_summary_for_web(tool: &ToolBlock) -> Option<String> {
    match tool.phase {
        ToolPhase::Start | ToolPhase::Running => tool
            .detail
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned(),
        ToolPhase::Error => Some(
            tool.detail
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "unknown error".to_string()),
        ),
        ToolPhase::Done => classify_tool_result_display(tool)
            .and_then(|display| display.summary)
            .or_else(|| {
                tool.detail
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            })
            .or_else(|| Some("Done".to_string())),
    }
}

fn tool_fields_for_web(tool: &ToolBlock) -> Vec<ToolWebField> {
    if matches!(tool.phase, ToolPhase::Done) {
        if let Some(display) = classify_tool_result_display(tool) {
            if !display.fields.is_empty() {
                return display.fields;
            }
        }
    }

    let mut fields = Vec::new();
    if let Some(ref structured) = tool.structured {
        match structured {
            ToolStructuredDetail::FileEdit { file_path, .. }
            | ToolStructuredDetail::FileWrite { file_path, .. }
            | ToolStructuredDetail::FileRead { file_path, .. } => {
                fields.push(ToolWebField {
                    label: "File".to_string(),
                    value: file_path.clone(),
                });
            }
            ToolStructuredDetail::BashExec {
                command_preview,
                exit_code,
                ..
            } => {
                fields.push(ToolWebField {
                    label: "Command".to_string(),
                    value: command_preview.clone(),
                });
                if let Some(exit_code) = exit_code {
                    fields.push(ToolWebField {
                        label: "Exit".to_string(),
                        value: exit_code.to_string(),
                    });
                }
            }
            ToolStructuredDetail::Search {
                pattern, matches, ..
            } => {
                if !pattern.is_empty() {
                    fields.push(ToolWebField {
                        label: "Pattern".to_string(),
                        value: pattern.clone(),
                    });
                }
                if let Some(matches) = matches {
                    fields.push(ToolWebField {
                        label: "Matches".to_string(),
                        value: matches.to_string(),
                    });
                }
            }
            ToolStructuredDetail::Generic => {}
        }
    }
    fields
}

fn tool_preview_for_web(tool: &ToolBlock) -> Option<ToolWebPreview> {
    if matches!(tool.phase, ToolPhase::Done) {
        if let Some(display) = classify_tool_result_display(tool) {
            if display.preview.is_some() {
                return display.preview;
            }
        }
    }

    let structured = tool.structured.as_ref()?;
    match structured {
        ToolStructuredDetail::FileEdit { diff_preview, .. }
        | ToolStructuredDetail::FileWrite { diff_preview, .. } => {
            diff_preview.as_ref().map(|diff| ToolWebPreview {
                kind: "diff".to_string(),
                text: diff.clone(),
                truncated: false,
            })
        }
        ToolStructuredDetail::BashExec {
            output_preview,
            truncated,
            ..
        } => output_preview.as_ref().map(|preview| ToolWebPreview {
            kind: "code".to_string(),
            text: preview.clone(),
            truncated: *truncated,
        }),
        _ => None,
    }
}

pub fn history_tool_call_to_web(
    tool_call_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    status: Option<&str>,
    _raw: Option<&str>,
) -> serde_json::Value {
    let normalized_status = status.unwrap_or("pending");
    let detail = history_tool_call_detail(input, normalized_status);
    let structured = extract_tool_input_structured(tool_name, input);
    let phase = match normalized_status {
        "running" => ToolPhase::Running,
        "completed" => ToolPhase::Done,
        "error" => ToolPhase::Error,
        _ => ToolPhase::Start,
    };

    let mut block = ToolBlock {
        name: tool_name.to_string(),
        phase,
        detail,
        structured: None,
    };
    if let Some(structured) = structured {
        block = block.with_structured(structured);
    }

    let mut web = output_block_to_web(&OutputBlock::Tool(block));
    if let serde_json::Value::Object(ref mut map) = web {
        map.insert("id".to_string(), json!(tool_call_id));
    }
    apply_history_tool_call_display_override(&mut web, tool_name, input);
    web
}

pub fn history_tool_result_to_web(
    tool_call_id: &str,
    tool_name: &str,
    title: Option<&str>,
    content: &str,
    is_error: bool,
    metadata: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    let detail = history_tool_result_detail(title, content);
    let structured = extract_tool_result_structured(tool_name, content, metadata);
    let mut block = if is_error {
        ToolBlock::error(
            tool_name.to_string(),
            detail.unwrap_or_else(|| content.to_string()),
        )
    } else {
        ToolBlock::done(tool_name.to_string(), detail)
    };
    if let Some(structured) = structured {
        block = block.with_structured(structured);
    }
    let mut web = output_block_to_web(&OutputBlock::Tool(block));
    if let serde_json::Value::Object(ref mut map) = web {
        map.insert("id".to_string(), json!(tool_call_id));
    }
    apply_history_tool_result_display_override(&mut web, tool_name, title, metadata);
    apply_history_tool_result_interaction(&mut web, tool_name, title, content, is_error);
    web
}

pub fn history_session_event_to_web(
    event: &str,
    title: impl Into<String>,
    status: Option<&str>,
    summary: Option<String>,
    fields: Vec<(String, String, Option<String>)>,
    body: Option<String>,
) -> serde_json::Value {
    output_block_to_web(&OutputBlock::SessionEvent(SessionEventBlock {
        event: event.to_string(),
        title: title.into(),
        status: status.map(str::to_string),
        summary,
        fields: fields
            .into_iter()
            .map(|(label, value, tone)| SessionEventField { label, value, tone })
            .collect(),
        body,
    }))
}

fn history_tool_call_detail(input: &serde_json::Value, status: &str) -> Option<String> {
    let _ = status;
    tool_call_observable_arguments(input)
}

fn history_tool_result_detail(title: Option<&str>, content: &str) -> Option<String> {
    match title.map(str::trim).filter(|value| !value.is_empty()) {
        Some(title) => Some(format!("{title}: {content}")),
        None if content.trim().is_empty() => None,
        None => Some(content.to_string()),
    }
}

fn apply_history_tool_call_display_override(
    web: &mut serde_json::Value,
    tool_name: &str,
    input: &serde_json::Value,
) {
    match tool_name {
        "question" => {
            let Some(questions) = input.get("questions").and_then(|value| value.as_array()) else {
                return;
            };
            if questions.is_empty() {
                return;
            }
            let summary = Some(if questions.len() == 1 {
                "1 question requested".to_string()
            } else {
                format!("{} questions requested", questions.len())
            });
            let fields = questions
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let label = item
                        .get("header")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("Question {}", index + 1));
                    let question = item.get("question").and_then(|value| value.as_str())?;
                    Some(json!({
                        "label": label,
                        "value": question,
                    }))
                })
                .collect::<Vec<_>>();
            apply_display_override(web, summary, fields, None);
        }
        "todowrite" | "todo_write" => {
            let Some(todos) = input.get("todos").and_then(|value| value.as_array()) else {
                return;
            };
            let summary = Some(format!("{} todo items proposed", todos.len()));
            let fields = todo_summary_fields_from_array(todos);
            let preview = todo_preview_from_array(todos);
            apply_display_override(web, summary, fields, preview);
        }
        "todoread" | "todo_read" => {
            apply_display_override(
                web,
                Some("Read current todo list".to_string()),
                Vec::new(),
                None,
            );
        }
        _ => {}
    }
}

fn apply_history_tool_result_display_override(
    web: &mut serde_json::Value,
    tool_name: &str,
    title: Option<&str>,
    metadata: &HashMap<String, serde_json::Value>,
) {
    match tool_name {
        "question" => {
            let summary = metadata
                .get("display.summary")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or_else(|| title.map(str::to_string));
            let fields = metadata
                .get("display.fields")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|field| {
                            Some(json!({
                                "label": field
                                    .get("label")
                                    .or_else(|| field.get("key"))?
                                    .as_str()?,
                                "value": field.get("value")?.as_str().unwrap_or(""),
                            }))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            apply_display_override(web, summary, fields, None);
        }
        "todowrite" | "todo_write" | "todoread" | "todo_read" => {
            let todos = metadata
                .get("todos")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let summary = title.map(str::to_string).or_else(|| {
                metadata
                    .get("count")
                    .and_then(|value| value.as_u64())
                    .map(|count| format!("{count} todo items"))
            });
            let fields = todo_summary_fields_from_array(&todos);
            let preview = todo_preview_from_array(&todos);
            apply_display_override(web, summary, fields, preview);
        }
        _ => {}
    }
}

fn apply_history_tool_result_interaction(
    web: &mut serde_json::Value,
    tool_name: &str,
    title: Option<&str>,
    content: &str,
    is_error: bool,
) {
    if tool_name != "question" {
        return;
    }
    let status = if is_error {
        let lower = format!(
            "{} {}",
            title.unwrap_or_default().to_ascii_lowercase(),
            content.to_ascii_lowercase()
        );
        if lower.contains("reject") {
            "rejected"
        } else if lower.contains("cancel") {
            "cancelled"
        } else {
            "error"
        }
    } else {
        "answered"
    };
    let Some(map) = web.as_object_mut() else {
        return;
    };
    map.insert(
        "interaction".to_string(),
        json!({
            "type": "question",
            "status": status,
            "can_reply": false,
            "can_reject": false,
        }),
    );
}

fn apply_display_override(
    web: &mut serde_json::Value,
    summary: Option<String>,
    fields: Vec<serde_json::Value>,
    preview: Option<serde_json::Value>,
) {
    let Some(map) = web.as_object_mut() else {
        return;
    };
    let display = map
        .entry("display".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut();
    let Some(display) = display else {
        return;
    };
    if let Some(summary) = summary {
        display.insert("summary".to_string(), json!(summary));
    }
    if !fields.is_empty() {
        display.insert("fields".to_string(), serde_json::Value::Array(fields));
    }
    if let Some(preview) = preview {
        display.insert("preview".to_string(), preview);
    }
}

fn todo_summary_fields_from_array(todos: &[serde_json::Value]) -> Vec<serde_json::Value> {
    if todos.is_empty() {
        return Vec::new();
    }
    let mut pending = 0_u64;
    let mut in_progress = 0_u64;
    let mut completed = 0_u64;
    for todo in todos {
        match todo
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("pending")
        {
            "completed" => completed += 1,
            "in_progress" | "in-progress" | "in progress" => in_progress += 1,
            _ => pending += 1,
        }
    }
    vec![
        json!({ "label": "Count", "value": todos.len().to_string() }),
        json!({ "label": "Pending", "value": pending.to_string() }),
        json!({ "label": "In Progress", "value": in_progress.to_string() }),
        json!({ "label": "Completed", "value": completed.to_string() }),
    ]
}

fn todo_preview_from_array(todos: &[serde_json::Value]) -> Option<serde_json::Value> {
    if todos.is_empty() {
        return None;
    }
    let lines = todos
        .iter()
        .take(8)
        .filter_map(|todo| {
            let content = todo.get("content").and_then(|value| value.as_str())?;
            let status = todo
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("pending");
            Some(format!("- [{}] {}", status, content))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    Some(json!({
        "kind": "text",
        "text": lines.join("\n"),
        "truncated": todos.len() > lines.len(),
    }))
}

// ── Structured detail extraction ──────────────────────────────────────

/// Extract structured detail from tool call input arguments (for ToolStart/ToolEnd).
/// The `input` is the JSON value of the tool call arguments.
fn extract_tool_input_structured(
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<ToolStructuredDetail> {
    match tool_name {
        "edit" | "multiedit" => {
            let file_path = input
                .get("file_path")
                .or_else(|| input.get("filePath"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ToolStructuredDetail::FileEdit {
                file_path,
                diff_preview: None,
            })
        }
        "write" => {
            let file_path = input
                .get("file_path")
                .or_else(|| input.get("filePath"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ToolStructuredDetail::FileWrite {
                file_path,
                bytes: None,
                lines: None,
                diff_preview: None,
            })
        }
        "read" => {
            let file_path = input
                .get("file_path")
                .or_else(|| input.get("filePath"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ToolStructuredDetail::FileRead {
                file_path,
                total_lines: None,
                truncated: false,
            })
        }
        "bash" => {
            let command_preview = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ToolStructuredDetail::BashExec {
                command_preview,
                exit_code: None,
                output_preview: None,
                truncated: false,
            })
        }
        "grep" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ToolStructuredDetail::Search {
                pattern,
                matches: None,
                truncated: false,
            })
        }
        "glob" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ToolStructuredDetail::Search {
                pattern,
                matches: None,
                truncated: false,
            })
        }
        _ => None,
    }
}

/// Extract structured detail from tool result metadata (for ToolResult).
fn extract_tool_result_structured(
    tool_name: &str,
    output: &str,
    meta: &HashMap<String, serde_json::Value>,
) -> Option<ToolStructuredDetail> {
    match tool_name {
        "edit" | "multiedit" => {
            let file_path = meta_str(meta, "filepath").unwrap_or_default();
            let diff_preview = meta_str(meta, "diff");
            Some(ToolStructuredDetail::FileEdit {
                file_path,
                diff_preview,
            })
        }
        "write" => {
            let file_path = meta_str(meta, "filepath").unwrap_or_default();
            let bytes = meta_u64(meta, "bytes");
            let lines = meta_u64(meta, "lines");
            let diff_preview = meta_str(meta, "diff");
            Some(ToolStructuredDetail::FileWrite {
                file_path,
                bytes,
                lines,
                diff_preview,
            })
        }
        "read" => {
            let file_path = meta_str(meta, "filepath").unwrap_or_default();
            let total_lines = meta_u64(meta, "total_lines");
            let truncated = meta_bool(meta, "truncated");
            Some(ToolStructuredDetail::FileRead {
                file_path,
                total_lines,
                truncated,
            })
        }
        "bash" => {
            let command_preview = String::new(); // command is in tool input, not result metadata
            let exit_code = meta_i64(meta, "exit_code");
            // Use the tool output text as output preview for bash
            let output_preview = if output.trim().is_empty() {
                None
            } else {
                Some(output.to_string())
            };
            let truncated = meta_bool(meta, "truncated");
            Some(ToolStructuredDetail::BashExec {
                command_preview,
                exit_code,
                output_preview,
                truncated,
            })
        }
        "grep" => {
            let pattern = String::new(); // pattern is in tool input
            let matches = meta_u64(meta, "matches");
            let truncated = meta_bool(meta, "truncated");
            Some(ToolStructuredDetail::Search {
                pattern,
                matches,
                truncated,
            })
        }
        "glob" => {
            let pattern = String::new();
            let matches = meta_u64(meta, "count");
            let truncated = meta_bool(meta, "truncated");
            Some(ToolStructuredDetail::Search {
                pattern,
                matches,
                truncated,
            })
        }
        _ => None,
    }
}

// ── Metadata helpers ──────────────────────────────────────────────────

fn meta_str(meta: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    meta.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn meta_u64(meta: &HashMap<String, serde_json::Value>, key: &str) -> Option<u64> {
    meta.get(key).and_then(|v| v.as_u64())
}

fn meta_i64(meta: &HashMap<String, serde_json::Value>, key: &str) -> Option<i64> {
    meta.get(key).and_then(|v| v.as_i64())
}

fn meta_bool(meta: &HashMap<String, serde_json::Value>, key: &str) -> bool {
    meta.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn output_block_to_web(block: &OutputBlock) -> serde_json::Value {
    let mut web = match block {
        OutputBlock::Status(StatusBlock { tone, text }) => json!({
            "kind": "status",
            "tone": tone_to_web(tone),
            "text": text,
        }),
        OutputBlock::Message(MessageBlock { role, phase, text }) => json!({
            "kind": "message",
            "role": role_to_web(role),
            "phase": phase_to_web(phase),
            "text": text,
        }),
        OutputBlock::Reasoning(ReasoningBlock { phase, text }) => json!({
            "kind": "reasoning",
            "phase": phase_to_web(phase),
            "text": text,
        }),
        OutputBlock::Tool(ToolBlock {
            name,
            phase,
            detail,
            structured,
        }) => {
            let tool = ToolBlock {
                name: name.clone(),
                phase: *phase,
                detail: detail.clone(),
                structured: structured.clone(),
            };
            let mut obj = serde_json::json!({
                "kind": "tool",
                "name": name,
                "phase": tool_phase_to_web(phase),
                "detail": detail,
                "display": {
                    "header": format_tool_header(&tool),
                    "summary": tool_summary_for_web(&tool),
                    "fields": tool_fields_for_web(&tool).into_iter().map(|field| json!({
                        "label": field.label,
                        "value": field.value,
                    })).collect::<Vec<_>>(),
                    "preview": tool_preview_for_web(&tool).map(|preview| json!({
                        "kind": preview.kind,
                        "text": preview.text,
                        "truncated": preview.truncated,
                    })),
                }
            });
            if let Some(ref s) = structured {
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert("structured".to_string(), structured_to_web(s));
                }
            }
            obj
        }
        OutputBlock::SessionEvent(SessionEventBlock {
            event,
            title,
            status,
            summary,
            fields,
            body,
        }) => json!({
            "kind": "session_event",
            "event": event,
            "title": title,
            "status": status,
            "summary": summary,
            "fields": fields.iter().map(|field| json!({
                "label": field.label,
                "value": field.value,
                "tone": field.tone,
            })).collect::<Vec<_>>(),
            "body": body,
        }),
        OutputBlock::QueueItem(QueueItemBlock { position, text }) => json!({
            "kind": "queue_item",
            "position": position,
            "text": text,
            "display": {
                "summary": format!("Queued [{}] {}", position, text),
            }
        }),
        OutputBlock::Inspect(inspect) => json!({
            "kind": "inspect",
            "stage_ids": inspect.stage_ids,
            "filter_stage_id": inspect.filter_stage_id,
            "events": inspect.events.iter().map(|e| json!({
                "ts": e.ts,
                "event_type": e.event_type,
                "execution_id": e.execution_id,
                "stage_id": e.stage_id,
            })).collect::<Vec<_>>(),
        }),
    };
    attach_presentation_metadata(block, &mut web);
    web
}

fn attach_presentation_metadata(block: &OutputBlock, web: &mut serde_json::Value) {
    let serde_json::Value::Object(map) = web else {
        return;
    };

    let (group, slot, rank) = match block {
        OutputBlock::Message(message) => match message.role {
            MessageRole::User => ("prompt", "user", 0),
            MessageRole::System => ("system", "system", 0),
            MessageRole::Assistant => ("answer", "final_answer", 90),
        },
        OutputBlock::Reasoning(_) => ("reasoning", "reasoning", 10),
        OutputBlock::Tool(tool) => ("tool", tool.name.as_str(), 20),
        OutputBlock::SessionEvent(event) => ("event", event.event.as_str(), 25),
        OutputBlock::Inspect(_) => ("inspect", "inspect", 40),
        OutputBlock::Status(_) => ("status", "status", 5),
        OutputBlock::QueueItem(_) => ("queue", "queue", 0),
    };

    map.insert(
        "presentation".to_string(),
        json!({
            "group": group,
            "slot": slot,
            "rank": rank,
        }),
    );
}

pub fn output_blocks_to_web(blocks: &[OutputBlock]) -> Vec<serde_json::Value> {
    blocks.iter().map(output_block_to_web).collect()
}

fn tone_to_web(tone: &BlockTone) -> &'static str {
    match tone {
        BlockTone::Title => "title",
        BlockTone::Normal => "normal",
        BlockTone::Muted => "muted",
        BlockTone::Success => "success",
        BlockTone::Warning => "warning",
        BlockTone::Error => "error",
    }
}

fn role_to_web(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    }
}

fn phase_to_web(phase: &MessagePhase) -> &'static str {
    match phase {
        MessagePhase::Start => "start",
        MessagePhase::Delta => "delta",
        MessagePhase::End => "end",
        MessagePhase::Full => "full",
    }
}

fn tool_phase_to_web(phase: &ToolPhase) -> &'static str {
    match phase {
        ToolPhase::Start => "start",
        ToolPhase::Running => "running",
        ToolPhase::Done => "done",
        ToolPhase::Error => "error",
    }
}

fn structured_to_web(detail: &ToolStructuredDetail) -> serde_json::Value {
    match detail {
        ToolStructuredDetail::FileEdit {
            file_path,
            diff_preview,
        } => json!({
            "type": "file_edit",
            "file_path": file_path,
            "diff_preview": diff_preview,
        }),
        ToolStructuredDetail::FileWrite {
            file_path,
            bytes,
            lines,
            diff_preview,
        } => json!({
            "type": "file_write",
            "file_path": file_path,
            "bytes": bytes,
            "lines": lines,
            "diff_preview": diff_preview,
        }),
        ToolStructuredDetail::FileRead {
            file_path,
            total_lines,
            truncated,
        } => json!({
            "type": "file_read",
            "file_path": file_path,
            "total_lines": total_lines,
            "truncated": truncated,
        }),
        ToolStructuredDetail::BashExec {
            command_preview,
            exit_code,
            output_preview,
            truncated,
        } => json!({
            "type": "bash_exec",
            "command_preview": command_preview,
            "exit_code": exit_code,
            "output_preview": output_preview,
            "truncated": truncated,
        }),
        ToolStructuredDetail::Search {
            pattern,
            matches,
            truncated,
        } => json!({
            "type": "search",
            "pattern": pattern,
            "matches": matches,
            "truncated": truncated,
        }),
        ToolStructuredDetail::Generic => json!({
            "type": "generic",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_output_block_to_web_shape() {
        let block = OutputBlock::Message(MessageBlock::delta(MessageRole::Assistant, "hello"));
        let web = output_block_to_web(&block);
        assert_eq!(
            web.get("kind").and_then(|value| value.as_str()),
            Some("message")
        );
        assert_eq!(
            web.get("phase").and_then(|value| value.as_str()),
            Some("delta")
        );
        assert_eq!(
            web.get("role").and_then(|value| value.as_str()),
            Some("assistant")
        );
        assert_eq!(
            web.pointer("/presentation/group")
                .and_then(|value| value.as_str()),
            Some("answer")
        );
    }

    #[test]
    fn history_tool_result_preserves_id_and_structured_metadata() {
        let metadata = HashMap::from([
            ("filepath".to_string(), json!("/src/main.rs")),
            ("diff".to_string(), json!("+line")),
        ]);
        let web = history_tool_result_to_web(
            "tool_123",
            "edit",
            Some("Edited"),
            "done",
            false,
            &metadata,
        );
        assert_eq!(
            web.get("id").and_then(|value| value.as_str()),
            Some("tool_123")
        );
        assert_eq!(
            web.pointer("/structured/type")
                .and_then(|value| value.as_str()),
            Some("file_edit")
        );
        assert_eq!(
            web.pointer("/structured/file_path")
                .and_then(|value| value.as_str()),
            Some("/src/main.rs")
        );
    }

    #[test]
    fn history_tool_call_uses_observable_arguments() {
        let web = history_tool_call_to_web(
            "call_1",
            "read",
            &json!({"filePath": "src/lib.rs", "offset": 4}),
            Some("running"),
            None,
        );
        assert_eq!(
            web.get("id").and_then(|value| value.as_str()),
            Some("call_1")
        );
        assert!(web
            .pointer("/display/summary")
            .and_then(|value| value.as_str())
            .is_some());
    }

    #[test]
    fn history_session_event_serializes_typed_card() {
        let web = history_session_event_to_web(
            "compaction",
            "Context compacted",
            Some("completed"),
            Some("Freed context".to_string()),
            vec![("Tokens".to_string(), "1024".to_string(), None)],
            None,
        );
        assert_eq!(
            web.get("kind").and_then(|value| value.as_str()),
            Some("session_event")
        );
        assert_eq!(
            web.get("event").and_then(|value| value.as_str()),
            Some("compaction")
        );
    }
}
