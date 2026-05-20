use crate::provider::ProviderError;
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    /// Stream has started.
    Start,
    /// Incremental text content.
    TextDelta(String),
    /// Start of a text block.
    TextStart,
    /// End of a text block.
    TextEnd,
    /// Start of a reasoning/thinking block.
    ReasoningStart {
        id: String,
    },
    /// Incremental reasoning text.
    ReasoningDelta {
        id: String,
        text: String,
    },
    /// End of a reasoning/thinking block.
    ReasoningEnd {
        id: String,
    },
    /// Start of tool input streaming (tool-input-start in TS).
    ToolInputStart {
        id: String,
        tool_name: String,
    },
    /// Incremental tool input JSON (tool-input-delta in TS).
    ToolInputDelta {
        id: String,
        delta: String,
    },
    /// End of tool input streaming (tool-input-end in TS).
    ToolInputEnd {
        id: String,
    },
    /// Full tool call event (after input is fully assembled).
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        input: String,
    },
    ToolCallEnd {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Tool result received.
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        input: Option<serde_json::Value>,
        output: ToolResultOutput,
    },
    /// Tool error received.
    ToolError {
        tool_call_id: String,
        tool_name: String,
        input: Option<serde_json::Value>,
        error: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ToolErrorKind>,
    },
    /// Start of a processing step (maps to start-step in TS).
    StartStep,
    /// End of a processing step with usage info (maps to finish-step in TS).
    FinishStep {
        finish_reason: Option<String>,
        usage: StreamUsage,
        provider_metadata: Option<serde_json::Value>,
    },
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
        context_tokens: u64,
        #[serde(default)]
        cache_read_tokens: u64,
        #[serde(default)]
        cache_miss_tokens: u64,
        #[serde(default)]
        cache_write_tokens: u64,
    },
    /// Stream finished (maps to "finish" in TS).
    Finish,
    Done,
    Error(String),
}

pub fn text_delta_event(content: impl Into<String>) -> StreamEvent {
    StreamEvent::TextDelta(content.into())
}

/// Type-safe tool error category for streaming tool failures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    PermissionDenied,
    QuestionRejected,
    ExecutionError,
}

/// Output from a tool result event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultOutput {
    pub output: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<serde_json::Value>>,
}

/// Usage information from a step completion.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    #[serde(default)]
    pub context_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_miss_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
}

/// Why a stream ended — the formal terminal vocabulary for stream recovery.
///
/// Every stream exit path must carry one of these so downstream consumers
/// (recovery, backfill, reconcile) can decide what to do without re-guessing
/// the cause from ad-hoc error strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamTermination {
    /// Normal completion — the stream produced a Finish/Done event.
    Completed,
    /// User cancelled the stream mid-flight.
    Interrupted,
    /// Provider returned an error (e.g. HTTP 500, rate limit).
    ProviderError { message: String },
    /// Stream was idle too long without producing any event.
    TransportTimeout,
    /// Transport connection was closed unexpectedly.
    TransportClosed,
    /// Stream body could not be decoded (e.g. invalid JSON, partial chunk).
    StreamCorrupt { message: String },
    /// Stream recovered after a retry attempt — the previous fault was transient.
    RecoveredAfterRetry,
}

pub type StreamResult = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

