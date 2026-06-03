use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;
use agendao_storage::{MemoryRepository, MemoryRepositoryFilter};
use agendao_types::{
    MemoryConsolidationRequest, MemoryConsolidationResponse, MemoryConsolidationRun,
    MemoryConsolidationRunQuery, MemoryKind, MemoryRecord, MemoryRecordId, MemoryRuleHit,
    MemoryRuleHitQuery, MemoryRulePack, MemoryScope, MemoryStatus, MemoryValidationStatus,
};
use sha2::{Digest, Sha256};

use crate::authority::ResolvedMemoryContext;
use crate::rules::builtin_rule_packs;

const CONSOLIDATION_PACK_ID: &str = "builtin.memory.consolidation.core";
const REFLECTION_PACK_ID: &str = "builtin.memory.reflection.core";

#[derive(Clone)]
pub struct MemoryConsolidationEngine {
    repository: Arc<MemoryRepository>,
}

impl MemoryConsolidationEngine {
    pub fn new(repository: Arc<MemoryRepository>) -> Self {
        Self { repository }
    }

    pub async fn list_rule_packs(&self) -> Result<Vec<MemoryRulePack>> {
        self.ensure_builtin_rule_packs().await
    }

    pub async fn list_rule_hits(&self, query: &MemoryRuleHitQuery) -> Result<Vec<MemoryRuleHit>> {
        Ok(self.repository.list_rule_hits(Some(query)).await?)
    }

    pub async fn list_consolidation_runs(
        &self,
        query: &MemoryConsolidationRunQuery,
    ) -> Result<Vec<MemoryConsolidationRun>> {
        Ok(self
            .repository
            .list_consolidation_runs(query.limit.map(|value| value as i64))
            .await?)
    }

