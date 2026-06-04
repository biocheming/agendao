use std::collections::BTreeMap;

use agendao_types::{
    ManagedSkillRecord, SkillArtifactCacheEntry, SkillDistributionRecord, SkillHubIndexResponse,
    SkillHubPolicy, SkillHubRemoteInstallApplyRequest, SkillHubRemoteInstallPlanRequest,
    SkillHubRemoteUpdateApplyRequest, SkillHubRemoteUpdatePlanRequest, SkillHubSyncApplyRequest,
    SkillHubSyncPlanRequest, SkillHubUsageLedgerResponse, SkillHubVitalityUpdateRequest,
    SkillManagedLifecycleRecord, SkillNegativeEntropyDiagnostic, SkillOperationalSnapshot,
    SkillRemoteInstallPlan, SkillRemoteInstallResponse, SkillRetirementReasonKind,
    SkillSemanticConflictDiagnostic, SkillSourceKind, SkillSourceRef, SkillSyncAction,
    SkillSyncPlan, SkillVitalityState,
};
use chrono::{Local, TimeZone};
use serde::Serialize;

use crate::api_client::{
    CliApiClient, SkillHubIndexRefreshRequest, SkillHubManagedDetachRequest,
    SkillHubManagedRemoveRequest, SkillHubReviewCandidatesSyncRequest,
};
#[cfg(feature = "proposal-db")]
use crate::cli::ProposalCommands;
use crate::cli::{
    SkillCommands, SkillHubCommands, SkillHubOutputFormat, SkillRetirementReasonKindArg,
    SkillSourceArgs, SkillSourceKindArg, SkillVitalityStateArg,
};
#[cfg(feature = "proposal-db")]
use crate::cli_local_data;
use crate::import_export::{export_skill_data, import_skill_data};
use crate::server_lifecycle::FrontendRuntimeContext;
use crate::util::truncate_text;

#[derive(Debug, Clone)]
struct SkillReviewCandidateView {
    skill_name: String,
    source_scope: agendao_types::SkillOperationalSourceScope,
    runtime_use_count: u64,
    runtime_error_count: u64,
    write_count: u64,
    state: SkillVitalityState,
    reason_kind: SkillRetirementReasonKind,
    related_skill_name: Option<String>,
    summary: String,
    updated_at: i64,
    evidence_tags: Vec<String>,
    evidence_lines: Vec<String>,
}

