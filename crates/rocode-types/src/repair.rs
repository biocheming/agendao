//! Unified repair event types — the single source of truth for all tool-call
//! repair telemetry across the system.
//!
//! ## Design principles (from ROCode Constitution)
//! - Every repair event has exactly one authoritative schema (this module).
//! - Adapters reference the schema; they never replicate it.
//! - The session telemetry accumulator reads structured fields, not loose keys.

use crate::ToolRepairCount;
use serde::{Deserialize, Serialize};

/// A single repair event recorded during tool-call processing.
///
/// This replaces the previously loose `Map<String, Value>` pattern with a
/// type-safe struct whose fields are stable and queryable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RepairEvent {
    /// The classification of this repair (e.g. "alias_normalization",
    /// "basename_auto_repair", "tool_name_repair").
    pub repair_kind: String,

    /// Which architectural layer recorded the repair.
    /// Common values: "tool", "session_prompt", "provider", "sanitizer".
    pub layer: String,

    /// The tool whose call triggered this repair.
    pub tool_name: String,

    /// The specific argument field that was repaired, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,

    /// Human-readable explanation of why the repair was needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// The raw shape the model emitted before repair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_shape: Option<serde_json::Value>,

    /// The normalized shape after repair was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_shape: Option<serde_json::Value>,

    /// Whether this repair result was injected back into the model context
    /// (e.g. as a synthetic tool_result or corrected input).
    #[serde(default)]
    pub injected_into_model_context: bool,

    /// Whether a hypothetical strict-mode execution would have rejected
    /// the original model output instead of repairing it.
    #[serde(default)]
    pub strict_mode_would_fail: bool,

    /// When a permissive reroute rewrites the execution path, this records
    /// the original error kind that would have propagated in strict mode.
    /// Uses the same stable strings as `classify_error_kind`:
    /// `invalid_arguments`, `permission_denied`, `timeout`,
    /// `provider_rejected`, `execution_error`, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_error_kind: Option<String>,
}

impl RepairEvent {
    /// Create a minimal repair event with only the required three fields.
    pub fn new(
        repair_kind: impl Into<String>,
        layer: impl Into<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        Self {
            repair_kind: repair_kind.into(),
            layer: layer.into(),
            tool_name: tool_name.into(),
            field: None,
            reason: None,
            raw_shape: None,
            normalized_shape: None,
            injected_into_model_context: false,
            strict_mode_would_fail: false,
            original_error_kind: None,
        }
    }

    /// Convert to a loose JSON object for backward-compatible storage
    /// in the existing `toolRepairTelemetry` metadata slot.
    pub fn to_loose_map(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "kind".to_string(),
            serde_json::Value::String(self.repair_kind.clone()),
        );
        map.insert(
            "layer".to_string(),
            serde_json::Value::String(self.layer.clone()),
        );
        map.insert(
            "tool".to_string(),
            serde_json::Value::String(self.tool_name.clone()),
        );
        if let Some(ref field) = self.field {
            map.insert(
                "field".to_string(),
                serde_json::Value::String(field.clone()),
            );
        }
        if let Some(ref reason) = self.reason {
            map.insert(
                "reason".to_string(),
                serde_json::Value::String(reason.clone()),
            );
        }
        if let Some(ref raw_shape) = self.raw_shape {
            map.insert("raw_shape".to_string(), raw_shape.clone());
        }
        if let Some(ref normalized_shape) = self.normalized_shape {
            map.insert("normalized_shape".to_string(), normalized_shape.clone());
        }
        map.insert(
            "injected_into_model_context".to_string(),
            serde_json::Value::Bool(self.injected_into_model_context),
        );
        map.insert(
            "strict_mode_would_fail".to_string(),
            serde_json::Value::Bool(self.strict_mode_would_fail),
        );
        if let Some(ref original_error_kind) = self.original_error_kind {
            map.insert(
                "original_error_kind".to_string(),
                serde_json::Value::String(original_error_kind.clone()),
            );
        }
        map
    }

    /// Reconstruct from a loose JSON object (backward-compatible read).
    pub fn from_loose_map(map: &serde_json::Map<String, serde_json::Value>) -> Option<Self> {
        let repair_kind = map.get("kind")?.as_str()?.to_string();
        let layer = map.get("layer")?.as_str()?.to_string();
        let tool_name = map.get("tool")?.as_str()?.to_string();
        Some(Self {
            repair_kind,
            layer,
            tool_name,
            field: map
                .get("field")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            reason: map
                .get("reason")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            raw_shape: map.get("raw_shape").cloned(),
            normalized_shape: map.get("normalized_shape").cloned(),
            injected_into_model_context: map
                .get("injected_into_model_context")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            strict_mode_would_fail: map
                .get("strict_mode_would_fail")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            original_error_kind: map
                .get("original_error_kind")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
        })
    }
}

