//! 金 — Session recovery list dialog（Part 3, B 层第二批）。
//!
//! per-session 视图（需 active session_id）。把 `SessionRecoveryProtocol`
//! 的 actions + checkpoints 映射成行。读视图 first slice（道纪第十条）：
//! 列表权威已成，写端（execute recovery action 需 confirm +
//! execute_session_recovery）留 B 层第三批。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct RecoveryEntry {
    pub label: String,
    pub detail: String,
}

pub struct RecoveryListDialog {
    pub visible: bool,
    entries: Vec<RecoveryEntry>,
    selected: usize,
}

impl RecoveryListDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: Vec::new(), selected: 0 }
    }

    pub fn set_entries(&mut self, entries: Vec<RecoveryEntry>) {
        self.entries = entries;
        self.selected = 0;
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<RecoveryEntry> {
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
                display: "  (No recovery actions or checkpoints)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "Recovery",
                colors::ACCENT_YELLOW,
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        let items: Vec<ListItem> = self.entries.iter().enumerate().take(16).map(|(i, e)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{} — {}", marker, e.label, e.detail),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Recovery",
            colors::ACCENT_YELLOW,
            &items,
            self.selected,
            "↑↓ navigate  Enter: select (read-only)  Esc: close",
            ctx, geom, 16,
        );
    }
}
