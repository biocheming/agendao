use crate::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, RuntimeInstructionSource, RuntimeSkillBootstrapReport,
    RuntimeSkillMaterialization, RuntimeSkillMaterializationAction, RuntimeSkillSourceKind,
    SkillArtifactStore, SkillAuthority, SkillDistributionResolver, SkillError, SkillGuardEngine,
    SkillHubSnapshot, SkillHubStore, SkillLifecycleCoordinator, SkillSyncPlanner, SkillWriteAction,
    SkillWriteResult, WriteSkillFileRequest,
};
use rocode_config::ConfigStore;
use rocode_types::{
    BundledSkillManifest, ManagedSkillRecord, SkillArtifactCacheEntry, SkillArtifactCacheStatus,
    SkillAuditEvent, SkillAuditKind, SkillDistributionRecord, SkillGovernanceTimelineEntry,
    SkillGovernanceTimelineKind, SkillGovernanceTimelineStatus, SkillGovernanceWriteResult,
    SkillGuardReport, SkillGuardStatus, SkillGuardViolation, SkillHubManagedDetachResponse,
    SkillHubManagedRemoveResponse, SkillHubPolicy, SkillHubTimelineQuery,
    SkillManagedLifecycleRecord, SkillManagedLifecycleState, SkillRemoteInstallAction,
    SkillRemoteInstallEntry, SkillRemoteInstallPlan, SkillRemoteInstallResponse,
    SkillSourceIndexSnapshot, SkillSourceRef, SkillSyncAction, SkillSyncPlan,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernedWriteResult {
    #[serde(flatten)]
    pub result: SkillWriteResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_report: Option<SkillGuardReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernedSyncResult {
    pub plan: SkillSyncPlan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guard_reports: Vec<SkillGuardReport>,
}

pub struct SkillGovernanceAuthority {
    skill_authority: SkillAuthority,
    hub_store: Arc<SkillHubStore>,
    sync_planner: Arc<SkillSyncPlanner>,
    guard_engine: Arc<SkillGuardEngine>,
    distribution_resolver: Arc<SkillDistributionResolver>,
    artifact_store: Arc<SkillArtifactStore>,
    lifecycle: Arc<SkillLifecycleCoordinator>,
}

impl SkillGovernanceAuthority {
    pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self {
        let base_dir = base_dir.into();
        Self {
            skill_authority: SkillAuthority::new(base_dir.clone(), config_store.clone()),
            hub_store: Arc::new(SkillHubStore::new(base_dir.clone())),
            sync_planner: Arc::new(SkillSyncPlanner::new()),
            guard_engine: Arc::new(SkillGuardEngine::new()),
            distribution_resolver: Arc::new(SkillDistributionResolver::new()),
            artifact_store: Arc::new(SkillArtifactStore::new(base_dir.clone(), config_store)),
            lifecycle: Arc::new(SkillLifecycleCoordinator::new()),
        }
    }

    pub fn skill_authority(&self) -> &SkillAuthority {
        &self.skill_authority
    }

    pub fn hub_store(&self) -> Arc<SkillHubStore> {
        Arc::clone(&self.hub_store)
    }

    pub fn sync_planner(&self) -> Arc<SkillSyncPlanner> {
        Arc::clone(&self.sync_planner)
    }

    pub fn guard_engine(&self) -> Arc<SkillGuardEngine> {
        Arc::clone(&self.guard_engine)
    }

    pub fn distribution_resolver(&self) -> Arc<SkillDistributionResolver> {
        Arc::clone(&self.distribution_resolver)
    }

    pub fn artifact_store(&self) -> Arc<SkillArtifactStore> {
        Arc::clone(&self.artifact_store)
    }

    pub fn lifecycle(&self) -> Arc<SkillLifecycleCoordinator> {
        Arc::clone(&self.lifecycle)
    }

    pub fn governance_snapshot(&self) -> SkillHubSnapshot {
        self.hub_store.snapshot()
    }

    pub fn managed_skills(&self) -> Vec<ManagedSkillRecord> {
        self.hub_store.managed_skills()
    }

    pub fn distributions(&self) -> Vec<SkillDistributionRecord> {
        self.hub_store.distributions()
    }

    pub fn artifact_cache(&self) -> Vec<SkillArtifactCacheEntry> {
        self.hub_store.artifact_cache()
    }

    pub fn artifact_policy(&self) -> SkillHubPolicy {
        self.artifact_store.policy()
    }

    pub fn reconcile_artifact_cache_policy(
        &self,
    ) -> Result<Vec<SkillArtifactCacheEntry>, SkillError> {
        let existing = self.hub_store.artifact_cache();
        let retained = self.artifact_store.evict_expired_entries(&existing)?;
        self.hub_store.replace_artifact_cache(retained.clone())?;
        let retained_ids = retained
            .iter()
            .map(|entry| entry.artifact.artifact_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let distributions = self.distributions();
        let policy = self.artifact_policy();
        for entry in existing
            .into_iter()
            .filter(|entry| !retained_ids.contains(entry.artifact.artifact_id.as_str()))
        {
            self.append_audit_event(artifact_evicted_audit_event(
                &entry,
                distributions.iter().find(|record| {
                    record.resolution.artifact.artifact_id == entry.artifact.artifact_id
                }),
                &policy,
            ))?;
        }
        Ok(retained)
    }

    pub fn lifecycle_records(&self) -> Vec<SkillManagedLifecycleRecord> {
        self.hub_store.lifecycle()
    }

    pub fn audit_tail(&self) -> Vec<SkillAuditEvent> {
        self.hub_store.audit_tail()
    }

    pub fn refresh_source_index(
        &self,
        source: &SkillSourceRef,
        actor: &str,
    ) -> Result<SkillSourceIndexSnapshot, SkillError> {
        let snapshot = match source.source_kind {
            rocode_types::SkillSourceKind::Bundled => {
                let manifest =
                    self.hub_store
                        .bundled_manifest()
                        .ok_or_else(|| SkillError::ReadFailed {
                            path: self.hub_store.bundled_manifest_path(),
                            message: "missing bundled manifest for bundled sync source".to_string(),
                        })?;
                let root = self.resolve_source_root(&source.locator);
                let source_snapshot = self
                    .sync_planner
                    .build_bundled_source_snapshot(source, &root, &manifest)?;
                self.sync_planner.source_index_snapshot(&source_snapshot)
            }
            rocode_types::SkillSourceKind::LocalPath => {
                let root = self.resolve_source_root(&source.locator);
                let source_snapshot = self
                    .sync_planner
                    .build_local_source_snapshot(source, &root)?;
                self.sync_planner.source_index_snapshot(&source_snapshot)
            }
            rocode_types::SkillSourceKind::Git
            | rocode_types::SkillSourceKind::Archive
            | rocode_types::SkillSourceKind::Registry => self
                .hub_store
                .upsert_remote_source_index(crate::hub::refresh_remote_source_index(
                    self.hub_store.base_dir(),
                    source,
                    self.artifact_policy().fetch_timeout_ms,
                )?)?,
        };
        if !matches!(
            source.source_kind,
            rocode_types::SkillSourceKind::Git
                | rocode_types::SkillSourceKind::Archive
                | rocode_types::SkillSourceKind::Registry
        ) {
            self.hub_store.upsert_source_index(snapshot.clone())?;
        }
        self.append_audit_event(source_index_refresh_audit_event(source, actor, &snapshot))?;
        Ok(snapshot)
    }

    pub fn governance_timeline(
        &self,
        query: &SkillHubTimelineQuery,
    ) -> Vec<SkillGovernanceTimelineEntry> {
        let normalized_skill_filter = query.skill_name.as_deref().map(normalize_name);
        let source_filter = trimmed_option(query.source_id.as_deref());
        let limit = query.limit.unwrap_or(120).clamp(1, 500);

        let managed_records = self.managed_skills();
        let managed_by_name = managed_records
            .iter()
            .map(|record| (normalize_name(&record.skill_name), record.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut entries = managed_records
            .into_iter()
            .filter(|record| {
                timeline_matches_filters(
                    Some(record.skill_name.as_str()),
                    record
                        .source
                        .as_ref()
                        .map(|source| source.source_id.as_str()),
                    normalized_skill_filter.as_deref(),
                    source_filter.as_deref(),
                )
            })
            .map(managed_record_timeline_entry)
            .collect::<Vec<_>>();

        entries.extend(self.audit_tail().into_iter().filter_map(|event| {
            if !timeline_matches_filters(
                event.skill_name.as_deref(),
                event.source_id.as_deref(),
                normalized_skill_filter.as_deref(),
                source_filter.as_deref(),
            ) {
                return None;
            }
            Some(audit_event_timeline_entry(
                &event,
                event
                    .skill_name
                    .as_deref()
                    .and_then(|name| managed_by_name.get(&normalize_name(name)).cloned()),
            ))
        }));

        entries.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.entry_id.cmp(&right.entry_id))
        });
        entries.truncate(limit);
        entries
    }

    pub fn upsert_managed_skill(
        &self,
        record: ManagedSkillRecord,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_managed_skill(record)
    }

    pub fn replace_source_indices(
        &self,
        source_indices: Vec<SkillSourceIndexSnapshot>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_source_indices(source_indices)
    }

    pub fn replace_distributions(
        &self,
        distributions: Vec<SkillDistributionRecord>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_distributions(distributions)
    }

    pub fn replace_artifact_cache(
        &self,
        artifact_cache: Vec<SkillArtifactCacheEntry>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_artifact_cache(artifact_cache)
    }

    pub fn upsert_distribution(
        &self,
        distribution: SkillDistributionRecord,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_distribution(distribution)
    }

    pub fn upsert_artifact_cache_entry(
        &self,
        entry: SkillArtifactCacheEntry,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_artifact_cache_entry(entry)
    }

    pub fn replace_lifecycle_records(
        &self,
        lifecycle: Vec<SkillManagedLifecycleRecord>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_lifecycle(lifecycle)
    }

    pub fn upsert_lifecycle_record(
        &self,
        record: SkillManagedLifecycleRecord,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_lifecycle_record(record)
    }

    pub fn replace_bundled_manifest(
        &self,
        bundled_manifest: Option<BundledSkillManifest>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_bundled_manifest(bundled_manifest)
    }

    pub fn append_audit_event(&self, event: SkillAuditEvent) -> Result<(), crate::SkillError> {
        self.hub_store.append_audit_event(event)
    }

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
                let record = resolved.record.clone();
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
                skill_name: distribution.skill_name.clone(),
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
        Ok(SkillHubManagedRemoveResponse {
            lifecycle,
            deleted_from_workspace,
            result,
        })
    }

    pub fn refresh_managed_workspace_state(&self) -> Result<Vec<ManagedSkillRecord>, SkillError> {
        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            None,
        )?;
        let records = resolved
            .into_iter()
            .map(|record| record.record)
            .collect::<Vec<_>>();
        self.hub_store.replace_managed_skills(records.clone())?;
        self.update_distribution_runtime_state(
            &self
                .sync_planner
                .refresh_managed_records(&records, &catalog, None)?,
        )?;
        Ok(records)
    }

    pub fn create_skill(
        &self,
        req: CreateSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let duplicate_conflict = self
            .skill_authority
            .discover_skills()
            .iter()
            .any(|skill| skill.name.eq_ignore_ascii_case(req.name.trim()));
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.guard_engine.evaluate_create(
                &req.name,
                &req.description,
                &req.body,
                duplicate_conflict,
                now_unix_timestamp(),
            ),
        )?;
        let result = self.skill_authority.create_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn patch_skill(
        &self,
        req: PatchSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let current = self.skill_authority.resolve_skill(&req.name, None)?;
        let next_name = req.new_name.as_deref().unwrap_or(current.name.as_str());
        let duplicate_conflict = !next_name.eq_ignore_ascii_case(&current.name)
            && self
                .skill_authority
                .discover_skills()
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(next_name));
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.guard_engine.evaluate_patch(
                &current.name,
                next_name,
                req.body.as_deref(),
                duplicate_conflict,
                now_unix_timestamp(),
            ),
        )?;
        let result = self.skill_authority.patch_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn materialize_runtime_skills(
        &self,
        instructions: &[RuntimeInstructionSource],
        actor: &str,
    ) -> Result<RuntimeSkillBootstrapReport, SkillError> {
        let (specs, warnings) = crate::runtime::collect_runtime_skill_specs(
            self.skill_authority.base_dir(),
            instructions,
        );
        self.materialize_runtime_specs(specs, warnings, actor)
    }

    pub fn materialize_runtime_skill_by_name(
        &self,
        skill_name: &str,
        instructions: &[RuntimeInstructionSource],
        actor: &str,
    ) -> Result<RuntimeSkillBootstrapReport, SkillError> {
        let (specs, warnings) = crate::runtime::collect_runtime_skill_specs(
            self.skill_authority.base_dir(),
            instructions,
        );
        let filtered = specs
            .into_iter()
            .filter(|spec| spec.name.eq_ignore_ascii_case(skill_name))
            .collect::<Vec<_>>();
        self.materialize_runtime_specs(filtered, warnings, actor)
    }

    fn materialize_runtime_specs(
        &self,
        specs: Vec<crate::runtime::RuntimeSkillSpec>,
        warnings: Vec<String>,
        actor: &str,
    ) -> Result<RuntimeSkillBootstrapReport, SkillError> {
        let mut report = RuntimeSkillBootstrapReport {
            materializations: Vec::new(),
            imported_legacy_sources: specs
                .iter()
                .filter(|spec| matches!(spec.source_kind, RuntimeSkillSourceKind::LegacyMarkdown))
                .filter_map(|spec| spec.source_path.clone())
                .collect(),
            warnings,
        };

        for spec in specs {
            let existing = self.skill_authority.resolve_skill(&spec.name, None);
            match existing {
                Ok(meta) => {
                    if !self.skill_authority.is_skill_meta_writable(&meta) {
                        report.materializations.push(RuntimeSkillMaterialization {
                            skill_name: spec.name.clone(),
                            action: RuntimeSkillMaterializationAction::Skipped,
                            source_kind: spec.source_kind,
                            source_path: spec.source_path.clone(),
                            detail: Some(format!(
                                "existing skill is outside the workspace sandbox: {}",
                                meta.location.display()
                            )),
                        });
                        continue;
                    }

                    let loaded = self.skill_authority.load_skill(&spec.name, None)?;
                    let description_matches = meta.description.trim() == spec.description.trim();
                    let body_matches = loaded.content.trim() == spec.body.trim();
                    if description_matches && body_matches {
                        report.materializations.push(RuntimeSkillMaterialization {
                            skill_name: spec.name.clone(),
                            action: RuntimeSkillMaterializationAction::Unchanged,
                            source_kind: spec.source_kind,
                            source_path: spec.source_path.clone(),
                            detail: None,
                        });
                        continue;
                    }

                    let content = crate::write::build_skill_document(
                        &crate::write::build_create_frontmatter(
                            &spec.name,
                            &spec.description,
                            None,
                        )?,
                        &spec.body,
                    )?;
                    let _ = self.edit_skill(
                        EditSkillRequest {
                            name: spec.name.clone(),
                            content,
                        },
                        actor,
                    )?;
                    report.materializations.push(RuntimeSkillMaterialization {
                        skill_name: spec.name.clone(),
                        action: RuntimeSkillMaterializationAction::Refreshed,
                        source_kind: spec.source_kind,
                        source_path: spec.source_path.clone(),
                        detail: None,
                    });
                }
                Err(SkillError::UnknownSkill { .. }) => {
                    let _ = self.create_skill(
                        CreateSkillRequest {
                            name: spec.name.clone(),
                            description: spec.description.clone(),
                            body: spec.body.clone(),
                            frontmatter: None,
                            category: None,
                            directory_name: None,
                        },
                        actor,
                    )?;
                    report.materializations.push(RuntimeSkillMaterialization {
                        skill_name: spec.name.clone(),
                        action: RuntimeSkillMaterializationAction::Created,
                        source_kind: spec.source_kind,
                        source_path: spec.source_path.clone(),
                        detail: None,
                    });
                }
                Err(error) => return Err(error),
            }
        }

        Ok(report)
    }

    pub fn edit_skill(
        &self,
        req: EditSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let current = self.skill_authority.resolve_skill(&req.name, None)?;
        let next_name = crate::write::parse_skill_document(&req.content)
            .ok()
            .and_then(|document| {
                crate::write::read_frontmatter_value(&document.frontmatter_lines, "name")
            })
            .unwrap_or_else(|| current.name.clone());
        let duplicate_conflict = !next_name.eq_ignore_ascii_case(&current.name)
            && self
                .skill_authority
                .discover_skills()
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(&next_name));
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.guard_engine.evaluate_edit(
                &next_name,
                &req.content,
                duplicate_conflict,
                now_unix_timestamp(),
            ),
        )?;
        let result = self.skill_authority.edit_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn write_supporting_file(
        &self,
        req: WriteSkillFileRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.guard_engine.evaluate_supporting_file(
                &req.name,
                &req.file_path,
                &req.content,
                now_unix_timestamp(),
            ),
        )?;
        let result = self.skill_authority.write_supporting_file(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn remove_supporting_file(
        &self,
        req: RemoveSkillFileRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let result = self.skill_authority.remove_supporting_file(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report: None,
        })
    }

    pub fn delete_skill(
        &self,
        req: DeleteSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let result = self.skill_authority.delete_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report: None,
        })
    }

    pub fn plan_sync(&self, source: &SkillSourceRef) -> Result<SkillSyncPlan, SkillError> {
        let source_snapshot = self.build_source_snapshot(source)?;
        self.hub_store
            .upsert_source_index(self.sync_planner.source_index_snapshot(&source_snapshot))?;

        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            Some(&source_snapshot),
        )?;
        self.hub_store.replace_managed_skills(
            resolved
                .iter()
                .map(|record| record.record.clone())
                .collect::<Vec<_>>(),
        )?;

        let plan = self
            .sync_planner
            .plan_sync(&source_snapshot, &resolved, &catalog);
        self.append_audit_event(sync_audit_event(
            SkillAuditKind::SyncPlanCreated,
            source,
            "authority:skill_sync_plan",
            &plan,
        ))?;
        Ok(plan)
    }

    pub fn run_guard_for_skill(
        &self,
        skill_name: &str,
        actor: &str,
    ) -> Result<Vec<SkillGuardReport>, SkillError> {
        let meta = self.skill_authority.resolve_skill(skill_name, None)?;
        let markdown_content = self.skill_authority.load_skill_source(skill_name, None)?;
        let supporting_files = meta
            .supporting_files
            .iter()
            .map(|file| {
                let content = std::fs::read_to_string(&file.location).map_err(|error| {
                    SkillError::ReadFailed {
                        path: file.location.clone(),
                        message: error.to_string(),
                    }
                })?;
                Ok((file.relative_path.clone(), content))
            })
            .collect::<Result<Vec<_>, SkillError>>()?;

        let report = self.guard_engine.evaluate_imported_skill(
            &meta.name,
            &markdown_content,
            &supporting_files,
            false,
            now_unix_timestamp(),
        );
        self.audit_guard_observation(actor, None, &report)?;
        Ok(vec![report])
    }

    pub fn run_guard_for_source(
        &self,
        source: &SkillSourceRef,
        actor: &str,
    ) -> Result<Vec<SkillGuardReport>, SkillError> {
        let source_snapshot = self.build_source_snapshot(source)?;
        self.hub_store
            .upsert_source_index(self.sync_planner.source_index_snapshot(&source_snapshot))?;

        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            Some(&source_snapshot),
        )?;
        let catalog_by_name = catalog
            .iter()
            .map(|meta| (normalize_name(&meta.name), meta))
            .collect::<BTreeMap<_, _>>();
        let managed_by_name = resolved
            .iter()
            .filter(|record| {
                record
                    .record
                    .source
                    .as_ref()
                    .map(|managed_source| managed_source.source_id == source.source_id)
                    .unwrap_or(false)
            })
            .map(|record| (normalize_name(&record.record.skill_name), record))
            .collect::<BTreeMap<_, _>>();

        let mut reports = Vec::new();
        for entry in &source_snapshot.entries {
            let normalized_name = normalize_name(&entry.skill_name);
            let duplicate_conflict = catalog_by_name.contains_key(&normalized_name)
                && !managed_by_name.contains_key(&normalized_name);
            let report = self.guard_engine.evaluate_imported_skill(
                &entry.skill_name,
                &entry.markdown_content,
                &entry
                    .supporting_files
                    .iter()
                    .map(|file| (file.relative_path.clone(), file.content.clone()))
                    .collect::<Vec<_>>(),
                duplicate_conflict,
                now_unix_timestamp(),
            );
            self.audit_guard_observation(actor, Some(source), &report)?;
            reports.push(report);
        }
        Ok(reports)
    }

    pub fn apply_sync(
        &self,
        source: &SkillSourceRef,
        actor: &str,
    ) -> Result<SkillGovernedSyncResult, SkillError> {
        let source_snapshot = self.build_source_snapshot(source)?;
        self.hub_store
            .upsert_source_index(self.sync_planner.source_index_snapshot(&source_snapshot))?;

        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            Some(&source_snapshot),
        )?;
        let plan = self
            .sync_planner
            .plan_sync(&source_snapshot, &resolved, &catalog);

        let source_entries = source_snapshot
            .entries
            .iter()
            .map(|entry| (normalize_name(&entry.skill_name), entry))
            .collect::<BTreeMap<_, _>>();
        let resolved_managed = resolved
            .iter()
            .map(|record| (normalize_name(&record.record.skill_name), record))
            .collect::<BTreeMap<_, _>>();
        let catalog_by_name = catalog
            .iter()
            .map(|meta| (normalize_name(&meta.name), meta))
            .collect::<BTreeMap<_, _>>();
        let mut guard_reports = Vec::new();

        for plan_entry in &plan.entries {
            let normalized_name = normalize_name(&plan_entry.skill_name);
            let source_entry = source_entries.get(&normalized_name).copied();
            let managed_record = resolved_managed.get(&normalized_name).copied();
            let catalog_entry = catalog_by_name.get(&normalized_name).copied();

            match plan_entry.action {
                SkillSyncAction::Install => {
                    let source_entry =
                        source_entry.ok_or_else(|| SkillError::InvalidSkillContent {
                            message: format!(
                                "sync plan for `{}` was missing source content",
                                plan_entry.skill_name
                            ),
                        })?;
                    if let Some(report) =
                        self.apply_import_guard(actor, source, source_entry, false)?
                    {
                        guard_reports.push(report);
                    }
                    let result = self.install_skill_from_source(source_entry)?;
                    self.append_audit_event(write_audit_event(
                        SkillAuditKind::HubInstall,
                        actor,
                        &result,
                        Some(source),
                    ))?;
                    self.hub_store
                        .upsert_managed_skill(self.synced_managed_record(source, source_entry)?)?;
                }
                SkillSyncAction::Update => {
                    let source_entry =
                        source_entry.ok_or_else(|| SkillError::InvalidSkillContent {
                            message: format!(
                                "sync plan for `{}` was missing source content",
                                plan_entry.skill_name
                            ),
                        })?;
                    if let Some(report) =
                        self.apply_import_guard(actor, source, source_entry, false)?
                    {
                        guard_reports.push(report);
                    }
                    let result = self.update_skill_from_source(source_entry, catalog_entry)?;
                    self.append_audit_event(write_audit_event(
                        SkillAuditKind::HubUpdate,
                        actor,
                        &result,
                        Some(source),
                    ))?;
                    self.hub_store
                        .upsert_managed_skill(self.synced_managed_record(source, source_entry)?)?;
                }
                SkillSyncAction::SkipLocalModification => {
                    if let Some(managed_record) = managed_record {
                        let mut next_record = managed_record.record.clone();
                        next_record.locally_modified = true;
                        next_record.deleted_locally = false;
                        self.hub_store.upsert_managed_skill(next_record)?;
                    }
                }
                SkillSyncAction::SkipDeletedLocally => {
                    if let Some(managed_record) = managed_record {
                        let mut next_record = managed_record.record.clone();
                        next_record.deleted_locally = true;
                        next_record.locally_modified = false;
                        self.hub_store.upsert_managed_skill(next_record)?;
                    }
                }
                SkillSyncAction::RemoveManaged => {
                    if let Some(managed_record) = managed_record {
                        let mut deleted_from_workspace = false;
                        if let Some(current_hash) = managed_record.current_hash.as_deref() {
                            if managed_record.record.local_hash.as_deref() == Some(current_hash) {
                                self.skill_authority.delete_skill(DeleteSkillRequest {
                                    name: managed_record.record.skill_name.clone(),
                                })?;
                                deleted_from_workspace = true;
                            }
                        }
                        self.hub_store
                            .remove_managed_skill(&managed_record.record.skill_name)?;
                        self.append_audit_event(hub_remove_audit_event(
                            source,
                            actor,
                            &managed_record.record,
                            deleted_from_workspace,
                        ))?;
                    }
                }
                SkillSyncAction::Noop => {
                    if let (Some(_managed_record), Some(source_entry)) =
                        (managed_record, source_entry)
                    {
                        self.hub_store.upsert_managed_skill(
                            self.synced_managed_record(source, source_entry)?,
                        )?;
                    }
                }
            }
        }

        self.refresh_managed_workspace_state()?;
        self.append_audit_event(sync_audit_event(
            SkillAuditKind::SyncApplyCompleted,
            source,
            actor,
            &plan,
        ))?;
        Ok(SkillGovernedSyncResult {
            plan,
            guard_reports,
        })
    }

    fn build_source_snapshot(
        &self,
        source: &SkillSourceRef,
    ) -> Result<crate::sync::SkillSyncSourceSnapshot, SkillError> {
        if !crate::sync::source_root_kind_supported(source) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "unsupported skill source kind for sync: {:?}",
                    source.source_kind
                ),
            });
        }

        let root = self.resolve_source_root(&source.locator);
        if !root.exists() {
            return Err(SkillError::ReadFailed {
                path: root,
                message: "sync source root does not exist".to_string(),
            });
        }

        match source.source_kind {
            rocode_types::SkillSourceKind::Bundled => {
                let manifest =
                    self.hub_store
                        .bundled_manifest()
                        .ok_or_else(|| SkillError::ReadFailed {
                            path: self.hub_store.bundled_manifest_path(),
                            message: "missing bundled manifest for bundled sync source".to_string(),
                        })?;
                self.sync_planner
                    .build_bundled_source_snapshot(source, &root, &manifest)
            }
            rocode_types::SkillSourceKind::LocalPath => {
                self.sync_planner.build_local_source_snapshot(source, &root)
            }
            _ => Err(SkillError::InvalidSkillContent {
                message: format!(
                    "unsupported skill source kind for sync: {:?}",
                    source.source_kind
                ),
            }),
        }
    }

    fn resolve_source_root(&self, locator: &str) -> PathBuf {
        let path = PathBuf::from(locator);
        if path.is_absolute() {
            path
        } else {
            self.hub_store.base_dir().join(path)
        }
    }

    fn apply_guard_report(
        &self,
        actor: &str,
        source: Option<&SkillSourceRef>,
        report: SkillGuardReport,
    ) -> Result<Option<SkillGuardReport>, SkillError> {
        if report.violations.is_empty() {
            return Ok(None);
        }

        self.audit_guard_observation(actor, source, &report)?;
        let blocked = report.status == SkillGuardStatus::Blocked;
        if blocked {
            return Err(SkillError::GuardBlocked { report });
        }
        Ok(Some(report))
    }

    fn apply_import_guard(
        &self,
        actor: &str,
        source: &SkillSourceRef,
        entry: &crate::sync::SkillSyncSourceEntry,
        duplicate_conflict: bool,
    ) -> Result<Option<SkillGuardReport>, SkillError> {
        let report = self.guard_engine.evaluate_imported_skill(
            &entry.skill_name,
            &entry.markdown_content,
            &entry
                .supporting_files
                .iter()
                .map(|file| (file.relative_path.clone(), file.content.clone()))
                .collect::<Vec<_>>(),
            duplicate_conflict,
            now_unix_timestamp(),
        );
        self.apply_guard_report(actor, Some(source), report)
    }

    fn audit_guard_observation(
        &self,
        actor: &str,
        source: Option<&SkillSourceRef>,
        report: &SkillGuardReport,
    ) -> Result<(), SkillError> {
        if report.violations.is_empty() {
            return Ok(());
        }
        self.append_audit_event(guard_audit_event(
            if report.status == SkillGuardStatus::Blocked {
                SkillAuditKind::GuardBlocked
            } else {
                SkillAuditKind::GuardWarned
            },
            source,
            actor,
            report,
        ))
    }

    fn install_skill_from_source(
        &self,
        entry: &crate::sync::SkillSyncSourceEntry,
    ) -> Result<SkillWriteResult, SkillError> {
        let (category, directory_name) = create_target_from_relative_path(&entry.relative_path)?;
        let result = self.skill_authority.create_skill(CreateSkillRequest {
            name: entry.skill_name.clone(),
            description: entry.description.clone(),
            body: entry.body.clone(),
            frontmatter: None,
            category,
            directory_name,
        })?;
        self.sync_supporting_files(entry, None)?;
        Ok(result)
    }

    fn update_skill_from_source(
        &self,
        entry: &crate::sync::SkillSyncSourceEntry,
        existing: Option<&crate::SkillMeta>,
    ) -> Result<SkillWriteResult, SkillError> {
        let result = self.skill_authority.edit_skill(EditSkillRequest {
            name: entry.skill_name.clone(),
            content: entry.markdown_content.clone(),
        })?;
        self.sync_supporting_files(entry, existing)?;
        Ok(result)
    }

    fn sync_supporting_files(
        &self,
        entry: &crate::sync::SkillSyncSourceEntry,
        existing: Option<&crate::SkillMeta>,
    ) -> Result<(), SkillError> {
        let source_files = entry
            .supporting_files
            .iter()
            .map(|file| (file.relative_path.as_str(), file))
            .collect::<BTreeMap<_, _>>();

        if let Some(existing) = existing {
            for file in &existing.supporting_files {
                if !source_files.contains_key(file.relative_path.as_str()) {
                    self.skill_authority
                        .remove_supporting_file(RemoveSkillFileRequest {
                            name: entry.skill_name.clone(),
                            file_path: file.relative_path.clone(),
                        })?;
                }
            }
        }

        for source_file in &entry.supporting_files {
            self.skill_authority
                .write_supporting_file(WriteSkillFileRequest {
                    name: entry.skill_name.clone(),
                    file_path: source_file.relative_path.clone(),
                    content: source_file.content.clone(),
                })?;
        }
        Ok(())
    }

    fn sync_remote_supporting_files(
        &self,
        package: &crate::artifact::SkillArtifactPackage,
    ) -> Result<(), SkillError> {
        let existing = self
            .skill_authority
            .resolve_skill(&package.skill_name, None)
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
            let guard_report = self.apply_guard_report(
                actor,
                Some(source),
                self.guard_engine.evaluate_imported_skill(
                    &package.skill_name,
                    &package.markdown_content(),
                    &package
                        .supporting_files
                        .iter()
                        .map(|file| (file.relative_path.clone(), file.content.clone()))
                        .collect::<Vec<_>>(),
                    duplicate_conflict,
                    now_unix_timestamp(),
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
                .resolve_skill(&package.skill_name, None)?;
            let local_hash = crate::sync::hash_skill_meta(&resolved_meta)?;
            let installed_at = now_unix_timestamp();
            let mut distribution = plan_for_apply.distribution.clone();
            distribution.installed = Some(rocode_types::SkillInstalledDistribution {
                installed_at,
                workspace_skill_path: resolved_meta.location.to_string_lossy().to_string(),
                installed_revision: distribution.release.revision.clone(),
                local_hash: Some(local_hash.clone()),
            });
            distribution.lifecycle = SkillManagedLifecycleState::Installed;
            self.upsert_distribution(distribution.clone())?;
            self.upsert_managed_skill(ManagedSkillRecord {
                skill_name: package.skill_name.clone(),
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
                    distribution.skill_name.clone(),
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
                        plan.distribution.skill_name.clone(),
                        SkillManagedLifecycleState::ApplyFailed,
                        now_unix_timestamp(),
                        Some(error.to_string()),
                    ),
                )?;
                Err(error)
            }
        }
    }

    fn record_lifecycle(
        &self,
        actor: Option<&str>,
        record: SkillManagedLifecycleRecord,
    ) -> Result<(), SkillError> {
        let previous = self
            .lifecycle_records()
            .into_iter()
            .find(|entry| entry.distribution_id == record.distribution_id);
        let changed = previous
            .as_ref()
            .map(|entry| entry.state != record.state || entry.error != record.error)
            .unwrap_or(true);
        self.upsert_lifecycle_record(record.clone())?;
        if changed {
            if let Some(actor) = actor {
                self.append_audit_event(lifecycle_transition_audit_event(
                    actor,
                    previous.as_ref(),
                    &record,
                ))?;
            }
        }
        Ok(())
    }

    fn refresh_managed_record_for_source_skill(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
    ) -> Result<crate::sync::ResolvedManagedSkillRecord, SkillError> {
        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            None,
        )?;
        let records = resolved
            .iter()
            .map(|record| record.record.clone())
            .collect::<Vec<_>>();
        self.hub_store.replace_managed_skills(records)?;
        self.update_distribution_runtime_state(&resolved)?;
        resolved
            .into_iter()
            .find(|record| {
                record.record.skill_name.eq_ignore_ascii_case(skill_name)
                    && record
                        .record
                        .source
                        .as_ref()
                        .map(|managed_source| managed_source.source_id == source.source_id)
                        .unwrap_or(false)
            })
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not managed by source `{}`",
                    skill_name.trim(),
                    source.source_id
                ),
            })
    }

    fn update_distribution_runtime_state(
        &self,
        managed_records: &[crate::sync::ResolvedManagedSkillRecord],
    ) -> Result<(), SkillError> {
        let mut distributions = self.distributions();
        let mut touched = BTreeMap::<String, SkillManagedLifecycleRecord>::new();
        for distribution in &mut distributions {
            let Some(managed_record) = managed_records.iter().find(|record| {
                record
                    .record
                    .skill_name
                    .eq_ignore_ascii_case(&distribution.skill_name)
                    && record
                        .record
                        .source
                        .as_ref()
                        .map(|source| source.source_id == distribution.source.source_id)
                        .unwrap_or(false)
            }) else {
                continue;
            };

            let next_state = self.lifecycle.managed_runtime_state(
                &managed_record.record,
                release_identity(&distribution.release),
            );
            distribution.lifecycle = next_state.clone();
            touched.insert(
                distribution.distribution_id.clone(),
                self.lifecycle.build_record(
                    distribution.distribution_id.clone(),
                    distribution.source.source_id.clone(),
                    distribution.skill_name.clone(),
                    next_state,
                    now_unix_timestamp(),
                    None,
                ),
            );
        }

        for distribution in distributions {
            self.upsert_distribution(distribution)?;
        }
        for record in managed_records {
            let distribution_id = self
                .current_distribution_for_managed_record(&record.record)
                .map(|distribution| distribution.distribution_id)
                .unwrap_or_else(|| {
                    unresolved_distribution_id(
                        record
                            .record
                            .source
                            .as_ref()
                            .expect("managed record source must exist"),
                        &record.record.skill_name,
                    )
                });
            touched.entry(distribution_id.clone()).or_insert_with(|| {
                self.lifecycle.build_record(
                    distribution_id,
                    record
                        .record
                        .source
                        .as_ref()
                        .expect("managed record source must exist")
                        .source_id
                        .clone(),
                    record.record.skill_name.clone(),
                    self.lifecycle.managed_runtime_state(&record.record, None),
                    now_unix_timestamp(),
                    None,
                )
            });
        }
        for lifecycle in touched.into_values() {
            self.upsert_lifecycle_record(lifecycle)?;
        }
        Ok(())
    }

    fn current_distribution_for_managed_record(
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

    fn synced_managed_record(
        &self,
        source: &SkillSourceRef,
        entry: &crate::sync::SkillSyncSourceEntry,
    ) -> Result<ManagedSkillRecord, SkillError> {
        let meta = self
            .skill_authority
            .resolve_skill(&entry.skill_name, None)?;
        let local_hash = crate::sync::hash_skill_meta(&meta)?;
        Ok(ManagedSkillRecord {
            skill_name: entry.skill_name.clone(),
            source: Some(source.clone()),
            installed_revision: entry
                .revision
                .clone()
                .or_else(|| Some(entry.content_hash.clone())),
            local_hash: Some(local_hash),
            last_synced_at: Some(now_unix_timestamp()),
            locally_modified: false,
            deleted_locally: false,
        })
    }
}

