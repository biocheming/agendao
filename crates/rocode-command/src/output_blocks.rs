use crate::cli_markdown;
use crate::cli_panel::truncate_display;
use crate::cli_style::CliStyle;
pub use rocode_content::output_blocks::*;

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
        OutputBlock::SchedulerStage(stage) => render_scheduler_stage_block(stage),
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
    match &tool.detail {
        Some(detail) if !detail.trim().is_empty() => {
            format!("[tool:{phase}] {} :: {}\n", tool.name, detail)
        }
        _ => format!("[tool:{phase}] {}\n", tool.name),
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

fn render_scheduler_stage_block(stage: &SchedulerStageBlock) -> String {
    let mut out = String::new();
    let header = scheduler_stage_header(stage);
    out.push_str(&format!("[scheduler_stage] {header}\n"));
    if stage
        .decision
        .as_ref()
        .map(|decision| decision.spec.show_header_divider)
        .unwrap_or(true)
    {
        out.push_str(&format!("{}\n", "─".repeat(40)));
    }

    let mut summary = Vec::new();
    if let Some(step) = stage.step {
        summary.push(format!("step={step}"));
    }
    if let Some(status) = stage.status.as_deref() {
        summary.push(format!("status={}", scheduler_status_label(status)));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        summary.push(format!("waiting_on={waiting_on}"));
    }
    summary.push(format!("tokens={}", scheduler_stage_token_summary(stage)));
    if !summary.is_empty() {
        out.push_str(&format!("  {}\n", summary.join(" · ")));
    }
    if let Some(detail) = scheduler_stage_secondary_token_summary(stage) {
        out.push_str(&format!("  usage: {detail}\n"));
    }
    if let Some(detail) = scheduler_stage_skill_tree_summary(stage) {
        out.push_str(&format!("  skill tree: {detail}\n"));
    }
    if let Some(ref attached_id) = stage.attached_session_id {
        out.push_str(&format!("  attached session: {attached_id}\n"));
    }
    if let Some(focus) = stage.focus.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(&format!("  focus: {focus}\n"));
    }
    if let Some(last_event) = stage
        .last_event
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(&format!("  last: {last_event}\n"));
    }
    if let Some(activity) = stage.activity.as_deref().filter(|value| !value.is_empty()) {
        out.push_str("  activity:\n");
        for line in activity.lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    let mut available = Vec::new();
    if let Some(count) = stage.available_skill_count {
        available.push(format!("skills {count}"));
    }
    if let Some(count) = stage.available_agent_count {
        available.push(format!("agents {count}"));
    }
    if let Some(count) = stage.available_category_count {
        available.push(format!("categories {count}"));
    }
    if !available.is_empty() {
        out.push_str(&format!("  available: {}\n", available.join(" · ")));
    }
    if !stage.active_skills.is_empty() {
        out.push_str(&format!(
            "  active skills: {}\n",
            stage.active_skills.join(", ")
        ));
    }
    if !stage.active_agents.is_empty() {
        out.push_str(&format!(
            "  active agents: {}\n",
            stage.active_agents.join(", ")
        ));
    }
    if !stage.active_categories.is_empty() {
        out.push_str(&format!(
            "  active categories: {}\n",
            stage.active_categories.join(", ")
        ));
    }
    if let Some(decision) = stage.decision.as_ref() {
        out.push_str(&format!("  ◈ {}\n", decision.title));
        for field in &decision.fields {
            out.push_str(&format!(
                "  • {}: {}\n",
                field.label,
                decision_field_display_value(field)
            ));
        }
        for section in &decision.sections {
            if decision.spec.section_spacing == "loose" {
                out.push('\n');
            }
            out.push_str(&format!("  ✦ {}\n", section.title));
            for line in section.body.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }
    }
    let body = stage.text.trim();
    if !body.is_empty() && stage.decision.is_none() {
        let body = body.to_string();
        out.push_str(&body);
        out.push('\n');
    }
    out
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
        OutputBlock::SchedulerStage(stage) => render_scheduler_stage_rich(stage, style),
        OutputBlock::Inspect(inspect) => render_inspect_block(inspect),
    }
}

fn render_status_rich(status: &StatusBlock, style: &CliStyle) -> String {
    match status.tone {
        BlockTone::Title => {
            format!(
                "{} {}\n",
                style.bold_cyan(style.bullet()),
                style.bold(&status.text)
            )
        }
        BlockTone::Normal => {
            format!(
                "{} {}\n",
                style.dim(style.bullet()),
                style.dim(&status.text)
            )
        }
        BlockTone::Muted => {
            format!("  {}\n", style.dim(&status.text))
        }
        BlockTone::Success => {
            format!(
                "{} {}\n",
                style.bold_green(style.check()),
                style.green(&status.text)
            )
        }
        BlockTone::Warning => {
            format!(
                "{} {}\n",
                style.bold_yellow(style.warning_icon()),
                style.yellow(&status.text)
            )
        }
        BlockTone::Error => {
            format!(
                "{} {}\n",
                style.bold_red(style.cross()),
                style.red(&status.text)
            )
        }
    }
}

