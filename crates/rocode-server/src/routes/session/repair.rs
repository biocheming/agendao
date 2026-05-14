//! Session repair query handlers (P1.3).

use axum::extract::{Path, Query, State};
use axum::Json;
use rocode_api::SessionRepairSummaryResponse;
use serde::Deserialize;
use std::sync::Arc;

use rocode_session::repair_query::{
    build_session_repair_query_snapshot, load_session_repair_query_snapshot,
    query_session_repair_snapshot,
};
use rocode_types::{RepairKind, RepairQuery};

use crate::{ApiError, Result, ServerState};

#[derive(Debug, Deserialize)]
pub struct RepairQueryParams {
    pub tool_name: Option<String>,
    pub repair_kind: Option<String>,
    pub layer: Option<String>,
    #[serde(default)]
    pub strict_only: bool,
    #[serde(default)]
    pub include_samples: bool,
    pub limit: Option<usize>,
}

impl RepairQueryParams {
    fn to_query(&self) -> RepairQuery {
        RepairQuery {
            repair_kind: self
                .repair_kind
                .as_deref()
                .and_then(RepairKind::from_legacy_str),
            tool_name: self.tool_name.clone(),
            layer: self.layer.clone(),
            strict_only: Some(self.strict_only),
            include_samples: Some(self.include_samples),
            limit: self.limit,
            ..Default::default()
        }
    }
}

/// GET /session/{id}/repair/summary
pub async fn get_session_repair_summary(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionRepairSummaryResponse>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let snapshot = load_session_repair_query_snapshot(session)
        .or_else(|| build_session_repair_query_snapshot(session));

    Ok(Json(SessionRepairSummaryResponse {
        session_id,
        snapshot,
    }))
}

/// GET /session/{id}/repair/query
pub async fn query_session_repair(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Query(params): Query<RepairQueryParams>,
) -> Result<Json<rocode_types::RepairQueryResponse>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let query = params.to_query();
    Ok(Json(query_session_repair_snapshot(session, &query)))
}