fn audit_kind_for_write_action(action: &SkillWriteAction) -> SkillAuditKind {
    match action {
        SkillWriteAction::Created => SkillAuditKind::Create,
        SkillWriteAction::Patched => SkillAuditKind::Patch,
        SkillWriteAction::Edited => SkillAuditKind::Edit,
        SkillWriteAction::SupportingFileWritten => SkillAuditKind::WriteFile,
        SkillWriteAction::SupportingFileRemoved => SkillAuditKind::RemoveFile,
        SkillWriteAction::Deleted => SkillAuditKind::Delete,
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

fn release_identity(release: &rocode_types::SkillDistributionRelease) -> Option<&str> {
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

fn source_index_refresh_audit_event(
    source: &SkillSourceRef,
    actor: &str,
    snapshot: &SkillSourceIndexSnapshot,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-index-refresh-{}-{}",
            created_at,
            source
                .source_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::SourceIndexRefreshed,
        skill_name: None,
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "source_kind": format!("{:?}", source.source_kind).to_ascii_lowercase(),
            "locator": source.locator.clone(),
            "revision": source.revision.clone(),
            "entry_count": snapshot.entries.len(),
            "updated_at": snapshot.updated_at,
        }),
    }
}

fn remote_plan_audit_event(
    kind: SkillAuditKind,
    actor: &str,
    plan: &SkillRemoteInstallPlan,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-remote-plan-{}-{}",
            created_at,
            plan.distribution
                .distribution_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(plan.entry.skill_name.clone()),
        source_id: Some(plan.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "distribution_id": plan.distribution.distribution_id,
            "artifact_id": plan.distribution.resolution.artifact.artifact_id,
            "artifact_locator": plan.distribution.resolution.artifact.locator,
            "revision": plan.distribution.release.revision,
            "version": plan.distribution.release.version,
            "action": format!("{:?}", plan.entry.action).to_ascii_lowercase(),
            "reason": plan.entry.reason,
        }),
    }
}

