use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json;
use sqlx::{FromRow, Row, SqlitePool};

use rocode_types::{
    MemoryConflictView, MemoryConsolidationRun, MemoryEvidenceRef, MemoryKind, MemoryRecord,
    MemoryRecordId, MemoryRuleHit, MemoryRuleHitQuery, MemoryRulePack, MemoryRulePackKind,
    MemoryScope, MemoryStatus, MemoryValidationReport, MemoryValidationStatus, MessagePart,
    MessageRole, Session, SessionMessage, SessionShare, SessionStatus, SessionSummary, SessionTime,
    SessionUsage,
};

use crate::database::DatabaseError;

// ── Shared SQL constants (single source of truth for upsert schemas) ────────

const SESSION_UPSERT_SQL: &str = r#"
INSERT INTO sessions (
    id, project_id, parent_id, slug, directory, title, version, share_url,
    summary_additions, summary_deletions, summary_files, summary_diffs,
    revert, permission, metadata,
    usage_input_tokens, usage_context_tokens, usage_output_tokens, usage_reasoning_tokens,
    usage_cache_write_tokens, usage_cache_read_tokens, usage_cache_miss_tokens, usage_total_cost,
    status, created_at, updated_at, time_compacting, time_archived
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
    title = excluded.title, version = excluded.version, share_url = excluded.share_url,
    summary_additions = excluded.summary_additions, summary_deletions = excluded.summary_deletions,
    summary_files = excluded.summary_files, summary_diffs = excluded.summary_diffs,
    revert = excluded.revert, permission = excluded.permission, metadata = excluded.metadata,
    usage_input_tokens = excluded.usage_input_tokens,
    usage_context_tokens = excluded.usage_context_tokens,
    usage_output_tokens = excluded.usage_output_tokens,
    usage_reasoning_tokens = excluded.usage_reasoning_tokens,
    usage_cache_write_tokens = excluded.usage_cache_write_tokens,
    usage_cache_read_tokens = excluded.usage_cache_read_tokens,
    usage_cache_miss_tokens = excluded.usage_cache_miss_tokens,
    usage_total_cost = excluded.usage_total_cost,
    status = excluded.status, updated_at = excluded.updated_at,
    time_compacting = excluded.time_compacting, time_archived = excluded.time_archived
"#;

const MESSAGE_UPSERT_SQL: &str = r#"
INSERT INTO messages (
    id, session_id, role, created_at, provider_id, model_id,
    tokens_input, tokens_context, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_miss, tokens_cache_write,
    cost, finish, metadata, data
)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
    session_id = excluded.session_id,
    role = excluded.role,
    created_at = excluded.created_at,
    provider_id = excluded.provider_id,
    model_id = excluded.model_id,
    tokens_input = excluded.tokens_input,
    tokens_context = excluded.tokens_context,
    tokens_output = excluded.tokens_output,
    tokens_reasoning = excluded.tokens_reasoning,
    tokens_cache_read = excluded.tokens_cache_read,
    tokens_cache_miss = excluded.tokens_cache_miss,
    tokens_cache_write = excluded.tokens_cache_write,
    cost = excluded.cost,
    finish = excluded.finish,
    metadata = excluded.metadata,
    data = excluded.data
"#;

const MEMORY_RECORD_UPSERT_SQL: &str = r#"
INSERT INTO memory_records (
    id, kind, scope, status, title, summary, trigger_conditions, normalized_facts, boundaries,
    confidence, source_session_id, workspace_identity, created_at, updated_at,
    last_validated_at, expires_at, derived_skill_name, linked_skill_name, validation_status
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
    kind = excluded.kind,
    scope = excluded.scope,
    status = excluded.status,
    title = excluded.title,
    summary = excluded.summary,
    trigger_conditions = excluded.trigger_conditions,
    normalized_facts = excluded.normalized_facts,
    boundaries = excluded.boundaries,
    confidence = excluded.confidence,
    source_session_id = excluded.source_session_id,
    workspace_identity = excluded.workspace_identity,
    created_at = excluded.created_at,
    updated_at = excluded.updated_at,
    last_validated_at = excluded.last_validated_at,
    expires_at = excluded.expires_at,
    derived_skill_name = excluded.derived_skill_name,
    linked_skill_name = excluded.linked_skill_name,
    validation_status = excluded.validation_status
"#;

fn serialize_json_for_db<T: Serialize>(context: &str, field: &str, value: &T) -> Option<String> {
    match serde_json::to_string(value) {
        Ok(json) => Some(json),
        Err(error) => {
            tracing::error!(
                context,
                field,
                %error,
                "failed to serialize field for database persistence"
            );
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryRepositoryFilter {
    pub scopes: Vec<MemoryScope>,
    pub kinds: Vec<MemoryKind>,
    pub statuses: Vec<MemoryStatus>,
    pub search: Option<String>,
    pub workspace_identity: Option<String>,
    pub source_session_id: Option<String>,
    pub derived_skill_name: Option<String>,
    pub linked_skill_name: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct MemoryRetrievalLogEntry {
    pub session_id: Option<String>,
    pub query: Option<String>,
    pub stage: Option<String>,
    pub scopes: Vec<MemoryScope>,
    pub retrieved_count: u32,
    pub used_count: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct MemoryConflictRecord {
    pub id: String,
    pub left_memory_id: String,
    pub right_memory_id: String,
    pub conflict_kind: String,
    pub detail: String,
    pub detected_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAdapterReplayRecord {
    pub adapter_id: String,
    pub source: String,
    pub external_event_id: String,
    pub idempotency_key: String,
    pub external_user_id: String,
    pub external_conversation_id: String,
    pub session_id: String,
    pub actor_id: String,
    pub workspace_id: String,
    pub nonce: String,
    pub signature_hash: String,
    pub received_at_ms: i64,
    pub recorded_at_ms: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalAdapterReplayInsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Clone)]
pub struct ExternalAdapterReplayRepository {
    pool: SqlitePool,
}

impl ExternalAdapterReplayRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record(
        &self,
        record: &ExternalAdapterReplayRecord,
    ) -> Result<ExternalAdapterReplayInsertOutcome, DatabaseError> {
        let result = sqlx::query(
            r#"
            INSERT INTO external_adapter_replay (
                adapter_id, source, external_event_id, idempotency_key,
                external_user_id, external_conversation_id, session_id, actor_id, workspace_id,
                nonce, signature_hash, received_at_ms, recorded_at_ms, status
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&record.adapter_id)
        .bind(&record.source)
        .bind(&record.external_event_id)
        .bind(&record.idempotency_key)
        .bind(&record.external_user_id)
        .bind(&record.external_conversation_id)
        .bind(&record.session_id)
        .bind(&record.actor_id)
        .bind(&record.workspace_id)
        .bind(&record.nonce)
        .bind(&record.signature_hash)
        .bind(record.received_at_ms)
        .bind(record.recorded_at_ms)
        .bind(&record.status)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(ExternalAdapterReplayInsertOutcome::Inserted),
            Err(error) if is_sqlite_unique_constraint_error(&error) => {
                Ok(ExternalAdapterReplayInsertOutcome::Duplicate)
            }
            Err(error) => Err(DatabaseError::QueryError(error.to_string())),
        }
    }

    pub async fn get_by_event(
        &self,
        adapter_id: &str,
        source: &str,
        external_event_id: &str,
    ) -> Result<Option<ExternalAdapterReplayRecord>, DatabaseError> {
        sqlx::query_as::<_, ExternalAdapterReplayRow>(
            r#"
            SELECT adapter_id, source, external_event_id, idempotency_key,
                   external_user_id, external_conversation_id, session_id, actor_id, workspace_id,
                   nonce, signature_hash, received_at_ms, recorded_at_ms, status
            FROM external_adapter_replay
            WHERE adapter_id = ? AND source = ? AND external_event_id = ?
            "#,
        )
        .bind(adapter_id)
        .bind(source)
        .bind(external_event_id)
        .fetch_optional(&self.pool)
        .await
        .map(|row| row.map(ExternalAdapterReplayRow::into_record))
        .map_err(|error| DatabaseError::QueryError(error.to_string()))
    }

    pub async fn prune_recorded_before(&self, cutoff_ms: i64) -> Result<u64, DatabaseError> {
        sqlx::query("DELETE FROM external_adapter_replay WHERE recorded_at_ms < ?")
            .bind(cutoff_ms)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(|error| DatabaseError::QueryError(error.to_string()))
    }
}

#[derive(Debug, FromRow)]
struct ExternalAdapterReplayRow {
    adapter_id: String,
    source: String,
    external_event_id: String,
    idempotency_key: String,
    external_user_id: String,
    external_conversation_id: String,
    session_id: String,
    actor_id: String,
    workspace_id: String,
    nonce: String,
    signature_hash: String,
    received_at_ms: i64,
    recorded_at_ms: i64,
    status: String,
}

impl ExternalAdapterReplayRow {
    fn into_record(self) -> ExternalAdapterReplayRecord {
        ExternalAdapterReplayRecord {
            adapter_id: self.adapter_id,
            source: self.source,
            external_event_id: self.external_event_id,
            idempotency_key: self.idempotency_key,
            external_user_id: self.external_user_id,
            external_conversation_id: self.external_conversation_id,
            session_id: self.session_id,
            actor_id: self.actor_id,
            workspace_id: self.workspace_id,
            nonce: self.nonce,
            signature_hash: self.signature_hash,
            received_at_ms: self.received_at_ms,
            recorded_at_ms: self.recorded_at_ms,
            status: self.status,
        }
    }
}

fn is_sqlite_unique_constraint_error(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .map(|database_error| {
            database_error.code().as_deref() == Some("2067")
                || database_error
                    .message()
                    .contains("UNIQUE constraint failed")
        })
        .unwrap_or(false)
}

#[derive(Debug, FromRow)]
struct MemoryRecordRow {
    id: String,
    kind: String,
    scope: String,
    status: String,
    title: String,
    summary: String,
    trigger_conditions: Option<String>,
    normalized_facts: Option<String>,
    boundaries: Option<String>,
    confidence: Option<f64>,
    source_session_id: Option<String>,
    workspace_identity: Option<String>,
    created_at: i64,
    updated_at: i64,
    last_validated_at: Option<i64>,
    expires_at: Option<i64>,
    derived_skill_name: Option<String>,
    linked_skill_name: Option<String>,
    validation_status: String,
}

impl MemoryRecordRow {
    fn into_record(self, evidence_refs: Vec<MemoryEvidenceRef>) -> MemoryRecord {
        MemoryRecord {
            id: MemoryRecordId(self.id),
            kind: string_to_memory_kind(&self.kind),
            scope: string_to_memory_scope(&self.scope),
            status: string_to_memory_status(&self.status),
            title: self.title,
            summary: self.summary,
            trigger_conditions: parse_json_vec(self.trigger_conditions),
            normalized_facts: parse_json_vec(self.normalized_facts),
            boundaries: parse_json_vec(self.boundaries),
            confidence: self.confidence.map(|value| value as f32),
            evidence_refs,
            source_session_id: self.source_session_id,
            workspace_identity: self.workspace_identity,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_validated_at: self.last_validated_at,
            expires_at: self.expires_at,
            derived_skill_name: self.derived_skill_name,
            linked_skill_name: self.linked_skill_name,
            validation_status: string_to_memory_validation_status(&self.validation_status),
        }
    }
}

#[derive(Debug, FromRow)]
struct MemoryEvidenceRow {
    memory_id: String,
    evidence_index: i64,
    session_id: Option<String>,
    message_id: Option<String>,
    tool_call_id: Option<String>,
    stage_id: Option<String>,
    note: Option<String>,
}

#[derive(Debug, FromRow)]
struct MemoryValidationRunRow {
    memory_id: Option<String>,
    status: String,
    issues: Option<String>,
    checked_at: i64,
}

#[derive(Debug, FromRow)]
struct MemoryConflictRow {
    id: String,
    left_memory_id: Option<String>,
    right_memory_id: Option<String>,
    conflict_kind: String,
    detail: String,
    detected_at: i64,
}

#[derive(Debug, FromRow)]
struct MemoryConsolidationRunRow {
    run_id: String,
    started_at: i64,
    finished_at: Option<i64>,
    merged_count: i64,
    promoted_count: i64,
    conflict_count: i64,
}

#[derive(Debug, FromRow)]
struct MemoryRulePackRow {
    id: String,
    rule_pack_kind: String,
    version: String,
    body: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, FromRow)]
struct MemoryRuleHitRow {
    id: String,
    rule_pack_id: Option<String>,
    memory_id: Option<String>,
    run_id: Option<String>,
    hit_kind: String,
    detail: Option<String>,
    created_at: i64,
}

#[derive(Clone)]
pub struct MemoryRepository {
    pool: SqlitePool,
}

impl MemoryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn upsert_record(&self, record: &MemoryRecord) -> Result<(), DatabaseError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;

        bind_memory_record_upsert(sqlx::query(MEMORY_RECORD_UPSERT_SQL), record)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        sqlx::query("DELETE FROM memory_evidence WHERE memory_id = ?")
            .bind(&record.id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        for (index, evidence) in record.evidence_refs.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO memory_evidence (
                    memory_id, evidence_index, session_id, message_id, tool_call_id, stage_id, note
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&record.id.0)
            .bind(index as i64)
            .bind(&evidence.session_id)
            .bind(&evidence.message_id)
            .bind(&evidence.tool_call_id)
            .bind(&evidence.stage_id)
            .bind(&evidence.note)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;
        Ok(())
    }

    pub async fn get_record(&self, id: &str) -> Result<Option<MemoryRecord>, DatabaseError> {
        let row = sqlx::query_as::<_, MemoryRecordRow>(
            r#"SELECT
                id, kind, scope, status, title, summary, trigger_conditions, normalized_facts,
                boundaries, confidence, source_session_id, workspace_identity, created_at,
                updated_at, last_validated_at, expires_at, derived_skill_name,
                linked_skill_name, validation_status
               FROM memory_records
               WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let Some(row) = row else {
            return Ok(None);
        };
        let evidence = self.load_evidence_map(&[row.id.clone()]).await?;
        Ok(Some(row.into_record(
            evidence.get(id).cloned().unwrap_or_default(),
        )))
    }

    pub async fn list_records(
        &self,
        filter: Option<&MemoryRepositoryFilter>,
    ) -> Result<Vec<MemoryRecord>, DatabaseError> {
        let filter = filter.cloned().unwrap_or_default();
        let mut sql = String::from(
            r#"SELECT
                id, kind, scope, status, title, summary, trigger_conditions, normalized_facts,
                boundaries, confidence, source_session_id, workspace_identity, created_at,
                updated_at, last_validated_at, expires_at, derived_skill_name,
                linked_skill_name, validation_status
               FROM memory_records
               WHERE 1 = 1"#,
        );

        if !filter.scopes.is_empty() {
            sql.push_str(" AND scope IN (");
            sql.push_str(&repeat_placeholders(filter.scopes.len()));
            sql.push(')');
        }
        if !filter.kinds.is_empty() {
            sql.push_str(" AND kind IN (");
            sql.push_str(&repeat_placeholders(filter.kinds.len()));
            sql.push(')');
        }
        if !filter.statuses.is_empty() {
            sql.push_str(" AND status IN (");
            sql.push_str(&repeat_placeholders(filter.statuses.len()));
            sql.push(')');
        }
        if filter.search.is_some() {
            sql.push_str(" AND (title LIKE ? OR summary LIKE ? OR normalized_facts LIKE ?)");
        }
        if filter.workspace_identity.is_some() {
            sql.push_str(" AND workspace_identity = ?");
        }
        if filter.source_session_id.is_some() {
            sql.push_str(" AND source_session_id = ?");
        }
        if filter.derived_skill_name.is_some() {
            sql.push_str(" AND derived_skill_name = ?");
        }
        if filter.linked_skill_name.is_some() {
            sql.push_str(" AND linked_skill_name = ?");
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT ?");

        let mut query = sqlx::query_as::<_, MemoryRecordRow>(&sql);
        for scope in &filter.scopes {
            query = query.bind(memory_scope_to_str(scope));
        }
        for kind in &filter.kinds {
            query = query.bind(memory_kind_to_str(kind));
        }
        for status in &filter.statuses {
            query = query.bind(memory_status_to_str(status));
        }
        if let Some(search) = filter.search.as_deref() {
            let pattern = format!("%{}%", search);
            query = query
                .bind(pattern.clone())
                .bind(pattern.clone())
                .bind(pattern);
        }
        if let Some(workspace_identity) = filter.workspace_identity.as_deref() {
            query = query.bind(workspace_identity);
        }
        if let Some(source_session_id) = filter.source_session_id.as_deref() {
            query = query.bind(source_session_id);
        }
        if let Some(derived_skill_name) = filter.derived_skill_name.as_deref() {
            query = query.bind(derived_skill_name);
        }
        if let Some(linked_skill_name) = filter.linked_skill_name.as_deref() {
            query = query.bind(linked_skill_name);
        }
        query = query.bind(filter.limit.unwrap_or(100));

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let evidence = self.load_evidence_map(&ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let id = row.id.clone();
                row.into_record(evidence.get(&id).cloned().unwrap_or_default())
            })
            .collect())
    }

    pub async fn delete_record(&self, id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM memory_records WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        Ok(())
    }

    pub async fn record_validation_run(
        &self,
        report: &MemoryValidationReport,
    ) -> Result<(), DatabaseError> {
        let run_id = validation_run_id(report);
        let issues = serde_json::to_string(&report.issues)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO memory_validation_runs (run_id, memory_id, status, issues, checked_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(run_id) DO UPDATE SET
                memory_id = excluded.memory_id,
                status = excluded.status,
                issues = excluded.issues,
                checked_at = excluded.checked_at
            "#,
        )
        .bind(run_id)
        .bind(report.record_id.as_ref().map(|id| id.0.as_str()))
        .bind(memory_validation_status_to_str(&report.status))
        .bind(issues)
        .bind(report.checked_at)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        Ok(())
    }

    pub async fn latest_validation_report(
        &self,
        memory_id: &str,
    ) -> Result<Option<MemoryValidationReport>, DatabaseError> {
        let row = sqlx::query_as::<_, MemoryValidationRunRow>(
            r#"
            SELECT memory_id, status, issues, checked_at
            FROM memory_validation_runs
            WHERE memory_id = ?
            ORDER BY checked_at DESC
            LIMIT 1
            "#,
        )
        .bind(memory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(row.map(|row| MemoryValidationReport {
            record_id: row.memory_id.map(MemoryRecordId),
            status: string_to_memory_validation_status(&row.status),
            issues: parse_json_vec(row.issues),
            checked_at: row.checked_at,
        }))
    }

    pub async fn record_consolidation_run(
        &self,
        run: &MemoryConsolidationRun,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT INTO memory_consolidation_runs (
                run_id, started_at, finished_at, merged_count, promoted_count, conflict_count
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(run_id) DO UPDATE SET
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                merged_count = excluded.merged_count,
                promoted_count = excluded.promoted_count,
                conflict_count = excluded.conflict_count
            "#,
        )
        .bind(&run.run_id)
        .bind(run.started_at)
        .bind(run.finished_at)
        .bind(run.merged_count as i64)
        .bind(run.promoted_count as i64)
        .bind(run.conflict_count as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        Ok(())
    }

    pub async fn list_consolidation_runs(
        &self,
        limit: Option<i64>,
    ) -> Result<Vec<MemoryConsolidationRun>, DatabaseError> {
        let rows = sqlx::query_as::<_, MemoryConsolidationRunRow>(
            r#"
            SELECT run_id, started_at, finished_at, merged_count, promoted_count, conflict_count
            FROM memory_consolidation_runs
            ORDER BY started_at DESC, run_id DESC
            LIMIT ?
            "#,
        )
        .bind(limit.unwrap_or(20))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| MemoryConsolidationRun {
                run_id: row.run_id,
                started_at: row.started_at,
                finished_at: row.finished_at,
                merged_count: row.merged_count.max(0) as u32,
                promoted_count: row.promoted_count.max(0) as u32,
                conflict_count: row.conflict_count.max(0) as u32,
            })
            .collect())
    }

    pub async fn record_retrieval(
        &self,
        entry: &MemoryRetrievalLogEntry,
    ) -> Result<(), DatabaseError> {
        let scopes = serde_json::to_string(&entry.scopes)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO memory_retrieval_log (
                session_id, query, stage, scopes, retrieved_count, used_count, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&entry.session_id)
        .bind(&entry.query)
        .bind(&entry.stage)
        .bind(scopes)
        .bind(entry.retrieved_count as i64)
        .bind(entry.used_count as i64)
        .bind(entry.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        Ok(())
    }

    pub async fn list_retrieval_logs(
        &self,
        session_id: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<MemoryRetrievalLogEntry>, DatabaseError> {
        let limit = limit.unwrap_or(20).clamp(1, 500);
        let rows = if let Some(session_id) = session_id {
            sqlx::query(
                r#"
                SELECT session_id, query, stage, scopes, retrieved_count, used_count, created_at
                FROM memory_retrieval_log
                WHERE session_id = ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(session_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
        } else {
            sqlx::query(
                r#"
                SELECT session_id, query, stage, scopes, retrieved_count, used_count, created_at
                FROM memory_retrieval_log
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
        };

        rows.into_iter()
            .map(|row| {
                let scopes =
                    serde_json::from_str::<Vec<MemoryScope>>(&row.get::<String, _>("scopes"))
                        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
                Ok(MemoryRetrievalLogEntry {
                    session_id: row.get("session_id"),
                    query: row.get("query"),
                    stage: row.get("stage"),
                    scopes,
                    retrieved_count: row.get::<i64, _>("retrieved_count").max(0) as u32,
                    used_count: row.get::<i64, _>("used_count").max(0) as u32,
                    created_at: row.get("created_at"),
                })
            })
            .collect()
    }

    pub async fn upsert_rule_pack(&self, pack: &MemoryRulePack) -> Result<(), DatabaseError> {
        let body =
            serde_json::to_string(pack).map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO memory_rule_packs (
                id, rule_pack_kind, version, body, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                rule_pack_kind = excluded.rule_pack_kind,
                version = excluded.version,
                body = excluded.body,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&pack.id)
        .bind(memory_rule_pack_kind_to_str(&pack.rule_pack_kind))
        .bind(&pack.version)
        .bind(body)
        .bind(pack.created_at)
        .bind(pack.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        Ok(())
    }

    pub async fn list_rule_packs(
        &self,
        kind: Option<MemoryRulePackKind>,
    ) -> Result<Vec<MemoryRulePack>, DatabaseError> {
        let rows = if let Some(kind) = kind {
            sqlx::query_as::<_, MemoryRulePackRow>(
                r#"
                SELECT id, rule_pack_kind, version, body, created_at, updated_at
                FROM memory_rule_packs
                WHERE rule_pack_kind = ?
                ORDER BY id ASC
                "#,
            )
            .bind(memory_rule_pack_kind_to_str(&kind))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
        } else {
            sqlx::query_as::<_, MemoryRulePackRow>(
                r#"
                SELECT id, rule_pack_kind, version, body, created_at, updated_at
                FROM memory_rule_packs
                ORDER BY rule_pack_kind ASC, id ASC
                "#,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
        };

        rows.into_iter().map(parse_memory_rule_pack_row).collect()
    }

    pub async fn record_rule_hits(&self, hits: &[MemoryRuleHit]) -> Result<(), DatabaseError> {
        if hits.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;

        for hit in hits {
            sqlx::query(
                r#"
                INSERT INTO memory_rule_hits (
                    id, rule_pack_id, memory_id, run_id, hit_kind, detail, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    rule_pack_id = excluded.rule_pack_id,
                    memory_id = excluded.memory_id,
                    run_id = excluded.run_id,
                    hit_kind = excluded.hit_kind,
                    detail = excluded.detail,
                    created_at = excluded.created_at
                "#,
            )
            .bind(&hit.id)
            .bind(&hit.rule_pack_id)
            .bind(hit.memory_id.as_ref().map(|id| id.0.as_str()))
            .bind(&hit.run_id)
            .bind(&hit.hit_kind)
            .bind(&hit.detail)
            .bind(hit.created_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;
        Ok(())
    }

    pub async fn list_rule_hits(
        &self,
        query: Option<&MemoryRuleHitQuery>,
    ) -> Result<Vec<MemoryRuleHit>, DatabaseError> {
        let query = query.cloned().unwrap_or_default();
        let mut sql = String::from(
            "SELECT id, rule_pack_id, memory_id, run_id, hit_kind, detail, created_at \
             FROM memory_rule_hits WHERE 1=1",
        );
        if query.run_id.is_some() {
            sql.push_str(" AND run_id = ?");
        }
        if query.memory_id.is_some() {
            sql.push_str(" AND memory_id = ?");
        }
        sql.push_str(" ORDER BY created_at DESC, id ASC LIMIT ?");

        let mut stmt = sqlx::query_as::<_, MemoryRuleHitRow>(&sql);
        if let Some(run_id) = query.run_id.as_deref() {
            stmt = stmt.bind(run_id);
        }
        if let Some(memory_id) = query.memory_id.as_ref() {
            stmt = stmt.bind(&memory_id.0);
        }
        stmt = stmt.bind(query.limit.unwrap_or(100) as i64);

        let rows = stmt
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| MemoryRuleHit {
                id: row.id,
                rule_pack_id: row.rule_pack_id,
                memory_id: row.memory_id.map(MemoryRecordId),
                run_id: row.run_id,
                hit_kind: row.hit_kind,
                detail: row.detail,
                created_at: row.created_at,
            })
            .collect())
    }

    pub async fn replace_conflicts_for_memory(
        &self,
        memory_id: &str,
        conflicts: &[MemoryConflictRecord],
    ) -> Result<(), DatabaseError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;

        sqlx::query("DELETE FROM memory_conflicts WHERE left_memory_id = ? OR right_memory_id = ?")
            .bind(memory_id)
            .bind(memory_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        for conflict in conflicts {
            sqlx::query(
                r#"
                INSERT INTO memory_conflicts (
                    id, left_memory_id, right_memory_id, conflict_kind, detail, detected_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    left_memory_id = excluded.left_memory_id,
                    right_memory_id = excluded.right_memory_id,
                    conflict_kind = excluded.conflict_kind,
                    detail = excluded.detail,
                    detected_at = excluded.detected_at
                "#,
            )
            .bind(&conflict.id)
            .bind(&conflict.left_memory_id)
            .bind(&conflict.right_memory_id)
            .bind(&conflict.conflict_kind)
            .bind(&conflict.detail)
            .bind(conflict.detected_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;
        Ok(())
    }

    pub async fn list_conflicts_for_memory(
        &self,
        memory_id: &str,
    ) -> Result<Vec<MemoryConflictView>, DatabaseError> {
        let rows = sqlx::query_as::<_, MemoryConflictRow>(
            r#"
            SELECT id, left_memory_id, right_memory_id, conflict_kind, detail, detected_at
            FROM memory_conflicts
            WHERE left_memory_id = ? OR right_memory_id = ?
            ORDER BY detected_at DESC, id ASC
            "#,
        )
        .bind(memory_id)
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let left = row.left_memory_id?;
                let right = row.right_memory_id?;
                let (record_id, other_record_id) = if left == memory_id {
                    (left, right)
                } else if right == memory_id {
                    (right, left)
                } else {
                    return None;
                };

                Some(MemoryConflictView {
                    id: row.id,
                    record_id: MemoryRecordId(record_id),
                    other_record_id: MemoryRecordId(other_record_id),
                    conflict_kind: row.conflict_kind,
                    detail: row.detail,
                    detected_at: row.detected_at,
                })
            })
            .collect())
    }

    async fn load_evidence_map(
        &self,
        memory_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<MemoryEvidenceRef>>, DatabaseError> {
        let mut map = std::collections::HashMap::new();
        if memory_ids.is_empty() {
            return Ok(map);
        }

        for chunk in memory_ids.chunks(500) {
            let sql = format!(
                "SELECT memory_id, evidence_index, session_id, message_id, tool_call_id, stage_id, note \
                 FROM memory_evidence WHERE memory_id IN ({}) ORDER BY memory_id ASC, evidence_index ASC",
                repeat_placeholders(chunk.len())
            );
            let mut query = sqlx::query_as::<_, MemoryEvidenceRow>(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            let rows = query
                .fetch_all(&self.pool)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            for row in rows {
                let _ = row.evidence_index;
                map.entry(row.memory_id)
                    .or_insert_with(Vec::new)
                    .push(MemoryEvidenceRef {
                        session_id: row.session_id,
                        message_id: row.message_id,
                        tool_call_id: row.tool_call_id,
                        stage_id: row.stage_id,
                        note: row.note,
                    });
            }
        }

        Ok(map)
    }
}

fn bind_memory_record_upsert<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    record: &'q MemoryRecord,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    query
        .bind(&record.id.0)
        .bind(memory_kind_to_str(&record.kind))
        .bind(memory_scope_to_str(&record.scope))
        .bind(memory_status_to_str(&record.status))
        .bind(&record.title)
        .bind(&record.summary)
        .bind(serialize_json_for_db(
            "memory_record_upsert",
            "trigger_conditions",
            &record.trigger_conditions,
        ))
        .bind(serialize_json_for_db(
            "memory_record_upsert",
            "normalized_facts",
            &record.normalized_facts,
        ))
        .bind(serialize_json_for_db(
            "memory_record_upsert",
            "boundaries",
            &record.boundaries,
        ))
        .bind(record.confidence.map(f64::from))
        .bind(&record.source_session_id)
        .bind(&record.workspace_identity)
        .bind(record.created_at)
        .bind(record.updated_at)
        .bind(record.last_validated_at)
        .bind(record.expires_at)
        .bind(&record.derived_skill_name)
        .bind(&record.linked_skill_name)
        .bind(memory_validation_status_to_str(&record.validation_status))
}

fn bind_session_upsert<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    session: &'q Session,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    let usage = session.usage.as_ref();
    query
        .bind(&session.id)
        .bind(&session.project_id)
        .bind(&session.parent_id)
        .bind(&session.slug)
        .bind(&session.directory)
        .bind(&session.title)
        .bind(&session.version)
        .bind(session.share.as_ref().map(|s| s.url.as_str()))
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.additions as i64)
                .unwrap_or(0),
        )
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.deletions as i64)
                .unwrap_or(0),
        )
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.files as i64)
                .unwrap_or(0),
        )
        .bind(
            session.summary.as_ref().and_then(|s| {
                serialize_json_for_db("session_upsert_bind", "summary_diffs", &s.diffs)
            }),
        )
        .bind(
            session
                .revert
                .as_ref()
                .and_then(|r| serialize_json_for_db("session_upsert_bind", "revert", r)),
        )
        .bind(
            session
                .permission
                .as_ref()
                .and_then(|p| serialize_json_for_db("session_upsert_bind", "permission", p)),
        )
        .bind(serialize_json_for_db(
            "session_upsert_bind",
            "metadata",
            &session.metadata,
        ))
        .bind(usage.map(|u| u.input_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.context_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.output_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.reasoning_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_write_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_miss_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.total_cost).unwrap_or(0.0))
        .bind(status_to_string(&session.status))
        .bind(session.time.created)
        .bind(session.time.updated)
        .bind(session.time.compacting)
        .bind(session.time.archived)
}

fn role_to_str(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

fn message_model_provider_id(message: &SessionMessage) -> Option<&str> {
    message
        .metadata
        .get("model_provider")
        .and_then(|value| value.as_str())
}

fn message_model_id(message: &SessionMessage) -> Option<&str> {
    message
        .metadata
        .get("model_id")
        .and_then(|value| value.as_str())
}

fn message_usage_from_row(
    tokens_input: Option<i64>,
    tokens_context: Option<i64>,
    tokens_output: Option<i64>,
    tokens_reasoning: Option<i64>,
    tokens_cache_read: Option<i64>,
    tokens_cache_miss: Option<i64>,
    tokens_cache_write: Option<i64>,
    cost: Option<f64>,
) -> Option<rocode_types::MessageUsage> {
    let has_usage = tokens_input.unwrap_or(0) > 0
        || tokens_context.unwrap_or(0) > 0
        || tokens_output.unwrap_or(0) > 0
        || tokens_reasoning.unwrap_or(0) > 0
        || tokens_cache_read.unwrap_or(0) > 0
        || tokens_cache_miss.unwrap_or(0) > 0
        || tokens_cache_write.unwrap_or(0) > 0
        || cost.unwrap_or(0.0) > 0.0;
    if !has_usage {
        return None;
    }

    Some(rocode_types::MessageUsage {
        input_tokens: tokens_input.unwrap_or(0) as u64,
        output_tokens: tokens_output.unwrap_or(0) as u64,
        reasoning_tokens: tokens_reasoning.unwrap_or(0) as u64,
        cache_read_tokens: tokens_cache_read.unwrap_or(0) as u64,
        cache_miss_tokens: tokens_cache_miss.unwrap_or(0) as u64,
        cache_write_tokens: tokens_cache_write.unwrap_or(0) as u64,
        context_tokens: tokens_context.unwrap_or(0) as u64,
        total_cost: cost.unwrap_or(0.0),
    })
}

#[derive(Debug, FromRow)]
struct SessionRow {
    id: String,
    project_id: String,
    parent_id: Option<String>,
    slug: String,
    directory: String,
    title: String,
    version: String,
    share_url: Option<String>,
    summary_additions: Option<i64>,
    summary_deletions: Option<i64>,
    summary_files: Option<i64>,
    summary_diffs: Option<String>,
    revert: Option<String>,
    permission: Option<String>,
    metadata: Option<String>,
    usage_input_tokens: Option<i64>,
    usage_context_tokens: Option<i64>,
    usage_output_tokens: Option<i64>,
    usage_reasoning_tokens: Option<i64>,
    usage_cache_write_tokens: Option<i64>,
    usage_cache_read_tokens: Option<i64>,
    usage_cache_miss_tokens: Option<i64>,
    usage_total_cost: Option<f64>,
    status: String,
    created_at: i64,
    updated_at: i64,
    time_compacting: Option<i64>,
    time_archived: Option<i64>,
}

impl SessionRow {
    fn into_session(self) -> Session {
        let summary = if self.summary_additions.is_some()
            || self.summary_deletions.is_some()
            || self.summary_files.is_some()
        {
            Some(SessionSummary {
                additions: self.summary_additions.unwrap_or(0) as u64,
                deletions: self.summary_deletions.unwrap_or(0) as u64,
                files: self.summary_files.unwrap_or(0) as u64,
                diffs: self
                    .summary_diffs
                    .and_then(|d| serde_json::from_str(&d).ok()),
            })
        } else {
            None
        };

        let created_dt = DateTime::from_timestamp_millis(self.created_at).unwrap_or_else(Utc::now);
        let updated_dt = DateTime::from_timestamp_millis(self.updated_at).unwrap_or_else(Utc::now);

        Session {
            id: self.id,
            slug: self.slug,
            project_id: self.project_id,
            directory: self.directory,
            parent_id: self.parent_id,
            title: self.title,
            version: self.version,
            time: SessionTime {
                created: self.created_at,
                updated: self.updated_at,
                compacting: self.time_compacting,
                archived: self.time_archived,
            },
            messages: vec![],
            summary,
            share: self.share_url.map(|url| SessionShare { url }),
            revert: self.revert.and_then(|r| serde_json::from_str(&r).ok()),
            permission: self.permission.and_then(|p| serde_json::from_str(&p).ok()),
            metadata: self
                .metadata
                .and_then(|m| serde_json::from_str(&m).ok())
                .unwrap_or_default(),
            usage: if self.usage_input_tokens.is_some() {
                Some(SessionUsage {
                    input_tokens: self.usage_input_tokens.unwrap_or(0) as u64,
                    context_tokens: self.usage_context_tokens.unwrap_or(0) as u64,
                    output_tokens: self.usage_output_tokens.unwrap_or(0) as u64,
                    reasoning_tokens: self.usage_reasoning_tokens.unwrap_or(0) as u64,
                    cache_write_tokens: self.usage_cache_write_tokens.unwrap_or(0) as u64,
                    cache_read_tokens: self.usage_cache_read_tokens.unwrap_or(0) as u64,
                    cache_miss_tokens: self.usage_cache_miss_tokens.unwrap_or(0) as u64,
                    total_cost: self.usage_total_cost.unwrap_or(0.0),
                })
            } else {
                None
            },
            status: string_to_status(&self.status),
            created_at: created_dt,
            updated_at: updated_dt,
        }
    }
}

#[derive(Clone)]
pub struct SessionRepository {
    pool: SqlitePool,
}

impl SessionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, session: &Session) -> Result<(), DatabaseError> {
        let summary_diffs = session
            .summary
            .as_ref()
            .and_then(|s| serialize_json_for_db("session_create", "summary_diffs", &s.diffs));

        let revert_json = session
            .revert
            .as_ref()
            .and_then(|r| serialize_json_for_db("session_create", "revert", r));

        let permission_json = session
            .permission
            .as_ref()
            .and_then(|p| serialize_json_for_db("session_create", "permission", p));
        let metadata_json = serialize_json_for_db("session_create", "metadata", &session.metadata);

        let share_url = session.share.as_ref().map(|s| s.url.as_str());

        let usage = session.usage.as_ref();

        sqlx::query(
            r#"
            INSERT INTO sessions (
                id, project_id, parent_id, slug, directory, title, version, share_url,
                summary_additions, summary_deletions, summary_files, summary_diffs,
                revert, permission, metadata,
                usage_input_tokens, usage_context_tokens, usage_output_tokens, usage_reasoning_tokens,
                usage_cache_write_tokens, usage_cache_read_tokens, usage_cache_miss_tokens, usage_total_cost,
                status, created_at, updated_at, time_compacting, time_archived
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&session.id)
        .bind(&session.project_id)
        .bind(&session.parent_id)
        .bind(&session.slug)
        .bind(&session.directory)
        .bind(&session.title)
        .bind(&session.version)
        .bind(share_url)
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.additions as i64)
                .unwrap_or(0),
        )
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.deletions as i64)
                .unwrap_or(0),
        )
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.files as i64)
                .unwrap_or(0),
        )
        .bind(summary_diffs)
        .bind(revert_json)
        .bind(permission_json)
        .bind(metadata_json)
        .bind(usage.map(|u| u.input_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.context_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.output_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.reasoning_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_write_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_miss_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.total_cost).unwrap_or(0.0))
        .bind(status_to_string(&session.status))
        .bind(session.time.created)
        .bind(session.time.updated)
        .bind(session.time.compacting)
        .bind(session.time.archived)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn get(&self, id: &str) -> Result<Option<Session>, DatabaseError> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"SELECT 
                id, project_id, parent_id, slug, directory, title, version, share_url,
                summary_additions, summary_deletions, summary_files, summary_diffs,
                revert, permission, metadata,
                usage_input_tokens, usage_context_tokens, usage_output_tokens, usage_reasoning_tokens,
                usage_cache_write_tokens, usage_cache_read_tokens, usage_cache_miss_tokens, usage_total_cost,
                status, created_at, updated_at, time_compacting, time_archived
            FROM sessions WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(row.map(|r| r.into_session()))
    }

    pub async fn list(
        &self,
        project_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Session>, DatabaseError> {
        let rows = match project_id {
            Some(pid) => sqlx::query_as::<_, SessionRow>(
                r#"SELECT 
                        id, project_id, parent_id, slug, directory, title, version, share_url,
                        summary_additions, summary_deletions, summary_files, summary_diffs,
                        revert, permission, metadata,
                        usage_input_tokens, usage_context_tokens, usage_output_tokens, usage_reasoning_tokens,
                        usage_cache_write_tokens, usage_cache_read_tokens, usage_cache_miss_tokens, usage_total_cost,
                        status, created_at, updated_at, time_compacting, time_archived
                    FROM sessions WHERE project_id = ? 
                    ORDER BY updated_at DESC LIMIT ?"#,
            )
            .bind(pid)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?,
            None => sqlx::query_as::<_, SessionRow>(
                r#"SELECT 
                        id, project_id, parent_id, slug, directory, title, version, share_url,
                        summary_additions, summary_deletions, summary_files, summary_diffs,
                        revert, permission, metadata,
                        usage_input_tokens, usage_context_tokens, usage_output_tokens, usage_reasoning_tokens,
                        usage_cache_write_tokens, usage_cache_read_tokens, usage_cache_miss_tokens, usage_total_cost,
                        status, created_at, updated_at, time_compacting, time_archived
                    FROM sessions 
                    ORDER BY updated_at DESC LIMIT ?"#,
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?,
        };

        Ok(rows.into_iter().map(|r| r.into_session()).collect())
    }

    pub async fn update(&self, session: &Session) -> Result<(), DatabaseError> {
        let summary_diffs = session
            .summary
            .as_ref()
            .and_then(|s| serialize_json_for_db("session_update", "summary_diffs", &s.diffs));

        let revert_json = session
            .revert
            .as_ref()
            .and_then(|r| serialize_json_for_db("session_update", "revert", r));

        let permission_json = session
            .permission
            .as_ref()
            .and_then(|p| serialize_json_for_db("session_update", "permission", p));

        let share_url = session.share.as_ref().map(|s| s.url.as_str());
        let metadata_json = serialize_json_for_db("session_update", "metadata", &session.metadata);

        let usage = session.usage.as_ref();

        sqlx::query(
            r#"
            UPDATE sessions SET
                title = ?, version = ?, share_url = ?,
                summary_additions = ?, summary_deletions = ?, summary_files = ?, summary_diffs = ?,
                revert = ?, permission = ?, metadata = ?,
                usage_input_tokens = ?, usage_context_tokens = ?, usage_output_tokens = ?, usage_reasoning_tokens = ?,
                usage_cache_write_tokens = ?, usage_cache_read_tokens = ?, usage_cache_miss_tokens = ?, usage_total_cost = ?,
                status = ?, updated_at = ?, time_compacting = ?, time_archived = ?
            WHERE id = ?
            "#,
        )
        .bind(&session.title)
        .bind(&session.version)
        .bind(share_url)
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.additions as i64)
                .unwrap_or(0),
        )
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.deletions as i64)
                .unwrap_or(0),
        )
        .bind(
            session
                .summary
                .as_ref()
                .map(|s| s.files as i64)
                .unwrap_or(0),
        )
        .bind(summary_diffs)
        .bind(revert_json)
        .bind(permission_json)
        .bind(metadata_json)
        .bind(usage.map(|u| u.input_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.context_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.output_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.reasoning_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_write_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_miss_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.total_cost).unwrap_or(0.0))
        .bind(status_to_string(&session.status))
        .bind(session.time.updated)
        .bind(session.time.compacting)
        .bind(session.time.archived)
        .bind(&session.id)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn upsert(&self, session: &Session) -> Result<(), DatabaseError> {
        bind_session_upsert(sqlx::query(SESSION_UPSERT_SQL), session)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_children(&self, parent_id: &str) -> Result<Vec<Session>, DatabaseError> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"SELECT 
                id, project_id, parent_id, slug, directory, title, version, share_url,
                summary_additions, summary_deletions, summary_files, summary_diffs,
                revert, permission, metadata,
                usage_input_tokens, usage_context_tokens, usage_output_tokens, usage_reasoning_tokens,
                usage_cache_write_tokens, usage_cache_read_tokens, usage_cache_miss_tokens, usage_total_cost,
                status, created_at, updated_at, time_compacting, time_archived
            FROM sessions WHERE parent_id = ? 
            ORDER BY created_at DESC"#,
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_session()).collect())
    }

    /// Atomically upsert a session, upsert its messages, and delete stale messages
    /// that no longer exist in the session layer (e.g. after revert/delete).
    pub async fn flush_with_messages(
        &self,
        session: &Session,
        messages: &[SessionMessage],
    ) -> Result<(), DatabaseError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;

        // Upsert session
        bind_session_upsert(sqlx::query(SESSION_UPSERT_SQL), session)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        // Upsert messages
        for msg in messages {
            let data_json = serde_json::to_string(&msg.parts)
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            let metadata_json = serde_json::to_string(&msg.metadata)
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            let usage = msg.usage.as_ref();
            sqlx::query(MESSAGE_UPSERT_SQL)
                .bind(&msg.id)
                .bind(&msg.session_id)
                .bind(role_to_str(&msg.role))
                .bind(msg.created_at.timestamp_millis())
                .bind(message_model_provider_id(msg))
                .bind(message_model_id(msg))
                .bind(usage.map(|u| u.input_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.context_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.output_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.reasoning_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.cache_miss_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.cache_write_tokens as i64).unwrap_or(0))
                .bind(usage.map(|u| u.total_cost).unwrap_or(0.0))
                .bind(&msg.finish)
                .bind(&metadata_json)
                .bind(&data_json)
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        }

        // Delete stale messages
        let keep_ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
        if keep_ids.is_empty() {
            sqlx::query("DELETE FROM messages WHERE session_id = ?")
                .bind(&session.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        } else if keep_ids.len() <= 998 {
            let placeholders: String = keep_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "DELETE FROM messages WHERE session_id = ? AND id NOT IN ({})",
                placeholders
            );
            let mut query = sqlx::query(&sql).bind(&session.id);
            for id in &keep_ids {
                query = query.bind(*id);
            }
            query
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        } else {
            sqlx::query("CREATE TEMP TABLE IF NOT EXISTS _keep_msg_ids (id TEXT PRIMARY KEY)")
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            sqlx::query("DELETE FROM _keep_msg_ids")
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            for chunk in keep_ids.chunks(500) {
                let placeholders: String =
                    chunk.iter().map(|_| "(?)").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "INSERT OR IGNORE INTO _keep_msg_ids (id) VALUES {}",
                    placeholders
                );
                let mut query = sqlx::query(&sql);
                for id in chunk {
                    query = query.bind(*id);
                }
                query
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            }
            sqlx::query(
                "DELETE FROM messages WHERE session_id = ? AND id NOT IN (SELECT id FROM _keep_msg_ids)",
            )
            .bind(&session.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            sqlx::query("DROP TABLE IF EXISTS _keep_msg_ids")
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;
        Ok(())
    }
}

