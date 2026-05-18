mod config;
mod external_adapter;
mod file;
#[cfg(debug_assertions)]
mod frontend_smoke;
mod global;
mod mcp;
mod memory;
mod multimodal;
mod permission;
mod plugin_auth;
mod process;
mod project;
mod provider;
mod provider_diagnostics;
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
pub use mcp::*;
pub use permission::*;
pub use project::*;
pub use provider::*;
pub use pty::*;
pub use session::*;
pub use tui::*;
pub use workspace::*;

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

use crate::session_runtime::events::{broadcast_config_updated, ServerEvent};
use crate::web;
use crate::{ApiError, Result, ServerState};
use rocode_agent::{AgentMode, AgentRegistry};
use rocode_command::{CommandRegistry, ResolvedUiCommand};
use rocode_config::Config as AppConfig;
use rocode_orchestrator::{SchedulerConfig, SchedulerPresetKind, AUTO_SCHEDULER_PROFILE_NAME};
use rocode_permission::PermissionRuleset;
use rocode_plugin::subprocess::{PluginLoader, PluginSubprocessError};
use rocode_provider::AuthInfo;

pub fn router() -> Router<Arc<ServerState>> {
    let router = Router::new()
        .route("/", get(web::web_index))
        .route("/favicon.ico", get(web::root_favicon))
        .route("/apple-touch-icon.png", get(web::root_apple_touch_icon))
        .route("/web", get(web::web_index))
        .route("/web/", get(web::web_index))
        .route("/web/{*path}", get(web::web_file))
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
        .route("/doc", get(get_doc))
        .route("/log", post(write_log))
        .nest("/session", session_routes())
        .nest("/provider", provider_routes())
        .nest("/config", config_routes())
        .nest("/external-adapter", external_adapter_routes())
        .nest("/mcp", mcp_routes())
        .nest("/file", file_routes())
        .nest("/find", find_routes())
        .nest("/permission", permission_routes())
        .nest("/project", project_routes())
        .nest("/pty", pty_routes())
        .nest("/question", question_routes())
        .nest("/tui", tui_routes())
        .nest("/process", process_routes())
        .nest("/task", task_routes())
        .nest("/workspace", workspace_routes())
        .nest("/global", global_routes())
        .nest("/experimental", experimental_routes())
        .nest("/plugin", plugin_auth_routes())
        .nest("/web-plugin", web_plugin_routes());

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

// --- /doc endpoint: returns OpenAPI-style documentation info ---

#[derive(Debug, Serialize)]
struct DocInfo {
    title: String,
    version: String,
    description: String,
    openapi: String,
}

#[derive(Debug, Serialize)]
struct DocResponse {
    info: DocInfo,
}

async fn get_doc() -> Json<DocResponse> {
    Json(DocResponse {
        info: DocInfo {
            title: "rocode".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: "rocode api".to_string(),
            openapi: "3.1.1".to_string(),
        },
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

#[derive(Debug, Deserialize)]
struct EventStreamQuery {
    /// Optional session ID to filter events by. When set, only events belonging
    /// to this session (or global events like `config.updated`) are forwarded.
    #[serde(default)]
    session: Option<String>,
    /// P2-1: subscription tier override (tui, web, cli). When absent, the
    /// server applies the legacy compatible default (full capabilities).
    #[serde(default)]
    tier: Option<String>,
}

async fn event_stream(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<EventStreamQuery>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    // P2-1: resolve subscription capabilities via the single wire-format
    // entry point in rocode-api. No other module parses tier strings.
    let subscription = rocode_api::ResolvedFrontendSubscription::from_wire_tier(
        query.tier.as_deref(),
    );
    tracing::debug!(
        tier = query.tier.as_deref().unwrap_or("default"),
        is_legacy = subscription.is_legacy_compat,
        "resolved frontend subscription for /event SSE"
    );
    // P2-2: pass subscription into stream_server_events for capability-based filtering.
    stream_server_events(state.event_bus.subscribe(), query.session, subscription)
}

const EVENT_OUTPUT_BLOCK_BATCH_MS: u64 = 16;

pub(crate) fn stream_server_events(
    mut rx: broadcast::Receiver<String>,
    session_filter: Option<String>,
    subscription: rocode_api::ResolvedFrontendSubscription,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let (tx, out_rx) = mpsc::channel(128);

    tokio::spawn(async move {
        let mut pending: Option<ServerEvent> = None;
        let mut pending_due_at: Option<tokio::time::Instant> = None;
        let delay = std::time::Duration::from_millis(EVENT_OUTPUT_BLOCK_BATCH_MS);

        // Closure to check if an event matches the session filter.
        // Global events (session_id == None) always pass through.
        let matches_filter = |event: &ServerEvent| -> bool {
            let Some(ref filter) = session_filter else {
                return true; // no filter — pass everything
            };
            match event.session_id() {
                Some(sid) => sid == filter.as_str(),
                None => true, // global events pass through
            }
        };

        // P2-2: subscription-aware event filter.
        let caps = subscription.capabilities;
        let skipped_count = std::sync::atomic::AtomicU64::new(0);
        let subscribable = |event: &ServerEvent| -> bool {
            let ok = event_passes_subscription_caps(event, &caps);
            if !ok {
                skipped_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            ok
        };

        // Same check but for raw JSON strings that failed to parse as ServerEvent.
        // Extract "sessionID" from JSON to apply filter.
        let raw_matches_filter = |raw: &str| -> bool {
            let Some(ref filter) = session_filter else {
                return true;
            };
            // Fast-path: if no "sessionID" key, treat as global.
            let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
                return true;
            };
            match value.get("sessionID").and_then(|v| v.as_str()) {
                Some(sid) => sid == filter.as_str(),
                None => {
                    // Also check "parentID" for attached_session events.
                    match value.get("parentID").and_then(|v| v.as_str()) {
                        Some(pid) => pid == filter.as_str(),
                        None => true, // global event
                    }
                }
            }
        };

        loop {
            if pending.is_some() {
                let due_at = pending_due_at.unwrap_or_else(|| tokio::time::Instant::now() + delay);
                tokio::select! {
                    recv = rx.recv() => {
                        match recv {
                            Ok(raw) => {
                                if let Some(next) = parse_server_event(&raw) {
                                    // Apply session filter — skip events for other sessions.
                                    if !matches_filter(&next) {
                                        continue;
                                    }
                                    // P2-2: apply subscription capability filter.
                                    if !subscribable(&next) {
                                        continue;
                                    }
                                    if let Some(current) = pending.as_mut() {
                                        if merge_output_block_delta(current, &next) {
                                            continue;
                                        }
                                    }
                                    if let Some(flushed) = pending.take() {
                                        pending_due_at = None;
                                        if send_server_event_json(&tx, &flushed).await.is_err() {
                                            break;
                                        }
                                    }
                                    if is_mergeable_output_delta(&next) {
                                        pending = Some(next);
                                        pending_due_at = Some(tokio::time::Instant::now() + delay);
                                    } else if send_server_event_json(&tx, &next).await.is_err() {
                                        break;
                                    }
                                } else {
                                    // Raw event that didn't parse — apply filter on raw JSON.
                                    if !raw_matches_filter(&raw) {
                                        continue;
                                    }
                                    if let Some(flushed) = pending.take() {
                                        pending_due_at = None;
                                        if send_server_event_json(&tx, &flushed).await.is_err() {
                                            break;
                                        }
                                    }
                                    if send_raw_server_event(&tx, raw).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                if let Some(flushed) = pending.take() {
                                    pending_due_at = None;
                                    if send_server_event_json(&tx, &flushed).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                let skipped = skipped_count.load(std::sync::atomic::Ordering::Relaxed);
                                if skipped > 0 {
                                    tracing::debug!(
                                        skipped,
                                        tier = ?subscription.tier,
                                        "SSE event stream closed; subscription-filtered events skipped"
                                    );
                                }
                                if let Some(flushed) = pending.take() {
                                    if let Err(error) = send_server_event_json(&tx, &flushed).await {
                                        let _ = error;
                                        tracing::debug!(
                                            "Failed to flush pending server event after broadcast channel closed"
                                        );
                                    }
                                }
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(due_at) => {
                        if let Some(flushed) = pending.take() {
                            pending_due_at = None;
                            if send_server_event_json(&tx, &flushed).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            } else {
                match rx.recv().await {
                    Ok(raw) => {
                        if let Some(event) = parse_server_event(&raw) {
                            // Apply session filter.
                            if !matches_filter(&event) {
                                continue;
                            }
                            // P2-2: apply subscription capability filter (same as pending branch).
                            if !subscribable(&event) {
                                continue;
                            }
                            if is_mergeable_output_delta(&event) {
                                pending = Some(event);
                                pending_due_at = Some(tokio::time::Instant::now() + delay);
                            } else if send_server_event_json(&tx, &event).await.is_err() {
                                break;
                            }
                        } else {
                            // Raw event — apply filter on raw JSON.
                            if !raw_matches_filter(&raw) {
                                continue;
                            }
                            if send_raw_server_event(&tx, raw).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(out_rx))
}

fn parse_server_event(raw: &str) -> Option<ServerEvent> {
    serde_json::from_str(raw).ok()
}

/// P2-2: subscription capability filter — pure function for testability.
/// Returns true if the event should be forwarded to this subscriber.
fn event_passes_subscription_caps(
    event: &ServerEvent,
    caps: &rocode_api::FrontendSubscriptionCapabilities,
) -> bool {
    if !caps.final_only && caps.reasoning_delta && caps.message_text_delta
        && caps.tool_progress && caps.runtime_live_view
    {
        return true; // full capabilities — no filtering needed
    }
    match event {
        ServerEvent::OutputBlock { block, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "reasoning" => !caps.final_only && caps.reasoning_delta,
                "message" => !caps.final_only && caps.message_text_delta,
                "scheduler_stage" | "tool" => !caps.final_only && caps.tool_progress,
                _ => !caps.final_only,
            }
        }
        ServerEvent::Usage { .. } => !caps.final_only && caps.runtime_live_view,
        // Non-droppable events: always pass.
        ServerEvent::SessionUpdated { .. }
        | ServerEvent::SessionStatus { .. }
        | ServerEvent::Error { .. }
        | ServerEvent::PermissionRequested { .. }
        | ServerEvent::PermissionResolved { .. }
        | ServerEvent::QuestionCreated { .. }
        | ServerEvent::QuestionResolved { .. }
        | ServerEvent::ToolCallLifecycle { .. }
        | ServerEvent::ConfigUpdated
        | ServerEvent::TopologyChanged { .. }
        | ServerEvent::AttachedSessionAttached { .. }
        | ServerEvent::AttachedSessionDetached { .. }
        | ServerEvent::DiffUpdated { .. } => true,
    }
}

fn is_mergeable_output_delta(event: &ServerEvent) -> bool {
    let ServerEvent::OutputBlock { id, block, .. } = event else {
        return false;
    };
    if id.as_deref().is_none_or(str::is_empty) {
        return false;
    }
    matches!(
        (
            block.get("kind").and_then(|value| value.as_str()),
            block.get("phase").and_then(|value| value.as_str()),
        ),
        (Some("message"), Some("delta")) | (Some("reasoning"), Some("delta"))
    )
}

fn merge_output_block_delta(current: &mut ServerEvent, next: &ServerEvent) -> bool {
    let (
        ServerEvent::OutputBlock {
            session_id: current_session,
            id: current_id,
            block: current_block,
        },
        ServerEvent::OutputBlock {
            session_id: next_session,
            id: next_id,
            block: next_block,
        },
    ) = (current, next)
    else {
        return false;
    };

    if current_session != next_session || current_id != next_id {
        return false;
    }

    let current_kind = current_block.get("kind").and_then(|value| value.as_str());
    let next_kind = next_block.get("kind").and_then(|value| value.as_str());
    let current_phase = current_block.get("phase").and_then(|value| value.as_str());
    let next_phase = next_block.get("phase").and_then(|value| value.as_str());
    if current_kind != next_kind || current_phase != Some("delta") || next_phase != Some("delta") {
        return false;
    }
    if current_kind == Some("message")
        && current_block.get("role").and_then(|value| value.as_str())
            != next_block.get("role").and_then(|value| value.as_str())
    {
        return false;
    }

    let Some(next_text) = next_block.get("text").and_then(|value| value.as_str()) else {
        return false;
    };
    let Some(current_text) = current_block
        .get_mut("text")
        .and_then(|value| value.as_str())
    else {
        return false;
    };

    current_block["text"] = serde_json::Value::String(format!("{current_text}{next_text}"));
    true
}

async fn send_raw_server_event(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    raw: String,
) -> std::result::Result<(), ()> {
    tx.send(Ok(Event::default().data(raw)))
        .await
        .map_err(|_| ())
}

async fn send_server_event_json(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    event: &ServerEvent,
) -> std::result::Result<(), ()> {
    let Some(json) = event.to_json_string() else {
        return Ok(());
    };
    send_raw_server_event(tx, json).await
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
    invocation: Option<rocode_command::CommandInvocationSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interactive: Option<rocode_command::CommandInteractiveSpec>,
    source: rocode_command::CommandSource,
}

#[derive(Debug, Clone, Serialize)]
struct UiCommandApiSpec {
    #[serde(flatten)]
    command: rocode_command::UiCommandSpec,
    argument_kind: rocode_command::UiCommandArgumentKind,
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
struct AgentInfo {
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
struct ExecutionModeInfo {
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
/// `ROCODE_INTERNAL_TOKEN` env var and sends it back in `x-rocode-internal-token` header.
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
        .get("x-rocode-plugin-internal")
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
        .get("x-rocode-internal-token")
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

async fn list_agents(
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
            return Err(ApiError::InternalError(format!(
                "Failed to load scheduler config for execution modes: {}",
                error
            )));
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

async fn list_execution_modes(
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

pub async fn refresh_agent_cache(config_store: &rocode_config::ConfigStore) {
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
) -> Result<Json<Vec<rocode_skill::SkillMetaView>>> {
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
    fn execution_modes_fail_explicitly_when_scheduler_config_cannot_be_loaded() {
        let config = AppConfig {
            scheduler_path: Some("/definitely/missing/rocode.scheduler.jsonc".to_string()),
            ..Default::default()
        };

        let error = build_execution_mode_list(Some(&config))
            .expect_err("broken scheduler config should fail mode listing explicitly");

        match error {
            ApiError::InternalError(message) => {
                assert!(message.contains("Failed to load scheduler config for execution modes"));
            }
            other => panic!("expected internal error, got {other:?}"),
        }
    }

    #[test]
    fn merge_output_block_delta_coalesces_message_text_for_same_session_and_id() {
        let mut current = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-1".to_string()),
            block: json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "hel",
            }),
        };
        let next = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-1".to_string()),
            block: json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "lo",
            }),
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
            block: json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "hel",
            }),
        };
        let next = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("msg-2".to_string()),
            block: json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "lo",
            }),
        };

        assert!(!merge_output_block_delta(&mut current, &next));
    }

    #[test]
    fn merge_output_block_delta_rejects_non_delta_or_non_output_events() {
        let mut current = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("reasoning-1".to_string()),
            block: json!({
                "kind": "reasoning",
                "phase": "delta",
                "text": "thinking",
            }),
        };
        let full = ServerEvent::OutputBlock {
            session_id: "session-a".to_string(),
            id: Some("reasoning-1".to_string()),
            block: json!({
                "kind": "reasoning",
                "phase": "full",
                "text": "thinking done",
            }),
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

    // ── P2-2 subscription filter tests ──────────────────────────────────

    fn reasoning_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "reasoning", "phase": "delta", "text": "think"}),
        }
    }

    fn message_block() -> ServerEvent {
        ServerEvent::OutputBlock {
            session_id: "sess-1".to_string(),
            id: Some("block-1".to_string()),
            block: serde_json::json!({"kind": "message", "phase": "delta", "text": "hello"}),
        }
    }

    fn caps(full: bool) -> rocode_api::FrontendSubscriptionCapabilities {
        if full {
            return rocode_api::FrontendSubscriptionCapabilities::default();
        }
        rocode_api::FrontendSubscriptionTier::CliLowFrequency.default_capabilities()
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

        let cli_sub = rocode_api::ResolvedFrontendSubscription::from_tier(
            rocode_api::FrontendSubscriptionTier::CliLowFrequency,
        );
        let sse = super::stream_server_events(rx, None, cli_sub);

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
        let collected = axum::body::to_bytes(body, 4096).await.expect("collect body");
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
}
