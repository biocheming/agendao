use std::sync::atomic::{AtomicU64, Ordering};

use agendao_command_render::block_projection::output_block_to_web;
use agendao_command_render::output_blocks::OutputBlock;
use agendao_types::{ControlInputKind, ControlInputPhase};
use serde::{Deserialize, Serialize};

/// Observable telemetry for the server event bus.
///
/// Constitution §8: every active executor must be observable in its authority
/// registry. These counters let operators answer "are events flowing?" and
/// "how many clients are attached?".
///
/// NOTE: The underlying `tokio::broadcast::Sender` does not expose per-receiver
/// lag or buffer fill. `send_error_count` counts "no active receiver" events,
/// not backpressure. This telemetry should be read alongside per-connection
/// SSE queue metrics for a complete picture of event delivery health.
#[derive(Debug)]
pub struct EventBusTelemetry {
    /// Total events sent to the broadcast channel (successful sends).
    pub send_count: AtomicU64,
    /// Failed sends — `broadcast::Sender::send()` fails when zero receivers are
    /// active, not when receivers are full/lagged.
    pub send_error_count: AtomicU64,
    /// Peak number of concurrent receivers ever observed.
    pub max_receivers: AtomicU64,
    /// Timestamp (ms) of the most recent successful send.
    pub last_send_at_ms: AtomicU64,
    /// Timestamp (ms) of the most recent send error.
    pub last_send_error_at_ms: AtomicU64,
    // ── P3-H: P3-specific observability counters ──────────────────────
    /// LiveSnapshotCoalescer: number of deltas accumulated into snapshots.
    pub coalesced_snapshot_count: AtomicU64,
    /// Output blocks received without live_identity (legacy passthrough).
    pub identity_missing_count: AtomicU64,
    /// P3-I: Coalesced full snapshots emitted to frontends.
    /// If a frontend's visible state is append-only (instead of replace),
    /// this count will diverge from the number of visible entries. This
    /// counter provides the server-side reference point for detecting
    /// visible-state replay.
    pub full_snapshot_emitted_count: AtomicU64,
}

impl Default for EventBusTelemetry {
    fn default() -> Self {
        Self {
            send_count: AtomicU64::new(0),
            send_error_count: AtomicU64::new(0),
            max_receivers: AtomicU64::new(0),
            last_send_at_ms: AtomicU64::new(0),
            last_send_error_at_ms: AtomicU64::new(0),
            coalesced_snapshot_count: AtomicU64::new(0),
            identity_missing_count: AtomicU64::new(0),
            full_snapshot_emitted_count: AtomicU64::new(0),
        }
    }
}

impl EventBusTelemetry {
    pub fn record_send(&self, receiver_count: usize) {
        self.send_count.fetch_add(1, Ordering::Relaxed);
        let now = chrono::Utc::now().timestamp_millis() as u64;
        self.last_send_at_ms.store(now, Ordering::Relaxed);
        self.max_receivers
            .fetch_max(receiver_count as u64, Ordering::Relaxed);
    }

    pub fn record_send_error(&self) {
        self.send_error_count.fetch_add(1, Ordering::Relaxed);
        let now = chrono::Utc::now().timestamp_millis() as u64;
        self.last_send_error_at_ms.store(now, Ordering::Relaxed);
    }

    // ── P3-H: Convenience incrementors ────────────────────────────────

