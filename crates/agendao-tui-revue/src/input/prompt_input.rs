//! 木 — PromptInput: single authority for all user text input.
//!
//! Multi-line composer: editing authority is revue's `TextArea` (cursor,
//! selection, undo, word navigation), rendering is a local `PromptView`
//! that adds the ❯ arrow prefix, continuation-line indent, adaptive
//! height (capped at `MAX_VISIBLE_LINES` with a scrollbar past that) and
//! a blinking block cursor driven by the app-level blink tick.
//!
//! Key contract: `Enter` submits; `Shift+Enter` / `Ctrl+Enter` insert a
//! newline (routed by `app::keymap` via [`PromptInput::insert_newline`]
//! before the bare-Enter path, so the two never collide).

use std::cell::Cell;
use std::rc::Rc;

use revue::event::Key;
use revue::render::{Cell as BufCell, Modifier};
use revue::widget::TextArea;
use revue::widget::traits::{RenderContext, View};

use crate::theme::colors;

#[derive(Clone, Debug)]
pub enum PromptAction { None, Consumed, Submit(String), SubmitShell(String) }

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode { Normal, Shell }

/// 输入框可见行数上限：超出出滚动条（土律·单点常量）。
pub(crate) const MAX_VISIBLE_LINES: u16 = 10;
/// 首行 ❯ 箭头前缀宽；续行同宽空格缩进对齐。
pub(crate) const PROMPT_INDENT: u16 = 2;

/// Render-time geometry published back to [`PromptInput`] for mouse
/// hit-testing (absolute screen coords — `ctx.area` is absolute, see
/// `ScrollableTranscript`). Written every frame by `PromptView::render`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PromptViewGeom {
    /// Left edge of the whole input content area (arrow column included).
    pub x: u16,
    /// Top row of the input content area.
    pub y: u16,
    /// Full width of the content area.
    pub width: u16,
    /// Hit rows: visible text rows + 1 (bottom border line still focuses).
    pub hit_rows: u16,
    /// Scroll window (line, col) at render time — click → cursor mapping
    /// must read the same scroll the user saw (金律·渲染/命中同源)。
    pub scroll_line: usize,
    pub scroll_col: usize,
}

pub struct PromptInput {
    /// 多行编辑权威（cursor/selection/undo）。内部恒 focused(true)——
    /// 聚焦闸门由自有 `focused` 字段承担，editor 只负责编辑语义。
    editor: TextArea,
    mode: InputMode,
    focused: bool,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
    normal_placeholders: Vec<String>,
    shell_placeholders: Vec<String>,
    /// Optional path for persisting history to disk.
    history_path: Option<std::path::PathBuf>,
    /// 当前 placeholder（mode/随机选择后固定，避免每帧重建 editor）。
    placeholder: String,
    /// render → event 的几何回流（Rc<Cell>：`View::render` 只有 &self，
    /// 与 ScrollableTranscript 的 publish 通道同构）。
    geom: Rc<Cell<Option<PromptViewGeom>>>,
}

fn default_history_path() -> std::path::PathBuf {
    // 输入历史统一收在 agendao_home（~/.agendao,土律·单点权威）。
    agendao_util::agendao_home().join("prompt-history.json")
}

fn load_history(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history(path: &std::path::Path, history: &[String]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(history) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true).create(true).truncate(true)
                .mode(0o600)
                .open(path)
            {
                let _ = f.write_all(json.as_bytes());
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::write(path, &json);
        }
    }
}

impl Default for PromptInput {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptInput {
    pub fn new() -> Self {
        Self {
            editor: TextArea::new().focused(true),
            mode: InputMode::Normal,
            focused: false,
            history: Vec::new(),
            history_idx: None,
            draft: None,
            normal_placeholders: vec!["Ask anything...".into()],
            shell_placeholders: vec!["Run a command...".into()],
            history_path: None,
            placeholder: "Ask anything...".into(),
            geom: Rc::new(Cell::new(None)),
        }
    }

    /// Load history from default path.
    pub fn with_persistence(mut self) -> Self {
        let path = default_history_path();
        self.history = load_history(&path);
        self.history_path = Some(path);
        self
    }

    pub fn with_placeholders(mut self, normal: &[&str], shell: &[&str]) -> Self {
        self.normal_placeholders = normal.iter().map(|s| s.to_string()).collect();
        self.shell_placeholders = shell.iter().map(|s| s.to_string()).collect();
        // Pick a random one
        let idx = (std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as usize)
            % self.normal_placeholders.len();
        self.placeholder = self.normal_placeholders[idx].clone();
        self
    }

