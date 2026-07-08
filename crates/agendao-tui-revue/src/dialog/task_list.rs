//! 金 — Global agent task list dialog。
//!
//! `/task` 全局注册表（非 per-session）。写操作闭环（B 层第三批）：
//! c=cancel 走 confirm 类——关 list → 开 ConfirmDialog → 确认后 Panel::None
//! （cancel 影响运行中任务，需二次确认）。下次 OpenTasks 重拉回流。
//! 无 remove_by_id：cancel 后 dialog 已 close，就地移除无 runtime read path
//! （道纪第十条——避免伪权威）。Enter=View 关闭。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct TaskEntry {
    pub id: String,
    pub agent_name: String,
    pub status: String,
    pub step: Option<u32>,
    pub max_steps: Option<u32>,
}

/// 列表按键动作（per-dialog action enum）。cancel 走 confirm 类。
pub enum TaskAction {
    Cancel(TaskEntry),
    View(TaskEntry),
}

pub struct TaskListDialog {
    pub visible: bool,
    entries: Vec<TaskEntry>,
    selected: usize,
}

impl TaskListDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: Vec::new(), selected: 0 }
    }

    pub fn set_entries(&mut self, entries: Vec<TaskEntry>) {
        self.entries = entries;
        self.selected = 0;
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    /// c=cancel（不 close——panel_dispatch 关 list + 开 confirm）/ Enter=view（关闭）。
    pub fn handle_key(&mut self, key: &Key) -> Option<TaskAction> {
        if !self.visible { return None; }
        if self.entries.is_empty() {
            if matches!(key, Key::Escape) { self.close(); }
            return None;
        }
        let len = self.entries.len();
        let pick = || self.entries.get(self.selected).cloned();
        match key {
            Key::Up    => { self.selected = (self.selected + len - 1) % len; None }
            Key::Down  => { self.selected = (self.selected + 1) % len; None }
            Key::Home  => { self.selected = 0; None }
            Key::End   => { self.selected = len - 1; None }
            Key::Enter => {
                let pick = pick();
                self.close();
                pick.map(TaskAction::View)
            }
            Key::Char('c') => pick().map(TaskAction::Cancel),
            Key::Escape => { self.close(); None }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }
        if self.entries.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No active agent tasks)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "Tasks",
                colors::ACCENT_GREEN,
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        let items: Vec<ListItem> = self.entries.iter().enumerate().map(|(i, t)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            let step_str = match (t.step, t.max_steps) {
                (Some(s), Some(m)) => format!("{}/{}", s, m),
                (Some(s), None) => s.to_string(),
                _ => "-".to_string(),
            };
            ListItem::Row {
                display: format!("{}[{}] {} ({} · step {})", marker, t.status, t.agent_name, t.id, step_str),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Tasks",
            colors::ACCENT_GREEN,
            &items,
            self.selected,
            "↑↓ navigate  c: cancel  Enter: view  Esc: close",
            ctx, geom, 12,
        );
    }
}
