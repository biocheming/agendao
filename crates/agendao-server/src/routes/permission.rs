use agendao_permission::{
    AskOutcome, Pattern, PermissionEngine, PermissionInfo, PermissionLifetime,
    PermissionMatcherKind, Response, TimeInfo,
};
use agendao_tool::default_supported_lifetimes_for_class;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use once_cell::sync::Lazy;
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
    pub scope_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_lifetimes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_target_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_tags: Vec<String>,
    pub input: serde_json::Value,
    pub message: String,
}

pub(crate) static PERMISSION_ENGINE: Lazy<Mutex<PermissionEngine>> =
    Lazy::new(|| Mutex::new(PermissionEngine::new()));
static PERMISSION_WAITERS: Lazy<Mutex<HashMap<String, oneshot::Sender<PermissionReply>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

const PERMISSION_GRANTED_BY_TURN_COUNT_METADATA_KEY: &str = "permission_granted_by_turn_count";
const PERMISSION_GRANTED_BY_SESSION_COUNT_METADATA_KEY: &str =
    "permission_granted_by_session_count";
const PERMISSION_GRANTED_BY_MATCHER_KIND_METADATA_KEY: &str = "permission_granted_by_matcher_kind";
const LAST_PERMISSION_MATCHER_KIND_METADATA_KEY: &str = "last_permission_matcher_kind";
const LAST_PERMISSION_GRANT_TARGET_METADATA_KEY: &str = "last_permission_grant_target";

#[derive(Debug)]
struct PermissionReply {
    reply: String,
    message: Option<String>,
}