fn artifact_evicted_audit_event(
    entry: &SkillArtifactCacheEntry,
    distribution: Option<&SkillDistributionRecord>,
    policy: &SkillHubPolicy,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-artifact-evicted-{}-{}",
            created_at,
            entry
                .artifact
                .artifact_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::ArtifactEvicted,
        skill_name: distribution.map(|record| record.skill_name.clone()),
        source_id: distribution.map(|record| record.source.source_id.clone()),
        actor: "authority:artifact_cache_policy".to_string(),
        created_at,
        payload: json!({
            "artifact_id": entry.artifact.artifact_id,
            "artifact_locator": entry.artifact.locator,
            "cached_at": entry.cached_at,
            "local_path": entry.local_path,
            "extracted_path": entry.extracted_path,
            "previous_status": format!("{:?}", entry.status).to_ascii_lowercase(),
            "retention_seconds": policy.artifact_cache_retention_seconds,
            "reason": "retention_expired",
        }),
    }
}

fn lifecycle_transition_audit_event(
    actor: &str,
    previous: Option<&SkillManagedLifecycleRecord>,
    current: &SkillManagedLifecycleRecord,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-lifecycle-{}-{}",
            created_at,
            current
                .distribution_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::LifecycleTransitioned,
        skill_name: Some(current.skill_name.clone()),
        source_id: Some(current.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "distribution_id": current.distribution_id,
            "from_state": previous.map(|entry| format!("{:?}", entry.state).to_ascii_lowercase()),
            "to_state": format!("{:?}", current.state).to_ascii_lowercase(),
            "error": current.error,
        }),
    }
}

