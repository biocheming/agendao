//! 金 — Skill evolution proposal list dialog。
//!
//! 与 `/skills` 同范式（B 层第一批内联列表标准）：keymap dispatch 时
//! list_skill_proposals + open，handle_key 给 `Option<SkillProposalAction>`，
//! render 用 PromptGeom + render_list_dialog_bottom 走 A1 几何。
//!
//! 写操作闭环（B 层第三批）：a=approve / r=reject 直接执行
//! update_skill_proposal_status + remove_by_id 回流（水生木）；dialog 保持
//! 打开支持批量。Enter=View 关闭。approve/reject 走 "accepted"/"rejected"
//! （ProposalStatus 枚举值，server `/skill/proposal/{id}/status` 接受）。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct SkillProposalEntry {
    pub id: String,
    pub title: String,
    pub status: String,
    pub kind: String,
}

/// 列表按键动作（per-dialog action enum，跟 ProviderAction/ExportAction 先例）。
/// 变体携 entry（Clone 小 struct）——handler 拿到 id（API）/title（toast）全部信息。
pub enum SkillProposalAction {
    Approve(SkillProposalEntry),
    Reject(SkillProposalEntry),
    View(SkillProposalEntry),
}

pub struct SkillProposalDialog {
    pub visible: bool,
    proposals: Vec<SkillProposalEntry>,
    selected: usize,
}

impl SkillProposalDialog {
    pub fn new() -> Self {
        Self { visible: false, proposals: Vec::new(), selected: 0 }
    }

    pub fn set_proposals(&mut self, proposals: Vec<SkillProposalEntry>) {
        self.proposals = proposals;
        self.selected = 0;
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    /// approve/reject 后移除已处理条目（列表 pending-filtered，状态变更后
    /// 条目离开 pending 列表——悲观移除，与 API Ok 同步）。entries 私有，
    /// handler 无法直接过滤，故经此方法（土律：唯一所有权）。
    pub fn remove_by_id(&mut self, id: &str) {
        if let Some(idx) = self.proposals.iter().position(|p| p.id == id) {
            self.proposals.remove(idx);
            if self.selected >= self.proposals.len() {
                self.selected = self.proposals.len().saturating_sub(1);
            }
        }
    }

    /// a=approve / r=reject（保持 dialog 打开，支持批量）/ Enter=view（关闭）。
    pub fn handle_key(&mut self, key: &Key) -> Option<SkillProposalAction> {
        if !self.visible { return None; }
        if self.proposals.is_empty() {
            if matches!(key, Key::Escape) { self.close(); }
            return None;
        }
        let len = self.proposals.len();
        let pick = || self.proposals.get(self.selected).cloned();
        match key {
            Key::Up    => { self.selected = (self.selected + len - 1) % len; None }
            Key::Down  => { self.selected = (self.selected + 1) % len; None }
            Key::Home  => { self.selected = 0; None }
            Key::End   => { self.selected = len - 1; None }
            Key::Enter => {
                let pick = pick();
                self.close();
                pick.map(SkillProposalAction::View)
            }
            Key::Char('a') => pick().map(SkillProposalAction::Approve),
            Key::Char('r') => pick().map(SkillProposalAction::Reject),
            Key::Escape => { self.close(); None }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }
        if self.proposals.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No pending proposals)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "Proposals",
                colors::ACCENT_PURPLE,
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        let items: Vec<ListItem> = self.proposals.iter().enumerate().take(12).map(|(i, p)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}[{}] {} — {}", marker, p.status, p.title, p.kind),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Proposals",
            colors::ACCENT_PURPLE,
            &items,
            self.selected,
            "↑↓ navigate  a: approve  r: reject  Enter: view  Esc: close",
            ctx, geom, 12,
        );
    }
}
