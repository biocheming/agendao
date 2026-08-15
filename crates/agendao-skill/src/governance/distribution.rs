use super::audit::{
    distribution_audit_event, hub_detach_audit_event, hub_remove_audit_event,
    remote_plan_audit_event, write_audit_event,
};
use super::SkillGovernanceAuthority;
use crate::util::now_unix_timestamp;
use crate::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, RemoveSkillFileRequest, SkillError,
    SkillWriteResult, WriteSkillFileRequest,
};
use agendao_types::{
    ManagedSkillRecord, SkillArtifactCacheEntry, SkillArtifactCacheStatus, SkillAuditKind,
    SkillDistributionRecord, SkillGovernanceWriteResult, SkillHubManagedDetachResponse,
    SkillHubManagedRemoveResponse, SkillManagedLifecycleRecord, SkillManagedLifecycleState,
    SkillOperationalSourceScope, SkillRemoteInstallAction, SkillRemoteInstallEntry,
    SkillRemoteInstallPlan, SkillRemoteInstallResponse, SkillSourceRef, SkillWriteLedgerAction,
};
use std::collections::BTreeMap;

impl SkillGovernanceAuthority {
    pub fn resolve_distribution(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillDistributionRecord, SkillError> {
        let source_index = self
            .hub_store
            .source_index(&source.source_id)
            .unwrap_or(self.refresh_source_index(source, actor)?);
        match self.distribution_resolver.resolve_distribution(
            self.hub_store.base_dir(),
            source,
            &source_index,
            skill_name,
            self.artifact_policy().fetch_timeout_ms,
        ) {
            Ok(resolved) => {
                let record = resolved.record;
                self.upsert_distribution(record.clone())?;
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: record.distribution_id.clone(),
                        source_id: source.source_id.clone(),
                        skill_name: record.skill_name.clone(),
                        state: SkillManagedLifecycleState::Resolved,
                        updated_at: record.resolution.resolved_at,
                        error: None,
                    },
                )?;
                self.append_audit_event(distribution_audit_event(
                    SkillAuditKind::SourceResolved,
                    actor,
                    &record,
                    None,
                ))?;
                Ok(record)
            }
            Err(error) => {
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: unresolved_distribution_id(source, skill_name),
                        source_id: source.source_id.clone(),
                        skill_name: skill_name.trim().to_string(),
                        state: SkillManagedLifecycleState::ResolutionFailed,
                        updated_at: now_unix_timestamp(),
                        error: Some(error.to_string()),
                    },
                )?;
                Err(error)
            }
        }
    }

    pub fn fetch_distribution_artifact(
        &self,
        distribution_id: &str,
        actor: &str,
    ) -> Result<SkillArtifactCacheEntry, SkillError> {
        let _ = self.reconcile_artifact_cache_policy()?;
        let distribution = self
            .distributions()
            .into_iter()
            .find(|record| record.distribution_id == distribution_id)
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!("unknown distribution `{distribution_id}`"),
            })?;

        match self
            .artifact_store
            .fetch_artifact(&distribution.resolution.artifact)
        {
            Ok(entry) => {
                self.upsert_artifact_cache_entry(entry.clone())?;
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: distribution.distribution_id.clone(),
                        source_id: distribution.source.source_id.clone(),
                        skill_name: distribution.skill_name.clone(),
                        state: SkillManagedLifecycleState::Fetched,
                        updated_at: entry.cached_at,
                        error: None,
                    },
                )?;
                self.append_audit_event(distribution_audit_event(
                    SkillAuditKind::ArtifactFetched,
                    actor,
                    &distribution,
                    None,
                ))?;
                Ok(entry)
            }
            Err(error) => {
                self.upsert_artifact_cache_entry(SkillArtifactCacheEntry {
                    artifact: distribution.resolution.artifact.clone(),
                    cached_at: now_unix_timestamp(),
                    local_path: self
                        .artifact_store
                        .artifact_cache_dir()
                        .to_string_lossy()
                        .to_string(),
                    extracted_path: None,
                    status: SkillArtifactCacheStatus::Failed,
                    error: Some(error.to_string()),
                })?;
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: distribution.distribution_id.clone(),
                        source_id: distribution.source.source_id.clone(),
                        skill_name: distribution.skill_name.clone(),
                        state: SkillManagedLifecycleState::FetchFailed,
                        updated_at: now_unix_timestamp(),
                        error: Some(error.to_string()),
                    },
                )?;
                self.append_audit_event(distribution_audit_event(
                    SkillAuditKind::ArtifactFetchFailed,
                    actor,
                    &distribution,
                    Some(error.to_string()),
                ))?;
                Err(error)
            }
        }
    }

    pub fn plan_remote_install(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallPlan, SkillError> {
        let distribution = self.resolve_distribution(source, skill_name, actor)?;
        let action = self.remote_install_action(&distribution)?;
        let plan = SkillRemoteInstallPlan {
            source_id: source.source_id.clone(),
            distribution: distribution.clone(),
            entry: SkillRemoteInstallEntry {
                distribution_id: distribution.distribution_id.clone(),
                source_id: source.source_id.clone(),
                skill_name: distribution.skill_name.clone(),
                action,
                reason: remote_install_reason(&distribution),
            },
        };
        self.record_lifecycle(
            Some(actor),
            SkillManagedLifecycleRecord {
                distribution_id: distribution.distribution_id.clone(),
                source_id: source.source_id.clone(),
                skill_name: distribution.skill_name,
                state: SkillManagedLifecycleState::PlannedInstall,
                updated_at: now_unix_timestamp(),
                error: None,
            },
        )?;
        self.append_audit_event(remote_plan_audit_event(
            match plan.entry.action {
                SkillRemoteInstallAction::Install => SkillAuditKind::RemoteInstallPlanned,
                SkillRemoteInstallAction::Update => SkillAuditKind::RemoteUpdatePlanned,
            },
            actor,
            &plan,
        ))?;
        Ok(plan)
    }

    pub fn apply_remote_install(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallResponse, SkillError> {
        let plan = self.plan_remote_install(source, skill_name, actor)?;
        self.apply_remote_plan(source, actor, plan)
    }

    pub fn plan_remote_update(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallPlan, SkillError> {
        let managed = self.refresh_managed_record_for_source_skill(source, skill_name)?;
        let mut distribution = self.resolve_distribution(source, skill_name, actor)?;
        let installed_distribution = self
            .current_distribution_for_managed_record(&managed.record)
            .and_then(|record| record.installed);
        if distribution.installed.is_none() {
            distribution.installed = installed_distribution;
        }

        let lifecycle_state = self
            .lifecycle
            .managed_runtime_state(&managed.record, release_identity(&distribution.release));
        let update_available = self.lifecycle.update_available(
            managed.record.installed_revision.as_deref(),
            release_identity(&distribution.release),
        );
        if !update_available && lifecycle_state != SkillManagedLifecycleState::Diverged {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is already current for source `{}`",
                    managed.record.skill_name, source.source_id
                ),
            });
        }

        distribution.lifecycle = lifecycle_state.clone();
        self.upsert_distribution(distribution.clone())?;
        self.record_lifecycle(
            Some(actor),
            self.lifecycle.build_record(
                distribution.distribution_id.clone(),
                source.source_id.clone(),
                distribution.skill_name.clone(),
                lifecycle_state.clone(),
                now_unix_timestamp(),
                None,
            ),
        )?;
        self.append_audit_event(remote_plan_audit_event(
            SkillAuditKind::RemoteUpdatePlanned,
            actor,
            &SkillRemoteInstallPlan {
                source_id: source.source_id.clone(),
                distribution: distribution.clone(),
                entry: SkillRemoteInstallEntry {
                    distribution_id: distribution.distribution_id.clone(),
                    source_id: source.source_id.clone(),
                    skill_name: distribution.skill_name.clone(),
                    action: SkillRemoteInstallAction::Update,
                    reason: remote_update_reason(
                        &distribution,
                        &managed.record,
                        lifecycle_state.clone(),
                    ),
                },
            },
        ))?;

        Ok(SkillRemoteInstallPlan {
            source_id: source.source_id.clone(),
            distribution: distribution.clone(),
            entry: SkillRemoteInstallEntry {
                distribution_id: distribution.distribution_id.clone(),
                source_id: source.source_id.clone(),
                skill_name: distribution.skill_name.clone(),
                action: SkillRemoteInstallAction::Update,
                reason: remote_update_reason(&distribution, &managed.record, lifecycle_state),
            },
        })
    }

    pub fn apply_remote_update(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallResponse, SkillError> {
        let plan = self.plan_remote_update(source, skill_name, actor)?;
        self.apply_remote_plan(source, actor, plan)
    }

    pub fn detach_managed_skill(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillHubManagedDetachResponse, SkillError> {
        let managed = self.refresh_managed_record_for_source_skill(source, skill_name)?;
        let removed = self
            .hub_store
            .remove_managed_skill(&managed.record.skill_name)?
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not managed by source `{}`",
                    skill_name.trim(),
                    source.source_id
                ),
            })?;
        let timestamp = now_unix_timestamp();
        let distribution_id = self
            .current_distribution_for_managed_record(&removed)
            .map(|distribution| distribution.distribution_id)
            .unwrap_or_else(|| unresolved_distribution_id(source, &removed.skill_name));
        if let Some(mut distribution) = self.current_distribution_for_managed_record(&removed) {
            distribution.lifecycle = SkillManagedLifecycleState::Detached;
            self.upsert_distribution(distribution)?;
        }
        let lifecycle = self.lifecycle.build_record(
            distribution_id,
            source.source_id.clone(),
            removed.skill_name.clone(),
            SkillManagedLifecycleState::Detached,
            timestamp,
            None,
        );
        self.record_lifecycle(Some(actor), lifecycle.clone())?;
        self.append_audit_event(hub_detach_audit_event(source, actor, &removed))?;
        self.record_skill_write_action(
            &removed.skill_name,
            None,
            SkillWriteLedgerAction::Detach,
            SkillOperationalSourceScope::WorkspaceLocal,
            None,
            None,
        )?;
        Ok(SkillHubManagedDetachResponse { lifecycle })
    }

    pub fn remove_managed_skill(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillHubManagedRemoveResponse, SkillError> {
        let managed = self.refresh_managed_record_for_source_skill(source, skill_name)?;
        let mut deleted_from_workspace = false;
        let mut result = None;
        if let Some(current_hash) = managed.current_hash.as_deref() {
            if managed.record.local_hash.as_deref() == Some(current_hash) {
                let write_result = self.skill_authority.delete_skill(DeleteSkillRequest {
                    name: managed.record.skill_name.clone(),
                })?;
                deleted_from_workspace = true;
                result = Some(governance_write_result(&write_result));
            }
        }

        let removed = self
            .hub_store
            .remove_managed_skill(&managed.record.skill_name)?
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not managed by source `{}`",
                    skill_name.trim(),
                    source.source_id
                ),
            })?;
        let timestamp = now_unix_timestamp();
        let distribution_id = self
            .current_distribution_for_managed_record(&removed)
            .map(|distribution| distribution.distribution_id)
            .unwrap_or_else(|| unresolved_distribution_id(source, &removed.skill_name));
        if let Some(mut distribution) = self.current_distribution_for_managed_record(&removed) {
            distribution.lifecycle = SkillManagedLifecycleState::Removed;
            if deleted_from_workspace {
                distribution.installed = None;
            }
            self.upsert_distribution(distribution)?;
        }
        let lifecycle = self.lifecycle.build_record(
            distribution_id,
            source.source_id.clone(),
            removed.skill_name.clone(),
            SkillManagedLifecycleState::Removed,
            timestamp,
            None,
        );
        self.record_lifecycle(Some(actor), lifecycle.clone())?;
        self.append_audit_event(hub_remove_audit_event(
            source,
            actor,
            &removed,
            deleted_from_workspace,
        ))?;
        self.record_skill_write_action(
            &removed.skill_name,
            None,
            SkillWriteLedgerAction::Remove,
            SkillOperationalSourceScope::Managed,
            None,
            None,
        )?;
        Ok(SkillHubManagedRemoveResponse {
            lifecycle,
            deleted_from_workspace,
            result,
        })
    }

    fn sync_remote_supporting_files(
        &self,
        package: &crate::artifact::SkillArtifactPackage,
    ) -> Result<(), SkillError> {
        let existing = self
            .skill_authority
            .resolve_skill_for_inspection(&package.skill_name, None)
            .ok();
        let source_files = package
            .supporting_files
            .iter()
            .map(|file| (file.relative_path.as_str(), file))
            .collect::<BTreeMap<_, _>>();

        if let Some(existing) = existing.as_ref() {
            for file in &existing.supporting_files {
                if !source_files.contains_key(file.relative_path.as_str()) {
                    self.skill_authority
                        .remove_supporting_file(RemoveSkillFileRequest {
                            name: package.skill_name.clone(),
                            file_path: file.relative_path.clone(),
                        })?;
                }
            }
        }

        for file in &package.supporting_files {
            self.skill_authority
                .write_supporting_file(WriteSkillFileRequest {
                    name: package.skill_name.clone(),
                    file_path: file.relative_path.clone(),
                    content: file.content.clone(),
                })?;
        }
        Ok(())
    }

    fn remote_install_action(
        &self,
        distribution: &SkillDistributionRecord,
    ) -> Result<SkillRemoteInstallAction, SkillError> {
        match self
            .hub_store
            .managed_skill(&distribution.skill_name)
            .filter(|record| {
                record
                    .source
                    .as_ref()
                    .map(|source| source.source_id == distribution.source.source_id)
                    .unwrap_or(false)
            }) {
            Some(_) => Ok(SkillRemoteInstallAction::Update),
            None => {
                if self
                    .skill_authority
                    .discover_skills()
                    .iter()
                    .any(|skill| skill.name.eq_ignore_ascii_case(&distribution.skill_name))
                {
                    return Err(SkillError::InvalidSkillContent {
                        message: format!(
                            "skill `{}` already exists in workspace and is not managed by source `{}`",
                            distribution.skill_name, distribution.source.source_id
                        ),
                    });
                }
                Ok(SkillRemoteInstallAction::Install)
            }
        }
    }

    fn apply_remote_plan(
        &self,
        source: &SkillSourceRef,
        actor: &str,
        plan: SkillRemoteInstallPlan,
    ) -> Result<SkillRemoteInstallResponse, SkillError> {
        let plan_for_apply = plan.clone();
        let artifact_cache =
            self.fetch_distribution_artifact(&plan.distribution.distribution_id, actor)?;
        let apply = (|| -> Result<SkillRemoteInstallResponse, SkillError> {
            let package = self.artifact_store.load_package(&artifact_cache)?;
            if !package
                .skill_name
                .eq_ignore_ascii_case(&plan_for_apply.distribution.skill_name)
            {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "artifact package resolved `{}` but distribution expected `{}`",
                        package.skill_name, plan_for_apply.distribution.skill_name
                    ),
                });
            }

            let duplicate_conflict =
                matches!(
                    plan_for_apply.entry.action,
                    SkillRemoteInstallAction::Install
                ) && self.skill_authority.discover_skills().iter().any(|skill| {
                    skill
                        .name
                        .eq_ignore_ascii_case(&plan_for_apply.distribution.skill_name)
                });
            let current_meta = if matches!(
                plan_for_apply.entry.action,
                SkillRemoteInstallAction::Update
            ) {
                self.skill_authority
                    .resolve_skill_for_inspection(&package.skill_name, None)
                    .ok()
            } else {
                None
            };
            let guard_report = self.apply_guard_report(
                actor,
                Some(source),
                self.evaluate_imported_skill_guard_report(
                    &package.skill_name,
                    &package.markdown_content(),
                    package
                        .supporting_files
                        .iter()
                        .map(|file| (file.relative_path.as_str(), file.content.as_str())),
                    duplicate_conflict,
                    package.category.as_deref().or(current_meta
                        .as_ref()
                        .and_then(|meta| meta.category.as_deref())),
                    current_meta.as_ref().map(|meta| meta.name.as_str()),
                ),
            )?;

            let result = match plan_for_apply.entry.action {
                SkillRemoteInstallAction::Install => {
                    self.skill_authority.create_skill(CreateSkillRequest {
                        name: package.skill_name.clone(),
                        description: package.description.clone(),
                        body: package.body.clone().unwrap_or_else(|| {
                            extract_body_from_markdown(&package.markdown_content())
                        }),
                        frontmatter: None,
                        category: package.category.clone(),
                        directory_name: package.directory_name.clone(),
                    })?
                }
                SkillRemoteInstallAction::Update => {
                    self.skill_authority.edit_skill(EditSkillRequest {
                        name: package.skill_name.clone(),
                        content: package.markdown_content(),
                    })?
                }
            };
            self.sync_remote_supporting_files(&package)?;

            let resolved_meta = self
                .skill_authority
                .resolve_skill_for_inspection(&package.skill_name, None)?;
            let local_hash = crate::sync::hash_skill_meta(&resolved_meta)?;
            let installed_at = now_unix_timestamp();
            let mut distribution = plan_for_apply.distribution.clone();
            distribution.installed = Some(agendao_types::SkillInstalledDistribution {
                installed_at,
                workspace_skill_path: resolved_meta.location.to_string_lossy().to_string(),
                installed_revision: distribution.release.revision.clone(),
                local_hash: Some(local_hash.clone()),
            });
            distribution.lifecycle = SkillManagedLifecycleState::Installed;
            self.upsert_distribution(distribution.clone())?;
            self.upsert_managed_skill(ManagedSkillRecord {
                skill_name: package.skill_name,
                source: Some(source.clone()),
                installed_revision: release_identity(&distribution.release).map(ToOwned::to_owned),
                local_hash: Some(local_hash),
                last_synced_at: Some(installed_at),
                locally_modified: false,
                deleted_locally: false,
            })?;
            self.record_lifecycle(
                Some(actor),
                self.lifecycle.build_record(
                    distribution.distribution_id.clone(),
                    source.source_id.clone(),
                    distribution.skill_name,
                    SkillManagedLifecycleState::Installed,
                    installed_at,
                    None,
                ),
            )?;
            self.append_audit_event(write_audit_event(
                match plan_for_apply.entry.action {
                    SkillRemoteInstallAction::Install => SkillAuditKind::HubInstall,
                    SkillRemoteInstallAction::Update => SkillAuditKind::HubUpdate,
                },
                actor,
                &result,
                Some(source),
            ))?;
            self.record_skill_write_action(
                &result.skill_name,
                None,
                match plan_for_apply.entry.action {
                    SkillRemoteInstallAction::Install => SkillWriteLedgerAction::Install,
                    SkillRemoteInstallAction::Update => SkillWriteLedgerAction::Update,
                },
                SkillOperationalSourceScope::Managed,
                Some(result.location.as_path()),
                result.supporting_file.as_deref(),
            )?;

            Ok(SkillRemoteInstallResponse {
                plan: plan_for_apply.clone(),
                artifact_cache,
                guard_report,
                result: governance_write_result(&result),
            })
        })();

        match apply {
            Ok(response) => Ok(response),
            Err(error) => {
                self.record_lifecycle(
                    Some(actor),
                    self.lifecycle.build_record(
                        plan.distribution.distribution_id.clone(),
                        source.source_id.clone(),
                        plan.distribution.skill_name,
                        SkillManagedLifecycleState::ApplyFailed,
                        now_unix_timestamp(),
                        Some(error.to_string()),
                    ),
                )?;
                Err(error)
            }
        }
    }

    pub(super) fn current_distribution_for_managed_record(
        &self,
        record: &ManagedSkillRecord,
    ) -> Option<SkillDistributionRecord> {
        let source_id = record.source.as_ref()?.source_id.as_str();
        let installed_revision = record.installed_revision.as_deref();
        let mut candidates = self
            .distributions()
            .into_iter()
            .filter(|distribution| {
                distribution.source.source_id == source_id
                    && distribution
                        .skill_name
                        .eq_ignore_ascii_case(&record.skill_name)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.resolution
                .resolved_at
                .cmp(&right.resolution.resolved_at)
                .then_with(|| left.distribution_id.cmp(&right.distribution_id))
        });
        candidates
            .iter()
            .find(|distribution| {
                release_identity(&distribution.release) == installed_revision
                    || distribution
                        .installed
                        .as_ref()
                        .and_then(|installed| installed.installed_revision.as_deref())
                        == installed_revision
            })
            .cloned()
            .or_else(|| candidates.pop())
    }
}

