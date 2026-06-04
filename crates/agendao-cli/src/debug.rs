#[cfg(feature = "session-db")]
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::Duration;

use crate::api_client::{
    CliApiClient, RepairQuery, RepairQueryResponse, SessionRepairSummaryResponse,
    SkillCatalogQuery, SkillDetailQuery, SkillHubGuardRunRequest, SkillHubIndexRefreshRequest,
    SkillHubManagedDetachRequest, SkillHubManagedRemoveRequest, SkillHubRemoteInstallApplyRequest,
    SkillHubRemoteInstallPlanRequest, SkillHubRemoteUpdateApplyRequest,
    SkillHubRemoteUpdatePlanRequest, SkillHubSyncApplyRequest, SkillHubSyncPlanRequest,
    SkillHubTimelineQuery, SkillSourceKind, SkillSourceRef,
};
use agendao_agent::AgentRegistry;
use agendao_config::loader::load_config;
use agendao_config::{LspConfig, LspServerConfig as ConfigLspServerConfig};
use agendao_grep::{FileSearchOptions, Ripgrep};
use agendao_lsp::{LspClient, LspServerConfig};
use agendao_session::snapshot::Snapshot;
use agendao_tool::{registry::create_default_registry, ToolContext};

use crate::cli::*;
#[cfg(feature = "session-db")]
use crate::cli_local_data;
use crate::server_lifecycle::FrontendRuntimeContext;

fn resolve_document_input_to_path(input: &str) -> anyhow::Result<PathBuf> {
    if input.starts_with("file://") {
        let url = url::Url::parse(input)?;
        return url
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("Invalid file URI: {}", input));
    }
    let path = PathBuf::from(input);
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()?.join(path))
}

fn select_lsp_server(
    config: &agendao_config::Config,
    file_hint: Option<&Path>,
) -> anyhow::Result<(String, ConfigLspServerConfig)> {
    let Some(lsp_config) = &config.lsp else {
        anyhow::bail!("No `lsp` configuration found in agendao.json(c).");
    };

    let servers = match lsp_config {
        LspConfig::Disabled(false) => {
            anyhow::bail!("LSP is disabled by config (`\"lsp\": false`).");
        }
        LspConfig::Disabled(true) => {
            anyhow::bail!("Invalid `lsp: true` config. Use an object mapping LSP servers.");
        }
        LspConfig::Enabled(map) => map,
    };

    let ext = file_hint
        .and_then(|p| p.extension().and_then(|x| x.to_str()))
        .map(|x| format!(".{}", x.to_ascii_lowercase()));

    let mut fallback: Option<(String, ConfigLspServerConfig)> = None;
    for (id, server) in servers {
        if server.disabled.unwrap_or(false) || server.command.is_empty() {
            continue;
        }
        if fallback.is_none() {
            fallback = Some((id.clone(), server.clone()));
        }

        if let Some(ref ext) = ext {
            if !server.extensions.is_empty()
                && !server
                    .extensions
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(ext.as_str()))
            {
                continue;
            }
        }
        return Ok((id.clone(), server.clone()));
    }

    fallback
        .ok_or_else(|| anyhow::anyhow!("No enabled LSP server with an executable command found."))
}

async fn create_lsp_client(file_hint: Option<&Path>) -> anyhow::Result<LspClient> {
    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;
    let (id, server) = select_lsp_server(&config, file_hint)?;
    let command = server.command[0].clone();
    let args = server.command.iter().skip(1).cloned().collect::<Vec<_>>();
    let initialization_options = server
        .initialization
        .map(serde_json::to_value)
        .transpose()?;

    LspClient::start(
        LspServerConfig {
            id,
            command,
            args,
            initialization_options,
        },
        cwd,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

fn infer_language_id(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" => "cpp",
        "json" => "json",
        "md" => "markdown",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "sh" | "bash" | "zsh" => "shellscript",
        _ => "plaintext",
    }
}

fn resolve_debug_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn resolve_context_docs_registry_path_from_config() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;
    let runtime_config = agendao_tool::ToolRuntimeConfig::from_config(&config);
    let configured = runtime_config
        .context_docs_registry_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "context_docs registry path is not configured; set docs.contextDocsRegistryPath in agendao.json or agendao.jsonc"
            )
        })?;
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(cwd.join(path))
    }
}

