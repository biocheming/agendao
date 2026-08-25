use anyhow::Context;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, header::HeaderValue, request::Parts, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use agendao_memory::MemoryAuthority;
use agendao_plugin::init_global;
use agendao_plugin::subprocess::{
    PluginAuthBridge, PluginContext, PluginFetchRequest, PluginLoader,
};
use agendao_provider::ModelCatalogAuthority;
use agendao_provider::{
    bootstrap_config_from_raw, create_registry_from_bootstrap_config, register_custom_fetch_proxy,
    unregister_custom_fetch_proxy, AuthInfo, AuthManager, BootstrapConfig,
    ConfigModel as BootstrapConfigModel, ConfigProvider as BootstrapConfigProvider,
    CustomFetchProxy, CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    ProviderError, ProviderRegistry,
};
use agendao_runtime_context::{ResolvedWorkspaceContext, ResolvedWorkspaceContextAuthority};
use agendao_session::{SessionManager, SessionPrompt, SessionStateManager};
use agendao_state::UserStateAuthority;
use agendao_storage::{Database, MemoryRepository, MessageRepository, SessionRepository};

use crate::routes;
use crate::session_runtime::memory::RuntimeMemoryAuthority;
use crate::session_runtime::steering::SessionSteeringQueueStore;
use crate::session_runtime::telemetry::RuntimeTelemetryAuthority;
use agendao_server_core::runtime_events::{EventBusTelemetry, ServerBusEvent, ServerEvent};

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:3000";
const CATALOG_REFRESH_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// 启动阶段计时日志（可选诊断）。统一打到 `agendao::startup` target，
/// 用 `RUST_LOG=agendao::startup=info` 即可看到各阶段耗时，默认 warn 级别零开销。
macro_rules! startup_timing {
    ($phase:expr, $start:expr) => {
        tracing::info!(
            target: "agendao::startup",
            phase = $phase,
            elapsed_ms = $start.elapsed().as_millis() as u64,
            "startup phase done"
        )
    };
}

#[derive(Debug, Clone)]
pub struct ServerRuntimeOptions {
    pub port: u16,
    pub hostname: String,
    pub cwd: Option<PathBuf>,
    pub web_dist: Option<PathBuf>,
    pub embedded_web_assets: Option<crate::web::EmbeddedWebAssetLoader>,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    /// Optional Unix socket path for local IPC
    pub unix_socket_path: Option<String>,
}

struct PluginBridgeFetchProxy {
    bridge: Arc<PluginAuthBridge>,
    loader: Arc<PluginLoader>,
}

#[async_trait]
impl CustomFetchProxy for PluginBridgeFetchProxy {
    async fn fetch(
        &self,
        request: CustomFetchRequest,
    ) -> Result<CustomFetchResponse, ProviderError> {
        self.loader.touch_activity();
        let response = self
            .bridge
            .fetch_proxy(PluginFetchRequest {
                url: request.url,
                method: request.method,
                headers: request.headers,
                body: request.body,
            })
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        Ok(CustomFetchResponse {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }

    async fn fetch_stream(
        &self,
        request: CustomFetchRequest,
    ) -> Result<CustomFetchStreamResponse, ProviderError> {
        self.loader.touch_activity();
        let response = self
            .bridge
            .fetch_proxy_stream(PluginFetchRequest {
                url: request.url,
                method: request.method,
                headers: request.headers,
                body: request.body,
            })
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let stream = ReceiverStream::new(response.chunks)
            .map(|item| item.map_err(|e| ProviderError::NetworkError(e.to_string())));
        Ok(CustomFetchStreamResponse {
            status: response.status,
            headers: response.headers,
            stream: Box::pin(stream),
        })
    }
}

pub(crate) fn sync_custom_fetch_proxy(
    provider_id: &str,
    bridge: Arc<PluginAuthBridge>,
    loader: &Arc<PluginLoader>,
    enabled: bool,
) {
    if enabled {
        register_custom_fetch_proxy(
            provider_id.to_string(),
            Arc::new(PluginBridgeFetchProxy {
                bridge: bridge.clone(),
                loader: Arc::clone(loader),
            }),
        );
        if provider_id == "github-copilot" {
            register_custom_fetch_proxy(
                "github-copilot-enterprise",
                Arc::new(PluginBridgeFetchProxy {
                    bridge,
                    loader: Arc::clone(loader),
                }),
            );
        }
    } else {
        unregister_custom_fetch_proxy(provider_id);
        if provider_id == "github-copilot" {
            unregister_custom_fetch_proxy("github-copilot-enterprise");
        }
    }
}

pub(crate) async fn refresh_plugin_auth_state(
    loader: &Arc<PluginLoader>,
    auth_manager: Arc<AuthManager>,
) -> bool {
    let mut any_custom_fetch = false;
    let bridges = loader.auth_bridges().await;
    for (provider_id, bridge) in bridges {
        match bridge.load().await {
            Ok(result) => {
                any_custom_fetch |= result.has_custom_fetch;
                sync_custom_fetch_proxy(
                    &provider_id,
                    bridge.clone(),
                    loader,
                    result.has_custom_fetch,
                );

                if let Some(api_key) = result.api_key {
                    auth_manager
                        .set(
                            &provider_id,
                            AuthInfo::Api {
                                key: api_key.clone(),
                            },
                        )
                        .await;
                    if provider_id == "github-copilot" {
                        auth_manager
                            .set("github-copilot-enterprise", AuthInfo::Api { key: api_key })
                            .await;
                    }
                }
            }
            Err(error) => {
                sync_custom_fetch_proxy(&provider_id, bridge.clone(), loader, false);
                tracing::warn!(provider = provider_id, %error, "failed to load plugin auth");
            }
        }
    }
    any_custom_fetch
}

fn plugin_idle_timeout() -> Duration {
    let secs = std::env::var("AGENDAO_PLUGIN_IDLE_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(90);
    Duration::from_secs(secs)
}

fn spawn_plugin_idle_monitor(loader: Arc<PluginLoader>) {
    let timeout = plugin_idle_timeout();
    if timeout.is_zero() {
        tracing::info!("plugin idle shutdown disabled (timeout=0)");
        return;
    }
    let poll = Duration::from_secs((timeout.as_secs() / 3).clamp(5, 30));

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(poll).await;
            if !loader.has_live_clients().await {
                continue;
            }
            if !loader.is_idle_for(timeout) {
                continue;
            }

            let bridges = loader.auth_bridges().await;
            for (provider_id, bridge) in bridges {
                sync_custom_fetch_proxy(&provider_id, bridge, &loader, false);
            }
            loader.shutdown_all().await;
            tracing::info!(
                timeout_secs = timeout.as_secs(),
                "plugin subprocesses shut down due to idleness"
            );
        }
    });
}

