//! 金 — WrapEditor: soft-wrap（自动折行）编辑器控件。
//!
//! 两层结构（编辑/视图分离，与 scroll_view 的"细化 revue 控件"同宗）：
//!
//! - [`WrapEditor`]：编辑层。持有 revue `TextArea`（编辑权威：cursor/
//!   selection/word navigation）+ render → event 的几何回流通道
//!   （`Rc<Cell<Option<EditorGeom>>>`，`View::render` 只有 &self，与
//!   ScrollableTranscript 的 publish 通道同构）+ 折行布局缓存
//!   （`Rc<RefCell<Option<WrapCache>>>`，布局期/render/Up-Down/点击
//!   共用同一 WrapLayout，金律：渲染/命中/移动同源）。覆盖 prompt 对
//!   TextArea 的全部调用面，含 readline ctrl chord（A/E/W/U/K、词跳）
//!   的多行 kill 实现——undo/redo（^Z/^Y）不在本层：快照栈的权威在
//!   调用方（prompt_input），本层经 [`WrapEditor::snapshot`] /
//!   [`WrapEditor::restore`] 供数，kill 变更前经 `before_change`
//!   回调请调用方记快照（空 kill 不记，粒度可预期）。
//! - [`EditorView`]：视图层。按折行后的视觉行渲染（❯ 箭头、
//!   PROMPT_INDENT 缩进、垂直滚动窗、滚动条、块光标闪烁、全区 fill
//!   防残字），实现 revue `View` trait。长行不再水平滚动——折行布局
//!   的唯一权威是本模块的 [`compute_layout`]（滚动条占列的循环依赖
//!   也在此收敛：视觉行数超 MAX_VISIBLE_LINES 则让出 1 列重排）。
//!
//! 折行口径：unicode-width 感知（宽字符=2 列、tab=4 列，与
//! `display_width_to` 同源），贪心取"放得下的最后一个断词点"（空白/
//! 标点/括号/CJK 过渡处可断），超长无断点词回退按列硬折；空行占一
//! 视觉行。折行边界列（既是上段末也是下段首）归属下一段起点——
//! Up/Down 与点击映射据此钳回，避免光标在边界处"卡住跳行"。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use revue::event::Key;
use revue::render::{Cell as BufCell, Modifier};
use revue::widget::TextArea;
use revue::widget::traits::{RenderContext, View};

use crate::theme::colors;

/// 输入框可见行数上限：超出出滚动条（土律·单点常量）。
pub(crate) const MAX_VISIBLE_LINES: u16 = 10;
/// 首行 ❯ 箭头前缀宽；续行同宽空格缩进对齐。
pub(crate) const PROMPT_INDENT: u16 = 2;

/// Render-time geometry published back to [`WrapEditor`] for mouse
/// hit-testing (absolute screen coords — `ctx.area` is absolute, see
/// `ScrollableTranscript`). Written every frame by `EditorView::render`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EditorGeom {
    /// Left edge of the whole input content area (arrow column included).
    pub x: u16,
    /// Top row of the input content area.
    pub y: u16,
    /// Full width of the content area.
    pub width: u16,
    /// Hit rows: visible text rows + 1 (bottom border line still focuses).
    pub hit_rows: u16,
    /// 视觉行滚动窗（折行口径）——点击 → 光标映射必须读用户看到的
    /// 同一滚动（金律·渲染/命中同源）。
    pub scroll_row: usize,
}

/// Up/Down 视觉行移动结果（AtTop/AtBottom 供调用方进历史导航）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualMove { Moved, AtTop, AtBottom }

// ── 折行布局（soft-wrap 单点权威）──

/// 一个视觉行：逻辑行的一段（char 偏移口径）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WrapLine {
    /// 逻辑行号（内容按 \n 分行）。
    pub logical: usize,
    /// 段首字符在逻辑行内的 char 偏移。
    pub start: usize,
    /// 段尾 char 偏移（不含）；段间连续、合起来覆盖整个逻辑行。
    pub end: usize,
    /// 该段显示宽度（列）。
    pub width: u16,
}

/// 折行布局：内容按显示宽切成的视觉行序列（渲染/命中/移动同源）。
#[derive(Clone, Debug)]
pub(crate) struct WrapLayout {
    pub lines: Vec<WrapLine>,
}

/// 折行布局缓存：按 (内容, 可用文本宽) 失效。布局期（wrapped_height）、
/// render、Up/Down、点击共用——render 侧以 `ctx.area.width` 复核，
/// 不一致即重算（resize 一帧延迟可接受）。
#[derive(Clone, Debug)]
pub(crate) struct WrapCache {
    pub content: String,
    /// 请求侧可用文本宽（已扣 ❯ 缩进，未扣滚动条列）。
    pub area_w: u16,
    /// 实际折行宽（滚动条占列修正后）。
    pub text_w: u16,
    pub layout: WrapLayout,
    pub scrollbar: bool,
}

