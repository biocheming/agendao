use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{ApiError, Result, ServerState};
use agendao_plugin::subprocess::{PluginAuthBridge, PluginLoader};

// Re-use the global plugin loader accessor from the parent module.
use super::get_plugin_loader;

pub(crate) fn plugin_auth_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/auth", get(list_plugin_auth))
        .route("/{name}/auth/authorize", post(plugin_auth_authorize))
        .route("/{name}/auth/callback", post(plugin_auth_callback))
        .route("/{name}/auth/load", post(plugin_auth_load))
        .route("/{name}/auth/fetch", post(plugin_auth_fetch))
}

#[derive(Debug, Serialize)]
struct PluginAuthInfo {
    provider: String,
    methods: Vec<PluginAuthMethodInfo>,
}

#[derive(Debug, Serialize)]
struct PluginAuthMethodInfo {
    #[serde(rename = "type")]
    method_type: String,
    label: String,
}

async fn list_plugin_auth(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<PluginAuthInfo>>> {
    let _ = ensure_plugin_loader_active(&state).await?;
    let Some(loader) = get_plugin_loader() else {
        return Ok(Json(Vec::new()));
    };

    let bridges = loader.auth_bridges().await;
    let result: Vec<PluginAuthInfo> = bridges
        .values()
        .map(|bridge| PluginAuthInfo {
            provider: bridge.provider().to_string(),
            methods: bridge
                .methods()
                .iter()
                .map(|m| PluginAuthMethodInfo {
                    method_type: m.method_type.clone(),
                    label: m.label.clone(),
                })
                .collect(),
        })
        .collect();

    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
struct PluginAuthAuthorizeRequest {
    method: usize,
    #[serde(default)]
    inputs: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct PluginAuthAuthorizeResponse {
    url: Option<String>,
    instructions: Option<String>,
    method: Option<String>,
}

async fn plugin_auth_authorize(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginAuthAuthorizeRequest>,
) -> Result<Json<PluginAuthAuthorizeResponse>> {
    let bridge = get_auth_bridge(&state, &name).await?;

    let result = bridge
        .authorize(req.method, req.inputs)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(PluginAuthAuthorizeResponse {
        url: result.url,
        instructions: result.instructions,
        method: result.method,
    }))
}

#[derive(Debug, Deserialize)]
struct PluginAuthCallbackRequest {
    code: Option<String>,
}

async fn plugin_auth_callback(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginAuthCallbackRequest>,
) -> Result<Json<serde_json::Value>> {
    let bridge = get_auth_bridge(&state, &name).await?;

    let result = bridge
        .callback(req.code.as_deref())
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(result))
}

#[derive(Debug, Serialize)]
struct PluginAuthLoadResponse {
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    #[serde(rename = "hasCustomFetch")]
    has_custom_fetch: bool,
}

async fn plugin_auth_load(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<PluginAuthLoadResponse>> {
    let bridge = get_auth_bridge(&state, &name).await?;

    let result = bridge
        .load()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(PluginAuthLoadResponse {
        api_key: result.api_key,
        has_custom_fetch: result.has_custom_fetch,
    }))
}

#[derive(Debug, Deserialize)]
struct PluginAuthFetchRequest {
    url: String,
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
}

#[derive(Debug, Serialize)]
struct PluginAuthFetchResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

async fn plugin_auth_fetch(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginAuthFetchRequest>,
) -> Result<Json<PluginAuthFetchResponse>> {
    let bridge = get_auth_bridge(&state, &name).await?;

    let fetch_req = agendao_plugin::subprocess::PluginFetchRequest {
        url: req.url,
        method: req.method,
        headers: req.headers,
        body: req.body,
    };

    let result = bridge
        .fetch_proxy(fetch_req)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(PluginAuthFetchResponse {
        status: result.status,
        headers: result.headers,
        body: result.body,
    }))
}

/// Helper: look up the auth bridge for a provider name.
async fn get_auth_bridge(
    state: &Arc<ServerState>,
    provider: &str,
) -> Result<Arc<PluginAuthBridge>> {
    let _ = ensure_plugin_loader_active(state).await?;
    let loader = get_plugin_loader()
        .ok_or_else(|| ApiError::NotFound("no plugin loader initialized".into()))?;

    loader
        .auth_bridge(provider)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("no auth plugin for provider: {}", provider)))
}

pub(crate) async fn ensure_plugin_loader_active(
    state: &Arc<ServerState>,
) -> Result<Option<&'static Arc<PluginLoader>>> {
    let Some(loader) = get_plugin_loader() else {
        return Ok(None);
    };
    let started = loader
        .ensure_started()
        .await
        .map_err(|e| ApiError::InternalError(format!("failed to start plugin loader: {}", e)))?;
    if started {
        let _any_custom_fetch =
            crate::server::refresh_plugin_auth_state(loader, state.auth_manager.clone()).await;
    }
    loader.touch_activity();
    Ok(Some(loader))
}
