use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::response::sse::Event;
use rocode_command::agent_presenter::output_block_to_web;
use rocode_command::output_blocks::OutputBlock;
use rocode_command::stage_protocol::{telemetry_event_names, StageEvent};
use rocode_session::prompt::{OutputBlockEvent, OutputBlockHook};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ServerState;
use rocode_types::{ControlInputKind, ControlInputPhase};

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
        self.coalesced_snapshot_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_identity_missing(&self) {
        self.identity_missing_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot suitable for telemetry export.
    pub fn snapshot(&self) -> rocode_api::EventBusTelemetrySummary {
        rocode_api::EventBusTelemetrySummary {
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
        live_identity: Option<rocode_types::LiveMessagePartIdentity>,
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
    #[serde(
        rename = "question.resolved",
        alias = "question.replied",
        alias = "question.rejected"
    )]
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
    #[serde(rename = "permission.resolved", alias = "permission.replied")]
    PermissionResolved {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "permissionID", alias = "requestID")]
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
    #[serde(rename = "attached_session.attached")]
    AttachedSessionAttached {
        #[serde(rename = "parentID")]
        parent_id: String,
        #[serde(rename = "attachedID")]
        attached_id: String,
    },
    #[serde(rename = "attached_session.detached")]
    AttachedSessionDetached {
        #[serde(rename = "parentID")]
        parent_id: String,
        #[serde(rename = "attachedID")]
        attached_id: String,
    },
    #[serde(rename = "diff.updated", alias = "session.diff")]
    DiffUpdated {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        diff: Vec<DiffEntry>,
    },
}

impl ServerEvent {
    pub(crate) fn output_block(
        session_id: impl Into<String>,
        block: &OutputBlock,
        id: Option<&str>,
        live_identity: Option<rocode_types::LiveMessagePartIdentity>,
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
    /// Session-scoped events carry a `session_id` or equivalent (`parent_id`).
    /// Global events like `ConfigUpdated` return `None`.
    pub(crate) fn session_id(&self) -> Option<&str> {
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
            | Self::DiffUpdated { session_id, .. } => Some(session_id),
            Self::AttachedSessionAttached { parent_id, .. }
            | Self::AttachedSessionDetached { parent_id, .. } => Some(parent_id),
            Self::Usage {
                session_id: None, ..
            }
            | Self::Error {
                session_id: None, ..
            }
            | Self::ConfigUpdated => None,
        }
    }