async fn resolve_server_skill_catalog(
    session_id: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let query = session_id.map(|session_id| SkillCatalogQuery {
        session_id: Some(session_id.to_string()),
        ..Default::default()
    });
    let mut skills = client.list_skills(query.as_ref()).await?;
    skills.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(skills
        .into_iter()
        .map(|skill| serde_json::json!(skill))
        .collect())
}

async fn resolve_server_skill_detail(
    name: &str,
    session_id: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let detail = client
        .get_skill_detail(&SkillDetailQuery {
            name: name.to_string(),
            session_id: session_id.map(ToOwned::to_owned),
            ..Default::default()
        })
        .await?;
    Ok(serde_json::json!(detail))
}

fn debug_skill_source_kind_to_api(kind: SkillSourceKindArg) -> SkillSourceKind {
    match kind {
        SkillSourceKindArg::Bundled => SkillSourceKind::Bundled,
        SkillSourceKindArg::LocalPath => SkillSourceKind::LocalPath,
        SkillSourceKindArg::Git => SkillSourceKind::Git,
        SkillSourceKindArg::Archive => SkillSourceKind::Archive,
        SkillSourceKindArg::Registry => SkillSourceKind::Registry,
    }
}

async fn resolve_server_skill_hub_managed(
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    Ok(serde_json::json!(client.list_skill_hub_managed().await?))
}

async fn resolve_server_skill_hub_index(
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    Ok(serde_json::json!(client.list_skill_hub_index().await?))
}

