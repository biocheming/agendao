//! OpenAI Responses API types and streaming support.
//!
//! This module provides full feature parity with the TypeScript SDK's
//! `openai-responses-language-model.ts`, including:
//! - Response chunk types (12+ discriminated types)
//! - Streaming state management
//! - Response parsing schemas
//! - Model configuration detection
//! - Provider options schema

mod helpers;
mod runtime;
mod types;
mod validation;

#[cfg(test)]
#[path = "tests.rs"]
mod runtime_tests;

// Re-export all public items for backward compatibility
pub use types::*;
pub use validation::*;
