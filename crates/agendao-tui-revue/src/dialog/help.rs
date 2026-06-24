//! 金 — Help dialog with keybindings.
//!
//! 真相唯一(土律):此处的 (key, desc) 表是所有快捷键的「文档权威」。
//! 新增 keymap.rs 分支时,同步追加一条 (key, desc) ——否则用户按 '?'
//! 看不到新键,等同于功能未交付(第十条:可观测性权利)。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct HelpDialog {
    pub visible: bool,
}

/// Help entry kinds — Section header(组标题)or Binding(键+描述)。
/// Help 已经从 11 条单层列表扩到分组形,Section 头分隔,与
/// dialog::backdrop ListItem 同思想(单 enum 多变体复用渲染框)。
enum HelpEntry {
    Section(&'static str),
    Binding(&'static str, &'static str),
}

impl HelpDialog {
    pub fn new() -> Self { Self { visible: false } }
    pub fn toggle(&mut self) { self.visible = !self.visible; }
    pub fn dismiss(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible { return false; }
        match key {
            Key::Escape | Key::Char('q') | Key::Char('h') | Key::Char('?') => { self.dismiss(); true }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }

        use HelpEntry::*;
        let entries: &[HelpEntry] = &[
            Section("─ Composer ─"),
            Binding("Enter", "Send prompt"),
            Binding("↑/↓", "Prompt history"),
            Binding("Tab", "Autocomplete / next foldable block"),
            Binding("Ctrl+B", "Toggle sidebar"),
            Binding("Ctrl+P", "Command palette"),

            Section("─ Transcript (cursor) ─"),
            Binding("Tab", "Next foldable block (cursor)"),
            Binding("Space", "Toggle fold at cursor (prompt empty)"),
            Binding("e", "Edit & resend cursor UserPrompt"),
            Binding("c", "Copy cursor block to clipboard (OSC52)"),
            Binding("PgUp/PgDn", "Scroll transcript by page"),

            Section("─ Sessions list (/sessions) ─"),
            Binding("type", "Filter sessions"),
            Binding("↑/↓", "Navigate"),
            Binding("Enter", "Open selected"),
            Binding("x", "Mark/unmark cursor row"),
            Binding("D", "Delete all marked sessions (Confirm)"),
            Binding("Esc", "Close"),

            Section("─ Slash commands ─"),
            Binding("/help", "Open this help"),
            Binding("/models", "Switch model"),
            Binding("/agents", "Switch agent"),
            Binding("/sessions", "Browse sessions"),
            Binding("/copy", "Copy full transcript (OSC52)"),
            Binding("/revise", "Revise & resend a previous prompt"),
            Binding("/fork", "Fork current session"),
            Binding("/mode", "Switch composer mode/profile"),
            Binding("/themes", "Cycle theme"),

            Section("─ Global ─"),
            Binding("Esc/q", "Quit"),
            Binding("h", "Home screen"),
            Binding("?", "Toggle help"),
            Binding("Ctrl+C", "Force quit"),
        ];

        let mut content = vstack().gap(0);
        for entry in entries {
            content = match entry {
                HelpEntry::Section(title) => content.child(
                    Text::new(*title).fg(colors::ACCENT_BLUE),
                ),
                HelpEntry::Binding(key, desc) => content.child(
                    hstack().gap(2)
                        .child(Text::new(format!("{:>12}", key)).fg(colors::ACCENT_CYAN))
                        .child(Text::new(*desc).fg(colors::FG_SECONDARY)),
                ),
            };
        }

        backdrop::render_dialog_bottom(
            "Help — Keybindings",
            colors::ACCENT_BLUE,
            content,
            "Esc/q/h/? to close",
            ctx, geom, entries.len() as u16 + 4,
        );
    }
}
