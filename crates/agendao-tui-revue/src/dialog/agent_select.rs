//! 金 — Agent selection dialog.
//!
//! 内联 Up/Down/Enter/Esc 导航（不再走 ListDialogState 抽象层）：
//! 单用户的"抽象"不是抽象——ModelSelect/SessionList 各有 FlatRow/
//! filtered_indices 的领域形态，根本接不进 `items: Vec<T>` 契约。
//! 把 8 行 match 留在领域内，导航语义即金律自身：每个 dialog 的
//! `Outcome` 是真正的成形权威。

use crate::dialog::backdrop::{self, ListItem};
use crate::theme::colors;
use revue::event::Key;
use revue::prelude::*;

#[derive(Clone)]
pub struct AgentEntry {
    pub name: String,
    pub display: String,
    pub description: String,
}

pub struct AgentSelectDialog {
    pub visible: bool,
    agents: Vec<AgentEntry>,
    selected: usize,
    current: Option<String>,
}

impl Default for AgentSelectDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentSelectDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            agents: Vec::new(),
            selected: 0,
            current: None,
        }
    }

    pub fn set_agents(&mut self, agents: Vec<AgentEntry>) {
        self.agents = agents;
        self.selected = 0;
    }

    pub fn set_current(&mut self, current: Option<String>) {
        self.current = current.filter(|value| !value.trim().is_empty());
    }

    pub fn open(&mut self) {
        self.visible = true;
        if let Some(current) = self.current.as_deref() {
            if let Some(index) = self.agents.iter().position(|agent| agent.name == current) {
                self.selected = index;
            }
        }
    }
    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<AgentEntry> {
        if !self.visible {
            return None;
        }
        if self.agents.is_empty() {
            if matches!(key, Key::Escape) {
                self.close();
            }
            return None;
        }
        let len = self.agents.len();
        match key {
            Key::Up => {
                self.selected = (self.selected + len - 1) % len;
                None
            }
            Key::Down => {
                self.selected = (self.selected + 1) % len;
                None
            }
            Key::Home => {
                self.selected = 0;
                None
            }
            Key::End => {
                self.selected = len - 1;
                None
            }
            Key::Enter => {
                let pick = self.agents.get(self.selected).cloned();
                self.close();
                pick
            }
            Key::Escape => {
                self.close();
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible {
            return;
        }
        // U16：空态明示（原渲染零行空框 + 导航 hint——无法导航的死端）。
        if self.agents.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No agents configured)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: "Select Agent",
                    border_color: colors::ACCENT_PURPLE(),
                },
                &items,
                0,
                "Esc: close",
                ctx,
                geom,
                3,
            );
            return;
        }
        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        let items: Vec<ListItem> = self
            .agents
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let marker = if i == self.selected { "▶ " } else { "  " };
                let current = if self.current.as_deref() == Some(a.name.as_str()) {
                    "  [current]"
                } else {
                    ""
                };
                ListItem::Row {
                    display: format!("{}{} — {}{}", marker, a.display, a.description, current),
                    muted: false,
                }
            })
            .collect();
        backdrop::render_list_dialog_bottom(
            backdrop::ListDialogHeading {
                title: "Select Agent",
                border_color: colors::ACCENT_PURPLE(),
            },
            &items,
            self.selected,
            "↑↓ navigate  Home/End: jump  Enter: select  Esc: close",
            ctx,
            geom,
            12,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_agent_is_selected_on_open() {
        let mut dialog = AgentSelectDialog::new();
        dialog.set_agents(vec![
            AgentEntry {
                name: "plan".into(),
                display: "Plan".into(),
                description: String::new(),
            },
            AgentEntry {
                name: "build".into(),
                display: "Build".into(),
                description: String::new(),
            },
        ]);
        dialog.set_current(Some("build".into()));
        dialog.open();

        assert_eq!(dialog.selected, 1);
        assert_eq!(dialog.current.as_deref(), Some("build"));
    }
}
