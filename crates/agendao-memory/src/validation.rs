use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Result;
use agendao_storage::{MemoryConflictRecord, MemoryRepository, MemoryRepositoryFilter};
use agendao_types::{
    MemoryKind, MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus, MemoryValidationReport,
    MemoryValidationStatus,
};
use sha2::{Digest, Sha256};

use crate::authority::ResolvedMemoryContext;

#[derive(Clone)]
pub struct MemoryValidationEngine {
    repository: Arc<MemoryRepository>,
}

#[derive(Debug, Clone)]
pub struct MemoryValidationOutcome {
    pub record: MemoryRecord,
    pub report: MemoryValidationReport,
    pub conflicts: Vec<MemoryConflictRecord>,
}

impl MemoryValidationEngine {
    pub fn new(repository: Arc<MemoryRepository>) -> Self {
        Self { repository }
    }

    pub async fn validate_record(
        &self,
        record: &MemoryRecord,
        context: &ResolvedMemoryContext,
    ) -> Result<MemoryValidationOutcome> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut candidate = record.clone();
        let mut issues = Vec::new();
        let mut failed = false;
        let mut warning = false;

        if !context.allowed_scopes.contains(&candidate.scope) {
            failed = true;
            issues.push(format!(
                "scope_illegal:{} is not allowed for workspace mode {:?}",
                scope_label(&candidate.scope),
                context.workspace_mode
            ));
        }

