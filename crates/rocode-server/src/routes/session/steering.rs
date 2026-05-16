//! Steering API: submit a mid-run steering message to the owner session.
//! Constitution §9: TUI/CLI/Web submit; runtime consumes at tool boundary.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::session_runtime::steering::PendingSteeringMessage;
use crate::{ApiError, Result, ServerState};

#[derive(Debug, Deserialize)]
pub struct SubmitSteeringRequest {
    pub text: String,
    #[serde(default = "default_steering_mode")]
    pub mode: String,
}

fn default_steering_mode() -> String {
    "next_tool_boundary".to_string()
}

fn validate_submit_steering_request(body: &SubmitSteeringRequest) -> Result<()> {
    if body.mode != "next_tool_boundary" {
        return Err(ApiError::BadRequest(format!(
            "unsupported steering mode '{}'; P0 only supports 'next_tool_boundary'",
            body.mode
        )));
    }

    if body.text.trim().is_empty() {
        return Err(ApiError::BadRequest("steering text cannot be empty".into()));
    }

    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct SubmitSteeringResponse {
    pub id: String,
    pub owner_session_id: String,
    pub pending_count: usize,
}

/// POST /session/{id}/steer
///
/// Resolves the owner session (attached sessions auto-resolve to parent),
/// enqueues the steering message, and updates runtime observability.
pub async fn submit_session_steering(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SubmitSteeringRequest>,
) -> Result<Json<SubmitSteeringResponse>> {
    let owner_session_id = resolve_steering_owner_session_id(&state, &session_id).await?;
    validate_submit_steering_request(&body)?;
    let text = body.text.trim().to_string();

    let steer_id = format!("steer_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().timestamp_millis();
    let source_session_id = if owner_session_id == session_id {
        None
    } else {
        Some(session_id.clone())
    };

    let message = PendingSteeringMessage {
        id: steer_id.clone(),
        owner_session_id: owner_session_id.clone(),
        text: text.clone(),
        created_at: now,
        source_session_id: source_session_id.clone(),
        deliver_at: body.mode.clone(),
    };
    let summary = message.to_summary();

    let pending_count = {
        let mut store = state.steering_store.lock().await;
        store.enqueue(&owner_session_id, message);
        store.pending_count(&owner_session_id)
    };

    // Update runtime observable state (Constitution §8).
    state
        .runtime_telemetry
        .steering_enqueued(&owner_session_id, summary)
        .await;

    // Signal session update hook.
    state
        .runtime_telemetry
        .record_session_updated(&owner_session_id, "steering_enqueued")
        .await;

    Ok(Json(SubmitSteeringResponse {
        id: steer_id,
        owner_session_id,
        pending_count,
    }))
}

/// Resolve the true owner session id for a steering request.
/// Constitution §5: prompt continuity ownership must be unambiguous.
///
/// Only `SchedulerStageOutputSession` (attached output) sessions auto-resolve
/// to their parent. Root sessions, delegated subsessions, and explicit forks
/// keep their own prompt continuity — steering targets them directly.
async fn resolve_steering_owner_session_id(
    state: &ServerState,
    session_id: &str,
) -> Result<String> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;

    let record = session.record();

    // Only SchedulerStageOutputSession (attached output) resolves to parent.
    let is_attached_output = record
        .metadata
        .get(rocode_session::session::SESSION_CONTEXT_KIND_METADATA_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s == "scheduler_stage_output_session")
        .unwrap_or(false);

    if is_attached_output {
        if let Some(parent_id) = &record.parent_id {
            if sessions.get(parent_id).is_some() {
                return Ok(parent_id.clone());
            }
            return Err(ApiError::BadRequest(format!(
                "resolved owner session {} not found",
                parent_id
            )));
        }
    }

    Ok(session_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerState;
    use axum::extract::{Path, State};
    use axum::Json;
    use rocode_session::Session;
    use rocode_types::SessionContextKind;
    use std::sync::Arc;

    #[test]
    fn rejects_non_next_tool_boundary_mode() {
        let body = SubmitSteeringRequest {
            text: "valid text".into(),
            mode: "immediate".into(),
        };
        assert!(validate_submit_steering_request(&body).is_err());
    }

    #[test]
    fn accepts_next_tool_boundary_mode() {
        let body = SubmitSteeringRequest {
            text: "valid text".into(),
            mode: "next_tool_boundary".into(),
        };
        assert!(validate_submit_steering_request(&body).is_ok());
    }

    #[tokio::test]
    async fn attached_output_session_resolves_to_parent_owner_on_submit() {
        let state = Arc::new(ServerState::new());
        let (parent_id, child_id) = {
            let mut sessions = state.sessions.lock().await;
            let parent = sessions.create("project", "/tmp/project");
            let child = Session::attached_with_context_kind(
                &parent,
                SessionContextKind::SchedulerStageOutputSession,
            );
            let parent_id = parent.id.clone();
            let child_id = child.id.clone();
            sessions.update(child);
            (parent_id, child_id)
        };

        let Json(response) = submit_session_steering(
            State(state.clone()),
            Path(child_id.clone()),
            Json(SubmitSteeringRequest {
                text: "stop after current tool".into(),
                mode: "next_tool_boundary".into(),
            }),
        )
        .await
        .expect("attached output steering should succeed");

        assert_eq!(response.owner_session_id, parent_id);
        assert_eq!(response.pending_count, 1);

        let pending = state
            .runtime_telemetry
            .runtime_state()
            .get(&response.owner_session_id)
            .await
            .expect("runtime state should exist");
        assert_eq!(pending.pending_steering.len(), 1);
        assert_eq!(
            pending.pending_steering[0].source_session_id.as_deref(),
            Some(child_id.as_str())
        );
    }

    #[tokio::test]
    async fn delegated_subsession_keeps_own_owner_on_submit() {
        let state = Arc::new(ServerState::new());
        let child_id = {
            let mut sessions = state.sessions.lock().await;
            let parent = sessions.create("project", "/tmp/project");
            let child = Session::attached_with_context_kind(
                &parent,
                SessionContextKind::DelegatedSubsession,
            );
            let child_id = child.id.clone();
            sessions.update(child);
            child_id
        };

        let Json(response) = submit_session_steering(
            State(state),
            Path(child_id.clone()),
            Json(SubmitSteeringRequest {
                text: "use worker only".into(),
                mode: "next_tool_boundary".into(),
            }),
        )
        .await
        .expect("delegated subsession steering should succeed");

        assert_eq!(response.owner_session_id, child_id);
    }
}
