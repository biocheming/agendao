//! 金 — Reusable Widgets
//!
//! Agendao 控件"细化层"——以组合（composition）方式在 revue 原生
//! 控件之上加值，让业务方在不接触 revue 内部的前提下获得更丰富的
//! 交互形态。
//!
//! ## 现有控件
//!
//! - [`scrollbar::Scrollbar`] — 单列可视化（箭头 + thumb + track），
//!   不绑定任何滚动数据，可被任意 widget 复用。
//! - [`scroll_view::ScrollView`] — 细化版 `revue::ScrollView`，兼容
//!   上游同名 builder，叠加箭头 / 拖拽 / 翻页点击，所有 ScrollView
//!   使用者切换到本类型即获得增强。
//! - [`status_icon::status_icon`] — 状态图标+颜色的单一权威
//!   (ToolPhase / TodoStatus / StageState / Result)，消除 session 与
//!   sidebar 的口径分裂（金律：输出成形语法单点）。
//! - [`spinner`] — 可插拔 glyph 集（Braille/Dots）+ 平台感知，替代硬编码帧。
//! - [`blink`] — 600ms 周期闪烁原语（useBlink 风格），驱动工具状态点。
//! - [`bg_stack`] — 带整块背景的 Stack 包装（render 时 `fill_bg` 铺底），
//!   让 message block 获得同色系明度层次（revue Stack 不自带背景 fill）。
//! - [`vline`] — 垂直分隔线（`VLine`）：与 `bg_stack` 对偶，1 列宽整高填 `│`，
//!   只动 `cell.symbol`+`fg` 保留 bg；作 sidebar↔主区纯黑合一边界。
//! - [`wrap_editor`] — soft-wrap 编辑器控件（`WrapEditor` 编辑层 +
//!   `EditorView` 视图层）：细化 `revue::TextArea`，自带 ❯ 箭头、
//!   滚动窗、闪烁光标与命中几何回流；prompt 输入框的编辑/渲染权威。

pub mod scrollbar;
pub mod scroll_view;
pub mod status_icon;
pub mod spinner;
pub mod blink;
pub mod bg_stack;
pub mod vline;
pub mod wrap_editor;

pub use scrollbar::{Scrollbar, ScrollbarDrag, ScrollbarHit};
pub use scroll_view::{scroll_view, ScrollView, ScrollbarOverlay};
pub use vline::VLine;
