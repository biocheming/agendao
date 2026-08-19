pub use agendao_output_blocks::*;

#[cfg(feature = "terminal-ui")]
use crate::cli_markdown;
#[cfg(feature = "terminal-ui")]
use crate::cli_panel::truncate_display;
#[cfg(feature = "terminal-ui")]
use crate::cli_style::CliStyle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolWebField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolWebPreview {
    pub kind: String,
    pub text: String,
    pub truncated: bool,
}

pub fn render_cli_block(block: &OutputBlock) -> String {
    match block {
        OutputBlock::Status(status) => render_status_block(status),
        OutputBlock::Message(message) => render_message_block(message),
        OutputBlock::Reasoning(reasoning) => render_reasoning_block(reasoning),
        OutputBlock::Tool(tool) => render_tool_block(tool),
        OutputBlock::SessionEvent(event) => render_session_event_block(event),
        OutputBlock::QueueItem(item) => render_queue_item_block(item),
        OutputBlock::Inspect(inspect) => render_inspect_block(inspect),
    }
}

fn render_status_block(status: &StatusBlock) -> String {
    let label = match status.tone {
        BlockTone::Title => "STATUS",
        BlockTone::Normal => "status",
        BlockTone::Muted => "status",
        BlockTone::Success => "status+",
        BlockTone::Warning => "status!",
        BlockTone::Error => "status-",
    };
    format!("[{label}] {}\n", status.text)
}

fn render_message_block(message: &MessageBlock) -> String {
    let role = match message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    };
    match message.phase {
        MessagePhase::Start => format!("[message:{role}] "),
        MessagePhase::Delta => message.text.clone(),
        MessagePhase::End => "\n".to_string(),
        MessagePhase::Full => format!("[message:{role}] {}\n", message.text),
    }
}

fn render_reasoning_block(reasoning: &ReasoningBlock) -> String {
    match reasoning.phase {
        MessagePhase::Start => "\n[thinking]\n│ ".to_string(),
        MessagePhase::Delta => {
            let cleaned = strip_think_tags(&reasoning.text);
            if cleaned.is_empty() {
                String::new()
            } else {
                indent_continuation_lines(&cleaned, "│ ")
            }
        }
        MessagePhase::End => "\n".to_string(),
        MessagePhase::Full => {
            let cleaned = strip_think_tags(&reasoning.text).trim().to_string();
            if cleaned.is_empty() {
                String::new()
            } else {
                format!(
                    "[thinking]\n│ {}\n",
                    indent_continuation_lines(&cleaned, "│ ")
                )
            }
        }
    }
}

fn render_tool_block(tool: &ToolBlock) -> String {
    let phase = match tool.phase {
        ToolPhase::Start => "start",
        ToolPhase::Running => "running",
        ToolPhase::Done => "done",
        ToolPhase::Error => "error",
    };
    let label = if is_skill_tool_name(&tool.name) {
        tool_cli_activity_label(tool)
    } else {
        tool.name.clone()
    };
    match &tool.detail {
        Some(detail) if !detail.trim().is_empty() => {
            format!("[tool:{phase}] {label} :: {}\n", detail)
        }
        _ => format!("[tool:{phase}] {label}\n"),
    }
}

fn render_session_event_block(event: &SessionEventBlock) -> String {
    let mut out = String::new();
    let status = event
        .status
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" · {value}"))
        .unwrap_or_default();
    out.push_str(&format!(
        "[session_event] {} [{}{}]\n",
        event.title, event.event, status
    ));
    if let Some(summary) = event.summary.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(&format!("  summary: {summary}\n"));
    }
    for field in &event.fields {
        out.push_str(&format!("  {}: {}\n", field.label, field.value));
    }
    if let Some(body) = event.body.as_deref().filter(|value| !value.is_empty()) {
        out.push_str("  body:\n");
        for line in body.lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out
}

fn render_queue_item_block(item: &QueueItemBlock) -> String {
    format!("[queue_item] [{}] {}\n", item.position, item.text)
}

