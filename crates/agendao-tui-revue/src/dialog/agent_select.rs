//! 金 — Agent selection dialog.
//!
//! Old TUI: ratatui list with colored agent names.
//! New: Revue Border::rounded() + vstack() + Text::new().

use revue::prelude::*;
use revue::event::Key;

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
        let mut content = vstack().gap(0);
        for (i, agent) in self.agents.iter().enumerate().take(12) {
            let marker = if i == self.selected { "▶" } else { " " };
            let color = if i == self.selected { Color::rgb(125, 207, 255) } else { Color::rgb(169, 177, 214) };
            let line = format!("{} {} — {}", marker, agent.display, agent.description);
            content = content.child(Text::new(line).fg(color));
        }
        let dialog = Border::rounded().title(" Select Agent ").fg(Color::rgb(187, 154, 247)).child(content);
        let w = 52u16.min(ctx.area.width - 4);
        let h = (self.agents.len().min(12) as u16 + 3).min(ctx.area.height - 4);
        let x = (ctx.area.width - w) / 2; let y = (ctx.area.height - h) / 2;
        revue::widget::positioned(dialog).x(x as i16).y(y as i16).width(w).height(h).render(ctx);
    }
}