    /// 换 placeholder 并清空内容（mode 切换 / submit 后复位共用）。
    fn reset_editor(&mut self, placeholder: &str) {
        self.editor.set_content("");
        self.placeholder = placeholder.to_string();
    }

    fn normal_placeholder(&self) -> &str {
        self.normal_placeholders.first().map(|s| s.as_str()).unwrap_or("Ask anything...")
    }

    fn shell_placeholder(&self) -> &str {
        self.shell_placeholders.first().map(|s| s.as_str()).unwrap_or("Run a command...")
    }

    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        // Shell mode toggle
        if let Key::Char('!') = key {
            if self.editor.get_content().trim().is_empty() {
                self.mode = InputMode::Shell;
                self.focused = true;
                let ph = self.shell_placeholder().to_string();
                self.reset_editor(&ph);
                return PromptAction::None;
            }
        }
        if matches!(key, Key::Escape) && self.mode == InputMode::Shell {
            self.mode = InputMode::Normal;
            self.focused = false;
            let ph = self.normal_placeholder().to_string();
            self.reset_editor(&ph);
            return PromptAction::None;
        }

        match key {
            Key::Enter => {
                let text = self.editor.get_content().trim().to_string();
                if !text.is_empty() {
                    self.history.push(text.clone());
                    if let Some(ref path) = self.history_path {
                        save_history(path, &self.history);
                    }
                    self.history_idx = None;
                    self.draft = None;
                    self.focused = false;
                    if self.mode == InputMode::Shell {
                        self.mode = InputMode::Normal;
                        let ph = self.normal_placeholder().to_string();
                        self.reset_editor(&ph);
                        return PromptAction::SubmitShell(text);
                    }
                    let ph = self.placeholder.clone();
                    self.reset_editor(&ph);
                    return PromptAction::Submit(text);
                }
                PromptAction::None
            }
            // Up/Down：多行时光标在文本内行间移动；到顶/底才进历史。
            Key::Up => {
                if self.editor.cursor_position().0 > 0 {
                    self.focused = true;
                    self.editor.handle_key(key);
                    PromptAction::Consumed
                } else {
                    self.history_up();
                    PromptAction::Consumed
                }
            }
            Key::Down => {
                let last = self.editor.line_count().saturating_sub(1);
                if self.editor.cursor_position().0 < last {
                    self.focused = true;
                    self.editor.handle_key(key);
                    PromptAction::Consumed
                } else {
                    self.history_down();
                    PromptAction::Consumed
                }
            }
            // Tab 不插入（对齐旧单行行为：Tab 归 keymap 的 transcript 导航）。
            Key::Tab => PromptAction::None,
            _ => {
                self.focused = true;
                let changed = self.editor.handle_key(key);
                if changed { PromptAction::Consumed } else { PromptAction::None }
            }
        }
    }

    /// Shift+Enter / Ctrl+Enter 换行（Enter 发送语义不变——keymap 单点路由）。
    pub fn insert_newline(&mut self) {
        self.focused = true;
        self.editor.handle_key(&Key::Enter);
    }

    fn history_up(&mut self) -> PromptAction {
        if self.history.is_empty() { return PromptAction::None; }
        if self.history_idx.is_none() {
            self.draft = Some(self.editor.get_content());
            self.history_idx = Some(self.history.len().saturating_sub(1));
        } else if let Some(idx) = self.history_idx {
            if idx > 0 { self.history_idx = Some(idx - 1); }
        }
        if let Some(idx) = self.history_idx {
            if let Some(entry) = self.history.get(idx).cloned() {
                self.editor.set_content(&entry);
                self.editor.move_document_end();
            }
        }
        PromptAction::None
    }

    fn history_down(&mut self) -> PromptAction {
        if self.history_idx.is_none() { return PromptAction::None; }
        if let Some(idx) = self.history_idx {
            if idx + 1 < self.history.len() {
                self.history_idx = Some(idx + 1);
                if let Some(entry) = self.history.get(idx + 1).cloned() {
                    self.editor.set_content(&entry);
                    self.editor.move_document_end();
                }
            } else {
                self.history_idx = None;
                let draft = self.draft.take().unwrap_or_default();
                self.editor.set_content(&draft);
                self.editor.move_document_end();
            }
        }
        PromptAction::None
    }

    pub fn text(&self) -> String { self.editor.get_content() }