pub struct ServerState {
    pub(crate) workspace_root: PathBuf,
    pub(crate) sessions: Arc<Mutex<SessionManager>>,
    pub(crate) providers: Arc<tokio::sync::RwLock<ProviderRegistry>>,
    pub(crate) catalog_authority: Arc<ModelCatalogAuthority>,
    pub(crate) resolved_context: tokio::sync::RwLock<ResolvedWorkspaceContext>,
    pub(crate) config_store: Arc<agendao_config::ConfigStore>,
    pub(crate) external_tool_catalogs: Arc<Vec<agendao_config::ResolvedExternalToolCatalog>>,
    pub(crate) user_state: Arc<UserStateAuthority>,
    pub(crate) resolved_context_authority: Arc<ResolvedWorkspaceContextAuthority>,
    pub(crate) tool_registry: Arc<agendao_tool::ToolRegistry>,
    /// The single sandbox launch authority for tool execution: every
    /// model-reachable process spawn flows through here (Phase 4).
    pub(crate) sandbox_authority: Arc<crate::sandbox_authority::SandboxAuthority>,
    pub(crate) prompt_runner: Arc<SessionPrompt>,
    pub(crate) runtime_memory: Arc<RuntimeMemoryAuthority>,
    pub(crate) runtime_telemetry: Arc<RuntimeTelemetryAuthority>,
    pub(crate) steering_store: Arc<tokio::sync::Mutex<SessionSteeringQueueStore>>,
    pub submission_authority: Arc<agendao_server_core::submission_authority::SubmissionAuthority>,
    pub(crate) queued_followups:
        Arc<tokio::sync::Mutex<HashMap<String, std::collections::VecDeque<serde_json::Value>>>>,
    /// Task-governance stall observation windows (server memory only).
    pub(crate) stall_windows: Arc<crate::session_runtime::task_ledger_stall::StallWindows>,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) event_bus: broadcast::Sender<Arc<ServerBusEvent>>,
    /// Canonical bus for projected FrontendEvents. All transports (SSE, Unix,
    /// Direct) consume from this bus once wired. Populated by the single
    /// FrontendProjector subscriber.
    pub(crate) frontend_bus:
        broadcast::Sender<Arc<agendao_server_core::frontend_events::FrontendBusEvent>>,
    /// Guards `ensure_frontend_projector()`: `true` once the background
    /// projector task has been spawned. The guard tracks projector lifecycle
    /// directly, independent of downstream transport subscriber count.
    pub(crate) frontend_projector_spawned: std::sync::atomic::AtomicBool,
    catalog_refresh_loop_spawned: std::sync::atomic::AtomicBool,
    /// Observable event bus telemetry (P0-2). Tracks send volume, errors, and
    /// receiver count so operators can distinguish event backlog from other
    /// sources of CPU/memory pressure.
    pub(crate) event_bus_telemetry: Option<Arc<EventBusTelemetry>>,
    pub(crate) api_perf: Arc<ApiPerfCounters>,
    pub(crate) session_repo: Option<SessionRepository>,
    pub(crate) message_repo: Option<MessageRepository>,
    pub(crate) external_adapter_replay_repo:
        Option<Arc<agendao_storage::ExternalAdapterReplayRepository>>,
    pub(crate) proposal_repo: Option<Arc<agendao_storage::SkillEvolutionProposalRepository>>,
    pub(crate) category_registry: Arc<agendao_config::CategoryRegistry>,
    /// Shared todo state: written by TodoWrite tool callbacks, read by the
    /// session todos route.
    pub(crate) todo_manager: Arc<agendao_session::TodoManager>,
    /// Stale-file guard shared across read→write tool sequences.
    pub(crate) file_time_tracker: Arc<agendao_tool::FileTimeTracker>,
    /// Cancellation token for the background recheck/wake loop.
    /// Cancelled on ServerState drop.
    pub(crate) recheck_cancel: tokio_util::sync::CancellationToken,
    /// 懒加载记账：启动 `load_sessions_from_storage` 只恢复元数据，被标记的
    /// 会话消息体尚未入内存。`ensure_session_messages_hydrated` 在消息读/写
    /// 闸门处按需回填并移出集合；flush 路径对仍 dehydrated 的会话只写 session
    /// 行、绝不动 messages（防止空内存镜像删光库存消息）。
    pub(crate) dehydrated_sessions: tokio::sync::RwLock<std::collections::HashSet<String>>,
    session_persist_gates: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}

impl Drop for ServerState {
    fn drop(&mut self) {
        self.recheck_cancel.cancel();
    }
}

pub struct ApiPerfCounters {
    pub(crate) list_messages_calls: AtomicU64,
    pub(crate) list_messages_incremental_calls: AtomicU64,
    pub(crate) list_messages_full_calls: AtomicU64,
}

impl ApiPerfCounters {
    pub fn new() -> Self {
        Self {
            list_messages_calls: AtomicU64::new(0),
            list_messages_incremental_calls: AtomicU64::new(0),
            list_messages_full_calls: AtomicU64::new(0),
        }
    }
}

impl Default for ApiPerfCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerState {
    /// Subscribe directly to the canonical projected frontend event bus.
    /// Direct transports must create this receiver before submitting a prompt:
    /// `broadcast` has no replay and a late subscription would otherwise lose
    /// the first planning/model events.
    pub fn subscribe_frontend_events(
        &self,
    ) -> broadcast::Receiver<Arc<agendao_server_core::frontend_events::FrontendBusEvent>> {
        self.frontend_bus.subscribe()
    }

    pub fn new() -> Self {
        Self::new_for_workspace(default_workspace_root())
    }

    pub fn new_for_workspace(workspace_root: PathBuf) -> Self {
        let config_store = Arc::new(agendao_config::ConfigStore::new(
            agendao_config::Config::default(),
        ));
        let user_state = Arc::new(UserStateAuthority::from_config_store(&config_store));
        let resolved_context_authority = Arc::new(ResolvedWorkspaceContextAuthority::new(
            config_store.clone(),
            user_state.clone(),
        ));
        let runtime_memory = Arc::new(RuntimeMemoryAuthority::new(Arc::new(MemoryAuthority::new(
            user_state.clone(),
            resolved_context_authority.clone(),
        ))));
        let (tx, _) = broadcast::channel(1024);
        let (frontend_tx, _) = broadcast::channel(1024);
        let event_bus_telemetry = Arc::new(EventBusTelemetry::default());
        let runtime_telemetry = Arc::new(RuntimeTelemetryAuthority::new(
            tx.clone(),
            Some(event_bus_telemetry.clone()),
        ));
        let steering_store = Arc::new(tokio::sync::Mutex::new(SessionSteeringQueueStore::new()));
        let queued_followups = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let todo_manager = Arc::new(agendao_session::TodoManager::new());
        let file_time_tracker = Arc::new(agendao_tool::FileTimeTracker::default());
        let sandbox_authority = Arc::new(crate::sandbox_authority::SandboxAuthority::new(
            crate::sandbox_authority::SandboxAuthorityConfig::for_session(
                agendao_types::SessionPermissionMode::Default,
            ),
            crate::sandbox_authority::production_backend_registry(),
            Arc::new(crate::sandbox_authority::ProjectingSandboxEventSink::new(
                tx.clone(),
            )),
        ));
        Self {
            workspace_root: normalize_workspace_root(workspace_root),
            sessions: Arc::new(Mutex::new(SessionManager::new())),
            providers: Arc::new(tokio::sync::RwLock::new(ProviderRegistry::new())),
            catalog_authority: agendao_provider::default_model_catalog_authority(),
            resolved_context: tokio::sync::RwLock::new(ResolvedWorkspaceContext::empty()),
            config_store,
            external_tool_catalogs: Arc::new(Vec::new()),
            user_state,
            resolved_context_authority,
            tool_registry: Arc::new(agendao_tool::ToolRegistry::new()),
            sandbox_authority,
            prompt_runner: Arc::new(
                SessionPrompt::new(Arc::new(tokio::sync::RwLock::new(
                    SessionStateManager::new(),
                )))
                .with_memory_authority(runtime_memory.memory_authority())
                .with_todo_manager(todo_manager.clone())
                .with_file_time_tracker(file_time_tracker.clone()),
            ),
            runtime_memory,
            runtime_telemetry,
            steering_store,
            submission_authority: Arc::new(
                agendao_server_core::submission_authority::SubmissionAuthority::new(),
            ),
            queued_followups,
            stall_windows: Arc::new(
                crate::session_runtime::task_ledger_stall::StallWindows::default(),
            ),
            auth_manager: Arc::new(AuthManager::new()),
            event_bus: tx,
            frontend_bus: frontend_tx,
            frontend_projector_spawned: std::sync::atomic::AtomicBool::new(false),
            catalog_refresh_loop_spawned: std::sync::atomic::AtomicBool::new(false),
            event_bus_telemetry: Some(event_bus_telemetry),
            api_perf: Arc::new(ApiPerfCounters::new()),
            session_repo: None,
            message_repo: None,
            external_adapter_replay_repo: None,
            proposal_repo: None,
            category_registry: Arc::new(agendao_config::CategoryRegistry::empty()),
            todo_manager,
            file_time_tracker,
            recheck_cancel: tokio_util::sync::CancellationToken::new(),
            dehydrated_sessions: tokio::sync::RwLock::new(std::collections::HashSet::new()),
            session_persist_gates: Mutex::new(HashMap::new()),
        }
    }

