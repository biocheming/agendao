//! 金 — Agent selection dialog.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct AgentEntry {
    pub name: String, pub display: String, pub description: String,
}

pub struct AgentSelectDialog {
    pub visible: bool,
    agents: Vec<AgentEntry>,
    selected: usize,
}

impl AgentSelectDialog {
    pub fn new() -> Self {
        Self { visible: false, agents: vec![], selected: 0 }
    }

    pub fn set_agents(&mut self, agents: Vec<AgentEntry>) { self.agents = agents; self.selected = 0; }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> Option<AgentEntry> {
        if !self.visible { return None; }
        match key {
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => { let max = self.agents.len().saturating_sub(1); self.selected = (self.selected + 1).min(max); None }
            Key::Enter => { let a = self.agents.get(self.selected).cloned(); self.close(); a }
            Key::Escape => { self.close(); None }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let items: Vec<ListItem> = self.agents.iter().enumerate().take(12).map(|(i, a)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{} — {}", marker, a.display, a.description),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog(
            "Select Agent",
            colors::ACCENT_PURPLE,
            &items,
            self.selected,
            "↑↓ navigate  Enter: select  Esc: close",
            ctx, 56, 12,
        );
    }
}
