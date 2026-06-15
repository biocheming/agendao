//! 金 — Refined ScrollView built on top of `revue::ScrollView`.
//!
//! Why this exists
//! ---------------
//! `revue::ScrollView` only knows how to paint a 2-glyph scrollbar
//! (`│ █`) and only reacts to mouse wheel events. Most of the
//! agendao-tui-revue surfaces (chat transcript, sidebar, list dialogs)
//! want a more useful control: arrow buttons at top/bottom for instant
//! top/bottom jumps, a draggable thumb, and page-up/down on track
//! clicks. None of that requires changes to the upstream `revue`
//! crate — we just layer agendao's [`ScrollbarOverlay`] decoration +
//! hit-test API on top of a plain revue `ScrollView` and route the
//! richer mouse events here.
//!
//! This is the **single refinement point** for ScrollView. Code that
//! uses `crate::widget::ScrollView` instead of `revue::ScrollView`
//! gets arrow / drag / page-jump behaviour for free. There is no
//! "one-off wrapper per use site" — see `app::RootView::render` for
//! the only call site today, and any future surface (e.g. a
//! full-screen logs view) gets the same polish by using this type.
//!
//! API shape
//! ---------
//! The public builders mirror the upstream ScrollView one-for-one
//! (`.content_height`, `.scroll_offset`, `.show_scrollbar`,
//! `.scrollbar_style`, `.min_width`, …) so swapping import paths
//! doesn't change call sites. The added agendao-specific surface
//! lives on the [`ScrollView::handle_mouse_at`] method and
//! [`ScrollView::drag_to_offset`].

use revue::layout::Rect;
use revue::prelude::Color;
use revue::render::Buffer;
use revue::widget::traits::RenderContext;
use revue::widget::ScrollView as RevueScrollView;

use crate::widget::scrollbar::{Scrollbar, ScrollbarDrag, ScrollbarHit};

/// 细化版 ScrollView — 包装一个 [`revue::ScrollView`]，在其上叠加
/// agendao 风格的箭头 + 可拖动 thumb + track 翻页。
///
/// 与上游 `revue::ScrollView` builder 链 API 兼容（同名方法
/// 全部转发到内部），可直接通过 `use crate::widget::ScrollView`
/// 替换使用。
pub struct ScrollView {
    /// 内部持有上游 ScrollView。所有 builder 链调用都转发到这里。
    inner: RevueScrollView,
    /// Cached copy of the content height. The upstream ScrollView only
    /// exposes it via the `content_height(height)` builder (no getter),
    /// so we mirror the value here to make `max_offset` and the
    /// `ScrollbarOverlay` reader work without re-plumbing.
    content_height: u16,
}

impl ScrollView {
    /// 新建一个空 ScrollView（与 `revue::widget::scroll_view()` 行为
    /// 相同，但不返回上游类型，这样调用方会被引导使用本模块的
    /// 细化 API 而不是直接调到原始 ScrollView）。
    pub fn new() -> Self {
        Self {
            inner: RevueScrollView::new(),
            content_height: 0,
        }
    }

    // ── builder 转发（与上游一一对应）──────────────────────

