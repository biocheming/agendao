use std::sync::Arc;

use agendao_permission::PermissionLifetime;
use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;

use super::permission::request_permission;
use crate::session_runtime::events::broadcast_server_event;
use crate::session_runtime::question::request_question_answers;
use crate::ServerState;
use agendao_server_core::runtime_control::SessionRunStatus;
use agendao_server_core::runtime_events::ServerEvent;

pub(crate) fn frontend_smoke_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/question", post(frontend_smoke_question))
        .route("/permission", post(frontend_smoke_permission))
        .route("/output-block", post(frontend_smoke_output_block))
}

#[derive(Debug, Deserialize)]
struct FrontendSmokeQuestionRequest {
    session_id: String,
    questions: Vec<agendao_tool::QuestionDef>,
}

#[derive(Debug, Deserialize)]
struct FrontendSmokePermissionRequest {
    session_id: String,
    permission: String,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    always: Vec<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    filepath: Option<String>,
    #[serde(default)]
    supported_lifetimes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FrontendSmokeOutputBlockRequest {
    session_id: String,
    block: serde_json::Value,
    #[serde(default)]
    id: Option<String>,
}

async fn frontend_smoke_question(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<FrontendSmokeQuestionRequest>,
) -> Json<bool> {
    let state = state.clone();
    tokio::spawn(async move {
        let session_id = req.session_id;
        let _ = request_question_answers(
            state.clone(),
            session_id.clone(),
            req.questions,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        finish_smoke_interaction(&state, &session_id).await;
    });
    Json(true)
}

async fn frontend_smoke_permission(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<FrontendSmokePermissionRequest>,
) -> Json<bool> {
    let supported_lifetimes = req
        .supported_lifetimes
        .into_iter()
        .filter_map(|value| match value.as_str() {
            "once" => Some(PermissionLifetime::Once),
            "turn" => Some(PermissionLifetime::Turn),
            "session" | "always" => Some(PermissionLifetime::Session),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut request = agendao_tool::PermissionRequest::new(req.permission)
        .with_patterns(req.patterns)
        .with_supported_lifetimes(supported_lifetimes);
    for always in req.always {
        request = request.with_always(always);
    }
    if let Some(description) = req.description {
        request = request.with_metadata("description", serde_json::json!(description));
    }
    if let Some(command) = req.command {
        request = request.with_metadata("command", serde_json::json!(command));
    }
    if let Some(filepath) = req.filepath {
        request = request.with_metadata("filepath", serde_json::json!(filepath));
    }

    let state = state.clone();
    tokio::spawn(async move {
        let session_id = req.session_id;
        let _ = request_permission(state.clone(), session_id.clone(), request).await;
        finish_smoke_interaction(&state, &session_id).await;
    });
    Json(true)
}

async fn finish_smoke_interaction(state: &Arc<ServerState>, session_id: &str) {
    state
        .runtime_telemetry
        .set_session_run_status(session_id, SessionRunStatus::Idle)
        .await;
    broadcast_server_event(
        state,
        &ServerEvent::SessionStatus {
            session_id: session_id.to_string(),
            status: serde_json::to_value(SessionRunStatus::Idle).unwrap_or(serde_json::Value::Null),
        },
    );
}

async fn frontend_smoke_output_block(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<FrontendSmokeOutputBlockRequest>,
) -> Json<bool> {
    broadcast_server_event(
        state.as_ref(),
        &ServerEvent::OutputBlock {
            session_id: req.session_id,
            block: req.block,
            id: req.id,
            live_identity: None,
        },
    );
    Json(true)
}
