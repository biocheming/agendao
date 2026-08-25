//! 金 — Markdown rendering backed by `ratatui-markdown`.
//!
//! revue's built-in markdown widget uses pulldown-cmark directly but its
//! table rendering is essentially a no-op (it only sets `in_table = true`
//! without drawing any borders), and code blocks have a hard-coded 30-char
//! border width.  ratatui-markdown gives us:
//!
//! - Unicode box-drawing tables (┌─┬─┐ / ├─┼─┤ / └─┴─┘)
//! - CJK-aware text wrapping
//! - Custom render hooks for every element type
//! - Adaptive code-block borders
//!
//! This module converts ratatui-markdown's output (`ratatui::text::Line`)
//! into revue cells so the rest of the TUI doesn't need to know about
//! ratatui at all.

use ratatui_markdown::markdown::MarkdownRenderer;
use revue::prelude::Color as RevueColor;
use revue::render::{Cell, Modifier};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::Hasher;
use std::sync::Arc;
use unicode_width::UnicodeWidthChar;

// ── Color conversion ──────────────────────────────────────────

/// Convert a ratatui `Color` into a revue `Color`.
fn convert_color(c: ratatui::style::Color) -> RevueColor {
    use ratatui::style::Color;
    match c {
        Color::Reset => RevueColor::TRANSPARENT,
        // ANSI 16-color palette → approximate RGB
        Color::Black => RevueColor::rgb(0, 0, 0),
        Color::Red => RevueColor::rgb(205, 0, 0),
        Color::Green => RevueColor::rgb(0, 205, 0),
        Color::Yellow => RevueColor::rgb(205, 205, 0),
        Color::Blue => RevueColor::rgb(0, 0, 238),
        Color::Magenta => RevueColor::rgb(205, 0, 205),
        Color::Cyan => RevueColor::rgb(0, 205, 205),
        Color::Gray => RevueColor::rgb(229, 229, 229),
        Color::DarkGray => RevueColor::rgb(127, 127, 127),
        Color::LightRed => RevueColor::rgb(255, 0, 0),
        Color::LightGreen => RevueColor::rgb(0, 255, 0),
        Color::LightYellow => RevueColor::rgb(255, 255, 0),
        Color::LightBlue => RevueColor::rgb(92, 92, 255),
        Color::LightMagenta => RevueColor::rgb(255, 0, 255),
        Color::LightCyan => RevueColor::rgb(0, 255, 255),
        Color::White => RevueColor::rgb(255, 255, 255),
        Color::Rgb(r, g, b) => RevueColor::rgb(r, g, b),
        Color::Indexed(i) => {
            // Fallback: use the index as a gray value
            RevueColor::rgb(i, i, i)
        }
    }
}

// ── Modifier conversion ───────────────────────────────────────

/// Convert ratatui modifiers into revue modifiers.
fn convert_modifier(m: ratatui::style::Modifier) -> Modifier {
    let mut out = Modifier::empty();
    if m.contains(ratatui::style::Modifier::BOLD) {
        out |= Modifier::BOLD;
    }
    if m.contains(ratatui::style::Modifier::ITALIC) {
        out |= Modifier::ITALIC;
    }
    if m.contains(ratatui::style::Modifier::UNDERLINED) {
        out |= Modifier::UNDERLINE;
    }
    if m.contains(ratatui::style::Modifier::DIM) {
        out |= Modifier::DIM;
    }
    if m.contains(ratatui::style::Modifier::CROSSED_OUT) {
        out |= Modifier::CROSSED_OUT;
    }
    if m.contains(ratatui::style::Modifier::REVERSED) {
        out |= Modifier::REVERSE;
    }
    out
}

// ── Line → cells ──────────────────────────────────────────────

