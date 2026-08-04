//! 金 — Execution mode selection dialog.
//!
//! Web 端 (`apps/agendao-web/src/App.tsx:836`) 把 `selectedMode` 存
//! `"kind:id"` 字符串，发送时按 kind 分流到 `agent` / `scheduler_profile`
//! 槽。TUI 对齐此契约：`store.selected_mode` 同样存 `"kind:id"`，
//! [`ModeSelectDialog`] 负责让用户在 `ExecutionModeInfo` 列表里选一项，
//! dispatch 处（keymap.rs）按 kind 分发。
//!
//! 模式无分组、无 variant、无可用性判断（server 端 `list_execution_modes`
//! 已过滤 hidden）——比 [`ModelSelectDialog`] 简单，结构贴近
//! [`AgentSelectDialog`]：内联 Up/Down/Home/End/Enter/Esc 导航。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone, Debug, PartialEq)]
pub struct ModeEntry {
    /// "agent" | "preset" | "profile" —— 对齐 `ExecutionModeInfo.kind`，
    /// dispatch 处按此分发到 agent 或 scheduler_profile 槽。
    pub kind: String,
    /// 实际下发 id（如 "sisyphus" / "reviewer"）。
    pub id: String,
    /// 列表显示用名（可读名，来自 `ExecutionModeInfo.name`）。
    pub display: String,
    pub description: Option<String>,
}

impl ModeEntry {
    /// store 契约：`"kind:id"` 字符串（web `App.tsx:836` 即按 `split(":", 2)` 解析）。
    pub fn composite(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
}

pub struct ModeSelectDialog {
    pub visible: bool,
    entries: Vec<ModeEntry>,
    selected: usize,
}

impl Default for ModeSelectDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeSelectDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: Vec::new(), selected: 0 }
    }

    pub fn open_with(&mut self, entries: Vec<ModeEntry>) {
        self.entries = entries;
        self.selected = 0;
        self.visible = true;
    }

    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<ModeEntry> {
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
        // 注意:不要 .take(14) —— backdrop::render_positioned_list 已经实现 sliding
        // viewport(selected 出窗自动滚动),先 take(14) 会让 backdrop 看不到超出条目,
        // 导致 ↑/↓ 选到第 15 条时视野不跟随(金律违例:截断了唯一成形语法的输入)。
        let items: Vec<ListItem> = self.entries.iter().enumerate().map(|(i, e)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            // kind 在前缀里短标注，便于用户一眼分清 agent / preset / profile。
            let kind_tag = match e.kind.as_str() {
                "agent"   => "agent  ",
                "preset"  => "preset ",
                "profile" => "profile",
                _         => "mode   ",
            };
            let desc = e.description.as_deref().unwrap_or("");
            let display = if desc.is_empty() {
                format!("{}[{}] {}", marker, kind_tag, e.display)
            } else {
                format!("{}[{}] {} — {}", marker, kind_tag, e.display, desc)
            };
            ListItem::Row { display, muted: false }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Select Mode",
            colors::ACCENT_PURPLE(),
            &items,
            self.selected,
            "↑↓ navigate  Home/End: jump  Enter: select  Esc: close",
            ctx, geom, 14,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, id: &str) -> ModeEntry {
        ModeEntry {
            kind: kind.into(), id: id.into(),
            display: id.into(), description: None,
        }
    }

    #[test]
    fn composite_formats_kind_id() {
        assert_eq!(entry("preset", "sisyphus").composite(), "preset:sisyphus");
        assert_eq!(entry("agent", "reviewer").composite(), "agent:reviewer");
    }

    #[test]
    fn enter_returns_selected_and_closes() {
        let mut d = ModeSelectDialog::new();
        d.open_with(vec![entry("preset", "a"), entry("agent", "b")]);
        let _ = d.handle_key(&Key::Down);
        let picked = d.handle_key(&Key::Enter).unwrap();
        assert_eq!(picked.composite(), "agent:b");
        assert!(!d.is_open());
    }

    #[test]
    fn down_wraps_around() {
        let mut d = ModeSelectDialog::new();
        d.open_with(vec![entry("preset", "a"), entry("preset", "b")]);
        let _ = d.handle_key(&Key::Down);
        let _ = d.handle_key(&Key::Down); // wrap back to 0
        let picked = d.handle_key(&Key::Enter).unwrap();
        assert_eq!(picked.id, "a");
    }

    #[test]
    fn esc_closes_without_selection() {
        let mut d = ModeSelectDialog::new();
        d.open_with(vec![entry("preset", "a")]);
        assert_eq!(d.handle_key(&Key::Escape), None);
        assert!(!d.is_open());
    }

    #[test]
    fn empty_entries_only_esc() {
        let mut d = ModeSelectDialog::new();
        d.open_with(vec![]);
        assert_eq!(d.handle_key(&Key::Down), None);
        assert!(d.is_open());
        assert_eq!(d.handle_key(&Key::Escape), None);
        assert!(!d.is_open());
    }
}
