use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};
use reratui::Component;

use agendao_command_render::branding::{logo_lines, LOGO};

use crate::ui::{BufferSurface, RenderSurface};

/// Re-export for backward compatibility.
pub fn exit_logo_lines(pad: &str) -> Vec<String> {
    logo_lines(pad)
}

#[derive(Clone)]
pub struct Logo {
    border_color: Color,
    primary_color: Color,
    secondary_color: Color,
    muted_color: Color,
    text_color: Color,
}

impl Logo {
    pub fn new(
        border_color: Color,
        primary_color: Color,
        secondary_color: Color,
        muted_color: Color,
        text_color: Color,
    ) -> Self {
        Self {
            border_color,
            primary_color,
            secondary_color,
            muted_color,
            text_color,
        }
    }

    pub fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect) {
        let lines: Vec<Line> = LOGO
            .iter()
            .map(|line| {
                let color = if line.contains('▓') || line.contains('█') {
                    self.primary_color
                } else if line.contains('▒') {
                    self.secondary_color
                } else if line.contains('●') || line.contains('○') {
                    self.primary_color
                } else if line.contains('╭')
                    || line.contains('╮')
                    || line.contains('╰')
                    || line.contains('╯')
                    || line.contains('┬')
                    || line.contains('┴')
                    || line.contains('─')
                {
                    self.border_color
                } else if line.contains('A')
                    || line.contains('G')
                    || line.contains('N')
                    || line.contains('D')
                    || line.contains('O')
                {
                    self.primary_color
                } else if line.contains('T') && line.contains('h') {
                    self.muted_color
                } else {
                    self.text_color
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

impl Component for Logo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let mut surface = BufferSurface::new(buffer);
        self.render_surface(&mut surface, area);
    }
}
