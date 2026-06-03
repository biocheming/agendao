//! Steering API: submit a mid-run steering message to the owner session.
//! Constitution §9: TUI/CLI/Web submit; runtime consumes at tool boundary.

use axum::extract::{Path, State};
use axum::Json;
use agendao_types::SessionMessage;
use serde::Deserialize;
use std::sync::Arc;

use crate::session_runtime::events::{broadcast_session_reconcile, ReconcileReason};
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

    // Immediate transcript echo: push two lines into the owner session so the
    // user sees instant feedback even before the next tool boundary (§8 observability).
    {
        let mut sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&owner_session_id) {
            let now_ms = chrono::Utc::now().timestamp_millis();
            // Line 1: meta notice — when this steering will be applied.
            let mut notice = SessionMessage::user(
                &owner_session_id,
                format!(
                    "Steering: will be applied at next tool boundary (pending: {})",
                    pending_count
                ),
            );
            // Hidden from model-visible replay: this is UI feedback, not a user instruction.
            notice.metadata.insert(
                "runtime_hint".to_string(),
                serde_json::json!("steering_preview"),
            );
            notice.metadata.insert(
                "steering_mode".to_string(),
                serde_json::json!("next_tool_boundary"),
            );
            notice
                .metadata
                .insert("steering_status".to_string(), serde_json::json!("pending"));
            notice.metadata.insert(
                "steering_enqueued_at".to_string(),
                serde_json::json!(now_ms),
            );
            notice.metadata.insert(
                "steering_owner_session_id".to_string(),
                serde_json::json!(&owner_session_id),
            );
            if let Some(ref source) = source_session_id {
                notice.metadata.insert(
                    "steering_source_session_id".to_string(),
                    serde_json::json!(source),
                );
            }
            // Stamp canonical source metadata (System origin).
            let (admission, authority) = agendao_types::origin_to_admission_authority(
                agendao_types::MessageSourceOrigin::System,
            );
            agendao_types::apply_message_source_metadata(
                &mut notice.metadata,
                agendao_types::MessageSourceOrigin::System,
                agendao_types::MessageSourceSurface::HttpApi,
            );
            agendao_types::apply_message_admission_metadata(
                &mut notice.metadata,
                admission,
                authority,
            );
            session.push_message(notice);

            // Line 2: the actual queued steering text.
            let mut preview = SessionMessage::user(&owner_session_id, &text);
            // Hidden from model-visible replay: the model must not see a duplicate
            // of the steering text before it is consumed at the tool boundary.
            preview.metadata.insert(
                "runtime_hint".to_string(),
                serde_json::json!("steering_preview"),
            );
            preview.metadata.insert(
                "steering_mode".to_string(),
                serde_json::json!("next_tool_boundary"),
            );
            preview
                .metadata
                .insert("steering_status".to_string(), serde_json::json!("pending"));
            preview.metadata.insert(
                "steering_enqueued_at".to_string(),
                serde_json::json!(now_ms),
            );
            preview.metadata.insert(
                "steering_owner_session_id".to_string(),
                serde_json::json!(&owner_session_id),
            );
            if let Some(ref source) = source_session_id {
                preview.metadata.insert(
                    "steering_source_session_id".to_string(),
                    serde_json::json!(source),
                );
            }
            agendao_types::apply_message_source_metadata(
                &mut preview.metadata,
                agendao_types::MessageSourceOrigin::System,
                agendao_types::MessageSourceSurface::HttpApi,
            );
            agendao_types::apply_message_admission_metadata(
                &mut preview.metadata,
                admission,
                authority,
            );
            session.push_message(preview);
        }
    }

    // Update runtime observable state (Constitution §8).
    state
        .runtime_telemetry
        .steering_enqueued(&owner_session_id, summary)
        .await;

    broadcast_session_reconcile(
        state.as_ref(),
        owner_session_id.clone(),
        ReconcileReason::Steering,
    );

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
        .get(agendao_session::session::SESSION_CONTEXT_KIND_METADATA_KEY)
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
    use agendao_session::Session;
    use agendao_types::SessionContextKind;
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