fn render_message_rich(message: &MessageBlock, style: &CliStyle) -> String {
    match message.phase {
        MessagePhase::Start => {
            let bullet = match message.role {
                MessageRole::User => style.bold_green(style.bullet()),
                MessageRole::Assistant => style.bold_cyan(style.bullet()),
                MessageRole::System => style.bold_yellow(style.bullet()),
            };
            format!("{} ", bullet)
        }
        MessagePhase::Delta => message.text.clone(),
        MessagePhase::End => "\n".to_string(),
        MessagePhase::Full => {
            let rendered = match message.role {
                MessageRole::User => message.text.clone(),
                MessageRole::Assistant | MessageRole::System => {
                    cli_markdown::render_markdown(&message.text, style)
                }
            };
            let indent = match message.role {
                MessageRole::User => "  ",
                MessageRole::Assistant => "  ",
                MessageRole::System => "  ",
            };
            let bullet = match message.role {
                MessageRole::User => style.bold_green(style.bullet()),
                MessageRole::Assistant => style.bold_cyan(style.bullet()),
                MessageRole::System => style.bold_yellow(style.bullet()),
            };
            let indented = indent_continuation_lines(rendered.trim_end(), indent);
            format!("{} {}\n", bullet, indented)
        }
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

fn render_reasoning_rich(reasoning: &ReasoningBlock, style: &CliStyle) -> String {
    match reasoning.phase {
        MessagePhase::Start => format!(
            "  {} {}\n  {} ",
            style.dim("╭"),
            style.dim("thinking"),
            style.dim("│")
        ),
        MessagePhase::Delta => {
            let cleaned = strip_think_tags(&reasoning.text);
            if cleaned.is_empty() {
                String::new()
            } else {
                let indented = indent_continuation_lines(&cleaned, "│ ");
                style.dim(&indented)
            }
        }
        MessagePhase::End => "\n".to_string(),
        MessagePhase::Full => {
            let cleaned = strip_think_tags(&reasoning.text).trim().to_string();
            if cleaned.is_empty() {
                String::new()
            } else {
                let indented = indent_continuation_lines(&cleaned, "│ ");
                format!(
                    "  {} {}\n  {}\n",
                    style.dim("╭"),
                    style.dim("thinking"),
                    style.dim(&format!("│ {}", indented))
                )
            }
        }
    }
}

fn render_tool_rich(tool: &ToolBlock, style: &CliStyle) -> String {
    match tool.phase {
        ToolPhase::Start => {
            let label = format_tool_header(tool);
            format!(
                "{} {}\n",
                style.bold_cyan(style.bullet()),
                style.bold(&label)
            )
        }
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
            format!(
                "  {} {}\n",
                style.tree_end(),
                style.red(&format!("Error: {}", collapsed))
            )
        }
    }
}

fn render_session_event_rich(event: &SessionEventBlock, style: &CliStyle) -> String {
    let tone = event.status.as_deref().unwrap_or("");
    let heading = match tone {
        "completed" | "done" | "success" => style.green(&event.title),
        "error" | "failed" => style.red(&event.title),
        "running" | "in_progress" => style.yellow(&event.title),
        _ => style.bold(&event.title),
    };
    let mut out = format!(
        "{} {} {}\n",
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
    out
}

fn render_queue_item_rich(item: &QueueItemBlock, style: &CliStyle) -> String {
    format!(
        "{} {}\n",
        style.dim(style.bullet()),
        style.dim(&format!("Queued [{}] {}", item.position, item.text))
    )
}

pub(crate) fn tool_web_header(tool: &ToolBlock) -> String {
    format_tool_header(tool)
}

pub(crate) fn tool_web_summary(tool: &ToolBlock) -> Option<String> {
    match tool.phase {
        ToolPhase::Start => tool
            .detail
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned(),
        ToolPhase::Running => tool
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
        ToolPhase::Done => {
            if let Some(ref structured) = tool.structured {
                match structured {
                    ToolStructuredDetail::FileEdit { .. } => Some(
                        tool.detail
                            .as_ref()
                            .filter(|value| !value.trim().is_empty())
                            .cloned()
                            .unwrap_or_else(|| "edited".to_string()),
                    ),
                    ToolStructuredDetail::FileWrite { bytes, lines, .. } => {
                        let mut parts = Vec::new();
                        if let Some(lines) = lines {
                            parts.push(format!("{lines} lines"));
                        }
                        if let Some(bytes) = bytes {
                            parts.push(format!("{bytes} bytes"));
                        }
                        Some(if parts.is_empty() {
                            "written".to_string()
                        } else {
                            format!("wrote {}", parts.join(", "))
                        })
                    }
                    ToolStructuredDetail::FileRead {
                        total_lines,
                        truncated,
                        ..
                    } => {
                        let mut parts = Vec::new();
                        if let Some(total_lines) = total_lines {
                            parts.push(format!("{total_lines} lines"));
                        }
                        if *truncated {
                            parts.push("truncated".to_string());
                        }
                        Some(if parts.is_empty() {
                            "read".to_string()
                        } else {
                            parts.join(", ")
                        })
                    }
                    ToolStructuredDetail::BashExec {
                        exit_code,
                        truncated,
                        ..
                    } => {
                        let mut summary = match exit_code {
                            Some(code) => format!("exit {code}"),
                            None => "exit 0".to_string(),
                        };
                        if *truncated {
                            summary.push_str(" · truncated");
                        }
                        Some(summary)
                    }
                    ToolStructuredDetail::Search {
                        matches, truncated, ..
                    } => {
                        let mut parts = Vec::new();
                        if let Some(matches) = matches {
                            parts.push(format!("{matches} matches"));
                        }
                        if *truncated {
                            parts.push("truncated".to_string());
                        }
                        Some(if parts.is_empty() {
                            "searched".to_string()
                        } else {
                            parts.join(", ")
                        })
                    }
                    ToolStructuredDetail::Generic => tool
                        .detail
                        .as_ref()
                        .filter(|value| !value.trim().is_empty())
                        .cloned()
                        .or_else(|| Some("Done".to_string())),
                }
            } else {
                tool.detail
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
                    .or_else(|| Some("Done".to_string()))
            }
        }
    }
}

pub(crate) fn tool_web_fields(tool: &ToolBlock) -> Vec<ToolWebField> {
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

pub(crate) fn tool_web_preview(tool: &ToolBlock) -> Option<ToolWebPreview> {
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

fn render_scheduler_stage_rich(stage: &SchedulerStageBlock, style: &CliStyle) -> String {
    let header = scheduler_stage_header(stage);
    let header_rendered = match stage.status.as_deref().unwrap_or_default() {
        "done" => style.bold_green(&header),
        "blocked" => style.bold_red(&header),
        "cancelled" => style.bold_red(&header),
        "waiting" => style.bold_yellow(&header),
        "cancelling" => style.bold_yellow(&header),
        _ => style.bold_cyan(&header),
    };
    let mut out = String::new();
    out.push('\n');
    let bullet = match stage.status.as_deref().unwrap_or_default() {
        "done" => style.bold_green(style.bullet()),
        "blocked" => style.bold_red(style.bullet()),
        "cancelled" => style.bold_red(style.bullet()),
        "waiting" => style.bold_yellow(style.bullet()),
        "cancelling" => style.bold_yellow(style.bullet()),
        _ => style.bold_cyan(style.bullet()),
    };
    out.push_str(&format!("{} {}\n", bullet, header_rendered));
    if stage
        .decision
        .as_ref()
        .map(|decision| decision.spec.show_header_divider)
        .unwrap_or(true)
    {
        let divider_width = stage_card_content_width(style).min(72);
        out.push_str(&format!(
            "  {}\n",
            style.markdown_hr(&"─".repeat(divider_width))
        ));
    }

    let mut summary = Vec::new();
    if let Some(step) = stage.step {
        summary.push(format!("step {}", step));
    }
    if let Some(status) = stage.status.as_deref().filter(|value| !value.is_empty()) {
        summary.push(scheduler_status_label(status).to_string());
    }
    if let Some(waiting_on) = stage
        .waiting_on
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        summary.push(format!("waiting on {}", waiting_on));
    }
    summary.push(format!("tokens {}", scheduler_stage_token_summary(stage)));
    if !summary.is_empty() {
        let summary_text = summary.join(" · ");
        out.push_str(&stage_tree_line(style, &summary_text, |text| {
            match stage.status.as_deref().unwrap_or_default() {
                "done" => style.green(text),
                "blocked" => style.red(text),
                "cancelled" => style.red(text),
                "waiting" => style.yellow(text),
                "cancelling" => style.yellow(text),
                _ => style.cyan(text),
            }
        }));
    }
    if let Some(detail) = scheduler_stage_secondary_token_summary(stage) {
        out.push_str(&stage_tree_field(style, "Usage", &detail, |text| {
            style.dim(text)
        }));
    }
    if let Some(detail) = scheduler_stage_skill_tree_summary(stage) {
        out.push_str(&stage_tree_field(style, "Skill Tree", &detail, |text| {
            style.dim(text)
        }));
    }
    if let Some(ref attached_id) = stage.attached_session_id {
        out.push_str(&stage_tree_field(
            style,
            "Attached Session",
            attached_id,
            |text| style.cyan(text),
        ));
    }
    if let Some(focus) = stage.focus.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(&stage_tree_field(style, "Focus", focus, |text| {
            style.dim(text)
        }));
    }
    if let Some(last_event) = stage
        .last_event
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(&stage_tree_field(style, "Last", last_event, |text| {
            style.dim(text)
        }));
    }
    if let Some(activity) = stage.activity.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(&stage_tree_line(style, "Activity:", |text| style.dim(text)));
        for line in activity.lines() {
            out.push_str(&stage_tree_line(style, line, |text| style.dim(text)));
        }
    }
    let mut available = Vec::new();
    if let Some(count) = stage.available_skill_count {
        available.push(format!("skills {count}"));
    }
    if let Some(count) = stage.available_agent_count {
        available.push(format!("agents {count}"));
    }
    if let Some(count) = stage.available_category_count {
        available.push(format!("categories {count}"));
    }
    if !available.is_empty() {
        out.push_str(&stage_tree_field(
            style,
            "Available",
            &available.join(" · "),
            |text| style.dim(text),
        ));
    }
    if !stage.active_skills.is_empty() {
        out.push_str(&stage_tree_field(
            style,
            "Active Skills",
            &stage.active_skills.join(", "),
            |text| style.dim(text),
        ));
    }
    if !stage.active_agents.is_empty() {
        out.push_str(&stage_tree_field(
            style,
            "Active Agents",
            &stage.active_agents.join(", "),
            |text| style.dim(text),
        ));
    }
    if !stage.active_categories.is_empty() {
        out.push_str(&stage_tree_field(
            style,
            "Active Categories",
            &stage.active_categories.join(", "),
            |text| style.dim(text),
        ));
    }
    if let Some(decision) = stage.decision.as_ref() {
        out.push_str(&stage_tree_line(
            style,
            &format!("◈ {}", decision.title),
            |text| style.bold(text),
        ));
        for field in &decision.fields {
            out.push_str(&stage_tree_decision_field(style, field));
        }
        for section in &decision.sections {
            out.push_str(&stage_tree_line(
                style,
                &format!("✦ {}", section.title),
                |text| style.bold(text),
            ));
            let rendered = cli_markdown::render_markdown(&section.body, style);
            for line in rendered.trim_end().lines() {
                if line.trim().is_empty() {
                    continue;
                }
                out.push_str(&stage_tree_line(style, line, |text| text.to_string()));
            }
        }
    }

    let body = stage.text.trim();
    if !body.is_empty() && stage.decision.is_none() {
        let body = body.to_string();
        let rendered = cli_markdown::render_markdown(&body, style);
        for line in rendered.trim_end().lines() {
            if line.trim().is_empty() {
                continue;
            }
            out.push_str(&stage_tree_line(style, line, |text| text.to_string()));
        }
    }
    out
}

