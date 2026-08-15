//! Steering API: submit a mid-run steering message to the owner session.
//! Constitution §9: TUI/CLI/Web submit; runtime consumes at tool boundary.

use agendao_types::SessionMessage;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::session_runtime::events::broadcast_session_reconcile;
use crate::session_runtime::steering::PendingSteeringMessage;
use crate::{ApiError, Result, ServerState};
use agendao_server_core::runtime_events::ReconcileReason;

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
/// Enqueues a steering message for the addressed session and updates runtime observability.
pub async fn submit_session_steering(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SubmitSteeringRequest>,
) -> Result<Json<SubmitSteeringResponse>> {
    {
        let sessions = state.sessions.lock().await;
        if sessions.get(&session_id).is_none() {
            return Err(ApiError::SessionNotFound(session_id.clone()));
        }
    }
    let owner_session_id = session_id.clone();
    validate_submit_steering_request(&body)?;
    let text = body.text.trim().to_string();

    let steer_id = format!("steer_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().timestamp_millis();
    let source_session_id = None;

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
    )
    .await;

    Ok(Json(SubmitSteeringResponse {
        id: steer_id,
        owner_session_id,
        pending_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
