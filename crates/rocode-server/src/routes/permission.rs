use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use once_cell::sync::Lazy;
use rocode_permission::{
    AskOutcome, Pattern, PermissionEngine, PermissionInfo, PermissionLifetime, Response, TimeInfo,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

use crate::{ApiError, Result, ServerState};

pub(crate) fn permission_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_permissions))
        .route("/{id}/reply", post(reply_permission))
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionRequestInfo {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_lifetimes: Vec<String>,
    pub input: serde_json::Value,
    pub message: String,
}

pub(crate) static PERMISSION_ENGINE: Lazy<Mutex<PermissionEngine>> =
    Lazy::new(|| Mutex::new(PermissionEngine::new()));
static PERMISSION_WAITERS: Lazy<Mutex<HashMap<String, oneshot::Sender<PermissionReply>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
struct PermissionReply {
    reply: String,
    message: Option<String>,
}

fn permission_request_message(request: &rocode_tool::PermissionRequest) -> String {
    request
        .metadata
        .get("description")
        .and_then(|value| value.as_str())
        .or_else(|| {
            request
                .metadata
                .get("question")
                .and_then(|value| value.as_str())
        })
        .or_else(|| {
            request
                .metadata
                .get("command")
                .and_then(|value| value.as_str())
        })
        .map(str::to_string)
        .or_else(|| {
            (!request.patterns.is_empty())
                .then(|| format!("{}: {}", request.permission, request.patterns.join(", ")))
        })
        .unwrap_or_else(|| format!("Permission required: {}", request.permission))
}

fn permission_request_info(info: &PermissionInfo) -> PermissionRequestInfo {
    PermissionRequestInfo {
        id: info.id.clone(),
        session_id: info.session_id.clone(),
        tool: info.permission_type.clone(),
        permission_class: info.permission_class.map(|class| class.as_str().to_string()),
        scope_key: info.scope_key.clone(),
        origin_tool: info.origin_tool.clone(),
        supported_lifetimes: info
            .supported_lifetimes
            .iter()
            .map(|lifetime| lifetime.as_str().to_string())
            .collect(),
        input: serde_json::json!({
            "permission": info.permission_type,
            "permission_class": info.permission_class.map(|class| class.as_str()),
            "scope_key": info.scope_key,
            "origin_tool": info.origin_tool,
            "supported_lifetimes": info.supported_lifetimes.iter().map(|lifetime| lifetime.as_str()).collect::<Vec<_>>(),
            "patterns": match &info.pattern {
                Some(Pattern::Single(pattern)) => serde_json::json!([pattern]),
                Some(Pattern::Multiple(patterns)) => serde_json::json!(patterns),
                None => serde_json::json!([]),
            },
            "metadata": info.metadata,
        }),
        message: info.message.clone(),
    }
}

fn request_pattern(request: &rocode_tool::PermissionRequest) -> Option<Pattern> {
    match request.patterns.as_slice() {
        [] => None,
        [single] => Some(Pattern::Single(single.clone())),
        patterns => Some(Pattern::Multiple(patterns.to_vec())),
    }
}

