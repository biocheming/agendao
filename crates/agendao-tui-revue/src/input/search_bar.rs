//! 木 — Transcript 搜索条（Ctrl+F）：会话内全文检索与块跳转。
//!
//! 与 slash_popup 不同，搜索条**自维字符缓冲**（query 不派生自输入框）：
//! 搜索词是临时态，Esc 即弃，不该污染 prompt 单点权威（木律：输入变体
//! 必须复用同一输入权威——搜索是旁路查询，不是待发草稿）。
//!
//! 匹配语义：全部块类型经 `block_to_text`（session_store 单点序列化）
//! 大小写不敏感子串匹配；命中收集块索引，跳转复用 `transcript_cursor`
//! 现成机制（块自动获 BG_HIGHLIGHT + ▶ + ensure_cursor_visible 滚动跟随），
//! 渲染层零改动。

use crate::store::session_store::{block_to_text, SessionStore};
use crate::theme::colors;
use revue::event::Key;
use revue::prelude::*;
use revue::runtime::render::Cell;

/// `handle_key` 的返回：搜索条只裁决语义，cursor/panel 状态归调用方
/// （SlashKeyOutcome 同范式——组件无副作用，副作用在 panel_dispatch 收口）。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SearchKeyOutcome {
    /// Enter / Down：跳下一个匹配（末尾回绕）。
    Next,
    /// Shift+Enter / Up / 'N'：跳上一个匹配（首回绕）。
    Prev,
    /// Esc：关闭搜索条，清除跳转高亮。
    Close,
    /// 普通字符/编辑键已被搜索条消化（query 变化，调用方需 update_matches）。
    Consumed,
    /// 未处理（理论不达——搜索态收所有键；留作扩展位）。
    Pass,
}

pub struct SearchBar {
    pub visible: bool,
    /// 搜索词（组件自维：临时态，不进 prompt 权威）。
    pub query: String,
    /// 当前命中索引（matches 内偏移，非块索引）。
    pub selected: usize,
    /// 命中块索引列表（update_matches 重算）。
    pub matches: Vec<usize>,
    /// query 变化后首次 Enter 先"激活"当前 selected（与状态栏 `N/M`
    /// 显示对齐），不前进——显示与跳转必须同块（金律：成形一致）。
    pending_activate: bool,
}

impl Default for SearchBar {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchBar {
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            selected: 0,
            matches: Vec::new(),
            pending_activate: true,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        // 二次 Ctrl+F 重开不清 query？——清。搜索词不暂存（MVP 无历史），
        // 重开即新搜（与 /sessions filter 同口径）。
        self.query.clear();
        self.selected = 0;
        self.matches.clear();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected = 0;
        self.matches.clear();
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// 当前选中命中对应的块索引（无命中 None）。
    pub fn current_block(&self) -> Option<usize> {
        self.matches.get(self.selected).copied()
    }

    /// 位置指示文案：`3/17 matches`（渲染唯一口径）。
    pub fn status_text(&self) -> String {
        if self.matches.is_empty() {
            "0 matches".to_string()
        } else {
            format!("{}/{} matches", self.selected + 1, self.matches.len())
        }
    }