fn status_to_string(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Completed => "completed",
        SessionStatus::Archived => "archived",
        SessionStatus::Compacting => "compacting",
    }
}

fn string_to_status(s: &str) -> SessionStatus {
    match s {
        "completed" => SessionStatus::Completed,
        "archived" => SessionStatus::Archived,
        "compacting" => SessionStatus::Compacting,
        _ => SessionStatus::Active,
    }
}

fn repeat_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_json_vec(raw: Option<String>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn validation_run_id(report: &MemoryValidationReport) -> String {
    match report.record_id.as_ref() {
        Some(record_id) => format!("validation_{}_{}", record_id.0, report.checked_at),
        None => format!("validation_global_{}", report.checked_at),
    }
}

fn memory_kind_to_str(kind: &MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Preference => "preference",
        MemoryKind::EnvironmentFact => "environment_fact",
        MemoryKind::WorkspaceConvention => "workspace_convention",
        MemoryKind::Lesson => "lesson",
        MemoryKind::Pattern => "pattern",
        MemoryKind::MethodologyCandidate => "methodology_candidate",
    }
}

fn string_to_memory_kind(value: &str) -> MemoryKind {
    match value {
        "preference" => MemoryKind::Preference,
        "environment_fact" => MemoryKind::EnvironmentFact,
        "workspace_convention" => MemoryKind::WorkspaceConvention,
        "lesson" => MemoryKind::Lesson,
        "methodology_candidate" => MemoryKind::MethodologyCandidate,
        _ => MemoryKind::Pattern,
    }
}