/// Builder for `RepairEvent` — provides a fluent, type-safe way to construct
/// repair events with optional fields.
#[derive(Debug, Clone, Default)]
pub struct RepairEventBuilder {
    event: RepairEvent,
}

impl RepairEventBuilder {
    pub fn new(
        repair_kind: impl Into<String>,
        layer: impl Into<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        Self {
            event: RepairEvent::new(repair_kind, layer, tool_name),
        }
    }

    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.event.field = Some(field.into());
        self
    }

    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.event.reason = Some(reason.into());
        self
    }

    pub fn raw_shape(mut self, value: serde_json::Value) -> Self {
        self.event.raw_shape = Some(value);
        self
    }

    pub fn normalized_shape(mut self, value: serde_json::Value) -> Self {
        self.event.normalized_shape = Some(value);
        self
    }

    pub fn injected_into_model_context(mut self, value: bool) -> Self {
        self.event.injected_into_model_context = value;
        self
    }

    pub fn strict_mode_would_fail(mut self, value: bool) -> Self {
        self.event.strict_mode_would_fail = value;
        self
    }

    pub fn build(self) -> RepairEvent {
        self.event
    }
}

// ── Tool Batch Summary types (P2.2) ────────────────────────────────────

/// Whether this tool batch advanced the task goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBatchGoalStatus {
    /// All calls succeeded with clear output.
    Advanced,
    /// All calls failed for a single identifiable reason.
    Blocked,
    /// Mix of successes and failures.
    Mixed,
    /// No progress made but no clear blocker identified.
    NoProgress,
}

/// The category of reason blocking progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBatchBlockReason {
    InvalidArguments,
    PermissionDenied,
    Timeout,
    ProviderRejected,
    ToolExecutionError,
    UserInputRequired,
    Unknown,
}

impl ToolBatchBlockReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::PermissionDenied => "permission_denied",
            Self::Timeout => "timeout",
            Self::ProviderRejected => "provider_rejected",
            Self::ToolExecutionError => "tool_execution_error",
            Self::UserInputRequired => "user_input_required",
            Self::Unknown => "unknown",
        }
    }
}

/// A lightweight follow-up item for the next turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolBatchFollowUpItem {
    /// e.g. "retry", "inspect_artifact", "ask_user", "fix_args"
    pub kind: String,
    pub text: String,
}

/// Tool batch summary emitted after one round of tool execution.
///
/// Provides the model with a structured read on what happened, whether the
/// goal advanced, what blocked progress, and what to do next.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolBatchSummary {
    pub tools_used: Vec<String>,
    pub success_count: u32,
    pub error_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_kinds: Vec<String>,

    pub goal_status: ToolBatchGoalStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<ToolBatchBlockReason>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts_created: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_follow_up: Vec<ToolBatchFollowUpItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_items: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_next_step: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repair_events: Vec<RepairEvent>,
}

impl ToolBatchSummary {
    pub fn empty() -> Self {
        Self {
            tools_used: Vec::new(),
            success_count: 0,
            error_count: 0,
            error_kinds: Vec::new(),
            goal_status: ToolBatchGoalStatus::NoProgress,
            blocked_by: Vec::new(),
            artifacts_created: Vec::new(),
            pending_follow_up: Vec::new(),
            unresolved_items: Vec::new(),
            recommended_next_step: None,
            repair_events: Vec::new(),
        }
    }