    /// Replace the input text wholesale (e.g. restoring a stashed draft).
    /// 喂回输入框权威 —— 水生木闭环（stash 恢复项回灌下一轮输入）。
    pub fn set_text(&mut self, text: &str) {
        self.editor.set_content(text);
        self.editor.move_document_end();
    }
    pub fn clear(&mut self) {
        self.editor.set_content("");
        self.focused = false;
    }
    pub fn is_focused(&self) -> bool { self.focused }

    /// Focus the input — shows the block cursor. Used when entering a route
    /// that is "ready to type" (e.g. Home), so the cursor is visible on entry
    /// rather than only after the first keystroke/click.
    pub fn focus(&mut self) { self.focused = true; }

    /// 内容行数（布局高度自适应的唯一权威）。
    pub fn content_lines(&self) -> u16 {
        self.editor.line_count().max(1) as u16
    }

    /// 可见行数：自适应内容、封顶 MAX_VISIBLE_LINES（超出出滚动条）。
    pub fn visible_height(&self) -> u16 {
        self.content_lines().clamp(1, MAX_VISIBLE_LINES)
    }

    /// Handle a mouse click at (x, y) — absolute screen coords.
    /// 命中区来自 render 发布的真实几何（替代旧 y>=35 硬编码）：
    /// 命中 → 聚焦并把光标定位到点击的字符位置；未命中 → 失焦。
    pub fn handle_click(&mut self, x: u16, y: u16) -> bool {
        if let Some(g) = self.geom.get() {
            if y >= g.y && y < g.y + g.hit_rows && x >= g.x && x < g.x + g.width {
                self.focused = true;
                let line = g.scroll_line + (y - g.y) as usize;
                // scroll_col 以显示列计 → 换算回字符索引（宽字符行内不跑偏）。
                let target_dx = if x >= g.x + PROMPT_INDENT {
                    g.scroll_col + (x - g.x - PROMPT_INDENT) as usize
                } else {
                    g.scroll_col
                };
                let line_text = self
                    .editor
                    .get_content()
                    .lines()
                    .nth(line)
                    .unwrap_or("")
                    .to_string();
                let col = char_index_at_display_col(&line_text, target_dx);
                self.editor.set_cursor(line, col);
                return true;
            }
        }
        self.focused = false;
        false
    }
    pub fn mode(&self) -> &InputMode { &self.mode }

    /// Show status hint above the prompt bar.
    pub fn status_hint(&self, is_running: bool) -> String {
        if is_running { return "Running... Esc: stop".into(); }
        let len = self.editor.get_content().trim().len();
        if self.focused && len > 0 {
            format!("{} chars | Enter: send | Alt+Enter: newline", len)
        } else if self.focused {
            "Type to start... | Enter: send | Alt+Enter: newline".into()
        } else {
            "Click below to type, or just start typing...".into()
        }
    }

    /// Snapshot a renderable view of the composer.
    /// `cursor_on` = 闪烁相（app 层 blink tick 推导）&& 希望画光标。
    pub fn view(&self, cursor_on: bool) -> PromptView {
        PromptView {
            lines: self.editor.get_content().split('\n').map(|s| s.to_string()).collect(),
            cursor: self.editor.cursor_position(),
            focused: self.focused,
            cursor_on: self.focused && cursor_on,
            placeholder: self.placeholder.clone(),
            geom: self.geom.clone(),
        }
    }
}

// ── 金：PromptView — 多行输入框渲染（❯ 箭头 + 缩进 + 滚动条 + 闪烁光标）──

pub struct PromptView {
    lines: Vec<String>,
    /// (line, col) 字符坐标。
    cursor: (usize, usize),
    focused: bool,
    /// 本帧是否画块光标（focused && blink 相位亮）。
    cursor_on: bool,
    placeholder: String,
    geom: Rc<Cell<Option<PromptViewGeom>>>,
}

impl PromptView {
    /// 行 `idx` 中字符列 `col` 之前的显示宽度（宽字符感知，tab 按 4 空格）。
    fn display_width_to(line: &str, col: usize) -> u16 {
        line.chars().take(col).map(|ch| match ch {
            '\t' => 4,
            c => unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16,
        }).sum()
    }
}

