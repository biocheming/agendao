use super::audit::{artifact_evicted_audit_event, lifecycle_transition_audit_event};
use super::SkillGovernanceAuthority;
use crate::util::now_unix_timestamp;
use crate::{SkillError, SkillHubSnapshot};
use agendao_types::{
    BundledSkillManifest, ManagedSkillRecord, SkillArtifactCacheEntry, SkillAuditEvent,
    SkillCapabilityGroup, SkillDistributionRecord, SkillEvolutionEvidenceSummary, SkillHubPolicy,
    SkillManagedLifecycleRecord, SkillOperationalSnapshot, SkillOperationalSourceScope,
    SkillRelationshipEdge, SkillSourceIndexSnapshot, SkillUsageLedgerEntry, SkillWriteLedgerAction,
    SkillWriteLedgerEntry,
};
use std::path::Path;

impl SkillGovernanceAuthority {
    pub fn governance_snapshot(&self) -> SkillHubSnapshot {
        self.hub_store.snapshot()
    }

    pub fn managed_skills(&self) -> Vec<ManagedSkillRecord> {
        self.hub_store.managed_skills()
    }

    pub fn skill_operational_snapshots(&self) -> Vec<SkillOperationalSnapshot> {
        self.hub_store.skill_operational_snapshots()
    }

    pub fn skill_composition_relationships(&self) -> Vec<SkillRelationshipEdge> {
        self.hub_store.composition_relationships()
    }