pub(crate) async fn request_permission(
    state: Arc<ServerState>,
    session_id: String,
    request: rocode_tool::PermissionRequest,
) -> std::result::Result<(), rocode_tool::ToolError> {
    let permission_id = format!("permission_{}", uuid::Uuid::new_v4().simple());
    let info = PermissionInfo {
        id: permission_id.clone(),
        permission_type: request.permission.clone(),
        pattern: request_pattern(&request),
        permission_class: request.permission_class,
        scope_key: request.scope_key.clone(),
        origin_tool: request.origin_tool.clone(),
        supported_lifetimes: if request.supported_lifetimes.is_empty() {
            vec![PermissionLifetime::Once, PermissionLifetime::Session]
        } else {
            request.supported_lifetimes.clone()
        },
        session_id: session_id.clone(),
        message_id: String::new(),
        call_id: None,
        message: permission_request_message(&request),
        metadata: request.metadata.clone(),
        time: TimeInfo {
            created: chrono::Utc::now().timestamp_millis().max(0) as u64,
        },
    };

    {
        let mut engine = PERMISSION_ENGINE.lock().await;
        match engine.ask(info.clone()).await {
            Ok(AskOutcome::Granted) => return Ok(()),
            Ok(AskOutcome::Pending) => {}
            Err(_) => {
                return Err(rocode_tool::ToolError::PermissionDenied(format!(
                    "Permission rejected for {}",
                    request.permission
                )));
            }
        }
    }

    let request_info = permission_request_info(&info);
    let (tx, rx) = oneshot::channel();

    PERMISSION_WAITERS
        .lock()
        .await
        .insert(permission_id.clone(), tx);

    // Update aggregated runtime state: pending permission.
    state
        .runtime_telemetry
        .permission_requested(
            &session_id,
            &permission_id,
            serde_json::to_value(&request_info).unwrap_or(serde_json::Value::Null),
        )
        .await;

    let wait_result = tokio::time::timeout(std::time::Duration::from_secs(300), rx).await;
    PERMISSION_WAITERS.lock().await.remove(&permission_id);

    // Clear pending permission from aggregated runtime state.
    state
        .runtime_telemetry
        .clear_permission_pending(&session_id)
        .await;

    match wait_result {
        Ok(Ok(PermissionReply { reply, message })) => match reply.as_str() {
            "once" | "turn" | "session" | "always" => Ok(()),
            "reject" => Err(rocode_tool::ToolError::PermissionDenied(
                message
                    .unwrap_or_else(|| format!("Permission rejected for {}", request.permission)),
            )),
            other => Err(rocode_tool::ToolError::ExecutionError(format!(
                "Invalid permission reply: {}",
                other
            ))),
        },
        Ok(Err(_)) => {
            PERMISSION_ENGINE
                .lock()
                .await
                .remove_pending(&permission_id);
            Err(rocode_tool::ToolError::ExecutionError(
                "Permission response channel closed".to_string(),
            ))
        }
        Err(_) => {
            PERMISSION_ENGINE
                .lock()
                .await
                .remove_pending(&permission_id);
            Err(rocode_tool::ToolError::PermissionDenied(
                "Permission request timed out".to_string(),
            ))
        }
    }
}