    pub async fn run_consolidation(
        &self,
        context: &ResolvedMemoryContext,
        request: &MemoryConsolidationRequest,
    ) -> Result<MemoryConsolidationResponse> {
        let now = chrono::Utc::now().timestamp_millis();
        let run_id = consolidation_run_id(&context.workspace_key, now);
        self.ensure_builtin_rule_packs().await?;

        let mut statuses = vec![MemoryStatus::Validated, MemoryStatus::Consolidated];
        if request.include_candidates {
            statuses.push(MemoryStatus::Candidate);
        }
        let records = self
            .repository
            .list_records(Some(&MemoryRepositoryFilter {
                scopes: context.allowed_scopes.clone(),
                statuses,
                workspace_identity: Some(context.workspace_key.clone()),
                limit: Some(request.limit.unwrap_or(400) as i64),
                ..MemoryRepositoryFilter::default()
            }))
            .await?;

        let mut record_index: HashMap<String, MemoryRecord> = records
            .into_iter()
            .map(|record| (record.id.0.clone(), record))
            .collect();
        let mut merged_record_ids = Vec::new();
        let mut promoted_record_ids = Vec::new();
        let mut archived_record_ids = Vec::new();
        let mut reflection_notes = Vec::new();
        let mut rule_hits = Vec::new();
        let mut merged_count = 0u32;
        let mut promoted_count = 0u32;

        for group in merge_groups(record_index.values().cloned().collect()) {
            let Some(primary_id) = select_primary_id(&group) else {
                continue;
            };
            let Some(primary) = record_index.get(&primary_id).cloned() else {
                continue;
            };
            let merged = merged_record(&primary, &group, now);
            self.repository.upsert_record(&merged).await?;
            record_index.insert(merged.id.0.clone(), merged.clone());
            merged_record_ids.push(merged.id.clone());
            rule_hits.push(rule_hit(
                &run_id,
                Some(CONSOLIDATION_PACK_ID),
                Some(merged.id.clone()),
                "merge.similar.summary",
                Some(format!(
                    "Merged {} related records into {}.",
                    group.len(),
                    merged.id.0
                )),
                now,
            ));

            for peer in group {
                if peer.id == merged.id {
                    continue;
                }
                let archived = archive_after_merge(&peer, &merged.id, now);
                self.repository.upsert_record(&archived).await?;
                archived_record_ids.push(archived.id.clone());
                record_index.insert(archived.id.0.clone(), archived.clone());
                rule_hits.push(rule_hit(
                    &run_id,
                    Some(CONSOLIDATION_PACK_ID),
                    Some(archived.id.clone()),
                    "merge.similar.summary",
                    Some(format!("Archived after merge into {}.", merged.id.0)),
                    now,
                ));
                merged_count += 1;
            }
        }

        for cluster in lesson_clusters(record_index.values().cloned().collect()) {
            let pattern = build_pattern_record(&cluster, &context.workspace_key, now);
            let existing = self.repository.get_record(&pattern.id.0).await?;
            if existing.as_ref() != Some(&pattern) {
                self.repository.upsert_record(&pattern).await?;
                record_index.insert(pattern.id.0.clone(), pattern.clone());
                promoted_record_ids.push(pattern.id.clone());
                promoted_count += 1;
            }
            if let Some(linked_skill_name) = pattern.linked_skill_name.as_deref() {
                rule_hits.push(rule_hit(
                    &run_id,
                    Some(CONSOLIDATION_PACK_ID),
                    Some(pattern.id.clone()),
                    "promotion.pattern.from_skill_feedback",
                    Some(format!(
                        "Repeated lesson cluster converged on linked skill `{}` and produced pattern {}.",
                        linked_skill_name, pattern.id.0
                    )),
                    now,
                ));
            }

            for lesson in cluster {
                let consolidated = mark_record_as_consolidated_source(
                    &lesson,
                    &pattern.id,
                    "Derived into consolidated pattern.",
                    now,
                );
                self.repository.upsert_record(&consolidated).await?;
                record_index.insert(consolidated.id.0.clone(), consolidated.clone());
                rule_hits.push(rule_hit(
                    &run_id,
                    Some(CONSOLIDATION_PACK_ID),
                    Some(consolidated.id.clone()),
                    "promotion.pattern.from_repeated_lessons",
                    Some(format!(
                        "Contributed to consolidated pattern {}.",
                        pattern.id.0
                    )),
                    now,
                ));
            }
        }

        for pattern in methodology_sources(record_index.values().cloned().collect()) {
            let methodology = build_methodology_record(&pattern, &context.workspace_key, now);
            let existing = self.repository.get_record(&methodology.id.0).await?;
            if existing.as_ref() != Some(&methodology) {
                self.repository.upsert_record(&methodology).await?;
                promoted_record_ids.push(methodology.id.clone());
                promoted_count += 1;
            }
            rule_hits.push(rule_hit(
                &run_id,
                Some(CONSOLIDATION_PACK_ID),
                Some(pattern.id.clone()),
                "promotion.methodology.from_structured_pattern",
                Some(format!(
                    "Promoted structured pattern {} into methodology candidate {}.",
                    pattern.id.0, methodology.id.0
                )),
                now,
            ));
            if let Some(linked_skill_name) = methodology.linked_skill_name.as_deref() {
                rule_hits.push(rule_hit(
                    &run_id,
                    Some(REFLECTION_PACK_ID),
                    Some(methodology.id.clone()),
                    "reflection.methodology.skill_patch_candidate",
                    Some(format!(
                        "Methodology candidate {} looks ready to patch or refine existing skill `{}`.",
                        methodology.id.0, linked_skill_name
                    )),
                    now,
                ));
            }
        }

        reflection_notes.extend(build_reflection_notes(
            merged_count,
            promoted_count,
            &record_index,
            request.include_candidates,
        ));

        for note in &reflection_notes {
            rule_hits.push(rule_hit(
                &run_id,
                Some(REFLECTION_PACK_ID),
                None,
                "reflection.extract_methodology_scope",
                Some(note.clone()),
                now,
            ));
        }

        let run = MemoryConsolidationRun {
            run_id: run_id.clone(),
            started_at: now,
            finished_at: Some(now),
            merged_count,
            promoted_count,
            conflict_count: 0,
        };
        self.repository.record_consolidation_run(&run).await?;
        self.repository.record_rule_hits(&rule_hits).await?;

        Ok(MemoryConsolidationResponse {
            run,
            merged_record_ids,
            promoted_record_ids,
            archived_record_ids,
            reflection_notes,
            rule_hits,
        })
    }

