//! 火 — Transport layer: protocol-agnostic event sources.
//!
//! All three agendao transports (local-direct / unix-socket / HTTP-SSE)
//! deliver `FrontendEvent`. This module provides feature-gated
//! implementations that forward events to `EventBus::sender()`.

#[cfg(feature = "local-server")]
pub mod local;
#[cfg(not(feature = "local-server"))]
pub mod noop;

#[cfg(feature = "local-server")]
pub use local::spawn_event_source;
#[cfg(not(feature = "local-server"))]
pub use noop::spawn_event_source;
