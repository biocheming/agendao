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

use crate::theme::colors;
use revue::prelude::*;
use revue::runtime::render::Cell;

/// 表单校验错误行（红 ⚠ 前缀）——所有编辑弹窗共用（土律·单点权威，U5）。
/// 用法：校验失败的弹窗 `content.child_sized(validation_error_line(e), 1)`
/// 并把对话框高度 +1；不关窗、聚焦出错字段，让用户就地改正。
pub fn validation_error_line(error: &str) -> Text {
    Text::new(format!("⚠ {}", error)).fg(colors::STATUS_ERROR())
}

/// 列表 sliding viewport 唯一权威 — selected 出窗时自动滚动。
///
/// 输入:`total` 列表总长、`selected` 当前选中绝对索引、`rows` 视窗最多容纳行数。
/// 返回:`[start, end)` 窗口区间——保证 `selected` 永远在区间内。
///
/// 算法语义(沿用大多数编辑器的「scroll just enough」):
/// - 列表能整窗装下 → start=0
/// - selected 在前 rows-1 位 → start=0(钉顶,看到头部标题)
/// - selected 在尾巴(`selected+1 >= total`) → start=total-rows(钉底)
/// - 一般情况 → selected 沉到底数第 2 行,留 1 行 lookahead
///
/// 唯一权威:`render_positioned_list` 和 `input::slash_popup::render_popup` 都
/// 调这里,不再各自实现(金律·成形语法唯一)。任何修改 viewport 行为(如
/// 改 lookahead 数、改钉顶/钉底语义)只动这一函数。
pub fn list_viewport_window(total: usize, selected: usize, rows: usize) -> (usize, usize) {
    let rows = rows.min(total.max(1));
    let start = if total <= rows {
        0
    } else if selected + 1 >= total {
        total.saturating_sub(rows)
    } else if selected < rows.saturating_sub(1) {
        0
    } else {
        selected + 2 - rows.min(selected + 2)
    };
    let end = (start + rows).min(total);
    (start, end)
}

/// Paint the modal's dialog rect as an opaque `BG_SURFACE` stage.
///
/// `positioned` overlays don't clear their own background, and terminals
/// can't render alpha — so without an explicit fill the dialog leaks the
/// transcript behind it (the slash-popup transparency bug, same root
/// cause). We paint only the dialog rect so the decision content sits on
/// a solid, slightly-raised surface.
///
/// We deliberately do NOT dim the rest of the screen. AgenDao's
/// permission/question are inline in the transcript flow (terminal
/// inline-CLI style), and the remaining modals (/models, /sessions, …)
/// float as a bright box over a *visible* transcript — not under a black
/// wash. Must run *before* the positioned dialog renders, so the border
/// + text draw on top. `x`/`y` are relative to `ctx.area`;
///   `Buffer::fill` is absolute, so we add `ctx.area.{x,y}` when filling.
fn paint_modal_backdrop(ctx: &mut RenderContext, x: u16, y: u16, w: u16, h: u16, bg: Color) {
    let area = ctx.area;
    ctx.buffer.fill(
        area.x.saturating_add(x),
        area.y.saturating_add(y),
        w,
        h,
        Cell::new(' ').bg(bg),
    );
}

/// 当前路由输入框的屏幕几何（绝对坐标）。所有 `/` 弹框（SlashPopup 补全框 +
/// Bottom 锚点对话框）的宽/x/垂直位置都从此派生——唯一真相（土律），避免补全框
/// 与对话框两处各自算居中/宽度而漂移。由 `app::prompt_geometry` 按 route 计算。
#[derive(Clone, Copy, Debug)]
pub struct PromptGeom {
    /// 输入框左边沿绝对 x。
    pub x: u16,
    /// 输入框上沿绝对 y（弹框贴其上方）。
    pub y_top: u16,
    /// 输入框宽度（弹框同宽）。
    pub w: u16,
}

