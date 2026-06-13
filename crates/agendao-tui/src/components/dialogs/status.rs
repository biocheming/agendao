use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use reratui::hooks::use_context;
use reratui::Component;
use std::cell::Cell;

use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

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

#[derive(Clone)]
pub struct StatusDialog {
    lines: Vec<StatusLine>,
    open: bool,
    title: String,
    footer_hint: Option<String>,
    scroll_offset: Cell<u16>,
    last_rendered_area: Cell<Option<Rect>>,
    last_content_area: Cell<Option<Rect>>,
    close_button_area: Cell<Option<Rect>>,
}

impl StatusDialog {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            open: false,
            title: "Status".to_string(),
            footer_hint: None,
            scroll_offset: Cell::new(0),
            last_rendered_area: Cell::new(None),
            last_content_area: Cell::new(None),
            close_button_area: Cell::new(None),
        }
    }

    pub fn set_lines(&mut self, lines: Vec<String>) {
        self.lines = lines.into_iter().map(StatusLine::normal).collect();
        self.scroll_offset.set(0);
    }

    pub fn set_status_lines(&mut self, lines: Vec<StatusLine>) {
        self.lines = lines;
        self.scroll_offset.set(0);
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
        self.scroll_offset.set(0);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.scroll_offset.set(0);
        self.last_rendered_area.set(None);
        self.last_content_area.set(None);
        self.close_button_area.set(None);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn scroll_up(&self) {
        self.scroll_offset
            .set(self.scroll_offset.get().saturating_sub(1));
    }

    pub fn scroll_down(&self) {
        self.scroll_offset
            .set(self.scroll_offset.get().saturating_add(1));
    }

    pub fn page_up(&self, page_size: u16) {
        self.scroll_offset
            .set(self.scroll_offset.get().saturating_sub(page_size.max(1)));
    }

    pub fn page_down(&self, page_size: u16) {
        self.scroll_offset
            .set(self.scroll_offset.get().saturating_add(page_size.max(1)));
    }

    pub fn scroll_to_top(&self) {
        self.scroll_offset.set(0);
    }

    pub fn scroll_to_bottom(&self) {
        self.scroll_offset.set(u16::MAX);
    }

    pub fn contains_point(&self, col: u16, row: u16) -> bool {
        self.last_rendered_area
            .get()
            .is_some_and(|area| contains_point(area, col, row))
    }

    pub fn selection_area(&self) -> Option<Rect> {
        self.last_content_area.get()
    }

    pub fn handle_click(&self, col: u16, row: u16) -> bool {
        self.close_button_area
            .get()
            .is_some_and(|area| contains_point(area, col, row))
    }

    fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
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
        self.last_content_area.set(Some(layout[0]));

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
        let viewport_height = layout[0].height as usize;
        let max_scroll = lines.len().saturating_sub(viewport_height) as u16;
        let scroll = self.scroll_offset.get().min(max_scroll);
        self.scroll_offset.set(scroll);
        surface.render_widget(Paragraph::new(lines).scroll((scroll, 0)), layout[0]);

        if max_scroll > 0 && layout[0].width > 1 {
            let scroll_area = Rect {
                x: layout[0]
                    .x
                    .saturating_add(layout[0].width.saturating_sub(1)),
                y: layout[0].y,
                width: 1,
                height: layout[0].height,
            };
            let mut scrollbar_state = ScrollbarState::new((max_scroll as usize).saturating_add(1))
                .position(scroll as usize)
                .viewport_content_length(viewport_height);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(theme.primary))
                .track_style(Style::default().fg(theme.border));
            surface.render_stateful_widget(scrollbar, scroll_area, &mut scrollbar_state);
        }

        let footer = self.footer_hint.as_ref().map_or_else(
            || {
                if max_scroll > 0 {
                    "Up/Down/PgUp/PgDn scroll · Drag to select · Ctrl+Shift+C copy · Esc close"
                        .to_string()
                } else {
                    "Drag to select · Ctrl+Shift+C copy · Esc close".to_string()
                }
            },
            |hint| {
                if max_scroll > 0 {
                    format!(
                        "{hint} · Up/Down/PgUp/PgDn scroll · Drag to select · Ctrl+Shift+C copy · Esc close"
                    )
                } else {
                    format!("{hint} · Drag to select · Ctrl+Shift+C copy · Esc close")
                }
            },
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

impl Component for StatusDialog {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let mut surface = BufferSurface::new(buffer);
        self.render_surface(&mut surface, area, &theme);
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

        dialog.render_surface(&mut surface, area, &Theme::dark());

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
        dialog.render_surface(&mut surface, area, &Theme::dark());

        assert!(dialog.contains_point(15, 5));
        assert!(dialog.handle_click(102, 4));
        assert!(!dialog.handle_click(15, 5));
    }

    #[test]
    fn status_dialog_scrolls_long_content() {
        let mut dialog = StatusDialog::new();
        dialog.set_status_lines(
            (0..40)
                .map(|i| StatusLine::normal(format!("line {i}")))
                .collect(),
        );
        dialog.open();
        dialog.page_down(10);

        let area = Rect::new(0, 0, 80, 18);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);
        dialog.render_surface(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| cell.symbol() == "1")
            .count();
        assert!(
            rendered > 0,
            "expected scrolled content to render later lines"
        );
    }
}
