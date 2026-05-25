use serde_json::Value;

use crate::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSegmentTone {
    Primary,
    Muted,
    Success,
    Error,
    Info,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSegmentDisplayLine {
    pub text: String,
    pub tone: TerminalSegmentTone,
}

impl TerminalSegmentDisplayLine {
    pub fn new(text: impl Into<String>, tone: TerminalSegmentTone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

pub fn tool_glyph(name: &str) -> &'static str {
    match name {
        "bash" | "shell" => "$",
        "read" | "readFile" | "read_file" => "→",
        "write" | "writeFile" | "write_file" => "←",
        "edit" | "editFile" | "edit_file" => "←",
        "glob" | "grep" | "search" | "ripgrep" => "✱",
        "list" | "ls" | "listDir" | "list_dir" => "→",
        "webfetch" | "web_fetch" | "fetch" => "%",
        "codesearch" | "code_search" => "◇",
        "websearch" | "web_search" => "◈",
        "task" | "subagent" => "#",
        "apply_patch" | "applyPatch" => "%",
        "skill" => "⚙",
        "batch" => "⫘",
        "question" => "?",
        "todowrite" | "todo_write" | "todoRead" | "todo_read" => "☐",
        _ => "⚙",
    }
}

pub fn normalize_tool_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

pub fn format_preview_line(line: &str, max_chars: usize) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated)
}

pub fn is_denied_result(result_text: &str) -> bool {
    let lower = result_text.to_ascii_lowercase();
    lower.contains("permission denied")
        || lower.contains("denied")
        || lower.contains("not permitted")
        || lower.contains("forbidden")
}

pub fn tool_argument_preview(normalized_name: &str, arguments: &str) -> Option<String> {
    let raw = arguments.trim();
    let parsed = serde_json::from_str::<Value>(raw).ok();
    let object = parsed.as_ref().and_then(|v| v.as_object());

    if normalized_name == "bash" || normalized_name == "shell" {
        let command = parsed
            .as_ref()
            .and_then(extract_shell_command)
            .or_else(|| (!raw.is_empty()).then_some(raw.to_string()))?;
        return Some(format!("$ {}", command.trim()));
    }

    if matches!(
        normalized_name,
        "read" | "readfile" | "read_file" | "list" | "ls" | "listdir" | "list_dir"
    ) {
        return Some(format!(
            "→ {}",
            parsed
                .as_ref()
                .and_then(extract_path)
                .unwrap_or_else(|| ".".to_string())
        ));
    }

    if matches!(
        normalized_name,
        "write" | "writefile" | "write_file" | "edit" | "editfile" | "edit_file"
    ) {
        if let Some(path) = parsed.as_ref().and_then(extract_path) {
            return Some(format!("← {}", path));
        }
        if let Some(path) = extract_jsonish_path_from_raw(raw) {
            return Some(format!("← {}", path));
        }
    }

    if matches!(normalized_name, "webfetch" | "web_fetch") {
        if let Some(url) = parsed
            .as_ref()
            .and_then(|value| extract_string_key(value, &["url"]))
        {
            return Some(url);
        }
    }

    if matches!(
        normalized_name,
        "codesearch" | "code_search" | "websearch" | "web_search" | "grep" | "glob"
    ) {
        if let Some(query) = parsed
            .as_ref()
            .and_then(|value| extract_string_key(value, &["query", "pattern"]))
        {
            let path = parsed.as_ref().and_then(extract_path);
            return Some(match path {
                Some(path) => format!("\"{}\" in {}", query, path),
                None => format!("\"{}\"", query),
            });
        }
    }

    if normalized_name == "batch" {
        if let Some(calls) = parsed
            .as_ref()
            .and_then(|v| v.get("toolCalls").or_else(|| v.get("tool_calls")))
            .and_then(|v| v.as_array())
        {
            let count = calls.len();
            let mut names = Vec::new();
            for call in calls {
                if let Some(name) = call
                    .get("tool")
                    .or_else(|| call.get("name"))
                    .or_else(|| call.get("tool_name"))
                    .and_then(|v| v.as_str())
                {
                    if !names.iter().any(|seen| seen == name) {
                        names.push(name.to_string());
                    }
                }
            }
            return if names.is_empty() {
                Some(format!("{} tools", count))
            } else {
                Some(format!("{} tools ({})", count, names.join(", ")))
            };
        }
    }

    if normalized_name == "question" {
        if let Some(questions) = parsed
            .as_ref()
            .and_then(|v| v.get("questions"))
            .and_then(|v| v.as_array())
        {
            let count = questions.len();
            return Some(match count {
                0 => "0 questions".to_string(),
                1 => "1 question".to_string(),
                _ => format!("{count} questions"),
            });
        }
    }

    if matches!(normalized_name, "todowrite" | "todo_write") {
        if let Some(todos) = parsed
            .as_ref()
            .and_then(|v| v.get("todos"))
            .and_then(|v| v.as_array())
        {
            let count = todos.len();
            return Some(match count {
                0 => "0 todos".to_string(),
                1 => "1 todo".to_string(),
                _ => format!("{count} todos"),
            });
        }
    }

    if normalized_name == "task" {
        let category = parsed
            .as_ref()
            .and_then(|value| extract_string_key(value, &["category", "type", "subagent_type"]));
        let description = parsed
            .as_ref()
            .and_then(|value| extract_string_key(value, &["description"]));
        let prompt = parsed
            .as_ref()
            .and_then(|value| extract_string_key(value, &["prompt", "input", "text"]))
            .and_then(|value| extract_task_prompt_preview(&value));
        return match (category, description, prompt) {
            (Some(category), Some(description), _) => Some(format!(
                "{} task {}",
                category,
                format_preview_line(&description, 56)
            )),
            (Some(category), None, Some(prompt)) => Some(format!("{category} task {prompt}")),
            (Some(category), None, None) => Some(format!("{category} task")),
            (None, Some(description), _) => Some(format_preview_line(&description, 72)),
            (None, None, Some(prompt)) => Some(prompt),
            (None, None, None) => None,
        };
    }

    if matches!(normalized_name, "apply_patch" | "applypatch") {
        return Some("Patch".to_string());
    }

    if raw.is_empty() {
        return None;
    }

    if let Some(preview) = object.and_then(|value| {
        format_primitive_arguments(
            value,
            &[
                "content",
                "new_string",
                "old_string",
                "patch",
                "prompt",
                "questions",
                "todos",
            ],
        )
    }) {
        return Some(preview);
    }

    let first = raw.lines().next().unwrap_or(raw).trim();
    if first.is_empty() {
        None
    } else {
        Some(format_preview_line(first, 84))
    }
}

