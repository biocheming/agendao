//! AgenDao TUI — Revue-based reactive terminal frontend.
//!
//! Architecture (AgenDao 五行):
//!   store/    → 土 (State/Config authority)
//!   input/    → 木 (Input authority)
//!   execution/→ 火 (Execution authority)
//!   output/   → 金 (Output authority)
//!   telemetry/→ 水 (Telemetry/feedback authority)

pub mod app;
pub mod store;
pub mod screen;

pub use app::run_app;
