use super::{
    normalize_name, set_intersection_count, skill_diagnostic_sort_key, SkillGovernanceAuthority,
};
use crate::{SkillConditions, SkillDetailView, SkillError};
use agendao_types::{
    SkillGovernanceDiagnosticSeverity, SkillGuardSeverity, SkillGuardViolation,
    SkillOperationalSnapshot, SkillSemanticConflictDiagnostic, SkillSemanticConflictKind,
};
use std::collections::{BTreeMap, BTreeSet};

impl SkillGovernanceAuthority {
    pub(super) fn build_skill_semantic_descriptor(
        &self,
        meta: &crate::SkillMeta,
        _snapshot_by_name: &BTreeMap<String, SkillOperationalSnapshot>,
    ) -> Result<SkillSemanticDescriptor, SkillError> {
        let detail = self
            .skill_authority
            .load_skill_detail_for_meta_for_inspection(meta)
            .unwrap_or_default();
        Ok(build_skill_semantic_descriptor_from_parts(
            &meta.name,
            &meta.description,
            meta.category.as_deref(),
            &meta.conditions,
            &detail,
        ))
    }

    pub(super) fn skill_semantic_analysis_inputs(
        &self,
    ) -> Result<
        (
            Vec<SkillSemanticDescriptor>,
            BTreeMap<String, SkillOperationalSnapshot>,
        ),
        SkillError,
    > {
        let snapshots = self.skill_operational_snapshots();
        let snapshot_by_name = snapshots
            .iter()
            .cloned()
            .map(|snapshot| (normalize_name(&snapshot.skill_name), snapshot))
            .collect::<BTreeMap<_, _>>();
        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let mut descriptors = Vec::with_capacity(catalog.len());
        for meta in &catalog {
            descriptors.push(self.build_skill_semantic_descriptor(meta, &snapshot_by_name)?);
        }
        Ok((descriptors, snapshot_by_name))
    }
}

const SKILL_GUARD_RULE_SEMANTIC_OVERLAP: &str = "semantic.skill_overlap";
const SKILL_GUARD_RULE_TRIGGER_OVERLAP: &str = "semantic.trigger_overlap";

#[derive(Debug, Clone)]
pub(super) struct SkillSemanticDescriptor {
    pub(super) skill_name: String,
    pub(super) normalized_name: String,
    pub(super) category: Option<String>,
    pub(super) tokens: BTreeSet<String>,
    pub(super) trigger_terms: BTreeSet<String>,
    pub(super) related_skills: BTreeSet<String>,
    pub(super) requires_tools: BTreeSet<String>,
    pub(super) requires_toolsets: BTreeSet<String>,
}

pub(super) fn build_skill_semantic_descriptor_from_parts(
    skill_name: &str,
    description: &str,
    category: Option<&str>,
    conditions: &SkillConditions,
    detail: &SkillDetailView,
) -> SkillSemanticDescriptor {
    let normalized_name = normalize_name(skill_name);

    let mut tokens = BTreeSet::new();
    for token in skill_descriptor_tokens(skill_name) {
        tokens.insert(token);
    }
    for token in skill_descriptor_tokens(description) {
        tokens.insert(token);
    }
    if let Some(category) = category {
        for token in skill_descriptor_tokens(category) {
            tokens.insert(token);
        }
    }
    for token in &detail.tags {
        for normalized in skill_descriptor_tokens(token) {
            tokens.insert(normalized);
        }
    }

    let mut trigger_terms = BTreeSet::new();
    for value in conditions
        .requires_tools
        .iter()
        .chain(conditions.requires_toolsets.iter())
        .chain(conditions.fallback_for_tools.iter())
        .chain(conditions.fallback_for_toolsets.iter())
    {
        let normalized = normalize_name(value);
        if !normalized.is_empty() {
            trigger_terms.insert(normalized);
        }
    }

    let related_skills = detail
        .related_skills
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    let requires_tools = conditions
        .requires_tools
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    let requires_toolsets = conditions
        .requires_toolsets
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    SkillSemanticDescriptor {
        skill_name: skill_name.to_string(),
        normalized_name,
        category: category.map(normalize_name),
        tokens,
        trigger_terms,
        related_skills,
        requires_tools,
        requires_toolsets,
    }
}