fn memory_scope_to_str(scope: &MemoryScope) -> &'static str {
    match scope {
        MemoryScope::GlobalUser => "global_user",
        MemoryScope::GlobalWorkspace => "global_workspace",
        MemoryScope::WorkspaceShared => "workspace_shared",
        MemoryScope::WorkspaceSandbox => "workspace_sandbox",
        MemoryScope::SessionEphemeral => "session_ephemeral",
    }
}

fn string_to_memory_scope(value: &str) -> MemoryScope {
    match value {
        "global_user" => MemoryScope::GlobalUser,
        "global_workspace" => MemoryScope::GlobalWorkspace,
        "workspace_sandbox" => MemoryScope::WorkspaceSandbox,
        "session_ephemeral" => MemoryScope::SessionEphemeral,
        _ => MemoryScope::WorkspaceShared,
    }
}

fn memory_status_to_str(status: &MemoryStatus) -> &'static str {
    match status {
        MemoryStatus::Candidate => "candidate",
        MemoryStatus::Validated => "validated",
        MemoryStatus::Consolidated => "consolidated",
        MemoryStatus::Archived => "archived",
        MemoryStatus::Rejected => "rejected",
    }
}

fn string_to_memory_status(value: &str) -> MemoryStatus {
    match value {
        "validated" => MemoryStatus::Validated,
        "consolidated" => MemoryStatus::Consolidated,
        "archived" => MemoryStatus::Archived,
        "rejected" => MemoryStatus::Rejected,
        _ => MemoryStatus::Candidate,
    }
}

