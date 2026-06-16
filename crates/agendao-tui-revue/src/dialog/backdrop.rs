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
use revue::runtime::render::Cell;
use crate::theme::colors;

/// Paint the two-tier modal backdrop used by every dialog.
///
/// 1. A near-black wash (`BG_OVERLAY`) over the *whole* screen — 阴:
///    collapse the user's attention onto the modal, dim out the transcript
///    / status bar / prompt behind it. Without this the modal floats at
///    the same visual tier as the background (the slash-popup transparency
///    bug, same root cause) and the user can't tell "the system is waiting
///    on my decision" from "background text".
/// 2. `BG_SURFACE` over the dialog rect itself — 阳: the modal's own
///    stage, lighter than the wash so the decision content rises above it.
///
/// Must run *before* the positioned dialog renders, so the border + text
/// draw on top. `x`/`y` are relative to `ctx.area` (same space as
/// `positioned`); `Buffer::fill` is absolute — so we add `ctx.area.{x,y}`
/// when filling. Mixing the two fills the wrong rect and the modal leaks
/// the transcript through (learned from the slash_popup fix).
fn paint_modal_backdrop(ctx: &mut RenderContext, x: u16, y: u16, w: u16, h: u16) {
    let area = ctx.area;
    ctx.buffer.fill(
        area.x, area.y, area.width, area.height,
        Cell::new(' ').bg(colors::BG_OVERLAY),
    );
    ctx.buffer.fill(
        area.x.saturating_add(x), area.y.saturating_add(y), w, h,
        Cell::new(' ').bg(colors::BG_SURFACE),
    );
}

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

    paint_modal_backdrop(ctx, x, y, w, h);

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

/// Layout of a list dialog after it has been rendered, returned by
/// [`render_list_dialog_with_layout`] so callers can place a tooltip /
/// popover anchored to the selected row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListDialogLayout {
    /// Absolute screen coordinates of the dialog's outer rectangle
    /// (inclusive of border).
    pub dialog_x: u16,
    pub dialog_y: u16,
    pub dialog_w: u16,
    pub dialog_h: u16,
    /// Y coordinate of the row currently containing the cursor, or
    /// `None` if the selected index points at a header / empty list.
    /// In absolute screen coordinates.
    pub selected_row_y: Option<u16>,
    /// Inner usable width inside the dialog border (excluding the
    /// row prefix/suffix decorations). Use this to decide whether
    /// the selected row's text is being truncated.
    pub inner_w: u16,
    /// Geometry of the agendao interactive scrollbar overlay rendered
    /// along the dialog's right edge. `None` when the list fits in
    /// the viewport (no scroll needed). Item count + visible rows
    /// are reported in *items* (not pixels) so callers translate
    /// hits to the right `selected` index, not to byte offsets.
    pub scrollbar: Option<ListDialogScrollbarArea>,
}

/// Geometry of the interactive scrollbar overlay drawn on a list
/// dialog. Coordinates are absolute screen positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListDialogScrollbarArea {
    /// Absolute screen rect of the scrollbar column (1 cell wide).
    pub area: Rect,
    /// Total number of items in the list.
    pub item_count: u16,
    /// Number of items visible in the viewport at once.
    pub visible_rows: u16,
    /// Maximum value of `selected.saturating_sub(start)` once the
    /// viewport is at the bottom — i.e. the largest
    /// `selected_in_window` value the thumb can reach.
    pub max_offset: u16,
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
    let _ = render_list_dialog_with_layout(
        title, border_color, items, selected, footer_hint, ctx, max_w, visible_rows,
    );
}

/// Same as [`render_list_dialog`] but also returns the layout of the
/// rendered dialog. Callers that want to overlay a tooltip / popover
/// anchored to the selected row use this variant; the regular
/// [`render_list_dialog`] is unchanged for callers that don't.
pub fn render_list_dialog_with_layout(
    title: &str,
    border_color: Color,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    visible_rows: usize,
) -> ListDialogLayout {
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

    paint_modal_backdrop(ctx, x, y, w, h);

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
                // Unified ❯ pointer (aligned with Claude Code/Codex and
                // with our own slash_popup). Muted rows get no glyph —
                // their disabled state reads from the dim FG_MUTED color,
                // not from a special prefix. Non-selected rows use a
                // 2-space prefix to keep the column aligned with ❯.
                // (Previously this row used ▌ + ✓, and muted used ○ —
                // three different marks; now one across the whole app.)
                let (prefix, suffix) = if is_sel {
                    ("❯ ", "")
                } else {
                    ("  ", "")
                };

                // Build the unstyled row text (prefix + display) so we
                // can size the bg fill correctly.
                let line = format!("{}{}", prefix, display);

                // Pad the selected row to fill inner width (display
                // columns, UAX#11) so the highlight bg spans the whole
                // row instead of just the text cells. suffix is now
                // empty — there's no right-edge marker (❯ + bold bg is
                // the selection signal, per Claude Code/Codex).
                let padded = {
                    use unicode_width::UnicodeWidthStr;
                    let used = UnicodeWidthStr::width(line.as_str())
                        + UnicodeWidthStr::width(suffix);
                    if is_sel && used < inner_w {
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

    // Overlay agendao's interactive scrollbar on the dialog's right edge
    // when the list is taller than the viewport. The bar lives inside
    // the dialog border (column `w - 2` from the dialog's left) and
    // spans `rows` rows (the visible list height, excluding the footer
    // hint). Arrow buttons at top/bottom + draggable thumb are layered
    // on the same column; mouse events route through the published
    // layout below.
    let list_overlay = if total > rows {
        let sb_x = ctx.area.x.saturating_add(x).saturating_add(w.saturating_sub(2));
        let sb_y = ctx.area.y.saturating_add(y).saturating_add(1); // skip top border
        let sb_h = rows as u16;
        let sb_area = Rect::new(sb_x, sb_y, 1, sb_h);
        let max_offset_in_items = total.saturating_sub(rows);
        let selected_in_window = (selected.saturating_sub(start)) as u16;
        let overlay = crate::widget::ScrollbarOverlay::new(
            (ctx.area.x, ctx.area.y),
            sb_area,
            // content_h here = total item count (not pixels). thumb
            // sizing math works the same way — viewport_h is the number
            // of items visible, content_h is the total.
            total as u16,
            rows as u16,
            selected_in_window,
        );
        overlay.render(ctx);
        Some(ListDialogScrollbarArea {
            area: sb_area,
            item_count: total as u16,
            visible_rows: rows as u16,
            max_offset: max_offset_in_items as u16,
        })
    } else {
        None
    };

    // Compute selected row's absolute Y on the screen so a caller can
    // anchor a popover next to it. Only Row items get a meaningful y;
    // headers don't.
    let selected_row_y = if selected >= start && selected < end
        && matches!(items.get(selected), Some(ListItem::Row { .. }))
    {
        let row_offset = (selected - start) as u16;
        Some(ctx.area.y.saturating_add(y + 1 + row_offset))
    } else {
        None
    };

    ListDialogLayout {
        dialog_x: ctx.area.x.saturating_add(x),
        dialog_y: ctx.area.y.saturating_add(y),
        dialog_w: w,
        dialog_h: h,
        selected_row_y,
        inner_w: inner_w as u16,
        scrollbar: list_overlay,
    }
}
