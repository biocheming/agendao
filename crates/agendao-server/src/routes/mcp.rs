use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::mcp_oauth::{
    LocalMcpConfig, McpOAuthError, McpOAuthManager, McpRuntimeConfig,
    McpServerInfo as McpServerInfoStruct, McpServerLogEntry, RemoteMcpConfig,
};
use crate::{ApiError, Result, ServerState};
use agendao_config::{McpOAuthConfig, McpServerConfig as LoadedMcpServerConfig};

pub(crate) fn mcp_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(get_mcp_status).post(add_mcp_server))
        .route("/{name}/auth", post(start_mcp_auth).delete(remove_mcp_auth))
        .route("/{name}/auth/callback", post(mcp_auth_callback))
        .route("/{name}/auth/authenticate", post(mcp_authenticate))
        .route("/{name}/connect", post(connect_mcp))
        .route("/{name}/disconnect", post(disconnect_mcp))
        .route("/{name}/logs", get(get_mcp_logs))
        .route("/{name}/restart", post(restart_mcp))
}

#[derive(Debug, Serialize)]
pub struct McpStatusInfo {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

static MCP_OAUTH_MANAGER: std::sync::OnceLock<McpOAuthManager> = std::sync::OnceLock::new();

fn get_mcp_oauth_manager() -> &'static McpOAuthManager {
    MCP_OAUTH_MANAGER.get_or_init(McpOAuthManager::new)
}

/// config 写面（PUT/DELETE `/config/mcp/{key}`）同步 runtime manager 的入口。
/// 不写时 manager 只增不删（`sync_mcp_from_disk` 的 has_server 短路），
/// 删除/启停后状态列表会残留旧值——写后同步让启停即时生效。
pub(crate) async fn sync_manager_after_mcp_config_write(
    key: &str,
    cfg: Option<&LoadedMcpServerConfig>,
) {
    let manager = get_mcp_oauth_manager();
    let runtime = cfg.and_then(|c| parse_runtime_from_loaded_config(c.clone()).ok().flatten());
    match runtime {
        Some((config, enabled)) => {
            let was_connected = manager
                .get_server(key)
                .await
                .is_some_and(|info| info.status == "connected");
            if manager.has_server(key).await && !enabled {
                let _ = manager.disconnect(key).await;
            }
            manager.add_server(key.to_string(), config, enabled).await;
            // 原本连着的：重连以应用新 command/url/enabled。
            if enabled && was_connected {
                let _ = manager.connect(key).await;
            }
        }
        // 无 runtime（Enabled{} 变体 / DELETE）：从 manager 摘除。
        None => {
            manager.remove_server(key).await;
        }
    }
}

impl From<McpServerInfoStruct> for McpStatusInfo {
    fn from(info: McpServerInfoStruct) -> Self {
        Self {
            name: info.name,
            status: info.status,
            tools: info.tools,
            resources: info.resources,
            error: info.error,
        }
    }
}

pub(crate) async fn get_mcp_status(
    State(state): State<Arc<ServerState>>,
) -> Json<HashMap<String, McpStatusInfo>> {
    let manager = get_mcp_oauth_manager();
    if let Err(error) = sync_mcp_from_disk(manager, &state.config_store).await {
        tracing::warn!(%error, "failed to sync MCP servers from config");
    }
    let servers = manager.list_servers().await;
    let mut result = HashMap::new();
    for server in servers {
        result.insert(server.name.clone(), McpStatusInfo::from(server));
    }
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct AddMcpRequest {
    pub name: String,
    pub config: McpConfigInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfigInput {
    #[serde(rename = "type")]
    pub server_type: Option<String>,
    pub command: Option<Vec<String>>,
    pub environment: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub timeout: Option<u64>,
    pub oauth: Option<McpOAuthConfig>,
}

async fn add_mcp_server(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<AddMcpRequest>,
) -> Result<Json<HashMap<String, McpStatusInfo>>> {
    let manager = get_mcp_oauth_manager();
    let (runtime, enabled) = parse_runtime_from_request(req.config)?;
    manager.add_server(req.name.clone(), runtime, enabled).await;
    if enabled {
        manager
            .connect(&req.name)
            .await
            .map_err(mcp_error_to_api_error)?;
    }

    let servers = manager.list_servers().await;
    let mut result = HashMap::new();
    for server in servers {
        result.insert(server.name.clone(), McpStatusInfo::from(server));
    }
    Ok(Json(result))
}

pub(crate) async fn start_mcp_auth(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    let state = manager
        .start_oauth(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(serde_json::json!({
        "authorization_url": state.authorization_url,
        "client_id": state.client_id,
        "status": state.status
    })))
}

#[derive(Debug, Deserialize)]
pub struct McpAuthCallbackRequest {
    pub code: String,
}

async fn mcp_auth_callback(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<McpAuthCallbackRequest>,
) -> Result<Json<McpStatusInfo>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    let server_info = manager
        .handle_callback(&name, &req.code)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpStatusInfo::from(server_info)))
}

pub(crate) async fn mcp_authenticate(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<McpStatusInfo>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    let server_info = manager
        .authenticate(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpStatusInfo::from(server_info)))
}