async fn resolve_server_skill_hub_index_refresh(
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .refresh_skill_hub_index(&SkillHubIndexRefreshRequest {
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_audit(
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    Ok(serde_json::json!(client.list_skill_hub_audit().await?))
}

async fn resolve_server_skill_hub_timeline(
    skill_name: Option<&str>,
    source_id: Option<&str>,
    limit: Option<usize>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .list_skill_hub_timeline(&SkillHubTimelineQuery {
            skill_name: skill_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            source_id: source_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            limit,
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_guard(
    name: Option<&str>,
    source_id: Option<&str>,
    source_kind: Option<SkillSourceKindArg>,
    locator: Option<&str>,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let request = if let Some(name) = name.map(str::trim).filter(|value| !value.is_empty()) {
        SkillHubGuardRunRequest {
            skill_name: Some(name.to_string()),
            source: None,
        }
    } else {
        let source_id = source_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--source-id is required when --name is not set"))?;
        let source_kind = source_kind
            .ok_or_else(|| anyhow::anyhow!("--source-kind is required when --name is not set"))?;
        let locator = locator
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--locator is required when --name is not set"))?;
        SkillHubGuardRunRequest {
            skill_name: None,
            source: Some(SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            }),
        }
    };
    Ok(serde_json::json!(
        client.run_skill_hub_guard(&request).await?
    ))
}

async fn resolve_server_skill_hub_sync_plan(
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .plan_skill_hub_sync(&SkillHubSyncPlanRequest {
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_sync_apply(
    session_id: &str,
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .apply_skill_hub_sync(&SkillHubSyncApplyRequest {
            session_id: session_id.to_string(),
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_distributions(
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    Ok(serde_json::json!(
        client.list_skill_hub_distributions().await?
    ))
}

async fn resolve_server_skill_hub_artifact_cache(
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    Ok(serde_json::json!(
        client.list_skill_hub_artifact_cache().await?
    ))
}

async fn resolve_server_skill_hub_lifecycle(
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    Ok(serde_json::json!(client.list_skill_hub_lifecycle().await?))
}

async fn resolve_server_skill_hub_install_plan(
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    skill_name: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .plan_skill_hub_remote_install(&SkillHubRemoteInstallPlanRequest {
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
            skill_name: skill_name.to_string(),
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_install_apply(
    session_id: &str,
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    skill_name: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .apply_skill_hub_remote_install(&SkillHubRemoteInstallApplyRequest {
            session_id: session_id.to_string(),
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
            skill_name: skill_name.to_string(),
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_update_plan(
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    skill_name: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .plan_skill_hub_remote_update(&SkillHubRemoteUpdatePlanRequest {
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
            skill_name: skill_name.to_string(),
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_update_apply(
    session_id: &str,
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    skill_name: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .apply_skill_hub_remote_update(&SkillHubRemoteUpdateApplyRequest {
            session_id: session_id.to_string(),
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
            skill_name: skill_name.to_string(),
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_detach(
    session_id: &str,
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    skill_name: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .detach_skill_hub_managed(&SkillHubManagedDetachRequest {
            session_id: session_id.to_string(),
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
            skill_name: skill_name.to_string(),
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_skill_hub_remove(
    session_id: &str,
    source_id: &str,
    source_kind: SkillSourceKindArg,
    locator: &str,
    skill_name: &str,
    revision: Option<&str>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<serde_json::Value> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    let response = client
        .remove_skill_hub_managed(&SkillHubManagedRemoveRequest {
            session_id: session_id.to_string(),
            source: SkillSourceRef {
                source_id: source_id.to_string(),
                source_kind: debug_skill_source_kind_to_api(source_kind),
                locator: locator.to_string(),
                revision: revision.map(ToOwned::to_owned),
            },
            skill_name: skill_name.to_string(),
        })
        .await?;
    Ok(serde_json::json!(response))
}

async fn resolve_server_session_repair_summary(
    session_id: &str,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<SessionRepairSummaryResponse> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    client.get_session_repair_summary(session_id).await
}

async fn resolve_server_repair_query(
    query: &RepairQuery,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<RepairQueryResponse> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    let client = CliApiClient::new(base_url);
    if let Some(session_id) = query.session_id.as_deref() {
        client.query_session_repair(session_id, query).await
    } else {
        client.query_global_repair(query).await
    }
}

fn print_repair_summary(summary: &SessionRepairSummaryResponse) {
    println!("session_id: {}", summary.session_id);
    let Some(snapshot) = &summary.snapshot else {
        println!("snapshot: <none>");
        return;
    };

    println!("updated_at: {}", snapshot.updated_at);
    println!("total_events: {}", snapshot.summary.total_events);
    println!("distinct_tools: {}", snapshot.summary.distinct_tools);
    println!(
        "distinct_repair_kinds: {}",
        snapshot.summary.distinct_repair_kinds
    );
    println!(
        "strict_would_fail_count: {}",
        snapshot.summary.strict_would_fail_count
    );
    println!("injected_count: {}", snapshot.summary.injected_count);

    if !snapshot.summary.top_repairs.is_empty() {
        println!("top_repairs:");
        for entry in &snapshot.summary.top_repairs {
            println!("  - {}: {}", entry.key, entry.count);
        }
    }
    if !snapshot.summary.top_tools.is_empty() {
        println!("top_tools:");
        for entry in &snapshot.summary.top_tools {
            println!("  - {}: {}", entry.key, entry.count);
        }
    }
    println!("rows: {}", snapshot.rows.len());
    println!("samples: {}", snapshot.samples.len());
}

fn print_repair_query_response(response: &RepairQueryResponse) {
    if let Some(summary) = &response.summary {
        println!("scope: session");
        println!("total_events: {}", summary.total_events);
        println!("distinct_tools: {}", summary.distinct_tools);
        println!("distinct_repair_kinds: {}", summary.distinct_repair_kinds);
        println!(
            "strict_would_fail_count: {}",
            summary.strict_would_fail_count
        );
        println!("injected_count: {}", summary.injected_count);
    }

    if let Some(summary) = &response.model_summary {
        println!("scope: global");
        println!(
            "provider_id: {}",
            summary.provider_id.as_deref().unwrap_or("<multiple>")
        );
        println!(
            "model_id: {}",
            summary.model_id.as_deref().unwrap_or("<multiple>")
        );
        println!("session_count: {}", summary.session_count);
        println!("total_events: {}", summary.total_events);
        println!(
            "strict_would_fail_count: {}",
            summary.strict_would_fail_count
        );
        if !summary.top_repairs.is_empty() {
            println!("top_repairs:");
            for entry in &summary.top_repairs {
                println!("  - {}: {}", entry.key, entry.count);
            }
        }
        if !summary.top_tools.is_empty() {
            println!("top_tools:");
            for entry in &summary.top_tools {
                println!("  - {}: {}", entry.key, entry.count);
            }
        }
    }

    println!("rows: {}", response.rows.len());
    for row in &response.rows {
        println!(
            "  - tool={} repair={} layer={} count={} strict={} injected={} success={} error={}",
            row.tool_name,
            row.repair_kind.as_str(),
            row.layer,
            row.count,
            row.strict_would_fail_count,
            row.injected_count,
            row.success_count,
            row.error_count
        );
    }

    if !response.samples.is_empty() {
        println!("samples: {}", response.samples.len());
        for sample in &response.samples {
            println!(
                "  - tool={} repair={} layer={} strict={} injected={} outcome={}",
                sample.tool_name,
                sample.repair_kind.as_str(),
                sample.layer,
                sample.strict_mode_would_fail,
                sample.injected_into_model_context,
                sample
                    .outcome
                    .map(|value| format!("{:?}", value).to_ascii_lowercase())
                    .unwrap_or_else(|| "unknown".to_string())
            );
        }
    }

    println!("truncated: {}", response.truncated);
}

pub(crate) async fn handle_debug_command(
    action: DebugCommands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match action {
        DebugCommands::Paths => {
            println!("Global paths:");
            println!("  {:<12} {}", "cwd", std::env::current_dir()?.display());
            println!(
                "  {:<12} {}",
                "home",
                dirs::home_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".to_string())
            );
            println!(
                "  {:<12} {}",
                "config",
                dirs::config_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".to_string())
            );
            println!(
                "  {:<12} {}",
                "data",
                dirs::data_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".to_string())
            );

            println!(
                "  {:<12} {}",
                "cache",
                dirs::cache_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".to_string())
            );
        }
        DebugCommands::Config => {
            let cwd = std::env::current_dir()?;
            let config = load_config(&cwd)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        DebugCommands::Skill => {
            let list = resolve_server_skill_catalog(None, runtime_context).await?;
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        DebugCommands::Skills { action } => match action {
            DebugSkillsCommands::List { session_id } => {
                let list =
                    resolve_server_skill_catalog(session_id.as_deref(), runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&list)?);
            }
            DebugSkillsCommands::View { name, session_id } => {
                let detail =
                    resolve_server_skill_detail(&name, session_id.as_deref(), runtime_context)
                        .await?;
                println!("{}", serde_json::to_string_pretty(&detail)?);
            }
            DebugSkillsCommands::Managed => {
                let value = resolve_server_skill_hub_managed(runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Index => {
                let value = resolve_server_skill_hub_index(runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Distributions => {
                let value = resolve_server_skill_hub_distributions(runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::ArtifactCache => {
                let value = resolve_server_skill_hub_artifact_cache(runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Lifecycle => {
                let value = resolve_server_skill_hub_lifecycle(runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::IndexRefresh {
                source_id,
                source_kind,
                locator,
                revision,
            } => {
                let value = resolve_server_skill_hub_index_refresh(
                    &source_id,
                    source_kind,
                    &locator,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Audit => {
                let value = resolve_server_skill_hub_audit(runtime_context).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Timeline {
                skill_name,
                source_id,
                limit,
            } => {
                let value = resolve_server_skill_hub_timeline(
                    skill_name.as_deref(),
                    source_id.as_deref(),
                    limit,
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Guard {
                name,
                source_id,
                source_kind,
                locator,
                revision,
            } => {
                let value = resolve_server_skill_hub_guard(
                    name.as_deref(),
                    source_id.as_deref(),
                    source_kind,
                    locator.as_deref(),
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::SyncPlan {
                source_id,
                source_kind,
                locator,
                revision,
            } => {
                let value = resolve_server_skill_hub_sync_plan(
                    &source_id,
                    source_kind,
                    &locator,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::SyncApply {
                session_id,
                source_id,
                source_kind,
                locator,
                revision,
            } => {
                let value = resolve_server_skill_hub_sync_apply(
                    &session_id,
                    &source_id,
                    source_kind,
                    &locator,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::InstallPlan {
                source_id,
                source_kind,
                locator,
                skill_name,
                revision,
            } => {
                let value = resolve_server_skill_hub_install_plan(
                    &source_id,
                    source_kind,
                    &locator,
                    &skill_name,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::InstallApply {
                session_id,
                source_id,
                source_kind,
                locator,
                skill_name,
                revision,
            } => {
                let value = resolve_server_skill_hub_install_apply(
                    &session_id,
                    &source_id,
                    source_kind,
                    &locator,
                    &skill_name,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::UpdatePlan {
                source_id,
                source_kind,
                locator,
                skill_name,
                revision,
            } => {
                let value = resolve_server_skill_hub_update_plan(
                    &source_id,
                    source_kind,
                    &locator,
                    &skill_name,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::UpdateApply {
                session_id,
                source_id,
                source_kind,
                locator,
                skill_name,
                revision,
            } => {
                let value = resolve_server_skill_hub_update_apply(
                    &session_id,
                    &source_id,
                    source_kind,
                    &locator,
                    &skill_name,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Detach {
                session_id,
                source_id,
                source_kind,
                locator,
                skill_name,
                revision,
            } => {
                let value = resolve_server_skill_hub_detach(
                    &session_id,
                    &source_id,
                    source_kind,
                    &locator,
                    &skill_name,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DebugSkillsCommands::Remove {
                session_id,
                source_id,
                source_kind,
                locator,
                skill_name,
                revision,
            } => {
                let value = resolve_server_skill_hub_remove(
                    &session_id,
                    &source_id,
                    source_kind,
                    &locator,
                    &skill_name,
                    revision.as_deref(),
                    runtime_context,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        },
        DebugCommands::Scrap => {
            #[cfg(feature = "session-db")]
            {
                let sessions = cli_local_data::list_sessions(None, 10_000).await?;
                let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for session in sessions {
                    grouped
                        .entry(session.project_id)
                        .or_default()
                        .push(session.directory);
                }
                println!("{}", serde_json::to_string_pretty(&grouped)?);
            }
            #[cfg(not(feature = "session-db"))]
            {
                anyhow::bail!("debug scrap requires the `session-db` CLI feature");
            }
        }
        DebugCommands::Wait => loop {
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        },
        DebugCommands::Snapshot { action } => {
            let cwd = std::env::current_dir()?;
            match action {
                DebugSnapshotCommands::Track => {
                    println!("{}", Snapshot::track(&cwd)?);
                }
                DebugSnapshotCommands::Patch { hash } => {
                    let output = ProcessCommand::new("git")
                        .args(["show", "--no-color", &hash])
                        .output()
                        .map_err(|e| anyhow::anyhow!("Failed to run git show: {}", e))?;

                    if output.status.success() {
                        print!("{}", String::from_utf8_lossy(&output.stdout));
                    } else {
                        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr));
                    }
                }
                DebugSnapshotCommands::Diff { hash } => {
                    let diffs = Snapshot::diff(&cwd, &hash)?;
                    println!("{}", serde_json::to_string_pretty(&diffs)?);
                }
            }
        }
        DebugCommands::File { action } => match action {
            DebugFileCommands::Search { query } => {
                let files = Ripgrep::files(".", FileSearchOptions::default())?;
                let matches: Vec<String> = files
                    .into_iter()
                    .filter_map(|p| {
                        let p = p.to_string_lossy().to_string();
                        p.contains(&query).then_some(p)
                    })
                    .collect();
                for line in matches {
                    println!("{}", line);
                }
            }
            DebugFileCommands::Read { path } => {
                let content = fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path, e))?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": path,
                        "content": content
                    }))?
                );
            }
            DebugFileCommands::Status => {
                let output = ProcessCommand::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .map_err(|e| anyhow::anyhow!("Failed to run git status: {}", e))?;
                let status = String::from_utf8_lossy(&output.stdout);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "cwd": std::env::current_dir()?.display().to_string(),
                        "git_status_porcelain": status.lines().collect::<Vec<_>>()
                    }))?
                );
            }

            DebugFileCommands::List { path } => {
                let mut entries = Vec::new();
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    let meta = entry.metadata()?;
                    entries.push(serde_json::json!({
                        "name": entry.file_name().to_string_lossy().to_string(),
                        "path": entry.path().display().to_string(),
                        "is_dir": meta.is_dir(),
                        "is_file": meta.is_file(),
                        "len": meta.len(),
                    }));
                }
                println!("{}", serde_json::to_string_pretty(&entries)?);
            }
            DebugFileCommands::Tree { dir } => {
                let base = dir.unwrap_or_else(|| PathBuf::from("."));
                let tree = Ripgrep::tree(base, Some(200))?;
                println!("{}", tree);
            }
        },
        DebugCommands::Rg { action } => match action {
            DebugRgCommands::Tree { limit } => {
                let tree = Ripgrep::tree(".", limit)?;
                println!("{}", tree);
            }
            DebugRgCommands::Files { query, glob, limit } => {
                let mut options = FileSearchOptions::default();
                if let Some(glob) = glob {
                    options.glob = vec![glob];
                }
                let mut files = Ripgrep::files(".", options)?;
                if let Some(query) = query {
                    files.retain(|p| p.to_string_lossy().contains(&query));
                }
                if let Some(limit) = limit {
                    files.truncate(limit);
                }
                for file in files {
                    println!("{}", file.display());
                }
            }
            DebugRgCommands::Search {
                pattern,
                glob,
                limit,
            } => {
                let mut matches = Ripgrep::search_with_limit(".", &pattern, limit.unwrap_or(200))
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                if !glob.is_empty() {
                    matches.retain(|m| glob.iter().any(|g| m.path.contains(g)));
                }
                println!("{}", serde_json::to_string_pretty(&matches)?);
            }
        },

        DebugCommands::Lsp { action } => match action {
            DebugLspCommands::Diagnostics { file } => {
                let path = resolve_document_input_to_path(&file)?;
                let content = fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
                let client = create_lsp_client(Some(&path)).await?;
                client
                    .open_document(&path, &content, infer_language_id(&path))
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

                let mut rx = client.subscribe();
                let _ = tokio::time::timeout(Duration::from_millis(1200), rx.recv()).await;
                let diagnostics = client.get_diagnostics(&path).await;
                println!("{}", serde_json::to_string_pretty(&diagnostics)?);
            }
            DebugLspCommands::Symbols { query } => {
                let client = create_lsp_client(None).await?;
                let symbols = client
                    .workspace_symbol(&query)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                println!("{}", serde_json::to_string_pretty(&symbols)?);
            }
            DebugLspCommands::DocumentSymbols { uri } => {
                let path = resolve_document_input_to_path(&uri)?;
                let content = fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
                let client = create_lsp_client(Some(&path)).await?;
                client
                    .open_document(&path, &content, infer_language_id(&path))
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let symbols = client
                    .document_symbol(&path)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                println!("{}", serde_json::to_string_pretty(&symbols)?);
            }
        },

        DebugCommands::Docs { action } => match action {
            DebugDocsCommands::Validate { registry, index } => {
                let output = if let Some(index_path) = index {
                    let index_path = resolve_debug_path(index_path)?;
                    serde_json::to_value(agendao_tool::validate_docs_index_file(&index_path)?)?
                } else {
                    let registry_path = if let Some(registry_path) = registry {
                        resolve_debug_path(registry_path)?
                    } else {
                        resolve_context_docs_registry_path_from_config()?
                    };
                    serde_json::to_value(agendao_tool::validate_registry_file(&registry_path)?)?
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        },
        DebugCommands::Repair { action } => match action {
            DebugRepairCommands::Summary { session_id } => {
                let response =
                    resolve_server_session_repair_summary(&session_id, runtime_context).await?;
                print_repair_summary(&response);
            }
            DebugRepairCommands::Query {
                session_id,
                provider_id,
                model_id,
                tool_name,
                repair_kind,
                layer,
                strict_only,
                include_samples,
                limit,
            } => {
                let response = resolve_server_repair_query(
                    &RepairQuery {
                        session_id,
                        provider_id,
                        model_id,
                        tool_name,
                        repair_kind: repair_kind
                            .as_deref()
                            .and_then(crate::api_client::RepairKind::from_legacy_str),
                        layer,
                        strict_only: Some(strict_only),
                        include_samples: Some(include_samples),
                        limit,
                    },
                    runtime_context,
                )
                .await?;
                print_repair_query_response(&response);
            }
        },

        DebugCommands::Agent { name, tool, params } => {
            let cwd = std::env::current_dir()?;
            let config = load_config(&cwd)?;
            let registry = AgentRegistry::from_config(&config);
            let Some(agent) = registry.get(&name) else {
                anyhow::bail!("Agent not found: {}", name);
            };
            if let Some(tool_name) = tool {
                let args = if let Some(raw) = params {
                    serde_json::from_str::<serde_json::Value>(&raw).map_err(|e| {
                        anyhow::anyhow!("Invalid --params JSON for tool `{}`: {}", tool_name, e)
                    })?
                } else {
                    serde_json::json!({})
                };
                let cwd = std::env::current_dir()?;
                let tool_registry = Arc::new(create_default_registry().await);
                let ctx = ToolContext::new(
                    format!("debug-{}", name),
                    "debug-message".to_string(),
                    cwd.display().to_string(),
                )
                .with_agent(name.clone())
                .with_tool_runtime_config(agendao_tool::ToolRuntimeConfig::from_config(&config))
                .with_registry(tool_registry.clone());
                let output = tool_registry
                    .execute(&tool_name, args, ctx)
                    .await
                    .map_err(|e| anyhow::anyhow!("Tool execution failed: {}", e))?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "agent": agent
                    }))?
                );
            }
        }
    }
    Ok(())
}