    /// 重算命中（query 每次变化后调用；线性扫描，messages 数量级下无性能顾虑）。
    /// selected clamp 到新命中集——防止上轮选中的偏移悬空指向错块。
    pub fn update_matches(&mut self, session: &SessionStore) {
        self.pending_activate = true;
        let needle = self.query.trim().to_lowercase();
        if needle.is_empty() {
            self.matches.clear();
            self.selected = 0;
            return;
        }
        self.matches = session
            .messages
            .get()
            .iter()
            .enumerate()
            .filter(|(_, block)| block_to_text(block).to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    pub(crate) fn handle_key(&mut self, key: &Key) -> SearchKeyOutcome {
        if !self.visible {
            return SearchKeyOutcome::Pass;
        }
        match key {
            Key::Escape => {
                self.close();
                SearchKeyOutcome::Close
            }
            Key::Enter | Key::Down => SearchKeyOutcome::Next,
            Key::Up => SearchKeyOutcome::Prev,
            Key::Char(c) => {
                self.query.push(*c);
                SearchKeyOutcome::Consumed
            }
            Key::Backspace => {
                self.query.pop();
                SearchKeyOutcome::Consumed
            }
            _ => SearchKeyOutcome::Consumed,
        }
    }

    /// 跳转目标（Next/Prev 后的块索引），回绕语义在此收口。query 刚变化
    /// 后的首次调用先激活当前 selected（跳到状态栏显示的那块），此后
    /// 每次 Enter 前进一个——显示 N/M 与跳转目标必须一致。
    pub fn advance(&mut self, forward: bool) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        if self.pending_activate {
            self.pending_activate = false;
            return self.current_block();
        }
        let len = self.matches.len();
        self.selected = if forward {
            (self.selected + 1) % len
        } else {
            (self.selected + len - 1) % len
        };
        self.current_block()
    }

    /// 渲染一行搜索条（背景由调用方 fill_background 预填——revue positioned
    /// 浮层不清背景，slash 同坑）。`🔍 [query] N/M · Enter:next ↑:prev Esc:close`
    /// （无 'n' 导航键——搜索态字母全是 query 字符，vi 风格 n/N 会吞掉
    /// 含 n 的搜索词。）
    pub fn render_bar(&self, w: u16) -> impl View {
        let mut text = format!(" 🔍 [{}] ", self.query);
        let hint = format!(
            "{} · Enter:next ↑:prev Esc:close",
            self.status_text()
        );
        // 窄宽截断留 1 列右边距，防 positioned 裁半个 CJK（slash 同口径）。
        let full = format!("{text}{hint}");
        let max = w.saturating_sub(1).max(8) as usize;
        if full.chars().count() > max {
            let taken: String = full.chars().take(max.saturating_sub(1)).collect();
            text = taken + "…";
        } else {
            text = full;
        }
        vstack().gap(0).child(Text::new(text).fg(colors::ACCENT_CYAN()).bg(colors::BG_SURFACE()))
    }

    /// 实色预填搜索条区域（调用方在 render_bar + positioned 前调用）。
    pub fn fill_background(&self, buf: &mut Buffer, x: u16, y: u16, w: u16, h: u16) {
        buf.fill(x, y, w, h, Cell::new(' ').bg(colors::BG_SURFACE()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::session_store::SessionStore;
    use crate::store::types::{FoldState, TranscriptBlock};

    fn user_prompt(text: &str) -> TranscriptBlock {
        TranscriptBlock::UserPrompt {
            id: format!("u-{text}"),
            content: text.to_string(),
            fold: FoldState::Truncated,
            failed: false,
        }
    }

    fn assistant(text: &str) -> TranscriptBlock {
        TranscriptBlock::AssistantMsg {
            id: format!("a-{text}"),
            content: text.to_string(),
            fold: FoldState::Truncated,
        }
    }

    /// 匹配语义：全部文本块大小写不敏感子串；空 query 无命中。
    #[test]
    fn update_matches_case_insensitive_across_blocks() {
        let mut bar = SearchBar::new();
        let session = SessionStore::new();
        session.messages.set(vec![
            user_prompt("fix the login bug"),
            assistant("Fixed THE Login flow"),
            user_prompt("unrelated"),
        ]);

        bar.query = "the login".into();
        bar.update_matches(&session);
        assert_eq!(bar.matches, vec![0, 1], "两个块都含 'the login'（大小写不敏感）");

        bar.query = "  ".into();
        bar.update_matches(&session);
        assert!(bar.matches.is_empty(), "空/纯空白 query 无命中");
    }

    /// 回绕：末尾 Next 回首、开头 Prev 回尾；空命中集 None；
    /// pending_activate 首跳落在当前 selected。
    #[test]
    fn advance_wraps_and_empty_is_none() {
        let mut bar = SearchBar::new();
        bar.matches = vec![2, 5, 9];
        bar.selected = 2;
        // 首次（activate）落在 selected 指向的块。
        assert_eq!(bar.advance(true), Some(9), "activate 跳到 selected 块");
        assert_eq!(bar.advance(true), Some(2), "末尾 Next 回绕到首");
        assert_eq!(bar.advance(false), Some(9), "再 Prev 回绕回尾");
        bar.matches.clear();
        assert_eq!(bar.advance(true), None);
    }

    /// selected clamp：命中集缩小后不悬空。
    #[test]
    fn update_matches_clamps_selected() {
        let mut bar = SearchBar::new();
        let session = SessionStore::new();
        session
            .messages
            .set(vec![user_prompt("alpha"), user_prompt("beta alpha")]);
        bar.query = "alpha".into();
        bar.update_matches(&session);
        assert_eq!(bar.matches, vec![0, 1]);
        bar.selected = 1;
        // query 收窄到只命中 1 块 → selected 必须被拉回。
        bar.query = "beta".into();
        bar.update_matches(&session);
        assert_eq!(bar.matches, vec![1]);
        assert_eq!(bar.selected, 0);
        assert_eq!(bar.current_block(), Some(1));
    }

    /// 键裁决：Enter=Next、Esc=Close（含状态复位）、字符进 query
    /// （'n' 也是普通字符——含 n 的搜索词必须可输入）。
    #[test]
    fn handle_key_semantics() {
        let mut bar = SearchBar::new();
        bar.open();
        assert_eq!(bar.handle_key(&Key::Char('a')), SearchKeyOutcome::Consumed);
        assert_eq!(bar.query, "a");
        assert_eq!(bar.handle_key(&Key::Backspace), SearchKeyOutcome::Consumed);
        assert_eq!(bar.query, "");
        assert_eq!(bar.handle_key(&Key::Enter), SearchKeyOutcome::Next);
        assert_eq!(bar.handle_key(&Key::Down), SearchKeyOutcome::Next);
        assert_eq!(bar.handle_key(&Key::Up), SearchKeyOutcome::Prev);
        assert_eq!(bar.handle_key(&Key::Char('n')), SearchKeyOutcome::Consumed);
        assert_eq!(bar.query, "n", "'n' 进 query，不是导航键");
        assert_eq!(bar.handle_key(&Key::Escape), SearchKeyOutcome::Close);
        assert!(!bar.is_open());
        assert!(bar.query.is_empty(), "Esc 清 query（临时态即弃）");
    }

    /// status 文案：选中位/命中数；0 命中显示 0。
    #[test]
    fn status_text_counts() {
        let mut bar = SearchBar::new();
        assert_eq!(bar.status_text(), "0 matches");
        bar.matches = vec![1, 3];
        bar.selected = 1;
        assert_eq!(bar.status_text(), "2/2 matches");
    }
}