/// 按 display width (UAX#11) 截断字符串到 `max_w`,超出末尾加 `…`。Home 窄输入框
/// (64 宽)下 SlashPopup 命令名/描述与 Bottom 列表行(/sessions 长 session 标题)共用
/// ——revue positioned 按 width 硬裁会切 CJK 半个字,故主动按 display width 截断。
/// 唯一截断实现(水律:消灭第二处实现)。
pub(crate) fn truncate_to_width(s: &str, max_w: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if UnicodeWidthStr::width(s) <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw + 1 > max_w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out
}

/// U14①：footer hint 超宽裁剪——保尾部丢头部（头部补 … 明示省略）。
/// 各 dialog 的 hint 文案惯例把逃生键（Esc/Enter）放在尾部，而原
/// 居中渲染超宽时两端裁剪会把 "Enter: …  Esc: close" 整体吃掉
/// （Home 路由输入框仅 64 宽时 mcp_list 120 字符 hint 即中招）。
/// 居中绘制前先把文本收到 ≤ width，居中就只剩≤宽文本，不再双端
/// 丢字。
pub(crate) fn fit_hint_tail(hint: &str, width: u16) -> String {
    let w = width as usize;
    let chars: Vec<char> = hint.chars().collect();
    if chars.len() <= w {
        return hint.to_string();
    }
    if w <= 1 {
        return "…".to_string();
    }
    let tail: String = chars[chars.len() - (w - 1)..].iter().collect();
    format!("…{}", tail)
}

/// Where a dialog/list anchors on screen.
///
/// Two strategies share one rendering core (唯一成形语法 — 金律):
///   - [`DialogAnchor::Centered`] — float in the middle (original behaviour:
///     rename, confirm, alert, provider, stash, export, fork, help).
///   - [`DialogAnchor::Bottom`] — follow the input box geometry (PromptGeom):
///     width = input box width, x aligned to the input box, pinned just above
///     it. Same geometry authority the slash-popup completion box uses, so
///     every `/`-triggered panel (completion + /models, /sessions, /agents,
///     /help, /rename, /stash) reads identically and sits on the input box
///     rather than floating mid-screen.
enum DialogAnchor {
    Centered,
    Bottom,
}

struct DialogPlacement {
    anchor: DialogAnchor,
    max_w: u16,
    max_h: u16,
    geom: Option<PromptGeom>,
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
) -> revue::prelude::Rect {
    render_positioned_dialog(
        title,
        border_color,
        content,
        footer_hint,
        ctx,
        DialogPlacement {
            anchor: DialogAnchor::Centered,
            max_w,
            max_h,
            geom: None,
        },
    )
}

/// Frameless dialog following the input box geometry — width = input box width,
/// x aligned to the input box, pinned just above it. 与 [`render_list_dialog_bottom`]
/// 同一种成形语法（标题行 + content + hint，无框，`BG_PRIMARY` 底）。所有 /命令
/// 单步对话框（/help /rename /confirm /stash）与 picker 的空/loading/error 态
/// 都走这里——几何唯一权威 `PromptGeom`（土律），与 SlashPopup 补全框同构。
pub fn render_dialog_bottom(
    title: &str,
    border_color: Color,
    content: Stack,
    footer_hint: &str,
    ctx: &mut RenderContext,
    geom: PromptGeom,
    max_h: u16,
) -> revue::prelude::Rect {
    // Bottom 几何由 geom 决定；max_w 在 Bottom 路径忽略，传 geom.w 占位。
    render_positioned_dialog(
        title,
        border_color,
        content,
        footer_hint,
        ctx,
        DialogPlacement {
            anchor: DialogAnchor::Bottom,
            max_w: geom.w,
            max_h,
            geom: Some(geom),
        },
    )
}

