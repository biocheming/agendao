//! Skill evolution proposal repository.
//!
//! Stores and queries `SkillEvolutionProposal` records generated from
//! methodology candidates after memory consolidation.

use anyhow::Result;
use agendao_types::{
    ProposalStatus, SkillEvolutionProposal, SkillEvolutionProposalGenerationSummary,
    SkillEvolutionProposalKind, SuggestedSkillChange,
};
use sqlx::SqlitePool;

pub struct SkillEvolutionProposalRepository {
    pool: SqlitePool,
}

impl SkillEvolutionProposalRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a draft proposal, skipping if `evidence_hash` already exists.
    /// Returns the proposal ID if inserted, or None if skipped (duplicate).
    pub async fn insert_draft(&self, proposal: &SkillEvolutionProposal) -> Result<Option<String>> {
        let memory_ids_json = serde_json::to_string(&proposal.memory_record_ids)?;
        let changes_json = serde_json::to_string(&proposal.suggested_changes)?;
        let kind = serde_json::to_string(&proposal.proposal_kind)?
            .trim_matches('"')
            .to_string();
        let status = serde_json::to_string(&proposal.status)?
            .trim_matches('"')
            .to_string();

        let result = sqlx::query_scalar::<_, String>(
            r#"
            INSERT OR IGNORE INTO skill_evolution_proposals
                (id, session_id, memory_record_ids_json, linked_skill_name,
                 proposal_kind, title, rationale, suggested_changes_json,
                 status, evidence_hash, created_at_ms, updated_at_ms)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id
            "#,
        )
        .bind(&proposal.id)
        .bind(&proposal.session_id)
        .bind(&memory_ids_json)
        .bind(&proposal.linked_skill_name)
        .bind(&kind)
        .bind(&proposal.title)
        .bind(&proposal.rationale)
        .bind(&changes_json)
        .bind(&status)
        .bind(&proposal.evidence_hash)
        .bind(proposal.created_at_ms)
        .bind(proposal.updated_at_ms)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    /// Insert a draft proposal and supersede any existing Draft proposals
    /// for the same `linked_skill_name` whose evidence has changed.
    /// Returns `(inserted_id, superseded_ids)`.
    pub async fn upsert_draft_and_supersede_old(
        &self,
        proposal: &SkillEvolutionProposal,
    ) -> Result<(Option<String>, Vec<String>)> {
        // Try insert first.
        let inserted = self.insert_draft(proposal).await?;
        if inserted.is_some() {
            // New evidence: supersede older Drafts for the same skill.
            let superseded = self
                .mark_drafts_superseded_by_skill(
                    proposal.linked_skill_name.as_deref(),
                    Some(&proposal.id),
                )
                .await?;
            Ok((inserted, superseded))
        } else {
            Ok((None, vec![]))
        }
    }

    /// Mark all Draft proposals for a given `linked_skill_name` as superseded,
    /// except the one with `except_id`.
    async fn mark_drafts_superseded_by_skill(
        &self,
        linked_skill_name: Option<&str>,
        except_id: Option<&str>,
    ) -> Result<Vec<String>> {
        let skill = linked_skill_name.unwrap_or("");
        let status_draft: String = serde_json::to_string(&ProposalStatus::Draft)?
            .trim_matches('"')
            .to_string();
        let status_superseded: String = serde_json::to_string(&ProposalStatus::Superseded)?
            .trim_matches('"')
            .to_string();
        let now = chrono::Utc::now().timestamp_millis();

        let ids: Vec<String> = if let Some(except) = except_id {
            sqlx::query_scalar(
                r#"
                UPDATE skill_evolution_proposals
                SET status = ?, updated_at_ms = ?
                WHERE linked_skill_name = ?
                  AND status = ?
                  AND id != ?
                RETURNING id
                "#,
            )
            .bind(&status_superseded)
            .bind(now)
            .bind(skill)
            .bind(&status_draft)
            .bind(except)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_scalar(
                r#"
                UPDATE skill_evolution_proposals
                SET status = ?, updated_at_ms = ?
                WHERE linked_skill_name = ? AND status = ?
                RETURNING id
                "#,
            )
            .bind(&status_superseded)
            .bind(now)
            .bind(skill)
            .bind(&status_draft)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(ids)
    }

    /// List proposals by status.
    pub async fn list_by_status(
        &self,
        status: &ProposalStatus,
    ) -> Result<Vec<SkillEvolutionProposal>> {
        let status_str: String = serde_json::to_string(status)?.trim_matches('"').to_string();

        let rows = sqlx::query_as::<_, ProposalRow>(
            "SELECT * FROM skill_evolution_proposals WHERE status = ? ORDER BY created_at_ms DESC",
        )
        .bind(&status_str)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(proposal_from_row).collect()
    }

    /// List proposals for a session.
    pub async fn list_by_session_id(
        &self,
        session_id: &str,
    ) -> Result<Vec<SkillEvolutionProposal>> {
        let rows = sqlx::query_as::<_, ProposalRow>(
            "SELECT * FROM skill_evolution_proposals WHERE session_id = ? ORDER BY created_at_ms DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(proposal_from_row).collect()
    }

    /// Update proposal status.
    pub async fn update_status(&self, id: &str, status: &ProposalStatus) -> Result<()> {
        let status_str: String = serde_json::to_string(status)?.trim_matches('"').to_string();
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE skill_evolution_proposals SET status = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(&status_str)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a single proposal by id.
    pub async fn get_by_id(&self, id: &str) -> Result<Option<SkillEvolutionProposal>> {
        let row = sqlx::query_as::<_, ProposalRow>(
            "SELECT * FROM skill_evolution_proposals WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(proposal_from_row).transpose()
    }

    /// Transition a proposal to a new status, validating the transition.
    /// Returns an error if the transition is not allowed.
    ///
    /// Allowed transitions:
    /// - Draft → Accepted, Rejected, Superseded
    /// - Accepted → Rejected, Applied
    pub async fn transition_status(&self, id: &str, next: &ProposalStatus) -> Result<()> {
        let Some(current) = self.get_by_id(id).await? else {
            anyhow::bail!("proposal not found: {}", id);
        };

        let allowed = match (&current.status, next) {
            (ProposalStatus::Draft, ProposalStatus::Accepted)
            | (ProposalStatus::Draft, ProposalStatus::Rejected)
            | (ProposalStatus::Draft, ProposalStatus::Superseded)
            | (ProposalStatus::Accepted, ProposalStatus::Rejected)
            | (ProposalStatus::Accepted, ProposalStatus::Applied) => true,
            _ => false,
        };

        if !allowed {
            anyhow::bail!(
                "invalid status transition: {:?} → {:?}",
                current.status,
                next
            );
        }

        self.update_status(id, next).await
    }
}

// ── raw row type for sqlx::query_as ──────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct ProposalRow {
    id: String,
    session_id: String,
    memory_record_ids_json: String,
    linked_skill_name: Option<String>,
    proposal_kind: String,
    title: String,
    rationale: String,
    suggested_changes_json: String,
    status: String,
    evidence_hash: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

fn proposal_from_row(row: ProposalRow) -> Result<SkillEvolutionProposal> {
    Ok(SkillEvolutionProposal {
        id: row.id,
        session_id: row.session_id,
        memory_record_ids: serde_json::from_str(&row.memory_record_ids_json)?,
        linked_skill_name: row.linked_skill_name,
        proposal_kind: serde_json::from_str(&format!("\"{}\"", row.proposal_kind))?,
        title: row.title,
        rationale: row.rationale,
        suggested_changes: serde_json::from_str(&row.suggested_changes_json)?,
        status: serde_json::from_str(&format!("\"{}\"", row.status))?,
        evidence_hash: row.evidence_hash,
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
    })
}

// ── Proposal generation ──────────────────────────────────────────────────

/// Generate `SkillEvolutionProposal` records from methodology candidates
/// promoted during consolidation, and persist them via the repository.
///
/// Deduplication is by `evidence_hash`; same evidence → skip.
/// New evidence for same `linked_skill_name` → old Draft proposals superseded.
pub async fn generate_skill_evolution_proposals(
    repo: &SkillEvolutionProposalRepository,
    promotion_pool: &[agendao_types::MemoryRecord],
    session_id: &str,
) -> Result<SkillEvolutionProposalGenerationSummary> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut created = 0u32;
    let mut skipped = 0u32;
    let mut seen = 0u32;

    for record in promotion_pool {
        if record.kind != agendao_types::MemoryKind::MethodologyCandidate {
            continue;
        }
        seen += 1;

        let proposal_kind = match record.linked_skill_name.as_deref() {
            Some(_) => SkillEvolutionProposalKind::PatchExistingSkill,
            None => SkillEvolutionProposalKind::CreateNewSkill,
        };

        let (title, suggested_changes) = build_suggested_changes(record, &proposal_kind);

        let evidence_hash = SkillEvolutionProposal::compute_evidence_hash(
            &proposal_kind,
            record.linked_skill_name.as_deref(),
            &[record.id.0.clone()],
            &suggested_changes,
        );

        let proposal = SkillEvolutionProposal {
            id: format!("skp_{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            memory_record_ids: vec![record.id.0.clone()],
            linked_skill_name: record.linked_skill_name.clone(),
            proposal_kind,
            title: title.clone(),
            rationale: format!(
                "Derived from methodology candidate '{}' (confidence {}). \
                 Evidence: {}. Boundaries: {}.",
                record.title,
                record.confidence.unwrap_or(0.0),
                record
                    .evidence_refs
                    .iter()
                    .map(|e| e.note.as_deref().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join(", "),
                record.boundaries.join("; "),
            ),
            suggested_changes,
            status: ProposalStatus::Draft,
            evidence_hash,
            created_at_ms: now,
            updated_at_ms: now,
        };

        match repo.upsert_draft_and_supersede_old(&proposal).await {
            Ok((Some(_), _)) => created += 1,
            Ok((None, _)) => skipped += 1,
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id,
                    "skill evolution proposal upsert failed"
                );
                skipped += 1;
            }
        }
    }

    Ok(SkillEvolutionProposalGenerationSummary {
        proposals_created: created,
        proposals_skipped: skipped,
        methodology_candidates_seen: seen,
    })
}

fn build_suggested_changes(
    record: &agendao_types::MemoryRecord,
    kind: &SkillEvolutionProposalKind,
) -> (String, Vec<SuggestedSkillChange>) {
    match kind {
        SkillEvolutionProposalKind::PatchExistingSkill => {
            let skill_name = record.linked_skill_name.as_deref().unwrap_or("unknown");
            let mut changes = Vec::new();
            let mut title_parts = vec![format!("Patch skill '{}'", skill_name)];

            // Trigger conditions → AddTriggerCondition
            for trigger in &record.trigger_conditions {
                changes.push(SuggestedSkillChange::AddTriggerCondition {
                    text: trigger.clone(),
                    evidence_refs: record.evidence_refs.iter().map(|e| format_ref(e)).collect(),
                });
                title_parts.push(format!("add trigger '{}'", trigger));
            }

            // Normalized facts → AddCoreStep
            for fact in &record.normalized_facts {
                if !fact.starts_with("tool_name=")
                    && !fact.starts_with("tool_outcome=")
                    && !fact.starts_with("stage_id=")
                    && !fact.starts_with("session_id=")
                    && !fact.starts_with("skill_name=")
                    && !fact.starts_with("linked_skill_name=")
                {
                    changes.push(SuggestedSkillChange::AddCoreStep {
                        text: fact.clone(),
                        evidence_refs: record.evidence_refs.iter().map(|e| format_ref(e)).collect(),
                    });
                }
            }

            // Boundaries → AddBoundary
            for boundary in &record.boundaries {
                changes.push(SuggestedSkillChange::AddBoundary {
                    text: boundary.clone(),
                    evidence_refs: record.evidence_refs.iter().map(|e| format_ref(e)).collect(),
                });
            }

            let title = if title_parts.len() > 1 {
                title_parts.join("; ")
            } else {
                format!("Patch skill '{}' from evidence", skill_name)
            };

            (title, changes)
        }
        SkillEvolutionProposalKind::CreateNewSkill => {
            let suggested_name = record
                .derived_skill_name
                .clone()
                .unwrap_or_else(|| "unnamed-skill".to_string());

            let when_to_use: Vec<String> = record.trigger_conditions.iter().cloned().collect();
            let core_steps: Vec<String> = record
                .normalized_facts
                .iter()
                .filter(|f| {
                    !f.starts_with("tool_name=")
                        && !f.starts_with("tool_outcome=")
                        && !f.starts_with("stage_id=")
                })
                .cloned()
                .collect();
            let boundaries: Vec<String> = record.boundaries.clone();
            let validation: Vec<String> =
                record.evidence_refs.iter().map(|e| format_ref(e)).collect();

            (
                format!("Create skill '{}'", suggested_name),
                vec![SuggestedSkillChange::CreateSkillDraft {
                    suggested_name,
                    when_to_use,
                    core_steps,
                    boundaries,
                    validation,
                }],
            )
        }
    }
}

fn format_ref(evidence: &agendao_types::MemoryEvidenceRef) -> String {
    let mut parts = Vec::new();
    if let Some(session_id) = evidence.session_id.as_deref() {
        parts.push(format!("session={}", session_id));
    }
    if let Some(tool_call_id) = evidence.tool_call_id.as_deref() {
        parts.push(format!("tool={}", tool_call_id));
    }
    if let Some(note) = evidence.note.as_deref() {
        parts.push(note.to_string());
    }
    parts.join(" · ")
}
