use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use rocode_types::{
    MemoryConflictResponse, MemoryConsolidationRequest, MemoryConsolidationResponse,
    MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery, MemoryDetailView,
    MemoryListQuery, MemoryListResponse, MemoryRecordId, MemoryRetrievalPreviewResponse,
    MemoryRetrievalQuery, MemoryRuleHitListResponse, MemoryRuleHitQuery,
    MemoryRulePackListResponse, MemoryValidationReportResponse,
};

use crate::{ApiError, Result, ServerState};

pub(crate) fn memory_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/list", get(list_memory))
        .route("/search", get(search_memory))
        .route("/rule-packs", get(list_memory_rule_packs))
        .route("/rule-hits", get(list_memory_rule_hits))
        .route("/consolidation/runs", get(list_consolidation_runs))
        .route("/consolidate", post(run_memory_consolidation))
        .route("/retrieval-preview", get(get_memory_retrieval_preview))
        .route("/{id}", get(get_memory_detail))
        .route("/{id}/validation-report", get(get_memory_validation_report))
        .route("/{id}/conflicts", get(get_memory_conflicts))
}

async fn list_memory(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MemoryListQuery>,
) -> Result<Json<MemoryListResponse>> {
    let response = state
        .runtime_memory
        .list_memory_for_query(&query)
        .await
        .map_err(|error| ApiError::InternalError(format!("failed to list memory: {error}")))?;
    Ok(Json(response))
}

async fn search_memory(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MemoryListQuery>,
) -> Result<Json<MemoryListResponse>> {
    let response = state
        .runtime_memory
        .search_memory_for_query(&query)
        .await
        .map_err(|error| ApiError::InternalError(format!("failed to search memory: {error}")))?;
    Ok(Json(response))
}

async fn list_memory_rule_packs(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<MemoryRulePackListResponse>> {
    let response = state
        .runtime_memory
        .list_memory_rule_packs()
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to list memory rule packs: {error}"))
        })?;
    Ok(Json(response))
}

async fn list_memory_rule_hits(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MemoryRuleHitQuery>,
) -> Result<Json<MemoryRuleHitListResponse>> {
    let response = state
        .runtime_memory
        .list_memory_rule_hits(&query)
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to list memory rule hits: {error}"))
        })?;
    Ok(Json(response))
}

async fn list_consolidation_runs(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MemoryConsolidationRunQuery>,
) -> Result<Json<MemoryConsolidationRunListResponse>> {
    let response = state
        .runtime_memory
        .list_consolidation_runs(&query)
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to list memory consolidation runs: {error}"))
        })?;
    Ok(Json(response))
}

async fn run_memory_consolidation(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<MemoryConsolidationRequest>,
) -> Result<Json<MemoryConsolidationResponse>> {
    let response = state
        .runtime_memory
        .run_consolidation(&request)
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to run memory consolidation: {error}"))
        })?;
    Ok(Json(response))
}

async fn get_memory_retrieval_preview(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MemoryRetrievalQuery>,
) -> Result<Json<MemoryRetrievalPreviewResponse>> {
    let response = state
        .runtime_memory
        .build_retrieval_preview(&query)
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to build memory retrieval preview: {error}"))
        })?;
    Ok(Json(response))
}

async fn get_memory_detail(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<MemoryDetailView>> {
    state
        .runtime_memory
        .get_memory_detail(&MemoryRecordId(id.clone()))
        .await
        .map_err(|error| ApiError::InternalError(format!("failed to load memory detail: {error}")))?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("memory record not found: {}", id)))
}

async fn get_memory_validation_report(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<MemoryValidationReportResponse>> {
    state
        .runtime_memory
        .get_memory_validation_report(&MemoryRecordId(id.clone()))
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to load memory validation report: {error}"))
        })?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("memory record not found: {}", id)))
}

async fn get_memory_conflicts(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<MemoryConflictResponse>> {
    state
        .runtime_memory
        .get_memory_conflicts(&MemoryRecordId(id.clone()))
        .await
        .map_err(|error| {
            ApiError::InternalError(format!("failed to load memory conflicts: {error}"))
        })?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("memory record not found: {}", id)))
}
