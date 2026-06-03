use axum::{
    extract::Path,
    routing::{delete, get},
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;

use crate::{ApiError, Result, ServerState};
use agendao_core::process_registry::{global_registry, ProcessKind};

#[derive(Debug, Serialize)]
struct ProcessResponse {
    pid: u32,
    name: String,
    kind: String,
    started_at: i64,
    cpu_percent: f32,
    memory_kb: u64,
}

fn kind_to_str(kind: ProcessKind) -> &'static str {
    match kind {
        ProcessKind::Plugin => "plugin",
        ProcessKind::Bash => "bash",
        ProcessKind::Agent => "agent",
        ProcessKind::Mcp => "mcp",
        ProcessKind::Lsp => "lsp",
    }
}

pub(crate) fn process_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_processes))
        .route("/{pid}", delete(kill_process))
}

async fn list_processes() -> Json<Vec<ProcessResponse>> {
    global_registry().refresh_stats();
    let procs = global_registry().list();
    Json(
        procs
            .into_iter()
            .map(|p| ProcessResponse {
                pid: p.pid,
                name: p.name,
                kind: kind_to_str(p.kind).to_string(),
                started_at: p.started_at,
                cpu_percent: p.cpu_percent,
                memory_kb: p.memory_kb,
            })
            .collect(),
    )
}

async fn kill_process(Path(pid): Path<u32>) -> Result<Json<serde_json::Value>> {
    agendao_orchestrator::global_lifecycle()
        .kill_process(pid)
        .map_err(|e| {
            ApiError::NotFound(format!(
                "Process {} not found or cannot be killed: {}",
                pid, e
            ))
        })?;
    Ok(Json(serde_json::json!({ "killed": pid })))
}
