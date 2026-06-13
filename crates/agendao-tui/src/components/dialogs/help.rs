use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use reratui::hooks::use_context;
use reratui::Component;

use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

#[derive(Clone)]
pub struct HelpDialog {
    open: bool,
}

impl HelpDialog {
    pub fn new() -> Self {
        Self { open: false }
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(74, 20, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Help ",
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
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let lines = vec![
            Line::from(Span::styled(
                "General",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Ctrl+X  Open command list"),
            Line::from("  Alt+Right  Open attached session"),
            Line::from("  Alt+Left   Return to parent session"),
            Line::from("  Ctrl+P  Open command palette"),
            Line::from("  F1      Open help"),
            Line::from("  Ctrl+K  Cancel running tool/session"),
            Line::from("  Ctrl+G  Toggle reasoning blocks"),
            Line::from("  Ctrl+C or Ctrl+X q  Exit TUI"),
            Line::from(""),
            Line::from(Span::styled(
                "Prompt",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Enter   Submit prompt"),
            Line::from("  Ctrl+U  Clear prompt"),
            Line::from("  Ctrl+V  Paste text or clipboard image"),
            Line::from("  Ctrl+Shift+C  Copy prompt"),
            Line::from("  Ctrl+Shift+X  Cut prompt"),
            Line::from("  Alt+Up/Alt+Down  Prompt history"),
            Line::from("  /image <path>  Attach a local image"),
            Line::from(""),
            Line::from(Span::styled(
                "Dialogs",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Ctrl+M  Model list"),
            Line::from("  Ctrl+L  Cycle model variant"),
            Line::from("  Use /agents to open full agent list"),
            Line::from("  Ctrl+S  Toggle sidebar"),
            Line::from("  Use command palette for session/theme/status/MCP dialogs"),
            Line::from("  Use command palette -> Toggle appearance to switch dark/light"),
        ];

        surface.render_widget(
            Paragraph::new(lines).style(Style::default().fg(theme.text)),
            layout[0],
        );
        surface.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" close", Style::default().fg(theme.text_muted)),
            ])),
            layout[1],
        );
    }
}

impl Component for HelpDialog {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let mut surface = BufferSurface::new(buffer);
        self.render_surface(&mut surface, area, &theme);
    }
}

impl Default for HelpDialog {
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
    use crate::ui::BufferSurface;
    use ratatui::buffer::Buffer;

    #[test]
    fn help_dialog_renders_to_buffer_surface() {
        let mut dialog = HelpDialog::new();
        dialog.open();

        let area = Rect::new(0, 0, 100, 30);
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
}
