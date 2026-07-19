use super::audit::guard_audit_event;
use super::normalize_name;
use super::semantic::{
    build_skill_semantic_conflict, build_skill_semantic_descriptor_from_parts,
    semantic_conflict_guard_violation, semantic_detail_related_skills, semantic_detail_tags,
    SkillSemanticDescriptor,
};
use super::SkillGovernanceAuthority;
use crate::util::now_unix_timestamp;
use crate::{
    CreateSkillRequest, EditSkillRequest, PatchSkillRequest, SkillConditions, SkillDetailView,
    SkillError,
};
use agendao_types::{
    SkillAuditKind, SkillGuardReport, SkillGuardStatus, SkillGuardViolation, SkillSourceRef,
};
use std::collections::{BTreeMap, BTreeSet};

impl SkillGovernanceAuthority {
    pub(super) fn evaluate_create_guard_report(
        &self,
        req: &CreateSkillRequest,
        duplicate_conflict: bool,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_create(
            &req.name,
            &req.description,
            &req.body,
            duplicate_conflict,
            now_unix_timestamp(),
        );
        let preview_markdown = crate::write::build_create_frontmatter(
            &req.name,
            &req.description,
            req.frontmatter.as_ref(),
        )
        .and_then(|frontmatter| crate::write::build_skill_document(&frontmatter, &req.body))
        .ok();
        self.with_semantic_overlap_guard_warnings(
            report,
            preview_markdown.as_deref(),
            req.category.as_deref(),
            &[],
            None,
        )
    }

    pub(super) fn evaluate_patch_guard_report(
        &self,
        current: &crate::SkillMeta,
        req: &PatchSkillRequest,
        next_name: &str,
        duplicate_conflict: bool,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_patch(
            &current.name,
            next_name,
            req.body.as_deref(),
            duplicate_conflict,
            now_unix_timestamp(),
        );
        let preview_markdown = self.build_patch_preview_markdown(current, req).ok();
        self.with_semantic_overlap_guard_warnings(
            report,
            preview_markdown.as_deref(),
            current.category.as_deref(),
            &[current.name.as_str()],
            Some(current.name.as_str()),
        )
    }

    pub(super) fn evaluate_edit_guard_report(
        &self,
        current: &crate::SkillMeta,
        req: &EditSkillRequest,
        next_name: &str,
        duplicate_conflict: bool,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_edit(
            next_name,
            &req.content,
            duplicate_conflict,
            now_unix_timestamp(),
        );
        self.with_semantic_overlap_guard_warnings(
            report,
            Some(req.content.as_str()),
            current.category.as_deref(),
            &[current.name.as_str()],
            Some(current.name.as_str()),
        )
    }

    pub(super) fn evaluate_imported_skill_guard_report(
        &self,
        skill_name: &str,
        markdown_content: &str,
        supporting_files: &[(String, String)],
        duplicate_conflict: bool,
        category: Option<&str>,
        current_skill_name: Option<&str>,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_imported_skill(
            skill_name,
            markdown_content,
            supporting_files,
            duplicate_conflict,
            now_unix_timestamp(),
        );
        let exclude_names = current_skill_name
            .map(|value| vec![value])
            .unwrap_or_default();
        self.with_semantic_overlap_guard_warnings(
            report,
            Some(markdown_content),
            category,
            exclude_names.as_slice(),
            current_skill_name,
        )
    }

    fn build_patch_preview_markdown(
        &self,
        current: &crate::SkillMeta,
        req: &PatchSkillRequest,
    ) -> Result<String, SkillError> {
        let mut document = crate::write::load_skill_document(&current.location)?;
        let mut frontmatter = crate::write::parse_skill_frontmatter(&document)?;
        let next_name = match req.new_name.as_deref() {
            Some(value) => crate::write::validate_skill_name(value)?,
            None => current.name.clone(),
        };
        let next_description = match req.description.as_deref() {
            Some(value) => crate::write::validate_skill_description(&next_name, value)?,
            None => current.description.clone(),
        };
        let next_body = match req.body.as_deref() {
            Some(value) => crate::write::validate_skill_body(value)?,
            None => document.body.clone(),
        };

        frontmatter.name = next_name;
        frontmatter.description = next_description;
        if let Some(patch) = req.frontmatter.as_ref() {
            crate::write::apply_frontmatter_patch(&mut frontmatter, patch);
        }
        document.frontmatter_lines = crate::write::render_skill_frontmatter_lines(&frontmatter)?;
        document.body = next_body;
        Ok(crate::write::render_skill_document(&document))
    }