    pub fn record_coalesced_snapshot(&self) {
        self.coalesced_snapshot_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_identity_missing(&self) {
        self.identity_missing_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_full_snapshot_emitted(&self) {
        self.full_snapshot_emitted_count
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot suitable for telemetry export.
    pub fn snapshot(&self) -> agendao_api::EventBusTelemetrySummary {
        agendao_api::EventBusTelemetrySummary {
            send_count: self.send_count.load(Ordering::Relaxed),
            send_error_count: self.send_error_count.load(Ordering::Relaxed),
            max_receivers: self.max_receivers.load(Ordering::Relaxed),
            last_send_at_ms: self.last_send_at_ms.load(Ordering::Relaxed),
            last_send_error_at_ms: self.last_send_error_at_ms.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionResolutionKind {
    Answered,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallPhase {
    Start,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffEntry {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// Canonical event contract for all frontend event consumption.
///
/// This is the single source of truth for server→frontend events.
/// Direct (in-process) and Unix socket paths MUST emit events with
/// the same semantic categories as HTTP `/event` SSE. Frontend-local
/// event types (`StateChange`, `CliServerEvent`) are projections of
/// this contract — they translate ServerEvent into internal dispatch
/// but MUST NOT define independent event semantics.
///
/// See docs/frontend-transport-event-matrix-2026-05-28.md for the
/// full transport × event coverage matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    #[serde(rename = "output_block")]
    OutputBlock {
        #[serde(rename = "sessionID")]
        session_id: String,
        block: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// P3-A: live identity for routing without heuristic guessing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live_identity: Option<agendao_types::LiveMessagePartIdentity>,
    },
    #[serde(rename = "usage")]
    Usage {
        #[serde(rename = "sessionID", skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(rename = "sessionID", skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        done: Option<bool>,
    },
    #[serde(rename = "session.updated")]
    SessionUpdated {
        #[serde(rename = "sessionID")]
        session_id: String,
        source: String,
    },
    #[serde(rename = "session.status")]
    SessionStatus {
        #[serde(rename = "sessionID")]
        session_id: String,
        status: serde_json::Value,
    },
    #[serde(rename = "question.created")]
    QuestionCreated {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "requestID")]
        request_id: String,
        questions: serde_json::Value,
    },
    #[serde(rename = "question.resolved")]
    QuestionResolved {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "requestID")]
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolution: Option<QuestionResolutionKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        answers: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "permission.requested")]
    PermissionRequested {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "permissionID")]
        permission_id: String,
        info: serde_json::Value,
    },
    #[serde(rename = "permission.resolved")]
    PermissionResolved {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "permissionID")]
        permission_id: String,
        reply: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "control_input.transition")]
    ControlInputTransition {
        #[serde(rename = "sessionID")]
        session_id: String,
        kind: ControlInputKind,
        phase: ControlInputPhase,
        at: i64,
    },
    #[serde(rename = "config.updated")]
    ConfigUpdated,
    #[serde(rename = "tool_call.lifecycle")]
    ToolCallLifecycle {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        phase: ToolCallPhase,
        #[serde(rename = "toolName", skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    #[serde(rename = "execution.topology.changed")]
    TopologyChanged {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "executionID", skip_serializing_if = "Option::is_none")]
        execution_id: Option<String>,
        #[serde(rename = "stageID", skip_serializing_if = "Option::is_none")]
        stage_id: Option<String>,
    },
    #[serde(rename = "diff.updated")]
    DiffUpdated {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        diff: Vec<DiffEntry>,
    },
    #[serde(rename = "todo.updated")]
    TodoUpdated {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        todos: Vec<agendao_types::TodoInfo>,
    },
}

impl ServerEvent {
    pub fn output_block(
        session_id: impl Into<String>,
        block: &OutputBlock,
        id: Option<&str>,
        live_identity: Option<agendao_types::LiveMessagePartIdentity>,
    ) -> Self {
        Self::OutputBlock {
            session_id: session_id.into(),
            block: output_block_to_web(block),
            id: id.map(ToOwned::to_owned),
            live_identity,
        }
    }

    /// Extract the session ID associated with this event, if any.
    ///
    /// Session-scoped events carry a `session_id`.
    /// Global events like `ConfigUpdated` return `None`.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::OutputBlock { session_id, .. }
            | Self::Usage {
                session_id: Some(session_id),
                ..
            }
            | Self::Error {
                session_id: Some(session_id),
                ..
            }
            | Self::SessionUpdated { session_id, .. }
            | Self::SessionStatus { session_id, .. }
            | Self::QuestionCreated { session_id, .. }
            | Self::QuestionResolved { session_id, .. }
            | Self::PermissionRequested { session_id, .. }
            | Self::PermissionResolved { session_id, .. }
            | Self::ControlInputTransition { session_id, .. }
            | Self::ToolCallLifecycle { session_id, .. }
            | Self::TopologyChanged { session_id, .. }
            | Self::DiffUpdated { session_id, .. }
            | Self::TodoUpdated { session_id, .. } => Some(session_id),
            Self::Usage {
                session_id: None, ..
            }
            | Self::Error {
                session_id: None, ..
            }
            | Self::ConfigUpdated => None,
        }
    }

    pub fn event_name(&self) -> &'static str {
        match self {
            Self::OutputBlock { .. } => "output_block",
            Self::Usage { .. } => "usage",
            Self::Error { .. } => "error",
            Self::SessionUpdated { .. } => "session.updated",
            Self::SessionStatus { .. } => "session.status",
            Self::QuestionCreated { .. } => "question.created",
            Self::QuestionResolved { .. } => "question.resolved",
            Self::PermissionRequested { .. } => "permission.requested",
            Self::PermissionResolved { .. } => "permission.resolved",
            Self::ControlInputTransition { .. } => "control_input.transition",
            Self::ConfigUpdated => "config.updated",
            Self::ToolCallLifecycle { .. } => "tool_call.lifecycle",
            Self::TopologyChanged { .. } => "execution.topology.changed",
            Self::DiffUpdated { .. } => "diff.updated",
            Self::TodoUpdated { .. } => "todo.updated",
        }
    }

    pub fn to_json_string(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    pub fn to_json_value(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self).ok()
    }
}