/// Convert a single `ratatui::text::Line` into a vector of revue `Cell`s.
///
/// CJK characters and emojis occupy 2 columns: revue's buffer convention
/// requires the first column to hold the symbol cell and the second column
/// to hold a `Cell::continuation()` (symbol == `'\0'`). Without the
/// continuation marker, downstream rendering (and any subsequent
/// `set(x, y, ...)` at the second column) would misalign — which manifests
/// as garbled CJK output where every other character is overwritten by the
/// right half of its neighbour.
///
/// Zero-width chars (combining marks, ZWJ, etc.) are skipped entirely.
pub fn line_to_cells(line: &ratatui::text::Line) -> Vec<Cell> {
    let mut cells = Vec::new();
    for span in &line.spans {
        let fg = span.style.fg.map(convert_color);
        let bg = span.style.bg.map(convert_color);
        let modifier = convert_modifier(span.style.add_modifier);
        for ch in span.content.chars() {
            if ch == '\n' {
                continue; // newlines are line separators, not cells
            }
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if w == 0 {
                // Zero-width / combining / control char: drop it
                // (revue's buffer has no slot for combining marks).
                continue;
            }
            let mut cell = Cell::new(ch);
            if let Some(c) = fg {
                cell.fg = Some(c);
            }
            if let Some(c) = bg {
                cell.bg = Some(c);
            }
            cell.modifier = modifier;
            cells.push(cell);
            // Wide char: emit continuation cells for columns 2..w
            for _ in 1..w {
                let mut cont = Cell::continuation();
                if let Some(c) = bg {
                    cont.bg = Some(c);
                }
                cells.push(cont);
            }
        }
    }
    cells
}

/// Convert a slice of `ratatui::text::Line`s into a flat vec of cells,
/// one row per line (padded to `max_width`).
///
/// `line_to_cells` already emits continuation cells for wide chars, so
/// `cells.len()` equals the visual column count and indexing into the
/// row offset is column-correct.
///
/// Returns `(cells, row_count)` where `cells.len() == row_count * max_width`.
pub fn lines_to_cell_grid(lines: &[ratatui::text::Line], max_width: u16) -> (Vec<Cell>, u16) {
    let w = max_width as usize;
    let row_count = lines.len() as u16;
    let mut grid = vec![Cell::empty(); row_count as usize * w];
    for (y, line) in lines.iter().enumerate() {
        let row_cells = line_to_cells(line);
        let row_offset = y * w;
        let mut x = 0usize;
        while x < row_cells.len() && x < w {
            let cell = row_cells[x];
            let next_is_cont = row_cells
                .get(x + 1)
                .map(|c| c.is_continuation())
                .unwrap_or(false);
            if next_is_cont && x + 1 >= w {
                break; // wide char wouldn't fit
            }
            grid[row_offset + x] = cell;
            x += 1;
        }
    }
    (grid, row_count)
}

// ── Render cache ─────────────────────────────────────────────
//
// `MarkdownRenderer::parse` + `render` are pure: the output `Vec<Line>`
// depends only on (text, width) — `NoopTheme` is constant and the parser
// is width-independent (width only affects `render`'s wrapping).  During
// streaming the TUI re-renders at ~20fps and rebuilds the view tree every
// frame, so the same (often large) markdown text was parsed+rendered
// dozens of times per second.  This thread-local LRU memoizes the final
// rendered lines keyed by content hash + width: identical text renders
// once, streaming appends change the hash and naturally re-render.
//
// thread_local (not a global Mutex): the TUI is single-threaded, so this
// avoids locking entirely; tests get an isolated cache per test thread.

/// Max cached (text, width) entries.  A transcript rarely shows more than
/// a handful of distinct markdown blocks in the viewport; 64 leaves ample
/// headroom while bounding memory (each entry holds its rendered lines).
const RENDER_CACHE_CAP: usize = 64;

/// Cache key: hash of the markdown text + normalized render width.
type CacheKey = (u64, u16, bool);

struct CacheEntry {
    /// Original text, kept to verify hash hits (guard against collisions).
    text: Arc<str>,
    /// Rendered output, shared with every consumer via `Arc`.
    lines: Arc<Vec<ratatui::text::Line<'static>>>,
}

