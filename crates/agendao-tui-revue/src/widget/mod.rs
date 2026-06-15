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

pub mod scrollbar;
pub mod scroll_view;

pub use scrollbar::{Scrollbar, ScrollbarDrag, ScrollbarHit};
pub use scroll_view::{scroll_view, ScrollView, ScrollbarOverlay};