/// In-process payload of the canonical ServerEvent bus.
///
/// The event bus is an in-process typed channel: a publisher pushes one typed
/// `ServerEvent` and every subscriber shares it through `Arc`. In-process
/// subscribers see the typed event directly with no JSON round trip.
///
/// The canonical JSON wire text is materialized lazily on first demand and
/// then shared by every network-boundary (SSE) subscriber of the same event,
/// so each event is serialized at most once per process instead of once per
/// subscriber.
#[derive(Debug)]
pub struct ServerBusEvent {
    event: ServerEvent,
    json: std::sync::OnceLock<String>,
}

impl ServerBusEvent {
    pub fn event(event: ServerEvent) -> Self {
        Self {
            event,
            json: std::sync::OnceLock::new(),
        }
    }

    pub fn event_ref(&self) -> &ServerEvent {
        &self.event
    }

    /// Canonical JSON wire text — byte-identical to
    /// `serde_json::to_string` of the typed event, serialized at most once
    /// and shared by all consumers of this envelope.
    pub fn json(&self) -> &str {
        self.json.get_or_init(|| {
            serde_json::to_string(&self.event).unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to serialize ServerEvent for wire broadcast");
                String::new()
            })
        })
    }

    /// Test probe: `true` once the JSON wire text has been materialized.
    /// In-process-only pipelines must leave this `false`.
    pub fn is_json_materialized(&self) -> bool {
        self.json.get().is_some()
    }
}

/// Reconcile reason — categorises every `session.updated` / `SessionReconcile`
/// emit site so we can measure which paths still drive full refreshes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileReason {
    /// Final alignment after a turn completes (prompt.final, prompt.completed).
    TurnFinal,
    /// Session metadata mutation (title, compact, delete, fork).
    MetadataChange,
    /// Permission state changed (pending / resolved).
    Permission,
    /// Steering message enqueued or consumed.
    Steering,
    /// Run status transition (idle → running → completed).
    StatusChange,
    /// Scheduler / stage topology changed.
    Topology,
    /// P3-F: Turn completed but the provider stream did not finish cleanly.
    /// Frontends should refresh from stored messages rather than relying on
    /// the incomplete live stream.
    Backfill,
}

