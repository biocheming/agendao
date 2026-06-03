//! Plugin subprocess JSON-RPC 2.0 protocol types.
//!
//! JSON-RPC 2.0 envelope types are re-exported from `agendao_core::jsonrpc`
//! (the single authority per Constitution Article 1). This module provides
//! backward-compatible aliases so consumer code migrates incrementally.

// Re-export from the single authority.
pub use agendao_core::jsonrpc::{
    JsonRpcError as RpcError, JsonRpcMessage as RpcMessage, JsonRpcNotification as RpcNotification,
    JsonRpcRequest as RpcRequest, JsonRpcResponse as RpcResponse,
};
