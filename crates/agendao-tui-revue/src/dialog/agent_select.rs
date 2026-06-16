//! 金 — Agent selection dialog.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};
use crate::widget::list_dialog::{ListAction, ListDialogState, key_name};

#[derive(Clone)]
pub struct AgentEntry {
    pub name: String, pub display: String, pub description: String,
}

pub struct AgentSelectDialog {
    pub visible: bool,
    /// 选择导航 + 输入归一到 [`ListDialogState`]（土/木律单点），
    /// 取代原先散落的 `agents` + `selected` + 手写 Up/Down 钳位边界。
    list: ListDialogState<AgentEntry>,
}

impl AgentSelectDialog {
    pub fn new() -> Self {
        Self { visible: false, list: ListDialogState::new(vec![]) }
    }

    pub fn set_agents(&mut self, agents: Vec<AgentEntry>) {
        self.list = ListDialogState::new(agents);
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> Option<AgentEntry> {
        if !self.visible { return None; }
        // 导航/确认/取消语义全交状态机；本 dialog 只负责把
        // ListAction 映射回 AgentEntry（领域成形，金律）。
        match self.list.handle(&key_name(key)) {
            ListAction::Confirm(i) => {
                let a = self.list.items.get(i).cloned();
                self.close();
                a
            }
            ListAction::Cancel => { self.close(); None }
            ListAction::None => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let items: Vec<ListItem> = self.list.items.iter().enumerate().take(12).map(|(i, a)| {
            let marker = if i == self.list.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{} — {}", marker, a.display, a.description),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Select Agent",
            colors::ACCENT_PURPLE,
            &items,
            self.list.selected,
            "↑↓ navigate  Enter: select  Esc: close",
            ctx, 56, 12,
        );
    }
}