impl ReconcileReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TurnFinal => "turn.final",
            Self::MetadataChange => "metadata.change",
            Self::Permission => "permission",
            Self::Steering => "steering",
            Self::StatusChange => "status.change",
            Self::Topology => "topology",
            Self::Backfill => "backfill",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiffEntry, EventBusTelemetry, QuestionResolutionKind, ServerBusEvent, ServerEvent,
        ToolCallPhase,
    };
    use agendao_command_render::output_blocks::{OutputBlock, StatusBlock};
    use agendao_types::{
        ControlInputKind, ControlInputPhase, LiveMessagePartIdentity, LiveMessagePartKind,
        LivePartPhase, ASSISTANT_TEXT_MAIN_PART_KEY,
    };

    // ── ServerBusEvent: lazy shared JSON wire text ─────────────────────

    fn sample_bus_event() -> ServerBusEvent {
        ServerBusEvent::event(ServerEvent::SessionUpdated {
            session_id: "ses_1".to_string(),
            source: "turn.final".to_string(),
        })
    }

    #[test]
    fn typed_bus_event_starts_unmaterialized() {
        let bus = sample_bus_event();
        assert!(!bus.is_json_materialized());
        assert!(matches!(
            bus.event_ref(),
            ServerEvent::SessionUpdated { .. }
        ));
    }

    #[test]
    fn bus_event_json_matches_direct_serialization_byte_for_byte() {
        let bus = sample_bus_event();
        let expected = serde_json::to_string(bus.event_ref()).expect("direct json");
        assert_eq!(bus.json(), expected);
    }

    #[test]
    fn bus_event_json_is_serialized_once_and_shared() {
        let bus = sample_bus_event();
        let first = bus.json() as *const str;
        let second = bus.json() as *const str;
        assert!(
            bus.is_json_materialized(),
            "json() must materialize the wire text"
        );
        assert_eq!(
            first, second,
            "repeated json() must return the same allocation (serialize once, share)"
        );
    }

    #[test]
    fn server_event_serializes_output_block_wrapper() {
        let event = ServerEvent::output_block(
            "session-1",
            &OutputBlock::Status(StatusBlock::success("ok")),
            Some("block-1"),
            Some(LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::Snapshot,
            }),
        );

        let value = event.to_json_value().expect("event json");
        assert_eq!(value["type"], "output_block");
        assert_eq!(value["sessionID"], "session-1");
        assert_eq!(value["id"], "block-1");
        assert_eq!(value["block"]["kind"], "status");
        assert_eq!(value["block"]["tone"], "success");
        assert_eq!(value["block"]["text"], "ok");
        assert_eq!(value["live_identity"]["message_id"], "msg-1");
        assert_eq!(
            value["live_identity"]["part_key"],
            ASSISTANT_TEXT_MAIN_PART_KEY
        );
        assert_eq!(value["live_identity"]["part_kind"], "assistant_text");
        assert_eq!(value["live_identity"]["phase"], "snapshot");
    }

    #[test]
    fn config_updated_event_serializes_as_tagged_type() {
        let value = ServerEvent::ConfigUpdated
            .to_json_value()
            .expect("event json");
        assert_eq!(value, serde_json::json!({ "type": "config.updated" }));
    }

    #[test]
    fn question_resolved_serializes_with_canonical_type() {
        let value = ServerEvent::QuestionResolved {
            session_id: "session-1".to_string(),
            request_id: "question-1".to_string(),
            resolution: Some(QuestionResolutionKind::Answered),
            answers: Some(serde_json::json!([["Yes"]])),
            reason: None,
        }
        .to_json_value()
        .expect("event json");

        assert_eq!(value["type"], "question.resolved");
        assert_eq!(value["resolution"], "answered");
        assert_eq!(value["requestID"], "question-1");
    }

    #[test]
    fn tool_call_lifecycle_serializes_with_phase() {
        let value = ServerEvent::ToolCallLifecycle {
            session_id: "session-1".to_string(),
            tool_call_id: "tool-1".to_string(),
            phase: ToolCallPhase::Start,
            tool_name: Some("shell".to_string()),
        }
        .to_json_value()
        .expect("event json");

        assert_eq!(value["type"], "tool_call.lifecycle");
        assert_eq!(value["phase"], "start");
        assert_eq!(value["toolName"], "shell");
    }

    #[test]
    fn session_updated_serializes_as_tagged_type() {
        let value = ServerEvent::SessionUpdated {
            session_id: "session-1".to_string(),
            source: "prompt.final".to_string(),
        }
        .to_json_value()
        .expect("event json");

        assert_eq!(value["type"], "session.updated");
        assert_eq!(value["sessionID"], "session-1");
        assert_eq!(value["source"], "prompt.final");
    }

    #[test]
    fn event_bus_telemetry_snapshot_reports_counters() {
        let telemetry = EventBusTelemetry::default();
        telemetry.record_send(3);
        telemetry.record_send_error();

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.send_count, 1);
        assert_eq!(snapshot.send_error_count, 1);
        assert_eq!(snapshot.max_receivers, 3);
        assert!(snapshot.last_send_at_ms > 0);
        assert!(snapshot.last_send_error_at_ms > 0);
    }

    #[test]
    fn diff_updated_serializes_with_canonical_type() {
        let value = ServerEvent::DiffUpdated {
            session_id: "session-1".to_string(),
            diff: vec![DiffEntry {
                path: "src/main.rs".to_string(),
                additions: 12,
                deletions: 3,
            }],
        }
        .to_json_value()
        .expect("event json");

        assert_eq!(value["type"], "diff.updated");
        assert_eq!(value["sessionID"], "session-1");
        assert_eq!(value["diff"][0]["path"], "src/main.rs");
    }

    #[test]
    fn control_input_transition_serializes_with_canonical_type() {
        let value = ServerEvent::ControlInputTransition {
            session_id: "session-1".to_string(),
            kind: ControlInputKind::Steering,
            phase: ControlInputPhase::Queued,
            at: 123,
        }
        .to_json_value()
        .expect("event json");

        assert_eq!(value["type"], "control_input.transition");
        assert_eq!(value["sessionID"], "session-1");
        assert_eq!(value["kind"], "steering");
        assert_eq!(value["phase"], "queued");
        assert_eq!(value["at"], 123);
    }

    #[test]
    fn removed_wire_aliases_are_rejected() {
        for json in [
            serde_json::json!({
                "type": "question.replied", "sessionID": "s-1", "requestID": "q-1",
                "answers": [["Yes"]],
            }),
            serde_json::json!({
                "type": "permission.replied", "sessionID": "s-1", "requestID": "p-1",
                "reply": "once",
            }),
            serde_json::json!({
                "type": "permission.resolved", "sessionID": "s-1", "requestID": "p-1",
                "reply": "once",
            }),
            serde_json::json!({
                "type": "session.diff", "sessionID": "s-1",
                "diff": [{"path": "src/main.rs", "additions": 1, "deletions": 0}],
            }),
        ] {
            assert!(serde_json::from_value::<ServerEvent>(json).is_err());
        }
    }
}

