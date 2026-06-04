use std::sync::Arc;

use agendao_stage_protocol::StageSummary;
use axum::extract::{Path, State};
use axum::Json;

use crate::{Result, ServerState};

use super::cancel::ensure_session_exists;

pub(super) async fn get_session_stages(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<StageSummary>>> {
    ensure_session_exists(&state, &session_id).await?;
    Ok(Json(
        state
            .runtime_telemetry
            .list_stage_summaries(&session_id)
            .await,
    ))
}
