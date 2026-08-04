//! 金 — Session list dialog: browse and switch sessions.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem, ListDialogLayout};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    pub status_hint: String,
}

/// SessionListDialog::handle_key 返回值——Open 单选 vs DeleteBatch 批量删,
/// panel_dispatch 按变体分流(土律:同 dialog 多动作时用 enum 而非多函数)。
#[derive(Clone, Debug)]
pub enum SessionListAction {
    Open(SessionEntry),
    DeleteBatch(Vec<String>),
    /// U15②：空态下按 'n' 开新会话（空态文案承诺的动作必须真实可达——
    /// 原文案写 "Enter 开启新会话" 但 Enter 只关框，说谎）。
    NewSession,
}

pub struct SessionListDialog {
    pub visible: bool,
    pub sessions: Vec<SessionEntry>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// Live search query — type to narrow the visible list. Matches
    /// either the title or the session id (case-insensitive substring).
    pub query: String,
    /// Directory the list is scoped to (canonical path) — purely for
    /// display. Title shows "in <basename>" so the user can tell at a
    /// glance whether they're seeing all sessions or a directory scope.
    /// Empty string means "no scope set".
    pub directory_scope: String,
    /// 批量删除标记：与 `sessions` 同长度索引,true = 当前项已被 'x' 勾选。
    /// 'D' (Shift-d) 收集所有 true 项交给 panel_dispatch 走 Confirm 批量删除。
    /// set_sessions/close 时清空,保证不跨次悬空(道纪第九条:写入即承诺回收)。
    marked: Vec<bool>,
    /// U17②：位置记忆——close 时记录光标，下次 set_sessions 恢复（clamp
    /// 到新长度；列表内容可能已变，取索引近似，GUI 惯例）。
    remembered: usize,
}

