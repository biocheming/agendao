use std::sync::Arc;

use anyhow::Result;
use rocode_command::stage_protocol::StageSummary;
use rocode_memory::{
    MemoryAuthority, SkillUsageObservation, SkillWriteObservation, ToolMemoryObservation,
};
use rocode_types::{
    MemoryConflictResponse, MemoryConsolidationRequest, MemoryConsolidationResponse,
    MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery, MemoryDetailView,
    MemoryListQuery, MemoryListResponse, MemoryRecordId, MemoryRetrievalPacket,
    MemoryRetrievalPreviewResponse, MemoryRetrievalQuery, MemoryRuleHitListResponse,
    MemoryRuleHitQuery, MemoryRulePackListResponse, MemoryValidationReportResponse, Session,
    SessionMemoryInsight, SessionMemoryTelemetrySummary, SkillGuardReport,
};

#[derive(Clone)]
pub(crate) struct RuntimeMemoryAuthority {
    memory: Arc<MemoryAuthority>,
}

impl RuntimeMemoryAuthority {
    pub(crate) fn new(memory: Arc<MemoryAuthority>) -> Self {
        Self { memory }
    }

    #[cfg(test)]
    pub(crate) async fn list_memory(
        &self,
        filter: Option<&rocode_memory::MemoryFilter<'_>>,
    ) -> Result<Vec<rocode_types::MemoryCardView>> {
        self.memory.list_memory(filter).await
    }

    pub(crate) async fn list_memory_for_query(
        &self,
        query: &MemoryListQuery,
    ) -> Result<MemoryListResponse> {
        self.memory.list_memory_for_query(query).await
    }

    pub(crate) async fn search_memory_for_query(
        &self,
        query: &MemoryListQuery,
    ) -> Result<MemoryListResponse> {
        self.memory.search_memory_for_query(query).await
    }

    pub(crate) async fn list_memory_rule_packs(&self) -> Result<MemoryRulePackListResponse> {
        self.memory.list_memory_rule_packs().await
    }

    pub(crate) async fn list_memory_rule_hits(
        &self,
        query: &MemoryRuleHitQuery,
    ) -> Result<MemoryRuleHitListResponse> {
        self.memory.list_memory_rule_hits(query).await
    }

    pub(crate) async fn list_consolidation_runs(
        &self,
        query: &MemoryConsolidationRunQuery,
    ) -> Result<MemoryConsolidationRunListResponse> {
        self.memory.list_consolidation_runs(query).await
    }

    pub(crate) async fn run_consolidation(
        &self,
        request: &MemoryConsolidationRequest,
    ) -> Result<MemoryConsolidationResponse> {
        self.memory.run_consolidation(request).await
    }

