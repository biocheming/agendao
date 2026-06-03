// =============================================================================
// Streaming ToolCall Parser — Production-Grade Implementation
// =============================================================================
//
// Modular structure:
//   - types.rs:    ToolSchema, ToolParseResult, ParseError, TrackedObject, ScannerState
//   - sanitize.rs: Phase 0 (strip framing noise) + Phase 1 (normalize syntax)
//   - repair.rs:   Phase 2 (structural JSON repair) + detect_tool + public API
//   - tests.rs:    Comprehensive test suite

mod repair;
mod sanitize;

pub mod types;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use repair::{detect_tool, repair_json};
pub use repair::{repair_json_standalone, sanitize_standalone};
pub use types::{ParseError, ToolParseResult, ToolSchema};
use types::{ScannerState, TrackedObject};

use serde_json::Value;

// ─── Main Parser ────────────────────────────────────────────────────────────

pub struct StreamingToolParser {
    buffer: String,
    state: ScannerState,
    objects: Vec<TrackedObject>,
    schemas: Vec<ToolSchema>,
}
impl StreamingToolParser {
    pub fn new(schemas: Vec<ToolSchema>) -> Self {
        Self {
            buffer: String::new(),
            state: ScannerState::new(),
            objects: Vec::new(),
            schemas,
        }
    }

    /// Current buffer content (for diagnostics).
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Number of top-level objects tracked so far.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    // ── Push delta ──────────────────────────────────────────────────────

    /// Push streaming delta into the parser. Core entry point.
    pub fn push(&mut self, delta: &str) {
        let base_offset = self.buffer.len();
        self.buffer.push_str(delta);

        let mut local_byte = 0usize;
        for ch in delta.chars() {
            let char_byte_offset = base_offset + local_byte;
            self.scan_char(ch, char_byte_offset);
            local_byte += ch.len_utf8();
        }

        self.state.byte_offset = self.buffer.len();
    }

    /// State machine: process a single character.
    fn scan_char(&mut self, ch: char, byte_offset: usize) {
        if self.state.in_string {
            if self.state.escape {
                self.state.escape = false;
                return;
            }
            if ch == '\\' {
                self.state.escape = true;
                return;
            }
            if ch == '"' {
                self.state.in_string = false;
            }
            return;
        }

        match ch {
            '"' => {
                self.state.in_string = true;
            }
            '{' => {
                if self.state.brace_depth == 0 && self.state.bracket_depth == 0 {
                    self.objects.push(TrackedObject {
                        start: byte_offset,
                        end: None,
                    });
                }
                self.state.brace_depth += 1;
            }
            '}' => {
                self.state.brace_depth = (self.state.brace_depth - 1).max(0);
                if self.state.brace_depth == 0 && self.state.bracket_depth == 0 {
                    if let Some(obj) = self.objects.last_mut() {
                        if obj.end.is_none() {
                            obj.end = Some(byte_offset + ch.len_utf8());
                        }
                    }
                }
            }
            '[' => {
                self.state.bracket_depth += 1;
            }
            ']' => {
                self.state.bracket_depth = (self.state.bracket_depth - 1).max(0);
            }
            _ => {}
        }
    }

    // ── Try parse (partial, tolerant) ───────────────────────────────────

    /// Try to parse the last discovered object. Safe to call mid-stream.
    pub fn try_parse(&self) -> Result<ToolParseResult, ParseError> {
        self.try_parse_object(self.objects.len().saturating_sub(1))
    }

    /// Try to parse all discovered objects.
    pub fn try_parse_all(&self) -> Vec<Result<ToolParseResult, ParseError>> {
        (0..self.objects.len())
            .map(|i| self.try_parse_object(i))
            .collect()
    }
    fn try_parse_object(&self, index: usize) -> Result<ToolParseResult, ParseError> {
        let obj = self.objects.get(index).ok_or(ParseError::NoObject)?;

        let end = obj.end.unwrap_or(self.buffer.len());
        let slice = &self.buffer[obj.start..end];

        let mut repairs = Vec::new();
        let repaired = repair_json(slice, false, &mut repairs);

        let value: Value =
            serde_json::from_str(&repaired).map_err(|e| ParseError::InvalidJson {
                repaired: repaired.clone(),
                serde_error: e.to_string(),
            })?;

        let tool_name = detect_tool(&value, &self.schemas).ok_or(ParseError::NoToolMatch {
            value: value.clone(),
        })?;

        Ok(ToolParseResult {
            tool_name,
            value,
            span: (obj.start, end),
            repairs,
        })
    }

    // ── Finalize (aggressive) ───────────────────────────────────────────

    /// Call when stream ends. Uses more aggressive repair strategies.
    pub fn finalize(&self) -> Result<ToolParseResult, ParseError> {
        self.finalize_object(self.objects.len().saturating_sub(1))
    }

    /// Finalize all discovered objects.
    pub fn finalize_all(&self) -> Vec<Result<ToolParseResult, ParseError>> {
        (0..self.objects.len())
            .map(|i| self.finalize_object(i))
            .collect()
    }

    fn finalize_object(&self, index: usize) -> Result<ToolParseResult, ParseError> {
        let obj = self.objects.get(index).ok_or(ParseError::NoObject)?;

        let end = obj.end.unwrap_or(self.buffer.len());
        let slice = &self.buffer[obj.start..end];

        let mut repairs = Vec::new();
        let repaired = repair_json(slice, true, &mut repairs);

        let value: Value =
            serde_json::from_str(&repaired).map_err(|e| ParseError::InvalidJson {
                repaired: repaired.clone(),
                serde_error: e.to_string(),
            })?;

        let tool_name = detect_tool(&value, &self.schemas).ok_or(ParseError::NoToolMatch {
            value: value.clone(),
        })?;

        Ok(ToolParseResult {
            tool_name,
            value,
            span: (obj.start, end),
            repairs,
        })
    }

    /// Reset parser state for reuse.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.state = ScannerState::new();
        self.objects.clear();
    }
}
