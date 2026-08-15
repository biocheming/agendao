use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::worktree::{self, WorktreeInfo as WorktreeInfoStruct};
use crate::{ApiError, Result, ServerState};
use agendao_config::Config as AppConfig;
use agendao_session::query_model_repair_summary;
use agendao_types::{RepairKind, RepairQuery, RepairQueryResponse};

pub(crate) fn global_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/health", get(global_health))
        .route("/event", get(global_event_stream))
        .route("/diagnostics", get(global_diagnostics))
        .route("/perf", get(global_perf))
        .route("/config", get(get_global_config))
        .route("/repair/query", get(query_global_repair))
}

pub(crate) fn experimental_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route(
            "/worktree",
            get(list_worktrees)
                .post(create_worktree)
                .delete(remove_worktree),
        )
        .route("/worktree/reset", post(reset_worktree))
        .route("/resource", get(list_resources))
}

#[derive(Debug, Serialize)]
pub struct GlobalHealthResponse {
    pub healthy: bool,
    pub version: String,
}

async fn global_health() -> Json<GlobalHealthResponse> {
    Json(GlobalHealthResponse {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[derive(Debug, Serialize)]
pub struct GlobalPerfResponse {
    pub list_messages_calls: u64,
    pub list_messages_incremental_calls: u64,
    pub list_messages_full_calls: u64,
}

#[derive(Debug, Serialize)]
pub struct GlobalDiagnosticsResponse {}

async fn global_event_stream(
    State(state): State<Arc<ServerState>>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    super::stream_server_events(
        state.event_bus.subscribe(),
        None,
        agendao_api::ResolvedFrontendSubscription::from_tier(
            agendao_api::FrontendSubscriptionTier::TuiHighFrequency,
        ),
        state.event_bus_telemetry.clone(),
    )
}

async fn get_global_config(State(state): State<Arc<ServerState>>) -> Result<Json<AppConfig>> {
    let config = state.config_store.config();
    Ok(Json((*config).clone()))
}

async fn global_diagnostics() -> Json<GlobalDiagnosticsResponse> {
    Json(GlobalDiagnosticsResponse {})
}

async fn global_perf(State(state): State<Arc<ServerState>>) -> Json<GlobalPerfResponse> {
    Json(GlobalPerfResponse {
        list_messages_calls: state.api_perf.list_messages_calls.load(Ordering::Relaxed),
        list_messages_incremental_calls: state
            .api_perf
            .list_messages_incremental_calls
            .load(Ordering::Relaxed),
        list_messages_full_calls: state
            .api_perf
            .list_messages_full_calls
            .load(Ordering::Relaxed),
    })
}

#[derive(Debug, Deserialize)]
pub struct GlobalRepairQueryParams {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub tool_name: Option<String>,
    pub repair_kind: Option<String>,
    pub layer: Option<String>,
    #[serde(default)]
    pub strict_only: bool,
    #[serde(default)]
    pub include_samples: bool,
    pub limit: Option<usize>,
}

impl GlobalRepairQueryParams {
    fn to_query(&self) -> RepairQuery {
        RepairQuery {
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
            tool_name: self.tool_name.clone(),
            repair_kind: self.repair_kind.as_deref().and_then(RepairKind::parse),
            layer: self.layer.clone(),
            strict_only: Some(self.strict_only),
            include_samples: Some(self.include_samples),
            limit: self.limit,
            ..Default::default()
        }
    }
}

async fn query_global_repair(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<GlobalRepairQueryParams>,
) -> Result<Json<RepairQueryResponse>> {
    let query = params.to_query();
    let sessions = state.sessions.lock().await;
    Ok(Json(query_model_repair_summary(sessions.list(), &query)))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub head: String,
}

impl From<WorktreeInfoStruct> for WorktreeInfo {
    fn from(info: WorktreeInfoStruct) -> Self {
        Self {
            path: info.path,
            branch: info.branch,
            head: info.head,
        }
    }
}

async fn list_worktrees(State(state): State<Arc<ServerState>>) -> Json<Vec<WorktreeInfo>> {
    let cwd = state.project_root();
    let worktrees = worktree::list_worktrees(&cwd).unwrap_or_default();
    Json(worktrees.into_iter().map(|w| w.into()).collect())
}

#[derive(Debug, Deserialize)]
pub struct CreateWorktreeRequest {
    pub branch: Option<String>,
    pub path: Option<String>,
}

async fn create_worktree(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateWorktreeRequest>,
) -> Result<Json<WorktreeInfo>> {
    let cwd = state.project_root();

    let info = worktree::create_worktree(&cwd, req.branch.as_deref(), req.path.as_deref())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(info.into()))
}

#[derive(Debug, Deserialize)]
pub struct RemoveWorktreeRequest {
    pub path: String,
    pub force: Option<bool>,
}

async fn remove_worktree(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RemoveWorktreeRequest>,
) -> Result<Json<bool>> {
    let cwd = state.project_root();

    worktree::remove_worktree(&cwd, &req.path, req.force.unwrap_or(false))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(true))
}

async fn reset_worktree(State(state): State<Arc<ServerState>>) -> Result<Json<bool>> {
    let cwd = state.project_root();

    worktree::prune_worktrees(&cwd).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(true))
}

#[derive(Debug, Serialize)]
pub struct ResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
}

async fn list_resources() -> Json<Vec<ResourceInfo>> {
    Json(Vec::new())
}
