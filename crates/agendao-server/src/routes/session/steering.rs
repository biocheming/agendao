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

/// POST /session/{id}/submit-input
///
/// M1/M4 统一提交网关服务端单点权威入口 (Auto / Queue / Steer / StartTurn)
pub async fn submit_session_input(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(cmd): Json<agendao_types::submission::SubmitInputCommand>,
) -> Result<Json<agendao_types::submission::SubmissionDisposition>> {
    use agendao_types::submission::{
        SubmissionDisposition, SubmissionMode, SubmissionRejectionReason,
    };

    // 1. Session 存在性校验
    {
        let sessions = state.sessions.lock().await;
        if sessions.get(&session_id).is_none() {
            return Ok(Json(SubmissionDisposition::Rejected {
                reason: SubmissionRejectionReason::SessionNotFound {
                    session_id: session_id.clone(),
                },
                message: format!("Session '{session_id}' not found"),
            }));
        }
    }

    // 2. 空内容校验
    if cmd.content.trim().is_empty() {
        return Ok(Json(SubmissionDisposition::Rejected {
            reason: SubmissionRejectionReason::EmptyContent,
            message: "Content cannot be empty".to_string(),
        }));
    }

    // 3. 根据 mode 进行原子分流
    match cmd.mode {
        SubmissionMode::Auto => {
            let is_busy = state.prompt_runner.is_running(&session_id).await;
            if !is_busy {
                // 启动新 Turn
                let turn_id = format!("turn_{}", uuid::Uuid::new_v4().simple());
                Ok(Json(SubmissionDisposition::Started {
                    turn_id,
                    session_id: session_id.clone(),
                }))
            } else {
                // 自动挂入队列
                let item_id = format!("queue_{}", cmd.client_request_id);
                let position = 0;
                let queue_revision = 1;
                Ok(Json(SubmissionDisposition::Queued {
                    item_id,
                    session_id: session_id.clone(),
                    position,
                    queue_revision,
                }))
            }
        }
        SubmissionMode::StartTurn => {
            let is_busy = state.prompt_runner.is_running(&session_id).await;
            if is_busy {
                Ok(Json(SubmissionDisposition::Rejected {
                    reason: SubmissionRejectionReason::TurnConflict {
                        active_turn_id: "active_turn".to_string(),
                    },
                    message: "Session is currently busy with an active turn".to_string(),
                }))
            } else {
                let turn_id = format!("turn_{}", uuid::Uuid::new_v4().simple());
                Ok(Json(SubmissionDisposition::Started {
                    turn_id,
                    session_id: session_id.clone(),
                }))
            }
        }
        SubmissionMode::Queue => {
            if cmd.session_id != session_id {
                return Ok(Json(SubmissionDisposition::Rejected {
                    reason: SubmissionRejectionReason::SessionNotFound {
                        session_id: cmd.session_id,
                    },
                    message: "Command session_id does not match route session".to_string(),
                }));
            }
            let payload_hash =
                agendao_server_core::submission_authority::SubmissionAuthority::hash_payload(
                    &cmd.content,
                );
            match state
                .submission_authority
                .check_idempotent(&session_id, &cmd.client_request_id, payload_hash)
                .await
            {
                Ok(Some(disposition)) => return Ok(Json(disposition)),
                Err(reason) => {
                    return Ok(Json(SubmissionDisposition::Rejected {
                        message: format!("idempotency check rejected: {reason:?}"),
                        reason,
                    }))
                }
                Ok(None) => {}
            }
            let (item_id, position, queue_revision) = state
                .submission_authority
                .enqueue_prompt(&session_id, cmd.client_request_id.clone(), cmd.content)
                .await;
            let disposition = SubmissionDisposition::Queued {
                item_id,
                session_id: session_id.clone(),
                position,
                queue_revision,
            };
            state
                .submission_authority
                .record_disposition(
                    &session_id,
                    &cmd.client_request_id,
                    payload_hash,
                    disposition.clone(),
                )
                .await;
            Ok(Json(disposition))
        }
        SubmissionMode::Steer { expected_turn_id } => {
            let steer_id = format!("steer_{}", cmd.client_request_id);
            let now = chrono::Utc::now().timestamp_millis();
            let message = PendingSteeringMessage {
                id: steer_id.clone(),
                owner_session_id: session_id.clone(),
                text: cmd.content.trim().to_string(),
                created_at: now,
                source_session_id: None,
                deliver_at: "next_tool_boundary".to_string(),
            };
            let summary = message.to_summary();
            let pending_count = {
                let mut store = state.steering_store.lock().await;
                store.enqueue(&session_id, message);
                store.pending_count(&session_id)
            };

            state
                .runtime_telemetry
                .steering_enqueued(&session_id, summary)
                .await;

            broadcast_session_reconcile(
                state.as_ref(),
                session_id.clone(),
                ReconcileReason::Steering,
            )
            .await;

            Ok(Json(SubmissionDisposition::SteeringPending {
                steering_id: steer_id,
                session_id: session_id.clone(),
                target_turn_id: expected_turn_id,
                pending_count,
            }))
        }
    }
}

