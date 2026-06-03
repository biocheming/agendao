use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
}

pub struct TagDialog {
    pub tags: Vec<Tag>,
    pub selected_tags: Vec<String>,
    pub state: ListState,
    pub open: bool,
}

impl TagDialog {
    pub fn new() -> Self {
        Self {
            tags: Vec::new(),
            selected_tags: Vec::new(),
            state: ListState::default(),
            open: false,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.state.select(Some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_tags(&mut self, tags: Vec<Tag>) {
        self.tags = tags;
    }

    pub fn toggle_selection(&mut self) {
        if let Some(selected) = self.state.selected() {
            if let Some(tag) = self.tags.get(selected) {
                if self.selected_tags.contains(&tag.id) {
                    self.selected_tags.retain(|id| id != &tag.id);
                } else {
                    self.selected_tags.push(tag.id.clone());
                }
            }
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
            let new = (selected + 1).min(self.tags.len().saturating_sub(1));
            self.state.select(Some(new));
        }
    }

    pub fn selected_tags(&self) -> &[String] {
        &self.selected_tags
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open || self.tags.is_empty() {
            return;
        }

        let height = (self.tags.len() as u16 + 2).min(15);
        let width = 40u16;
        let popup_area = super::centered_rect(width, height, area);
        let block = Block::default()
            .title(" Select Tags ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let content_area = super::dialog_inner(block.inner(popup_area));

        let items: Vec<ListItem> = self
            .tags
            .iter()
            .map(|tag| {
                let is_checked = self.selected_tags.contains(&tag.id);
                let check_mark = if is_checked { "☑" } else { "☐" };
                ListItem::new(Line::from(vec![
                    Span::styled(check_mark, Style::default().fg(theme.primary)),
                    Span::styled(format!(" {}", tag.name), Style::default().fg(theme.text)),
                ]))
            })
            .collect();

        surface.render_widget(block, popup_area);

        let list = List::new(items).highlight_style(Style::default().fg(theme.primary));

        surface.render_widget(list, content_area);
    }
}

impl Default for TagDialog {
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
    fn tag_dialog_renders_to_buffer_surface() {
        let mut dialog = TagDialog::new();
        dialog.set_tags(vec![Tag {
            id: "frontend".to_string(),
            name: "Frontend".to_string(),
            color: None,
        }]);
        dialog.open();

        let area = Rect::new(0, 0, 80, 24);
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