fn write_audit_event(
    kind: SkillAuditKind,
    actor: &str,
    result: &SkillWriteResult,
    source: Option<&SkillSourceRef>,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-write-{}-{}",
            created_at,
            result
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(result.skill_name.clone()),
        source_id: source.map(|source| source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "action": format!("{:?}", result.action).to_ascii_lowercase(),
            "location": result.location.to_string_lossy().to_string(),
            "supporting_file": result.supporting_file,
            "category": result.skill.as_ref().and_then(|skill| skill.category.clone()),
        }),
    }
}

fn guard_audit_event(
    kind: SkillAuditKind,
    source: Option<&SkillSourceRef>,
    actor: &str,
    report: &SkillGuardReport,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-guard-{}-{}",
            created_at,
            report
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(report.skill_name.clone()),
        source_id: source.map(|source| source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "status": format!("{:?}", report.status).to_ascii_lowercase(),
            "violation_count": report.violations.len(),
            "violations": report.violations,
        }),
    }
}

fn hub_remove_audit_event(
    source: &SkillSourceRef,
    actor: &str,
    record: &ManagedSkillRecord,
    deleted_from_workspace: bool,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-hub-remove-{}-{}",
            created_at,
            record
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::HubRemove,
        skill_name: Some(record.skill_name.clone()),
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "deleted_from_workspace": deleted_from_workspace,
            "installed_revision": record.installed_revision,
            "local_hash": record.local_hash,
        }),
    }
}

