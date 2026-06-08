mod config;
mod event_stream;
mod external_adapter;
mod file;
#[cfg(debug_assertions)]
mod frontend_smoke;
mod global;
mod http_surface;
#[cfg(feature = "mcp")]
mod mcp;
mod memory;
mod multimodal;
mod permission;
mod plugin_auth;
mod process;
mod project;
mod provider;
mod provider_diagnostics;
#[cfg(feature = "pty")]
mod pty;
mod session;
mod skill_catalog;
mod skill_hub;
mod skill_proposal;
mod stream;
mod task;
mod tui;
mod web_plugin;
mod workspace;

// Re-export all pub items from sub-modules so `pub use routes::*` in lib.rs continues to work.
use self::event_stream::*;
use self::external_adapter::external_adapter_routes;
use self::memory::memory_routes;
use self::multimodal::multimodal_routes;
use self::plugin_auth::{ensure_plugin_loader_active, plugin_auth_routes};
use self::process::process_routes;
use self::skill_catalog::{
    extract_skill_methodology, get_skill_detail, list_skill_catalog_entries, manage_skill,
    preview_skill_methodology, resolve_skill_catalog, SkillCatalogQuery,
};
use self::skill_hub::skill_hub_routes;
use self::skill_proposal::skill_proposal_routes;
use self::task::task_routes;
use self::web_plugin::web_plugin_routes;
use self::workspace::workspace_routes;
pub use config::*;
pub use file::*;
pub use global::*;
pub use http_surface::attach_http_shell_routes;
#[cfg(feature = "mcp")]
pub use mcp::*;
pub use permission::*;
pub use project::*;
pub use provider::*;
#[cfg(feature = "pty")]
pub use pty::*;
pub use session::*;
pub use tui::*;
pub use workspace::RecentModelsPayload;

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, Sse},
    routing::{get, post, put},
    Json, Router,
};
use futures::stream::Stream;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;

use crate::session_runtime::events::broadcast_config_updated;
use crate::{ApiError, Result, ServerState};
use agendao_agent::{AgentMode, AgentRegistry};
use agendao_command::{
    CommandInteractiveSpec, CommandInvocationSpec, CommandRegistry, CommandSource,
    ResolvedUiCommand, UiCommandArgumentKind, UiCommandSpec,
};
use agendao_config::Config as AppConfig;
use agendao_orchestrator::{SchedulerConfig, SchedulerPresetKind, AUTO_SCHEDULER_PROFILE_NAME};
use agendao_permission::PermissionRuleset;
use agendao_plugin::subprocess::{PluginLoader, PluginSubprocessError};
use agendao_provider::AuthInfo;
use agendao_server_core::runtime_events::ServerEvent;

