use super::semantic::SkillSemanticDescriptor;
use super::{
    dedupe_string_reasons, normalize_name, required_nonempty_text, set_intersection_count,
};
use crate::SkillError;
use agendao_types::{
    SkillCapabilityMemberRole, SkillOperationalSnapshot, SkillRelationshipEdge,
    SkillSemanticConflictDiagnostic, SkillSemanticConflictKind,
};

pub(super) fn relationship_other_skill_name(
    relationship: &SkillRelationshipEdge,
    skill_name: &str,
) -> Option<String> {
    if relationship
        .left_skill_name
        .eq_ignore_ascii_case(skill_name)
    {
        return Some(relationship.right_skill_name.clone());
    }
    if relationship
        .right_skill_name
        .eq_ignore_ascii_case(skill_name)
    {
        return Some(relationship.left_skill_name.clone());
    }
    None
}

pub(super) fn relationship_pair_key(
    left_skill_name: &str,
    right_skill_name: &str,
) -> (String, String) {
    let left = normalize_name(left_skill_name);
    let right = normalize_name(right_skill_name);
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

pub(super) fn ordered_skill_names(
    left_skill_name: &str,
    right_skill_name: &str,
) -> (String, String) {
    if left_skill_name <= right_skill_name {
        (left_skill_name.to_string(), right_skill_name.to_string())
    } else {
        (right_skill_name.to_string(), left_skill_name.to_string())
    }
}

pub(super) fn normalize_runtime_selected_skill_names(raw_names: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for raw_name in raw_names {
        let trimmed = raw_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !names
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(trimmed))
        {
            names.push(trimmed.to_string());
        }
    }
    names
}

pub(super) fn skill_names_includes_pair(
    skill_names: &[String],
    left_skill_name: &str,
    right_skill_name: &str,
) -> bool {
    let left_present = skill_names
        .iter()
        .any(|skill_name| skill_name.eq_ignore_ascii_case(left_skill_name));
    let right_present = skill_names
        .iter()
        .any(|skill_name| skill_name.eq_ignore_ascii_case(right_skill_name));
    left_present && right_present
}

pub(super) fn build_skill_redundant_relationship_candidate(
    conflict: &SkillSemanticConflictDiagnostic,
) -> Option<SkillRelationshipEdge> {
    if conflict.score < 70 || conflict.preferred_skill_name.is_none() {
        return None;
    }
    if !matches!(
        conflict.kind,
        SkillSemanticConflictKind::ReplacementHint | SkillSemanticConflictKind::NearDuplicate
    ) {
        return None;
    }

    Some(SkillRelationshipEdge {
        left_skill_name: conflict.left_skill_name.clone(),
        right_skill_name: conflict.right_skill_name.clone(),
        relation_kind: agendao_types::SkillRelationshipKind::RedundantOverlap,
        state: agendao_types::SkillRelationshipState::Observed,
        score: conflict.score,
        reasons: dedupe_string_reasons(conflict.reasons.clone()),
        preferred_skill_name: conflict.preferred_skill_name.clone(),
        observed_at: None,
        updated_at: None,
    })
}

pub(super) fn build_skill_specialization_relationship_candidate(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    conflict: &SkillSemanticConflictDiagnostic,
) -> Option<SkillRelationshipEdge> {
    if conflict.score < 55 {
        return None;
    }

    let specialization = specialization_variant_direction(left, right)?;
    let mut reasons = conflict
        .reasons
        .iter()
        .filter(|reason| !reason.contains("usage ledger currently favors"))
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    reasons.extend(specialization.reasons);
    if conflict.preferred_skill_name.as_deref()
        == Some(specialization.canonical_skill_name.as_str())
    {
        reasons.push(format!(
            "usage ledger also currently favors `{}` within this variant family",
            specialization.canonical_skill_name
        ));
    }
    let (left_skill_name, right_skill_name) =
        ordered_skill_names(&left.skill_name, &right.skill_name);

    Some(SkillRelationshipEdge {
        left_skill_name,
        right_skill_name,
        relation_kind: agendao_types::SkillRelationshipKind::SpecializationVariant,
        state: agendao_types::SkillRelationshipState::Observed,
        score: conflict
            .score
            .saturating_add((specialization.strict_signal_count as u16).saturating_mul(5))
            .min(100),
        reasons: dedupe_string_reasons(reasons),
        preferred_skill_name: Some(specialization.canonical_skill_name),
        observed_at: None,
        updated_at: None,
    })
}

