use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rocode_command::cli_style::CliStyle;
use rocode_plugin::init_global;
use rocode_plugin::subprocess::{PluginContext, PluginLoader};
use rocode_provider::{
    bootstrap_config_from_raw, create_registry_from_bootstrap_config, AuthInfo,
    ConfigModel as BootstrapConfigModel, ConfigProvider as BootstrapConfigProvider,
    ProviderRegistry,
};

const DEFAULT_PLUGIN_SERVER_URL: &str = "http://127.0.0.1:3000";

pub(crate) async fn setup_providers(
    config: &rocode_config::Config,
) -> anyhow::Result<ProviderRegistry> {
    let cwd = std::env::current_dir()?;
    setup_providers_for_dir(config, &cwd).await
}

pub(crate) async fn setup_providers_for_dir(
    config: &rocode_config::Config,
    cwd: &Path,
) -> anyhow::Result<ProviderRegistry> {
    // Ensure models.dev cache exists on first run so bootstrap can read it.
    // Bootstrap is synchronous and only reads the cache file.
    let models_registry = rocode_provider::ModelsRegistry::default();
    match tokio::time::timeout(Duration::from_secs(10), models_registry.get()).await {
        Ok(data) => {
            tracing::debug!(
                providers = data.len(),
                "models.dev cache ready for CLI bootstrap"
            );
        }
        Err(_) => {
            tracing::warn!(
                "timed out fetching models.dev data; provider catalogue may be incomplete"
            );
        }
    }

    let auth_store = load_plugin_auth_store(config, cwd).await;

    // Convert config providers to bootstrap format
    let bootstrap_providers = convert_config_providers(config);
    let bootstrap_config = bootstrap_config_from_raw(
        bootstrap_providers,
        config.disabled_providers.clone(),
        config.enabled_providers.clone(),
        config.model.clone(),
        config.small_model.clone(),
    );

    Ok(create_registry_from_bootstrap_config(
        &bootstrap_config,
        &auth_store,
    ))
}

/// Convert rocode_config::ProviderConfig map to bootstrap ConfigProvider map.
fn convert_config_providers(
    config: &rocode_config::Config,
) -> std::collections::HashMap<String, BootstrapConfigProvider> {
    convert_config_providers_with_mode(config, ProviderBootstrapMode::Runtime)
}

pub(crate) fn convert_config_providers_for_artifact(
    config: &rocode_config::Config,
) -> std::collections::HashMap<String, BootstrapConfigProvider> {
    convert_config_providers_with_mode(config, ProviderBootstrapMode::Artifact)
}

