/// Unix Socket Server - Listens on Unix domain socket and dispatches to OrchestrationCore.
///
/// This server implements the JSON-RPC protocol over Unix sockets,
/// allowing local processes to communicate with OrchestrationCore without HTTP overhead.
///
/// Session authority is shared with the HTTP server: `OrchestrationCore` is
/// constructed with `rocode_session::SessionManager` (the same instance as
/// `ServerState.sessions`), so `prompt`, `list_sessions`, and `get_session`
/// all read/write the same session data — no text mirror needed.

use anyhow::{Context, Result};
use rocode_orchestrator::OrchestrationCore;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::server::ServerState;
use rocode_session_core::SessionStore;

/// Unix Socket server. Generic over `S: SessionStore` so it can accept
/// `OrchestrationCore<rocode_session::SessionManager>` for unified authority.
pub struct UnixSocketServer<S: SessionStore = rocode_session_core::SessionManager> {
    state: Arc<ServerState>,
    core: Arc<OrchestrationCore<S>>,
    socket_path: String,
}

impl<S: SessionStore> UnixSocketServer<S> {
    pub fn new(
        state: Arc<ServerState>,
        core: Arc<OrchestrationCore<S>>,
        socket_path: String,
    ) -> Self {
        Self { state, core, socket_path }
    }

    /// Start the Unix socket server
    pub async fn serve(&self) -> Result<()>
    where
        S: Send + 'static,
    {
        // 1. Reject symlinks anywhere in the ancestor chain.
        //    create_dir_all, remove_file, and bind all follow symlinks
        //    silently — we must check before any of them touch the path.
        if let Some(parent) = Path::new(&self.socket_path).parent() {
            for ancestor in parent.ancestors() {
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                if ancestor.is_symlink() {
                    anyhow::bail!(
                        "Socket parent path contains a symlink: {}",
                        ancestor.display()
                    );
                }
            }
        }

        // 2. Create and lock the parent directory.
        if let Some(parent) = Path::new(&self.socket_path).parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create socket parent directory")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                    .context("Failed to secure socket parent directory")?;
            }
        }

        // 3. Remove stale socket file — use symlink_metadata to avoid
        //    following a symlink that may have been placed after step 1.
        if Path::new(&self.socket_path).exists() {
            if std::fs::symlink_metadata(&self.socket_path)
                .map(|m| m.is_symlink())
                .unwrap_or(true)
            {
                anyhow::bail!(
                    "Socket path is a symlink, refusing to remove: {}",
                    self.socket_path
                );
            }
            std::fs::remove_file(&self.socket_path)
                .context("Failed to remove existing socket file")?;
        }

        // 4. Bind with restricted umask.
        #[cfg(unix)]
        let _umask_guard = UmaskGuard::set(0o077);

        let listener = UnixListener::bind(&self.socket_path)
            .context("Failed to bind Unix socket")?;

        tracing::info!("Unix socket server listening on {}", self.socket_path);

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let state = Arc::clone(&self.state);
                    let core = Arc::clone(&self.core);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state, core).await {
                            tracing::error!("Error handling connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Error accepting connection: {}", e);
                }
            }
        }
    }
}

impl<S: SessionStore> Drop for UnixSocketServer<S> {
    fn drop(&mut self) {
        // Clean up socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// RAII guard that restores the previous umask on drop.
/// Ensures umask is restored even if `UnixListener::bind()` fails
/// (the `?` operator would skip the manual `libc::umask(old)` call).
#[cfg(unix)]
struct UmaskGuard {
    old: libc::mode_t,
}

#[cfg(unix)]
impl UmaskGuard {
    fn set(mask: libc::mode_t) -> Self {
        let old = unsafe { libc::umask(mask) };
        Self { old }
    }
}

#[cfg(unix)]
impl Drop for UmaskGuard {
    fn drop(&mut self) {
        unsafe {
            libc::umask(self.old);
        }
    }
}

async fn handle_connection<S: SessionStore + Send + 'static>(
    stream: UnixStream,
    state: Arc<ServerState>,
    core: Arc<OrchestrationCore<S>>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    while reader.read_line(&mut line).await? > 0 {
        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                tracing::warn!("JSON-RPC parse error: {}", e);
                let error_response = JsonRpcResponse::<()> {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: "Parse error".to_string(),
                    }),
                    id: 0,
                };
                let response_json = serde_json::to_string(&error_response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                line.clear();
                continue;
            }
        };

        let response = handle_request(request, &state, &core).await;
        let response_json = serde_json::to_string(&response)?;
        writer.write_all(response_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        line.clear();
    }

    Ok(())
}

async fn handle_request<S: SessionStore + Send + 'static>(
    request: JsonRpcRequest,
    state: &Arc<ServerState>,
    core: &Arc<OrchestrationCore<S>>,
) -> JsonRpcResponse<serde_json::Value> {
    let result = match request.method.as_str() {
        "prompt" => handle_prompt(request.params, state, core).await,
        "list_sessions" => handle_list_sessions(state).await,
        "get_workspace_context" => handle_get_workspace_context(state).await,
        "get_recent_models" => handle_get_recent_models(state).await,
        "put_recent_models" => handle_put_recent_models(request.params, state).await,
        "get_all_providers" => handle_get_all_providers(state).await,
        "list_execution_modes" => handle_list_execution_modes(state).await,
        "list_agents" => handle_list_agents(state).await,
        "get_session" => handle_get_session(request.params, state).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: format!("Method not found: {}", request.method),
        }),
    };

    match result {
        Ok(value) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(value),
            error: None,
            id: request.id,
        },
        Err(error) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id: request.id,
        },
    }
}

