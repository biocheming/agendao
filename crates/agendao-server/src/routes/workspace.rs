use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use axum::{extract::State, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{ApiError, Result, ServerState};

pub(crate) fn workspace_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/context", get(get_workspace_context))
        .route(
            "/recent-models",
            get(get_workspace_recent_models).put(put_workspace_recent_models),
        )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecentModelsPayload {
    #[serde(default)]
    pub recent_models: Vec<RecentModelEntry>,
}

pub(crate) async fn get_workspace_context(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<ResolvedWorkspaceContext>> {
    let context = state
        .refresh_resolved_context()
        .await
        .map_err(|error| ApiError::InternalError(error.to_string()))?;
    Ok(Json(context))
}

pub(crate) async fn get_workspace_recent_models(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<RecentModelsPayload>> {
    let context = state
        .refresh_resolved_context()
        .await
        .map_err(|error| ApiError::InternalError(error.to_string()))?;
    Ok(Json(RecentModelsPayload {
        recent_models: context.recent_models,
    }))
}

pub(crate) async fn put_workspace_recent_models(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<RecentModelsPayload>,
) -> Result<Json<RecentModelsPayload>> {
    state
        .user_state
        .save_recent_models(&payload.recent_models)
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let context = state
        .refresh_resolved_context()
        .await
        .map_err(|error| ApiError::InternalError(error.to_string()))?;
    Ok(Json(RecentModelsPayload {
        recent_models: context.recent_models,
    }))
}