pub(super) fn build_skill_complementary_relationship_candidate(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    conflict: Option<&SkillSemanticConflictDiagnostic>,
    left_snapshot: Option<&SkillOperationalSnapshot>,
    right_snapshot: Option<&SkillOperationalSnapshot>,
) -> Option<SkillRelationshipEdge> {
    if conflict.map(|entry| entry.score >= 70).unwrap_or(false) {
        return None;
    }

    let same_category = left.category.is_some() && left.category == right.category;
    let direct_related = left.related_skills.contains(&right.normalized_name)
        || right.related_skills.contains(&left.normalized_name);
    let shared_related = set_intersection_count(&left.related_skills, &right.related_skills);
    let shared_tools = set_intersection_count(&left.requires_tools, &right.requires_tools);
    let shared_toolsets = set_intersection_count(&left.requires_toolsets, &right.requires_toolsets);
    let shared_last_category = shared_usage_value(
        left_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_category.as_deref()),
        right_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_category.as_deref()),
    );
    let shared_last_stage = shared_usage_value(
        left_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_stage_id.as_deref()),
        right_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_stage_id.as_deref()),
    );

    let has_anchor = direct_related || shared_related > 0 || same_category;
    let has_domain = shared_tools > 0
        || shared_toolsets > 0
        || shared_last_category.is_some()
        || shared_last_stage.is_some()
        || conflict.is_some();
    if !has_anchor || !has_domain {
        return None;
    }

    let mut score = 0u16;
    let mut reasons = Vec::new();
    if direct_related {
        score += 30;
        reasons.push("frontmatter related_skills directly links the pair".to_string());
    } else if shared_related > 0 {
        score += 20;
        reasons.push(format!(
            "related_skills metadata points at {shared_related} shared adjacent skill(s)"
        ));
    }
    if same_category {
        score += 20;
        if let Some(category) = left.category.as_deref() {
            reasons.push(format!("shared category `{category}`"));
        }
    }
    if shared_tools > 0 {
        score += 15;
        reasons.push(format!(
            "runtime tool requirements share {shared_tools} tool(s): {}",
            join_terms(left.requires_tools.intersection(&right.requires_tools))
        ));
    }
    if shared_toolsets > 0 {
        score += 15;
        reasons.push(format!(
            "runtime toolset requirements share {shared_toolsets} toolset(s): {}",
            join_terms(
                left.requires_toolsets
                    .intersection(&right.requires_toolsets)
            )
        ));
    }
    if let Some(category) = shared_last_category {
        score += 5;
        reasons.push(format!(
            "usage ledger recently observed both skills under runtime category `{category}`"
        ));
    }
    if let Some(stage) = shared_last_stage {
        score += 5;
        reasons.push(format!(
            "usage ledger recently observed both skills in runtime stage `{stage}`"
        ));
    }
    if let Some(conflict) = conflict {
        score += 10;
        if let Some(reason) = conflict
            .reasons
            .iter()
            .find(|reason| !reason.contains("usage ledger currently favors"))
        {
            reasons.push(format!(
                "semantic overlap stays below merge threshold but still signals shared working surface: {reason}"
            ));
        }
    }
    if score < 45 {
        return None;
    }

    let (left_skill_name, right_skill_name) =
        ordered_skill_names(&left.skill_name, &right.skill_name);
    Some(SkillRelationshipEdge {
        left_skill_name,
        right_skill_name,
        relation_kind: agendao_types::SkillRelationshipKind::ComplementaryComponent,
        state: agendao_types::SkillRelationshipState::Observed,
        score: score.min(100),
        reasons: dedupe_string_reasons(reasons),
        preferred_skill_name: None,
        observed_at: None,
        updated_at: None,
    })
}

#[derive(Debug, Clone)]
struct SkillSpecializationVariantDirection {
    canonical_skill_name: String,
    reasons: Vec<String>,
    strict_signal_count: usize,
}

fn specialization_variant_direction(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
) -> Option<SkillSpecializationVariantDirection> {
    let left_specializes_right = skill_narrowing_reasons(left, right);
    let right_specializes_left = skill_narrowing_reasons(right, left);
    match (left_specializes_right, right_specializes_left) {
        (Some((reasons, strict_signal_count)), None) => Some(SkillSpecializationVariantDirection {
            canonical_skill_name: right.skill_name.clone(),
            reasons,
            strict_signal_count,
        }),
        (None, Some((reasons, strict_signal_count))) => Some(SkillSpecializationVariantDirection {
            canonical_skill_name: left.skill_name.clone(),
            reasons,
            strict_signal_count,
        }),
        _ => None,
    }
}