fn hub_detach_audit_event(
    source: &SkillSourceRef,
    actor: &str,
    record: &ManagedSkillRecord,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-hub-detach-{}-{}",
            created_at,
            record
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::HubDetach,
        skill_name: Some(record.skill_name.clone()),
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "installed_revision": record.installed_revision,
            "local_hash": record.local_hash,
            "locally_modified": record.locally_modified,
            "deleted_locally": record.deleted_locally,
        }),
    }
}

fn sync_audit_event(
    kind: SkillAuditKind,
    source: &SkillSourceRef,
    actor: &str,
    plan: &SkillSyncPlan,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-sync-{}-{}",
            created_at,
            source
                .source_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: None,
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "source_kind": format!("{:?}", source.source_kind).to_ascii_lowercase(),
            "entry_count": plan.entries.len(),
            "entries": plan.entries.iter().map(|entry| {
                json!({
                    "skill_name": entry.skill_name,
                    "action": format!("{:?}", entry.action).to_ascii_lowercase(),
                    "reason": entry.reason,
                })
            }).collect::<Vec<_>>(),
        }),
    }
}

fn distribution_audit_event(
    kind: SkillAuditKind,
    actor: &str,
    distribution: &SkillDistributionRecord,
    error: Option<String>,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-distribution-{}-{}",
            created_at,
            distribution
                .distribution_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(distribution.skill_name.clone()),
        source_id: Some(distribution.source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "distribution_id": distribution.distribution_id,
            "source_kind": format!("{:?}", distribution.source.source_kind).to_ascii_lowercase(),
            "version": distribution.release.version,
            "revision": distribution.release.revision,
            "artifact_id": distribution.resolution.artifact.artifact_id,
            "artifact_locator": distribution.resolution.artifact.locator,
            "error": error,
        }),
    }
}

