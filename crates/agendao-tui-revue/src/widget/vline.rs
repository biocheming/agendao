//! 金 — 垂直分隔线：1 列宽、整高填 `│`（纯黑合一边界）。
//!
//! 与 [`crate::widget::bg_stack::BgStack`] 是对偶关系——两者共享同一套
//! `ctx.buffer.get_mut(x, y)` → `&mut Cell` 的 fill 模式，只是「涂」的维度不同：
//!
//! - `BgStack` 只动 `cell.bg`（保留 char/fg）→ 给一块区域铺整块背景。
//! - `VLine` 只动 `cell.symbol` + `cell.fg`（保留 bg）→ 给一列区域画整高竖线。
//!
//! 纯黑合一（Pure Black Unity）：sidebar 不再包 BgStack(BG_DEEP)，与主窗口
//! 共享终端纯黑背景；两区之间仅靠本控件画一根极暗淡（`SIDEBAR_DIVIDER`
//! #2e3440）的 `│` 划界。背景透出终端黑、线压到「刚可辨」的暗度——深川·流白。

use revue::prelude::*;

/// 垂直分隔线：`render` 时遍历 area 把每个 cell 的 `symbol` 设为 `│`、
/// `fg` 设为给定色（保留 bg，让终端黑透出）。
///
/// 用法：`VLine::new(colors::SIDEBAR_DIVIDER)`，置于 hstack 中作 1 列宽 child。
pub struct VLine {
    color: Color,
}

impl VLine {
    /// 以给定前景色画整高 `│` 列。颜色通常用极暗的 [`SIDEBAR_DIVIDER`]。
    ///
    /// [`SIDEBAR_DIVIDER`]: crate::theme::colors::SIDEBAR_DIVIDER
    pub fn new(color: Color) -> Self {
        Self { color }
    }
}

impl View for VLine {
    fn render(&self, ctx: &mut RenderContext) {
        // 遍历 area 整块，逐 cell 设竖线符号 + 前景色（保留 bg）。
        // 宽度通常 = 1（hstack 里 child_sized(_, 1)），但即使更宽也整块填满。
        let area = ctx.area;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = ctx.buffer.get_mut(x, y) {
                    cell.symbol = '│';
                    cell.fg = Some(self.color);
                }
            }
        }
    }

    // 无子节点——children() 走 View trait 默认实现（返回 &[]）。
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_color() {
        let v = VLine::new(Color::rgb(46, 52, 64));
        assert_eq!(v.color, Color::rgb(46, 52, 64));
    }
}
