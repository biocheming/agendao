use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use agendao_command::{CommandRegistry, UiActionId};

use crate::command::fuzzy_match;
use crate::theme::Theme;
use crate::ui::RenderSurface;

pub struct SlashCommandPopup {
    pub registry: CommandRegistry,
    pub query: String,
    pub filtered: Vec<UiActionId>,
    pub state: ListState,
    pub open: bool,
    pub selected_action: Option<UiActionId>,
}

impl SlashCommandPopup {
    pub fn new() -> Self {
        Self {
            registry: CommandRegistry::new(),
            query: String::new(),
            filtered: Vec::new(),
            state: ListState::default(),
            open: false,
            selected_action: None,
        }
    }

    pub fn open(&mut self) {
        self.query = String::new();
        self.refresh_filter();
        self.state.select(Some(0));
        self.open = true;
        self.selected_action = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.filtered.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn take_action(&mut self) -> Option<UiActionId> {
        self.selected_action.take()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    fn refresh_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = self
                .registry
                .ui_suggested_slash_commands()
                .iter()
                .map(|cmd| cmd.action_id)
                .collect();
        } else {
            let mut scored: Vec<(UiActionId, i32)> = self
                .registry
                .ui_all_slash_commands()
                .into_iter()
                .filter_map(|cmd| {
                    let slash = cmd.slash.as_ref()?;
                    let name_score = fuzzy_match(&self.query, slash.name);
                    let alias_score = slash
                        .aliases
                        .iter()
                        .filter_map(|alias| fuzzy_match(&self.query, alias))
                        .max();
                    let title_score = fuzzy_match(&self.query, cmd.title);
                    let best = name_score
                        .into_iter()
                        .chain(alias_score)
                        .chain(title_score)
                        .max()?;
                    Some((cmd.action_id, best))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(action_id, _)| action_id).collect();
        }
        self.state.select(Some(0));
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.refresh_filter();
    }

    pub fn handle_backspace(&mut self) -> bool {
        if self.query.pop().is_some() {
            self.refresh_filter();
            true
        } else {
            false
        }
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            let new = selected.saturating_sub(1);
            self.state.select(Some(new));
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            let new = (selected + 1).min(self.filtered.len().saturating_sub(1));
            self.state.select(Some(new));
        }
    }

    pub fn select_current(&mut self) {
        if let Some(idx) = self.state.selected() {
            if let Some(action_id) = self.filtered.get(idx) {
                if self.registry.ui_command(*action_id).is_some() {
                    self.selected_action = Some(*action_id);
                    self.close();
                }
            }
        }
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open || self.filtered.is_empty() {
            return;
        }

        let width = 50.min(area.width.saturating_sub(4));
        let height = (10.min(self.filtered.len()) as u16).saturating_add(2);

        let x = area.x + (area.width - width) / 2;
        let y = area.y.saturating_sub(height + 1);

        let popup_area = Rect::new(x.max(1), y.max(1), width, height);

        let query_line = Line::from(vec![
            Span::raw("/"),
            Span::styled(&self.query, Style::default().fg(theme.primary)),
        ]);

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .enumerate()
            .filter_map(|(idx, action_id)| {
                let cmd = self.registry.ui_command(*action_id)?;
                let slash = cmd.slash.as_ref()?;
                let title = slash.name;
                let keybind = cmd.keybind;

                let is_selected = self.state.selected() == Some(idx);
                let style = if is_selected {
                    Style::default()
                        .fg(theme.primary)
                        .bg(theme.background_element)
                } else {
                    Style::default().fg(theme.text)
                };

                let content = if let Some(kb) = keybind {
                    Line::from(vec![
                        Span::styled(title, style),
                        Span::styled(format!("  ({})", kb), Style::default().fg(theme.text_muted)),
                    ])
                } else {
                    Line::from(Span::styled(title, style))
                };

                Some(ListItem::new(content))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title(query_line)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border)),
            )
            .highlight_style(Style::default().fg(theme.primary));

        surface.render_widget(list, popup_area);
    }
}

impl Default for SlashCommandPopup {
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
    fn slash_popup_renders_to_buffer_surface() {
        let mut popup = SlashCommandPopup::new();
        popup.open();

        let area = Rect::new(0, 10, 100, 20);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        popup.render(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }
}
