use std::convert::Infallible;
use std::sync::Arc;

use axum::response::sse::Event;
use rocode_command::agent_presenter::output_block_to_web;
use rocode_command::output_blocks::OutputBlock;
use rocode_command::stage_protocol::{StageEvent, telemetry_event_names};
use rocode_session::prompt::{OutputBlockEvent, OutputBlockHook};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ServerState;

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
    ) -> Self {
        Self::OutputBlock {
            session_id: session_id.into(),
            block: output_block_to_web(block),
            id: id.map(ToOwned::to_owned),
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
    ServerEvent::output_block(event.session_id.clone(), &event.block, event.id.as_deref())
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

pub(crate) fn broadcast_session_updated(
    state: &ServerState,
    session_id: impl Into<String>,
    source: impl Into<String>,
) {
    let session_id = session_id.into();
    let source = source.into();
    broadcast_server_event(
        state,
        &ServerEvent::SessionUpdated {
            session_id: session_id.clone(),
            source: source.clone(),
        },
    );
    let telemetry = state.runtime_telemetry.clone();
    tokio::spawn(async move {
        telemetry.record_session_updated(&session_id, &source).await;
    });
}

pub(crate) fn broadcast_config_updated(state: &ServerState) {
    broadcast_server_event(state, &ServerEvent::ConfigUpdated);
}

#[cfg(test)]
mod tests {
    use super::{
        DiffEntry, QuestionResolutionKind, ServerEvent, ToolCallPhase, broadcast_session_updated,
    };
    use rocode_command::output_blocks::{OutputBlock, StatusBlock};
    use rocode_command::stage_protocol::{StageEvent, telemetry_event_names};
    use crate::ServerState;

    #[test]
    fn server_event_serializes_output_block_wrapper() {
        let event = ServerEvent::output_block(
            "session-1",
            &OutputBlock::Status(StatusBlock::success("ok")),
            Some("block-1"),
        );

        let value = event.to_json_value().expect("event json");
        assert_eq!(value["type"], "output_block");
        assert_eq!(value["sessionID"], "session-1");
        assert_eq!(value["id"], "block-1");
        assert_eq!(value["block"]["kind"], "status");
        assert_eq!(value["block"]["tone"], "success");
        assert_eq!(value["block"]["text"], "ok");
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
    fn broadcast_session_updated_emits_server_event_payload() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(async {
            let state = ServerState::new();
            let mut rx = state.event_bus.subscribe();

            broadcast_session_updated(&state, "session-1", "prompt.final");

            let payload = rx.recv().await.expect("session.updated payload");
            let value: serde_json::Value =
                serde_json::from_str(&payload).expect("valid json payload");
            assert_eq!(value["type"], "session.updated");
            assert_eq!(value["sessionID"], "session-1");
            assert_eq!(value["source"], "prompt.final");
        });
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
    fn legacy_question_replied_deserializes_as_question_resolved() {
        let event: ServerEvent = serde_json::from_value(serde_json::json!({
            "type": "question.replied",
            "sessionID": "session-1",
            "requestID": "question-1",
            "answers": [["Yes"]],
        }))
        .expect("legacy event");

        assert!(matches!(
            event,
            ServerEvent::QuestionResolved { request_id, .. } if request_id == "question-1"
        ));
    }

    #[test]
    fn legacy_permission_replied_deserializes_as_permission_resolved() {
        let event: ServerEvent = serde_json::from_value(serde_json::json!({
            "type": "permission.replied",
            "sessionID": "session-1",
            "requestID": "permission-1",
            "reply": "once",
        }))
        .expect("legacy event");

        assert!(matches!(
            event,
            ServerEvent::PermissionResolved { permission_id, .. }
                if permission_id == "permission-1"
        ));
    }

    #[test]
    fn legacy_session_diff_deserializes_as_diff_updated() {
        let event: ServerEvent = serde_json::from_value(serde_json::json!({
            "type": "session.diff",
            "sessionID": "session-1",
            "diff": [{
                "path": "src/main.rs",
                "additions": 1,
                "deletions": 0,
            }],
        }))
        .expect("legacy event");

        assert!(matches!(
            event,
            ServerEvent::DiffUpdated { session_id, diff }
                if session_id == "session-1" && diff.len() == 1
        ));
    }
}