fn scheduler_stage_header(stage: &SchedulerStageBlock) -> String {
    let label = stage
        .profile
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|profile| {
            if stage
                .title
                .to_ascii_lowercase()
                .starts_with(&profile.to_ascii_lowercase())
            {
                stage.title.clone()
            } else {
                format!("{profile} · {}", stage.title)
            }
        })
        .unwrap_or_else(|| stage.title.clone());
    match (stage.stage_index, stage.stage_total) {
        (Some(index), Some(total)) if total > 0 => format!("{label} [{index}/{total}]"),
        _ => label,
    }
}

fn scheduler_stage_token_summary(stage: &SchedulerStageBlock) -> String {
    format!(
        "{}/{}",
        stage
            .prompt_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
        stage
            .completion_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string())
    )
}

fn scheduler_stage_secondary_token_summary(stage: &SchedulerStageBlock) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(reasoning) = stage.reasoning_tokens {
        parts.push(format!("reasoning {reasoning}"));
    }
    if let Some(cache_read) = stage.cache_read_tokens {
        parts.push(format!("cache read {cache_read}"));
    }
    if let Some(cache_write) = stage.cache_write_tokens {
        parts.push(format!("cache write {cache_write}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn scheduler_stage_skill_tree_summary(stage: &SchedulerStageBlock) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(estimated) = stage.estimated_context_tokens {
        parts.push(format!("est {estimated}"));
    }
    if let Some(budget) = stage.skill_tree_budget {
        parts.push(format!("budget {budget}"));
    }
    if let Some(strategy) = stage
        .skill_tree_truncation_strategy
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("truncate {strategy}"));
    }
    if let Some(truncated) = stage.skill_tree_truncated {
        parts.push(if truncated {
            "truncated".to_string()
        } else {
            "full".to_string()
        });
    }
    if let Some(retry_attempt) = stage.retry_attempt {
        parts.push(format!("retry {retry_attempt}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn stage_card_content_width(style: &CliStyle) -> usize {
    usize::from(style.width).saturating_sub(8).clamp(24, 96)
}

fn stage_tree_line(
    style: &CliStyle,
    raw_text: &str,
    render: impl FnOnce(&str) -> String,
) -> String {
    let max_width = stage_card_content_width(style);
    let truncated = truncate_display(raw_text, max_width);
    format!("  {} {}\n", style.dim(style.tree_end()), render(&truncated))
}

fn stage_tree_field(
    style: &CliStyle,
    label: &str,
    value: &str,
    render: impl FnOnce(&str) -> String,
) -> String {
    let reserved = label.len().saturating_add(2);
    let max_width = stage_card_content_width(style).saturating_sub(reserved);
    let truncated = truncate_display(value, max_width.max(8));
    let body = format!("{label}: {truncated}");
    format!("  {} {}\n", style.dim(style.tree_end()), render(&body))
}

fn stage_tree_decision_field(style: &CliStyle, field: &SchedulerDecisionField) -> String {
    let label = field.label.trim();
    let reserved = label.len().saturating_add(2);
    let max_width = stage_card_content_width(style)
        .saturating_sub(reserved)
        .max(8);
    let value = truncate_display(&decision_field_display_value(field), max_width);
    let rendered_value = decision_field_rendered_value_text(field, &value, style);
    format!(
        "  {} {} {}\n",
        style.dim(style.tree_end()),
        style.bold(&format!("{label}:")),
        rendered_value
    )
}

fn scheduler_status_label(status: &str) -> &str {
    match status {
        "waiting" => "? waiting",
        "running" => "@ running",
        "cancelling" => "~ cancelling",
        "cancelled" => "x cancelled",
        "done" => "+ done",
        "blocked" => "! blocked",
        _ => status,
    }
}

fn decision_field_display_value(field: &SchedulerDecisionField) -> String {
    field.value.clone()
}

fn decision_field_rendered_value_text(
    field: &SchedulerDecisionField,
    value: &str,
    style: &CliStyle,
) -> String {
    match field.tone.as_deref() {
        Some("success") => style.bold_green(value),
        Some("warning") => style.bold_yellow(value),
        Some("error") => style.bold_red(value),
        Some("info") => style.bold_cyan(value),
        Some("muted") => style.dim(value),
        Some("status") => match value.to_ascii_lowercase().as_str() {
            "done" => style.bold_green(value),
            "blocked" => style.bold_red(value),
            _ => style.bold_yellow(value),
        },
        _ => value.to_string(),
    }
}

/// Rich rendering of completed tool results.
fn render_tool_done_rich(tool: &ToolBlock, style: &CliStyle) -> String {
    if let Some(ref structured) = tool.structured {
        match structured {
            ToolStructuredDetail::FileEdit {
                file_path: _,
                diff_preview,
            } => {
                if let Some(diff) = diff_preview {
                    let rendered_diff = render_diff_preview(diff, style);
                    return render_tool_detail_block(
                        Some(style.dim("updated")),
                        Some(rendered_diff),
                        style,
                    );
                }
            }
            ToolStructuredDetail::FileWrite {
                file_path: _,
                bytes,
                lines,
                diff_preview,
            } => {
                let mut summary_parts = Vec::new();
                if let Some(l) = lines {
                    summary_parts.push(format!("{} lines", l));
                }
                if let Some(b) = bytes {
                    summary_parts.push(format!("{} bytes", b));
                }
                let summary = if summary_parts.is_empty() {
                    "written".to_string()
                } else {
                    format!("wrote {}", summary_parts.join(", "))
                };
                if let Some(diff) = diff_preview {
                    let rendered_diff = render_diff_preview(diff, style);
                    return render_tool_detail_block(
                        Some(style.dim(&summary)),
                        Some(rendered_diff),
                        style,
                    );
                }
                return render_tool_detail_block(Some(style.dim(&summary)), None, style);
            }
            ToolStructuredDetail::FileRead {
                file_path: _,
                total_lines,
                truncated,
            } => {
                let mut parts = Vec::new();
                if let Some(n) = total_lines {
                    parts.push(format!("{} lines", n));
                }
                if *truncated {
                    parts.push("truncated".to_string());
                }
                let summary = if parts.is_empty() {
                    "read".to_string()
                } else {
                    parts.join(", ")
                };
                return render_tool_detail_block(Some(style.dim(&summary)), None, style);
            }
            ToolStructuredDetail::BashExec {
                command_preview: _,
                exit_code,
                output_preview,
                truncated,
            } => {
                let preview_line = output_preview.as_ref().map(|preview| {
                    let collapsed = style.collapse_with_width(preview, 5, 2, None);
                    style.dim(&collapsed)
                });
                let exit_str = match exit_code {
                    Some(0) | None => style.green("exit 0"),
                    Some(code) => style.red(&format!("exit {}", code)),
                };
                let mut suffix = exit_str;
                if *truncated {
                    suffix.push_str(&style.dim(" (truncated)"));
                }
                let body = preview_line
                    .as_ref()
                    .map(|preview| format!("{preview}\n{suffix}"))
                    .unwrap_or(suffix);
                return render_tool_detail_block(None, Some(body), style);
            }
            ToolStructuredDetail::Search {
                pattern: _,
                matches,
                truncated,
            } => {
                let mut parts = Vec::new();
                if let Some(n) = matches {
                    parts.push(format!("{} matches", n));
                }
                if *truncated {
                    parts.push("truncated".to_string());
                }
                let summary = if parts.is_empty() {
                    "searched".to_string()
                } else {
                    parts.join(", ")
                };
                return render_tool_detail_block(Some(style.dim(&summary)), None, style);
            }
            ToolStructuredDetail::Generic => {}
        }
    }

    // Fallback: no structured data
    let detail = tool.detail.as_deref().unwrap_or("");
    if detail.is_empty() {
        render_tool_detail_block(Some(style.green("Done")), None, style)
    } else {
        let collapsed = style.collapse_with_width(detail, 5, 2, None);
        render_tool_detail_block(Some(collapsed), None, style)
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
fn format_tool_header(tool: &ToolBlock) -> String {
    let display = tool_display_name(&tool.name);

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
        "task" => "Task".to_string(),
        "task_flow" => "TaskFlow".to_string(),
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
        "plan_enter" => "PlanEnter".to_string(),
        "plan_exit" => "PlanExit".to_string(),
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
            event: "subtask".to_string(),
            title: "Subtask · inspect scheduler".to_string(),
            status: Some("pending".to_string()),
            summary: Some("Subtask `task_1` is `pending`.".to_string()),
            fields: vec![SessionEventField {
                label: "ID".to_string(),
                value: "task_1".to_string(),
                tone: None,
            }],
            body: None,
        }));
        assert!(line.contains("[session_event] Subtask · inspect scheduler [subtask · pending]"));
        assert!(line.contains("summary: Subtask `task_1` is `pending`."));
    }

    #[test]
    fn renders_queue_item_blocks() {
        let line = render_cli_block(&OutputBlock::QueueItem(QueueItemBlock {
            position: 2,
            text: "run verification".to_string(),
        }));
        assert_eq!(line, "[queue_item] [2] run verification\n");
    }

    #[test]
    fn renders_scheduler_stage_blocks() {
        let line = render_cli_block(&OutputBlock::SchedulerStage(Box::new(
            SchedulerStageBlock {
                stage_id: None,
                profile: Some("prometheus".to_string()),
                stage: "plan".to_string(),
                title: "Prometheus · Plan".to_string(),
                text: "Drafting plan".to_string(),
                stage_index: Some(2),
                stage_total: Some(5),
                step: Some(3),
                status: Some("running".to_string()),
                focus: Some("planning".to_string()),
                last_event: Some("Tool finished: Read".to_string()),
                waiting_on: Some("model".to_string()),
                estimated_context_tokens: Some(2048),
                skill_tree_budget: Some(4096),
                skill_tree_truncation_strategy: Some("tail".to_string()),
                skill_tree_truncated: Some(false),
                retry_attempt: None,
                activity: Some("Task → build\n- label: Schema migration".to_string()),
                loop_budget: None,
                available_skill_count: None,
                available_agent_count: None,
                available_category_count: None,
                active_skills: Vec::new(),
                active_agents: Vec::new(),
                active_categories: Vec::new(),
                done_agent_count: 0,
                total_agent_count: 0,
                prompt_tokens: Some(1200),
                context_tokens: Some(1200),
                completion_tokens: Some(320),
                reasoning_tokens: Some(0),
                cache_read_tokens: Some(0),
                cache_miss_tokens: Some(0),
                cache_write_tokens: Some(0),
                decision: None,
                attached_session_id: None,
            },
        )));
        assert!(line.contains("[scheduler_stage] Prometheus · Plan [2/5]"));
        assert!(line.contains("step=3"));
        assert!(line.contains("waiting_on=model"));
        assert!(line.contains("tokens=1200/320"));
        assert!(line.contains("usage: reasoning 0 · cache read 0 · cache write 0"));
        assert!(line.contains("skill tree: est 2048"));
        assert!(line.contains("budget 4096"));
        assert!(line.contains("activity:"));
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
        assert!(!out.starts_with('\n'));
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
        assert!(out.contains("thinking"));
    }

    #[test]
    fn rich_reasoning_end_leaves_a_block_boundary() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(&OutputBlock::Reasoning(ReasoningBlock::end()), &style);
        assert_eq!(out, "\n");
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
    fn rich_scheduler_stage_includes_runtime_fields() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_cli_block_rich(
            &OutputBlock::SchedulerStage(Box::new(SchedulerStageBlock {
                stage_id: None,
                profile: Some("atlas".to_string()),
                stage: "coordination-gate".to_string(),
                title: "Atlas · Coordination Gate".to_string(),
                text: "Need one more verification pass".to_string(),
                stage_index: Some(3),
                stage_total: Some(4),
                step: Some(2),
                status: Some("waiting".to_string()),
                focus: Some("verification".to_string()),
                last_event: Some("Question started".to_string()),
                waiting_on: Some("user".to_string()),
                estimated_context_tokens: Some(256),
                skill_tree_budget: Some(512),
                skill_tree_truncation_strategy: Some("head-tail".to_string()),
                skill_tree_truncated: Some(true),
                retry_attempt: Some(2),
                activity: Some("Question (1)\n- Scope: proceed with review?".to_string()),
                loop_budget: None,
                available_skill_count: Some(3),
                available_agent_count: Some(2),
                available_category_count: Some(1),
                active_skills: vec!["debug".to_string()],
                active_agents: vec!["explore".to_string()],
                active_categories: vec!["frontend".to_string()],
                done_agent_count: 0,
                total_agent_count: 0,
                prompt_tokens: Some(980),
                context_tokens: Some(980),
                completion_tokens: Some(221),
                reasoning_tokens: Some(0),
                cache_read_tokens: Some(0),
                cache_miss_tokens: Some(0),
                cache_write_tokens: Some(0),
                decision: Some(SchedulerDecisionBlock {
                    kind: "gate".to_string(),
                    title: "Decision".to_string(),
                    spec: default_scheduler_decision_render_spec(),
                    fields: vec![
                        SchedulerDecisionField {
                            label: "Outcome".to_string(),
                            value: "Continue".to_string(),
                            tone: Some("status".to_string()),
                        },
                        SchedulerDecisionField {
                            label: "Why".to_string(),
                            value: "Need one more worker round".to_string(),
                            tone: None,
                        },
                        SchedulerDecisionField {
                            label: "Next Action".to_string(),
                            value: "Verify task B with concrete evidence".to_string(),
                            tone: Some("warning".to_string()),
                        },
                    ],
                    sections: Vec::new(),
                }),
                attached_session_id: None,
            })),
            &style,
        );
        assert!(out.contains("Atlas · Coordination Gate [3/4]"));
        assert!(out.contains("step 2"));
        assert!(out.contains("waiting on user"));
        assert!(out.contains("tokens 980/221"));
        assert!(out.contains("Usage: reasoning 0 · cache read 0 · cache write 0"));
        assert!(out.contains("Skill Tree: est 256"));
        assert!(out.contains("Activity:"));
        assert!(out.contains("◈ Decision"));
    }

    #[test]
    fn rich_scheduler_stage_truncates_long_runtime_lines_for_cli_width() {
        let style = CliStyle {
            color: true,
            width: 48,
        };
        let out = render_cli_block_rich(
            &OutputBlock::SchedulerStage(Box::new(SchedulerStageBlock {
                stage_id: None,
                profile: Some("prometheus".to_string()),
                stage: "route".to_string(),
                title: "Prometheus · Route".to_string(),
                text: String::new(),
                stage_index: Some(1),
                stage_total: Some(5),
                step: Some(1),
                status: Some("running".to_string()),
                focus: Some("Decide the correct workflow and preserve request intent for a very long biomedical planning request".to_string()),
                last_event: Some("Step 1 started with model analysis and route rubric evaluation".to_string()),
                waiting_on: Some("model".to_string()),
                estimated_context_tokens: Some(8192),
                skill_tree_budget: Some(4096),
                skill_tree_truncation_strategy: Some("tail".to_string()),
                skill_tree_truncated: Some(true),
                retry_attempt: None,
                activity: None,
                loop_budget: None,
                available_skill_count: None,
                available_agent_count: None,
                available_category_count: None,
                active_skills: Vec::new(),
                active_agents: Vec::new(),
                active_categories: Vec::new(),
                done_agent_count: 0,
                total_agent_count: 0,
                prompt_tokens: Some(4045),
                context_tokens: Some(4045),
                completion_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_miss_tokens: None,
                cache_write_tokens: None,
                decision: None,
                attached_session_id: None,
            })),
            &style,
        );
        assert!(out.contains("Focus:"));
        assert!(out.contains("Last:"));
        assert!(out.contains("…"));
        assert!(!out.contains("━━━━━━━━"));
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

    #[test]
    fn plain_scheduler_stage_renders_attached_session_id() {
        let stage = SchedulerStageBlock {
            stage_id: None,
            profile: None,
            stage: "execution".to_string(),
            title: "Execution".to_string(),
            text: String::new(),
            stage_index: None,
            stage_total: None,
            step: None,
            status: Some("running".to_string()),
            focus: None,
            last_event: None,
            waiting_on: None,
            estimated_context_tokens: None,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            activity: None,
            loop_budget: None,
            available_skill_count: None,
            available_agent_count: None,
            available_category_count: None,
            active_skills: Vec::new(),
            active_agents: Vec::new(),
            active_categories: Vec::new(),
            done_agent_count: 0,
            total_agent_count: 0,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            decision: None,
            attached_session_id: Some("child-abc-123".to_string()),
        };
        let out = render_cli_block(&OutputBlock::SchedulerStage(Box::new(stage)));
        assert!(out.contains("attached session: child-abc-123"));
    }

    #[test]
    fn rich_scheduler_stage_renders_attached_session_id() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let stage = SchedulerStageBlock {
            stage_id: None,
            profile: None,
            stage: "execution".to_string(),
            title: "Execution".to_string(),
            text: String::new(),
            stage_index: None,
            stage_total: None,
            step: None,
            status: Some("running".to_string()),
            focus: None,
            last_event: None,
            waiting_on: None,
            estimated_context_tokens: None,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            activity: None,
            loop_budget: None,
            available_skill_count: None,
            available_agent_count: None,
            available_category_count: None,
            active_skills: Vec::new(),
            active_agents: Vec::new(),
            active_categories: Vec::new(),
            done_agent_count: 0,
            total_agent_count: 0,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            decision: None,
            attached_session_id: Some("child-xyz-789".to_string()),
        };
        let out = render_cli_block_rich(&OutputBlock::SchedulerStage(Box::new(stage)), &style);
        assert!(out.contains("Attached Session"));
        assert!(out.contains("child-xyz-789"));
    }

    #[test]
    fn to_summary_projects_stage_block_correctly() {
        use crate::stage_protocol::StageStatus;

        let stage = SchedulerStageBlock {
            stage_id: Some("stage_abc".to_string()),
            profile: Some("atlas".to_string()),
            stage: "planning".to_string(),
            title: "Planning".to_string(),
            text: "Analyzing requirements...".to_string(),
            stage_index: Some(1),
            stage_total: Some(3),
            step: Some(2),
            status: Some("running".to_string()),
            focus: Some("code analysis".to_string()),
            last_event: Some("tool_call".to_string()),
            waiting_on: None,
            estimated_context_tokens: Some(300),
            skill_tree_budget: Some(512),
            skill_tree_truncation_strategy: Some("head-tail".to_string()),
            skill_tree_truncated: Some(false),
            retry_attempt: Some(1),
            activity: Some("reading files".to_string()),
            loop_budget: Some("step-limit:5".to_string()),
            available_skill_count: Some(10),
            available_agent_count: Some(3),
            available_category_count: Some(2),
            active_skills: vec!["read".to_string()],
            active_agents: vec!["planner".to_string(), "reviewer".to_string()],
            active_categories: vec!["coding".to_string()],
            done_agent_count: 0,
            total_agent_count: 0,
            prompt_tokens: Some(100),
            context_tokens: Some(100),
            completion_tokens: Some(50),
            reasoning_tokens: Some(25),
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            decision: None,
            attached_session_id: Some("child_001".to_string()),
        };

        let summary = stage.to_summary();
        assert_eq!(summary.stage_id, "stage_abc");
        assert_eq!(summary.stage_name, "planning");
        assert_eq!(summary.index, Some(1));
        assert_eq!(summary.total, Some(3));
        assert_eq!(summary.step, Some(2));
        assert_eq!(summary.step_total, Some(5)); // parsed from "step-limit:5"
        assert_eq!(summary.status, StageStatus::Running);
        assert_eq!(summary.prompt_tokens, Some(100));
        assert_eq!(summary.completion_tokens, Some(50));
        assert_eq!(summary.reasoning_tokens, Some(25));
        assert_eq!(summary.focus, Some("code analysis".to_string()));
        assert_eq!(summary.last_event, Some("tool_call".to_string()));
        assert_eq!(summary.active_agent_count, 2); // two active agents
        assert_eq!(summary.active_tool_count, 0); // always 0 from presentation layer
        assert_eq!(summary.attached_session_count, 1);
        assert_eq!(
            summary.primary_attached_session_id,
            Some("child_001".to_string())
        );
    }

    #[test]
    fn to_summary_defaults_when_stage_id_missing() {
        use crate::stage_protocol::StageStatus;

        let stage = SchedulerStageBlock {
            stage_id: None,
            profile: None,
            stage: "init".to_string(),
            title: String::new(),
            text: String::new(),
            stage_index: None,
            stage_total: None,
            step: None,
            status: None,
            focus: None,
            last_event: None,
            waiting_on: None,
            estimated_context_tokens: None,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            activity: None,
            loop_budget: Some("unbounded".to_string()),
            available_skill_count: None,
            available_agent_count: None,
            available_category_count: None,
            active_skills: Vec::new(),
            active_agents: Vec::new(),
            active_categories: Vec::new(),
            done_agent_count: 0,
            total_agent_count: 0,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            decision: None,
            attached_session_id: None,
        };

        let summary = stage.to_summary();
        assert_eq!(summary.stage_id, ""); // defaults to empty
        assert_eq!(summary.status, StageStatus::Running); // None → Running
        assert_eq!(summary.step_total, None); // "unbounded" → None
        assert_eq!(summary.attached_session_count, 0);
        assert_eq!(summary.primary_attached_session_id, None);
    }
}
