use std::collections::BTreeMap;

use chrono::{Local, TimeZone};
use rocode_types::{
    ManagedSkillRecord, SkillArtifactCacheEntry, SkillDistributionRecord, SkillHubIndexResponse,
    SkillHubPolicy, SkillHubRemoteInstallApplyRequest, SkillHubRemoteInstallPlanRequest,
    SkillHubRemoteUpdateApplyRequest, SkillHubRemoteUpdatePlanRequest, SkillHubSyncApplyRequest,
    SkillHubSyncPlanRequest, SkillManagedLifecycleRecord, SkillRemoteInstallPlan,
    SkillRemoteInstallResponse, SkillSourceKind, SkillSourceRef, SkillSyncAction, SkillSyncPlan,
};
use serde::Serialize;

use crate::api_client::{
    CliApiClient, SkillHubIndexRefreshRequest, SkillHubManagedDetachRequest,
    SkillHubManagedRemoveRequest,
};
use crate::cli::{
    SkillCommands, SkillHubCommands, SkillHubOutputFormat, SkillSourceArgs, SkillSourceKindArg,
};
use crate::server_lifecycle::FrontendRuntimeContext;
use crate::util::truncate_text;

pub(crate) async fn handle_skill_command(
    action: SkillCommands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match action {
        SkillCommands::Hub { action } => handle_skill_hub_command(action, runtime_context).await,
    }
}

