use std::sync::Arc;

use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use crate::runtime_control::{
    build_session_execution_topology, ExecutionKind, ExecutionRecord, QuestionInfo, QuestionReply,
    RuntimeControlRegistry, SessionExecutionTopology, SessionRunStatus, TopologyChangeContext,
};
use crate::session_runtime::events::{DiffEntry, EventBusTelemetry, QuestionResolutionKind, ServerEvent};
use crate::session_runtime::stage_summary::StageSummaryStore;
use crate::session_runtime::state::{
    InterruptTarget, RuntimeProtocolUpdate, RuntimeStateStore, SessionRuntimeState,
};
use crate::stage_event_log::{EventFilter, StageEventLog};
use rocode_command::stage_protocol::{telemetry_event_names, EventScope, StageEvent, StageSummary};
use rocode_plugin::{HookContext, HookEvent};
use rocode_session::{
    SessionMessage, SessionTelemetrySnapshot, SessionTelemetrySnapshotVersion, SessionUsage,
};
use rocode_types::{SessionMemoryTelemetrySummary, SessionToolRepairTelemetrySummary};
use rocode_types::{ControlInputKind, ControlInputPhase};

pub(crate) struct RuntimeTelemetryAuthority {
    event_bus: broadcast::Sender<String>,
    event_bus_telemetry: Option<Arc<EventBusTelemetry>>,
    runtime_state: Arc<RuntimeStateStore>,
    runtime_control: Arc<RuntimeControlRegistry>,
    stage_event_log: Arc<StageEventLog>,
    stage_summaries: Arc<StageSummaryStore>,
}

impl RuntimeTelemetryAuthority {
    pub(crate) fn new(
        event_bus: broadcast::Sender<String>,
        event_bus_telemetry: Option<Arc<EventBusTelemetry>>,
    ) -> Self {
        let runtime_state = Arc::new(RuntimeStateStore::new());
        let stage_event_log = Arc::new(StageEventLog::new());
        let stage_summaries = Arc::new(StageSummaryStore::new());
        let callback_event_bus = event_bus.clone();
        let callback_telemetry = event_bus_telemetry.clone();
        let callback_stage_event_log = stage_event_log.clone();
        let runtime_control = Arc::new(RuntimeControlRegistry::with_topology_callback(Arc::new(
            move |ctx: &TopologyChangeContext| {
                let log = callback_stage_event_log.clone();
                let event_bus = callback_event_bus.clone();
                let telemetry = callback_telemetry.clone();
                let session_id = ctx.session_id.clone();
                let event = Self::topology_changed_stage_event(ctx);
                tokio::spawn(async move {
                    Self::record_transportable_stage_event(
                        log, &event_bus, telemetry.as_deref(), &session_id, event,
                    )
                    .await;
                });
            },
        )));

        Self {
            event_bus,
            event_bus_telemetry,
            runtime_state,
            runtime_control,
            stage_event_log,
            stage_summaries,
        }
    }

    pub(crate) fn runtime_control(&self) -> Arc<RuntimeControlRegistry> {
        self.runtime_control.clone()
    }

    pub(crate) fn runtime_state(&self) -> Arc<RuntimeStateStore> {
        self.runtime_state.clone()
    }

