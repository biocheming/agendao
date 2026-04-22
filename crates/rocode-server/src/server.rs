use async_trait::async_trait;
use axum::http::{header::HeaderValue, request::Parts};
use futures::StreamExt;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use rocode_memory::MemoryAuthority;
use rocode_plugin::init_global;
use rocode_plugin::subprocess::{
    PluginAuthBridge, PluginContext, PluginFetchRequest, PluginLoader,
};
use rocode_provider::{
    bootstrap_config_from_raw, create_registry_from_bootstrap_config, register_custom_fetch_proxy,
    unregister_custom_fetch_proxy, AuthInfo, AuthManager, BootstrapConfig,
    ConfigModel as BootstrapConfigModel, ConfigProvider as BootstrapConfigProvider,
    CustomFetchProxy, CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    ModelCatalogAuthority, ProviderError, ProviderRegistry,
};
use rocode_runtime_context::{ResolvedWorkspaceContext, ResolvedWorkspaceContextAuthority};
use rocode_session::{SessionManager, SessionPrompt, SessionStateManager};
use rocode_state::UserStateAuthority;
use rocode_storage::{Database, MemoryRepository, MessageRepository, SessionRepository};

use crate::routes;
use crate::runtime_control::RuntimeControlRegistry;
use crate::session_runtime::memory::RuntimeMemoryAuthority;
use crate::session_runtime::telemetry::RuntimeTelemetryAuthority;

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:3000";

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
    let secs = std::env::var("ROCODE_PLUGIN_IDLE_SECS")
        .ok()
        .or_else(|| std::env::var("OPENCODE_PLUGIN_IDLE_SECS").ok())
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
    pub sessions: Mutex<SessionManager>,
    pub providers: tokio::sync::RwLock<ProviderRegistry>,
    pub catalog_authority: Arc<ModelCatalogAuthority>,
    pub resolved_context: tokio::sync::RwLock<ResolvedWorkspaceContext>,
    pub config_store: Arc<rocode_config::ConfigStore>,
    pub user_state: Arc<UserStateAuthority>,
    pub resolved_context_authority: Arc<ResolvedWorkspaceContextAuthority>,
    pub tool_registry: Arc<rocode_tool::ToolRegistry>,
    pub prompt_runner: Arc<SessionPrompt>,
    pub(crate) runtime_memory: Arc<RuntimeMemoryAuthority>,
    pub(crate) runtime_telemetry: Arc<RuntimeTelemetryAuthority>,
    // Legacy compatibility fields. Phase 1 keeps them so existing write paths
    // can keep compiling while read paths start going through the authority.
    pub(crate) runtime_control: Arc<RuntimeControlRegistry>,
    pub auth_manager: Arc<AuthManager>,
    pub event_bus: broadcast::Sender<String>,
    pub api_perf: Arc<ApiPerfCounters>,
    pub(crate) session_repo: Option<SessionRepository>,
    pub(crate) message_repo: Option<MessageRepository>,
    pub category_registry: Arc<rocode_config::CategoryRegistry>,
    pub(crate) todo_manager: rocode_session::TodoManager,
}