struct RenderCache {
    map: HashMap<CacheKey, CacheEntry>,
    /// Recency order, front = least recently used.
    lru: VecDeque<CacheKey>,
    /// Total parse+render executions (misses).  Test instrumentation.
    misses: u64,
}

thread_local! {
    static RENDER_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache {
        map: HashMap::new(),
        lru: VecDeque::new(),
        misses: 0,
    });
}

fn hash_text(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(text.as_bytes());
    h.finish()
}

/// Parse + render `text` at `width`, memoized by content hash.
///
/// Returns a shared `Arc` — cache hits cost one hash + map lookup, no
/// re-parse and no line cloning.
fn render_lines_cached_with_streaming(
    text: &Arc<str>,
    width: u16,
    streaming: bool,
) -> Arc<Vec<ratatui::text::Line<'static>>> {
    let width = width.max(20);
    let key = (hash_text(text), width, streaming);
    RENDER_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let hit = c.map.get(&key).and_then(|entry| {
            (entry.text.as_ref() == text.as_ref()).then(|| Arc::clone(&entry.lines))
        });
        if let Some(lines) = hit {
            // Refresh recency (n ≤ 64, linear reposition is fine).
            if let Some(pos) = c.lru.iter().position(|k| *k == key) {
                let k = c.lru.remove(pos).unwrap();
                c.lru.push_back(k);
            }
            return lines;
        }
        c.misses += 1;
        let safe_text = if streaming {
            normalize_streaming_markdown(text)
        } else {
            text.to_string()
        };
        let renderer = MarkdownRenderer::new(width as usize);
        let blocks = renderer.parse(&safe_text);
        let lines = Arc::new(renderer.render(&blocks, &NoopTheme));
        if c.map.len() >= RENDER_CACHE_CAP {
            if let Some(evict) = c.lru.pop_front() {
                c.map.remove(&evict);
            }
        }
        c.lru.push_back(key);
        c.map.insert(
            key,
            CacheEntry {
                text: Arc::clone(text),
                lines: Arc::clone(&lines),
            },
        );
        lines
    })
}

/// Production TUI stream tolerance: close an unmatched fenced block for the
/// current render pass only. The source text remains untouched; when a later
/// delta supplies the real closing fence, its distinct cache key re-renders
/// canonical markdown. This prevents an open fence from swallowing the rest
/// of the transcript frame.
fn normalize_streaming_markdown(text: &str) -> String {
    let fence_count = text
        .lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .count();
    let mut out = if fence_count % 2 == 1 {
        format!("{text}\n```")
    } else {
        text.to_owned()
    };
    let lines: Vec<&str> = out.lines().collect();
    if lines.len() >= 2 {
        let header = lines[lines.len() - 2].trim();
        let tail = lines[lines.len() - 1].trim();
        let is_table = header.starts_with('|') && header.ends_with('|') && tail.starts_with('|');
        if is_table && !tail.ends_with('|') {
            out.push('|');
        }
    }
    out
}

/// (misses, cached entries) — test instrumentation.
#[cfg(test)]
fn render_cache_stats() -> (u64, usize) {
    RENDER_CACHE.with(|c| {
        let c = c.borrow();
        (c.misses, c.map.len())
    })
}

// ── Markdown render helper ────────────────────────────────────

/// Stores markdown text; renders lazily at whatever width the
/// layout provides when `View::render` is called.
pub struct RevueMarkdown {
    text: Arc<str>,
    streaming: bool,
    /// Estimate row count at a typical width for height calculations.
    est_rows: u16,
}

impl Default for RevueMarkdown {
    fn default() -> Self {
        Self::new()
    }
}

impl RevueMarkdown {
    pub fn new() -> Self {
        Self {
            text: Arc::from(""),
            streaming: true,
            est_rows: 0,
        }
    }