    pub(super) fn with_semantic_overlap_guard_warnings(
        &self,
        mut report: SkillGuardReport,
        markdown_content: Option<&str>,
        category: Option<&str>,
        exclude_names: &[&str],
        current_skill_name: Option<&str>,
    ) -> SkillGuardReport {
        let Some(markdown_content) = markdown_content else {
            return report;
        };
        let Some(candidate) = self.semantic_descriptor_from_markdown(markdown_content, category)
        else {
            return report;
        };
        let violations =
            self.semantic_overlap_guard_violations(&candidate, exclude_names, current_skill_name);
        if violations.is_empty() {
            return report;
        }
        if report.status == SkillGuardStatus::Passed {
            report.status = SkillGuardStatus::Warn;
        }
        report.violations.extend(violations);
        report
    }

    fn semantic_descriptor_from_markdown(
        &self,
        markdown_content: &str,
        category: Option<&str>,
    ) -> Option<SkillSemanticDescriptor> {
        let document = crate::write::parse_skill_document(markdown_content).ok()?;
        let frontmatter = crate::write::parse_skill_frontmatter(&document).ok()?;
        let agendao = frontmatter
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.agendao.as_ref());
        let detail = SkillDetailView {
            tags: semantic_detail_tags(&frontmatter),
            related_skills: semantic_detail_related_skills(&frontmatter),
            ..SkillDetailView::default()
        };
        let conditions = SkillConditions {
            requires_tools: agendao
                .map(|metadata| metadata.requires_tools.clone())
                .unwrap_or_default(),
            fallback_for_tools: agendao
                .map(|metadata| metadata.fallback_for_tools.clone())
                .unwrap_or_default(),
            requires_toolsets: agendao
                .map(|metadata| metadata.requires_toolsets.clone())
                .unwrap_or_default(),
            fallback_for_toolsets: agendao
                .map(|metadata| metadata.fallback_for_toolsets.clone())
                .unwrap_or_default(),
            stage_filter: agendao
                .map(|metadata| metadata.stage_filter.clone())
                .unwrap_or_default(),
        };
        Some(build_skill_semantic_descriptor_from_parts(
            &frontmatter.name,
            &frontmatter.description,
            category,
            &conditions,
            &detail,
        ))
    }

    fn semantic_overlap_guard_violations(
        &self,
        candidate: &SkillSemanticDescriptor,
        exclude_names: &[&str],
        current_skill_name: Option<&str>,
    ) -> Vec<SkillGuardViolation> {
        let snapshot_by_name = self
            .skill_operational_snapshots()
            .into_iter()
            .map(|snapshot| (normalize_name(&snapshot.skill_name), snapshot))
            .collect::<BTreeMap<_, _>>();
        let exclude = exclude_names
            .iter()
            .map(|name| normalize_name(name))
            .collect::<BTreeSet<_>>();
        let candidate_snapshot = current_skill_name.and_then(|name| {
            let normalized = normalize_name(name);
            snapshot_by_name.get(&normalized)
        });
        let mut conflicts = self
            .skill_authority
            .list_skill_catalog(None)
            .unwrap_or_default()
            .into_iter()
            .filter(|meta| !exclude.contains(&normalize_name(&meta.name)))
            .filter_map(|meta| {
                let existing = self
                    .build_skill_semantic_descriptor(&meta, &snapshot_by_name)
                    .ok()?;
                build_skill_semantic_conflict(
                    candidate,
                    &existing,
                    candidate_snapshot,
                    snapshot_by_name.get(&normalize_name(&existing.skill_name)),
                )
            })
            .collect::<Vec<_>>();
        conflicts.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.left_skill_name.cmp(&right.left_skill_name))
                .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
        });
        conflicts
            .into_iter()
            .take(3)
            .map(|conflict| semantic_conflict_guard_violation(candidate, &conflict))
            .collect()
    }

    pub(super) fn apply_guard_report(
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

    pub(super) fn audit_guard_observation(
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
}