/// Core: render a single-content dialog at `anchor`. Split out so the Centered
/// and Bottom wrappers share one border/title/positioned pipeline. Geometry
/// differs only by `anchor`.
///
/// 返回对话框外框的**绝对屏幕坐标** Rect（含边框），供调用方发布为鼠标
/// 命中区（土律：几何唯一权威——命中不再各自重算居中公式）。
fn render_positioned_dialog(
    title: &str,
    border_color: Color,
    content: Stack,
    footer_hint: &str,
    ctx: &mut RenderContext,
    placement: DialogPlacement,
) -> revue::prelude::Rect {
    let DialogPlacement {
        anchor,
        max_w,
        max_h,
        geom,
    } = placement;
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
            // 跟随输入框几何（prompt_geometry 唯一权威，土律）：宽=输入框宽、
            // x 对齐输入框、紧贴输入框正上方。max_w 在 Bottom 路径忽略（用 geom.w）。
            let g = geom.expect("Bottom anchor requires prompt geometry");
            let w = g.w;
            let h = max_h.min(g.y_top.saturating_sub(area.y).saturating_sub(1));
            let x = g.x.saturating_sub(area.x);
            let y = g.y_top.saturating_sub(area.y).saturating_sub(h);
            (w, h, x, y)
        }
    };

    // 实色填对话框矩形（实色不透字契约：positioned 浮层不清背景）。Bottom 现在
    // 跟随输入框宽，只填框区即可——对话框缩到输入框宽后不再与 Home col 0 按钮框
    // 水平重叠，无需全屏填遮底。居中框填框区（BG_SURFACE）。
    let (fill_x, fill_w, fill_bg) = match anchor {
        DialogAnchor::Centered => (x, w, colors::BG_SURFACE()),
        DialogAnchor::Bottom => (x, w, colors::BG_PRIMARY()),
    };
    paint_modal_backdrop(ctx, fill_x, y, fill_w, h, fill_bg);

    // Bottom 锚点无框（标题行 + content + hint，整片 BG_PRIMARY 融入终端），
    // 与 render_positioned_list 的 Bottom 分支同构——金律：Bottom 锚点只有
    // 一种成形语法。Centered 才套 Border::rounded（留给 alert/provider 等非
    // /命令对话框）。此前 Bottom 也套圆角框，导致 /sessions 的 loading 态
    // (render_dialog_bottom) 与加载完 (render_list_dialog_bottom) 框型不一致。
    match anchor {
        DialogAnchor::Bottom => {
            let view = vstack()
                .gap(0)
                .child_sized(
                    Text::new(format!(" {} ", title))
                        .fg(border_color)
                        .bg(colors::BG_PRIMARY())
                        .bold(),
                    1,
                )
                .child_flex(content, 1.0)
                .child_sized(
                    Text::new(fit_hint_tail(footer_hint, w))
                        .fg(colors::FG_MUTED())
                        .bg(colors::BG_PRIMARY())
                        .align(Alignment::Center),
                    1,
                );
            revue::widget::positioned(view)
                .x(x as i16)
                .y(y as i16)
                .width(w)
                .height(h)
                .render(ctx);
        }
        DialogAnchor::Centered => {
            let dialog = Border::rounded()
                .title(format!(" {} ", title))
                .fg(border_color)
                .child(
                    // flex 内容 + 固定 footer：此前两个等权 child 均分高度,
                    // 内容超半即被截尾(如 provider_edit 四字段丢最后一个)。
                    vstack().gap(1).child_flex(content, 1.0).child_sized(
                        Text::new(fit_hint_tail(footer_hint, w.saturating_sub(2)))
                            .fg(colors::FG_MUTED())
                            .align(Alignment::Center),
                        1,
                    ),
                );
            revue::widget::positioned(dialog)
                .x(x as i16)
                .y(y as i16)
                .width(w)
                .height(h)
                .render(ctx);
        }
    }
    // 实色不透字契约·第二道：positioned 内的 revue Text 以 `bg=None` 经
    // `buffer.set` 整体替换 cell，会把 paint_modal_backdrop 预填的底色在
    // 文字格上重新抹掉（与 `widget::bg_stack` 文档所述透字机理同源——
    // model/provider 编辑弹窗「浮在内容上文字互相渗透」正是此因）。
    // 渲染后再扫一遍对话框矩形，只给 bg 仍为 None 的 cell 补底色；
    // 内容自带的 bg（如列表选中行高亮）不受影响。
    for cy in (area.y + y)..(area.y + y + h) {
        for cx in (area.x + fill_x)..(area.x + fill_x + fill_w) {
            if let Some(cell) = ctx.buffer.get_mut(cx, cy) {
                if cell.bg.is_none() {
                    cell.bg = Some(fill_bg);
                }
            }
        }
    }

    // positioned 坐标相对 ctx.area 原点；鼠标命中用绝对屏幕坐标，此处换算后返回。
    revue::prelude::Rect::new(area.x + x, area.y + y, w, h)
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