    async fn ensure_builtin_rule_packs(&self) -> Result<Vec<MemoryRulePack>> {
        let now = chrono::Utc::now().timestamp_millis();
        for pack in builtin_rule_packs(now) {
            self.repository.upsert_rule_pack(&pack).await?;
        }
        Ok(self.repository.list_rule_packs(None).await?)
    }
}

fn merge_groups(records: Vec<MemoryRecord>) -> Vec<Vec<MemoryRecord>> {
    let mut grouped = BTreeMap::<String, Vec<MemoryRecord>>::new();
    for record in records {
        if !eligible_for_merge(&record) {
            continue;
        }
        let key = format!(
            "{}|{}|{}|{}",
            kind_key(&record.kind),
            scope_key(&record.scope),
            record.workspace_identity.as_deref().unwrap_or(""),
            normalize_text(&record.title)
        );
        grouped.entry(key).or_default().push(record);
    }

    grouped
        .into_values()
        .filter_map(|group| {
            if group.len() < 2 {
                return None;
            }
            let primary = group
                .iter()
                .max_by_key(|record| merge_priority(record))
                .cloned()?;
            let filtered = group
                .into_iter()
                .filter(|record| {
                    record.id == primary.id || shared_signal_count(record, &primary) >= 1
                })
                .collect::<Vec<_>>();
            (filtered.len() >= 2).then_some(filtered)
        })
        .collect()
}

fn lesson_clusters(records: Vec<MemoryRecord>) -> Vec<Vec<MemoryRecord>> {
    let mut grouped = BTreeMap::<String, Vec<MemoryRecord>>::new();
    for record in records {
        if !eligible_for_pattern_promotion(&record) {
            continue;
        }
        let key = lesson_cluster_key(&record);
        grouped.entry(key).or_default().push(record);
    }

    grouped
        .into_values()
        .filter(|group| group.len() >= 2)
        .collect()
}

fn methodology_sources(records: Vec<MemoryRecord>) -> Vec<MemoryRecord> {
    records
        .into_iter()
        .filter(|record| {
            record.kind == MemoryKind::Pattern
                && matches!(
                    record.status,
                    MemoryStatus::Validated | MemoryStatus::Consolidated
                )
                && record.validation_status == MemoryValidationStatus::Passed
                && methodology_ready(record)
        })
        .collect()
}

fn eligible_for_merge(record: &MemoryRecord) -> bool {
    record.kind != MemoryKind::Lesson
        && matches!(
            record.status,
            MemoryStatus::Validated | MemoryStatus::Consolidated | MemoryStatus::Candidate
        )
        && record.validation_status != MemoryValidationStatus::Failed
        && !normalize_text(&record.title).is_empty()
        && record.status != MemoryStatus::Archived
}

fn eligible_for_pattern_promotion(record: &MemoryRecord) -> bool {
    record.kind == MemoryKind::Lesson
        && matches!(
            record.status,
            MemoryStatus::Validated | MemoryStatus::Consolidated | MemoryStatus::Candidate
        )
        && matches!(
            record.validation_status,
            MemoryValidationStatus::Passed | MemoryValidationStatus::Warning
        )
}

fn methodology_ready(record: &MemoryRecord) -> bool {
    let validation_terms = count_validation_terms(record);
    record.trigger_conditions.len() >= 1
        && record.normalized_facts.len() >= 2
        && record.boundaries.len() >= 2
        && has_non_goal_boundary(record)
        && record.evidence_refs.len() >= 2
        && validation_terms >= 1
}