fn skill_narrowing_reasons(
    candidate: &SkillSemanticDescriptor,
    broad: &SkillSemanticDescriptor,
) -> Option<(Vec<String>, usize)> {
    let mut reasons = Vec::new();
    let mut strict_signal_count = 0usize;

    if !broad.requires_tools.is_subset(&candidate.requires_tools) {
        return None;
    }
    let extra_tools = candidate
        .requires_tools
        .difference(&broad.requires_tools)
        .cloned()
        .collect::<Vec<_>>();
    if !extra_tools.is_empty() {
        strict_signal_count += 1;
        reasons.push(format!(
            "`{}` adds narrower runtime tool requirements beyond `{}`: {}",
            candidate.skill_name,
            broad.skill_name,
            join_terms(extra_tools.iter())
        ));
    }

    if !broad
        .requires_toolsets
        .is_subset(&candidate.requires_toolsets)
    {
        return None;
    }
    let extra_toolsets = candidate
        .requires_toolsets
        .difference(&broad.requires_toolsets)
        .cloned()
        .collect::<Vec<_>>();
    if !extra_toolsets.is_empty() {
        strict_signal_count += 1;
        reasons.push(format!(
            "`{}` adds narrower runtime toolset requirements beyond `{}`: {}",
            candidate.skill_name,
            broad.skill_name,
            join_terms(extra_toolsets.iter())
        ));
    }

    if strict_signal_count == 0 {
        return None;
    }

    Some((reasons, strict_signal_count))
}

fn shared_usage_value(left: Option<&str>, right: Option<&str>) -> Option<String> {
    let left = left.map(str::trim).filter(|value| !value.is_empty())?;
    let right = right.map(str::trim).filter(|value| !value.is_empty())?;
    if normalize_name(left) == normalize_name(right) {
        Some(left.to_string())
    } else {
        None
    }
}

pub(super) fn sort_skill_relationship_edges(edges: &mut [SkillRelationshipEdge]) {
    edges.sort_by(|left, right| {
        left.left_skill_name
            .cmp(&right.left_skill_name)
            .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
            .then_with(|| {
                relationship_kind_sort_key(left.relation_kind)
                    .cmp(&relationship_kind_sort_key(right.relation_kind))
            })
            .then_with(|| right.score.cmp(&left.score))
    });
}

fn relationship_kind_sort_key(kind: agendao_types::SkillRelationshipKind) -> u8 {
    match kind {
        agendao_types::SkillRelationshipKind::RedundantOverlap => 0,
        agendao_types::SkillRelationshipKind::SpecializationVariant => 1,
        agendao_types::SkillRelationshipKind::ComplementaryComponent => 2,
    }
}

pub(super) fn relationship_identity_key(
    left_skill_name: &str,
    right_skill_name: &str,
    relation_kind: agendao_types::SkillRelationshipKind,
) -> (String, String, u8) {
    let (left, right) = relationship_pair_key(left_skill_name, right_skill_name);
    (left, right, relationship_kind_sort_key(relation_kind))
}

pub(super) fn relationship_edge_identity_key(edge: &SkillRelationshipEdge) -> (String, String, u8) {
    relationship_identity_key(
        &edge.left_skill_name,
        &edge.right_skill_name,
        edge.relation_kind,
    )
}

pub(super) fn merge_relationship_inspection_entry(
    stored: &SkillRelationshipEdge,
    candidate: &SkillRelationshipEdge,
) -> SkillRelationshipEdge {
    SkillRelationshipEdge {
        left_skill_name: candidate.left_skill_name.clone(),
        right_skill_name: candidate.right_skill_name.clone(),
        relation_kind: stored.relation_kind,
        state: stored.state,
        score: candidate.score.max(stored.score),
        reasons: if stored.reasons.is_empty() {
            candidate.reasons.clone()
        } else {
            dedupe_string_reasons(
                stored
                    .reasons
                    .iter()
                    .cloned()
                    .chain(candidate.reasons.iter().cloned())
                    .collect(),
            )
        },
        preferred_skill_name: stored
            .preferred_skill_name
            .clone()
            .or_else(|| candidate.preferred_skill_name.clone()),
        observed_at: stored.observed_at.or(candidate.observed_at),
        updated_at: stored.updated_at,
    }
}