fn render_inspect_block(inspect: &InspectBlock) -> String {
    let mut out = String::new();
    if let Some(ref stage_id) = inspect.filter_stage_id {
        out.push_str(&format!("[inspect] Stage: {stage_id}\n"));
        out.push_str(&format!("{}  events:\n", "─".repeat(40)));
        if inspect.events.is_empty() {
            out.push_str("  (no events)\n");
        } else {
            for row in &inspect.events {
                let eid = row.execution_id.as_deref().unwrap_or("—");
                out.push_str(&format!(
                    "  ts={} type={} exec={}\n",
                    row.ts, row.event_type, eid,
                ));
            }
        }
    } else {
        out.push_str(&format!(
            "[inspect] {} stage{} in session\n",
            inspect.stage_ids.len(),
            if inspect.stage_ids.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
        for sid in &inspect.stage_ids {
            out.push_str(&format!("  • {sid}\n"));
        }
        if inspect.stage_ids.is_empty() {
            out.push_str("  (no stages recorded)\n");
        }
    }
    out
}

// ── Rich rendering ──────────────────────────────────────────────────

/// Render an `OutputBlock` with ANSI colors, icons, and structure.
/// Falls back to plain text when `style.color` is false.
#[cfg(feature = "terminal-ui")]
pub fn render_cli_block_rich(block: &OutputBlock, style: &CliStyle) -> String {
    if !style.color {
        return render_cli_block(block);
    }
    match block {
        OutputBlock::Status(status) => render_status_rich(status, style),
        OutputBlock::Message(message) => render_message_rich(message, style),
        OutputBlock::Reasoning(reasoning) => render_reasoning_rich(reasoning, style),
        OutputBlock::Tool(tool) => render_tool_rich(tool, style),
        OutputBlock::SessionEvent(event) => render_session_event_rich(event, style),
        OutputBlock::QueueItem(item) => render_queue_item_rich(item, style),
        OutputBlock::Inspect(inspect) => render_inspect_rich(inspect, style),
    }
}

#[cfg(not(feature = "terminal-ui"))]
pub fn render_cli_block_rich(block: &OutputBlock, _style: &()) -> String {
    render_cli_block(block)
}

#[cfg(feature = "terminal-ui")]
fn render_inspect_rich(inspect: &InspectBlock, style: &CliStyle) -> String {
    let plain = render_inspect_block(inspect);
    let mut out = String::new();
    out.push_str(&format!(
        "{} {} {}\n",
        render_block_badge(style, "INSPECT", (244, 251, 255), (60, 76, 120)),
        style.bold_cyan(style.tree_end()),
        style.bold("Inspection")
    ));
    for line in plain.lines() {
        out.push_str(&format!("  {}\n", line));
    }
    append_block_divider(out, style, (60, 76, 120))
}

#[cfg(feature = "terminal-ui")]
fn render_status_rich(status: &StatusBlock, style: &CliStyle) -> String {
    let (badge, icon, body, divider) = match status.tone {
        BlockTone::Title => (
            render_block_badge(style, "STATUS", (255, 255, 255), (28, 94, 168)),
            style.bold_cyan(style.bullet()),
            style.bold(&status.text),
            (28, 94, 168),
        ),
        BlockTone::Normal => (
            render_block_badge(style, "NOTE", (244, 247, 250), (80, 96, 112)),
            style.dim(style.bullet()),
            style.dim(&status.text),
            (80, 96, 112),
        ),
        BlockTone::Muted => (
            render_block_badge(style, "INFO", (230, 235, 240), (92, 92, 92)),
            style.dim(style.bullet()),
            style.dim(&status.text),
            (92, 92, 92),
        ),
        BlockTone::Success => (
            render_block_badge(style, "DONE", (245, 255, 246), (26, 129, 74)),
            style.bold_green(style.check()),
            style.green(&status.text),
            (26, 129, 74),
        ),
        BlockTone::Warning => (
            render_block_badge(style, "WARN", (33, 28, 12), (245, 190, 64)),
            style.bold_yellow(style.warning_icon()),
            style.yellow(&status.text),
            (245, 190, 64),
        ),
        BlockTone::Error => (
            render_block_badge(style, "ERROR", (255, 244, 244), (166, 42, 42)),
            style.bold_red(style.cross()),
            style.red(&status.text),
            (166, 42, 42),
        ),
    };
    append_block_divider(format!("{badge} {icon} {body}\n"), style, divider)
}

#[cfg(feature = "terminal-ui")]
fn render_message_rich(message: &MessageBlock, style: &CliStyle) -> String {
    match message.phase {
        MessagePhase::Start => {
            let bullet = render_message_bullet(message.role, style);
            let badge = render_message_badge(message.role, style);
            format!("{badge} {bullet} ")
        }
        MessagePhase::Delta => render_message_delta(&message.text, message.role, style),
        MessagePhase::End => format!(
            "\n{}\n",
            render_message_divider_for_role(style, message.role)
        ),
        MessagePhase::Full => {
            let rendered = render_message_body(&message.text, message.role, style);
            let indent = match message.role {
                MessageRole::User => "  ",
                MessageRole::Assistant => "  ",
                MessageRole::System => "  ",
            };
            let bullet = render_message_bullet(message.role, style);
            let badge = render_message_badge(message.role, style);
            let indented = indent_continuation_lines(rendered.trim_end(), indent);
            format!(
                "{} {} {}\n{}\n",
                badge,
                bullet,
                indented,
                render_message_divider_for_role(style, message.role)
            )
        }
    }
}

#[cfg(feature = "terminal-ui")]
fn render_message_bullet(role: MessageRole, style: &CliStyle) -> String {
    match role {
        MessageRole::User => style.bold_green(style.bullet()),
        MessageRole::Assistant => style.bold_cyan(style.bullet()),
        MessageRole::System => style.bold_yellow(style.bullet()),
    }
}

#[cfg(feature = "terminal-ui")]
fn render_message_badge(role: MessageRole, style: &CliStyle) -> String {
    match role {
        MessageRole::User => render_block_badge(style, "USER", (248, 255, 249), (24, 132, 83)),
        MessageRole::Assistant => {
            render_block_badge(style, "ASSIST", (244, 251, 255), (28, 112, 166))
        }
        MessageRole::System => render_block_badge(style, "SYSTEM", (35, 27, 5), (240, 197, 71)),
    }
}

#[cfg(feature = "terminal-ui")]
fn render_message_body(text: &str, role: MessageRole, style: &CliStyle) -> String {
    match role {
        MessageRole::User => style.green(text),
        MessageRole::Assistant => cli_markdown::render_markdown(text, style),
        MessageRole::System => style.yellow(text),
    }
}

#[cfg(feature = "terminal-ui")]
fn render_message_delta(text: &str, role: MessageRole, style: &CliStyle) -> String {
    match role {
        MessageRole::User => style.green(text),
        MessageRole::Assistant | MessageRole::System => text.to_string(),
    }
}

#[cfg(feature = "terminal-ui")]
fn render_message_divider_for_role(style: &CliStyle, role: MessageRole) -> String {
    match role {
        MessageRole::User => render_block_divider(style, (24, 132, 83)),
        MessageRole::Assistant => render_block_divider(style, (28, 112, 166)),
        MessageRole::System => render_block_divider(style, (240, 197, 71)),
    }
}

fn indent_continuation_lines(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len() + prefix.len() * 2);
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(prefix);
            }
        }
        out.push_str(line);
    }
    out
}

/// Strip `<think>` / `</think>` / `<think/>` tags that some models wrap around
/// reasoning content (e.g. GLM-5, DeepSeek).
fn strip_think_tags(text: &str) -> String {
    text.replace("<think>", "")
        .replace("</think>", "")
        .replace("<think/>", "")
}