fn build_pattern_record(cluster: &[MemoryRecord], workspace_key: &str, now: i64) -> MemoryRecord {
    let title_seed = cluster
        .iter()
        .map(|record| normalize_title_hint(&record.title))
        .find(|title| !title.is_empty())
        .unwrap_or_else(|| "repeated lesson cluster".to_string());
    let pattern_id = stable_id(
        "mem_pattern",
        &[workspace_key, &title_seed, &lesson_cluster_key(&cluster[0])],
    );
    let summary = format!(
        "Consolidated pattern from {} related lesson memories. Repeated signals: {}.",
        cluster.len(),
        join_top_values(
            cluster
                .iter()
                .flat_map(|record| record.normalized_facts.iter())
        )
    );

    MemoryRecord {
        id: pattern_id,
        kind: MemoryKind::Pattern,
        scope: cluster[0].scope.clone(),
        status: MemoryStatus::Consolidated,
        title: format!("Pattern: {}", title_seed),
        summary,
        trigger_conditions: unique_values(
            cluster
                .iter()
                .flat_map(|record| record.trigger_conditions.clone()),
        ),
        normalized_facts: unique_values(
            cluster
                .iter()
                .flat_map(|record| record.normalized_facts.clone()),
        ),
        boundaries: unique_values(
            cluster
                .iter()
                .flat_map(|record| record.boundaries.clone())
                .chain(std::iter::once(
                    "Derived from repeated validated lessons; verify live state with tools."
                        .to_string(),
                )),
        ),
        confidence: Some(0.78),
        evidence_refs: unique_evidence(
            cluster
                .iter()
                .flat_map(|record| record.evidence_refs.clone()),
        ),
        source_session_id: cluster[0].source_session_id.clone(),
        workspace_identity: Some(workspace_key.to_string()),
        created_at: cluster
            .iter()
            .map(|record| record.created_at)
            .min()
            .unwrap_or(now),
        updated_at: now,
        last_validated_at: Some(now),
        expires_at: None,
        derived_skill_name: None,
        linked_skill_name: shared_linked_skill_name(cluster),
        validation_status: MemoryValidationStatus::Passed,
    }
}

fn build_methodology_record(pattern: &MemoryRecord, workspace_key: &str, now: i64) -> MemoryRecord {
    let derived_skill_name = pattern
        .derived_skill_name
        .clone()
        .or_else(|| pattern.linked_skill_name.clone())
        .or_else(|| {
            let value = slugify(
                pattern
                    .title
                    .strip_prefix("Pattern: ")
                    .unwrap_or(pattern.title.as_str()),
            );
            (!value.is_empty()).then_some(value)
        });
    let derived_skill_fact = derived_skill_name
        .as_deref()
        .map(|skill_name| format!("derived_skill_name={skill_name}"));
    let linked_skill_fact = pattern
        .linked_skill_name
        .as_deref()
        .map(|skill_name| format!("linked_skill_name={skill_name}"));

    MemoryRecord {
        id: stable_id("mem_methodology", &[workspace_key, &pattern.id.0]),
        kind: MemoryKind::MethodologyCandidate,
        scope: pattern.scope.clone(),
        status: MemoryStatus::Consolidated,
        title: format!(
            "Methodology candidate: {}",
            pattern
                .title
                .strip_prefix("Pattern: ")
                .unwrap_or(pattern.title.as_str())
        ),
        summary: format!(
            "Structured methodology candidate derived from consolidated pattern {}. Validation cues: {}.",
            pattern.id.0,
            collect_validation_clues(pattern)
        ),
        trigger_conditions: pattern.trigger_conditions.clone(),
        normalized_facts: unique_values(
            pattern
                .normalized_facts
                .iter()
                .cloned()
                .chain(std::iter::once(format!("source_pattern_id={}", pattern.id.0)))
                .chain(derived_skill_fact)
                .chain(linked_skill_fact),
        ),
        boundaries: unique_values(
            pattern
                .boundaries
                .iter()
                .cloned()
                .chain(std::iter::once(
                    "Still requires explicit skill extraction before becoming executable."
                        .to_string(),
                ))
                .chain(pattern.linked_skill_name.as_deref().map(|skill_name| {
                    format!(
                        "Likely patches existing skill `{}`; verify current skill body, validation commands, and non-goals before patching.",
                        skill_name
                    )
                })),
        ),
        confidence: Some(0.84),
        evidence_refs: pattern.evidence_refs.clone(),
        source_session_id: pattern.source_session_id.clone(),
        workspace_identity: Some(workspace_key.to_string()),
        created_at: pattern.created_at,
        updated_at: now,
        last_validated_at: Some(now),
        expires_at: None,
        derived_skill_name,
        linked_skill_name: pattern.linked_skill_name.clone(),
        validation_status: MemoryValidationStatus::Passed,
    }
}