    pub(crate) fn event_name(&self) -> &'static str {
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
            Self::AttachedSessionAttached { .. } => "attached_session.attached",
            Self::AttachedSessionDetached { .. } => "attached_session.detached",
            Self::DiffUpdated { .. } => "diff.updated",
        }
    }

    pub(crate) fn to_json_string(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    pub(crate) fn to_json_value(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self).ok()
    }

    pub(crate) fn to_sse_event(&self) -> Option<Event> {
        Event::default()
            .event(self.event_name())
            .json_data(self)
            .ok()
    }

    pub(crate) fn from_stage_event(event: &StageEvent) -> Option<Self> {
        match event.event_type.as_str() {
            telemetry_event_names::SESSION_UPDATED => Some(Self::SessionUpdated {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                source: event.payload.get("source")?.as_str()?.to_string(),
            }),
            telemetry_event_names::SESSION_STATUS => Some(Self::SessionStatus {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                status: event.payload.get("status")?.clone(),
            }),
            telemetry_event_names::SESSION_USAGE => Some(Self::Usage {
                session_id: event
                    .payload
                    .get("sessionID")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                prompt_tokens: event.payload.get("prompt_tokens")?.as_u64()?,
                completion_tokens: event.payload.get("completion_tokens")?.as_u64()?,
                message_id: event
                    .payload
                    .get("message_id")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            }),
            telemetry_event_names::SESSION_ERROR => Some(Self::Error {
                session_id: event
                    .payload
                    .get("sessionID")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                error: event.payload.get("error")?.as_str()?.to_string(),
                message_id: event
                    .payload
                    .get("message_id")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                done: event.payload.get("done").and_then(|value| value.as_bool()),
            }),
            telemetry_event_names::QUESTION_CREATED => Some(Self::QuestionCreated {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                request_id: event.payload.get("requestID")?.as_str()?.to_string(),
                questions: event.payload.get("questions")?.clone(),
            }),
            telemetry_event_names::QUESTION_RESOLVED => Some(Self::QuestionResolved {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                request_id: event.payload.get("requestID")?.as_str()?.to_string(),
                resolution: event
                    .payload
                    .get("resolution")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok()),
                answers: event.payload.get("answers").cloned(),
                reason: event
                    .payload
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            }),
            telemetry_event_names::PERMISSION_REQUESTED => Some(Self::PermissionRequested {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                permission_id: event.payload.get("permissionID")?.as_str()?.to_string(),
                info: event.payload.get("info")?.clone(),
            }),
            telemetry_event_names::PERMISSION_RESOLVED => Some(Self::PermissionResolved {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                permission_id: event.payload.get("permissionID")?.as_str()?.to_string(),
                reply: event.payload.get("reply")?.as_str()?.to_string(),
                message: event
                    .payload
                    .get("message")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            }),
            telemetry_event_names::TOOL_STARTED => Some(Self::ToolCallLifecycle {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                tool_call_id: event.payload.get("toolCallId")?.as_str()?.to_string(),
                phase: ToolCallPhase::Start,
                tool_name: event
                    .payload
                    .get("toolName")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            }),
            telemetry_event_names::TOOL_COMPLETED => Some(Self::ToolCallLifecycle {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                tool_call_id: event.payload.get("toolCallId")?.as_str()?.to_string(),
                phase: ToolCallPhase::Complete,
                tool_name: event
                    .payload
                    .get("toolName")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            }),
            telemetry_event_names::EXECUTION_TOPOLOGY_CHANGED => Some(Self::TopologyChanged {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                execution_id: event
                    .payload
                    .get("executionID")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                stage_id: event
                    .payload
                    .get("stageID")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            }),
            telemetry_event_names::DIFF_UPDATED => Some(Self::DiffUpdated {
                session_id: event.payload.get("sessionID")?.as_str()?.to_string(),
                diff: serde_json::from_value(event.payload.get("diff")?.clone()).ok()?,
            }),
            telemetry_event_names::ATTACHED_SESSION_ATTACHED => {
                Some(Self::AttachedSessionAttached {
                    parent_id: event.payload.get("parentID")?.as_str()?.to_string(),
                    attached_id: event.payload.get("attachedID")?.as_str()?.to_string(),
                })
            }
            telemetry_event_names::ATTACHED_SESSION_DETACHED => {
                Some(Self::AttachedSessionDetached {
                    parent_id: event.payload.get("parentID")?.as_str()?.to_string(),
                    attached_id: event.payload.get("attachedID")?.as_str()?.to_string(),
                })
            }
            _ => None,
        }
    }
}

pub(crate) fn server_output_block_event(event: &OutputBlockEvent) -> ServerEvent {
    ServerEvent::output_block(event.session_id.clone(), &event.block, event.id.as_deref(), event.live_identity.clone())
}

pub(crate) async fn send_sse_server_event(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    event: &ServerEvent,
) {
    if let Some(sse_event) = event.to_sse_event() {
        if let Err(error) = tx.send(Ok(sse_event)).await {
            tracing::debug!(
                error = %error,
                "Failed to send SSE server event to runtime subscriber"
            );
        }
    }
}

pub(crate) fn broadcast_server_event(state: &ServerState, event: &ServerEvent) {
    if let Some(payload) = event.to_json_string() {
        state.broadcast(&payload);
    }
}

pub(crate) fn broadcast_output_block_event(state: &ServerState, event: &OutputBlockEvent) {
    let server_event = server_output_block_event(event);
    broadcast_server_event(state, &server_event);
}

pub(crate) fn server_output_block_hook(state: Arc<ServerState>) -> OutputBlockHook {
    Arc::new(move |event| {
        let state = state.clone();
        Box::pin(async move {
            broadcast_output_block_event(state.as_ref(), &event);
        })
    })
}

pub(crate) async fn emit_output_block_via_hook(
    output_hook: Option<&OutputBlockHook>,
    event: OutputBlockEvent,
) {
    let Some(output_hook) = output_hook else {
        return;
    };
    output_hook(event).await;
}

pub(crate) fn sse_output_block_hook(
    tx: mpsc::Sender<std::result::Result<Event, Infallible>>,
) -> OutputBlockHook {
    Arc::new(move |event| {
        let tx = tx.clone();
        Box::pin(async move {
            let server_event = server_output_block_event(&event);
            send_sse_server_event(&tx, &server_event).await;
        })
    })
}