fn memory_validation_status_to_str(status: &MemoryValidationStatus) -> &'static str {
    match status {
        MemoryValidationStatus::Pending => "pending",
        MemoryValidationStatus::Passed => "passed",
        MemoryValidationStatus::Warning => "warning",
        MemoryValidationStatus::Failed => "failed",
    }
}

fn string_to_memory_validation_status(value: &str) -> MemoryValidationStatus {
    match value {
        "passed" => MemoryValidationStatus::Passed,
        "warning" => MemoryValidationStatus::Warning,
        "failed" => MemoryValidationStatus::Failed,
        _ => MemoryValidationStatus::Pending,
    }
}

fn memory_rule_pack_kind_to_str(kind: &MemoryRulePackKind) -> &'static str {
    match kind {
        MemoryRulePackKind::Validation => "validation",
        MemoryRulePackKind::Consolidation => "consolidation",
        MemoryRulePackKind::Reflection => "reflection",
    }
}

fn string_to_memory_rule_pack_kind(value: &str) -> MemoryRulePackKind {
    match value {
        "validation" => MemoryRulePackKind::Validation,
        "reflection" => MemoryRulePackKind::Reflection,
        _ => MemoryRulePackKind::Consolidation,
    }
}

fn parse_memory_rule_pack_row(row: MemoryRulePackRow) -> Result<MemoryRulePack, DatabaseError> {
    match serde_json::from_str::<MemoryRulePack>(&row.body) {
        Ok(mut pack) => {
            pack.id = row.id;
            pack.rule_pack_kind = string_to_memory_rule_pack_kind(&row.rule_pack_kind);
            pack.version = row.version;
            pack.created_at = row.created_at;
            pack.updated_at = row.updated_at;
            Ok(pack)
        }
        Err(_) => Ok(MemoryRulePack {
            id: row.id,
            rule_pack_kind: string_to_memory_rule_pack_kind(&row.rule_pack_kind),
            version: row.version,
            rules: Vec::new(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }),
    }
}

#[derive(Clone)]
pub struct MessageRepository {
    pool: SqlitePool,
}

impl MessageRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, message: &SessionMessage) -> Result<(), DatabaseError> {
        let data_json = serde_json::to_string(&message.parts)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        let metadata_json = serde_json::to_string(&message.metadata)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        let usage = message.usage.as_ref();