// ── Canonical Runtime Event Surface ──────────────────────────────────────────
//
// Constitution §6 (single plugin contract) and §8 (observability rights):
// every event that crosses the server→frontend boundary MUST belong to one of
// the canonical kinds defined below. No frontend may invent its own event
// semantics; all adapters reference this single authority.
//
// This surface is the foundation for P1-2 (session.updated downgrade) and
// P1-3 (frontend incremental update). Until every canonical kind has a
// concrete delivery path, session.updated remains the reconcile fallback.
//
// ── Canonical Event Kinds ────────────────────────────────────────────────────
//
// Kind                  High-freq  Mergeable  Droppable  Must-deliver  Notes
// ───────────────────── ─────────  ─────────  ─────────  ────────────  ───────
// message_delta         yes        yes        yes        no            Streaming text; final completed msg provides the complete content.
// message_completed     no         no         no         yes           One per assistant/tool message. Carries finish reason, usage.
// tool_call_started     no         no         no         yes           Emitted when tool execution begins.
// tool_call_delta       yes        yes        yes        no            Progress/streaming output from a running tool.
// tool_call_completed   no         no         no         yes           Carries final output, exit code, timing.
// permission_pending    no         no         no         yes           Triggers UI permission prompt.
// permission_resolved   no         no         no         yes           Carries grant/deny decision.
// steering_queued       no         no         no         yes           User injected mid-run steering; UI shows pending preview.
// steering_consumed     no         no         no         yes           Steering was applied at next tool boundary.
// runtime_status_changed no        no         no         yes           Run status transition (idle→running→completed/error).
// session_reconcile     no         no         no         yes           Final alignment event; replaces wholesale session.updated refresh.
//
// Existing ServerEvent variants map to canonical kinds as follows:
//
//   ServerEvent::OutputBlock        → message_delta (text) or tool_call_delta (tool output)
//   ServerEvent::Usage              → (no canonical kind; usage is a side-channel metric)
//   ServerEvent::Error              → runtime_status_changed (when done=true) or message_completed (error finish)
//   ServerEvent::SessionUpdated     → session_reconcile (P1-2: downgraded to fallback)
//   ServerEvent::SessionStatus      → runtime_status_changed
//   ServerEvent::PermissionRequested→ permission_pending
//   ServerEvent::PermissionResolved → permission_resolved
//   ServerEvent::ToolCallLifecycle  → tool_call_started / tool_call_completed
//   ServerEvent::ConfigUpdated      → (no canonical kind; infrastructure event)
//   ServerEvent::TopologyChanged    → (no canonical kind; infrastructure event)
//
// Events without a canonical kind are server-internal or infrastructure
// signals that frontends observe via telemetry snapshots, not via the
// streaming event path.