    pub fn has_meaningful_progress(&self) -> bool {
        self.success_count > 0 && self.error_count == 0
    }

    pub fn is_blocked(&self) -> bool {
        !self.blocked_by.is_empty()
    }

    /// Format as a compact model-visible block. Empty fields are omitted.
    /// Output order: tools → goal_status → success/error → blocked_by →
    /// artifacts → next_step → unresolved.
    pub fn format_for_context(&self) -> String {
        let mut lines = vec!["<tool-batch-summary>".to_string()];

        if !self.tools_used.is_empty() {
            lines.push(format!("  tools: {}", self.tools_used.join(", ")));
        }

        lines.push(format!(
            "  goal_status: {}",
            match self.goal_status {
                ToolBatchGoalStatus::Advanced => "advanced",
                ToolBatchGoalStatus::Blocked => "blocked",
                ToolBatchGoalStatus::Mixed => "mixed",
                ToolBatchGoalStatus::NoProgress => "no_progress",
            }
        ));

        if self.success_count > 0 || self.error_count > 0 {
            lines.push(format!(
                "  success: {}  errors: {}",
                self.success_count, self.error_count
            ));
        }

        if !self.blocked_by.is_empty() {
            let reasons: Vec<&str> = self.blocked_by.iter().map(|b| b.as_str()).collect();
            lines.push(format!("  blocked_by: {}", reasons.join(", ")));
        }

        if !self.artifacts_created.is_empty() {
            lines.push(format!(
                "  artifacts: {}",
                self.artifacts_created.join(", ")
            ));
        }

        if let Some(ref next) = self.recommended_next_step {
            lines.push(format!("  next_step: {}", next));
        }

        if !self.unresolved_items.is_empty() {
            lines.push(format!(
                "  unresolved: {}",
                self.unresolved_items.join("; ")
            ));
        }

        lines.push("</tool-batch-summary>".to_string());
        lines.join("\n")
    }
}

/// Strict / permissive repair policy — governs whether synthetic repairs
/// are injected into the model context.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepairPolicy {
    /// Interactive mode: repairs are injected so the conversation continues.
    #[default]
    Permissive,

    /// Fidelity mode: synthetic repairs are NOT injected; raw errors propagate.
    Strict,
}

impl RepairPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Permissive => "permissive",
            Self::Strict => "strict",
        }
    }
}

// ── Shared Sanitizer Types ──────────────────────────────────────────────

/// The lifecycle phase during which sanitization runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizerStage {
    /// First request in a turn — messages are being assembled for the model.
    PreRequest,

    /// A failed request is being retried with a different provider/model.
    FallbackRetry,

    /// A previously paused or interrupted session is being resumed.
    SessionResume,

    /// Messages surviving compaction are being re-validated.
    PostCompaction,
}

impl SanitizerStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreRequest => "pre_request",
            Self::FallbackRetry => "fallback_retry",
            Self::SessionResume => "session_resume",
            Self::PostCompaction => "post_compaction",
        }
    }
}

impl std::fmt::Display for SanitizerStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A specific sanitization action taken during message cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SanitizerAction {
    /// An orphaned tool_result had no matching assistant tool_use.
    OrphanedToolResult { tool_call_id: String },

    /// A duplicate tool_use ID was detected and resolved.
    DuplicateToolId { tool_call_id: String },

    /// An assistant message containing only thinking/reasoning blocks was dropped.
    ThinkingOnlyAssistant,

    /// A trailing thinking block in an otherwise valid assistant message was removed.
    TrailingInvalidThinkingBlock,

    /// Continuation-related provider options were stripped for a new boundary.
    FallbackContinuationStrip { removed_keys: Vec<String> },

    /// Compacted or trimmed message sequence left residue that needed cleanup.
    CompactionResidue { reason: String },

    /// Malformed assistant message was replaced with a synthetic placeholder.
    AssistantMalformedPlaceholder,

    /// An orphaned continuation signature was detected and auto-healed.
    OrphanedContinuationAutoHeal,
}

