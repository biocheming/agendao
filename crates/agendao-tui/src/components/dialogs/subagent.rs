use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug)]
pub struct SubagentInfo {
    pub id: String,
    pub name: String,
    pub category: String,
    pub messages: Vec<SubagentMessage>,
}

#[derive(Clone, Debug)]
pub struct SubagentMessage {
    pub role: String,
    pub content: String,
}

pub struct SubagentDialog {
    pub subagent: Option<SubagentInfo>,
    pub open: bool,
    pub scroll_offset: u16,
}

impl SubagentDialog {
    pub fn new() -> Self {
        Self {
            subagent: None,
            open: false,
            scroll_offset: 0,
        }
    }

    pub fn open(&mut self, subagent: SubagentInfo) {
        self.subagent = Some(subagent);
        self.open = true;
        self.scroll_offset = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.subagent = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let subagent = match &self.subagent {
            Some(s) => s,
            None => return,
        };

        let height = area.height.saturating_sub(4).min(20);
        let width = area.width.saturating_sub(4).min(80);
        let popup_area = super::centered_rect(width, height, area);

        let title = format!(" Subagent: {} [{}] ", subagent.name, subagent.category);

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            &title,
            Style::default().fg(theme.primary).bold(),
        )));
        lines.push(Line::from(""));

        for msg in &subagent.messages {
            let role_style = if msg.role == "user" {
                Style::default().fg(theme.primary)
            } else {
                Style::default().fg(theme.success)
            };
            lines.push(Line::from(vec![
                Span::styled(&msg.role, role_style.bold()),
                Span::raw(":"),
            ]));
            lines.push(Line::from(Span::styled(
                &msg.content,
                Style::default().fg(theme.text),
            )));
            lines.push(Line::from(""));
        }

        let block = Block::default()
            .title(" Subagent ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let content_area = super::dialog_inner(block.inner(popup_area));
        surface.render_widget(block, popup_area);

        let paragraph = Paragraph::new(lines)
            .style(Style::default().bg(theme.background_panel))
            .scroll((self.scroll_offset, 0));

        surface.render_widget(paragraph, content_area);
    }
}

impl Default for SubagentDialog {
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
    fn subagent_dialog_renders_to_buffer_surface() {
        let mut dialog = SubagentDialog::new();
        dialog.open(SubagentInfo {
            id: "sub-1".to_string(),
            name: "Explorer".to_string(),
            category: "explorer".to_string(),
            messages: vec![SubagentMessage {
                role: "assistant".to_string(),
                content: "Mapped the remaining overlays.".to_string(),
            }],
        });

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
