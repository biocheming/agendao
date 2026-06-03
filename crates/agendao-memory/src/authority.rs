use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;
use agendao_command::stage_protocol::StageSummary;
use agendao_config::WorkspaceMode;
use agendao_runtime_context::ResolvedWorkspaceContextAuthority;
use agendao_state::UserStateAuthority;
use agendao_storage::{MemoryRepository, MemoryRepositoryFilter, MemoryRetrievalLogEntry};
use agendao_types::{
    MemoryCardView, MemoryConflictResponse, MemoryConsolidationRequest,
    MemoryConsolidationResponse, MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery,
    MemoryContract, MemoryDetailView, MemoryEvidenceRef, MemoryKind, MemoryListQuery,
    MemoryListResponse, MemoryRecallView, MemoryRecord, MemoryRecordId, MemoryRetrievalPacket,
    MemoryRetrievalPreviewResponse, MemoryRetrievalQuery, MemoryRuleHitListResponse,
    MemoryRuleHitQuery, MemoryRulePackListResponse, MemoryScope, MemoryStatus,
    MemoryValidationReportResponse, MemoryValidationStatus, MessageRole, Session,
    SessionMemoryInsight, SessionMemoryTelemetrySummary, SkillGuardReport, SkillGuardStatus,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::consolidation::MemoryConsolidationEngine;
use crate::validation::MemoryValidationEngine;

#[derive(Debug, Clone, Default)]
pub struct MemoryFilter<'a> {
    pub kinds: Option<&'a [MemoryKind]>,
    pub scopes: Option<&'a [MemoryScope]>,
    pub statuses: Option<&'a [MemoryStatus]>,
    pub search: Option<&'a str>,
    pub source_session_id: Option<&'a str>,
    pub derived_skill_name: Option<&'a str>,
    pub linked_skill_name: Option<&'a str>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedMemoryContext {
    pub workspace_key: String,
    pub workspace_mode: WorkspaceMode,
    pub allowed_scopes: Vec<MemoryScope>,
}

#[derive(Debug, Clone)]
pub struct ToolMemoryObservation<'a> {
    pub session_id: &'a str,
    pub tool_call_id: &'a str,
    pub tool_name: &'a str,
    pub stage_id: Option<&'a str>,
    pub output: &'a str,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct SkillWriteObservation<'a> {
    pub session_id: &'a str,
    pub tool_call_id: Option<&'a str>,
    pub skill_name: &'a str,
    pub action: &'a str,
    pub location: Option<&'a str>,
    pub supporting_file: Option<&'a str>,
    pub guard_report: Option<&'a SkillGuardReport>,
}

#[derive(Debug, Clone)]
pub struct SkillUsageObservation<'a> {
    pub session_id: &'a str,
    pub tool_call_id: &'a str,
    pub tool_name: &'a str,
    pub stage_id: Option<&'a str>,
    pub skill_name: &'a str,
    pub category: Option<&'a str>,
    pub output: &'a str,
    pub is_error: bool,
}

pub const MEMORY_FROZEN_SNAPSHOT_METADATA_KEY: &str = "memory_frozen_snapshot";
pub const MEMORY_LAST_PREFETCH_METADATA_KEY: &str = "memory_last_prefetch_packet";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedMemorySnapshot {
    pub packet: MemoryRetrievalPacket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_block: Option<String>,
}

#[derive(Clone)]
pub struct MemoryAuthority {
    _user_state: Arc<UserStateAuthority>,
    resolved_context_authority: Arc<ResolvedWorkspaceContextAuthority>,
    repository: Option<Arc<MemoryRepository>>,
    validation_engine: Option<Arc<MemoryValidationEngine>>,
    consolidation_engine: Option<Arc<MemoryConsolidationEngine>>,
}

impl MemoryAuthority {
    pub fn new(
        user_state: Arc<UserStateAuthority>,
        resolved_context_authority: Arc<ResolvedWorkspaceContextAuthority>,
    ) -> Self {
        Self {
            _user_state: user_state,
            resolved_context_authority,
            repository: None,
            validation_engine: None,
            consolidation_engine: None,
        }
    }

    pub fn with_repository(mut self, repository: Arc<MemoryRepository>) -> Self {
        self.validation_engine = Some(Arc::new(MemoryValidationEngine::new(repository.clone())));
        self.consolidation_engine =
            Some(Arc::new(MemoryConsolidationEngine::new(repository.clone())));
        self.repository = Some(repository);
        self
    }

    pub async fn resolve_context(&self) -> Result<ResolvedMemoryContext> {
        let resolved = self.resolved_context_authority.resolve().await?;
        Ok(ResolvedMemoryContext {
            workspace_key: resolved.identity.workspace_key,
            workspace_mode: resolved.mode,
            allowed_scopes: allowed_scopes_for_mode(resolved.mode),
        })
    }

    pub async fn list_memory(
        &self,
        filter: Option<&MemoryFilter<'_>>,
    ) -> Result<Vec<MemoryCardView>> {
        Ok(self
            .list_memory_records(filter)
            .await?
            .iter()
            .map(MemoryCardView::from)
            .collect())
    }

    async fn list_memory_records(
        &self,
        filter: Option<&MemoryFilter<'_>>,
    ) -> Result<Vec<MemoryRecord>> {
        let Some(repository) = &self.repository else {
            let _ = self.resolve_context().await?;
            return Ok(Vec::new());
        };

        let context = self.resolve_context().await?;
        let storage_filter = repository_filter(filter, &context);
        Ok(repository.list_records(Some(&storage_filter)).await?)
    }

    pub async fn list_memory_response(
        &self,
        filter: Option<&MemoryFilter<'_>>,
    ) -> Result<MemoryListResponse> {
        Ok(MemoryListResponse {
            items: self.list_memory(filter).await?,
            contract: MemoryContract {
                filter_query_parameters: vec![
                    "scope".to_string(),
                    "kind".to_string(),
                    "status".to_string(),
                    "search".to_string(),
                    "source_session_id".to_string(),
                    "derived_skill_name".to_string(),
                    "linked_skill_name".to_string(),
                    "limit".to_string(),
                ],
                search_fields: vec![
                    "title".to_string(),
                    "summary".to_string(),
                    "normalized_facts".to_string(),
                ],
                non_search_fields: vec![
                    "trigger_conditions".to_string(),
                    "boundaries".to_string(),
                    "evidence_refs".to_string(),
                    "workspace_identity".to_string(),
                    "derived_skill_name".to_string(),
                    "linked_skill_name".to_string(),
                ],
                note: "Memory list search is limited to lightweight recalled fields. Provenance and validation detail remain detail-only surfaces.".to_string(),
            },
        })
    }

    pub async fn list_memory_for_query(
        &self,
        query: &MemoryListQuery,
    ) -> Result<MemoryListResponse> {
        let filter = MemoryFilter {
            kinds: (!query.kinds.is_empty()).then_some(query.kinds.as_slice()),
            scopes: (!query.scopes.is_empty()).then_some(query.scopes.as_slice()),
            statuses: (!query.statuses.is_empty()).then_some(query.statuses.as_slice()),
            search: query.search.as_deref(),
            source_session_id: query.source_session_id.as_deref(),
            derived_skill_name: query.derived_skill_name.as_deref(),
            linked_skill_name: query.linked_skill_name.as_deref(),
            limit: query.limit,
        };
        self.list_memory_response(Some(&filter)).await
    }

    pub async fn search_memory_for_query(
        &self,
        query: &MemoryListQuery,
    ) -> Result<MemoryListResponse> {
        self.list_memory_for_query(query).await
    }

