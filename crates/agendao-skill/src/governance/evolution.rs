use super::audit::vitality_transition_audit_event;
use super::semantic::{
    collect_skill_semantic_conflicts, semantic_conflict_is_review_candidate,
    semantic_conflict_redundant_skill_name, semantic_conflict_review_candidate_summary,
};
use super::{
    normalize_name, skill_diagnostic_sort_key, SkillCompositionConsumptionContext,
    SkillGovernanceAuthority,
};
use crate::util::now_unix_timestamp;
use crate::SkillError;
use agendao_types::{
    SkillCapabilityMemberRole, SkillEvolutionEvidenceSummary, SkillGovernanceDiagnosticSeverity,
    SkillNegativeEntropyDiagnostic, SkillNegativeEntropySignal, SkillOperationalSnapshot,
    SkillOperationalSourceScope, SkillRetirementReason, SkillRetirementReasonKind,
    SkillSemanticConflictDiagnostic, SkillVitalityRecord, SkillVitalityState, SkillWriteLedgerEntry,
};
use std::collections::{BTreeMap, BTreeSet};

impl SkillGovernanceAuthority {
    pub fn effective_skill_vitality_state(&self, skill_name: &str) -> SkillVitalityState {
        self.hub_store
            .skill_operational_snapshot(skill_name)
            .as_ref()
            .and_then(|snapshot| snapshot.vitality.as_ref())
            .map(|record| record.state)
            .unwrap_or(SkillVitalityState::Active)
    }

    pub fn ensure_skill_runtime_available(&self, skill_name: &str) -> Result<(), SkillError> {
        let state = self.effective_skill_vitality_state(skill_name);
        match state {
            SkillVitalityState::Retired | SkillVitalityState::Archived => {
                Err(SkillError::SkillRuntimeUnavailable {
                    name: skill_name.trim().to_string(),
                    state,
                })
            }
            SkillVitalityState::Active | SkillVitalityState::ReviewCandidate => Ok(()),
        }
    }

    pub fn sync_negative_entropy_review_candidates(
        &self,
        actor: &str,
    ) -> Result<Vec<SkillOperationalSnapshot>, SkillError> {
        let diagnostics = self.skill_negative_entropy_diagnostics()?;
        let mut updated = Vec::new();
        for diagnostic in diagnostics.into_iter().filter(|entry| {
            entry.source_scope == SkillOperationalSourceScope::WorkspaceLocal
                && entry.severity == SkillGovernanceDiagnosticSeverity::Warn
        }) {
            let current = self
                .hub_store
                .skill_operational_snapshot(&diagnostic.skill_name);
            if !should_sync_negative_entropy_review_candidate(current.as_ref()) {
                continue;
            }

            let composition_context =
                self.skill_composition_consumption_context(&diagnostic.skill_name);
            let reason = SkillRetirementReason {
                kind: SkillRetirementReasonKind::NegativeEntropy,
                summary: negative_entropy_review_candidate_summary(
                    &diagnostic,
                    &composition_context,
                ),
                noted_at: now_unix_timestamp(),
                related_skill_name: composition_context
                    .related_skill_name_for_review(&diagnostic.skill_name),
            };
            updated.push(self.set_skill_vitality_state(
                &diagnostic.skill_name,
                SkillVitalityState::ReviewCandidate,
                reason,
                actor,
            )?);
        }
        Ok(updated)
    }

    pub fn sync_semantic_conflict_review_candidates(
        &self,
        actor: &str,
    ) -> Result<Vec<SkillOperationalSnapshot>, SkillError> {
        let diagnostics = self.skill_semantic_conflict_diagnostics()?;
        let mut updated = Vec::new();
        let mut seen_redundant = BTreeSet::new();
        for conflict in diagnostics
            .into_iter()
            .filter(semantic_conflict_is_review_candidate)
        {
            let Some(preferred_skill_name) = conflict.preferred_skill_name.clone() else {
                continue;
            };
            let Some(redundant_skill_name) =
                semantic_conflict_redundant_skill_name(&conflict, &preferred_skill_name)
            else {
                continue;
            };
            if !seen_redundant.insert(normalize_name(&redundant_skill_name)) {
                continue;
            }

            let current = self.prepare_operational_snapshot(
                &redundant_skill_name,
                None,
                SkillOperationalSourceScope::Unknown,
            )?;
            if current.source_scope != SkillOperationalSourceScope::WorkspaceLocal {
                continue;
            }
            if !should_sync_semantic_conflict_review_candidate(
                Some(&current),
                &preferred_skill_name,
            ) {
                continue;
            }

            let reason = SkillRetirementReason {
                kind: SkillRetirementReasonKind::SemanticConflict,
                summary: semantic_conflict_review_candidate_summary(
                    &conflict,
                    &preferred_skill_name,
                ),
                noted_at: now_unix_timestamp(),
                related_skill_name: Some(preferred_skill_name.clone()),
            };
            updated.push(self.set_skill_vitality_state(
                &redundant_skill_name,
                SkillVitalityState::ReviewCandidate,
                reason,
                actor,
            )?);
        }
        Ok(updated)
    }