pub fn render_tool_segment_lines(
    name: &str,
    arguments: &str,
    state: TerminalToolState,
    result: Option<&TerminalToolResultInfo>,
    show_tool_details: bool,
) -> Vec<TerminalSegmentDisplayLine> {
    let normalized = normalize_tool_name(name);
    if matches!(state, TerminalToolState::Completed)
        && !show_tool_details
        && !matches!(normalized.as_str(), "task" | "todowrite" | "todo_write")
    {
        return Vec::new();
    }

    let mut lines = vec![render_tool_header_line(name, arguments, state)];

    if let Some(info) = result {
        if info.is_error {
            let first_line = info.output.lines().next().unwrap_or(&info.output).trim();
            let mut text = format!("Error: {}", format_preview_line(first_line, 96));
            if is_denied_result(&info.output) {
                text.push_str(" (denied)");
            }
            lines.push(TerminalSegmentDisplayLine::new(
                text,
                TerminalSegmentTone::Error,
            ));
            return lines;
        }

        if let Some(summary) = info
            .metadata
            .as_ref()
            .and_then(|m| m.get("display.summary"))
            .and_then(|v| v.as_str())
        {
            lines.push(TerminalSegmentDisplayLine::new(
                format_preview_line(summary, 96),
                TerminalSegmentTone::Muted,
            ));
            if let Some(metadata) = info.metadata.as_ref() {
                if let Some(fields) = metadata.get("display.fields").and_then(|v| v.as_array()) {
                    if let Some(field) = fields.first() {
                        let label = field
                            .get("label")
                            .or_else(|| field.get("key"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("?");
                        let value = field
                            .get("value")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");
                        let rendered = format!("{}: {}", label, format_preview_line(value, 80));
                        lines.push(TerminalSegmentDisplayLine::new(
                            rendered,
                            TerminalSegmentTone::Primary,
                        ));
                    }
                } else if let Some(preview) = metadata
                    .get("display.preview")
                    .and_then(|value| value.as_object())
                    .and_then(|preview| preview.get("text"))
                    .and_then(|value| value.as_str())
                {
                    let first_line = preview.lines().next().unwrap_or(preview).trim();
                    if !first_line.is_empty() {
                        lines.push(TerminalSegmentDisplayLine::new(
                            format_preview_line(first_line, 96),
                            TerminalSegmentTone::Muted,
                        ));
                    }
                }
            }
            return lines;
        }

        let output_lines = info
            .output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        if output_lines.is_empty() {
            return lines;
        }

        let first_line = format_preview_line(output_lines[0], 96);
        if show_tool_details {
            lines.push(TerminalSegmentDisplayLine::new(
                first_line,
                TerminalSegmentTone::Muted,
            ));
            for line in output_lines.iter().skip(1).take(4) {
                lines.push(TerminalSegmentDisplayLine::new(
                    format_preview_line(line, 96),
                    TerminalSegmentTone::Muted,
                ));
            }
            let shown = output_lines.len().min(5);
            if output_lines.len() > shown {
                lines.push(TerminalSegmentDisplayLine::new(
                    format!("… ({} more lines)", output_lines.len() - shown),
                    TerminalSegmentTone::Muted,
                ));
            }
        } else {
            let suffix = if output_lines.len() > 1 {
                format!("{} (+{} lines)", first_line, output_lines.len() - 1)
            } else {
                first_line
            };
            lines.push(TerminalSegmentDisplayLine::new(
                suffix,
                TerminalSegmentTone::Muted,
            ));
        }
    }

    lines
}

pub fn render_tool_header_line(
    name: &str,
    arguments: &str,
    state: TerminalToolState,
) -> TerminalSegmentDisplayLine {
    let normalized = normalize_tool_name(name);
    let icon = match state {
        TerminalToolState::Pending => "◯",
        TerminalToolState::Running => "◌",
        TerminalToolState::Completed => "●",
        TerminalToolState::Failed => "✗",
    };
    let tone = match state {
        TerminalToolState::Pending => TerminalSegmentTone::Warning,
        TerminalToolState::Running => TerminalSegmentTone::Info,
        TerminalToolState::Completed => TerminalSegmentTone::Success,
        TerminalToolState::Failed => TerminalSegmentTone::Error,
    };

    let mut header = format!("{} {} {}", icon, tool_glyph(name), name);
    if let Some(preview) = tool_argument_preview(&normalized, arguments) {
        header.push_str("  ");
        header.push_str(&preview);
    }

    TerminalSegmentDisplayLine::new(header, tone)
}

pub fn render_file_segment_line(path: &str, mime: &str) -> TerminalSegmentDisplayLine {
    TerminalSegmentDisplayLine::new(
        format!("[file] {} ({})", path, mime),
        TerminalSegmentTone::Info,
    )
}

pub fn render_image_segment_line(url: &str) -> TerminalSegmentDisplayLine {
    TerminalSegmentDisplayLine::new(format!("[image] {}", url), TerminalSegmentTone::Info)
}

fn extract_shell_command(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in ["command", "cmd", "script", "input", "text"] {
        if let Some(command) = object.get(key).and_then(|v| v.as_str()) {
            let trimmed = command.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn extract_path(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in [
        "path",
        "file_path",
        "filePath",
        "file",
        "filename",
        "filepath",
        "absolute_path",
        "absolutePath",
        "target",
        "destination",
        "to",
        "from",
    ] {
        if let Some(path) = object.get(key).and_then(|v| v.as_str()) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn extract_string_key(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(content) = object.get(*key).and_then(|value| value.as_str()) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn format_primitive_arguments(
    object: &serde_json::Map<String, Value>,
    omit: &[&str],
) -> Option<String> {
    let mut parts = Vec::new();

    for (key, value) in object {
        if omit.contains(&key.as_str()) {
            continue;
        }

        let rendered = match value {
            Value::String(content) => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    continue;
                }
                format_preview_line(trimmed, 28)
            }
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => continue,
        };

        parts.push(format!("{key}={rendered}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(format!("[{}]", parts.join(", ")))
    }
}

fn extract_task_prompt_preview(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }

    for line in trimmed.lines() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        if candidate.starts_with('#') {
            continue;
        }
        if let Some(rest) = candidate
            .strip_prefix("- [ ] ")
            .or_else(|| candidate.strip_prefix("* [ ] "))
        {
            return Some(format_preview_line(rest.trim(), 72));
        }
        return Some(format_preview_line(candidate, 72));
    }

    None
}

fn extract_jsonish_string_field(input: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\"", field);
    let field_idx = input.find(&needle)?;
    let after_field = &input[field_idx + needle.len()..];
    let colon_idx = after_field.find(':')?;
    let mut chars = after_field[colon_idx + 1..].chars().peekable();

    while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
        chars.next();
    }
    if !matches!(chars.next(), Some('"')) {
        return None;
    }

    let mut value = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            match ch {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                '/' => value.push('/'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                other => value.push(other),
            }
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => return Some(value),
            other => value.push(other),
        }
    }

    Some(value)
}

pub fn extract_jsonish_path_from_raw(raw: &str) -> Option<String> {
    let direct = extract_jsonish_string_field(raw, "file_path")
        .or_else(|| extract_jsonish_string_field(raw, "filePath"));
    if direct.is_some() {
        return direct;
    }

    if raw.contains("\\\"") {
        let de_escaped = raw.replace("\\\"", "\"");
        return extract_jsonish_string_field(&de_escaped, "file_path")
            .or_else(|| extract_jsonish_string_field(&de_escaped, "filePath"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        format_preview_line, render_file_segment_line, render_image_segment_line,
        render_tool_segment_lines, tool_argument_preview,
    };
    use crate::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};
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
    fn preview_line_truncates_with_ellipsis() {
        let rendered = format_preview_line("abcdefghijklmnopqrstuvwxyz", 8);
        assert_eq!(rendered, "abcdefg…");
    }

    #[test]
    fn generic_preview_compacts_json_to_key_values() {
        let preview = tool_argument_preview("unknown", r#"{"path":".","recursive":true}"#);
        assert_eq!(preview.as_deref(), Some("[path=., recursive=true]"));
    }

    #[test]
    fn batch_preview_shows_tool_count_and_names() {
        let args = r#"{"toolCalls":[{"tool":"read"},{"tool":"edit"},{"tool":"read"}]}"#;
        let preview = tool_argument_preview("batch", args);
        assert_eq!(preview.as_deref(), Some("3 tools (read, edit)"));
    }

    #[test]
    fn question_preview_shows_question_count() {
        let args = r#"{"questions":[{"question":"Pick one"},{"question":"Pick two"}]}"#;
        let preview = tool_argument_preview("question", args);
        assert_eq!(preview.as_deref(), Some("2 questions"));
    }

    #[test]
    fn todowrite_preview_shows_todo_count() {
        let args = r#"{"todos":[{"content":"Add tests"},{"content":"Refine TUI"}]}"#;
        let preview = tool_argument_preview("todowrite", args);
        assert_eq!(preview.as_deref(), Some("2 todos"));
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
    fn file_and_image_lines_use_shared_labels() {
        assert_eq!(
            render_file_segment_line("/tmp/a.png", "image/png").text,
            "[file] /tmp/a.png (image/png)"
        );
        assert_eq!(
            render_image_segment_line("https://example.com/a.png").text,
            "[image] https://example.com/a.png"
        );
    }

    #[test]
    fn tool_segment_lines_include_display_field_after_summary() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!("11 skills · literature-research/skills"),
        );
        metadata.insert(
            "display.fields".to_string(),
            serde_json::json!([{ "label": "Scope", "value": "literature-research/skills" }]),
        );
        let result = TerminalToolResultInfo {
            output: String::new(),
            is_error: false,
            title: Some("SkillsList".to_string()),
            metadata: Some(metadata),
        };

        let lines = render_tool_segment_lines(
            "SkillsList",
            "{}",
            TerminalToolState::Completed,
            Some(&result),
            true,
        );

        assert!(lines.iter().any(|line| line.text.contains("11 skills")));
        assert!(lines
            .iter()
            .any(|line| line.text.contains("Scope: literature-research/skills")));
    }
}