/// 共享缓存通道（WrapEditor ↔ EditorView；render 只有 &self，RefCell 内部可变）。
pub(crate) type SharedLayout = Rc<RefCell<Option<WrapCache>>>;

/// 单字符显示宽（宽字符=2、tab=4——`display_width_to` 同源口径）。
fn char_width(ch: char) -> u16 {
    match ch {
        '\t' => 4,
        c => unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16,
    }
}

fn is_wide(c: char) -> bool {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) >= 2
}

/// 软折行断点口径：`a` 之后可断 = a 非字母数字（空白/标点/括号/符号），
/// 或 a/b 任一是宽字符（CJK 过渡处可断）。
fn can_break_after(a: char, b: char) -> bool {
    !a.is_alphanumeric() || is_wide(a) || is_wide(b)
}

/// 单个逻辑行的贪心折行：每段取"放得下的最后一个断词点"，无断点硬折。
fn wrap_line(logical: usize, line: &str, w: u16, out: &mut Vec<WrapLine>) {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        // 空行占一视觉行。
        out.push(WrapLine { logical, start: 0, end: 0, width: 0 });
        return;
    }
    let widths: Vec<u16> = chars.iter().map(|&c| char_width(c)).collect();
    let w = w as usize;
    let mut start = 0usize;
    while start < chars.len() {
        let mut acc = 0usize;
        let mut last_break: Option<usize> = None;
        let mut i = start;
        while i < chars.len() {
            let cw = widths[i] as usize;
            if acc + cw > w {
                break;
            }
            acc += cw;
            if i + 1 < chars.len() && can_break_after(chars[i], chars[i + 1]) {
                last_break = Some(i + 1);
            }
            i += 1;
        }
        let end = if i == chars.len() {
            i // 余下全放得下
        } else if let Some(b) = last_break.filter(|&b| b > start) {
            b // 放得下的最后一个断词点
        } else {
            i.max(start + 1) // 超长无断点词：按列硬折（保证前进）
        };
        let width: u16 = widths[start..end].iter().sum();
        out.push(WrapLine { logical, start, end, width });
        start = end;
    }
}

/// 折行布局 + 滚动条占列修正（循环依赖收敛点）：视觉行数超
/// MAX_VISIBLE_LINES 则让出右缘 1 列滚动条后重排。布局期高度与
/// render 共用此函数（土律·单点口径）。
pub(crate) fn compute_layout(content: &str, area_w: u16) -> (WrapLayout, u16, bool) {
    let w = area_w.max(1);
    let mut lines = Vec::new();
    for (logical, line) in content.split('\n').enumerate() {
        wrap_line(logical, line, w, &mut lines);
    }
    if lines.len() > MAX_VISIBLE_LINES as usize {
        let w2 = w.saturating_sub(1).max(1);
        if w2 < w {
            let mut lines = Vec::new();
            for (logical, line) in content.split('\n').enumerate() {
                wrap_line(logical, line, w2, &mut lines);
            }
            return (WrapLayout { lines }, w2, true);
        }
        return (WrapLayout { lines }, w, true);
    }
    (WrapLayout { lines }, w, false)
}

/// 缓存口径的 layout 获取：内容/宽度一致直接命中，否则重算并更新缓存。
fn layout_cached(shared: &SharedLayout, content: &str, area_w: u16) -> WrapCache {
    {
        let borrow = shared.borrow();
        if let Some(c) = borrow.as_ref() {
            if c.content == content && c.area_w == area_w {
                return c.clone();
            }
        }
    }
    let (layout, text_w, scrollbar) = compute_layout(content, area_w);
    let cache = WrapCache { content: content.to_string(), area_w, text_w, layout, scrollbar };
    *shared.borrow_mut() = Some(cache.clone());
    cache
}

impl WrapLayout {
    /// 逻辑 (line, col) → (视觉行, 视觉列)。折行边界列归属下一段起点
    /// （col == seg.end 且非逻辑行末段 → 由下一段以视觉列 0 认领）。
    fn visual_of(&self, lines: &[String], logical: usize, col: usize) -> (usize, usize) {
        let line_text = lines.get(logical).map(|s| s.as_str()).unwrap_or("");
        let line_len = line_text.chars().count();
        let col = col.min(line_len);
        for (i, seg) in self.lines.iter().enumerate() {
            if seg.logical != logical {
                continue;
            }
            let last_of_line = i + 1 >= self.lines.len() || self.lines[i + 1].logical != logical;
            if col < seg.end || (col == seg.end && last_of_line) {
                let vcol: u16 = line_text
                    .chars()
                    .skip(seg.start)
                    .take(col.saturating_sub(seg.start))
                    .map(char_width)
                    .sum();
                return (i, vcol as usize);
            }
        }
        // 逻辑行越界（渲染/内容瞬态不一致）：落到末视觉行，不 panic。
        (self.lines.len().saturating_sub(1), 0)
    }
}