#[cfg(test)]
/// Authority enum for every event that crosses the server→frontend boundary.
///
/// This is the single source of truth that P1-2 and P1-3 build on.
/// Frontends subscribe to these kinds; server-side emitters map concrete
/// `ServerEvent` payloads into the appropriate canonical kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalEventKind {
    /// High-frequency streaming text from an assistant message.
    /// Mergeable: consecutive deltas for the same message can be coalesced.
    /// Droppable: if backpressure requires it; the completed message provides
    ///   the authoritative final text.
    MessageDelta,
    /// A message (assistant, tool, or user) has been finalized in the transcript.
    /// Carries finish reason, usage, and the complete message content.
    /// Must-deliver: frontends MUST receive this to stay in sync.
    MessageCompleted,
    /// A tool call has started executing.
    ToolCallStarted,
    /// High-frequency streaming output from a running tool (e.g. terminal output,
    /// long-running process stdout).
    /// Mergeable: consecutive deltas for the same tool call can be coalesced.
    /// Droppable: the completed event carries the final output.
    ToolCallDelta,
    /// A tool call has completed with final output, exit code, and timing.
    ToolCallCompleted,
    /// A permission request is pending user action.
    PermissionPending,
    /// A permission request has been resolved (granted or denied).
    PermissionResolved,
    /// A mid-run steering message has been queued for the next tool boundary.
    SteeringQueued,
    /// A steering message has been consumed (injected at a tool boundary).
    SteeringConsumed,
    /// The session run status has changed (idle, running, completed, error).
    RuntimeStatusChanged,
    /// Final alignment event. Replaces wholesale `session.updated` refresh.
    /// Frontends use this to reconcile local state after incremental updates.
    SessionReconcile,
}

#[cfg(test)]
impl CanonicalEventKind {
    /// Whether this event kind produces high-frequency traffic.
    /// High-frequency events are candidates for merging and dropping under backpressure.
    pub fn is_high_frequency(self) -> bool {
        matches!(self, Self::MessageDelta | Self::ToolCallDelta)
    }

    /// Whether consecutive events of this kind for the same entity
    /// (same message, same tool call) can be coalesced into a single event.
    pub fn is_mergeable(self) -> bool {
        matches!(self, Self::MessageDelta | Self::ToolCallDelta)
    }

    /// Whether this event can be dropped under extreme backpressure
    /// without breaking the frontend's ability to reach a consistent state.
    /// Droppable events must have a corresponding must-deliver event
    /// that carries the authoritative final state.
    pub fn is_droppable(self) -> bool {
        matches!(self, Self::MessageDelta | Self::ToolCallDelta)
    }

    /// Whether this event MUST reach every active frontend.
    /// If false, the event can be skipped for certain subscription tiers
    /// (e.g. final-only mode, CLI summary mode).
    pub fn is_must_deliver(self) -> bool {
        !self.is_droppable()
    }
}

#[cfg(test)]
/// Registry of all canonical event kinds with their attributes.
///
/// This is the authority read by P1-2 subscription negotiation and P1-3
/// frontend incremental update logic.
pub struct CanonicalEventRegistry;