        let role_str = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        };

        sqlx::query(
            r#"
            INSERT INTO messages (
                id, session_id, role, created_at, provider_id, model_id,
                tokens_input, tokens_context, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_miss, tokens_cache_write,
                cost, finish, metadata, data
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&message.id)
        .bind(&message.session_id)
        .bind(role_str)
        .bind(message.created_at.timestamp_millis())
        .bind(message_model_provider_id(message))
        .bind(message_model_id(message))
        .bind(usage.map(|u| u.input_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.context_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.output_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.reasoning_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_miss_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.cache_write_tokens as i64).unwrap_or(0))
        .bind(usage.map(|u| u.total_cost).unwrap_or(0.0))
        .bind(&message.finish)
        .bind(&metadata_json)
        .bind(&data_json)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn upsert(&self, message: &SessionMessage) -> Result<(), DatabaseError> {
        let data_json = serde_json::to_string(&message.parts)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        let metadata_json = serde_json::to_string(&message.metadata)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
        let usage = message.usage.as_ref();

        sqlx::query(MESSAGE_UPSERT_SQL)
            .bind(&message.id)
            .bind(&message.session_id)
            .bind(role_to_str(&message.role))
            .bind(message.created_at.timestamp_millis())
            .bind(message_model_provider_id(message))
            .bind(message_model_id(message))
            .bind(usage.map(|u| u.input_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.context_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.output_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.reasoning_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.cache_miss_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.cache_write_tokens as i64).unwrap_or(0))
            .bind(usage.map(|u| u.total_cost).unwrap_or(0.0))
            .bind(&message.finish)
            .bind(&metadata_json)
            .bind(&data_json)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionMessage>, DatabaseError> {
        #[derive(FromRow)]
        struct MessageRow {
            id: String,
            session_id: String,
            role: String,
            created_at: i64,
            tokens_input: Option<i64>,
            tokens_context: Option<i64>,
            tokens_output: Option<i64>,
            tokens_reasoning: Option<i64>,
            tokens_cache_read: Option<i64>,
            tokens_cache_miss: Option<i64>,
            tokens_cache_write: Option<i64>,
            cost: Option<f64>,
            finish: Option<String>,
            metadata: Option<String>,
            data: Option<String>,
        }

        let rows = sqlx::query_as::<_, MessageRow>(
            r#"SELECT
                   id, session_id, role, created_at,
                   tokens_input, tokens_context, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_miss, tokens_cache_write,
                   cost, finish, metadata, data
               FROM messages WHERE session_id = ? ORDER BY created_at ASC"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let messages: Vec<SessionMessage> = rows
            .into_iter()
            .filter_map(|row| {
                let msg_role = match row.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    "tool" => MessageRole::Tool,
                    _ => return None,
                };

                let parts: Vec<MessagePart> = row
                    .data
                    .and_then(|c| serde_json::from_str(&c).ok())
                    .unwrap_or_default();

                let created =
                    DateTime::from_timestamp_millis(row.created_at).unwrap_or_else(Utc::now);

                Some(SessionMessage {
                    id: row.id,
                    session_id: row.session_id,
                    role: msg_role,
                    parts,
                    created_at: created,
                    metadata: row
                        .metadata
                        .and_then(|m| serde_json::from_str(&m).ok())
                        .unwrap_or_default(),
                    usage: message_usage_from_row(
                        row.tokens_input,
                        row.tokens_context,
                        row.tokens_output,
                        row.tokens_reasoning,
                        row.tokens_cache_read,
                        row.tokens_cache_miss,
                        row.tokens_cache_write,
                        row.cost,
                    ),
                    finish: row.finish,
                })
            })
            .collect();

        Ok(messages)
    }

    pub async fn get(&self, id: &str) -> Result<Option<SessionMessage>, DatabaseError> {
        #[derive(FromRow)]
        struct MessageRow {
            id: String,
            session_id: String,
            role: String,
            created_at: i64,
            tokens_input: Option<i64>,
            tokens_context: Option<i64>,
            tokens_output: Option<i64>,
            tokens_reasoning: Option<i64>,
            tokens_cache_read: Option<i64>,
            tokens_cache_miss: Option<i64>,
            tokens_cache_write: Option<i64>,
            cost: Option<f64>,
            finish: Option<String>,
            metadata: Option<String>,
            data: Option<String>,
        }

        let row = sqlx::query_as::<_, MessageRow>(
            r#"SELECT
                   id, session_id, role, created_at,
                   tokens_input, tokens_context, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_miss, tokens_cache_write,
                   cost, finish, metadata, data
               FROM messages WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        match row {
            Some(row) => {
                let msg_role = match row.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    "tool" => MessageRole::Tool,
                    _ => return Ok(None),
                };

                let parts: Vec<MessagePart> = row
                    .data
                    .and_then(|c| serde_json::from_str(&c).ok())
                    .unwrap_or_default();

                let created =
                    DateTime::from_timestamp_millis(row.created_at).unwrap_or_else(Utc::now);

                Ok(Some(SessionMessage {
                    id: row.id,
                    session_id: row.session_id,
                    role: msg_role,
                    parts,
                    created_at: created,
                    metadata: row
                        .metadata
                        .and_then(|m| serde_json::from_str(&m).ok())
                        .unwrap_or_default(),
                    usage: message_usage_from_row(
                        row.tokens_input,
                        row.tokens_context,
                        row.tokens_output,
                        row.tokens_reasoning,
                        row.tokens_cache_read,
                        row.tokens_cache_miss,
                        row.tokens_cache_write,
                        row.cost,
                    ),
                    finish: row.finish,
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn delete(&self, id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_for_session(&self, session_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
    pub position: i64,
}

pub struct TodoRepository {
    pool: SqlitePool,
}

impl TodoRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_for_session(&self, session_id: &str) -> Result<Vec<TodoItem>, DatabaseError> {
        #[derive(FromRow)]
        struct TodoRow {
            todo_id: String,
            content: String,
            status: String,
            priority: String,
            position: i64,
        }

        let rows = sqlx::query_as::<_, TodoRow>(
            r#"SELECT todo_id, content, status, priority, position 
               FROM todos WHERE session_id = ? ORDER BY position ASC"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let todos: Vec<TodoItem> = rows
            .into_iter()
            .map(|row| TodoItem {
                id: row.todo_id,
                content: row.content,
                status: row.status,
                priority: row.priority,
                position: row.position,
            })
            .collect();

        Ok(todos)
    }

    pub async fn upsert(&self, session_id: &str, todo: &TodoItem) -> Result<(), DatabaseError> {
        let now = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO todos (session_id, todo_id, content, status, priority, position, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(session_id, todo_id) DO UPDATE SET
                content = excluded.content,
                status = excluded.status,
                priority = excluded.priority,
                position = excluded.position,
                updated_at = excluded.updated_at
            "#
        )
        .bind(session_id)
        .bind(&todo.id)
        .bind(&todo.content)
        .bind(&todo.status)
        .bind(&todo.priority)
        .bind(todo.position)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete(&self, session_id: &str, todo_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM todos WHERE session_id = ? AND todo_id = ?")
            .bind(session_id)
            .bind(todo_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_for_session(&self, session_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM todos WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionShareRow {
    pub session_id: String,
    pub id: String,
    pub secret: String,
    pub url: String,
}

pub struct ShareRepository {
    pool: SqlitePool,
}

impl ShareRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, session_id: &str) -> Result<Option<SessionShareRow>, DatabaseError> {
        #[derive(FromRow)]
        struct ShareRow {
            session_id: String,
            id: String,
            secret: String,
            url: String,
        }

        let row = sqlx::query_as::<_, ShareRow>(
            r#"SELECT session_id, id, secret, url FROM session_shares WHERE session_id = ?"#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(row.map(|r| SessionShareRow {
            session_id: r.session_id,
            id: r.id,
            secret: r.secret,
            url: r.url,
        }))
    }

    pub async fn upsert(&self, share: &SessionShareRow) -> Result<(), DatabaseError> {
        let now = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO session_shares (session_id, id, secret, url, created_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(session_id) DO UPDATE SET
                id = excluded.id,
                secret = excluded.secret,
                url = excluded.url
            "#,
        )
        .bind(&share.session_id)
        .bind(&share.id)
        .bind(&share.secret)
        .bind(&share.url)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete(&self, session_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM session_shares WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartRow {
    pub id: String,
    pub message_id: String,
    pub session_id: String,
    pub part_type: String,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_arguments: Option<String>,
    pub tool_result: Option<String>,
    pub tool_error: Option<String>,
    pub tool_status: Option<String>,
    pub sort_order: i64,
}

pub struct PartRepository {
    pool: SqlitePool,
}

impl PartRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_for_message(&self, message_id: &str) -> Result<Vec<PartRow>, DatabaseError> {
        #[derive(FromRow)]
        struct Row {
            id: String,
            message_id: String,
            session_id: String,
            part_type: String,
            text: Option<String>,
            tool_name: Option<String>,
            tool_call_id: Option<String>,
            tool_arguments: Option<String>,
            tool_result: Option<String>,
            tool_error: Option<String>,
            tool_status: Option<String>,
            sort_order: i64,
        }

        let rows = sqlx::query_as::<_, Row>(
            r#"SELECT id, message_id, session_id, part_type, text, 
                      tool_name, tool_call_id, tool_arguments, tool_result, 
                      tool_error, tool_status, sort_order
               FROM parts WHERE message_id = ? ORDER BY sort_order ASC"#,
        )
        .bind(message_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| PartRow {
                id: r.id,
                message_id: r.message_id,
                session_id: r.session_id,
                part_type: r.part_type,
                text: r.text,
                tool_name: r.tool_name,
                tool_call_id: r.tool_call_id,
                tool_arguments: r.tool_arguments,
                tool_result: r.tool_result,
                tool_error: r.tool_error,
                tool_status: r.tool_status,
                sort_order: r.sort_order,
            })
            .collect())
    }

    pub async fn list_for_session(&self, session_id: &str) -> Result<Vec<PartRow>, DatabaseError> {
        #[derive(FromRow)]
        struct Row {
            id: String,
            message_id: String,
            session_id: String,
            part_type: String,
            text: Option<String>,
            tool_name: Option<String>,
            tool_call_id: Option<String>,
            tool_arguments: Option<String>,
            tool_result: Option<String>,
            tool_error: Option<String>,
            tool_status: Option<String>,
            sort_order: i64,
        }

        let rows = sqlx::query_as::<_, Row>(
            r#"SELECT id, message_id, session_id, part_type, text, 
                      tool_name, tool_call_id, tool_arguments, tool_result, 
                      tool_error, tool_status, sort_order
               FROM parts WHERE session_id = ? ORDER BY sort_order ASC"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| PartRow {
                id: r.id,
                message_id: r.message_id,
                session_id: r.session_id,
                part_type: r.part_type,
                text: r.text,
                tool_name: r.tool_name,
                tool_call_id: r.tool_call_id,
                tool_arguments: r.tool_arguments,
                tool_result: r.tool_result,
                tool_error: r.tool_error,
                tool_status: r.tool_status,
                sort_order: r.sort_order,
            })
            .collect())
    }

    pub async fn upsert(&self, part: &PartRow) -> Result<(), DatabaseError> {
        let now = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO parts (id, message_id, session_id, part_type, text, 
                              tool_name, tool_call_id, tool_arguments, tool_result, 
                              tool_error, tool_status, sort_order, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                text = excluded.text,
                tool_name = excluded.tool_name,
                tool_call_id = excluded.tool_call_id,
                tool_arguments = excluded.tool_arguments,
                tool_result = excluded.tool_result,
                tool_error = excluded.tool_error,
                tool_status = excluded.tool_status,
                sort_order = excluded.sort_order
            "#,
        )
        .bind(&part.id)
        .bind(&part.message_id)
        .bind(&part.session_id)
        .bind(&part.part_type)
        .bind(&part.text)
        .bind(&part.tool_name)
        .bind(&part.tool_call_id)
        .bind(&part.tool_arguments)
        .bind(&part.tool_result)
        .bind(&part.tool_error)
        .bind(&part.tool_status)
        .bind(part.sort_order)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM parts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_for_message(&self, message_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM parts WHERE message_id = ?")
            .bind(message_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_for_session(&self, session_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM parts WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use chrono::Utc;
    use rocode_types::{
        MessageRole, PartType, Session, SessionMessage, SessionStatus, SessionTime,
    };
    use std::collections::HashMap;

    fn make_session(id: &str) -> Session {
        Session {
            id: id.to_string(),
            slug: format!("slug-{}", id),
            project_id: "proj-1".to_string(),
            directory: "/tmp/test".to_string(),
            parent_id: None,
            title: format!("Session {}", id),
            version: "1.0.0".to_string(),
            time: SessionTime::default(),
            messages: vec![],
            summary: None,
            share: None,
            revert: None,
            permission: None,
            usage: None,
            status: SessionStatus::Active,
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_message(id: &str, session_id: &str, role: MessageRole) -> SessionMessage {
        SessionMessage {
            id: id.to_string(),
            session_id: session_id.to_string(),
            role,
            parts: vec![],
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        }
    }

    fn make_memory_record(id: &str, scope: MemoryScope) -> MemoryRecord {
        MemoryRecord {
            id: MemoryRecordId(id.to_string()),
            kind: MemoryKind::Pattern,
            scope,
            status: MemoryStatus::Candidate,
            title: format!("Memory {}", id),
            summary: "Reusable observation".to_string(),
            trigger_conditions: vec!["when running tests".to_string()],
            normalized_facts: vec!["tool:cargo_test".to_string()],
            boundaries: vec!["Candidate only".to_string()],
            confidence: Some(0.7),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some("ses_1".to_string()),
                message_id: Some("msg_1".to_string()),
                tool_call_id: Some("call_1".to_string()),
                stage_id: Some("stage_1".to_string()),
                note: Some("captured from tool output".to_string()),
            }],
            source_session_id: Some("ses_1".to_string()),
            workspace_identity: Some("ws:test".to_string()),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_100,
            last_validated_at: None,
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Pending,
        }
    }

    #[tokio::test]
    async fn session_metadata_roundtrips() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());

        let mut session = make_session("s_meta");
        session.metadata.insert(
            "scheduler_profile".to_string(),
            serde_json::json!("sisyphus"),
        );
        session
            .metadata
            .insert("scheduler_applied".to_string(), serde_json::json!(true));

        session_repo.upsert(&session).await.unwrap();

        let loaded = session_repo.get("s_meta").await.unwrap().unwrap();
        assert_eq!(
            loaded.metadata.get("scheduler_profile"),
            Some(&serde_json::json!("sisyphus"))
        );
        assert_eq!(
            loaded.metadata.get("scheduler_applied"),
            Some(&serde_json::json!(true))
        );
    }

    #[tokio::test]
    async fn message_metadata_roundtrips() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        session_repo.upsert(&make_session("s_meta")).await.unwrap();

        let mut message = make_message("m_meta", "s_meta", MessageRole::User);
        message.metadata.insert(
            "resolved_system_prompt".to_string(),
            serde_json::json!("You are Sisyphus"),
        );
        message.metadata.insert(
            "resolved_scheduler_profile".to_string(),
            serde_json::json!("sisyphus"),
        );
        message
            .metadata
            .insert("mode".to_string(), serde_json::json!("sisyphus"));

        message_repo.create(&message).await.unwrap();

        let loaded = message_repo.get("m_meta").await.unwrap().unwrap();
        assert_eq!(
            loaded.metadata.get("resolved_system_prompt"),
            Some(&serde_json::json!("You are Sisyphus"))
        );
        assert_eq!(
            loaded.metadata.get("resolved_scheduler_profile"),
            Some(&serde_json::json!("sisyphus"))
        );
        assert_eq!(
            loaded.metadata.get("mode"),
            Some(&serde_json::json!("sisyphus"))
        );
    }

    #[tokio::test]
    async fn message_usage_roundtrips() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        session_repo.upsert(&make_session("s_usage")).await.unwrap();

        let mut message = make_message("m_usage", "s_usage", MessageRole::Assistant);
        message
            .metadata
            .insert("model_provider".to_string(), serde_json::json!("deepseek"));
        message.metadata.insert(
            "model_id".to_string(),
            serde_json::json!("deepseek-v4-flash"),
        );
        message.usage = Some(rocode_types::MessageUsage {
            input_tokens: 43_218,
            output_tokens: 155,
            reasoning_tokens: 12,
            cache_read_tokens: 4,
            cache_miss_tokens: 6,
            cache_write_tokens: 2,
            context_tokens: 12_345,
            total_cost: 0.1234,
        });

        message_repo.upsert(&message).await.unwrap();

        let loaded = message_repo.get("m_usage").await.unwrap().unwrap();
        let usage = loaded.usage.expect("usage should roundtrip");
        assert_eq!(usage.input_tokens, 43_218);
        assert_eq!(usage.output_tokens, 155);
        assert_eq!(usage.reasoning_tokens, 12);
        assert_eq!(usage.cache_read_tokens, 4);
        assert_eq!(usage.cache_miss_tokens, 6);
        assert_eq!(usage.cache_write_tokens, 2);
        assert_eq!(usage.context_tokens, 12_345);
        assert!((usage.total_cost - 0.1234).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn session_usage_context_tokens_roundtrips() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());

        let mut session = make_session("s_usage_context");
        session.usage = Some(SessionUsage {
            input_tokens: 100_000,
            context_tokens: 42_000,
            output_tokens: 250,
            reasoning_tokens: 32,
            cache_write_tokens: 7,
            cache_read_tokens: 11,
            cache_miss_tokens: 13,
            total_cost: 0.5678,
        });

        session_repo.upsert(&session).await.unwrap();

        let loaded = session_repo.get("s_usage_context").await.unwrap().unwrap();
        let usage = loaded.usage.expect("usage should roundtrip");
        assert_eq!(usage.input_tokens, 100_000);
        assert_eq!(usage.context_tokens, 42_000);
        assert_eq!(usage.output_tokens, 250);
        assert_eq!(usage.reasoning_tokens, 32);
        assert_eq!(usage.cache_write_tokens, 7);
        assert_eq!(usage.cache_read_tokens, 11);
        assert_eq!(usage.cache_miss_tokens, 13);
        assert!((usage.total_cost - 0.5678).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn flush_with_messages_atomicity() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        let session = make_session("s1");
        let msgs = vec![
            make_message("m1", "s1", MessageRole::User),
            make_message("m2", "s1", MessageRole::Assistant),
        ];

        session_repo
            .flush_with_messages(&session, &msgs)
            .await
            .unwrap();

        let loaded = session_repo.get("s1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().title, "Session s1");

        let loaded_msgs = message_repo.list_for_session("s1").await.unwrap();
        assert_eq!(loaded_msgs.len(), 2);
        assert_eq!(loaded_msgs[0].id, "m1");
        assert_eq!(loaded_msgs[1].id, "m2");
    }

    #[tokio::test]
    async fn flush_with_messages_roundtrips_message_payload_and_usage() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        let session = make_session("s_flush_usage");
        let mut message = make_message("m_flush_usage", "s_flush_usage", MessageRole::Assistant);
        message.parts.push(MessagePart {
            id: "prt_flush_usage".to_string(),
            part_type: PartType::Text {
                text: "answer".to_string(),
                synthetic: None,
                ignored: None,
            },
            created_at: Utc::now(),
            message_id: None,
        });
        message
            .metadata
            .insert("model_provider".to_string(), serde_json::json!("deepseek"));
        message.metadata.insert(
            "model_id".to_string(),
            serde_json::json!("deepseek-v4-flash"),
        );
        message.metadata.insert(
            "scheduler_profile".to_string(),
            serde_json::json!("sisyphus"),
        );
        message.finish = Some("stop".to_string());
        message.usage = Some(rocode_types::MessageUsage {
            input_tokens: 19_199,
            context_tokens: 19_199,
            output_tokens: 121,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_miss_tokens: 0,
            cache_write_tokens: 0,
            total_cost: 0.00272174,
        });

        session_repo
            .flush_with_messages(&session, &[message])
            .await
            .unwrap();

        let loaded = message_repo
            .get("m_flush_usage")
            .await
            .unwrap()
            .expect("message should persist");
        assert_eq!(loaded.finish.as_deref(), Some("stop"));
        assert_eq!(
            loaded.metadata.get("scheduler_profile"),
            Some(&serde_json::json!("sisyphus"))
        );
        assert_eq!(loaded.parts.len(), 1);
        match &loaded.parts[0].part_type {
            PartType::Text { text, .. } => assert_eq!(text, "answer"),
            other => panic!("expected text part, got {other:?}"),
        }
        let usage = loaded.usage.expect("usage should persist");
        assert_eq!(usage.input_tokens, 19_199);
        assert_eq!(usage.context_tokens, 19_199);
        assert_eq!(usage.output_tokens, 121);
        assert!((usage.total_cost - 0.00272174).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn flush_deletes_stale_messages() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        let session = make_session("s1");
        let msgs = vec![
            make_message("m1", "s1", MessageRole::User),
            make_message("m2", "s1", MessageRole::Assistant),
            make_message("m3", "s1", MessageRole::User),
        ];

        session_repo
            .flush_with_messages(&session, &msgs)
            .await
            .unwrap();
        assert_eq!(message_repo.list_for_session("s1").await.unwrap().len(), 3);

        // Simulate revert: flush with only m1
        let msgs_after_revert = vec![make_message("m1", "s1", MessageRole::User)];
        session_repo
            .flush_with_messages(&session, &msgs_after_revert)
            .await
            .unwrap();

        let remaining = message_repo.list_for_session("s1").await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "m1");

        assert!(message_repo.get("m2").await.unwrap().is_none());
        assert!(message_repo.get("m3").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_stale_large_set_uses_temp_table() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        let session = make_session("s1");

        // 1100 messages exceeds the 998 inline limit → temp table path
        let mut msgs: Vec<SessionMessage> = (0..1100)
            .map(|i| make_message(&format!("m{}", i), "s1", MessageRole::User))
            .collect();

        session_repo
            .flush_with_messages(&session, &msgs)
            .await
            .unwrap();
        assert_eq!(
            message_repo.list_for_session("s1").await.unwrap().len(),
            1100
        );

        // Remove last 100
        msgs.truncate(1000);
        session_repo
            .flush_with_messages(&session, &msgs)
            .await
            .unwrap();

        let remaining = message_repo.list_for_session("s1").await.unwrap();
        assert_eq!(remaining.len(), 1000);
        assert!(message_repo.get("m1099").await.unwrap().is_none());
        assert!(message_repo.get("m0").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn upsert_updates_existing_session() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());

        let mut session = make_session("s1");
        session_repo.upsert(&session).await.unwrap();

        session.title = "Updated Title".to_string();
        session_repo.upsert(&session).await.unwrap();

        let loaded = session_repo.get("s1").await.unwrap().unwrap();
        assert_eq!(loaded.title, "Updated Title");

        let all = session_repo.list(None, 100).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn flush_rolls_back_on_mid_transaction_failure() {
        let db = Database::in_memory().await.unwrap();
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        // Establish baseline: session "v1" with messages m1, m2
        let mut session = make_session("s1");
        session.title = "v1".to_string();
        let msgs = vec![
            make_message("m1", "s1", MessageRole::User),
            make_message("m2", "s1", MessageRole::Assistant),
        ];
        session_repo
            .flush_with_messages(&session, &msgs)
            .await
            .unwrap();

        // Sabotage: rename messages table so message upsert fails inside the tx
        sqlx::query("ALTER TABLE messages RENAME TO messages_backup")
            .execute(db.pool())
            .await
            .unwrap();

        // Attempt flush with updated title — session upsert succeeds within tx,
        // but message upsert hits the missing table and the whole tx should roll back.
        session.title = "v2".to_string();
        let new_msgs = vec![make_message("m3", "s1", MessageRole::User)];
        let result = session_repo.flush_with_messages(&session, &new_msgs).await;
        assert!(
            result.is_err(),
            "flush should fail when messages table is missing"
        );

        // Restore messages table
        sqlx::query("ALTER TABLE messages_backup RENAME TO messages")
            .execute(db.pool())
            .await
            .unwrap();

        // Verify rollback: session title must still be "v1"
        let loaded = session_repo.get("s1").await.unwrap().unwrap();
        assert_eq!(
            loaded.title, "v1",
            "session upsert should have been rolled back"
        );

        // Verify original messages are intact
        let loaded_msgs = message_repo.list_for_session("s1").await.unwrap();
        assert_eq!(
            loaded_msgs.len(),
            2,
            "original messages should survive the failed tx"
        );
        assert_eq!(loaded_msgs[0].id, "m1");
        assert_eq!(loaded_msgs[1].id, "m2");
    }

    #[tokio::test]
    async fn memory_repository_roundtrips_record_and_evidence() {
        let db = Database::in_memory().await.unwrap();
        let repo = MemoryRepository::new(db.pool().clone());
        let record = make_memory_record("mem_1", MemoryScope::WorkspaceShared);

        repo.upsert_record(&record).await.unwrap();

        let loaded = repo
            .get_record("mem_1")
            .await
            .unwrap()
            .expect("record should exist");
        assert_eq!(loaded.title, record.title);
        assert_eq!(loaded.normalized_facts, record.normalized_facts);
        assert_eq!(loaded.evidence_refs, record.evidence_refs);
    }

    #[tokio::test]
    async fn memory_repository_filters_by_scope_and_search() {
        let db = Database::in_memory().await.unwrap();
        let repo = MemoryRepository::new(db.pool().clone());
        let mut shared = make_memory_record("mem_shared", MemoryScope::WorkspaceShared);
        shared.title = "Test workflow".to_string();
        let mut sandbox = make_memory_record("mem_sandbox", MemoryScope::WorkspaceSandbox);
        sandbox.title = "Deploy workflow".to_string();

        repo.upsert_record(&shared).await.unwrap();
        repo.upsert_record(&sandbox).await.unwrap();

        let filtered = repo
            .list_records(Some(&MemoryRepositoryFilter {
                scopes: vec![MemoryScope::WorkspaceShared],
                search: Some("Test".to_string()),
                ..MemoryRepositoryFilter::default()
            }))
            .await
            .unwrap();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id.0, "mem_shared");
    }

    #[tokio::test]
    async fn memory_repository_returns_latest_validation_report() {
        let db = Database::in_memory().await.unwrap();
        let repo = MemoryRepository::new(db.pool().clone());
        let record = make_memory_record("mem_validation", MemoryScope::WorkspaceShared);

        repo.upsert_record(&record).await.unwrap();
        repo.record_validation_run(&MemoryValidationReport {
            record_id: Some(record.id.clone()),
            status: MemoryValidationStatus::Warning,
            issues: vec!["missing follow-up verification".to_string()],
            checked_at: 100,
        })
        .await
        .unwrap();
        repo.record_validation_run(&MemoryValidationReport {
            record_id: Some(record.id.clone()),
            status: MemoryValidationStatus::Passed,
            issues: vec![],
            checked_at: 200,
        })
        .await
        .unwrap();

        let latest = repo
            .latest_validation_report(&record.id.0)
            .await
            .unwrap()
            .expect("latest validation report should exist");

        assert_eq!(latest.status, MemoryValidationStatus::Passed);
        assert_eq!(latest.checked_at, 200);
        assert!(latest.issues.is_empty());
    }

    #[tokio::test]
    async fn memory_repository_lists_conflicts_for_selected_record() {
        let db = Database::in_memory().await.unwrap();
        let repo = MemoryRepository::new(db.pool().clone());
        let left = make_memory_record("mem_left", MemoryScope::WorkspaceShared);
        let right = make_memory_record("mem_right", MemoryScope::WorkspaceShared);

        repo.upsert_record(&left).await.unwrap();
        repo.upsert_record(&right).await.unwrap();
        repo.replace_conflicts_for_memory(
            &left.id.0,
            &[MemoryConflictRecord {
                id: "conflict_1".to_string(),
                left_memory_id: left.id.0.clone(),
                right_memory_id: right.id.0.clone(),
                conflict_kind: "contradiction".to_string(),
                detail: "Two records disagree on the preferred workflow".to_string(),
                detected_at: 300,
            }],
        )
        .await
        .unwrap();

        let conflicts = repo.list_conflicts_for_memory(&left.id.0).await.unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].record_id.0, left.id.0);
        assert_eq!(conflicts[0].other_record_id.0, right.id.0);
        assert_eq!(conflicts[0].conflict_kind, "contradiction");
    }

    #[tokio::test]
    async fn memory_repository_roundtrips_rule_packs_and_hits() {
        let db = Database::in_memory().await.unwrap();
        let repo = MemoryRepository::new(db.pool().clone());
        let record = make_memory_record("mem_rule", MemoryScope::WorkspaceShared);
        repo.upsert_record(&record).await.unwrap();

        let pack = MemoryRulePack {
            id: "builtin.consolidation.core".to_string(),
            rule_pack_kind: MemoryRulePackKind::Consolidation,
            version: "2026.04.13".to_string(),
            rules: vec![rocode_types::MemoryRuleDefinition {
                id: "merge.similar.summary".to_string(),
                description: "Merge similar memory summaries into one consolidated record."
                    .to_string(),
                tags: vec!["merge".to_string()],
                promotion_target: None,
            }],
            created_at: 400,
            updated_at: 400,
        };
        repo.upsert_rule_pack(&pack).await.unwrap();
        repo.record_consolidation_run(&MemoryConsolidationRun {
            run_id: "run_1".to_string(),
            started_at: 450,
            finished_at: Some(500),
            merged_count: 1,
            promoted_count: 1,
            conflict_count: 0,
        })
        .await
        .unwrap();
        repo.record_rule_hits(&[MemoryRuleHit {
            id: "hit_1".to_string(),
            rule_pack_id: Some(pack.id.clone()),
            memory_id: Some(record.id.clone()),
            run_id: Some("run_1".to_string()),
            hit_kind: "merge.similar.summary".to_string(),
            detail: Some("Merged into canonical consolidated record.".to_string()),
            created_at: 500,
        }])
        .await
        .unwrap();

        let packs = repo
            .list_rule_packs(Some(MemoryRulePackKind::Consolidation))
            .await
            .unwrap();
        assert_eq!(packs, vec![pack.clone()]);

        let runs = repo.list_consolidation_runs(Some(10)).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run_1");

        let hits = repo
            .list_rule_hits(Some(&MemoryRuleHitQuery {
                run_id: Some("run_1".to_string()),
                memory_id: Some(record.id.clone()),
                limit: Some(10),
            }))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule_pack_id.as_deref(), Some(pack.id.as_str()));
        assert_eq!(hits[0].memory_id.as_ref(), Some(&record.id));
    }

    fn replay_record(event_id: &str, nonce: &str) -> ExternalAdapterReplayRecord {
        ExternalAdapterReplayRecord {
            adapter_id: "generic".to_string(),
            source: "generic-webhook".to_string(),
            external_event_id: event_id.to_string(),
            idempotency_key: format!("external:generic:generic-webhook:{event_id}"),
            external_user_id: "user_1".to_string(),
            external_conversation_id: "chat_1".to_string(),
            session_id: "ses_1".to_string(),
            actor_id: "actor_1".to_string(),
            workspace_id: "ws_1".to_string(),
            nonce: nonce.to_string(),
            signature_hash: "sha256:test".to_string(),
            received_at_ms: 1_714_000_000_000,
            recorded_at_ms: 1_714_000_000_001,
            status: "accepted".to_string(),
        }
    }

    #[tokio::test]
    async fn external_adapter_replay_repository_records_and_rejects_duplicates() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let record = replay_record("evt_1", "nonce_1");

        assert_eq!(
            repo.record(&record).await.unwrap(),
            ExternalAdapterReplayInsertOutcome::Inserted
        );
        assert_eq!(
            repo.record(&record).await.unwrap(),
            ExternalAdapterReplayInsertOutcome::Duplicate
        );

        let stored = repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .expect("replay row should exist");
        assert_eq!(stored, record);
    }

    #[tokio::test]
    async fn external_adapter_replay_repository_scopes_nonce_to_adapter_and_source() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let first = replay_record("evt_1", "nonce_1");
        let second = replay_record("evt_2", "nonce_1");

        assert_eq!(
            repo.record(&first).await.unwrap(),
            ExternalAdapterReplayInsertOutcome::Inserted
        );
        assert_eq!(
            repo.record(&second).await.unwrap(),
            ExternalAdapterReplayInsertOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn external_adapter_replay_repository_prunes_old_rows() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let old = replay_record("evt_old", "nonce_old");
        let mut fresh = replay_record("evt_new", "nonce_new");
        fresh.recorded_at_ms = old.recorded_at_ms + 10_000;

        assert_eq!(
            repo.record(&old).await.unwrap(),
            ExternalAdapterReplayInsertOutcome::Inserted
        );
        assert_eq!(
            repo.record(&fresh).await.unwrap(),
            ExternalAdapterReplayInsertOutcome::Inserted
        );

        assert_eq!(
            repo.prune_recorded_before(fresh.recorded_at_ms)
                .await
                .unwrap(),
            1
        );
        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_old")
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_new")
            .await
            .unwrap()
            .is_some());
    }
}