fn merged_record(primary: &MemoryRecord, group: &[MemoryRecord], now: i64) -> MemoryRecord {
    MemoryRecord {
        id: primary.id.clone(),
        kind: primary.kind.clone(),
        scope: primary.scope.clone(),
        status: MemoryStatus::Consolidated,
        title: primary.title.clone(),
        summary: unique_values(group.iter().map(|record| record.summary.clone()).chain(
            std::iter::once(format!(
                "Consolidated from {} related memory records.",
                group.len()
            )),
        ))
        .join(" "),
        trigger_conditions: unique_values(
            group
                .iter()
                .flat_map(|record| record.trigger_conditions.clone()),
        ),
        normalized_facts: unique_values(
            group
                .iter()
                .flat_map(|record| record.normalized_facts.clone()),
        ),
        boundaries: unique_values(
            group
                .iter()
                .flat_map(|record| record.boundaries.clone())
                .chain(std::iter::once(format!(
                    "Canonical consolidated record for {}.",
                    primary.title
                ))),
        ),
        confidence: Some(
            group
                .iter()
                .filter_map(|record| record.confidence)
                .fold(0.0_f32, f32::max)
                .max(primary.confidence.unwrap_or(0.0))
                .max(0.72),
        ),
        evidence_refs: unique_evidence(
            group.iter().flat_map(|record| record.evidence_refs.clone()),
        ),
        source_session_id: primary.source_session_id.clone(),
        workspace_identity: primary.workspace_identity.clone(),
        created_at: primary.created_at,
        updated_at: now,
        last_validated_at: Some(now),
        expires_at: None,
        derived_skill_name: primary.derived_skill_name.clone(),
        linked_skill_name: primary.linked_skill_name.clone(),
        validation_status: MemoryValidationStatus::Passed,
    }
}

fn archive_after_merge(record: &MemoryRecord, target: &MemoryRecordId, now: i64) -> MemoryRecord {
    MemoryRecord {
        status: MemoryStatus::Archived,
        updated_at: now,
        last_validated_at: Some(now),
        boundaries: unique_values(record.boundaries.iter().cloned().chain(std::iter::once(
            format!("Archived after consolidation into {}.", target.0),
        ))),
        ..record.clone()
    }
}

fn mark_record_as_consolidated_source(
    record: &MemoryRecord,
    target: &MemoryRecordId,
    note: &str,
    now: i64,
) -> MemoryRecord {
    MemoryRecord {
        status: MemoryStatus::Consolidated,
        updated_at: now,
        last_validated_at: Some(now),
        boundaries: unique_values(
            record
                .boundaries
                .iter()
                .cloned()
                .chain(std::iter::once(format!("{} {}", note, target.0))),
        ),
        validation_status: MemoryValidationStatus::Passed,
        ..record.clone()
    }
}

