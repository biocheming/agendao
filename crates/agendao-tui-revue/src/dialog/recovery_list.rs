//! 金 — Session recovery list dialog。
//!
//! per-session 视图（需 active session_id）。把 `SessionRecoveryProtocol`
//! 的 actions + checkpoints 映射成行。写操作闭环（B 层第三批）：
//! x=execute 走 confirm 类（execute 影响执行流，需二次确认）——仅 action
//! 行可执行（checkpoint 无 action_kind，'x' 返回 None）。Enter=View 关闭。
//! session_id 不在 dialog 持有，panel_dispatch 处理 Execute 时从
//! active_session 取（modal 不变量保证 open→confirm 期间不变）。

use revue::prelude::*;
use revue::event::Key;
use agendao_client::RecoveryActionKind;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct RecoveryEntry {
    pub label: String,
    pub detail: String,
    /// action 行给 Some（可 execute）；checkpoint 行给 None（'x' 无反应）。
    pub action_kind: Option<RecoveryActionKind>,
    pub target_id: Option<String>,
}

/// 列表按键动作。Execute 携 label（confirm message 友好显示）+ action_kind +
/// target_id（API 参数）。confirm 类——panel_dispatch 关 list + 开 ConfirmDialog。
pub enum RecoveryAction {
    Execute {
        label: String,
        action_kind: RecoveryActionKind,
        target_id: Option<String>,
    },
    View(RecoveryEntry),
}

pub struct RecoveryListDialog {
    pub visible: bool,
    entries: Vec<RecoveryEntry>,
    selected: usize,
    /// U17②：位置记忆——close 时记录光标，重开恢复（clamp 到新长度）。
    remembered: usize,
}

impl Default for RecoveryListDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoveryListDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: Vec::new(), selected: 0, remembered: 0 }
    }

    pub fn set_entries(&mut self, entries: Vec<RecoveryEntry>) {
        let n = entries.len();
        self.entries = entries;
        // U17②：恢复上次光标位置（clamp 到新长度）而非一律归零。
        self.selected = self.remembered.min(n.saturating_sub(1));
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) {
        // U17②：关框记住光标位置（下次重开恢复）。
        self.remembered = self.selected;
        self.visible = false;
    }
    pub fn is_open(&self) -> bool { self.visible }

    /// x=execute（仅 action_kind.is_some()，不 close——panel_dispatch 关+开 confirm）
    /// / Enter=view（关闭）。
    pub fn handle_key(&mut self, key: &Key) -> Option<RecoveryAction> {
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
                pick.map(RecoveryAction::View)
            }
            // checkpoint（action_kind=None）按 x 无反应——不伪执行（道纪第十条）。
            // RecoveryActionKind 非 Copy，clone Option 后 map move。
            Key::Char('x') => self.entries.get(self.selected).and_then(|e| {
                e.action_kind.clone().map(|k| RecoveryAction::Execute {
                    label: e.label.clone(),
                    action_kind: k,
                    target_id: e.target_id.clone(),
                })
            }),
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
                colors::ACCENT_YELLOW(),
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        let items: Vec<ListItem> = self.entries.iter().enumerate().map(|(i, e)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            let exec_hint = if e.action_kind.is_some() { " [x: exec]" } else { "" };
            ListItem::Row {
                display: format!("{}{} — {}{}", marker, e.label, e.detail, exec_hint),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Recovery",
            colors::ACCENT_YELLOW(),
            &items,
            self.selected,
            "↑↓ navigate  Home/End: jump  x: execute (actions only)  Enter: view  Esc: close",
            ctx, geom, 16,
        );
    }
}