fn unresolved_distribution_id(source: &SkillSourceRef, skill_name: &str) -> String {
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

fn create_target_from_relative_path(
    relative_path: &str,
) -> Result<(Option<String>, Option<String>), SkillError> {
    let parent =
        Path::new(relative_path)
            .parent()
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!("invalid source skill path `{relative_path}`"),
            })?;
    let components = parent
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let directory_name = components.last().cloned();
    let category = if components.len() > 1 {
        Some(components[..components.len() - 1].join("/"))
    } else {
        None
    };
    Ok((category, directory_name))
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn timeline_matches_filters(
    skill_name: Option<&str>,
    source_id: Option<&str>,
    skill_filter: Option<&str>,
    source_filter: Option<&str>,
) -> bool {
    if let Some(skill_filter) = skill_filter {
        if skill_name.map(normalize_name).as_deref() != Some(skill_filter) {
            return false;
        }
    }
    if let Some(source_filter) = source_filter {
        if source_id.map(str::trim) != Some(source_filter) {
            return false;
        }
    }
    true
}

fn managed_record_timeline_entry(record: ManagedSkillRecord) -> SkillGovernanceTimelineEntry {
    let status = if record.deleted_locally || record.locally_modified {
        SkillGovernanceTimelineStatus::Warn
    } else {
        SkillGovernanceTimelineStatus::Success
    };
    let summary = if let Some(source) = record.source.as_ref() {
        format!(
            "{} · revision {} · {}",
            source.source_id,
            record.installed_revision.as_deref().unwrap_or("--"),
            managed_record_state_label(&record)
        )
    } else {
        format!("workspace-local · {}", managed_record_state_label(&record))
    };

    SkillGovernanceTimelineEntry {
        entry_id: format!(
            "managed-{}",
            record
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillGovernanceTimelineKind::ManagedSnapshot,
        created_at: record.last_synced_at.unwrap_or_default(),
        skill_name: Some(record.skill_name.clone()),
        source_id: record
            .source
            .as_ref()
            .map(|source| source.source_id.clone()),
        actor: None,
        title: format!("Managed provenance · {}", record.skill_name),
        summary,
        status,
        managed_record: Some(record.clone()),
        guard_report: None,
        payload: json!({
            "installed_revision": record.installed_revision,
            "local_hash": record.local_hash,
            "last_synced_at": record.last_synced_at,
            "locally_modified": record.locally_modified,
            "deleted_locally": record.deleted_locally,
        }),
    }
}

fn managed_record_state_label(record: &ManagedSkillRecord) -> &'static str {
    if record.deleted_locally {
        "deleted locally"
    } else if record.locally_modified {
        "locally modified"
    } else {
        "clean"
    }
}