/// 视觉列 → 逻辑 col（段内）。`last_of_line` = 该段是否逻辑行末段：
/// 非末段时 `char_index_at_display_col` 可能返回段尾（边界列，归属
/// 下一段起点）——钳回本段最后一个字符，否则 Up/Down 会在边界处
/// 被下一段认领而"卡住跳行"。
fn col_at_visual_col(line: &str, seg: WrapLine, last_of_line: bool, vcol: usize) -> usize {
    let seg_text: String = line.chars().skip(seg.start).take(seg.end - seg.start).collect();
    let idx = char_index_at_display_col(&seg_text, vcol);
    let seg_len = seg.end - seg.start;
    let idx = if !last_of_line && idx >= seg_len { seg_len - 1 } else { idx };
    seg.start + idx
}

/// 编辑层：多行编辑权威 + 折行布局缓存 + 命中几何回流。
pub struct WrapEditor {
    /// 多行编辑权威（cursor/selection/undo）。内部恒 focused(true)——
    /// 聚焦闸门由调用方承担，text_area 只负责编辑语义。
    text_area: TextArea,
    /// render → event 的几何回流；同时承担视口状态（scroll_row 读
    /// 上一帧几何续算，消除抖动——见 EditorView::render）。
    geom: Rc<Cell<Option<EditorGeom>>>,
    /// 折行布局缓存（布局期/render/Up-Down/点击同源）。
    shared: SharedLayout,
    /// 连续 Up/Down 维持的首选视觉列（desired col）；任何其他编辑/
    /// 光标操作后重置（None）。
    desired_col: Option<usize>,
}

impl Default for WrapEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl WrapEditor {
    pub fn new() -> Self {
        Self {
            text_area: TextArea::new().focused(true),
            geom: Rc::new(Cell::new(None)),
            shared: Rc::new(RefCell::new(None)),
            desired_col: None,
        }
    }

    // ── TextArea 调用面转发 ────────────────────────────────

    pub fn text(&self) -> String { self.text_area.get_content() }
    pub fn set_content(&mut self, text: &str) {
        self.desired_col = None;
        self.text_area.set_content(text);
    }
    pub fn insert_str(&mut self, s: &str) {
        self.desired_col = None;
        self.text_area.insert_str(s);
    }
    pub fn handle_key(&mut self, key: &Key) -> bool {
        self.desired_col = None;
        self.text_area.handle_key(key)
    }
    /// 换行（Enter 键语义转发 TextArea；提交路由不归本层）。
    pub fn insert_newline(&mut self) {
        self.desired_col = None;
        self.text_area.handle_key(&Key::Enter);
    }
    pub fn cursor_position(&self) -> (usize, usize) { self.text_area.cursor_position() }
    pub fn set_cursor(&mut self, line: usize, col: usize) {
        self.desired_col = None;
        self.text_area.set_cursor(line, col);
    }
    pub fn line_count(&self) -> usize { self.text_area.line_count() }
    pub fn move_document_end(&mut self) {
        self.desired_col = None;
        self.text_area.move_document_end();
    }

    // ── 折行布局 ─────────────────────────────────────────

    /// 布局期高度：按可用文本宽（已扣 ❯ 缩进）折行后的可见行数
    /// （clamp 1..=MAX_VISIBLE_LINES；同帧 render 经缓存复用此布局）。
    pub fn wrapped_height(&self, text_w: u16) -> u16 {
        let content = self.text_area.get_content();
        let cache = layout_cached(&self.shared, &content, text_w);
        (cache.layout.lines.len() as u16).clamp(1, MAX_VISIBLE_LINES)
    }

    /// 当前折行布局：优先渲染/布局期缓存（与可视宽同口径）；无缓存
    /// （尚未渲染/布局过）回退不折行（视觉行 == 逻辑行）。
    fn current_layout(&self, content: &str) -> WrapLayout {
        {
            let borrow = self.shared.borrow();
            if let Some(c) = borrow.as_ref() {
                if c.content == content {
                    return c.layout.clone();
                }
            }
        }
        compute_layout(content, u16::MAX).0
    }

    // ── 快照（外层 undo 权威经此取数/回放）──────────────────

    /// 内容 + 光标快照。
    pub fn snapshot(&self) -> (String, (usize, usize)) {
        (self.text(), self.cursor_position())
    }

    /// 回放快照（undo/redo 恢复）。
    pub fn restore(&mut self, snap: &(String, (usize, usize))) {
        self.desired_col = None;
        self.text_area.set_content(&snap.0);
        self.text_area.set_cursor(snap.1 .0, snap.1 .1);
    }

    // ── readline ctrl 集 ──────────────────────────────────