pub(super) fn semantic_detail_tags(frontmatter: &crate::SkillFrontmatter) -> Vec<String> {
    frontmatter
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.hermes.as_ref())
        .map(|metadata| metadata.tags.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| frontmatter.tags.clone())
}

pub(super) fn semantic_detail_related_skills(frontmatter: &crate::SkillFrontmatter) -> Vec<String> {
    frontmatter
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.hermes.as_ref())
        .map(|metadata| metadata.related_skills.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| frontmatter.related_skills.clone())
}

pub(super) fn semantic_conflict_guard_violation(
    candidate: &SkillSemanticDescriptor,
    conflict: &SkillSemanticConflictDiagnostic,
) -> SkillGuardViolation {
    let counterpart = semantic_conflict_counterpart_skill_name(candidate, conflict)
        .unwrap_or(conflict.right_skill_name.as_str());
    let overlap_reason = conflict
        .reasons
        .iter()
        .filter(|reason| !reason.contains("usage ledger currently favors"))
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    let mut message = format!(
        "skill `{}` overlaps existing `{}` ({}, score {}): {}.",
        candidate.skill_name,
        counterpart,
        semantic_conflict_guard_kind_label(conflict.kind),
        conflict.score,
        if overlap_reason.is_empty() {
            "semantic overlap was detected".to_string()
        } else {
            overlap_reason
        }
    );
    if let Some(preferred_skill_name) = conflict.preferred_skill_name.as_deref() {
        message.push_str(&format!(
            " usage ledger currently favors `{preferred_skill_name}` in this overlap pair."
        ));
    }
    SkillGuardViolation {
        rule_id: semantic_conflict_guard_rule_id(conflict.kind).to_string(),
        severity: SkillGuardSeverity::Warn,
        message,
        file_path: Some("SKILL.md".to_string()),
    }
}

fn semantic_conflict_counterpart_skill_name<'a>(
    candidate: &SkillSemanticDescriptor,
    conflict: &'a SkillSemanticConflictDiagnostic,
) -> Option<&'a str> {
    if conflict
        .left_skill_name
        .eq_ignore_ascii_case(&candidate.skill_name)
    {
        return Some(conflict.right_skill_name.as_str());
    }
    if conflict
        .right_skill_name
        .eq_ignore_ascii_case(&candidate.skill_name)
    {
        return Some(conflict.left_skill_name.as_str());
    }
    None
}

fn semantic_conflict_guard_rule_id(kind: SkillSemanticConflictKind) -> &'static str {
    match kind {
        SkillSemanticConflictKind::TriggerOverlap => SKILL_GUARD_RULE_TRIGGER_OVERLAP,
        SkillSemanticConflictKind::NearDuplicate | SkillSemanticConflictKind::ReplacementHint => {
            SKILL_GUARD_RULE_SEMANTIC_OVERLAP
        }
    }
}

fn semantic_conflict_guard_kind_label(kind: SkillSemanticConflictKind) -> &'static str {
    match kind {
        SkillSemanticConflictKind::NearDuplicate => "near duplicate",
        SkillSemanticConflictKind::TriggerOverlap => "trigger overlap",
        SkillSemanticConflictKind::ReplacementHint => "replacement hint",
    }
}

