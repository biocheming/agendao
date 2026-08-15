use std::sync::Arc;

use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use agendao_plugin::{HookContext, HookEvent};
use agendao_server_core::runtime_control::{
    build_session_execution_topology, ExecutionKind, ExecutionPatch, ExecutionRecord, QuestionInfo,
    QuestionReply, RuntimeControlRegistry, SessionExecutionTopology, SessionRunStatus,
    TopologyChangeContext,
};
use agendao_server_core::runtime_events::{
    EventBusTelemetry, QuestionResolutionKind, ServerBusEvent, ServerEvent,
};
use agendao_server_core::runtime_state::{
    InterruptTarget, RuntimeProtocolUpdate, RuntimeStateStore, SessionRuntimeState,
};
use agendao_session::{SessionTelemetrySnapshot, SessionTelemetrySnapshotVersion, SessionUsage};
use agendao_types::{ControlInputKind, ControlInputPhase};
use agendao_types::{SessionMemoryTelemetrySummary, SessionToolRepairTelemetrySummary};

pub(crate) struct RuntimeTelemetryAuthority {
    event_bus: broadcast::Sender<Arc<ServerBusEvent>>,
    event_bus_telemetry: Option<Arc<EventBusTelemetry>>,
    runtime_state: Arc<RuntimeStateStore>,
    runtime_control: Arc<RuntimeControlRegistry>,
}

impl RuntimeTelemetryAuthority {
    pub(crate) fn new(
        event_bus: broadcast::Sender<Arc<ServerBusEvent>>,
        event_bus_telemetry: Option<Arc<EventBusTelemetry>>,
    ) -> Self {
        let runtime_state = Arc::new(RuntimeStateStore::new());
        let callback_event_bus = event_bus.clone();
        let callback_telemetry = event_bus_telemetry.clone();
        let runtime_control = Arc::new(RuntimeControlRegistry::with_topology_callback(Arc::new(
            move |ctx: &TopologyChangeContext| {
                Self::broadcast_server_event_payload(
                    &callback_event_bus,
                    callback_telemetry.as_deref(),
                    ServerEvent::TopologyChanged {
                        session_id: ctx.session_id.clone(),
                        execution_id: Some(ctx.execution_id.clone()),
                        stage_id: ctx.stage_id.clone(),
                    },
                );
            },
        )));

        Self {
            event_bus,
            event_bus_telemetry,
            runtime_state,
            runtime_control,
        }
    }

    pub(crate) fn runtime_control(&self) -> Arc<RuntimeControlRegistry> {
        self.runtime_control.clone()
    }

    pub(crate) fn runtime_state(&self) -> Arc<RuntimeStateStore> {
        self.runtime_state.clone()
    }

