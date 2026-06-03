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
use agendao_config::McpServerConfig as LoadedMcpServerConfig;

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

async fn get_mcp_status(
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
#[serde(untagged)]
pub enum McpCommandInput {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub struct McpConfigInput {
    #[serde(rename = "type")]
    pub server_type: Option<String>,
    pub command: Option<McpCommandInput>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub environment: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub timeout: Option<u64>,
    pub oauth: Option<serde_json::Value>,
    pub client_id: Option<String>,
    pub authorization_url: Option<String>,
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

async fn start_mcp_auth(
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

async fn mcp_authenticate(
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

async fn remove_mcp_auth(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name, &state.config_store).await?;
    manager.remove_oauth(&name).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

async fn connect_mcp(
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

async fn disconnect_mcp(
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
        let oauth_enabled = !matches!(config.oauth, Some(serde_json::Value::Bool(false)));
        return Ok((
            McpRuntimeConfig::Remote(RemoteMcpConfig {
                url,
                oauth_enabled,
                client_id: config.client_id,
                authorization_url: config.authorization_url,
            }),
            enabled,
        ));
    }

    let (command, args) = parse_command_and_args(config.command, config.args.unwrap_or_default())?;
    Ok((
        McpRuntimeConfig::Local(LocalMcpConfig {
            command,
            args,
            env: config.env.or(config.environment),
            timeout: config.timeout,
        }),
        enabled,
    ))
}

fn parse_command_and_args(
    command: Option<McpCommandInput>,
    extra_args: Vec<String>,
) -> Result<(String, Vec<String>)> {
    match command {
        Some(McpCommandInput::String(cmd)) => {
            if cmd.trim().is_empty() {
                return Err(ApiError::BadRequest(
                    "MCP local config `command` cannot be empty".to_string(),
                ));
            }
            Ok((cmd, extra_args))
        }
        Some(McpCommandInput::Array(parts)) => {
            let mut iter = parts.into_iter();
            let cmd = iter
                .next()
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "MCP local config `command` array cannot be empty".to_string(),
                    )
                })?
                .trim()
                .to_string();
            if cmd.is_empty() {
                return Err(ApiError::BadRequest(
                    "MCP local config `command` cannot be empty".to_string(),
                ));
            }
            let mut args: Vec<String> = iter.collect();
            args.extend(extra_args);
            Ok((cmd, args))
        }
        None => Err(ApiError::BadRequest(
            "MCP local config requires `command`".to_string(),
        )),
    }
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
                return Ok(Some((
                    McpRuntimeConfig::Remote(RemoteMcpConfig {
                        url,
                        oauth_enabled: true,
                        client_id: server.client_id,
                        authorization_url: server.authorization_url,
                    }),
                    enabled,
                )));
            }

            if !server.command.is_empty() {
                let mut cmd_iter = server.command.into_iter();
                let command = cmd_iter.next().unwrap();
                let mut args: Vec<String> = cmd_iter.collect();
                args.extend(server.args);
                return Ok(Some((
                    McpRuntimeConfig::Local(LocalMcpConfig {
                        command,
                        args,
                        env: server.env,
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