pub(super) fn build_skill_semantic_conflict(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    left_snapshot: Option<&SkillOperationalSnapshot>,
    right_snapshot: Option<&SkillOperationalSnapshot>,
) -> Option<SkillSemanticConflictDiagnostic> {
    if left.normalized_name == right.normalized_name {
        return None;
    }

    let shared_tokens = set_intersection_count(&left.tokens, &right.tokens);
    let token_jaccard = set_jaccard_ratio(&left.tokens, &right.tokens);
    let shared_triggers = set_intersection_count(&left.trigger_terms, &right.trigger_terms);
    let trigger_jaccard = set_jaccard_ratio(&left.trigger_terms, &right.trigger_terms);
    let same_category = left.category.is_some() && left.category == right.category;
    let related_overlap = left.related_skills.contains(&right.normalized_name)
        || right.related_skills.contains(&left.normalized_name)
        || set_intersection_count(&left.related_skills, &right.related_skills) > 0;

    let mut score = 0u16;
    let mut reasons = Vec::new();

    if same_category {
        score += 15;
        if let Some(category) = left.category.as_deref() {
            reasons.push(format!("shared category `{category}`"));
        }
    }

    if shared_triggers > 0 && trigger_jaccard >= 0.6 {
        score += 35;
        reasons.push(format!(
            "runtime trigger conditions heavily overlap ({shared_triggers} shared trigger terms)"
        ));
    } else if shared_triggers >= 2 {
        score += 20;
        reasons.push(format!(
            "runtime trigger conditions overlap ({shared_triggers} shared trigger terms)"
        ));
    }

    if shared_tokens >= 3 && token_jaccard >= 0.45 {
        score += 30;
        reasons.push(format!(
            "name/description tokens strongly overlap ({shared_tokens} shared descriptor terms)"
        ));
    } else if shared_tokens >= 4 {
        score += 20;
        reasons.push(format!(
            "descriptor vocabulary overlaps ({shared_tokens} shared terms)"
        ));
    }

    if related_overlap {
        score += 10;
        reasons.push(
            "frontmatter related-skills metadata points at the same capability cluster".to_string(),
        );
    }

    if score < 45 {
        return None;
    }

    let left_runtime_use_count = left_snapshot
        .and_then(|snapshot| snapshot.usage.as_ref())
        .map(|usage| usage.runtime_use_count)
        .unwrap_or(0);
    let right_runtime_use_count = right_snapshot
        .and_then(|snapshot| snapshot.usage.as_ref())
        .map(|usage| usage.runtime_use_count)
        .unwrap_or(0);
    let left_last_used_at =
        left_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_used_at);
    let right_last_used_at =
        right_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_used_at);

    let preferred_skill_name = preferred_skill_name(
        left,
        right,
        left_runtime_use_count,
        right_runtime_use_count,
        left_last_used_at,
        right_last_used_at,
    );
    if let Some(preferred) = preferred_skill_name.as_deref() {
        reasons.push(format!(
            "usage ledger currently favors `{preferred}` as the more active skill in this overlap pair"
        ));
    }

    let kind = if score >= 70 && preferred_skill_name.is_some() {
        SkillSemanticConflictKind::ReplacementHint
    } else if score >= 70 {
        SkillSemanticConflictKind::NearDuplicate
    } else {
        SkillSemanticConflictKind::TriggerOverlap
    };
    let severity = if matches!(
        kind,
        SkillSemanticConflictKind::ReplacementHint | SkillSemanticConflictKind::NearDuplicate
    ) {
        SkillGovernanceDiagnosticSeverity::Warn
    } else {
        SkillGovernanceDiagnosticSeverity::Info
    };

    let (
        left_skill_name,
        right_skill_name,
        left_runtime_use_count,
        right_runtime_use_count,
        left_last_used_at,
        right_last_used_at,
        preferred_skill_name,
    ) = if left.skill_name <= right.skill_name {
        (
            left.skill_name.clone(),
            right.skill_name.clone(),
            left_runtime_use_count,
            right_runtime_use_count,
            left_last_used_at,
            right_last_used_at,
            preferred_skill_name,
        )
    } else {
        (
            right.skill_name.clone(),
            left.skill_name.clone(),
            right_runtime_use_count,
            left_runtime_use_count,
            right_last_used_at,
            left_last_used_at,
            preferred_skill_name,
        )
    };

    Some(SkillSemanticConflictDiagnostic {
        left_skill_name,
        right_skill_name,
        kind,
        severity,
        score,
        reasons,
        preferred_skill_name,
        left_runtime_use_count,
        right_runtime_use_count,
        left_last_used_at,
        right_last_used_at,
    })
}

