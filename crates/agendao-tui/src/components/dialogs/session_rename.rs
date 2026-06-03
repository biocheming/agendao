use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

pub struct SessionRenameDialog {
    open: bool,
    session_id: Option<String>,
    input: String,
}

impl SessionRenameDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            session_id: None,
            input: String::new(),
        }
    }

    pub fn open(&mut self, session_id: String, title: String) {
        self.open = true;
        self.session_id = Some(session_id);
        self.input = title;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.session_id = None;
        self.input.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn handle_input(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn handle_backspace(&mut self) {
        self.input.pop();
    }

    pub fn confirm(&mut self) -> Option<(String, String)> {
        let session_id = self.session_id.clone()?;
        let title = self.input.trim().to_string();
        if title.is_empty() {
            return None;
        }
        self.close();
        Some((session_id, title))
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(70, 8, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Rename Session ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        surface.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.primary)),
                Span::styled(&self.input, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])),
            layout[0],
        );

        surface.render_widget(
            Paragraph::new("Enter save  Esc cancel").style(Style::default().fg(theme.text_muted)),
            layout[2],
        );
    }
}

impl Default for SessionRenameDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn session_rename_dialog_renders_to_buffer_surface() {
        let mut dialog = SessionRenameDialog::new();
        dialog.open("session-1".to_string(), "Migration".to_string());

        let area = Rect::new(0, 0, 100, 24);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        dialog.render(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }
}