    pub fn skill_capability_groups(&self) -> Vec<SkillCapabilityGroup> {
        self.hub_store.capability_groups()
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

    pub fn record_runtime_skill_usage(
        &self,
        skill_name: &str,
        tool_name: &str,
        stage_id: Option<&str>,
        category: Option<&str>,
        is_error: bool,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        let now = now_unix_timestamp();
        let usage = snapshot
            .usage
            .get_or_insert_with(SkillUsageLedgerEntry::default);
        usage.first_seen_at.get_or_insert(now);
        usage.last_used_at = Some(now);
        usage.runtime_use_count += 1;
        if is_error {
            usage.runtime_error_count += 1;
        } else {
            usage.runtime_success_count += 1;
        }
        usage.last_stage_id = stage_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        usage.last_tool_name = Some(tool_name.trim().to_string()).filter(|value| !value.is_empty());
        usage.last_category = category
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub fn record_skill_memory_promotion_signal(
        &self,
        skill_name: &str,
        promoted_record_count: u64,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        if matches!(snapshot.source_scope, SkillOperationalSourceScope::Unknown) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is unresolved and cannot record memory promotion evidence",
                    skill_name.trim()
                ),
            });
        }
        if promoted_record_count == 0 {
            return Ok(snapshot);
        }

        let now = now_unix_timestamp();
        let evolution = snapshot
            .evolution
            .get_or_insert_with(SkillEvolutionEvidenceSummary::default);
        evolution.memory_promotion_count += promoted_record_count;
        evolution.last_memory_promotion_at = Some(now);
        evolution.last_positive_signal_at = Some(
            evolution
                .last_positive_signal_at
                .map(|current| current.max(now))
                .unwrap_or(now),
        );

        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub fn record_skill_proposal_signal(
        &self,
        skill_name: &str,
        draft_proposal_count: u64,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        if matches!(snapshot.source_scope, SkillOperationalSourceScope::Unknown) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is unresolved and cannot record proposal evidence",
                    skill_name.trim()
                ),
            });
        }

        let now = now_unix_timestamp();
        let evolution = snapshot
            .evolution
            .get_or_insert_with(SkillEvolutionEvidenceSummary::default);
        evolution.last_observed_draft_proposal_count = draft_proposal_count;
        if draft_proposal_count > 0 {
            evolution.proposal_signal_count += 1;
            evolution.last_proposal_at = Some(now);
            evolution.last_positive_signal_at = Some(
                evolution
                    .last_positive_signal_at
                    .map(|current| current.max(now))
                    .unwrap_or(now),
            );
        }

        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub(super) fn record_lifecycle(
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

    pub(super) fn record_skill_write_action(
        &self,
        skill_name: &str,
        previous_skill_name: Option<&str>,
        action: SkillWriteLedgerAction,
        fallback_scope: SkillOperationalSourceScope,
        last_location: Option<&Path>,
        last_supporting_file: Option<&str>,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot =
            self.prepare_operational_snapshot(skill_name, previous_skill_name, fallback_scope)?;
        let now = now_unix_timestamp();
        let writes = snapshot
            .writes
            .get_or_insert_with(SkillWriteLedgerEntry::default);
        writes.first_written_at.get_or_insert(now);
        writes.last_write_at = Some(now);
        writes.last_action = Some(action);
        writes.last_location = last_location.map(|path| path.to_string_lossy().to_string());
        writes.last_supporting_file = last_supporting_file.map(ToOwned::to_owned);

        match action {
            SkillWriteLedgerAction::Create => writes.create_count += 1,
            SkillWriteLedgerAction::Patch => writes.patch_count += 1,
            SkillWriteLedgerAction::Edit => writes.edit_count += 1,
            SkillWriteLedgerAction::WriteFile => writes.supporting_file_write_count += 1,
            SkillWriteLedgerAction::RemoveFile => writes.supporting_file_remove_count += 1,
            SkillWriteLedgerAction::Install => writes.install_count += 1,
            SkillWriteLedgerAction::Update => writes.update_count += 1,
            SkillWriteLedgerAction::Detach => writes.detach_count += 1,
            SkillWriteLedgerAction::Remove => writes.remove_count += 1,
            SkillWriteLedgerAction::Delete => writes.delete_count += 1,
        }

        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub(super) fn prepare_operational_snapshot(
        &self,
        skill_name: &str,
        previous_skill_name: Option<&str>,
        fallback_scope: SkillOperationalSourceScope,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        if let Some(previous_skill_name) =
            previous_skill_name.filter(|previous| !previous.eq_ignore_ascii_case(skill_name))
        {
            self.hub_store
                .rename_skill_operational_snapshot(previous_skill_name, skill_name)?;
        }

        let mut snapshot = self
            .hub_store
            .skill_operational_snapshot(skill_name)
            .unwrap_or_else(|| SkillOperationalSnapshot {
                skill_name: skill_name.to_string(),
                ..SkillOperationalSnapshot::default()
            });
        snapshot.skill_name = skill_name.to_string();

        let (source_scope, source_id) = self.resolve_operational_identity(skill_name);
        if !matches!(source_scope, SkillOperationalSourceScope::Unknown) {
            snapshot.source_scope = source_scope;
            snapshot.source_id = source_id;
        } else if !matches!(fallback_scope, SkillOperationalSourceScope::Unknown) {
            snapshot.source_scope = fallback_scope;
            if !matches!(fallback_scope, SkillOperationalSourceScope::Managed) {
                snapshot.source_id = None;
            }
        }

        Ok(snapshot)
    }

    fn resolve_operational_identity(
        &self,
        skill_name: &str,
    ) -> (SkillOperationalSourceScope, Option<String>) {
        if let Some(managed) = self.hub_store.managed_skill(skill_name) {
            return (
                SkillOperationalSourceScope::Managed,
                managed.source.map(|source| source.source_id),
            );
        }

        match self
            .skill_authority
            .resolve_skill_for_inspection(skill_name, None)
        {
            Ok(meta) => {
                if meta
                    .location
                    .starts_with(self.skill_authority.workspace_skill_root())
                {
                    (SkillOperationalSourceScope::WorkspaceLocal, None)
                } else {
                    (SkillOperationalSourceScope::DiscoveredReadOnly, None)
                }
            }
            Err(_) => (SkillOperationalSourceScope::Unknown, None),
        }
    }
}
