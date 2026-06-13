//! 金 — Todo/Task list rendering.
//!
//! Old TUI: ratatui Paragraph + Span::styled() for icon+text (~40 lines).
//! New: Revue vstack() + Text::new() declarative layout via CSS classes.

use revue::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum TodoStatus { Pending, InProgress, Completed, Cancelled }

#[derive(Clone)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

impl TodoStatus {
    pub fn icon(&self) -> &'static str { match self {
        Self::Pending => "◯", Self::InProgress => "◐", Self::Completed => "●", Self::Cancelled => "◯",
    }}
    pub fn color(&self) -> Color { match self {
        Self::Pending => Color::rgb(128, 128, 128),
        Self::InProgress => Color::rgb(224, 175, 104),
        Self::Completed => Color::rgb(158, 206, 106),
        Self::Cancelled => Color::rgb(86, 95, 137),
    }}
}

/// Render a list of todo items using Revue vstack.
pub fn render_todo_list(items: &[TodoItem]) -> revue::widget::Stack {
    let mut stack = vstack().gap(0);
    for item in items {
        let icon = item.status.icon();
        let line = format!("{} {}", icon, item.content);
        stack = stack.child(Text::new(line).fg(item.status.color()));
    }
    stack
}

/// Render in sidebar panel format with title.
pub fn render_todo_panel(items: &[TodoItem]) -> revue::widget::Stack {
    if items.is_empty() {
        return vstack().child(Text::new("(none)").fg(Color::rgb(86, 95, 137)));
    }
    let mut s = vstack().gap(0);
    for item in items {
        s = s.child(Text::new(format!("{} {}", item.status.icon(), item.content)).fg(item.status.color()));
    }
    s
}
