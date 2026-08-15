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

use crate::dialog::backdrop::{self, ListItem};
use crate::theme::colors;
use revue::event::Key;
use revue::prelude::*;

#[derive(Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub location: String,
}

/// Enter 选中——dispatch 拉详情后调 `show_detail` 回填（dialog 不关）。
pub enum SkillListAction {
    View(SkillEntry),
    /// U16：空态 's' → 跳 Settings 管理 skills（空态承诺的下一步必须
    /// 真实可达）。
    OpenSettings,
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
    /// U17⑤：实时过滤 query（对 name+description 大小写不敏感子串）。
    /// skills 是列表型 dialog 里唯一可能超 20 项且无字母动作冲突的，
    /// type-ahead 只落在这里（mcp_list 的 c/d/a/A/x/n/e 全是字母动作）。
    query: String,
    /// U17②：位置记忆——close 记录 selected，set_skills 重开时 clamp 恢复。
    remembered: usize,
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
        Self {
            visible: false,
            skills: Vec::new(),
            selected: 0,
            detail: None,
            query: String::new(),
            remembered: 0,
        }
    }

    pub fn set_skills(&mut self, skills: Vec<SkillEntry>) {
        self.skills = skills;
        // U17②：重开恢复上次位置（clamp 到当前长度——列表可能变短）。
        self.selected = self.remembered.min(self.skills.len().saturating_sub(1));
        self.detail = None;
    }

    /// dispatch 拉到详情后回填：进入 detail mode。
    pub fn show_detail(&mut self, title: String, lines: Vec<String>) {
        self.detail = Some(SkillDetailView {
            title,
            lines,
            scroll: 0,
        });
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
    }
    pub fn close(&mut self) {
        // U17②：关框记住位置，下次 set_skills 恢复。
        self.remembered = self.selected;
        self.visible = false;
        self.detail = None;
    }
    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// U17⑤：过滤后的绝对索引视图（query 空 = 全量）。selected 索引的
    /// 是这个视图，Enter/渲染经它映射回 skills 绝对索引。
    fn filtered_indices(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        if q.is_empty() {
            return (0..self.skills.len()).collect();
        }
        self.skills
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.name.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// U17⑤：bracketed paste 进过滤 query（与 session/model 同范式）。
    pub fn paste_query(&mut self, text: &str) -> bool {
        if !self.visible || self.detail.is_some() {
            return false;
        }
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return true;
        }
        self.query.push_str(&clean);
        self.selected = 0;
        true
    }

    /// detail mode 下键全部内部消费（滚动/返回）；list mode Enter → View。
    pub fn handle_key(&mut self, key: &Key) -> Option<SkillListAction> {
        if !self.visible {
            return None;
        }
        if let Some(ref mut detail) = self.detail {
            let max_scroll = detail.lines.len().saturating_sub(DETAIL_VIEWPORT);
            match key {
                Key::Up => {
                    detail.scroll = detail.scroll.saturating_sub(1);
                }
                Key::Down => {
                    detail.scroll = (detail.scroll + 1).min(max_scroll);
                }
                Key::PageUp => {
                    detail.scroll = detail.scroll.saturating_sub(DETAIL_VIEWPORT);
                }
                Key::PageDown => {
                    detail.scroll = (detail.scroll + DETAIL_VIEWPORT).min(max_scroll);
                }
                Key::Home => {
                    detail.scroll = 0;
                }
                Key::End => {
                    detail.scroll = max_scroll;
                }
                Key::Escape => {
                    self.detail = None;
                }
                _ => {}
            }
            return None;
        }
        if self.skills.is_empty() {
            match key {
                Key::Escape => {
                    self.close();
                }
                // U16：空态给真实下一步——'s' 跳 Settings 管理 skills。
                Key::Char('s') => {
                    self.close();
                    return Some(SkillListAction::OpenSettings);
                }
                _ => {}
            }
            return None;
        }
        // U17⑤：导航/Enter 作用于过滤视图；len==0（无命中）时不取模防 panic。
        let filtered = self.filtered_indices();
        let len = filtered.len();
        match key {
            Key::Up => {
                if len > 0 {
                    self.selected = (self.selected + len - 1) % len;
                }
                None
            }
            Key::Down => {
                if len > 0 {
                    self.selected = (self.selected + 1) % len;
                }
                None
            }
            Key::Home => {
                self.selected = 0;
                None
            }
            Key::End => {
                self.selected = len.saturating_sub(1);
                None
            }
            Key::Enter => {
                // U17④：无命中 Enter 不静默关框——保持打开等用户改 query；
                // dialog 保持打开——dispatch 拉详情回填 show_detail。
                filtered
                    .get(self.selected)
                    .and_then(|&i| self.skills.get(i))
                    .cloned()
                    .map(SkillListAction::View)
            }
            Key::Escape => {
                self.close();
                None
            }
            Key::Backspace => {
                if self.query.pop().is_some() {
                    self.selected = 0;
                }
                None
            }
            // U17⑤：type-ahead——graphic/空格进 query（本 dialog 列表模式
            // 无字母动作，不冲突；空态分支已在上方拦截 's'）。
            Key::Char(c) if c.is_ascii_graphic() || *c == ' ' => {
                self.query.push(*c);
                self.selected = 0;
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible {
            return;
        }
        if let Some(ref detail) = self.detail {
            let items: Vec<ListItem> = detail
                .lines
                .iter()
                .enumerate()
                .map(|(i, line)| ListItem::Row {
                    display: format!("  {line}"),
                    muted: i == 0,
                })
                .collect();
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: &detail.title,
                    border_color: colors::ACCENT_PURPLE(),
                },
                &items,
                // 用选中索引驱动 sliding viewport：scroll 即"选中行"。
                detail.scroll,
                "↑↓/PgUp/PgDn scroll  Home/End: top/bottom  Esc: back",
                ctx,
                geom,
                DETAIL_VIEWPORT,
            );
            return;
        }
        // 空态：列表为空时仍要给可见反馈（避免"按 /skills 没反应"误判为
        // 已关）；用 muted 行说明，并给真实下一步（U16：'s' 跳 Settings）。
        if self.skills.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No skills available — manage them in Settings)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: "Skills",
                    border_color: colors::ACCENT_PURPLE(),
                },
                &items,
                0,
                "s: open settings  Esc: close",
                ctx,
                geom,
                3,
            );
            return;
        }
        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        // U17⑤：渲染过滤视图；U17①：无命中给可见提示行而非空白列表。
        let filtered = self.filtered_indices();
        let items: Vec<ListItem> = if filtered.is_empty() {
            vec![ListItem::Row {
                display: format!("  No matches for '{}'", self.query),
                muted: true,
            }]
        } else {
            filtered
                .iter()
                .enumerate()
                .map(|(vi, &ai)| {
                    let s = &self.skills[ai];
                    let marker = if vi == self.selected { "▶ " } else { "  " };
                    ListItem::Row {
                        display: format!("{}{} — {}", marker, s.name, s.description),
                        muted: false,
                    }
                })
                .collect()
        };
        let title = if self.query.is_empty() {
            "Skills".to_string()
        } else {
            format!("Skills — filter: {}", self.query)
        };
        backdrop::render_list_dialog_bottom(
            backdrop::ListDialogHeading {
                title: &title,
                border_color: colors::ACCENT_PURPLE(),
            },
            &items,
            self.selected,
            "type filter  ⌫ erase  ↑↓ navigate  Home/End: jump  Enter: detail  Esc: close",
            ctx,
            geom,
            12,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, desc: &str) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            description: desc.into(),
            location: String::new(),
        }
    }

    /// U17⑤：type-ahead 缩小过滤视图；导航/Enter 作用于过滤后顺序，
    /// Enter 映射回正确条目。
    #[test]
    fn type_ahead_narrows_and_enter_views_filtered() {
        let mut d = SkillListDialog::new();
        d.open();
        d.set_skills(vec![
            entry("alpha", "first"),
            entry("beta", "second"),
            entry("alpine", "third"),
        ]);
        d.handle_key(&Key::Char('a'));
        d.handle_key(&Key::Char('l'));
        // 命中 alpha/alpine（beta 被滤掉）；光标 0 = alpha，Down → alpine。
        d.handle_key(&Key::Down);
        match d.handle_key(&Key::Enter) {
            Some(SkillListAction::View(e)) => assert_eq!(e.name, "alpine"),
            _ => panic!("expected View(alpine)"),
        }
    }

    /// U17⑤：description 也参与匹配（大小写不敏感）。
    #[test]
    fn type_ahead_matches_description_case_insensitive() {
        let mut d = SkillListDialog::new();
        d.open();
        d.set_skills(vec![entry("alpha", "First"), entry("beta", "second")]);
        d.handle_key(&Key::Char('F')); // 大写 F 命中 "First"
        match d.handle_key(&Key::Enter) {
            Some(SkillListAction::View(e)) => assert_eq!(e.name, "alpha"),
            _ => panic!("expected View(alpha)"),
        }
    }

    /// U17④：过滤无命中时 Enter 是 no-op（不静默关框）。
    #[test]
    fn enter_no_match_stays_open() {
        let mut d = SkillListDialog::new();
        d.open();
        d.set_skills(vec![entry("alpha", "first")]);
        d.handle_key(&Key::Char('z'));
        assert!(d.handle_key(&Key::Enter).is_none(), "无命中 Enter 不成动作");
        assert!(d.is_open(), "无命中 Enter 不关框");
    }

    /// U17⑤：非空列表里 's' 是过滤字符——OpenSettings 只在空态分支。
    #[test]
    fn s_in_nonempty_list_filters_not_settings() {
        let mut d = SkillListDialog::new();
        d.open();
        d.set_skills(vec![entry("alpha", "first")]);
        assert!(d.handle_key(&Key::Char('s')).is_none());
        assert_eq!(d.query, "s");
        assert!(d.is_open());
    }

    /// U17②：关框记住光标，重开恢复并 clamp 到新长度。
    #[test]
    fn position_memory_restored_and_clamped() {
        let mut d = SkillListDialog::new();
        d.open();
        d.set_skills(vec![entry("a", "1"), entry("b", "2"), entry("c", "3")]);
        d.handle_key(&Key::Down);
        d.handle_key(&Key::Down);
        assert_eq!(d.selected, 2);
        d.close();
        // 重开：open 清 query 但不动 selected；set_skills 恢复记忆位置。
        d.open();
        d.set_skills(vec![entry("a", "1"), entry("b", "2")]);
        assert_eq!(d.selected, 1, "记忆位置 clamp 到新长度");
    }

    /// U17⑤：⌫ 回删 query；清空后恢复全量视图。
    #[test]
    fn backspace_erases_query() {
        let mut d = SkillListDialog::new();
        d.open();
        d.set_skills(vec![entry("alpha", "first"), entry("beta", "second")]);
        d.handle_key(&Key::Char('a'));
        d.handle_key(&Key::Char('l'));
        assert_eq!(d.query, "al");
        d.handle_key(&Key::Backspace);
        d.handle_key(&Key::Backspace);
        assert!(d.query.is_empty());
        // 全量恢复：光标 0 = alpha。
        match d.handle_key(&Key::Enter) {
            Some(SkillListAction::View(e)) => assert_eq!(e.name, "alpha"),
            _ => panic!("expected View(alpha)"),
        }
    }
}