    async fn session_persist_gate(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.session_persist_gates.lock().await;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(session_id).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(Mutex::new(()));
        gates.insert(session_id.to_string(), Arc::downgrade(&gate));
        gate
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn project_root(&self) -> PathBuf {
        self.config_store
            .project_dir()
            .unwrap_or_else(|| self.workspace_root.clone())
    }

    /// Capture the permission mode for a session and construct an immutable
    /// sandbox authority for that launch.  Authorities are deliberately not
    /// cached or mutated: a permission-mode update affects subsequent
    /// launches, while an already prepared execution keeps its own snapshot.
    pub async fn sandbox_authority_for_session(
        &self,
        session_id: &str,
    ) -> Arc<crate::sandbox_authority::SandboxAuthority> {
        let mode = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|session| session.permission.as_ref().map(|rules| rules.mode))
                .unwrap_or_default()
        };
        Arc::new(self.sandbox_authority.for_session_mode(mode))
    }

    /// Native execution is a session-mode capability hint only; the sandbox
    /// authority remains the final policy裁决点.
    pub const fn sandbox_native_allowed_for_mode(
        mode: agendao_types::SessionPermissionMode,
    ) -> bool {
        matches!(mode, agendao_types::SessionPermissionMode::UnsandboxedYolo)
    }

    pub async fn new_with_storage() -> anyhow::Result<Self> {
        Self::new_with_storage_for_url_in_workspace(
            DEFAULT_SERVER_URL.to_string(),
            default_workspace_root(),
        )
        .await
    }

    pub async fn new_with_storage_for_url(server_url: String) -> anyhow::Result<Self> {
        Self::new_with_storage_for_url_in_workspace(server_url, default_workspace_root()).await
    }

    pub async fn new_with_storage_for_url_in_workspace(
        server_url: String,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Self> {
        let startup_begin = std::time::Instant::now();
        let mut phase_start = startup_begin;
        let workspace_root = normalize_workspace_root(workspace_root);

        // Configuration is a startup contract. Parse it before opening the
        // database or spawning plugins so an invalid file fails immediately
        // without leaving partially initialized background work behind.
        let config_store = Arc::new(
            agendao_config::ConfigStore::from_project_dir(&workspace_root).with_context(|| {
                format!(
                    "failed to load workspace configuration for {}",
                    workspace_root.display()
                )
            })?,
        );
        startup_timing!("config_load", phase_start);
        phase_start = std::time::Instant::now();

        // SQLite 打开 + 迁移（~数百 ms 的 IO/DDL）与后续 plugin/catalog 阶段
        // 互不依赖，提前 spawn 让它们在后台并行推进。
        let db_spawn_start = std::time::Instant::now();
        let db_handle = tokio::spawn(Database::new());

        let mut state = Self::new_for_workspace(workspace_root.clone());
        let auth_manager = Arc::new(AuthManager::load_from_file(&auth_data_dir()).await);
        state.auth_manager = auth_manager.clone();
        startup_timing!("auth_load", phase_start);

        let user_state = Arc::new(UserStateAuthority::from_config_store(&config_store));
        let resolved_context_authority = Arc::new(ResolvedWorkspaceContextAuthority::new(
            config_store.clone(),
            user_state.clone(),
        ));

        // 插件子进程引导（内建 auth 插件 spawn 等）与 models catalog 加载、
        // 后台的 database_open 互不依赖，先 spawn 并行推进；在读取 auth_store
        // 前 join，保持「registry 构建能看到插件写入的 auth 状态」这一既有语义。
        let plugin_spawn_start = std::time::Instant::now();
        let plugin_handle = {
            let server_url = server_url.clone();
            let auth_manager = auth_manager.clone();
            let config_store = config_store.clone();
            let workspace_root = workspace_root.clone();
            // Plugin hosts are user-configured integrations: they launch
            // contained through the server's sandbox authority, never a
            // direct spawn.
            let plugin_sandbox = agendao_sandbox::IntegrationSandboxContext::new(
                state.sandbox_authority.clone(),
                workspace_root.clone(),
                agendao_sandbox::plugin_runtime_roots(),
            )
            .expect("built-in integration runtime roots must resolve");
            tokio::spawn(async move {
                load_plugin_auth_store(
                    &server_url,
                    auth_manager,
                    &config_store,
                    &workspace_root,
                    plugin_sandbox,
                )
                .await;
            })
        };
        phase_start = std::time::Instant::now();
        let bootstrap_config = {
            let config = config_store.config();
            bootstrap_config_from_config(&config)
        };

        // Ensure models.dev cache exists before bootstrap (which reads it synchronously).
        {
            let provider_count = tokio::time::timeout(Duration::from_secs(10), async {
                state
                    .catalog_authority
                    .map_snapshot(|snapshot| snapshot.data.len())
                    .await
            })
            .await;
            match provider_count {
                Ok(count) => {
                    tracing::info!(providers = count, "models.dev cache ready");
                }
                Err(_) => {
                    tracing::warn!(
                        "timed out fetching models.dev data; built-in model list may be incomplete"
                    );
                }
            }
        }
        startup_timing!("models_catalog", phase_start);
        if let Err(error) = plugin_handle.await {
            tracing::warn!(%error, "plugin bootstrap task failed to join");
        }
        startup_timing!("plugin_bootstrap", plugin_spawn_start);
        phase_start = std::time::Instant::now();
        let auth_store = auth_manager.list().await;

        state.providers = Arc::new(tokio::sync::RwLock::new(
            create_registry_from_bootstrap_config(&bootstrap_config, &auth_store),
        ));
        startup_timing!("provider_registry_build", phase_start);
        phase_start = std::time::Instant::now();
        state.config_store = config_store.clone();
        state.user_state = user_state;
        state.resolved_context_authority = resolved_context_authority.clone();
        state.runtime_memory =
            Arc::new(RuntimeMemoryAuthority::new(Arc::new(MemoryAuthority::new(
                state.user_state.clone(),
                state.resolved_context_authority.clone(),
            ))));
        let _ = state.refresh_resolved_context().await;
        startup_timing!("resolved_context", phase_start);
        phase_start = std::time::Instant::now();

        // Load task category registry from configured path
        let category_registry = if let Some(path) = config_store.resolved_task_category_path().await
        {
            match agendao_config::CategoryRegistry::load(&path) {
                Ok(registry) => {
                    tracing::info!(
                        path = %path.display(),
                        "loaded task category registry"
                    );
                    registry
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        path = %path.display(),
                        "failed to load task category registry, using builtins"
                    );
                    agendao_config::CategoryRegistry::with_builtins()
                }
            }
        } else {
            agendao_config::CategoryRegistry::with_builtins()
        };
        state.category_registry = Arc::new(category_registry);
        let tool_runtime_config =
            agendao_tool::ToolRuntimeConfig::from_config(&config_store.config());
        let external_tool_catalogs = agendao_config::load_external_tool_catalogs_for_project(
            &workspace_root,
        )
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to load external tool catalogs from toolImports");
            Vec::new()
        });
        state.tool_registry = Arc::new(
            agendao_tool::create_default_registry_with_config(Some(&config_store.config())).await,
        );
        startup_timing!("tool_registry", phase_start);
        state.external_tool_catalogs = Arc::new(external_tool_catalogs);
        let db = db_handle
            .await
            .map_err(|error| anyhow::anyhow!("database open task failed to join: {}", error))??;
        startup_timing!("database_open", db_spawn_start);
        phase_start = std::time::Instant::now();
        let pool = db.pool().clone();
        let memory_repo = Arc::new(MemoryRepository::new(pool.clone()));
        let memory_authority = Arc::new(
            MemoryAuthority::new(
                state.user_state.clone(),
                state.resolved_context_authority.clone(),
            )
            .with_repository(memory_repo.clone()),
        );
        state.runtime_memory = Arc::new(RuntimeMemoryAuthority::new(memory_authority.clone()));
        let proposal_repo = Arc::new(agendao_storage::SkillEvolutionProposalRepository::new(
            pool.clone(),
        ));
        state.proposal_repo = Some(proposal_repo.clone());
        state.external_adapter_replay_repo = Some(Arc::new(
            agendao_storage::ExternalAdapterReplayRepository::new(pool.clone()),
        ));
        state.prompt_runner = Arc::new(
            SessionPrompt::new(Arc::new(tokio::sync::RwLock::new(
                SessionStateManager::new(),
            )))
            .with_config_store(config_store.clone())
            .with_tool_runtime_config(tool_runtime_config)
            .with_memory_authority(memory_authority)
            .with_proposal_repo(proposal_repo)
            .with_todo_manager(state.todo_manager.clone())
            .with_file_time_tracker(state.file_time_tracker.clone()),
        );
        state.session_repo = Some(SessionRepository::new(pool.clone()));
        state.message_repo = Some(MessageRepository::new(pool));
        state.load_sessions_from_storage().await?;
        startup_timing!("sessions_restore", phase_start);
        startup_timing!("total", startup_begin);
        // Spawn background recheck/wake loop, cancelled when ServerState is dropped.
        crate::session_runtime::recheck_loop::spawn_recheck_wake_loop(
            state.runtime_telemetry.clone(),
            state.recheck_cancel.clone(),
        );
        Ok(state)
    }

    /// Broadcast a typed ServerEvent on the in-process event bus.
    ///
    /// The event is shared with all subscribers through `Arc` with no JSON
    /// round trip; the JSON wire text is materialized lazily (and at most
    /// once) when a network-boundary subscriber demands it.
    pub fn broadcast_event(&self, event: ServerEvent) {
        self.send_on_event_bus(ServerBusEvent::event(event));
    }

    fn send_on_event_bus(&self, payload: ServerBusEvent) {
        let receiver_count = self.event_bus.receiver_count();
        if self.event_bus.send(Arc::new(payload)).is_err() {
            tracing::warn!("failed to broadcast server event (no active receivers)");
            if let Some(ref telemetry) = self.event_bus_telemetry {
                telemetry.record_send_error();
            }
        } else if let Some(ref telemetry) = self.event_bus_telemetry {
            telemetry.record_send(receiver_count);
        }
    }

    /// Ensures the FrontendEvent projector is running.
    ///
    /// Idempotent: tracks projector lifecycle via an `AtomicBool` guard
    /// (not via downstream `frontend_bus` subscriber count — the projector
    /// subscribes to `event_bus`, not `frontend_bus`). Safe to call from
    /// any server entry point; the projector is spawned at most once.
    pub fn ensure_frontend_projector(&self) {
        use std::sync::atomic::Ordering;
        if !self.frontend_projector_spawned.swap(true, Ordering::SeqCst) {
            crate::session_runtime::frontend_projection::spawn_frontend_projector(
                self.event_bus.clone(),
                self.frontend_bus.clone(),
                self.runtime_telemetry.clone(),
                self.sessions.clone(),
            );
        }
    }

    /// Keep the shared models.dev catalogue fresh without delaying startup.
    /// The first tick runs immediately; later hourly ticks are cheap TTL checks
    /// and only perform network I/O once the 24-hour catalogue TTL expires.
    pub fn ensure_catalog_refresh_loop(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;
        if self
            .catalog_refresh_loop_spawned
            .swap(true, Ordering::SeqCst)
        {
            return;
        }

        let state = Arc::downgrade(self);
        let cancel = self.recheck_cancel.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CATALOG_REFRESH_CHECK_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = interval.tick() => {}
                }

                let Some(state) = state.upgrade() else {
                    break;
                };
                let generation_before = state.catalog_authority.snapshot().await.generation;
                let Some(result) = state.catalog_authority.refresh_if_stale().await else {
                    continue;
                };

                if result.snapshot.generation != generation_before {
                    state.rebuild_providers().await;
                    crate::session_runtime::events::broadcast_config_updated(state.as_ref());
                    tracing::info!(
                        generation = result.snapshot.generation,
                        "refreshed stale models.dev catalogue and rebuilt providers"
                    );
                } else if let Some(error) = result.error_message.as_deref() {
                    tracing::warn!(%error, "failed to refresh stale models.dev catalogue; using cached snapshot");
                } else {
                    tracing::debug!(status = ?result.status, "validated models.dev catalogue freshness");
                }
            }
        });
    }

    /// Rebuild the provider registry from the stored bootstrap config,
    /// derived from the current config store plus the current auth store.
    /// Call this after auth/config mutations so newly connected providers become
    /// available immediately and the registry stays single-sourced.
    pub async fn rebuild_providers(&self) {
        let auth_store = self.auth_manager.list().await;
        let config = self.config_store.config();
        let bootstrap_config = bootstrap_config_from_config(&config);
        let new_registry = create_registry_from_bootstrap_config(&bootstrap_config, &auth_store);
        let _ = self.refresh_resolved_context().await;

        *self.providers.write().await = new_registry;
    }

    /// Rebuild the tool registry against the current config so `disabled_tools`
    /// changes take effect without a server restart. The swap happens in place
    /// (`ToolRegistry::replace_with`), so sessions/tasks holding the same
    /// `Arc<ToolRegistry>` observe the new tool set on their next lookup.
    pub async fn rebuild_tool_registry(&self) {
        let config = self.config_store.config();
        let new_registry = agendao_tool::create_default_registry_with_config(Some(&config)).await;
        self.tool_registry.replace_with(new_registry).await;
    }

    pub async fn refresh_resolved_context(&self) -> anyhow::Result<ResolvedWorkspaceContext> {
        let resolved = self.resolved_context_authority.resolve().await?;
        *self.resolved_context.write().await = resolved.clone();
        Ok(resolved)
    }

    async fn load_sessions_from_storage(&self) -> anyhow::Result<()> {
        let Some(session_repo) = &self.session_repo else {
            return Ok(());
        };

        // 懒加载（水律·回流按需）：启动只恢复元数据（标题/用量/时间戳均在
        // sessions 行内），**不逐会话拉取消息体**。此前每个 session 一条
        // N+1 SQL + 全量消息 JSON 三遍往返（from_str/to_value/from_value），
        // 大库（>100MB 消息）下启动要花数秒。消息体由
        // `ensure_session_messages_hydrated` 在打开/prompt/fork 时按需回填。
        let stored_sessions = session_repo.list(None, 100_000).await?;
        let mut manager = self.sessions.lock().await;
        let mut dehydrated = self.dehydrated_sessions.write().await;

        for stored in stored_sessions {
            // `stored` 已是 agendao_types::Session（即 SessionRecord），直接包裹，
            // 避免逐会话 serde 往返（to_value/from_value 对含大 metadata 的会话
            // 是纯 CPU 浪费，大库下占启动恢复耗时的大头）。
            let id = stored.id.clone();
            let session: agendao_session::Session = agendao_session::Session::from(stored);
            manager.restore(session);
            dehydrated.insert(id);
        }

        Ok(())
    }

    /// 懒水合闸门：首次访问某会话的消息体时从 storage 回填。
    ///
    /// 快路径零成本（不在 dehydrated 集合 → 直接返回）。回填仅在内存消息
    /// 仍为空时进行——绝不覆盖运行中会话已入内存的新消息（配对防护：
    /// 与 flush 路径的「dehydrated 只写行」守卫共同保证库存消息不被清）。
    pub(crate) async fn ensure_session_messages_hydrated(
        &self,
        session_id: &str,
    ) -> anyhow::Result<()> {
        if !self.dehydrated_sessions.read().await.contains(session_id) {
            return Ok(());
        }
        let Some(message_repo) = &self.message_repo else {
            return Ok(());
        };
        let messages = message_repo.list_for_session(session_id).await?;
        let mut manager = self.sessions.lock().await;
        if let Some(session) = manager.get_mut(session_id) {
            if session.record().messages.is_empty() {
                *session.messages_mut() = messages;
            }
        }
        drop(manager);
        self.dehydrated_sessions.write().await.remove(session_id);
        Ok(())
    }

    /// Flush a single session (and its messages) to storage inside a transaction.
    /// The shared Arc is an immutable snapshot; unlike the old path this does
    /// not clone the Session or round-trip it through serde before the write.
    /// Used at message/run boundaries — avoids scanning all sessions.
    pub async fn flush_session_to_storage(&self, session_id: &str) -> anyhow::Result<()> {
        let Some(session_repo) = &self.session_repo else {
            return Ok(());
        };
        let gate = self.session_persist_gate(session_id).await;
        let _persist_guard = gate.lock().await;
        let session = {
            let manager = self.sessions.lock().await;
            manager.get_shared(session_id)
        };

        let Some(session) = session else {
            return Ok(());
        };

        let record = session.record();

        // 懒加载守卫：会话消息体未水合时，内存里的空消息列表不是真相——
        // 只 upsert session 行（rename/用量等元数据变更照写），**绝不进入
        // flush_with_messages 的 delete-stale 分支**（否则库存消息被清空）。
        if self.dehydrated_sessions.read().await.contains(session_id) {
            session_repo.upsert(record).await?;
            return Ok(());
        }

        session_repo
            .flush_with_messages(record, &record.messages)
            .await?;
        self.runtime_memory.ingest_session_record(record).await?;

        Ok(())
    }

    /// Persist session-row state without rewriting its message collection.
    ///
    /// Governance seams only mutate metadata/status fields. Keeping those
    /// commits durable via a row upsert avoids serializing and upserting every
    /// historical message several times during one run.
    pub async fn flush_session_record_to_storage(&self, session_id: &str) -> anyhow::Result<()> {
        let Some(session_repo) = &self.session_repo else {
            return Ok(());
        };
        let gate = self.session_persist_gate(session_id).await;
        let _persist_guard = gate.lock().await;
        let session = {
            let manager = self.sessions.lock().await;
            manager.get_shared(session_id)
        };
        let Some(session) = session else {
            return Ok(());
        };

        session_repo.upsert(session.record()).await?;

        Ok(())
    }

    pub async fn sync_sessions_to_storage(&self) -> anyhow::Result<()> {
        let (Some(session_repo), Some(message_repo)) = (&self.session_repo, &self.message_repo)
        else {
            return Ok(());
        };

        let snapshot: Vec<Arc<agendao_session::Session>> = {
            let manager = self.sessions.lock().await;
            manager.list_shared()
        };

        // Clean up sessions that were deleted in-memory but still persisted.
        let snapshot_ids: HashSet<String> = snapshot.iter().map(|s| s.id.clone()).collect();
        let persisted = session_repo.list(None, 100_000).await?;

        for stale in persisted {
            if !snapshot_ids.contains(&stale.id) {
                message_repo.delete_for_session(&stale.id).await?;
                session_repo.delete(&stale.id).await?;
            }
        }

        // Flush each session transactionally (upsert session + messages + delete stale).
        for session in snapshot {
            let record = session.record();

            // 懒加载守卫：dehydrated 会话的内存消息为空——只写 session 行，
            // 跳过 messages 的 upsert/delete-stale（防库存消息被空镜像清掉）。
            if self.dehydrated_sessions.read().await.contains(&record.id) {
                session_repo.upsert(record).await?;
                continue;
            }

            session_repo
                .flush_with_messages(record, &record.messages)
                .await?;
            self.runtime_memory.ingest_session_record(record).await?;
        }

        Ok(())
    }
}

