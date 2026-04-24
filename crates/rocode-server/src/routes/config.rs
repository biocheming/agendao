use axum::{
    extract::{Path, State},
    routing::{get, put},
    Json, Router,
};
use rocode_orchestrator::SchedulerConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

use crate::session_runtime::events::broadcast_config_updated;
use crate::{Result, ServerState};
use rocode_config::{
    Config as AppConfig, McpServerConfig, ModelConfig, PluginConfig, ProviderConfig,
};

pub(crate) fn config_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(get_config).patch(patch_config))
        .route("/providers", get(get_config_providers))
        .route(
            "/provider/{key}",
            put(put_provider_config).delete(delete_provider_config),
        )
        .route(
            "/provider/{key}/models/{model_key}",
            put(put_provider_model_config).delete(delete_provider_model_config),
        )
        .route(
            "/plugin/{key}",
            put(put_plugin_config).delete(delete_plugin_config),
        )
        .route("/mcp/{key}", put(put_mcp_config).delete(delete_mcp_config))
        .route(
            "/scheduler",
            get(get_scheduler_config).put(put_scheduler_config),
        )
}

async fn get_config(State(state): State<Arc<ServerState>>) -> Result<Json<AppConfig>> {
    let config = state.config_store.config();
    Ok(Json((*config).clone()))
}

async fn patch_config(
    State(state): State<Arc<ServerState>>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .patch(patch)
        .map_err(|e| crate::ApiError::BadRequest(e.to_string()))?;
    state.rebuild_providers().await;
    state.config_store.invalidate_plugin_cache().await;
    broadcast_config_updated(state.as_ref());
    // Invalidate mode caches so next request rebuilds with new config
    *crate::routes::AGENT_LIST_CACHE.write().await = None;
    *crate::routes::MODE_LIST_CACHE.write().await = None;
    Ok(Json((*updated).clone()))
}

async fn finalize_config_change(
    state: &ServerState,
    updated: Arc<AppConfig>,
) -> Result<Json<AppConfig>> {
    state.rebuild_providers().await;
    state.config_store.invalidate_plugin_cache().await;
    broadcast_config_updated(state);
    *crate::routes::AGENT_LIST_CACHE.write().await = None;
    *crate::routes::MODE_LIST_CACHE.write().await = None;
    Ok(Json((*updated).clone()))
}

#[derive(Debug, Serialize)]
pub struct ConfigProvidersResponse {
    pub providers: Vec<crate::routes::provider::ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
}

async fn get_config_providers(
    State(state): State<Arc<ServerState>>,
) -> Json<ConfigProvidersResponse> {
    let variant_lookup = crate::routes::provider::get_model_variant_lookup(state.as_ref()).await;
    let models = state.providers.read().await.list_models();
    let mut provider_names: HashMap<String, String> = HashMap::new();
    let mut provider_model_map: HashMap<
        String,
        HashMap<String, crate::routes::provider::ModelInfo>,
    > = HashMap::new();
    for m in models {
        let provider_id = m.provider.clone();
        let model_id = m.id.clone();
        provider_names
            .entry(provider_id.clone())
            .or_insert_with(|| provider_id.clone());
        let variants =
            crate::routes::provider::variants_for_model(&variant_lookup, &provider_id, &model_id);
        crate::routes::provider::upsert_runtime_model_info(
            &mut provider_model_map,
            &provider_id,
            crate::routes::provider::runtime_model_info(&m, variants),
        );
    }
    let config = state.config_store.config();
    if let Some(configured_providers) = &config.provider {
        for (provider_id, provider) in configured_providers {
            provider_names
                .entry(provider_id.clone())
                .or_insert_with(|| provider.name.clone().unwrap_or_else(|| provider_id.clone()));
            if let Some(models) = &provider.models {
                for (configured_model_key, configured_model) in models {
                    let model_id = configured_model
                        .model
                        .clone()
                        .unwrap_or_else(|| configured_model_key.clone());
                    let variants = configured_model
                        .variants
                        .as_ref()
                        .map(|items| items.keys().cloned().collect::<Vec<_>>())
                        .filter(|items| !items.is_empty())
                        .unwrap_or_else(|| {
                            crate::routes::provider::variants_for_model(
                                &variant_lookup,
                                provider_id,
                                &model_id,
                            )
                        });
                    crate::routes::provider::upsert_config_model_info(
                        &mut provider_model_map,
                        provider_id,
                        crate::routes::provider::configured_model_info(
                            provider_id,
                            model_id,
                            configured_model,
                            variants,
                        ),
                    );
                }
            }
        }
    }
    for provider_id in provider_names.keys() {
        provider_model_map.entry(provider_id.clone()).or_default();
    }
    let provider_map: HashMap<String, Vec<crate::routes::provider::ModelInfo>> = provider_model_map
        .into_iter()
        .map(|(provider_id, model_map)| {
            let mut entries = model_map.into_values().collect::<Vec<_>>();
            entries.sort_by(|a, b| a.id.cmp(&b.id));
            (provider_id, entries)
        })
        .collect();
    let providers: Vec<crate::routes::provider::ProviderInfo> = provider_map
        .into_iter()
        .map(|(id, models)| crate::routes::provider::ProviderInfo {
            id: id.clone(),
            name: provider_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| id.clone()),
            models,
        })
        .collect();
    let default_model: HashMap<String, String> = providers
        .iter()
        .filter_map(|p| p.models.first().map(|m| (p.id.clone(), m.id.clone())))
        .collect();
    Json(ConfigProvidersResponse {
        providers,
        default_model,
    })
}