/// Reconcile reason — categorises every `session.updated` / `SessionReconcile`
/// emit site so we can measure which paths still drive full refreshes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconcileReason {
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
    fn as_str(self) -> &'static str {
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

/// P1-2: canonical session reconcile event.
///
/// This is the downgraded successor to `session.updated`. It carries a
/// `ReconcileReason` so frontends can decide whether to do a full state
/// refresh (metadata change) or just reconcile incremental deltas (turn final).
pub(crate) fn broadcast_session_reconcile(
    state: &ServerState,
    session_id: impl Into<String>,
    reason: ReconcileReason,
) {
    let session_id = session_id.into();
    let source = reason.as_str();
    broadcast_server_event(
        state,
        &ServerEvent::SessionUpdated {
            session_id: session_id.clone(),
            source: source.to_string(),
        },
    );
    let telemetry = state.runtime_telemetry.clone();
    tokio::spawn(async move {
        telemetry
            .record_session_updated(&session_id, &source)
            .await;
    });
}

pub(crate) fn broadcast_config_updated(state: &ServerState) {
    broadcast_server_event(state, &ServerEvent::ConfigUpdated);
}

#[cfg(test)]
mod tests {
    use super::{
        broadcast_session_reconcile, DiffEntry, EventBusTelemetry, QuestionResolutionKind,
        ReconcileReason, ServerEvent, ToolCallPhase,
    };
    use crate::ServerState;
    use rocode_command::output_blocks::{OutputBlock, StatusBlock};
    use rocode_command::stage_protocol::{telemetry_event_names, StageEvent};
    use rocode_types::{
        ControlInputKind, ControlInputPhase, LiveMessagePartIdentity, LiveMessagePartKind,
        LivePartPhase,
    };

