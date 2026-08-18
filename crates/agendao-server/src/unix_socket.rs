/// Unix Socket Server - Listens on Unix domain socket and dispatches to canonical session APIs.
///
/// This server implements the JSON-RPC protocol over Unix sockets,
/// allowing local processes to communicate without HTTP overhead.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::server::ServerState;
use crate::session_runtime::frontend_subscription::{
    frontend_event_passes_subscription_caps, frontend_event_session_id,
};
pub struct UnixSocketServer {
    state: Arc<ServerState>,
    socket_path: String,
}

impl UnixSocketServer {
    pub fn new(state: Arc<ServerState>, socket_path: String) -> Self {
        Self { state, socket_path }
    }

    /// Start the Unix socket server
    pub async fn serve(&self) -> Result<()> {
        let listener = self.bind()?;
        self.serve_bound(listener).await
    }

    /// Bind synchronously so combined HTTP + socket startup can fail fast
    /// before advertising a listener that never came up.
    pub fn bind(&self) -> Result<UnixListener> {
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

        // 2. Create a private parent when absent. Existing parents such as
        // /tmp must never have their permissions rewritten process-wide.
        if let Some(parent) = Path::new(&self.socket_path).parent() {
            prepare_socket_parent(parent)?;
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

        UnixListener::bind(&self.socket_path).context("Failed to bind Unix socket")
    }

    pub async fn serve_bound(&self, listener: UnixListener) -> Result<()> {
        tracing::info!("Unix socket server listening on {}", self.socket_path);

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let state = Arc::clone(&self.state);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state).await {
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

fn prepare_socket_parent(parent: &Path) -> Result<()> {
    let existed = parent.exists();
    std::fs::create_dir_all(parent).context("Failed to create socket parent directory")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        if !existed {
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .context("Failed to secure socket parent directory")?;
            return Ok(());
        }

        let metadata = std::fs::metadata(parent).context("Failed to inspect socket parent")?;
        if !metadata.is_dir() {
            anyhow::bail!("Socket parent is not a directory: {}", parent.display());
        }
        let mode = metadata.mode();
        let writable_by_others = mode & 0o022 != 0;
        let sticky = mode & 0o1000 != 0;
        if writable_by_others && !sticky {
            anyhow::bail!(
                "Socket parent is writable by other users without sticky bit: {}",
                parent.display()
            );
        }
    }

    Ok(())
}

impl Drop for UnixSocketServer {
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

async fn handle_connection(stream: UnixStream, state: Arc<ServerState>) -> Result<()> {
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

        // Check for subscribe_events — enters streaming event mode.
        if request.method == "subscribe_events" {
            let session_id = request
                .params
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let subscription = agendao_api::ResolvedFrontendSubscription::from_wire_tier(
                request.params.get("tier").and_then(|v| v.as_str()),
            );
            let response = handle_request(request, &state).await;
            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;

            if response.error.is_none() {
                let subscription = subscription.expect("subscribe validation and parsing agree");
                stream_frontend_events_to_writer(
                    &state,
                    session_id.as_deref(),
                    subscription,
                    writer,
                )
                .await;
            }
            return Ok(());
        }

        let response = handle_request(request, &state).await;
        let response_json = serde_json::to_string(&response)?;
        writer.write_all(response_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        line.clear();
    }

    Ok(())
}

async fn handle_request(
    request: JsonRpcRequest,
    state: &Arc<ServerState>,
) -> JsonRpcResponse<serde_json::Value> {
    let result = match request.method.as_str() {
        "prompt" => handle_prompt(request.params, state).await,
        "create_session" => handle_create_session(request.params, state).await,
        "list_sessions" => handle_list_sessions(request.params, state).await,
        "list_messages" => handle_list_messages(request.params, state).await,
        "get_session_info" => handle_get_session_info(request.params, state).await,
        "get_session_runtime" => handle_get_session_runtime(request.params, state).await,
        "get_session_todos" => handle_get_session_todos(request.params, state).await,
        "get_task_ledger" => handle_get_task_ledger(request.params, state).await,
        "apply_task_ledger_op" => handle_apply_task_ledger_op(request.params, state).await,
        "get_config" => handle_get_config(state).await,
        "patch_config" => handle_patch_config(request.params, state).await,
        "put_disabled_config" => handle_put_disabled_config(request.params, state).await,
        "get_workspace_context" => handle_get_workspace_context(state).await,
        "get_recent_models" => handle_get_recent_models(state).await,
        "put_recent_models" => handle_put_recent_models(request.params, state).await,
        "get_all_providers" => handle_get_all_providers(state).await,
        "get_provider_descriptor" => handle_get_provider_descriptor(request.params, state).await,
        "connect_provider" => handle_connect_provider(request.params, state).await,
        "update_provider" => handle_update_provider(request.params, state).await,
        "delete_provider" => handle_delete_provider(request.params, state).await,
        "get_provider_model_config" => {
            handle_get_provider_model_config(request.params, state).await
        }
        "put_provider_model_config" => {
            handle_put_provider_model_config(request.params, state).await
        }
        "delete_provider_model_config" => {
            handle_delete_provider_model_config(request.params, state).await
        }
        "set_provider_disabled" => handle_set_provider_disabled(request.params, state).await,
        "test_provider_connection" => handle_test_provider_connection(request.params, state).await,
        "refresh_provider_catalog" => handle_refresh_provider_catalog(state).await,
        "list_execution_modes" => handle_list_execution_modes(state).await,
        "list_agents" => handle_list_agents(state).await,
        "list_tools" => handle_list_tools(state).await,
        "list_skills" => handle_list_skills(request.params, state).await,
        "get_skill_detail" => handle_get_skill_detail(request.params, state).await,
        "manage_skill" => handle_manage_skill(request.params, state).await,
        "list_skill_proposals" => handle_list_skill_proposals(request.params, state).await,
        "update_skill_proposal_status" => {
            handle_update_skill_proposal_status(request.params, state).await
        }
        "get_mcp_status" => handle_get_mcp_status(state).await,
        "connect_mcp" => handle_connect_mcp(request.params, state).await,
        "disconnect_mcp" => handle_disconnect_mcp(request.params, state).await,
        "start_mcp_auth" => handle_start_mcp_auth(request.params, state).await,
        "authenticate_mcp" => handle_authenticate_mcp(request.params, state).await,
        "remove_mcp_auth" => handle_remove_mcp_auth(request.params, state).await,
        "put_mcp_config" => handle_put_mcp_config(request.params, state).await,
        "delete_mcp_config" => handle_delete_mcp_config(request.params, state).await,
        "list_plugins" => handle_list_plugins(state).await,
        "put_plugin_config" => handle_put_plugin_config(request.params, state).await,
        "delete_plugin_config" => handle_delete_plugin_config(request.params, state).await,
        "get_session_recovery" => handle_get_session_recovery(request.params, state).await,
        "list_questions" => handle_list_questions(state).await,
        "reply_question" => handle_reply_question(request.params, state).await,
        "reject_question" => handle_reject_question(request.params, state).await,
        "list_permissions" => handle_list_permissions(state).await,
        "reply_permission" => handle_reply_permission(request.params, state).await,
        "set_session_permission_mode" => {
            handle_set_session_permission_mode(request.params, state).await
        }
        "abort_session" => handle_abort_session(request.params, state).await,
        "cancel_tool_call" => handle_cancel_tool_call(request.params, state).await,
        "execute_shell" => handle_execute_shell(request.params, state).await,
        "fork_session" => handle_fork_session(request.params, state).await,
        "execute_session_recovery" => handle_execute_session_recovery(request.params, state).await,
        "compact_session" => handle_compact_session(request.params, state).await,
        "update_session_title" => handle_update_session_title(request.params, state).await,
        "delete_session" => handle_delete_session(request.params, state).await,
        "get_session" => handle_get_session(request.params, state).await,
        "subscribe_events" => handle_subscribe_events(request.params, state).await,
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

#[derive(serde::Deserialize)]
struct TaskLedgerSessionParams {
    session_id: String,
}

async fn handle_get_task_ledger(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let req: TaskLedgerSessionParams = serde_json::from_value(params).map_err(|e| {
        tracing::warn!("JSON-RPC invalid params: {}", e);
        JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
        }
    })?;
    let snapshot =
        crate::session_runtime::task_ledger::task_ledger_snapshot(state, &req.session_id)
            .await
            .map_err(|e| to_rpc_error_from_api(&e))?;
    serde_json::to_value(snapshot)
        .map_err(|e| to_rpc_error_from_api(&crate::error::ApiError::InternalError(e.to_string())))
}

#[derive(serde::Deserialize)]
struct ApplyTaskLedgerOpParams {
    session_id: String,
    expected_revision: u64,
    op: agendao_types::task_ledger::TaskLedgerOp,
}

async fn handle_apply_task_ledger_op(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let req: ApplyTaskLedgerOpParams = serde_json::from_value(params).map_err(|e| {
        tracing::warn!("JSON-RPC invalid params: {}", e);
        JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
        }
    })?;
    let (ledger, cause) = crate::session_runtime::task_ledger::apply_task_ledger_op(
        state,
        &req.session_id,
        req.expected_revision,
        req.op,
    )
    .await
    .map_err(|e| to_rpc_error_from_api(&e))?;
    serde_json::to_value(serde_json::json!({ "ledger": ledger, "cause": cause }))
        .map_err(|e| to_rpc_error_from_api(&crate::error::ApiError::InternalError(e.to_string())))
}

fn to_rpc_error_from_api(error: &crate::error::ApiError) -> JsonRpcError {
    JsonRpcError {
        code: match error {
            crate::error::ApiError::SessionNotFound(_) | crate::error::ApiError::NotFound(_) => {
                -32001
            }
            crate::error::ApiError::RevisionConflict { .. } => -32009,
            crate::error::ApiError::PermissionDenied(_) => -32003,
            _ => -32000,
        },
        message: error.to_string(),
    }
}

async fn handle_prompt(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let req: PromptRequest = serde_json::from_value(params).map_err(|e| {
        tracing::warn!("JSON-RPC invalid params: {}", e);
        JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
        }
    })?;

    let mut session_id = req.session_id.clone();
    let previous_assistant_id = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .and_then(|session| session.last_owner_local_assistant_message())
            .map(|message| message.id.clone())
    };

    if previous_assistant_id.is_none() {
        let session_exists = {
            let sessions = state.sessions.lock().await;
            sessions.get(&session_id).is_some()
        };
        if !session_exists {
            let created = crate::local_create_session(
                Arc::clone(state),
                agendao_api::CreateSessionRequest {
                    scheduler: None,
                    directory: Some(state.workspace_root.to_string_lossy().into_owned()),
                    project_id: None,
                    title: None,
                },
            )
            .await
            .map_err(to_rpc_internal_error)?;
            session_id = created.id;
        }
    }

    let request = agendao_api::PromptRequest {
        message: Some(req.text),
        parts: None,
        idempotency_key: None,
        ingress_source: Some("cli".to_string()),
        source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
        source_surface: Some(agendao_types::MessageSourceSurface::UnixSocket),
        agent: req.agent_id,
        scheduler: req.scheduler,
        model: req.model,
        variant: req.variant,
        command: req.command,
        arguments: None,
    };

    crate::local_prompt(state.clone(), &session_id, request)
        .await
        .map_err(|e| {
            tracing::error!("JSON-RPC execution error: {}", e);
            JsonRpcError {
                code: -32000,
                message: "Execution error".to_string(),
            }
        })?;

    let completed = tokio::time::timeout(std::time::Duration::from_secs(1_800), async {
        loop {
            let result = {
                let sessions = state.sessions.lock().await;
                sessions.get(&session_id).and_then(|session| {
                    let message = session.last_owner_local_assistant_message()?;
                    (previous_assistant_id.as_deref() != Some(message.id.as_str())
                        && message.finish.is_some())
                    .then(|| {
                        (
                            message.id.clone(),
                            crate::session_runtime::assistant_visible_text(message),
                        )
                    })
                })
            };
            if let Some(result) = result {
                break result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| JsonRpcError {
        code: -32000,
        message: "Prompt execution timed out".to_string(),
    })?;

    let response = PromptResponse {
        session_id,
        message_id: completed.0,
        text: completed.1,
    };

    serde_json::to_value(response).map_err(|e| {
        tracing::error!("JSON-RPC internal error: {}", e);
        JsonRpcError {
            code: -32603,
            message: "Internal error".to_string(),
        }
    })
}

async fn handle_create_session(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let session = crate::local_create_session(Arc::clone(state), request)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(session).map_err(to_rpc_serde_error)
}

async fn handle_list_sessions(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ListSessionsRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let items = crate::local_list_sessions(
        Arc::clone(state),
        request.directory,
        request.search,
        request.limit,
    )
    .await
    .map_err(to_rpc_internal_error)?;

    serde_json::to_value(items).map_err(|e| {
        tracing::error!("Session list serialization error: {}", e);
        JsonRpcError {
            code: -32603,
            message: "Internal error".to_string(),
        }
    })
}

async fn handle_list_messages(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let messages = crate::local_list_messages(Arc::clone(state), &request.session_id, None, None)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(messages).map_err(to_rpc_serde_error)
}

async fn handle_get_session_info(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let session = crate::local_get_session(Arc::clone(state), &request.session_id)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(session).map_err(to_rpc_serde_error)
}

async fn handle_get_session_runtime(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let runtime = crate::local_get_session_runtime(Arc::clone(state), &request.session_id)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(runtime).map_err(to_rpc_serde_error)
}

async fn handle_get_session_todos(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let todos = crate::local_get_session_todos(Arc::clone(state), &request.session_id)
        .await
        .map_err(to_rpc_internal_error)?;
    let todos: Vec<agendao_api::ApiTodoItem> = todos
        .into_iter()
        .map(|todo| agendao_api::ApiTodoItem {
            id: todo.id,
            content: todo.content,
            status: todo.status,
            priority: todo.priority,
        })
        .collect();
    serde_json::to_value(todos).map_err(to_rpc_serde_error)
}

async fn handle_get_config(state: &Arc<ServerState>) -> Result<serde_json::Value, JsonRpcError> {
    let config = crate::local_get_config(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(config).map_err(to_rpc_serde_error)
}

async fn handle_patch_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let config = crate::local_patch_config(Arc::clone(state), params)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(config).map_err(to_rpc_serde_error)
}

async fn handle_put_disabled_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let update = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let config = crate::local_put_disabled_config(Arc::clone(state), update)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(config).map_err(to_rpc_serde_error)
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
    let response = crate::local_put_recent_models(Arc::clone(state), req.recent_models)
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

async fn handle_get_provider_descriptor(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ProviderRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_get_provider_descriptor(Arc::clone(state), &request.provider_id)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_connect_provider(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_connect_provider(Arc::clone(state), request)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(true))
}

async fn handle_update_provider(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: UpdateProviderRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let updated = crate::local_update_provider(
        Arc::clone(state),
        &request.provider_id,
        request.name,
        request.base_url,
        request.protocol,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(updated))
}

async fn handle_delete_provider(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ProviderRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let deleted = crate::local_delete_provider(Arc::clone(state), &request.provider_id)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(deleted))
}

async fn handle_get_provider_model_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ProviderModelRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_get_provider_model_config(
        Arc::clone(state),
        &request.provider_id,
        &request.model_key,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_put_provider_model_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: PutProviderModelRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_put_provider_model_config(
        Arc::clone(state),
        &request.provider_id,
        &request.model_key,
        request.model,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_delete_provider_model_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ProviderModelRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_delete_provider_model_config(
        Arc::clone(state),
        &request.provider_id,
        &request.model_key,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_set_provider_disabled(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SetProviderDisabledRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let updated = crate::local_set_provider_disabled(
        Arc::clone(state),
        &request.provider_id,
        request.disabled,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(updated))
}

async fn handle_test_provider_connection(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ProviderRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_test_provider_connection(Arc::clone(state), &request.provider_id)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_refresh_provider_catalog(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_refresh_provider_catalog(Arc::clone(state))
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

async fn handle_list_agents(state: &Arc<ServerState>) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_list_agents(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_tools(state: &Arc<ServerState>) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_list_tools(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_skills(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let query = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_list_skills(Arc::clone(state), query)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_get_skill_detail(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let query = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_get_skill_detail(Arc::clone(state), query)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_manage_skill(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_manage_skill(Arc::clone(state), request)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_skill_proposals(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: StatusRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_list_skill_proposals(Arc::clone(state), &request.status)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_update_skill_proposal_status(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ProposalStatusRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response =
        crate::local_update_skill_proposal_status(Arc::clone(state), &request.id, &request.status)
            .await
            .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_get_mcp_status(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_get_mcp_status(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_connect_mcp(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: NameRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_connect_mcp(Arc::clone(state), &request.name)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(response))
}

async fn handle_disconnect_mcp(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: NameRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_disconnect_mcp(Arc::clone(state), &request.name)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(response))
}

async fn handle_start_mcp_auth(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: NameRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_start_mcp_auth(Arc::clone(state), &request.name)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_authenticate_mcp(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: NameRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_authenticate_mcp(Arc::clone(state), &request.name)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_remove_mcp_auth(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: NameRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_remove_mcp_auth(Arc::clone(state), &request.name)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(response))
}

async fn handle_put_mcp_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: PutMcpConfigRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_put_mcp_config(Arc::clone(state), &request.key, request.mcp)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_delete_mcp_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: KeyRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_delete_mcp_config(Arc::clone(state), &request.key)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_plugins(state: &Arc<ServerState>) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_list_plugins(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_put_plugin_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: PutPluginConfigRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_put_plugin_config(Arc::clone(state), &request.key, request.plugin)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_delete_plugin_config(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: KeyRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_delete_plugin_config(Arc::clone(state), &request.key)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_get_session_recovery(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_get_session_recovery(Arc::clone(state), &request.session_id)
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_list_questions(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let response = crate::local_list_questions(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_reply_question(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ReplyQuestionRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_reply_question(Arc::clone(state), &request.question_id, request.answers)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(true))
}

async fn handle_reject_question(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: IdRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_reject_question(Arc::clone(state), &request.id)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(true))
}

async fn handle_list_permissions(
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let permissions = crate::local_list_permissions(Arc::clone(state))
        .await
        .map_err(to_rpc_internal_error)?;
    serde_json::to_value(permissions).map_err(to_rpc_serde_error)
}

async fn handle_reply_permission(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ReplyPermissionRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_reply_permission(
        Arc::clone(state),
        &request.permission_id,
        request.reply,
        request.message,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(true))
}

async fn handle_set_session_permission_mode(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SetSessionPermissionModeRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let session = crate::local_set_session_permission_mode(
        Arc::clone(state),
        &request.session_id,
        request.mode,
    )
    .await
    .map_err(to_rpc_internal_error)?;
    serde_json::to_value(session).map_err(to_rpc_serde_error)
}

async fn handle_abort_session(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_abort_session(Arc::clone(state), &request.session_id)
        .await
        .map_err(to_rpc_internal_error)
}

async fn handle_cancel_tool_call(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: CancelToolCallRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_cancel_tool_call(
        Arc::clone(state),
        &request.session_id,
        &request.tool_call_id,
    )
    .await
    .map_err(to_rpc_internal_error)
}

async fn handle_execute_shell(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ExecuteShellRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_execute_shell(
        Arc::clone(state),
        &request.session_id,
        request.command,
        request.workdir,
    )
    .await
    .map_err(to_rpc_internal_error)
}

async fn handle_fork_session(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ForkSessionRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response =
        crate::local_fork_session(Arc::clone(state), &request.session_id, request.message_id)
            .await
            .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_execute_session_recovery(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: ExecuteSessionRecoveryRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    crate::local_execute_session_recovery(Arc::clone(state), &request.session_id, request.action)
        .await
        .map_err(to_rpc_internal_error)
}

async fn handle_compact_session(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: CompactSessionRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response =
        crate::local_compact_session(Arc::clone(state), &request.session_id, request.focus)
            .await
            .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_update_session_title(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: UpdateSessionTitleRequest =
        serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response =
        crate::local_update_session_title(Arc::clone(state), &request.session_id, &request.title)
            .await
            .map_err(to_rpc_internal_error)?;
    serde_json::to_value(response).map_err(to_rpc_serde_error)
}

async fn handle_delete_session(
    params: serde_json::Value,
    state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    let request: SessionRequest = serde_json::from_value(params).map_err(to_rpc_invalid_params)?;
    let response = crate::local_delete_session(Arc::clone(state), &request.session_id)
        .await
        .map_err(to_rpc_internal_error)?;
    Ok(serde_json::Value::Bool(response))
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

    // Use the same agendao_session::SessionManager as HTTP routes
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
                        if let agendao_types::PartType::Text { text, .. } = &p.part_type {
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

async fn handle_subscribe_events(
    params: serde_json::Value,
    _state: &Arc<ServerState>,
) -> Result<serde_json::Value, JsonRpcError> {
    agendao_api::ResolvedFrontendSubscription::from_wire_tier(
        params.get("tier").and_then(|value| value.as_str()),
    )
    .map_err(|message| JsonRpcError {
        code: -32602,
        message,
    })?;
    let session_id = params.get("session_id").and_then(|v| v.as_str());
    Ok(serde_json::json!({
        "subscribed": true,
        "session_id": session_id,
    }))
}

async fn stream_frontend_events_to_writer(
    state: &Arc<ServerState>,
    session_id: Option<&str>,
    subscription: agendao_api::ResolvedFrontendSubscription,
    mut writer: impl tokio::io::AsyncWrite + Unpin,
) {
    use tokio::io::AsyncWriteExt;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut rx = crate::session_runtime::local_frontend::spawn_local_frontend_events(
        Arc::clone(state),
        cancel.clone(),
    );
    while let Some(event) = rx.recv().await {
        if let Some(filter) = session_id {
            if frontend_event_session_id(&event) != Some(filter) {
                continue;
            }
        }
        if !frontend_event_passes_subscription_caps(&event, &subscription.capabilities) {
            continue;
        }
        let Ok(line) = serde_json::to_string(&event) else {
            break;
        };
        if writer.write_all(line.as_bytes()).await.is_err()
            || writer.write_all(b"\n").await.is_err()
            || writer.flush().await.is_err()
        {
            break;
        }
    }
    // 客户端断开或 bridge 结束时停掉后台 bridge 任务,避免它作为僵尸订阅
    // 永久挂在 frontend_bus 上。
    cancel.cancel();
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

fn to_rpc_invalid_params(error: serde_json::Error) -> JsonRpcError {
    tracing::warn!(%error, "JSON-RPC invalid params");
    JsonRpcError {
        code: -32602,
        message: "Invalid params".to_string(),
    }
}

// ============================================================================
// JSON-RPC Protocol Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    _jsonrpc: String,
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
    scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ListSessionsRequest {
    #[serde(default)]
    directory: Option<String>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SessionRequest {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct ReplyPermissionRequest {
    permission_id: String,
    reply: String,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetSessionPermissionModeRequest {
    session_id: String,
    mode: agendao_types::SessionPermissionMode,
}

#[derive(Debug, Deserialize)]
struct StatusRequest {
    status: String,
}

#[derive(Debug, Deserialize)]
struct ProposalStatusRequest {
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct ReplyQuestionRequest {
    question_id: String,
    answers: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct IdRequest {
    id: String,
}

#[derive(Debug, Deserialize)]
struct CompactSessionRequest {
    session_id: String,
    #[serde(default)]
    focus: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSessionTitleRequest {
    session_id: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct ProviderRequest {
    provider_id: String,
}

#[derive(Debug, Deserialize)]
struct UpdateProviderRequest {
    provider_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProviderModelRequest {
    provider_id: String,
    model_key: String,
}

#[derive(Debug, Deserialize)]
struct PutProviderModelRequest {
    provider_id: String,
    model_key: String,
    model: agendao_config::ModelConfig,
}

#[derive(Debug, Deserialize)]
struct SetProviderDisabledRequest {
    provider_id: String,
    disabled: bool,
}

#[derive(Debug, Deserialize)]
struct NameRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct KeyRequest {
    key: String,
}

#[derive(Debug, Deserialize)]
struct PutMcpConfigRequest {
    key: String,
    mcp: agendao_config::McpServerConfig,
}

#[derive(Debug, Deserialize)]
struct PutPluginConfigRequest {
    key: String,
    plugin: agendao_config::PluginConfig,
}

#[derive(Debug, Deserialize)]
struct CancelToolCallRequest {
    session_id: String,
    tool_call_id: String,
}

#[derive(Debug, Deserialize)]
struct ExecuteShellRequest {
    session_id: String,
    command: String,
    #[serde(default)]
    workdir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ForkSessionRequest {
    session_id: String,
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExecuteSessionRecoveryRequest {
    session_id: String,
    action: agendao_api::RecoveryActionKind,
}

#[cfg(test)]
mod tests {
    use super::{frontend_event_passes_subscription_caps, prepare_socket_parent, PromptRequest};
    use agendao_server_core::frontend_events::FrontendEvent;
    use agendao_server_core::runtime_events::ToolCallPhase;

    #[cfg(unix)]
    #[test]
    fn socket_parent_security_preserves_existing_sticky_directory() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o1777))
            .expect("set sticky test mode");

        prepare_socket_parent(root.path()).expect("sticky shared parent should be accepted");

        assert_eq!(
            std::fs::metadata(root.path()).expect("metadata").mode() & 0o7777,
            0o1777
        );
    }

    #[cfg(unix)]
    #[test]
    fn socket_parent_security_makes_new_directory_private() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("tempdir");
        let parent = root.path().join("private/socket");

        prepare_socket_parent(&parent).expect("new socket parent");

        assert_eq!(
            std::fs::metadata(parent).expect("metadata").mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn prompt_request_deserializes_command_scheduler_and_variant() {
        let request: PromptRequest = serde_json::from_value(serde_json::json!({
            "session_id": "ses_1",
            "text": "/run cargo test",
            "agent_id": "build",
            "scheduler": {"kind": "auto"},
            "model": "openai/gpt-5",
            "variant": "fast",
            "command": "run"
        }))
        .expect("deserialize unix prompt request");

        assert_eq!(request.command.as_deref(), Some("run"));
        assert!(matches!(
            request.scheduler,
            Some(agendao_orchestrator::selector::SchedulerChoice::Auto)
        ));
        assert_eq!(request.variant.as_deref(), Some("fast"));
    }

    #[test]
    fn unix_socket_cli_tier_keeps_final_message_and_tool_completion() {
        let cli_caps =
            agendao_api::FrontendSubscriptionTier::CliLowFrequency.default_capabilities();

        let delta = FrontendEvent::OutputBlockAppended {
            session_id: "ses_1".to_string(),
            id: Some("msg_1".to_string()),
            block: serde_json::json!({
                "kind": "message",
                "phase": "delta",
                "text": "hi"
            }),
            live_identity: None,
        };
        let tool_done = FrontendEvent::ToolCallUpsert {
            session_id: "ses_1".to_string(),
            tool_call_id: "tool_1".to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Complete,
        };
        let full = FrontendEvent::OutputBlockAppended {
            session_id: "ses_1".to_string(),
            id: Some("msg_1".to_string()),
            block: serde_json::json!({
                "kind": "message",
                "phase": "full",
                "text": "hi"
            }),
            live_identity: None,
        };

        assert!(
            !frontend_event_passes_subscription_caps(&delta, &cli_caps),
            "CLI tier must not receive message delta over unix transport"
        );
        assert!(
            frontend_event_passes_subscription_caps(&full, &cli_caps),
            "CLI tier must receive the completed message"
        );
        assert!(
            frontend_event_passes_subscription_caps(&tool_done, &cli_caps),
            "must-deliver tool lifecycle events must survive unix transport filtering"
        );
    }
}

#[derive(Debug, Serialize)]
struct PromptResponse {
    session_id: String,
    message_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct PutRecentModelsRequest {
    recent_models: Vec<agendao_state::RecentModelEntry>,
}

#[derive(Debug, Deserialize)]
struct GetSessionRequest {
    session_id: String,
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
