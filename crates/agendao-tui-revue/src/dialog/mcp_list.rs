//! 金 — MCP server status list dialog（Part 2, B 层第二批）。
//!
//! 与 `/skills` 同范式。读视图 first slice（道纪第十条）：列表权威已成，
//! 写端（connect/disconnect/restart MCP 需独立 dialog + API）留后续。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct McpEntry {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
}

pub struct McpListDialog {
    pub visible: bool,
    entries: Vec<McpEntry>,
    selected: usize,
}

impl McpListDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: Vec::new(), selected: 0 }
    }

    pub fn set_entries(&mut self, entries: Vec<McpEntry>) {
        self.entries = entries;
        self.selected = 0;
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<McpEntry> {
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
                display: "  (No MCP servers configured)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "MCP Servers",
                colors::ACCENT_CYAN,
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        let items: Vec<ListItem> = self.entries.iter().enumerate().take(12).map(|(i, m)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}[{}] {} · tools:{} res:{}", marker, m.status, m.name, m.tools, m.resources),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "MCP Servers",
            colors::ACCENT_CYAN,
            &items,
            self.selected,
            "↑↓ navigate  Enter: select (read-only)  Esc: close",
            ctx, geom, 12,
        );
    }
}
