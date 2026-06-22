//! 金 — 带整块背景的 Stack 包装控件。
//!
//! revue 的 Stack 自身不 fill 背景——revue 的背景填充由各 widget 自管
//!（`View::render` 无默认 fill，`Stack::render` 只布局 children）。要让一个
//! message block 获得整块淡背景（opencode 式同色系明度阶梯），需要额外的
//! fill。但 revue 的 Text 经 `draw_text_with_style` 用 `buffer.set` **整体
//! 替换** cell、且 `Text.bg` 默认 `None`——任何预填的背景都会被文字 cell
//! 覆盖清除，造成「文字格无背景、空白格有背景」的破碎。
//!
//! 正解：先让 inner Stack 正常画完内容，再遍历 area 只把每个 cell 的 `bg`
//! 设为块色（不动 char/fg）——既铺满整块背景、又保留文字可读。本控件封装
//! 这一行为。角色到背景色的映射见 [`crate::screen::block_bg`]（与
//! `block_accent` 同构）。

use revue::prelude::*;

/// 整块背景包装：`render` 时先画 inner Stack 内容，再遍历 area 只补 `cell.bg`
/// （保留 char/fg），让 message block 获得同色系明度背景层次。
///
/// 用法：`BgStack::new(stack, colors::BG_SURFACE)`，由 `block_bg` 决定哪些
/// block 加背景、用哪一档明度。
pub struct BgStack {
    inner: revue::widget::Stack,
    bg: Color,
}

impl BgStack {
    pub fn new(inner: revue::widget::Stack, bg: Color) -> Self {
        Self { inner, bg }
    }
}

impl View for BgStack {
    fn render(&self, ctx: &mut RenderContext) {
        // 先画内容（阳：生发）。
        self.inner.render(ctx);
        // 再补涂整块背景（阴：承载）——只动 cell.bg，保留文字 char/fg。
        // revue Text 经 buffer.set 整体替换 cell，故必须内容之后补、且只改 bg。
        let area = ctx.area;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = ctx.buffer.get_mut(x, y) {
                    cell.bg = Some(self.bg);
                }
            }
        }
    }

    // 转发 children 供 DOM/devtools 遍历；渲染布局由 inner.render 负责。
    fn children(&self) -> &[Box<dyn View>] {
        self.inner.children()
    }
}