    /// Ctrl 组合键（readline 集，U2）：agendao 自实现——revue 是第三方库
    /// 不可改，TextArea 又无 ctrl 路由，故 readline chord（A/E/W/U/K、
    /// 词跳）在此以公开 API 组合（readline.rs 同宗，Input/TextArea 各一份）。
    /// ^Z/^Y 不在此绑定：undo 快照栈权威在调用方（prompt_input），由其
    /// 自行路由。变更类 chord（kill）动手前调 `before_change(self)` 请
    /// 调用方记快照（空 kill 不触发）。
    /// 返回 true=已消费；未绑定 chord 返回 false（调用方吞掉，
    /// 绝不剥修饰键退化成插入字面字母）。
    pub fn handle_ctrl_key(
        &mut self,
        event: &revue::event::KeyEvent,
        mut before_change: impl FnMut(&Self),
    ) -> bool {
        use revue::event::Key as K;
        self.desired_col = None;
        match event.key {
            K::Char('a') => self.text_area.move_home(),
            K::Char('e') => self.text_area.move_end(),
            K::Left => self.text_area.move_word_left(),
            K::Right => self.text_area.move_word_right(),
            K::Char('w') | K::Backspace => self.kill_word_before_cursor(&mut before_change),
            K::Char('u') => self.kill_to_line_start(&mut before_change),
            K::Char('k') => self.kill_to_line_end(&mut before_change),
            _ => return false,
        }
        true
    }

    /// 多行 kill 的公共骨架：把内容按 char 线性索引切掉 [from, to)，
    /// 光标落到 from（经 (line,col) 换算，readline.rs 单点工具）。
    fn kill_range(&mut self, from: usize, to: usize, before_change: &mut impl FnMut(&Self)) {
        if from >= to {
            return;
        }
        before_change(self);
        let content = self.text_area.get_content();
        let mut chars: Vec<char> = content.chars().collect();
        let to = to.min(chars.len());
        let from = from.min(to);
        chars.drain(from..to);
        let new: String = chars.into_iter().collect();
        let (line, col) = crate::input::readline::linear_to_line_col(&new, from);
        self.text_area.set_content(&new);
        self.text_area.set_cursor(line, col);
    }

    fn cursor_linear(&self) -> usize {
        let (line, col) = self.text_area.cursor_position();
        crate::input::readline::line_col_to_linear(&self.text_area.get_content(), line, col)
    }

    /// Ctrl+W：删光标前一词（readline 口径：先空白后非空白）。
    fn kill_word_before_cursor(&mut self, before_change: &mut impl FnMut(&Self)) {
        let cursor = self.cursor_linear();
        let start = crate::input::readline::word_start_before(&self.text_area.get_content(), cursor);
        self.kill_range(start, cursor, before_change);
    }

    /// Ctrl+U：删到行首（多行时只动当前行，readline 口径）。
    fn kill_to_line_start(&mut self, before_change: &mut impl FnMut(&Self)) {
        let (line, _) = self.text_area.cursor_position();
        let start = crate::input::readline::line_col_to_linear(&self.text_area.get_content(), line, 0);
        self.kill_range(start, self.cursor_linear(), before_change);
    }

    /// Ctrl+K：删到行尾（多行时只动当前行）。
    fn kill_to_line_end(&mut self, before_change: &mut impl FnMut(&Self)) {
        let (line, _) = self.text_area.cursor_position();
        let content = self.text_area.get_content();
        let line_len = content.split('\n').nth(line).map(|l| l.chars().count()).unwrap_or(0);
        let end = crate::input::readline::line_col_to_linear(&content, line, line_len);
        self.kill_range(self.cursor_linear(), end, before_change);
    }

    // ── Up/Down（视觉行化）────────────────────────────────

    /// Up：按折行布局回上一视觉行（不代理 TextArea::move_up——多行
    /// 内行间与折行段间统一为视觉行坐标）。连续 Up/Down 维持首选
    /// 视觉列（desired col）。返回 AtTop = 已在首视觉行（调用方进
    /// 历史）；Moved = 光标已定位。
    pub fn move_visual_up(&mut self) -> VisualMove {
        let content = self.text_area.get_content();
        let lines: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
        let layout = self.current_layout(&content);
        let (line, col) = self.text_area.cursor_position();
        let (vrow, vcol) = layout.visual_of(&lines, line, col);
        let desired = self.desired_col.unwrap_or(vcol);
        if vrow == 0 {
            return VisualMove::AtTop;
        }
        let seg = layout.lines[vrow - 1];
        // vrow 是光标本行（有效索引），其是否同逻辑行的续段决定 seg 是否末段。
        let last_of_line = layout.lines[vrow].logical != seg.logical;
        let col = col_at_visual_col(&lines[seg.logical], seg, last_of_line, desired);
        self.text_area.set_cursor(seg.logical, col);
        self.desired_col = Some(desired);
        VisualMove::Moved
    }

