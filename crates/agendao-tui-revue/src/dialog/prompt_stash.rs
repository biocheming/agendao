//! 金 — Prompt stash: save/load prompt drafts.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct StashEntry {
    pub text: String,
    pub created_at: i64,
}

pub struct StashDialog {
    pub visible: bool,
    pub entries: Vec<StashEntry>,
    pub selected: usize,
}

impl StashDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: vec![], selected: 0 }
    }

    pub fn open(&mut self) { self.visible = true; self.selected = 0; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn set_entries(&mut self, entries: Vec<StashEntry>) {
        self.entries = entries;
        self.selected = 0;
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<String> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => {
                let max = self.entries.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
                None
            }
            Key::Enter => {
                let text = self.entries.get(self.selected).map(|e| e.text.clone());
                self.close();
                text
            }
            Key::Delete | Key::Char('d') => {
                if self.selected < self.entries.len() {
                    self.entries.remove(self.selected);
                    if self.selected >= self.entries.len().saturating_sub(1) {
                        self.selected = self.entries.len().saturating_sub(1);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        if self.entries.is_empty() {
            let content = vstack().child(Text::new("(empty stash)").fg(colors::FG_MUTED));
            backdrop::render_dialog("Prompt Stash", colors::ACCENT_PURPLE, content,
                "Esc: close", ctx, 60, 5);
            return;
        }

        let items: Vec<ListItem> = self.entries.iter().enumerate().take(10).map(|(i, entry)| {
            let preview: String = entry.text.chars().take(60).collect();
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{}", marker, preview),
                muted: false,
            }
        }).collect();

        backdrop::render_list_dialog(
            "Prompt Stash",
            colors::ACCENT_PURPLE,
            &items,
            self.selected,
            "↑↓ navigate  Enter: restore  d: delete  Esc: close",
            ctx, 60, 10,
        );
    }
}