async fn handle_skill_hub_command(
    action: SkillHubCommands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    let client = hub_client(runtime_context).await?;
    match action {
        SkillHubCommands::Status { output } => {
            let managed = client.list_skill_hub_managed().await?;
            let index = client.list_skill_hub_index().await?;
            let distributions = client.list_skill_hub_distributions().await?;
            let artifact_cache = client.list_skill_hub_artifact_cache().await?;
            let policy = client.list_skill_hub_policy().await?;
            let lifecycle = client.list_skill_hub_lifecycle().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&serde_json::json!({
                    "managed": managed,
                    "index": index,
                    "distributions": distributions,
                    "artifact_cache": artifact_cache,
                    "policy": policy,
                    "lifecycle": lifecycle,
                }))?;
            } else {
                print_hub_status(
                    managed.managed_skills,
                    index.source_indices,
                    distributions.distributions,
                    artifact_cache.artifact_cache,
                    policy.policy,
                    lifecycle.lifecycle,
                );
            }
        }
        SkillHubCommands::Managed { output } => {
            let response = client.list_skill_hub_managed().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_managed_skills(response.managed_skills);
            }
        }
        SkillHubCommands::Index { output } => {
            let response = client.list_skill_hub_index().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_source_index(response);
            }
        }
        SkillHubCommands::Distributions { output } => {
            let distributions = client.list_skill_hub_distributions().await?;
            let artifact_cache = client.list_skill_hub_artifact_cache().await?;
            let lifecycle = client.list_skill_hub_lifecycle().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&serde_json::json!({
                    "distributions": distributions,
                    "artifact_cache": artifact_cache,
                    "lifecycle": lifecycle,
                }))?;
            } else {
                print_distributions(
                    distributions.distributions,
                    artifact_cache.artifact_cache,
                    lifecycle.lifecycle,
                );
            }
        }
        SkillHubCommands::ArtifactCache { output } => {
            let response = client.list_skill_hub_artifact_cache().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_artifact_cache(response.artifact_cache);
            }
        }
        SkillHubCommands::Policy { output } => {
            let response = client.list_skill_hub_policy().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_policy(&response.policy);
            }
        }
        SkillHubCommands::Lifecycle { output } => {
            let response = client.list_skill_hub_lifecycle().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_lifecycle(response.lifecycle);
            }
        }
        SkillHubCommands::IndexRefresh { source, output } => {
            let response = client
                .refresh_skill_hub_index(&SkillHubIndexRefreshRequest {
                    source: source_ref(&source),
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                println!(
                    "Refreshed source index for {} ({} entries).",
                    response.snapshot.source.source_id,
                    response.snapshot.entries.len()
                );
                println!(
                    "  Updated: {}",
                    format_timestamp(response.snapshot.updated_at)
                );
                println!("  Source: {}", source_label(&response.snapshot.source));
            }
        }
        SkillHubCommands::SyncPlan { source, output } => {
            let response = client
                .plan_skill_hub_sync(&SkillHubSyncPlanRequest {
                    source: source_ref(&source),
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_sync_plan(
                    "Built hub sync plan",
                    &response.plan,
                    response.guard_reports.len(),
                );
            }
        }
        SkillHubCommands::SyncApply {
            session_id,
            source,
            output,
        } => {
            let response = client
                .apply_skill_hub_sync(&SkillHubSyncApplyRequest {
                    session_id,
                    source: source_ref(&source),
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_sync_plan(
                    "Applied hub sync",
                    &response.plan,
                    response.guard_reports.len(),
                );
            }
        }
        SkillHubCommands::InstallPlan {
            source,
            skill_name,
            output,
        } => {
            let response = client
                .plan_skill_hub_remote_install(&SkillHubRemoteInstallPlanRequest {
                    source: source_ref(&source),
                    skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_remote_plan("Built remote install plan", &response);
            }
        }
        SkillHubCommands::InstallApply {
            session_id,
            source,
            skill_name,
            output,
        } => {
            let response = client
                .apply_skill_hub_remote_install(&SkillHubRemoteInstallApplyRequest {
                    session_id,
                    source: source_ref(&source),
                    skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_remote_apply("Applied remote install", &response);
            }
        }
        SkillHubCommands::UpdatePlan {
            source,
            skill_name,
            output,
        } => {
            let response = client
                .plan_skill_hub_remote_update(&SkillHubRemoteUpdatePlanRequest {
                    source: source_ref(&source),
                    skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_remote_plan("Built remote update plan", &response);
            }
        }
        SkillHubCommands::UpdateApply {
            session_id,
            source,
            skill_name,
            output,
        } => {
            let response = client
                .apply_skill_hub_remote_update(&SkillHubRemoteUpdateApplyRequest {
                    session_id,
                    source: source_ref(&source),
                    skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_remote_apply("Applied remote update", &response);
            }
        }
        SkillHubCommands::Detach {
            session_id,
            source,
            skill_name,
            output,
        } => {
            let response = client
                .detach_skill_hub_managed(&SkillHubManagedDetachRequest {
                    session_id,
                    source: source_ref(&source),
                    skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                println!("Detached managed skill {}.", response.lifecycle.skill_name);
                print_lifecycle_line(&response.lifecycle);
            }
        }
        SkillHubCommands::Remove {
            session_id,
            source,
            skill_name,
            output,
        } => {
            let response = client
                .remove_skill_hub_managed(&SkillHubManagedRemoveRequest {
                    session_id,
                    source: source_ref(&source),
                    skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                println!("Removed managed skill {}.", response.lifecycle.skill_name);
                print_lifecycle_line(&response.lifecycle);
                println!(
                    "  Workspace copy deleted: {}",
                    if response.deleted_from_workspace {
                        "yes"
                    } else {
                        "no"
                    }
                );
                if let Some(result) = response.result.as_ref() {
                    println!("  Write result: {} -> {}", result.action, result.location);
                }
            }
        }
    }
    Ok(())
}

async fn hub_client(runtime_context: &FrontendRuntimeContext) -> anyhow::Result<CliApiClient> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    Ok(CliApiClient::new(base_url))
}

fn source_ref(source: &SkillSourceArgs) -> SkillSourceRef {
    SkillSourceRef {
        source_id: source.source_id.clone(),
        source_kind: match source.source_kind {
            SkillSourceKindArg::Bundled => SkillSourceKind::Bundled,
            SkillSourceKindArg::LocalPath => SkillSourceKind::LocalPath,
            SkillSourceKindArg::Git => SkillSourceKind::Git,
            SkillSourceKindArg::Archive => SkillSourceKind::Archive,
            SkillSourceKindArg::Registry => SkillSourceKind::Registry,
        },
        locator: source.locator.clone(),
        revision: source.revision.clone(),
    }
}

fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_hub_status(
    managed: Vec<ManagedSkillRecord>,
    index: Vec<rocode_types::SkillSourceIndexSnapshot>,
    distributions: Vec<SkillDistributionRecord>,
    artifact_cache: Vec<SkillArtifactCacheEntry>,
    policy: SkillHubPolicy,
    lifecycle: Vec<SkillManagedLifecycleRecord>,
) {
    let index_entry_count = index
        .iter()
        .map(|snapshot| snapshot.entries.len())
        .sum::<usize>();
    let artifact_failures = artifact_cache
        .iter()
        .filter(|entry| entry.error.as_deref().is_some())
        .count();
    let lifecycle_failures = lifecycle
        .iter()
        .filter(|record| record.error.as_deref().is_some())
        .count();

    println!("Skill hub status");
    println!("  Managed: {}", managed.len());
    println!(
        "  Indexed sources: {} ({} indexed skills)",
        index.len(),
        index_entry_count
    );
    println!("  Distributions: {}", distributions.len());
    println!(
        "  Artifact cache: {} ({} failures)",
        artifact_cache.len(),
        artifact_failures
    );
    println!(
        "  Policy: retention {} · timeout {} · download {} · extract {}",
        format_duration_seconds(policy.artifact_cache_retention_seconds),
        format_duration_ms(policy.fetch_timeout_ms),
        format_bytes(policy.max_download_bytes),
        format_bytes(policy.max_extract_bytes),
    );
    println!(
        "  Lifecycle records: {} ({} failures)",
        lifecycle.len(),
        lifecycle_failures
    );

    if !index.is_empty() {
        println!("\nSources:");
        let mut index = index;
        index.sort_by(|left, right| left.source.source_id.cmp(&right.source.source_id));
        for snapshot in index {
            println!(
                "  - {} · {} entries · updated {}",
                source_label(&snapshot.source),
                snapshot.entries.len(),
                format_timestamp(snapshot.updated_at)
            );
        }
    }

    if !managed.is_empty() {
        println!("\nManaged skills:");
        let mut managed = managed;
        managed.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        for record in managed.into_iter().take(12) {
            let mut flags = Vec::new();
            if record.locally_modified {
                flags.push("locally-modified");
            }
            if record.deleted_locally {
                flags.push("deleted-locally");
            }
            println!(
                "  - {}{}",
                record.skill_name,
                source_suffix(record.source.as_ref(), &flags)
            );
        }
    }

    let mut lifecycle_errors = lifecycle
        .iter()
        .filter_map(|record| {
            record
                .error
                .as_deref()
                .map(|error| (record.source_id.as_str(), record.skill_name.as_str(), error))
        })
        .collect::<Vec<_>>();
    lifecycle_errors.sort();
    if !lifecycle_errors.is_empty() {
        println!("\nLifecycle failure reasons:");
        for (source_id, skill_name, error) in lifecycle_errors.into_iter().take(12) {
            println!("  - {}/{}: {}", source_id, skill_name, error);
        }
    }

    let mut artifact_errors = artifact_cache
        .iter()
        .filter_map(|entry| {
            entry
                .error
                .as_deref()
                .map(|error| (entry.artifact.artifact_id.as_str(), error))
        })
        .collect::<Vec<_>>();
    artifact_errors.sort();
    if !artifact_errors.is_empty() {
        println!("\nArtifact fetch failures:");
        for (artifact_id, error) in artifact_errors.into_iter().take(12) {
            println!("  - {}: {}", artifact_id, error);
        }
    }
}

fn print_policy(policy: &SkillHubPolicy) {
    println!("Skill hub policy");
    println!(
        "  Artifact cache retention: {}",
        format_duration_seconds(policy.artifact_cache_retention_seconds)
    );
    println!(
        "  Fetch timeout: {}",
        format_duration_ms(policy.fetch_timeout_ms)
    );
    println!(
        "  Max download size: {}",
        format_bytes(policy.max_download_bytes)
    );
    println!(
        "  Max extract size: {}",
        format_bytes(policy.max_extract_bytes)
    );
}

fn print_managed_skills(mut records: Vec<ManagedSkillRecord>) {
    records.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
    println!("Managed skills: {}", records.len());
    for record in records {
        let mut flags = Vec::new();
        if record.locally_modified {
            flags.push("locally-modified");
        }
        if record.deleted_locally {
            flags.push("deleted-locally");
        }
        println!(
            "  - {}{}",
            record.skill_name,
            source_suffix(record.source.as_ref(), &flags)
        );
        if let Some(revision) = record.installed_revision.as_deref() {
            println!("    Installed revision: {}", revision);
        }
        if let Some(hash) = record.local_hash.as_deref() {
            println!("    Local hash: {}", hash);
        }
        if let Some(last_synced_at) = record.last_synced_at {
            println!("    Last synced: {}", format_timestamp(last_synced_at));
        }
    }
}

fn print_source_index(mut response: SkillHubIndexResponse) {
    response
        .source_indices
        .sort_by(|left, right| left.source.source_id.cmp(&right.source.source_id));
    println!("Indexed sources: {}", response.source_indices.len());
    for snapshot in response.source_indices {
        let preview = snapshot
            .entries
            .iter()
            .take(8)
            .map(|entry| entry.skill_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  - {} · {} entries · updated {}",
            source_label(&snapshot.source),
            snapshot.entries.len(),
            format_timestamp(snapshot.updated_at)
        );
        if !preview.is_empty() {
            println!("    Skills: {}", preview);
        }
    }
}

fn print_distributions(
    mut distributions: Vec<SkillDistributionRecord>,
    artifact_cache: Vec<SkillArtifactCacheEntry>,
    lifecycle: Vec<SkillManagedLifecycleRecord>,
) {
    let artifact_by_id = artifact_cache
        .into_iter()
        .map(|entry| (entry.artifact.artifact_id.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let lifecycle_by_distribution = lifecycle
        .into_iter()
        .map(|record| (record.distribution_id.clone(), record))
        .collect::<BTreeMap<_, _>>();

    distributions.sort_by(|left, right| {
        left.source
            .source_id
            .cmp(&right.source.source_id)
            .then_with(|| left.skill_name.cmp(&right.skill_name))
    });
    println!("Distributions: {}", distributions.len());
    for record in distributions {
        let lifecycle_record = lifecycle_by_distribution.get(&record.distribution_id);
        let artifact_record = artifact_by_id.get(&record.resolution.artifact.artifact_id);
        println!(
            "  - {}/{} · {:?}",
            record.source.source_id, record.skill_name, record.lifecycle
        );
        println!(
            "    Release: version {} · revision {}",
            optional_text(record.release.version.as_deref()),
            optional_text(record.release.revision.as_deref())
        );
        println!(
            "    Resolution: {:?} · artifact {} ({:?}) · resolved {}",
            record.resolution.resolver_kind,
            record.resolution.artifact.artifact_id,
            record.resolution.artifact.kind,
            format_timestamp(record.resolution.resolved_at)
        );
        if let Some(installed) = record.installed.as_ref() {
            println!(
                "    Installed: {} · {}",
                format_timestamp(installed.installed_at),
                installed.workspace_skill_path
            );
        }
        if let Some(reason) = lifecycle_record
            .and_then(|record| record.error.as_deref())
            .or_else(|| artifact_record.and_then(|record| record.error.as_deref()))
        {
            println!("    Failure reason: {}", reason);
        }
    }
}

fn print_artifact_cache(mut entries: Vec<SkillArtifactCacheEntry>) {
    entries.sort_by(|left, right| left.artifact.artifact_id.cmp(&right.artifact.artifact_id));
    println!("Artifact cache entries: {}", entries.len());
    for entry in entries {
        println!(
            "  - {} · {:?} · cached {}",
            entry.artifact.artifact_id,
            entry.status,
            format_timestamp(entry.cached_at)
        );
        println!(
            "    Artifact: {:?} · locator {}",
            entry.artifact.kind,
            truncate_text(&entry.artifact.locator, 96)
        );
        println!("    Local path: {}", entry.local_path);
        if let Some(extracted_path) = entry.extracted_path.as_deref() {
            println!("    Extracted path: {}", extracted_path);
        }
        if let Some(error) = entry.error.as_deref() {
            println!("    Failure reason: {}", error);
        }
    }
}

fn print_lifecycle(mut records: Vec<SkillManagedLifecycleRecord>) {
    records.sort_by(|left, right| {
        left.source_id
            .cmp(&right.source_id)
            .then_with(|| left.skill_name.cmp(&right.skill_name))
    });
    println!("Lifecycle records: {}", records.len());
    for record in records {
        print_lifecycle_line(&record);
    }
}

fn print_lifecycle_line(record: &SkillManagedLifecycleRecord) {
    println!(
        "  State: {:?} · {} / {} · updated {}",
        record.state,
        record.source_id,
        record.skill_name,
        format_timestamp(record.updated_at)
    );
    println!("  Distribution: {}", record.distribution_id);
    if let Some(error) = record.error.as_deref() {
        println!("  Failure reason: {}", error);
    }
}

fn print_sync_plan(prefix: &str, plan: &SkillSyncPlan, guard_reports: usize) {
    println!(
        "{} for {} ({} entries).",
        prefix,
        plan.source_id,
        plan.entries.len()
    );
    let mut counts = BTreeMap::<&'static str, usize>::new();
    for entry in &plan.entries {
        let key = match entry.action {
            SkillSyncAction::Install => "install",
            SkillSyncAction::Update => "update",
            SkillSyncAction::SkipLocalModification => "skip_local_modification",
            SkillSyncAction::SkipDeletedLocally => "skip_deleted_locally",
            SkillSyncAction::RemoveManaged => "remove_managed",
            SkillSyncAction::Noop => "noop",
        };
        *counts.entry(key).or_default() += 1;
    }
    if !counts.is_empty() {
        let summary = counts
            .into_iter()
            .map(|(action, count)| format!("{action}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  Actions: {}", summary);
    }
    if guard_reports > 0 {
        println!("  Guard reports: {}", guard_reports);
    }
    for entry in plan.entries.iter().take(12) {
        println!(
            "  - {} · {:?} · {}",
            entry.skill_name, entry.action, entry.reason
        );
    }
}

fn print_remote_plan(prefix: &str, plan: &SkillRemoteInstallPlan) {
    println!(
        "{} for {} from {} ({:?}).",
        prefix, plan.entry.skill_name, plan.entry.source_id, plan.entry.action
    );
    println!("  Reason: {}", plan.entry.reason);
    println!("  Distribution: {}", plan.distribution.distribution_id);
    println!(
        "  Release: version {} · revision {}",
        optional_text(plan.distribution.release.version.as_deref()),
        optional_text(plan.distribution.release.revision.as_deref())
    );
    println!(
        "  Artifact: {} ({:?})",
        plan.distribution.resolution.artifact.artifact_id,
        plan.distribution.resolution.artifact.kind
    );
}

fn print_remote_apply(prefix: &str, response: &SkillRemoteInstallResponse) {
    println!(
        "{} for {} ({:?}).",
        prefix, response.result.skill_name, response.plan.entry.action
    );
    println!("  Workspace path: {}", response.result.location);
    println!(
        "  Distribution: {}",
        response.plan.distribution.distribution_id
    );
    println!(
        "  Artifact cache: {} ({:?})",
        response.artifact_cache.artifact.artifact_id, response.artifact_cache.status
    );
    if let Some(error) = response.artifact_cache.error.as_deref() {
        println!("  Artifact failure reason: {}", error);
    }
    if let Some(report) = response.guard_report.as_ref() {
        println!(
            "  Guard: {:?} ({} violations)",
            report.status,
            report.violations.len()
        );
    }
}

fn source_label(source: &SkillSourceRef) -> String {
    format!(
        "{} [{:?}] {}{}",
        source.source_id,
        source.source_kind,
        truncate_text(&source.locator, 72),
        source
            .revision
            .as_deref()
            .map(|revision| format!(" @ {}", revision))
            .unwrap_or_default()
    )
}

fn source_suffix(source: Option<&SkillSourceRef>, flags: &[&str]) -> String {
    let mut suffix = Vec::new();
    if let Some(source) = source {
        suffix.push(format!("source {}", source.source_id));
    }
    if !flags.is_empty() {
        suffix.push(flags.join(", "));
    }
    if suffix.is_empty() {
        String::new()
    } else {
        format!(" ({})", suffix.join(" · "))
    }
}

fn optional_text(value: Option<&str>) -> &str {
    value.filter(|value| !value.is_empty()).unwrap_or("--")
}

fn format_duration_seconds(value: u64) -> String {
    if value % 86_400 == 0 {
        format!("{}d ({}s)", value / 86_400, value)
    } else if value % 3_600 == 0 {
        format!("{}h ({}s)", value / 3_600, value)
    } else if value % 60 == 0 {
        format!("{}m ({}s)", value / 60, value)
    } else {
        format!("{}s", value)
    }
}

fn format_duration_ms(value: u64) -> String {
    if value % 1000 == 0 {
        format!("{}s ({}ms)", value / 1000, value)
    } else {
        format!("{}ms", value)
    }
}

fn format_bytes(value: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if value >= MIB && value % MIB == 0 {
        format!("{} MiB ({} bytes)", value / MIB, value)
    } else if value >= KIB && value % KIB == 0 {
        format!("{} KiB ({} bytes)", value / KIB, value)
    } else {
        format!("{} bytes", value)
    }
}

fn format_timestamp(timestamp: i64) -> String {
    match Local.timestamp_opt(timestamp, 0).single() {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S %z").to_string(),
        None => timestamp.to_string(),
    }
}
