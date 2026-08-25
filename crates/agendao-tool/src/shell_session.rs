//! Persistent, PTY-backed shell sessions.
//!
//! The public tool is intentionally thin. `launcher` owns sandbox starts,
//! `manager` owns live terminal state, and `runtime` owns operation dispatch.

mod launcher;
mod manager;
mod runtime;
mod schema;

pub use runtime::ShellSessionTool;

#[cfg(test)]
use manager::{append_session_output, ShellSessionState, ShellSessionView};
#[cfg(test)]
use schema::{shell_metadata, BUFFER_LIMIT};

#[cfg(test)]
#[path = "shell_session_tests.rs"]
mod tests;