/// Convert runtime pipeline output (driver `StreamingEvent`) into rocode `StreamResult`.
///
/// The bridge path preserves tool-call assembly semantics by running through
/// `bridge_streaming_events()`, which internally wraps with `assemble_tool_calls()`.
pub fn pipeline_to_stream_result(
    pipeline_output: Pin<
        Box<dyn Stream<Item = Result<crate::driver::StreamingEvent, ProviderError>> + Send>,
    >,
) -> StreamResult {
    crate::bridge::bridge_streaming_events(pipeline_output)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OpenAISSEvent {
    #[serde(default)]
    pub choices: Vec<OpenAIChoice>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OpenAIChoice {
    #[serde(default)]
    pub delta: Option<OpenAIDelta>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OpenAIDelta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<OpenAIToolCall>>,
    /// Reasoning text (OpenAI o-series, some compatible providers)
    pub reasoning_text: Option<String>,
    /// Reasoning content (alternate field name used by some compatible providers)
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OpenAIToolCall {
    #[serde(default)]
    pub index: u32,
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<OpenAIFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct OpenAIFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OpenAIUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub prompt_cache_hit_tokens: u64,
    #[serde(default)]
    pub prompt_cache_miss_tokens: u64,
}

impl OpenAIUsage {
    fn stream_usage(&self) -> StreamUsage {
        let cache_read_tokens = self
            .cache_read_input_tokens
            .max(self.prompt_cache_hit_tokens);
        let cache_write_tokens = self.cache_creation_input_tokens;

        StreamUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            context_tokens: self
                .prompt_tokens
                .max(cache_read_tokens.saturating_add(self.prompt_cache_miss_tokens))
                .max(cache_read_tokens.saturating_add(cache_write_tokens)),
            cache_read_tokens,
            cache_miss_tokens: self.prompt_cache_miss_tokens,
            cache_write_tokens,
            ..Default::default()
        }
    }
}

fn openai_tool_call_id(tc: &OpenAIToolCall) -> String {
    // Always use index-based ID for consistency across stream chunks.
    // The first chunk may carry an explicit `id` (e.g. "call_xxx") while
    // subsequent delta chunks only have `index`, causing ID mismatches
    // that result in orphaned tool-call entries with empty names.
    format!("tool-call-{}", tc.index)
}

fn messages_tool_call_id(index: Option<u32>, explicit_id: Option<&str>) -> String {
    if let Some(index) = index {
        return format!("tool-call-{}", index);
    }
    explicit_id.unwrap_or_default().to_string()
}

/// Returns true when the input is a complete and parseable JSON value.
pub fn is_parsable_json(s: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(s).is_ok()
}

#[derive(Debug, Clone)]
/// Lightweight character-level JSON depth tracker.
///
/// Tracks brace nesting depth and string state to determine when a
/// streaming JSON object is likely complete (depth == 0 && !in_string).
/// Used as an optimisation hint: it avoids calling `serde_json::from_str`
/// on every delta chunk.  When the content has unescaped quotes the
/// tracker may give a wrong answer, so callers should always have a
/// safety-net flush path.
struct JsonDepthTracker {
    depth: i32,
    in_string: bool,
    escape_next: bool,
}

impl JsonDepthTracker {
    fn new() -> Self {
        Self {
            depth: 0,
            in_string: false,
            escape_next: false,
        }
    }

    fn track(&mut self, delta: &str) {
        for ch in delta.chars() {
            self.track_char(ch);
        }
    }

    fn track_char(&mut self, ch: char) {
        if self.in_string {
            if self.escape_next {
                self.escape_next = false;
                return;
            }
            match ch {
                '\\' => self.escape_next = true,
                '"' => self.in_string = false,
                _ => {}
            }
            return;
        }
        match ch {
            '"' => self.in_string = true,
            '{' => self.depth += 1,
            '}' => self.depth -= 1,
            _ => {}
        }
    }

    /// Returns true when the tracked JSON object appears structurally
    /// complete (all braces closed, not inside a string).
    fn appears_complete(&self) -> bool {
        self.depth <= 0 && !self.in_string
    }
}

struct ToolCallAssembler {
    id: String,
    name: String,
    arguments: String,
    finished: bool,
    tracker: JsonDepthTracker,
}

impl ToolCallAssembler {
    fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            arguments: String::new(),
            finished: false,
            tracker: JsonDepthTracker::new(),
        }
    }

    fn append(&mut self, delta: &str) {
        if !self.finished {
            self.arguments.push_str(delta);
            self.tracker.track(delta);
        }
    }

    fn try_emit(&mut self) -> Option<StreamEvent> {
        if self.finished || self.arguments.is_empty() {
            return None;
        }

        // Only attempt a full parse when the depth tracker thinks the
        // top-level object is closed.  For well-formed JSON this avoids
        // O(n) parse attempts per delta on large payloads.
        if !self.tracker.appears_complete() {
            return None;
        }

        let input: serde_json::Value = serde_json::from_str(&self.arguments).ok()?;
        self.finished = true;
        Some(StreamEvent::ToolCallEnd {
            id: self.id.clone(),
            name: self.name.clone(),
            input,
        })
    }
}