impl View for PromptView {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;
        if area.width == 0 || area.height == 0 {
            return;
        }
        // 全区先填空：revue 局部脏区渲染（app/mod.rs render_to_buffer）会
        // copy 旧 buffer 只清脏区——本 view 不绘的 cell（续行缩进 x0-1、行尾
        // 余白、滚动条出现前的右缘列、内容收缩后的空行）会原样保留前几帧
        // 残字（实测滚动后行首漏出 ❯/历史字符）。自绘全区后输出只取决于本帧。
        ctx.buffer.fill(area.x, area.y, area.width, area.height, BufCell::new(' '));
        let rows = area.height as usize;
        let total = self.lines.len();
        let need_scrollbar = total > rows;
        let text_w = area
            .width
            .saturating_sub(PROMPT_INDENT)
            .saturating_sub(if need_scrollbar { 1 } else { 0 });

        // ── 垂直滚动窗：跟随光标行（读上一帧几何，消除抖动）──
        let prev = self.geom.get();
        let mut scroll_line = prev.map(|g| g.scroll_line).unwrap_or(0);
        if self.cursor.0 < scroll_line {
            scroll_line = self.cursor.0;
        } else if self.cursor.0 >= scroll_line + rows {
            scroll_line = self.cursor.0 + 1 - rows;
        }
        // ── 水平滚动窗：跟随光标列（宽字符按显示宽近似）──
        let cursor_line_text = self.lines.get(self.cursor.0).cloned().unwrap_or_default();
        let cursor_dx = Self::display_width_to(&cursor_line_text, self.cursor.1) as usize;
        let mut scroll_col = prev.map(|g| g.scroll_col).unwrap_or(0);
        // scroll_col 以「显示列」计；点击命中同口径（近似 char=cell，CJK 偏差可接受）。
        if cursor_dx < scroll_col {
            scroll_col = cursor_dx;
        } else if text_w > 0 && cursor_dx >= scroll_col + text_w as usize {
            scroll_col = cursor_dx + 1 - text_w as usize;
        }

        // ── 几何发布（命中同源）──
        self.geom.set(Some(PromptViewGeom {
            x: area.x,
            y: area.y,
            width: area.width,
            hit_rows: area.height + 1, // +1 底边框行点击仍聚焦
            scroll_line,
            scroll_col,
        }));

        let empty = total == 1 && self.lines[0].is_empty();

        // ── placeholder：空且未聚焦 ──
        if empty && !self.focused {
            let ph = &self.placeholder;
            let mut x = PROMPT_INDENT;
            for ch in ph.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if x + cw > PROMPT_INDENT + text_w {
                    break;
                }
                let mut cell = BufCell::new(ch);
                cell.fg = Some(colors::FG_MUTED());
                ctx.set(x, 0, cell);
                x += cw;
            }
        }

        // ── ❯ 箭头：内容首行（滚走后自然消失），续行缩进对齐 ──
        if scroll_line == 0 && rows > 0 {
            let mut cell = BufCell::new('❯');
            cell.fg = Some(colors::E_TEAL());
            ctx.set(0, 0, cell);
        }

        // ── 文本行 ──
        for row in 0..rows {
            let line_idx = scroll_line + row;
            if line_idx >= total {
                break;
            }
            let y = row as u16;
            let line = &self.lines[line_idx];

            // 按显示列切窗口：跳过 scroll_col 显示列。
            let mut display_x: u16 = 0;
            let mut skipped: u16 = 0;
            for (char_idx, ch) in line.chars().enumerate() {
                let cw = match ch {
                    '\t' => 4,
                    c => unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16,
                };
                if (skipped as usize) < scroll_col {
                    skipped += cw;
                    continue;
                }
                if display_x + cw > text_w {
                    break;
                }
                let is_cursor = self.cursor_on
                    && self.cursor.0 == line_idx
                    && self.cursor.1 == char_idx;
                let draw = if ch == '\t' { ' ' } else { ch };
                let mut cell = BufCell::new(draw);
                if is_cursor {
                    cell.bg = Some(revue::prelude::Color::WHITE);
                    cell.modifier = Modifier::BOLD;
                } else {
                    cell.fg = Some(colors::FG_PRIMARY());
                }
                ctx.set(PROMPT_INDENT + display_x, y, cell);
                // tab 补齐剩余空格
                for pad in 1..cw {
                    if display_x + pad < text_w {
                        let sp = BufCell::new(' ');
                        ctx.set(PROMPT_INDENT + display_x + pad, y, sp);
                    }
                }
                display_x += cw;
            }

            // 光标在行尾（含空行）：画空白块光标
            if self.cursor_on
                && self.cursor.0 == line_idx
                && self.cursor.1 >= line.chars().count()
            {
                let dx = Self::display_width_to(line, self.cursor.1) as usize;
                if dx >= scroll_col {
                    let x = (dx - scroll_col) as u16;
                    if x < text_w {
                        let mut cell = BufCell::new(' ');
                        cell.bg = Some(revue::prelude::Color::WHITE);
                        cell.modifier = Modifier::BOLD;
                        ctx.set(PROMPT_INDENT + x, y, cell);
                    }
                }
            }
        }

        // ── 滚动条：内容超窗时右缘 1 列（│ 轨 / █ 拇指）──
        if need_scrollbar {
            let x = area.width - 1;
            let track = colors::FG_MUTED();
            for row in 0..area.height {
                let mut cell = BufCell::new('│');
                cell.fg = Some(track);
                ctx.set(x, row, cell);
            }
            let thumb_h = ((rows * rows) / total).max(1).min(rows) as u16;
            let max_scroll = total - rows;
            let thumb_top = if max_scroll > 0 {
                (scroll_line * (rows - thumb_h as usize)) / max_scroll
            } else {
                0
            } as u16;
            for dy in 0..thumb_h {
                let mut cell = BufCell::new('█');
                cell.fg = Some(colors::E_TEAL());
                ctx.set(x, thumb_top + dy, cell);
            }
        }
    }
}

