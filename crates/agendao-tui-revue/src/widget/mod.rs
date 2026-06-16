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
//! - [`role_chip`] — message block 角色标签 (role → chip 文本 + 语义色) 单点。
//! - [`blink`] — 600ms 周期闪烁原语（claudecode useBlink），驱动工具状态点。
//! - [`message_response`] — claudecode `⎿` 缩进视觉语法（tool 子消息前缀）。

pub mod scrollbar;
pub mod scroll_view;
pub mod status_icon;
pub mod spinner;
pub mod role_chip;
pub mod blink;
pub mod message_response;

pub use scrollbar::{Scrollbar, ScrollbarDrag, ScrollbarHit};
pub use scroll_view::{scroll_view, ScrollView, ScrollbarOverlay};