pub(crate) async fn handle_skill_command(
    action: SkillCommands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match action {
        SkillCommands::Export { output } => export_skill_data(output),
        SkillCommands::Import { file } => import_skill_data(file),
        SkillCommands::Hub { action } => handle_skill_hub_command(action, runtime_context).await,
        SkillCommands::Proposal { action } => {
            #[cfg(feature = "proposal-db")]
            {
                handle_proposal_command(action).await
            }
            #[cfg(not(feature = "proposal-db"))]
            {
                let _ = action;
                anyhow::bail!("skill proposal commands require the `proposal-db` CLI feature")
            }
        }
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
            let usage = client.list_skill_hub_usage().await?;
            let index = client.list_skill_hub_index().await?;
            let distributions = client.list_skill_hub_distributions().await?;
            let artifact_cache = client.list_skill_hub_artifact_cache().await?;
            let policy = client.list_skill_hub_policy().await?;
            let lifecycle = client.list_skill_hub_lifecycle().await?;
            let negative_entropy = client.list_skill_hub_negative_entropy().await?;
            let semantic_conflicts = client.list_skill_hub_semantic_conflicts().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&serde_json::json!({
                    "managed": managed,
                    "usage": usage,
                    "index": index,
                    "distributions": distributions,
                    "artifact_cache": artifact_cache,
                    "policy": policy,
                    "lifecycle": lifecycle,
                    "negative_entropy": negative_entropy,
                    "semantic_conflicts": semantic_conflicts,
                }))?;
            } else {
                print_hub_status(
                    managed.managed_skills,
                    usage,
                    index.source_indices,
                    distributions.distributions,
                    artifact_cache.artifact_cache,
                    policy.policy,
                    lifecycle.lifecycle,
                    negative_entropy.candidates,
                    semantic_conflicts.conflicts,
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
        SkillHubCommands::Usage { output } => {
            let response = client.list_skill_hub_usage().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                let negative_entropy = client.list_skill_hub_negative_entropy().await?;
                let semantic_conflicts = client.list_skill_hub_semantic_conflicts().await?;
                print_usage_ledger(
                    response,
                    &negative_entropy.candidates,
                    &semantic_conflicts.conflicts,
                );
            }
        }
        SkillHubCommands::NegativeEntropy { output } => {
            let response = client.list_skill_hub_negative_entropy().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_negative_entropy(response.candidates);
            }
        }
        SkillHubCommands::ReviewCandidatesSync { session_id, output } => {
            let response = client
                .sync_skill_hub_review_candidates(&SkillHubReviewCandidatesSyncRequest {
                    session_id,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                println!(
                    "Marked {} workspace-local review candidate(s).",
                    response.updated.len()
                );
                let negative_entropy = client.list_skill_hub_negative_entropy().await?;
                let semantic_conflicts = client.list_skill_hub_semantic_conflicts().await?;
                print_usage_ledger(
                    SkillHubUsageLedgerResponse {
                        entries: response.updated,
                    },
                    &negative_entropy.candidates,
                    &semantic_conflicts.conflicts,
                );
            }
        }
        SkillHubCommands::SemanticConflicts { output } => {
            let response = client.list_skill_hub_semantic_conflicts().await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                print_semantic_conflicts(response.conflicts);
            }
        }
        SkillHubCommands::SemanticConflictReviewSync { session_id, output } => {
            let response = client
                .sync_skill_hub_semantic_conflict_review_candidates(
                    &SkillHubReviewCandidatesSyncRequest { session_id },
                )
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                println!(
                    "Marked {} semantic-conflict review candidate(s).",
                    response.updated.len()
                );
                let negative_entropy = client.list_skill_hub_negative_entropy().await?;
                let semantic_conflicts = client.list_skill_hub_semantic_conflicts().await?;
                print_usage_ledger(
                    SkillHubUsageLedgerResponse {
                        entries: response.updated,
                    },
                    &negative_entropy.candidates,
                    &semantic_conflicts.conflicts,
                );
            }
        }
        SkillHubCommands::VitalitySet {
            session_id,
            skill_name,
            state,
            reason_kind,
            summary,
            related_skill_name,
            output,
        } => {
            let response = client
                .update_skill_hub_vitality(&SkillHubVitalityUpdateRequest {
                    session_id,
                    skill_name,
                    state: skill_vitality_state_from_arg(state),
                    reason_kind: skill_retirement_reason_kind_from_arg(reason_kind, state),
                    summary,
                    related_skill_name,
                })
                .await?;
            if matches!(output.format, SkillHubOutputFormat::Json) {
                print_json(&response)?;
            } else {
                println!(
                    "Updated vitality for {} -> {}.",
                    response.snapshot.skill_name,
                    format_vitality_state(response.snapshot.vitality.as_ref().map(|v| v.state))
                );
                print_usage_ledger(
                    SkillHubUsageLedgerResponse {
                        entries: vec![response.snapshot],
                    },
                    &[],
                    &[],
                );
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
    usage: SkillHubUsageLedgerResponse,
    index: Vec<agendao_types::SkillSourceIndexSnapshot>,
    distributions: Vec<SkillDistributionRecord>,
    artifact_cache: Vec<SkillArtifactCacheEntry>,
    policy: SkillHubPolicy,
    lifecycle: Vec<SkillManagedLifecycleRecord>,
    negative_entropy: Vec<SkillNegativeEntropyDiagnostic>,
    semantic_conflicts: Vec<SkillSemanticConflictDiagnostic>,
) {
    let review_candidate_views =
        build_review_candidate_views(&usage.entries, &negative_entropy, &semantic_conflicts);
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
    let runtime_used_count = usage
        .entries
        .iter()
        .filter(|entry| {
            entry
                .usage
                .as_ref()
                .map(|usage| usage.runtime_use_count > 0)
                .unwrap_or(false)
        })
        .count();
    let never_reused_count = usage
        .entries
        .iter()
        .filter(|entry| {
            entry
                .writes
                .as_ref()
                .map(total_usage_write_count)
                .unwrap_or(0)
                > 0
                && entry
                    .usage
                    .as_ref()
                    .map(|usage| usage.runtime_use_count)
                    .unwrap_or(0)
                    == 0
        })
        .count();

    println!("Skill hub status");
    println!("  Managed: {}", managed.len());
    println!(
        "  Usage ledger: {} entries ({} runtime-used, {} never reused)",
        usage.entries.len(),
        runtime_used_count,
        never_reused_count
    );
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
    println!(
        "  Diagnostics: {} negative-entropy candidates · {} semantic overlap pairs",
        negative_entropy.len(),
        semantic_conflicts.len()
    );
    println!(
        "  Review candidates: {} owner-local vitality mark(s)",
        review_candidate_views.len()
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

    let mut top_runtime_used = usage.entries.clone();
    top_runtime_used.sort_by(|left, right| {
        right
            .usage
            .as_ref()
            .map(|usage| usage.runtime_use_count)
            .unwrap_or(0)
            .cmp(
                &left
                    .usage
                    .as_ref()
                    .map(|usage| usage.runtime_use_count)
                    .unwrap_or(0),
            )
            .then_with(|| left.skill_name.cmp(&right.skill_name))
    });
    let top_runtime_used = top_runtime_used
        .into_iter()
        .filter(|entry| {
            entry
                .usage
                .as_ref()
                .map(|usage| usage.runtime_use_count > 0)
                .unwrap_or(false)
        })
        .take(8)
        .collect::<Vec<_>>();
    if !top_runtime_used.is_empty() {
        println!("\nTop runtime-used skills:");
        for entry in top_runtime_used {
            let use_count = entry
                .usage
                .as_ref()
                .map(|usage| usage.runtime_use_count)
                .unwrap_or(0);
            println!(
                "  - {} · {} use(s) · last used {}",
                entry.skill_name,
                use_count,
                format_optional_timestamp(
                    entry.usage.as_ref().and_then(|usage| usage.last_used_at)
                )
            );
        }
    }

    print_review_candidate_views(
        "\nReview candidates:",
        &review_candidate_views,
        Some("These are owner-local vitality marks. The lines below explain why a skill was marked review_candidate using ledger state plus negative-entropy / semantic-conflict evidence."),
        8,
    );

    if !negative_entropy.is_empty() {
        println!("\nNegative entropy diagnostics:");
        for item in negative_entropy.into_iter().take(8) {
            let signals = item
                .signals
                .iter()
                .map(|signal| format_negative_entropy_signal(*signal).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  - {} · {} · {}",
                item.skill_name,
                format_diagnostic_severity(item.severity),
                signals
            );
            if let Some(reason) = item.reasons.first() {
                println!("    {}", reason);
            }
        }
    }

    if !semantic_conflicts.is_empty() {
        println!("\nSemantic overlap priorities:");
        for item in semantic_conflicts.into_iter().take(8) {
            println!(
                "  - {} <> {} · {} · score {}",
                item.left_skill_name,
                item.right_skill_name,
                format_semantic_conflict_kind(item.kind),
                item.score
            );
            if let Some(preferred) = item.preferred_skill_name.as_deref() {
                println!("    Preferred by ledger: {}", preferred);
            }
            if let Some(reason) = item.reasons.first() {
                println!("    {}", reason);
            }
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

fn print_usage_ledger(
    mut response: SkillHubUsageLedgerResponse,
    negative_entropy: &[SkillNegativeEntropyDiagnostic],
    semantic_conflicts: &[SkillSemanticConflictDiagnostic],
) {
    let review_candidate_views =
        build_review_candidate_views(&response.entries, negative_entropy, semantic_conflicts);
    response
        .entries
        .sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
    println!("Skill usage ledger: {}", response.entries.len());
    for entry in response.entries {
        let runtime_use_count = entry
            .usage
            .as_ref()
            .map(|usage| usage.runtime_use_count)
            .unwrap_or(0);
        let runtime_error_count = entry
            .usage
            .as_ref()
            .map(|usage| usage.runtime_error_count)
            .unwrap_or(0);
        let write_count = entry
            .writes
            .as_ref()
            .map(total_usage_write_count)
            .unwrap_or(0);
        println!(
            "  - {} · scope {} · vitality {} · use {} · writes {} · errors {}",
            entry.skill_name,
            format_source_scope(entry.source_scope),
            format_vitality_state(entry.vitality.as_ref().map(|record| record.state)),
            runtime_use_count,
            write_count,
            runtime_error_count
        );
        println!(
            "    Last used: {} · last written: {}",
            format_optional_timestamp(entry.usage.as_ref().and_then(|usage| usage.last_used_at)),
            format_optional_timestamp(
                entry
                    .writes
                    .as_ref()
                    .and_then(|writes| writes.last_write_at)
            )
        );
        if let Some(vitality) = entry.vitality.as_ref() {
            println!(
                "    Vitality reason: {} · {} · updated {}",
                format_skill_retirement_reason_kind_human(vitality.reason.kind),
                vitality.reason.summary,
                format_timestamp(vitality.updated_at)
            );
            if let Some(related_skill_name) = vitality.reason.related_skill_name.as_deref() {
                println!("    Related skill: {}", related_skill_name);
            }
        }
    }
    print_review_candidate_views(
        "\nReview candidate explanation:",
        &review_candidate_views,
        Some("Only owner-local vitality entries marked review_candidate appear here. The explanation joins usage-ledger state with the matching governance diagnostics."),
        usize::MAX,
    );
}

fn print_negative_entropy(mut candidates: Vec<SkillNegativeEntropyDiagnostic>) {
    candidates.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
    println!("Negative entropy diagnostics: {}", candidates.len());
    for item in candidates {
        let signals = item
            .signals
            .iter()
            .map(|signal| format_negative_entropy_signal(*signal).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  - {} · {} · {}",
            item.skill_name,
            format_diagnostic_severity(item.severity),
            signals
        );
        println!(
            "    use {} · writes {} · last used {} · last written {}",
            item.runtime_use_count,
            item.write_count,
            format_optional_timestamp(item.last_used_at),
            format_optional_timestamp(item.last_write_at)
        );
        for reason in item.reasons.into_iter().take(2) {
            println!("    {}", reason);
        }
    }
}

fn print_semantic_conflicts(mut conflicts: Vec<SkillSemanticConflictDiagnostic>) {
    conflicts.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.left_skill_name.cmp(&right.left_skill_name))
            .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
    });
    println!("Semantic conflict diagnostics: {}", conflicts.len());
    for item in conflicts {
        println!(
            "  - {} <> {} · {} · score {}",
            item.left_skill_name,
            item.right_skill_name,
            format_semantic_conflict_kind(item.kind),
            item.score
        );
        println!(
            "    ledger: {} use(s) vs {} use(s)",
            item.left_runtime_use_count, item.right_runtime_use_count
        );
        if let Some(preferred) = item.preferred_skill_name.as_deref() {
            println!("    preferred skill: {}", preferred);
        }
        for reason in item.reasons.into_iter().take(2) {
            println!("    {}", reason);
        }
    }
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

fn total_usage_write_count(entry: &agendao_types::SkillWriteLedgerEntry) -> u64 {
    entry.create_count
        + entry.patch_count
        + entry.edit_count
        + entry.supporting_file_write_count
        + entry.supporting_file_remove_count
        + entry.install_count
        + entry.update_count
        + entry.detach_count
        + entry.remove_count
        + entry.delete_count
}

fn format_optional_timestamp(timestamp: Option<i64>) -> String {
    timestamp
        .map(format_timestamp)
        .unwrap_or_else(|| "--".to_string())
}

fn format_source_scope(scope: agendao_types::SkillOperationalSourceScope) -> &'static str {
    match scope {
        agendao_types::SkillOperationalSourceScope::WorkspaceLocal => "workspace_local",
        agendao_types::SkillOperationalSourceScope::Managed => "managed",
        agendao_types::SkillOperationalSourceScope::DiscoveredReadOnly => "discovered_read_only",
        agendao_types::SkillOperationalSourceScope::Unknown => "unknown",
    }
}

fn format_vitality_state(state: Option<SkillVitalityState>) -> &'static str {
    match state.unwrap_or(SkillVitalityState::Active) {
        SkillVitalityState::Active => "active",
        SkillVitalityState::ReviewCandidate => "review_candidate",
        SkillVitalityState::Retired => "retired",
        SkillVitalityState::Archived => "archived",
    }
}

fn format_skill_retirement_reason_kind_human(kind: SkillRetirementReasonKind) -> &'static str {
    match kind {
        SkillRetirementReasonKind::NegativeEntropy => "negative entropy",
        SkillRetirementReasonKind::SemanticConflict => "semantic conflict",
        SkillRetirementReasonKind::ManualOverride => "manual override",
        SkillRetirementReasonKind::Restored => "restored",
    }
}

