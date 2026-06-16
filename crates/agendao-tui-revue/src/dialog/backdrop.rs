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

/// Paint the modal's dialog rect as an opaque `BG_SURFACE` stage.
///
/// `positioned` overlays don't clear their own background, and terminals
/// can't render alpha — so without an explicit fill the dialog leaks the
/// transcript behind it (the slash-popup transparency bug, same root
/// cause). We paint only the dialog rect so the decision content sits on
/// a solid, slightly-raised surface.
///
/// We deliberately do NOT dim the rest of the screen. AgenDao's
/// permission/question are inline in the transcript flow (Claude
/// Code/Codex style), and the remaining modals (/models, /sessions, …)
/// float as a bright box over a *visible* transcript — not under a black
/// wash. Must run *before* the positioned dialog renders, so the border
/// + text draw on top. `x`/`y` are relative to `ctx.area`;
/// `Buffer::fill` is absolute, so we add `ctx.area.{x,y}` when filling.
fn paint_modal_backdrop(ctx: &mut RenderContext, x: u16, y: u16, w: u16, h: u16, bg: Color) {
    let area = ctx.area;
    ctx.buffer.fill(
        area.x.saturating_add(x), area.y.saturating_add(y), w, h,
        Cell::new(' ').bg(bg),
    );
}

/// Where a dialog/list anchors on screen.
///
/// Two strategies share one rendering core (唯一成形语法 — 金律):
///   - [`DialogAnchor::Centered`] — float in the middle (original behaviour:
///     rename, confirm, alert, provider, stash, export, fork, help).
///   - [`DialogAnchor::Bottom`] — pin just above the input box, left-aligned
///     full-width. Mirrors the slash-popup bottom anchor: the prompt occupies
///     the bottom 5 rows (`prompt_y = area.y + height - 5`), so the panel sits
///     at `y = (height-5) - h`. Used by the command-picker panels
///     (/models, /sessions, /agents) so they read as "sitting on the input box"
///     rather than "floating in the middle of the screen".
enum DialogAnchor {
    Centered,
    Bottom,
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
    let _ = render_positioned_dialog(
        DialogAnchor::Centered, title, border_color, content, footer_hint,
        ctx, max_w, max_h,
    );
}

/// Same as [`render_dialog`] but pinned above the input box, left-aligned
/// full-width — for a command picker's empty/loading/error state
/// (/sessions loading…). Keeps the panel at the same anchor as its list so
/// loading→loaded doesn't make the box jump from centre to bottom.
pub fn render_dialog_bottom(
    title: &str,
    border_color: Color,
    content: Stack,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    max_h: u16,
) {
    let _ = render_positioned_dialog(
        DialogAnchor::Bottom, title, border_color, content, footer_hint,
        ctx, max_w, max_h,
    );
}

/// Core: render a single-content dialog at `anchor`. Split out so the Centered
/// and Bottom wrappers share one border/title/positioned pipeline. Geometry
/// differs only by `anchor`.
fn render_positioned_dialog(
    anchor: DialogAnchor,
    title: &str,
    border_color: Color,
    content: Stack,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    max_h: u16,
) {
    let area = ctx.area;
    let (w, h, x, y) = match anchor {
        DialogAnchor::Centered => {
            let w = max_w.min(area.width.saturating_sub(4));
            let h = max_h.min(area.height.saturating_sub(4));
            let x = (area.width.saturating_sub(w)) / 2;
            let y = (area.height.saturating_sub(h)) / 2;
            (w, h, x, y)
        }
        DialogAnchor::Bottom => {
            // 占满宽(左右各留 2 列,与输入框/slash 对齐);高度留 6 行
            // (5 行 prompt + 1 行 margin),避免空状态框压住输入框本体。
            let w = area.width.saturating_sub(4);
            let h = max_h.min(area.height.saturating_sub(6));
            let x = 2u16;
            let y = area.height.saturating_sub(5).saturating_sub(h);
            (w, h, x, y)
        }
    };

    // 贴底模式实色填全宽(col 0 起),而非仅框区(col 2+)。Home/会话屏下层按钮框
    // (Tip/Environment 等)左边框在 col 0,贴底框宽达 width-4 与之水平重叠;若只填
    // 框区,col 0-1 会透出下层边框,与列表左边框交错(实色不透字契约)。居中框在
    // 屏幕中央,与 col 0 的按钮框不重叠,填框区即可。
    let (fill_x, fill_w, fill_bg) = match anchor {
        DialogAnchor::Centered => (x, w, colors::BG_SURFACE),
        DialogAnchor::Bottom => (0u16, area.width, colors::BG_PRIMARY),
    };
    paint_modal_backdrop(ctx, fill_x, y, fill_w, h, fill_bg);

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
/// [`render_list_dialog_bottom_with_layout`] so callers can place a tooltip /
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

/// Render a list-style dialog (centred) with selection highlighting and
/// scrolling. Used by non-picker lists (provider manager, prompt stash) that
/// still want the centred modal. Picker panels (/models, /sessions, /agents)
/// use [`render_list_dialog_bottom`] instead.
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
    let _ = render_positioned_list(
        DialogAnchor::Centered, title, border_color, items, selected,
        footer_hint, ctx, max_w, visible_rows,
    );
}

