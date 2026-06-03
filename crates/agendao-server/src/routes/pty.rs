use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

use crate::pty::{PtyManager, PtySession as PtySessionStruct, PtySubscription};
use crate::routes::permission::request_permission;
use crate::{ApiError, Result, ServerState};

pub(crate) fn pty_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_pty).post(create_pty))
        .route("/{id}", get(get_pty).put(update_pty).delete(delete_pty))
        .route("/{id}/resize", axum::routing::post(resize_pty))
        .route("/{id}/connect", get(pty_connect))
}

#[derive(Debug, Serialize)]
pub struct PtyInfo {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub status: String,
}

impl From<PtySessionStruct> for PtyInfo {
    fn from(session: PtySessionStruct) -> Self {
        Self {
            id: session.id,
            command: session.command,
            cwd: session.cwd,
            status: match session.status {
                crate::pty::PtyStatus::Running => "running".to_string(),
                crate::pty::PtyStatus::Exited => "exited".to_string(),
                crate::pty::PtyStatus::Error => "error".to_string(),
            },
        }
    }
}

static PTY_MANAGER: std::sync::OnceLock<PtyManager> = std::sync::OnceLock::new();

fn get_pty_manager() -> &'static PtyManager {
    PTY_MANAGER.get_or_init(PtyManager::new)
}

async fn list_pty() -> Json<Vec<PtyInfo>> {
    let manager = get_pty_manager();
    let sessions = manager.list_sessions().await;
    Json(sessions.into_iter().map(PtyInfo::from).collect())
}

#[derive(Debug, Deserialize)]
pub struct CreatePtyRequest {
    pub command: String,
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub session_id: Option<String>,
}

fn resolve_pty_cwd(state: &ServerState, cwd: Option<&str>) -> Result<String> {
    let root = state
        .project_root()
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve project root: {}", e)))?;
    let requested = cwd
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let candidate = if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve PTY cwd: {}", e)))?;
    if !canonical.starts_with(&root) {
        return Err(ApiError::BadRequest(
            "PTY cwd must stay inside the project directory".to_string(),
        ));
    }
    if !canonical.is_dir() {
        return Err(ApiError::BadRequest(
            "PTY cwd must be an existing directory".to_string(),
        ));
    }
    Ok(canonical.to_string_lossy().to_string())
}

fn required_session_id(value: Option<String>) -> Result<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::BadRequest("PTY creation requires a session_id".to_string()))
}

fn build_pty_permission_request(command: &str, cwd: &str) -> agendao_tool::PermissionRequest {
    agendao_tool::PermissionRequest::new("pty")
        .with_pattern(command.to_string())
        .with_metadata(
            "description",
            serde_json::json!(format!("Start terminal command `{command}` in {cwd}")),
        )
        .with_metadata("command", serde_json::json!(command))
        .with_metadata("cwd", serde_json::json!(cwd))
}

fn map_tool_error_to_api_error(error: agendao_tool::ToolError) -> ApiError {
    match error {
        agendao_tool::ToolError::PermissionDenied(message) => ApiError::PermissionDenied(message),
        agendao_tool::ToolError::InvalidArguments(message)
        | agendao_tool::ToolError::ValidationError(message) => ApiError::BadRequest(message),
        agendao_tool::ToolError::FileNotFound(message) => ApiError::NotFound(message),
        agendao_tool::ToolError::ExecutionError(message)
        | agendao_tool::ToolError::Timeout(message)
        | agendao_tool::ToolError::BinaryFile(message)
        | agendao_tool::ToolError::QuestionRejected(message) => ApiError::InternalError(message),
        agendao_tool::ToolError::Cancelled => ApiError::InternalError("Cancelled".to_string()),
    }
}

async fn create_pty(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreatePtyRequest>,
) -> Result<Json<PtyInfo>> {
    let manager = get_pty_manager();
    let cwd = resolve_pty_cwd(state.as_ref(), req.cwd.as_deref())?;
    let session_id = required_session_id(req.session_id)?;
    request_permission(
        state,
        session_id,
        build_pty_permission_request(&req.command, &cwd),
    )
    .await
    .map_err(map_tool_error_to_api_error)?;
    let session = manager
        .create_session(&req.command, Some(&cwd), req.env)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(PtyInfo::from(session)))
}