fn skill_vitality_state_from_arg(state: SkillVitalityStateArg) -> SkillVitalityState {
    match state {
        SkillVitalityStateArg::Active => SkillVitalityState::Active,
        SkillVitalityStateArg::ReviewCandidate => SkillVitalityState::ReviewCandidate,
        SkillVitalityStateArg::Retired => SkillVitalityState::Retired,
        SkillVitalityStateArg::Archived => SkillVitalityState::Archived,
    }
}

fn skill_retirement_reason_kind_from_arg(
    reason_kind: Option<SkillRetirementReasonKindArg>,
    state: SkillVitalityStateArg,
) -> SkillRetirementReasonKind {
    match reason_kind {
        Some(SkillRetirementReasonKindArg::NegativeEntropy) => {
            SkillRetirementReasonKind::NegativeEntropy
        }
        Some(SkillRetirementReasonKindArg::SemanticConflict) => {
            SkillRetirementReasonKind::SemanticConflict
        }
        Some(SkillRetirementReasonKindArg::ManualOverride) => {
            SkillRetirementReasonKind::ManualOverride
        }
        Some(SkillRetirementReasonKindArg::Restored) => SkillRetirementReasonKind::Restored,
        None => match state {
            SkillVitalityStateArg::Active => SkillRetirementReasonKind::Restored,
            SkillVitalityStateArg::ReviewCandidate => SkillRetirementReasonKind::NegativeEntropy,
            SkillVitalityStateArg::Retired | SkillVitalityStateArg::Archived => {
                SkillRetirementReasonKind::ManualOverride
            }
        },
    }
}