pub struct ListDialogHeading<'a> {
    pub title: &'a str,
    pub border_color: Color,
}

/// Render a list dialog following the input box geometry —
/// width = input box width, x aligned to the input box, pinned just above it
/// (the command-picker anchor: /models, /sessions, /agents). Reads as "sitting
/// on the input box" rather than "floating in the middle". The list, sliding
/// viewport, scrollbar and selection contract are identical to the centred
/// variant — only the geometry source differs (PromptGeom, 土律).
pub fn render_list_dialog_bottom(
    heading: ListDialogHeading<'_>,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    geom: PromptGeom,
    visible_rows: usize,
) {
    let ListDialogHeading {
        title,
        border_color,
    } = heading;
    let _ = render_positioned_list(
        title,
        border_color,
        items,
        selected,
        footer_hint,
        ctx,
        ListPlacement {
            anchor: DialogAnchor::Bottom,
            max_w: geom.w,
            visible_rows,
            geom: Some(geom),
        },
    );
}

/// Same as [`render_list_dialog_bottom`] but also returns the layout of the
/// rendered dialog. Callers that want to overlay a tooltip / popover anchored
/// to the selected row, or publish the scrollbar geometry for mouse handling,
/// use this variant (/sessions).
pub fn render_list_dialog_bottom_with_layout(
    heading: ListDialogHeading<'_>,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    geom: PromptGeom,
    visible_rows: usize,
) -> ListDialogLayout {
    let ListDialogHeading {
        title,
        border_color,
    } = heading;
    render_positioned_list(
        title,
        border_color,
        items,
        selected,
        footer_hint,
        ctx,
        ListPlacement {
            anchor: DialogAnchor::Bottom,
            max_w: geom.w,
            visible_rows,
            geom: Some(geom),
        },
    )
}

/// Geometry for [`render_positioned_list`]: anchoring mode plus the sizing
/// inputs that mode needs (max width, viewport rows, optional prompt geometry).
struct ListPlacement {
    anchor: DialogAnchor,
    max_w: u16,
    visible_rows: usize,
    geom: Option<PromptGeom>,
}