async fn list_permissions() -> Json<Vec<PermissionRequestInfo>> {
    let engine = PERMISSION_ENGINE.lock().await;
    let mut result: Vec<_> = engine
        .list()
        .into_iter()
        .map(permission_request_info)
        .collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct ReplyPermissionRequest {
    pub reply: String,
    pub message: Option<String>,
}

pub(crate) async fn reply_permission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ReplyPermissionRequest>,
) -> Result<Json<bool>> {
    let response = match req.reply.as_str() {
        "once" => Response::Once,
        "turn" => Response::Turn,
        "session" | "always" => Response::Always,
        "reject" => Response::Reject,
        _ => {
            return Err(ApiError::BadRequest(
                "Invalid reply; expected `once`, `turn`, `session`, or `reject`".to_string(),
            ));
        }
    };

    let permission = PERMISSION_ENGINE
        .lock()
        .await
        .respond_by_id(&id, response)
        .map_err(|_| ApiError::NotFound(format!("Permission request not found: {}", id)))?;

    if let Some(waiter) = PERMISSION_WAITERS.lock().await.remove(&id) {
        let _ = waiter.send(PermissionReply {
            reply: req.reply.clone(),
            message: req.message.clone(),
        });
    }

    state
        .runtime_telemetry
        .permission_resolved(&permission.session_id, &id, &req.reply, req.message.clone())
        .await;
    Ok(Json(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::extract::State;
    use axum::Json;

    static TEST_PERMISSION_LOCK: Lazy<tokio::sync::Mutex<()>> =
        Lazy::new(|| tokio::sync::Mutex::new(()));

    #[tokio::test]
    async fn request_permission_emits_requested_and_resolved_events() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        PERMISSION_ENGINE.lock().await.clear_session("session-1");
        let state = Arc::new(ServerState::new());
        let mut rx = state.event_bus.subscribe();

        let state_for_request = state.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                "session-1".to_string(),
                rocode_tool::PermissionRequest::new("bash")
                    .with_pattern("cargo test")
                    .with_metadata("command", serde_json::json!("cargo test")),
            )
            .await
        });

        let permission_id = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(id) = engine.list().first().map(|info| info.id.clone()) {
                break id;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        let requested = rx.recv().await.expect("requested event");
        let requested_json: serde_json::Value =
            serde_json::from_str(&requested).expect("requested json");
        assert_eq!(requested_json["type"], "permission.requested");
        assert_eq!(requested_json["permissionID"], permission_id);
        assert_eq!(requested_json["sessionID"], "session-1");

        let reply = ReplyPermissionRequest {
            reply: "once".to_string(),
            message: Some("approved".to_string()),
        };
        let _ = reply_permission(
            State(state.clone()),
            Path(permission_id.clone()),
            Json(reply),
        )
        .await
        .expect("reply should succeed");

        let resolved = rx.recv().await.expect("resolved event");
        let resolved_json: serde_json::Value =
            serde_json::from_str(&resolved).expect("resolved json");
        assert_eq!(resolved_json["type"], "permission.resolved");
        assert_eq!(resolved_json["permissionID"], permission_id);
        assert_eq!(resolved_json["reply"], "once");

        request_task
            .await
            .expect("request task join")
            .expect("permission allowed");
        PERMISSION_ENGINE.lock().await.clear_session("session-1");
    }

    #[tokio::test]
    async fn reply_permission_session_remembers_future_requests() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        const SESSION_ID: &str = "session-grant";
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);

        let state = Arc::new(ServerState::new());
        let state_for_request = state.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                SESSION_ID.to_string(),
                rocode_tool::PermissionRequest::new("bash")
                    .with_pattern("cargo test")
                    .with_metadata("command", serde_json::json!("cargo test"))
                    .with_supported_lifetimes(vec![
                        PermissionLifetime::Once,
                        PermissionLifetime::Turn,
                        PermissionLifetime::Session,
                    ]),
            )
            .await
        });

        let permission_id = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(id) = engine.list().first().map(|info| info.id.clone()) {
                break id;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        let _ = reply_permission(
            State(state.clone()),
            Path(permission_id),
            Json(ReplyPermissionRequest {
                reply: "session".to_string(),
                message: None,
            }),
        )
        .await
        .expect("reply should succeed");

        request_task
            .await
            .expect("request task join")
            .expect("permission allowed");

        request_permission(
            state,
            SESSION_ID.to_string(),
            rocode_tool::PermissionRequest::new("bash")
                .with_pattern("cargo test")
                .with_metadata("command", serde_json::json!("cargo test"))
                .with_supported_lifetimes(vec![
                    PermissionLifetime::Once,
                    PermissionLifetime::Turn,
                    PermissionLifetime::Session,
                ]),
        )
        .await
        .expect("repeat request should be auto-approved");

        assert!(PERMISSION_ENGINE.lock().await.list().is_empty());
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);
    }

    #[tokio::test]
    async fn request_permission_always_hint_does_not_auto_approve() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        const SESSION_ID: &str = "session-always-hint";
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);

        let state = Arc::new(ServerState::new());
        let state_for_request = state.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                SESSION_ID.to_string(),
                rocode_tool::PermissionRequest::new("bash")
                    .with_pattern("cargo test")
                    .with_always("cargo *"),
            )
            .await
        });

        let permission_id = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(id) = engine.list().first().map(|info| info.id.clone()) {
                break id;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        assert!(
            !request_task.is_finished(),
            "always hints must wait for an explicit permission reply"
        );

        let _ = reply_permission(
            State(state.clone()),
            Path(permission_id),
            Json(ReplyPermissionRequest {
                reply: "once".to_string(),
                message: None,
            }),
        )
        .await
        .expect("reply should succeed");

        request_task
            .await
            .expect("request task join")
            .expect("permission allowed");
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);
    }

    #[tokio::test]
    async fn inspect_read_permission_is_auto_granted_without_pending_prompt() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        const SESSION_ID: &str = "session-inspect-read";
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);

        let state = Arc::new(ServerState::new());
        request_permission(
            state,
            SESSION_ID.to_string(),
            rocode_tool::PermissionRequest::new("read").with_pattern("src/lib.rs"),
        )
        .await
        .expect("inspect_read should be auto-approved");

        assert!(PERMISSION_ENGINE.lock().await.list().is_empty());
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);
    }
}