#[cfg(test)]
impl CanonicalEventRegistry {
    /// Every canonical event kind, in order of definition.
    pub fn all() -> &'static [CanonicalEventKind] {
        &[
            CanonicalEventKind::MessageDelta,
            CanonicalEventKind::MessageCompleted,
            CanonicalEventKind::ToolCallStarted,
            CanonicalEventKind::ToolCallDelta,
            CanonicalEventKind::ToolCallCompleted,
            CanonicalEventKind::PermissionPending,
            CanonicalEventKind::PermissionResolved,
            CanonicalEventKind::SteeringQueued,
            CanonicalEventKind::SteeringConsumed,
            CanonicalEventKind::RuntimeStatusChanged,
            CanonicalEventKind::SessionReconcile,
        ]
    }

    /// Kinds for CLI low-frequency / summary mode.
    ///
    /// This is the set of all non-droppable events — every event whose delivery
    /// is required for the frontend to maintain a consistent state, minus
    /// streaming deltas. Derived from the attribute table: `!k.is_droppable()`.
    /// This is NOT a hand-picked subset; it is mechanically derived from the
    /// canonical attributes so the "must deliver" contract cannot drift.
    pub fn cli_low_frequency() -> Vec<CanonicalEventKind> {
        Self::all()
            .iter()
            .filter(|k| !k.is_droppable())
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod canonical_event_tests {
    use super::*;

    #[test]
    fn all_kinds_have_consistent_attribute_rules() {
        for kind in CanonicalEventRegistry::all() {
            // mergeable implies high-frequency (you don't merge rare events).
            if kind.is_mergeable() {
                assert!(
                    kind.is_high_frequency(),
                    "{kind:?}: mergeable events must be high-frequency"
                );
            }
            // droppable implies mergeable (you can only drop if you can merge first).
            if kind.is_droppable() {
                assert!(
                    kind.is_mergeable(),
                    "{kind:?}: droppable events must be mergeable"
                );
            }
            // must-deliver is the inverse of droppable.
            assert_eq!(
                kind.is_must_deliver(),
                !kind.is_droppable(),
                "{kind:?}: must_deliver must be !droppable"
            );
        }
    }

    #[test]
    fn canonical_kind_droppable_contract_is_consistent() {
        // Table-driven: each (kind, expected_droppable, expected_must_deliver).
        let cases = &[
            (CanonicalEventKind::MessageDelta, true, false),
            (CanonicalEventKind::MessageCompleted, false, true),
            (CanonicalEventKind::ToolCallDelta, true, false),
            (CanonicalEventKind::ToolCallStarted, false, true),
            (CanonicalEventKind::ToolCallCompleted, false, true),
            (CanonicalEventKind::PermissionPending, false, true),
            (CanonicalEventKind::PermissionResolved, false, true),
            (CanonicalEventKind::SteeringQueued, false, true),
            (CanonicalEventKind::SteeringConsumed, false, true),
            (CanonicalEventKind::SessionReconcile, false, true),
            (CanonicalEventKind::RuntimeStatusChanged, false, true),
        ];
        for (kind, expect_droppable, expect_must_deliver) in cases {
            assert_eq!(
                kind.is_droppable(),
                *expect_droppable,
                "{kind:?}.is_droppable()"
            );
            assert_eq!(
                kind.is_must_deliver(),
                *expect_must_deliver,
                "{kind:?}.is_must_deliver()"
            );
            assert_eq!(
                kind.is_must_deliver(),
                !kind.is_droppable(),
                "{kind:?}: must_deliver != !droppable"
            );
        }
    }

    #[test]
    fn cli_low_frequency_is_mechanically_derived_from_non_droppable() {
        let kinds = CanonicalEventRegistry::cli_low_frequency();
        for &kind in CanonicalEventRegistry::all() {
            if !kind.is_droppable() {
                assert!(
                    kinds.contains(&kind),
                    "{kind:?} is non-droppable but missing from cli_low_frequency"
                );
            }
        }
        for &kind in &kinds {
            assert!(
                !kind.is_droppable(),
                "{kind:?} in cli_low_frequency must be non-droppable"
            );
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    // ── ReconcileReason wire contract ────────────────────────────────────
    // These strings are the wire protocol between server and all three
    // frontends. Changing any of them breaks CLI/TUI/Web source-string
    // matching. The CLI-side counterpart is cli_session_update_requires_refresh
    // in session_projection.rs.

    #[test]
    fn reconcile_reason_wire_strings_are_stable() {
        assert_eq!(ReconcileReason::TurnFinal.as_str(), "turn.final");
        assert_eq!(ReconcileReason::MetadataChange.as_str(), "metadata.change");
        assert_eq!(ReconcileReason::Permission.as_str(), "permission");
        assert_eq!(ReconcileReason::Steering.as_str(), "steering");
        assert_eq!(ReconcileReason::StatusChange.as_str(), "status.change");
        assert_eq!(ReconcileReason::Topology.as_str(), "topology");
        assert_eq!(ReconcileReason::Backfill.as_str(), "backfill");
    }
}
