//! 金 — Help dialog with keybindings.
//!
//! 真相唯一(土律):此处的 (key, desc) 表是所有快捷键的「文档权威」。
//! 新增 keymap.rs 分支时,同步追加一条 (key, desc) ——否则用户按 '?'
//! 看不到新键,等同于功能未交付(第十条:可观测性权利)。
//!
//! U19:slash 命令段与 `agendao-command` CommandRegistry 一一对应——
//! tests::help_covers_every_registry_slash_command 双向守住(表外无命令、
//! 命令外无表行),新增 slash  command 不在表内即测试红。弹窗热键各段的
//! 权威仍是各 dialog 的 footer hint(U14 的 source-scan 测试守 hint↔键表
//! 一致),本表是其汇总视图。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

pub struct HelpDialog {
    pub visible: bool,
    /// U19:表扩到 ~100 行后必须滚动——scroll 即 sliding viewport 的
    /// "选中行"(与 skill detail 同模式,复用 render_list_dialog_bottom)。
    scroll: usize,
}

/// Help entry kinds — Section header(组标题)or Binding(键+描述)。
/// Help 已经从 11 条单层列表扩到分组形,Section 头分隔,与
/// dialog::backdrop ListItem 同思想(单 enum 多变体复用渲染框)。
pub enum HelpEntry {
    Section(&'static str),
    Binding(&'static str, &'static str),
}

/// 全部快捷键的**文档权威**(土律·第四条·单点权威)。`?` Help dialog 与
/// Settings→Keybindings 分类都读这一份;新增 keymap 分支时只在此追加一条,
/// 两处展示自动同步(避免"改一处漏一处"的双份真相)。
pub const KEYBINDINGS: &[HelpEntry] = {
    use HelpEntry::*;
    &[
        Section("─ Composer ─"),
        Binding("Enter", "Send prompt"),
        Binding("↑/↓", "Prompt history"),
        Binding("Ctrl+Z / Ctrl+Y", "Undo / redo prompt text"),
        Binding("Tab", "Autocomplete / next foldable block"),
        Binding("Ctrl+B", "Toggle sidebar"),
        Binding("Ctrl+P", "Command palette"),

        Section("─ Transcript (cursor) ─"),
        Binding("j/k · Tab/S-Tab", "Cursor next/prev foldable block"),
        Binding("Space", "Toggle fold at cursor (prompt empty)"),
        Binding("e", "Edit & resend cursor UserPrompt"),
        Binding("c", "Copy cursor block (OSC52)"),
        Binding("C", "Copy visible screen (OSC52)"),
        Binding("g/G · Home/End", "Jump top/bottom"),
        Binding("PgUp/PgDn · wheel", "Scroll transcript"),

        Section("─ Sessions (/sessions) ─"),
        Binding("type · ⌫", "Filter sessions"),
        Binding("↑/↓ · Home/End", "Navigate"),
        Binding("Enter", "Open selected"),
        Binding("x · D", "Mark row · delete all marked (Confirm)"),
        Binding("n", "New session (empty list)"),
        Binding("Esc", "Close"),

        Section("─ Pickers (/models · /mode · /agents) ─"),
        Binding("type · ⌫", "Filter (models)"),
        Binding("Tab", "Cycle variant (models)"),
        Binding("↑/↓ · Home/End", "Navigate"),
        Binding("Enter", "Select"),
        Binding("Esc", "Close"),

        Section("─ Skills (/skills · /proposals) ─"),
        Binding("type · ⌫", "Filter skills"),
        Binding("Enter", "Detail view (Esc: back)"),
        Binding("a/r", "Approve/reject (proposals)"),
        Binding("s", "Open settings (empty list)"),
        Binding("Esc", "Close"),

        Section("─ MCP & tasks (/mcps · /tasks) ─"),
        Binding("c/d", "Connect/disconnect (mcps) · cancel (tasks)"),
        Binding("a/A", "OAuth start/finish (mcps)"),
        Binding("x", "Clear auth (mcps) · execute (tasks)"),
        Binding("n/e", "Add/edit server (mcps)"),
        Binding("Enter", "View"),

        Section("─ Upkeep (/stash · /fork · /recover · /notifications) ─"),
        Binding("Enter", "Restore (stash) · fork (fork) · view"),
        Binding("d", "Delete (stash)"),
        Binding("x", "Execute action (recover)"),
        Binding("Esc", "Close/cancel"),

        Section("─ Permission prompt ─"),
        Binding("↑/↓ · Enter", "Navigate · confirm option"),
        Binding("y/a · 1-3", "Quick allow"),
        Binding("n/d · 0", "Deny"),
        Binding("Esc", "Hide"),

        Section("─ Editors (provider · model · mcp · plugin · rename) ─"),
        Binding("Tab", "Next field"),
        Binding("←/→", "Cycle option (effort/transport/protocol)"),
        Binding("F2", "Show/hide API key (provider)"),
        Binding("Enter", "Save/install"),
        Binding("Esc", "Cancel"),

        Section("─ Settings (/settings) ─"),
        Binding("Tab", "Cycle panes / categories"),
        Binding("↑/↓", "Navigate rows"),
        Binding("Enter", "Enter category / toggle row"),
        Binding("a/e/d", "Add/Edit/Delete provider (Model Settings)"),
        Binding("m", "Add model to provider"),
        Binding("c/d", "Connect/Disconnect MCP server"),
        Binding("a/r", "Approve/Reject skill proposal"),

        // ── Slash commands:与 CommandRegistry 一一对应(测试双向守)。
        // server 端缺能力的条目诚实标注,不伪"已通"(道纪第十条)。
        Section("─ Slash commands ─"),
        Binding("/abort", "Abort the active run"),
        Binding("/agent", "Switch agent"),
        Binding("/command", "Command palette (= Ctrl+P)"),
        Binding("/compact", "Compact history (optional focus)"),
        Binding("/connect", "Connect a new provider"),
        Binding("/copy", "Copy full transcript"),
        Binding("/delete", "Delete current session permanently"),
        Binding("/density", "Toggle compact/cozy density"),
        Binding("/editor", "External editor (coming soon)"),
        Binding("/exit", "Quit (aliases: /quit /q)"),
        Binding("/export", "Export session as markdown"),
        Binding("/fork", "Fork from a message"),
        Binding("/header", "Toggle session header"),
        Binding("/help", "This help"),
        Binding("/highlight", "Semantic highlight (TUI: n/a yet)"),
        Binding("/mcps", "Manage MCP servers"),
        Binding("/mode", "Switch execution mode"),
        Binding("/models", "Switch model"),
        Binding("/new", "New session"),
        Binding("/notifications", "Notification history"),
        Binding("/parent", "Parent session (not supported yet)"),
        Binding("/preset", "Switch preset (opens /mode)"),
        Binding("/proposals", "Skill evolution proposals"),
        Binding("/recover", "Recovery actions"),
        Binding("/redo", "Redo (server stub; Ctrl+Y = text redo)"),
        Binding("/rename", "Rename session"),
        Binding("/revise", "Revise & resend prompt under cursor"),
        Binding("/scrollbar", "Toggle scrollbar"),
        Binding("/sessions", "Browse sessions"),
        Binding("/settings", "Settings screen"),
        Binding("/share", "Share session link"),
        Binding("/sidebar", "Toggle sidebar (= Ctrl+B)"),
        Binding("/skills", "Browse skills"),
        Binding("/stash", "Stash draft / browse stash"),
        Binding("/status", "System status"),
        Binding("/tasks", "Agent tasks"),
        Binding("/themes", "Cycle theme (dark/light via Ctrl+P)"),
        Binding("/thinking", "Toggle thinking blocks"),
        Binding("/timeline", "Timeline (not supported yet)"),
        Binding("/timestamps", "Toggle timestamps"),
        Binding("/tips.toggle", "Toggle home tips"),
        Binding("/undo", "Undo (server stub; Ctrl+Z = text undo)"),
        Binding("/unshare", "Revoke share link"),
        Binding("/voice", "Voice input (not supported yet)"),

        Section("─ Global ─"),
        Binding("q", "Quit (empty prompt, press twice)"),
        Binding("Esc", "Close dialog / double-tap: interrupt run"),
        Binding("h", "Home screen"),
        Binding("?", "Toggle help"),
        Binding("Ctrl+C", "Quit now (unsent draft auto-stashed)"),
    ]
};

/// help 视口行数(sliding viewport,scroll=选中行)。
const HELP_VIEWPORT: usize = 18;

impl Default for HelpDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpDialog {
    pub fn new() -> Self { Self { visible: false, scroll: 0 } }
    pub fn toggle(&mut self) { self.visible = !self.visible; }
    pub fn dismiss(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible { return false; }
        let max_scroll = KEYBINDINGS.len().saturating_sub(HELP_VIEWPORT);
        match key {
            Key::Escape | Key::Char('q') | Key::Char('h') | Key::Char('?') => { self.dismiss(); true }
            // U19:~100 行表必须可滚(原一次性全量渲染,超高被裁)。
            Key::Up => { self.scroll = self.scroll.saturating_sub(1); true }
            Key::Down => { self.scroll = (self.scroll + 1).min(max_scroll); true }
            Key::PageUp => { self.scroll = self.scroll.saturating_sub(HELP_VIEWPORT); true }
            Key::PageDown => { self.scroll = (self.scroll + HELP_VIEWPORT).min(max_scroll); true }
            Key::Home => { self.scroll = 0; true }
            Key::End => { self.scroll = max_scroll; true }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }

        // Section → muted 行;Binding → 键右对齐 + 描述(与渲染同口径成形)。
        let items: Vec<ListItem> = KEYBINDINGS.iter().map(|entry| match entry {
            HelpEntry::Section(title) => ListItem::Row {
                display: (*title).to_string(),
                muted: true,
            },
            HelpEntry::Binding(key, desc) => ListItem::Row {
                display: format!("{:>16}  {}", key, desc),
                muted: false,
            },
        }).collect();

        backdrop::render_list_dialog_bottom(
            "Help — Keybindings",
            colors::ACCENT_BLUE(),
            &items,
            self.scroll,
            "↑↓/PgUp/PgDn scroll  Home/End: top/bottom  Esc/q/h/?: close",
            ctx, geom, HELP_VIEWPORT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U19 验收:help 与 CommandRegistry 一一对应——registry 里每个 slash
    /// 命令(含别名之外的正式名)都必须在 KEYBINDINGS 表内;表里每个 /x
    /// 行也必须能解析回 registry(正式名或别名)。双向守,防再次脱节。
    #[test]
    fn help_covers_every_registry_slash_command() {
        let registry = agendao_command::CommandRegistry::new();
        let table_keys: Vec<&str> = KEYBINDINGS.iter().filter_map(|e| match e {
            HelpEntry::Binding(k, _) if k.starts_with('/') => Some(*k),
            _ => None,
        }).collect();

        // 正向:registry → 表。注意:registry 的 slash.name 已含前导 "/"。
        for cmd in registry.ui_all_slash_commands() {
            let slash = cmd.slash.as_ref().expect("ui_all_slash_commands 只返回带 slash 的");
            let key = slash.name;
            assert!(
                table_keys.contains(&key),
                "registry 命令 {key} 不在 help 表内——新增 slash 必须同步 KEYBINDINGS"
            );
        }

        // 反向:表 → registry(正式名或别名均可解析;name 有无前导 "/"
        // 两种口径都试,不假设 registry 的存储口径)。
        for key in &table_keys {
            let stripped = key.trim_start_matches('/');
            assert!(
                registry.ui_slash_command(key).is_some()
                    || registry.ui_slash_command(stripped).is_some(),
                "help 表行 {key} 在 registry 里查无此命令——删命令必须同步删表行"
            );
        }
    }

    /// U19:滚动 clamp——表超长后 End 到尾、Up 不破零。
    #[test]
    fn scroll_clamped_to_table() {
        let mut d = HelpDialog::new();
        d.toggle();
        assert!(d.handle_key(&Key::End));
        assert_eq!(d.scroll, KEYBINDINGS.len() - HELP_VIEWPORT);
        assert!(d.handle_key(&Key::Up));
        assert_eq!(d.scroll, KEYBINDINGS.len() - HELP_VIEWPORT - 1);
        d.handle_key(&Key::Home);
        assert_eq!(d.scroll, 0);
        d.handle_key(&Key::Up);
        assert_eq!(d.scroll, 0, "顶端 Up 不破零");
    }

    /// 关闭键仍优先于滚动(q/h/?/Esc dismiss)。
    #[test]
    fn close_keys_still_dismiss() {
        for key in [Key::Escape, Key::Char('q'), Key::Char('h'), Key::Char('?')] {
            let mut d = HelpDialog::new();
            d.toggle();
            assert!(d.handle_key(&key));
            assert!(!d.visible, "{key:?} 应关闭 help");
        }
    }
}