async fn put_provider_config(
    State(state): State<Arc<ServerState>>,
    Path(key): Path<String>,
    Json(provider): Json<ProviderConfig>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            let provider_map = config.provider.get_or_insert_with(HashMap::new);
            provider_map.insert(key.clone(), provider);
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

async fn delete_provider_config(
    State(state): State<Arc<ServerState>>,
    Path(key): Path<String>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            let provider_map = config.provider.get_or_insert_with(HashMap::new);
            provider_map.remove(&key);
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

async fn put_provider_model_config(
    State(state): State<Arc<ServerState>>,
    Path((key, model_key)): Path<(String, String)>,
    Json(model): Json<ModelConfig>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            let provider_map = config.provider.get_or_insert_with(HashMap::new);
            let provider = provider_map
                .entry(key.clone())
                .or_insert_with(ProviderConfig::default);
            let models = provider.models.get_or_insert_with(HashMap::new);
            if let Some(existing) = models.get_mut(&model_key) {
                merge_model_config(existing, model);
            } else {
                models.insert(model_key.clone(), model);
            }
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

async fn delete_provider_model_config(
    State(state): State<Arc<ServerState>>,
    Path((key, model_key)): Path<(String, String)>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            let provider_map = config.provider.get_or_insert_with(HashMap::new);
            if let Some(provider) = provider_map.get_mut(&key) {
                if let Some(models) = provider.models.as_mut() {
                    models.remove(&model_key);
                }
            }
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

async fn put_plugin_config(
    State(state): State<Arc<ServerState>>,
    Path(key): Path<String>,
    Json(plugin): Json<PluginConfig>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            config.plugin.insert(key.clone(), plugin);
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

fn merge_model_config(existing: &mut ModelConfig, patch: ModelConfig) {
    let ModelConfig {
        name,
        model,
        api_key,
        base_url,
        variants,
        tool_call,
        modalities,
        reasoning,
        attachment,
        temperature,
        interleaved,
        options,
        cost,
        limit,
        headers,
        family,
        status,
        release_date,
        experimental,
        provider,
    } = patch;

    if let Some(value) = name {
        existing.name = Some(value);
    }
    if let Some(value) = model {
        existing.model = Some(value);
    }
    if let Some(value) = api_key {
        existing.api_key = Some(value);
    }
    if let Some(value) = base_url {
        existing.base_url = Some(value);
    }
    if let Some(value) = variants {
        existing.variants = Some(value);
    }
    if let Some(value) = tool_call {
        existing.tool_call = Some(value);
    }
    if let Some(value) = modalities {
        existing.modalities = Some(value);
    }
    if let Some(value) = reasoning {
        existing.reasoning = Some(value);
    }
    if let Some(value) = attachment {
        existing.attachment = Some(value);
    }
    if let Some(value) = temperature {
        existing.temperature = Some(value);
    }
    if let Some(value) = interleaved {
        existing.interleaved = Some(value);
    }
    if let Some(value) = options {
        existing.options = Some(value);
    }
    if let Some(value) = cost {
        existing.cost = Some(value);
    }
    if let Some(value) = limit {
        existing.limit = Some(value);
    }
    if let Some(value) = headers {
        existing.headers = Some(value);
    }
    if let Some(value) = family {
        existing.family = Some(value);
    }
    if let Some(value) = status {
        existing.status = Some(value);
    }
    if let Some(value) = release_date {
        existing.release_date = Some(value);
    }
    if let Some(value) = experimental {
        existing.experimental = Some(value);
    }
    if let Some(value) = provider {
        existing.provider = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::merge_model_config;
    use rocode_config::ModelConfig;
    use std::collections::HashMap;

    #[test]
    fn merge_model_config_preserves_unedited_fields() {
        let mut existing = ModelConfig {
            name: Some("Existing Name".to_string()),
            model: Some("existing-id".to_string()),
            base_url: Some("https://example.invalid".to_string()),
            headers: Some(HashMap::from([(
                "Authorization".to_string(),
                "Bearer secret".to_string(),
            )])),
            options: Some(HashMap::from([(
                "tier".to_string(),
                serde_json::json!("premium"),
            )])),
            ..ModelConfig::default()
        };

        let patch = ModelConfig {
            name: Some("Updated Name".to_string()),
            reasoning: Some(true),
            ..ModelConfig::default()
        };

        merge_model_config(&mut existing, patch);

        assert_eq!(existing.name.as_deref(), Some("Updated Name"));
        assert_eq!(existing.model.as_deref(), Some("existing-id"));
        assert_eq!(
            existing.base_url.as_deref(),
            Some("https://example.invalid")
        );
        assert_eq!(
            existing
                .headers
                .as_ref()
                .and_then(|headers| headers.get("Authorization"))
                .map(String::as_str),
            Some("Bearer secret")
        );
        assert_eq!(
            existing
                .options
                .as_ref()
                .and_then(|options| options.get("tier")),
            Some(&serde_json::json!("premium"))
        );
        assert_eq!(existing.reasoning, Some(true));
    }

    #[test]
    fn merge_model_config_replaces_explicit_fields() {
        let mut existing = ModelConfig {
            reasoning: Some(true),
            headers: Some(HashMap::from([("X-Old".to_string(), "1".to_string())])),
            ..ModelConfig::default()
        };

        let patch = ModelConfig {
            reasoning: Some(false),
            headers: Some(HashMap::from([("X-New".to_string(), "2".to_string())])),
            ..ModelConfig::default()
        };

        merge_model_config(&mut existing, patch);

        assert_eq!(existing.reasoning, Some(false));
        assert_eq!(
            existing
                .headers
                .as_ref()
                .and_then(|headers| headers.get("X-New"))
                .map(String::as_str),
            Some("2")
        );
        assert!(existing
            .headers
            .as_ref()
            .is_some_and(|headers| !headers.contains_key("X-Old")));
    }
}

async fn delete_plugin_config(
    State(state): State<Arc<ServerState>>,
    Path(key): Path<String>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            config.plugin.remove(&key);
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

async fn put_mcp_config(
    State(state): State<Arc<ServerState>>,
    Path(key): Path<String>,
    Json(mcp): Json<McpServerConfig>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            let mcp_map = config.mcp.get_or_insert_with(HashMap::new);
            mcp_map.insert(key.clone(), mcp);
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

async fn delete_mcp_config(
    State(state): State<Arc<ServerState>>,
    Path(key): Path<String>,
) -> Result<Json<AppConfig>> {
    let updated = state
        .config_store
        .replace_with(|config| {
            let mcp_map = config.mcp.get_or_insert_with(HashMap::new);
            mcp_map.remove(&key);
            Ok(())
        })
        .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    finalize_config_change(&state, updated).await
}

#[derive(Debug, Serialize)]
pub struct SchedulerConfigResponse {
    #[serde(rename = "path")]
    pub raw_path: Option<String>,
    #[serde(rename = "resolvedPath")]
    pub resolved_path: Option<String>,
    pub exists: bool,
    pub content: String,
    #[serde(rename = "defaultProfile")]
    pub default_profile: Option<String>,
    pub profiles: Vec<SchedulerProfileSummary>,
    #[serde(rename = "parseError")]
    pub parse_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SchedulerProfileSummary {
    pub key: String,
    pub orchestrator: Option<String>,
    pub description: Option<String>,
    pub stages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PutSchedulerConfigRequest {
    #[serde(default)]
    path: Option<String>,
    content: String,
}

fn summarize_scheduler_profiles(
    content: &str,
) -> (Option<String>, Vec<SchedulerProfileSummary>, Option<String>) {
    match SchedulerConfig::load_from_str(content) {
        Ok(config) => {
            let mut profiles = config
                .profiles
                .into_iter()
                .map(|(key, profile)| SchedulerProfileSummary {
                    key,
                    orchestrator: profile.orchestrator,
                    description: profile.description,
                    stages: profile
                        .stages
                        .into_iter()
                        .map(|stage| stage.kind().event_name().to_string())
                        .collect(),
                })
                .collect::<Vec<_>>();
            profiles.sort_by(|a, b| a.key.cmp(&b.key));
            (
                config
                    .defaults
                    .and_then(|defaults| defaults.profile)
                    .filter(|value| !value.trim().is_empty()),
                profiles,
                None,
            )
        }
        Err(error) => (None, Vec::new(), Some(error.to_string())),
    }
}

async fn scheduler_config_response(
    state: &Arc<ServerState>,
) -> Result<Json<SchedulerConfigResponse>> {
    let config = state.config_store.config();
    let raw_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let resolved_path = state.config_store.resolved_scheduler_path().await;

    let (exists, content) = match resolved_path.as_ref() {
        Some(path) => match fs::read_to_string(path).await {
            Ok(content) => (true, content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
            Err(error) => {
                return Err(crate::ApiError::InternalError(format!(
                    "failed to read scheduler config: {error}"
                )));
            }
        },
        None => (false, String::new()),
    };

    let (default_profile, profiles, parse_error) = if content.trim().is_empty() {
        (None, Vec::new(), None)
    } else {
        summarize_scheduler_profiles(&content)
    };

    Ok(Json(SchedulerConfigResponse {
        raw_path,
        resolved_path: resolved_path.map(|path| path.display().to_string()),
        exists,
        content,
        default_profile,
        profiles,
        parse_error,
    }))
}

async fn get_scheduler_config(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<SchedulerConfigResponse>> {
    scheduler_config_response(&state).await
}

fn resolve_scheduler_write_target(
    state: &Arc<ServerState>,
    requested_path: Option<String>,
) -> Result<(String, PathBuf)> {
    let raw_path = requested_path
        .or_else(|| state.config_store.config().scheduler_path.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".rocode/scheduler.jsonc".to_string());

    let path = PathBuf::from(&raw_path);
    if path.is_absolute() {
        return Ok((raw_path, path));
    }

    let project_dir = state.config_store.project_dir().ok_or_else(|| {
        crate::ApiError::BadRequest("scheduler config requires a project directory".to_string())
    })?;
    Ok((raw_path, project_dir.join(path)))
}

async fn put_scheduler_config(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<PutSchedulerConfigRequest>,
) -> Result<Json<SchedulerConfigResponse>> {
    let (raw_path, resolved_path) = resolve_scheduler_write_target(&state, request.path)?;

    if let Some(parent) = resolved_path.parent() {
        fs::create_dir_all(parent).await.map_err(|error| {
            crate::ApiError::InternalError(format!(
                "failed to create scheduler config directory: {error}"
            ))
        })?;
    }

    fs::write(&resolved_path, &request.content)
        .await
        .map_err(|error| {
            crate::ApiError::InternalError(format!("failed to write scheduler config: {error}"))
        })?;

    if state.config_store.config().scheduler_path.as_deref() != Some(raw_path.as_str()) {
        state
            .config_store
            .patch(serde_json::json!({ "schedulerPath": raw_path }))
            .map_err(|error| crate::ApiError::BadRequest(error.to_string()))?;
    }

    state.config_store.invalidate_plugin_cache().await;
    broadcast_config_updated(state.as_ref());
    *crate::routes::AGENT_LIST_CACHE.write().await = None;
    *crate::routes::MODE_LIST_CACHE.write().await = None;

    scheduler_config_response(&state).await
}