pub(super) fn collect_skill_semantic_conflicts(
    descriptors: &[SkillSemanticDescriptor],
    snapshot_by_name: &BTreeMap<String, SkillOperationalSnapshot>,
) -> Vec<SkillSemanticConflictDiagnostic> {
    let mut diagnostics = Vec::new();
    for left_index in 0..descriptors.len() {
        for right_index in (left_index + 1)..descriptors.len() {
            if let Some(conflict) = build_skill_semantic_conflict(
                &descriptors[left_index],
                &descriptors[right_index],
                snapshot_by_name.get(&normalize_name(&descriptors[left_index].skill_name)),
                snapshot_by_name.get(&normalize_name(&descriptors[right_index].skill_name)),
            ) {
                diagnostics.push(conflict);
            }
        }
    }

    diagnostics.sort_by(|left, right| {
        skill_diagnostic_sort_key(left.severity)
            .cmp(&skill_diagnostic_sort_key(right.severity))
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| left.left_skill_name.cmp(&right.left_skill_name))
            .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
    });
    diagnostics
}

fn preferred_skill_name(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    left_runtime_use_count: u64,
    right_runtime_use_count: u64,
    left_last_used_at: Option<i64>,
    right_last_used_at: Option<i64>,
) -> Option<String> {
    if left_runtime_use_count > right_runtime_use_count {
        return Some(left.skill_name.clone());
    }
    if right_runtime_use_count > left_runtime_use_count {
        return Some(right.skill_name.clone());
    }
    match (left_last_used_at, right_last_used_at) {
        (Some(left_ts), Some(right_ts)) if left_ts > right_ts => Some(left.skill_name.clone()),
        (Some(left_ts), Some(right_ts)) if right_ts > left_ts => Some(right.skill_name.clone()),
        (Some(_), None) => Some(left.skill_name.clone()),
        (None, Some(_)) => Some(right.skill_name.clone()),
        _ => None,
    }
}

pub(super) fn semantic_conflict_is_review_candidate(
    conflict: &SkillSemanticConflictDiagnostic,
) -> bool {
    conflict.severity == SkillGovernanceDiagnosticSeverity::Warn
        && conflict.kind == SkillSemanticConflictKind::ReplacementHint
        && conflict.preferred_skill_name.is_some()
}

pub(super) fn semantic_conflict_redundant_skill_name(
    conflict: &SkillSemanticConflictDiagnostic,
    preferred_skill_name: &str,
) -> Option<String> {
    if conflict
        .left_skill_name
        .eq_ignore_ascii_case(preferred_skill_name)
    {
        return Some(conflict.right_skill_name.clone());
    }
    if conflict
        .right_skill_name
        .eq_ignore_ascii_case(preferred_skill_name)
    {
        return Some(conflict.left_skill_name.clone());
    }
    None
}

pub(super) fn semantic_conflict_review_candidate_summary(
    conflict: &SkillSemanticConflictDiagnostic,
    preferred_skill_name: &str,
) -> String {
    let overlap_reason = conflict
        .reasons
        .iter()
        .find(|reason| !reason.contains("usage ledger currently favors"))
        .cloned()
        .unwrap_or_else(|| "semantic overlap was detected".to_string());
    format!(
        "{overlap_reason}; usage ledger currently favors `{preferred_skill_name}` as the more active skill in this overlap pair"
    )
}

fn set_jaccard_ratio(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = set_intersection_count(left, right) as f32;
    let union = (left.len() + right.len() - shared as usize) as f32;
    if union <= 0.0 {
        0.0
    } else {
        shared / union
    }
}

fn skill_descriptor_tokens(value: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "be", "for", "from", "how", "into", "of", "or", "that", "the",
        "this", "to", "use", "with",
    ];

    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| part.len() >= 3)
        .filter(|part| !STOP_WORDS.contains(&part.as_str()))
        .collect()
}