    pub fn set_skill_vitality_state(
        &self,
        skill_name: &str,
        state: SkillVitalityState,
        reason: SkillRetirementReason,
        actor: &str,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        if snapshot.source_scope != SkillOperationalSourceScope::WorkspaceLocal {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not a workspace-local mutable skill and cannot change vitality state",
                    skill_name.trim()
                ),
            });
        }

        let previous_state = snapshot
            .vitality
            .as_ref()
            .map(|record| record.state)
            .unwrap_or(SkillVitalityState::Active);
        let vitality = SkillVitalityRecord {
            state,
            updated_at: reason.noted_at,
            reason: reason.clone(),
        };
        snapshot.vitality = Some(vitality.clone());
        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        self.append_audit_event(vitality_transition_audit_event(
            &snapshot,
            previous_state,
            &vitality,
            actor,
        ))?;
        Ok(snapshot)
    }

    pub fn skill_negative_entropy_diagnostics(
        &self,
    ) -> Result<Vec<SkillNegativeEntropyDiagnostic>, SkillError> {
        let snapshots = self.skill_operational_snapshots();
        let conflicts = self.skill_semantic_conflict_diagnostics()?;
        let mut overlap_counts = BTreeMap::<String, u64>::new();
        for conflict in &conflicts {
            *overlap_counts
                .entry(normalize_name(&conflict.left_skill_name))
                .or_default() += 1;
            *overlap_counts
                .entry(normalize_name(&conflict.right_skill_name))
                .or_default() += 1;
        }

        let now = now_unix_timestamp();
        let mut diagnostics = Vec::new();
        for snapshot in snapshots {
            let runtime_use_count = snapshot
                .usage
                .as_ref()
                .map(|entry| entry.runtime_use_count)
                .unwrap_or(0);
            let runtime_error_count = snapshot
                .usage
                .as_ref()
                .map(|entry| entry.runtime_error_count)
                .unwrap_or(0);
            let last_used_at = snapshot.usage.as_ref().and_then(|entry| entry.last_used_at);
            let write_count = snapshot
                .writes
                .as_ref()
                .map(total_skill_write_count)
                .unwrap_or(0);
            let last_write_at = snapshot
                .writes
                .as_ref()
                .and_then(|entry| entry.last_write_at);
            let semantic_overlap_count = overlap_counts
                .get(&normalize_name(&snapshot.skill_name))
                .copied()
                .unwrap_or(0);
            let recent_positive_evolution = snapshot
                .evolution
                .as_ref()
                .and_then(|summary| summary.last_positive_signal_at)
                .is_some_and(|timestamp| {
                    now.saturating_sub(timestamp) < SKILL_POSITIVE_EVOLUTION_GRACE_SECONDS
                });
            let composition_context =
                self.skill_composition_consumption_context(&snapshot.skill_name);

            let mut signals = Vec::new();
            let mut reasons = Vec::new();

            if write_count > 0 && runtime_use_count == 0 {
                signals.push(SkillNegativeEntropySignal::NeverReused);
                reasons.push(format!(
                    "write history exists ({write_count} write actions) but runtime reuse has never been recorded"
                ));
            }

            if write_count >= 3 && runtime_use_count <= 1 {
                signals.push(SkillNegativeEntropySignal::WriteHeavyLowReuse);
                reasons.push(format!(
                    "write churn is high ({write_count} write actions) while runtime reuse remains low ({runtime_use_count})"
                ));
            }

            if is_skill_timestamp_stale(last_used_at, now, SKILL_NEGATIVE_ENTROPY_STALE_SECONDS)
                && is_skill_timestamp_stale(
                    last_write_at.or(last_used_at),
                    now,
                    SKILL_NEGATIVE_ENTROPY_STALE_SECONDS,
                )
            {
                signals.push(SkillNegativeEntropySignal::StaleUnused);
                reasons.push(format!(
                    "no recent use or write activity in the last {} days",
                    SKILL_NEGATIVE_ENTROPY_STALE_SECONDS / 86_400
                ));
            }

            if matches!(snapshot.source_scope, SkillOperationalSourceScope::Managed)
                && runtime_use_count == 0
                && last_used_at.is_none()
            {
                signals.push(SkillNegativeEntropySignal::DormantManaged);
                reasons.push(
                    "managed skill has been installed or tracked, but runtime usage has not been observed yet"
                        .to_string(),
                );
            }

            if signals.is_empty() {
                continue;
            }

            if semantic_overlap_count > 0 {
                reasons.push(format!(
                    "semantic conflict diagnostics report {semantic_overlap_count} overlap candidate(s)"
                ));
            }
            if runtime_error_count > 0 {
                reasons.push(format!(
                    "runtime ledger recorded {runtime_error_count} error event(s)"
                ));
            }
            if recent_positive_evolution {
                if let Some(evolution) = snapshot.evolution.as_ref() {
                    reasons.push(format_skill_positive_evolution_reason(evolution, now));
                }
            }
            if let Some(reason) = format_negative_entropy_composition_reason(
                &snapshot.skill_name,
                &composition_context,
            ) {
                reasons.push(reason);
            }

            let severity = skill_negative_entropy_severity(
                snapshot.source_scope,
                signals.as_slice(),
                recent_positive_evolution,
                &composition_context,
            );
            diagnostics.push(SkillNegativeEntropyDiagnostic {
                skill_name: snapshot.skill_name,
                source_scope: snapshot.source_scope,
                source_id: snapshot.source_id,
                signals,
                severity,
                runtime_use_count,
                runtime_error_count,
                write_count,
                last_used_at,
                last_write_at,
                semantic_overlap_count,
                reasons,
            });
        }

        diagnostics.sort_by(|left, right| {
            skill_diagnostic_sort_key(left.severity)
                .cmp(&skill_diagnostic_sort_key(right.severity))
                .then_with(|| {
                    right
                        .semantic_overlap_count
                        .cmp(&left.semantic_overlap_count)
                })
                .then_with(|| right.write_count.cmp(&left.write_count))
                .then_with(|| left.runtime_use_count.cmp(&right.runtime_use_count))
                .then_with(|| left.skill_name.cmp(&right.skill_name))
        });
        Ok(diagnostics)
    }

    pub fn skill_semantic_conflict_diagnostics(
        &self,
    ) -> Result<Vec<SkillSemanticConflictDiagnostic>, SkillError> {
        let (descriptors, snapshot_by_name) = self.skill_semantic_analysis_inputs()?;
        Ok(collect_skill_semantic_conflicts(
            &descriptors,
            &snapshot_by_name,
        ))
    }
}