#[cfg(feature = "terminal-ui")]
fn render_reasoning_rich(reasoning: &ReasoningBlock, style: &CliStyle) -> String {
    let header_badge = render_block_badge(style, "THINKING", (35, 27, 5), (240, 197, 71));
    let header_bullet = style.bold_yellow(style.bullet());
    let continuation_prefix = "  ";
    match reasoning.phase {
        MessagePhase::Start => format!("{header_badge} {header_bullet} "),
        MessagePhase::Delta => {
            let cleaned = strip_think_tags(&reasoning.text);
            if cleaned.is_empty() {
                String::new()
            } else {
                let indented = indent_continuation_lines(&cleaned, continuation_prefix);
                style.dim(&indented)
            }
        }
        MessagePhase::End => format!("\n{}\n", render_block_divider(style, (240, 197, 71))),
        MessagePhase::Full => {
            let cleaned = strip_think_tags(&reasoning.text).trim().to_string();
            if cleaned.is_empty() {
                String::new()
            } else {
                let indented = indent_continuation_lines(&cleaned, continuation_prefix);
                append_block_divider(
                    format!("{header_badge} {header_bullet} {}\n", style.dim(&indented)),
                    style,
                    (240, 197, 71),
                )
            }
        }
    }
}

#[cfg(feature = "terminal-ui")]
fn render_tool_rich(tool: &ToolBlock, style: &CliStyle) -> String {
    match tool.phase {
        ToolPhase::Start => render_tool_header_line(tool, style),
        ToolPhase::Running => {
            let detail = tool.detail.as_deref().unwrap_or("");
            if detail.is_empty() {
                String::new()
            } else {
                let collapsed = style.collapse_with_width(detail, 5, 2, None);
                format!(
                    "  {} {}\n",
                    style.dim(style.tree_end()),
                    style.dim(&collapsed)
                )
            }
        }
        ToolPhase::Done => render_tool_done_rich(tool, style),
        ToolPhase::Error => {
            let detail = tool.detail.as_deref().unwrap_or("unknown error");
            let collapsed = style.collapse(detail, 5, 2);
            append_block_divider(
                format!(
                    "{}  {} {}\n",
                    render_tool_header_line(tool, style),
                    style.tree_end(),
                    style.red(&format!("Error: {}", collapsed))
                ),
                style,
                (166, 42, 42),
            )
        }
    }
}

#[cfg(feature = "terminal-ui")]
fn render_tool_header_line(tool: &ToolBlock, style: &CliStyle) -> String {
    let label = format_tool_header(tool);
    format!(
        "{} {} {}\n",
        render_block_badge(style, "TOOL", (244, 251, 255), (47, 80, 126)),
        style.bold_cyan(style.bullet()),
        style.bold(&label)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolResultDisplay {
    pub(crate) summary: Option<String>,
    pub(crate) fields: Vec<ToolWebField>,
    pub(crate) preview: Option<ToolWebPreview>,
    pub(crate) cli_show_fields: bool,
}

#[cfg(feature = "terminal-ui")]
fn governed_preview_text(detail: &str) -> (&str, bool) {
    detail
        .split_once("\n\nPreview:\n")
        .map(|(_, preview)| (preview, true))
        .unwrap_or((detail, false))
}

#[cfg(feature = "terminal-ui")]
fn collapsed_preview_lines(text: &str, max_lines: usize, max_chars: usize) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(max_lines)
        .map(|line| truncate_display(line, max_chars))
        .collect()
}

#[cfg(feature = "terminal-ui")]
fn first_prefixed_line<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with(prefix))
}

#[cfg(feature = "terminal-ui")]
fn parse_listing_entry(line: &str) -> (Option<String>, String) {
    let entry = line.trim().trim_start_matches("- ").trim();
    if let Some(rest) = entry.strip_prefix('[') {
        if let Some((category, remainder)) = rest.split_once(']') {
            let label = remainder.trim().trim_end_matches(':').trim();
            let name = label
                .split_once(':')
                .map(|(name, _)| name.trim())
                .unwrap_or(label);
            return (Some(category.to_string()), name.to_string());
        }
    }
    let name = entry
        .split_once(':')
        .map(|(name, _)| name.trim())
        .unwrap_or(entry);
    (None, name.to_string())
}

#[cfg(feature = "terminal-ui")]
fn classify_discovery_result(detail: &str) -> Option<ToolResultDisplay> {
    let (source, governed) = governed_preview_text(detail);
    let header = source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    if !header.starts_with("Available ") {
        return None;
    }

    let entries = source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- "))
        .map(parse_listing_entry)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }

    let mut scopes = Vec::new();
    for scope in entries.iter().filter_map(|(scope, _)| scope.as_ref()) {
        if !scopes.iter().any(|existing| existing == scope) {
            scopes.push(scope.clone());
        }
    }

    let mut summary = if header.starts_with("Available skill categories") {
        format!("{} categories", entries.len())
    } else if header.starts_with("Available skills") {
        if scopes.len() == 1 {
            format!("{} skills · {}", entries.len(), scopes[0])
        } else if scopes.len() > 1 {
            format!("{} skills · {} categories", entries.len(), scopes.len())
        } else {
            format!("{} skills", entries.len())
        }
    } else {
        let noun = header
            .strip_prefix("Available ")
            .and_then(|rest| rest.split(':').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("items")
            .to_ascii_lowercase();
        format!("{} {}", entries.len(), noun)
    };
    if governed {
        summary.push_str(" · preview");
    }

    let preview = entries
        .into_iter()
        .take(4)
        .map(|(_, name)| truncate_display(&name, 72))
        .collect::<Vec<_>>();

    let mut fields = Vec::new();
    if scopes.len() == 1 && header.starts_with("Available skills") {
        fields.push(ToolWebField {
            label: "Scope".to_string(),
            value: scopes[0].clone(),
        });
    }

    Some(ToolResultDisplay {
        summary: Some(summary),
        fields,
        preview: (!preview.is_empty()).then(|| ToolWebPreview {
            kind: "text".to_string(),
            text: preview.join("\n"),
            truncated: governed,
        }),
        cli_show_fields: true,
    })
}