fn format_diagnostic_severity(
    severity: agendao_types::SkillGovernanceDiagnosticSeverity,
) -> &'static str {
    match severity {
        agendao_types::SkillGovernanceDiagnosticSeverity::Info => "info",
        agendao_types::SkillGovernanceDiagnosticSeverity::Warn => "warn",
    }
}

fn format_negative_entropy_signal(
    signal: agendao_types::SkillNegativeEntropySignal,
) -> &'static str {
    match signal {
        agendao_types::SkillNegativeEntropySignal::NeverReused => "never_reused",
        agendao_types::SkillNegativeEntropySignal::StaleUnused => "stale_unused",
        agendao_types::SkillNegativeEntropySignal::WriteHeavyLowReuse => "write_heavy_low_reuse",
        agendao_types::SkillNegativeEntropySignal::DormantManaged => "dormant_managed",
    }
}

fn format_semantic_conflict_kind(kind: agendao_types::SkillSemanticConflictKind) -> &'static str {
    match kind {
        agendao_types::SkillSemanticConflictKind::NearDuplicate => "near_duplicate",
        agendao_types::SkillSemanticConflictKind::TriggerOverlap => "trigger_overlap",
        agendao_types::SkillSemanticConflictKind::ReplacementHint => "replacement_hint",
    }
}