const SKILL_NEGATIVE_ENTROPY_STALE_SECONDS: i64 = 30 * 24 * 60 * 60;
const SKILL_POSITIVE_EVOLUTION_GRACE_SECONDS: i64 = SKILL_NEGATIVE_ENTROPY_STALE_SECONDS;

fn should_sync_negative_entropy_review_candidate(
    snapshot: Option<&SkillOperationalSnapshot>,
) -> bool {
    !matches!(
        snapshot
            .and_then(|entry| entry.vitality.as_ref())
            .map(|record| record.state),
        Some(
            SkillVitalityState::ReviewCandidate
                | SkillVitalityState::Retired
                | SkillVitalityState::Archived
        )
    )
}

fn negative_entropy_review_candidate_summary(
    diagnostic: &SkillNegativeEntropyDiagnostic,
    context: &SkillCompositionConsumptionContext,
) -> String {
    let base = diagnostic
        .reasons
        .first()
        .cloned()
        .unwrap_or_else(|| "negative entropy review candidate".to_string());
    let Some(canonical_skill_name) = context.related_skill_name_for_review(&diagnostic.skill_name)
    else {
        return base;
    };
    let qualifier = match context.family_member_role {
        Some(SkillCapabilityMemberRole::Specialization) => format!(
            "the skill is governed as a specialization variant under canonical skill `{canonical_skill_name}`"
        ),
        Some(SkillCapabilityMemberRole::MergeCandidate) => format!(
            "the skill is governed as a merge candidate under canonical skill `{canonical_skill_name}`"
        ),
        _ => format!(
            "the skill is governed relative to canonical skill `{canonical_skill_name}`"
        ),
    };
    format!("{base}; {qualifier}")
}

