//! 金 — Skill evolution proposal list dialog（Part 1, B 层第二批）。
//!
//! 与 `/skills` 同范式（B 层第一批建立的内联列表标准）：keymap dispatch
//! 时 list_skill_proposals + open，handle_key 给 Outcome（Option<entry>），
//! render 用 PromptGeom + render_list_dialog_bottom 走 A1 几何。
//!
//! 读视图 first slice（道纪第十条）：列表权威已成（金的成形），写端
//! （approve/reject proposal 需 update_skill_proposal_status + confirm）
//! 留 B 层第三批——比把假"已批准"toast 抹上诚实得多。

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

    /// Enter → 返回选中 entry；其他键不消费但仍 visible。
    pub fn handle_key(&mut self, key: &Key) -> Option<SkillProposalEntry> {
        if !self.visible { return None; }
        if self.proposals.is_empty() {
            if matches!(key, Key::Escape) { self.close(); }
            return None;
        }
        let len = self.proposals.len();
        match key {
            Key::Up    => { self.selected = (self.selected + len - 1) % len; None }
            Key::Down  => { self.selected = (self.selected + 1) % len; None }
            Key::Home  => { self.selected = 0; None }
            Key::End   => { self.selected = len - 1; None }
            Key::Enter => {
                let pick = self.proposals.get(self.selected).cloned();
                self.close();
                pick
            }
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
            "↑↓ navigate  Enter: select (read-only)  Esc: close",
            ctx, geom, 12,
        );
    }
}
