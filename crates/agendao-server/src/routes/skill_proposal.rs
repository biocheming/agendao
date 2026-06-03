//! Skill evolution proposal API.
//!
//! Query routes follow the same pattern as `memory.rs`: axum handlers
//! that call through `ServerState` to the shared repository.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use agendao_types::{ProposalStatus, SkillEvolutionProposal};
use serde::Deserialize;

use crate::{ApiError, Result, ServerState};

// ── routes ────────────────────────────────────────────────────────────────

pub(crate) fn skill_proposal_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_proposals))
        .route("/{id}", get(get_proposal))
        .route("/{id}/status", post(update_proposal_status))
}

// ── query params ───────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default = "default_status")]
    status: String,
}

fn default_status() -> String {
    "draft".to_string()
}

#[derive(Deserialize)]
struct StatusUpdate {
    status: String,
}

// ── handlers ──────────────────────────────────────────────────────────────

async fn list_proposals(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<SkillEvolutionProposal>>> {
    let repo = state
        .proposal_repo
        .as_deref()
        .ok_or_else(|| ApiError::InternalError("proposal repository not available".to_string()))?;

    let status: ProposalStatus = serde_json::from_str(&format!("\"{}\"", query.status))
        .map_err(|e| ApiError::BadRequest(format!("invalid status: {e}")))?;

    let proposals = repo
        .list_by_status(&status)
        .await
        .map_err(|e| ApiError::InternalError(format!("failed to list proposals: {e}")))?;

    Ok(Json(proposals))
}

async fn get_proposal(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SkillEvolutionProposal>> {
    let repo = state
        .proposal_repo
        .as_deref()
        .ok_or_else(|| ApiError::InternalError("proposal repository not available".to_string()))?;

    let proposal = repo
        .get_by_id(&id)
        .await
        .map_err(|e| ApiError::InternalError(format!("failed to get proposal: {e}")))?
        .ok_or_else(|| ApiError::NotFound("proposal not found".to_string()))?;

    Ok(Json(proposal))
}

async fn update_proposal_status(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(body): Json<StatusUpdate>,
) -> Result<Json<SkillEvolutionProposal>> {
    let repo = state
        .proposal_repo
        .as_deref()
        .ok_or_else(|| ApiError::InternalError("proposal repository not available".to_string()))?;

    let status: ProposalStatus = serde_json::from_str(&format!("\"{}\"", body.status))
        .map_err(|e| ApiError::BadRequest(format!("invalid status: {e}")))?;

    repo.transition_status(&id, &status)
        .await
        .map_err(|e| ApiError::BadRequest(format!("status transition failed: {e}")))?;

    // Return updated proposal.
    let proposal = repo
        .get_by_id(&id)
        .await
        .map_err(|e| ApiError::InternalError(format!("failed to get updated proposal: {e}")))?
        .ok_or_else(|| ApiError::NotFound("proposal not found".to_string()))?;

    Ok(Json(proposal))
}
