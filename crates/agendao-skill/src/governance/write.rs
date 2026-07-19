use super::audit::write_audit_event;
use super::{SkillGovernanceAuthority, SkillGovernedWriteResult};
use crate::util::now_unix_timestamp;
use crate::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, RuntimeInstructionSource, RuntimeSkillBootstrapReport,
    RuntimeSkillMaterialization, RuntimeSkillMaterializationAction, RuntimeSkillSourceKind,
    SkillError, SkillWriteAction, WriteSkillFileRequest,
};
use agendao_types::{SkillAuditKind, SkillOperationalSourceScope, SkillWriteLedgerAction};
use std::path::Path;

impl SkillGovernanceAuthority {
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
            self.evaluate_create_guard_report(&req, duplicate_conflict),
        )?;
        let result = self.skill_authority.create_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::Create,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
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
        let current = self
            .skill_authority
            .resolve_skill_for_inspection(&req.name, None)?;
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
            self.evaluate_patch_guard_report(&current, &req, next_name, duplicate_conflict),
        )?;
        let result = self.skill_authority.patch_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            Some(&current.name),
            SkillWriteLedgerAction::Patch,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
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
            let existing = self
                .skill_authority
                .resolve_skill_for_inspection(&spec.name, None);
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

                    let loaded = self
                        .skill_authority
                        .load_skill_for_inspection(&spec.name, None)?;
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
        let current = self
            .skill_authority
            .resolve_skill_for_inspection(&req.name, None)?;
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
            self.evaluate_edit_guard_report(&current, &req, &next_name, duplicate_conflict),
        )?;
        let result = self.skill_authority.edit_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            Some(&current.name),
            SkillWriteLedgerAction::Edit,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
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
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::WriteFile,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
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
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::RemoveFile,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
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
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::Delete,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report: None,
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

pub(super) fn create_target_from_relative_path(
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
