use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use reratui::hooks::use_context;
use reratui::Component;

use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

#[derive(Clone)]
pub struct SkillProposalReviewItem {
    pub id: String,
    pub title: String,
    pub kind_label: String,
    pub skill_name: String,
    pub first_change: String,
}

#[derive(Clone)]
pub struct SkillProposalReviewDialog {
    items: Vec<SkillProposalReviewItem>,
    state: ListState,
    open: bool,
    action_pending: Option<ProposalAction>,
    status_message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalAction {
    Accept,
    Reject,
    Skip,
}

impl SkillProposalReviewDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
            open: false,
            action_pending: None,
            status_message: None,
        }
    }

    pub fn open(&mut self, items: Vec<SkillProposalReviewItem>) {
        let empty = items.is_empty();
        self.items = items;
        self.open = true;
        self.action_pending = None;
        self.status_message = None;
        self.state.select(if empty { None } else { Some(0) });
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.action_pending = None;
        self.status_message = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let index = match self.state.selected() {
            Some(i) if i + 1 < self.items.len() => i + 1,
            _ => 0,
        };
        self.state.select(Some(index));
    }

    pub fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let index = match self.state.selected() {
            Some(0) | None => self.items.len().saturating_sub(1),
            Some(i) => i - 1,
        };
        self.state.select(Some(index));
    }

    pub fn selected_item(&self) -> Option<&SkillProposalReviewItem> {
        self.state.selected().and_then(|i| self.items.get(i))
    }

    pub fn take_action(&mut self) -> Option<(ProposalAction, String)> {
        let action = self.action_pending.take()?;
        let item = self.selected_item()?;
        Some((action, item.id.clone()))
    }

    pub fn set_status(&mut self, message: &str) {
        self.status_message = Some(message.to_string());
    }

    pub fn remove_current(&mut self) {
        let Some(index) = self.state.selected() else {
            return;
        };
        self.items.remove(index);
        if self.items.is_empty() {
            self.state.select(None);
        } else if index >= self.items.len() {
            self.state.select(Some(self.items.len() - 1));
        } else {
            self.state.select(Some(index));
        }
    }

    pub fn pending_accept(&mut self) {
        self.action_pending = Some(ProposalAction::Accept);
    }

    pub fn pending_reject(&mut self) {
        self.action_pending = Some(ProposalAction::Reject);
    }

    pub fn pending_skip(&mut self) {
        self.action_pending = Some(ProposalAction::Skip);
    }

    fn render_surface<S: RenderSurface>(&mut self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let area = super::centered_rect(72, 18, area);
        surface.render_widget(Clear, area);

        let block = Block::default()
            .title(" Skill Proposals ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border));

        let inner = block.inner(area);
        surface.render_widget(block, area);

        let hint_height: u16 = 1;
        let status_height: u16 = if self.status_message.is_some() { 1 } else { 0 };
        let list_height = inner.height.saturating_sub(status_height + hint_height);

        let mut constraints = vec![Constraint::Length(list_height)];
        if status_height > 0 {
            constraints.push(Constraint::Length(status_height));
        }
        constraints.push(Constraint::Length(hint_height));

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        // Proposal list
        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| {
                let primary =
                    Line::from(Span::styled(&item.title, Style::default().fg(theme.text)));
                let secondary = Line::from(vec![
                    Span::styled(
                        &item.kind_label,
                        Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::DIM),
                    ),
                    Span::styled(
                        format!("  {}", item.skill_name),
                        Style::default().fg(theme.text_muted),
                    ),
                ]);
                let third = Line::from(Span::styled(
                    format!("  {}", item.first_change),
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::DIM),
                ));
                ListItem::new(vec![primary, secondary, third])
            })
            .collect();

        let list = List::new(list_items)
            .block(Block::default())
            .highlight_style(
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        surface.render_stateful_widget(list, rows[0], &mut self.state);

        // Status message
        let hint_idx = if status_height > 0 { 2 } else { 1 };
        if let Some(ref msg) = self.status_message {
            let status = Paragraph::new(msg.clone()).style(Style::default().fg(theme.primary));
            surface.render_widget(status, rows[1]);
        }

        // Hint bar
        let hint = Line::from(vec![
            Span::styled(
                "A",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("ccept  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                "R",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("eject  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                "S",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("kip  ", Style::default().fg(theme.text_muted)),
            Span::styled("↑↓/Esc", Style::default().fg(theme.text_muted)),
        ]);
        surface.render_widget(Paragraph::new(hint), rows[hint_idx]);
    }

}

impl Component for SkillProposalReviewDialog {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let mut surface = BufferSurface::new(buffer);
        let mut dialog = self.clone();
        dialog.render_surface(&mut surface, area, &theme);
    }
}
