//! 木 — Slash command popup: / triggered command palette.
//!
//! Uses agendao_command::CommandRegistry for real slash commands
//! with fuzzy matching, keyboard navigation, and declarative Revue layout.

use agendao_command::{CommandRegistry, UiActionId, UiCommandSpec};
use revue::prelude::*;
use revue::event::Key;
use revue::runtime::render::Cell;
use crate::theme::colors;

/// Simple fuzzy match: check if all chars of `query` appear in `target` in order.
pub(crate) fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return Some(0); }
    let t = target.to_lowercase();
    let mut qi = q.chars();
    let mut current = qi.next();
    let mut score = 0i32;
    for (i, tc) in t.chars().enumerate() {
        if let Some(qc) = current {
            if qc == tc { score += 100 - (i as i32).min(50); current = qi.next(); }
        } else { break; }
    }
    if current.is_none() { Some(score) } else { None }
}

pub struct SlashPopup {
    pub visible: bool,
    pub query: String,
    pub selected: usize,
    /// All slash commands from the registry
    all_commands: Vec<UiCommandSpec>,
    /// Filtered indices into all_commands
    filtered: Vec<usize>,
    selected_action: Option<UiActionId>,
}

impl SlashPopup {
    pub fn new() -> Self {
        Self::with_dir(None)
    }

    /// Optionally load custom commands from a project directory.
    pub fn with_dir(dir: Option<&std::path::Path>) -> Self {
        let mut registry = CommandRegistry::new();
        if let Some(d) = dir {
            let _ = registry.load_from_directory(d);
        }
        let all_commands: Vec<UiCommandSpec> = registry
            .ui_all_slash_commands()
            .into_iter()
            .cloned()
            .collect();
        Self {
            visible: false,
            query: String::new(),
            selected: 0,
            all_commands,
            filtered: Vec::new(),
            selected_action: None,
        }
    }

    pub fn open(&mut self) {
        self.visible = true; self.selected = 0; self.query.clear();
        self.refresh_filter();
    }