/// Convert agendao_config::ProviderConfig map to bootstrap ConfigProvider map.
fn convert_config_providers_for_bootstrap(
    config: &agendao_config::Config,
) -> std::collections::HashMap<String, BootstrapConfigProvider> {
    let Some(ref providers) = config.provider else {
        return std::collections::HashMap::new();
    };

    providers
        .iter()
        .map(|(id, provider)| (id.clone(), provider_to_bootstrap(provider)))
        .collect()
}

pub(crate) fn bootstrap_config_from_config(config: &agendao_config::Config) -> BootstrapConfig {
    let providers = convert_config_providers_for_bootstrap(config);
    bootstrap_config_from_raw(
        providers,
        config.disabled_providers.clone(),
        config.enabled_providers.clone(),
        config.model.clone(),
        config.small_model.clone(),
    )
}

fn provider_to_bootstrap(provider: &agendao_config::ProviderConfig) -> BootstrapConfigProvider {
    let options = provider.options.clone().unwrap_or_default();

    let models = provider.models.as_ref().map(|models| {
        models
            .iter()
            .map(|(id, model)| (id.clone(), model_to_bootstrap(id, model)))
            .collect()
    });

    BootstrapConfigProvider {
        name: provider.name.clone(),
        api_key: provider.api_key.clone(),
        api: provider.base_url.clone(),
        npm: provider.npm.clone(),
        api_style: provider.api_style.clone(),
        api_shape: provider.api_shape.clone(),
        transport: provider.transport.clone(),
        usage_shape: provider.usage_shape.clone(),
        quirks: (!provider.quirks.is_empty()).then_some(provider.quirks.clone()),
        options: (!options.is_empty()).then_some(options),
        models,
        blacklist: (!provider.blacklist.is_empty()).then_some(provider.blacklist.clone()),
        whitelist: (!provider.whitelist.is_empty()).then_some(provider.whitelist.clone()),
        ..Default::default()
    }
}