fn convert_config_providers_with_mode(
    config: &rocode_config::Config,
    mode: ProviderBootstrapMode,
) -> std::collections::HashMap<String, BootstrapConfigProvider> {
    let Some(ref providers) = config.provider else {
        return std::collections::HashMap::new();
    };

    providers
        .iter()
        .map(|(id, p)| (id.clone(), provider_to_bootstrap(p, mode)))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderBootstrapMode {
    Runtime,
    Artifact,
}

fn provider_to_bootstrap(
    provider: &rocode_config::ProviderConfig,
    mode: ProviderBootstrapMode,
) -> BootstrapConfigProvider {
    let mut options = provider.options.clone().unwrap_or_default();
    if mode == ProviderBootstrapMode::Runtime {
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
        api_style: provider.api_style.clone(),
        api_shape: provider.api_shape.clone(),
        transport: provider.transport.clone(),
        usage_shape: provider.usage_shape.clone(),
        quirks: (!provider.quirks.is_empty()).then_some(provider.quirks.clone()),
        env: provider.env.clone(),
        options: (!options.is_empty()).then_some(options),
        models,
        blacklist: (!provider.blacklist.is_empty()).then_some(provider.blacklist.clone()),
        whitelist: (!provider.whitelist.is_empty()).then_some(provider.whitelist.clone()),
    }
}

fn model_to_bootstrap(id: &str, model: &rocode_config::ModelConfig) -> BootstrapConfigModel {
    let mut options = model.options.clone().unwrap_or_default();
    if let Some(api_key) = &model.api_key {
        options
            .entry("apiKey".to_string())
            .or_insert_with(|| serde_json::Value::String(api_key.clone()));
    }

    let variants = model.variants.as_ref().map(|variants| {
        variants
            .iter()
            .map(|(name, variant)| (name.clone(), variant_to_bootstrap(variant)))
            .collect()
    });

    let cost = model
        .cost
        .as_ref()
        .map(|c| rocode_provider::bootstrap::ConfigModelCost {
            input: c.input,
            output: c.output,
            cache_read: c.cache_read,
            cache_write: c.cache_write,
        });

    let limit = model
        .limit
        .as_ref()
        .map(|l| rocode_provider::bootstrap::ConfigModelLimit {
            context: l.context,
            output: l.output,
        });

    let modalities =
        model
            .modalities
            .as_ref()
            .map(|m| rocode_provider::bootstrap::ConfigModalities {
                input: m.input.clone(),
                output: m.output.clone(),
            });

    BootstrapConfigModel {
        id: model.model.clone().or_else(|| Some(id.to_string())),
        name: model.name.clone(),
        family: model.family.clone(),
        status: model.status.clone(),
        temperature: model.temperature,
        reasoning: model.reasoning,
        attachment: model.attachment,
        tool_call: model.tool_call,
        interleaved: model.interleaved.as_ref().map(|value| match value {
            rocode_config::ModelInterleavedConfig::Bool(enabled) => {
                rocode_provider::bootstrap::InterleavedConfig::Bool(*enabled)
            }
            rocode_config::ModelInterleavedConfig::Field { field } => {
                rocode_provider::bootstrap::InterleavedConfig::Field {
                    field: field.clone(),
                }
            }
        }),
        cost,
        limit,
        modalities,
        release_date: model.release_date.clone(),
        headers: model.headers.clone(),
        provider: model
            .provider
            .as_ref()
            .map(|p| rocode_provider::bootstrap::ConfigModelProvider {
                api: p.api.clone(),
                npm: p.npm.clone(),
            })
            .or_else(|| {
                model
                    .base_url
                    .as_ref()
                    .map(|url| rocode_provider::bootstrap::ConfigModelProvider {
                        api: Some(url.clone()),
                        npm: None,
                    })
            }),
        options: (!options.is_empty()).then_some(options),
        variants,
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

async fn load_plugin_auth_store(
    config: &rocode_config::Config,
    cwd: &Path,
) -> HashMap<String, AuthInfo> {
    let loader = match PluginLoader::new() {
        Ok(loader) => Arc::new(loader),
        Err(error) => {
            tracing::warn!(%error, "failed to initialize plugin loader in CLI");
            return HashMap::new();
        }
    };
    init_global(loader.hook_system());
    rocode_plugin::set_global_loader(loader.clone());

    let directory = cwd.to_string_lossy().to_string();
    let server_url = std::env::var("ROCODE_SERVER_URL")
        .or_else(|_| std::env::var("OPENCODE_SERVER_URL"))
        .unwrap_or_else(|_| DEFAULT_PLUGIN_SERVER_URL.into());
    let context = PluginContext {
        worktree: directory.clone(),
        directory,
        server_url,
        internal_token: String::new(),
    };

    let native_plugin_paths: Vec<(String, PathBuf)> = config
        .plugin
        .iter()
        .filter_map(|(name, cfg)| {
            if !cfg.is_native() {
                return None;
            }
            let path = cfg.dylib_path()?;
            Some((name.clone(), resolve_native_plugin_path(cwd, path)))
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
                    "failed to load native plugin in CLI"
                );
            }
        }
    }

    if let Err(error) = loader.load_builtins(&context).await {
        tracing::warn!(%error, "failed to load builtin auth plugins in CLI");
    }

    if !config.plugin.is_empty() {
        let specs: Vec<String> = config
            .plugin
            .iter()
            .filter_map(|(name, cfg)| {
                if cfg.is_native() {
                    return None;
                }
                let spec = cfg.to_loader_spec(name);
                if spec.is_none() {
                    tracing::info!(
                        plugin = name,
                        r#type = cfg.plugin_type.as_str(),
                        "plugin type not yet supported by loader, skipping"
                    );
                }
                spec
            })
            .collect();
        if !specs.is_empty() {
            if let Err(error) = loader.load_all(&specs, &context).await {
                tracing::warn!(%error, "failed to load configured plugins in CLI");
            }
        }
    }

    let mut auth_store = HashMap::new();
    for (provider_id, bridge) in loader.auth_bridges().await {
        match bridge.load().await {
            Ok(result) => {
                if let Some(api_key) = result.api_key {
                    auth_store.insert(
                        provider_id.clone(),
                        AuthInfo::Api {
                            key: api_key.clone(),
                        },
                    );
                    if provider_id == "github-copilot" {
                        auth_store.insert(
                            "github-copilot-enterprise".to_string(),
                            AuthInfo::Api { key: api_key },
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(provider = provider_id, %error, "failed to load plugin auth in CLI");
            }
        }
    }

    auth_store
}

fn resolve_native_plugin_path(cwd: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

pub(crate) fn render_help(style: &CliStyle) -> String {
    let pad = 17; // column width for command names

    let fmt = |cmd: &str, desc: &str| -> String {
        format!(
            "{} {}",
            style.bold(format!("{:<pad$}", cmd).as_str()),
            style.dim(desc)
        )
    };

    // ── Basic commands ──────────────────────────────────────────
    let basic_lines: Vec<String> = vec![
        fmt("exit, quit", "End the session"),
        fmt("help", "Show this help message"),
        fmt("clear", "Clear screen"),
    ];

    // ── Slash commands ──────────────────────────────────────────
    let slash_lines: Vec<String> = vec![
        fmt("/help", "Show this help message"),
        fmt("/abort", "Cancel the current running response"),
        fmt("/recover", "List recovery actions for the last run"),
        fmt("/recover <id>", "Execute a recovery action"),
        fmt("/new", "Start a new session"),
        fmt("/clear", "Clear screen"),
        fmt("/status", "Show session status"),
        fmt("/runtime", "Show current runtime telemetry"),
        fmt("/usage", "Show authority-backed session usage"),
        fmt(
            "/insights",
            "Show persisted runtime explain, multimodal explain, and memory insights",
        ),
        fmt(
            "/validation",
            "Show workspace config validation aggregated from config owners",
        ),
        fmt("/events [filters]", "Show runtime event log"),
        fmt("/memory [search]", "List workspace memory records"),
        fmt(
            "/memory preview [query]",
            "Preview which memory records would be injected this turn",
        ),
        fmt("/memory show <id>", "Show memory detail"),
        fmt("/memory validation <id>", "Show latest validation report"),
        fmt(
            "/memory conflicts <id>",
            "Show recorded duplicate/contradiction conflicts",
        ),
        fmt("/memory rules", "List builtin memory rule packs"),
        fmt(
            "/memory hits [run=<id> record=<id> limit=<n>]",
            "List persisted memory rule hits",
        ),
        fmt("/memory runs", "List memory consolidation runs"),
        fmt(
            "/memory consolidate [candidates] [limit=<n>]",
            "Run authority-backed consolidation and reflection",
        ),
        fmt(
            "/events next|prev|first|clear",
            "Page or reset the current event view",
        ),
        fmt(
            "/events page <n>",
            "Jump to a specific page for the current filter",
        ),
        fmt(
            "/events stage=<id> type=<kind> limit=<n> [page=<n>]",
            "Filter events with structured query terms",
        ),
        fmt("/inspect [stage_id]", "Alias for /events with stage filter"),
        fmt("/models", "List all available models"),
        fmt("/models refresh", "Refresh the shared model catalogue"),
        fmt("/model <id>", "Switch to a specific model"),
        fmt(
            "/model add <provider>/<key> key=value...",
            "Add a provider-scoped model override",
        ),
        fmt(
            "/model edit <provider>/<key> key=value...",
            "Edit a configured model override",
        ),
        fmt(
            "/model delete <provider>/<key>",
            "Delete a configured model override",
        ),
        fmt("/agents", "List available agents"),
        fmt("/agent <name>", "Switch to a specific agent"),
        fmt("/presets", "List available scheduler presets"),
        fmt("/preset <name>", "Switch to a specific scheduler preset"),
        fmt("/providers", "List configured providers"),
        fmt("/sessions", "List local sessions"),
        fmt("/parent", "Return to the parent session"),
        fmt("/tasks", "List agent tasks"),
        fmt(
            "/compact [focus]",
            "Compact conversation history, optionally around a topic",
        ),
        fmt("/copy", "Copy last assistant reply"),
    ];

    // ── Tips ────────────────────────────────────────────────────
    let tip_lines: Vec<String> = vec![
        format!(
            "Use {} to specify a model at startup",
            style.bold("--model")
        ),
        format!(
            "Use {} to specify an agent at startup",
            style.bold("--agent")
        ),
        format!(
            "Any text not starting with {} is sent as a prompt",
            style.bold("/")
        ),
    ];

    // ── Render ──────────────────────────────────────────────────
    let mut out = String::new();
    for (title, lines) in [
        ("Commands", basic_lines.as_slice()),
        ("Slash Commands", slash_lines.as_slice()),
        ("Tips", tip_lines.as_slice()),
    ] {
        out.push_str(&format!(
            "\r\n  {} {}\r\n",
            style.bold_cyan(style.bullet()),
            style.bold(title),
        ));
        for line in lines {
            out.push_str(&format!("    {}\r\n", line));
        }
    }
    out.push_str("\r\n");
    out
}
