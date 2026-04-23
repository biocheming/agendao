use axum::{
    extract::{Path, State},
    routing::{get, patch},
    Json, Router,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{ApiError, Result, ServerState};

pub(crate) fn project_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_projects))
        .route("/current", get(get_current_project))
        .route("/{id}", patch(update_project))
}

#[derive(Debug, Serialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub vcs: bool,
}

#[derive(Debug, Clone, Default)]
struct ProjectMetadata {
    name: Option<String>,
    icon: Option<String>,
}

static PROJECT_METADATA: Lazy<RwLock<HashMap<String, ProjectMetadata>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn is_git_repository(path: &FsPath) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim() == "true",
        _ => false,
    }
}

async fn current_project_info(state: &ServerState) -> Result<ProjectInfo> {
    let canonical = state
        .project_root()
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve project path: {}", e)))?;
    let project_id = canonical.to_string_lossy().to_string();
    let project_path = canonical.to_string_lossy().to_string();
    let default_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".to_string());

    let metadata = PROJECT_METADATA.read().await;
    let name = metadata
        .get(&project_id)
        .and_then(|m| m.name.clone())
        .unwrap_or(default_name);
    let _icon = metadata.get(&project_id).and_then(|m| m.icon.clone());

    Ok(ProjectInfo {
        id: project_id,
        name,
        path: project_path,
        vcs: is_git_repository(&canonical),
    })
}

async fn list_projects(State(state): State<Arc<ServerState>>) -> Result<Json<Vec<ProjectInfo>>> {
    Ok(Json(vec![current_project_info(state.as_ref()).await?]))
}

async fn get_current_project(State(state): State<Arc<ServerState>>) -> Result<Json<ProjectInfo>> {
    Ok(Json(current_project_info(state.as_ref()).await?))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub icon: Option<String>,
}

async fn update_project(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectInfo>> {
    let current = current_project_info(state.as_ref()).await?;
    if id != current.id {
        return Err(ApiError::NotFound(format!("Project not found: {}", id)));
    }

    let mut metadata = PROJECT_METADATA.write().await;
    let entry = metadata.entry(id).or_default();
    if let Some(name) = req.name {
        entry.name = Some(name);
    }
    if let Some(icon) = req.icon {
        entry.icon = Some(icon);
    }
    drop(metadata);

    Ok(Json(current_project_info(state.as_ref()).await?))
}