    pub async fn get_memory_detail(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<MemoryDetailView>> {
        let Some(repository) = &self.repository else {
            return Ok(None);
        };
        let context = self.resolve_context().await?;
        let Some(record) = repository.get_record(&record_id.0).await? else {
            return Ok(None);
        };
        if !record_visible_in_context(&record, &context) {
            return Ok(None);
        }
        Ok(Some(MemoryDetailView { record }))
    }

    pub async fn get_memory_validation_report(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<MemoryValidationReportResponse>> {
        let Some(repository) = &self.repository else {
            return Ok(None);
        };
        let context = self.resolve_context().await?;
        let Some(record) = repository.get_record(&record_id.0).await? else {
            return Ok(None);
        };
        if !record_visible_in_context(&record, &context) {
            return Ok(None);
        }
        Ok(Some(MemoryValidationReportResponse {
            record_id: record_id.clone(),
            latest: repository.latest_validation_report(&record_id.0).await?,
        }))
    }

    pub async fn get_memory_conflicts(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<MemoryConflictResponse>> {
        let Some(repository) = &self.repository else {
            return Ok(None);
        };
        let context = self.resolve_context().await?;
        let Some(record) = repository.get_record(&record_id.0).await? else {
            return Ok(None);
        };
        if !record_visible_in_context(&record, &context) {
            return Ok(None);
        }
        Ok(Some(MemoryConflictResponse {
            record_id: record_id.clone(),
            conflicts: repository.list_conflicts_for_memory(&record_id.0).await?,
        }))
    }

    pub async fn list_memory_rule_packs(&self) -> Result<MemoryRulePackListResponse> {
        let Some(engine) = &self.consolidation_engine else {
            return Ok(MemoryRulePackListResponse { items: Vec::new() });
        };
        Ok(MemoryRulePackListResponse {
            items: engine.list_rule_packs().await?,
        })
    }

    pub async fn list_memory_rule_hits(
        &self,
        query: &MemoryRuleHitQuery,
    ) -> Result<MemoryRuleHitListResponse> {
        let Some(engine) = &self.consolidation_engine else {
            return Ok(MemoryRuleHitListResponse { items: Vec::new() });
        };
        let context = self.resolve_context().await?;
        let mut items = Vec::new();
        for hit in engine.list_rule_hits(query).await? {
            let visible = match hit.memory_id.as_ref() {
                Some(memory_id) => self.record_is_visible(memory_id, &context).await,
                None => true,
            };
            if visible {
                items.push(hit);
            }
        }
        Ok(MemoryRuleHitListResponse { items })
    }

    pub async fn list_consolidation_runs(
        &self,
        query: &MemoryConsolidationRunQuery,
    ) -> Result<MemoryConsolidationRunListResponse> {
        let Some(engine) = &self.consolidation_engine else {
            return Ok(MemoryConsolidationRunListResponse { items: Vec::new() });
        };
        Ok(MemoryConsolidationRunListResponse {
            items: engine.list_consolidation_runs(query).await?,
        })
    }

    pub async fn run_consolidation(
        &self,
        request: &MemoryConsolidationRequest,
    ) -> Result<MemoryConsolidationResponse> {
        let Some(engine) = &self.consolidation_engine else {
            let now = chrono::Utc::now().timestamp_millis();
            return Ok(MemoryConsolidationResponse {
                run: agendao_types::MemoryConsolidationRun {
                    run_id: "mem_consolidation_noop".to_string(),
                    started_at: now,
                    finished_at: Some(now),
                    merged_count: 0,
                    promoted_count: 0,
                    conflict_count: 0,
                },
                merged_record_ids: Vec::new(),
                promoted_record_ids: Vec::new(),
                archived_record_ids: Vec::new(),
                reflection_notes: vec!["Memory repository is not configured.".to_string()],
                rule_hits: Vec::new(),
            });
        };
        let context = self.resolve_context().await?;
        engine.run_consolidation(&context, request).await
    }

    pub async fn build_session_memory_telemetry(
        &self,
        session: &Session,
    ) -> Result<Option<SessionMemoryTelemetrySummary>> {
        let view = self.build_session_memory_view(session).await?;
        Ok(view.map(|(summary, _, _, _)| summary))
    }

    pub async fn build_session_memory_insight(
        &self,
        session: &Session,
    ) -> Result<Option<SessionMemoryInsight>> {
        let Some((summary, frozen_snapshot, last_prefetch_packet, recent_session_records)) =
            self.build_session_memory_view(session).await?
        else {
            return Ok(None);
        };

        Ok(Some(SessionMemoryInsight {
            summary,
            frozen_snapshot,
            last_prefetch_packet,
            recent_session_records,
        }))
    }

    pub async fn build_frozen_snapshot(&self) -> Result<MemoryRetrievalPacket> {
        let context = self.resolve_context().await?;
        let retrieval_statuses = [MemoryStatus::Validated, MemoryStatus::Consolidated];
        let filter = MemoryFilter {
            scopes: Some(&context.allowed_scopes),
            statuses: Some(&retrieval_statuses),
            limit: Some(8),
            ..MemoryFilter::default()
        };
        let records = self.list_memory_records(Some(&filter)).await?;
        let items = records
            .iter()
            .map(|record| recall_view_from_record(record, "always_on_snapshot"))
            .collect();

        Ok(MemoryRetrievalPacket {
            generated_at: chrono::Utc::now().timestamp_millis(),
            snapshot: true,
            query: None,
            scopes: context.allowed_scopes,
            items,
            note: Some(
                "Frozen snapshot for this session. It is stable across turns and limited to validated/consolidated records.".to_string(),
            ),
            budget_limit: Some(8),
        })
    }

    pub async fn build_prefetch_packet(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> Result<MemoryRetrievalPacket> {
        let context = self.resolve_context().await?;
        let requested_scopes = if query.scopes.is_empty() {
            context.allowed_scopes.clone()
        } else {
            intersect_scopes(&context.allowed_scopes, &query.scopes)
        };
        let retrieval_statuses = [MemoryStatus::Validated, MemoryStatus::Consolidated];
        let filter = MemoryFilter {
            scopes: Some(&requested_scopes),
            kinds: (!query.kinds.is_empty()).then_some(query.kinds.as_slice()),
            statuses: Some(&retrieval_statuses),
            search: query.query.as_deref(),
            source_session_id: query.session_id.as_deref(),
            limit: Some(query.limit.unwrap_or(6)),
            ..MemoryFilter::default()
        };
        let records = self.list_memory_records(Some(&filter)).await?;
        let items = records
            .iter()
            .map(|record| recall_view_from_record(record, &prefetch_reason(record, query)))
            .collect::<Vec<_>>();

        if let Some(repository) = &self.repository {
            repository
                .record_retrieval(&MemoryRetrievalLogEntry {
                    session_id: query.session_id.clone(),
                    query: query.query.clone(),
                    stage: query.stage.clone(),
                    scopes: requested_scopes.clone(),
                    retrieved_count: items.len() as u32,
                    used_count: 0,
                    created_at: chrono::Utc::now().timestamp_millis(),
                })
                .await?;
        }

        Ok(MemoryRetrievalPacket {
            generated_at: chrono::Utc::now().timestamp_millis(),
            snapshot: false,
            query: query.query.clone(),
            scopes: requested_scopes,
            items,
            note: Some(
                "Turn-scoped retrieval preview sourced from validated/consolidated memory records."
                    .to_string(),
            ),
            budget_limit: Some(query.limit.unwrap_or(6)),
        })
    }

    pub async fn build_retrieval_preview(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> Result<MemoryRetrievalPreviewResponse> {
        Ok(MemoryRetrievalPreviewResponse {
            packet: self.build_prefetch_packet(query).await?,
            contract: MemoryContract {
                filter_query_parameters: vec![
                    "query".to_string(),
                    "stage".to_string(),
                    "limit".to_string(),
                    "kinds".to_string(),
                    "scopes".to_string(),
                    "session_id".to_string(),
                ],
                search_fields: vec![
                    "title".to_string(),
                    "summary".to_string(),
                    "normalized_facts".to_string(),
                    "trigger_conditions".to_string(),
                ],
                non_search_fields: vec![
                    "evidence_refs".to_string(),
                    "workspace_identity".to_string(),
                    "boundaries".to_string(),
                ],
                note: "Retrieval preview is the formal explanation surface for why a memory record would be injected into the current turn.".to_string(),
            },
        })
    }

    pub async fn record_prefetch_usage(
        &self,
        session_id: &str,
        packet: &MemoryRetrievalPacket,
    ) -> Result<()> {
        let Some(repository) = &self.repository else {
            return Ok(());
        };

        repository
            .record_retrieval(&MemoryRetrievalLogEntry {
                session_id: Some(session_id.to_string()),
                query: packet.query.clone(),
                stage: None,
                scopes: packet.scopes.clone(),
                retrieved_count: 0,
                used_count: packet.items.len() as u32,
                created_at: chrono::Utc::now().timestamp_millis(),
            })
            .await?;
        Ok(())
    }

    pub async fn ingest_tool_result_observation(
        &self,
        observation: &ToolMemoryObservation<'_>,
    ) -> Result<Option<MemoryRecord>> {
        if self.repository.is_none() {
            return Ok(None);
        }

        let context = self.resolve_context().await?;
        let now = chrono::Utc::now().timestamp_millis();
        let summary = summarize_tool_output(observation.output);
        let tool_name = observation.tool_name.trim();
        let record = MemoryRecord {
            id: hashed_record_id(
                "mem_tool",
                &[
                    observation.session_id,
                    observation.tool_call_id,
                    tool_name,
                    if observation.is_error { "error" } else { "ok" },
                ],
            ),
            kind: if tool_name.eq_ignore_ascii_case("skill_manage") {
                MemoryKind::MethodologyCandidate
            } else if observation.is_error {
                MemoryKind::Lesson
            } else {
                MemoryKind::Pattern
            },
            scope: candidate_scope_for_mode(context.workspace_mode),
            status: MemoryStatus::Candidate,
            title: if observation.is_error {
                format!("Tool failure candidate: {}", tool_name)
            } else {
                format!("Tool pattern candidate: {}", tool_name)
            },
            summary,
            trigger_conditions: vec![format!("tool:{}", tool_name)],
            normalized_facts: {
                let mut facts = vec![
                    format!("tool_name={}", tool_name),
                    format!(
                        "tool_outcome={}",
                        if observation.is_error {
                            "error"
                        } else {
                            "success"
                        }
                    ),
                ];
                if let Some(stage_id) = observation.stage_id {
                    facts.push(format!("stage_id={}", stage_id));
                }
                facts
            },
            boundaries: vec![
                "Derived from a single tool result.".to_string(),
                "Validate before promoting into durable memory or skill state.".to_string(),
            ],
            confidence: Some(if observation.is_error { 0.45 } else { 0.35 }),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some(observation.session_id.to_string()),
                message_id: None,
                tool_call_id: Some(observation.tool_call_id.to_string()),
                stage_id: observation.stage_id.map(ToOwned::to_owned),
                note: Some("runtime.tool_end".to_string()),
            }],
            source_session_id: Some(observation.session_id.to_string()),
            workspace_identity: Some(context.workspace_key.clone()),
            created_at: now,
            updated_at: now,
            last_validated_at: None,
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Pending,
        };

        Ok(Some(self.persist_candidate_record(record, &context).await?))
    }

    pub async fn ingest_session_record(&self, session: &Session) -> Result<Option<MemoryRecord>> {
        if self.repository.is_none() {
            return Ok(None);
        }

        let Some((note_message, user_prompt)) = latest_skill_save_suggestion(session) else {
            return Ok(None);
        };

        let context = self.resolve_context().await?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut trigger_conditions = vec!["runtime_hint:skill_save_suggestion".to_string()];
        if let Some(prompt) = user_prompt.as_deref() {
            trigger_conditions.push(format!("task_prompt={}", truncate(prompt, 160)));
        }

        let mut normalized_facts = vec![
            format!("session_id={}", session.id),
            format!("session_title={}", truncate(&session.title, 120)),
            "memory_source=session_runtime".to_string(),
        ];
        if let Some(summary) = &session.summary {
            normalized_facts.push(format!(
                "session_diff=files:{} additions:{} deletions:{}",
                summary.files, summary.additions, summary.deletions
            ));
        }

        let record = MemoryRecord {
            id: hashed_record_id("mem_session", &[&session.id, "skill_save_suggestion"]),
            kind: MemoryKind::MethodologyCandidate,
            scope: candidate_scope_for_mode(context.workspace_mode),
            status: MemoryStatus::Candidate,
            title: format!(
                "Methodology candidate from session {}",
                truncate(&session.title, 80)
            ),
            summary: truncate(
                &format!(
                    "{}{}",
                    note_message.get_text(),
                    user_prompt
                        .as_deref()
                        .map(|prompt| format!(" User task signal: {}", truncate(prompt, 180)))
                        .unwrap_or_default()
                ),
                320,
            ),
            trigger_conditions,
            normalized_facts,
            boundaries: vec![
                "Derived from runtime skill-save heuristic.".to_string(),
                "Do not promote until triggers, steps, validation, and boundaries are explicit."
                    .to_string(),
            ],
            confidence: Some(0.6),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some(session.id.clone()),
                message_id: Some(note_message.id.clone()),
                tool_call_id: None,
                stage_id: None,
                note: Some("runtime_hint=skill_save_suggestion".to_string()),
            }],
            source_session_id: Some(session.id.clone()),
            workspace_identity: Some(context.workspace_key.clone()),
            created_at: now,
            updated_at: now,
            last_validated_at: None,
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Pending,
        };

        Ok(Some(self.persist_candidate_record(record, &context).await?))
    }

    pub async fn ingest_stage_summary_observation(
        &self,
        session_id: &str,
        stage: &StageSummary,
    ) -> Result<Option<MemoryRecord>> {
        if self.repository.is_none() {
            return Ok(None);
        }

        let context = self.resolve_context().await?;
        let focus = stage
            .focus
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let waiting_on = stage
            .waiting_on
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let last_event = stage
            .last_event
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if focus.is_none() && waiting_on.is_none() && last_event.is_none() {
            return Ok(None);
        }

        let now = chrono::Utc::now().timestamp_millis();
        let status_label = stage_status_label(&stage.status);
        let mut facts = vec![
            format!("stage_id={}", stage.stage_id),
            format!("stage_name={}", stage.stage_name),
            format!("stage_status={}", status_label),
        ];
        if let Some(waiting_on) = waiting_on {
            facts.push(format!("waiting_on={}", waiting_on));
        }
        if let Some(last_event) = last_event {
            facts.push(format!("last_event={}", last_event));
        }

        let summary = [
            focus.map(|value| format!("focus: {}", value)),
            waiting_on.map(|value| format!("waiting_on: {}", value)),
            last_event.map(|value| format!("last_event: {}", value)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

        let record = MemoryRecord {
            id: hashed_record_id("mem_stage", &[session_id, &stage.stage_id, status_label]),
            kind: if matches!(
                stage.status,
                agendao_command::stage_protocol::StageStatus::Blocked
                    | agendao_command::stage_protocol::StageStatus::Retrying
            ) {
                MemoryKind::Lesson
            } else {
                MemoryKind::Pattern
            },
            scope: candidate_scope_for_mode(context.workspace_mode),
            status: MemoryStatus::Candidate,
            title: format!("Stage observation: {} / {}", stage.stage_name, status_label),
            summary: truncate(&summary, 240),
            trigger_conditions: vec![
                format!("stage:{}", stage.stage_name),
                format!("stage_status:{}", status_label),
            ],
            normalized_facts: facts,
            boundaries: vec![
                "Derived from scheduler stage summary telemetry.".to_string(),
                "Validate against repeated runs before promotion.".to_string(),
            ],
            confidence: Some(0.25),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some(session_id.to_string()),
                message_id: None,
                tool_call_id: None,
                stage_id: Some(stage.stage_id.clone()),
                note: Some("runtime.stage_summary".to_string()),
            }],
            source_session_id: Some(session_id.to_string()),
            workspace_identity: Some(context.workspace_key.clone()),
            created_at: now,
            updated_at: now,
            last_validated_at: None,
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Pending,
        };

        Ok(Some(self.persist_candidate_record(record, &context).await?))
    }

    pub async fn ingest_skill_write_observation(
        &self,
        observation: &SkillWriteObservation<'_>,
    ) -> Result<Vec<MemoryRecord>> {
        if self.repository.is_none() {
            return Ok(Vec::new());
        }

        let context = self.resolve_context().await?;
        let mut records = Vec::new();
        let mut seen_ids = BTreeSet::new();

        for record in self
            .link_skill_candidates_from_observation(observation, &context)
            .await?
        {
            if seen_ids.insert(record.id.0.clone()) {
                records.push(record);
            }
        }

        if observation.action.eq_ignore_ascii_case("create") {
            let promotion = self
                .persist_candidate_record(
                    build_skill_promotion_record(observation, &context),
                    &context,
                )
                .await?;
            if seen_ids.insert(promotion.id.0.clone()) {
                records.push(promotion);
            }
        }

        if should_emit_skill_feedback_lesson(observation) {
            let lesson = self
                .persist_candidate_record(
                    build_skill_feedback_lesson_record(observation, &context),
                    &context,
                )
                .await?;
            if seen_ids.insert(lesson.id.0.clone()) {
                records.push(lesson);
            }
        }

        Ok(records)
    }

    pub async fn ingest_skill_usage_observation(
        &self,
        observation: &SkillUsageObservation<'_>,
    ) -> Result<Option<MemoryRecord>> {
        if self.repository.is_none() {
            return Ok(None);
        }

        let context = self.resolve_context().await?;
        let record = build_skill_usage_record(observation, &context);
        Ok(Some(self.persist_candidate_record(record, &context).await?))
    }

    pub async fn validate_record(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<agendao_types::MemoryValidationReport>> {
        let Some(validation_engine) = &self.validation_engine else {
            return Ok(None);
        };
        let context = self.resolve_context().await?;
        Ok(validation_engine
            .validate_record_by_id(record_id, &context)
            .await?
            .map(|outcome| outcome.report))
    }

    async fn persist_candidate_record(
        &self,
        record: MemoryRecord,
        context: &ResolvedMemoryContext,
    ) -> Result<MemoryRecord> {
        let Some(repository) = &self.repository else {
            return Ok(record);
        };
        repository.upsert_record(&record).await?;
        if let Some(validation_engine) = &self.validation_engine {
            return Ok(validation_engine
                .validate_and_apply(&record, context)
                .await?
                .record);
        }
        Ok(record)
    }

    async fn persist_record(
        &self,
        record: MemoryRecord,
        context: &ResolvedMemoryContext,
    ) -> Result<MemoryRecord> {
        let Some(repository) = &self.repository else {
            return Ok(record);
        };
        repository.upsert_record(&record).await?;
        if let Some(validation_engine) = &self.validation_engine {
            return Ok(validation_engine
                .validate_and_apply(&record, context)
                .await?
                .record);
        }
        Ok(record)
    }

    async fn link_skill_candidates_from_observation(
        &self,
        observation: &SkillWriteObservation<'_>,
        context: &ResolvedMemoryContext,
    ) -> Result<Vec<MemoryRecord>> {
        let Some(repository) = &self.repository else {
            return Ok(Vec::new());
        };

        let mut linked = Vec::new();
        let mut seen_ids = BTreeSet::new();
        let normalized_skill_name = normalize_skill_name(observation.skill_name);
        let action = normalize_skill_action(observation.action);
        let now = chrono::Utc::now().timestamp_millis();

        if let Some(tool_call_id) = observation.tool_call_id {
            let exact_record_id = hashed_record_id(
                "mem_tool",
                &[observation.session_id, tool_call_id, "skill_manage", "ok"],
            );
            if let Some(record) = repository.get_record(&exact_record_id.0).await? {
                let updated = self
                    .persist_record(
                        apply_skill_linkage_to_record(
                            record,
                            observation,
                            &action,
                            &normalized_skill_name,
                            now,
                        ),
                        context,
                    )
                    .await?;
                seen_ids.insert(updated.id.0.clone());
                linked.push(updated);
            }
        }

        let methodology_records = repository
            .list_records(Some(&MemoryRepositoryFilter {
                scopes: context.allowed_scopes.clone(),
                kinds: vec![MemoryKind::MethodologyCandidate],
                statuses: vec![
                    MemoryStatus::Candidate,
                    MemoryStatus::Validated,
                    MemoryStatus::Consolidated,
                ],
                workspace_identity: Some(context.workspace_key.clone()),
                limit: Some(500),
                ..MemoryRepositoryFilter::default()
            }))
            .await?;

        let mut strong_matches = methodology_records
            .iter()
            .filter(|record| {
                skill_record_matches_observation(
                    record,
                    observation,
                    &normalized_skill_name,
                    MatchStrength::Strong,
                )
            })
            .cloned()
            .collect::<Vec<_>>();

        if strong_matches.is_empty() {
            let session_candidates = methodology_records
                .iter()
                .filter(|record| {
                    record.source_session_id.as_deref() == Some(observation.session_id)
                        && record.linked_skill_name.is_none()
                })
                .cloned()
                .collect::<Vec<_>>();
            if session_candidates.len() == 1 {
                strong_matches = session_candidates;
            }
        }

        for record in strong_matches {
            if seen_ids.contains(&record.id.0) {
                continue;
            }
            let updated = self
                .persist_record(
                    apply_skill_linkage_to_record(
                        record,
                        observation,
                        &action,
                        &normalized_skill_name,
                        now,
                    ),
                    context,
                )
                .await?;
            seen_ids.insert(updated.id.0.clone());
            linked.push(updated);
        }

        Ok(linked)
    }

    async fn record_is_visible(
        &self,
        record_id: &MemoryRecordId,
        context: &ResolvedMemoryContext,
    ) -> bool {
        let Some(repository) = &self.repository else {
            return false;
        };
        match repository.get_record(&record_id.0).await {
            Ok(Some(record)) => record_visible_in_context(&record, context),
            _ => false,
        }
    }
}

pub fn render_frozen_snapshot_block(packet: &MemoryRetrievalPacket) -> Option<String> {
    render_memory_packet_block("Frozen Memory Snapshot", packet)
}

pub fn render_prefetch_packet_block(packet: &MemoryRetrievalPacket) -> Option<String> {
    render_memory_packet_block("Turn Memory Recall", packet)
}

fn render_memory_packet_block(title: &str, packet: &MemoryRetrievalPacket) -> Option<String> {
    if packet.items.is_empty() {
        return None;
    }

    let mut lines = vec![format!("{}:", title)];
    if let Some(query) = packet
        .query
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("- query: {}", query.trim()));
    }
    if !packet.scopes.is_empty() {
        lines.push(format!(
            "- scopes: {}",
            packet
                .scopes
                .iter()
                .map(scope_label)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(limit) = packet.budget_limit {
        lines.push(format!("- budget_limit: {}", limit));
    }

    for item in &packet.items {
        lines.push(format!(
            "- {} [{:?} / {:?} / {:?}]",
            item.card.title, item.card.kind, item.card.status, item.card.validation_status
        ));
        lines.push(format!("  why: {}", item.why_recalled));
        lines.push(format!("  summary: {}", item.card.summary));
        if let Some(last_validated_at) = item.card.last_validated_at {
            lines.push(format!("  last_validated_at: {}", last_validated_at));
        }
        if let Some(evidence) = item.evidence_summary.as_deref() {
            lines.push(format!("  evidence: {}", evidence));
        }
    }

    Some(lines.join("\n"))
}

pub fn allowed_scopes_for_mode(mode: WorkspaceMode) -> Vec<MemoryScope> {
    match mode {
        WorkspaceMode::Shared => vec![
            MemoryScope::GlobalUser,
            MemoryScope::GlobalWorkspace,
            MemoryScope::WorkspaceShared,
        ],
        WorkspaceMode::Isolated => {
            vec![MemoryScope::WorkspaceSandbox, MemoryScope::SessionEphemeral]
        }
    }
}

fn repository_filter(
    filter: Option<&MemoryFilter<'_>>,
    context: &ResolvedMemoryContext,
) -> MemoryRepositoryFilter {
    let filter = filter.cloned().unwrap_or_default();
    let scopes = match filter.scopes {
        Some(scopes) => intersect_scopes(&context.allowed_scopes, scopes),
        None => context.allowed_scopes.clone(),
    };

    MemoryRepositoryFilter {
        scopes,
        kinds: filter.kinds.unwrap_or(&[]).to_vec(),
        statuses: filter.statuses.unwrap_or(&[]).to_vec(),
        search: filter.search.map(ToOwned::to_owned),
        workspace_identity: Some(context.workspace_key.clone()),
        source_session_id: filter.source_session_id.map(ToOwned::to_owned),
        derived_skill_name: filter.derived_skill_name.map(ToOwned::to_owned),
        linked_skill_name: filter.linked_skill_name.map(ToOwned::to_owned),
        limit: Some(filter.limit.unwrap_or(100) as i64),
    }
}

fn candidate_scope_for_mode(mode: WorkspaceMode) -> MemoryScope {
    match mode {
        WorkspaceMode::Shared => MemoryScope::WorkspaceShared,
        WorkspaceMode::Isolated => MemoryScope::WorkspaceSandbox,
    }
}

fn intersect_scopes(allowed: &[MemoryScope], requested: &[MemoryScope]) -> Vec<MemoryScope> {
    requested
        .iter()
        .filter(|scope| allowed.contains(scope))
        .cloned()
        .collect()
}

fn record_visible_in_context(record: &MemoryRecord, context: &ResolvedMemoryContext) -> bool {
    context.allowed_scopes.contains(&record.scope)
        && record
            .workspace_identity
            .as_deref()
            .map(|workspace| workspace == context.workspace_key)
            .unwrap_or(true)
}

fn recall_view_from_record(record: &MemoryRecord, why_recalled: &str) -> MemoryRecallView {
    MemoryRecallView {
        card: MemoryCardView::from(record),
        why_recalled: why_recalled.to_string(),
        evidence_summary: record.evidence_refs.first().map(|evidence| {
            let mut parts = Vec::new();
            if let Some(session_id) = evidence.session_id.as_deref() {
                parts.push(format!("session={}", session_id));
            }
            if let Some(tool_call_id) = evidence.tool_call_id.as_deref() {
                parts.push(format!("tool={}", tool_call_id));
            }
            if let Some(stage_id) = evidence.stage_id.as_deref() {
                parts.push(format!("stage={}", stage_id));
            }
            if let Some(note) = evidence.note.as_deref() {
                parts.push(note.to_string());
            }
            parts.join(" · ")
        }),
    }
}

fn prefetch_reason(record: &MemoryRecord, query: &MemoryRetrievalQuery) -> String {
    let query_text = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let stage_text = query
        .stage
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let normalized_haystack = [
        record.title.to_ascii_lowercase(),
        record.summary.to_ascii_lowercase(),
        record.normalized_facts.join(" ").to_ascii_lowercase(),
        record.trigger_conditions.join(" ").to_ascii_lowercase(),
    ]
    .join("\n");

    let mut reasons = Vec::new();
    if let Some(query_text) = query_text {
        let query_lower = query_text.to_ascii_lowercase();
        if normalized_haystack.contains(&query_lower) {
            reasons.push(format!("matches_query:{}", truncate(query_text, 80)));
        } else {
            reasons.push(format!("query_scope:{}", truncate(query_text, 80)));
        }
    }
    if let Some(stage_text) = stage_text {
        reasons.push(format!("stage:{}", stage_text));
    }
    if reasons.is_empty() {
        reasons.push("workspace_recall".to_string());
    }
    reasons.join(" · ")
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

fn stage_status_label(status: &agendao_command::stage_protocol::StageStatus) -> &'static str {
    match status {
        agendao_command::stage_protocol::StageStatus::Running => "running",
        agendao_command::stage_protocol::StageStatus::Waiting => "waiting",
        agendao_command::stage_protocol::StageStatus::Done => "done",
        agendao_command::stage_protocol::StageStatus::Cancelled => "cancelled",
        agendao_command::stage_protocol::StageStatus::Cancelling => "cancelling",
        agendao_command::stage_protocol::StageStatus::Blocked => "blocked",
        agendao_command::stage_protocol::StageStatus::Retrying => "retrying",
    }
}

impl MemoryAuthority {
    async fn build_session_memory_view(
        &self,
        session: &Session,
    ) -> Result<
        Option<(
            SessionMemoryTelemetrySummary,
            Option<MemoryRetrievalPacket>,
            Option<MemoryRetrievalPacket>,
            Vec<MemoryCardView>,
        )>,
    > {
        let context = self.resolve_context().await?;
        let frozen_snapshot =
            load_persisted_memory_snapshot(session).map(|snapshot| snapshot.packet);
        let last_prefetch_packet = load_last_prefetch_packet(session);
        let latest_consolidation_run = self
            .list_consolidation_runs(&MemoryConsolidationRunQuery { limit: Some(1) })
            .await?
            .items
            .into_iter()
            .next();
        let recent_rule_hits = self
            .list_memory_rule_hits(&MemoryRuleHitQuery {
                limit: Some(5),
                ..Default::default()
            })
            .await?
            .items;
        let (session_records, retrieval_logs) =
            self.session_runtime_memory_state(session, &context).await?;

        if frozen_snapshot.is_none()
            && last_prefetch_packet.is_none()
            && latest_consolidation_run.is_none()
            && recent_rule_hits.is_empty()
            && session_records.is_empty()
            && retrieval_logs.is_empty()
        {
            return Ok(None);
        }

        let candidate_count = session_records
            .iter()
            .filter(|record| record.status == MemoryStatus::Candidate)
            .count() as u32;
        let validated_count = session_records
            .iter()
            .filter(|record| {
                matches!(
                    record.status,
                    MemoryStatus::Validated | MemoryStatus::Consolidated
                )
            })
            .count() as u32;
        let rejected_count = session_records
            .iter()
            .filter(|record| {
                matches!(
                    record.status,
                    MemoryStatus::Rejected | MemoryStatus::Archived
                )
            })
            .count() as u32;
        let warning_count = session_records
            .iter()
            .filter(|record| record.validation_status == MemoryValidationStatus::Warning)
            .count() as u32;
        let methodology_candidate_count = session_records
            .iter()
            .filter(|record| record.kind == MemoryKind::MethodologyCandidate)
            .count() as u32;
        let derived_skill_candidate_count = session_records
            .iter()
            .filter(|record| {
                record.kind == MemoryKind::MethodologyCandidate
                    && record.derived_skill_name.is_some()
            })
            .count() as u32;
        let linked_skill_count = session_records
            .iter()
            .filter(|record| record.linked_skill_name.is_some())
            .count() as u32;
        let skill_feedback_lesson_count = session_records
            .iter()
            .filter(|record| {
                record.kind == MemoryKind::Lesson && record.linked_skill_name.is_some()
            })
            .count() as u32;
        let retrieval_run_count = retrieval_logs
            .iter()
            .filter(|entry| entry.retrieved_count > 0)
            .count() as u32;
        let retrieval_hit_count = retrieval_logs
            .iter()
            .map(|entry| entry.retrieved_count)
            .sum();
        let retrieval_use_count = retrieval_logs.iter().map(|entry| entry.used_count).sum();
        let recent_session_records = session_records
            .iter()
            .take(8)
            .map(MemoryCardView::from)
            .collect::<Vec<_>>();

        let summary = SessionMemoryTelemetrySummary {
            workspace_key: context.workspace_key,
            workspace_mode: workspace_mode_label(context.workspace_mode).to_string(),
            allowed_scopes: context.allowed_scopes,
            frozen_snapshot_generated_at: frozen_snapshot
                .as_ref()
                .map(|packet| packet.generated_at),
            frozen_snapshot_items: frozen_snapshot
                .as_ref()
                .map(|packet| packet.items.len() as u32)
                .unwrap_or_default(),
            last_prefetch_generated_at: last_prefetch_packet
                .as_ref()
                .map(|packet| packet.generated_at),
            last_prefetch_items: last_prefetch_packet
                .as_ref()
                .map(|packet| packet.items.len() as u32)
                .unwrap_or_default(),
            last_prefetch_query: last_prefetch_packet
                .as_ref()
                .and_then(|packet| packet.query.clone()),
            candidate_count,
            validated_count,
            rejected_count,
            warning_count,
            methodology_candidate_count,
            derived_skill_candidate_count,
            linked_skill_count,
            skill_feedback_lesson_count,
            retrieval_run_count,
            retrieval_hit_count,
            retrieval_use_count,
            latest_consolidation_run,
            recent_rule_hits,
        };

        Ok(Some((
            summary,
            frozen_snapshot,
            last_prefetch_packet,
            recent_session_records,
        )))
    }

    async fn session_runtime_memory_state(
        &self,
        session: &Session,
        context: &ResolvedMemoryContext,
    ) -> Result<(Vec<MemoryRecord>, Vec<MemoryRetrievalLogEntry>)> {
        let Some(repository) = &self.repository else {
            return Ok((Vec::new(), Vec::new()));
        };

        let mut session_records = repository
            .list_records(Some(&MemoryRepositoryFilter {
                scopes: context.allowed_scopes.clone(),
                statuses: vec![
                    MemoryStatus::Candidate,
                    MemoryStatus::Validated,
                    MemoryStatus::Consolidated,
                    MemoryStatus::Rejected,
                    MemoryStatus::Archived,
                ],
                workspace_identity: Some(context.workspace_key.clone()),
                source_session_id: Some(session.id.clone()),
                limit: Some(64),
                ..MemoryRepositoryFilter::default()
            }))
            .await?;
        session_records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

        let retrieval_logs = repository
            .list_retrieval_logs(Some(&session.id), Some(20))
            .await?;
        Ok((session_records, retrieval_logs))
    }
}

pub fn load_persisted_memory_snapshot(session: &Session) -> Option<PersistedMemorySnapshot> {
    session
        .metadata
        .get(MEMORY_FROZEN_SNAPSHOT_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

pub fn persist_persisted_memory_snapshot(
    session: &mut Session,
    snapshot: &PersistedMemorySnapshot,
) -> Result<()> {
    let value = serde_json::to_value(snapshot)?;
    session
        .metadata
        .insert(MEMORY_FROZEN_SNAPSHOT_METADATA_KEY.to_string(), value);
    Ok(())
}

pub fn load_last_prefetch_packet(session: &Session) -> Option<MemoryRetrievalPacket> {
    session
        .metadata
        .get(MEMORY_LAST_PREFETCH_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

pub fn persist_last_prefetch_packet(
    session: &mut Session,
    packet: &MemoryRetrievalPacket,
) -> Result<()> {
    let value = serde_json::to_value(packet)?;
    session
        .metadata
        .insert(MEMORY_LAST_PREFETCH_METADATA_KEY.to_string(), value);
    Ok(())
}

fn workspace_mode_label(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Shared => "shared",
        WorkspaceMode::Isolated => "isolated",
    }
}

fn latest_skill_save_suggestion(
    session: &Session,
) -> Option<(&agendao_types::SessionMessage, Option<String>)> {
    let note_message = session.messages.iter().rev().find(|message| {
        message.role == MessageRole::Assistant
            && message
                .metadata
                .get("runtime_hint")
                .and_then(|value| value.as_str())
                == Some("skill_save_suggestion")
    })?;

    let user_prompt = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .and_then(|message| {
            message
                .metadata
                .get("resolved_user_prompt")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    let text = message.get_text();
                    (!text.trim().is_empty()).then_some(text)
                })
        });

    Some((note_message, user_prompt))
}

fn summarize_tool_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        "Tool produced no textual output.".to_string()
    } else {
        truncate(trimmed, 280)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchStrength {
    Strong,
}

fn should_emit_skill_feedback_lesson(observation: &SkillWriteObservation<'_>) -> bool {
    !observation.action.eq_ignore_ascii_case("create")
        || observation
            .guard_report
            .is_some_and(|report| report.status != SkillGuardStatus::Passed)
}

fn apply_skill_linkage_to_record(
    mut record: MemoryRecord,
    observation: &SkillWriteObservation<'_>,
    action: &str,
    normalized_skill_name: &str,
    now: i64,
) -> MemoryRecord {
    record.linked_skill_name = Some(observation.skill_name.to_string());
    if record
        .derived_skill_name
        .as_deref()
        .map(normalize_skill_name)
        .unwrap_or_default()
        .is_empty()
        && observation.action.eq_ignore_ascii_case("create")
    {
        record.derived_skill_name = Some(observation.skill_name.to_string());
    }
    record.title = format!(
        "Methodology candidate linked to skill {}",
        observation.skill_name
    );
    record.summary = truncate(
        &format!(
            "Skill `{}` was {} and linked back to this methodology candidate. Reuse only after checking trigger conditions, validation, and current workspace constraints.",
            observation.skill_name, action
        ),
        320,
    );
    push_unique(
        &mut record.trigger_conditions,
        format!("skill:{}", observation.skill_name),
    );
    push_unique(
        &mut record.trigger_conditions,
        format!("skill_action:{}", action),
    );
    push_unique(
        &mut record.normalized_facts,
        format!("skill_name={}", observation.skill_name),
    );
    push_unique(
        &mut record.normalized_facts,
        format!("skill_action={}", action),
    );
    push_unique(
        &mut record.normalized_facts,
        format!("linked_skill_name={}", observation.skill_name),
    );
    if !normalized_skill_name.is_empty() {
        push_unique(
            &mut record.normalized_facts,
            format!("skill_name_normalized={}", normalized_skill_name),
        );
    }
    if let Some(location) = observation.location {
        push_unique(
            &mut record.normalized_facts,
            format!("skill_path={}", location),
        );
    }
    if let Some(file_path) = observation.supporting_file {
        push_unique(
            &mut record.normalized_facts,
            format!("supporting_file={}", file_path),
        );
    }
    push_unique(
        &mut record.boundaries,
        "Linked to a concrete workspace skill; re-check the current SKILL.md before reuse."
            .to_string(),
    );
    record.updated_at = now;
    record
}

fn build_skill_promotion_record(
    observation: &SkillWriteObservation<'_>,
    context: &ResolvedMemoryContext,
) -> MemoryRecord {
    let now = chrono::Utc::now().timestamp_millis();
    let action = normalize_skill_action(observation.action);
    MemoryRecord {
        id: hashed_record_id(
            "mem_skill_promotion",
            &[
                observation.session_id,
                observation.skill_name,
                action.as_str(),
                observation.location.unwrap_or(""),
            ],
        ),
        kind: MemoryKind::MethodologyCandidate,
        scope: candidate_scope_for_mode(context.workspace_mode),
        status: MemoryStatus::Candidate,
        title: format!("Skill promotion candidate: {}", observation.skill_name),
        summary: truncate(
            &format!(
                "Session {} promoted a reusable methodology into workspace skill `{}` via action `{}`.",
                observation.session_id, observation.skill_name, action
            ),
            320,
        ),
        trigger_conditions: vec![
            format!("skill:{}", observation.skill_name),
            format!("skill_action:{}", action),
        ],
        normalized_facts: build_skill_observation_facts(observation, &action),
        boundaries: vec![
            "Represents promotion from reusable methodology into governed skill state."
                .to_string(),
            "Do not treat the skill content as timeless; patch the skill when environment assumptions change."
                .to_string(),
        ],
        confidence: Some(0.8),
        evidence_refs: vec![MemoryEvidenceRef {
            session_id: Some(observation.session_id.to_string()),
            message_id: None,
            tool_call_id: observation.tool_call_id.map(ToOwned::to_owned),
            stage_id: None,
            note: Some("skill_manage.write".to_string()),
        }],
        source_session_id: Some(observation.session_id.to_string()),
        workspace_identity: Some(context.workspace_key.clone()),
        created_at: now,
        updated_at: now,
        last_validated_at: None,
        expires_at: None,
        derived_skill_name: Some(observation.skill_name.to_string()),
        linked_skill_name: Some(observation.skill_name.to_string()),
        validation_status: MemoryValidationStatus::Pending,
    }
}

fn build_skill_feedback_lesson_record(
    observation: &SkillWriteObservation<'_>,
    context: &ResolvedMemoryContext,
) -> MemoryRecord {
    let now = chrono::Utc::now().timestamp_millis();
    let action = normalize_skill_action(observation.action);
    let summary = match observation.supporting_file {
        Some(file_path) => format!(
            "Skill `{}` required `{}` on supporting file `{}`. Treat this as feedback that the prior methodology or packaging was incomplete.",
            observation.skill_name, action, file_path
        ),
        None => format!(
            "Skill `{}` required `{}`. Treat this as feedback that the prior methodology or environment guidance needed correction.",
            observation.skill_name, action
        ),
    };
    let mut boundaries = vec![
        "Applies to the linked skill and workspace context captured in evidence.".to_string(),
        "Re-check the current skill content and runtime environment before reusing this lesson."
            .to_string(),
    ];
    if let Some(report) = observation.guard_report {
        boundaries.push(format!(
            "Guard status was {:?}; violations should be reviewed before further reuse.",
            report.status
        ));
    }

    MemoryRecord {
        id: hashed_record_id(
            "mem_skill_feedback",
            &[
                observation.session_id,
                observation.skill_name,
                action.as_str(),
                observation.supporting_file.unwrap_or(""),
                observation.location.unwrap_or(""),
            ],
        ),
        kind: MemoryKind::Lesson,
        scope: candidate_scope_for_mode(context.workspace_mode),
        status: MemoryStatus::Candidate,
        title: format!("Skill feedback lesson: {}", observation.skill_name),
        summary: truncate(&summary, 320),
        trigger_conditions: vec![
            format!("skill:{}", observation.skill_name),
            format!("skill_action:{}", action),
        ],
        normalized_facts: build_skill_observation_facts(observation, &action),
        boundaries,
        confidence: Some(0.72),
        evidence_refs: vec![MemoryEvidenceRef {
            session_id: Some(observation.session_id.to_string()),
            message_id: None,
            tool_call_id: observation.tool_call_id.map(ToOwned::to_owned),
            stage_id: None,
            note: Some("skill_manage.feedback".to_string()),
        }],
        source_session_id: Some(observation.session_id.to_string()),
        workspace_identity: Some(context.workspace_key.clone()),
        created_at: now,
        updated_at: now,
        last_validated_at: None,
        expires_at: None,
        derived_skill_name: None,
        linked_skill_name: Some(observation.skill_name.to_string()),
        validation_status: MemoryValidationStatus::Pending,
    }
}

fn build_skill_usage_record(
    observation: &SkillUsageObservation<'_>,
    context: &ResolvedMemoryContext,
) -> MemoryRecord {
    let now = chrono::Utc::now().timestamp_millis();
    let tool_name = observation.tool_name.trim();
    let outcome = if observation.is_error {
        "error"
    } else {
        "success"
    };
    let summary = if observation.is_error {
        format!(
            "Skill `{}` was used through `{}` and ended with an error. {}",
            observation.skill_name,
            tool_name,
            summarize_tool_output(observation.output)
        )
    } else {
        format!(
            "Skill `{}` was used through `{}` and completed successfully. {}",
            observation.skill_name,
            tool_name,
            summarize_tool_output(observation.output)
        )
    };
    let mut trigger_conditions = vec![
        format!("skill:{}", observation.skill_name),
        format!("tool:{}", tool_name),
    ];
    if let Some(stage_id) = observation.stage_id {
        trigger_conditions.push(format!("stage_id:{}", stage_id));
    }
    if let Some(category) = observation.category {
        trigger_conditions.push(format!("category:{}", category));
    }

    let mut normalized_facts = vec![
        format!("skill_name={}", observation.skill_name),
        format!(
            "skill_name_normalized={}",
            normalize_skill_name(observation.skill_name)
        ),
        format!("tool_name={}", tool_name),
        format!("tool_outcome={}", outcome),
        "memory_source=skill_runtime_usage".to_string(),
    ];
    if let Some(stage_id) = observation.stage_id {
        normalized_facts.push(format!("stage_id={}", stage_id));
    }
    if let Some(category) = observation.category {
        normalized_facts.push(format!("category={}", category));
    }

    MemoryRecord {
        id: hashed_record_id(
            "mem_skill_usage",
            &[
                observation.session_id,
                observation.tool_call_id,
                observation.skill_name,
                tool_name,
                outcome,
            ],
        ),
        kind: if observation.is_error {
            MemoryKind::Lesson
        } else {
            MemoryKind::Pattern
        },
        scope: candidate_scope_for_mode(context.workspace_mode),
        status: MemoryStatus::Candidate,
        title: if observation.is_error {
            format!("Skill usage lesson: {}", observation.skill_name)
        } else {
            format!("Skill usage pattern: {}", observation.skill_name)
        },
        summary: truncate(&summary, 320),
        trigger_conditions,
        normalized_facts,
        boundaries: vec![
            "Derived from explicit runtime skill loading, not inferred from generic tool use."
                .to_string(),
            "Re-check the current skill body and workspace state before reusing this feedback."
                .to_string(),
        ],
        confidence: Some(if observation.is_error { 0.7 } else { 0.5 }),
        evidence_refs: vec![MemoryEvidenceRef {
            session_id: Some(observation.session_id.to_string()),
            message_id: None,
            tool_call_id: Some(observation.tool_call_id.to_string()),
            stage_id: observation.stage_id.map(ToOwned::to_owned),
            note: Some("skill_runtime_usage".to_string()),
        }],
        source_session_id: Some(observation.session_id.to_string()),
        workspace_identity: Some(context.workspace_key.clone()),
        created_at: now,
        updated_at: now,
        last_validated_at: None,
        expires_at: None,
        derived_skill_name: None,
        linked_skill_name: Some(observation.skill_name.to_string()),
        validation_status: MemoryValidationStatus::Pending,
    }
}

fn build_skill_observation_facts(
    observation: &SkillWriteObservation<'_>,
    action: &str,
) -> Vec<String> {
    let mut facts = vec![
        format!("skill_name={}", observation.skill_name),
        format!(
            "skill_name_normalized={}",
            normalize_skill_name(observation.skill_name)
        ),
        format!("skill_action={}", action),
        "memory_source=skill_manage".to_string(),
    ];
    if let Some(location) = observation.location {
        facts.push(format!("skill_path={}", location));
    }
    if let Some(file_path) = observation.supporting_file {
        facts.push(format!("supporting_file={}", file_path));
    }
    if let Some(report) = observation.guard_report {
        facts.push(format!("guard_status={:?}", report.status).to_ascii_lowercase());
        if !report.violations.is_empty() {
            facts.push(format!("guard_violation_count={}", report.violations.len()));
        }
    }
    facts
}

fn skill_record_matches_observation(
    record: &MemoryRecord,
    observation: &SkillWriteObservation<'_>,
    normalized_skill_name: &str,
    _: MatchStrength,
) -> bool {
    if record.linked_skill_name.as_deref() == Some(observation.skill_name) {
        return true;
    }

    if record
        .derived_skill_name
        .as_deref()
        .map(normalize_skill_name)
        .is_some_and(|value| value == normalized_skill_name)
    {
        return true;
    }

    record_mentions_skill_name(record, normalized_skill_name)
}

fn record_mentions_skill_name(record: &MemoryRecord, normalized_skill_name: &str) -> bool {
    if normalized_skill_name.is_empty() {
        return false;
    }

    let mut haystacks = vec![
        record.title.as_str(),
        record.summary.as_str(),
        record.derived_skill_name.as_deref().unwrap_or(""),
        record.linked_skill_name.as_deref().unwrap_or(""),
    ];
    haystacks.extend(record.trigger_conditions.iter().map(String::as_str));
    haystacks.extend(record.normalized_facts.iter().map(String::as_str));
    haystacks.extend(record.boundaries.iter().map(String::as_str));

    haystacks
        .into_iter()
        .map(normalize_skill_name)
        .any(|value| value.contains(normalized_skill_name))
}

fn normalize_skill_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn normalize_skill_action(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace('_', " ");
    if normalized.is_empty() {
        "updated".to_string()
    } else {
        normalized
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn hashed_record_id(prefix: &str, parts: &[&str]) -> MemoryRecordId {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let suffix = format!("{digest:x}");
    MemoryRecordId(format!("{}_{}", prefix, &suffix[..24]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_config::ConfigStore;
    use agendao_storage::Database;
    use agendao_types::SkillGuardViolation;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock error")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("failed to create test temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn authority_for(root: &Path, isolated: bool) -> MemoryAuthority {
        if isolated {
            fs::create_dir_all(root.join(".agendao")).expect("failed to create .agendao");
        }
        let store = Arc::new(
            ConfigStore::from_project_dir(root).expect("config store should resolve test root"),
        );
        let user_state = Arc::new(UserStateAuthority::from_config_store(&store));
        let resolved_context = Arc::new(ResolvedWorkspaceContextAuthority::new(
            store.clone(),
            user_state.clone(),
        ));
        MemoryAuthority::new(user_state, resolved_context)
    }

    fn authority_with_repository(
        root: &Path,
        isolated: bool,
        repository: Arc<MemoryRepository>,
    ) -> MemoryAuthority {
        authority_for(root, isolated).with_repository(repository)
    }

    fn make_candidate_record(
        id: &str,
        workspace_identity: String,
        facts: Vec<&str>,
    ) -> MemoryRecord {
        let now = chrono::Utc::now().timestamp_millis();
        MemoryRecord {
            id: MemoryRecordId(id.to_string()),
            kind: MemoryKind::Pattern,
            scope: MemoryScope::WorkspaceShared,
            status: MemoryStatus::Candidate,
            title: "Reusable test workflow".to_string(),
            summary: "A validated reusable workflow candidate from runtime telemetry.".to_string(),
            trigger_conditions: vec!["tool:cargo_test".to_string()],
            normalized_facts: facts.into_iter().map(str::to_string).collect(),
            boundaries: vec!["Validate before long-term promotion.".to_string()],
            confidence: Some(0.5),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some("ses_1".to_string()),
                message_id: Some("msg_1".to_string()),
                tool_call_id: Some("call_1".to_string()),
                stage_id: Some("stage_1".to_string()),
                note: Some("test fixture".to_string()),
            }],
            source_session_id: Some("ses_1".to_string()),
            workspace_identity: Some(workspace_identity),
            created_at: now,
            updated_at: now,
            last_validated_at: None,
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Pending,
        }
    }

    fn make_validated_lesson_record(
        id: &str,
        workspace_identity: String,
        note: &str,
    ) -> MemoryRecord {
        let now = chrono::Utc::now().timestamp_millis();
        MemoryRecord {
            id: MemoryRecordId(id.to_string()),
            kind: MemoryKind::Lesson,
            scope: MemoryScope::WorkspaceShared,
            status: MemoryStatus::Validated,
            title: "Tool failure candidate: cargo test".to_string(),
            summary: format!("Repeated test failure lesson: {}", note),
            trigger_conditions: vec!["tool:cargo_test".to_string(), "verify:test".to_string()],
            normalized_facts: vec![
                "tool_name=cargo_test".to_string(),
                "tool_outcome=error".to_string(),
                "verify:test".to_string(),
            ],
            boundaries: vec![
                "Only applies to test execution in this workspace.".to_string(),
                "Re-check the current test output before reuse.".to_string(),
            ],
            confidence: Some(0.7),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some("ses_lesson".to_string()),
                message_id: Some(format!("msg_{id}")),
                tool_call_id: Some(format!("call_{id}")),
                stage_id: Some("stage_test".to_string()),
                note: Some("lesson fixture".to_string()),
            }],
            source_session_id: Some("ses_lesson".to_string()),
            workspace_identity: Some(workspace_identity),
            created_at: now,
            updated_at: now,
            last_validated_at: Some(now),
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: None,
            validation_status: MemoryValidationStatus::Passed,
        }
    }

    #[test]
    fn allowed_scopes_follow_workspace_mode() {
        assert_eq!(
            allowed_scopes_for_mode(WorkspaceMode::Shared),
            vec![
                MemoryScope::GlobalUser,
                MemoryScope::GlobalWorkspace,
                MemoryScope::WorkspaceShared,
            ]
        );
        assert_eq!(
            allowed_scopes_for_mode(WorkspaceMode::Isolated),
            vec![MemoryScope::WorkspaceSandbox, MemoryScope::SessionEphemeral]
        );
    }

    #[tokio::test]
    async fn resolve_context_uses_shared_workspace_rules() {
        let dir = TestDir::new("agendao_memory_shared");
        let authority = authority_for(&dir.path, false);

        let resolved = authority
            .resolve_context()
            .await
            .expect("memory context should resolve");

        assert_eq!(resolved.workspace_mode, WorkspaceMode::Shared);
        assert_eq!(
            resolved.allowed_scopes,
            vec![
                MemoryScope::GlobalUser,
                MemoryScope::GlobalWorkspace,
                MemoryScope::WorkspaceShared,
            ]
        );
    }

    #[tokio::test]
    async fn resolve_context_uses_isolated_workspace_rules() {
        let dir = TestDir::new("agendao_memory_isolated");
        let authority = authority_for(&dir.path, true);

        let resolved = authority
            .resolve_context()
            .await
            .expect("memory context should resolve");

        assert_eq!(resolved.workspace_mode, WorkspaceMode::Isolated);
        assert_eq!(
            resolved.allowed_scopes,
            vec![MemoryScope::WorkspaceSandbox, MemoryScope::SessionEphemeral]
        );
    }

    #[tokio::test]
    async fn list_memory_reads_repository_backed_records() {
        let dir = TestDir::new("agendao_memory_repo");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository.clone());

        repository
            .upsert_record(&MemoryRecord {
                id: MemoryRecordId("mem_1".to_string()),
                kind: MemoryKind::Pattern,
                scope: MemoryScope::WorkspaceShared,
                status: MemoryStatus::Candidate,
                title: "Reusable test pattern".to_string(),
                summary: "Pattern from repo".to_string(),
                trigger_conditions: vec![],
                normalized_facts: vec!["tool_name=cargo_test".to_string()],
                boundaries: vec![],
                confidence: Some(0.4),
                evidence_refs: vec![],
                source_session_id: Some("ses_1".to_string()),
                workspace_identity: Some(
                    authority
                        .resolve_context()
                        .await
                        .expect("context should resolve")
                        .workspace_key,
                ),
                created_at: 1,
                updated_at: 2,
                last_validated_at: None,
                expires_at: None,
                derived_skill_name: None,
                linked_skill_name: None,
                validation_status: MemoryValidationStatus::Pending,
            })
            .await
            .expect("record should persist");

        let items = authority
            .list_memory(None)
            .await
            .expect("list should succeed");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Reusable test pattern");
    }

    #[tokio::test]
    async fn validated_records_enter_snapshot_and_candidates_do_not() {
        let dir = TestDir::new("agendao_memory_snapshot");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository);

        let result = authority
            .ingest_tool_result_observation(&ToolMemoryObservation {
                session_id: "ses_1",
                tool_call_id: "call_1",
                tool_name: "cargo_test",
                stage_id: Some("stage_1"),
                output: "all tests passed",
                is_error: false,
            })
            .await
            .expect("ingest should succeed")
            .expect("candidate should persist");

        assert_eq!(result.status, MemoryStatus::Validated);
        assert_eq!(result.validation_status, MemoryValidationStatus::Passed);

        let snapshot = authority
            .build_frozen_snapshot()
            .await
            .expect("snapshot should build");
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].card.id, result.id);
    }