// ── Canonical RepairKind (P1.3) ─────────────────────────────────────────

/// Stable, queryable classification of every repair event.
///
/// This replaces ad-hoc string literals like `"tool_name_repair"` across the
/// codebase. Every repair site MUST use one of these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepairKind {
    ToolNameRepair,
    ArgumentNormalization,
    ArgumentPrevalidationFallback,
    InvalidToolReroute,
    ExecutionErrorNoReroute,
    BasenameAutoRepair,
    JsonStringObjectParse,
    SanitizerOrphanedToolResult,
    SanitizerDuplicateToolId,
    SanitizerThinkingOnlyAssistant,
    SanitizerTrailingInvalidThinkingBlock,
    SanitizerFallbackContinuationStrip,
    SanitizerCompactionResidue,
    SanitizerAssistantMalformedPlaceholder,
    SanitizerOrphanedContinuationAutoHeal,
    ProviderFallbackRetry,
    ProviderRequestRejected,
    ThinkingReplayBoundaryReset,
}

impl RepairKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolNameRepair => "tool_name_repair",
            Self::ArgumentNormalization => "argument_normalization",
            Self::ArgumentPrevalidationFallback => "argument_prevalidation_fallback",
            Self::InvalidToolReroute => "invalid_tool_reroute",
            Self::ExecutionErrorNoReroute => "execution_error_no_reroute",
            Self::BasenameAutoRepair => "basename_auto_repair",
            Self::JsonStringObjectParse => "json_string_object_parse",
            Self::SanitizerOrphanedToolResult => "orphaned_tool_result",
            Self::SanitizerDuplicateToolId => "duplicate_tool_id",
            Self::SanitizerThinkingOnlyAssistant => "thinking_only_assistant",
            Self::SanitizerTrailingInvalidThinkingBlock => "trailing_invalid_thinking_block",
            Self::SanitizerFallbackContinuationStrip => "fallback_continuation_strip",
            Self::SanitizerCompactionResidue => "compaction_residue",
            Self::SanitizerAssistantMalformedPlaceholder => "assistant_malformed_placeholder",
            Self::SanitizerOrphanedContinuationAutoHeal => "orphaned_continuation_auto_heal",
            Self::ProviderFallbackRetry => "provider_fallback_retry",
            Self::ProviderRequestRejected => "provider_request_rejected",
            Self::ThinkingReplayBoundaryReset => "thinking_replay_boundary_reset",
        }
    }

    /// Parse a legacy string literal back into a stable `RepairKind`.
    /// Accepts both the canonical snake_case and legacy forms.
    pub fn from_legacy_str(value: &str) -> Option<Self> {
        match value {
            "tool_name_repair" => Some(Self::ToolNameRepair),
            "argument_normalization" => Some(Self::ArgumentNormalization),
            "argument_prevalidation_fallback" => Some(Self::ArgumentPrevalidationFallback),
            "invalid_tool_reroute" => Some(Self::InvalidToolReroute),
            "execution_error_no_reroute" => Some(Self::ExecutionErrorNoReroute),
            "basename_auto_repair" => Some(Self::BasenameAutoRepair),
            "json_string_object_parse" => Some(Self::JsonStringObjectParse),
            "orphaned_tool_result" => Some(Self::SanitizerOrphanedToolResult),
            "duplicate_tool_id" => Some(Self::SanitizerDuplicateToolId),
            "thinking_only_assistant" => Some(Self::SanitizerThinkingOnlyAssistant),
            "trailing_invalid_thinking_block" => Some(Self::SanitizerTrailingInvalidThinkingBlock),
            "fallback_continuation_strip" => Some(Self::SanitizerFallbackContinuationStrip),
            "compaction_residue" => Some(Self::SanitizerCompactionResidue),
            "assistant_malformed_placeholder" => Some(Self::SanitizerAssistantMalformedPlaceholder),
            "orphaned_continuation_auto_heal" => Some(Self::SanitizerOrphanedContinuationAutoHeal),
            "provider_fallback_retry" => Some(Self::ProviderFallbackRetry),
            "provider_request_rejected" => Some(Self::ProviderRequestRejected),
            "thinking_replay_boundary_reset" => Some(Self::ThinkingReplayBoundaryReset),
            // Legacy aliases
            "alias_normalization" | "field_alias_normalization" => {
                Some(Self::ArgumentNormalization)
            }
            "fallback_normalization" => Some(Self::ArgumentPrevalidationFallback),
            _ => None,
        }
    }
}

