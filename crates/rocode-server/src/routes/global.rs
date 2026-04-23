use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::worktree::{self, WorktreeInfo as WorktreeInfoStruct};
use crate::{ApiError, Result, ServerState};
use rocode_config::Config as AppConfig;

pub(crate) fn global_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/health", get(global_health))
        .route("/event", get(global_event_stream))
        .route("/diagnostics", get(global_diagnostics))
        .route("/perf", get(global_perf))
        .route("/config", get(get_global_config))
        .route("/dispose", post(dispose_all))
}

pub(crate) fn experimental_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_experimental))
        .route("/analyze", post(experimental_analyze))
        .route("/generate", post(experimental_generate))
        .route("/refactor", post(experimental_refactor))
        .route("/test", post(experimental_test))
        .route(
            "/{feature}",
            post(enable_experimental).delete(disable_experimental),
        )
        .route("/tool/ids", get(list_tool_ids))
        .route("/tool", get(list_tools))
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
    super::stream_server_events(state.event_bus.subscribe(), None)
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

async fn dispose_all() -> Json<bool> {
    Json(true)
}

#[derive(Debug, Serialize)]
pub struct ExperimentalFeature {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

async fn list_experimental() -> Json<Vec<ExperimentalFeature>> {
    Json(Vec::new())
}

async fn enable_experimental(Path(_feature): Path<String>) -> Result<Json<bool>> {
    Ok(Json(true))
}

async fn disable_experimental(Path(_feature): Path<String>) -> Result<Json<bool>> {
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct ExperimentalTaskRequest {
    pub prompt: Option<String>,
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ExperimentalTaskResponse {
    pub operation: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

fn make_experimental_response(
    operation: &str,
    payload: ExperimentalTaskRequest,
) -> ExperimentalTaskResponse {
    ExperimentalTaskResponse {
        operation: operation.to_string(),
        status: "accepted".to_string(),
        message: format!(
            "Experimental endpoint `{}` is available but currently returns a placeholder response in Rust.",
            operation
        ),
        prompt: payload.prompt,
        context: payload.context,
    }
}

async fn experimental_analyze(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("analyze", payload))
}

async fn experimental_generate(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("generate", payload))
}

async fn experimental_refactor(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("refactor", payload))
}

async fn experimental_test(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("test", payload))
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

async fn list_tool_ids() -> Json<Vec<String>> {
    Json(vec![
        "read".to_string(),
        "write".to_string(),
        "edit".to_string(),
        "bash".to_string(),
        "glob".to_string(),
        "grep".to_string(),
        "ls".to_string(),
        "webfetch".to_string(),
        "websearch".to_string(),
        "task".to_string(),
        "lsp".to_string(),
        "batch".to_string(),
        "plan_enter".to_string(),
        "plan_exit".to_string(),
        "todoread".to_string(),
        "todowrite".to_string(),
        "codesearch".to_string(),
        "apply_patch".to_string(),
        "skills_list".to_string(),
        "skill_view".to_string(),
        "skill".to_string(),
        "multiedit".to_string(),
    ])
}

async fn list_tools(Query(_params): Query<HashMap<String, String>>) -> Json<Vec<ToolInfo>> {
    Json(vec![
        ToolInfo {
            id: "read".to_string(),
            name: "Read".to_string(),
            description: "Read files".to_string(),
        },
        ToolInfo {
            id: "write".to_string(),
            name: "Write".to_string(),
            description: "Write files".to_string(),
        },
        ToolInfo {
            id: "edit".to_string(),
            name: "Edit".to_string(),
            description: "Edit files".to_string(),
        },
        ToolInfo {
            id: "bash".to_string(),
            name: "Bash".to_string(),
            description: "Execute commands".to_string(),
        },
        ToolInfo {
            id: "skills_list".to_string(),
            name: "Skills List".to_string(),
            description: "List available skills".to_string(),
        },
        ToolInfo {
            id: "skill_view".to_string(),
            name: "Skill View".to_string(),
            description: "Load a skill or one of its supporting files".to_string(),
        },
        ToolInfo {
            id: "skill".to_string(),
            name: "Skill".to_string(),
            description: "Deprecated compatibility wrapper around skill_view".to_string(),
        },
    ])
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
