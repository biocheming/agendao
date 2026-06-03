use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug)]
pub struct StashItem {
    pub input: String,
    pub created_at: i64,
}

pub struct PromptStashDialog {
    entries: Vec<StashItem>,
    filtered: Vec<usize>,
    query: String,
    state: ListState,
    open: bool,
}

impl PromptStashDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            entries: Vec::new(),
            filtered: Vec::new(),
            query: String::new(),
            state,
            open: false,
        }
    }

    pub fn set_entries(&mut self, entries: Vec<StashItem>) {
        self.entries = entries;
        self.filter();
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.filter();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.filter();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.filter();
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected > 0 {
                self.state.select(Some(selected - 1));
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected < self.filtered.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
            }
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.state
            .selected()
            .and_then(|idx| self.filtered.get(idx))
            .copied()
    }

    pub fn remove_selected(&mut self) -> Option<usize> {
        let index = self.selected_index()?;
        if index >= self.entries.len() {
            return None;
        }
        self.entries.remove(index);
        self.filter();
        Some(index)
    }

    fn filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.input.to_ascii_lowercase().contains(&query))
            .map(|(idx, _)| idx)
            .collect();
        self.state.select(if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(84, 20, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Prompt Stash ",
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
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);

        surface.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.primary)),
                Span::styled(&self.query, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])),
            layout[0],
        );

        let items = self
            .filtered
            .iter()
            .filter_map(|idx| self.entries.get(*idx))
            .map(|entry| {
                let preview = entry.input.lines().next().unwrap_or_default().trim();
                let time = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(entry.created_at)
                    .map(|dt| {
                        dt.with_timezone(&chrono::Local)
                            .format("%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_else(|| "-".to_string());
                ListItem::new(Line::from(vec![
                    Span::styled(preview, Style::default().fg(theme.text)),
                    Span::styled(format!("  {}", time), Style::default().fg(theme.text_muted)),
                ]))
            })
            .collect::<Vec<_>>();

        surface.render_stateful_widget(
            List::new(items).highlight_style(
                Style::default()
                    .bg(theme.background_element)
                    .add_modifier(Modifier::BOLD),
            ),
            layout[1],
            &mut self.state.clone(),
        );

        surface.render_widget(
            Paragraph::new("Enter load  d delete  Esc close")
                .style(Style::default().fg(theme.text_muted)),
            layout[2],
        );
    }
}

impl Default for PromptStashDialog {
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
    fn prompt_stash_dialog_renders_to_buffer_surface() {
        let mut dialog = PromptStashDialog::new();
        dialog.set_entries(vec![StashItem {
            input: "Summarize the latest migration state".to_string(),
            created_at: 0,
        }]);
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
}