/// Outcome classification for a tool call that had repairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairOutcomeKind {
    Success,
    ExecutionError,
    InvalidArguments,
    PermissionDenied,
    ProviderRejected,
    Canceled,
    Unknown,
}

// ── Query / Aggregate Types (P1.3) ──────────────────────────────────────

/// Filter parameters for repair queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_kind: Option<RepairKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_samples: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Aggregated row in a repair query result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairAggregateRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub tool_name: String,
    pub repair_kind: RepairKind,
    pub layer: String,
    pub count: u64,
    #[serde(default)]
    pub strict_would_fail_count: u64,
    #[serde(default)]
    pub injected_count: u64,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub error_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_at: Option<i64>,
}

/// A single repair event sampled for detailed inspection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairSample {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub tool_name: String,
    pub repair_kind: RepairKind,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_shape: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_shape: Option<serde_json::Value>,
    #[serde(default)]
    pub strict_mode_would_fail: bool,
    #[serde(default)]
    pub injected_into_model_context: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RepairOutcomeKind>,
    pub created_at: i64,
}

/// Per-session repair summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRepairQuerySummary {
    pub total_events: u64,
    pub distinct_tools: u64,
    pub distinct_repair_kinds: u64,
    #[serde(default)]
    pub strict_would_fail_count: u64,
    #[serde(default)]
    pub injected_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_repairs: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_tools: Vec<ToolRepairCount>,
}

/// Per-model repair summary across sessions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRepairQuerySummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default)]
    pub session_count: u64,
    #[serde(default)]
    pub total_events: u64,
    #[serde(default)]
    pub strict_would_fail_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_repairs: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_tools: Vec<ToolRepairCount>,
}

/// Full session-scoped repair snapshot (persisted to metadata).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRepairQuerySnapshot {
    pub summary: SessionRepairQuerySummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RepairAggregateRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<RepairSample>,
    pub updated_at: i64,
}

/// Unified response for any repair query endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairQueryResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionRepairQuerySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_summary: Option<ModelRepairQuerySummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RepairAggregateRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<RepairSample>,
    #[serde(default)]
    pub truncated: bool,
}

// ── Cross-type mapping helpers (P1.3) ───────────────────────────────────

impl SanitizerAction {
    /// Map this sanitizer action to its stable `RepairKind`.
    pub fn repair_kind(&self) -> RepairKind {
        match self {
            Self::OrphanedToolResult { .. } => RepairKind::SanitizerOrphanedToolResult,
            Self::DuplicateToolId { .. } => RepairKind::SanitizerDuplicateToolId,
            Self::ThinkingOnlyAssistant => RepairKind::SanitizerThinkingOnlyAssistant,
            Self::TrailingInvalidThinkingBlock => RepairKind::SanitizerTrailingInvalidThinkingBlock,
            Self::FallbackContinuationStrip { .. } => {
                RepairKind::SanitizerFallbackContinuationStrip
            }
            Self::CompactionResidue { .. } => RepairKind::SanitizerCompactionResidue,
            Self::AssistantMalformedPlaceholder => {
                RepairKind::SanitizerAssistantMalformedPlaceholder
            }
            Self::OrphanedContinuationAutoHeal => RepairKind::SanitizerOrphanedContinuationAutoHeal,
        }
    }
}

impl RepairEvent {
    /// Attempt to normalize this event's `repair_kind` string into a stable
    /// `RepairKind` enum value.
    pub fn normalized_kind(&self) -> Option<RepairKind> {
        RepairKind::from_legacy_str(&self.repair_kind)
    }
}

#[cfg(test)]
mod repair_kind_tests {
    use super::*;

