//! 金 — Skill list dialog (Part 7, B 层补齐)。
//!
//! 与 `/agents` 同范式（A2 已删 ListDialogState 后的内联标准）：keymap
//! dispatch 时 list_skills + open，handle_key 给 Outcome（Option<entry>），
//! render 用 PromptGeom + render_list_dialog_bottom 走 A1 几何。
//!
//! 当前只做"读视图"：选中后 toast skill 名（暂不挂载——挂载需
//! manage_skill + scoping，独立工程）。这是诚实的 first slice：
//! 列表权威已成（金的成形），运行端（挂载/卸载）留待后续——比把
//! 假"已挂载"toast 抹上诚实得多（道纪第十条）。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub location: String,
}

pub struct SkillListDialog {
    pub visible: bool,
    skills: Vec<SkillEntry>,
    selected: usize,
}

impl SkillListDialog {
    pub fn new() -> Self {
        Self { visible: false, skills: Vec::new(), selected: 0 }
    }

    pub fn set_skills(&mut self, skills: Vec<SkillEntry>) {
        self.skills = skills;
        self.selected = 0;
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    /// Enter → 返回选中 entry；其他键不消费但仍 visible。
    pub fn handle_key(&mut self, key: &Key) -> Option<SkillEntry> {
        if !self.visible { return None; }
        if self.skills.is_empty() {
            if matches!(key, Key::Escape) { self.close(); }
            return None;
        }
        let len = self.skills.len();
        match key {
            Key::Up    => { self.selected = (self.selected + len - 1) % len; None }
            Key::Down  => { self.selected = (self.selected + 1) % len; None }
            Key::Home  => { self.selected = 0; None }
            Key::End   => { self.selected = len - 1; None }
            Key::Enter => {
                let pick = self.skills.get(self.selected).cloned();
                self.close();
                pick
            }
            Key::Escape => { self.close(); None }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }
        // 空态：列表为空时仍要给可见反馈（避免"按 /skills 没反应"误判为
        // 已关）；用 muted 行说明，Esc 关闭。
        if self.skills.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No skills available)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "Skills",
                colors::ACCENT_PURPLE,
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        let items: Vec<ListItem> = self.skills.iter().enumerate().take(12).map(|(i, s)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{} — {}", marker, s.name, s.description),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Skills",
            colors::ACCENT_PURPLE,
            &items,
            self.selected,
            "↑↓ navigate  Enter: select (read-only)  Esc: close",
            ctx, geom, 12,
        );
    }
}