async fn handle_prompt<S: SessionStore + Send + 'static>(
    params: serde_json::Value,
    state: &Arc<ServerState>,
    core: &Arc<OrchestrationCore<S>>,
) -> Result<serde_json::Value, JsonRpcError> {
    let req: PromptRequest = serde_json::from_value(params).map_err(|e| {
        tracing::warn!("JSON-RPC invalid params: {}", e);
        JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
        }
    })?;

    // Ensure session exists in the shared authority (OrchestrationCore
    // now uses the same SessionManager as ServerState).
    {
        let mut sessions = state.sessions.lock().await;
        if sessions.get_mut(&req.session_id).is_none() {
            sessions.create(&req.session_id, state.workspace_root.to_string_lossy().as_ref());
        }
    }

    // Execute via OrchestrationCore — reads and writes the SAME
    // SessionManager as ServerState.sessions. No text mirror needed.
    let options = rocode_orchestrator::PromptExecutionOptions {
        agent_id: req.agent_id,
        scheduler_profile: None,
        model: req.model,
        variant: None,
        continue_last: req.continue_last,
        source_origin: Some(rocode_types::MessageSourceOrigin::Operator),
        source_surface: Some(rocode_types::MessageSourceSurface::UnixSocket),
        ingress_source: None,
        idempotency_key: None,
    };

    let result = core
        .execute_prompt(&req.session_id, &req.text, options)
        .await
        .map_err(|e| {
            tracing::error!("JSON-RPC execution error: {}", e);
            JsonRpcError {
                code: -32000,
                message: "Execution error".to_string(),
            }
        })?;

    let response = PromptResponse {
        session_id: result.session_id,
        message_id: result.message_id,
        text: result.text,
    };

    serde_json::to_value(response).map_err(|e| {
        tracing::error!("JSON-RPC internal error: {}", e);
        JsonRpcError {
            code: -32603,
            message: "Internal error".to_string(),
        }
    })
}

async fn handle_list_sessions(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    // Use the same rocode_session::SessionManager as HTTP routes
    let sessions = state.sessions.lock().await;
    let items: Vec<SessionListItem> = sessions
        .list()
        .iter()
        .map(|s| {
            let record = s.record();
            SessionListItem {
                id: record.id.clone(),
                created_at: record.created_at.to_rfc3339(),
                last_message: record
                    .messages
                    .last()
                    .map(|m| {
                        m.parts
                            .iter()
                            .filter_map(|p| {
                                if let rocode_types::PartType::Text { text, .. } = &p.part_type {
                                    Some(text.chars().take(100).collect::<String>())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("")
                    }),
            }
        })
        .collect();

    serde_json::to_value(items).map_err(|e| {
        tracing::error!("Session list serialization error: {}", e);
        JsonRpcError {
            code: -32603,
            message: "Internal error".to_string(),
        }
    })
}

async fn handle_get_recent_models(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_get_recent_models(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_put_recent_models(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let req: PutRecentModelsRequest = serde_json::from_value(params).map_err(|e| {
        tracing::warn!("put_recent_models invalid params: {}", e);
        JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
        }
    })?;
    let response = crate::local_put_recent_models(
        Arc::clone(state),
        req.recent_models,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_get_workspace_context(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_get_workspace_context(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_get_all_providers(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_get_all_providers(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_execution_modes(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_list_execution_modes(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_agents(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_list_agents(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_get_session(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let req: GetSessionRequest = serde_json::from_value(params).map_err(|e| {
        tracing::warn!("get_session invalid params: {}", e);
        JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
        }
    })?;

    // Use the same rocode_session::SessionManager as HTTP routes
    let sessions = state.sessions.lock().await;
    let session = sessions.get(&req.session_id).ok_or_else(|| {
        tracing::warn!("get_session: session not found: {}", req.session_id);
        JsonRpcError {
            code: -32000,
            message: "Session not found".to_string(),
        }
    })?;

    let detail = SessionDetail {
        id: session.record().id.clone(),
        messages: session
            .record()
            .messages
            .iter()
            .map(|m| SessionMessage {
                id: m.id.clone(),
                role: format!("{:?}", m.role),
                content: m
                    .parts
                    .iter()
                    .filter_map(|p| {
                        if let rocode_types::PartType::Text { text, .. } = &p.part_type {
                            Some(text.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            })
            .collect(),
    };

    serde_json::to_value(detail).map_err(|e| {
        tracing::error!("get_session serialization error: {}", e);
        JsonRpcError {
            code: -32603,
            message: "Internal error".to_string(),
        }
    })
}

fn to_rpc_internal_error(error: anyhow::Error) -> JsonRpcError {
    tracing::error!("JSON-RPC internal error: {}", error);
    JsonRpcError {
        code: -32603,
        message: "Internal error".to_string(),
    }
}

fn to_rpc_serde_error(error: serde_json::Error) -> JsonRpcError {
    tracing::error!("JSON-RPC serialization error: {}", error);
    JsonRpcError {
        code: -32603,
        message: "Internal error".to_string(),
    }
}

// ============================================================================
// JSON-RPC Protocol Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    params: serde_json::Value,
    id: u64,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse<T> {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct PromptRequest {
    session_id: String,
    text: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    continue_last: bool,
}

#[derive(Debug, Serialize)]
struct PromptResponse {
    session_id: String,
    message_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct PutRecentModelsRequest {
    recent_models: Vec<rocode_state::RecentModelEntry>,
}

#[derive(Debug, Deserialize)]
struct GetSessionRequest {
    session_id: String,
}

#[derive(Debug, Serialize)]
struct SessionListItem {
    id: String,
    created_at: String,
    last_message: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionDetail {
    id: String,
    messages: Vec<SessionMessage>,
}

#[derive(Debug, Serialize)]
struct SessionMessage {
    id: String,
    role: String,
    content: String,
}