fn build_reflection_notes(
    merged_count: u32,
    promoted_count: u32,
    record_index: &HashMap<String, MemoryRecord>,
    include_candidates: bool,
) -> Vec<String> {
    let candidate_count = record_index
        .values()
        .filter(|record| record.status == MemoryStatus::Candidate)
        .count();
    let methodology_count = record_index
        .values()
        .filter(|record| record.kind == MemoryKind::MethodologyCandidate)
        .count();
    let linked_skill_methodology_count = record_index
        .values()
        .filter(|record| {
            record.kind == MemoryKind::MethodologyCandidate && record.linked_skill_name.is_some()
        })
        .count();

    let mut notes = Vec::new();
    if merged_count > 0 {
        notes.push(format!(
            "Merged {} overlapping memory records into canonical consolidated entries.",
            merged_count
        ));
    }
    if promoted_count > 0 {
        notes.push(format!(
            "Promoted {} memory clusters into higher-order pattern or methodology records.",
            promoted_count
        ));
    }
    if methodology_count > 0 {
        notes.push(format!(
            "{} methodology candidates now exist; next step is explicit skill extraction with validation commands and non-goals.",
            methodology_count
        ));
    }
    if linked_skill_methodology_count > 0 {
        notes.push(format!(
            "{} skill-linked methodology candidates look more like patch/refinement work than brand-new skills; review existing SKILL.md before extraction.",
            linked_skill_methodology_count
        ));
    }
    if candidate_count > 0 && !include_candidates {
        notes.push(format!(
            "{} candidate records remain outside this consolidation pass; rerun with candidate inclusion only after they pass validation.",
            candidate_count
        ));
    }
    if notes.is_empty() {
        notes.push(
            "No memory clusters met the merge or promotion thresholds; validation signal is still too thin."
                .to_string(),
        );
    }
    notes
}

fn select_primary_id(group: &[MemoryRecord]) -> Option<String> {
    group
        .iter()
        .max_by_key(|record| merge_priority(record))
        .map(|record| record.id.0.clone())
}

fn merge_priority(record: &MemoryRecord) -> (u8, i64, i64) {
    let status_rank = match record.status {
        MemoryStatus::Consolidated => 3,
        MemoryStatus::Validated => 2,
        MemoryStatus::Candidate => 1,
        _ => 0,
    };
    (
        status_rank,
        record.last_validated_at.unwrap_or_default(),
        record.updated_at,
    )
}

fn lesson_cluster_key(record: &MemoryRecord) -> String {
    let stable_facts = record
        .normalized_facts
        .iter()
        .filter(|fact| {
            !fact.starts_with("session_id=")
                && !fact.starts_with("stage_id=")
                && !fact.starts_with("last_event=")
        })
        .cloned()
        .collect::<Vec<_>>();
    format!(
        "{}|{}|{}|{}|{}",
        scope_key(&record.scope),
        record.workspace_identity.as_deref().unwrap_or(""),
        normalize_title_hint(&record.title),
        normalize_collection(&record.trigger_conditions),
        normalize_collection(&stable_facts)
    )
}

fn shared_signal_count(left: &MemoryRecord, right: &MemoryRecord) -> usize {
    let left_signals = signal_set(left);
    let right_signals = signal_set(right);
    left_signals.intersection(&right_signals).count()
}

