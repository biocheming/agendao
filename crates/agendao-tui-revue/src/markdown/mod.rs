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
/// Each char becomes one cell. CJK chars become one cell (wide chars are
/// handled by the terminal, not by cells).
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
            let mut cell = Cell::new(ch);
            if let Some(c) = fg {
                cell.fg = Some(c);
            }
            if let Some(c) = bg {
                cell.bg = Some(c);
            }
            cell.modifier = modifier;
            cells.push(cell);
        }
    }
    cells
}

/// Convert a slice of `ratatui::text::Line`s into a flat vec of cells,
/// one row per line (padded to `max_width`).
///
/// Returns `(cells, row_count)` where `cells.len() == row_count * max_width`.
pub fn lines_to_cell_grid(lines: &[ratatui::text::Line], max_width: u16) -> (Vec<Cell>, u16) {
    let w = max_width as usize;
    let row_count = lines.len() as u16;
    let mut grid = vec![Cell::empty(); row_count as usize * w];
    for (y, line) in lines.iter().enumerate() {
        let row_cells = line_to_cells(line);
        let row_offset = y * w;
        for (x, cell) in row_cells.iter().enumerate() {
            if x >= w {
                break;
            }
            grid[row_offset + x] = *cell;
        }
    }
    (grid, row_count)
}

// ── Markdown render helper ────────────────────────────────────

/// Stores markdown text; renders lazily at whatever width the
/// layout provides when `View::render` is called.
pub struct RevueMarkdown {
    text: String,
    /// Estimate row count at a typical width for height calculations.
    est_rows: u16,
}

impl RevueMarkdown {
    pub fn new() -> Self {
        Self { text: String::new(), est_rows: 0 }
    }

    /// Store the markdown text. A rough line-count estimate is
    /// pre-computed at a generous width so `line_count()` returns
    /// something reasonable for layout without knowing the final width.
    pub fn set_content(&mut self, markdown_text: &str) {
        self.text = markdown_text.to_string();
        // Quick estimate at 100 cols — close enough for scroll layout.
        let renderer = MarkdownRenderer::new(100);
        let blocks = renderer.parse(&self.text);
        let lines = renderer.render(&blocks, &NoopTheme);
        self.est_rows = lines.len() as u16;
    }

    /// Rough row count (estimated at 100 cols). The actual row count
    /// may differ slightly at narrow/wide terminals.
    pub fn line_count(&self) -> u16 {
        self.est_rows.max(1)
    }

    /// Build a Stack that lazily renders at the actual layout width.
    pub fn as_stack(&self) -> revue::widget::Stack {
        let text = self.text.clone();
        let rows = self.est_rows;
        let widget = MarkdownCellView { text, rows };
        revue::widget::vstack().child_sized(widget, rows)
    }
}

// ── Lazy-rendering revue View ────────────────────────────────

use revue::widget::traits::{RenderContext as RevueRenderCtx, View};

struct MarkdownCellView {
    text: String,
    #[allow(dead_code)]
    rows: u16,
}

impl View for MarkdownCellView {
    fn render(&self, ctx: &mut RevueRenderCtx) {
        let area = ctx.area;
        let w = area.width.max(20) as usize;
        let h = area.height;
        if w < 2 || h == 0 { return; }

        // Render at the actual available width — adaptive!
        let renderer = MarkdownRenderer::new(w);
        let blocks = renderer.parse(&self.text);
        let lines = renderer.render(&blocks, &NoopTheme);

        for (y, line) in lines.iter().enumerate() {
            if y as u16 >= h { break; }
            let cells = line_to_cells(line);
            for (x, cell) in cells.iter().enumerate() {
                if x >= w { break; }
                ctx.set(x as u16, y as u16, *cell);
            }
        }
    }
}

// ── Minimal theme (Tokyo Night palette) ───────────────────────

use ratatui_markdown::theme::{Generation, RichTextTheme};

struct NoopTheme;

impl RichTextTheme for NoopTheme {
    fn generation(&self) -> Generation { Generation(1) }

    fn get_text_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(192, 202, 245)  // FG_PRIMARY
    }
    fn get_muted_text_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(86, 95, 137)     // FG_MUTED
    }
    fn get_primary_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(125, 207, 255)   // ACCENT_CYAN
    }
    fn get_popup_selected_background(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(47, 51, 70)      // BG_SURFACE
    }
    fn get_border_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(59, 66, 97)      // BORDER
    }
    fn get_focused_border_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(125, 207, 255)   // ACCENT_CYAN
    }
    fn get_secondary_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(122, 162, 247)   // ACCENT_BLUE
    }
    fn get_info_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(125, 207, 255)   // ACCENT_CYAN
    }
    fn get_json_key_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(122, 162, 247)   // ACCENT_BLUE
    }
    fn get_json_string_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(158, 206, 106)   // ACCENT_GREEN
    }
    fn get_json_number_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(224, 175, 104)   // ACCENT_YELLOW
    }
    fn get_json_bool_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(187, 154, 247)   // ACCENT_PURPLE
    }
    fn get_json_null_color(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(86, 95, 137)     // FG_MUTED
    }
    fn get_accent_yellow(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(224, 175, 104)   // ACCENT_YELLOW
    }
}