fn governance_write_result(result: &SkillWriteResult) -> SkillGovernanceWriteResult {
    SkillGovernanceWriteResult {
        action: format!("{:?}", result.action).to_ascii_lowercase(),
        skill_name: result.skill_name.clone(),
        location: result.location.to_string_lossy().to_string(),
        supporting_file: result.supporting_file.clone(),
    }
}

fn remote_install_reason(distribution: &SkillDistributionRecord) -> String {
    let release_hint = release_identity(&distribution.release).unwrap_or("unversioned");
    format!("{} via {}", release_hint, distribution.source.source_id)
}

fn remote_update_reason(
    distribution: &SkillDistributionRecord,
    record: &ManagedSkillRecord,
    lifecycle_state: SkillManagedLifecycleState,
) -> String {
    let release_hint = release_identity(&distribution.release).unwrap_or("unversioned");
    match lifecycle_state {
        SkillManagedLifecycleState::Diverged => format!(
            "repair local divergence{} via {}",
            if record.installed_revision.as_deref() != Some(release_hint) {
                format!(
                    " and move {} -> {}",
                    record.installed_revision.as_deref().unwrap_or("unknown"),
                    release_hint
                )
            } else {
                String::new()
            },
            distribution.source.source_id
        ),
        _ => format!(
            "{} -> {} via {}",
            record.installed_revision.as_deref().unwrap_or("unknown"),
            release_hint,
            distribution.source.source_id
        ),
    }
}

pub(super) fn release_identity(release: &agendao_types::SkillDistributionRelease) -> Option<&str> {
    release
        .revision
        .as_deref()
        .or(release.version.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn extract_body_from_markdown(markdown_content: &str) -> String {
    if let Ok(document) = crate::write::parse_skill_document(markdown_content) {
        return document.body;
    }
    markdown_content.trim().to_string()
}

pub(super) fn unresolved_distribution_id(source: &SkillSourceRef, skill_name: &str) -> String {
    format!(
        "dist:{}:{}:unresolved",
        source
            .source_id
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
        skill_name
            .trim()
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
    )
}
