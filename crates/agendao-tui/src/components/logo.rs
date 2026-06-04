use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use agendao_command_render::branding::{logo_lines, LOGO};

use crate::ui::RenderSurface;

/// Re-export for backward compatibility.
pub fn exit_logo_lines(pad: &str) -> Vec<String> {
    logo_lines(pad)
}

pub struct Logo {
    primary_color: Color,
    muted_color: Color,
}

impl Logo {
    pub fn new(text_color: Color, text_muted_color: Color, _bg_color: Color) -> Self {
        Self {
            primary_color: text_color,
            muted_color: text_muted_color,
        }
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect) {
        let lines: Vec<Line> = LOGO
            .iter()
            .map(|line| {
                let color = if line.contains('▓') || line.contains('◉') {
                    self.primary_color
                } else {
                    self.muted_color
                };
                Line::from(Span::styled(
                    *line,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ))
            })
            .collect();

        let paragraph =
            Paragraph::new(Text::from(lines)).alignment(ratatui::layout::Alignment::Center);

        surface.render_widget(paragraph, area);
    }
}
