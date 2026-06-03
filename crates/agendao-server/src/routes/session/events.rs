use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;

use crate::{Result, ServerState};

// ─── Stage Event Log endpoints ────────────────────────────────────────

/// Query parameters for `GET /session/{id}/events`.
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Filter by stage_id.
    #[serde(default)]
    pub stage_id: Option<String>,
    /// Filter by execution_id.
    #[serde(default)]
    pub execution_id: Option<String>,
    /// Filter by event_type (e.g. `"execution.topology.changed"`).
    #[serde(default)]
    pub event_type: Option<String>,
    /// Only return events with `ts >= since` (epoch milliseconds).
    #[serde(default)]
    pub since: Option<i64>,
    /// Maximum number of events to return.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Number of matching events to skip.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// `GET /session/{id}/events` — query stage events for a session.
pub(super) async fn get_session_events(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<agendao_command::stage_protocol::StageEvent>>> {
    let filter = crate::stage_event_log::EventFilter {
        stage_id: query.stage_id,
        execution_id: query.execution_id,
        event_type: query.event_type,
        since: query.since,
        limit: query.limit,
        offset: query.offset,
    };
    let events = state
        .runtime_telemetry
        .query_stage_events(&session_id, &filter)
        .await;
    Ok(Json(events))
}

/// `GET /session/{id}/events/stages` — list distinct stage IDs that have events.
pub(super) async fn get_session_event_stages(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<String>>> {
    let ids = state.runtime_telemetry.list_stage_ids(&session_id).await;
    Ok(Json(ids))
}
