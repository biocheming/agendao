use crate::cli_markdown;
use crate::cli_style::CliStyle;
use crate::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};
use crate::terminal_segment_display::{
    render_tool_header_line, TerminalSegmentDisplayLine, TerminalSegmentTone,
};
use crate::terminal_tool_block_display::{
    build_file_items, build_image_items, build_tool_body_items, TerminalBlockItem,
    TerminalToolBlockItem,
};

pub fn render_cli_tool_lines(
    name: &str,
    arguments: &str,
    state: TerminalToolState,
    result: Option<&TerminalToolResultInfo>,
    show_tool_details: bool,
    style: &CliStyle,
) -> Vec<String> {
    let mut lines = vec![render_cli_display_line(
        &render_tool_header_line(name, arguments, state),
        style,
    )];
    let body_items = build_tool_body_items(name, arguments, state, result, show_tool_details);
    lines.extend(render_cli_tool_block_items(&body_items, style));
    lines
}

pub fn render_cli_tool_block_items(
    items: &[TerminalToolBlockItem],
    style: &CliStyle,
) -> Vec<String> {
    render_cli_block_items(items, style)
}

pub fn render_cli_file_lines(path: &str, mime: &str, style: &CliStyle) -> Vec<String> {
    render_cli_block_items(&build_file_items(path, mime), style)
}

pub fn render_cli_image_lines(url: &str, style: &CliStyle) -> Vec<String> {
    render_cli_block_items(&build_image_items(url), style)
}

pub fn render_cli_block_items(items: &[TerminalBlockItem], style: &CliStyle) -> Vec<String> {
    let mut lines = Vec::new();
    for item in items {
        match item {
            TerminalBlockItem::Line(line) => lines.push(render_cli_display_line(line, style)),
            TerminalBlockItem::Markdown { content } => {
                append_multiline_render(&mut lines, &cli_markdown::render_markdown(content, style));
            }
            TerminalBlockItem::Diff { label, content } => {
                if let Some(label) = label {
                    lines.push(render_cli_display_line(label, style));
                }
                lines.extend(render_cli_diff_lines(content, style));
            }
        }
    }
    lines
}

fn render_cli_display_line(line: &TerminalSegmentDisplayLine, style: &CliStyle) -> String {
    match line.tone {
        TerminalSegmentTone::Primary => line.text.clone(),
        TerminalSegmentTone::Muted => style.dim(&line.text),
        TerminalSegmentTone::Success => style.green(&line.text),
        TerminalSegmentTone::Error => style.red(&line.text),
        TerminalSegmentTone::Info => style.cyan(&line.text),
        TerminalSegmentTone::Warning => style.yellow(&line.text),
    }
}

fn render_cli_diff_lines(diff: &str, style: &CliStyle) -> Vec<String> {
    diff.lines()
        .map(|line| {
            if line.starts_with("@@") {
                style.cyan(line)
            } else if line.starts_with("+++") || line.starts_with("---") {
                style.dim(line)
            } else if line.starts_with('+') {
                style.green(line)
            } else if line.starts_with('-') {
                style.red(line)
            } else {
                line.to_string()
            }
        })
        .collect()
}

fn append_multiline_render(lines: &mut Vec<String>, rendered: &str) {
    if rendered.is_empty() {
        return;
    }

    let mut parts: Vec<String> = rendered.split('\n').map(str::to_string).collect();
    if parts.last().is_some_and(|line| line.is_empty()) {
        parts.pop();
    }
    lines.extend(parts);
}

#[cfg(test)]
mod tests {
    use super::{render_cli_image_lines, render_cli_tool_lines};
    use crate::cli_style::CliStyle;
    use crate::terminal_presentation::{TerminalToolResultInfo, TerminalToolState};
    use std::collections::HashMap;

    #[test]
    fn render_cli_tool_lines_uses_shared_task_block_items() {
        let style = CliStyle::plain();
        let lines = render_cli_tool_lines(
            "task",
            r###"{"category":"visual-engineering","prompt":"## 1. TASK\nRedesign page\n- [ ] 修改 t2.html"}"###,
            TerminalToolState::Running,
            None,
            false,
            &style,
        );

        assert!(lines.iter().any(|line| line.contains("task")));
        assert!(lines
            .iter()
            .any(|line| line.contains("Delegating task to subagent")));
        assert!(lines
            .iter()
            .any(|line| line.contains("Checklist (1 items):")));
    }

    #[test]
    fn render_cli_tool_lines_render_diff_and_markdown_items() {
        let style = CliStyle::plain();
        let mut metadata = HashMap::new();
        metadata.insert(
            "diff".to_string(),
            serde_json::json!("@@ -1 +1 @@\n-old\n+new"),
        );
        let lines = render_cli_tool_lines(
            "write",
            r#"{"file_path":"./new_file.txt"}"#,
            TerminalToolState::Completed,
            Some(&TerminalToolResultInfo {
                output: "Successfully wrote 10 bytes (2 lines) to ./new_file.txt".to_string(),
                is_error: false,
                title: None,
                metadata: Some(metadata),
            }),
            true,
            &style,
        );

        assert!(lines.iter().any(|line| line.contains("Write Complete")));
        assert!(lines.iter().any(|line| line.contains("@@ -1 +1 @@")));
        assert!(lines.iter().any(|line| line.contains("+new")));
    }

    #[test]
    fn render_cli_image_lines_summarize_inline_data_url() {
        let style = CliStyle::plain();
        let lines = render_cli_image_lines("data:image/png;base64,QUJDRA==", &style);
        assert!(lines
            .iter()
            .any(|line| line.contains("[image] inline image")));
        assert!(lines.iter().any(|line| line.contains("type: image/png")));
        assert!(lines.iter().any(|line| line.contains("size: 4 B")));
    }
}
