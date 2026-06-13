//! 水 — Telemetry & event stream authority.
//!
//! Receives FrontendEvent from any transport (local direct / unix socket / HTTP SSE),
//! routes them to SessionStore Signals, and renders telemetry views.

pub mod event_bus;
pub mod event_handler;

pub use event_bus::EventBus;
pub use event_handler::apply_frontend_event;