fn model_to_bootstrap(id: &str, model: &agendao_config::ModelConfig) -> BootstrapConfigModel {
    let variants = model.variants.as_ref().map(|variants| {
        variants
            .iter()
            .map(|(name, variant)| (name.clone(), variant_to_bootstrap(variant)))
            .collect()
    });

    BootstrapConfigModel {
        id: model.model.clone().or_else(|| Some(id.to_string())),
        name: model.name.clone(),
        provider: model.base_url.as_ref().map(|url| {
            agendao_provider::bootstrap::ConfigModelProvider {
                api: Some(url.clone()),
                npm: None,
            }
        }),
        options: model.options.clone(),
        variants,
        ..Default::default()
    }
}

fn variant_to_bootstrap(
    variant: &agendao_config::ModelVariantConfig,
) -> HashMap<String, serde_json::Value> {
    let mut values = variant.extra.clone();
    if let Some(disabled) = variant.disabled {
        values.insert("disabled".to_string(), serde_json::Value::Bool(disabled));
    }
    values
}

fn auth_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("AGENDAO_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    // 凭证统一收在 agendao_home 根（~/.agendao/auth.json,土律·单点权威）;
    // AGENDAO_DATA_DIR 保留为向后兼容覆盖通道。
    agendao_util::agendao_home()
}

async fn load_plugin_auth_store(
    server_url: &str,
    auth_manager: Arc<AuthManager>,
    config_store: &agendao_config::ConfigStore,
    workspace_root: &Path,
    plugin_sandbox: agendao_sandbox::IntegrationSandboxContext,
) {
    let config = (*config_store.config()).clone();

    let phase_start = std::time::Instant::now();
    let loader = match PluginLoader::new().map(|loader| loader.with_sandbox(plugin_sandbox)) {
        Ok(loader) => Arc::new(loader),
        Err(error) => {
            tracing::warn!(%error, "failed to initialize plugin loader");
            return;
        }
    };
    startup_timing!("plugin_loader_new", phase_start);
    init_global(loader.hook_system());
    agendao_plugin::set_global_loader(loader.clone());

    let directory = workspace_root.to_string_lossy().to_string();
    let context = PluginContext {
        worktree: directory.clone(),
        directory,
        server_url: server_url.to_string(),
        internal_token: routes::internal_token().to_string(),
    };

    for name in config.plugin.keys() {
        if !agendao_config::matching::plugin_load_allowed(&config, name) {
            tracing::debug!(
                plugin = %name,
                "plugin disabled via disabled_plugins, skipping load"
            );
        }
    }

    let native_plugin_paths: Vec<(String, PathBuf)> = config
        .plugin
        .iter()
        .filter(|(name, _)| agendao_config::matching::plugin_load_allowed(&config, name))
        .filter_map(|(name, cfg)| {
            if !cfg.is_native() {
                return None;
            }
            let path = cfg.dylib_path()?;
            Some((
                name.clone(),
                resolve_native_plugin_path(workspace_root, path),
            ))
        })
        .collect();

    if !native_plugin_paths.is_empty() {
        let hook_system = loader.hook_system();
        let native_loader = agendao_plugin::global_native_loader();
        let mut native_loader = native_loader.lock().await;
        for (name, path) in native_plugin_paths {
            if let Err(error) = native_loader.load(&path, hook_system.as_ref()).await {
                tracing::warn!(
                    plugin = name,
                    path = %path.display(),
                    %error,
                    "failed to load native plugin"
                );
            }
        }
    }

    let plugin_specs: Vec<String> = config
        .plugin
        .iter()
        .filter(|(name, _)| agendao_config::matching::plugin_load_allowed(&config, name))
        .filter_map(|(name, cfg)| {
            if cfg.is_native() {
                None
            } else {
                cfg.to_loader_spec(name)
            }
        })
        .collect();
    loader
        .configure_bootstrap(context.clone(), plugin_specs.clone(), true)
        .await;

    if let Err(error) = loader.load_builtins(&context).await {
        tracing::warn!(%error, "failed to load builtin auth plugins");
    }
    startup_timing!("plugin_builtins_spawn", phase_start);
    let phase_start = std::time::Instant::now();

    if !plugin_specs.is_empty() {
        if let Err(error) = loader.load_all(&plugin_specs, &context).await {
            tracing::warn!(%error, "failed to load configured plugins");
            return;
        }
    }
    startup_timing!("plugin_specs_load", phase_start);
    let phase_start = std::time::Instant::now();

    let _any_custom_fetch = refresh_plugin_auth_state(&loader, auth_manager.clone()).await;
    routes::set_plugin_loader(loader.clone());
    routes::refresh_agent_cache(config_store).await;
    spawn_plugin_idle_monitor(loader);
    startup_timing!("plugin_auth_state", phase_start);
}

