use agendao_orchestrator::SchedulerConfig;
use axum::{
    extract::{Path, State},
    routing::{get, put},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

use crate::session_runtime::events::broadcast_config_updated;
use crate::{Result, ServerState};
use agendao_config::{
    Config as AppConfig, McpServerConfig, ModelConfig, PluginConfig, ProviderConfig,
};
use agendao_types::{
    ConfigPolicyValidationEffect, ConfigPolicyValidationItem, ConfigPolicyValidationOwner,
    ConfigPolicyValidationScope, ConfigPolicyValidationScopeKind, ConfigPolicyValidationSeverity,
    ConfigPolicyValidationSnapshot,
};

use super::external_adapter::collect_external_adapter_config_validation;
use super::provider::collect_provider_profile_validation;
use super::session::collect_skill_tree_validation;

pub(crate) fn config_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(get_config).patch(patch_config))
        .route("/disabled", put(put_disabled_config))
        .route("/validation", get(get_config_validation))
        .route("/providers", get(get_config_providers))
        .route(
            "/provider/{key}",
            put(put_provider_config).delete(delete_provider_config),
        )
        .route(
            "/provider/{key}/models/{model_key}",
            get(get_provider_model_config)
                .put(put_provider_model_config)
                .delete(delete_provider_model_config),
        )
        .route(
            "/plugin/{key}",
            put(put_plugin_config).delete(delete_plugin_config),
        )
        .route("/plugins", get(get_plugin_list))
        .route("/mcp/{key}", put(put_mcp_config).delete(delete_mcp_config))
        .route(
            "/scheduler",
            get(get_scheduler_config).put(put_scheduler_config),
        )
}

pub(crate) async fn get_config(State(state): State<Arc<ServerState>>) -> Result<Json<AppConfig>> {
    let config = state.config_store.config();
    Ok(Json((*config).clone()))
}

