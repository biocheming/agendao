//! 金 — MCP server status list dialog。
//!
//! 与 `/skills` 同范式。写操作闭环（B 层第三批）：c=connect / d=disconnect
//! 直接执行（前置 status 校验——已 connected 不重复 connect，未 connected
//! 不 disconnect），Ok 后重拉 get_mcp_status 回流（status 字段变化非移除，
//! 重拉是唯一权威——水生木）。dialog 保持打开支持批量。Enter=View 关闭。

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

/// 列表按键动作（per-dialog action enum）。
pub enum McpAction {
    Connect(McpEntry),
    Disconnect(McpEntry),
    View(McpEntry),
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

    /// c=connect / d=disconnect（保持 dialog 打开，支持批量）/ Enter=view（关闭）。
    pub fn handle_key(&mut self, key: &Key) -> Option<McpAction> {
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
                pick.map(McpAction::View)
            }
            Key::Char('c') => pick().map(McpAction::Connect),
            Key::Char('d') => pick().map(McpAction::Disconnect),
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
            "↑↓ navigate  c: connect  d: disconnect  Enter: view  Esc: close",
            ctx, geom, 12,
        );
    }
}