    #[test]
    fn server_event_serializes_output_block_wrapper() {
        let event = ServerEvent::output_block(
            "session-1",
            &OutputBlock::Status(StatusBlock::success("ok")),
            Some("block-1"),
            Some(LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "text/main".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::Snapshot,
                legacy_block_id: Some("block-1".to_string()),
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
        assert_eq!(value["live_identity"]["part_key"], "text/main");
        assert_eq!(value["live_identity"]["part_kind"], "assistant_text");
        assert_eq!(value["live_identity"]["phase"], "snapshot");
        assert_eq!(value["live_identity"]["legacy_block_id"], "block-1");
    }

    #[test]
    fn config_updated_event_serializes_as_tagged_type() {
        let value = ServerEvent::ConfigUpdated
            .to_json_value()
            .expect("event json");
        assert_eq!(value, serde_json::json!({ "type": "config.updated" }));
    }

    #[test]
    fn attached_session_attached_serializes_with_parent_and_attached_ids() {
        let value = ServerEvent::AttachedSessionAttached {
            parent_id: "parent-1".to_string(),
            attached_id: "child-1".to_string(),
        }
        .to_json_value()
        .expect("event json");
        assert_eq!(value["type"], "attached_session.attached");
        assert_eq!(value["parentID"], "parent-1");
        assert_eq!(value["attachedID"], "child-1");
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
    fn stage_event_maps_tool_started_to_transport_event() {
        let event = StageEvent {
            event_id: "evt_1".to_string(),
            scope: rocode_command::stage_protocol::EventScope::Stage,
            stage_id: Some("stage_1".to_string()),
            execution_id: Some("tool_call:tool-1".to_string()),
            event_type: telemetry_event_names::TOOL_STARTED.to_string(),
            ts: 1,
            payload: serde_json::json!({
                "sessionID": "session-1",
                "toolCallId": "tool-1",
                "toolName": "shell",
            }),
        };

        let mapped = ServerEvent::from_stage_event(&event).expect("mapped event");
        let value = mapped.to_json_value().expect("event json");
        assert_eq!(value["type"], "tool_call.lifecycle");
        assert_eq!(value["phase"], "start");
        assert_eq!(value["toolName"], "shell");
    }

    #[test]
    fn stage_event_maps_session_status_to_transport_event() {
        let event = StageEvent {
            event_id: "evt_1".to_string(),
            scope: rocode_command::stage_protocol::EventScope::Session,
            stage_id: None,
            execution_id: None,
            event_type: telemetry_event_names::SESSION_STATUS.to_string(),
            ts: 1,
            payload: serde_json::json!({
                "sessionID": "session-1",
                "status": { "type": "retry", "attempt": 2, "message": "wait", "next": 123 }
            }),
        };

        let mapped = ServerEvent::from_stage_event(&event).expect("mapped event");
        let value = mapped.to_json_value().expect("event json");
        assert_eq!(value["type"], "session.status");
        assert_eq!(value["status"]["type"], "retry");
        assert_eq!(value["status"]["attempt"], 2);
    }

    #[test]
    fn stage_event_maps_session_updated_to_transport_event() {
        let event = StageEvent {
            event_id: "evt_1".to_string(),
            scope: rocode_command::stage_protocol::EventScope::Session,
            stage_id: None,
            execution_id: None,
            event_type: telemetry_event_names::SESSION_UPDATED.to_string(),
            ts: 1,
            payload: serde_json::json!({
                "sessionID": "session-1",
                "source": "prompt.completed",
            }),
        };

        let mapped = ServerEvent::from_stage_event(&event).expect("mapped event");
        let value = mapped.to_json_value().expect("event json");
        assert_eq!(value["type"], "session.updated");
        assert_eq!(value["source"], "prompt.completed");
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
    fn broadcast_session_reconcile_emits_server_event_payload() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(async {
            let state = ServerState::new();
            let mut rx = state.event_bus.subscribe();

            broadcast_session_reconcile(&state, "session-1", ReconcileReason::TurnFinal);

            let payload = rx.recv().await.expect("session.updated payload");
            let value: serde_json::Value =
                serde_json::from_str(&payload).expect("valid json payload");
            assert_eq!(value["type"], "session.updated");
            assert_eq!(value["sessionID"], "session-1");
            assert_eq!(value["source"], "turn.final");
        });
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
    fn stage_event_maps_session_usage_to_transport_event() {
        let event = StageEvent {
            event_id: "evt_1".to_string(),
            scope: rocode_command::stage_protocol::EventScope::Session,
            stage_id: None,
            execution_id: None,
            event_type: telemetry_event_names::SESSION_USAGE.to_string(),
            ts: 1,
            payload: serde_json::json!({
                "sessionID": "session-1",
                "message_id": "msg-1",
                "prompt_tokens": 12,
                "completion_tokens": 34,
                "reasoning_tokens": 5,
            }),
        };

        let mapped = ServerEvent::from_stage_event(&event).expect("mapped event");
        let value = mapped.to_json_value().expect("event json");
        assert_eq!(value["type"], "usage");
        assert_eq!(value["sessionID"], "session-1");
        assert_eq!(value["prompt_tokens"], 12);
        assert_eq!(value["completion_tokens"], 34);
    }

    #[test]
    fn stage_event_maps_session_error_to_transport_event() {
        let event = StageEvent {
            event_id: "evt_1".to_string(),
            scope: rocode_command::stage_protocol::EventScope::Session,
            stage_id: None,
            execution_id: None,
            event_type: telemetry_event_names::SESSION_ERROR.to_string(),
            ts: 1,
            payload: serde_json::json!({
                "sessionID": "session-1",
                "message_id": "msg-1",
                "done": true,
                "error": "boom",
            }),
        };

        let mapped = ServerEvent::from_stage_event(&event).expect("mapped event");
        let value = mapped.to_json_value().expect("event json");
        assert_eq!(value["type"], "error");
        assert_eq!(value["message_id"], "msg-1");
        assert_eq!(value["done"], true);
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
    fn legacy_wire_aliases_deserialize_to_canonical_variants() {
        let cases: &[(&str, serde_json::Value)] = &[
            ("question.replied", serde_json::json!({
                "type": "question.replied", "sessionID": "s-1", "requestID": "q-1",
                "answers": [["Yes"]],
            })),
            ("permission.replied", serde_json::json!({
                "type": "permission.replied", "sessionID": "s-1", "requestID": "p-1",
                "reply": "once",
            })),
            ("session.diff", serde_json::json!({
                "type": "session.diff", "sessionID": "s-1",
                "diff": [{"path": "src/main.rs", "additions": 1, "deletions": 0}],
            })),
        ];
        for (alias, json) in cases {
            let event: ServerEvent =
                serde_json::from_value(json.clone()).expect(&format!("legacy event {alias}"));
            match alias.as_ref() {
                "question.replied" => assert!(matches!(
                    event, ServerEvent::QuestionResolved { request_id, .. }
                        if request_id == "q-1"
                )),
                "permission.replied" => assert!(matches!(
                    event, ServerEvent::PermissionResolved { permission_id, .. }
                        if permission_id == "p-1"
                )),
                "session.diff" => assert!(matches!(
                    event, ServerEvent::DiffUpdated { session_id, diff }
                        if session_id == "s-1" && diff.len() == 1
                )),
                _ => panic!("unexpected alias {alias}"),
            }
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
            assert_eq!(kind.is_must_deliver(), !kind.is_droppable(),
                "{kind:?}: must_deliver != !droppable");
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
            assert!(!kind.is_droppable(), "{kind:?} in cli_low_frequency must be non-droppable");
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use rocode_command::stage_protocol::{telemetry_event_names, EventScope, StageEvent};

    fn stage_event(event_type: &str, payload: serde_json::Value) -> StageEvent {
        StageEvent {
            event_id: format!("evt_{}", uuid::Uuid::new_v4().simple()),
            scope: EventScope::Session,
            stage_id: None,
            execution_id: None,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now().timestamp_millis(),
            payload,
        }
    }

    // ── Permission lifecycle: pending → resolved ───────────────────────────

    #[test]
    fn permission_pending_maps_to_correct_server_event() {
        let event = stage_event(
            telemetry_event_names::PERMISSION_REQUESTED,
            serde_json::json!({
                "sessionID": "sess-1",
                "permissionID": "perm-1",
                "info": { "tool": "bash", "pattern": "rm -rf" },
            }),
        );
        let transport = ServerEvent::from_stage_event(&event)
            .expect("permission.requested should map to a ServerEvent");
        assert!(matches!(
            transport,
            ServerEvent::PermissionRequested { ref session_id, ref permission_id, .. }
                if session_id == "sess-1" && permission_id == "perm-1"
        ));
    }

    #[test]
    fn permission_resolved_maps_to_correct_server_event() {
        let event = stage_event(
            telemetry_event_names::PERMISSION_RESOLVED,
            serde_json::json!({
                "sessionID": "sess-1",
                "permissionID": "perm-1",
                "reply": "once",
            }),
        );
        let transport = ServerEvent::from_stage_event(&event)
            .expect("permission.resolved should map to a ServerEvent");
        assert!(matches!(
            transport,
            ServerEvent::PermissionResolved { ref session_id, ref permission_id, ref reply, .. }
                if session_id == "sess-1" && permission_id == "perm-1" && reply == "once"
        ));
    }

    // ── Tool call lifecycle: started → completed ───────────────────────────

    #[test]
    fn tool_call_started_maps_to_correct_server_event() {
        let event = stage_event(
            telemetry_event_names::TOOL_STARTED,
            serde_json::json!({
                "sessionID": "sess-1",
                "toolCallId": "call-1",
                "toolName": "bash",
            }),
        );
        // ToolCallStarted goes through the ToolCallLifecycle mapping.
        let transport = ServerEvent::from_stage_event(&event)
            .expect("tool_call.started should map to a ServerEvent");
        assert!(matches!(
            transport,
            ServerEvent::ToolCallLifecycle { ref session_id, ref tool_call_id, phase: ToolCallPhase::Start, .. }
                if session_id == "sess-1" && tool_call_id == "call-1"
        ));
    }

    #[test]
    fn tool_call_completed_maps_to_correct_server_event() {
        let event = stage_event(
            telemetry_event_names::TOOL_COMPLETED,
            serde_json::json!({
                "sessionID": "sess-1",
                "toolCallId": "call-1",
                "toolName": "bash",
            }),
        );
        let transport = ServerEvent::from_stage_event(&event)
            .expect("tool_call.completed should map to a ServerEvent");
        assert!(matches!(
            transport,
            ServerEvent::ToolCallLifecycle { phase: ToolCallPhase::Complete, .. }
        ));
    }

    // ── Reconcile / session.updated lifecycle ──────────────────────────────

    #[test]
    fn session_updated_maps_to_correct_server_event() {
        let event = stage_event(
            telemetry_event_names::SESSION_UPDATED,
            serde_json::json!({
                "sessionID": "sess-1",
                "source": "turn.final",
            }),
        );
        let transport = ServerEvent::from_stage_event(&event)
            .expect("session.updated should map to a ServerEvent");
        assert!(matches!(
            transport,
            ServerEvent::SessionUpdated { ref session_id, ref source }
                if session_id == "sess-1" && source == "turn.final"
        ));
    }

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
