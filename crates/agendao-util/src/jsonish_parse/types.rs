// =============================================================================
// Streaming ToolCall Parser — Production-Grade Implementation
// =============================================================================
//
// A streaming JSON parser designed for LLM tool-call recovery. Handles:
//   - Token-by-token streaming with byte-offset tracking
//   - Multi-object detection (tool call arrays)
//   - Array [] and object {} bracket balancing
//   - State-machine-aware repair (single quotes, control chars, trailing commas)
//   - Truncated JSON aggressive close
//   - Schema-aware tool detection with scoring
//
// KNOWN LIMITATION: The quote-tracking state machine (`in_string` toggled by `"`)
// will desync when string content contains unescaped double quotes (e.g. HTML
// attributes like `lang="zh-CN"`). This affects `escape_control_chars_in_strings`
// and `balance_brackets_stateful`. For such cases, the structural recovery in
// `util::json::recover_tool_call_ultra` should be used as a fallback.
// =============================================================================

use serde_json::Value;
use std::fmt;

// ─── Tool Schema ────────────────────────────────────────────────────────────

/// Describes a tool's JSON structure for matching parsed objects to tools.
#[derive(Clone, Debug)]
pub struct ToolSchema {
    pub name: String,
    /// Keys that must be present (higher match weight).
    pub required_keys: Vec<String>,
    /// Optional keys (lower match weight).
    pub optional_keys: Vec<String>,
}

// ─── Parse Result ───────────────────────────────────────────────────────────

/// Successful parse result with diagnostics.
#[derive(Debug, Clone)]
pub struct ToolParseResult {
    pub tool_name: String,
    pub value: Value,
    /// Byte range of this object in the original buffer.
    pub span: (usize, usize),
    /// Repair operations applied (for diagnostics).
    pub repairs: Vec<String>,
}

/// Parse failure diagnostics.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// No JSON object found in buffer.
    NoObject,
    /// JSON structure found but repair failed.
    InvalidJson {
        repaired: String,
        serde_error: String,
    },
    /// Valid JSON but no tool schema matched.
    NoToolMatch { value: Value },
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NoObject => write!(f, "No JSON object found in buffer"),
            ParseError::InvalidJson {
                repaired,
                serde_error,
            } => write!(
                f,
                "JSON repair failed: {} (repaired: {}...)",
                serde_error,
                &repaired[..repaired.len().min(100)]
            ),
            ParseError::NoToolMatch { .. } => write!(f, "No tool schema matched"),
        }
    }
}

// ─── Tracked Object ─────────────────────────────────────────────────────────

/// Tracks a top-level JSON object discovered in the buffer.
#[derive(Clone, Debug)]
pub(super) struct TrackedObject {
    /// Byte offset of the opening `{`.
    pub(super) start: usize,
    /// Byte offset past the closing `}`, if seen.
    pub(super) end: Option<usize>,
}

// ─── Scanner State ──────────────────────────────────────────────────────────

/// Character-level state machine for JSON structure tracking.
#[derive(Clone, Debug)]
pub(super) struct ScannerState {
    pub(super) brace_depth: i32,
    pub(super) bracket_depth: i32,
    pub(super) in_string: bool,
    pub(super) escape: bool,
    pub(super) byte_offset: usize,
}

impl ScannerState {
    pub(super) fn new() -> Self {
        Self {
            brace_depth: 0,
            bracket_depth: 0,
            in_string: false,
            escape: false,
            byte_offset: 0,
        }
    }
}
