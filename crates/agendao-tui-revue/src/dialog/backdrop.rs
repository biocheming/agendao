//! Reusable dialog backdrop — consistent overlay + border + footer.
//!
//! Two entry points:
//!   - `render_dialog` — a centered modal frame around any custom `Stack`
//!     content (used by single-step prompts: rename, confirm, alert).
//!   - `render_list_dialog` — a centered modal wrapping a manually-rendered
//!     row list with the host's cursor index. We tried `revue::OptionList`
//!     here first but its public API doesn't expose a "set highlighted by
//!     external index" setter, only step-by-step `highlight_first/next`,
//!     and `next` skips disabled rows — so external indices drift past
//!     muted (disconnected-provider) rows and the highlight ends up on
//!     the wrong line. The hand-written version below mirrors OptionList's
//!     visual contract (full-row bg padding, ▸/> prefix, bold highlight)
//!     while letting the calling dialog keep authoritative ownership of
//!     `selected`. Once OptionList grows a public `set_highlighted(usize)`
//!     setter we can switch back.

use revue::prelude::*;
use crate::theme::colors;

/// Render a centered modal dialog with a custom content stack.
pub fn render_dialog(
    title: &str,
    border_color: Color,
    content: Stack,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    max_h: u16,
) {
    let area = ctx.area;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = max_h.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;

    let dialog = Border::rounded()
        .title(format!(" {} ", title))
        .fg(border_color)
        .child(
            vstack().gap(1)
                .child(content)
                .child(
                    Text::new(footer_hint)
                        .fg(colors::FG_MUTED)
                        .align(Alignment::Center)
                )
        );

    revue::widget::positioned(dialog)
        .x(x as i16)
        .y(y as i16)
        .width(w)
        .height(h)
        .render(ctx);
}

/// A single item in a list dialog.
pub enum ListItem {
    Header(String),
    Row { display: String, muted: bool },
}

