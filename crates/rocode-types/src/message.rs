use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FilePartSource {
    #[serde(rename = "file")]
    File { path: String, text: FileSourceText },
    #[serde(rename = "symbol")]
    Symbol {
        path: String,
        name: String,
        kind: i32,
        range: LspRange,
        text: FileSourceText,
    },
    #[serde(rename = "resource")]
    Resource {
        client_name: String,
        uri: String,
        text: FileSourceText,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSourceText {
    pub value: String,
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: i32,
    pub character: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub mime: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FilePartSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<AgentSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSource {
    pub value: String,
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub prompt: String,
    pub description: String,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub attempt: i32,
    pub error: serde_json::Value,
    pub time: RetryTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryTime {
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStartPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFinishPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub cost: f64,
    pub tokens: StepTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i32>,
    pub input: i32,
    pub output: i32,
    pub reasoning: i32,
    pub cache: CacheTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheTokens {
    pub read: i32,
    pub write: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum ToolState {
    #[serde(rename = "pending")]
    Pending {
        input: serde_json::Value,
        raw: String,
    },
    #[serde(rename = "running")]
    Running {
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
        time: RunningTime,
    },
    #[serde(rename = "completed")]
    Completed {
        input: serde_json::Value,
        output: String,
        title: String,
        metadata: HashMap<String, serde_json::Value>,
        time: CompletedTime,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<FilePart>>,
    },
    #[serde(rename = "error")]
    Error {
        input: serde_json::Value,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
        time: ErrorTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningTime {
    pub start: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTime {
    pub start: i64,
    pub end: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacted: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorTime {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub tool: String,
    pub state: ToolState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Usage statistics for a single message.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_miss_tokens: u64,
    /// Latest prompt/context window occupancy for this assistant turn.
    /// This is not cumulative: it is the number that should drive context
    /// pressure meters and auto-compaction decisions.
    #[serde(default)]
    pub context_tokens: u64,
    pub total_cost: f64,
}

impl MessageUsage {
    /// Actual request-context size for this completed assistant turn.
    ///
    /// This is the "what we really sent to the provider for this request"
    /// book, not the workflow cumulative total.
    pub fn request_context_tokens(&self) -> Option<u64> {
        let tokens = self.context_tokens.max(self.input_tokens);
        (tokens > 0).then_some(tokens)
    }

    /// Per-turn live context pressure. For completed turns this falls back to
    /// the request-context size because message usage does not track post-turn
    /// subtree aggregation.
    pub fn live_context_tokens(&self) -> Option<u64> {
        self.request_context_tokens()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub session_id: String,
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
    pub created_at: DateTime<Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    /// The finish reason from the LLM provider (e.g. "stop", "tool-calls").
    /// Set during streaming when FinishStep is received.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePart {
    pub id: String,
    pub part_type: PartType,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PartType {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        synthetic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ignored: Option<bool>,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(default)]
        status: ToolCallStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<ToolState>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<serde_json::Value>>,
    },
    Reasoning {
        text: String,
    },
    File {
        url: String,
        filename: String,
        mime: String,
    },
    StepStart {
        id: String,
        name: String,
    },
    StepFinish {
        id: String,
        output: Option<String>,
    },
    Snapshot {
        content: String,
    },
    Patch {
        old_string: String,
        new_string: String,
        filepath: String,
    },
    Agent {
        name: String,
        status: String,
    },
    Subtask {
        id: String,
        description: String,
        status: String,
    },
    Retry {
        count: u32,
        reason: String,
    },
    Compaction {
        summary: String,
    },
}

impl SessionMessage {
    pub fn user(session_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self::user_inner(session_id, text, HashMap::new())
    }

    /// User message with canonical source metadata.
    pub fn user_with_source(
        session_id: impl Into<String>,
        text: impl Into<String>,
        origin: MessageSourceOrigin,
        surface: MessageSourceSurface,
    ) -> Self {
        let mut metadata = HashMap::new();
        apply_message_source_metadata(&mut metadata, origin, surface);
        let (admission, authority_class) = origin_to_admission_authority(origin);
        apply_message_admission_metadata(&mut metadata, admission, authority_class);
        Self::user_inner(session_id, text, metadata)
    }

    fn user_inner(
        session_id: impl Into<String>,
        text: impl Into<String>,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            id: format!("msg_{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            role: MessageRole::User,
            parts: vec![MessagePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                part_type: PartType::Text {
                    text: text.into(),
                    synthetic: None,
                    ignored: None,
                },
                created_at: Utc::now(),
                message_id: None,
            }],
            created_at: Utc::now(),
            metadata,
            usage: None,
            finish: None,
        }
    }

    pub fn assistant(session_id: impl Into<String>) -> Self {
        Self {
            id: format!("msg_{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            role: MessageRole::Assistant,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        }
    }

    pub fn tool(session_id: impl Into<String>) -> Self {
        Self {
            id: format!("msg_{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            role: MessageRole::Tool,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        }
    }

    pub fn add_text(&mut self, text: impl Into<String>) {
        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Text {
                text: text.into(),
                synthetic: None,
                ignored: None,
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn mark_text_parts_synthetic(&mut self) {
        for part in &mut self.parts {
            if let PartType::Text { synthetic, .. } = &mut part.part_type {
                *synthetic = Some(true);
            }
        }
    }

    pub fn add_file(
        &mut self,
        url: impl Into<String>,
        filename: impl Into<String>,
        mime: impl Into<String>,
    ) {
        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::File {
                url: url.into(),
                filename: filename.into(),
                mime: mime.into(),
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_reasoning(&mut self, text: impl Into<String>) {
        let text = text.into();
        for part in self.parts.iter_mut().rev() {
            if let PartType::Reasoning { text: existing } = &mut part.part_type {
                existing.push_str(&text);
                return;
            }
        }

        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Reasoning { text },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_tool_call(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) {
        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::ToolCall {
                id: id.into(),
                name: name.into(),
                input,
                status: ToolCallStatus::Running,
                raw: None,
                state: None,
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) {
        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::ToolResult {
                tool_call_id: tool_call_id.into(),
                content: content.into(),
                is_error,
                title: None,
                metadata: None,
                attachments: None,
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_agent(&mut self, name: impl Into<String>) {
        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Agent {
                name: name.into(),
                status: "pending".to_string(),
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_subtask(&mut self, id: impl Into<String>, description: impl Into<String>) {
        self.parts.push(MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Subtask {
                id: id.into(),
                description: description.into(),
                status: "pending".to_string(),
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn get_text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Append text to the last text part, or add a new text part if none exists.
    pub fn append_text(&mut self, text: &str) {
        for part in self.parts.iter_mut().rev() {
            if let PartType::Text {
                text: ref mut existing,
                ..
            } = part.part_type
            {
                existing.push_str(text);
                return;
            }
        }
        self.add_text(text);
    }

    /// Replace all text parts with a single text part containing the given content.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.parts
            .retain(|p| !matches!(p.part_type, PartType::Text { .. }));
        self.add_text(text);
    }

    pub fn get_reasoning(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Reasoning { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_reasoning_appends_to_last_reasoning_part() {
        let mut message = SessionMessage::assistant("session-1");
        message.add_reasoning("alpha");
        message.add_reasoning(" beta");

        let reasoning_parts = message
            .parts
            .iter()
            .filter(|part| matches!(part.part_type, PartType::Reasoning { .. }))
            .count();

        assert_eq!(reasoning_parts, 1);
        assert_eq!(message.get_reasoning(), "alpha beta");
    }

    // ── P3-H: Live identity wire format round-trip tests ──────────────

    #[test]
    fn live_message_part_kind_serde_round_trip() {
        let variants = [
            (LiveMessagePartKind::AssistantText, "assistant_text"),
            (
                LiveMessagePartKind::AssistantReasoning,
                "assistant_reasoning",
            ),
            (LiveMessagePartKind::ToolCall, "tool_call"),
            (LiveMessagePartKind::ToolResult, "tool_result"),
            (LiveMessagePartKind::SchedulerStage, "scheduler_stage"),
        ];
        for (variant, wire) in variants {
            let json = serde_json::to_string(&variant).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let back: LiveMessagePartKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn live_part_phase_serde_round_trip() {
        let phases = [
            (LivePartPhase::Start, "start"),
            (LivePartPhase::Append, "append"),
            (LivePartPhase::Snapshot, "snapshot"),
            (LivePartPhase::End, "end"),
        ];
        for (phase, wire) in phases {
            let json = serde_json::to_string(&phase).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let back: LivePartPhase = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, phase);
        }
    }

    #[test]
    fn live_message_part_identity_serde_round_trip() {
        let identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("block-1".to_string()),
        };
        let json = serde_json::to_string(&identity).expect("serialize");
        let back: LiveMessagePartIdentity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, identity);
    }

    #[test]
    fn live_message_part_identity_without_legacy_block_id() {
        let identity = LiveMessagePartIdentity {
            message_id: "msg-2".to_string(),
            part_key: tool_call_part_key("call-1"),
            part_kind: LiveMessagePartKind::ToolCall,
            phase: LivePartPhase::Start,
            legacy_block_id: None,
        };
        let json = serde_json::to_string(&identity).expect("serialize");
        assert!(!json.contains("legacy_block_id"));
        let back: LiveMessagePartIdentity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, identity);
        assert!(back.legacy_block_id.is_none());
    }

    #[test]
    fn live_message_part_identity_wire_field_names() {
        let identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Append,
            legacy_block_id: None,
        };
        let value = serde_json::to_value(&identity).expect("serialize");
        assert_eq!(value["message_id"], "msg-1");
        assert_eq!(value["part_key"], ASSISTANT_REASONING_MAIN_PART_KEY);
        assert_eq!(value["part_kind"], "assistant_reasoning");
        assert_eq!(value["phase"], "append");
        assert!(value.get("legacy_block_id").is_none());
    }
}

// ── P3-A: Live Identity Contract ─────────────────────────────────────────
// Stable identity for every live output crossing the server→frontend boundary.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMessagePartKind {
    AssistantText,
    AssistantReasoning,
    ToolCall,
    ToolResult,
    SchedulerStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LivePartPhase {
    Start,
    Append,
    Snapshot,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlInputKind {
    Followup,
    Steering,
    Interrupt,
    Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlInputPhase {
    Ingress,
    Queued,
    Adopted,
    Consumed,
    Cleared,
}

/// Canonical identity for a live streaming message part.
///
/// `session_id` is intentionally NOT included here — it lives in the outer
/// event envelope (OutputBlockEvent, ServerEvent::OutputBlock) where it has
/// a single owner. This type is the `{message, part}` locator within a
/// session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct LiveMessagePartIdentity {
    /// The assistant/agent message this part belongs to.
    pub message_id: String,
    /// Stable key within the message, e.g. "text/main", "reasoning/main",
    /// "tool_call/{call_id}", "tool_result/{call_id}".
    pub part_key: String,
    pub part_kind: LiveMessagePartKind,
    pub phase: LivePartPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_block_id: Option<String>,
}

pub const ASSISTANT_TEXT_PART_KEY_PREFIX: &str = "text/";
pub const ASSISTANT_REASONING_PART_KEY_PREFIX: &str = "reasoning/";
pub const ASSISTANT_TEXT_MAIN_PART_KEY: &str = "text/main";
pub const ASSISTANT_REASONING_MAIN_PART_KEY: &str = "reasoning/main";
pub const TOOL_CALL_PART_KEY_PREFIX: &str = "tool_call/";
pub const TOOL_RESULT_PART_KEY_PREFIX: &str = "tool_result/";
pub const SCHEDULER_STAGE_PART_KEY_PREFIX: &str = "scheduler/";

pub fn assistant_text_part_key(segment: &str) -> String {
    format!("{ASSISTANT_TEXT_PART_KEY_PREFIX}{segment}")
}

pub fn assistant_reasoning_part_key(segment: &str) -> String {
    format!("{ASSISTANT_REASONING_PART_KEY_PREFIX}{segment}")
}

pub fn tool_call_part_key(tool_call_id: &str) -> String {
    format!("{TOOL_CALL_PART_KEY_PREFIX}{tool_call_id}")
}

pub fn tool_result_part_key(tool_call_id: &str) -> String {
    format!("{TOOL_RESULT_PART_KEY_PREFIX}{tool_call_id}")
}

pub fn scheduler_stage_part_key(stage_id: &str) -> String {
    format!("{SCHEDULER_STAGE_PART_KEY_PREFIX}{stage_id}")
}

pub fn live_slot_key(message_id: &str, part_key: &str) -> String {
    format!("{message_id}:{part_key}")
}

pub fn tool_id_from_part_key(part_key: &str) -> Option<&str> {
    if let Some(candidate) = part_key.strip_prefix(TOOL_CALL_PART_KEY_PREFIX) {
        return (!candidate.trim().is_empty()).then_some(candidate);
    }
    if let Some(candidate) = part_key.strip_prefix(TOOL_RESULT_PART_KEY_PREFIX) {
        return (!candidate.trim().is_empty()).then_some(candidate);
    }
    None
}

// ============================================================================
// Message Source Metadata — canonical contract for message origin tracking.
// All message-writing entry points MUST use these keys and helpers so
// downstream consumers (telemetry, audit, UI) see a single stable schema.
// ============================================================================

/// Metadata key: who originated this message.
pub const MESSAGE_SOURCE_ORIGIN_KEY: &str = "message_source.origin";
/// Metadata key: which surface/transport was used.
pub const MESSAGE_SOURCE_SURFACE_KEY: &str = "message_source.surface";
/// Metadata key: this is a synthetic/system-generated message.
pub const MESSAGE_SOURCE_SYNTHETIC_KEY: &str = "message_source.synthetic";
/// Metadata key: this message was imported (fork/history).
pub const MESSAGE_SOURCE_IMPORTED_KEY: &str = "message_source.imported";

/// Who originated this message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageSourceOrigin {
    /// Human operator (CLI/TUI/Web input).
    Operator,
    /// Scheduler / runtime injected.
    Scheduler,
    /// External adapter / webhook / callback.
    ExternalTrigger,
    /// Child agent handoff.
    ChildHandoff,
    /// Imported from fork / history.
    ImportedHistory,
    /// System-generated (steering, notice, etc.).
    System,
}

/// Which surface/transport the message arrived through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageSourceSurface {
    Cli,
    Tui,
    Web,
    HttpApi,
    UnixSocket,
    Direct,
    ExternalAdapter,
}

/// Apply canonical source metadata to a SessionMessage.
pub fn apply_message_source_metadata(
    metadata: &mut HashMap<String, serde_json::Value>,
    origin: MessageSourceOrigin,
    surface: MessageSourceSurface,
) {
    metadata.insert(
        MESSAGE_SOURCE_ORIGIN_KEY.to_string(),
        serde_json::to_value(origin).unwrap_or_default(),
    );
    metadata.insert(
        MESSAGE_SOURCE_SURFACE_KEY.to_string(),
        serde_json::to_value(surface).unwrap_or_default(),
    );
}

/// Read source origin from metadata (best-effort; returns None if key missing).
pub fn message_source_origin(metadata: &HashMap<String, serde_json::Value>) -> Option<MessageSourceOrigin> {
    metadata
        .get(MESSAGE_SOURCE_ORIGIN_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Read source surface from metadata (best-effort).
pub fn message_source_surface(metadata: &HashMap<String, serde_json::Value>) -> Option<MessageSourceSurface> {
    metadata
        .get(MESSAGE_SOURCE_SURFACE_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

// ── Admission context & authority class ───────────────────────────────

/// Metadata key: how was this message admitted (authenticated, anonymous, etc.).
pub const MESSAGE_ADMISSION_CONTEXT_KEY: &str = "message_source.admission";
/// Metadata key: what authority class does this message carry.
pub const MESSAGE_AUTHORITY_CLASS_KEY: &str = "message_source.authority_class";

/// How the message was admitted into the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageAdmissionContext {
    /// Authenticated user (password, API key, OAuth).
    Authenticated,
    /// Anonymous / unauthenticated.
    Anonymous,
    /// Internal system component (scheduler, runtime).
    Internal,
    /// External integration (webhook, adapter).
    External,
}

/// Authority class carried by this message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageAuthorityClass {
    /// Human user.
    User,
    /// System / runtime.
    System,
    /// Scheduler.
    Scheduler,
    /// External adapter.
    ExternalAdapter,
}

/// Single authority: derive admission context + authority class from an origin.
pub fn origin_to_admission_authority(
    origin: MessageSourceOrigin,
) -> (MessageAdmissionContext, MessageAuthorityClass) {
    match origin {
        MessageSourceOrigin::Operator => (
            MessageAdmissionContext::Authenticated,
            MessageAuthorityClass::User,
        ),
        MessageSourceOrigin::Scheduler => (
            MessageAdmissionContext::Internal,
            MessageAuthorityClass::Scheduler,
        ),
        MessageSourceOrigin::ExternalTrigger => (
            MessageAdmissionContext::External,
            MessageAuthorityClass::ExternalAdapter,
        ),
        MessageSourceOrigin::ChildHandoff => (
            MessageAdmissionContext::Internal,
            MessageAuthorityClass::System,
        ),
        MessageSourceOrigin::ImportedHistory => (
            MessageAdmissionContext::Internal,
            MessageAuthorityClass::System,
        ),
        MessageSourceOrigin::System => (
            MessageAdmissionContext::Internal,
            MessageAuthorityClass::System,
        ),
    }
}

/// Apply admission context and authority class to message metadata.
pub fn apply_message_admission_metadata(
    metadata: &mut HashMap<String, serde_json::Value>,
    admission: MessageAdmissionContext,
    authority_class: MessageAuthorityClass,
) {
    metadata.insert(
        MESSAGE_ADMISSION_CONTEXT_KEY.to_string(),
        serde_json::to_value(admission).unwrap_or_default(),
    );
    metadata.insert(
        MESSAGE_AUTHORITY_CLASS_KEY.to_string(),
        serde_json::to_value(authority_class).unwrap_or_default(),
    );
}

/// Read admission context from metadata.
pub fn message_admission_context(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<MessageAdmissionContext> {
    metadata
        .get(MESSAGE_ADMISSION_CONTEXT_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Read authority class from metadata.
pub fn message_authority_class(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<MessageAuthorityClass> {
    metadata
        .get(MESSAGE_AUTHORITY_CLASS_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}