fn should_sync_semantic_conflict_review_candidate(
    snapshot: Option<&SkillOperationalSnapshot>,
    preferred_skill_name: &str,
) -> bool {
    let Some(vitality) = snapshot.and_then(|entry| entry.vitality.as_ref()) else {
        return true;
    };
    match vitality.state {
        SkillVitalityState::Retired | SkillVitalityState::Archived => false,
        SkillVitalityState::Active => true,
        SkillVitalityState::ReviewCandidate => {
            !(vitality.reason.kind == SkillRetirementReasonKind::SemanticConflict
                && vitality
                    .reason
                    .related_skill_name
                    .as_deref()
                    .map(normalize_name)
                    == Some(normalize_name(preferred_skill_name)))
        }
    }
}

fn skill_negative_entropy_severity(
    source_scope: SkillOperationalSourceScope,
    signals: &[SkillNegativeEntropySignal],
    recent_positive_evolution: bool,
    composition_context: &SkillCompositionConsumptionContext,
) -> SkillGovernanceDiagnosticSeverity {
    if recent_positive_evolution {
        return SkillGovernanceDiagnosticSeverity::Info;
    }
    if composition_context.complementary_protected() {
        return SkillGovernanceDiagnosticSeverity::Info;
    }
    if matches!(source_scope, SkillOperationalSourceScope::WorkspaceLocal)
        && signals.iter().any(|signal| {
            matches!(
                signal,
                SkillNegativeEntropySignal::NeverReused
                    | SkillNegativeEntropySignal::StaleUnused
                    | SkillNegativeEntropySignal::WriteHeavyLowReuse
            )
        })
    {
        SkillGovernanceDiagnosticSeverity::Warn
    } else {
        SkillGovernanceDiagnosticSeverity::Info
    }
}

fn total_skill_write_count(entry: &SkillWriteLedgerEntry) -> u64 {
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

fn is_skill_timestamp_stale(timestamp: Option<i64>, now: i64, threshold_seconds: i64) -> bool {
    timestamp
        .map(|value| value > 0 && now.saturating_sub(value) >= threshold_seconds)
        .unwrap_or(false)
}

fn format_skill_positive_evolution_reason(
    evolution: &SkillEvolutionEvidenceSummary,
    now: i64,
) -> String {
    let days_ago = evolution
        .last_positive_signal_at
        .map(|timestamp| now.saturating_sub(timestamp) / 86_400)
        .unwrap_or(0);
    format!(
        "recent memory/proposal evolution signal observed {} day(s) ago ({} memory promotion(s), {} proposal signal(s), {} active draft proposal(s)); review severity is downgraded while the skill is still evolving",
        days_ago,
        evolution.memory_promotion_count,
        evolution.proposal_signal_count,
        evolution.last_observed_draft_proposal_count
    )
}

fn format_negative_entropy_composition_reason(
    skill_name: &str,
    context: &SkillCompositionConsumptionContext,
) -> Option<String> {
    if context.complementary_protected() {
        if context.complementary_peer_skill_names.is_empty() {
            return Some(
                "skill is explicitly governed as a complementary component; low standalone reuse is expected and is not treated as pure redundancy"
                    .to_string(),
            );
        }
        return Some(format!(
            "skill is explicitly governed as a complementary component alongside {}; low standalone reuse is expected and is not treated as pure redundancy",
            context.complementary_peer_skill_names.join(", ")
        ));
    }

    let canonical_skill_name = context.related_skill_name_for_review(skill_name)?;
    let relation_label = match context.family_member_role {
        Some(SkillCapabilityMemberRole::Specialization) => "specialization member",
        Some(SkillCapabilityMemberRole::MergeCandidate) => "merge candidate",
        _ => "family member",
    };
    if let Some(capability_id) = context.canonical_family_id.as_deref() {
        return Some(format!(
            "skill is an explicit {relation_label} in canonical family `{capability_id}` led by `{canonical_skill_name}`; low reuse is evaluated relative to that family owner"
        ));
    }
    Some(format!(
        "skill is an explicit {relation_label} governed relative to canonical skill `{canonical_skill_name}`; low reuse is evaluated relative to that owner"
    ))
}

pub(super) fn format_skill_vitality_state(state: SkillVitalityState) -> &'static str {
    match state {
        SkillVitalityState::Active => "active",
        SkillVitalityState::ReviewCandidate => "review_candidate",
        SkillVitalityState::Retired => "retired",
        SkillVitalityState::Archived => "archived",
    }
}

pub(super) fn format_skill_retirement_reason_kind(kind: SkillRetirementReasonKind) -> &'static str {
    match kind {
        SkillRetirementReasonKind::NegativeEntropy => "negative_entropy",
        SkillRetirementReasonKind::SemanticConflict => "semantic_conflict",
        SkillRetirementReasonKind::ManualOverride => "manual_override",
        SkillRetirementReasonKind::Restored => "restored",
    }
}