/// Render a list-style dialog with selection highlighting and scrolling.
///
/// Key visual contract (mirrors `revue::OptionList` rendering):
///   - selected row gets a `▸ ` prefix, others get `  `
///   - selected row right-pads with spaces to inner width so the bg
///     color fills the row instead of just the text cells
///   - selected row uses bold + FG_PRIMARY + BG_HIGHLIGHT
///   - muted rows render in disabled fg (FG_MUTED) regardless of selection
///   - group headers render in ACCENT_BLUE bold
pub fn render_list_dialog(
    title: &str,
    border_color: Color,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    visible_rows: usize,
) {
    let area = ctx.area;
    let total = items.len();

    // Auto-size dialog height: shrink-wrap to actual content when the
    // list is shorter than `visible_rows`, otherwise cap at visible_rows.
    // Total dialog height = top border (1) + N list rows + footer hint (1) + bottom border (1).
    let rows = visible_rows.min(total.max(1));
    let h = (rows as u16 + 3).min(area.height.saturating_sub(4));
    let w = max_w.min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;

    // Sliding viewport. The host's `selected` index counts ALL items
    // (Header rows included), so the viewport math operates on the same
    // coordinate space — no need to translate "Row index" vs "item index".
    //
    // Algorithm: maintain a window [start, start+rows). When the cursor
    // moves outside that window we shift start by exactly enough to
    // bring `selected` back into view. This is the same behaviour vim
    // and most editors use ("scroll just enough"), and unlike the
    // previous `selected.saturating_sub(rows-2)` formula it correctly
    // handles BOTH directions (up overflow, down overflow) AND large
    // jumps from filter-induced cursor relocations.
    //
    // We compute `start` from selected directly each render — no state
    // is kept across calls. That's safe because the host pushes a new
    // `selected` every redraw, so the visible window always reflects
    // the latest cursor position.
    let start = if total <= rows {
        0
    } else if selected < rows.saturating_sub(1) {
        // Cursor is in the first window; pin to top so the user sees
        // the top headers. The `-1` keeps one row of context above.
        0
    } else if selected + 1 >= total {
        // Cursor at the very end — anchor the window to the bottom.
        total.saturating_sub(rows)
    } else {
        // General case: place selected so it sits two rows from the
        // bottom of the viewport. That gives 1 row of look-ahead while
        // still showing as much history as possible.
        selected + 2 - rows.min(selected + 2)
    };
    let end = (start + rows).min(total);

    // Inner width = dialog width minus the rounded border (2 cells)
    // minus 1 trailing column of breathing room before the right edge.
    let inner_w = w.saturating_sub(3) as usize;

    let mut list_content = vstack().gap(0);
    for (i, item) in items[start..end].iter().enumerate() {
        let abs = start + i;
        let is_sel = abs == selected;
        match item {
            ListItem::Header(label) => {
                // Mockup E group header: amber UPPERCASE with extra
                // letter-spacing (we approximate by adding spaces).
                // The triangle prefix `▸` reads as "section start" —
                // distinct from the selected-row marker `▌` (left bar).
                let stripped = label.strip_prefix("▸ ").unwrap_or(label.as_str());
                let upper = stripped.to_uppercase();
                list_content = list_content.child_sized(
                    Text::new(format!(" ▸ {}", upper))
                        .bold()
                        .fg(colors::E_AMBER),
                    1,
                );
            }
            ListItem::Row { display, muted } => {
                // Mockup E selected row contract:
                //   - left bar: 2px solid teal `▌` (we use 1 char wide
                //     because TUI cells are 1-cell minimum)
                //   - bg: pre-composited cyan-tinted glass surface
                //   - fg: white (FG_PRIMARY) bold
                //   - right marker: ` ✓ ` in solid teal
                // Muted rows get a leading `○` glyph instead of `▌`.
                //
                // Non-selected rows use plain 2-space prefix to keep
                // the column alignment with the selected row.
                let (prefix, suffix) = if is_sel {
                    ("▌ ", " ✓ ")
                } else if *muted {
                    ("○ ", "   ")
                } else {
                    ("  ", "   ")
                };

                // Build the unstyled row text (without prefix/suffix
                // styling) so we can size the bg fill correctly.
                let line = format!("{}{}", prefix, display);

                // Pad selected/muted rows to fill inner width using
                // display columns (UAX#11). Suffix `" ✓ "` is 3 cells.
                let padded = {
                    use unicode_width::UnicodeWidthStr;
                    let used = UnicodeWidthStr::width(line.as_str())
                        + UnicodeWidthStr::width(suffix);
                    if is_sel && used < inner_w {
                        // Insert spaces between display and `✓` so the
                        // checkmark sits at the right edge, like the
                        // mockup `::after { margin-left: auto }` rule.
                        format!("{}{}{}", line, " ".repeat(inner_w - used), suffix)
                    } else {
                        format!("{}{}", line, suffix)
                    }
                };

                // Foreground priority: selected wins over muted so the
                // cursor stays visible even on disconnected-provider
                // rows. Mockup uses `color:#e4e3e0` (close to FG_PRIMARY).
                let color = if is_sel {
                    colors::FG_PRIMARY
                } else if *muted {
                    colors::FG_MUTED
                } else {
                    colors::FG_SECONDARY
                };
                let mut row = Text::new(padded).fg(color);
                if is_sel {
                    row = row.bg(colors::SURFACE_SELECTED).bold();
                }
                list_content = list_content.child_sized(row, 1);
            }
        }
    }

    // Position indicator in title (e.g. " Models 47/5140 ")
    let title_with_pos = if total > rows {
        format!(" {} {}/{} ", title, selected + 1, total)
    } else {
        format!(" {} ", title)
    };

    let dialog = Border::rounded()
        .title(title_with_pos)
        .fg(border_color)
        .child(
            // Inner vstack: list flexes to take all remaining height,
            // footer hint pinned to its single row. Without explicit
            // sizing the dialog vstack defaults to Auto and splits the
            // height EQUALLY between list and hint — that's why a 22-row
            // dialog only painted 10 list rows and left 8 rows of dead
            // air below the visible items.
            vstack().gap(0)
                .child_flex(list_content, 1.0)
                .child_sized(
                    Text::new(footer_hint)
                        .fg(colors::FG_MUTED)
                        .align(Alignment::Center),
                    1,
                )
        );

    revue::widget::positioned(dialog)
        .x(x as i16)
        .y(y as i16)
        .width(w)
        .height(h)
        .render(ctx);
}