    /// Down：按折行布局进下一视觉行；返回 AtBottom = 已在末视觉行
    /// （调用方进历史/草稿）；Moved = 光标已定位。
    pub fn move_visual_down(&mut self) -> VisualMove {
        let content = self.text_area.get_content();
        let lines: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
        let layout = self.current_layout(&content);
        let (line, col) = self.text_area.cursor_position();
        let (vrow, vcol) = layout.visual_of(&lines, line, col);
        let desired = self.desired_col.unwrap_or(vcol);
        if vrow + 1 >= layout.lines.len() {
            return VisualMove::AtBottom;
        }
        let seg = layout.lines[vrow + 1];
        let last_of_line = vrow + 2 >= layout.lines.len()
            || layout.lines[vrow + 2].logical != seg.logical;
        let col = col_at_visual_col(&lines[seg.logical], seg, last_of_line, desired);
        self.text_area.set_cursor(seg.logical, col);
        self.desired_col = Some(desired);
        VisualMove::Moved
    }

    // ── 命中 & 视图 ───────────────────────────────────────

    /// Handle a mouse click at (x, y) — absolute screen coords.
    /// 命中区来自 render 发布的真实几何（替代旧 y>=35 硬编码）：
    /// 命中 → y 经视觉行 → 折行段换算逻辑 (line, col) 并返回 true
    /// （聚焦与否由调用方裁决）；未命中 → false。
    pub fn handle_click(&mut self, x: u16, y: u16) -> bool {
        self.desired_col = None;
        if let Some(g) = self.geom.get() {
            if y >= g.y && y < g.y + g.hit_rows && x >= g.x && x < g.x + g.width {
                let content = self.text_area.get_content();
                let lines: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
                let layout = self.current_layout(&content);
                let vrow = g.scroll_row + (y - g.y) as usize;
                // 命中区含内容下的空行（底边框行仍聚焦）：落到末段行尾。
                let (seg, last_of_line, at_end) = match layout.lines.get(vrow) {
                    Some(seg) => {
                        let last = vrow + 1 >= layout.lines.len()
                            || layout.lines[vrow + 1].logical != seg.logical;
                        (*seg, last, false)
                    }
                    None => match layout.lines.last() {
                        Some(seg) => (*seg, true, true),
                        None => return true, // layout 恒非空（空内容也有一行），防御
                    },
                };
                let col = if at_end {
                    seg.end
                } else {
                    // x → 显示列（❯ 缩进左侧视为段首）。
                    let dx = if x >= g.x + PROMPT_INDENT {
                        (x - g.x - PROMPT_INDENT) as usize
                    } else {
                        0
                    };
                    col_at_visual_col(&lines[seg.logical], seg, last_of_line, dx)
                };
                self.text_area.set_cursor(seg.logical, col);
                return true;
            }
        }
        false
    }

    /// Snapshot a renderable view of the editor.
    /// `cursor_on` = 闪烁相（app 层 blink tick 推导）&& 希望画光标；
    /// `focused` / `placeholder` 由调用方持有（聚焦闸门、placeholder
    /// 口径不归本层）。
    pub fn view(&self, cursor_on: bool, focused: bool, placeholder: String) -> EditorView {
        EditorView {
            lines: self.text_area.get_content().split('\n').map(|s| s.to_string()).collect(),
            cursor: self.text_area.cursor_position(),
            focused,
            cursor_on,
            placeholder,
            geom: self.geom.clone(),
            shared: self.shared.clone(),
        }
    }
}

// ── 金：EditorView — 多行输入框渲染（❯ 箭头 + 缩进 + 滚动条 + 闪烁光标）──

pub struct EditorView {
    lines: Vec<String>,
    /// (line, col) 字符坐标（逻辑口径；视觉位置经折行布局换算）。
    cursor: (usize, usize),
    focused: bool,
    /// 本帧是否画块光标（focused && blink 相位亮）。
    cursor_on: bool,
    placeholder: String,
    geom: Rc<Cell<Option<EditorGeom>>>,
    shared: SharedLayout,
}

impl View for EditorView {
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
        // 折行布局：以本帧实际宽度复核缓存（resize 一帧延迟可接受）。
        let content = self.lines.join("\n");
        let cache = layout_cached(&self.shared, &content, area.width.saturating_sub(PROMPT_INDENT));
        let layout = &cache.layout;
        let text_w = cache.text_w;
        let need_scrollbar = cache.scrollbar;
        let total = layout.lines.len();

        // 光标视觉坐标（折行边界列归属下一段起点）。
        let (cursor_vrow, _) = layout.visual_of(&self.lines, self.cursor.0, self.cursor.1);

        // ── 垂直滚动窗：跟随光标视觉行（读上一帧几何，消除抖动）──
        let prev = self.geom.get();
        let mut scroll_row = prev.map(|g| g.scroll_row).unwrap_or(0);
        if cursor_vrow < scroll_row {
            scroll_row = cursor_vrow;
        } else if cursor_vrow >= scroll_row + rows {
            scroll_row = cursor_vrow + 1 - rows;
        }