fn build_review_candidate_views(
    entries: &[SkillOperationalSnapshot],
    negative_entropy: &[SkillNegativeEntropyDiagnostic],
    semantic_conflicts: &[SkillSemanticConflictDiagnostic],
) -> Vec<SkillReviewCandidateView> {
    let mut views = entries
        .iter()
        .filter_map(|entry| {
            let vitality = entry.vitality.as_ref()?;
            if vitality.state != SkillVitalityState::ReviewCandidate {
                return None;
            }
            let runtime_use_count = entry
                .usage
                .as_ref()
                .map(|usage| usage.runtime_use_count)
                .unwrap_or(0);
            let runtime_error_count = entry
                .usage
                .as_ref()
                .map(|usage| usage.runtime_error_count)
                .unwrap_or(0);
            let write_count = entry
                .writes
                .as_ref()
                .map(total_usage_write_count)
                .unwrap_or(0);
            let mut evidence_tags = Vec::new();
            let mut evidence_lines = Vec::new();
            match vitality.reason.kind {
                SkillRetirementReasonKind::NegativeEntropy => {
                    if let Some(diagnostic) = negative_entropy
                        .iter()
                        .find(|item| skill_name_eq(&item.skill_name, &entry.skill_name))
                    {
                        evidence_tags.extend(
                            diagnostic
                                .signals
                                .iter()
                                .map(|signal| format_negative_entropy_signal(*signal).to_string()),
                        );
                        evidence_lines.push(format!(
                            "ledger: use {} · writes {} · overlap {}",
                            diagnostic.runtime_use_count,
                            diagnostic.write_count,
                            diagnostic.semantic_overlap_count
                        ));
                        evidence_lines.extend(
                            diagnostic
                                .reasons
                                .iter()
                                .filter(|line| line.as_str() != vitality.reason.summary)
                                .take(2)
                                .cloned(),
                        );
                    }
                }
                SkillRetirementReasonKind::SemanticConflict => {
                    if let Some(conflict) = find_semantic_conflict_for_entry(
                        entry,
                        vitality.reason.related_skill_name.as_deref(),
                        semantic_conflicts,
                    ) {
                        evidence_tags
                            .push(format_semantic_conflict_kind(conflict.kind).to_string());
                        evidence_tags.push(format!("score {}", conflict.score));
                        evidence_lines.push(format!(
                            "ledger pair: {} {} use(s) · {} {} use(s)",
                            conflict.left_skill_name,
                            conflict.left_runtime_use_count,
                            conflict.right_skill_name,
                            conflict.right_runtime_use_count
                        ));
                        if let Some(preferred_skill_name) = conflict.preferred_skill_name.as_deref()
                        {
                            evidence_lines.push(format!("ledger prefers {}", preferred_skill_name));
                        }
                        evidence_lines.extend(
                            conflict
                                .reasons
                                .iter()
                                .filter(|line| line.as_str() != vitality.reason.summary)
                                .take(2)
                                .cloned(),
                        );
                    }
                }
                SkillRetirementReasonKind::ManualOverride | SkillRetirementReasonKind::Restored => {
                }
            }
            Some(SkillReviewCandidateView {
                skill_name: entry.skill_name.clone(),
                source_scope: entry.source_scope,
                runtime_use_count,
                runtime_error_count,
                write_count,
                state: vitality.state,
                reason_kind: vitality.reason.kind,
                related_skill_name: vitality.reason.related_skill_name.clone(),
                summary: vitality.reason.summary.clone(),
                updated_at: vitality.updated_at,
                evidence_tags,
                evidence_lines,
            })
        })
        .collect::<Vec<_>>();
    views.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.skill_name.cmp(&right.skill_name))
    });
    views
}