pub struct ApiPerfCounters {
    pub list_messages_calls: AtomicU64,
    pub list_messages_incremental_calls: AtomicU64,
    pub list_messages_full_calls: AtomicU64,
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
    pub fn new() -> Self {
        let config_store = Arc::new(rocode_config::ConfigStore::new(
            rocode_config::Config::default(),
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
        let runtime_telemetry = Arc::new(RuntimeTelemetryAuthority::new(tx.clone()));
        let runtime_control = runtime_telemetry.runtime_control();
        Self {
            sessions: Mutex::new(SessionManager::new()),
            providers: tokio::sync::RwLock::new(ProviderRegistry::new()),
            catalog_authority: rocode_provider::default_model_catalog_authority(),
            resolved_context: tokio::sync::RwLock::new(ResolvedWorkspaceContext::empty()),
            config_store,
            user_state,
            resolved_context_authority,
            tool_registry: Arc::new(rocode_tool::ToolRegistry::new()),
            prompt_runner: Arc::new(SessionPrompt::new(Arc::new(tokio::sync::RwLock::new(
                SessionStateManager::new(),
            )))),
            runtime_memory,
            runtime_telemetry,
            runtime_control,
            auth_manager: Arc::new(AuthManager::new()),
            event_bus: tx,
            api_perf: Arc::new(ApiPerfCounters::new()),
            session_repo: None,
            message_repo: None,
            category_registry: Arc::new(rocode_config::CategoryRegistry::empty()),
            todo_manager: rocode_session::TodoManager::new(),
        }
    }

    pub async fn new_with_storage() -> anyhow::Result<Self> {
        Self::new_with_storage_for_url(DEFAULT_SERVER_URL.to_string()).await
    }

    pub async fn new_with_storage_for_url(server_url: String) -> anyhow::Result<Self> {
        let mut state = Self::new();
        let auth_manager = Arc::new(AuthManager::load_from_file(&auth_data_dir()).await);
        state.auth_manager = auth_manager.clone();

        // Load config and convert providers to bootstrap format
        let cwd = std::env::current_dir().unwrap_or_default();
        let config_store = match rocode_config::ConfigStore::from_project_dir(&cwd) {
            Ok(store) => Arc::new(store),
            Err(error) => {
                tracing::warn!(%error, "failed to load config, using defaults");
                Arc::new(rocode_config::ConfigStore::new(
                    rocode_config::Config::default(),
                ))
            }
        };
        let user_state = Arc::new(UserStateAuthority::from_config_store(&config_store));
        let resolved_context_authority = Arc::new(ResolvedWorkspaceContextAuthority::new(
            config_store.clone(),
            user_state.clone(),
        ));

        // Plugin bootstrap needs config_store for refresh_agent_cache
        load_plugin_auth_store(&server_url, auth_manager.clone(), &config_store).await;
        let auth_store = auth_manager.list().await;
        let bootstrap_config = {
            let config = config_store.config();
            bootstrap_config_from_config(&config)
        };

        // Ensure models.dev cache exists before bootstrap (which reads it synchronously).
        {
            match tokio::time::timeout(Duration::from_secs(10), state.catalog_authority.data())
                .await
            {
                Ok(data) => {
                    tracing::info!(providers = data.len(), "models.dev cache ready");
                }
                Err(_) => {
                    tracing::warn!(
                        "timed out fetching models.dev data; built-in model list may be incomplete"
                    );
                }
            }
        }

        state.providers = tokio::sync::RwLock::new(create_registry_from_bootstrap_config(
            &bootstrap_config,
            &auth_store,
        ));
        state.config_store = config_store.clone();
        state.user_state = user_state;
        state.resolved_context_authority = resolved_context_authority.clone();
        state.runtime_memory =
            Arc::new(RuntimeMemoryAuthority::new(Arc::new(MemoryAuthority::new(
                state.user_state.clone(),
                state.resolved_context_authority.clone(),
            ))));
        let _ = state.refresh_resolved_context().await;

        // Load task category registry from configured path
        let category_registry = if let Some(path) = config_store.resolved_task_category_path().await
        {
            match rocode_config::CategoryRegistry::load(&path) {
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
                    rocode_config::CategoryRegistry::with_builtins()
                }
            }
        } else {
            rocode_config::CategoryRegistry::with_builtins()
        };
        state.category_registry = Arc::new(category_registry);
        let tool_runtime_config =
            rocode_tool::ToolRuntimeConfig::from_config(&config_store.config());
        state.tool_registry = Arc::new(
            rocode_tool::create_default_registry_with_config(Some(&config_store.config())).await,
        );
        state.prompt_runner = Arc::new(
            SessionPrompt::new(Arc::new(tokio::sync::RwLock::new(
                SessionStateManager::new(),
            )))
            .with_config_store(config_store.clone())
            .with_tool_runtime_config(tool_runtime_config),
        );
        let db = Database::new().await?;
        let pool = db.pool().clone();
        let memory_repo = Arc::new(MemoryRepository::new(pool.clone()));
        state.session_repo = Some(SessionRepository::new(pool.clone()));
        state.message_repo = Some(MessageRepository::new(pool));
        state.runtime_memory = Arc::new(RuntimeMemoryAuthority::new(Arc::new(
            MemoryAuthority::new(
                state.user_state.clone(),
                state.resolved_context_authority.clone(),
            )
            .with_repository(memory_repo),
        )));
        state.load_sessions_from_storage().await?;
        Ok(state)
    }

    pub fn broadcast(&self, event: &str) {
        let _ = self.event_bus.send(event.to_string());
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

    pub async fn refresh_resolved_context(&self) -> anyhow::Result<ResolvedWorkspaceContext> {
        let resolved = self.resolved_context_authority.resolve().await?;
        *self.resolved_context.write().await = resolved.clone();
        Ok(resolved)
    }

    async fn load_sessions_from_storage(&self) -> anyhow::Result<()> {
        let (Some(session_repo), Some(message_repo)) = (&self.session_repo, &self.message_repo)
        else {
            return Ok(());
        };

        let stored_sessions = session_repo.list(None, 100_000).await?;
        let mut manager = self.sessions.lock().await;

        for mut stored in stored_sessions {
            let stored_messages = message_repo.list_for_session(&stored.id).await?;
            stored.messages = stored_messages;
            let session: rocode_session::Session =
                serde_json::from_value(serde_json::to_value(stored)?)?;
            manager.update(session);
        }

        Ok(())
    }

    /// Flush a single session (and its messages) to storage inside a transaction.
    /// Used after prompt ends — avoids scanning all sessions.
    pub async fn flush_session_to_storage(&self, session_id: &str) -> anyhow::Result<()> {
        let Some(session_repo) = &self.session_repo else {
            return Ok(());
        };

        let session = {
            let manager = self.sessions.lock().await;
            manager.get(session_id).cloned()
        };

        let Some(session) = session else {
            return Ok(());
        };

        let stored: rocode_types::Session =
            serde_json::from_value(serde_json::to_value(&session)?)?;
        let mut persisted = stored.clone();
        let messages = std::mem::take(&mut persisted.messages);

        session_repo
            .flush_with_messages(&persisted, &messages)
            .await?;
        self.runtime_memory.ingest_session_record(&stored).await?;
        let stage_summaries = self
            .runtime_telemetry
            .list_stage_summaries(session_id)
            .await;
        self.runtime_memory
            .ingest_stage_summaries(session_id, &stage_summaries)
            .await?;

        Ok(())
    }

    pub async fn sync_sessions_to_storage(&self) -> anyhow::Result<()> {
        let (Some(session_repo), Some(message_repo)) = (&self.session_repo, &self.message_repo)
        else {
            return Ok(());
        };

        let snapshot: Vec<rocode_session::Session> = {
            let manager = self.sessions.lock().await;
            manager.list().into_iter().cloned().collect()
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
            let stored_session: rocode_types::Session =
                serde_json::from_value(serde_json::to_value(&session)?)?;
            let mut persisted_session = stored_session.clone();
            let stored_messages = std::mem::take(&mut persisted_session.messages);

            session_repo
                .flush_with_messages(&persisted_session, &stored_messages)
                .await?;
            self.runtime_memory
                .ingest_session_record(&stored_session)
                .await?;
            let stage_summaries = self
                .runtime_telemetry
                .list_stage_summaries(&stored_session.id)
                .await;
            self.runtime_memory
                .ingest_stage_summaries(&stored_session.id, &stage_summaries)
                .await?;
        }

        Ok(())
    }
}

/// Convert rocode_config::ProviderConfig map to bootstrap ConfigProvider map.
fn convert_config_providers_for_bootstrap(
    config: &rocode_config::Config,
) -> std::collections::HashMap<String, BootstrapConfigProvider> {
    let Some(ref providers) = config.provider else {
        return std::collections::HashMap::new();
    };

    providers
        .iter()
        .map(|(id, provider)| (id.clone(), provider_to_bootstrap(provider)))
        .collect()
}

pub(crate) fn bootstrap_config_from_config(config: &rocode_config::Config) -> BootstrapConfig {
    let providers = convert_config_providers_for_bootstrap(config);
    bootstrap_config_from_raw(
        providers,
        config.disabled_providers.clone(),
        config.enabled_providers.clone(),
        config.model.clone(),
        config.small_model.clone(),
    )
}

fn provider_to_bootstrap(provider: &rocode_config::ProviderConfig) -> BootstrapConfigProvider {
    let mut options = provider.options.clone().unwrap_or_default();
    if let Some(api_key) = &provider.api_key {
        options
            .entry("apiKey".to_string())
            .or_insert_with(|| serde_json::Value::String(api_key.clone()));
    }
    if let Some(base_url) = &provider.base_url {
        options
            .entry("baseURL".to_string())
            .or_insert_with(|| serde_json::Value::String(base_url.clone()));
    }

    let models = provider.models.as_ref().map(|models| {
        models
            .iter()
            .map(|(id, model)| (id.clone(), model_to_bootstrap(id, model)))
            .collect()
    });

    BootstrapConfigProvider {
        name: provider.name.clone(),
        api: provider.base_url.clone(),
        npm: provider.npm.clone(),
        options: (!options.is_empty()).then_some(options),
        models,
        blacklist: (!provider.blacklist.is_empty()).then_some(provider.blacklist.clone()),
        whitelist: (!provider.whitelist.is_empty()).then_some(provider.whitelist.clone()),
        ..Default::default()
    }
}

fn model_to_bootstrap(id: &str, model: &rocode_config::ModelConfig) -> BootstrapConfigModel {
    let mut options = HashMap::new();
    if let Some(api_key) = &model.api_key {
        options.insert(
            "apiKey".to_string(),
            serde_json::Value::String(api_key.clone()),
        );
    }

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
            rocode_provider::bootstrap::ConfigModelProvider {
                api: Some(url.clone()),
                npm: None,
            }
        }),
        options: (!options.is_empty()).then_some(options),
        variants,
        ..Default::default()
    }
}

