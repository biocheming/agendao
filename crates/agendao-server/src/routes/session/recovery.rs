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

fn recovery_selection_overrides(
    session: &agendao_session::Session,
) -> (
    Option<String>,
    Option<agendao_orchestrator::selector::SchedulerChoice>,
) {
    let agent = session_agent_override(session);
    let scheduler = session_scheduler_override(session);
    match (&agent, &scheduler) {
        (Some(_), Some(agendao_orchestrator::selector::SchedulerChoice::Template { template }))
            if *template == agendao_orchestrator::templates::TemplateId::Direct =>
        {
            (agent, None)
        }
        (_, Some(_)) => (None, scheduler),
        _ => (agent, None),
    }
}

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
        .runtime_telemetry
        .session_execution_topology(&session_id)
        .await;
    let pending_question_count = state
        .runtime_telemetry
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
        .runtime_telemetry
        .session_execution_topology(&session_id)
        .await;
    let pending_question_count = state
        .runtime_telemetry
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
    let ledger = crate::session_runtime::task_ledger::ledger_snapshot_from_record(
        &session_id,
        session
            .record()
            .metadata
            .get(crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY),
    );
    let recovery_context = RecoveryExecutionContext::from_ledger(req.action.clone(), &ledger);
    let (agent, scheduler) = recovery_selection_overrides(&session);

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
            compose_resume_prompt(&base_prompt, &recovery_context),
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
            reasoning_effort: None,
            agent,
            scheduler,
            command: None,
            arguments: None,
            source_origin: Some(agendao_types::MessageSourceOrigin::System),
            source_surface: None,
            recovery: Some(recovery_context.clone()),
            auto_continuation_goal_generation: None,
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
        object.insert(
            "recovery_ledger_revision".to_string(),
            serde_json::json!(recovery_context.ledger_revision),
        );
        object.insert(
            "recovery_checkpoint_ids".to_string(),
            serde_json::json!(recovery_context.checkpoint_ids),
        );
        object.insert(
            "recovery_open_ids".to_string(),
            serde_json::json!(recovery_context.open_ids),
        );
        object.insert(
            "recovery_next_statement".to_string(),
            serde_json::json!(recovery_context.next_statement),
        );
    }
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_selection(
        agent: Option<&str>,
        scheduler: agendao_orchestrator::selector::SchedulerChoice,
    ) -> agendao_session::Session {
        let mut session = agendao_session::Session::new("project", "/tmp/project");
        if let Some(agent) = agent {
            session.insert_metadata("agent", serde_json::json!(agent));
        }
        session.insert_metadata(
            "scheduler",
            serde_json::to_value(scheduler).expect("serialize scheduler"),
        );
        session
    }

    #[test]
    fn recovery_prefers_non_direct_scheduler_over_runtime_agent_hint() {
        let session = session_with_selection(
            Some("build"),
            agendao_orchestrator::selector::SchedulerChoice::Auto,
        );
        let (agent, scheduler) = recovery_selection_overrides(&session);
        assert!(agent.is_none());
        assert_eq!(
            scheduler,
            Some(agendao_orchestrator::selector::SchedulerChoice::Auto)
        );
    }

    #[test]
    fn recovery_preserves_explicit_agent_for_direct_scheduler() {
        let session = session_with_selection(
            Some("build"),
            agendao_orchestrator::selector::SchedulerChoice::Template {
                template: agendao_orchestrator::templates::TemplateId::Direct,
            },
        );
        let (agent, scheduler) = recovery_selection_overrides(&session);
        assert_eq!(agent.as_deref(), Some("build"));
        assert!(scheduler.is_none());
    }
}