fn find_semantic_conflict_for_entry<'a>(
    entry: &SkillOperationalSnapshot,
    related_skill_name: Option<&str>,
    semantic_conflicts: &'a [SkillSemanticConflictDiagnostic],
) -> Option<&'a SkillSemanticConflictDiagnostic> {
    semantic_conflicts.iter().find(|item| {
        let matches_skill = skill_name_eq(&item.left_skill_name, &entry.skill_name)
            || skill_name_eq(&item.right_skill_name, &entry.skill_name);
        if !matches_skill {
            return false;
        }
        let Some(related_skill_name) = related_skill_name else {
            return true;
        };
        skill_name_eq(&item.left_skill_name, related_skill_name)
            || skill_name_eq(&item.right_skill_name, related_skill_name)
            || item
                .preferred_skill_name
                .as_deref()
                .map(|preferred| skill_name_eq(preferred, related_skill_name))
                .unwrap_or(false)
    })
}

fn print_review_candidate_views(
    title: &str,
    views: &[SkillReviewCandidateView],
    subtitle: Option<&str>,
    limit: usize,
) {
    if views.is_empty() {
        return;
    }
    println!("{title}");
    if let Some(subtitle) = subtitle {
        println!("  {}", subtitle);
    }
    for item in views.iter().take(limit) {
        println!(
            "  - {} · scope {} · vitality {} · {}",
            item.skill_name,
            format_source_scope(item.source_scope),
            format_vitality_state(Some(item.state)),
            format_skill_retirement_reason_kind_human(item.reason_kind),
        );
        println!("    marked because: {}", item.summary);
        println!(
            "    ledger: use {} · writes {} · errors {} · updated {}",
            item.runtime_use_count,
            item.write_count,
            item.runtime_error_count,
            format_timestamp(item.updated_at)
        );
        if let Some(related_skill_name) = item.related_skill_name.as_deref() {
            println!("    related skill: {}", related_skill_name);
        }
        if !item.evidence_tags.is_empty() {
            println!("    evidence: {}", item.evidence_tags.join(", "));
        }
        for line in &item.evidence_lines {
            println!("    why: {}", line);
        }
    }
}

