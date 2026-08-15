use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use crate::recovery::{
    build_session_recovery_protocol, compose_resume_prompt, compose_retry_prompt,
    protocol_allows_recovery_action, ExecuteRecoveryRequest, RecoveryActionKind,
    RecoveryExecutionContext, RecoveryProtocolStatus, SessionRecoveryProtocol,
};
use crate::{ApiError, Result, ServerState};

use super::cancel::{abort_session_execution, ensure_session_exists};
use super::prompt::{session_prompt, SessionPromptRequest};
use super::session_crud::{
    session_agent_override, session_model_override, session_scheduler_override,
    session_variant_override,
};

pub(super) async fn get_session_recovery(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionRecoveryProtocol>> {
    ensure_session_exists(&state, &session_id).await?;
    let session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?
    };
    let topology = state
        .runtime_control
        .list_session_execution_topology(&session_id)
        .await;
    let pending_question_count = state
        .runtime_control
        .list_questions_for_session(&session_id)
        .await
        .len();
    Ok(Json(build_session_recovery_protocol(
        &session_id,
        &session,
        &topology,
        pending_question_count,
    )))
}

pub(super) async fn execute_session_recovery(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(req): Json<ExecuteRecoveryRequest>,
) -> Result<Json<serde_json::Value>> {
    ensure_session_exists(&state, &session_id).await?;
    let session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?
    };
    let topology = state
        .runtime_control
        .list_session_execution_topology(&session_id)
        .await;
    let pending_question_count = state
        .runtime_control
        .list_questions_for_session(&session_id)
        .await
        .len();
    let protocol =
        build_session_recovery_protocol(&session_id, &session, &topology, pending_question_count);

    if !protocol_allows_recovery_action(&protocol, &req.action) {
        return Err(ApiError::BadRequest(format!(
            "Recovery action `{:?}` is not available for the current session state",
            req.action
        )));
    }

    if matches!(req.action, RecoveryActionKind::AbortRun) {
        let response = abort_session_execution(&state, &session_id).await;
        let mut value = response;
        if let Some(object) = value.as_object_mut() {
            object.insert("recovery_action".to_string(), serde_json::json!(req.action));
        }
        return Ok(Json(value));
    }

    if matches!(
        protocol.status,
        RecoveryProtocolStatus::Running | RecoveryProtocolStatus::AwaitingUser
    ) {
        return Err(ApiError::BadRequest(protocol.summary.unwrap_or_else(
            || "Session is not ready for recovery execution".to_string(),
        )));
    }

    let base_prompt = protocol.last_user_prompt.clone().ok_or_else(|| {
        ApiError::BadRequest("No prior user prompt is available for recovery".to_string())
    })?;

    let (composed_message, target_label) = match req.action {
        RecoveryActionKind::AbortRun => {
            debug_assert!(
                false,
                "abort recovery actions should be handled before composing a recovery prompt"
            );
            return Err(ApiError::BadRequest(
                "Abort actions must be handled via the abort recovery path".to_string(),
            ));
        }
        RecoveryActionKind::Retry => (compose_retry_prompt(&base_prompt), "last run".to_string()),
        RecoveryActionKind::Resume => (
            compose_resume_prompt(&base_prompt),
            "latest boundary".to_string(),
        ),
    };

    let response = session_prompt(
        State(state.clone()),
        HeaderMap::new(),
        Path(session_id.clone()),
        Json(SessionPromptRequest {
            message: Some(composed_message),
            parts: None,
            idempotency_key: None,
            ingress_source: Some("api".to_string()),
            model: session_model_override(&session),
            variant: session_variant_override(&session),
            agent: session_agent_override(&session),
            scheduler: session_scheduler_override(&session),
            command: None,
            arguments: None,
            source_origin: Some(agendao_types::MessageSourceOrigin::System),
            source_surface: None,
            recovery: Some(RecoveryExecutionContext {
                action: Some(req.action.clone()),
            }),
        }),
    )
    .await?;

    let mut value = response.0;
    if let Some(object) = value.as_object_mut() {
        object.insert("recovery_action".to_string(), serde_json::json!(req.action));
        object.insert(
            "recovery_target_label".to_string(),
            serde_json::json!(target_label),
        );
    }
    Ok(Json(value))
}
