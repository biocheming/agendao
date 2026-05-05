use std::collections::BTreeMap;

use chrono::{Local, TimeZone};
use rocode_config::loader::load_config;

use crate::api_client::{
    CliApiClient, ConfigPolicyValidationEffect, ConfigPolicyValidationItem,
    ConfigPolicyValidationOwner, ConfigPolicyValidationScopeKind, ConfigPolicyValidationSeverity,
    ConfigPolicyValidationSnapshot,
};
use crate::cli::{ConfigCommands, ConfigOutputArgs, ConfigOutputFormat};
use crate::server_lifecycle::FrontendRuntimeContext;

pub(crate) async fn handle_config_command(
    action: Option<ConfigCommands>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match action {
        None => show_config(),
        Some(ConfigCommands::Validation { output }) => {
            show_config_validation(runtime_context, &output).await
        }
    }
}

pub(crate) fn show_config() -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;
    let config = load_config(&current_dir)?;

    println!("\n╔══════════════════════════════════════════╗");
    println!("║         Configuration                      ║");
    println!("╚══════════════════════════════════════════╝\n");

    if let Some(ref model) = config.model {
        println!("Default model: {}", model);
    }

    if let Some(ref default_agent) = config.default_agent {
        println!("Default agent: {}", default_agent);
    }

    if !config.instructions.is_empty() {
        println!("\nInstructions:");
        for inst in &config.instructions {
            println!("  - {}", inst);
        }
    }

    println!("\nWorking directory: {}", current_dir.display());

    println!("\nEnvironment variables:");
    let env_vars = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "GOOGLE_API_KEY",
        "MISTRAL_API_KEY",
        "GROQ_API_KEY",
        "XAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "COHERE_API_KEY",
        "TOGETHER_API_KEY",
        "PERPLEXITY_API_KEY",
        "CEREBRAS_API_KEY",
        "GOOGLE_VERTEX_API_KEY",
        "AZURE_OPENAI_API_KEY",
        "AWS_ACCESS_KEY_ID",
    ];

    for var in env_vars {
        let status = if std::env::var(var).is_ok() {
            "✓ set"
        } else {
            "✗ not set"
        };
        println!("  {}: {}", var, status);
    }

    Ok(())
}

async fn show_config_validation(
    runtime_context: &FrontendRuntimeContext,
    output: &ConfigOutputArgs,
) -> anyhow::Result<()> {
    let client = config_client(runtime_context).await?;
    let snapshot = client.get_config_validation().await?;

    if matches!(output.format, ConfigOutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }

    print_config_validation_snapshot(&snapshot);
    Ok(())
}

async fn config_client(runtime_context: &FrontendRuntimeContext) -> anyhow::Result<CliApiClient> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    Ok(CliApiClient::new(base_url))
}

fn print_config_validation_snapshot(snapshot: &ConfigPolicyValidationSnapshot) {
    println!("Config validation");
    println!("  Source: /config/validation");
    for line in config_validation_lines(snapshot) {
        println!("{line}");
    }
}

pub(crate) fn config_validation_lines(snapshot: &ConfigPolicyValidationSnapshot) -> Vec<String> {
    let error_count = snapshot
        .reports
        .iter()
        .filter(|item| item.severity == ConfigPolicyValidationSeverity::Error)
        .count();
    let warning_count = snapshot
        .reports
        .iter()
        .filter(|item| item.severity == ConfigPolicyValidationSeverity::Warning)
        .count();

    let mut lines = vec![
        format!("Revision: {}", snapshot.revision),
        format!("Generated: {}", format_timestamp(snapshot.generated_at_ms)),
        format!(
            "Findings: {} ({} errors, {} warnings)",
            snapshot.reports.len(),
            error_count,
            warning_count
        ),
    ];

    if snapshot.reports.is_empty() {
        lines.push(String::new());
        lines
            .push("No validation findings are present in the current config snapshot.".to_string());
        return lines;
    }

    let mut grouped: BTreeMap<ConfigPolicyValidationOwner, Vec<&ConfigPolicyValidationItem>> =
        BTreeMap::new();
    for item in &snapshot.reports {
        grouped.entry(item.owner).or_default().push(item);
    }

    for (owner, items) in grouped {
        lines.push(String::new());
        lines.push(format!("{} ({})", owner_label(owner), items.len()));
        for item in items {
            lines.push(format!(
                "  - [{}] {} · {}",
                severity_label(item.severity),
                item.code,
                item.path
            ));
            lines.push(format!(
                "    Scope: {}",
                scope_label(item.scope.kind, item.scope.subject_id.as_deref())
            ));
            lines.push(format!("    Effect: {}", effect_label(item.effect)));
            lines.push(format!("    Message: {}", item.message));
            if let Some(fallback) = item.fallback.as_deref() {
                lines.push(format!("    Fallback: {}", fallback));
            }
        }
    }

    lines
}

fn owner_label(owner: ConfigPolicyValidationOwner) -> &'static str {
    match owner {
        ConfigPolicyValidationOwner::Scheduler => "Scheduler",
        ConfigPolicyValidationOwner::SkillTree => "Skill Tree",
        ConfigPolicyValidationOwner::ProviderProfile => "Provider Profile",
        ConfigPolicyValidationOwner::ExternalAdapter => "External Adapter",
    }
}

fn severity_label(severity: ConfigPolicyValidationSeverity) -> &'static str {
    match severity {
        ConfigPolicyValidationSeverity::Warning => "warning",
        ConfigPolicyValidationSeverity::Error => "error",
    }
}

fn effect_label(effect: ConfigPolicyValidationEffect) -> &'static str {
    match effect {
        ConfigPolicyValidationEffect::SoftFallback => "soft fallback",
        ConfigPolicyValidationEffect::FailClosedBootstrap => "fail-closed bootstrap",
        ConfigPolicyValidationEffect::FailClosedRequestGate => "fail-closed request gate",
    }
}

fn scope_label(kind: ConfigPolicyValidationScopeKind, subject_id: Option<&str>) -> String {
    let base = match kind {
        ConfigPolicyValidationScopeKind::SchedulerPath => "scheduler path",
        ConfigPolicyValidationScopeKind::SkillTree => "skill tree",
        ConfigPolicyValidationScopeKind::Provider => "provider",
        ConfigPolicyValidationScopeKind::ExternalAdapter => "external adapter",
    };
    match subject_id {
        Some(id) if !id.is_empty() => format!("{base} · {id}"),
        _ => base.to_string(),
    }
}

fn format_timestamp(ts: i64) -> String {
    match Local.timestamp_millis_opt(ts).single() {
        Some(dt) => dt.to_rfc3339(),
        None => ts.to_string(),
    }
}