fn skill_name_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
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

// ── Proposal commands ────────────────────────────────────────────────────

#[cfg(feature = "proposal-db")]
async fn handle_proposal_command(action: ProposalCommands) -> anyhow::Result<()> {
    match action {
        ProposalCommands::List { status } => {
            let status: agendao_types::ProposalStatus =
                serde_json::from_str(&format!("\"{}\"", status))?;
            let proposals = cli_local_data::list_skill_evolution_proposals(&status).await?;
            if proposals.is_empty() {
                println!("No proposals with status: {:?}", status);
                return Ok(());
            }
            for p in &proposals {
                let kind = match p.proposal_kind {
                    agendao_types::SkillEvolutionProposalKind::PatchExistingSkill => "patch",
                    agendao_types::SkillEvolutionProposalKind::CreateNewSkill => "create",
                };
                println!(
                    "{:<36} {:>8} {:>6} {:>12}  {}",
                    p.id,
                    kind,
                    status_label(&p.status),
                    p.linked_skill_name.as_deref().unwrap_or("-"),
                    p.title,
                );
            }
        }
        ProposalCommands::Show { id } => {
            let Some(proposal) = cli_local_data::get_skill_evolution_proposal(&id).await? else {
                anyhow::bail!("proposal not found: {}", id);
            };
            println!("ID:              {}", proposal.id);
            println!("Session:         {}", proposal.session_id);
            println!("Kind:            {:?}", proposal.proposal_kind);
            println!(
                "Skill:           {}",
                proposal
                    .linked_skill_name
                    .as_deref()
                    .unwrap_or("(new skill)")
            );
            println!("Status:          {:?}", proposal.status);
            println!(
                "Created:         {}",
                format_timestamp(proposal.created_at_ms / 1000)
            );
            println!();
            println!("Title:           {}", proposal.title);
            println!();
            println!("Rationale:");
            println!("  {}", proposal.rationale);
            println!();
            println!("Evidence (memory record IDs):");
            for rid in &proposal.memory_record_ids {
                println!("  - {}", rid);
            }
            println!();
            println!("Suggested changes:");
            for change in &proposal.suggested_changes {
                match change {
                    agendao_types::SuggestedSkillChange::AddTriggerCondition {
                        text,
                        evidence_refs,
                    } => {
                        println!("  + Trigger: {}", text);
                        if !evidence_refs.is_empty() {
                            println!("    refs: {}", evidence_refs.join(", "));
                        }
                    }
                    agendao_types::SuggestedSkillChange::AddCoreStep {
                        text,
                        evidence_refs,
                    } => {
                        println!("  + Step: {}", text);
                        if !evidence_refs.is_empty() {
                            println!("    refs: {}", evidence_refs.join(", "));
                        }
                    }
                    agendao_types::SuggestedSkillChange::AddBoundary {
                        text,
                        evidence_refs,
                    } => {
                        println!("  + Boundary: {}", text);
                        if !evidence_refs.is_empty() {
                            println!("    refs: {}", evidence_refs.join(", "));
                        }
                    }
                    agendao_types::SuggestedSkillChange::AddValidationStep {
                        text,
                        evidence_refs,
                    } => {
                        println!("  + Validation: {}", text);
                        if !evidence_refs.is_empty() {
                            println!("    refs: {}", evidence_refs.join(", "));
                        }
                    }
                    agendao_types::SuggestedSkillChange::CreateSkillDraft {
                        suggested_name,
                        when_to_use,
                        core_steps,
                        boundaries,
                        validation,
                    } => {
                        println!("  = Create skill '{}'", suggested_name);
                        println!("    When to use:");
                        for w in when_to_use {
                            println!("      - {}", w);
                        }
                        println!("    Steps:");
                        for s in core_steps {
                            println!("      - {}", s);
                        }
                        println!("    Boundaries:");
                        for b in boundaries {
                            println!("      - {}", b);
                        }
                        println!("    Validation:");
                        for v in validation {
                            println!("      - {}", v);
                        }
                    }
                }
                println!();
            }
        }
        ProposalCommands::Approve { id } => {
            cli_local_data::transition_skill_evolution_proposal(
                &id,
                &agendao_types::ProposalStatus::Accepted,
            )
            .await?;
            println!("Proposal {} approved.", id);
            println!("Note: Accepted does not modify SKILL.md. It marks the suggestion as approved for future application.");
        }
        ProposalCommands::Reject { id } => {
            cli_local_data::transition_skill_evolution_proposal(
                &id,
                &agendao_types::ProposalStatus::Rejected,
            )
            .await?;
            println!("Proposal {} rejected.", id);
        }
    }

    Ok(())
}

#[cfg(feature = "proposal-db")]
fn status_label(status: &agendao_types::ProposalStatus) -> &'static str {
    match status {
        agendao_types::ProposalStatus::Draft => "draft",
        agendao_types::ProposalStatus::Accepted => "accepted",
        agendao_types::ProposalStatus::Rejected => "rejected",
        agendao_types::ProposalStatus::Superseded => "superseded",
        agendao_types::ProposalStatus::Applied => "applied",
    }
}
