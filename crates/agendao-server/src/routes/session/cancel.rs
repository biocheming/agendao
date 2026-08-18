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

    // Pending permissions must resolve now: their waiting futures are about
    // to be dropped, and their 300s timeout lives inside those futures —
    // without this the popups would hang on every frontend until restart.
    let cancelled_permissions =
        crate::routes::permission::cancel_pending_permissions_for_session(state, session_id).await;

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

    if scheduler_running {
        let _ = cancel_questions_for_session(state.clone(), session_id).await;
    }

    if prompt_running {
        let _ = cancel_questions_for_session(state.clone(), session_id).await;
    }

    if scheduler_running || prompt_running {
        // Task-governance seam: the user interrupted the run; the ledger
        // keeps its pre-interrupt Next with provenance marked.
        crate::session_runtime::task_ledger_reducer::dispatch_seam(
            state,
            session_id,
            agendao_types::task_ledger::TaskLedgerSeamFact::RecoveryInterrupted,
        )
        .await;
        serde_json::json!({
            "aborted": true,
            "target": "run",
            "dropped_queued_prompts": dropped_queued_prompts,
            "cancelled_pending_permissions": cancelled_permissions,
        })
    } else {
        serde_json::json!({
            "aborted": false,
            "target": serde_json::Value::Null,
            "dropped_queued_prompts": dropped_queued_prompts,
            "cancelled_pending_permissions": cancelled_permissions,
        })
    }
}