    #[tokio::test]
    async fn duplicate_candidates_are_rejected_by_validation_engine() {
        let dir = TestDir::new("agendao_memory_duplicate");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository);
        let workspace_identity = authority
            .resolve_context()
            .await
            .expect("context should resolve")
            .workspace_key;
        let context = authority
            .resolve_context()
            .await
            .expect("context should resolve");

        let first = authority
            .persist_candidate_record(
                make_candidate_record(
                    "mem_a",
                    workspace_identity.clone(),
                    vec!["tool_name=cargo_test", "tool_outcome=success"],
                ),
                &context,
            )
            .await
            .expect("first record should persist");
        let second = authority
            .persist_candidate_record(
                make_candidate_record(
                    "mem_b",
                    workspace_identity,
                    vec!["tool_name=cargo_test", "tool_outcome=success"],
                ),
                &context,
            )
            .await
            .expect("second record should persist");

        assert_eq!(first.status, MemoryStatus::Validated);
        assert_eq!(second.status, MemoryStatus::Rejected);
        assert_eq!(second.validation_status, MemoryValidationStatus::Failed);
    }

    #[tokio::test]
    async fn contradictory_candidates_remain_candidates_with_warning() {
        let dir = TestDir::new("agendao_memory_contradiction");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository);
        let workspace_identity = authority
            .resolve_context()
            .await
            .expect("context should resolve")
            .workspace_key;
        let context = authority
            .resolve_context()
            .await
            .expect("context should resolve");

        let first = authority
            .persist_candidate_record(
                make_candidate_record(
                    "mem_c",
                    workspace_identity.clone(),
                    vec!["tool_name=cargo_test", "tool_outcome=success"],
                ),
                &context,
            )
            .await
            .expect("first record should persist");
        let second = authority
            .persist_candidate_record(
                make_candidate_record(
                    "mem_d",
                    workspace_identity,
                    vec!["tool_name=cargo_test", "tool_outcome=error"],
                ),
                &context,
            )
            .await
            .expect("second record should persist");

        assert_eq!(first.status, MemoryStatus::Validated);
        assert_eq!(second.status, MemoryStatus::Candidate);
        assert_eq!(second.validation_status, MemoryValidationStatus::Warning);
    }

    #[tokio::test]
    async fn consolidation_promotes_repeated_lessons_and_records_rule_hits() {
        let dir = TestDir::new("agendao_memory_consolidation");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository.clone());
        let workspace_identity = authority
            .resolve_context()
            .await
            .expect("context should resolve")
            .workspace_key;

        repository
            .upsert_record(&make_validated_lesson_record(
                "mem_lesson_1",
                workspace_identity.clone(),
                "fixtures missing",
            ))
            .await
            .expect("lesson should persist");
        repository
            .upsert_record(&make_validated_lesson_record(
                "mem_lesson_2",
                workspace_identity,
                "fixtures stale",
            ))
            .await
            .expect("lesson should persist");

        let response = authority
            .run_consolidation(&agendao_types::MemoryConsolidationRequest::default())
            .await
            .expect("consolidation should succeed");

        assert!(!response.promoted_record_ids.is_empty());
        assert!(!response.rule_hits.is_empty());

        let rules = authority
            .list_memory_rule_packs()
            .await
            .expect("rule packs should load");
        assert!(
            rules.items.len() >= 3,
            "builtin validation/consolidation/reflection packs should be present"
        );

        let runs = authority
            .list_consolidation_runs(&agendao_types::MemoryConsolidationRunQuery { limit: Some(10) })
            .await
            .expect("runs should load");
        assert_eq!(runs.items.len(), 1);

        let hits = authority
            .list_memory_rule_hits(&agendao_types::MemoryRuleHitQuery {
                run_id: Some(response.run.run_id.clone()),
                ..Default::default()
            })
            .await
            .expect("rule hits should load");
        assert!(!hits.items.is_empty());
    }

    #[tokio::test]
    async fn skill_write_observation_links_tool_candidate_and_emits_feedback_lesson() {
        let dir = TestDir::new("agendao_memory_skill_write");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository.clone());

        let tool_candidate = authority
            .ingest_tool_result_observation(&ToolMemoryObservation {
                session_id: "ses_skill",
                tool_call_id: "call_skill",
                tool_name: "skill_manage",
                stage_id: Some("stage_skill"),
                output:
                    "<skill_manage_result action=\"create\" skill=\"provider-refresh\" path=\".agendao/skills/provider-refresh/SKILL.md\">created</skill_manage_result>",
                is_error: false,
            })
            .await
            .expect("tool candidate should ingest")
            .expect("tool candidate should exist");

        let linked_records = authority
            .ingest_skill_write_observation(&SkillWriteObservation {
                session_id: "ses_skill",
                tool_call_id: Some("call_skill"),
                skill_name: "provider-refresh",
                action: "create",
                location: Some(".agendao/skills/provider-refresh/SKILL.md"),
                supporting_file: None,
                guard_report: None,
            })
            .await
            .expect("skill create observation should succeed");
        assert!(
            linked_records.iter().any(|record| {
                record.id == tool_candidate.id
                    && record.linked_skill_name.as_deref() == Some("provider-refresh")
            }),
            "existing skill_manage candidate should be linked back to the created skill"
        );
        assert!(linked_records.iter().any(|record| {
            record.derived_skill_name.as_deref() == Some("provider-refresh")
                && record.linked_skill_name.as_deref() == Some("provider-refresh")
        }));

        let updated_candidate = repository
            .get_record(&tool_candidate.id.0)
            .await
            .expect("candidate lookup should succeed")
            .expect("candidate should remain present");
        assert_eq!(
            updated_candidate.linked_skill_name.as_deref(),
            Some("provider-refresh")
        );

        authority
            .ingest_skill_write_observation(&SkillWriteObservation {
                session_id: "ses_skill",
                tool_call_id: Some("call_skill"),
                skill_name: "provider-refresh",
                action: "patch",
                location: Some(".agendao/skills/provider-refresh/SKILL.md"),
                supporting_file: Some("notes.md"),
                guard_report: Some(&SkillGuardReport {
                    skill_name: "provider-refresh".to_string(),
                    status: SkillGuardStatus::Warn,
                    violations: vec![SkillGuardViolation {
                        rule_id: "remote_fetch".to_string(),
                        severity: agendao_types::SkillGuardSeverity::Warn,
                        message: "remote fetch found".to_string(),
                        file_path: Some("notes.md".to_string()),
                    }],
                    scanned_at: 123,
                }),
            })
            .await
            .expect("skill patch observation should succeed");

        let lesson_records = repository
            .list_records(Some(&MemoryRepositoryFilter {
                scopes: authority
                    .resolve_context()
                    .await
                    .expect("context should resolve")
                    .allowed_scopes,
                kinds: vec![MemoryKind::Lesson],
                statuses: vec![MemoryStatus::Candidate, MemoryStatus::Validated],
                source_session_id: Some("ses_skill".to_string()),
                limit: Some(20),
                ..MemoryRepositoryFilter::default()
            }))
            .await
            .expect("lesson records should list");
        assert!(lesson_records.iter().any(|record| {
            record.linked_skill_name.as_deref() == Some("provider-refresh")
                && record
                    .normalized_facts
                    .iter()
                    .any(|fact| fact == "supporting_file=notes.md")
        }));

        let workspace_identity = authority
            .resolve_context()
            .await
            .expect("context should resolve")
            .workspace_key;
        repository
            .upsert_record(&MemoryRecord {
                id: MemoryRecordId("mem_validated_prefetch".to_string()),
                kind: MemoryKind::Pattern,
                scope: MemoryScope::WorkspaceShared,
                status: MemoryStatus::Validated,
                title: "Validated provider refresh pattern".to_string(),
                summary: "Reusable validated provider refresh pattern.".to_string(),
                trigger_conditions: vec!["skill:provider-refresh".to_string()],
                normalized_facts: vec!["skill_name=provider-refresh".to_string()],
                boundaries: vec!["Only use after checking provider config.".to_string()],
                confidence: Some(0.8),
                evidence_refs: vec![MemoryEvidenceRef {
                    session_id: Some("ses_skill".to_string()),
                    message_id: None,
                    tool_call_id: None,
                    stage_id: None,
                    note: Some("prefetch fixture".to_string()),
                }],
                source_session_id: Some("ses_skill".to_string()),
                workspace_identity: Some(workspace_identity),
                created_at: 10,
                updated_at: 10,
                last_validated_at: Some(10),
                expires_at: None,
                derived_skill_name: Some("provider-refresh".to_string()),
                linked_skill_name: Some("provider-refresh".to_string()),
                validation_status: MemoryValidationStatus::Passed,
            })
            .await
            .expect("validated prefetch record should persist");

        let packet = authority
            .build_prefetch_packet(&MemoryRetrievalQuery {
                query: Some("provider refresh".to_string()),
                session_id: Some("ses_skill".to_string()),
                limit: Some(4),
                ..Default::default()
            })
            .await
            .expect("prefetch should succeed");
        authority
            .record_prefetch_usage("ses_skill", &packet)
            .await
            .expect("prefetch usage should persist");

        let session = Session {
            id: "ses_skill".to_string(),
            slug: "ses_skill".to_string(),
            project_id: "project".to_string(),
            directory: dir.path.to_string_lossy().to_string(),
            parent_id: None,
            title: "skill memory session".to_string(),
            version: "1".to_string(),
            time: Default::default(),
            messages: Vec::new(),
            summary: None,
            share: None,
            revert: None,
            permission: None,
            usage: None,
            status: Default::default(),
            metadata: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let telemetry = authority
            .build_session_memory_telemetry(&session)
            .await
            .expect("telemetry should build")
            .expect("telemetry should exist");
        assert!(telemetry.warning_count >= 1);
        assert!(telemetry.methodology_candidate_count >= 1);
        assert!(telemetry.derived_skill_candidate_count >= 1);
        assert!(telemetry.linked_skill_count >= 2);
        assert!(telemetry.skill_feedback_lesson_count >= 1);
        assert_eq!(telemetry.retrieval_run_count, 1);
        assert!(telemetry.retrieval_hit_count >= 1);
        assert!(telemetry.retrieval_use_count >= 1);

        let filtered = authority
            .list_memory_for_query(&MemoryListQuery {
                linked_skill_name: Some("provider-refresh".to_string()),
                derived_skill_name: Some("provider-refresh".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .await
            .expect("filtered list should succeed");
        assert!(!filtered.items.is_empty());
        assert!(filtered.items.iter().all(|item| {
            item.linked_skill_name.as_deref() == Some("provider-refresh")
                && item.derived_skill_name.as_deref() == Some("provider-refresh")
        }));
    }

    #[tokio::test]
    async fn skill_usage_observation_creates_linked_runtime_feedback_record() {
        let dir = TestDir::new("agendao_memory_skill_usage");
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        let authority = authority_with_repository(&dir.path, false, repository.clone());

        let record = authority
            .ingest_skill_usage_observation(&SkillUsageObservation {
                session_id: "ses_usage",
                tool_call_id: "call_usage",
                tool_name: "task",
                stage_id: Some("stage_usage"),
                skill_name: "frontend-ui-ux",
                category: Some("frontend"),
                output: "Subtask completed after applying the frontend skill.",
                is_error: false,
            })
            .await
            .expect("skill usage should ingest")
            .expect("skill usage record should exist");

        assert_eq!(record.linked_skill_name.as_deref(), Some("frontend-ui-ux"));
        assert_eq!(record.kind, MemoryKind::Pattern);
        assert!(record
            .normalized_facts
            .iter()
            .any(|fact| fact == "memory_source=skill_runtime_usage"));

        let persisted = repository
            .get_record(&record.id.0)
            .await
            .expect("lookup should succeed")
            .expect("persisted record should exist");
        assert_eq!(
            persisted.linked_skill_name.as_deref(),
            Some("frontend-ui-ux")
        );
        assert!(persisted
            .trigger_conditions
            .iter()
            .any(|trigger| trigger == "category:frontend"));
    }
}