    #[test]
    fn repair_kind_round_trips_stably() {
        for kind in &[
            RepairKind::ToolNameRepair,
            RepairKind::ArgumentNormalization,
            RepairKind::BasenameAutoRepair,
            RepairKind::InvalidToolReroute,
            RepairKind::SanitizerOrphanedToolResult,
            RepairKind::ProviderFallbackRetry,
        ] {
            let s = kind.as_str();
            let parsed = RepairKind::from_legacy_str(s);
            assert_eq!(parsed, Some(*kind), "round-trip failed for {s}");
        }
    }

    #[test]
    fn repair_event_original_error_kind_survives_loose_map_roundtrip() {
        let mut event = RepairEvent::new("invalid_tool_reroute", "session_prompt", "test_tool");
        event.original_error_kind = Some("invalid_arguments".to_string());
        event.reason = Some("Error: Invalid arguments: missing field".to_string());

        let map = event.to_loose_map();
        let restored = RepairEvent::from_loose_map(&map).expect("should round-trip");

        assert_eq!(restored.original_error_kind.as_deref(), Some("invalid_arguments"));
        assert_eq!(restored.reason.as_deref(), Some("Error: Invalid arguments: missing field"));
        assert_eq!(restored.repair_kind, "invalid_tool_reroute");
    }

    #[test]
    fn repair_event_original_error_kind_is_none_when_not_set() {
        let event = RepairEvent::new("tool_name_repair", "tool", "read");
        let map = event.to_loose_map();
        let restored = RepairEvent::from_loose_map(&map).expect("should round-trip");

        assert_eq!(restored.original_error_kind, None);
    }

    #[test]
    fn sanitizer_action_maps_to_repair_kind() {
        assert_eq!(
            SanitizerAction::OrphanedToolResult {
                tool_call_id: "x".into()
            }
            .repair_kind(),
            RepairKind::SanitizerOrphanedToolResult
        );
        assert_eq!(
            SanitizerAction::ThinkingOnlyAssistant.repair_kind(),
            RepairKind::SanitizerThinkingOnlyAssistant
        );
        assert_eq!(
            SanitizerAction::FallbackContinuationStrip {
                removed_keys: vec!["x".into()]
            }
            .repair_kind(),
            RepairKind::SanitizerFallbackContinuationStrip
        );
    }

    #[test]
    fn legacy_repair_kind_strings_parse_to_enum() {
        assert_eq!(
            RepairKind::from_legacy_str("tool_name_repair"),
            Some(RepairKind::ToolNameRepair)
        );
        assert_eq!(
            RepairKind::from_legacy_str("orphaned_tool_result"),
            Some(RepairKind::SanitizerOrphanedToolResult)
        );
        // Legacy alias
        assert_eq!(
            RepairKind::from_legacy_str("alias_normalization"),
            Some(RepairKind::ArgumentNormalization)
        );
        // Unknown
        assert_eq!(RepairKind::from_legacy_str("nonexistent_kind"), None);
    }

    #[test]
    fn repair_event_normalized_kind_resolves_legacy_strings() {
        let event = RepairEvent::new("alias_normalization", "tool", "skill_manage");
        assert_eq!(
            event.normalized_kind(),
            Some(RepairKind::ArgumentNormalization)
        );
    }

    #[test]
    fn tool_batch_summary_format_includes_goal_status_and_blockers() {
        let summary = ToolBatchSummary {
            tools_used: vec!["read".into(), "edit".into()],
            success_count: 1,
            error_count: 1,
            error_kinds: vec!["invalid_arguments".into()],
            goal_status: ToolBatchGoalStatus::Mixed,
            blocked_by: vec![ToolBatchBlockReason::InvalidArguments],
            artifacts_created: vec!["src/foo.rs".into()],
            pending_follow_up: vec![ToolBatchFollowUpItem {
                kind: "fix_args".into(),
                text: "edit call failed validation".into(),
            }],
            unresolved_items: vec!["edit call failed validation".into()],
            recommended_next_step: Some("fix tool arguments before retrying".into()),
            repair_events: vec![],
        };

        let formatted = summary.format_for_context();
        assert!(formatted.contains("goal_status: mixed"));
        assert!(formatted.contains("blocked_by: invalid_arguments"));
        assert!(formatted.contains("artifacts: src/foo.rs"));
        assert!(formatted.contains("next_step: fix tool arguments before retrying"));
        assert!(formatted.contains("unresolved: edit call failed validation"));
        // Repair events should NOT be expanded in the context.
        assert!(!formatted.contains("repair_kind"));
        assert!(!formatted.contains("\"kind\""));
    }

