//! 金 — Skill list dialog (Part 7, B 层补齐)。
//!
//! 与 `/agents` 同范式（A2 已删 ListDialogState 后的内联标准）：keymap
//! dispatch 时 list_skills + open，handle_key 给 Outcome（Option<entry>），
//! render 用 PromptGeom + render_list_dialog_bottom 走 A1 几何。
//!
//! Enter 打开详情视图（F8 接线）：dispatch 拉 get_skill_detail（双模式），
//! 全文按行进 detail mode 滚动展示（↑↓/PgUp/PgDn），Esc 返回列表；再 Esc
//! 关 dialog。挂载（manage_skill + scoping）仍留待独立工程——读视图不假装
//! "已挂载"（道纪第十条）。

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

/// Enter 选中——dispatch 拉详情后调 `show_detail` 回填（dialog 不关）。
pub enum SkillListAction {
    View(SkillEntry),
}

/// 详情视图：title + 预切行 + 滚动偏移（内容长，全文行进）。
struct SkillDetailView {
    title: String,
    lines: Vec<String>,
    scroll: usize,
}

pub struct SkillListDialog {
    pub visible: bool,
    skills: Vec<SkillEntry>,
    selected: usize,
    detail: Option<SkillDetailView>,
}

/// 详情视口高度（与列表 min_height 对齐）。
const DETAIL_VIEWPORT: usize = 12;

impl Default for SkillListDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillListDialog {
    pub fn new() -> Self {
        Self { visible: false, skills: Vec::new(), selected: 0, detail: None }
    }

    pub fn set_skills(&mut self, skills: Vec<SkillEntry>) {
        self.skills = skills;
        self.selected = 0;
        self.detail = None;
    }

    /// dispatch 拉到详情后回填：进入 detail mode。
    pub fn show_detail(&mut self, title: String, lines: Vec<String>) {
        self.detail = Some(SkillDetailView { title, lines, scroll: 0 });
    }

    pub fn open(&mut self) { self.visible = true; }
    pub fn close(&mut self) { self.visible = false; self.detail = None; }
    pub fn is_open(&self) -> bool { self.visible }

    /// detail mode 下键全部内部消费（滚动/返回）；list mode Enter → View。
    pub fn handle_key(&mut self, key: &Key) -> Option<SkillListAction> {
        if !self.visible { return None; }
        if let Some(ref mut detail) = self.detail {
            let max_scroll = detail.lines.len().saturating_sub(DETAIL_VIEWPORT);
            match key {
                Key::Up => { detail.scroll = detail.scroll.saturating_sub(1); }
                Key::Down => { detail.scroll = (detail.scroll + 1).min(max_scroll); }
                Key::PageUp => { detail.scroll = detail.scroll.saturating_sub(DETAIL_VIEWPORT); }
                Key::PageDown => { detail.scroll = (detail.scroll + DETAIL_VIEWPORT).min(max_scroll); }
                Key::Home => { detail.scroll = 0; }
                Key::End => { detail.scroll = max_scroll; }
                Key::Escape => { self.detail = None; }
                _ => {}
            }
            return None;
        }
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
                // dialog 保持打开——dispatch 拉详情回填 show_detail。
                self.skills.get(self.selected).cloned().map(SkillListAction::View)
            }
            Key::Escape => { self.close(); None }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }
        if let Some(ref detail) = self.detail {
            let items: Vec<ListItem> = detail.lines.iter().enumerate().map(|(i, line)| {
                ListItem::Row {
                    display: format!("  {line}"),
                    muted: i == 0,
                }
            }).collect();
            backdrop::render_list_dialog_bottom(
                &detail.title,
                colors::ACCENT_PURPLE(),
                &items,
                // 用选中索引驱动 sliding viewport：scroll 即"选中行"。
                detail.scroll,
                "↑↓/PgUp/PgDn scroll  Esc: back",
                ctx, geom, DETAIL_VIEWPORT,
            );
            return;
        }
        // 空态：列表为空时仍要给可见反馈（避免"按 /skills 没反应"误判为
        // 已关）；用 muted 行说明，Esc 关闭。
        if self.skills.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No skills available)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "Skills",
                colors::ACCENT_PURPLE(),
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        let items: Vec<ListItem> = self.skills.iter().enumerate().map(|(i, s)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{} — {}", marker, s.name, s.description),
                muted: false,
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Skills",
            colors::ACCENT_PURPLE(),
            &items,
            self.selected,
            "↑↓ navigate  Enter: detail  Esc: close",
            ctx, geom, 12,
        );
    }
}
