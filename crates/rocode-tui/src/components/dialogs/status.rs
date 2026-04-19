use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::cell::Cell;

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug)]
pub enum StatusLineKind {
    Title,
    Normal,
    Muted,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct StatusLine {
    pub text: String,
    pub kind: StatusLineKind,
}

impl StatusLine {
    pub fn title(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Title,
        }
    }

    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Normal,
        }
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Muted,
        }
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Success,
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Warning,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Error,
        }
    }
}

pub struct StatusDialog {
    lines: Vec<StatusLine>,
    open: bool,
    title: String,
    footer_hint: Option<String>,
    last_rendered_area: Cell<Option<Rect>>,
    close_button_area: Cell<Option<Rect>>,
}

impl StatusDialog {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            open: false,
            title: "Status".to_string(),
            footer_hint: None,
            last_rendered_area: Cell::new(None),
            close_button_area: Cell::new(None),
        }
    }

    pub fn set_lines(&mut self, lines: Vec<String>) {
        self.lines = lines.into_iter().map(StatusLine::normal).collect();
    }

    pub fn set_status_lines(&mut self, lines: Vec<StatusLine>) {
        self.lines = lines;
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn set_footer_hint(&mut self, hint: Option<String>) {
        self.footer_hint = hint;
    }

    pub fn reset_chrome(&mut self) {
        self.title = "Status".to_string();
        self.footer_hint = None;
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.last_rendered_area.set(None);
        self.close_button_area.set(None);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn contains_point(&self, col: u16, row: u16) -> bool {
        self.last_rendered_area
            .get()
            .is_some_and(|area| contains_point(area, col, row))
    }

    pub fn handle_click(&self, col: u16, row: u16) -> bool {
        self.close_button_area
            .get()
            .is_some_and(|area| contains_point(area, col, row))
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(90, 24, area);
        self.last_rendered_area.set(Some(dialog_area));
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title.trim()),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);
        let close_button_area = Rect::new(
            dialog_area
                .x
                .saturating_add(dialog_area.width.saturating_sub(4)),
            dialog_area.y,
            3,
            1,
        );
        self.close_button_area.set(Some(close_button_area));
        surface.render_widget(
            Paragraph::new(Span::styled(
                "×",
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(theme.background_panel)),
            close_button_area,
        );

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let lines = if self.lines.is_empty() {
            vec![Line::from(Span::styled(
                "No status data available.",
                Style::default().fg(theme.text_muted),
            ))]
        } else {
            self.lines
                .iter()
                .map(|line| {
                    let style = match line.kind {
                        StatusLineKind::Title => Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                        StatusLineKind::Normal => Style::default().fg(theme.text),
                        StatusLineKind::Muted => Style::default().fg(theme.text_muted),
                        StatusLineKind::Success => Style::default().fg(theme.success),
                        StatusLineKind::Warning => Style::default().fg(theme.warning),
                        StatusLineKind::Error => Style::default().fg(theme.error),
                    };
                    Line::from(Span::styled(&line.text, style))
                })
                .collect()
        };
        surface.render_widget(Paragraph::new(lines), layout[0]);

        let footer = self.footer_hint.as_ref().map_or_else(
            || "Drag to select · Ctrl+Shift+C copy · Esc close".to_string(),
            |hint| format!("{hint} · Drag to select · Ctrl+Shift+C copy · Esc close"),
        );
        surface.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(theme.text_muted),
            ))),
            layout[1],
        );
    }
}

impl Default for StatusDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

fn contains_point(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x
        && col < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn status_dialog_renders_to_buffer_surface() {
        let mut dialog = StatusDialog::new();
        dialog.set_status_lines(vec![
            StatusLine::title("Runtime"),
            StatusLine::success("server connected"),
        ]);
        dialog.open();

        let area = Rect::new(0, 0, 120, 32);
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

    #[test]
    fn status_dialog_tracks_close_button_hitbox() {
        let mut dialog = StatusDialog::new();
        dialog.set_lines(vec!["hello".to_string()]);
        dialog.open();

        let area = Rect::new(0, 0, 120, 32);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);
        dialog.render(&mut surface, area, &Theme::dark());

        assert!(dialog.contains_point(15, 5));
        assert!(dialog.handle_click(102, 4));
        assert!(!dialog.handle_click(15, 5));
    }
}
