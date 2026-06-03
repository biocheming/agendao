use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug)]
pub struct ToolCallItem {
    pub id: String,
    pub tool_name: String,
}

pub struct ToolCallCancelDialog {
    items: Vec<ToolCallItem>,
    state: ListState,
    open: bool,
}

impl ToolCallCancelDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
            open: false,
        }
    }

    pub fn open(&mut self, items: Vec<ToolCallItem>) {
        self.items = items;
        self.open = true;
        self.state.select(Some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn selected(&self) -> Option<String> {
        self.state
            .selected()
            .and_then(|i| self.items.get(i))
            .map(|item| item.id.clone())
    }

    pub fn render<S: RenderSurface>(&mut self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let area = centered_rect(60, 50, area);
        surface.render_widget(Clear, area);

        let title = Block::default()
            .borders(Borders::ALL)
            .title(" Cancel Tool Call ")
            .border_style(Style::default().fg(theme.border));

        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| {
                ListItem::new(Line::from(vec![
                    Span::styled(&item.tool_name, Style::default().fg(theme.text)),
                    Span::styled(" (", Style::default().fg(theme.text_muted)),
                    Span::styled(
                        &item.id[..8.min(item.id.len())],
                        Style::default().fg(theme.text_muted),
                    ),
                    Span::styled(")", Style::default().fg(theme.text_muted)),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(title)
            .highlight_style(
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        surface.render_stateful_widget(list, area, &mut self.state);
    }
}

impl Default for ToolCallCancelDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::BufferSurface;
    use ratatui::buffer::Buffer;

    #[test]
    fn tool_call_cancel_dialog_renders_to_buffer_surface() {
        let mut dialog = ToolCallCancelDialog::new();
        dialog.open(vec![
            ToolCallItem {
                id: "tool-call-12345678".to_string(),
                tool_name: "exec_command".to_string(),
            },
            ToolCallItem {
                id: "tool-call-abcdef12".to_string(),
                tool_name: "apply_patch".to_string(),
            },
        ]);

        let area = Rect::new(0, 0, 100, 30);
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