/// Same as [`render_list_dialog`] but pinned above the input box, left-aligned
/// full-width — the command-picker anchor (/models, /sessions, /agents). Reads
/// as "sitting on the input box" rather than "floating in the middle". The
/// list, sliding viewport, scrollbar and selection contract are identical to
/// the centred variant — only the anchor differs.
pub fn render_list_dialog_bottom(
    title: &str,
    border_color: Color,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    visible_rows: usize,
) {
    let _ = render_positioned_list(
        DialogAnchor::Bottom, title, border_color, items, selected,
        footer_hint, ctx, max_w, visible_rows,
    );
}

/// Same as [`render_list_dialog_bottom`] but also returns the layout of the
/// rendered dialog. Callers that want to overlay a tooltip / popover anchored
/// to the selected row, or publish the scrollbar geometry for mouse handling,
/// use this variant (/sessions).
pub fn render_list_dialog_bottom_with_layout(
    title: &str,
    border_color: Color,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    max_w: u16,
    visible_rows: usize,
) -> ListDialogLayout {
    render_positioned_list(
        DialogAnchor::Bottom, title, border_color, items, selected,
        footer_hint, ctx, max_w, visible_rows,
    )
}

/// Core list renderer: the sliding viewport, selection contract, scrollbar
/// overlay and tooltip-anchor layout all live here. Only the geometry (w/h/x/y)
/// depends on `anchor`; everything below is shared so the centred and
/// bottom-anchored pickers look identical except for position.
fn render_positioned_list(
    anchor: DialogAnchor,
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
    let (w, h, x, y) = match anchor {
        DialogAnchor::Centered => {
            let w = max_w.min(area.width.saturating_sub(4));
            let h = (rows as u16 + 3).min(area.height.saturating_sub(4));
            let x = (area.width.saturating_sub(w)) / 2;
            let y = (area.height.saturating_sub(h)) / 2;
            (w, h, x, y)
        }
        DialogAnchor::Bottom => {
            // 占满宽(左右各留 2 列,与输入框/slash 对齐)。无框高度 = 标题(1)
            // + rows + hint(1) = rows+2;上限留 6 行(5 行 prompt + 1 行 margin)。
            let w = area.width.saturating_sub(4);
            let h = (rows as u16 + 2).min(area.height.saturating_sub(6));
            let x = 2u16;
            let y = area.height.saturating_sub(5).saturating_sub(h);
            (w, h, x, y)
        }
    };

    let (fill_x, fill_w, fill_bg) = match anchor {
        DialogAnchor::Centered => (x, w, colors::BG_SURFACE),
        DialogAnchor::Bottom => (0u16, area.width, colors::BG_PRIMARY),
    };
    paint_modal_backdrop(ctx, fill_x, y, fill_w, h, fill_bg);

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

    // Inner width for selected-row padding. Centered: minus rounded border (2)
    // + 1 trailing breathing column. Bottom (无框): full width — no border.
    let inner_w = match anchor {
        DialogAnchor::Centered => w.saturating_sub(3) as usize,
        DialogAnchor::Bottom => w as usize,
    };

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
                let mut hdr = Text::new(format!(" ▸ {}", upper)).bold().fg(colors::E_AMBER);
                // 无框贴底时补终端色 bg,否则文字格发黑/透字。
                if matches!(anchor, DialogAnchor::Bottom) {
                    hdr = hdr.bg(colors::BG_PRIMARY);
                }
                list_content = list_content.child_sized(hdr, 1);
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
                } else if matches!(anchor, DialogAnchor::Bottom) {
                    // 无框贴底:非选中行补终端色 bg,否则文字格发黑/透字。
                    row = row.bg(colors::BG_PRIMARY);
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

    if matches!(anchor, DialogAnchor::Bottom) {
        // 无框贴底:标题行 + 列表 + hint,整片 BG_PRIMARY 融入终端(不浮出亮框),
        // 仅选中行 SURFACE_SELECTED 高亮。对齐 codex/claude code 的轻量命令面板。
        // 标题行(1)替代了原 top border,故滚动条 sb_y=y+1、tooltip y+1+row_offset
        // 偏移与 Centered 一致,无需调整。
        let view = vstack().gap(0)
            .child_sized(
                Text::new(title_with_pos)
                    .fg(border_color)
                    .bg(colors::BG_PRIMARY)
                    .bold(),
                1,
            )
            .child_flex(list_content, 1.0)
            .child_sized(
                Text::new(footer_hint)
                    .fg(colors::FG_MUTED)
                    .bg(colors::BG_PRIMARY)
                    .align(Alignment::Center),
                1,
            );
        revue::widget::positioned(view)
            .x(x as i16)
            .y(y as i16)
            .width(w)
            .height(h)
            .render(ctx);
    } else {
        let dialog = Border::rounded()
            .title(title_with_pos)
            .fg(border_color)
            .child(
                // Inner vstack: list flexes to take all remaining height,
                // footer hint pinned to its single row. Without explicit
                // sizing the dialog vstack defaults to Auto and splits the
                // height EQUALLY between list and hint.
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