    pub(crate) async fn set_session_run_status(&self, session_id: &str, status: SessionRunStatus) {
        let previous = self.runtime_control.session_run_status(session_id).await;
        if previous == status {
            return;
        }

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
            SessionRunStatus::Blocked { reason, recheck_at } => {
                self.runtime_state
                    .mark_blocked(session_id, reason.clone(), *recheck_at)
                    .await;
            }
            SessionRunStatus::Sleeping { reason, wake_at } => {
                self.runtime_state
                    .mark_sleeping(session_id, reason.clone(), *wake_at)
                    .await;
            }
        }
        self.emit(ServerEvent::SessionStatus {
            session_id: session_id.to_string(),
            status: serde_json::to_value(status).unwrap_or(serde_json::Value::Null),
        });
    }

    /// Recheck a blocked session. Returns `Some(Idle)` when the session was
    /// blocked and its `recheck_at` has passed (or is `None`, allowing manual
    /// override). Returns `None` when the session is not blocked or the
    /// recheck time has not arrived.
    ///
    /// This method goes through `set_session_run_status` so that the
    /// `RuntimeStateStore` projection and event bus are updated atomically.
    pub(crate) async fn recheck_session(&self, session_id: &str) -> Option<SessionRunStatus> {
        let current = self.runtime_control.session_run_status(session_id).await;
        match current {
            SessionRunStatus::Blocked { recheck_at, .. } => {
                let now = chrono::Utc::now().timestamp_millis();
                if recheck_at.is_none_or(|ts| now >= ts) {
                    self.set_session_run_status(session_id, SessionRunStatus::Idle)
                        .await;
                    Some(SessionRunStatus::Idle)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Wake a sleeping session. Symmetric to `recheck_session`: returns
    /// `Some(Idle)` when the session was sleeping and its `wake_at` has
    /// passed (or is `None`, allowing manual override).
    pub(crate) async fn wake_session(&self, session_id: &str) -> Option<SessionRunStatus> {
        let current = self.runtime_control.session_run_status(session_id).await;
        match current {
            SessionRunStatus::Sleeping { wake_at, .. } => {
                let now = chrono::Utc::now().timestamp_millis();
                if wake_at.is_none_or(|ts| now >= ts) {
                    self.set_session_run_status(session_id, SessionRunStatus::Idle)
                        .await;
                    Some(SessionRunStatus::Idle)
                } else {
                    None
                }
            }
            _ => None,
        }
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

    pub(crate) async fn update_scheduler_run(&self, session_id: &str, patch: ExecutionPatch) {
        self.runtime_control
            .update_scheduler_run(session_id, patch)
            .await;
    }

    pub(crate) async fn register_scheduler_node(&self, session_id: &str, path: &str) {
        self.runtime_control
            .register_scheduler_node(session_id, path)
            .await;
    }

    pub(crate) async fn update_scheduler_node(
        &self,
        session_id: &str,
        path: &str,
        patch: ExecutionPatch,
    ) {
        self.runtime_control
            .update_scheduler_node(session_id, path, patch)
            .await;
    }

    pub(crate) async fn finish_scheduler_node(&self, session_id: &str, path: &str) {
        self.runtime_control
            .finish_scheduler_node(session_id, path)
            .await;
    }

    pub(crate) async fn register_question(
        &self,
        session_id: String,
        questions: Vec<agendao_tool::QuestionDef>,
    ) -> (QuestionInfo, oneshot::Receiver<QuestionReply>) {
        let questions_value =
            serde_json::to_value(&questions).unwrap_or_else(|_| serde_json::Value::Array(vec![]));
        let (info, rx) = self
            .runtime_control
            .register_question(session_id.clone(), questions)
            .await;
        self.runtime_state
            .question_created(&session_id, &info.id, questions_value)
            .await;
        self.emit(ServerEvent::QuestionCreated {
            session_id,
            request_id: info.id.clone(),
            questions: serde_json::to_value(&info.items)
                .unwrap_or_else(|_| serde_json::Value::Array(vec![])),
        });
        (info, rx)
    }

    pub(crate) async fn answer_question(
        &self,
        id: &str,
        answers: Vec<Vec<String>>,
    ) -> Option<QuestionInfo> {
        let info = self
            .runtime_control
            .answer_question(id, answers.clone())
            .await?;
        self.runtime_state.question_resolved(&info.session_id).await;
        self.finish_control_input_wait(&info.session_id).await;
        self.emit(ServerEvent::QuestionResolved {
            session_id: info.session_id.clone(),
            request_id: id.to_string(),
            resolution: Some(QuestionResolutionKind::Answered),
            answers: Some(serde_json::to_value(&answers).unwrap_or(serde_json::Value::Null)),
            reason: None,
        });
        Some(info)
    }

    pub(crate) async fn reject_question(&self, id: &str) -> Option<QuestionInfo> {
        let info = self.runtime_control.reject_question(id).await?;
        self.runtime_state.question_resolved(&info.session_id).await;
        self.finish_control_input_wait(&info.session_id).await;
        self.emit(ServerEvent::QuestionResolved {
            session_id: info.session_id.clone(),
            request_id: id.to_string(),
            resolution: Some(QuestionResolutionKind::Rejected),
            answers: None,
            reason: None,
        });
        Some(info)
    }

    pub(crate) async fn cancel_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
        let cancelled = self
            .runtime_control
            .cancel_questions_for_session(session_id)
            .await;
        if !cancelled.is_empty() {
            self.runtime_state.question_resolved(session_id).await;
            self.finish_control_input_wait(session_id).await;
        }
        for question in &cancelled {
            self.emit(ServerEvent::QuestionResolved {
                session_id: question.session_id.clone(),
                request_id: question.id.clone(),
                resolution: Some(QuestionResolutionKind::Cancelled),
                answers: None,
                reason: Some("cancelled".to_string()),
            });
        }
        cancelled
    }

    pub(crate) async fn drop_question(&self, session_id: &str, question_id: &str) {
        self.runtime_control.drop_question(question_id).await;
        self.runtime_state.question_resolved(session_id).await;
        self.finish_control_input_wait(session_id).await;
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
        self.emit(ServerEvent::PermissionRequested {
            session_id: session_id.to_string(),
            permission_id: permission_id.to_string(),
            info,
        });
    }

    pub(crate) async fn permission_resolved(
        &self,
        session_id: &str,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) {
        self.runtime_state.permission_resolved(session_id).await;
        self.finish_control_input_wait(session_id).await;
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
        self.emit(ServerEvent::PermissionResolved {
            session_id: session_id.to_string(),
            permission_id: permission_id.to_string(),
            reply: reply.to_string(),
            message,
        });
    }

    pub(crate) async fn clear_permission_pending(&self, session_id: &str) {
        self.runtime_state.permission_resolved(session_id).await;
        self.finish_control_input_wait(session_id).await;
        self.emit_control_input_transition(
            session_id,
            ControlInputKind::Permission,
            ControlInputPhase::Cleared,
            chrono::Utc::now().timestamp_millis(),
        )
        .await;
    }

    async fn finish_control_input_wait(&self, session_id: &str) {
        if !self.runtime_control.has_prompt_run(session_id).await {
            self.runtime_state.mark_idle(session_id).await;
        }
    }

    /// Update runtime state when a steering message is enqueued (Constitution §8).
    pub(crate) async fn steering_enqueued(
        &self,
        owner_session_id: &str,
        summary: agendao_server_core::runtime_state::PendingSteeringMessageSummary,
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

    pub(crate) async fn interrupt_requested(&self, session_id: &str, target: InterruptTarget) {
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
    ) {
        self.runtime_state
            .set_usage(session_id, usage.clone())
            .await;
        self.emit(ServerEvent::Usage {
            session_id: Some(session_id.to_string()),
            prompt_tokens: usage.input_tokens,
            completion_tokens: usage.output_tokens,
            message_id: message_id.map(ToOwned::to_owned),
        });
    }

    pub(crate) async fn clear_session_runtime(&self, session_id: &str) {
        self.runtime_state.remove(session_id).await;
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

    pub(crate) async fn build_persisted_snapshot(
        &self,
        session_id: &str,
        usage: SessionUsage,
        last_run_status: impl Into<String>,
        memory: Option<SessionMemoryTelemetrySummary>,
        tool_repair_summary: Option<SessionToolRepairTelemetrySummary>,
    ) -> Option<SessionTelemetrySnapshot> {
        let has_runtime = self.runtime_state.get(session_id).await.is_some();
        let usage_is_empty = usage.input_tokens == 0
            && usage.output_tokens == 0
            && usage.reasoning_tokens == 0
            && usage.cache_write_tokens == 0
            && usage.cache_read_tokens == 0
            && usage.cache_miss_tokens == 0
            && usage.total_cost == 0.0;
        if !has_runtime && usage_is_empty {
            return None;
        }

        Some(SessionTelemetrySnapshot {
            version: SessionTelemetrySnapshotVersion::V6,
            usage,
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

        agendao_plugin::trigger(
            HookContext::new(HookEvent::TelemetrySnapshotUpdated)
                .with_session(session_id)
                .with_data("sessionID", serde_json::json!(session_id))
                .with_data("snapshot", snapshot),
        )
        .await;
    }

    fn emit(&self, event: ServerEvent) {
        Self::broadcast_server_event_payload(
            &self.event_bus,
            self.event_bus_telemetry.as_deref(),
            event,
        );
    }

    /// Push a typed ServerEvent onto the in-process bus. The event is shared
    /// with subscribers through `Arc`; the JSON wire text is materialized
    /// lazily (at most once) at the network boundary.
    fn broadcast_server_event_payload(
        event_bus: &broadcast::Sender<Arc<ServerBusEvent>>,
        event_bus_telemetry: Option<&EventBusTelemetry>,
        event: ServerEvent,
    ) {
        let receiver_count = event_bus.receiver_count();
        if event_bus
            .send(Arc::new(ServerBusEvent::event(event)))
            .is_err()
        {
            tracing::warn!("failed to broadcast runtime telemetry event (no active receivers)");
            if let Some(telemetry) = event_bus_telemetry {
                telemetry.record_send_error();
            }
        } else if let Some(telemetry) = event_bus_telemetry {
            telemetry.record_send(receiver_count);
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
        self.emit(ServerEvent::ControlInputTransition {
            session_id: session_id.to_string(),
            kind,
            phase,
            at,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;
    use tokio::time::{timeout, Duration};

    async fn recv_session_status_payload(
        rx: &mut broadcast::Receiver<Arc<ServerBusEvent>>,
        wait: Duration,
    ) -> Option<String> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let remaining = deadline.saturating_duration_since(now);
            match timeout(remaining, rx.recv()).await {
                Ok(Ok(payload)) if payload.json().contains("\"type\":\"session.status\"") => {
                    return Some(payload.json().to_string());
                }
                Ok(Ok(_)) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => return None,
            }
        }
    }

    #[tokio::test]
    async fn duplicate_session_status_is_not_rebroadcast() {
        let (tx, mut rx) = broadcast::channel(8);
        let authority = RuntimeTelemetryAuthority::new(tx, None);

        authority
            .set_session_run_status("ses_status_dedupe", SessionRunStatus::Busy)
            .await;
        let first = recv_session_status_payload(&mut rx, Duration::from_millis(200))
            .await
            .expect("first status event should arrive");
        assert!(first.contains("\"type\":\"session.status\""));

        authority
            .set_session_run_status("ses_status_dedupe", SessionRunStatus::Busy)
            .await;
        assert!(
            recv_session_status_payload(&mut rx, Duration::from_millis(100))
                .await
                .is_none()
        );

        authority
            .set_session_run_status("ses_status_dedupe", SessionRunStatus::Idle)
            .await;
        let second = recv_session_status_payload(&mut rx, Duration::from_millis(200))
            .await
            .expect("idle transition should arrive");
        assert!(second.contains("\"type\":\"session.status\""));
        assert!(second.contains("\"type\":\"idle\""));
    }

    #[tokio::test]
    async fn standalone_permission_resolution_returns_runtime_to_idle() {
        let (tx, _rx) = broadcast::channel(8);
        let authority = RuntimeTelemetryAuthority::new(tx, None);
        let sid = "ses_standalone_permission";

        authority
            .permission_requested(sid, "permission_1", serde_json::json!({ "tool": "pty" }))
            .await;
        authority
            .permission_resolved(sid, "permission_1", "once", None)
            .await;

        assert_eq!(
            authority.runtime_state().get(sid).await.unwrap().run_status,
            agendao_server_core::runtime_state::RunStatus::Idle
        );
    }

    #[tokio::test]
    async fn prompt_permission_resolution_resumes_running_runtime() {
        let (tx, _rx) = broadcast::channel(8);
        let authority = RuntimeTelemetryAuthority::new(tx, None);
        let sid = "ses_prompt_permission";

        authority
            .set_session_run_status(sid, SessionRunStatus::Busy)
            .await;
        authority
            .permission_requested(sid, "permission_1", serde_json::json!({ "tool": "bash" }))
            .await;
        authority
            .permission_resolved(sid, "permission_1", "once", None)
            .await;

        assert_eq!(
            authority.runtime_state().get(sid).await.unwrap().run_status,
            agendao_server_core::runtime_state::RunStatus::Running
        );
    }

    #[tokio::test]
    async fn blocked_session_recheck_round_trip_via_telemetry_authority() {
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        let telemetry = RuntimeTelemetryAuthority::new(tx, None);
        let sid = "recheck-telemetry";

        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Blocked {
                    reason: Some("waiting".to_string()),
                    recheck_at: Some(1),
                },
            )
            .await;

        // Verify control + projection both see Blocked.
        assert!(matches!(
            telemetry.runtime_control().session_run_status(sid).await,
            SessionRunStatus::Blocked { .. }
        ));
        assert_eq!(
            telemetry
                .runtime_state()
                .get(sid)
                .await
                .expect("state should exist")
                .run_status,
            agendao_server_core::runtime_state::RunStatus::Blocked
        );

        // Recheck via telemetry authority — goes through the bridge.
        let result = telemetry.recheck_session(sid).await;
        assert!(result.is_some(), "recheck should succeed");
        assert!(matches!(result.unwrap(), SessionRunStatus::Idle));

        // Verify control AND projection are both updated.
        assert!(matches!(
            telemetry.runtime_control().session_run_status(sid).await,
            SessionRunStatus::Idle
        ));
        assert_eq!(
            telemetry
                .runtime_state()
                .get(sid)
                .await
                .expect("state should still exist")
                .run_status,
            agendao_server_core::runtime_state::RunStatus::Idle,
            "RuntimeStateStore must be updated via the telemetry bridge"
        );
    }

    #[tokio::test]
    async fn blocked_session_recheck_not_due_via_telemetry() {
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        let telemetry = RuntimeTelemetryAuthority::new(tx, None);
        let sid = "recheck-future";

        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Blocked {
                    reason: Some("waiting".to_string()),
                    recheck_at: Some(9999999999999i64),
                },
            )
            .await;

        let result = telemetry.recheck_session(sid).await;
        assert!(
            result.is_none(),
            "recheck should not fire before recheck_at"
        );
    }

    #[tokio::test]
    async fn sleeping_session_wake_round_trip_via_telemetry_authority() {
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        let telemetry = RuntimeTelemetryAuthority::new(tx, None);
        let sid = "wake-telemetry";

        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Sleeping {
                    reason: Some("paused until morning".to_string()),
                    wake_at: Some(1),
                },
            )
            .await;

        assert!(matches!(
            telemetry.runtime_control().session_run_status(sid).await,
            SessionRunStatus::Sleeping { .. }
        ));
        assert_eq!(
            telemetry
                .runtime_state()
                .get(sid)
                .await
                .expect("state should exist")
                .run_status,
            agendao_server_core::runtime_state::RunStatus::Sleeping
        );

        let result = telemetry.wake_session(sid).await;
        assert!(
            result.is_some(),
            "wake should succeed when wake_at has passed"
        );
        assert!(matches!(result.unwrap(), SessionRunStatus::Idle));

        assert!(matches!(
            telemetry.runtime_control().session_run_status(sid).await,
            SessionRunStatus::Idle
        ));
        assert_eq!(
            telemetry
                .runtime_state()
                .get(sid)
                .await
                .expect("state should still exist")
                .run_status,
            agendao_server_core::runtime_state::RunStatus::Idle,
            "RuntimeStateStore must be updated via the telemetry bridge on wake"
        );
    }

    #[tokio::test]
    async fn sleeping_session_wake_not_due_via_telemetry() {
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        let telemetry = RuntimeTelemetryAuthority::new(tx, None);
        let sid = "wake-future";

        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Sleeping {
                    reason: Some("sleeping".to_string()),
                    wake_at: Some(9999999999999i64),
                },
            )
            .await;

        let result = telemetry.wake_session(sid).await;
        assert!(result.is_none(), "wake should not fire before wake_at");
    }
}
