use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    EnvironmentFact,
    WorkspaceConvention,
    Lesson,
    Pattern,
    MethodologyCandidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    GlobalUser,
    GlobalWorkspace,
    WorkspaceShared,
    WorkspaceSandbox,
    SessionEphemeral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    #[default]
    Candidate,
    Validated,
    Consolidated,
    Archived,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryValidationStatus {
    #[default]
    Pending,
    Passed,
    Warning,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MemoryRecordId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MemoryEvidenceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    pub id: MemoryRecordId,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    #[serde(default)]
    pub status: MemoryStatus,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_conditions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normalized_facts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundaries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<MemoryEvidenceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_identity: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_skill_name: Option<String>,
    #[serde(default)]
    pub validation_status: MemoryValidationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCardView {
    pub id: MemoryRecordId,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    #[serde(default)]
    pub status: MemoryStatus,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub validation_status: MemoryValidationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at: Option<i64>,
}

impl From<&MemoryRecord> for MemoryCardView {
    fn from(value: &MemoryRecord) -> Self {
        Self {
            id: value.id.clone(),
            kind: value.kind.clone(),
            scope: value.scope.clone(),
            status: value.status.clone(),
            title: value.title.clone(),
            summary: value.summary.clone(),
            derived_skill_name: value.derived_skill_name.clone(),
            linked_skill_name: value.linked_skill_name.clone(),
            confidence: value.confidence,
            validation_status: value.validation_status.clone(),
            last_validated_at: value.last_validated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecallView {
    pub card: MemoryCardView,
    pub why_recalled: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryDetailView {
    pub record: MemoryRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MemoryRetrievalQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<MemoryKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<MemoryScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MemoryListQuery {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<MemoryKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<MemoryScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<MemoryStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRetrievalPacket {
    pub generated_at: i64,
    #[serde(default)]
    pub snapshot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<MemoryScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<MemoryRecallView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryValidationReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<MemoryRecordId>,
    #[serde(default)]
    pub status: MemoryValidationStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryValidationReportResponse {
    pub record_id: MemoryRecordId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<MemoryValidationReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryConflictView {
    pub id: String,
    pub record_id: MemoryRecordId,
    pub other_record_id: MemoryRecordId,
    pub conflict_kind: String,
    pub detail: String,
    pub detected_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryConflictResponse {
    pub record_id: MemoryRecordId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<MemoryConflictView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryConsolidationRun {
    pub run_id: String,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub merged_count: u32,
    #[serde(default)]
    pub promoted_count: u32,
    #[serde(default)]
    pub conflict_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRulePackKind {
    Validation,
    Consolidation,
    Reflection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRuleDefinition {
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_target: Option<MemoryKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRulePack {
    pub id: String,
    pub rule_pack_kind: MemoryRulePackKind,
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<MemoryRuleDefinition>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRuleHit {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_pack_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<MemoryRecordId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub hit_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MemoryConsolidationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_candidates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MemoryConsolidationRunQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MemoryRuleHitQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<MemoryRecordId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryConsolidationResponse {
    pub run: MemoryConsolidationRun,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_record_ids: Vec<MemoryRecordId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promoted_record_ids: Vec<MemoryRecordId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_record_ids: Vec<MemoryRecordId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reflection_notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_hits: Vec<MemoryRuleHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryConsolidationRunListResponse {
    pub items: Vec<MemoryConsolidationRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRulePackListResponse {
    pub items: Vec<MemoryRulePack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRuleHitListResponse {
    pub items: Vec<MemoryRuleHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryContract {
    pub filter_query_parameters: Vec<String>,
    pub search_fields: Vec<String>,
    pub non_search_fields: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryListResponse {
    pub items: Vec<MemoryCardView>,
    pub contract: MemoryContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRetrievalPreviewResponse {
    pub packet: MemoryRetrievalPacket,
    pub contract: MemoryContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMemoryTelemetrySummary {
    pub workspace_key: String,
    pub workspace_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_scopes: Vec<MemoryScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_snapshot_generated_at: Option<i64>,
    #[serde(default)]
    pub frozen_snapshot_items: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prefetch_generated_at: Option<i64>,
    #[serde(default)]
    pub last_prefetch_items: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prefetch_query: Option<String>,
    #[serde(default)]
    pub candidate_count: u32,
    #[serde(default)]
    pub validated_count: u32,
    #[serde(default)]
    pub rejected_count: u32,
    #[serde(default)]
    pub warning_count: u32,
    #[serde(default)]
    pub methodology_candidate_count: u32,
    #[serde(default)]
    pub derived_skill_candidate_count: u32,
    #[serde(default)]
    pub linked_skill_count: u32,
    #[serde(default)]
    pub skill_feedback_lesson_count: u32,
    #[serde(default)]
    pub retrieval_run_count: u32,
    #[serde(default)]
    pub retrieval_hit_count: u32,
    #[serde(default)]
    pub retrieval_use_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_consolidation_run: Option<MemoryConsolidationRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_rule_hits: Vec<MemoryRuleHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMemoryInsight {
    pub summary: SessionMemoryTelemetrySummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_snapshot: Option<MemoryRetrievalPacket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prefetch_packet: Option<MemoryRetrievalPacket>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_session_records: Vec<MemoryCardView>,
}

// ── Skill Evolution Proposals ──────────────────────────────────────────────
//
// Evidence-backed proposals for skill lifecycle changes, generated from
// methodology candidates after consolidation. Each proposal is a reviewed,
// deterministic suggestion — not an automatic patch.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillEvolutionProposalKind {
    PatchExistingSkill,
    CreateNewSkill,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    Accepted,
    Rejected,
    Superseded,
    Applied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuggestedSkillChange {
    AddTriggerCondition {
        text: String,
        evidence_refs: Vec<String>,
    },
    AddCoreStep {
        text: String,
        evidence_refs: Vec<String>,
    },
    AddBoundary {
        text: String,
        evidence_refs: Vec<String>,
    },
    AddValidationStep {
        text: String,
        evidence_refs: Vec<String>,
    },
    CreateSkillDraft {
        suggested_name: String,
        when_to_use: Vec<String>,
        core_steps: Vec<String>,
        boundaries: Vec<String>,
        validation: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEvolutionProposal {
    pub id: String,
    pub session_id: String,
    pub memory_record_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_skill_name: Option<String>,
    pub proposal_kind: SkillEvolutionProposalKind,
    pub title: String,
    pub rationale: String,
    pub suggested_changes: Vec<SuggestedSkillChange>,
    pub status: ProposalStatus,
    pub evidence_hash: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillEvolutionProposalGenerationSummary {
    #[serde(default)]
    pub proposals_created: u32,
    #[serde(default)]
    pub proposals_skipped: u32,
    #[serde(default)]
    pub methodology_candidates_seen: u32,
}

impl SkillEvolutionProposal {
    pub fn compute_evidence_hash(
        kind: &SkillEvolutionProposalKind,
        linked_skill_name: Option<&str>,
        memory_record_ids: &[String],
        changes: &[SuggestedSkillChange],
    ) -> String {
        use sha2::{Digest, Sha256};
        let mut sorted_ids = memory_record_ids.to_vec();
        sorted_ids.sort();
        // Use serde JSON for stable hashing — Debug output is not
        // guaranteed stable across compiler versions or refactors.
        let payload = serde_json::json!({
            "kind": kind,
            "linked_skill_name": linked_skill_name,
            "memory_record_ids": sorted_ids,
            "changes": changes,
        });
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())[..24].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_skill_change_serde_roundtrip() {
        let changes = vec![
            SuggestedSkillChange::AddTriggerCondition {
                text: "cargo test fails after edit".to_string(),
                evidence_refs: vec!["session=ses_1 · tool=call_1".to_string()],
            },
            SuggestedSkillChange::AddBoundary {
                text: "do not refactor before reproducing".to_string(),
                evidence_refs: vec![],
            },
        ];
        let json = serde_json::to_string(&changes).unwrap();
        let roundtripped: Vec<SuggestedSkillChange> = serde_json::from_str(&json).unwrap();
        assert_eq!(changes, roundtripped);
    }

    #[test]
    fn evidence_hash_deterministic() {
        let changes = vec![SuggestedSkillChange::AddCoreStep {
            text: "run cargo test".to_string(),
            evidence_refs: vec!["a".to_string()],
        }];
        let h1 = SkillEvolutionProposal::compute_evidence_hash(
            &SkillEvolutionProposalKind::PatchExistingSkill,
            Some("my-skill"),
            &["mem_1".to_string()],
            &changes,
        );
        let h2 = SkillEvolutionProposal::compute_evidence_hash(
            &SkillEvolutionProposalKind::PatchExistingSkill,
            Some("my-skill"),
            &["mem_1".to_string()],
            &changes,
        );
        assert_eq!(h1, h2);
    }

    #[test]
    fn evidence_hash_differs_on_different_kind() {
        let changes = vec![];
        let h1 = SkillEvolutionProposal::compute_evidence_hash(
            &SkillEvolutionProposalKind::PatchExistingSkill,
            None,
            &[],
            &changes,
        );
        let h2 = SkillEvolutionProposal::compute_evidence_hash(
            &SkillEvolutionProposalKind::CreateNewSkill,
            None,
            &[],
            &changes,
        );
        assert_ne!(h1, h2);
    }

    #[test]
    fn evidence_hash_memory_order_independent() {
        let changes = vec![];
        let h1 = SkillEvolutionProposal::compute_evidence_hash(
            &SkillEvolutionProposalKind::PatchExistingSkill,
            None,
            &["b".to_string(), "a".to_string()],
            &changes,
        );
        let h2 = SkillEvolutionProposal::compute_evidence_hash(
            &SkillEvolutionProposalKind::PatchExistingSkill,
            None,
            &["a".to_string(), "b".to_string()],
            &changes,
        );
        assert_eq!(h1, h2);
    }

    #[test]
    fn proposal_status_serde_roundtrip() {
        for status in &[
            ProposalStatus::Draft,
            ProposalStatus::Accepted,
            ProposalStatus::Rejected,
            ProposalStatus::Superseded,
            ProposalStatus::Applied,
        ] {
            let json = serde_json::to_string(status).unwrap();
            let back: ProposalStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, back);
        }
    }
}
