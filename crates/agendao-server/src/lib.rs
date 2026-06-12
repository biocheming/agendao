#![allow(ambiguous_glob_reexports)]

pub mod error;
#[cfg(feature = "mcp")]
pub mod mcp_oauth;
pub mod oauth;
pub mod openapi;
pub mod orchestration_adapter; // Phase 3: OrchestrationCore 接口验证
pub(crate) mod recovery;
pub(crate) mod request_options;
pub mod routes;
pub mod server;
pub(crate) mod session_runtime;
pub mod unix_socket; // Phase 5: Unix Socket 传输层
pub mod web;
pub mod worktree;

pub use agendao_server_core::runtime_control;
pub use agendao_server_core::runtime_state;
pub use agendao_server_core::stage_event_log;
pub use agendao_server_core::stage_summary_store;
pub use error::*;
#[cfg(feature = "mcp")]
pub use mcp_oauth::*;
pub use oauth::*;
pub use openapi::*;
pub use routes::*;
pub use session_runtime::direct_bridge::spawn_direct_event_bus;
pub use server::*;
pub use session_runtime::direct_bridge::spawn_direct_event_loop;
pub use unix_socket::*;
pub use web::*;
pub use worktree::*;