pub fn router() -> Router<Arc<ServerState>> {
    let router = attach_http_shell_routes(Router::new())
        .route("/health", get(health))
        .route("/event", get(event_stream))
        .route("/path", get(get_paths))
        .route("/vcs", get(get_vcs_info))
        .route("/command", get(list_commands))
        .route("/command/ui", get(list_ui_commands))
        .route("/command/ui/resolve", post(resolve_ui_command))
        .route("/agent", get(list_agents))
        .route("/mode", get(list_execution_modes))
        .route("/skill", get(list_skills))
        .route("/skill/catalog", get(list_skill_catalog_entries))
        .route("/skill/detail", get(get_skill_detail))
        .route(
            "/skill/methodology/extract",
            post(extract_skill_methodology),
        )
        .route(
            "/skill/methodology/preview",
            post(preview_skill_methodology),
        )
        .route("/skill/manage", post(manage_skill))
        .nest("/skill/hub", skill_hub_routes())
        .nest("/skill/proposal", skill_proposal_routes())
        .nest("/memory", memory_routes())
        .nest("/multimodal", multimodal_routes())
        .route("/lsp", get(get_lsp_status))
        .route("/formatter", get(get_formatter_status))
        .route("/auth/{id}", put(set_auth).delete(delete_auth))
        .route("/log", post(write_log))
        .nest("/session", session_routes())
        .nest("/provider", provider_routes())
        .nest("/config", config_routes())
        .nest("/external-adapter", external_adapter_routes())
        .nest("/file", file_routes())
        .nest("/find", find_routes())
        .nest("/permission", permission_routes())
        .nest("/project", project_routes())
        .nest("/question", question_routes())
        .nest("/tui", tui_routes())
        .nest("/process", process_routes())
        .nest("/task", task_routes())
        .nest("/workspace", workspace_routes())
        .nest("/global", global_routes())
        .nest("/experimental", experimental_routes())
        .nest("/plugin", plugin_auth_routes())
        .nest("/web-plugin", web_plugin_routes());

    #[cfg(feature = "mcp")]
    let router = router.nest("/mcp", mcp_routes());

    #[cfg(feature = "pty")]
    let router = router.nest("/pty", pty_routes());

    #[cfg(debug_assertions)]
    let router = router.nest(
        "/experimental/frontend-smoke",
        frontend_smoke::frontend_smoke_routes(),
    );

    router
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// --- /log endpoint: accepts a log entry and writes it via tracing ---

#[derive(Debug, Deserialize)]
struct WriteLogRequest {
    service: String,
    level: String,
    message: String,
    #[serde(default)]
    extra: Option<HashMap<String, serde_json::Value>>,
}

async fn write_log(Json(req): Json<WriteLogRequest>) -> Result<Json<bool>> {
    let extra_str = req
        .extra
        .as_ref()
        .map(|e| serde_json::to_string(e).unwrap_or_default())
        .unwrap_or_default();

    match req.level.as_str() {
        "debug" => tracing::debug!(service = %req.service, extra = %extra_str, "{}", req.message),
        "info" => tracing::info!(service = %req.service, extra = %extra_str, "{}", req.message),
        "warn" => tracing::warn!(service = %req.service, extra = %extra_str, "{}", req.message),
        "error" => tracing::error!(service = %req.service, extra = %extra_str, "{}", req.message),
        other => {
            return Err(ApiError::BadRequest(format!(
                "invalid log level: '{}', expected one of: debug, info, warn, error",
                other
            )));
        }
    }

    Ok(Json(true))
}

#[derive(Debug, Serialize)]
struct PathsResponse {
    home: String,
    config: String,
    data: String,
    cwd: String,
}

async fn get_paths(State(state): State<Arc<ServerState>>) -> Result<Json<PathsResponse>> {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let config = dirs::config_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let data = dirs::data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let cwd = state.project_root().to_string_lossy().to_string();
    Ok(Json(PathsResponse {
        home,
        config,
        data,
        cwd,
    }))
}

#[derive(Debug, Serialize)]
struct VcsInfo {
    system: Option<String>,
    branch: Option<String>,
    root: Option<String>,
}

async fn get_vcs_info() -> Result<Json<VcsInfo>> {
    Ok(Json(VcsInfo {
        system: Some("git".to_string()),
        branch: None,
        root: None,
    }))
}

#[derive(Debug, Clone, Serialize)]
struct CommandApiSpec {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scheduler_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invocation: Option<CommandInvocationSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interactive: Option<CommandInteractiveSpec>,
    source: CommandSource,
}

#[derive(Debug, Clone, Serialize)]
struct UiCommandApiSpec {
    #[serde(flatten)]
    command: UiCommandSpec,
    argument_kind: UiCommandArgumentKind,
}

async fn list_commands(State(state): State<Arc<ServerState>>) -> Result<Json<Vec<CommandApiSpec>>> {
    let mut registry = CommandRegistry::new();
    registry
        .load_from_directory(&state.project_root())
        .map_err(|error| {
            ApiError::InternalError(format!("Failed to load command registry: {error}"))
        })?;

    let mut commands = registry
        .list()
        .into_iter()
        .map(|command| CommandApiSpec {
            name: command.name.clone(),
            description: command.description.clone(),
            scheduler_profile: command.scheduler_profile.clone(),
            aliases: command.aliases.clone(),
            invocation: command.invocation.clone(),
            interactive: command.interactive.clone(),
            source: command.source.clone(),
        })
        .collect::<Vec<_>>();
    commands.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(Json(commands))
}

async fn list_ui_commands() -> Result<Json<Vec<UiCommandApiSpec>>> {
    let registry = CommandRegistry::new();
    Ok(Json(
        registry
            .ui_commands()
            .iter()
            .cloned()
            .map(|command| UiCommandApiSpec {
                argument_kind: command.argument_kind(),
                command,
            })
            .collect(),
    ))
}

#[derive(Debug, Clone, Deserialize)]
struct ResolveUiCommandRequest {
    input: String,
}

async fn resolve_ui_command(
    Json(req): Json<ResolveUiCommandRequest>,
) -> Result<Json<Option<ResolvedUiCommand>>> {
    let registry = CommandRegistry::new();
    Ok(Json(registry.resolve_ui_slash_input(&req.input)))
}

#[derive(Debug, Clone, Serialize)]
struct AgentApiModelRef {
    #[serde(rename = "modelID")]
    model_id: String,
    #[serde(rename = "providerID")]
    provider_id: String,
}

/// Matches the TS `Agent.Info` schema returned by the original OpenCode `/agent` endpoint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentInfo {
    /// Extra field for TUI backward compat (not in TS schema, harmless).
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    mode: AgentMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    native: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    permission: PermissionRuleset,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<AgentApiModelRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_prompt: Option<String>,
    options: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    steps: Option<u32>,
}

static AGENT_LIST_CACHE: Lazy<RwLock<Option<Vec<AgentInfo>>>> = Lazy::new(|| RwLock::new(None));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionModeInfo {
    id: String,
    name: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    orchestrator: Option<String>,
}

static MODE_LIST_CACHE: Lazy<RwLock<Option<Vec<ExecutionModeInfo>>>> =
    Lazy::new(|| RwLock::new(None));

/// Random token generated at server startup. Plugin-host receives it via
/// `AGENDAO_INTERNAL_TOKEN` env var and sends it back in `x-agendao-internal-token` header.
/// Prevents external clients from forging the internal-request header.
static INTERNAL_TOKEN: Lazy<String> = Lazy::new(|| {
    use std::fmt::Write;
    let mut buf = String::with_capacity(32);
    for b in &uuid::Uuid::new_v4().as_bytes()[..16] {
        let _ = write!(buf, "{:02x}", b);
    }
    buf
});

pub fn internal_token() -> &'static str {
    &INTERNAL_TOKEN
}

fn is_valid_internal_request(headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get("x-agendao-plugin-internal")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let trimmed = value.trim();
    if !(trimmed == "1" || trimmed.eq_ignore_ascii_case("true")) {
        return false;
    }
    // Verify token
    let Some(token) = headers
        .get("x-agendao-internal-token")
        .and_then(|v| v.to_str().ok())
    else {
        tracing::warn!("internal request header present but missing token");
        return false;
    };
    token.trim() == INTERNAL_TOKEN.as_str()
}

fn should_apply_plugin_config_hooks(headers: &HeaderMap) -> bool {
    !is_valid_internal_request(headers)
}

