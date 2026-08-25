//! Re-export of the sandbox execution boundary.
//!
//! The trait definition lives in `agendao-sandbox` (the domain crate)
//! since Phase 6: MCP, LSP, plugin hosts, and the PTY surface consume
//! the same authority without depending on a tooling crate. This module
//! keeps the historical `agendao_tool_core::SandboxExecutionBoundary`
//! path stable for existing consumers.

pub use agendao_sandbox::{
    IntegrationSandboxContext, ProfileKind, SandboxExecutionBoundary,
    SharedSandboxExecutionBoundary,
};