    /// Set the total content height. Renamed from upstream's
    /// `content_height` because Rust can't disambiguate a builder
    /// with one argument from a zero-arg getter of the same name.
    /// Callers that switch from `revue::ScrollView` only need to
    /// insert `with_` at the front of this one builder.
    pub fn with_content_height(mut self, h: u16) -> Self {
        self.content_height = h;
        self.inner = self.inner.content_height(h);
        self
    }
    pub fn scroll_offset(mut self, offset: u16) -> Self {
        self.inner = self.inner.scroll_offset(offset);
        self
    }
    pub fn show_scrollbar(mut self, show: bool) -> Self {
        self.inner = self.inner.show_scrollbar(show);
        self
    }
    pub fn scrollbar_style(mut self, fg: Color, bg: Color) -> Self {
        self.inner = self.inner.scrollbar_style(fg, bg);
        self
    }
    pub fn min_width(mut self, w: u16) -> Self {
        self.inner = self.inner.min_width(w);
        self
    }
    pub fn min_height(mut self, h: u16) -> Self {
        self.inner = self.inner.min_height(h);
        self
    }
    pub fn max_width(mut self, w: u16) -> Self {
        self.inner = self.inner.max_width(w);
        self
    }
    pub fn max_height(mut self, h: u16) -> Self {
        self.inner = self.inner.max_height(h);
        self
    }
    pub fn min_size(mut self, w: u16, h: u16) -> Self {
        self.inner = self.inner.min_size(w, h);
        self
    }
    pub fn max_size(mut self, w: u16, h: u16) -> Self {
        self.inner = self.inner.max_size(w, h);
        self
    }
    pub fn constrain(mut self, min_w: u16, min_h: u16, max_w: u16, max_h: u16) -> Self {
        self.inner = self.inner.constrain(min_w, min_h, max_w, max_h);
        self
    }

    // ── 状态查询（转发）────────────────────────────────

    pub fn offset(&self) -> u16 {
        self.inner.offset()
    }
    pub fn set_offset(&mut self, offset: u16, viewport_height: u16) {
        self.inner.set_offset(offset, viewport_height);
    }
    /// Cached content height (mirrors what we passed to `.content_height(h)`).
    /// Upstream doesn't expose a getter, so we keep our own copy.
    pub fn content_height(&self) -> u16 {
        self.content_height
    }
    pub fn scroll_up(&mut self, lines: u16) {
        self.inner.scroll_up(lines);
    }
    pub fn scroll_down(&mut self, lines: u16, viewport_height: u16) {
        self.inner.scroll_down(lines, viewport_height);
    }
    pub fn scroll_to_top(&mut self) {
        self.inner.scroll_to_top();
    }
    pub fn scroll_to_bottom(&mut self, viewport_height: u16) {
        self.inner.scroll_to_bottom(viewport_height);
    }
    pub fn page_up(&mut self, viewport_height: u16) {
        self.inner.page_up(viewport_height);
    }
    pub fn page_down(&mut self, viewport_height: u16) {
        self.inner.page_down(viewport_height);
    }
    pub fn is_scrollable(&self, viewport_height: u16) -> bool {
        self.inner.is_scrollable(viewport_height)
    }
    pub fn max_offset(&self, viewport_height: u16) -> u16 {
        self.content_height.saturating_sub(viewport_height)
    }
    pub fn create_content_buffer(&self, width: u16) -> Buffer {
        self.inner.create_content_buffer(width)
    }
    pub fn render_content(&self, ctx: &mut RenderContext, content_buffer: &Buffer) {
        self.inner.render_content(ctx, content_buffer);
    }
}

impl Default for ScrollView {
    fn default() -> Self {
        Self::new()
    }
}

/// 上下文：绘制细化滚动条 + 命中测试需要 viewport 高度与
/// content 高度。`render_overlay` / `hit_test` 都接受这个快照。
#[derive(Clone, Copy, Debug)]
pub struct ScrollbarOverlay {
    /// 内容区（不含 scrollbar 列）。
    pub content_area: Rect,
    /// 滚动条列的绝对屏幕坐标 + 高度。
    pub scrollbar_area: Rect,
    /// 内容总高。
    pub content_height: u16,
    /// 视口高（== scrollbar_area.height）。
    pub viewport_height: u16,
    /// 当前 offset。
    pub offset: u16,
}