pub(crate) async fn get_config_validation(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<ConfigPolicyValidationSnapshot>> {
    Ok(Json(build_config_policy_validation_snapshot(&state).await))
}

pub(crate) async fn patch_config(
    State(state): State<Arc<ServerState>>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<AppConfig>> {
    // disabled_tools 经 generic patch 写入时也要即时生效（重建 tool registry）。
    let touches_disabled_tools = patch.get("disabled_tools").is_some();
    let updated = state
        .config_store
        .patch(patch)
        .map_err(|e| crate::ApiError::BadRequest(e.to_string()))?;
    state.rebuild_providers().await;
    if touches_disabled_tools {
        state.rebuild_tool_registry().await;
    }
    state.config_store.invalidate_plugin_cache().await;
    broadcast_config_updated(state.as_ref());
    // Invalidate mode caches so next request rebuilds with new config
    *crate::routes::AGENT_LIST_CACHE.write().await = None;
    *crate::routes::MODE_LIST_CACHE.write().await = None;
    Ok(Json((*updated).clone()))
}

/// PUT `/config/disabled`：整体替换 `disabled_tools` / `skills.disabled` /
/// `disabled_plugins` 列表。
///
/// 与 `PATCH /config` 的关键差异：`Some(vec)` 允许空 vec **清空**列表——
/// patch 的 merge 语义（`merge_vec_replace_if_non_empty`）无法表达清空，
/// 而 Settings 启停开关需要能移除最后一个条目。`None`/缺省字段不动。
/// tools 变更后重建 tool registry（即时生效，无需重启）。
pub(crate) async fn put_disabled_config(
    State(state): State<Arc<ServerState>>,
    Json(update): Json<agendao_api::DisabledConfigUpdate>,
) -> Result<Json<AppConfig>> {
    let touches_tools = update.tools.is_some();
    let updated = state
        .config_store
        .replace_with(|config| {
            if let Some(tools) = &update.tools {
                config.disabled_tools = tools.clone();
            }
            if let Some(skills) = &update.skills {
                config
                    .skills
                    .get_or_insert_with(Default::default)
                    .disabled = skills.clone();
            }
            if let Some(plugins) = &update.plugins {
                config.disabled_plugins = plugins.clone();
            }
            Ok(())
        })
        .map_err(|e| crate::ApiError::BadRequest(e.to_string()))?;
    if touches_tools {
        state.rebuild_tool_registry().await;
    }
    state.config_store.invalidate_plugin_cache().await;
    broadcast_config_updated(state.as_ref());
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

pub(crate) async fn get_config_providers(
    State(state): State<Arc<ServerState>>,
) -> Json<ConfigProvidersResponse> {
    let variant_lookup = crate::routes::provider::get_model_variant_lookup(state.as_ref()).await;
    let models = state.providers.read().await.list_models();
    let mut provider_names: HashMap<String, String> = HashMap::new();
    let mut provider_base_urls: HashMap<String, String> = HashMap::new();
    // protocol 映射(土律单点):此端点只读 config(无 catalog 兜底),所以
    // 直接从 provider.npm 反推。`/provider` 端点有 catalog fallback,本端点没。
    let mut provider_protocols: HashMap<String, String> = HashMap::new();
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
            if let Some(base) = provider.base_url.as_ref().filter(|s| !s.is_empty()) {
                provider_base_urls
                    .entry(provider_id.clone())
                    .or_insert_with(|| base.clone());
            }
            if let Some(proto) = provider
                .npm
                .as_deref()
                .and_then(crate::routes::provider::npm_to_protocol)
            {
                provider_protocols
                    .entry(provider_id.clone())
                    .or_insert_with(|| proto.to_string());
            }
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
            name: provider_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| id.clone()),
            base_url: provider_base_urls.get(&id).cloned(),
            protocol: provider_protocols.get(&id).cloned(),
            disabled: state
                .config_store
                .config()
                .disabled_providers
                .contains(&id),
            id: id.clone(),
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

/// GET 单个 model 的 raw `ModelConfig`,用于 TUI Edit 模式 prefill。
/// 没有就 404——TUI 端 keymap 必须收到诚实信号(不假装空白),避免 PUT 半空覆写
/// 丢字段(土律·第十条 可观测性权利)。
pub(crate) async fn get_provider_model_config(
    State(state): State<Arc<ServerState>>,
    Path((key, model_key)): Path<(String, String)>,
) -> Result<Json<ModelConfig>> {
    let config = state.config_store.config();
    let model = config
        .provider
        .as_ref()
        .and_then(|p| p.get(&key))
        .and_then(|p| p.models.as_ref())
        .and_then(|m| m.get(&model_key))
        .cloned()
        .ok_or_else(|| {
            crate::ApiError::NotFound(format!(
                "model `{}` not found in provider `{}`",
                model_key, key
            ))
        })?;
    Ok(Json(model))
}

pub(crate) async fn put_provider_model_config(
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

pub(crate) async fn delete_provider_model_config(
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

pub(crate) async fn put_plugin_config(
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
        reasoning_effort,
        timeout_secs,
        stream_stall_timeout_secs,
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
    if let Some(value) = reasoning_effort {
        existing.reasoning_effort = Some(value);
    }
    if let Some(value) = timeout_secs {
        existing.timeout_secs = Some(value);
    }
    if let Some(value) = stream_stall_timeout_secs {
        existing.stream_stall_timeout_secs = Some(value);
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
    use super::{build_config_policy_validation_snapshot, merge_model_config};
    use crate::ServerState;
    use agendao_config::{
        CompositionConfig, Config, ConfigStore, ExternalAdapterConfig, ExternalAdapterEntryConfig,
        ModelConfig, ProviderConfig, SkillTreeConfig, SkillTreeNodeConfig,
    };
    use agendao_types::{
        ConfigPolicyValidationEffect, ConfigPolicyValidationOwner, ConfigPolicyValidationSeverity,
    };
    use axum::{extract::State, Json};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;

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

    fn validation_state(config: Config) -> Arc<ServerState> {
        let mut state = ServerState::new();
        state.config_store = Arc::new(ConfigStore::new(config));
        Arc::new(state)
    }

    #[tokio::test]
    async fn plugin_list_entries_mark_managed_and_disabled() {
        use agendao_config::PluginConfig;

        let state = validation_state(Config {
            plugin: HashMap::from([
                (
                    "alpha".to_string(),
                    PluginConfig {
                        plugin_type: "npm".to_string(),
                        package: Some("alpha-pkg".to_string()),
                        version: Some("1.2.3".to_string()),
                        ..Default::default()
                    },
                ),
                (
                    "beta".to_string(),
                    PluginConfig {
                        plugin_type: "file".to_string(),
                        path: Some("/nonexistent/beta.ts".to_string()),
                        ..Default::default()
                    },
                ),
            ]),
            disabled_plugins: vec!["alpha".to_string()],
            ..Default::default()
        });

        let entries = super::build_plugin_list_entries(&state).await;
        let alpha = entries
            .iter()
            .find(|e| e.name == "alpha")
            .expect("alpha entry");
        assert_eq!(alpha.source, "managed");
        assert_eq!(alpha.origin, "config (declared)");
        assert_eq!(alpha.version.as_deref(), Some("1.2.3"));
        assert!(alpha.disabled, "disabled_plugins 精确名命中");

        let beta = entries.iter().find(|e| e.name == "beta").expect("beta entry");
        assert_eq!(beta.source, "managed");
        assert!(!beta.disabled);
    }

    #[tokio::test]
    async fn put_disabled_config_replaces_plugins_list() {
        let state = validation_state(Config {
            disabled_plugins: vec!["old".to_string()],
            ..Default::default()
        });
        let Json(updated) = super::put_disabled_config(
            State(state),
            Json(agendao_api::DisabledConfigUpdate {
                tools: None,
                skills: None,
                plugins: Some(vec!["new-a".to_string(), "new-b".to_string()]),
            }),
        )
        .await
        .expect("put_disabled_config");
        assert_eq!(updated.disabled_plugins, vec!["new-a", "new-b"]);
    }

    #[tokio::test]
    async fn put_disabled_config_none_leaves_plugins_untouched() {
        let state = validation_state(Config {
            disabled_plugins: vec!["keep".to_string()],
            ..Default::default()
        });
        let Json(updated) = super::put_disabled_config(
            State(state),
            Json(agendao_api::DisabledConfigUpdate {
                tools: Some(vec![]),
                skills: None,
                plugins: None,
            }),
        )
        .await
        .expect("put_disabled_config");
        assert_eq!(updated.disabled_plugins, vec!["keep"]);
        assert!(updated.disabled_tools.is_empty());
    }

    #[tokio::test]
    async fn config_validation_snapshot_collects_first_batch_owner_errors() {
        let dir = tempdir().expect("tempdir");
        let scheduler_path = dir.path().join("scheduler.jsonc");
        std::fs::write(&scheduler_path, "{ invalid jsonc").expect("write invalid scheduler config");

        let state = validation_state(Config {
            scheduler_path: Some(scheduler_path.display().to_string()),
            composition: Some(CompositionConfig {
                skill_tree: Some(SkillTreeConfig {
                    enabled: Some(true),
                    root: Some(SkillTreeNodeConfig {
                        node_id: "root".to_string(),
                        markdown_path: "skill://root".to_string(),
                        children: Vec::new(),
                    }),
                    truncation_strategy: Some("middle".to_string()),
                    ..Default::default()
                }),
            }),
            provider: Some(HashMap::from([(
                "broken".to_string(),
                ProviderConfig {
                    api_style: Some("closeai-compatible".to_string()),
                    api_shape: Some("messages".to_string()),
                    transport: Some("bearer".to_string()),
                    usage_shape: Some("closeai-cached-tokens".to_string()),
                    ..Default::default()
                },
            )])),
            external_adapter: Some(ExternalAdapterConfig {
                adapters: HashMap::from([(
                    "generic".to_string(),
                    ExternalAdapterEntryConfig {
                        enabled: Some(true),
                        source: Some("generic-webhook".to_string()),
                        ..Default::default()
                    },
                )]),
                replay: None,
            }),
            ..Default::default()
        });

        let snapshot = build_config_policy_validation_snapshot(&state).await;

        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.reports.len(), 4);

        let scheduler = snapshot
            .reports
            .iter()
            .find(|item| item.owner == ConfigPolicyValidationOwner::Scheduler)
            .expect("scheduler validation item");
        assert_eq!(scheduler.path, "scheduler_path");
        assert_eq!(scheduler.code, "scheduler_config_parse_error");
        assert_eq!(scheduler.severity, ConfigPolicyValidationSeverity::Error);
        assert_eq!(scheduler.effect, ConfigPolicyValidationEffect::SoftFallback);

        let skill_tree = snapshot
            .reports
            .iter()
            .find(|item| item.owner == ConfigPolicyValidationOwner::SkillTree)
            .expect("skill tree validation item");
        assert_eq!(
            skill_tree.path,
            "composition.skill_tree.truncation_strategy"
        );
        assert_eq!(skill_tree.code, "skill_tree_unknown_truncation_strategy");
        assert_eq!(skill_tree.fallback.as_deref(), Some("head-tail"));

        let provider = snapshot
            .reports
            .iter()
            .find(|item| item.owner == ConfigPolicyValidationOwner::ProviderProfile)
            .expect("provider validation item");
        assert_eq!(provider.path, "provider.broken");
        assert_eq!(provider.code, "provider_profile_invalid");
        assert_eq!(
            provider.effect,
            ConfigPolicyValidationEffect::FailClosedBootstrap
        );

        let adapter = snapshot
            .reports
            .iter()
            .find(|item| item.owner == ConfigPolicyValidationOwner::ExternalAdapter)
            .expect("external adapter validation item");
        assert_eq!(adapter.path, "external_adapter.adapters.generic.secret_ref");
        assert_eq!(adapter.code, "external_adapter_missing_secret_ref");
        assert_eq!(
            adapter.effect,
            ConfigPolicyValidationEffect::FailClosedRequestGate
        );
    }

    #[tokio::test]
    async fn config_validation_snapshot_is_empty_for_clean_config() {
        let state = validation_state(Config::default());
        let snapshot = build_config_policy_validation_snapshot(&state).await;

        assert_eq!(snapshot.revision, 0);
        assert!(snapshot.generated_at_ms > 0);
        assert!(snapshot.reports.is_empty());
    }
}

pub(crate) async fn delete_plugin_config(
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

pub(crate) async fn put_mcp_config(
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
    // 写后同步 runtime manager：启停/改配置即时生效（否则 manager 只增不改，
    // 状态列表残留旧值）。cfg 取自 updated（写面唯一权威）。
    #[cfg(feature = "mcp")]
    if let Some(cfg) = updated
        .mcp
        .as_ref()
        .and_then(|mcp_map| mcp_map.get(&key))
    {
        crate::routes::mcp::sync_manager_after_mcp_config_write(&key, Some(cfg)).await;
    }
    finalize_config_change(&state, updated).await
}

pub(crate) async fn delete_mcp_config(
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
    // 同步摘除 runtime manager 条目（否则已删 server 仍在状态列表残留）。
    #[cfg(feature = "mcp")]
    crate::routes::mcp::sync_manager_after_mcp_config_write(&key, None).await;
    finalize_config_change(&state, updated).await
}

// ── Plugins 列表读面（GET `/config/plugins` / local_list_plugins）──

async fn get_plugin_list(
    State(state): State<Arc<ServerState>>,
) -> Json<Vec<agendao_api::PluginListEntry>> {
    Json(build_plugin_list_entries(&state).await)
}

/// 插件根目录的安装途径标签（渲染/命中同源：列表 detail 与删除提示共用）。
/// 归类顺序与 `collect_plugin_roots` 的 roots 顺序一致：
///   1. `~/.agendao/plugins`（尊重 AGENDAO_HOME）→ user 级
///   2. 项目 `.agendao/plugins`（project_dir 之下、含 `.agendao` 组件）→ 项目级
///   3. config `plugin_paths` 声明的解析路径 → external
///   4. 其余（xdg 遗留等）→ 诚实展示原始路径
fn plugin_root_label(
    root: &std::path::Path,
    project_dir: &std::path::Path,
    plugin_path_roots: &[PathBuf],
) -> String {
    let home = agendao_util::agendao_home();
    if root == home.join("plugins") || root == home.join("plugin") {
        return "user (~/.agendao/plugins)".to_string();
    }
    if root.starts_with(project_dir)
        && root.components().any(|c| c.as_os_str() == ".agendao")
    {
        return "project (.agendao/plugins)".to_string();
    }
    if plugin_path_roots.iter().any(|p| p == root) {
        return "external (config plugin_paths)".to_string();
    }
    format!("external ({})", root.display())
}

/// 已安装插件全集 = config.plugin（managed）+ discovery 扫描（discovered）。
/// 与 tool_catalog `build_tool_list_entries` 同范式：server 单点权威，
/// HTTP 路由与 local-direct 短路共用。
///
/// managed/discovered 判别：discovery 扫描产出 (file stem key, 绝对路径) 集合；
/// config.plugin 中 type=file 且 path 命中该集合的条目标 discovered（其真相在
/// 目录里，删除要去对应目录），其余为 config 声明的 managed。
pub(crate) async fn build_plugin_list_entries(
    state: &Arc<ServerState>,
) -> Vec<agendao_api::PluginListEntry> {
    let config = state.config_store.config();
    let project_dir = state
        .config_store
        .project_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    let plugin_path_roots: Vec<PathBuf> = config
        .plugin_paths
        .values()
        .map(|raw| agendao_config::resolve_configured_path(&project_dir, raw))
        .collect();

    // discovered：key → (绝对路径, 安装途径标签)。先列的 root 优先（同 loader 合并序）。
    let mut discovered: HashMap<String, (String, String)> = HashMap::new();
    for root in agendao_config::collect_plugin_roots(&project_dir, &config.plugin_paths) {
        let label = plugin_root_label(&root, &project_dir, &plugin_path_roots);
        for spec in agendao_config::load_plugins_from_path(&root) {
            let (key, cfg) = PluginConfig::from_file_spec(&spec);
            if let Some(path) = cfg.path {
                discovered.entry(key).or_insert((path, label.clone()));
            }
        }
    }

    let disabled = &config.disabled_plugins;
    let mut entries: Vec<agendao_api::PluginListEntry> = Vec::new();
    for (name, cfg) in &config.plugin {
        let hit = discovered.get(name).filter(|(path, _)| {
            cfg.plugin_type == "file" && cfg.path.as_deref() == Some(path.as_str())
        });
        let (source, origin) = match hit {
            Some((_, label)) => ("discovered", label.clone()),
            None => ("managed", "config (declared)".to_string()),
        };
        entries.push(agendao_api::PluginListEntry {
            name: name.clone(),
            plugin_type: cfg.plugin_type.clone(),
            source: source.to_string(),
            version: cfg.version.clone(),
            path: cfg.path.clone(),
            origin,
            disabled: agendao_config::matching::matching_disabled_pattern(disabled, name)
                .is_some(),
        });
    }
    // discovery 扫到但不在 config.plugin 的条目（loader 合并会覆盖本情形，
    // 防御性补齐，保持「扫描到即可见」的读面语义）。
    for (name, (path, origin)) in &discovered {
        if config.plugin.contains_key(name) {
            continue;
        }
        entries.push(agendao_api::PluginListEntry {
            name: name.clone(),
            plugin_type: "file".to_string(),
            source: "discovered".to_string(),
            version: None,
            path: Some(path.clone()),
            origin: origin.clone(),
            disabled: agendao_config::matching::matching_disabled_pattern(disabled, name)
                .is_some(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
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

#[derive(Debug)]
struct SchedulerConfigInspection {
    raw_path: Option<String>,
    resolved_path: Option<String>,
    exists: bool,
    content: String,
    default_profile: Option<String>,
    profiles: Vec<SchedulerProfileSummary>,
    parse_error: Option<String>,
    read_error: Option<String>,
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

async fn inspect_scheduler_config(state: &Arc<ServerState>) -> SchedulerConfigInspection {
    let config = state.config_store.config();
    let raw_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let resolved_path = state.config_store.resolved_scheduler_path().await;
    let resolved_path_string = resolved_path
        .as_ref()
        .map(|path| path.display().to_string());

    let (exists, content, read_error) = match resolved_path.as_ref() {
        Some(path) => match fs::read_to_string(path).await {
            Ok(content) => (true, content, None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (false, String::new(), None)
            }
            Err(error) => (false, String::new(), Some(error.to_string())),
        },
        None if raw_path.is_some() => (
            false,
            String::new(),
            Some(
                "configured scheduler_path could not be resolved against a project directory"
                    .to_string(),
            ),
        ),
        None => (false, String::new(), None),
    };

    let (default_profile, profiles, parse_error) = if content.trim().is_empty() {
        (None, Vec::new(), None)
    } else {
        summarize_scheduler_profiles(&content)
    };

    SchedulerConfigInspection {
        raw_path,
        resolved_path: resolved_path_string,
        exists,
        content,
        default_profile,
        profiles,
        parse_error,
        read_error,
    }
}

async fn scheduler_config_response(
    state: &Arc<ServerState>,
) -> Result<Json<SchedulerConfigResponse>> {
    let inspection = inspect_scheduler_config(state).await;
    if let Some(error) = inspection.read_error.as_deref() {
        return Err(crate::ApiError::InternalError(format!(
            "failed to read scheduler config: {error}"
        )));
    }

    Ok(Json(SchedulerConfigResponse {
        raw_path: inspection.raw_path,
        resolved_path: inspection.resolved_path,
        exists: inspection.exists,
        content: inspection.content,
        default_profile: inspection.default_profile,
        profiles: inspection.profiles,
        parse_error: inspection.parse_error,
    }))
}

async fn collect_scheduler_config_validation(
    state: &Arc<ServerState>,
) -> Vec<ConfigPolicyValidationItem> {
    let inspection = inspect_scheduler_config(state).await;
    let Some(raw_path) = inspection.raw_path.as_deref() else {
        return Vec::new();
    };

    let base_item = |code: &str, message: String| ConfigPolicyValidationItem {
        owner: ConfigPolicyValidationOwner::Scheduler,
        scope: ConfigPolicyValidationScope {
            kind: ConfigPolicyValidationScopeKind::SchedulerPath,
            subject_id: None,
        },
        path: "scheduler_path".to_string(),
        severity: ConfigPolicyValidationSeverity::Error,
        effect: ConfigPolicyValidationEffect::SoftFallback,
        code: code.to_string(),
        message,
        fallback: None,
    };

    if !inspection.exists {
        if let Some(error) = inspection.read_error {
            return vec![base_item(
                "scheduler_config_unreadable",
                format!("Configured scheduler_path `{raw_path}` could not be read: {error}"),
            )];
        }

        let location = inspection.resolved_path.as_deref().unwrap_or(raw_path);
        return vec![base_item(
            "scheduler_config_missing",
            format!("Configured scheduler_path `{raw_path}` does not exist at `{location}`."),
        )];
    }

    if let Some(error) = inspection.parse_error {
        return vec![base_item(
            "scheduler_config_parse_error",
            format!("Configured scheduler_path `{raw_path}` could not be parsed: {error}"),
        )];
    }

    Vec::new()
}

async fn build_config_policy_validation_snapshot(
    state: &Arc<ServerState>,
) -> ConfigPolicyValidationSnapshot {
    let config = state.config_store.config();
    let mut reports = collect_scheduler_config_validation(state).await;
    reports.extend(collect_skill_tree_validation(&config));
    reports.extend(collect_provider_profile_validation(&config));
    reports.extend(collect_external_adapter_config_validation(&config));
    reports.sort_by(|left, right| {
        left.owner
            .cmp(&right.owner)
            .then_with(|| left.scope.kind.cmp(&right.scope.kind))
            .then_with(|| left.scope.subject_id.cmp(&right.scope.subject_id))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.code.cmp(&right.code))
    });

    ConfigPolicyValidationSnapshot {
        revision: state.config_store.revision(),
        generated_at_ms: Utc::now().timestamp_millis(),
        reports,
    }
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
        .unwrap_or_else(|| ".agendao/scheduler.jsonc".to_string());

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
