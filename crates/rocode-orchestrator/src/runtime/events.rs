use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// LoopEvent – the single normalized event type that LoopSink receives.
// StreamEvent → LoopEvent conversion happens exactly once in the normalizer.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// Incremental text content from the model.
    TextChunk(String),

    /// Incremental reasoning / thinking text.
    ReasoningChunk { id: String, text: String },

    /// A fully assembled, ready-to-execute tool call.
    ToolCallReady(ToolCallReady),

    /// Streaming progress for a tool call (Sink may choose to ignore).
    ToolCallProgress {
        id: String,
        name: Option<String>,
        partial_input: String,
    },

    /// Model stream finished for this step.
    StepDone {
        finish_reason: FinishReason,
        usage: Option<StepUsage>,
    },

    /// Error from model stream.
    Error(String),
}

// ---------------------------------------------------------------------------
// ToolCallReady – a complete tool call ready for dispatch.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolCallReady {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// ToolResult – output from a dispatched tool call.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// StepBoundary – emitted at the start and end of each agentic step.
// End variant carries result context so Sink does not need to infer it.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum StepBoundary {
    Start {
        step: u32,
    },
    End {
        step: u32,
        finish_reason: FinishReason,
        tool_calls_count: u32,
        had_error: bool,
        usage: Option<StepUsage>,
    },
}

#[derive(Debug, Clone)]
pub enum StepCheckpointDirective {
    Continue,
    CompactRequestView {
        focus: Option<String>,
        reason: Option<String>,
    },
    ReplaceRequestView {
        messages: Vec<rocode_provider::Message>,
        reason: Option<String>,
    },
    Block {
        reason: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestViewMetrics {
    pub message_count: usize,
    pub system_prefix_messages: usize,
    pub compactable_messages: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_messages: usize,
    pub checkpoint_summary_messages: usize,
    pub estimated_context_tokens: Option<u64>,
    pub estimated_body_chars: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestViewMutationKind {
    Compacted,
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestViewMutation {
    pub kind: RequestViewMutationKind,
    pub reason: Option<String>,
    pub focus: Option<String>,
    pub before: RequestViewMetrics,
    pub after: RequestViewMetrics,
    pub compacted_message_count: Option<usize>,
    pub summary_chars: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepCheckpointSnapshot {
    pub assessment_index: usize,
    pub max_assessments: usize,
    pub current_view: RequestViewMetrics,
    pub previous_view: Option<RequestViewMetrics>,
    pub prior_mutations: Vec<RequestViewMutation>,
}

impl StepCheckpointSnapshot {
    pub fn remaining_assessments(&self) -> usize {
        self.max_assessments.saturating_sub(self.assessment_index)
    }

    pub fn rewrite_attempted(&self) -> bool {
        !self.prior_mutations.is_empty()
    }

    pub fn compaction_attempted(&self) -> bool {
        self.prior_mutations
            .iter()
            .any(|mutation| matches!(mutation.kind, RequestViewMutationKind::Compacted))
    }

    pub fn compaction_succeeded(&self) -> bool {
        self.compaction_attempted()
    }
}

// ---------------------------------------------------------------------------
// FinishReason – why a step or the entire loop ended.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    /// Model finished naturally (no tool calls in response).
    EndTurn,
    /// Model finished with tool calls pending execution.
    ToolUse,
    /// Max steps limit reached.
    MaxSteps,
    /// Cancelled via CancelToken.
    Cancelled,
    /// Model or stream error.
    Error(String),
    /// Provider-reported finish reason (passthrough).
    Provider(String),
}

// ---------------------------------------------------------------------------
// StepUsage – token usage for a single step.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepUsage {
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

impl StepUsage {
    pub fn merge_snapshot(&mut self, snapshot: &Self) {
        self.prompt_tokens = self.prompt_tokens.max(snapshot.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(snapshot.completion_tokens);
        self.context_tokens = self.context_tokens.max(snapshot.context_tokens);
        self.reasoning_tokens = self.reasoning_tokens.max(snapshot.reasoning_tokens);
        self.cache_read_tokens = self.cache_read_tokens.max(snapshot.cache_read_tokens);
        self.cache_miss_tokens = self.cache_miss_tokens.max(snapshot.cache_miss_tokens);
        self.cache_write_tokens = self.cache_write_tokens.max(snapshot.cache_write_tokens);
    }

    pub fn accumulate(&mut self, delta: &Self) {
        self.prompt_tokens += delta.prompt_tokens;
        self.completion_tokens += delta.completion_tokens;
        self.context_tokens = self
            .context_tokens
            .max(delta.context_tokens.max(delta.prompt_tokens));
        self.reasoning_tokens += delta.reasoning_tokens;
        self.cache_read_tokens += delta.cache_read_tokens;
        self.cache_miss_tokens += delta.cache_miss_tokens;
        self.cache_write_tokens += delta.cache_write_tokens;
    }
}

// ---------------------------------------------------------------------------
// LoopRequest – input to ModelCaller. Only conversation-level data.
// Model-specific config (temperature, max_tokens) is ModelCaller's concern.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LoopRequest {
    pub messages: Vec<rocode_provider::Message>,
    pub tools: Vec<rocode_provider::ToolDefinition>,
}

// ---------------------------------------------------------------------------
// LoopOutcome – final result of a run_loop invocation.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LoopOutcome {
    pub content: String,
    pub total_steps: u32,
    pub total_tool_calls: u32,
    pub finish_reason: FinishReason,
}

#[derive(Debug, Clone)]
pub enum ModelFailure {
    Provider(rocode_provider::ProviderErrorSummary),
    Message(String),
}

impl ModelFailure {
    pub fn message(&self) -> &str {
        match self {
            Self::Provider(summary) => summary.message.as_str(),
            Self::Message(message) => message.as_str(),
        }
    }

    pub fn provider_summary(&self) -> Option<&rocode_provider::ProviderErrorSummary> {
        match self {
            Self::Provider(summary) => Some(summary),
            Self::Message(_) => None,
        }
    }

    pub fn is_provider_not_found(&self) -> bool {
        matches!(
            self,
            Self::Provider(summary)
                if summary.kind == rocode_provider::ProviderErrorKind::ProviderNotFound
        )
    }
}

impl fmt::Display for ModelFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

// ---------------------------------------------------------------------------
// LoopError – errors that abort the loop or propagate from Sink/Dispatcher.
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("model call failed: {0}")]
    ModelError(ModelFailure),

    #[error("sink rejected event: {0}")]
    SinkError(String),

    #[error("tool dispatch failed: {tool} - {error}")]
    ToolDispatchError { tool: String, error: String },

    #[error("loop cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// CancelToken – cooperative cancellation check.
// ---------------------------------------------------------------------------

pub trait CancelToken: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

/// No-op cancel token that never cancels.
pub struct NeverCancel;

impl CancelToken for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}