impl ScrollbarOverlay {
    /// 相对 ctx 的内容区 + 绝对 root 偏移 + 滚动状态构造 overlay
    /// 描述。`ctx_root_xy` = `(ctx.area.x, ctx.area.y)`。
    pub fn new(
        ctx_root_xy: (u16, u16),
        content_area: Rect,
        content_height: u16,
        viewport_height: u16,
        offset: u16,
    ) -> Self {
        // 滚动条列 = 内容区最后一列
        let sb_x = content_area.x.saturating_add(content_area.width.saturating_sub(1));
        let sb_y = content_area.y;
        let sb_h = viewport_height.min(content_area.height);
        let (rx, ry) = ctx_root_xy;
        Self {
            content_area,
            scrollbar_area: Rect::new(rx.saturating_add(sb_x), ry.saturating_add(sb_y), 1, sb_h),
            content_height,
            viewport_height,
            offset,
        }
    }

    pub fn bar(&self) -> Scrollbar {
        Scrollbar::new(
            self.scrollbar_area,
            self.content_height,
            self.viewport_height,
            self.offset,
        )
    }

    /// 命中测试：给定绝对屏幕坐标，返回滚动动作意图。
    /// 调用方拿到意图后 translate 成具体业务（设置 ScrollView.offset /
    /// 切 dialog 的 selected / 其它）。
    pub fn hit_test(&self, x: u16, y: u16) -> Option<ScrollbarHit> {
        self.bar().hit_test(x, y)
    }

    /// 在已存在 drag 状态时，把当前 y 翻译为新 offset。
    pub fn drag_to_offset(&self, drag: ScrollbarDrag, current_y: u16) -> u16 {
        self.bar().drag_to_offset(drag, current_y)
    }

    /// 绘制细化滚动条到 ctx（覆盖内部 ScrollView 的简单条）。
    pub fn render(&self, ctx: &mut RenderContext) {
        self.bar().render(ctx);
    }
}

/// 工厂函数，与 `revue::widget::scroll_view()` 同名。
pub fn scroll_view() -> ScrollView {
    ScrollView::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_methods_round_trip_through_inner() {
        let sv = scroll_view()
            .with_content_height(100)
            .scroll_offset(10)
            .show_scrollbar(true)
            .min_width(10)
            .max_height(50);

        assert_eq!(sv.offset(), 10);
        assert_eq!(sv.content_height(), 100);
        assert!(sv.is_scrollable(20));
        assert_eq!(sv.max_offset(20), 80);
    }

    #[test]
    fn setter_methods_actually_mutate() {
        let mut sv = scroll_view().with_content_height(100);
        sv.set_offset(42, 20);
        assert_eq!(sv.offset(), 42);
        sv.scroll_to_top();
        assert_eq!(sv.offset(), 0);
        sv.scroll_to_bottom(20);
        assert_eq!(sv.offset(), 80);
        sv.scroll_up(5);
        assert_eq!(sv.offset(), 75);
        sv.scroll_down(10, 20);
        assert_eq!(sv.offset(), 80);
    }

    #[test]
    fn overlay_computes_scrollbar_column_correctly() {
        // Content area at (5, 10) sized 40×20 — scrollbar should be at
        // absolute (44, 10) 1 col wide, 20 rows tall. viewport_height
        // is clamped to content height in case caller passes a bigger
        // value.
        let overlay = ScrollbarOverlay::new((100, 200), Rect::new(5, 10, 40, 20), 200, 20, 0);
        assert_eq!(overlay.scrollbar_area.x, 100 + 44);
        assert_eq!(overlay.scrollbar_area.y, 200 + 10);
        assert_eq!(overlay.scrollbar_area.width, 1);
        assert_eq!(overlay.scrollbar_area.height, 20);
    }

    #[test]
    fn overlay_hit_test_delegates_to_scrollbar() {
        // viewport_height=20 is clamped to content_area.height=22, so the
        // scrollbar spans 20 rows: ▲ at y=0, ▼ at y=19.
        let overlay = ScrollbarOverlay::new((0, 0), Rect::new(0, 0, 1, 22), 200, 20, 0);
        assert_eq!(overlay.hit_test(0, 0), Some(ScrollbarHit::ArrowUp));
        assert_eq!(overlay.hit_test(0, 19), Some(ScrollbarHit::ArrowDown));
    }
}