/// Core list renderer: the sliding viewport, selection contract, scrollbar
/// overlay and tooltip-anchor layout all live here. Only the geometry (w/h/x/y)
/// depends on `anchor`; everything below is shared so the centred and
/// bottom-anchored pickers look identical except for position.
fn render_positioned_list(
    title: &str,
    border_color: Color,
    items: &[ListItem],
    selected: usize,
    footer_hint: &str,
    ctx: &mut RenderContext,
    placement: ListPlacement,
) -> ListDialogLayout {
    let ListPlacement {
        anchor,
        max_w,
        visible_rows,
        geom,
    } = placement;
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
            // 跟随输入框几何(土律:几何唯一真相):宽 = input 框宽,x 对齐 input 框,
            // 贴 input 框正上方。无框高度 = 标题(1) + rows + hint(1) = rows+2,
            // 上限不超过 input 框上方的可用空间。
            let g = geom.expect("Bottom anchor requires prompt geometry");
            let w = g.w;
            let h = (rows as u16 + 2).min(g.y_top.saturating_sub(area.y).saturating_sub(1));
            let x = g.x.saturating_sub(area.x);
            let y = g.y_top.saturating_sub(area.y).saturating_sub(h);
            (w, h, x, y)
        }
    };

    // 弹窗已缩到 input 框宽,不再与 Home col 0 按钮框水平重叠,实色底只填弹窗矩形
    // 本身(宽 w、x 起),无需全屏宽遮底。
    let (fill_x, fill_w, fill_bg) = match anchor {
        DialogAnchor::Centered => (x, w, colors::BG_SURFACE()),
        DialogAnchor::Bottom => (x, w, colors::BG_PRIMARY()),
    };
    paint_modal_backdrop(ctx, fill_x, y, fill_w, h, fill_bg);

    // Sliding viewport. The host's `selected` index counts ALL items
    // (Header rows included), so the viewport math operates on the same
    // coordinate space — no need to translate "Row index" vs "item index".
    //
    // 窗口算法收归 [`list_viewport_window`] 唯一权威(金律·成形语法唯一);
    // 任何关于「钉顶/钉底/lookahead」的调整都改那里一处。
    let (start, end) = list_viewport_window(total, selected, rows);

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
                let mut hdr = Text::new(format!(" ▸ {}", upper))
                    .bold()
                    .fg(colors::E_AMBER());
                // 无框贴底时补终端色 bg,否则文字格发黑/透字。
                if matches!(anchor, DialogAnchor::Bottom) {
                    hdr = hdr.bg(colors::BG_PRIMARY());
                }
                list_content = list_content.child_sized(hdr, 1);
            }
            ListItem::Row { display, muted } => {
                // Unified ❯ pointer (aligned with our own slash_popup).
                // Muted rows get no glyph —
                // their disabled state reads from the dim FG_MUTED color,
                // not from a special prefix. Non-selected rows use a
                // 2-space prefix to keep the column aligned with ❯.
                // (Previously this row used ▌ + ✓, and muted used ○ —
                // three different marks; now one across the whole app.)
                let (prefix, suffix) = if is_sel { ("❯ ", "") } else { ("  ", "") };

                // Build the unstyled row text (prefix + display) so we
                // can size the bg fill correctly. 按 inner_w 截断(留 …),
                // 否则 Home 64 宽下长 session 标题溢出 positioned 宽、
                // 裁半个 CJK——Bottom 几何已缩到输入框宽,内容必须自持。
                let line = format!("{}{}", prefix, display);
                let line = truncate_to_width(&line, inner_w);

                // Pad the selected row to fill inner width (display
                // columns, UAX#11) so the highlight bg spans the whole
                // row instead of just the text cells. suffix is now
                // empty — there's no right-edge marker (❯ + bold bg is
                // the selection signal).
                let padded = {
                    use unicode_width::UnicodeWidthStr;
                    let used =
                        UnicodeWidthStr::width(line.as_str()) + UnicodeWidthStr::width(suffix);
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
                    colors::FG_PRIMARY()
                } else if *muted {
                    colors::FG_MUTED()
                } else {
                    colors::FG_SECONDARY()
                };
                let mut row = Text::new(padded).fg(color);
                if is_sel {
                    row = row.bg(colors::SURFACE_SELECTED()).bold();
                } else if matches!(anchor, DialogAnchor::Bottom) {
                    // 无框贴底:非选中行补终端色 bg,否则文字格发黑/透字。
                    row = row.bg(colors::BG_PRIMARY());
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
        // 仅选中行 SURFACE_SELECTED 高亮，对齐轻量命令面板风格。
        // 标题行(1)替代了原 top border,故滚动条 sb_y=y+1、tooltip y+1+row_offset
        // 偏移与 Centered 一致,无需调整。
        let view = vstack()
            .gap(0)
            .child_sized(
                Text::new(title_with_pos)
                    .fg(border_color)
                    .bg(colors::BG_PRIMARY())
                    .bold(),
                1,
            )
            .child_flex(list_content, 1.0)
            .child_sized(
                Text::new(fit_hint_tail(footer_hint, w))
                    .fg(colors::FG_MUTED())
                    .bg(colors::BG_PRIMARY())
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
                vstack().gap(0).child_flex(list_content, 1.0).child_sized(
                    Text::new(fit_hint_tail(footer_hint, w.saturating_sub(2)))
                        .fg(colors::FG_MUTED())
                        .align(Alignment::Center),
                    1,
                ),
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
        let sb_x = ctx
            .area
            .x
            .saturating_add(x)
            .saturating_add(w.saturating_sub(2));
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
    let selected_row_y = if selected >= start
        && selected < end
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

#[cfg(test)]
mod tests {
    //! 回归保护：Bottom 锚点的单内容对话框必须真无框（四角无 ╭╮╰╯），
    //! 且整片 BG_PRIMARY 实色。此前 render_dialog_bottom 内部仍套
    //! Border::rounded，导致 /sessions 的 loading 态（走它）与加载完
    //! （render_list_dialog_bottom）框型不一致——金律违例。本测试钉住无框成形。

    use super::*;

    #[test]
    fn render_dialog_bottom_is_frameless() {
        let mut buf = Buffer::new(60, 20);
        let mut ctx = RenderContext::new(&mut buf, Rect::new(0, 0, 60, 20));
        let content = vstack().child(Text::new("body line").fg(colors::FG_SECONDARY()));
        // 输入框几何(模拟底部 prompt):宽 40、x=2、上沿 y_top=15。
        let geom = PromptGeom {
            x: 2,
            y_top: 15,
            w: 40,
        };
        render_dialog_bottom(
            "Title",
            colors::ACCENT_CYAN(),
            content,
            "hint",
            &mut ctx,
            geom,
            6,
        );

        // Bottom 几何跟随 geom:w=40, h=min(6, 15-0-1)=6, x=2, y=15-0-6=9
        // → 框区 [2,41]×[9,14],四角必须无 ╭╮╰╯(无框成形,金律)。
        for (x, y) in [(2, 9), (41, 9), (2, 14), (41, 14)] {
            let ch = buf.get(x, y).map(|c| c.symbol).unwrap_or(' ');
            assert!(
                !matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│'),
                "frameless violation: corner ({},{}) = {:?}",
                x,
                y,
                ch
            );
        }
        // 标题行 (y=9) 在框宽 [2,41] 内实色 BG_PRIMARY(paint_modal_backdrop 预填框矩形)。
        assert_eq!(
            buf.get(20, 9).and_then(|c| c.bg),
            Some(colors::BG_PRIMARY()),
            "title row must sit on solid BG_PRIMARY"
        );
        // 框外不再全宽遮底:右沿 41 之外(x=50)应透出下层,非 BG_PRIMARY。
        // 守住本轮核心改动——fill 从全屏宽收缩到输入框宽(土律:几何唯一真相)。
        assert_ne!(
            buf.get(50, 9).and_then(|c| c.bg),
            Some(colors::BG_PRIMARY()),
            "fill must follow dialog width, not full screen"
        );
    }

    #[test]
    fn render_dialog_centered_still_has_border() {
        // 对照组：Centered 锚点仍带圆角框（本次无框化不动它）。
        let mut buf = Buffer::new(60, 20);
        let mut ctx = RenderContext::new(&mut buf, Rect::new(0, 0, 60, 20));
        let content = vstack().child(Text::new("body").fg(colors::FG_SECONDARY()));
        render_dialog(
            "Title",
            colors::ACCENT_CYAN(),
            content,
            "hint",
            &mut ctx,
            40,
            6,
        );

        // Centered 几何：w=40, h=6, x=10, y=7 → 左上角 (10,7) 必须是边框字符
        let corner = buf.get(10, 7).map(|c| c.symbol).unwrap_or(' ');
        assert!(
            matches!(corner, '╭' | '╮' | '╰' | '╯' | '─' | '│'),
            "Centered must keep its border: (10,7) = {:?}",
            corner
        );
    }

    // ── list_viewport_window: sliding viewport 唯一权威的语义测试 ──
    //
    // 钉视野跟随选中的契约:无论 selected 在哪、列表多长,返回区间必须包含 selected,
    // 且端点行为(钉顶/钉底)符合「scroll just enough」语义。

    #[test]
    fn viewport_fits_when_total_le_rows() {
        // 列表整窗装下:无需滚动,窗口覆盖全列表。
        assert_eq!(list_viewport_window(5, 0, 8), (0, 5));
        assert_eq!(list_viewport_window(5, 4, 8), (0, 5));
        assert_eq!(list_viewport_window(8, 7, 8), (0, 8));
    }

    #[test]
    fn viewport_pins_top_when_selected_near_head() {
        // selected 在前 rows-1 位:窗口钉顶(start=0),看到列表头。
        assert_eq!(list_viewport_window(100, 0, 8), (0, 8));
        assert_eq!(list_viewport_window(100, 5, 8), (0, 8));
        assert_eq!(list_viewport_window(100, 6, 8), (0, 8)); // rows-1=7 边界
    }

    #[test]
    fn viewport_pins_bottom_when_selected_at_end() {
        // selected 在最后一项:窗口钉底(start=total-rows),看到列表尾。
        assert_eq!(list_viewport_window(100, 99, 8), (92, 100));
        assert_eq!(list_viewport_window(50, 49, 12), (38, 50));
    }

    #[test]
    fn viewport_slides_so_selected_stays_visible() {
        // 一般情况:selected 落在底数第 2 行,留 1 行 lookahead——
        // 关键不变量:start <= selected < end,即 selected 永远可见。
        for selected in 7..99 {
            let (start, end) = list_viewport_window(100, selected, 8);
            assert!(
                start <= selected && selected < end,
                "selected={} 不在 window [{},{}) 内",
                selected,
                start,
                end
            );
            assert!(end - start == 8, "窗口宽必须恒为 rows=8");
        }
    }

    #[test]
    fn viewport_handles_zero_total() {
        // 边界:空列表不 panic;窗口 (0,0)。
        let (start, end) = list_viewport_window(0, 0, 8);
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn fit_hint_tail_noop_when_fits() {
        assert_eq!(fit_hint_tail("Esc: close", 64), "Esc: close");
        assert_eq!(fit_hint_tail("abc", 3), "abc");
    }

    #[test]
    fn fit_hint_tail_preserves_escape_tail() {
        // U14①:mcp_list 的 120 字符 hint 在 64 宽输入框下必须保住尾部
        // "Esc: close"(逃生键),头部丢字用 … 明示。
        let hint = "↑↓ navigate  Home/End: jump  c: connect  d: disconnect  \
                    a/A: oauth  x: clear  n: add  e: edit  Enter: view  Esc: close";
        let got = fit_hint_tail(hint, 64);
        assert_eq!(got.chars().count(), 64);
        assert!(got.starts_with('…'), "截断须以 … 起头: {got}");
        assert!(got.ends_with("Esc: close"), "逃生键尾部必须保留: {got}");
    }

    #[test]
    fn fit_hint_tail_degenerate_width() {
        // w<=1 时只剩 …(连尾部也放不下,明示省略即可)。
        assert_eq!(fit_hint_tail("abcdef", 1), "…");
        assert_eq!(fit_hint_tail("abcdef", 0), "…");
    }

    /// U14② 验收：hint 与 handle_key 实际支持键一致。源级扫描钉住映射——
    /// 此后给 dialog 加新键却不写进 hint（或反之）会在这里立刻失败。
    #[test]
    fn dialog_hints_document_handled_keys() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/dialog");
        let read = |f: &str| {
            std::fs::read_to_string(format!("{dir}/{f}"))
                .unwrap_or_else(|e| panic!("read {f}: {e}"))
        };
        // Home/End 跳转处理器必须写进 hint。
        for f in [
            "agent_select.rs",
            "mode_select.rs",
            "mcp_list.rs",
            "skill_proposal.rs",
            "recovery_list.rs",
            "session_fork.rs",
            "notifications.rs",
            "skill_list.rs",
        ] {
            let s = read(f);
            assert!(s.contains("Key::Home"), "{f}: 前置——应有 Home 处理");
            assert!(s.contains("Home/End"), "{f}: hint 未写 Home/End");
        }
        // 过滤弹窗的 Backspace 擦除键必须写进 hint。
        for f in ["model_select.rs", "session_list.rs"] {
            let s = read(f);
            assert!(
                s.contains("Key::Backspace"),
                "{f}: 前置——应有 Backspace 处理"
            );
            assert!(s.contains('⌫'), "{f}: hint 未写 ⌫");
        }
        // confirm 的 'q' 取消键、permission 的 'a'/'d' 快捷键必须写进 hint。
        assert!(
            read("confirm.rs").contains("n/Esc/q: cancel"),
            "confirm hint 未写 q"
        );
        let p = read("permission.rs");
        assert!(p.contains("↵ select"), "permission hint 未写 Enter");
        assert!(p.contains("y/a allow once"), "permission hint 未写 a");
        assert!(p.contains("0/n/d deny"), "permission hint 未写 d");
    }
}
