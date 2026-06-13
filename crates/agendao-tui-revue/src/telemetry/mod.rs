//! 水 — Telemetry & event stream authority.

pub mod event_bus;
pub mod event_handler;
pub mod sidebar;

pub use event_bus::EventBus;
pub use event_handler::apply_frontend_event;
pub use sidebar::SessionSidebar;
