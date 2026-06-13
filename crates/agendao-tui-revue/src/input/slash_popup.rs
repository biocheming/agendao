//! 木 — Slash command popup: / triggered command palette.
//!
//! Old TUI: agendao_command::CommandRegistry + fuzzy_match, ratatui Paragraph.
//! New: Revue Border::rounded() + vstack() + Text::new() declarative layout.
//! TODO: integrate agendao_command::CommandRegistry for dynamic commands.

use revue::prelude::*;
use revue::event::Key;

pub struct SlashPopup {
    pub visible: bool,
    pub query: String,
    pub selected: usize,
    commands: Vec<SlashItem>,
}

#[derive(Clone)]
pub struct SlashItem { pub name: String, pub desc: String, pub action: SlashAction }

#[derive(Clone, Debug)]
pub enum SlashAction {
    Help, Clear, ModelSelect, AgentSelect, ThemeSelect,
    SessionList, SessionNew, SessionExport, SessionRename,
    ToggleSidebar, ShowStatus, Quit, Custom(String),
}

impl SlashPopup {
    pub fn new() -> Self {
        Self { visible: false, query: String::new(), selected: 0, commands: Self::default_commands() }
    }

    fn default_commands() -> Vec<SlashItem> { vec![
        SlashItem { name: "help".into(), desc: "Show keybindings".into(), action: SlashAction::Help },
        SlashItem { name: "clear".into(), desc: "Clear input".into(), action: SlashAction::Clear },
        SlashItem { name: "model".into(), desc: "Select model".into(), action: SlashAction::ModelSelect },
        SlashItem { name: "agent".into(), desc: "Select agent".into(), action: SlashAction::AgentSelect },
        SlashItem { name: "theme".into(), desc: "Select theme".into(), action: SlashAction::ThemeSelect },
        SlashItem { name: "session list".into(), desc: "List sessions".into(), action: SlashAction::SessionList },
        SlashItem { name: "session new".into(), desc: "New session".into(), action: SlashAction::SessionNew },
        SlashItem { name: "session export".into(), desc: "Export session".into(), action: SlashAction::SessionExport },
        SlashItem { name: "session rename".into(), desc: "Rename session".into(), action: SlashAction::SessionRename },
        SlashItem { name: "sidebar".into(), desc: "Toggle sidebar".into(), action: SlashAction::ToggleSidebar },
        SlashItem { name: "status".into(), desc: "View status".into(), action: SlashAction::ShowStatus },
        SlashItem { name: "quit".into(), desc: "Exit agendao".into(), action: SlashAction::Quit },
    ]}

    pub fn open(&mut self) { self.visible = true; self.selected = 0; self.query.clear(); }
    pub fn close(&mut self) { self.visible = false; self.query.clear(); }

    pub fn handle_key(&mut self, key: &Key) -> Option<SlashAction> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Enter => {
                let action = self.filtered().get(self.selected).map(|c| c.action.clone());
                if action.is_some() { self.close(); }
                action
            },
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => { let max = self.filtered().len().saturating_sub(1); self.selected = (self.selected + 1).min(max); None }
            Key::Backspace => { self.query.pop(); self.selected = 0; None }
            Key::Char(c) if c.is_alphanumeric() || *c == ' ' => { self.query.push(*c); self.selected = 0; None }
            _ => None,
        }
    }

    fn filtered(&self) -> Vec<&SlashItem> {
        let q = self.query.to_lowercase();
        if q.is_empty() { self.commands.iter().collect() }
        else { self.commands.iter().filter(|c| c.name.to_lowercase().contains(&q)).collect() }
    }

    /// Render popup above the prompt via Revue positioned Border + vstack.
    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let cmds = self.filtered();
        let mut content = vstack().gap(0);
        for (i, cmd) in cmds.iter().enumerate().take(8) {
            let marker = if i == self.selected { "▶" } else { " " };
            let color = if i == self.selected { Color::rgb(125, 207, 255) } else { Color::rgb(169, 177, 214) };
            content = content.child(Text::new(format!("{} /{}  {}", marker, cmd.name, cmd.desc)).fg(color));
        }

        let dialog = Border::rounded()
            .title(format!(" /{} ", self.query))
            .fg(Color::rgb(125, 207, 255))
            .child(content);

        let w = 44u16.min(ctx.area.width - 4);
        let h = (cmds.len().min(8) as u16 + 3).min(ctx.area.height - 4);
        let x = 2u16;
        let y = ctx.area.height.saturating_sub(h + 3);
        revue::widget::positioned(dialog).x(x as i16).y(y as i16).width(w).height(h).render(ctx);
    }
}