fn permission_request_message(request: &agendao_tool::PermissionRequest) -> String {
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

fn permission_scope_label(scope_key: Option<&str>) -> Option<String> {
    let scope_key = scope_key?.trim();
    if scope_key.is_empty() {
        return None;
    }

    if let Some(rest) = scope_key.strip_prefix("cmd:") {
        let families = rest
            .split('+')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if families.is_empty() {
            return Some("Shell command family".to_string());
        }
        return Some(format!("Shell commands: {}", families.join(", ")));
    }

    if let Some(rest) = scope_key.strip_prefix("task:agent:") {
        return Some(format!("Task agent: {rest}"));
    }
    if let Some(rest) = scope_key.strip_prefix("task:category:") {
        return Some(format!("Task category: {rest}"));
    }
    if let Some(rest) = scope_key.strip_prefix("task_flow:") {
        let label = match rest {
            "create" => "create task",
            "resume" => "resume task",
            "get" => "inspect task",
            "list" => "list tasks",
            "cancel" => "cancel task",
            _ => rest,
        };
        return Some(format!("Task flow: {label}"));
    }
    if let Some(rest) = scope_key.strip_prefix("shell_session:") {
        let label = match rest {
            "start" => "start session",
            "write" => "send input",
            "read" => "read output",
            "status" => "inspect session",
            "terminate" => "terminate session",
            _ => rest,
        };
        return Some(format!("Shell session: {label}"));
    }
    if let Some(rest) = scope_key.strip_prefix("workspace:batch:") {
        let count = rest
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .count();
        return Some(format!("Workspace batch edit ({count} files)"));
    }
    if let Some(rest) = scope_key.strip_prefix("workspace:/") {
        return Some(format!("Workspace path: /{rest}"));
    }
    if scope_key == "workspace:/" {
        return Some("Workspace root".to_string());
    }
    if let Some(rest) = scope_key.strip_prefix("workspace:") {
        return Some(format!("Workspace path: {rest}"));
    }
    if let Some(rest) = scope_key.strip_prefix("fs:") {
        return Some(format!("External path: {rest}"));
    }
    if let Some(rest) = scope_key.strip_prefix("net:") {
        if rest == "search" {
            return Some("Web search".to_string());
        }
        return Some(format!("Network host: {rest}"));
    }

    Some(scope_key.to_string())
}

fn permission_matcher_label(
    matcher_kind: Option<PermissionMatcherKind>,
    matcher_key: Option<&str>,
) -> Option<String> {
    let matcher_key = matcher_key?.trim();
    if matcher_key.is_empty() {
        return None;
    }

    match matcher_kind {
        Some(PermissionMatcherKind::ScopeOnly) => None,
        Some(PermissionMatcherKind::ExactInput) => Some(format!("Exact input: {matcher_key}")),
        Some(PermissionMatcherKind::StructuredFamily) => {
            let label = matcher_key
                .strip_prefix("cmd:")
                .unwrap_or(matcher_key)
                .replace('+', ", ");
            Some(format!("Command family: {label}"))
        }
        Some(PermissionMatcherKind::SemanticPattern) => {
            Some(format!("Semantic pattern: {matcher_key}"))
        }
        None => None,
    }
}

fn permission_grant_target_summary(info: &PermissionInfo) -> Option<String> {
    permission_scope_label(info.scope_key.as_deref())
        .or_else(|| permission_matcher_label(info.matcher_kind, info.matcher_key.as_deref()))
}

fn permission_matcher_kind_key(info: &PermissionInfo) -> String {
    info.matcher_kind
        .map(|kind| kind.as_str().to_string())
        .unwrap_or_else(|| "legacy".to_string())
}

fn increment_session_metadata_counter(session: &mut agendao_session::Session, key: &str) {
    let next = session
        .record()
        .metadata
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
        .saturating_add(1);
    session.insert_metadata(key.to_string(), serde_json::json!(next));
}

fn increment_session_metadata_map_counter(
    session: &mut agendao_session::Session,
    key: &str,
    entry_key: &str,
) {
    let mut object = session
        .record()
        .metadata
        .get(key)
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let next = object
        .get(entry_key)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
        .saturating_add(1);
    object.insert(entry_key.to_string(), serde_json::json!(next));
    session.insert_metadata(key.to_string(), serde_json::Value::Object(object));
}

fn record_permission_pending_duration(
    session: &mut agendao_session::Session,
    requested_at_ms: u64,
) {
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    session.insert_metadata(
        "last_permission_pending_ms".to_string(),
        serde_json::json!(now.saturating_sub(requested_at_ms)),
    );
}

fn record_permission_grant_telemetry(
    session: &mut agendao_session::Session,
    permission: &PermissionInfo,
    response: Response,
) {
    let matcher_kind_key = permission_matcher_kind_key(permission);
    increment_session_metadata_map_counter(
        session,
        PERMISSION_GRANTED_BY_MATCHER_KIND_METADATA_KEY,
        &matcher_kind_key,
    );
    session.insert_metadata(
        LAST_PERMISSION_MATCHER_KIND_METADATA_KEY.to_string(),
        serde_json::json!(matcher_kind_key),
    );
    if let Some(target) = permission_grant_target_summary(permission) {
        session.insert_metadata(
            LAST_PERMISSION_GRANT_TARGET_METADATA_KEY.to_string(),
            serde_json::json!(target),
        );
    } else {
        session.insert_metadata(
            LAST_PERMISSION_GRANT_TARGET_METADATA_KEY.to_string(),
            serde_json::Value::Null,
        );
    }
    match response {
        Response::Turn => increment_session_metadata_counter(
            session,
            PERMISSION_GRANTED_BY_TURN_COUNT_METADATA_KEY,
        ),
        Response::Always => increment_session_metadata_counter(
            session,
            PERMISSION_GRANTED_BY_SESSION_COUNT_METADATA_KEY,
        ),
        Response::Once | Response::Reject => {}
    }
}

fn permission_request_info(info: &PermissionInfo) -> PermissionRequestInfo {
    PermissionRequestInfo {
        id: info.id.clone(),
        session_id: info.session_id.clone(),
        tool: info.permission_type.clone(),
        permission_class: info
            .permission_class
            .map(|class| class.as_str().to_string()),
        scope_key: info.scope_key.clone(),
        scope_label: permission_scope_label(info.scope_key.as_deref()),
        origin_tool: info.origin_tool.clone(),
        supported_lifetimes: info
            .supported_lifetimes
            .iter()
            .map(|lifetime| lifetime.as_str().to_string())
            .collect(),
        matcher_kind: info.matcher_kind.map(|kind| kind.as_str().to_string()),
        matcher_key: info.matcher_key.clone(),
        matcher_label: permission_matcher_label(info.matcher_kind, info.matcher_key.as_deref()),
        grant_target_summary: permission_grant_target_summary(info),
        risk_tags: info.risk_tags.clone(),
        input: serde_json::json!({
            "permission": info.permission_type,
            "permission_class": info.permission_class.map(|class| class.as_str()),
            "scope_key": info.scope_key,
            "origin_tool": info.origin_tool,
            "supported_lifetimes": info.supported_lifetimes.iter().map(|lifetime| lifetime.as_str()).collect::<Vec<_>>(),
            "matcher_kind": info.matcher_kind.map(|kind| kind.as_str()),
            "matcher_key": info.matcher_key,
            "risk_tags": info.risk_tags,
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

fn request_pattern(request: &agendao_tool::PermissionRequest) -> Option<Pattern> {
    match request.patterns.as_slice() {
        [] => None,
        [single] => Some(Pattern::Single(single.clone())),
        patterns => Some(Pattern::Multiple(patterns.to_vec())),
    }
}

pub(crate) async fn request_permission(
    state: Arc<ServerState>,
    session_id: String,
    request: agendao_tool::PermissionRequest,
) -> std::result::Result<(), agendao_tool::ToolError> {
    let permission_id = format!("permission_{}", uuid::Uuid::new_v4().simple());
    let info = PermissionInfo {
        id: permission_id.clone(),
        permission_type: request.permission.clone(),
        pattern: request_pattern(&request),
        permission_class: request.permission_class,
        scope_key: request.scope_key.clone(),
        matcher_kind: request.matcher_kind,
        matcher_key: request.matcher_key.clone(),
        origin_tool: request.origin_tool.clone(),
        risk_tags: request.risk_tags.clone(),
        supported_lifetimes: if request.supported_lifetimes.is_empty() {
            request
                .permission_class
                .map(default_supported_lifetimes_for_class)
                .unwrap_or_else(|| vec![PermissionLifetime::Once])
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
            Ok(AskOutcome::Pending) => {
                // Matcher miss: no existing grant covered this request, so it entered pending.
                // Increment miss_count for the telemetry read model to explain WHY.
                let mut sessions = state.sessions.lock().await;
                if let Some(session) = sessions.get_mut(&session_id) {
                    let current = session
                        .record()
                        .metadata
                        .get("last_permission_miss_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    session.insert_metadata(
                        "last_permission_miss_count".to_string(),
                        serde_json::json!(current + 1),
                    );
                }
            }
            Err(_) => {
                return Err(agendao_tool::ToolError::PermissionDenied(format!(
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

    // P3: Broadcast PermissionRequested to canonical event bus
    let requested_event = agendao_server_core::runtime_events::ServerEvent::PermissionRequested {
        session_id: session_id.clone(),
        permission_id: permission_id.clone(),
        info: serde_json::to_value(&request_info).unwrap_or(serde_json::Value::Null),
    };
    crate::session_runtime::events::broadcast_server_event(&state, &requested_event);

    // Push session.updated so frontend refresh path triggers for pending state.
    crate::session_runtime::events::broadcast_session_reconcile(
        &state,
        &session_id,
        crate::session_runtime::events::ReconcileReason::Permission,
    );

    let wait_result = tokio::time::timeout(std::time::Duration::from_secs(300), rx).await;
    PERMISSION_WAITERS.lock().await.remove(&permission_id);

    // Clear pending permission from aggregated runtime state.
    state
        .runtime_telemetry
        .clear_permission_pending(&session_id)
        .await;

    match wait_result {
        Ok(Ok(PermissionReply { reply, message })) => {
            // P3: Broadcast PermissionResolved to canonical event bus
            let resolved_event = agendao_server_core::runtime_events::ServerEvent::PermissionResolved {
                session_id: session_id.clone(),
                permission_id: permission_id.clone(),
                reply: reply.clone(),
                message: message.clone(),
            };
            crate::session_runtime::events::broadcast_server_event(&state, &resolved_event);

            match reply.as_str() {
                "once" | "turn" | "session" | "always" => Ok(()),
                "reject" => Err(agendao_tool::ToolError::PermissionDenied(
                    message
                        .unwrap_or_else(|| format!("Permission rejected for {}", request.permission)),
                )),
                other => Err(agendao_tool::ToolError::ExecutionError(format!(
                    "Invalid permission reply: {}",
                    other
                ))),
            }
        }
        Ok(Err(_)) => {
            let mut sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                record_permission_pending_duration(session, info.time.created);
            }
            drop(sessions);
            PERMISSION_ENGINE
                .lock()
                .await
                .remove_pending(&permission_id);
            // Pending cleared — broadcast so frontend doesn't see stale pending.
            crate::session_runtime::events::broadcast_session_reconcile(
                &state,
                &session_id,
                crate::session_runtime::events::ReconcileReason::Permission,
            );
            Err(agendao_tool::ToolError::ExecutionError(
                "Permission response channel closed".to_string(),
            ))
        }
        Err(_) => {
            let mut sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                record_permission_pending_duration(session, info.time.created);
            }
            drop(sessions);
            PERMISSION_ENGINE
                .lock()
                .await
                .remove_pending(&permission_id);
            // Pending cleared — broadcast so frontend doesn't see stale pending.
            crate::session_runtime::events::broadcast_session_reconcile(
                &state,
                &session_id,
                crate::session_runtime::events::ReconcileReason::Permission,
            );
            Err(agendao_tool::ToolError::PermissionDenied(
                "Permission request timed out".to_string(),
            ))
        }
    }
}

pub(crate) async fn list_permissions() -> Json<Vec<PermissionRequestInfo>> {
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

    let session_id = permission.session_id.clone();
    let was_grant = response != Response::Reject;

    if was_grant {
        let mut sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            let matcher_kind = permission_matcher_kind_key(&permission);
            record_permission_grant_telemetry(session, &permission, response);
            record_permission_pending_duration(session, permission.time.created);
            // Store structured hit reason for telemetry read model (plan §8 path C).
            session.insert_metadata(
                "last_permission_hit_matcher_kind".to_string(),
                serde_json::json!(matcher_kind),
            );
        }
    } else {
        let mut sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            record_permission_pending_duration(session, permission.time.created);
        }
    }
    // Note: miss_count is incremented in request_permission() when AskOutcome::Pending —
    // that is the true "matcher miss" (no grant covered this request). Reject is a user
    // decision, not a matcher miss, so we do NOT increment miss_count here.

    if let Some(waiter) = PERMISSION_WAITERS.lock().await.remove(&id) {
        let _ = waiter.send(PermissionReply {
            reply: req.reply.clone(),
            message: req.message.clone(),
        });
    }

    state
        .runtime_telemetry
        .permission_resolved(&session_id, &id, &req.reply, req.message.clone())
        .await;

    // Push session.updated so frontend refresh path triggers (plan §8 path D).
    crate::session_runtime::events::broadcast_session_reconcile(
        &state,
        &session_id,
        crate::session_runtime::events::ReconcileReason::Permission,
    );
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
                agendao_tool::PermissionRequest::new("bash")
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

        let mut saw_requested = false;
        let mut saw_control_queued = false;
        let mut saw_pending_updated = false;
        for _ in 0..4 {
            let raw = rx.recv().await.expect("pending lifecycle event");
            let json: serde_json::Value =
                serde_json::from_str(&raw).expect("pending lifecycle json");
            match json["type"].as_str() {
                Some("permission.requested") => {
                    assert_eq!(json["permissionID"], permission_id);
                    assert_eq!(json["sessionID"], "session-1");
                    saw_requested = true;
                }
                Some("control_input.transition") if json["kind"] == "permission" => {
                    assert_eq!(json["phase"], "queued");
                    saw_control_queued = true;
                }
                Some("session.updated") => {
                    saw_pending_updated = true;
                }
                _ => {}
            }
            if saw_requested && saw_control_queued && saw_pending_updated {
                break;
            }
        }
        assert!(saw_requested, "missing permission.requested event");
        assert!(
            saw_control_queued,
            "missing permission queued control event"
        );
        assert!(
            saw_pending_updated,
            "missing pending session.updated reconcile"
        );

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

        let mut saw_consumed = false;
        let mut saw_cleared = false;
        let mut saw_resolved = false;
        let mut saw_resolved_updated = false;
        for _ in 0..6 {
            let raw = rx.recv().await.expect("resolved lifecycle event");
            let json: serde_json::Value =
                serde_json::from_str(&raw).expect("resolved lifecycle json");
            match json["type"].as_str() {
                Some("permission.resolved") => {
                    assert_eq!(json["permissionID"], permission_id);
                    assert_eq!(json["reply"], "once");
                    saw_resolved = true;
                }
                Some("control_input.transition") if json["kind"] == "permission" => {
                    match json["phase"].as_str() {
                        Some("consumed") => saw_consumed = true,
                        Some("cleared") => saw_cleared = true,
                        _ => {}
                    }
                }
                Some("session.updated") => {
                    saw_resolved_updated = true;
                }
                _ => {}
            }
            if saw_consumed && saw_cleared && saw_resolved && saw_resolved_updated {
                break;
            }
        }
        assert!(saw_consumed, "missing permission consumed control event");
        assert!(saw_cleared, "missing permission cleared control event");
        assert!(saw_resolved, "missing permission.resolved event");
        assert!(
            saw_resolved_updated,
            "missing resolved session.updated reconcile"
        );

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
                agendao_tool::PermissionRequest::new("bash")
                    .with_pattern("cargo test")
                    .with_scope_key("cmd:cargo *")
                    .with_matcher(PermissionMatcherKind::StructuredFamily, "cmd:cargo *")
                    .with_risk_tag("dangerous_exec")
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
            state.clone(),
            SESSION_ID.to_string(),
            agendao_tool::PermissionRequest::new("bash")
                .with_pattern("cargo check")
                .with_scope_key("cmd:cargo *")
                .with_matcher(PermissionMatcherKind::StructuredFamily, "cmd:cargo *")
                .with_risk_tag("dangerous_exec")
                .with_metadata("command", serde_json::json!("cargo check"))
                .with_supported_lifetimes(vec![
                    PermissionLifetime::Once,
                    PermissionLifetime::Turn,
                    PermissionLifetime::Session,
                ]),
        )
        .await
        .expect("repeat request should be auto-approved");

        assert!(PERMISSION_ENGINE.lock().await.list().is_empty());

        let state_for_request = state.clone();
        let different_family_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                SESSION_ID.to_string(),
                agendao_tool::PermissionRequest::new("bash")
                    .with_pattern("git status")
                    .with_scope_key("cmd:git *")
                    .with_matcher(PermissionMatcherKind::StructuredFamily, "cmd:git *")
                    .with_risk_tag("dangerous_exec")
                    .with_metadata("command", serde_json::json!("git status"))
                    .with_supported_lifetimes(vec![
                        PermissionLifetime::Once,
                        PermissionLifetime::Turn,
                        PermissionLifetime::Session,
                    ]),
            )
            .await
        });

        let pending = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(info) = engine.list().first().cloned().cloned() {
                break info;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };
        assert_eq!(pending.matcher_key.as_deref(), Some("cmd:git *"));

        let _ = reply_permission(
            State(state),
            Path(pending.id.clone()),
            Json(ReplyPermissionRequest {
                reply: "once".to_string(),
                message: None,
            }),
        )
        .await
        .expect("reply should succeed");

        let result = different_family_task.await.expect("join different family");
        result.expect("different family should still resolve after explicit approval");
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);
    }

    #[tokio::test]
    async fn request_permission_exposes_structured_authority_fields() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        const SESSION_ID: &str = "session-structured-fields";
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);

        let state = Arc::new(ServerState::new());
        let state_for_request = state.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                SESSION_ID.to_string(),
                agendao_tool::PermissionRequest::new("bash")
                    .with_pattern("cargo test")
                    .with_scope_key("cmd:cargo *")
                    .with_matcher(PermissionMatcherKind::StructuredFamily, "cmd:cargo *")
                    .with_risk_tag("dangerous_exec")
                    .with_metadata("command", serde_json::json!("cargo test"))
                    .with_supported_lifetimes(vec![
                        PermissionLifetime::Once,
                        PermissionLifetime::Turn,
                        PermissionLifetime::Session,
                    ]),
            )
            .await
        });

        let permission = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(info) = engine.list().first().cloned().cloned() {
                break info;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        let rendered = permission_request_info(&permission);
        assert_eq!(rendered.matcher_kind.as_deref(), Some("structured_family"));
        assert_eq!(rendered.matcher_key.as_deref(), Some("cmd:cargo *"));
        assert_eq!(
            rendered.matcher_label.as_deref(),
            Some("Command family: cargo *")
        );
        assert_eq!(
            rendered.grant_target_summary.as_deref(),
            Some("Shell commands: cargo *")
        );
        assert_eq!(rendered.risk_tags, vec!["dangerous_exec".to_string()]);

        let _ = reply_permission(
            State(state),
            Path(permission.id.clone()),
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
    async fn request_permission_exposes_scope_only_authority_fields() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        const SESSION_ID: &str = "session-scope-only-fields";
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);

        let state = Arc::new(ServerState::new());
        let state_for_request = state.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                SESSION_ID.to_string(),
                agendao_tool::PermissionRequest::new("task_flow")
                    .with_scope_key("task_flow:create")
                    .with_supported_lifetimes(agendao_tool::structured_dangerous_exec_lifetimes())
                    .with_risk_tag("dangerous_exec")
                    .with_metadata("operation", serde_json::json!("create")),
            )
            .await
        });

        let permission = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(info) = engine.list().first().cloned().cloned() {
                break info;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        let rendered = permission_request_info(&permission);
        assert_eq!(rendered.matcher_kind.as_deref(), Some("scope_only"));
        assert_eq!(rendered.matcher_key.as_deref(), Some("task_flow:create"));
        assert_eq!(rendered.matcher_label, None);
        assert_eq!(
            rendered.grant_target_summary.as_deref(),
            Some("Task flow: create task")
        );
        assert_eq!(rendered.risk_tags, vec!["dangerous_exec".to_string()]);

        let _ = reply_permission(
            State(state),
            Path(permission.id.clone()),
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
    async fn reply_permission_records_permission_grant_telemetry_on_owner_session() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };
        PERMISSION_ENGINE.lock().await.clear_session(&session_id);

        let state_for_scope_only = state.clone();
        let session_id_for_scope_only = session_id.clone();
        let scope_only_task = tokio::spawn(async move {
            request_permission(
                state_for_scope_only,
                session_id_for_scope_only,
                agendao_tool::PermissionRequest::new("task_flow")
                    .with_scope_key("task_flow:create")
                    .with_supported_lifetimes(agendao_tool::structured_dangerous_exec_lifetimes())
                    .with_risk_tag("dangerous_exec")
                    .with_metadata("operation", serde_json::json!("create")),
            )
            .await
        });

        let scope_only_permission = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(info) = engine.list().first().cloned().cloned() {
                break info;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        let _ = reply_permission(
            State(state.clone()),
            Path(scope_only_permission.id.clone()),
            Json(ReplyPermissionRequest {
                reply: "turn".to_string(),
                message: None,
            }),
        )
        .await
        .expect("turn reply should succeed");

        scope_only_task
            .await
            .expect("scope-only request task join")
            .expect("scope-only permission allowed");

        let state_for_structured = state.clone();
        let session_id_for_structured = session_id.clone();
        let structured_task = tokio::spawn(async move {
            request_permission(
                state_for_structured,
                session_id_for_structured,
                agendao_tool::PermissionRequest::new("bash")
                    .with_pattern("cargo test")
                    .with_scope_key("cmd:cargo *")
                    .with_matcher(PermissionMatcherKind::StructuredFamily, "cmd:cargo *")
                    .with_risk_tag("dangerous_exec")
                    .with_metadata("command", serde_json::json!("cargo test"))
                    .with_supported_lifetimes(vec![
                        PermissionLifetime::Once,
                        PermissionLifetime::Turn,
                        PermissionLifetime::Session,
                    ]),
            )
            .await
        });

        let structured_permission = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(info) = engine.list().first().cloned().cloned() {
                break info;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        let _ = reply_permission(
            State(state.clone()),
            Path(structured_permission.id.clone()),
            Json(ReplyPermissionRequest {
                reply: "session".to_string(),
                message: None,
            }),
        )
        .await
        .expect("session reply should succeed");

        structured_task
            .await
            .expect("structured request task join")
            .expect("structured permission allowed");

        let session = {
            let sessions = state.sessions.lock().await;
            sessions
                .get(&session_id)
                .cloned()
                .expect("session should still exist")
        };

        assert_eq!(
            session
                .record()
                .metadata
                .get(PERMISSION_GRANTED_BY_TURN_COUNT_METADATA_KEY),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            session
                .record()
                .metadata
                .get(PERMISSION_GRANTED_BY_SESSION_COUNT_METADATA_KEY),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            session
                .record()
                .metadata
                .get(PERMISSION_GRANTED_BY_MATCHER_KIND_METADATA_KEY),
            Some(&serde_json::json!({
                "scope_only": 1,
                "structured_family": 1
            }))
        );
        assert_eq!(
            session
                .record()
                .metadata
                .get(LAST_PERMISSION_MATCHER_KIND_METADATA_KEY),
            Some(&serde_json::json!("structured_family"))
        );
        assert_eq!(
            session
                .record()
                .metadata
                .get(LAST_PERMISSION_GRANT_TARGET_METADATA_KEY),
            Some(&serde_json::json!("Shell commands: cargo *"))
        );

        PERMISSION_ENGINE.lock().await.clear_session(&session_id);
    }

    #[tokio::test]
    async fn request_permission_always_hint_does_not_auto_approve() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };
        PERMISSION_ENGINE.lock().await.clear_session(&session_id);

        let state_for_request = state.clone();
        let sid = session_id.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                sid,
                agendao_tool::PermissionRequest::new("bash")
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

        // Verify matcher miss was recorded: entering pending = no grant matched.
        let sessions = state.sessions.lock().await;
        let session = sessions.get(&session_id).expect("session should exist");
        let miss_count = session
            .record()
            .metadata
            .get("last_permission_miss_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(
            miss_count, 1,
            "miss_count should increment on AskOutcome::Pending"
        );

        PERMISSION_ENGINE.lock().await.clear_session(&session_id);
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
            agendao_tool::PermissionRequest::new("read").with_pattern("src/lib.rs"),
        )
        .await
        .expect("inspect_read should be auto-approved");

        assert!(PERMISSION_ENGINE.lock().await.list().is_empty());
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);
    }

    #[tokio::test]
    async fn request_permission_uses_class_default_lifetimes_when_missing() {
        let _guard = TEST_PERMISSION_LOCK.lock().await;
        const SESSION_ID: &str = "session-default-lifetimes";
        PERMISSION_ENGINE.lock().await.clear_session(SESSION_ID);

        let state = Arc::new(ServerState::new());
        let state_for_request = state.clone();
        let request_task = tokio::spawn(async move {
            request_permission(
                state_for_request,
                SESSION_ID.to_string(),
                agendao_tool::PermissionRequest::new("edit").with_pattern("src/lib.rs"),
            )
            .await
        });

        let permission = loop {
            let engine = PERMISSION_ENGINE.lock().await;
            if let Some(info) = engine.list().first().cloned().cloned() {
                break info;
            }
            drop(engine);
            tokio::task::yield_now().await;
        };

        assert_eq!(
            permission.supported_lifetimes,
            vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ]
        );

        let _ = reply_permission(
            State(state.clone()),
            Path(permission.id.clone()),
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
}