fn resolve_native_plugin_path(cwd: &std::path::Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn default_workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn normalize_workspace_root(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

static EXTRA_CORS_WHITELIST: Lazy<RwLock<HashSet<String>>> =
    Lazy::new(|| RwLock::new(HashSet::new()));

fn normalize_origin(origin: &str) -> Option<String> {
    let trimmed = origin.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn set_cors_whitelist(origins: Vec<String>) {
    let mut next = HashSet::new();
    for origin in origins {
        if let Some(normalized) = normalize_origin(&origin) {
            next.insert(normalized);
        }
    }

    match EXTRA_CORS_WHITELIST.write() {
        Ok(mut guard) => *guard = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

fn is_extra_allowed_origin(origin: &str) -> bool {
    let normalized = normalize_origin(origin).unwrap_or_else(|| origin.to_string());
    match EXTRA_CORS_WHITELIST.read() {
        Ok(guard) => guard.contains(&normalized),
        Err(poisoned) => poisoned.into_inner().contains(&normalized),
    }
}

fn is_allowed_origin(origin: &str) -> bool {
    origin.starts_with("http://localhost:")
        || origin.starts_with("http://127.0.0.1:")
        || origin == "tauri://localhost"
        || origin == "http://tauri.localhost"
        || origin == "https://tauri.localhost"
        || (origin.starts_with("https://") && origin.ends_with(".opencode.ai"))
        || is_extra_allowed_origin(origin)
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |origin: &HeaderValue, _parts: &Parts| {
                origin.to_str().map(is_allowed_origin).unwrap_or(false)
            },
        ))
        .allow_methods(Any)
        .allow_headers(Any)
}

fn server_password() -> Option<String> {
    std::env::var("AGENDAO_SERVER_PASSWORD")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_public_server_path(method: &Method, path: &str) -> bool {
    *method == Method::OPTIONS
        || path == "/"
        || path == "/health"
        || path == "/favicon.ico"
        || path == "/apple-touch-icon.png"
        || path == "/web"
        || path == "/web/"
        || path.starts_with("/web/")
}

fn bearer_token(value: &HeaderValue) -> Option<&str> {
    value
        .to_str()
        .ok()
        .and_then(|raw| raw.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
}

fn request_server_password(req: &Request<Body>) -> Option<String> {
    if let Some(value) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(bearer_token)
    {
        return Some(value.to_string());
    }

    if let Some(value) = req
        .headers()
        .get("x-agendao-server-password")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(value.to_string());
    }

    req.uri().query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes()).find_map(|(key, value)| {
            matches!(
                key.as_ref(),
                "server_password" | "agendao_server_password" | "api_server_password"
            )
            .then(|| value.into_owned())
        })
    })
}

async fn server_auth_middleware(req: Request<Body>, next: Next) -> Response {
    let Some(expected) = server_password() else {
        return next.run(req).await;
    };

    if is_public_server_path(req.method(), req.uri().path()) {
        return next.run(req).await;
    }

    if request_server_password(&req).as_deref() == Some(expected.as_str()) {
        return next.run(req).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        "AGENDAO_SERVER_PASSWORD is required for this request",
    )
        .into_response()
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_matches(['[', ']']);
    matches!(host, "localhost")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn requires_server_password(bind_host: &str, mdns_enabled: bool) -> bool {
    mdns_enabled || !is_loopback_host(bind_host)
}

fn ensure_server_password_policy(bind_host: &str, mdns_enabled: bool) -> anyhow::Result<()> {
    if requires_server_password(bind_host, mdns_enabled) && server_password().is_none() {
        anyhow::bail!(
            "AGENDAO_SERVER_PASSWORD must be set when binding to a non-loopback host or enabling mDNS"
        );
    }
    Ok(())
}

fn service_name_from_mdns_domain(domain: &str, port: u16) -> String {
    let trimmed = domain
        .trim()
        .trim_end_matches('.')
        .trim_end_matches(".local");
    if trimmed.is_empty() {
        format!("agendao-{}", port)
    } else {
        trimmed.to_string()
    }
}

struct MdnsPublisher {
    child: Child,
}

impl Drop for MdnsPublisher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_mdns_command(command: &str, args: &[String]) -> io::Result<MdnsPublisher> {
    let mut child = ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Ok(Some(status)) = child.try_wait() {
        return Err(io::Error::other(format!(
            "mDNS publisher exited immediately with status {}",
            status
        )));
    }

    Ok(MdnsPublisher { child })
}

fn start_mdns_publisher_if_needed(
    enabled: bool,
    bind_host: &str,
    port: u16,
    mdns_domain: &str,
) -> Option<MdnsPublisher> {
    if !enabled {
        return None;
    }
    if is_loopback_host(bind_host) {
        eprintln!("Warning: mDNS enabled but hostname is loopback; skipping mDNS publish.");
        return None;
    }

    let service_name = service_name_from_mdns_domain(mdns_domain, port);
    let attempts: Vec<(String, Vec<String>)> = if cfg!(target_os = "macos") {
        vec![(
            "dns-sd".to_string(),
            vec![
                "-R".to_string(),
                service_name.clone(),
                "_http._tcp".to_string(),
                "local.".to_string(),
                port.to_string(),
                "path=/".to_string(),
            ],
        )]
    } else if cfg!(target_os = "linux") {
        vec![
            (
                "avahi-publish-service".to_string(),
                vec![
                    service_name.clone(),
                    "_http._tcp".to_string(),
                    port.to_string(),
                    "path=/".to_string(),
                ],
            ),
            (
                "avahi-publish".to_string(),
                vec![
                    "-s".to_string(),
                    service_name.clone(),
                    "_http._tcp".to_string(),
                    port.to_string(),
                    "path=/".to_string(),
                ],
            ),
        ]
    } else {
        eprintln!("Warning: mDNS requested but this platform has no configured publisher command.");
        return None;
    };

    let mut last_error: Option<String> = None;
    for (command, args) in attempts {
        match spawn_mdns_command(&command, &args) {
            Ok(publisher) => {
                eprintln!(
                    "mDNS publish enabled via `{}` as service `{}` on port {}.",
                    command, service_name, port
                );
                return Some(publisher);
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    last_error = Some(format!("{}: {}", command, err));
                }
            }
        }
    }

    if let Some(err) = last_error {
        eprintln!("Warning: failed to start mDNS publisher ({})", err);
    } else {
        eprintln!("Warning: mDNS requested but no supported publisher command was found on PATH.");
    }
    None
}

/// Shut down all native plugins.  Called on server exit alongside
/// the subprocess loader's `shutdown_all()`.
async fn shutdown_native_plugins() {
    let loader = agendao_plugin::global_native_loader();
    let mut native = loader.lock().await;
    if native.count() > 0 {
        tracing::info!(count = native.count(), "shutting down native plugins");
        let hook_system = agendao_plugin::global();
        native.shutdown(hook_system.as_ref()).await;
    }
}

pub async fn run_server_runtime(options: ServerRuntimeOptions) -> anyhow::Result<()> {
    crate::web::configure_web_dist_root(options.web_dist.clone());
    crate::web::configure_embedded_web_assets(options.embedded_web_assets);
    let workspace_root =
        normalize_workspace_root(options.cwd.clone().unwrap_or_else(default_workspace_root));

    let bind_host = if options.mdns && options.hostname == "127.0.0.1" {
        "0.0.0.0".to_string()
    } else {
        options.hostname
    };
    ensure_server_password_policy(&bind_host, options.mdns)?;
    if server_password().is_none() {
        eprintln!("Warning: AGENDAO_SERVER_PASSWORD is not set; loopback server is unsecured.");
    }
    let start_http = options.port != 0 || options.unix_socket_path.is_none();
    let bind_port = if options.port == 0 {
        if options.unix_socket_path.is_some() {
            0 // Unix-socket-only mode: port 0 means HTTP is disabled
        } else {
            3000
        }
    } else {
        options.port
    };
    set_cors_whitelist(options.cors);
    let result = if start_http {
        let _mdns_publisher = start_mdns_publisher_if_needed(
            options.mdns,
            &bind_host,
            bind_port,
            &options.mdns_domain,
        );
        let addr: SocketAddr = format!("{}:{}", bind_host, bind_port).parse()?;
        println!(
            "Starting AgenDao server on {} (workspace: {})",
            addr,
            workspace_root.display()
        );
        run_server_with_unix_socket(addr, workspace_root, options.unix_socket_path).await
    } else {
        println!(
            "Starting AgenDao server on Unix socket {} (workspace: {})",
            options.unix_socket_path.as_deref().unwrap_or("<unknown>"),
            workspace_root.display()
        );
        run_unix_socket_only(workspace_root, options.unix_socket_path.unwrap()).await
    };
    // P3.2: controlled shutdown of native plugins after server stops.
    shutdown_native_plugins().await;
    result
}

pub async fn run_server(addr: SocketAddr, workspace_root: PathBuf) -> anyhow::Result<()> {
    let server_url = if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{}", addr)
    };
    let state = Arc::new(
        ServerState::new_with_storage_for_url_in_workspace(server_url, workspace_root).await?,
    );
    state.ensure_frontend_projector();
    state.ensure_catalog_refresh_loop();

    let app = routes::router()
        .layer(middleware::from_fn(server_auth_middleware))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

pub async fn run_server_with_state(
    addr: SocketAddr,
    state: Arc<ServerState>,
) -> anyhow::Result<()> {
    state.ensure_frontend_projector();
    state.ensure_catalog_refresh_loop();
    let app = routes::router()
        .layer(middleware::from_fn(server_auth_middleware))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Start server in Unix-socket-only mode (HTTP disabled).
#[cfg(unix)]
pub async fn run_unix_socket_only(
    workspace_root: PathBuf,
    socket_path: String,
) -> anyhow::Result<()> {
    let server_url = format!("unix://{}", socket_path);
    let state = Arc::new(
        ServerState::new_with_storage_for_url_in_workspace(server_url, workspace_root.clone())
            .await?,
    );
    state.ensure_frontend_projector();
    state.ensure_catalog_refresh_loop();

    let unix_server =
        crate::unix_socket::UnixSocketServer::new(Arc::clone(&state), socket_path.clone());

    tracing::info!("Unix-socket-only mode: listening on {}", socket_path);
    unix_server.serve().await
}

/// Unix domain sockets are not available on Windows. Keep the public startup
/// surface explicit so callers receive a deterministic platform error.
#[cfg(not(unix))]
pub async fn run_unix_socket_only(
    _workspace_root: PathBuf,
    _socket_path: String,
) -> anyhow::Result<()> {
    anyhow::bail!("Unix-socket-only mode is unsupported on this platform")
}

async fn run_server_with_unix_socket(
    addr: SocketAddr,
    workspace_root: PathBuf,
    unix_socket_path: Option<String>,
) -> anyhow::Result<()> {
    let server_url = if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{}", addr)
    };
    let state = Arc::new(
        ServerState::new_with_storage_for_url_in_workspace(server_url, workspace_root.clone())
            .await?,
    );
    state.ensure_frontend_projector();
    state.ensure_catalog_refresh_loop();
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Start Unix socket server if path is provided.
    #[cfg(unix)]
    let unix_task = if let Some(socket_path) = unix_socket_path {
        let unix_server =
            crate::unix_socket::UnixSocketServer::new(Arc::clone(&state), socket_path.clone());
        let unix_listener = unix_server.bind()?;

        let task = tokio::spawn(async move {
            if let Err(e) = unix_server.serve_bound(unix_listener).await {
                tracing::error!("Unix socket server error: {}", e);
            }
        });

        tracing::info!("Unix socket server listening on {}", socket_path);
        Some(task)
    } else {
        None
    };
    #[cfg(not(unix))]
    let unix_task: Option<tokio::task::JoinHandle<()>> = if unix_socket_path.is_some() {
        anyhow::bail!("Unix socket transport is unsupported on this platform")
    } else {
        None
    };

    // Start HTTP server
    let app = routes::router()
        .layer(middleware::from_fn(server_auth_middleware))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    tracing::info!("HTTP server listening on {}", addr);

    let http_result = axum::serve(listener, app).await;
    if let Some(task) = unix_task {
        task.abort();
        let _ = task.await;
    }
    http_result?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[tokio::test]
    async fn record_only_flush_persists_metadata_without_rewriting_messages() {
        let db = Database::in_memory().await.expect("in-memory database");
        let mut state = ServerState::new();
        state.session_repo = Some(SessionRepository::new(db.pool().clone()));
        state.message_repo = Some(MessageRepository::new(db.pool().clone()));

        let session_id = {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions.create("project", "/tmp");
            let id = session.id.clone();
            session.push_message(agendao_types::SessionMessage::user(id.clone(), "first"));
            sessions.update(session);
            id
        };
        state
            .flush_session_to_storage(&session_id)
            .await
            .expect("initial full flush");

        {
            let mut sessions = state.sessions.lock().await;
            let session = sessions.get_mut(&session_id).expect("session");
            session.insert_metadata("task_ledger", serde_json::json!({ "revision": 2 }));
            session.push_message(agendao_types::SessionMessage::assistant(session_id.clone()));
        }
        state
            .flush_session_record_to_storage(&session_id)
            .await
            .expect("record-only flush");

        let stored = state
            .session_repo
            .as_ref()
            .expect("session repository")
            .get(&session_id)
            .await
            .expect("read session")
            .expect("stored session");
        assert_eq!(stored.metadata["task_ledger"]["revision"], 2);
        let messages = state
            .message_repo
            .as_ref()
            .expect("message repository")
            .list_for_session(&session_id)
            .await
            .expect("read messages");
        assert_eq!(
            messages.len(),
            1,
            "record-only flush must not touch messages"
        );

        state
            .flush_session_to_storage(&session_id)
            .await
            .expect("final full flush");
        let messages = state
            .message_repo
            .as_ref()
            .expect("message repository")
            .list_for_session(&session_id)
            .await
            .expect("read messages");
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn server_startup_rejects_invalid_workspace_config() {
        crate::isolate_test_config_home();
        let temp = tempfile::tempdir().expect("tempdir");
        let config_dir = temp.path().join(".agendao");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        let config_path = config_dir.join("agendao.json");
        std::fs::write(&config_path, "{ invalid json").expect("invalid config");

        let result = ServerState::new_with_storage_for_url_in_workspace(
            "http://127.0.0.1:0".to_string(),
            temp.path().to_path_buf(),
        )
        .await;
        let error = result.err().expect("startup must reject invalid config");
        let message = format!("{error:#}");
        assert!(message.contains("failed to load workspace configuration"));
        assert!(message.contains(&config_path.display().to_string()));
    }

    #[test]
    fn auth_public_path_only_allows_static_and_health_routes() {
        assert!(is_public_server_path(&Method::GET, "/health"));
        assert!(is_public_server_path(&Method::GET, "/web/app.js"));
        assert!(!is_public_server_path(&Method::GET, "/file/content"));
        assert!(!is_public_server_path(&Method::POST, "/provider/connect"));
    }

    #[test]
    fn request_server_password_accepts_header_and_query() {
        let header_req = Request::builder()
            .uri("/provider/connect")
            .header(header::AUTHORIZATION, "Bearer secret")
            .body(Body::empty())
            .expect("request should build");
        assert_eq!(
            request_server_password(&header_req).as_deref(),
            Some("secret")
        );

        let query_req = Request::builder()
            .uri("/event?server_password=query-secret")
            .body(Body::empty())
            .expect("request should build");
        assert_eq!(
            request_server_password(&query_req).as_deref(),
            Some("query-secret")
        );
    }

    #[test]
    fn remote_bind_and_mdns_require_server_password() {
        assert!(!requires_server_password("127.0.0.1", false));
        assert!(!requires_server_password("localhost", false));
        assert!(!requires_server_password("::1", false));
        assert!(requires_server_password("0.0.0.0", false));
        assert!(requires_server_password("192.168.1.10", false));
        assert!(requires_server_password("127.0.0.1", true));
    }

    fn state_with_repos(
        session_repo: SessionRepository,
        message_repo: MessageRepository,
    ) -> ServerState {
        let mut state = ServerState::new();
        state.session_repo = Some(session_repo);
        state.message_repo = Some(message_repo);
        state
    }

    #[tokio::test]
    async fn storage_roundtrip_restores_sessions_and_messages() {
        let db = Database::in_memory()
            .await
            .expect("in-memory db should initialize");
        let pool = db.pool().clone();

        let state = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool.clone()),
        );
        let (session_id, user_created_at, assistant_created_at) = {
            let mut manager = state.sessions.lock().await;
            let session = manager.create("default", ".");
            let session_id = session.id.clone();

            let fixed_user_time = chrono::Utc
                .timestamp_millis_opt(1_700_000_000_000)
                .single()
                .expect("valid user timestamp");
            let fixed_assistant_time = chrono::Utc
                .timestamp_millis_opt(1_700_000_000_123)
                .single()
                .expect("valid assistant timestamp");

            let session = manager
                .get_mut(&session_id)
                .expect("session should be available for mutation");
            let user = session.add_user_message("hello");
            user.created_at = fixed_user_time;
            if let Some(part) = user.parts.first_mut() {
                part.created_at = fixed_user_time;
            }

            let assistant = session.add_assistant_message();
            assistant.created_at = fixed_assistant_time;
            assistant.add_text("world");
            if let Some(part) = assistant.parts.first_mut() {
                part.created_at = fixed_assistant_time;
            }

            (session_id, fixed_user_time, fixed_assistant_time)
        };

        state
            .sync_sessions_to_storage()
            .await
            .expect("session snapshot should sync to storage");

        let reloaded = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool),
        );
        reloaded
            .load_sessions_from_storage()
            .await
            .expect("sessions should reload from storage");

        // 懒加载：restore 只回元数据，消息体 dehydrated。
        {
            let manager = reloaded.sessions.lock().await;
            let session = manager
                .get(&session_id)
                .expect("session should exist after reload");
            assert!(
                session.messages.is_empty(),
                "lazy restore must not hydrate messages eagerly"
            );
        }
        assert!(reloaded
            .dehydrated_sessions
            .read()
            .await
            .contains(&session_id));

        // 水合闸门：访问消息时按需回填，内容与时序与库存一致。
        reloaded
            .ensure_session_messages_hydrated(&session_id)
            .await
            .expect("hydration should succeed");
        assert!(!reloaded
            .dehydrated_sessions
            .read()
            .await
            .contains(&session_id));

        let manager = reloaded.sessions.lock().await;
        let session = manager
            .get(&session_id)
            .expect("session should exist after reload");
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].created_at, user_created_at);
        assert_eq!(session.messages[1].created_at, assistant_created_at);
        assert_eq!(session.messages[0].get_text(), "hello");
        assert_eq!(session.messages[1].get_text(), "world");
    }

    #[tokio::test]
    async fn sync_preserves_messages_of_dehydrated_sessions() {
        // 数据安全回归：dehydrated 会话经全量 sync 不得丢库存消息
        // （flush 的 delete-stale 分支会把空内存镜像当成真相清库）。
        let db = Database::in_memory()
            .await
            .expect("in-memory db should initialize");
        let pool = db.pool().clone();

        let state = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool.clone()),
        );
        let session_id = {
            let mut manager = state.sessions.lock().await;
            let mut session = manager.create("default", ".");
            session.add_user_message("keep me");
            let id = session.id.clone();
            manager.update(session);
            id
        };
        state
            .sync_sessions_to_storage()
            .await
            .expect("initial sync should persist");

        // 重新加载（dehydrated），随后全量 sync——模拟 rename 等只改元数据后的 flush。
        let reloaded = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool.clone()),
        );
        reloaded
            .load_sessions_from_storage()
            .await
            .expect("reload should succeed");
        assert!(reloaded
            .dehydrated_sessions
            .read()
            .await
            .contains(&session_id));
        reloaded
            .sync_sessions_to_storage()
            .await
            .expect("sync with dehydrated session must succeed");

        // 库存消息必须原样保留。
        let message_repo = MessageRepository::new(pool);
        let messages = message_repo
            .list_for_session(&session_id)
            .await
            .expect("messages should be readable");
        assert_eq!(
            messages.len(),
            1,
            "dehydrated flush must not delete messages"
        );
        assert_eq!(messages[0].get_text(), "keep me");

        // 水合后再 sync 则恢复完整 flush 语义（消息仍在）。
        reloaded
            .ensure_session_messages_hydrated(&session_id)
            .await
            .expect("hydration should succeed");
        reloaded
            .sync_sessions_to_storage()
            .await
            .expect("post-hydration sync should succeed");
        let message_repo2 = reloaded.message_repo.as_ref().unwrap();
        let messages = message_repo2
            .list_for_session(&session_id)
            .await
            .expect("messages should be readable");
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn load_sessions_from_storage_does_not_enqueue_manager_events() {
        let db = Database::in_memory()
            .await
            .expect("in-memory db should initialize");
        let pool = db.pool().clone();

        let state = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool.clone()),
        );
        {
            let mut manager = state.sessions.lock().await;
            let mut session = manager.create("default", ".");
            session.add_user_message("x".repeat(16 * 1024));
            manager.update(session);
            let _ = manager.drain_events();
        }

        state
            .sync_sessions_to_storage()
            .await
            .expect("session snapshot should sync to storage");

        let reloaded = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool),
        );
        reloaded
            .load_sessions_from_storage()
            .await
            .expect("sessions should reload from storage");

        let mut manager = reloaded.sessions.lock().await;
        assert!(manager.get("missing").is_none());
        assert!(
            manager.drain_events().is_empty(),
            "cold-start restoration should not enqueue lifecycle events"
        );
    }

    #[tokio::test]
    async fn sync_removes_deleted_sessions_from_storage() {
        let db = Database::in_memory()
            .await
            .expect("in-memory db should initialize");
        let pool = db.pool().clone();
        let session_repo = SessionRepository::new(pool.clone());

        let state = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool),
        );
        let session_id = {
            let mut manager = state.sessions.lock().await;
            manager.create("default", ".").id.clone()
        };

        state
            .sync_sessions_to_storage()
            .await
            .expect("initial snapshot should sync");
        assert_eq!(
            session_repo
                .list(None, 10)
                .await
                .expect("list should succeed")
                .len(),
            1
        );

        {
            let mut manager = state.sessions.lock().await;
            manager.delete(&session_id);
        }

        state
            .sync_sessions_to_storage()
            .await
            .expect("delete sync should succeed");
        assert!(session_repo
            .get(&session_id)
            .await
            .expect("get should succeed")
            .is_none());
    }
}