    #[test]
    fn tool_batch_summary_omits_empty_sections() {
        let summary = ToolBatchSummary {
            tools_used: vec!["read".into()],
            success_count: 1,
            error_count: 0,
            error_kinds: vec![],
            goal_status: ToolBatchGoalStatus::Advanced,
            blocked_by: vec![],
            artifacts_created: vec![],
            pending_follow_up: vec![],
            unresolved_items: vec![],
            recommended_next_step: None,
            repair_events: vec![],
        };

        let formatted = summary.format_for_context();
        // Empty sections should not appear.
        assert!(!formatted.contains("blocked_by:"));
        assert!(!formatted.contains("artifacts:"));
        assert!(!formatted.contains("next_step:"));
        assert!(!formatted.contains("unresolved:"));
    }

    #[test]
    fn tool_batch_summary_progress_helpers_match_status() {
        let advanced = ToolBatchSummary {
            success_count: 3,
            error_count: 0,
            ..ToolBatchSummary::empty()
        };
        assert!(advanced.has_meaningful_progress());
        assert!(!advanced.is_blocked());

        let blocked = ToolBatchSummary {
            success_count: 0,
            error_count: 1,
            blocked_by: vec![ToolBatchBlockReason::InvalidArguments],
            ..ToolBatchSummary::empty()
        };
        assert!(!blocked.has_meaningful_progress());
        assert!(blocked.is_blocked());

        let mixed = ToolBatchSummary {
            success_count: 2,
            error_count: 1,
            ..ToolBatchSummary::empty()
        };
        assert!(!mixed.has_meaningful_progress());
        assert!(!mixed.is_blocked());
    }
}

impl SanitizerAction {
    /// Short, machine-stable kind string for telemetry aggregation.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::OrphanedToolResult { .. } => "orphaned_tool_result",
            Self::DuplicateToolId { .. } => "duplicate_tool_id",
            Self::ThinkingOnlyAssistant => "thinking_only_assistant",
            Self::TrailingInvalidThinkingBlock => "trailing_invalid_thinking_block",
            Self::FallbackContinuationStrip { .. } => "fallback_continuation_strip",
            Self::CompactionResidue { .. } => "compaction_residue",
            Self::AssistantMalformedPlaceholder => "assistant_malformed_placeholder",
            Self::OrphanedContinuationAutoHeal => "orphaned_continuation_auto_heal",
        }
    }

    /// Human-readable description suitable for debug logs.
    pub fn description(&self) -> String {
        match self {
            Self::OrphanedToolResult { tool_call_id } => {
                format!("orphaned tool_result without pending tool_use: {tool_call_id}")
            }
            Self::DuplicateToolId { tool_call_id } => {
                format!("duplicate tool_use id resolved: {tool_call_id}")
            }
            Self::ThinkingOnlyAssistant => {
                "dropped assistant message with only thinking blocks".to_string()
            }
            Self::TrailingInvalidThinkingBlock => {
                "removed trailing invalid thinking block".to_string()
            }
            Self::FallbackContinuationStrip { removed_keys } => {
                format!(
                    "stripped continuation keys for fallback: {}",
                    removed_keys.join(", ")
                )
            }
            Self::CompactionResidue { reason } => {
                format!("cleaned compaction residue: {reason}")
            }
            Self::AssistantMalformedPlaceholder => {
                "replaced malformed assistant message with placeholder".to_string()
            }
            Self::OrphanedContinuationAutoHeal => {
                "auto-healed orphaned continuation signature".to_string()
            }
        }
    }
}
