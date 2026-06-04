//! Stable low-level shared primitives for AgenDao.
//!
//! Boundary rules for this crate:
//! - Keep the default surface lightweight and broadly reusable.
//! - Do not introduce network, database, UI, or parser dependencies here.
//! - Runtime/process coordination stays behind explicit crate features.

#[cfg(feature = "agent-task-registry")]
pub mod agent_task_registry;
#[cfg(feature = "event-bus")]
pub mod bus;
#[cfg(feature = "subprocess-runtime")]
pub mod codec;
pub mod id;
pub mod jsonrpc;
#[cfg(feature = "process-registry")]
pub mod process_registry;
#[cfg(feature = "subprocess-runtime")]
pub mod stderr_drain;

#[cfg(feature = "event-bus")]
pub use bus::*;
pub use id::*;
