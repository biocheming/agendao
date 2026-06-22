//! 金 — Global agent task list dialog（Part 4, B 层第二批）。
//!
//! `/task` 全局注册表（非 per-session）。读视图 first slice（道纪第十条）：
//! 列表权威已成，写端（cancel_task 需 confirm + DELETE）留 B 层第三批。

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

    pub fn handle_key(&mut self, key: &Key) -> Option<TaskEntry> {
        if !self.visible { return None; }
        if self.entries.is_empty() {
            if matches!(key, Key::Escape) { self.close(); }
            return None;
        }
        let len = self.entries.len();
        match key {
            Key::Up    => { self.selected = (self.selected + len - 1) % len; None }
            Key::Down  => { self.selected = (self.selected + 1) % len; None }
            Key::Home  => { self.selected = 0; None }
            Key::End   => { self.selected = len - 1; None }
            Key::Enter => {
                let pick = self.entries.get(self.selected).cloned();
                self.close();
                pick
            }
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
        let items: Vec<ListItem> = self.entries.iter().enumerate().take(12).map(|(i, t)| {
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
            "↑↓ navigate  Enter: select (read-only)  Esc: close",
            ctx, geom, 12,
        );
    }
}