fn variant_to_bootstrap(
    variant: &rocode_config::ModelVariantConfig,
) -> HashMap<String, serde_json::Value> {
    let mut values = variant.extra.clone();
    if let Some(disabled) = variant.disabled {
        values.insert("disabled".to_string(), serde_json::Value::Bool(disabled));
    }
    values
}

fn auth_data_dir() -> PathBuf {
    if let Ok(path) =
        std::env::var("ROCODE_DATA_DIR").or_else(|_| std::env::var("OPENCODE_DATA_DIR"))
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("rocode")
        .join("data")
}

async fn load_plugin_auth_store(
    server_url: &str,
    auth_manager: Arc<AuthManager>,
    config_store: &rocode_config::ConfigStore,
) {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            tracing::warn!(%error, "failed to get current directory for plugin bootstrap");
            return;
        }
    };

    let config = (*config_store.config()).clone();

    let loader = match PluginLoader::new() {
        Ok(loader) => Arc::new(loader),
        Err(error) => {
            tracing::warn!(%error, "failed to initialize plugin loader");
            return;
        }
    };
    init_global(loader.hook_system());
    rocode_plugin::set_global_loader(loader.clone());

    let directory = cwd.to_string_lossy().to_string();
    let context = PluginContext {
        worktree: directory.clone(),
        directory,
        server_url: server_url.to_string(),
        internal_token: routes::internal_token().to_string(),
    };

    let native_plugin_paths: Vec<(String, PathBuf)> = config
        .plugin
        .iter()
        .filter_map(|(name, cfg)| {
            if !cfg.is_native() {
                return None;
            }
            let path = cfg.dylib_path()?;
            Some((name.clone(), resolve_native_plugin_path(&cwd, path)))
        })
        .collect();

    if !native_plugin_paths.is_empty() {
        let hook_system = loader.hook_system();
        let native_loader = rocode_plugin::global_native_loader();
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

    if !plugin_specs.is_empty() {
        if let Err(error) = loader.load_all(&plugin_specs, &context).await {
            tracing::warn!(%error, "failed to load configured plugins");
            return;
        }
    }

    let _any_custom_fetch = refresh_plugin_auth_state(&loader, auth_manager.clone()).await;
    routes::set_plugin_loader(loader.clone());
    routes::refresh_agent_cache(config_store).await;
    spawn_plugin_idle_monitor(loader);
}

fn resolve_native_plugin_path(cwd: &std::path::Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
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

pub async fn run_server(addr: SocketAddr) -> anyhow::Result<()> {
    let server_url = if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{}", addr)
    };
    let state = Arc::new(ServerState::new_with_storage_for_url(server_url).await?);

    let app = routes::router()
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
    let app = routes::router()
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