pub(super) fn validate_relationship_preferred_skill(
    left_skill_name: &str,
    right_skill_name: &str,
    relation_kind: agendao_types::SkillRelationshipKind,
    existing_preferred_skill_name: Option<&str>,
    requested_preferred_skill_name: Option<&str>,
) -> Result<Option<String>, SkillError> {
    match relation_kind {
        agendao_types::SkillRelationshipKind::RedundantOverlap
        | agendao_types::SkillRelationshipKind::SpecializationVariant => {
            let preferred_skill_name = requested_preferred_skill_name
                .or(existing_preferred_skill_name)
                .ok_or_else(|| SkillError::InvalidSkillContent {
                    message: format!(
                        "relationship `{}` requires a preferred canonical skill",
                        format_skill_relationship_kind(relation_kind)
                    ),
                })?;
            canonicalize_pair_skill_name(left_skill_name, right_skill_name, preferred_skill_name)
                .map(Some)
        }
        agendao_types::SkillRelationshipKind::ComplementaryComponent => {
            if requested_preferred_skill_name.is_some() {
                return Err(SkillError::InvalidSkillContent {
                    message: "complementary_component does not allow preferred_skill_name"
                        .to_string(),
                });
            }
            Ok(None)
        }
    }
}

fn canonicalize_pair_skill_name(
    left_skill_name: &str,
    right_skill_name: &str,
    requested_skill_name: &str,
) -> Result<String, SkillError> {
    let requested = required_nonempty_text(requested_skill_name, "preferred_skill_name")?;
    if left_skill_name.eq_ignore_ascii_case(&requested) {
        return Ok(left_skill_name.to_string());
    }
    if right_skill_name.eq_ignore_ascii_case(&requested) {
        return Ok(right_skill_name.to_string());
    }
    Err(SkillError::InvalidSkillContent {
        message: format!(
            "preferred skill `{}` must match one of `{}` or `{}`",
            requested, left_skill_name, right_skill_name
        ),
    })
}

pub(super) fn format_skill_relationship_kind(
    kind: agendao_types::SkillRelationshipKind,
) -> &'static str {
    match kind {
        agendao_types::SkillRelationshipKind::RedundantOverlap => "redundant_overlap",
        agendao_types::SkillRelationshipKind::SpecializationVariant => "specialization_variant",
        agendao_types::SkillRelationshipKind::ComplementaryComponent => "complementary_component",
    }
}

pub(super) fn format_skill_relationship_state(
    state: agendao_types::SkillRelationshipState,
) -> &'static str {
    match state {
        agendao_types::SkillRelationshipState::Observed => "observed",
        agendao_types::SkillRelationshipState::Accepted => "accepted",
        agendao_types::SkillRelationshipState::Dismissed => "dismissed",
    }
}

pub(super) fn format_runtime_prefer_canonical_hint(
    skill_name: &str,
    preferred_skill_name: &str,
    role: Option<SkillCapabilityMemberRole>,
    capability_id: Option<&str>,
) -> String {
    let (relation_label, closing_clause) = match role {
        Some(SkillCapabilityMemberRole::Specialization) => (
            "specialization variant",
            format!("only use `{skill_name}` for its narrower responsibility"),
        ),
        Some(SkillCapabilityMemberRole::MergeCandidate) => (
            "merge candidate",
            "avoid splitting duplicate instructions across both skills".to_string(),
        ),
        _ => (
            "related member",
            format!("keep `{preferred_skill_name}` as the family owner when the two overlap"),
        ),
    };
    let family_clause = capability_id
        .map(|value| format!(" within canonical family `{}`", value.trim()))
        .unwrap_or_default();
    format!(
        "Skill `{skill_name}` is governed as a {relation_label}{family_clause} under preferred skill `{preferred_skill_name}`. Prefer the canonical workflow as the family owner, and {closing_clause}."
    )
}

pub(super) fn format_runtime_complementary_bundle_hint(
    skill_names: &[String],
    capability_id: Option<&str>,
) -> String {
    let listed = skill_names
        .iter()
        .map(|skill_name| format!("`{skill_name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let bundle_clause = capability_id
        .map(|value| format!(" in complementary bundle `{}`", value.trim()))
        .unwrap_or_default();
    format!(
        "Skills {listed} are governed as complementary components{bundle_clause}. Keep their responsibilities distinct and do not collapse one skill into another."
    )
}

fn join_terms<'a>(terms: impl IntoIterator<Item = &'a String>) -> String {
    terms
        .into_iter()
        .map(|term| term.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