#[cfg(feature = "terminal-ui")]
fn classify_load_result(detail: &str) -> Option<ToolResultDisplay> {
    let first_line = detail.lines().next().map(str::trim).unwrap_or_default();
    let rest = first_line.strip_prefix("Loaded ")?;
    let (source, governed) = governed_preview_text(detail);
    let summary = if let Some((kind, payload)) = rest.split_once(": ") {
        let label = payload
            .split_once(':')
            .map(|(head, _)| head)
            .unwrap_or(payload)
            .trim();
        let label = if label.is_empty() { kind.trim() } else { label };
        format!("loaded {label}")
    } else {
        format!("loaded {}", rest.trim())
    };

    let preview = first_prefixed_line(source, "Description:")
        .map(|line| truncate_display(line, 88))
        .or_else(|| {
            let lines = source
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty()
                        && !line.starts_with("Loaded ")
                        && !line.starts_with('<')
                        && !line.starts_with("</")
                })
                .take(3)
                .map(|line| truncate_display(line, 88))
                .collect::<Vec<_>>();
            (!lines.is_empty()).then(|| lines.join("\n"))
        });

    Some(ToolResultDisplay {
        summary: Some(if governed {
            format!("{summary} · preview")
        } else {
            summary
        }),
        fields: Vec::new(),
        preview: preview.map(|text| ToolWebPreview {
            kind: "text".to_string(),
            text,
            truncated: governed,
        }),
        cli_show_fields: true,
    })
}

#[cfg(feature = "terminal-ui")]
fn parse_fetch_like_detail(detail: &str) -> (Option<String>, Option<String>, &str) {
    let Some((prefix, body)) = detail.split_once(": ") else {
        return (None, None, detail);
    };
    if !prefix.starts_with("http://") && !prefix.starts_with("https://") {
        return (None, None, detail);
    }
    if let Some((url, mime)) = prefix.rsplit_once(" (") {
        if let Some(mime) = mime.strip_suffix(')') {
            return (Some(url.to_string()), Some(mime.to_string()), body);
        }
    }
    (Some(prefix.to_string()), None, body)
}