        if candidate.title.trim().len() < 8 {
            warning = true;
            issues.push("completeness:title_too_short".to_string());
        }
        if candidate.summary.trim().len() < 16 {
            warning = true;
            issues.push("completeness:summary_too_short".to_string());
        }
        if candidate.evidence_refs.is_empty() {
            warning = true;
            issues.push("completeness:missing_evidence".to_string());
        }
        if candidate.trigger_conditions.is_empty() {
            warning = true;
            issues.push("completeness:missing_triggers".to_string());
        }
        if matches!(
            candidate.scope,
            MemoryScope::WorkspaceShared | MemoryScope::WorkspaceSandbox
        ) && candidate
            .workspace_identity
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            failed = true;
            issues.push("scope:missing_workspace_identity".to_string());
        }
        if candidate.kind == MemoryKind::MethodologyCandidate {
            if count_validation_terms(&candidate) == 0 {
                warning = true;
                issues.push("methodology:missing_validation_recipe".to_string());
            }
            if !has_non_goal_boundary(&candidate) {
                warning = true;
                issues.push("methodology:missing_non_goal_boundary".to_string());
            }
        }
        if let Some(linked_skill_name) = normalized_optional(&candidate.linked_skill_name) {
            match linked_skill_signal(&candidate) {
                Some(signal) if signal != linked_skill_name => {
                    failed = true;
                    issues.push(format!(
                        "skill_link:mismatch:{}!={}",
                        signal, linked_skill_name
                    ));
                }
                None => {
                    warning = true;
                    issues.push("skill_link:missing_skill_signal".to_string());
                }
                _ => {}
            }
        }
        if candidate.kind == MemoryKind::Lesson
            && candidate
                .linked_skill_name
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            && !has_skill_feedback_signal(&candidate)
        {
            warning = true;
            issues.push("skill_feedback:missing_failure_or_change_signal".to_string());
        }

        if candidate
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            failed = true;
            issues.push("staleness:expired_record".to_string());
        } else if candidate.status == MemoryStatus::Candidate
            && candidate.updated_at < now - 1000 * 60 * 60 * 24 * 30
        {
            warning = true;
            issues.push("staleness:stale_candidate".to_string());
        }

        if contains_unsafe_memory_content(&candidate) {
            failed = true;
            issues.push("unsafe_content:prompt_injection_like_content".to_string());
        }

        let peers = self
            .repository
            .list_records(Some(&MemoryRepositoryFilter {
                workspace_identity: candidate.workspace_identity.clone(),
                scopes: context.allowed_scopes.clone(),
                statuses: vec![
                    MemoryStatus::Candidate,
                    MemoryStatus::Validated,
                    MemoryStatus::Consolidated,
                ],
                limit: Some(500),
                ..MemoryRepositoryFilter::default()
            }))
            .await?;

        let mut conflicts = Vec::new();
        let candidate_signature = canonical_signature(&candidate);
        let candidate_fact_map = normalized_fact_map(&candidate);
        let candidate_context_terms = context_terms(&candidate);

        for peer in peers {
            if peer.id == candidate.id || peer.status == MemoryStatus::Rejected {
                continue;
            }

            if canonical_signature(&peer) == candidate_signature {
                failed = true;
                issues.push(format!("dedup:duplicate_of={}", peer.id.0));
                conflicts.push(build_conflict(
                    &candidate.id,
                    &peer.id,
                    "duplicate",
                    "Canonical memory signature matches an existing record.",
                    now,
                ));
                continue;
            }

            if contradiction_exists(&candidate_fact_map, &normalized_fact_map(&peer))
                && !candidate_context_terms.is_disjoint(&context_terms(&peer))
            {
                warning = true;
                issues.push(format!("contradiction:conflicts_with={}", peer.id.0));
                conflicts.push(build_conflict(
                    &candidate.id,
                    &peer.id,
                    "contradiction",
                    "Overlapping context has conflicting normalized facts.",
                    now,
                ));
            }
        }

        candidate.last_validated_at = Some(now);
        candidate.updated_at = now;

        let report_status = if failed {
            candidate.validation_status = MemoryValidationStatus::Failed;
            if issues
                .iter()
                .any(|issue| issue.starts_with("staleness:expired"))
            {
                candidate.status = MemoryStatus::Archived;
            } else {
                candidate.status = MemoryStatus::Rejected;
            }
            MemoryValidationStatus::Failed
        } else if warning {
            candidate.validation_status = MemoryValidationStatus::Warning;
            if candidate.status != MemoryStatus::Consolidated {
                candidate.status = MemoryStatus::Candidate;
            }
            MemoryValidationStatus::Warning
        } else {
            candidate.validation_status = MemoryValidationStatus::Passed;
            if candidate.status == MemoryStatus::Candidate {
                candidate.status = MemoryStatus::Validated;
            }
            MemoryValidationStatus::Passed
        };

        Ok(MemoryValidationOutcome {
            record: candidate,
            report: MemoryValidationReport {
                record_id: Some(record.id.clone()),
                status: report_status,
                issues,
                checked_at: now,
            },
            conflicts,
        })
    }

    pub async fn validate_and_apply(
        &self,
        record: &MemoryRecord,
        context: &ResolvedMemoryContext,
    ) -> Result<MemoryValidationOutcome> {
        let outcome = self.validate_record(record, context).await?;
        self.repository.upsert_record(&outcome.record).await?;
        self.repository
            .record_validation_run(&outcome.report)
            .await?;
        self.repository
            .replace_conflicts_for_memory(&outcome.record.id.0, &outcome.conflicts)
            .await?;
        Ok(outcome)
    }

    pub async fn validate_record_by_id(
        &self,
        record_id: &MemoryRecordId,
        context: &ResolvedMemoryContext,
    ) -> Result<Option<MemoryValidationOutcome>> {
        let Some(record) = self.repository.get_record(&record_id.0).await? else {
            return Ok(None);
        };
        Ok(Some(self.validate_and_apply(&record, context).await?))
    }
}

fn contains_unsafe_memory_content(record: &MemoryRecord) -> bool {
    let haystack = [
        record.title.as_str(),
        record.summary.as_str(),
        &record.trigger_conditions.join("\n"),
        &record.normalized_facts.join("\n"),
        &record.boundaries.join("\n"),
    ]
    .join("\n")
    .to_ascii_lowercase();

    [
        "ignore previous instructions",
        "disregard all prior",
        "<system_reminder>",
        "<system>",
        "developer message",
        "system prompt",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn normalized_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn linked_skill_signal(record: &MemoryRecord) -> Option<String> {
    for trigger in &record.trigger_conditions {
        let trimmed = trigger.trim();
        if let Some(skill_name) = trimmed.strip_prefix("skill:") {
            let normalized = skill_name.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                return Some(normalized);
            }
        }
    }
    for fact in &record.normalized_facts {
        if let Some((key, value)) = fact.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            if matches!(key.as_str(), "skill_name" | "linked_skill_name") {
                let normalized = value.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    return Some(normalized);
                }
            }
        }
    }
    None
}