    pub(crate) async fn build_retrieval_preview(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> Result<MemoryRetrievalPreviewResponse> {
        self.memory.build_retrieval_preview(query).await
    }

    pub(crate) async fn get_memory_detail(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<MemoryDetailView>> {
        self.memory.get_memory_detail(record_id).await
    }

    pub(crate) async fn get_memory_validation_report(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<MemoryValidationReportResponse>> {
        self.memory.get_memory_validation_report(record_id).await
    }

    pub(crate) async fn get_memory_conflicts(
        &self,
        record_id: &MemoryRecordId,
    ) -> Result<Option<MemoryConflictResponse>> {
        self.memory.get_memory_conflicts(record_id).await
    }

    pub(crate) async fn build_frozen_snapshot(&self) -> Result<MemoryRetrievalPacket> {
        self.memory.build_frozen_snapshot().await
    }

    pub(crate) async fn build_prefetch_packet(
        &self,
        query: &MemoryRetrievalQuery,
    ) -> Result<MemoryRetrievalPacket> {
        self.memory.build_prefetch_packet(query).await
    }

    pub(crate) async fn record_prefetch_usage(
        &self,
        session_id: &str,
        packet: &MemoryRetrievalPacket,
    ) -> Result<()> {
        self.memory.record_prefetch_usage(session_id, packet).await
    }

    pub(crate) async fn ingest_session_record(&self, session: &Session) -> Result<()> {
        let _ = self.memory.ingest_session_record(session).await?;
        Ok(())
    }

    pub(crate) async fn ingest_stage_summaries(
        &self,
        session_id: &str,
        summaries: &[StageSummary],
    ) -> Result<()> {
        for summary in summaries {
            let _ = self
                .memory
                .ingest_stage_summary_observation(session_id, summary)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn ingest_tool_result(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        stage_id: Option<&str>,
        output: &str,
        is_error: bool,
    ) -> Result<()> {
        let _ = self
            .memory
            .ingest_tool_result_observation(&ToolMemoryObservation {
                session_id,
                tool_call_id,
                tool_name,
                stage_id,
                output,
                is_error,
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn ingest_skill_manage_result(
        &self,
        session_id: &str,
        tool_call_id: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<()> {
        let Some(metadata) = metadata else {
            return Ok(());
        };
        let Some(skill_name) = metadata.get("name").and_then(|value| value.as_str()) else {
            return Ok(());
        };
        let action = metadata
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("update");
        let location = metadata.get("location").and_then(|value| value.as_str());
        let supporting_file = metadata.get("file_path").and_then(|value| value.as_str());
        let guard_report = metadata
            .get("guard_report")
            .cloned()
            .and_then(|value| serde_json::from_value::<SkillGuardReport>(value).ok());

        let _ = self
            .memory
            .ingest_skill_write_observation(&SkillWriteObservation {
                session_id,
                tool_call_id: Some(tool_call_id),
                skill_name,
                action,
                location,
                supporting_file,
                guard_report: guard_report.as_ref(),
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn ingest_runtime_loaded_skills(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        stage_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
        output: &str,
        is_error: bool,
    ) -> Result<()> {
        let Some(metadata) = metadata else {
            return Ok(());
        };

        let loaded_skills = extract_loaded_skill_names(metadata);
        if loaded_skills.is_empty() {
            return Ok(());
        }

        let category = metadata
            .get("category")
            .and_then(|value| value.as_str())
            .or_else(|| {
                metadata
                    .get("task")
                    .and_then(|value| value.get("category"))
                    .and_then(|value| value.as_str())
            });

        for skill_name in loaded_skills {
            let _ = self
                .memory
                .ingest_skill_usage_observation(&SkillUsageObservation {
                    session_id,
                    tool_call_id,
                    tool_name,
                    stage_id,
                    skill_name: &skill_name,
                    category,
                    output,
                    is_error,
                })
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn ingest_skill_write_observation(
        &self,
        observation: &SkillWriteObservation<'_>,
    ) -> Result<()> {
        let _ = self
            .memory
            .ingest_skill_write_observation(observation)
            .await?;
        Ok(())
    }

    pub(crate) async fn build_session_memory_insight(
        &self,
        session: &Session,
    ) -> Result<Option<SessionMemoryInsight>> {
        self.memory.build_session_memory_insight(session).await
    }

    pub(crate) async fn build_session_memory_telemetry(
        &self,
        session: &Session,
    ) -> Result<Option<SessionMemoryTelemetrySummary>> {
        self.memory.build_session_memory_telemetry(session).await
    }
}

fn extract_loaded_skill_names(metadata: &serde_json::Value) -> Vec<String> {
    metadata
        .get("loadedSkills")
        .or_else(|| metadata.get("load_skills"))
        .or_else(|| {
            metadata
                .get("task")
                .and_then(|value| value.get("loadedSkills"))
        })
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocode_config::ConfigStore;
    use rocode_runtime_context::ResolvedWorkspaceContextAuthority;
    use rocode_state::UserStateAuthority;
    use rocode_storage::{Database, MemoryRepository};
    use std::sync::Arc;
    use tempfile::tempdir;

    async fn runtime_memory_for(dir: &std::path::Path) -> RuntimeMemoryAuthority {
        let config_store =
            Arc::new(ConfigStore::from_project_dir(dir).expect("project config store should load"));
        let user_state = Arc::new(UserStateAuthority::from_config_store(&config_store));
        let resolved_context_authority = Arc::new(ResolvedWorkspaceContextAuthority::new(
            config_store,
            user_state.clone(),
        ));
        let db = Database::in_memory().await.expect("db should initialize");
        let repository = Arc::new(MemoryRepository::new(db.pool().clone()));
        RuntimeMemoryAuthority::new(Arc::new(
            MemoryAuthority::new(user_state, resolved_context_authority)
                .with_repository(repository),
        ))
    }

    #[test]
    fn extract_loaded_skill_names_reads_top_level_and_nested_metadata() {
        let top_level = serde_json::json!({
            "loadedSkills": ["frontend-ui-ux", "debug"]
        });
        assert_eq!(
            extract_loaded_skill_names(&top_level),
            vec!["frontend-ui-ux".to_string(), "debug".to_string()]
        );

        let nested = serde_json::json!({
            "task": {
                "loadedSkills": ["review-pr"]
            }
        });
        assert_eq!(
            extract_loaded_skill_names(&nested),
            vec!["review-pr".to_string()]
        );
    }

    #[tokio::test]
    async fn ingest_runtime_loaded_skills_persists_linked_skill_usage_records() {
        let dir = tempdir().expect("tempdir");
        let runtime_memory = runtime_memory_for(dir.path()).await;

        runtime_memory
            .ingest_runtime_loaded_skills(
                "ses_runtime_skill",
                "call_runtime_skill",
                "task",
                Some("stage_exec"),
                Some(&serde_json::json!({
                    "loadedSkills": ["frontend-ui-ux"],
                    "category": "frontend"
                })),
                "Subtask completed with delegated frontend workflow.",
                false,
            )
            .await
            .expect("runtime loaded skills should ingest");

        let records = runtime_memory
            .list_memory(None)
            .await
            .expect("memory list should succeed");
        assert!(records.iter().any(|record| {
            record.title.contains("frontend-ui-ux") && record.summary.contains("completed")
        }));
    }
}