#[cfg(feature = "terminal-ui")]
fn extract_url_host(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

#[cfg(feature = "terminal-ui")]
fn classify_fetch_result(detail: &str) -> Option<ToolResultDisplay> {
    let (url, mime, body) = parse_fetch_like_detail(detail);
    let url = url?;
    let (source, governed) = governed_preview_text(body);
    let mut summary_parts = Vec::new();
    let host = extract_url_host(&url);
    summary_parts.push(host.clone());
    if let Some(mime) = mime.as_deref() {
        summary_parts.push(mime.to_string());
    }
    if governed {
        summary_parts.push("preview".to_string());
    }
    let preview = collapsed_preview_lines(source, 4, 96).join("\n");
    let mut fields = vec![ToolWebField {
        label: "Target".to_string(),
        value: url,
    }];
    if let Some(mime) = mime {
        fields.push(ToolWebField {
            label: "Type".to_string(),
            value: mime,
        });
    }

    Some(ToolResultDisplay {
        summary: Some(summary_parts.join(" · ")),
        fields,
        preview: (!preview.is_empty()).then(|| ToolWebPreview {
            kind: "text".to_string(),
            text: preview,
            truncated: governed,
        }),
        cli_show_fields: false,
    })
}

#[cfg(feature = "terminal-ui")]
fn classify_structured_tool_result(tool: &ToolBlock) -> Option<ToolResultDisplay> {
    let structured = tool.structured.as_ref()?;
    match structured {
        ToolStructuredDetail::FileEdit {
            file_path,
            diff_preview,
        } => Some(ToolResultDisplay {
            summary: Some("updated".to_string()),
            fields: vec![ToolWebField {
                label: "File".to_string(),
                value: file_path.clone(),
            }],
            preview: diff_preview.as_ref().map(|diff| ToolWebPreview {
                kind: "diff".to_string(),
                text: diff.clone(),
                truncated: false,
            }),
            cli_show_fields: true,
        }),
        ToolStructuredDetail::FileWrite {
            file_path,
            bytes,
            lines,
            diff_preview,
        } => {
            let mut summary_parts = Vec::new();
            if let Some(lines) = lines {
                summary_parts.push(format!("{lines} lines"));
            }
            if let Some(bytes) = bytes {
                summary_parts.push(format!("{bytes} bytes"));
            }
            let summary = if summary_parts.is_empty() {
                "written".to_string()
            } else {
                format!("wrote {}", summary_parts.join(", "))
            };
            Some(ToolResultDisplay {
                summary: Some(summary),
                fields: vec![ToolWebField {
                    label: "File".to_string(),
                    value: file_path.clone(),
                }],
                preview: diff_preview.as_ref().map(|diff| ToolWebPreview {
                    kind: "diff".to_string(),
                    text: diff.clone(),
                    truncated: false,
                }),
                cli_show_fields: true,
            })
        }
        ToolStructuredDetail::FileRead {
            file_path,
            total_lines,
            truncated,
        } => {
            let mut parts = Vec::new();
            if let Some(total_lines) = total_lines {
                parts.push(format!("{total_lines} lines"));
            }
            if *truncated {
                parts.push("truncated".to_string());
            }
            let summary = if parts.is_empty() {
                "read".to_string()
            } else {
                parts.join(" · ")
            };
            Some(ToolResultDisplay {
                summary: Some(summary),
                fields: vec![ToolWebField {
                    label: "File".to_string(),
                    value: file_path.clone(),
                }],
                preview: None,
                cli_show_fields: true,
            })
        }
        ToolStructuredDetail::BashExec {
            command_preview,
            exit_code,
            output_preview,
            truncated,
        } => {
            let mut summary = match exit_code {
                Some(code) => format!("exit {code}"),
                None => "exit 0".to_string(),
            };
            if *truncated {
                summary.push_str(" · truncated");
            }
            Some(ToolResultDisplay {
                summary: Some(summary),
                fields: vec![ToolWebField {
                    label: "Command".to_string(),
                    value: command_preview.clone(),
                }],
                preview: output_preview.as_ref().map(|preview| ToolWebPreview {
                    kind: "code".to_string(),
                    text: preview.clone(),
                    truncated: *truncated,
                }),
                cli_show_fields: true,
            })
        }
        ToolStructuredDetail::Search {
            pattern,
            matches,
            truncated,
        } => {
            let mut parts = Vec::new();
            if let Some(matches) = matches {
                parts.push(format!("{matches} matches"));
            }
            if *truncated {
                parts.push("truncated".to_string());
            }
            let summary = if parts.is_empty() {
                "searched".to_string()
            } else {
                parts.join(" · ")
            };
            let mut fields = Vec::new();
            if !pattern.is_empty() {
                fields.push(ToolWebField {
                    label: "Pattern".to_string(),
                    value: pattern.clone(),
                });
            }
            Some(ToolResultDisplay {
                summary: Some(summary),
                fields,
                preview: None,
                cli_show_fields: true,
            })
        }
        ToolStructuredDetail::Generic => None,
    }
}

#[cfg(feature = "terminal-ui")]
pub(crate) fn classify_tool_result_display(tool: &ToolBlock) -> Option<ToolResultDisplay> {
    let detail = tool.detail.as_deref().map(str::trim).unwrap_or_default();
    if !detail.is_empty() {
        if let Some(display) = classify_discovery_result(detail) {
            return Some(display);
        }
        if let Some(display) = classify_load_result(detail) {
            return Some(display);
        }
        if let Some(display) = classify_fetch_result(detail) {
            return Some(display);
        }
    }
    classify_structured_tool_result(tool)
}

#[cfg(feature = "terminal-ui")]
fn render_tool_result_display_rich(display: &ToolResultDisplay, style: &CliStyle) -> String {
    let mut body_lines = if display.cli_show_fields {
        display
            .fields
            .iter()
            .map(|field| {
                format!(
                    "{}: {}",
                    style.bold(&field.label),
                    truncate_display(&field.value, 56)
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if let Some(preview) = display.preview.as_ref() {
        let rendered = match preview.kind.as_str() {
            "diff" => render_diff_preview(&preview.text, style),
            "code" => {
                let collapsed = style.collapse_with_width(&preview.text, 5, 2, None);
                if preview.truncated {
                    format!("{collapsed}\n{}", style.dim("… truncated"))
                } else {
                    style.dim(&collapsed)
                }
            }
            _ => collapsed_preview_lines(&preview.text, 4, 96).join("\n"),
        };
        if !rendered.trim().is_empty() {
            body_lines.push(rendered);
        }
    }

    render_tool_detail_block(
        display.summary.as_ref().map(|summary| style.dim(summary)),
        (!body_lines.is_empty()).then(|| body_lines.join("\n")),
        style,
    )
}

#[cfg(feature = "terminal-ui")]
fn render_session_event_rich(event: &SessionEventBlock, style: &CliStyle) -> String {
    let tone = event.status.as_deref().unwrap_or("");
    let heading = match tone {
        "completed" | "done" | "success" => style.green(&event.title),
        "error" | "failed" => style.red(&event.title),
        "running" | "in_progress" => style.yellow(&event.title),
        _ => style.bold(&event.title),
    };
    let mut out = format!(
        "{} {} {} {}\n",
        render_block_badge(style, "EVENT", (244, 251, 255), (47, 80, 126)),
        style.bold_cyan(style.tree_end()),
        heading,
        style.dim(&format!("[{}]", event.event))
    );
    if let Some(summary) = event.summary.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(&format!("  {}\n", style.dim(summary)));
    }
    for field in &event.fields {
        out.push_str(&format!(
            "  {}: {}\n",
            style.bold(&field.label),
            field.value
        ));
    }
    if let Some(body) = event.body.as_deref().filter(|value| !value.is_empty()) {
        for line in body.lines() {
            out.push_str(&format!("  {}\n", line));
        }
    }
    append_block_divider(out, style, (47, 80, 126))
}

fn render_queue_item_rich(item: &QueueItemBlock, style: &CliStyle) -> String {
    append_block_divider(
        format!(
            "{} {} {}\n",
            render_block_badge(style, "QUEUE", (244, 247, 250), (80, 96, 112)),
            style.dim(style.bullet()),
            style.dim(&format!("Queued [{}] {}", item.position, item.text))
        ),
        style,
        (80, 96, 112),
    )
}

/// Rich rendering of completed tool results.
fn render_tool_done_rich(tool: &ToolBlock, style: &CliStyle) -> String {
    if let Some(display) = classify_tool_result_display(tool) {
        let mut block = render_tool_header_line(tool, style);
        block.push_str(&render_tool_result_display_rich(&display, style));
        return append_block_divider(block, style, (47, 80, 126));
    }

    // Fallback: no structured data
    let detail = tool.detail.as_deref().unwrap_or("");
    if detail.is_empty() {
        let mut block = render_tool_header_line(tool, style);
        block.push_str(&render_tool_detail_block(
            Some(style.green("Done")),
            None,
            style,
        ));
        append_block_divider(block, style, (47, 80, 126))
    } else {
        let collapsed = style.collapse_with_width(detail, 5, 2, None);
        let mut block = render_tool_header_line(tool, style);
        block.push_str(&render_tool_detail_block(Some(collapsed), None, style));
        append_block_divider(block, style, (47, 80, 126))
    }
}

fn render_tool_detail_block(
    summary: Option<String>,
    body: Option<String>,
    style: &CliStyle,
) -> String {
    let mut out = String::new();
    if let Some(summary) = summary.filter(|value| !value.trim().is_empty()) {
        out.push_str(&format!("  {} {}\n", style.tree_end(), summary));
    }
    if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
        for line in body.lines() {
            out.push_str(&format!("    {}\n", line));
        }
    }
    out
}

fn append_block_divider(mut block: String, style: &CliStyle, divider: (u8, u8, u8)) -> String {
    if !block.ends_with('\n') {
        block.push('\n');
    }
    block.push_str(&render_block_divider(style, divider));
    block.push('\n');
    block
}

fn render_block_divider(style: &CliStyle, rgb: (u8, u8, u8)) -> String {
    let line = format!("  {}", "─".repeat(28));
    if style.color {
        style.rgb(&line, rgb.0, rgb.1, rgb.2)
    } else {
        line
    }
}

fn render_block_badge(style: &CliStyle, label: &str, fg: (u8, u8, u8), bg: (u8, u8, u8)) -> String {
    if style.color {
        format!(
            "\x1b[1;38;2;{};{};{};48;2;{};{};{}m {} \x1b[0m",
            fg.0, fg.1, fg.2, bg.0, bg.1, bg.2, label
        )
    } else {
        format!("[{}]", label)
    }
}

/// Render a unified diff preview with ± color.
fn render_diff_preview(diff: &str, style: &CliStyle) -> String {
    let lines: Vec<&str> = diff.lines().collect();
    let mut out = Vec::new();
    let total = lines.len();
    let max_lines = 12;

    let visible: Vec<&str> = if total > max_lines {
        let mut v: Vec<&str> = lines[..max_lines].to_vec();
        v.push(""); // placeholder for summary
        v
    } else {
        lines.clone()
    };

    for (i, line) in visible.iter().enumerate() {
        if total > max_lines && i == max_lines {
            out.push(style.dim(&format!("… +{} lines", total - max_lines)));
            break;
        }
        let rendered = if line.starts_with('+') && !line.starts_with("+++") {
            style.green(line)
        } else if line.starts_with('-') && !line.starts_with("---") {
            style.red(line)
        } else if line.starts_with("@@") {
            style.cyan(line)
        } else {
            style.dim(line)
        };
        out.push(rendered);
    }
    out.join("\n")
}

/// Format tool header with arguments, e.g. `Edit(src/main.rs)` or `Bash(ls -la)`.
pub(crate) fn format_tool_header(tool: &ToolBlock) -> String {
    let display = tool_cli_activity_label(tool);

    // Try to extract a meaningful argument from the detail/structured
    let arg = if let Some(ref structured) = tool.structured {
        match structured {
            ToolStructuredDetail::FileEdit { file_path, .. }
            | ToolStructuredDetail::FileWrite { file_path, .. }
            | ToolStructuredDetail::FileRead { file_path, .. } => Some(file_path.clone()),
            ToolStructuredDetail::BashExec {
                command_preview, ..
            } => {
                let truncated: String = command_preview.chars().take(60).collect();
                if truncated.len() < command_preview.len() {
                    Some(format!("{}…", truncated))
                } else {
                    Some(truncated)
                }
            }
            ToolStructuredDetail::Search { pattern, .. } => Some(pattern.clone()),
            ToolStructuredDetail::Generic => None,
        }
    } else {
        None
    };

    match arg {
        Some(a) => format!("{}({})", display, a),
        None => display,
    }
}

fn is_skill_tool_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized == "skill"
        || normalized == "skillslist"
        || normalized == "skillview"
        || normalized == "skillscategories"
        || normalized.starts_with("skill")
}

pub fn tool_cli_activity_label(tool: &ToolBlock) -> String {
    let display = tool_display_name(&tool.name);
    if is_skill_tool_name(&tool.name) {
        if display == "Skill" {
            "Skill".to_string()
        } else {
            format!("Skill {}", display)
        }
    } else {
        display
    }
}

/// Convert internal tool ID to a human-readable display name.
fn tool_display_name(tool_id: &str) -> String {
    match tool_id {
        "read" => "Read".to_string(),
        "write" => "Write".to_string(),
        "edit" => "Edit".to_string(),
        "multiedit" => "MultiEdit".to_string(),
        "bash" => "Bash".to_string(),
        "glob" => "Glob".to_string(),
        "grep" => "Grep".to_string(),
        "ls" => "Ls".to_string(),
        "websearch" => "WebSearch".to_string(),
        "webfetch" => "WebFetch".to_string(),
        "question" => "Question".to_string(),
        "todo_read" => "TodoRead".to_string(),
        "todo_write" => "TodoWrite".to_string(),
        "apply_patch" => "ApplyPatch".to_string(),
        "skill" => "Skill".to_string(),
        "lsp" => "LSP".to_string(),
        "batch" => "Batch".to_string(),
        "codesearch" => "CodeSearch".to_string(),
        "context_docs" => "ContextDocs".to_string(),
        "github_research" => "GitHubResearch".to_string(),
        "repo_history" => "RepoHistory".to_string(),
        "media_inspect" => "MediaInspect".to_string(),
        "browser_session" => "BrowserSession".to_string(),
        "shell_session" => "ShellSession".to_string(),
        "ast_grep_search" => "AstGrepSearch".to_string(),
        "ast_grep_replace" => "AstGrepReplace".to_string(),
        other => {
            // CamelCase conversion for unknown tools
            let mut result = String::new();
            for (i, ch) in other.chars().enumerate() {
                if ch == '_' || ch == '-' {
                    continue;
                }
                if i == 0
                    || other.as_bytes().get(i.wrapping_sub(1)) == Some(&b'_')
                    || other.as_bytes().get(i.wrapping_sub(1)) == Some(&b'-')
                {
                    result.push(ch.to_uppercase().next().unwrap_or(ch));
                } else {
                    result.push(ch);
                }
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_status_blocks() {
        let line = render_cli_block(&OutputBlock::Status(StatusBlock::success("ready")));
        assert_eq!(line, "[status+] ready\n");
    }

    #[test]
    fn renders_message_blocks() {
        let start = render_cli_block(&OutputBlock::Message(MessageBlock::start(
            MessageRole::Assistant,
        )));
        let delta = render_cli_block(&OutputBlock::Message(MessageBlock::delta(
            MessageRole::Assistant,
            "hello",
        )));
        let end = render_cli_block(&OutputBlock::Message(MessageBlock::end(
            MessageRole::Assistant,
        )));
        assert_eq!(start, "[message:assistant] ");
        assert_eq!(delta, "hello");
        assert_eq!(end, "\n");
    }

    #[test]
    fn renders_tool_blocks() {
        let line = render_cli_block(&OutputBlock::Tool(ToolBlock::error("bash", "exit=1")));
        assert_eq!(line, "[tool:error] bash :: exit=1\n");
    }

    #[test]
    fn renders_session_event_blocks() {
        let line = render_cli_block(&OutputBlock::SessionEvent(SessionEventBlock {
            event: "scheduler_node".to_string(),
            title: "Scheduler node: inspect".to_string(),
            status: Some("pending".to_string()),
            summary: Some("Node `node_1` is `pending`.".to_string()),
            fields: vec![SessionEventField {
                label: "ID".to_string(),
                value: "node_1".to_string(),
                tone: None,
            }],
            body: None,
        }));
        assert!(line.contains("[session_event] Scheduler node: inspect [scheduler_node · pending]"));
        assert!(line.contains("summary: Node `node_1` is `pending`."));
    }

    #[test]
    fn renders_queue_item_blocks() {
        let line = render_cli_block(&OutputBlock::QueueItem(QueueItemBlock {
            position: 2,
            text: "run verification".to_string(),
        }));
        assert_eq!(line, "[queue_item] [2] run verification\n");
    }

    // ── Rich rendering tests ────────────────────────────────────

    #[test]
    fn rich_status_title_has_bullet() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Status(StatusBlock::title("Hello")), &style);
        assert!(out.contains("●"));
        assert!(out.contains("Hello"));
        assert!(out.contains("48;2;"));
    }

    #[test]
    fn rich_status_success_has_check() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Status(StatusBlock::success("Done")), &style);
        assert!(out.contains("✔"));
        assert!(out.contains("Done"));
    }

    #[test]
    fn rich_status_error_has_cross() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Status(StatusBlock::error("fail")), &style);
        assert!(out.contains("✗"));
        assert!(out.contains("fail"));
    }

    #[test]
    fn rich_tool_start_capitalized() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Tool(ToolBlock::start("edit")), &style);
        assert!(out.contains("Edit"));
        assert!(out.contains("●"));
        assert!(out.contains("48;2;"));
        assert!(!out.starts_with('\n'));
    }

    #[test]
    fn skill_tool_names_render_with_skill_prefix() {
        let style = CliStyle {
            color: false,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Tool(ToolBlock::start("SkillsList")), &style);
        assert!(out.contains("Skill SkillsList"), "{out}");
    }

    #[test]
    fn plain_skill_tool_block_uses_skill_aware_label() {
        let line = render_cli_block(&OutputBlock::Tool(ToolBlock::done(
            "SkillsList",
            Some("{\"category\":\"literature-research/skills\"}".to_string()),
        )));
        assert_eq!(
            line,
            "[tool:done] Skill SkillsList :: {\"category\":\"literature-research/skills\"}\n"
        );
    }

    #[test]
    fn rich_tool_error_red() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock::error("bash", "exit code 1")),
            &style,
        );
        assert!(out.contains("⎿"));
        assert!(out.contains("Error:"));
    }

    #[test]
    fn rich_message_start_has_bullet() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Message(MessageBlock::start(MessageRole::Assistant)),
            &style,
        );
        assert!(out.contains("●"));
        assert!(!out.starts_with('\n'));
    }

    #[test]
    fn rich_reasoning_start_has_no_leading_blank_line() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Reasoning(ReasoningBlock::start()), &style);
        assert!(!out.starts_with('\n'));
        assert!(out.contains("●"));
        assert!(out.contains("THINKING"));
        assert!(!out.contains("● Thinking"));
    }

    #[test]
    fn rich_reasoning_end_leaves_a_block_boundary() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Reasoning(ReasoningBlock::end()), &style);
        assert!(out.starts_with('\n'), "{out:?}");
        assert!(out.contains('─'), "{out:?}");
    }

    #[test]
    fn finalized_rich_blocks_end_with_divider() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let cases = vec![
            render_cli_block_rich(&OutputBlock::Status(StatusBlock::success("ok")), &style),
            render_cli_block_rich(
                &OutputBlock::Message(MessageBlock::full(MessageRole::Assistant, "hello")),
                &style,
            ),
            render_cli_block_rich(
                &OutputBlock::Reasoning(ReasoningBlock::full("thinking".to_string())),
                &style,
            ),
            render_cli_block_rich(
                &OutputBlock::Tool(ToolBlock::done("webfetch", Some("done".to_string()))),
                &style,
            ),
            render_cli_block_rich(
                &OutputBlock::SessionEvent(SessionEventBlock {
                    title: "Permission".to_string(),
                    event: "permission.requested".to_string(),
                    status: Some("running".to_string()),
                    summary: None,
                    fields: Vec::new(),
                    body: None,
                }),
                &style,
            ),
            render_cli_block_rich(
                &OutputBlock::QueueItem(QueueItemBlock {
                    position: 1,
                    text: "queued".to_string(),
                }),
                &style,
            ),
            render_cli_block_rich(
                &OutputBlock::Inspect(InspectBlock {
                    filter_stage_id: None,
                    stage_ids: vec!["stage-1".to_string()],
                    events: Vec::new(),
                }),
                &style,
            ),
        ];

        for rendered in cases {
            assert!(rendered.contains('─'), "{rendered}");
            assert!(rendered.ends_with('\n'), "{rendered}");
        }
    }

    #[test]
    fn rich_reasoning_full_uses_semantic_header_and_indented_body() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Reasoning(ReasoningBlock::full("line one\nline two".to_string())),
            &style,
        );

        assert!(out.contains("●"));
        assert!(out.contains("THINKING"));
        assert!(!out.contains("● Thinking"));
        assert!(out.contains("line one"));
        assert!(out.contains("  line two"));
    }

    #[test]
    fn rich_tool_done_groups_summary_and_preview_without_blank_gaps() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock {
                name: "write".to_string(),
                phase: ToolPhase::Done,
                detail: None,
                structured: Some(ToolStructuredDetail::FileWrite {
                    file_path: "src/main.rs".to_string(),
                    bytes: Some(42),
                    lines: Some(3),
                    diff_preview: Some("@@ -1 +1 @@\n-old\n+new".to_string()),
                }),
            }),
            &style,
        );
        assert!(out.contains("wrote 3 lines, 42 bytes"));
        assert!(out.contains("@@ -1 +1 @@"));
        assert!(!out.contains("\n\n"));
    }

    #[test]
    fn rich_skills_list_done_is_summarized_for_cli() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock::done(
                "SkillsList",
                Some(
                    "Available skills: <available_skills>\n- [literature-research/skills] author-network: Analyze an author's publication history\n- [literature-research/skills] semantic-scholar: Search Semantic Scholar\n- [literature-research/skills] evidence-synthesis: Synthesize evidence\n".to_string(),
                ),
            )),
            &style,
        );
        assert!(
            out.contains("3 skills · literature-research/skills"),
            "{out}"
        );
        assert!(out.contains("author-network"), "{out}");
        assert!(out.contains("semantic-scholar"), "{out}");
        assert!(!out.contains("<available_skills>"), "{out}");
    }

    #[test]
    fn rich_discovery_result_is_summarized_by_shape_not_tool_name() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock::done(
                "CatalogLookup",
                Some(
                    "Available datasets:\n- alpha: first entry\n- beta: second entry\n- gamma: third entry\n"
                        .to_string(),
                ),
            )),
            &style,
        );
        assert!(out.contains("3 datasets"), "{out}");
        assert!(out.contains("alpha"), "{out}");
        assert!(out.contains("beta"), "{out}");
    }

    #[test]
    fn rich_skill_view_done_prefers_loaded_summary_and_description_preview() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock::done(
                "SkillView",
                Some(
                    "Loaded skill: semantic-scholar: <skill_runtime_packet name=\"semantic-scholar\">\n\n# Skill: semantic-scholar\n\nDescription: Search Semantic Scholar for papers and citations.\n\n</skill_runtime_packet>\n".to_string(),
                ),
            )),
            &style,
        );
        assert!(out.contains("loaded semantic-scholar"), "{out}");
        assert!(
            out.contains("Description: Search Semantic Scholar"),
            "{out}"
        );
        assert!(!out.contains("<skill_runtime_packet"), "{out}");
    }

    #[test]
    fn rich_load_result_is_summarized_by_shape_not_tool_name() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock::done(
                "ArtifactLoader",
                Some(
                    "Loaded document: references/pubmed_search.md: <document>\n\nDescription: PubMed search reference.\n\n</document>\n"
                        .to_string(),
                ),
            )),
            &style,
        );
        assert!(out.contains("loaded references/pubmed_search.md"), "{out}");
        assert!(
            out.contains("Description: PubMed search reference."),
            "{out}"
        );
        assert!(!out.contains("<document>"), "{out}");
    }

    #[test]
    fn rich_webfetch_done_is_summarized_for_cli() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Tool(ToolBlock::done(
                "webfetch",
                Some(
                    "https://api.semanticscholar.org/graph/v1/paper/search?query=Xu (application/json): {\"total\":2,\"data\":[{\"title\":\"Paper A\"},{\"title\":\"Paper B\"}]}"
                        .to_string(),
                ),
            )),
            &style,
        );
        assert!(
            out.contains("api.semanticscholar.org · application/json"),
            "{out}"
        );
        assert!(out.contains("{\"total\":2"), "{out}");
        assert!(
            !out.contains("https://api.semanticscholar.org/graph/v1/paper/search"),
            "{out}"
        );
    }

    #[test]
    fn fetch_like_result_is_summarized_for_web_by_shape_not_tool_name() {
        let tool = ToolBlock::done(
            "HttpProbe",
            Some(
                "https://api.semanticscholar.org/graph/v1/paper/search?query=Xu (application/json): {\"total\":2,\"data\":[{\"title\":\"Paper A\"}]}"
                    .to_string(),
            ),
        );
        let display = classify_tool_result_display(&tool).expect("display");
        let summary = display.summary.expect("summary");
        let fields = display.fields;
        let preview = display.preview.expect("preview");

        assert!(
            summary.contains("api.semanticscholar.org · application/json"),
            "{summary}"
        );
        assert!(
            fields.iter().any(|field| field.label == "Target"),
            "{fields:?}"
        );
        assert!(
            fields.iter().any(|field| field.label == "Type"),
            "{fields:?}"
        );
        assert!(preview.text.contains("{\"total\":2"), "{preview:?}");
    }

    #[test]
    fn rich_session_event_has_no_leading_blank_line() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::SessionEvent(SessionEventBlock {
                title: "Web Search".to_string(),
                event: "websearch".to_string(),
                status: Some("completed".to_string()),
                summary: Some("query finished".to_string()),
                fields: vec![SessionEventField {
                    label: "query".to_string(),
                    value: "青岛小麦岛天气".to_string(),
                    tone: None,
                }],
                body: None,
            }),
            &style,
        );
        assert!(!out.starts_with('\n'));
        assert!(out.contains("Web Search"));
        assert!(out.contains("⎿"));
    }

    #[test]
    fn rich_full_message_indents_continuation_lines() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "line one\nline two",
            )),
            &style,
        );
        assert!(out.contains("line one"));
        assert!(out.contains("\n  line two"));
        assert!(!out.starts_with('\n'));
    }

    #[test]
    fn rich_prompt_assistant_done_share_left_baseline() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let prompt = render_cli_block_rich(
            &OutputBlock::Message(MessageBlock::full(MessageRole::User, "hi")),
            &style,
        );
        let assistant = render_cli_block_rich(
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "Hi! How can I help you today?",
            )),
            &style,
        );
        let done = render_cli_block_rich(
            &OutputBlock::Status(StatusBlock::success("Done. tokens: prompt=1 completion=2")),
            &style,
        );

        assert!(!prompt.starts_with('\n'));
        assert!(!assistant.starts_with('\n'));
        assert!(!done.starts_with('\n'));
        assert!(prompt.contains("hi"));
    }

    #[test]
    fn rich_fallback_to_plain_when_no_color() {
        let style = CliStyle::plain();
        let out = render_cli_block_rich(&OutputBlock::Status(StatusBlock::success("ok")), &style);
        assert_eq!(out, "[status+] ok\n");
    }

    #[test]
    fn rich_queue_item_renders_muted_summary() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::QueueItem(QueueItemBlock {
                position: 3,
                text: "follow up with more checks".to_string(),
            }),
            &style,
        );
        assert!(out.contains("Queued [3] follow up with more checks"));
    }

    #[test]
    fn tool_display_name_maps_known_tools() {
        assert_eq!(tool_display_name("bash"), "Bash");
        assert_eq!(tool_display_name("ast_grep_search"), "AstGrepSearch");
        assert_eq!(tool_display_name("websearch"), "WebSearch");
    }

    #[test]
    fn tool_display_name_converts_unknown() {
        assert_eq!(tool_display_name("my_custom_tool"), "MyCustomTool");
        assert_eq!(tool_display_name("something"), "Something");
    }
}