    pub fn open_with_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.refresh_filter();
        self.visible = true;
        self.selected_action = None;
    }

    pub fn close(&mut self) {
        self.visible = false; self.query.clear(); self.filtered.clear();
        self.selected = 0; self.selected_action = None;
    }

    pub fn is_open(&self) -> bool { self.visible }

    pub fn take_action(&mut self) -> Option<UiActionId> {
        self.selected_action.take()
    }

    /// Detect if the current prompt text contains a slash token.
    /// Returns the text after `/` (the query) if a slash command is detected.
    pub fn slash_token(text: &str) -> Option<String> {
        text.split_whitespace()
            .last()
            .filter(|token| token.starts_with('/'))
            .map(|token| token.trim_start_matches('/').to_string())
            .filter(|token| !token.is_empty())
    }

    /// Number of filtered results (for sizing the popup).
    pub fn filtered_count(&self) -> usize { self.filtered.len() }

    /// Push a character to the filter query.
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refresh_filter();
    }

    /// Pop last character from the filter query.
    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refresh_filter();
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<UiActionId> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Enter => {
                if let Some(idx) = self.filtered.get(self.selected) {
                    let action_id = self.all_commands[*idx].action_id;
                    // close() clears selected_action — remember the action
                    // BEFORE close(), then return it directly. Calling
                    // self.take_action() after close() always yields None.
                    self.close();
                    return Some(action_id);
                }
                self.take_action()
            }
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Key::Down => {
                let max = self.filtered.len().saturating_sub(1);
                if self.selected < max { self.selected += 1; }
                None
            }
            Key::Backspace => { self.pop_char(); None }
            Key::Char(c) if c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_' => {
                self.push_char(*c); None
            }
            _ => None,
        }
    }

    fn refresh_filter(&mut self) {
        if self.query.is_empty() {
            // Show suggested commands from the registry
            let registry = CommandRegistry::new();
            let suggested: Vec<UiActionId> = registry
                .ui_suggested_slash_commands()
                .into_iter()
                .map(|cmd| cmd.action_id)
                .collect();
            self.filtered = self.all_commands.iter().enumerate()
                .filter(|(_, cmd)| suggested.contains(&cmd.action_id))
                .map(|(i, _)| i)
                .collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self.all_commands.iter().enumerate()
                .filter_map(|(i, cmd)| {
                    let slash = cmd.slash.as_ref()?;
                    let name_score = fuzzy_match(&self.query, slash.name);
                    let alias_score = slash.aliases.iter()
                        .filter_map(|alias| fuzzy_match(&self.query, alias))
                        .max();
                    let title_score = fuzzy_match(&self.query, cmd.title);
                    let best = name_score.into_iter().chain(alias_score).chain(title_score).max()?;
                    Some((i, best))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = 0;
    }

    /// Render popup — 无框实色面板(背景由调用方 [`fill_background`] 预填整片)。
    /// ❯ pointer + 分类标题 + 底部 hint。去 border:revue 边框线格经 ctx.set 覆盖,
    /// 默认 bg=None(Cell::new),边框线要么实色要么发黑;用户要"border 不要背景",
    /// 故去框,靠实色面板与下层 BG_PRIMARY 区分。每行 Text 再 .bg(BG_SURFACE) 补文字格
    /// (revue 的 Text 渲染 ctx.set 也是覆盖写,不补 bg 则文字格发黑/透字)。
    pub fn render_popup(&self) -> impl View {
        let mut stack = vstack();
        if !self.visible {
            return stack;
        }

        // 空状态:背景由 fill_background 预填,这里只显示提示(文字格补 .bg)
        if self.filtered.is_empty() {
            stack = stack.child(
                Text::new("  No results ").fg(colors::FG_MUTED).bg(colors::BG_SURFACE),
            );
            return stack;
        }

        // 与 app/mod.rs 的 ph = filtered_count.min(8) + 4 高度预算对齐,
        // 超出 8 项由 "... and N more" 折叠,避免内容高度超过浮层被裁剪。
        let max_visible = 8usize.min(self.filtered.len());
        let mut list = vstack().gap(0);
        let mut last_category: Option<&str> = None;

        for (row_idx, &cmd_idx) in self.filtered.iter().enumerate().take(max_visible) {
            let cmd = &self.all_commands[cmd_idx];
            let is_selected = row_idx == self.selected;

            // 分类分隔
            let cat = cmd.category.label();
            if last_category.map(|c| c != cat).unwrap_or(true) {
                if last_category.is_some() {
                    list = list.child(Text::new("").bg(colors::BG_SURFACE));
                }
                list = list.child(
                    Text::new(&format!(" {}:", cat)).fg(colors::ACCENT_BLUE).bg(colors::BG_SURFACE),
                );
                last_category = Some(cat);
            }

            let slash_name = cmd.slash.as_ref()
                .map(|s| s.name.trim_start_matches('/'))
                .unwrap_or(cmd.title);

            // ❯ pointer + 文字色;.bg(BG_SURFACE) 补文字格,否则 ctx.set 默认 bg=None 发黑
            let pointer = if is_selected { "❯ " } else { "  " };
            let keybind_str = cmd.keybind.map(|k| format!(" ({})", k)).unwrap_or_default();
            let desc = format!("{} /{}{}  {}", pointer, slash_name, keybind_str, cmd.description);

            let text = if is_selected {
                Text::new(&desc).fg(colors::ACCENT_CYAN).bg(colors::BG_SURFACE)
            } else {
                Text::new(&desc).fg(colors::FG_SECONDARY).bg(colors::BG_SURFACE)
            };
            list = list.child(text);
        }

        if self.filtered.len() > max_visible {
            list = list.child(
                Text::new(format!("  ... and {} more", self.filtered.len() - max_visible))
                    .fg(colors::FG_MUTED).bg(colors::BG_SURFACE),
            );
        }

        // 底部 hint
        list = list.child(
            Text::new(" ↑/↓ navigate · Enter select · Esc cancel ")
                .fg(colors::FG_MUTED).bg(colors::BG_SURFACE),
        );

        stack = stack.child(list);
        stack
    }

    /// 实色填充 popup 区域,挡住下层 transcript。由调用方(app/mod.rs)在
    /// render_popup + positioned 渲染前调用。
    /// 根因——revue positioned 浮层不清背景(positioned.rs 只通过 sub_area 划区),
    /// 且内部 Stack/Text 渲染虽补了文字格 .bg,但 list 之外的浮层边缘/空白格仍透明。
    /// 故先实色预填整片,再让 render_popup 在其上绘制。守住"实色不透字"契约。
    pub fn fill_background(&self, buf: &mut Buffer, x: u16, y: u16, w: u16, h: u16) {
        buf.fill(x, y, w, h, Cell::new(' ').bg(colors::BG_SURFACE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::colors;

    /// fill_background 必须把指定矩形填成 BG_SURFACE 实色,且不污染区域外。
    /// 守住"实色不透字"契约——positioned 浮层不清背景,全靠这一步预填整片。
    #[test]
    fn fill_background_fills_region_solid() {
        let popup = SlashPopup::new();
        let mut buf = Buffer::new(20, 10);
        popup.fill_background(&mut buf, 2, 2, 10, 5);
        // 区域内实色
        assert_eq!(buf.get(5, 4).and_then(|c| c.bg), Some(colors::BG_SURFACE));
        assert_eq!(buf.get(10, 6).and_then(|c| c.bg), Some(colors::BG_SURFACE));
        // 区域外未被填充,保持 None
        assert_eq!(buf.get(0, 0).and_then(|c| c.bg), None);
        assert_eq!(buf.get(19, 9).and_then(|c| c.bg), None);
    }

    /// 完整流程:fill_background 预填后 render_popup 绘制,内部仍保持实色
    /// (每行 Text 已 .bg(BG_SURFACE) 补文字格)。守住"渲染后不透字"。
    #[test]
    fn render_popup_keeps_solid_after_fill() {
        let mut popup = SlashPopup::new();
        popup.open();
        let view = popup.render_popup();
        let mut buf = Buffer::new(60, 20);
        popup.fill_background(&mut buf, 0, 0, 60, 20);
        let mut ctx = RenderContext::new(&mut buf, Rect::new(0, 0, 60, 20));
        view.render(&mut ctx);
        let mut has_solid = false;
        for x in (1u16..59).step_by(5) {
            if has_solid { break; }
            for y in (1u16..19).step_by(2) {
                if buf.get(x, y).and_then(|c| c.bg) == Some(colors::BG_SURFACE) {
                    has_solid = true;
                    break;
                }
            }
        }
        assert!(has_solid, "popup 渲染后内部必须保持实色,否则透字");
    }

    /// 空状态分支(无匹配命令)同样要实色背景,不透字。
    #[test]
    fn render_popup_empty_state_keeps_solid() {
        let mut popup = SlashPopup::new();
        popup.open();
        popup.query = "zzz_no_match".to_string();
        popup.refresh_filter();
        assert_eq!(popup.filtered_count(), 0);
        let view = popup.render_popup();
        let mut buf = Buffer::new(60, 6);
        popup.fill_background(&mut buf, 0, 0, 60, 6);
        let mut ctx = RenderContext::new(&mut buf, Rect::new(0, 0, 60, 6));
        view.render(&mut ctx);
        let mut has_solid = false;
        for x in (1u16..59).step_by(5) {
            if has_solid { break; }
            for y in (1u16..5).step_by(2) {
                if buf.get(x, y).and_then(|c| c.bg) == Some(colors::BG_SURFACE) {
                    has_solid = true;
                    break;
                }
            }
        }
        assert!(has_solid, "空状态 popup 也要实色背景");
    }
}
