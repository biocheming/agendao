use super::audit::{hub_remove_audit_event, sync_audit_event, write_audit_event};
use super::distribution::{release_identity, unresolved_distribution_id};
use super::normalize_name;
use super::write::create_target_from_relative_path;
use super::{SkillGovernanceAuthority, SkillGovernedSyncResult};
use crate::util::now_unix_timestamp;
use crate::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, RemoveSkillFileRequest, SkillError,
    SkillWriteResult, WriteSkillFileRequest,
};
use agendao_types::{
    ManagedSkillRecord, SkillAuditKind, SkillGuardReport, SkillManagedLifecycleRecord,
    SkillOperationalSourceScope, SkillSourceRef, SkillSyncAction, SkillSyncPlan,
    SkillWriteLedgerAction,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

impl SkillGovernanceAuthority {
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
        let meta = self
            .skill_authority
            .resolve_skill_for_inspection(skill_name, None)?;
        let markdown_content = self
            .skill_authority
            .load_skill_source_for_inspection(skill_name, None)?;
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
            supporting_files
                .iter()
                .map(|(path, content)| (path.as_str(), content.as_str())),
            false,
            now_unix_timestamp(),
        );
        let report = self.with_semantic_overlap_guard_warnings(
            report,
            Some(markdown_content.as_str()),
            meta.category.as_deref(),
            &[],
            Some(meta.name.as_str()),
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
                entry
                    .supporting_files
                    .iter()
                    .map(|file| (file.relative_path.as_str(), file.content.as_str())),
                duplicate_conflict,
                now_unix_timestamp(),
            );
            let report = self.with_semantic_overlap_guard_warnings(
                report,
                Some(entry.markdown_content.as_str()),
                entry.category.as_deref(),
                &[],
                None,
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
                        self.apply_import_guard(actor, source, source_entry, false, None)?
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
                    self.record_skill_write_action(
                        &result.skill_name,
                        None,
                        SkillWriteLedgerAction::Install,
                        SkillOperationalSourceScope::Managed,
                        Some(result.location.as_path()),
                        result.supporting_file.as_deref(),
                    )?;
                }
                SkillSyncAction::Update => {
                    let source_entry =
                        source_entry.ok_or_else(|| SkillError::InvalidSkillContent {
                            message: format!(
                                "sync plan for `{}` was missing source content",
                                plan_entry.skill_name
                            ),
                        })?;
                    if let Some(report) = self.apply_import_guard(
                        actor,
                        source,
                        source_entry,
                        false,
                        Some(plan_entry.skill_name.as_str()),
                    )? {
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
                    self.record_skill_write_action(
                        &result.skill_name,
                        Some(&plan_entry.skill_name),
                        SkillWriteLedgerAction::Update,
                        SkillOperationalSourceScope::Managed,
                        Some(result.location.as_path()),
                        result.supporting_file.as_deref(),
                    )?;
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
                        self.record_skill_write_action(
                            &managed_record.record.skill_name,
                            None,
                            SkillWriteLedgerAction::Remove,
                            SkillOperationalSourceScope::Managed,
                            None,
                            None,
                        )?;
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
            agendao_types::SkillSourceKind::Bundled => {
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
            agendao_types::SkillSourceKind::LocalPath => {
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

    pub(super) fn resolve_source_root(&self, locator: &str) -> PathBuf {
        let path = PathBuf::from(locator);
        if path.is_absolute() {
            path
        } else {
            self.hub_store.base_dir().join(path)
        }
    }

    fn apply_import_guard(
        &self,
        actor: &str,
        source: &SkillSourceRef,
        entry: &crate::sync::SkillSyncSourceEntry,
        duplicate_conflict: bool,
        current_skill_name: Option<&str>,
    ) -> Result<Option<SkillGuardReport>, SkillError> {
        let report = self.evaluate_imported_skill_guard_report(
            &entry.skill_name,
            &entry.markdown_content,
            entry
                .supporting_files
                .iter()
                .map(|file| (file.relative_path.as_str(), file.content.as_str())),
            duplicate_conflict,
            entry.category.as_deref(),
            current_skill_name,
        );
        self.apply_guard_report(actor, Some(source), report)
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

    pub(super) fn refresh_managed_record_for_source_skill(
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

    pub(super) fn update_distribution_runtime_state(
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

    fn synced_managed_record(
        &self,
        source: &SkillSourceRef,
        entry: &crate::sync::SkillSyncSourceEntry,
    ) -> Result<ManagedSkillRecord, SkillError> {
        let meta = self
            .skill_authority
            .resolve_skill_for_inspection(&entry.skill_name, None)?;
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