pub(crate) async fn list_agents(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentInfo>>> {
    if !should_apply_plugin_config_hooks(&headers) {
        if let Some(cached) = AGENT_LIST_CACHE.read().await.clone() {
            return Ok(Json(cached));
        }
        let config = state.config_store.config();
        return Ok(Json(build_agent_list(Some(&config))));
    }

    // Ensure plugins are alive before calling config hooks (P1 fix: idle-shutdown recovery)
    let _ = ensure_plugin_loader_active(&state).await?;

    let mut config = (*state.config_store.config()).clone();
    if let Some(loader) = get_plugin_loader() {
        apply_plugin_config_hooks(loader, &mut config).await;
    }

    state.config_store.set_plugin_applied(config.clone()).await;
    let agents = build_agent_list(Some(&config));
    *AGENT_LIST_CACHE.write().await = Some(agents.clone());
    Ok(Json(agents))
}

fn build_agent_list(config: Option<&AppConfig>) -> Vec<AgentInfo> {
    let registry = AgentRegistry::from_optional_config(config);
    registry
        .list()
        .into_iter()
        .map(|agent| AgentInfo {
            id: agent.name.clone(),
            name: agent.name.clone(),
            description: agent.description.clone(),
            mode: agent.mode,
            native: if agent.native { Some(true) } else { None },
            hidden: if agent.hidden { Some(true) } else { None },
            top_p: agent.top_p,
            temperature: agent.temperature,
            color: agent.color.clone(),
            permission: agent.permission.clone(),
            model: agent.model.as_ref().map(|m| AgentApiModelRef {
                model_id: m.model_id.clone(),
                provider_id: m.provider_id.clone(),
            }),
            variant: agent.variant.clone(),
            prompt: agent.system_prompt.clone(),
            resolved_prompt: agent.resolved_system_prompt(),
            options: agent.options.clone(),
            steps: agent.max_steps,
        })
        .collect()
}

fn builtin_preset_mode_description(preset: SchedulerPresetKind) -> &'static str {
    match preset {
        SchedulerPresetKind::Sisyphus => "OMO-aligned delegation-first orchestration preset",
        SchedulerPresetKind::Prometheus => "OMO-aligned planning-first orchestration preset",
        SchedulerPresetKind::Atlas => "OMO-aligned graph-oriented orchestration preset",
        SchedulerPresetKind::Hephaestus => "OMO-aligned autonomous execution preset",
        SchedulerPresetKind::Verifier => {
            "Workflow-backed verifier preset for repeated candidate comparison and selection"
        }
    }
}

fn build_builtin_preset_mode_list() -> Vec<ExecutionModeInfo> {
    let mut items = vec![ExecutionModeInfo {
        id: AUTO_SCHEDULER_PROFILE_NAME.to_string(),
        name: AUTO_SCHEDULER_PROFILE_NAME.to_string(),
        kind: "preset".to_string(),
        description: Some("Automatic routing preset: choose the workflow per request".to_string()),
        mode: None,
        hidden: None,
        color: None,
        orchestrator: Some("sisyphus".to_string()),
    }];
    items.extend(
        SchedulerPresetKind::public_presets()
            .iter()
            .copied()
            .map(|preset| ExecutionModeInfo {
                id: preset.as_str().to_string(),
                name: preset.as_str().to_string(),
                kind: "preset".to_string(),
                description: Some(builtin_preset_mode_description(preset).to_string()),
                mode: None,
                hidden: None,
                color: None,
                orchestrator: Some(preset.as_str().to_string()),
            }),
    );
    items
}

fn build_external_scheduler_profile_mode_list(
    config: Option<&AppConfig>,
) -> Result<Vec<ExecutionModeInfo>> {
    let Some(config) = config else {
        return Ok(Vec::new());
    };

    let Some(scheduler_path) = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(Vec::new());
    };

    let scheduler_config = match SchedulerConfig::load_from_file(scheduler_path) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(path = %scheduler_path, %error, "failed to load external scheduler profiles for execution modes");
            return Ok(Vec::new());
        }
    };

    let mut profiles = scheduler_config
        .profiles
        .into_iter()
        .map(|(profile_name, profile)| ExecutionModeInfo {
            id: profile_name.clone(),
            name: profile_name,
            kind: "profile".to_string(),
            description: profile.description.clone(),
            mode: None,
            hidden: None,
            color: None,
            orchestrator: profile.orchestrator.clone(),
        })
        .collect::<Vec<_>>();
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

fn build_execution_mode_list(config: Option<&AppConfig>) -> Result<Vec<ExecutionModeInfo>> {
    let mut items = build_agent_list(config)
        .into_iter()
        .map(|agent| ExecutionModeInfo {
            id: agent.id,
            name: agent.name,
            kind: "agent".to_string(),
            description: agent.description,
            mode: Some(match agent.mode {
                AgentMode::All => "all".to_string(),
                AgentMode::Primary => "primary".to_string(),
                AgentMode::Subagent => "subagent".to_string(),
            }),
            hidden: agent.hidden,
            color: agent.color,
            orchestrator: None,
        })
        .collect::<Vec<_>>();
    items.extend(build_builtin_preset_mode_list());
    items.extend(build_external_scheduler_profile_mode_list(config)?);
    Ok(items)
}