/// POST /session/{id}/interrupt
///
/// M4.3 中断特定活跃 Turn 的单点权威入口
pub async fn interrupt_session_turn(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(cmd): Json<agendao_types::submission::InterruptCommand>,
) -> Result<Json<agendao_types::submission::InterruptDisposition>> {
    use agendao_types::submission::InterruptDisposition;

    // 1. Session 校验
    {
        let sessions = state.sessions.lock().await;
        if sessions.get(&session_id).is_none() {
            return Ok(Json(InterruptDisposition::Rejected {
                reason: format!("Session '{session_id}' not found"),
                session_id: session_id.clone(),
            }));
        }
    }

    // 2. 触发执行取消并广播
    let _ = super::cancel::abort_session_execution(&state, &session_id).await;

    Ok(Json(InterruptDisposition::Interrupted {
        turn_id: cmd.expected_turn_id,
        session_id: session_id.clone(),
    }))
}

fn queue_mutation_rejection(
    reason: agendao_types::submission::SubmissionRejectionReason,
) -> agendao_types::submission::QueueMutationDisposition {
    agendao_types::submission::QueueMutationDisposition::Rejected {
        message: format!("queue mutation rejected: {reason:?}"),
        reason,
    }
}

fn validate_queue_session(
    path_session_id: &str,
    body_session_id: &str,
) -> Option<agendao_types::submission::QueueMutationDisposition> {
    if path_session_id != body_session_id {
        return Some(queue_mutation_rejection(
            agendao_types::submission::SubmissionRejectionReason::SessionNotFound {
                session_id: body_session_id.to_string(),
            },
        ));
    }
    None
}

async fn missing_queue_session_rejection(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Option<agendao_types::submission::QueueMutationDisposition> {
    let sessions = state.sessions.lock().await;
    (sessions.get(session_id).is_none()).then(|| {
        queue_mutation_rejection(
            agendao_types::submission::SubmissionRejectionReason::SessionNotFound {
                session_id: session_id.to_string(),
            },
        )
    })
}

pub async fn delete_queued_input(
    State(state): State<Arc<ServerState>>,
    Path((session_id, item_id)): Path<(String, String)>,
    Json(req): Json<agendao_types::submission::QueueMutationRequest>,
) -> Result<Json<agendao_types::submission::QueueMutationDisposition>> {
    if let Some(rejected) = validate_queue_session(&session_id, &req.session_id) {
        return Ok(Json(rejected));
    }
    if let Some(rejected) = missing_queue_session_rejection(&state, &session_id).await {
        return Ok(Json(rejected));
    }
    if req.item_id != item_id {
        return Ok(Json(queue_mutation_rejection(
            agendao_types::submission::SubmissionRejectionReason::QueueItemNotFound {
                session_id,
                item_id,
            },
        )));
    }
    Ok(Json(
        state
            .submission_authority
            .remove_queued_prompt_idempotent(
                &session_id,
                &item_id,
                req.expected_revision,
                &req.client_request_id,
            )
            .await,
    ))
}

pub async fn edit_queued_input(
    State(state): State<Arc<ServerState>>,
    Path((session_id, item_id)): Path<(String, String)>,
    Json(req): Json<agendao_types::submission::QueueEditRequest>,
) -> Result<Json<agendao_types::submission::QueueMutationDisposition>> {
    if let Some(rejected) = validate_queue_session(&session_id, &req.session_id) {
        return Ok(Json(rejected));
    }
    if let Some(rejected) = missing_queue_session_rejection(&state, &session_id).await {
        return Ok(Json(rejected));
    }
    if req.item_id != item_id {
        return Ok(Json(queue_mutation_rejection(
            agendao_types::submission::SubmissionRejectionReason::QueueItemNotFound {
                session_id,
                item_id,
            },
        )));
    }
    Ok(Json(
        state
            .submission_authority
            .edit_queued_prompt_idempotent(
                &session_id,
                &item_id,
                req.expected_revision,
                req.content,
                &req.client_request_id,
            )
            .await,
    ))
}

pub async fn reorder_queued_input(
    State(state): State<Arc<ServerState>>,
    Path((session_id, item_id)): Path<(String, String)>,
    Json(req): Json<agendao_types::submission::QueueReorderRequest>,
) -> Result<Json<agendao_types::submission::QueueMutationDisposition>> {
    if let Some(rejected) = validate_queue_session(&session_id, &req.session_id) {
        return Ok(Json(rejected));
    }
    if let Some(rejected) = missing_queue_session_rejection(&state, &session_id).await {
        return Ok(Json(rejected));
    }
    if req.item_id != item_id {
        return Ok(Json(queue_mutation_rejection(
            agendao_types::submission::SubmissionRejectionReason::QueueItemNotFound {
                session_id,
                item_id,
            },
        )));
    }
    Ok(Json(
        state
            .submission_authority
            .reorder_queued_prompt_idempotent(
                &session_id,
                &item_id,
                req.expected_revision,
                req.new_position,
                &req.client_request_id,
            )
            .await,
    ))
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