        // ── 几何发布（命中同源）──
        self.geom.set(Some(EditorGeom {
            x: area.x,
            y: area.y,
            width: area.width,
            hit_rows: area.height + 1, // +1 底边框行点击仍聚焦
            scroll_row,
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
        if scroll_row == 0 && rows > 0 {
            let mut cell = BufCell::new('❯');
            cell.fg = Some(colors::E_TEAL());
            ctx.set(0, 0, cell);
        }

        // ── 视觉行 ──
        for row in 0..rows {
            let v = scroll_row + row;
            if v >= total {
                break;
            }
            let y = row as u16;
            let seg = &layout.lines[v];
            let line = &self.lines[seg.logical];

            let mut x: u16 = 0;
            for (ci, ch) in line.chars().enumerate().skip(seg.start).take(seg.end - seg.start) {
                let cw = char_width(ch);
                if x + cw > text_w {
                    break;
                }
                let is_cursor = self.cursor_on
                    && self.cursor.0 == seg.logical
                    && self.cursor.1 == ci;
                let draw = if ch == '\t' { ' ' } else { ch };
                let mut cell = BufCell::new(draw);
                if is_cursor {
                    cell.bg = Some(revue::prelude::Color::WHITE);
                    cell.modifier = Modifier::BOLD;
                } else {
                    cell.fg = Some(colors::FG_PRIMARY());
                }
                ctx.set(PROMPT_INDENT + x, y, cell);
                // tab 补齐剩余空格
                for pad in 1..cw {
                    if x + pad < text_w {
                        let sp = BufCell::new(' ');
                        ctx.set(PROMPT_INDENT + x + pad, y, sp);
                    }
                }
                x += cw;
            }

            // 光标在逻辑行尾（含空行）：本段是该逻辑行末段才画空白块光标
            // （折行边界列归下一段首字符认领，见 visual_of 口径）。
            let last_of_line = v + 1 >= total || layout.lines[v + 1].logical != seg.logical;
            if self.cursor_on
                && last_of_line
                && self.cursor.0 == seg.logical
                && self.cursor.1 >= line.chars().count()
            {
                let x = seg.width;
                if x < text_w {
                    let mut cell = BufCell::new(' ');
                    cell.bg = Some(revue::prelude::Color::WHITE);
                    cell.modifier = Modifier::BOLD;
                    ctx.set(PROMPT_INDENT + x, y, cell);
                }
            }
        }

        // ── 滚动条：视觉行超窗时右缘 1 列（│ 轨 / █ 拇指）──
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
                (scroll_row * (rows - thumb_h as usize)) / max_scroll
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
/// 的最后一个字符边界。点击命中与视觉列映射的唯一下钻口径（EditorView
/// 渲染同规则）。
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
    fn set_cursor_clamps_to_line_end() {
        let mut e = WrapEditor::new();
        e.set_content("ab\ncdef");
        e.set_cursor(0, 99);
        assert_eq!(e.cursor_position(), (0, 2));
    }

    // ── 折行布局 ─────────────────────────────────────────

    #[test]
    fn hard_breaks_long_word_without_break_points() {
        let (layout, text_w, scrollbar) = compute_layout(&"a".repeat(20), 8);
        assert!(!scrollbar);
        assert_eq!(text_w, 8);
        let segs: Vec<(usize, usize, u16)> =
            layout.lines.iter().map(|s| (s.start, s.end, s.width)).collect();
        assert_eq!(segs, vec![(0, 8, 8), (8, 16, 8), (16, 20, 4)]);
        assert!(layout.lines.iter().all(|s| s.logical == 0));
    }

    #[test]
    fn breaks_at_whitespace_before_hard_break() {
        // "foo bar" 宽 6：整词放不下 → 在空格后断（"foo " / "bar"），不硬折。
        let (layout, _, _) = compute_layout("foo bar", 6);
        let segs: Vec<(usize, usize)> = layout.lines.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(segs, vec![(0, 4), (4, 7)]);
    }

    #[test]
    fn cjk_wrap_never_splits_wide_char() {
        // "你好世界"（每字 2 列）宽 5：4 列放下两字 → 断在字界，不错列。
        let (layout, _, _) = compute_layout("你好世界", 5);
        let segs: Vec<(usize, usize, u16)> =
            layout.lines.iter().map(|s| (s.start, s.end, s.width)).collect();
        assert_eq!(segs, vec![(0, 2, 4), (2, 4, 4)]);
        // CJK→ASCII 过渡可断："你好ab" 宽 4 → "你好" / "ab"。
        let (layout, _, _) = compute_layout("你好ab", 4);
        let segs: Vec<(usize, usize)> = layout.lines.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(segs, vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn empty_line_occupies_one_visual_row() {
        let (layout, _, _) = compute_layout("a\n\nb", 10);
        assert_eq!(layout.lines.len(), 3);
        assert_eq!(layout.lines[1], WrapLine { logical: 1, start: 0, end: 0, width: 0 });
    }

    #[test]
    fn scrollbar_column_shrink_rewraps() {
        // 11 个短逻辑行：视觉行 > MAX_VISIBLE_LINES → 让出 1 列滚动条重排。
        let content = (0..11).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        let (layout, text_w, scrollbar) = compute_layout(&content, 10);
        assert!(scrollbar);
        assert_eq!(text_w, 9);
        assert_eq!(layout.lines.len(), 11);
    }

    #[test]
    fn width_change_reflows_via_cache() {
        let mut e = WrapEditor::new();
        e.set_content("aaaa bbbb cccc");
        // 宽 14：整行放得下 → 1 视觉行。
        assert_eq!(e.wrapped_height(14), 1);
        // 宽 6：断词点折成 3 视觉行（缓存按宽度失效重排）。
        assert_eq!(e.wrapped_height(6), 3);
        assert_eq!(e.wrapped_height(14), 1, "宽度回变同样重排");
    }

    #[test]
    fn wrapped_height_grows_with_content_and_caps_at_max() {
        let mut e = WrapEditor::new();
        e.set_content("a\nb\nc");
        assert_eq!(e.wrapped_height(20), 3);
        // 单行按宽折行也撑高：24 列内容宽 10 → 3 视觉行。
        e.set_content(&"a".repeat(24));
        assert_eq!(e.wrapped_height(10), 3);
        // 超 MAX 封顶。
        e.set_content(&(0..30).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n"));
        assert_eq!(e.wrapped_height(20), MAX_VISIBLE_LINES);
    }

    // ── Up/Down（视觉行化）────────────────────────────────

    #[test]
    fn visual_up_down_walk_wrapped_segments() {
        let mut e = WrapEditor::new();
        e.set_content(&"a".repeat(24)); // 1 逻辑行，宽 10 → 3 视觉行
        e.wrapped_height(10); // 布局期 prime 折行缓存
        e.set_cursor(0, 24); // 文档末 = 视觉行 2
        assert_eq!(e.move_visual_up(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (0, 14), "折行中段 Up 回上一视觉行");
        assert_eq!(e.move_visual_up(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (0, 4));
        assert_eq!(e.move_visual_up(), VisualMove::AtTop, "首视觉行才 AtTop");
        assert_eq!(e.move_visual_down(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (0, 14));
        assert_eq!(e.move_visual_down(), VisualMove::Moved);
        assert_eq!(e.move_visual_down(), VisualMove::AtBottom);
    }

    #[test]
    fn visual_up_down_keeps_desired_col_and_unsticks_boundary() {
        let mut e = WrapEditor::new();
        // 行 0 "aaaa"（恰满 4 列）；行 1 "aaa bbb" 宽 4 折成 "aaa " / "bbb"。
        e.set_content("aaaa\naaa bbb");
        e.wrapped_height(4);
        e.set_cursor(0, 4); // 行 0 行尾：视觉行 0、视觉列 4
        // Down → desired=4 超过 "aaa " 段内可放位置：段尾是折行边界
        // （归下一段起点）——必须钳回本段，不得被下一段认领卡住。
        assert_eq!(e.move_visual_down(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (1, 3), "边界列钳回本段最后一字符");
        // 继续 Down（desired 仍 4）→ 下一段 "bbb" 视觉列 4 → 段尾（逻辑行末段）。
        assert_eq!(e.move_visual_down(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (1, 7));
        // Up 回上一段（desired col 跨段保持，仍钳边界）……
        assert_eq!(e.move_visual_up(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (1, 3));
        // ……再 Up 回到行 0 行尾（恢复 desired col）。
        assert_eq!(e.move_visual_up(), VisualMove::Moved);
        assert_eq!(e.cursor_position(), (0, 4), "回到行 0 时恢复 desired col");
        // 普通编辑重置 desired col。
        e.handle_key(&Key::Char('x'));
        assert_eq!(e.move_visual_down(), VisualMove::Moved);
    }

    // ── 点击命中（折行口径）────────────────────────────────

    #[test]
    fn click_positions_cursor_via_published_geometry() {
        let mut e = WrapEditor::new();
        e.set_content("hello\nworld");
        e.move_document_end();
        // 模拟 render 发布的几何：内容区 (10, 20)，宽 40，3 行可见
        e.geom.set(Some(EditorGeom {
            x: 10, y: 20, width: 40, hit_rows: 3, scroll_row: 0,
        }));
        // 点击第 2 行 "world" 的 'r'（col 2）→ x = 10 + 2(indent) + 2
        assert!(e.handle_click(14, 21));
        assert_eq!(e.cursor_position(), (1, 2));
        // 未命中 → false，光标不动
        assert!(!e.handle_click(0, 0));
        assert_eq!(e.cursor_position(), (1, 2));
    }

    #[test]
    fn click_maps_display_col_to_char_index_with_wide_chars() {
        // "你好ab"：显示列 0-1=你 2-3=好 4=a 5=b。点显示列 4 → 字符索引 2。
        let mut e = WrapEditor::new();
        e.set_content("你好ab");
        e.geom.set(Some(EditorGeom {
            x: 10, y: 20, width: 40, hit_rows: 2, scroll_row: 0,
        }));
        assert!(e.handle_click(10 + PROMPT_INDENT + 4, 20));
        assert_eq!(e.cursor_position(), (0, 2));
        // 点击行尾之外 → 光标落在行尾（字符索引 = 4）
        assert!(e.handle_click(10 + PROMPT_INDENT + 30, 20));
        assert_eq!(e.cursor_position(), (0, 4));
    }

    #[test]
    fn click_maps_through_wrap_layout() {
        let mut e = WrapEditor::new();
        e.set_content("aaaa bbbb"); // 宽 6 → "aaaa " / "bbbb"
        e.wrapped_height(6);
        e.geom.set(Some(EditorGeom {
            x: 10, y: 20, width: 40, hit_rows: 3, scroll_row: 0,
        }));
        // 点视觉行 1 的第 2 显示列 → 逻辑 (0, 5+2=7)。
        assert!(e.handle_click(10 + PROMPT_INDENT + 2, 21));
        assert_eq!(e.cursor_position(), (0, 7));
        // 点内容下的空行（命中区内）→ 末段行尾。
        assert!(e.handle_click(10 + PROMPT_INDENT, 22));
        assert_eq!(e.cursor_position(), (0, 9));
    }

    // ── 滚动窗口渲染回归（12 行滚进 10 行窗）──

    const SCROLL_REPRO_LINES: [&str; 12] = [
        "first", "second", "third", "L4", "L5", "L6",
        "L7", "L8", "L9", "L10", "L11", "L12",
    ];

    fn mk_view(lines: &[&str], cursor: (usize, usize), geom: Rc<Cell<Option<EditorGeom>>>) -> EditorView {
        EditorView {
            lines: lines.iter().map(|s| s.to_string()).collect(),
            cursor,
            focused: true,
            cursor_on: false,
            placeholder: String::new(),
            geom,
            shared: Rc::new(RefCell::new(None)),
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

    /// 两帧残留回归：帧 1（10 行、scroll_row=0、row0 带 ❯ 箭头）→ 帧 2
    /// （12 行、scroll_row=2），同一 buffer 不清空——复刻 revue 局部脏区渲染
    /// （copy 旧 buffer 只清脏区）。EditorView 自己没绘的 cell（续行缩进、
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

    /// 折行渲染回归：长逻辑行在 render 侧按视觉行折行（不再水平滚动），
    /// 滚动条列让出后重排不错列。
    #[test]
    fn long_line_wraps_instead_of_horizontal_scroll() {
        let geom = Rc::new(Cell::new(None));
        let view = mk_view(&[&"a".repeat(12)], (0, 12), geom.clone());
        // 内容宽 8 - PROMPT_INDENT 2 = 6 → 折成 "aaaaaa" / "aaaaaa" 两视觉行。
        let mut buf = revue::render::Buffer::new(8, 3);
        {
            let mut ctx = revue::widget::traits::RenderContext::new(
                &mut buf,
                revue::layout::Rect::new(0, 0, 8, 3),
            );
            view.render(&mut ctx);
        }
        // 首行带 ❯ 箭头，续行 2 空格缩进；两视觉行各 6 个 a（宽 8-2=6 硬折）。
        assert_eq!(row_string(&buf, 0, 8).trim_end(), "❯ aaaaaa");
        assert_eq!(row_string(&buf, 1, 8).trim_end(), "  aaaaaa");
        // 光标在逻辑行尾 → 末段行尾块光标（cursor_on=false 时不画，此处只验文本）。
        assert_eq!(geom.get().unwrap().scroll_row, 0);
    }

    /// 光标随折行定位：光标逻辑 col 落在第二折行段时，块光标画在视觉行 1。
    #[test]
    fn cursor_renders_on_wrapped_visual_row() {
        let geom = Rc::new(Cell::new(None));
        let mut view = mk_view(&[&"a".repeat(12)], (0, 8), geom);
        view.cursor_on = true;
        let mut buf = revue::render::Buffer::new(8, 3);
        {
            let mut ctx = revue::widget::traits::RenderContext::new(
                &mut buf,
                revue::layout::Rect::new(0, 0, 8, 3),
            );
            view.render(&mut ctx);
        }
        // 视觉行 1 "aaaaaa"，光标 col 8 = 段内第 2 字符 → 块光标在 x=2+2=4。
        let cell = buf.get(2 + 2, 1).expect("cell");
        assert_eq!(cell.bg, Some(revue::prelude::Color::WHITE));
        assert_eq!(cell.symbol, 'a');
    }
}