async fn get_pty(Path(id): Path<String>) -> Result<Json<PtyInfo>> {
    let manager = get_pty_manager();
    let session = manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("PTY session not found".to_string()))?;
    Ok(Json(PtyInfo::from(session)))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePtyRequest {
    pub command: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResizePtyRequest {
    pub cols: u16,
    pub rows: u16,
}

async fn update_pty(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePtyRequest>,
) -> Result<Json<PtyInfo>> {
    let manager = get_pty_manager();
    let cwd = match req.cwd.as_deref() {
        Some(_) => Some(resolve_pty_cwd(state.as_ref(), req.cwd.as_deref())?),
        None => None,
    };
    let session = manager
        .update_session(&id, req.command.as_deref(), cwd.as_deref())
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;
    Ok(Json(PtyInfo::from(session)))
}

async fn delete_pty(Path(id): Path<String>) -> Result<Json<bool>> {
    let manager = get_pty_manager();
    let deleted = manager.delete_session(&id).await;
    Ok(Json(deleted))
}

async fn resize_pty(
    Path(id): Path<String>,
    Json(req): Json<ResizePtyRequest>,
) -> Result<Json<bool>> {
    let manager = get_pty_manager();
    manager
        .resize_session(&id, req.cols, req.rows)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct PtyConnectQuery {
    /// Byte cursor from which to replay buffered output.
    /// `-1` means skip all buffered output and only receive live data.
    /// Omitted or `0` means replay from the beginning of the retained buffer.
    pub cursor: Option<i64>,
}

/// WebSocket endpoint that bridges a client to a PTY session, matching the TS
/// `Pty.connect` protocol:
///   1. On connect: replay buffered output from the requested cursor, then send
///      a binary metadata frame (`0x00` + JSON `{"cursor":<n>}`) so the client
///      knows the current position.
///   2. Forward live PTY output to the client as binary frames.
///   3. Forward text/binary messages from the client into the PTY as input.
async fn pty_connect(
    Path(id): Path<String>,
    Query(query): Query<PtyConnectQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let manager = get_pty_manager();

    // Validate the session exists before upgrading.
    let subscription = match manager.subscribe(&id).await {
        Ok(sub) => sub,
        Err(_) => {
            return axum::response::Response::builder()
                .status(404)
                .body(axum::body::Body::from("PTY session not found"))
                .unwrap()
                .into_response();
        }
    };

    let cursor_param = query.cursor.unwrap_or(0);

    ws.on_upgrade(move |socket| handle_pty_websocket(socket, subscription, cursor_param))
        .into_response()
}

async fn handle_pty_websocket(mut socket: WebSocket, sub: PtySubscription, cursor_param: i64) {
    // --- Phase 1: Replay buffered output ---
    // Determine the byte offset to start replaying from.
    let from = if cursor_param == -1 {
        // Skip all buffered output.
        sub.cursor
    } else if cursor_param > 0 {
        cursor_param as usize
    } else {
        0
    };

    if from < sub.cursor && !sub.buffer.is_empty() {
        let offset = from.saturating_sub(sub.buffer_start);
        if offset < sub.buffer.len() {
            let replay = &sub.buffer[offset..];
            // Send in 64 KiB chunks to avoid oversized frames (matching TS).
            for chunk in replay.chunks(64 * 1024) {
                let bytes = axum::body::Bytes::copy_from_slice(chunk);
                if socket.send(Message::Binary(bytes)).await.is_err() {
                    return;
                }
            }
        }
    }

    // Send metadata frame: 0x00 byte prefix + JSON `{"cursor":<n>}`.
    {
        let meta_json = format!("{{\"cursor\":{}}}", sub.cursor);
        let json_bytes = meta_json.as_bytes();
        let mut frame = Vec::with_capacity(1 + json_bytes.len());
        frame.push(0x00);
        frame.extend_from_slice(json_bytes);
        let bytes = axum::body::Bytes::from(frame);
        if socket.send(Message::Binary(bytes)).await.is_err() {
            return;
        }
    }

    // --- Phase 2: Bridge live I/O ---
    // Use a channel to decouple the broadcast receiver from the socket send
    // loop, since WebSocket requires &mut self for both send and recv.
    let (ws_tx, mut ws_rx) = mpsc::channel::<Vec<u8>>(256);
    let mut rx = sub.rx;
    let writer = sub.writer;

    // Task: forward broadcast PTY output into the mpsc channel.
    let forward_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(data) => {
                    if ws_tx.send(data).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Main loop: multiplex between sending PTY output and receiving WS input.
    loop {
        tokio::select! {
            // Live PTY output ready to send to the client.
            Some(data) = ws_rx.recv() => {
                let bytes = axum::body::Bytes::from(data);
                if socket.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            // Client sent a message (input for the PTY).
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        let data = t.as_bytes().to_vec();
                        if data.is_empty() { continue; }
                        let w = writer.clone();
                        let res = tokio::task::spawn_blocking(move || {
                            let mut guard = w.lock().unwrap();
                            guard.write_all(&data)?;
                            guard.flush()
                        }).await;
                        if res.is_err() || res.unwrap().is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        let data = b.to_vec();
                        if data.is_empty() { continue; }
                        let w = writer.clone();
                        let res = tokio::task::spawn_blocking(move || {
                            let mut guard = w.lock().unwrap();
                            guard.write_all(&data)?;
                            guard.flush()
                        }).await;
                        if res.is_err() || res.unwrap().is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => continue,
                }
            }
        }
    }

    forward_task.abort();
}

#[cfg(test)]
mod tests {
    use super::{build_pty_permission_request, required_session_id};

    #[test]
    fn required_session_id_rejects_missing_or_empty_values() {
        assert!(required_session_id(None).is_err());
        assert!(required_session_id(Some("   ".to_string())).is_err());
        assert_eq!(
            required_session_id(Some(" session-1 ".to_string())).expect("session id"),
            "session-1"
        );
    }

    #[test]
    fn pty_permission_request_includes_command_and_cwd_metadata() {
        let request = build_pty_permission_request("/bin/bash", "/workspace");

        assert_eq!(request.permission, "pty");
        assert_eq!(request.patterns, vec!["/bin/bash".to_string()]);
        assert_eq!(
            request
                .metadata
                .get("command")
                .and_then(|value| value.as_str()),
            Some("/bin/bash")
        );
        assert_eq!(
            request.metadata.get("cwd").and_then(|value| value.as_str()),
            Some("/workspace")
        );
    }
}
