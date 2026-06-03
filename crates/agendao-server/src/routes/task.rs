use axum::{extract::Path, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use crate::{ApiError, Result, ServerState};
use agendao_core::agent_task_registry::{global_task_registry, AgentTaskStatus};

#[derive(Debug, Serialize)]
struct TaskSummary {
    id: String,
    agent_name: String,
    status: String,
    step: Option<u32>,
    max_steps: Option<u32>,
    prompt: String,
    started_at: i64,
    elapsed_seconds: i64,
}

#[derive(Debug, Serialize)]
struct TaskDetail {
    #[serde(flatten)]
    summary: TaskSummary,
    finished_at: Option<i64>,
    output_tail: Vec<String>,
}

fn status_str(status: &AgentTaskStatus) -> String {
    match status {
        AgentTaskStatus::Pending => "pending".to_string(),
        AgentTaskStatus::Running { .. } => "running".to_string(),
        AgentTaskStatus::Completed { .. } => "completed".to_string(),
        AgentTaskStatus::Cancelled => "cancelled".to_string(),
        AgentTaskStatus::Failed { error } => format!("failed: {}", error),
    }
}

fn current_step(status: &AgentTaskStatus) -> Option<u32> {
    match status {
        AgentTaskStatus::Running { step } => Some(*step),
        AgentTaskStatus::Completed { steps } => Some(*steps),
        _ => None,
    }
}

pub(crate) fn task_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_tasks))
        .route("/{id}", get(get_task).delete(cancel_task))
}

async fn list_tasks() -> Json<Vec<TaskSummary>> {
    let tasks = global_task_registry().list();
    let now = chrono::Utc::now().timestamp();
    Json(
        tasks
            .into_iter()
            .map(|t| TaskSummary {
                id: t.id,
                agent_name: t.agent_name,
                status: status_str(&t.status),
                step: current_step(&t.status),
                max_steps: t.max_steps,
                prompt: t.prompt,
                started_at: t.started_at,
                elapsed_seconds: now - t.started_at,
            })
            .collect(),
    )
}

async fn get_task(Path(id): Path<String>) -> Result<Json<TaskDetail>> {
    let task = global_task_registry()
        .get(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Task \"{}\" not found", id)))?;
    let now = chrono::Utc::now().timestamp();
    Ok(Json(TaskDetail {
        summary: TaskSummary {
            id: task.id,
            agent_name: task.agent_name,
            status: status_str(&task.status),
            step: current_step(&task.status),
            max_steps: task.max_steps,
            prompt: task.prompt,
            started_at: task.started_at,
            elapsed_seconds: now - task.started_at,
        },
        finished_at: task.finished_at,
        output_tail: task.output_tail.into_iter().collect(),
    }))
}

async fn cancel_task(Path(id): Path<String>) -> Result<Json<serde_json::Value>> {
    agendao_orchestrator::global_lifecycle()
        .cancel_task(&id)
        .map_err(ApiError::NotFound)?;
    Ok(Json(serde_json::json!({ "cancelled": id })))
}
