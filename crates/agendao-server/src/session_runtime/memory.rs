use std::sync::Arc;

use agendao_memory::{MemoryAuthority, SkillWriteObservation};
use agendao_types::{
    MemoryConflictResponse, MemoryConsolidationRequest, MemoryConsolidationResponse,
    MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery, MemoryDetailView,
    MemoryListQuery, MemoryListResponse, MemoryRecordId, MemoryRetrievalPacket,
    MemoryRetrievalPreviewResponse, MemoryRetrievalQuery, MemoryRuleHitListResponse,
    MemoryRuleHitQuery, MemoryRulePackListResponse, MemoryValidationReportResponse, Session,
    SessionMemoryInsight, SessionMemoryTelemetrySummary,
};
use anyhow::Result;

#[derive(Clone)]
pub(crate) struct RuntimeMemoryAuthority {
    memory: Arc<MemoryAuthority>,
}

impl RuntimeMemoryAuthority {
    pub(crate) fn new(memory: Arc<MemoryAuthority>) -> Self {
        Self { memory }
    }

    pub(crate) fn memory_authority(&self) -> Arc<MemoryAuthority> {
        self.memory.clone()
    }

    #[cfg(test)]
    pub(crate) async fn list_memory(
        &self,
        filter: Option<&agendao_memory::MemoryFilter<'_>>,
    ) -> Result<Vec<agendao_types::MemoryCardView>> {
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
