use agendao_command_render::terminal_segment_display::TerminalSegmentTone;
use agendao_command_render::terminal_tool_block_display::TerminalBlockItem;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::markdown::MarkdownRenderer;
use crate::theme::Theme;

pub fn render_shared_message_block_items(
    items: Vec<TerminalBlockItem>,
    marker: &'static str,
    marker_color: Color,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for item in items {
        match item {
            TerminalBlockItem::Line(line) => lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(marker_color)),
                Span::styled(line.text, Style::default().fg(tone_color(line.tone, theme))),
            ])),
            TerminalBlockItem::Markdown { content } => {
                for markdown_line in MarkdownRenderer::new(theme.clone()).to_lines(&content, None) {
                    let mut spans = vec![Span::styled(
                        marker,
                        Style::default().fg(marker_color),
                    )];
                    spans.extend(markdown_line.spans.into_iter().map(|span| {
                        Span::styled(span.content, span.style)
                    }));
                    lines.push(Line::from(spans));
                }
            }
            TerminalBlockItem::Diff { label, content } => {
                if let Some(label) = label {
                    lines.push(Line::from(vec![
                        Span::styled(marker, Style::default().fg(marker_color)),
                        Span::styled(
                            label.text,
                            Style::default()
                                .fg(tone_color(label.tone, theme))
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
                for raw_line in content.lines() {
                    let (text, color) = if raw_line.starts_with("@@") {
                        (raw_line, theme.info)
                    } else if raw_line.starts_with('+') {
                        (raw_line, theme.success)
                    } else if raw_line.starts_with('-') {
                        (raw_line, theme.error)
                    } else {
                        (raw_line, theme.text)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(marker, Style::default().fg(marker_color)),
                        Span::styled(text.to_string(), Style::default().fg(color)),
                    ]));
                }
            }
        }
    }
    lines
}

fn tone_color(tone: TerminalSegmentTone, theme: &Theme) -> Color {
    match tone {
        TerminalSegmentTone::Primary => theme.text,
        TerminalSegmentTone::Muted => theme.text_muted,
        TerminalSegmentTone::Success => theme.success,
        TerminalSegmentTone::Error => theme.error,
        TerminalSegmentTone::Info => theme.info,
        TerminalSegmentTone::Warning => theme.warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn markdown_block_items_render_via_markdown_renderer() {
        let lines = render_shared_message_block_items(
            vec![TerminalBlockItem::Markdown {
                content: "**bold**\n- item".to_string(),
            }],
            "│ ",
            Color::Blue,
            &Theme::default(),
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
        assert!(
            rendered.iter().any(|line| line == "│ bold"),
            "{rendered:?}"
        );
        assert!(
            rendered.iter().any(|line| line == "│ • item"),
            "{rendered:?}"
        );
    }
}