fn signal_set(record: &MemoryRecord) -> BTreeSet<String> {
    record
        .trigger_conditions
        .iter()
        .chain(record.normalized_facts.iter())
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn collect_validation_clues(record: &MemoryRecord) -> String {
    let mut clues = record
        .normalized_facts
        .iter()
        .filter(|fact| contains_validation_signal(fact))
        .cloned()
        .collect::<Vec<_>>();
    if clues.is_empty() {
        clues.extend(
            record
                .trigger_conditions
                .iter()
                .filter(|value| contains_validation_signal(value))
                .cloned(),
        );
    }
    if clues.is_empty() {
        clues.push("capture explicit validation recipe next".to_string());
    }
    join_top_values(clues.iter())
}

fn count_validation_terms(record: &MemoryRecord) -> usize {
    record
        .trigger_conditions
        .iter()
        .chain(record.normalized_facts.iter())
        .filter(|value| contains_validation_signal(value))
        .count()
}

fn contains_validation_signal(value: &str) -> bool {
    value
        .to_ascii_lowercase()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .any(validation_word_matches)
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

fn shared_linked_skill_name(cluster: &[MemoryRecord]) -> Option<String> {
    let mut values = cluster
        .iter()
        .filter_map(|record| {
            record
                .linked_skill_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    (values.len() == 1).then(|| values.remove(0))
}

fn join_top_values<'a, I>(values: I) -> String
where
    I: IntoIterator<Item = &'a String>,
{
    values
        .into_iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(", ")
}

fn unique_values<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let normalized = value.trim();
            if normalized.is_empty() {
                return None;
            }
            seen.insert(normalized.to_string())
                .then_some(normalized.to_string())
        })
        .collect()
}

fn unique_evidence<I>(values: I) -> Vec<agendao_types::MemoryEvidenceRef>
where
    I: IntoIterator<Item = agendao_types::MemoryEvidenceRef>,
{
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| {
            let key = format!(
                "{}|{}|{}|{}|{}",
                value.session_id.as_deref().unwrap_or(""),
                value.message_id.as_deref().unwrap_or(""),
                value.tool_call_id.as_deref().unwrap_or(""),
                value.stage_id.as_deref().unwrap_or(""),
                value.note.as_deref().unwrap_or("")
            );
            seen.insert(key)
        })
        .collect()
}

fn normalize_collection(values: &[String]) -> String {
    values
        .iter()
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("|")
}

fn normalize_title_hint(title: &str) -> String {
    normalize_text(
        title
            .trim()
            .strip_prefix("Tool failure candidate: ")
            .or_else(|| title.trim().strip_prefix("Tool pattern candidate: "))
            .or_else(|| title.trim().strip_prefix("Stage observation: "))
            .or_else(|| title.trim().strip_prefix("Pattern: "))
            .unwrap_or(title),
    )
}

fn normalize_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == ' ' {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_key(kind: &MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Preference => "preference",
        MemoryKind::EnvironmentFact => "environment_fact",
        MemoryKind::WorkspaceConvention => "workspace_convention",
        MemoryKind::Lesson => "lesson",
        MemoryKind::Pattern => "pattern",
        MemoryKind::MethodologyCandidate => "methodology_candidate",
    }
}

fn scope_key(scope: &MemoryScope) -> &'static str {
    match scope {
        MemoryScope::GlobalUser => "global_user",
        MemoryScope::GlobalWorkspace => "global_workspace",
        MemoryScope::WorkspaceShared => "workspace_shared",
        MemoryScope::WorkspaceSandbox => "workspace_sandbox",
        MemoryScope::SessionEphemeral => "session_ephemeral",
    }
}

fn stable_id(prefix: &str, parts: &[&str]) -> MemoryRecordId {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = format!("{:x}", hasher.finalize());
    MemoryRecordId(format!("{}_{}", prefix, &digest[..24]))
}

fn consolidation_run_id(workspace_key: &str, now: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_key.as_bytes());
    hasher.update(now.to_be_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("mem_consolidation_{}", &digest[..24])
}

fn slugify(value: &str) -> String {
    normalize_text(value).replace(' ', "_")
}