pub(crate) async fn list_execution_modes(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ExecutionModeInfo>>> {
    if !should_apply_plugin_config_hooks(&headers) {
        if let Some(cached) = MODE_LIST_CACHE.read().await.clone() {
            return Ok(Json(cached));
        }
        let config = state.config_store.config();
        return Ok(Json(build_execution_mode_list(Some(&config))?));
    }

    let _ = ensure_plugin_loader_active(&state).await?;

    let mut config = (*state.config_store.config()).clone();
    if let Some(loader) = get_plugin_loader() {
        apply_plugin_config_hooks(loader, &mut config).await;
    }

    state.config_store.set_plugin_applied(config.clone()).await;
    let modes = build_execution_mode_list(Some(&config))?;
    *MODE_LIST_CACHE.write().await = Some(modes.clone());
    Ok(Json(modes))
}

pub async fn refresh_agent_cache(config_store: &agendao_config::ConfigStore) {
    let mut config = (*config_store.config()).clone();

    if let Some(loader) = get_plugin_loader() {
        apply_plugin_config_hooks(loader, &mut config).await;
    }

    config_store.set_plugin_applied(config.clone()).await;
    let agents = build_agent_list(Some(&config));
    *AGENT_LIST_CACHE.write().await = Some(agents);
    match build_execution_mode_list(Some(&config)) {
        Ok(modes) => {
            *MODE_LIST_CACHE.write().await = Some(modes);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to refresh execution mode cache");
        }
    }
}

async fn apply_plugin_config_hooks(loader: &Arc<PluginLoader>, config: &mut AppConfig) {
    let mut config_value = match serde_json::to_value(config.clone()) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "failed to serialize config for plugin config hook");
            return;
        }
    };

    for client in loader.clients().await {
        match client
            .invoke_hook("config", config_value.clone(), config_value.clone())
            .await
        {
            Ok(next_config) => {
                if next_config.is_object() {
                    config_value = next_config;
                } else {
                    tracing::warn!(
                        plugin = client.name(),
                        "plugin config hook returned non-object config payload"
                    );
                }
            }
            Err(PluginSubprocessError::Rpc { code: -32601, .. }) => {
                // Plugin does not implement config hook.
            }
            Err(error) => {
                tracing::warn!(
                    plugin = client.name(),
                    %error,
                    "plugin config hook invocation failed"
                );
            }
        }
    }

    match serde_json::from_value::<AppConfig>(config_value) {
        Ok(next) => *config = next,
        Err(error) => {
            tracing::warn!(%error, "failed to deserialize config after plugin hooks");
        }
    }
}

async fn list_skills(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SkillCatalogQuery>,
) -> Result<Json<Vec<agendao_skill::SkillMetaView>>> {
    Ok(Json(resolve_skill_catalog(&state, &query).await?))
}

#[derive(Debug, Serialize)]
struct LspStatus {
    servers: Vec<String>,
}

async fn get_lsp_status() -> Result<Json<LspStatus>> {
    Ok(Json(LspStatus {
        servers: Vec::new(),
    }))
}

#[derive(Debug, Serialize)]
struct FormatterStatus {
    formatters: Vec<String>,
}

async fn get_formatter_status() -> Result<Json<FormatterStatus>> {
    Ok(Json(FormatterStatus {
        formatters: Vec::new(),
    }))
}

#[derive(Debug, Deserialize)]
struct SetAuthRequest {
    #[serde(flatten)]
    body: serde_json::Value,
}

async fn set_auth(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetAuthRequest>,
) -> Result<Json<serde_json::Value>> {
    let auth_info = parse_auth_info_payload(req.body)
        .ok_or_else(|| ApiError::BadRequest("Invalid auth payload".to_string()))?;
    state.auth_manager.set(&id, auth_info).await;

    // Rebuild the provider registry so newly-connected providers are
    // available immediately (e.g. their models show up in /provider/).
    state.rebuild_providers().await;
    broadcast_config_updated(state.as_ref());

    Ok(Json(serde_json::json!({ "success": true })))
}

async fn delete_auth(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    state.auth_manager.remove(&id).await;
    state.rebuild_providers().await;
    broadcast_config_updated(state.as_ref());
    Ok(Json(serde_json::json!({ "deleted": true })))
}

fn parse_auth_info_payload(payload: serde_json::Value) -> Option<AuthInfo> {
    if let Ok(auth) = serde_json::from_value::<AuthInfo>(payload.clone()) {
        return Some(auth);
    }

    let key = payload
        .get("api_key")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("apiKey").and_then(|v| v.as_str()))
        .or_else(|| payload.get("token").and_then(|v| v.as_str()))
        .or_else(|| payload.get("key").and_then(|v| v.as_str()))
        .map(str::to_string)?;

    Some(AuthInfo::Api { key })
}

// ===========================================================================
// Plugin auth routes
// ===========================================================================

static PLUGIN_LOADER: std::sync::OnceLock<Arc<PluginLoader>> = std::sync::OnceLock::new();

/// Register the global PluginLoader so routes can access auth bridges.
/// Called once during server startup after plugins are loaded.
pub fn set_plugin_loader(loader: Arc<PluginLoader>) {
    let _ = PLUGIN_LOADER.set(loader);
}

