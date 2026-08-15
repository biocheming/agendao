//! 金 — MCP server status list dialog。
//!
//! 与 `/skills` 同范式。写操作闭环（B 层第三批）：c=connect / d=disconnect
//! 直接执行（前置 status 校验——已 connected 不重复 connect，未 connected
//! 不 disconnect），Ok 后重拉 get_mcp_status 回流（status 字段变化非移除，
//! 重拉是唯一权威——水生木）。dialog 保持打开支持批量。Enter=View 关闭。
//! OAuth（F7 接线）：a=发起（展示授权 URL，浏览器完成授权）/ A=完成（服务端
//! 已授权则 connect）/ x=清除凭据。

use crate::dialog::backdrop::{self, ListItem};
use crate::theme::colors;
use revue::event::Key;
use revue::prelude::*;

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
    /// a：发起 OAuth——拿授权 URL 展示给用户（浏览器完成授权）。
    AuthStart(McpEntry),
    /// A：完成 OAuth——服务端在已授权时 connect（复用 authenticate 语义）。
    AuthFinish(McpEntry),
    /// x：清除已存 OAuth 凭据。
    AuthRemove(McpEntry),
    /// n：新增 server（复用 Settings 的 McpEditDialog add 模式）。
    Add,
    /// e：编辑选中 server（复用 McpEditDialog edit 模式，需 settings 行）。
    Edit(McpEntry),
    View(McpEntry),
}

pub struct McpListDialog {
    pub visible: bool,
    entries: Vec<McpEntry>,
    selected: usize,
    /// U17②：位置记忆——close 时记录光标，重开恢复（clamp 到新长度）。
    remembered: usize,
}

impl Default for McpListDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl McpListDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            selected: 0,
            remembered: 0,
        }
    }

    pub fn set_entries(&mut self, entries: Vec<McpEntry>) {
        let n = entries.len();
        self.entries = entries;
        // U17②：恢复上次光标位置（clamp 到新长度）而非一律归零。
        self.selected = self.remembered.min(n.saturating_sub(1));
    }

    pub fn open(&mut self) {
        self.visible = true;
    }
    pub fn close(&mut self) {
        // U17②：关框记住光标位置（下次重开恢复）。
        self.remembered = self.selected;
        self.visible = false;
    }
    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// c=connect / d=disconnect（保持 dialog 打开，支持批量）/ Enter=view（关闭）。
    pub fn handle_key(&mut self, key: &Key) -> Option<McpAction> {
        if !self.visible {
            return None;
        }
        if self.entries.is_empty() {
            match key {
                Key::Escape => {
                    self.close();
                }
                // 空列表也要能新增（否则 0 server 时 /mcp 是死端）。
                Key::Char('n') => return Some(McpAction::Add),
                _ => {}
            }
            return None;
        }
        let len = self.entries.len();
        let pick = || self.entries.get(self.selected).cloned();
        match key {
            Key::Up => {
                self.selected = (self.selected + len - 1) % len;
                None
            }
            Key::Down => {
                self.selected = (self.selected + 1) % len;
                None
            }
            Key::Home => {
                self.selected = 0;
                None
            }
            Key::End => {
                self.selected = len - 1;
                None
            }
            Key::Enter => {
                let pick = pick();
                self.close();
                pick.map(McpAction::View)
            }
            Key::Char('c') => pick().map(McpAction::Connect),
            Key::Char('d') => pick().map(McpAction::Disconnect),
            Key::Char('a') => pick().map(McpAction::AuthStart),
            Key::Char('A') => pick().map(McpAction::AuthFinish),
            Key::Char('x') => pick().map(McpAction::AuthRemove),
            Key::Char('n') => Some(McpAction::Add),
            Key::Char('e') => pick().map(McpAction::Edit),
            Key::Escape => {
                self.close();
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible {
            return;
        }
        if self.entries.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No MCP servers configured)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: "MCP Servers",
                    border_color: colors::ACCENT_CYAN(),
                },
                &items,
                0,
                "n: add server  Esc: close",
                ctx,
                geom,
                3,
            );
            return;
        }
        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let marker = if i == self.selected { "▶ " } else { "  " };
                ListItem::Row {
                    display: format!(
                        "{}[{}] {} · tools:{} res:{}",
                        marker, m.status, m.name, m.tools, m.resources
                    ),
                    muted: false,
                }
            })
            .collect();
        backdrop::render_list_dialog_bottom(
            backdrop::ListDialogHeading {
                title: "MCP Servers",
                border_color: colors::ACCENT_CYAN(),
            },
            &items,
            self.selected,
            "↑↓ navigate  Home/End: jump  c: connect  d: disconnect  a/A: oauth start/finish  x: clear auth  n: add  e: edit  Enter: view  Esc: close",
            ctx, geom, 12,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> McpEntry {
        McpEntry {
            name: name.into(),
            status: "connected".into(),
            tools: 1,
            resources: 0,
        }
    }

    /// U17②：关框记住光标，重开（新一轮 set_entries）恢复并 clamp 到新长度。
    /// 位置记忆是批量套用到 6 个 list dialog 的同构模式，这里守一份回归。
    #[test]
    fn position_memory_restored_and_clamped() {
        let mut d = McpListDialog::new();
        d.open();
        d.set_entries(vec![entry("a"), entry("b"), entry("c")]);
        d.handle_key(&Key::Down);
        d.handle_key(&Key::Down);
        assert_eq!(d.selected, 2);
        d.close();
        // 重开：列表变短（2 项）→ 记忆位置 clamp 到尾。
        d.set_entries(vec![entry("a"), entry("b")]);
        assert_eq!(d.selected, 1, "记忆位置 clamp 到新长度");
    }
}