pub(crate) async fn remove_mcp_auth(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    manager.remove_oauth(&name).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub(crate) async fn connect_mcp(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    manager
        .connect(&name)
        .await
        .map_err(mcp_error_to_api_error)?;
    Ok(Json(true))
}

pub(crate) async fn disconnect_mcp(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    manager
        .disconnect(&name)
        .await
        .map_err(mcp_error_to_api_error)?;
    Ok(Json(true))
}

#[derive(Debug, Serialize)]
pub struct McpLogsResponse {
    pub name: String,
    pub logs: Vec<McpServerLogEntry>,
}

async fn get_mcp_logs(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<McpLogsResponse>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    let logs = manager
        .get_logs(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpLogsResponse { name, logs }))
}

#[derive(Debug, Serialize)]
pub struct McpRestartResponse {
    pub success: bool,
    pub server: McpStatusInfo,
}

async fn restart_mcp(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<McpRestartResponse>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    let server = manager
        .restart(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpRestartResponse {
        success: true,
        server: server.into(),
    }))
}

fn mcp_error_to_api_error(error: McpOAuthError) -> ApiError {
    match error {
        McpOAuthError::ServerNotFound(name) => {
            ApiError::NotFound(format!("MCP server not found: {}", name))
        }
        McpOAuthError::OAuthNotSupported(name) => {
            ApiError::BadRequest(format!("MCP server does not support OAuth: {}", name))
        }
        McpOAuthError::OAuthInProgress => ApiError::BadRequest("OAuth already in progress".into()),
        McpOAuthError::OAuthFailed(message) => ApiError::BadRequest(message),
        McpOAuthError::RuntimeError(message) => ApiError::BadRequest(message),
    }
}

fn parse_runtime_from_request(config: McpConfigInput) -> Result<(McpRuntimeConfig, bool)> {
    let enabled = config.enabled.unwrap_or(true);
    let is_remote = matches!(config.server_type.as_deref(), Some("remote")) || config.url.is_some();

    if is_remote {
        let url = config
            .url
            .ok_or_else(|| ApiError::BadRequest("MCP remote config requires `url`".to_string()))?;
        let oauth_enabled = !matches!(config.oauth, Some(McpOAuthConfig::Disabled));
        let (client_id, authorization_url) = match config.oauth {
            Some(McpOAuthConfig::Config(oauth)) => (oauth.client_id, oauth.authorization_url),
            _ => (None, None),
        };
        return Ok((
            McpRuntimeConfig::Remote(RemoteMcpConfig {
                url,
                oauth_enabled,
                client_id,
                authorization_url,
            }),
            enabled,
        ));
    }

    let mut command = config.command.unwrap_or_default().into_iter();
    let program = command
        .next()
        .filter(|program| !program.trim().is_empty())
        .ok_or_else(|| {
            ApiError::BadRequest("MCP local config requires non-empty `command`".to_string())
        })?;
    Ok((
        McpRuntimeConfig::Local(LocalMcpConfig {
            command: program,
            args: command.collect(),
            env: config.environment,
            timeout: config.timeout,
        }),
        enabled,
    ))
}

fn parse_runtime_from_loaded_config(
    config: LoadedMcpServerConfig,
) -> Result<Option<(McpRuntimeConfig, bool)>> {
    match config {
        LoadedMcpServerConfig::Enabled { .. } => Ok(None),
        LoadedMcpServerConfig::Full(server) => {
            let server = *server;
            let enabled = server.enabled.unwrap_or(true);

            if let Some(url) = server.url {
                let (oauth_enabled, client_id, authorization_url) = match server.oauth {
                    Some(McpOAuthConfig::Disabled) => (false, None, None),
                    Some(McpOAuthConfig::Config(oauth)) => {
                        (true, oauth.client_id, oauth.authorization_url)
                    }
                    _ => (true, None, None),
                };
                return Ok(Some((
                    McpRuntimeConfig::Remote(RemoteMcpConfig {
                        url,
                        oauth_enabled,
                        client_id,
                        authorization_url,
                    }),
                    enabled,
                )));
            }

            if !server.command.is_empty() {
                let mut cmd_iter = server.command.into_iter();
                let command = cmd_iter.next().unwrap();
                return Ok(Some((
                    McpRuntimeConfig::Local(LocalMcpConfig {
                        command,
                        args: cmd_iter.collect(),
                        env: server.environment,
                        timeout: server.timeout,
                    }),
                    enabled,
                )));
            }

            Ok(None)
        }
    }
}

async fn sync_mcp_from_disk(
    manager: &McpOAuthManager,
    config_store: &agendao_config::ConfigStore,
) -> Result<()> {
    let config = (*config_store.config()).clone();

    let Some(mcp_map) = config.mcp else {
        return Ok(());
    };

    for (name, server_config) in mcp_map {
        if manager.has_server(&name).await {
            continue;
        }
        if let Some((runtime, enabled)) = parse_runtime_from_loaded_config(server_config)? {
            manager.add_server(name, runtime, enabled).await;
        }
    }

    Ok(())
}

async fn ensure_mcp_server_registered(
    manager: &McpOAuthManager,
    name: &str,
    config_store: &agendao_config::ConfigStore,
) -> Result<()> {
    if manager.has_server(name).await {
        return Ok(());
    }

    sync_mcp_from_disk(manager, config_store).await?;
    if manager.has_server(name).await {
        return Ok(());
    }

    Err(ApiError::NotFound(format!(
        "MCP server not found: {}",
        name
    )))
}