fn has_non_goal_boundary(record: &MemoryRecord) -> bool {
    record.boundaries.iter().any(|boundary| {
        let normalized = boundary.trim().to_ascii_lowercase();
        [
            "do not",
            "don't",
            "only",
            "unless",
            "requires",
            "avoid",
            "non-goal",
            "out of scope",
            "not for",
        ]
        .iter()
        .any(|term| normalized.contains(term))
    })
}

fn has_skill_feedback_signal(record: &MemoryRecord) -> bool {
    [
        record.title.as_str(),
        record.summary.as_str(),
        &record.trigger_conditions.join("\n"),
        &record.normalized_facts.join("\n"),
        &record.boundaries.join("\n"),
    ]
    .join("\n")
    .to_ascii_lowercase()
    .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
    .any(skill_feedback_word_matches)
}

fn count_validation_terms(record: &MemoryRecord) -> usize {
    record
        .trigger_conditions
        .iter()
        .chain(record.normalized_facts.iter())
        .map(|value| value.to_ascii_lowercase())
        .map(|value| {
            value
                .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
                .filter(|word| validation_word_matches(word))
                .count()
        })
        .sum()
}

fn validation_word_matches(word: &str) -> bool {
    matches!(
        word,
        "test"
            | "tests"
            | "check"
            | "checks"
            | "verify"
            | "verified"
            | "validate"
            | "validation"
            | "audit"
            | "probe"
            | "lint"
            | "diagnostic"
            | "diagnostics"
            | "doctor"
            | "healthcheck"
            | "health-check"
            | "smoketest"
            | "smoke-test"
            | "selftest"
            | "self-test"
    )
}

fn skill_feedback_word_matches(word: &str) -> bool {
    matches!(
        word,
        "fail"
            | "failed"
            | "failure"
            | "error"
            | "warn"
            | "warning"
            | "patch"
            | "patched"
            | "fix"
            | "fixed"
            | "change"
            | "changed"
            | "update"
            | "updated"
            | "guard"
            | "violation"
            | "regression"
            | "supporting_file"
    )
}

fn canonical_signature(record: &MemoryRecord) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope_label(&record.scope).as_bytes());
    hasher.update([0]);
    hasher.update(record.title.trim().to_ascii_lowercase().as_bytes());
    hasher.update([0]);
    hasher.update(record.summary.trim().to_ascii_lowercase().as_bytes());
    hasher.update([0]);

    for value in sorted_lowercase(&record.trigger_conditions) {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    for value in sorted_lowercase(&record.normalized_facts) {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }

    format!("{:x}", hasher.finalize())
}

fn normalized_fact_map(record: &MemoryRecord) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for fact in &record.normalized_facts {
        if let Some((key, value)) = fact.split_once('=') {
            map.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().to_ascii_lowercase(),
            );
        }
    }
    map
}

fn context_terms(record: &MemoryRecord) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for trigger in &record.trigger_conditions {
        terms.insert(trigger.trim().to_ascii_lowercase());
    }
    for fact in &record.normalized_facts {
        if let Some((key, _)) = fact.split_once('=') {
            terms.insert(key.trim().to_ascii_lowercase());
        } else {
            terms.insert(fact.trim().to_ascii_lowercase());
        }
    }
    terms
}

fn contradiction_exists(
    candidate: &BTreeMap<String, String>,
    peer: &BTreeMap<String, String>,
) -> bool {
    candidate
        .iter()
        .any(|(key, value)| peer.get(key).is_some_and(|peer_value| peer_value != value))
}

fn build_conflict(
    left: &MemoryRecordId,
    right: &MemoryRecordId,
    kind: &str,
    detail: &str,
    detected_at: i64,
) -> MemoryConflictRecord {
    let mut hasher = Sha256::new();
    hasher.update(left.0.as_bytes());
    hasher.update([0]);
    hasher.update(right.0.as_bytes());
    hasher.update([0]);
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    let id = format!("mem_conflict_{:x}", hasher.finalize());

    MemoryConflictRecord {
        id: id[..36.min(id.len())].to_string(),
        left_memory_id: left.0.clone(),
        right_memory_id: right.0.clone(),
        conflict_kind: kind.to_string(),
        detail: detail.to_string(),
        detected_at,
    }
}