fn rule_hit(
    run_id: &str,
    rule_pack_id: Option<&str>,
    memory_id: Option<MemoryRecordId>,
    hit_kind: &str,
    detail: Option<String>,
    created_at: i64,
) -> MemoryRuleHit {
    let mut hasher = Sha256::new();
    hasher.update(run_id.as_bytes());
    hasher.update(hit_kind.as_bytes());
    if let Some(rule_pack_id) = rule_pack_id {
        hasher.update(rule_pack_id.as_bytes());
    }
    if let Some(memory_id) = memory_id.as_ref() {
        hasher.update(memory_id.0.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());

    MemoryRuleHit {
        id: format!("mem_rule_hit_{}", &digest[..24]),
        rule_pack_id: rule_pack_id.map(ToOwned::to_owned),
        memory_id,
        run_id: Some(run_id.to_string()),
        hit_kind: hit_kind.to_string(),
        detail,
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lesson(id: &str, summary: &str) -> MemoryRecord {
        MemoryRecord {
            id: MemoryRecordId(id.to_string()),
            kind: MemoryKind::Lesson,
            scope: MemoryScope::WorkspaceShared,
            status: MemoryStatus::Validated,
            title: "Tool failure candidate: cargo test".to_string(),
            summary: summary.to_string(),
            trigger_conditions: vec!["tool:cargo_test".to_string(), "verify:test".to_string()],
            normalized_facts: vec![
                "tool_name=cargo_test".to_string(),
                "tool_outcome=error".to_string(),
                "verify:test".to_string(),
            ],
            boundaries: vec![
                "Validate live state with the current test output.".to_string(),
                "Only applies to this workspace test workflow.".to_string(),
            ],
            confidence: Some(0.6),
            evidence_refs: vec![agendao_types::MemoryEvidenceRef {
                session_id: Some("ses_1".to_string()),
                message_id: Some("msg_1".to_string()),
                tool_call_id: Some("call_1".to_string()),
                stage_id: Some("stage_1".to_string()),
                note: Some("fixture".to_string()),
            }],
            source_session_id: Some("ses_1".to_string()),
            workspace_identity: Some("ws:test".to_string()),
            created_at: 1,
            updated_at: 2,
            last_validated_at: Some(2),
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Passed,
        }
    }

    #[test]
    fn repeated_lessons_group_into_clusters() {
        let clusters = lesson_clusters(vec![
            lesson("mem_a", "cargo test failed because fixtures were missing"),
            lesson("mem_b", "cargo test failed because fixtures were stale"),
        ]);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 2);
    }

    #[test]
    fn structured_patterns_are_methodology_ready() {
        let mut pattern = build_pattern_record(
            &[lesson("mem_a", "one"), lesson("mem_b", "two")],
            "ws:test",
            10,
        );
        pattern.evidence_refs.push(agendao_types::MemoryEvidenceRef {
            session_id: Some("ses_2".to_string()),
            message_id: Some("msg_2".to_string()),
            tool_call_id: Some("call_2".to_string()),
            stage_id: Some("stage_2".to_string()),
            note: Some("fixture2".to_string()),
        });

        assert!(methodology_ready(&pattern));
    }

    #[test]
    fn linked_skill_feedback_cluster_promotes_into_linked_methodology() {
        let mut left = lesson("mem_a", "cargo test failed because fixtures were missing");
        left.linked_skill_name = Some("provider-refresh".to_string());
        left.normalized_facts
            .push("linked_skill_name=provider-refresh".to_string());
        left.trigger_conditions
            .push("skill:provider-refresh".to_string());

        let mut right = lesson("mem_b", "cargo test failed because fixtures were stale");
        right.linked_skill_name = Some("provider-refresh".to_string());
        right
            .normalized_facts
            .push("linked_skill_name=provider-refresh".to_string());
        right
            .trigger_conditions
            .push("skill:provider-refresh".to_string());

        let pattern = build_pattern_record(&[left, right], "ws:test", 10);
        assert_eq!(
            pattern.linked_skill_name.as_deref(),
            Some("provider-refresh")
        );

        let methodology = build_methodology_record(&pattern, "ws:test", 10);
        assert_eq!(
            methodology.linked_skill_name.as_deref(),
            Some("provider-refresh")
        );
        assert_eq!(
            methodology.derived_skill_name.as_deref(),
            Some("provider-refresh")
        );
    }
}