fn audit_event_timeline_entry(
    event: &SkillAuditEvent,
    managed_record: Option<ManagedSkillRecord>,
) -> SkillGovernanceTimelineEntry {
    let guard_report = guard_report_from_audit_event(event);
    SkillGovernanceTimelineEntry {
        entry_id: event.event_id.clone(),
        kind: event.kind.clone().into(),
        created_at: event.created_at,
        skill_name: event.skill_name.clone(),
        source_id: event.source_id.clone(),
        actor: Some(event.actor.clone()),
        title: audit_event_title(event),
        summary: audit_event_summary(event),
        status: audit_event_status(&event.kind),
        managed_record,
        guard_report,
        payload: event.payload.clone(),
    }
}

fn audit_event_title(event: &SkillAuditEvent) -> String {
    match event.kind {
        SkillAuditKind::SourceIndexRefreshed => format!(
            "Source index refreshed · {}",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::SourceResolved => format!(
            "Source resolved · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::ArtifactFetched => format!(
            "Artifact fetched · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::ArtifactEvicted => format!(
            "Artifact evicted · {}",
            event
                .skill_name
                .as_deref()
                .or(event.source_id.as_deref())
                .unwrap_or("artifact")
        ),
        SkillAuditKind::ArtifactFetchFailed => format!(
            "Artifact fetch failed · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::RemoteInstallPlanned => format!(
            "Remote install planned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::RemoteUpdatePlanned => format!(
            "Remote update planned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::LifecycleTransitioned => format!(
            "Lifecycle transitioned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Create => format!(
            "Workspace create · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Patch => format!(
            "Workspace patch · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Edit => format!(
            "Workspace edit · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Delete => format!(
            "Workspace delete · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::WriteFile => format!(
            "Supporting file write · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::RemoveFile => format!(
            "Supporting file remove · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubInstall => format!(
            "Hub install · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubUpdate => format!(
            "Hub update · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubDetach => format!(
            "Hub detach · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubRemove => format!(
            "Hub remove · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::SyncPlanCreated => format!(
            "Sync plan created · {}",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::SyncApplyCompleted => format!(
            "Sync apply completed · {}",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::GuardBlocked => format!(
            "Guard blocked · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::GuardWarned => format!(
            "Guard warned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
    }
}

fn audit_event_summary(event: &SkillAuditEvent) -> String {
    match event.kind {
        SkillAuditKind::SourceIndexRefreshed => {
            let entry_count = payload_usize(&event.payload, "entry_count").unwrap_or_default();
            let source_kind =
                payload_string(&event.payload, "source_kind").unwrap_or_else(|| "source".into());
            let locator =
                payload_string(&event.payload, "locator").unwrap_or_else(|| "locator".into());
            format!("{entry_count} entries · {source_kind} · {locator}")
        }
        SkillAuditKind::SourceResolved => format!(
            "{} · distribution {}",
            payload_string(&event.payload, "revision")
                .or_else(|| payload_string(&event.payload, "version"))
                .unwrap_or_else(|| "unversioned".to_string()),
            payload_string(&event.payload, "distribution_id")
                .unwrap_or_else(|| "distribution".to_string())
        ),
        SkillAuditKind::ArtifactFetched => format!(
            "{}",
            payload_string(&event.payload, "artifact_locator")
                .unwrap_or_else(|| "artifact cached".to_string())
        ),
        SkillAuditKind::ArtifactEvicted => format!(
            "{} · retention {}s",
            payload_string(&event.payload, "artifact_locator")
                .unwrap_or_else(|| "artifact cache entry".to_string()),
            payload_usize(&event.payload, "retention_seconds").unwrap_or_default()
        ),
        SkillAuditKind::ArtifactFetchFailed => format!(
            "{}",
            payload_string(&event.payload, "error")
                .unwrap_or_else(|| "artifact fetch failed".to_string())
        ),
        SkillAuditKind::RemoteInstallPlanned | SkillAuditKind::RemoteUpdatePlanned => format!(
            "{} · {}",
            payload_string(&event.payload, "action").unwrap_or_else(|| "plan".to_string()),
            payload_string(&event.payload, "reason").unwrap_or_else(|| "remote plan".to_string())
        ),
        SkillAuditKind::LifecycleTransitioned => format!(
            "{} -> {}",
            payload_string(&event.payload, "from_state").unwrap_or_else(|| "none".to_string()),
            payload_string(&event.payload, "to_state").unwrap_or_else(|| "unknown".to_string())
        ),
        SkillAuditKind::Create
        | SkillAuditKind::Patch
        | SkillAuditKind::Edit
        | SkillAuditKind::Delete => format!(
            "{}",
            payload_string(&event.payload, "location").unwrap_or_else(|| "workspace write".into())
        ),
        SkillAuditKind::WriteFile | SkillAuditKind::RemoveFile => {
            let file_path = payload_string(&event.payload, "supporting_file")
                .unwrap_or_else(|| "supporting file".to_string());
            format!(
                "{} · {}",
                event.skill_name.as_deref().unwrap_or("skill"),
                file_path
            )
        }
        SkillAuditKind::HubInstall | SkillAuditKind::HubUpdate => format!(
            "{} · {}",
            event.source_id.as_deref().unwrap_or("source"),
            payload_string(&event.payload, "location").unwrap_or_else(|| "workspace import".into())
        ),
        SkillAuditKind::HubDetach => format!(
            "{} · workspace content preserved",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::HubRemove => format!(
            "{} · deleted_from_workspace={}",
            event.source_id.as_deref().unwrap_or("source"),
            payload_bool(&event.payload, "deleted_from_workspace").unwrap_or(false)
        ),
        SkillAuditKind::SyncPlanCreated | SkillAuditKind::SyncApplyCompleted => format!(
            "{} entries · {}",
            payload_usize(&event.payload, "entry_count").unwrap_or_default(),
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::GuardBlocked | SkillAuditKind::GuardWarned => {
            let violation_count = payload_usize(&event.payload, "violation_count").unwrap_or(0);
            let first_rule = payload_first_guard_rule(&event.payload);
            if let Some(first_rule) = first_rule {
                format!("{violation_count} violations · first rule {first_rule}")
            } else {
                format!("{violation_count} violations")
            }
        }
    }
}

fn audit_event_status(kind: &SkillAuditKind) -> SkillGovernanceTimelineStatus {
    match kind {
        SkillAuditKind::ArtifactEvicted => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::ArtifactFetchFailed => SkillGovernanceTimelineStatus::Error,
        SkillAuditKind::GuardBlocked => SkillGovernanceTimelineStatus::Error,
        SkillAuditKind::GuardWarned => SkillGovernanceTimelineStatus::Warn,
        SkillAuditKind::SyncPlanCreated => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::RemoteInstallPlanned
        | SkillAuditKind::RemoteUpdatePlanned
        | SkillAuditKind::LifecycleTransitioned => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::HubDetach => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::HubRemove => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::SourceIndexRefreshed
        | SkillAuditKind::SourceResolved
        | SkillAuditKind::ArtifactFetched => SkillGovernanceTimelineStatus::Success,
        SkillAuditKind::Create
        | SkillAuditKind::Patch
        | SkillAuditKind::Edit
        | SkillAuditKind::Delete
        | SkillAuditKind::WriteFile
        | SkillAuditKind::RemoveFile
        | SkillAuditKind::HubInstall
        | SkillAuditKind::HubUpdate
        | SkillAuditKind::SyncApplyCompleted => SkillGovernanceTimelineStatus::Success,
    }
}

fn guard_report_from_audit_event(event: &SkillAuditEvent) -> Option<SkillGuardReport> {
    if !matches!(
        event.kind,
        SkillAuditKind::GuardBlocked | SkillAuditKind::GuardWarned
    ) {
        return None;
    }

    let skill_name = event.skill_name.clone()?;
    let status = match payload_string(&event.payload, "status").as_deref() {
        Some("passed") => SkillGuardStatus::Passed,
        Some("blocked") => SkillGuardStatus::Blocked,
        _ => SkillGuardStatus::Warn,
    };
    let violations = event
        .payload
        .get("violations")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<SkillGuardViolation>>(value).ok())
        .unwrap_or_default();

    Some(SkillGuardReport {
        skill_name,
        status,
        violations,
        scanned_at: event.created_at,
    })
}

fn payload_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key)?.as_str().map(|value| value.to_string())
}

fn payload_bool(payload: &Value, key: &str) -> Option<bool> {
    payload.get(key)?.as_bool()
}

fn payload_usize(payload: &Value, key: &str) -> Option<usize> {
    payload.get(key)?.as_u64().map(|value| value as usize)
}

fn payload_first_guard_rule(payload: &Value) -> Option<String> {
    let violations = payload.get("violations")?.as_array()?;
    violations
        .first()?
        .get("rule_id")?
        .as_str()
        .map(|value| value.to_string())
}

fn now_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