fn sorted_lowercase(values: &[String]) -> Vec<String> {
    let mut items: Vec<String> = values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    items.sort();
    items
}

fn scope_label(scope: &MemoryScope) -> &'static str {
    match scope {
        MemoryScope::GlobalUser => "global_user",
        MemoryScope::GlobalWorkspace => "global_workspace",
        MemoryScope::WorkspaceShared => "workspace_shared",
        MemoryScope::WorkspaceSandbox => "workspace_sandbox",
        MemoryScope::SessionEphemeral => "session_ephemeral",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agendao_config::WorkspaceMode;
    use agendao_storage::{Database, MemoryRepository};
    use agendao_types::{MemoryEvidenceRef, MemoryKind};

    use super::*;

    fn context() -> ResolvedMemoryContext {
        ResolvedMemoryContext {
            workspace_key: "ws:test".to_string(),
            workspace_mode: WorkspaceMode::Shared,
            allowed_scopes: vec![
                MemoryScope::GlobalUser,
                MemoryScope::GlobalWorkspace,
                MemoryScope::WorkspaceShared,
            ],
        }
    }

    fn base_record(id: &str, kind: MemoryKind) -> MemoryRecord {
        MemoryRecord {
            id: MemoryRecordId(id.to_string()),
            kind,
            scope: MemoryScope::WorkspaceShared,
            status: MemoryStatus::Candidate,
            title: "Reusable provider refresh workflow".to_string(),
            summary: "Refresh provider catalog, validate the response, and patch related skill guidance when the runtime changes.".to_string(),
            trigger_conditions: vec!["provider:refresh".to_string()],
            normalized_facts: vec!["step=refresh".to_string(), "workspace_mode=shared".to_string()],
            boundaries: vec!["Only use in this workspace.".to_string()],
            confidence: Some(0.7),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some("ses_validation".to_string()),
                message_id: None,
                tool_call_id: Some("call_validation".to_string()),
                stage_id: Some("stage_validation".to_string()),
                note: Some("fixture".to_string()),
            }],
            source_session_id: Some("ses_validation".to_string()),
            workspace_identity: Some("ws:test".to_string()),
            created_at: 10,
            updated_at: 10,
            last_validated_at: None,
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Pending,
        }
    }

    #[tokio::test]
    async fn methodology_without_validation_recipe_warns() {
        let db = Database::in_memory().await.expect("db should initialize");
        let engine =
            MemoryValidationEngine::new(Arc::new(MemoryRepository::new(db.pool().clone())));

        let mut record = base_record("mem_methodology_warning", MemoryKind::MethodologyCandidate);
        record.boundaries = vec!["Use this flow when provider metadata drifts.".to_string()];

        let outcome = engine
            .validate_record(&record, &context())
            .await
            .expect("validation should succeed");

        assert_eq!(outcome.report.status, MemoryValidationStatus::Warning);
        assert!(outcome
            .report
            .issues
            .iter()
            .any(|issue| issue == "methodology:missing_validation_recipe"));
        assert!(outcome
            .report
            .issues
            .iter()
            .any(|issue| issue == "methodology:missing_non_goal_boundary"));
    }

    #[tokio::test]
    async fn linked_skill_signal_mismatch_fails() {
        let db = Database::in_memory().await.expect("db should initialize");
        let engine =
            MemoryValidationEngine::new(Arc::new(MemoryRepository::new(db.pool().clone())));

        let mut record = base_record("mem_skill_link_fail", MemoryKind::Lesson);
        record.linked_skill_name = Some("provider-refresh".to_string());
        record
            .trigger_conditions
            .push("skill:frontend-ui-ux".to_string());
        record
            .normalized_facts
            .push("linked_skill_name=frontend-ui-ux".to_string());
        record.summary =
            "Skill patch failed and required a new supporting file before validation passed."
                .to_string();

        let outcome = engine
            .validate_record(&record, &context())
            .await
            .expect("validation should succeed");

        assert_eq!(outcome.report.status, MemoryValidationStatus::Failed);
        assert!(outcome
            .report
            .issues
            .iter()
            .any(|issue| issue.starts_with("skill_link:mismatch:")));
    }
}
