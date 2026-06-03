use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug)]
pub struct RecoveryActionItem {
    pub key: String,
    pub label: String,
    pub description: String,
}

pub struct RecoveryActionDialog {
    items: Vec<RecoveryActionItem>,
    state: ListState,
    open: bool,
}

impl RecoveryActionDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
            open: false,
        }
    }

    pub fn open(&mut self, items: Vec<RecoveryActionItem>) {
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
        let index = match self.state.selected() {
            Some(index) if index + 1 < self.items.len() => index + 1,
            _ => 0,
        };
        self.state.select(Some(index));
    }

    pub fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let index = match self.state.selected() {
            Some(0) | None => self.items.len() - 1,
            Some(index) => index - 1,
        };
        self.state.select(Some(index));
    }

    pub fn selected(&self) -> Option<String> {
        self.state
            .selected()
            .and_then(|index| self.items.get(index))
            .map(|item| item.key.clone())
    }

    pub fn render<S: RenderSurface>(&mut self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let area = super::centered_rect(72, 18, area);
        surface.render_widget(Clear, area);

        let items = self
            .items
            .iter()
            .map(|item| {
                ListItem::new(vec![
                    Line::from(Span::styled(&item.label, Style::default().fg(theme.text))),
                    Line::from(Span::styled(
                        &item.description,
                        Style::default().fg(theme.text_muted),
                    )),
                    Line::from(Span::styled(
                        format!("key: {}", item.key),
                        Style::default().fg(theme.primary),
                    )),
                ])
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Recovery Actions ")
                    .border_style(Style::default().fg(theme.border)),
            )
            .highlight_style(
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        surface.render_stateful_widget(list, area, &mut self.state);
    }
}

impl Default for RecoveryActionDialog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn recovery_action_dialog_renders_to_buffer_surface() {
        let mut dialog = RecoveryActionDialog::new();
        dialog.open(vec![RecoveryActionItem {
            key: "resume".to_string(),
            label: "Resume".to_string(),
            description: "Continue the interrupted execution".to_string(),
        }]);

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
}