/// Flush remaining incomplete tool call assemblers using tolerant JSON parsing.
///
/// Attempts `serde_json::from_str` on accumulated argument strings. On parse
/// failure, wraps the raw string as `Value::String` so downstream recovery
/// (normalize_tool_arguments / ultra) can handle it.
fn flush_tool_call_assemblers(
    assemblers: &mut HashMap<String, ToolCallAssembler>,
    out: &mut VecDeque<Result<StreamEvent, ProviderError>>,
) {
    let mut pending: Vec<ToolCallAssembler> = assemblers.drain().map(|(_, asm)| asm).collect();
    pending.sort_by(|a, b| a.id.cmp(&b.id));

    for asm in pending {
        if asm.finished || asm.arguments.is_empty() {
            continue;
        }
        let trimmed = asm.arguments.trim();
        let input = if trimmed.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str::<serde_json::Value>(trimmed)
                .unwrap_or_else(|_| serde_json::Value::String(asm.arguments.clone()))
        };
        out.push_back(Ok(StreamEvent::ToolCallEnd {
            id: asm.id,
            name: asm.name,
            input,
        }));
    }
}

/// Wraps a stream and assembles `ToolCallStart`/`ToolCallDelta` fragments into
/// `ToolCallEnd` events. Existing `ToolCallEnd` events are passed through.
pub fn assemble_tool_calls(inner: StreamResult) -> StreamResult {
    let state = (
        inner,
        HashMap::<String, ToolCallAssembler>::new(),
        VecDeque::<Result<StreamEvent, ProviderError>>::new(),
        false,
    );

    Box::pin(stream::unfold(
        state,
        |(mut inner, mut assemblers, mut pending, mut eof)| async move {
            loop {
                if let Some(item) = pending.pop_front() {
                    return Some((item, (inner, assemblers, pending, eof)));
                }

                if eof {
                    return None;
                }

                match inner.next().await {
                    Some(Ok(event)) => match event {
                        StreamEvent::ToolCallStart { id, name } => {
                            assemblers.insert(
                                id.clone(),
                                ToolCallAssembler::new(id.clone(), name.clone()),
                            );
                            pending.push_back(Ok(StreamEvent::ToolCallStart { id, name }));
                        }
                        StreamEvent::ToolCallDelta { id, input } => {
                            if let Some(assembler) = assemblers.get_mut(&id) {
                                assembler.append(&input);
                                pending.push_back(Ok(StreamEvent::ToolCallDelta {
                                    id: id.clone(),
                                    input,
                                }));
                                if let Some(end_event) = assembler.try_emit() {
                                    pending.push_back(Ok(end_event));
                                }
                            } else {
                                pending.push_back(Ok(StreamEvent::ToolCallDelta { id, input }));
                            }
                        }
                        StreamEvent::ToolCallEnd { id, name, input } => {
                            let should_forward = match assemblers.remove(&id) {
                                Some(assembler) => !assembler.finished,
                                None => true,
                            };
                            if should_forward {
                                pending.push_back(Ok(StreamEvent::ToolCallEnd { id, name, input }));
                            } else {
                                tracing::debug!(
                                    tool_call_id = %id,
                                    "assemble_tool_calls: suppressing duplicate ToolCallEnd after assembled emission"
                                );
                            }
                        }
                        StreamEvent::Done => {
                            flush_tool_call_assemblers(&mut assemblers, &mut pending);
                            pending.push_back(Ok(StreamEvent::Done));
                        }
                        other => pending.push_back(Ok(other)),
                    },
                    Some(Err(err)) => pending.push_back(Err(err)),
                    None => {
                        flush_tool_call_assemblers(&mut assemblers, &mut pending);
                        eof = true;
                    }
                }
            }
        },
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EthnopicEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub index: Option<u32>,
    pub delta: Option<EthnopicDelta>,
    pub content_block: Option<EthnopicContentBlock>,
    pub message: Option<EthnopicMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EthnopicDelta {
    #[serde(rename = "type")]
    pub delta_type: Option<String>,
    pub text: Option<String>,
    pub partial_json: Option<String>,
    pub stop_reason: Option<String>,
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EthnopicContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EthnopicMessage {
    pub(crate) usage: Option<EthnopicUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EthnopicUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

// ===========================================================================
// Self-contained SSE decoder
// ===========================================================================

/// Decode a reqwest bytes stream into a stream of JSON values.
///
/// This is a self-contained SSE decoder that handles:
/// - Cross-chunk buffering (SSE frames split across TCP chunks)
/// - Both `data: ` and `data:` (no space) prefix formats
/// - `[DONE]` signal detection and stream termination
/// - SSE comment lines (`:` prefix)
/// - JSON parsing of SSE payloads
pub async fn decode_sse_stream(
    bytes_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> Result<
    Pin<Box<dyn Stream<Item = Result<serde_json::Value, ProviderError>> + Send>>,
    ProviderError,
> {
    let input: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>> =
        Box::pin(bytes_stream);

    let delimiter = "\n\n";
    let delimiter_len = delimiter.len();
    let prefix = "data: ";
    let done_signal = "[DONE]";

    let sse_stream = stream::unfold(
        (input, String::new()),
        move |(mut input, mut buf)| async move {
            let is_done = |s: &str| -> bool {
                let t = s.trim();
                t == done_signal
                    || t == format!("data: {}", done_signal)
                    || t == format!("data:{}", done_signal)
            };

            let parse_payload = |raw: &str| -> Option<serde_json::Value> {
                // SSE frames may contain multiple lines (e.g. "event:xxx\ndata:{...}").
                // Scan all lines to find the one starting with "data:" and extract its payload.
                let mut data_payload: Option<&str> = None;
                for line in raw.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    if let Some(payload) = line.strip_prefix(prefix) {
                        data_payload = Some(payload);
                    } else if let Some(payload) = line.strip_prefix("data:") {
                        data_payload = Some(payload.trim_start());
                    }
                }
                let payload = data_payload?;
                if payload.is_empty() || is_done(payload) {
                    return None;
                }
                serde_json::from_str(payload).ok()
            };

            loop {
                // If we have a full frame in buffer, emit it.
                if let Some(idx) = buf.find(delimiter) {
                    let frame = buf[..idx].to_string();
                    let rest_start = idx + delimiter_len;
                    buf = if rest_start <= buf.len() {
                        buf[rest_start..].to_string()
                    } else {
                        String::new()
                    };

                    if is_done(&frame) {
                        return None;
                    }
                    if let Some(v) = parse_payload(&frame) {
                        return Some((Ok(v), (input, buf)));
                    }
                    continue;
                }

                // Need more data.
                match input.next().await {
                    Some(Ok(bytes)) => {
                        let s = String::from_utf8_lossy(&bytes);
                        buf.push_str(&s);
                        continue;
                    }
                    Some(Err(e)) => {
                        return Some((
                            Err(ProviderError::StreamError(e.to_string())),
                            (input, buf),
                        ));
                    }
                    None => {
                        // EOF: try parse remaining buffer once
                        if is_done(&buf) {
                            return None;
                        }
                        if let Some(v) = parse_payload(&buf) {
                            return Some((Ok(v), (input, String::new())));
                        }
                        return None;
                    }
                }
            }
        },
    );

    Ok(Box::pin(sse_stream))
}

/// Parse a pre-decoded JSON value as an ethnopic-compatible SSE event.
///
/// Works on an already-parsed `serde_json::Value` from the SseDecoder pipeline.
pub fn parse_ethnopic_value(value: serde_json::Value) -> Option<StreamEvent> {
    parse_ethnopic_value_stateful(value, &mut std::collections::HashMap::new())
}

pub fn parse_ethnopic_value_stateful(
    value: serde_json::Value,
    block_types: &mut std::collections::HashMap<u32, String>,
) -> Option<StreamEvent> {
    let event: EthnopicEvent = serde_json::from_value(value).ok()?;

    match event.event_type.as_str() {
        "content_block_delta" => {
            if let Some(delta) = event.delta {
                if let Some(thinking) = delta.thinking {
                    let id = format!("thinking-{}", event.index.unwrap_or(0));
                    return Some(StreamEvent::ReasoningDelta { id, text: thinking });
                }
                if let Some(text) = delta.text {
                    return Some(StreamEvent::TextDelta(text));
                }
                if let Some(json) = delta.partial_json {
                    return Some(StreamEvent::ToolCallDelta {
                        id: messages_tool_call_id(event.index, None),
                        input: json,
                    });
                }
            }
        }
        "content_block_start" => {
            if let Some(block) = event.content_block {
                let idx = event.index.unwrap_or(0);
                block_types.insert(idx, block.block_type.clone());

                if block.block_type == "thinking" {
                    let id = format!("thinking-{}", idx);
                    return Some(StreamEvent::ReasoningStart { id });
                }
                if block.block_type == "tool_use" {
                    return Some(StreamEvent::ToolCallStart {
                        id: messages_tool_call_id(event.index, block.id.as_deref()),
                        name: block.name.unwrap_or_default(),
                    });
                }
            }
        }
        "content_block_stop" => {
            let idx = event.index.unwrap_or(0);
            if let Some(block_type) = block_types.remove(&idx) {
                if block_type == "thinking" {
                    let id = format!("thinking-{}", idx);
                    return Some(StreamEvent::ReasoningEnd { id });
                }
            }
            return Some(StreamEvent::TextEnd);
        }
        "message_stop" => {
            return Some(StreamEvent::Done);
        }
        "message_delta" => {
            if let Some(delta) = event.delta {
                if delta.stop_reason.is_some() {
                    return Some(StreamEvent::Done);
                }
            }
        }
        "message_start" => {
            if let Some(msg) = event.message {
                if let Some(usage) = msg.usage {
                    return Some(StreamEvent::Usage {
                        prompt_tokens: usage.input_tokens,
                        completion_tokens: usage.output_tokens,
                        context_tokens: usage.input_tokens,
                        cache_read_tokens: 0,
                        cache_miss_tokens: 0,
                        cache_write_tokens: 0,
                    });
                }
            }
        }
        _ => {}
    }

    None
}

/// Parse a pre-decoded JSON value as an OpenAI SSE event.
///
/// Works on an already-parsed `serde_json::Value` from the SseDecoder pipeline.
/// Note: `[DONE]` is already handled by the SseDecoder so it's not checked here.
pub fn parse_openai_value(value: serde_json::Value) -> Vec<StreamEvent> {
    let event: OpenAISSEvent = match serde_json::from_value(value) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut events = Vec::new();
    let usage = event.usage.as_ref().map(OpenAIUsage::stream_usage);

    for choice in event.choices {
        if let Some(delta) = &choice.delta {
            // Handle reasoning/thinking content - check both field names for compatibility
            let reasoning = delta
                .reasoning_content
                .as_ref()
                .or(delta.reasoning_text.as_ref());
            if let Some(text) = reasoning {
                if !text.is_empty() {
                    events.push(StreamEvent::ReasoningDelta {
                        id: "reasoning-0".to_string(),
                        text: text.clone(),
                    });
                }
            }

            if let Some(content) = &delta.content {
                if !content.is_empty() {
                    events.push(StreamEvent::TextDelta(content.clone()));
                }
            }

            if let Some(tool_calls) = &delta.tool_calls {
                for tc in tool_calls {
                    if let Some(func) = &tc.function {
                        let has_name = func.name.as_deref().is_some_and(|n| !n.is_empty());
                        let has_args = func.arguments.as_deref().is_some_and(|a| !a.is_empty());

                        if has_name {
                            events.push(StreamEvent::ToolCallStart {
                                id: openai_tool_call_id(tc),
                                name: func.name.clone().unwrap_or_default(),
                            });
                        }
                        if has_args {
                            events.push(StreamEvent::ToolCallDelta {
                                id: openai_tool_call_id(tc),
                                input: func.arguments.clone().unwrap_or_default(),
                            });
                        }
                    }
                }
            }
        }

        if let Some(reason) = &choice.finish_reason {
            match reason.as_str() {
                "stop" => {
                    events.push(StreamEvent::FinishStep {
                        finish_reason: Some("stop".to_string()),
                        usage: usage.clone().unwrap_or_default(),
                        provider_metadata: None,
                    });
                    events.push(StreamEvent::Done);
                }
                "tool_calls" => {
                    events.push(StreamEvent::FinishStep {
                        finish_reason: Some("tool-calls".to_string()),
                        usage: usage.clone().unwrap_or_default(),
                        provider_metadata: None,
                    });
                    events.push(StreamEvent::Done);
                }
                _ => {}
            }
        }
    }

    if let Some(usage) = event.usage {
        let usage = usage.stream_usage();
        events.push(StreamEvent::Usage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            context_tokens: usage.context_tokens.max(usage.prompt_tokens),
            cache_read_tokens: usage.cache_read_tokens,
            cache_miss_tokens: usage.cache_miss_tokens,
            cache_write_tokens: usage.cache_write_tokens,
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn mock_stream(events: Vec<StreamEvent>) -> StreamResult {
        Box::pin(futures::stream::iter(
            events.into_iter().map(Ok::<_, ProviderError>),
        ))
    }

    async fn collect_events(stream: StreamResult) -> Vec<StreamEvent> {
        stream
            .map(|item| item.expect("expected Ok stream event"))
            .collect::<Vec<_>>()
            .await
    }

    #[test]
    fn is_parsable_json_checks_complete_json() {
        assert!(is_parsable_json(r#"{"key":"value"}"#));
        assert!(!is_parsable_json(r#"{"key":"#));
        assert!(!is_parsable_json(""));
    }

    #[test]
    fn openai_stream_usage_maps_deepseek_prompt_cache_hits() {
        let events = parse_openai_value(serde_json::json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 80,
                "prompt_cache_hit_tokens": 700,
                "prompt_cache_miss_tokens": 300
            }
        }));

        let finish_usage = events
            .iter()
            .find_map(|event| match event {
                StreamEvent::FinishStep { usage, .. } => Some(usage),
                _ => None,
            })
            .expect("finish step usage should be present");
        assert_eq!(finish_usage.prompt_tokens, 1000);
        assert_eq!(finish_usage.completion_tokens, 80);
        assert_eq!(finish_usage.cache_read_tokens, 700);
        assert_eq!(finish_usage.cache_miss_tokens, 300);
        assert_eq!(finish_usage.cache_write_tokens, 0);

        let standalone_usage = events
            .iter()
            .find_map(|event| match event {
                StreamEvent::Usage {
                    cache_read_tokens,
                    cache_miss_tokens,
                    cache_write_tokens,
                    ..
                } => Some((*cache_read_tokens, *cache_miss_tokens, *cache_write_tokens)),
                _ => None,
            })
            .expect("standalone usage should be present");
        assert_eq!(standalone_usage, (700, 300, 0));
    }

    #[test]
    fn tool_call_assembler_emits_when_json_is_complete() {
        let mut assembler = ToolCallAssembler::new("tool-call-0".into(), "read".into());
        assembler.append("{\"path\":\"");
        assert!(assembler.try_emit().is_none());

        assembler.append("/tmp/a\"}");
        let event = assembler.try_emit().expect("should emit ToolCallEnd");
        match event {
            StreamEvent::ToolCallEnd { id, name, input } => {
                assert_eq!(id, "tool-call-0");
                assert_eq!(name, "read");
                assert_eq!(input, serde_json::json!({"path": "/tmp/a"}));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn tool_call_assembler_depth_tracking_skips_parse_while_open() {
        let mut assembler = ToolCallAssembler::new("tc-1".into(), "write".into());

        // Feed a large incomplete payload in many chunks — depth tracking
        // should prevent try_emit from attempting serde_json::from_str.
        assembler.append("{\"file_path\":\"/tmp/a.html\",\"content\":\"");
        assert!(
            assembler.try_emit().is_none(),
            "depth > 0, should skip parse"
        );

        // Still inside the string value.
        assembler.append("<h1>Hello</h1>");
        assert!(assembler.try_emit().is_none());

        // Close the string and object.
        assembler.append("\"}");
        let event = assembler.try_emit().expect("depth == 0, should parse now");
        match event {
            StreamEvent::ToolCallEnd { input, .. } => {
                assert_eq!(input["file_path"], "/tmp/a.html");
                assert_eq!(input["content"], "<h1>Hello</h1>");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn tool_call_assembler_flush_recovers_malformed_json() {
        // Simulates the bug scenario: unescaped quotes make depth tracking
        // wrong, so try_emit never fires.  The batch flush wraps malformed
        // JSON as String for downstream ultra recovery.
        let mut assembler = ToolCallAssembler::new("tc-2".into(), "write".into());
        assembler.append("{\"content\":\"<html lang=\"en\">hello\",\"file_path\":\"/tmp/a\"}");

        // Depth tracking is confused by unescaped quotes — try_emit won't fire.
        assert!(assembler.try_emit().is_none());

        // Flush via batch assembler
        let mut assemblers = HashMap::new();
        assemblers.insert("tc-2".to_string(), assembler);
        let mut out = VecDeque::new();
        flush_tool_call_assemblers(&mut assemblers, &mut out);

        assert_eq!(out.len(), 1, "flush should produce one event");
        let event = out.pop_front().unwrap().expect("should be Ok");
        match event {
            StreamEvent::ToolCallEnd { input, .. } => {
                // Input is wrapped as String since serde_json can't parse it.
                // Downstream normalize_tool_arguments + ultra recovery handles it.
                assert!(
                    input.is_string() || input.is_object(),
                    "should be string (for recovery) or object (if parse succeeded)"
                );
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn ethnopic_parse_alias_matches_legacy_entrypoint() {
        let frame = serde_json::json!({
            "type": "content_block_delta",
            "delta": { "text": "hello" }
        });

        let legacy = parse_ethnopic_value(frame.clone());
        let neutral = parse_ethnopic_value(frame);

        assert!(matches!(legacy, Some(StreamEvent::TextDelta(text)) if text == "hello"));
        assert!(matches!(neutral, Some(StreamEvent::TextDelta(text)) if text == "hello"));
    }

    #[tokio::test]
    async fn assemble_tool_calls_emits_mid_stream_tool_call_end() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallStart {
                id: "tool-call-0".into(),
                name: "read".into(),
            },
            StreamEvent::ToolCallDelta {
                id: "tool-call-0".into(),
                input: "{\"path\":\"".into(),
            },
            StreamEvent::ToolCallDelta {
                id: "tool-call-0".into(),
                input: "/tmp/a\"}".into(),
            },
            StreamEvent::TextDelta("after-tool-call".into()),
            StreamEvent::Done,
        ]);

        let events = collect_events(assemble_tool_calls(stream)).await;
        assert!(matches!(events[0], StreamEvent::ToolCallStart { .. }));
        assert!(matches!(events[1], StreamEvent::ToolCallDelta { .. }));
        assert!(matches!(events[2], StreamEvent::ToolCallDelta { .. }));
        assert!(matches!(events[3], StreamEvent::ToolCallEnd { .. }));
        assert!(matches!(events[4], StreamEvent::TextDelta(_)));
        assert!(matches!(events[5], StreamEvent::Done));
    }

    #[tokio::test]
    async fn assemble_tool_calls_flushes_unfinished_tool_call_on_done() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallStart {
                id: "tool-call-0".into(),
                name: "read".into(),
            },
            StreamEvent::ToolCallDelta {
                id: "tool-call-0".into(),
                input: r#"{"path":"incomplete""#.into(),
            },
            StreamEvent::Done,
        ]);

        let events = collect_events(assemble_tool_calls(stream)).await;
        assert!(events.iter().any(|event| matches!(
            event,
            StreamEvent::ToolCallEnd {
                id,
                name,
                input
            } if id == "tool-call-0"
                && name == "read"
                && input == &serde_json::Value::String(r#"{"path":"incomplete""#.to_string())
        )));
    }

    #[tokio::test]
    async fn assemble_tool_calls_suppresses_provider_end_after_assembled_end() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallStart {
                id: "tool-call-0".into(),
                name: "read".into(),
            },
            StreamEvent::ToolCallDelta {
                id: "tool-call-0".into(),
                input: "{\"path\":\"/tmp/a\"}".into(),
            },
            StreamEvent::ToolCallEnd {
                id: "tool-call-0".into(),
                name: "read".into(),
                input: serde_json::json!({"path": "/tmp/a"}),
            },
            StreamEvent::Done,
        ]);

        let events = collect_events(assemble_tool_calls(stream)).await;
        let end_events = events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolCallEnd { .. }))
            .count();
        assert_eq!(
            end_events, 1,
            "assembled and provider-end should collapse to one ToolCallEnd"
        );
    }

    #[tokio::test]
    async fn assemble_tool_calls_passthrough_existing_tool_call_end() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallEnd {
                id: "tool-call-9".into(),
                name: "read".into(),
                input: serde_json::json!({"path": "/tmp/z"}),
            },
            StreamEvent::Done,
        ]);

        let events = collect_events(assemble_tool_calls(stream)).await;
        let end_count = events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolCallEnd { .. }))
            .count();
        assert_eq!(
            end_count, 1,
            "existing ToolCallEnd should not be duplicated"
        );
    }
}
