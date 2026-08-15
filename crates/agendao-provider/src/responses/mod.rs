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
#[cfg(feature = "streaming")]
mod runtime;
pub mod types;
pub mod validation;

#[cfg(test)]
#[path = "tests.rs"]
mod runtime_tests;