    /// Store the markdown text. 行数估算在**实际内容宽**上进行——
    /// 此前固定 100 cols 估算,窄于估算宽的实际渲染会把超出 est_rows
    /// 的换行行裁掉（长单行文本,如 provider 错误 JSON,在窄终端被静默截断）。
    pub fn set_content(&mut self, markdown_text: &str, width: u16) {
        self.set_content_with_streaming(markdown_text, width, true);
    }

    pub fn set_content_with_streaming(&mut self, markdown_text: &str, width: u16, streaming: bool) {
        self.text = Arc::from(markdown_text);
        self.streaming = streaming;
        // 用调用方给定的真实内容宽（transcript inner_w）估算,与渲染同口径；
        // 走缓存——同文本同宽重复估算（每帧重建 view 树）不再重复 parse。
        self.est_rows =
            render_lines_cached_with_streaming(&self.text, width, streaming).len() as u16;
    }

    /// Rough row count (estimated at 100 cols). The actual row count
    /// may differ slightly at narrow/wide terminals.
    pub fn line_count(&self) -> u16 {
        self.est_rows.max(1)
    }

    /// Build a Stack that lazily renders at the actual layout width.
    pub fn as_stack(&self) -> revue::widget::Stack {
        let text = Arc::clone(&self.text);
        let rows = self.est_rows;
        let widget = MarkdownCellView {
            text,
            streaming: self.streaming,
        };
        revue::widget::vstack().child_sized(widget, rows)
    }
}

// ── Lazy-rendering revue View ────────────────────────────────

use revue::widget::traits::{RenderContext as RevueRenderCtx, View};

struct MarkdownCellView {
    text: Arc<str>,
    streaming: bool,
}

impl View for MarkdownCellView {
    fn render(&self, ctx: &mut RevueRenderCtx) {
        let area = ctx.area;
        let w = area.width.max(20) as usize;
        let h = area.height;
        if w < 2 || h == 0 {
            return;
        }

        // Render at the actual available width — adaptive!
        // Cached by (text hash, width): identical text re-renders at 20fps
        // during streaming cost a hash + lookup instead of a full parse.
        let lines = render_lines_cached_with_streaming(&self.text, area.width, self.streaming);

        for (y, line) in lines.iter().enumerate() {
            if y as u16 >= h {
                break;
            }
            let cells = line_to_cells(line);
            // Guard: if a wide-char's main cell would land at x == w-1
            // (its continuation falls outside), drop the half-char.
            let mut x = 0usize;
            while x < cells.len() && x < w {
                let cell = cells[x];
                let next_is_cont = cells
                    .get(x + 1)
                    .map(|c| c.is_continuation())
                    .unwrap_or(false);
                if next_is_cont && x + 1 >= w {
                    // Wide char doesn't fit in the last column — leave blank.
                    break;
                }
                ctx.set(x as u16, y as u16, cell);
                x += 1;
            }
        }
    }
}

// ── Minimal theme (Tokyo Night palette) ───────────────────────

use ratatui_markdown::theme::{Generation, RichTextTheme};

struct NoopTheme;

impl RichTextTheme for NoopTheme {
    fn generation(&self) -> Generation {
        Generation(1)
    }