pub(crate) fn get_plugin_loader() -> Option<&'static Arc<PluginLoader>> {
    PLUGIN_LOADER.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn execution_modes_include_builtin_public_presets_without_scheduler_path() {
        let modes = build_execution_mode_list(Some(&AppConfig::default()))
            .expect("builtin mode list should resolve without external scheduler config");
        let preset_names = modes
            .into_iter()
            .filter(|mode| mode.kind == "preset")
            .map(|mode| mode.name)
            .collect::<Vec<_>>();

        assert_eq!(
            preset_names,
            vec![
                "auto",
                "sisyphus",
                "prometheus",
                "atlas",
                "hephaestus",
                "verifier",
            ]
        );
    }

    #[test]
    fn execution_modes_skip_external_profiles_when_scheduler_config_cannot_be_loaded() {
        let config = AppConfig {
            scheduler_path: Some("/definitely/missing/agendao.scheduler.jsonc".to_string()),
            ..Default::default()
        };

        let modes = build_execution_mode_list(Some(&config))
            .expect("broken external scheduler config should not fail built-in mode listing");

        let preset_names = modes
            .iter()
            .filter(|mode| mode.kind == "preset")
            .map(|mode| mode.name.as_str())
            .collect::<Vec<_>>();
        let profile_names = modes
            .iter()
            .filter(|mode| mode.kind == "profile")
            .map(|mode| mode.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            preset_names,
            vec![
                "auto",
                "sisyphus",
                "prometheus",
                "atlas",
                "hephaestus",
                "verifier",
            ]
        );
        assert!(profile_names.is_empty());
    }

    #[test]
    fn merge_output_block_delta_coalesces_message_text_for_same_session_and_id() {
        let mut current = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-1".to_string()),
            block: json!({
                "kind": "message", "phase": "delta", "role": "assistant", "text": "hel",
            }),
            live_identity: None,
        };
        let next = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-1".to_string()),
            block: json!({
                "kind": "message", "phase": "delta", "role": "assistant", "text": "lo",
            }),
            live_identity: None,
        };

        assert!(merge_output_block_delta(&mut current, &next));
        let ServerEvent::OutputBlock { block, .. } = current else {
            panic!("expected output block");
        };
        assert_eq!(
            block.get("text").and_then(|value| value.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn merge_output_block_delta_rejects_different_message_ids() {
        let mut current = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-1".to_string()),
            block: json!({ "kind": "message", "phase": "delta", "role": "assistant", "text": "hel" }),
            live_identity: None,
        };
        let next = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-2".to_string()),
            block: json!({ "kind": "message", "phase": "delta", "role": "assistant", "text": "lo" }),
            live_identity: None,
        };

        assert!(!merge_output_block_delta(&mut current, &next));
    }

    #[test]
    fn merge_output_block_delta_rejects_non_delta_or_non_output_events() {
        let mut current = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("reasoning-1".to_string()),
            block: json!({ "kind": "reasoning", "phase": "delta", "text": "thinking" }),
            live_identity: None,
        };
        let full = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("reasoning-1".to_string()),
            block: json!({ "kind": "reasoning", "phase": "full", "text": "thinking done" }),
            live_identity: None,
        };
        let usage = ServerEvent::Usage {
            session_id: Some("session-a".to_string()),
            prompt_tokens: 1,
            completion_tokens: 1,
            message_id: Some("reasoning-1".to_string()),
        };

        assert!(!merge_output_block_delta(&mut current, &full));
        assert!(!merge_output_block_delta(&mut current, &usage));
    }

    #[test]
    fn merge_output_block_delta_replaces_snapshot_for_same_live_identity() {
        let identity = agendao_types::LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
            part_kind: agendao_types::LiveMessagePartKind::AssistantReasoning,
            phase: agendao_types::LivePartPhase::Snapshot,
            legacy_block_id: Some("reasoning-1".to_string()),
        };
        let mut current = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("reasoning-1".to_string()),
            block: json!({ "kind": "reasoning", "phase": "full", "text": "think" }),
            live_identity: Some(identity.clone()),
        };
        let next = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("reasoning-1".to_string()),
            block: json!({ "kind": "reasoning", "phase": "full", "text": "thinking longer" }),
            live_identity: Some(identity),
        };

        assert!(merge_output_block_delta(&mut current, &next));
        let ServerEvent::OutputBlock {
            block,
            live_identity,
            ..
        } = current
        else {
            panic!("expected output block");
        };
        assert_eq!(
            block.get("text").and_then(|value| value.as_str()),
            Some("thinking longer")
        );
        assert_eq!(
            live_identity.map(|identity| identity.phase),
            Some(agendao_types::LivePartPhase::Snapshot)
        );
    }

    // ── P2-2 subscription filter tests ──────────────────────────────────

    fn reasoning_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "reasoning", "phase": "delta", "text": "think"}),
            live_identity: None,
        }
    }

    fn reasoning_full_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "reasoning", "phase": "full", "text": "thinking so far"}),
            live_identity: None,
        }
    }

    fn reasoning_end_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "reasoning", "phase": "end", "text": ""}),
            live_identity: None,
        }
    }

    fn message_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "message", "phase": "delta", "text": "hello"}),
            live_identity: None,
        }
    }

    fn tool_running_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("tool-1".to_string()),
            block: serde_json::json!({
                "kind": "tool",
                "phase": "running",
                "name": "webfetch",
                "detail": "fetching"
            }),
            live_identity: None,
        }
    }

    fn tool_done_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("tool-1".to_string()),
            block: serde_json::json!({
                "kind": "tool",
                "phase": "done",
                "name": "webfetch",
                "detail": "{\"status\":200}"
            }),
            live_identity: None,
        }
    }

    fn caps(full: bool) -> agendao_api::FrontendSubscriptionCapabilities {
        if full {
            return agendao_api::FrontendSubscriptionCapabilities::default();
        }
        agendao_api::FrontendSubscriptionTier::CliLowFrequency.default_capabilities()
    }

    #[test]
    fn full_capabilities_pass_everything() {
        let c = caps(true);
        assert!(event_passes_subscription_caps(&reasoning_block(), &c));
        assert!(event_passes_subscription_caps(&message_block(), &c));
    }

    #[test]
    fn final_only_skips_deltas_passes_non_droppable() {
        let c = caps(false);
        assert!(!event_passes_subscription_caps(&reasoning_block(), &c));
        assert!(!event_passes_subscription_caps(&message_block(), &c));
        assert!(!event_passes_subscription_caps(&tool_running_block(), &c));
        assert!(event_passes_subscription_caps(&tool_done_block(), &c));
        // Non-droppable events always pass.
        let perm = ServerEvent::PermissionResolved {
            session_id: "sess-1".to_string(),
            permission_id: "p-1".to_string(),
            reply: "once".to_string(),
            message: None,
        };
        assert!(event_passes_subscription_caps(&perm, &c));
        let session = ServerEvent::SessionUpdated {
            session_id: "sess-1".to_string(),
            source: "turn.final".to_string(),
        };
        assert!(event_passes_subscription_caps(&session, &c));
    }

    #[test]
    fn web_tier_skips_reasoning_delta_but_keeps_reasoning_boundaries_and_snapshots() {
        let c = agendao_api::FrontendSubscriptionTier::WebMediumFrequency.default_capabilities();
        assert!(!event_passes_subscription_caps(&reasoning_block(), &c));
        assert!(event_passes_subscription_caps(&reasoning_full_block(), &c));
        assert!(event_passes_subscription_caps(&reasoning_end_block(), &c));
        assert!(event_passes_subscription_caps(&message_block(), &c));
    }

    // ── P3-B live snapshot coalescer tests ─────────────────────────────

    fn coalesce_delta(text: &str, msg_id: &str, part_key: &str) -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({ "kind": "message", "phase": "delta", "text": text }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: msg_id.to_string(),
                part_key: part_key.to_string(),
                part_kind: agendao_types::LiveMessagePartKind::AssistantText,
                phase: agendao_types::LivePartPhase::Append,
                legacy_block_id: Some("block-1".to_string()),
            }),
        }
    }

    fn snapshot_block_text(event: &ServerEvent) -> Option<String> {
        let ServerEvent::OutputBlock { ref block, .. } = event else {
            return None;
        };
        block
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn snapshot_phase(event: &ServerEvent) -> Option<agendao_types::LivePartPhase> {
        let ServerEvent::OutputBlock {
            ref live_identity, ..
        } = event
        else {
            return None;
        };
        live_identity.as_ref().map(|id| id.phase)
    }

    #[test]
    fn coalescer_accumulates_deltas_into_growing_snapshot() {
        let mut c = LiveSnapshotCoalescer::new();

        let s1 = c.coalesce(coalesce_delta(
            "hello",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        assert_eq!(snapshot_block_text(&s1), Some("hello".to_string()));

        let s2 = c.coalesce(coalesce_delta(
            " world",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        assert_eq!(snapshot_block_text(&s2), Some("hello world".to_string()));
        assert_eq!(
            snapshot_phase(&s2),
            Some(agendao_types::LivePartPhase::Snapshot)
        );
    }

    #[test]
    fn web_tier_accepts_coalesced_reasoning_snapshot_from_delta() {
        let c = agendao_api::FrontendSubscriptionTier::WebMediumFrequency.default_capabilities();
        let mut coalescer = LiveSnapshotCoalescer::new();
        let coalesced = coalescer.coalesce(ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "reasoning", "phase": "delta", "text": "think"}),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
                part_kind: agendao_types::LiveMessagePartKind::AssistantReasoning,
                phase: agendao_types::LivePartPhase::Append,
                legacy_block_id: Some("block-1".to_string()),
            }),
        });

        let ServerEvent::OutputBlock {
            ref block,
            ref live_identity,
            ..
        } = coalesced
        else {
            panic!("expected output block");
        };
        assert_eq!(
            block.get("phase").and_then(|value| value.as_str()),
            Some("full")
        );
        assert_eq!(
            block.get("text").and_then(|value| value.as_str()),
            Some("think")
        );
        assert_eq!(
            live_identity.as_ref().map(|identity| identity.phase),
            Some(agendao_types::LivePartPhase::Snapshot)
        );
        assert!(
            event_passes_subscription_caps(&coalesced, &c),
            "web tier must accept reasoning delta after it has been coalesced into a snapshot"
        );
    }

    #[test]
    fn coalescer_passes_through_non_delta_unchanged() {
        let mut c = LiveSnapshotCoalescer::new();
        let perm = ServerEvent::PermissionResolved {
            session_id: "sess-1".to_string(),
            permission_id: "p-1".to_string(),
            reply: "once".to_string(),
            message: None,
        };
        let result = c.coalesce(perm);
        // Permission events pass through unchanged — verify the session_id is intact.
        assert_eq!(result.session_id(), Some("sess-1"));
    }

    #[test]
    fn coalescer_clears_state_on_end_phase() {
        let mut c = LiveSnapshotCoalescer::new();
        c.coalesce(coalesce_delta(
            "hello",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        assert_eq!(c.accum.len(), 1, "accum should track one entry");

        // End phase on the same identity must clear the entry.
        let end = ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({ "kind": "message", "phase": "end" }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
                part_kind: agendao_types::LiveMessagePartKind::AssistantText,
                phase: agendao_types::LivePartPhase::End,
                legacy_block_id: Some("block-1".to_string()),
            }),
        };
        c.coalesce(end);
        assert!(c.accum.is_empty(), "End must clear accumulated state");
    }

    #[test]
    fn coalescer_keys_state_by_session_message_and_part_key() {
        let mut c = LiveSnapshotCoalescer::new();

        let first_message = c.coalesce(coalesce_delta(
            "msg1",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        assert_eq!(
            snapshot_block_text(&first_message),
            Some("msg1".to_string())
        );

        let second_message = c.coalesce(coalesce_delta(
            "msg2",
            "msg-2",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        assert_eq!(
            snapshot_block_text(&second_message),
            Some("msg2".to_string())
        );

        let reasoning_part = c.coalesce(coalesce_delta(
            "thinking",
            "msg-1",
            agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
        ));
        assert_eq!(
            snapshot_block_text(&reasoning_part),
            Some("thinking".to_string())
        );

        // Same msg_id + part_key, different sessions — must not cross-contaminate.
        let session_scoped = ServerEvent::OutputBlock {
            session_id: "sess-2".to_string(),
            id: Some("block-2".to_string()),
            block: serde_json::json!({ "kind": "message", "phase": "delta", "text": "session-b" }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
                part_kind: agendao_types::LiveMessagePartKind::AssistantText,
                phase: agendao_types::LivePartPhase::Append,
                legacy_block_id: Some("block-2".to_string()),
            }),
        };
        let s2 = c.coalesce(session_scoped);
        assert_eq!(
            snapshot_block_text(&s2),
            Some("session-b".to_string()),
            "different sessions must not share accumulated text"
        );
    }

    #[test]
    fn coalescer_snapshot_has_full_phase_not_delta() {
        let mut c = LiveSnapshotCoalescer::new();
        let result = c.coalesce(coalesce_delta(
            "test",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        let ServerEvent::OutputBlock { ref block, .. } = result else {
            panic!("expected OutputBlock")
        };
        assert_eq!(
            block.get("phase").and_then(|v| v.as_str()),
            Some("full"),
            "snapshot must set block phase to 'full' so pending merge replaces snapshots instead of appending raw deltas"
        );
    }

    #[test]
    fn coalesced_snapshots_stay_mergeable_for_pending_debounce() {
        let mut c = LiveSnapshotCoalescer::new();
        let mut first = c.coalesce(coalesce_delta(
            "hel",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));
        let second = c.coalesce(coalesce_delta(
            "lo",
            "msg-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
        ));

        assert!(
            is_mergeable_output_delta(&first),
            "coalesced snapshot must still enter the pending debounce lane"
        );
        assert!(
            merge_output_block_delta(&mut first, &second),
            "later snapshots for the same live identity must replace earlier pending snapshots"
        );
        assert_eq!(snapshot_block_text(&first), Some("hello".to_string()));
    }

    #[test]
    fn coalescer_passes_through_output_block_without_live_identity() {
        let mut c = LiveSnapshotCoalescer::new();
        let block = ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({ "kind": "message", "phase": "delta", "text": "legacy" }),
            live_identity: None,
        };
        let result = c.coalesce(block);
        assert_eq!(
            snapshot_block_text(&result),
            Some("legacy".to_string()),
            "legacy blocks without live_identity should pass through unchanged"
        );
    }

    #[test]
    // LTS-B2: tool running detail is now coalesced into full-so-far
    // snapshots, same as assistant text/reasoning. Frontends receive
    // the complete accumulated detail, not prefix-level fragments.
    fn coalescer_accumulates_tool_detail_into_snapshot() {
        let mut c = LiveSnapshotCoalescer::new();
        let first = ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("tool-1".to_string()),
            block: serde_json::json!({
                "kind": "tool",
                "phase": "running",
                "name": "write_file",
                "detail": "chunk-1"
            }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::tool_call_part_key("call-1"),
                part_kind: agendao_types::LiveMessagePartKind::ToolCall,
                phase: agendao_types::LivePartPhase::Append,
                legacy_block_id: Some("tool-1".to_string()),
            }),
        };
        let second = ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("tool-1".to_string()),
            block: serde_json::json!({
                "kind": "tool",
                "phase": "running",
                "name": "write_file",
                "detail": " chunk-2"
            }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::tool_call_part_key("call-1"),
                part_kind: agendao_types::LiveMessagePartKind::ToolCall,
                phase: agendao_types::LivePartPhase::Append,
                legacy_block_id: Some("tool-1".to_string()),
            }),
        };

        let first = c.coalesce(first);
        let second = c.coalesce(second);

        let ServerEvent::OutputBlock {
            block: first_block,
            live_identity: first_identity,
            ..
        } = first
        else {
            panic!("expected OutputBlock")
        };
        let ServerEvent::OutputBlock {
            block: second_block,
            live_identity: second_identity,
            ..
        } = second
        else {
            panic!("expected OutputBlock")
        };

        // First delta: accumulated into snapshot.
        assert_eq!(first_block["kind"], "tool");
        assert_eq!(first_block["phase"], "full");
        assert_eq!(first_block["detail"], "chunk-1");
        assert_eq!(
            first_identity.as_ref().map(|identity| identity.phase),
            Some(agendao_types::LivePartPhase::Snapshot)
        );
        // Second delta: detail accumulated with previous.
        assert_eq!(second_block["kind"], "tool");
        assert_eq!(second_block["phase"], "full");
        assert_eq!(second_block["detail"], "chunk-1 chunk-2");
        assert_eq!(
            second_identity.as_ref().map(|identity| identity.phase),
            Some(agendao_types::LivePartPhase::Snapshot)
        );
        // Tool detail IS in the snapshot accumulator.
        assert!(
            c.accum.contains_key(&agendao_types::live_slot_key(
                "sess-1:msg-1",
                &agendao_types::tool_call_part_key("call-1"),
            )),
            "tool detail must enter snapshot accumulator for coalescing"
        );
    }

    #[test]
    fn coalescer_clears_tool_detail_state_on_end_phase() {
        let mut c = LiveSnapshotCoalescer::new();
        let delta = ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("tool-1".to_string()),
            block: serde_json::json!({
                "kind": "tool",
                "phase": "running",
                "name": "write_file",
                "detail": "accumulated detail"
            }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::tool_call_part_key("call-1"),
                part_kind: agendao_types::LiveMessagePartKind::ToolCall,
                phase: agendao_types::LivePartPhase::Append,
                legacy_block_id: Some("tool-1".to_string()),
            }),
        };
        c.coalesce(delta);
        assert!(c.accum.contains_key(&agendao_types::live_slot_key(
            "sess-1:msg-1",
            &agendao_types::tool_call_part_key("call-1"),
        )));

        let end = ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("tool-1".to_string()),
            block: serde_json::json!({ "kind": "tool", "phase": "done", "name": "write_file" }),
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: agendao_types::tool_call_part_key("call-1"),
                part_kind: agendao_types::LiveMessagePartKind::ToolCall,
                phase: agendao_types::LivePartPhase::End,
                legacy_block_id: Some("tool-1".to_string()),
            }),
        };
        c.coalesce(end);
        assert!(
            !c.accum.contains_key(&agendao_types::live_slot_key(
                "sess-1:msg-1",
                &agendao_types::tool_call_part_key("call-1"),
            )),
            "tool detail End phase must clear accumulated state"
        );
    }

    /// P2-2 regression: the pending.is_none() branch in stream_server_events
    /// must call subscribable() before sending or buffering the first event.
    /// This test verifies the pure function rejects the event types that hit
    /// the pending.is_none() path. The companion integration test
    /// (cli_stream_filters_first_event) guards against removal of the call site.
    #[test]
    fn first_event_pure_function_rejects_usage_and_message_delta_in_final_only() {
        let c = caps(false);
        assert!(!event_passes_subscription_caps(&usage_event(), &c));
        assert!(!event_passes_subscription_caps(&message_block(), &c));
    }

    fn usage_event() -> ServerEvent {
        ServerEvent::Usage {
            session_id: Some("sess-1".to_string()),
            prompt_tokens: 1,
            completion_tokens: 2,
            message_id: None,
        }
    }

    /// P2-2 integration: guards against removal of the subscribable() call
    /// in stream_server_events()'s pending.is_none() branch. Sends a message
    /// delta followed by a session.updated through a real broadcast channel
    /// with CLI-tier subscription, then asserts the delta is filtered.
    #[tokio::test]
    async fn cli_stream_filters_first_event_in_pending_is_none_path() {
        use tokio::sync::broadcast;
        let (tx, _) = broadcast::channel::<String>(16);
        let rx = tx.subscribe();

        let cli_sub = agendao_api::ResolvedFrontendSubscription::from_tier(
            agendao_api::FrontendSubscriptionTier::CliLowFrequency,
        );
        let sse = super::stream_server_events(rx, None, cli_sub, None);

        // First event: a non-mergeable Usage — hits the pending.is_none()
        // direct-send path. With CLI tier, it must be filtered.
        tx.send(
            serde_json::json!({
                "type": "usage", "sessionID": "sess-1",
                "prompt_tokens": 10, "completion_tokens": 20
            })
            .to_string(),
        )
        .expect("send usage");

        // Second event: a message delta — mergeable, would enter pending
        // buffer. With CLI tier, must be filtered.
        tx.send(
            serde_json::json!({
                "type": "output_block", "sessionID": "sess-1", "id": "block-1",
                "block": { "kind": "message", "phase": "delta", "text": "hello" }
            })
            .to_string(),
        )
        .expect("send message delta");

        // Third event: session.updated — must pass for CLI tier.
        tx.send(
            serde_json::json!({
                "type": "session.updated", "sessionID": "sess-1",
                "source": "turn.final"
            })
            .to_string(),
        )
        .expect("send session.updated");

        // Close the broadcast channel so the SSE stream task exits and the
        // body completes. Without this, to_bytes blocks forever on the live stream.
        drop(tx);

        use axum::response::IntoResponse;
        let body = sse.into_response().into_body();
        let collected = axum::body::to_bytes(body, 4096)
            .await
            .expect("collect body");
        let text = std::str::from_utf8(&collected).expect("utf-8");

        assert!(
            !text.contains("\"usage\""),
            "CLI tier must filter Usage in pending.is_none() path; got:\n{text}"
        );
        assert!(
            !text.contains("\"message\""),
            "CLI tier must filter message delta in pending buffer path; got:\n{text}"
        );
        assert!(
            text.contains("session.updated"),
            "CLI tier must deliver session.updated; got:\n{text}"
        );
    }

    #[tokio::test]
    async fn full_tier_stream_debounces_coalesced_snapshots_in_pending_path() {
        use tokio::sync::broadcast;
        let (tx, _) = broadcast::channel::<String>(16);
        let rx = tx.subscribe();

        let full_sub = agendao_api::ResolvedFrontendSubscription::from_tier(
            agendao_api::FrontendSubscriptionTier::TuiHighFrequency,
        );
        let sse = super::stream_server_events(rx, None, full_sub, None);

        tx.send(
            serde_json::json!({
                "type": "output_block",
                "sessionID": "sess-1",
                "id": "block-1",
                "block": { "kind": "message", "phase": "delta", "role": "assistant", "text": "hel" },
                "live_identity": {
                    "message_id": "msg-1",
                    "part_key": agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    "part_kind": "assistant_text",
                    "phase": "append",
                    "legacy_block_id": "block-1"
                }
            })
            .to_string(),
        )
        .expect("send first delta");

        tx.send(
            serde_json::json!({
                "type": "output_block",
                "sessionID": "sess-1",
                "id": "block-1",
                "block": { "kind": "message", "phase": "delta", "role": "assistant", "text": "lo" },
                "live_identity": {
                    "message_id": "msg-1",
                    "part_key": agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    "part_kind": "assistant_text",
                    "phase": "append",
                    "legacy_block_id": "block-1"
                }
            })
            .to_string(),
        )
        .expect("send second delta");

        drop(tx);

        use axum::response::IntoResponse;
        let body = sse.into_response().into_body();
        let collected = axum::body::to_bytes(body, 4096)
            .await
            .expect("collect body");
        let text = std::str::from_utf8(&collected).expect("utf-8");

        assert_eq!(
            text.matches("output_block").count(),
            1,
            "coalesced snapshots should stay in the pending debounce lane and flush once; got:\n{text}"
        );
        assert!(
            text.contains("\"text\":\"hello\""),
            "final flushed snapshot should contain the accumulated text; got:\n{text}"
        );
        assert!(
            !text.contains("\"text\":\"hel\""),
            "intermediate snapshot must not be flushed separately; got:\n{text}"
        );
    }
}