/// 显示列 → 字符索引（宽字符/tab 感知）：返回累计显示宽不超过 `display_col`
/// 的最后一个字符边界。点击命中的唯一下钻口径（PromptView 渲染同规则）。
fn char_index_at_display_col(line: &str, display_col: usize) -> usize {
    let mut acc = 0usize;
    for (i, ch) in line.chars().enumerate() {
        let cw = match ch {
            '\t' => 4,
            c => unicode_width::UnicodeWidthChar::width(c).unwrap_or(0),
        };
        if acc + cw > display_col {
            return i;
        }
        acc += cw;
    }
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_content_adapts_height_capped_at_max() {
        let mut p = PromptInput::new();
        assert_eq!(p.visible_height(), 1);
        p.set_text("a\nb\nc");
        assert_eq!(p.visible_height(), 3);
        let long = (0..20).map(|i| format!("line{}", i)).collect::<Vec<_>>().join("\n");
        p.set_text(&long);
        assert_eq!(p.visible_height(), MAX_VISIBLE_LINES);
    }

    #[test]
    fn insert_newline_keeps_enter_submit_semantics() {
        let mut p = PromptInput::new();
        p.set_text("hello");
        p.insert_newline();
        p.handle_key(&Key::Char('!'));
        // '!' 非空文本 → 不切 shell，落进编辑器
        assert_eq!(p.text(), "hello\n!");
        match p.handle_key(&Key::Enter) {
            PromptAction::Submit(t) => assert_eq!(t, "hello\n!"),
            other => panic!("expected Submit, got {:?}", other),
        }
        assert_eq!(p.text(), "");
    }

    #[test]
    fn up_down_move_within_multiline_before_history() {
        let mut p = PromptInput::new();
        p.set_text("l1\nl2");
        // cursor 在末行（set_text → move_document_end）
        assert_eq!(p.editor.cursor_position().0, 1);
        p.handle_key(&Key::Up);
        assert_eq!(p.editor.cursor_position().0, 0, "Up 在多行内先行间移动");
        p.handle_key(&Key::Down);
        assert_eq!(p.editor.cursor_position().0, 1);
    }

    #[test]
    fn click_positions_cursor_via_published_geometry() {
        let mut p = PromptInput::new();
        p.set_text("hello\nworld");
        // 模拟 render 发布的几何：内容区 (10, 20)，宽 40，3 行可见
        p.geom.set(Some(PromptViewGeom {
            x: 10, y: 20, width: 40, hit_rows: 3, scroll_line: 0, scroll_col: 0,
        }));
        // 点击第 2 行 "world" 的 'r'（col 2）→ x = 10 + 2(indent) + 2
        assert!(p.handle_click(14, 21));
        assert!(p.is_focused());
        assert_eq!(p.editor.cursor_position(), (1, 2));
        // 点远处 → 失焦
        assert!(!p.handle_click(0, 0));
        assert!(!p.is_focused());
    }

    #[test]
    fn set_cursor_clamps_to_line_end() {
        let mut p = PromptInput::new();
        p.set_text("ab\ncdef");
        p.editor.set_cursor(0, 99);
        assert_eq!(p.editor.cursor_position(), (0, 2));
    }

    #[test]
    fn click_maps_display_col_to_char_index_with_wide_chars() {
        // "你好ab"：显示列 0-1=你 2-3=好 4=a 5=b。点显示列 4 → 字符索引 2。
        let mut p = PromptInput::new();
        p.set_text("你好ab");
        p.geom.set(Some(PromptViewGeom {
            x: 10, y: 20, width: 40, hit_rows: 2, scroll_line: 0, scroll_col: 0,
        }));
        assert!(p.handle_click(10 + PROMPT_INDENT + 4, 20));
        assert_eq!(p.editor.cursor_position(), (0, 2));
        // 点击行尾之外 → 光标落在行尾（字符索引 = 4）
        assert!(p.handle_click(10 + PROMPT_INDENT + 30, 20));
        assert_eq!(p.editor.cursor_position(), (0, 4));
    }

    // ── 滚动窗口渲染回归（12 行滚进 10 行窗）──

    const SCROLL_REPRO_LINES: [&str; 12] = [
        "first", "second", "third", "L4", "L5", "L6",
        "L7", "L8", "L9", "L10", "L11", "L12",
    ];

    fn mk_view(lines: &[&str], cursor: (usize, usize), geom: Rc<Cell<Option<PromptViewGeom>>>) -> PromptView {
        PromptView {
            lines: lines.iter().map(|s| s.to_string()).collect(),
            cursor,
            focused: true,
            cursor_on: false,
            placeholder: String::new(),
            geom,
        }
    }

    fn row_string(buf: &revue::render::Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.get(x, y).map(|c| c.symbol).unwrap_or(' '))
            .collect()
    }

    /// 单帧纯度：12 行内容、光标在末行 → 窗口 [2..12)，每行必须原样
    /// （续行 2 空格缩进，行首无残留字符）。
    #[test]
    fn scrolled_window_renders_lines_verbatim() {
        let geom = Rc::new(Cell::new(None));
        let view = mk_view(&SCROLL_REPRO_LINES, (11, 3), geom);
        let mut buf = revue::render::Buffer::new(30, 10);
        {
            let mut ctx = revue::widget::traits::RenderContext::new(
                &mut buf,
                revue::layout::Rect::new(0, 0, 30, 10),
            );
            view.render(&mut ctx);
        }
        let expected = ["third", "L4", "L5", "L6", "L7", "L8", "L9", "L10", "L11", "L12"];
        // 文本区 = 宽 30 - 滚动条列(29)；逐行 trim_end 后必须恰为 "  <line>"。
        for (row, want) in expected.iter().enumerate() {
            let got = row_string(&buf, row as u16, 29);
            assert_eq!(got.trim_end(), format!("  {}", want), "row {} 渲染错位/残留", row);
        }
    }

    /// 两帧残留回归：帧 1（10 行、scroll_line=0、row0 带 ❯ 箭头）→ 帧 2
    /// （12 行、scroll_line=2），同一 buffer 不清空——复刻 revue 局部脏区渲染
    /// （copy 旧 buffer 只清脏区）。PromptView 自己没绘的 cell（续行缩进、
    /// 行尾余白）不得泄漏上一帧字符。
    #[test]
    fn scrolled_window_does_not_leak_previous_frame_glyphs() {
        let geom = Rc::new(Cell::new(None));
        let mut buf = revue::render::Buffer::new(30, 10);
        // 帧 1：10 行无滚动，row0 = "❯ first"
        {
            let view = mk_view(&SCROLL_REPRO_LINES[..10], (9, 3), geom.clone());
            let mut ctx = revue::widget::traits::RenderContext::new(
                &mut buf,
                revue::layout::Rect::new(0, 0, 30, 10),
            );
            view.render(&mut ctx);
        }
        // 帧 2：12 行滚动窗，buffer 未清空
        {
            let view = mk_view(&SCROLL_REPRO_LINES, (11, 3), geom.clone());
            let mut ctx = revue::widget::traits::RenderContext::new(
                &mut buf,
                revue::layout::Rect::new(0, 0, 30, 10),
            );
            view.render(&mut ctx);
        }
        let expected = ["third", "L4", "L5", "L6", "L7", "L8", "L9", "L10", "L11", "L12"];
        for (row, want) in expected.iter().enumerate() {
            let got = row_string(&buf, row as u16, 29);
            assert_eq!(
                got.trim_end(),
                format!("  {}", want),
                "row {} 泄漏了上一帧字符（实际 {:?}）",
                row,
                got
            );
        }
    }
}
