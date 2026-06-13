//! AgenDao TUI — Revue-based reactive terminal frontend.
//!
//! Architecture (AgenDao 五行):
//!   store/    → 土 (State/Config authority)
//!   input/    → 木 (Input authority)
//!   bridge/   → 火 (Execution authority)
//!   output/   → 金 (Output authority)
//!   telemetry/→ 水 (Telemetry/feedback authority)

pub mod app;
pub mod bridge;
pub mod input;
pub mod output;
pub mod store;
pub mod screen;

pub use app::run_app;