    pub(crate) async fn set_session_run_status(&self, session_id: &str, status: SessionRunStatus) {
        self.runtime_control
            .set_session_run_status(session_id, status.clone())
            .await;
        match &status {
            SessionRunStatus::Busy => {
                self.runtime_state.mark_running(session_id, None).await;
            }
            SessionRunStatus::Compacting => {
                self.runtime_state.mark_compacting(session_id).await;
            }
            SessionRunStatus::Idle => {
                self.runtime_state.mark_idle(session_id).await;
            }
            SessionRunStatus::Retry { .. } => {
                self.runtime_state.mark_running(session_id, None).await;
            }
        }
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::SESSION_STATUS,
                serde_json::json!({
                    "sessionID": session_id,
                    "status": status,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn session_run_statuses(
        &self,
    ) -> std::collections::HashMap<String, SessionRunStatus> {
        self.runtime_control.session_run_statuses().await
    }

    pub(crate) async fn has_prompt_run(&self, session_id: &str) -> bool {
        self.runtime_control.has_prompt_run(session_id).await
    }

    pub(crate) async fn request_scheduler_cancel(&self, session_id: &str) -> bool {
        self.runtime_control
            .request_scheduler_cancel(session_id)
            .await
    }

    pub(crate) async fn register_scheduler_run(
        &self,
        session_id: &str,
        token: CancellationToken,
        label: Option<String>,
    ) {
        self.runtime_control
            .register_scheduler_run(session_id, token, label)
            .await;
    }

    pub(crate) async fn finish_scheduler_run(&self, session_id: &str) {
        self.runtime_control.finish_scheduler_run(session_id).await;
    }

    pub(crate) async fn register_scheduler_stage(
        &self,
        session_id: &str,
        execution_id: String,
        label: String,
        metadata: serde_json::Value,
    ) {
        self.runtime_control
            .register_scheduler_stage(session_id, execution_id.clone(), label, metadata)
            .await;
        self.runtime_state
            .scheduler_stage_started(session_id, &execution_id)
            .await;
    }

    pub(crate) async fn mark_scheduler_stage_cancelling(&self, execution_id: &str) {
        self.runtime_control
            .mark_scheduler_stage_cancelling(execution_id)
            .await;
    }

    pub(crate) async fn finish_scheduler_stage(&self, execution_id: &str) {
        let stage_id = self.runtime_control.resolve_stage_id(execution_id).await;
        let session_id = self
            .runtime_control
            .list_all_executions()
            .await
            .into_iter()
            .find(|record| record.id == execution_id)
            .map(|record| record.session_id);
        self.runtime_control
            .finish_scheduler_stage(execution_id)
            .await;
        if let Some(session_id) = session_id {
            self.runtime_state
                .scheduler_stage_finished(&session_id, stage_id.as_deref())
                .await;
        }
    }

    pub(crate) async fn register_agent_task(
        &self,
        task_id: &str,
        session_id: &str,
        agent_name: &str,
        parent_id: Option<String>,
        stage_id: Option<String>,
    ) {
        self.runtime_control
            .register_agent_task(task_id, session_id, agent_name, parent_id, stage_id)
            .await;
    }

    pub(crate) async fn register_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        parent_id: Option<String>,
        stage_id: Option<String>,
    ) {
        self.runtime_control
            .register_tool_call(
                tool_call_id,
                session_id,
                tool_name,
                parent_id,
                stage_id.clone(),
            )
            .await;
        self.runtime_state
            .tool_started(session_id, tool_call_id, tool_name)
            .await;
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Stage,
                stage_id,
                Some(RuntimeControlRegistry::tool_call_execution_id(tool_call_id)),
                telemetry_event_names::TOOL_STARTED,
                serde_json::json!({
                    "sessionID": session_id,
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn finish_tool_call(&self, session_id: &str, tool_call_id: &str) {
        let execution_id = RuntimeControlRegistry::tool_call_execution_id(tool_call_id);
        let stage_id = self.runtime_control.resolve_stage_id(&execution_id).await;
        let tool_name = self
            .runtime_control
            .list_session_execution_records(session_id)
            .await
            .into_iter()
            .find(|record| record.id == execution_id)
            .and_then(|record| {
                record
                    .metadata
                    .as_ref()
                    .and_then(|value| value.get("tool_name"))
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
            });
        self.runtime_control.finish_tool_call(tool_call_id).await;
        self.runtime_state
            .tool_ended(session_id, tool_call_id)
            .await;
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Stage,
                stage_id,
                Some(execution_id),
                telemetry_event_names::TOOL_COMPLETED,
                serde_json::json!({
                    "sessionID": session_id,
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn register_question(
        &self,
        session_id: String,
        questions: Vec<rocode_tool::QuestionDef>,
    ) -> (QuestionInfo, oneshot::Receiver<QuestionReply>) {
        let questions_value =
            serde_json::to_value(&questions).unwrap_or_else(|_| serde_json::Value::Array(vec![]));
        let (info, rx) = self
            .runtime_control
            .register_question(session_id.clone(), questions)
            .await;
        let stage_id = self.runtime_control.resolve_stage_id(&info.id).await;
        self.runtime_state
            .question_created(&session_id, &info.id, questions_value)
            .await;
        self.record_stage_event(
            &session_id,
            StageEvent::new(
                EventScope::Stage,
                stage_id,
                Some(info.id.clone()),
                telemetry_event_names::QUESTION_CREATED,
                serde_json::json!({
                    "sessionID": session_id,
                    "requestID": info.id,
                    "questions": serde_json::to_value(&info.items)
                        .unwrap_or_else(|_| serde_json::Value::Array(vec![])),
                }),
            ),
        )
        .await;
        (info, rx)
    }

    pub(crate) async fn answer_question(
        &self,
        id: &str,
        answers: Vec<Vec<String>>,
    ) -> Option<QuestionInfo> {
        let stage_id = self.runtime_control.resolve_stage_id(id).await;
        let info = self
            .runtime_control
            .answer_question(id, answers.clone())
            .await?;
        self.runtime_state.question_resolved(&info.session_id).await;
        self.record_stage_event(
            &info.session_id,
            StageEvent::new(
                EventScope::Stage,
                stage_id,
                Some(id.to_string()),
                telemetry_event_names::QUESTION_RESOLVED,
                serde_json::json!({
                    "sessionID": info.session_id,
                    "requestID": id,
                    "resolution": QuestionResolutionKind::Answered,
                    "answers": serde_json::to_value(&answers).unwrap_or(serde_json::Value::Null),
                }),
            ),
        )
        .await;
        Some(info)
    }

    pub(crate) async fn reject_question(&self, id: &str) -> Option<QuestionInfo> {
        let stage_id = self.runtime_control.resolve_stage_id(id).await;
        let info = self.runtime_control.reject_question(id).await?;
        self.runtime_state.question_resolved(&info.session_id).await;
        self.record_stage_event(
            &info.session_id,
            StageEvent::new(
                EventScope::Stage,
                stage_id,
                Some(id.to_string()),
                telemetry_event_names::QUESTION_RESOLVED,
                serde_json::json!({
                    "sessionID": info.session_id,
                    "requestID": id,
                    "resolution": QuestionResolutionKind::Rejected,
                }),
            ),
        )
        .await;
        Some(info)
    }

    pub(crate) async fn cancel_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
        let cancelled = self
            .runtime_control
            .cancel_questions_for_session(session_id)
            .await;
        if !cancelled.is_empty() {
            self.runtime_state.question_resolved(session_id).await;
        }
        for question in &cancelled {
            self.record_stage_event(
                session_id,
                StageEvent::new(
                    EventScope::Stage,
                    self.runtime_control.resolve_stage_id(&question.id).await,
                    Some(question.id.clone()),
                    telemetry_event_names::QUESTION_RESOLVED,
                    serde_json::json!({
                        "sessionID": question.session_id,
                        "requestID": question.id,
                        "resolution": QuestionResolutionKind::Cancelled,
                        "reason": "cancelled",
                    }),
                ),
            )
            .await;
        }
        cancelled
    }

    pub(crate) async fn drop_question(&self, session_id: &str, question_id: &str) {
        self.runtime_control.drop_question(question_id).await;
        self.runtime_state.question_resolved(session_id).await;
    }

    pub(crate) async fn list_questions(&self) -> Vec<QuestionInfo> {
        self.runtime_control.list_questions().await
    }

    pub(crate) async fn list_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
        self.runtime_control
            .list_questions_for_session(session_id)
            .await
    }

    pub(crate) async fn permission_requested(
        &self,
        session_id: &str,
        permission_id: &str,
        info: serde_json::Value,
    ) {
        let requested_at = chrono::Utc::now().timestamp_millis();
        let tool = info
            .get("tool")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        self.runtime_state
            .permission_requested(session_id, permission_id, requested_at, tool)
            .await;
        self.emit_control_input_transition(
            session_id,
            ControlInputKind::Permission,
            ControlInputPhase::Queued,
            requested_at,
        )
        .await;
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::PERMISSION_REQUESTED,
                serde_json::json!({
                    "sessionID": session_id,
                    "permissionID": permission_id,
                    "info": info,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn permission_resolved(
        &self,
        session_id: &str,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) {
        self.runtime_state.permission_resolved(session_id).await;
        let now = chrono::Utc::now().timestamp_millis();
        self.emit_control_input_transition(
            session_id,
            ControlInputKind::Permission,
            ControlInputPhase::Consumed,
            now,
        )
        .await;
        self.emit_control_input_transition(
            session_id,
            ControlInputKind::Permission,
            ControlInputPhase::Cleared,
            now,
        )
        .await;
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::PERMISSION_RESOLVED,
                serde_json::json!({
                    "sessionID": session_id,
                    "permissionID": permission_id,
                    "reply": reply,
                    "message": message,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn clear_permission_pending(&self, session_id: &str) {
        self.runtime_state.permission_resolved(session_id).await;
        self.emit_control_input_transition(
            session_id,
            ControlInputKind::Permission,
            ControlInputPhase::Cleared,
            chrono::Utc::now().timestamp_millis(),
        )
        .await;
    }

    pub(crate) async fn attached_session_registered(
        &self,
        parent_id: &str,
        attached_id: &str,
        context_kind: rocode_types::SessionContextKind,
    ) {
        self.runtime_state
            .attached_session_registered(parent_id, attached_id, context_kind)
            .await;
        self.record_stage_event(
            parent_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::ATTACHED_SESSION_ATTACHED,
                serde_json::json!({
                    "parentID": parent_id,
                    "attachedID": attached_id,
                    "sessionContextKind": context_kind,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn attached_session_unregistered(&self, parent_id: &str, attached_id: &str) {
        self.runtime_state
            .attached_session_unregistered(parent_id, attached_id)
            .await;
        self.record_stage_event(
            parent_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::ATTACHED_SESSION_DETACHED,
                serde_json::json!({
                    "parentID": parent_id,
                    "attachedID": attached_id,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn diff_updated(&self, session_id: &str, diff: Vec<DiffEntry>) {
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::DIFF_UPDATED,
                serde_json::json!({
                    "sessionID": session_id,
                    "diff": diff,
                }),
            ),
        )
        .await;
    }

    pub(crate) async fn record_session_updated(&self, session_id: &str, source: &str) {
        self.record_stage_event(
            session_id,
            StageEvent::new(
                EventScope::Session,
                None,
                None,
                telemetry_event_names::SESSION_UPDATED,
                serde_json::json!({
                    "sessionID": session_id,
                    "source": source,
                }),
            ),
        )
        .await;
    }

    /// Update runtime state when a steering message is enqueued (Constitution §8).
    pub(crate) async fn steering_enqueued(
        &self,
        owner_session_id: &str,
        summary: crate::session_runtime::state::PendingSteeringMessageSummary,
    ) {
        self.runtime_state
            .steering_enqueued(owner_session_id, summary)
            .await;
        self.emit_control_input_transition(
            owner_session_id,
            ControlInputKind::Steering,
            ControlInputPhase::Queued,
            chrono::Utc::now().timestamp_millis(),
        )
        .await;
    }

    pub(crate) async fn interrupt_requested(
        &self,
        session_id: &str,
        target: InterruptTarget,
    ) {
        let now = chrono::Utc::now().timestamp_millis();
        self.runtime_state
            .interrupt_requested(session_id, now, target)
            .await;
        self.emit_control_input_transition(
            session_id,
            ControlInputKind::Interrupt,
            ControlInputPhase::Queued,
            now,
        )
        .await;
    }

    pub(crate) async fn record_session_usage(
        &self,
        session_id: &str,
        message_id: Option<&str>,
        usage: SessionUsage,
    ) -> Option<ServerEvent> {
        self.runtime_state
            .set_usage(session_id, usage.clone())
            .await;
        let event = StageEvent::new(
            EventScope::Session,
            None,
            None,
            telemetry_event_names::SESSION_USAGE,
            serde_json::json!({
                "sessionID": session_id,
                "message_id": message_id,
                "prompt_tokens": usage.input_tokens,
                "completion_tokens": usage.output_tokens,
                "reasoning_tokens": usage.reasoning_tokens,
                "cache_read_tokens": usage.cache_read_tokens,
                "cache_miss_tokens": usage.cache_miss_tokens,
                "cache_write_tokens": usage.cache_write_tokens,
                "context_tokens": usage.context_tokens,
                "total_cost": usage.total_cost,
            }),
        );
        let transport = ServerEvent::from_stage_event(&event);
        self.record_stage_event(session_id, event).await;
        transport
    }

    pub(crate) async fn record_session_error(
        &self,
        session_id: &str,
        message_id: Option<&str>,
        done: Option<bool>,
        error: &str,
    ) -> Option<ServerEvent> {
        let event = StageEvent::new(
            EventScope::Session,
            None,
            None,
            telemetry_event_names::SESSION_ERROR,
            serde_json::json!({
                "sessionID": session_id,
                "message_id": message_id,
                "done": done,
                "error": error,
            }),
        );
        let transport = ServerEvent::from_stage_event(&event);
        self.record_stage_event(session_id, event).await;
        transport
    }

    pub(crate) async fn clear_session_runtime(&self, session_id: &str) {
        self.runtime_state.remove(session_id).await;
        self.stage_event_log.clear_session(session_id).await;
        self.stage_summaries.clear_session(session_id).await;
    }

    pub(crate) async fn get_runtime_snapshot(
        &self,
        session_id: &str,
    ) -> Option<SessionRuntimeState> {
        self.runtime_state.get(session_id).await
    }

    pub(crate) async fn list_session_execution_records(
        &self,
        session_id: &str,
    ) -> Vec<ExecutionRecord> {
        self.runtime_control
            .list_session_execution_records(session_id)
            .await
    }

    pub(crate) async fn build_session_execution_topology(
        &self,
        session_id: String,
        extra_records: Vec<ExecutionRecord>,
    ) -> SessionExecutionTopology {
        let mut records = self.list_session_execution_records(&session_id).await;
        records.extend(extra_records);
        build_session_execution_topology(session_id, records)
    }

    pub(crate) async fn list_all_executions(&self) -> Vec<ExecutionRecord> {
        self.runtime_control.list_all_executions().await
    }

    pub(crate) async fn list_active_session_ids(&self) -> Vec<String> {
        self.runtime_control.list_active_session_ids().await
    }

    pub(crate) async fn cancel_execution(&self, execution_id: &str) -> Option<ExecutionKind> {
        self.runtime_control.cancel_execution(execution_id).await
    }

    pub(crate) async fn resolve_stage_id(&self, execution_id: &str) -> Option<String> {
        self.runtime_control.resolve_stage_id(execution_id).await
    }

    pub(crate) async fn finish_agent_task(&self, task_id: &str) {
        self.runtime_control.finish_agent_task(task_id).await;
    }

    pub(crate) async fn count_stage_agents(&self, stage_id: &str) -> (u32, u32) {
        self.runtime_control.count_stage_agents(stage_id).await
    }

    pub(crate) async fn query_stage_events(
        &self,
        session_id: &str,
        filter: &EventFilter,
    ) -> Vec<rocode_command::stage_protocol::StageEvent> {
        self.stage_event_log.query(session_id, filter).await
    }

    pub(crate) async fn list_stage_ids(&self, session_id: &str) -> Vec<String> {
        self.stage_event_log.stage_ids(session_id).await
    }

    pub(crate) async fn refresh_stage_summary_from_message(
        &self,
        session_id: &str,
        message: &SessionMessage,
    ) -> Option<StageSummary> {
        let block = super::scheduler_stage_block_from_message(message)?;
        let mut summary = block.to_summary();
        let stage_id = summary.stage_id.clone();
        if stage_id.is_empty() {
            return None;
        }

        let (done_agents, total_agents) = self.runtime_control.count_stage_agents(&stage_id).await;
        if total_agents > 0 {
            summary.active_agent_count = total_agents.saturating_sub(done_agents);
        }
        summary.active_tool_count = self
            .runtime_control
            .count_active_stage_tools(&stage_id)
            .await;

        let changed = self
            .stage_summaries
            .upsert(session_id, summary.clone())
            .await;
        if changed {
            self.emit_stage_summary_updated_hook(session_id, &summary)
                .await;
        }
        Some(summary)
    }

    pub(crate) async fn list_stage_summaries(&self, session_id: &str) -> Vec<StageSummary> {
        self.stage_summaries.list_for_session(session_id).await
    }

    pub(crate) async fn build_persisted_snapshot(
        &self,
        session_id: &str,
        usage: SessionUsage,
        last_run_status: impl Into<String>,
        memory: Option<SessionMemoryTelemetrySummary>,
        tool_repair_summary: Option<SessionToolRepairTelemetrySummary>,
    ) -> Option<SessionTelemetrySnapshot> {
        let has_runtime = self.runtime_state.get(session_id).await.is_some();
        let stage_summaries = self.stage_summaries.list_for_session(session_id).await;
        let usage_is_empty = usage.input_tokens == 0
            && usage.output_tokens == 0
            && usage.reasoning_tokens == 0
            && usage.cache_write_tokens == 0
            && usage.cache_read_tokens == 0
            && usage.cache_miss_tokens == 0
            && usage.total_cost == 0.0;
        if !has_runtime && stage_summaries.is_empty() && usage_is_empty {
            return None;
        }

        Some(SessionTelemetrySnapshot {
            version: SessionTelemetrySnapshotVersion::V6,
            usage,
            stage_summaries: stage_summaries.into_iter().map(Into::into).collect(),
            tool_repair_summary,
            memory,
            compaction_continuity: None,
            repair_query_snapshot: None,
            tool_trajectory_quality: None,
            tool_result_governance: None,
            pending_permission_count: 0,
            pending_followup_count: 0,
            granted_by_turn_count: 0,
            granted_by_session_count: 0,
            granted_by_matcher_kind: std::collections::BTreeMap::new(),
            last_permission_matcher_kind: None,
            last_permission_grant_target: None,
            last_permission_miss_count: 0,
            pending_steering_count: 0,
            consumed_steering_count: 0,
            last_steering_injected_at: None,
            last_steering_source_session_id: None,
            last_steering_latency_ms: None,
            last_permission_pending_ms: None,
            last_run_status: last_run_status.into(),
            updated_at: chrono::Utc::now().timestamp_millis(),
        })
    }

    pub(crate) async fn emit_telemetry_snapshot_updated_hook(
        &self,
        session_id: &str,
        snapshot: &SessionTelemetrySnapshot,
    ) {
        let Ok(snapshot) = serde_json::to_value(snapshot) else {
            tracing::warn!(
                session_id,
                "failed to serialize telemetry snapshot for plugin hook"
            );
            return;
        };

        rocode_plugin::trigger(
            HookContext::new(HookEvent::TelemetrySnapshotUpdated)
                .with_session(session_id)
                .with_data("sessionID", serde_json::json!(session_id))
                .with_data("snapshot", snapshot),
        )
        .await;
    }

    async fn record_stage_event(&self, session_id: &str, event: StageEvent) {
        Self::record_transportable_stage_event(
            self.stage_event_log.clone(),
            &self.event_bus,
            self.event_bus_telemetry.as_deref(),
            session_id,
            event,
        )
        .await;
    }

    async fn record_transportable_stage_event(
        stage_event_log: Arc<StageEventLog>,
        event_bus: &broadcast::Sender<String>,
        event_bus_telemetry: Option<&EventBusTelemetry>,
        session_id: &str,
        event: StageEvent,
    ) {
        if let Some(transport) = ServerEvent::from_stage_event(&event) {
            Self::broadcast_server_event_payload(event_bus, event_bus_telemetry, &transport);
        }
        stage_event_log.record(session_id, event).await;
    }

    fn broadcast_server_event_payload(
        event_bus: &broadcast::Sender<String>,
        event_bus_telemetry: Option<&EventBusTelemetry>,
        event: &ServerEvent,
    ) {
        if let Some(payload) = event.to_json_string() {
            let receiver_count = event_bus.receiver_count();
            if event_bus.send(payload).is_err() {
                tracing::warn!("failed to broadcast runtime telemetry event (no active receivers)");
                if let Some(telemetry) = event_bus_telemetry {
                    telemetry.record_send_error();
                }
            } else if let Some(telemetry) = event_bus_telemetry {
                telemetry.record_send(receiver_count);
            }
        }
    }

    pub(crate) async fn emit_control_input_transition(
        &self,
        session_id: &str,
        kind: ControlInputKind,
        phase: ControlInputPhase,
        at: i64,
    ) {
        self.runtime_state
            .apply_protocol_update(
                session_id,
                RuntimeProtocolUpdate::ControlInputTransition { kind, phase, at },
            )
            .await;
        let event = ServerEvent::ControlInputTransition {
            session_id: session_id.to_string(),
            kind,
            phase,
            at,
        };
        Self::broadcast_server_event_payload(
            &self.event_bus,
            self.event_bus_telemetry.as_deref(),
            &event,
        );
    }

    fn topology_changed_stage_event(ctx: &TopologyChangeContext) -> StageEvent {
        StageEvent {
            event_id: format!("evt_{}", uuid::Uuid::new_v4().simple()),
            scope: EventScope::Stage,
            stage_id: ctx.stage_id.clone(),
            execution_id: Some(ctx.execution_id.clone()),
            event_type: telemetry_event_names::EXECUTION_TOPOLOGY_CHANGED.to_string(),
            ts: chrono::Utc::now().timestamp_millis(),
            payload: serde_json::json!({
                "sessionID": ctx.session_id,
                "executionID": ctx.execution_id,
                "stageID": ctx.stage_id,
            }),
        }
    }

    async fn emit_stage_summary_updated_hook(&self, session_id: &str, summary: &StageSummary) {
        let Ok(summary) = serde_json::to_value(summary) else {
            tracing::warn!(
                session_id,
                "failed to serialize stage summary for plugin hook"
            );
            return;
        };

        rocode_plugin::trigger(
            HookContext::new(HookEvent::StageSummaryUpdated)
                .with_session(session_id)
                .with_data("sessionID", serde_json::json!(session_id))
                .with_data("summary", summary),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_runtime::{emit_scheduler_stage_message, SchedulerStageMessageInput};
    use crate::ServerState;
    use rocode_orchestrator::ExecutionContext;
    use rocode_plugin::{global, Hook, HookContext};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    async fn recv_stage_summary_hook_for_session(
        rx: &mut mpsc::UnboundedReceiver<HookContext>,
        session_id: &str,
        wait: Duration,
    ) -> Option<HookContext> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let remaining = deadline.saturating_duration_since(now);
            match timeout(remaining, rx.recv()).await {
                Ok(Some(ctx)) if ctx.session_id.as_deref() == Some(session_id) => return Some(ctx),
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => return None,
            }
        }
    }

    #[tokio::test]
    async fn refresh_stage_summary_emits_hook_from_authority_summary() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            let session = sessions.create("project", "/tmp/project");
            session.id.clone()
        };

        let hook_name = format!("stage-summary-updated-{}", uuid::Uuid::new_v4().simple());
        let (tx, mut rx) = mpsc::unbounded_channel();
        global()
            .register(Hook::new(
                &hook_name,
                HookEvent::StageSummaryUpdated,
                move |ctx| {
                    let tx = tx.clone();
                    async move {
                        let _ = tx.send(ctx);
                        Ok(())
                    }
                },
            ))
            .await;

        let exec_ctx = ExecutionContext {
            session_id: session_id.clone(),
            workdir: "/tmp/project".to_string(),
            agent_name: "test-agent".to_string(),
            metadata: HashMap::new(),
        };

        emit_scheduler_stage_message(SchedulerStageMessageInput {
            state: &state,
            session_id: &session_id,
            scheduler_profile: "prometheus",
            stage_name: "plan",
            stage_index: 1,
            stage_total: 2,
            content: "## Plan\n\n- inspect scheduler",
            exec_ctx: &exec_ctx,
            output_hook: None,
        })
        .await;

        let hook_ctx =
            recv_stage_summary_hook_for_session(&mut rx, &session_id, Duration::from_secs(1))
                .await
                .expect("hook payload should arrive");
        assert_eq!(hook_ctx.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(
            hook_ctx.get("sessionID"),
            Some(&serde_json::json!(session_id))
        );
        assert_eq!(
            hook_ctx.get("summary").and_then(|v| v.get("stage_name")),
            Some(&serde_json::json!("plan"))
        );
        assert!(hook_ctx
            .get("summary")
            .and_then(|value| value.get("stage_id"))
            .and_then(|value| value.as_str())
            .is_some());

        let message_snapshot = {
            let sessions = state.sessions.lock().await;
            let session = sessions.get(&session_id).expect("session should exist");
            session
                .messages
                .last()
                .cloned()
                .expect("stage message should exist")
        };

        let _ = state
            .runtime_telemetry
            .refresh_stage_summary_from_message(&session_id, &message_snapshot)
            .await;
        assert!(recv_stage_summary_hook_for_session(
            &mut rx,
            &session_id,
            Duration::from_millis(200)
        )
        .await
        .is_none());

        let _ = global()
            .remove(&HookEvent::StageSummaryUpdated, &hook_name)
            .await;
    }
}