impl Default for SessionListDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionListDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            sessions: vec![],
            selected: 0,
            loading: false,
            error: None,
            query: String::new(),
            directory_scope: String::new(),
            marked: vec![],
            remembered: 0,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
        self.query.clear();
        self.marked.clear();
    }

    pub fn close(&mut self) {
        // U17②：关框记住光标位置（下次重开恢复）。
        self.remembered = self.selected;
        self.visible = false;
        self.sessions.clear();
        self.error = None;
        self.loading = false;
        self.query.clear();
        self.directory_scope.clear();
        self.marked.clear();
    }

    pub fn is_open(&self) -> bool { self.visible }

    /// Record the canonical directory the list is scoped to. Used purely
    /// for display in the dialog title; the actual filtering is done at
    /// fetch time before `set_sessions` is called.
    pub fn set_directory_scope(&mut self, dir: String) {
        self.directory_scope = dir;
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionEntry>) {
        let n = sessions.len();
        self.sessions = sessions;
        self.loading = false;
        self.error = None;
        // U17②：恢复上次光标位置（clamp 到新长度）而非一律归零。
        self.selected = self.remembered.min(n.saturating_sub(1));
        // 重置标记位:set_sessions 是新一轮 fetch 的成形,旧标记不应跨次悬空。
        self.marked = vec![false; n];
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.loading = false;
        self.sessions.clear();
    }

    /// Return the currently filtered session list (indexes into self.sessions).
    fn filtered_indices(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        if q.is_empty() {
            return (0..self.sessions.len()).collect();
        }
        self.sessions.iter().enumerate()
            .filter(|(_, s)| s.title.to_lowercase().contains(&q) || s.id.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    /// handle_key 返回三态:
    /// - `Some(SessionListAction::Open(entry))` — Enter 单选打开会话
    /// - `Some(SessionListAction::DeleteBatch(ids))` — 'D' 触发批量删除(非空 marked)
    /// - `None` — 其它按键(导航/输入/标记 toggle/关闭)
    pub fn handle_key(&mut self, key: &Key) -> Option<SessionListAction> {
        if !self.visible { return None; }
        match key {
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Key::Down => {
                let max = self.filtered_indices().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
                None
            }
            Key::Enter => {
                let filtered = self.filtered_indices();
                // U17④：无命中时 Enter 不再静默关框（死端）——原地无操作，
                // 用户继续改 query 或 Esc 显式退出。有命中才关框返回 Open。
                if filtered.is_empty() { return None; }
                // U17④：防御性 clamp——selected 永不超过 filtered 尾（高亮
                // 消失/越界读取的保险，正常键流已钳制，这里兜最后一道）。
                self.selected = self.selected.min(filtered.len() - 1);
                let s = filtered.get(self.selected)
                    .and_then(|&i| self.sessions.get(i))
                    .cloned();
                self.close();
                s.map(SessionListAction::Open)
            }
            Key::Escape => { self.close(); None }
            Key::Backspace => {
                if self.query.pop().is_some() { self.selected = 0; }
                None
            }
            // 'x' = 批量选择标记,作用于当前 cursor 项(filtered_indices 索引映射)。
            // 与单选的 query 输入区分:query 只收 graphic 字符,'x' 落到此 arm 前
            // 必须先于 graphic arm 匹配——故放在 graphic arm 之上(match 顺序优先)。
            Key::Char('x') => {
                if let Some(&abs) = self.filtered_indices().get(self.selected) {
                    if let Some(m) = self.marked.get_mut(abs) { *m = !*m; }
                }
                None
            }
            // 'D' (Shift-d) = 批量删除已标记项。无标记时静默无操作
            // (panel_dispatch 收到 None 不做事;若想 toast 让用户知道未标记,
            //  在 panel_dispatch 侧加一次提示——这里保持 dialog 纯查询)。
            Key::Char('D') => {
                let ids: Vec<String> = self.marked.iter().enumerate()
                    .filter(|(_, &m)| m)
                    .filter_map(|(i, _)| self.sessions.get(i).map(|s| s.id.clone()))
                    .collect();
                if ids.is_empty() {
                    None
                } else {
                    Some(SessionListAction::DeleteBatch(ids))
                }
            }
            // U15②：空态下 'n' = 开新会话（空态文案承诺的可用动作）。非空时
            // 'n' 是普通过滤字符，落下面的 graphic arm——故 guard 必须带
            // is_empty 且本 arm 在 graphic arm 之上（match 顺序优先）。
            Key::Char('n') if self.sessions.is_empty() => {
                self.close();
                Some(SessionListAction::NewSession)
            }
            // Allow alphanumeric + space + dash/underscore/dot for filtering
            // ('x'/'D' 已在前面 arm 拦截,不会落到此处搜索)
            Key::Char(c) if c.is_ascii_graphic() || *c == ' ' => {
                self.query.push(*c);
                self.selected = 0;
                None
            }
            _ => None,
        }
    }

    /// 返回当前已标记的会话条目个数(给渲染层显示「N marked」hint 用)。
    pub fn marked_count(&self) -> usize {
        self.marked.iter().filter(|&&m| m).count()
    }

    /// 已标记后,SessionList 删除批量回填:删除成功的 ids 从内部状态摘除,
    /// 避免接下来仍能选中已删条目。panel_dispatch 在 delete_session 成功后调用。
    pub fn forget_sessions(&mut self, deleted_ids: &[String]) {
        let deleted: std::collections::HashSet<&String> = deleted_ids.iter().collect();
        // 同步剔除 sessions + marked,保持索引对齐(土律:同长度数组单一权威)
        let pairs: Vec<(SessionEntry, bool)> = std::mem::take(&mut self.sessions)
            .into_iter()
            .zip(std::mem::take(&mut self.marked))
            .filter(|(s, _)| !deleted.contains(&s.id))
            .collect();
        let (sessions, marked): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        self.sessions = sessions;
        self.marked = marked;
        let filtered_len = self.filtered_indices().len();
        if self.selected >= filtered_len {
            self.selected = filtered_len.saturating_sub(1);
        }
    }

    /// 粘贴 → 追加到实时过滤 query（剥离控制字符，与逐字输入同口径：
    /// 非 ASCII 的 CJK 也允许——matches_query 对小写化子串匹配）。
    pub fn paste_query(&mut self, text: &str) -> bool {
        if !self.visible {
            return false;
        }
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if !clean.is_empty() {
            self.query.push_str(&clean);
            self.selected = 0;
        }
        true
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }

        // Compose dialog title: include directory scope (basename) so the
        // user always sees whether the list is scoped or global.
        let scope_suffix = if self.directory_scope.is_empty() {
            String::new()
        } else {
            let base = std::path::Path::new(&self.directory_scope)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(self.directory_scope.as_str());
            format!(" — in {}", base)
        };

        if self.loading {
            let title = format!("Sessions{}", scope_suffix);
            let content = vstack().child(Text::new("Loading sessions...").fg(colors::FG_MUTED()));
            backdrop::render_dialog_bottom(&title, colors::ACCENT_CYAN(), content,
                "Loading...", ctx, geom, 5);
        } else if let Some(ref err) = self.error {
            let content = vstack().child(Text::new(format!("Error: {}", err)).fg(colors::ACCENT_RED()));
            backdrop::render_dialog_bottom("Sessions", colors::ACCENT_RED(), content,
                "Esc: close", ctx, geom, 5);
        } else if self.sessions.is_empty() {
            // 空状态：极简一行，scope 信息靠 title 的 "in <name>" 表达。
            // U15②：文案只承诺真实可达的动作（'n' 开新会话 / Esc 关框）——
            // 原 scoped 文案写 "Enter 开启新会话" 但 Enter 只关框，说谎。
            let title = format!("Sessions{}", scope_suffix);
            let msg = if self.directory_scope.is_empty() {
                "No sessions yet — n: new session"
            } else {
                "本目录下暂无会话 — n 开新会话，Esc 返回"
            };
            let body = vstack().child(Text::new(msg).fg(colors::FG_MUTED()));
            backdrop::render_dialog_bottom(&title, colors::ACCENT_CYAN(), body,
                "n: new session  Esc: close", ctx, geom, 5);
        } else {
            let filtered = self.filtered_indices();
            // U17①：过滤无命中 → 明示行（原渲染零行空框，用户分不清是
            // 没匹配还是没数据）。
            let items: Vec<ListItem> = if filtered.is_empty() {
                vec![ListItem::Row {
                    display: format!("  No matches for '{}'", self.query),
                    muted: true,
                }]
            } else {
            filtered.iter().map(|&i| {
                let s = &self.sessions[i];
                let status = if s.status_hint.is_empty() { String::new() } else { format!(" [{}]", s.status_hint) };
                // 标记位前缀:已 'x' 标记的项前面打 `[*]`(2列宽),未标记空白对齐。
                // 复用现有 ListItem::Row(display 单字段),前缀直接拼进 display
                // ——比给 backdrop 加 marked 形参侵入小(金律:成形点单一)。
                let mark = if self.marked.get(i).copied().unwrap_or(false) {
                    "[*] "
                } else {
                    "    "
                };
                ListItem::Row {
                    display: format!("{}{}{}", mark, s.title, status),
                    muted: false,
                }
            }).collect()
            };
            let marked_n = self.marked_count();
            let marked_hint = if marked_n > 0 {
                format!(" — {} marked", marked_n)
            } else { String::new() };
            let title = if self.query.is_empty() {
                format!("Sessions{}{}", scope_suffix, marked_hint)
            } else {
                format!("Sessions{} — query: {}{}", scope_suffix, self.query, marked_hint)
            };
            let footer = if marked_n > 0 {
                "type filter  ⌫ erase  ↑↓ nav  Enter open  x mark  D delete marked  Esc close"
            } else {
                "type filter  ⌫ erase  ↑↓ nav  Enter open  x mark  Esc close"
            };
            let layout = backdrop::render_list_dialog_bottom_with_layout(
                &title,
                colors::ACCENT_CYAN(),
                &items,
                self.selected,
                footer,
                ctx, geom, 18,
            );

            // Publish scrollbar geometry for the mouse handler.
            // Only SessionList publishes right now (the other list
            // dialogs use the simple render_list_dialog_bottom without a
            // publish channel). Extend if/when those need it.
            if let Ok(mut slot) = crate::app::session_list_scrollbar_slot().lock() {
                *slot = layout.scrollbar;
            }

            // Selected-row tooltip — only when the row's display would
            // overflow `inner_w` (i.e. the user can't actually read the
            // full title in the list). The popover floats just below the
            // dialog edge against its right side, so it doesn't cover
            // any other rows.
            self.maybe_render_tooltip(ctx, &items, layout);
        }
    }

    /// Draw a small popover that holds the full title of the selected
    /// row when (and only when) the visible row text would be truncated.
    /// Anchored just below the dialog, hugging the right edge — keeps the
    /// list visible while making the long title legible.
    fn maybe_render_tooltip(
        &self,
        ctx: &mut RenderContext,
        items: &[ListItem],
        layout: ListDialogLayout,
    ) {
        // Only Row items get a tooltip; headers don't have a `display`.
        let Some(_row_y) = layout.selected_row_y else { return; };
        let Some(item) = items.get(self.selected) else { return; };
        let display = match item {
            ListItem::Row { display, .. } => display.as_str(),
            ListItem::Header(_) => return,
        };

        // The list row is decorated with a 2-column prefix ("▌ " or "  ")
        // and a 3-column suffix (" ✓ " or "   "). The actual readable
        // budget for the display text is therefore `inner_w - 5`.
        let row_budget = (layout.inner_w as usize).saturating_sub(5);
        let display_w = UnicodeWidthStr::width(display);
        if display_w <= row_budget {
            // Fully visible already — no popover needed.
            return;
        }

        // Look up the full title from the original (non-truncated) entry
        // so the popover can show even more context than the row buffer.
        let filtered = self.filtered_indices();
        let entry_id = filtered
            .get(self.selected)
            .and_then(|&i| self.sessions.get(i));
        let Some(entry) = entry_id else { return; };

        // Popover sizing: max width = dialog width, max 4 lines wrapped.
        let pop_w = layout.dialog_w.min(80);
        let body_w = pop_w.saturating_sub(2) as usize; // border eats 2 cells

        // Wrap the full title to body_w. Plain greedy split by display
        // width — char-aware so CJK doesn't get cut mid-grapheme.
        let mut wrapped: Vec<String> = Vec::new();
        let mut line = String::new();
        let mut line_w = 0usize;
        for ch in entry.title.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if line_w + cw > body_w && !line.is_empty() {
                wrapped.push(std::mem::take(&mut line));
                line_w = 0;
            }
            line.push(ch);
            line_w += cw;
            if wrapped.len() >= 4 { break; }
        }
        if !line.is_empty() && wrapped.len() < 4 {
            wrapped.push(line);
        }
        let pop_h: u16 = 2 /* border */ + wrapped.len().max(1) as u16;

        // Anchor the popover just below the dialog (or above if there's
        // no room below). x aligns with the dialog's right edge so the
        // popover reads as "tooltip protruding from the selected row".
        let screen_h = ctx.area.height;
        let pop_x = layout.dialog_x;
        let dialog_bottom = layout.dialog_y.saturating_add(layout.dialog_h);
        let pop_y = if dialog_bottom + pop_h <= screen_h {
            dialog_bottom
        } else {
            // Place above the dialog instead.
            layout.dialog_y.saturating_sub(pop_h)
        };

        let mut body = vstack().gap(0);
        for line in &wrapped {
            body = body.child_sized(
                Text::new(line.as_str()).fg(colors::FG_PRIMARY()),
                1,
            );
        }
        let pop = Border::rounded()
            .title(" full title ")
            .fg(colors::ACCENT_CYAN())
            .child(body);

        // Convert absolute screen coords back to ctx-relative (positioned
        // expects ctx-relative). Negative offsets are fine — positioned
        // accepts i16.
        let rel_x = (pop_x as i16) - (ctx.area.x as i16);
        let rel_y = (pop_y as i16) - (ctx.area.y as i16);
        revue::widget::positioned(pop)
            .x(rel_x)
            .y(rel_y)
            .width(pop_w)
            .height(pop_h)
            .render(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> SessionEntry {
        SessionEntry { id: id.into(), title: format!("t-{id}"), status_hint: String::new() }
    }

    /// U15②：空态下 'n' 是"开新会话"动作（空态文案承诺的键必须真实可达）。
    #[test]
    fn n_in_empty_list_offers_new_session() {
        let mut d = SessionListDialog::new();
        d.open();
        d.set_sessions(Vec::new());
        assert!(matches!(
            d.handle_key(&Key::Char('n')),
            Some(SessionListAction::NewSession)
        ));
        assert!(!d.is_open(), "动作成形后 dialog 关闭");
    }

    /// U15②：非空时 'n' 仍是过滤字符，不被新会话 arm 抢走。
    #[test]
    fn n_in_nonempty_list_filters() {
        let mut d = SessionListDialog::new();
        d.open();
        d.set_sessions(vec![entry("a"), entry("b")]);
        assert!(d.handle_key(&Key::Char('n')).is_none());
        assert_eq!(d.query, "n");
        assert!(d.is_open(), "过滤不关框");
    }

    /// U17④：过滤无命中时 Enter 是 no-op（不再静默关框制造死端）。
    #[test]
    fn enter_no_match_is_noop() {
        let mut d = SessionListDialog::new();
        d.open();
        d.set_sessions(vec![entry("a"), entry("b")]);
        d.handle_key(&Key::Char('z'));
        assert!(d.handle_key(&Key::Enter).is_none(), "无命中 Enter 不成动作");
        assert!(d.is_open(), "无命中 Enter 不关框");
    }

    /// U17②：关框记住光标，重开（新一轮 set_sessions）恢复并 clamp 到新长度。
    #[test]
    fn position_memory_restored_and_clamped() {
        let mut d = SessionListDialog::new();
        d.open();
        d.set_sessions(vec![entry("a"), entry("b"), entry("c")]);
        d.handle_key(&Key::Down);
        d.handle_key(&Key::Down);
        assert_eq!(d.selected, 2);
        d.close();
        // 重开：列表变短（2 项）→ 记忆位置 clamp 到尾。
        d.set_sessions(vec![entry("a"), entry("b")]);
        assert_eq!(d.selected, 1, "记忆位置 clamp 到新长度");
    }
}
