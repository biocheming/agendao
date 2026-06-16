//! design-system（Layer 1）：主题收口 + 语义色单点 + Ds* 控件封装。
//!
//! 阴阳定位：阴面收口权威（`resolve_color` 单点、`Theme` 单一注册），
//! 阳面被 widget/screen 层消费。颜色真值权威是 `styles/base.css` 的
//! `:root` 变量；Rust 侧 `Theme` 只管运行时 variant 切换。
//!
//! 子模块按 Task 增量建立：theme（Task 1-2）→ color/text/primitives（Task 3-4）。

pub mod theme;