    fn get_text_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(192, 202, 245) // FG_PRIMARY
    }
    fn get_muted_text_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(86, 95, 137) // FG_MUTED
    }
    fn get_primary_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(125, 207, 255) // ACCENT_CYAN
    }
    fn get_popup_selected_background(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(47, 51, 70) // BG_SURFACE
    }
    fn get_border_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(59, 66, 97) // BORDER
    }
    fn get_focused_border_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(125, 207, 255) // ACCENT_CYAN
    }
    fn get_secondary_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(122, 162, 247) // ACCENT_BLUE
    }
    fn get_info_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(125, 207, 255) // ACCENT_CYAN
    }
    fn get_json_key_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(122, 162, 247) // ACCENT_BLUE
    }
    fn get_json_string_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(158, 206, 106) // ACCENT_GREEN
    }
    fn get_json_number_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(224, 175, 104) // ACCENT_YELLOW
    }
    fn get_json_bool_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(187, 154, 247) // ACCENT_PURPLE
    }
    fn get_json_null_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(86, 95, 137) // FG_MUTED
    }
    fn get_accent_yellow(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(224, 175, 104) // ACCENT_YELLOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::{Line, Span};

    #[test]
    fn line_to_cells_emits_continuation_for_wide_chars() {
        // "你好" → 2 CJK chars, each 2 columns wide → 4 cells
        let line = Line::from(Span::raw("你好"));
        let cells = line_to_cells(&line);
        assert_eq!(cells.len(), 4, "two CJK chars should yield 4 cells");
        assert_eq!(cells[0].symbol, '你');
        assert!(cells[1].is_continuation(), "cell[1] must be continuation");
        assert_eq!(cells[2].symbol, '好');
        assert!(cells[3].is_continuation(), "cell[3] must be continuation");
    }

    #[test]
    fn line_to_cells_mixed_ascii_and_cjk() {
        // "a你b" → 1 + 2 + 1 = 4 cells
        let line = Line::from(Span::raw("a你b"));
        let cells = line_to_cells(&line);
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0].symbol, 'a');
        assert_eq!(cells[1].symbol, '你');
        assert!(cells[2].is_continuation());
        assert_eq!(cells[3].symbol, 'b');
    }

    #[test]
    fn line_to_cells_skips_zero_width_chars() {
        // Zero-width joiner U+200D should be dropped.
        let line = Line::from(Span::raw("a\u{200D}b"));
        let cells = line_to_cells(&line);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].symbol, 'a');
        assert_eq!(cells[1].symbol, 'b');
    }

    #[test]
    fn line_to_cells_propagates_bg_to_continuation() {
        use ratatui::style::{Color, Style};
        let line = Line::from(Span::styled(
            "你",
            Style::default().bg(Color::Rgb(10, 20, 30)),
        ));
        let cells = line_to_cells(&line);
        assert_eq!(cells.len(), 2);
        assert!(cells[0].bg.is_some(), "main cell should carry bg");
        assert!(cells[1].bg.is_some(), "continuation should also carry bg");
    }

    // ── Render cache ──────────────────────────────────────────
    // 每个测试跑在独立线程 → thread_local 缓存天然隔离，无需手动清空。

    #[test]
    fn cache_hit_renders_identical_text_once() {
        let text: Arc<str> = Arc::from("# Title\n\nsome **bold** body");
        let (misses_before, _) = render_cache_stats();
        let a = render_lines_cached_with_streaming(&text, 80, true);
        let b = render_lines_cached_with_streaming(&text, 80, true);
        let (misses_after, _) = render_cache_stats();
        assert_eq!(
            misses_after - misses_before,
            1,
            "same text+width must parse once"
        );
        assert!(Arc::ptr_eq(&a, &b), "cache hit must share the same Arc");
        assert_eq!(a, b);
    }

    #[test]
    fn cache_miss_on_text_change_and_width_change() {
        let mut text: Arc<str> = Arc::from("streaming chunk 1");
        let (m0, _) = render_cache_stats();
        render_lines_cached_with_streaming(&text, 80, true);
        // 流式追加 → 内容变 → 重新 parse。
        text = Arc::from("streaming chunk 1 + appended");
        render_lines_cached_with_streaming(&text, 80, true);
        // 同文本不同宽 → wrap 结果不同 → 重新 parse。
        render_lines_cached_with_streaming(&text, 40, true);
        let (m1, _) = render_cache_stats();
        assert_eq!(
            m1 - m0,
            3,
            "text change and width change must each re-parse"
        );
    }

    #[test]
    fn streaming_unclosed_fence_is_rendered_without_swallowing_tail() {
        let text: Arc<str> = Arc::from("```rust\nlet x = 1;\nplain tail");
        let lines = render_lines_cached_with_streaming(&text, 80, true);
        assert!(lines
            .iter()
            .any(|line| line.to_string().contains("plain tail")));
        let closed: Arc<str> = Arc::from("```rust\nlet x = 1;\n```\nplain tail");
        let closed_lines = render_lines_cached_with_streaming(&closed, 80, true);
        assert!(closed_lines
            .iter()
            .any(|line| line.to_string().contains("plain tail")));
    }

    #[test]
    fn streaming_table_row_repair_is_only_for_streaming_mode() {
        let text: Arc<str> = Arc::from("| a | b |\n|---|---|\n| 1 | 2");
        let streaming = render_lines_cached_with_streaming(&text, 80, true);
        let finalized = render_lines_cached_with_streaming(&text, 80, false);
        assert!(!streaming.is_empty());
        assert!(!finalized.is_empty());
    }

    #[test]
    fn streaming_table_rebuilds_across_deltas_and_keeps_prose() {
        let h1: Arc<str> = Arc::from("| a | b |\n|---|---|\n| 1");
        let h2: Arc<str> = Arc::from("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4");
        let h3: Arc<str> = Arc::from("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4\nafter table");
        let text = |lines: &Arc<Vec<ratatui::text::Line<'static>>>| {
            lines.iter().map(ToString::to_string).collect::<String>()
        };
        assert!(text(&render_lines_cached_with_streaming(&h1, 80, true)).contains('1'));
        let second = text(&render_lines_cached_with_streaming(&h2, 80, true));
        for cell in ["1", "2", "3", "4"] {
            assert!(second.contains(cell));
        }
        let third = text(&render_lines_cached_with_streaming(&h3, 80, true));
        assert!(third.contains("after table"));
        let (before, _) = render_cache_stats();
        let _ = render_lines_cached_with_streaming(&h1, 80, false);
        let (after, _) = render_cache_stats();
        assert_eq!(
            after - before,
            1,
            "finalized canonical pass uses a distinct cache mode"
        );
    }

    #[test]
    fn cache_evicts_lru_beyond_capacity() {
        let (m0, _) = render_cache_stats();
        // 插入 CAP+10 个不同文本 → 只保留最近 CAP 条。
        for i in 0..(RENDER_CACHE_CAP + 10) {
            let text: Arc<str> = Arc::from(format!("unique entry number {}", i));
            render_lines_cached_with_streaming(&text, 80, true);
        }
        let (m1, entries) = render_cache_stats();
        assert_eq!(m1 - m0, (RENDER_CACHE_CAP + 10) as u64);
        assert!(entries <= RENDER_CACHE_CAP, "cache must not grow past cap");
        // 最早的条目已被逐出：再次渲染会 miss。
        let evicted: Arc<str> = Arc::from("unique entry number 0");
        render_lines_cached_with_streaming(&evicted, 80, true);
        let (m2, _) = render_cache_stats();
        assert_eq!(m2 - m1, 1, "evicted entry must re-parse");
        // 最近的条目仍在：命中不 miss。
        let recent: Arc<str> = Arc::from(format!("unique entry number {}", RENDER_CACHE_CAP + 9));
        render_lines_cached_with_streaming(&recent, 80, true);
        let (m3, _) = render_cache_stats();
        assert_eq!(m3 - m2, 0, "most-recent entry must still hit");
    }

    #[test]
    fn revue_markdown_est_rows_matches_cached_render() {
        let mut md = RevueMarkdown::new();
        md.set_content("# H\n\n- a\n- b\n", 80);
        assert!(md.line_count() >= 4, "heading + 2 list items ≥ 4 rows");
        // 同文本再次 set_content（每帧重建 view 树的路径）不重复 parse。
        let (m0, _) = render_cache_stats();
        md.set_content("# H\n\n- a\n- b\n", 80);
        let (m1, _) = render_cache_stats();
        assert_eq!(m1 - m0, 0, "re-set of identical content must hit cache");
    }
}
