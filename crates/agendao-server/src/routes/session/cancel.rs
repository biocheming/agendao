use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};

use crate::{ApiError, Result, ServerState};
use agendao_server_core::runtime_state::InterruptTarget;

use super::super::tui::cancel_questions_for_session;

pub(super) async fn abort_prompt(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    ensure_session_exists(&state, &id).await?;
    let response = abort_session_execution(&state, &id).await;
    Ok(Json(response))
}

pub(super) async fn abort_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    ensure_session_exists(&state, &id).await?;
    let response = abort_session_execution(&state, &id).await;
    Ok(Json(response))
}

pub(super) async fn ensure_session_exists(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Result<()> {
    let sessions = state.sessions.lock().await;
    if sessions.get(session_id).is_none() {
        return Err(ApiError::SessionNotFound(session_id.to_string()));
    }
    Ok(())
}

pub(super) async fn abort_session_execution(
    state: &Arc<ServerState>,
    session_id: &str,
) -> serde_json::Value {
    crate::routes::permission::PERMISSION_ENGINE
        .lock()
        .await
        .clear_turn(session_id);

    // Stop means stop for the whole session: prompts queued behind the
    // running turn are dropped instead of being auto-executed by a later run.
    let dropped_queued_prompts = super::prompt::drain_followup_prompts(state, session_id).await;
    let cancelled_auto_continuation = {
        let mut sessions = state.sessions.lock().await;
        sessions.get_mut(session_id).is_some_and(|session| {
            session
                .remove_metadata(
                    crate::session_runtime::task_ledger::TASK_LEDGER_AUTO_CONTINUATION_METADATA_KEY,
                )
                .is_some()
        })
    };
    if cancelled_auto_continuation {
        super::session_crud::persist_session_record_if_enabled(state, session_id).await;
    }

    // Pending permissions have no deadline and must resolve explicitly on
    // abort; otherwise their popups and waiter futures would remain live on
    // every frontend until restart.
    let cancelled_permissions =
        crate::routes::permission::cancel_pending_permissions_for_session(state, session_id).await;
    // Questions are an independent interaction registry. Clear them once
    // even if the scheduler token has already retired; otherwise an abort at
    // the run boundary can leave a stale awaiting-user prompt behind.
    let cancelled_questions = cancel_questions_for_session(state.clone(), session_id).await;

    let mut prompt_running = false;
    let scheduler_running = state
        .runtime_telemetry
        .request_scheduler_cancel(session_id)
        .await;

    if state.runtime_telemetry.has_prompt_run(session_id).await {
        prompt_running = true;
        state.prompt_runner.cancel(session_id).await;
    }

    if scheduler_running || prompt_running {
        state
            .runtime_telemetry
            .interrupt_requested(session_id, InterruptTarget::Run)
            .await;
    }

    let aborted = scheduler_running
        || prompt_running
        || dropped_queued_prompts > 0
        || cancelled_auto_continuation
        || cancelled_permissions > 0
        || cancelled_questions > 0;
    if aborted {
        // Task-governance seam: the user interrupted the run; the ledger
        // keeps its pre-interrupt Next with provenance marked.
        crate::session_runtime::task_ledger_reducer::dispatch_seam(
            state,
            session_id,
            agendao_types::task_ledger::TaskLedgerSeamFact::RecoveryInterrupted,
        )
        .await;
        serde_json::json!({
            "aborted": aborted,
            "target": "run",
            "dropped_queued_prompts": dropped_queued_prompts,
            "cancelled_task_ledger_auto_continuation": cancelled_auto_continuation,
            "cancelled_pending_permissions": cancelled_permissions,
            "cancelled_pending_questions": cancelled_questions,
        })
    } else {
        serde_json::json!({
            "aborted": false,
            "target": serde_json::Value::Null,
            "dropped_queued_prompts": dropped_queued_prompts,
            "cancelled_task_ledger_auto_continuation": cancelled_auto_continuation,
            "cancelled_pending_permissions": cancelled_permissions,
            "cancelled_pending_questions": cancelled_questions,
        })
    }
}
