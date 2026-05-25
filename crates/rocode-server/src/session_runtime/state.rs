//! Per-session aggregated runtime state.
//!
//! `SessionRuntimeState` is the server's single authoritative projection of
//! what a session is *doing right now*. It is maintained incrementally by the
//! existing lifecycle hooks (`SessionSchedulerLifecycleHook`, question/permission
//! routes) and exposed via `GET /session/{id}/runtime`.
//!
//! Design constraints (from the ROCode Constitution):
//! - Article 5 — unique state ownership: the `RuntimeStateStore` is the sole
//!   owner; consumers read through its API.
//! - Article 8 — observability: every active execution aspect must be
//!   reflected here.

use std::collections::HashMap;

use rocode_session::SessionUsage;
use rocode_types::{ControlInputKind, ControlInputPhase, SessionContextKind};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ── Primary state struct ────────────────────────────────────────────────────

/// Aggregated runtime snapshot for a single session.
///
/// Fields are kept intentionally flat and cheap to clone so that the
/// `GET /session/{id}/runtime` endpoint can return a snapshot without
/// holding the lock across serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRuntimeState {
    pub session_id: String,
    pub run_status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_stage_id: Option<String>,
    #[serde(default)]
    pub active_stage_count: u32,
    pub active_tools: Vec<ActiveToolSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<PendingQuestionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_permission: Option<PendingPermissionSummary>,
    #[serde(default)]
    pub pending_followup_count: u64,
    /// Constitution §8: pending steering messages must be observable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_steering: Vec<PendingSteeringMessageSummary>,
    #[serde(default)]
    pub interrupt: InterruptRuntimeState,
    pub attached_sessions: Vec<AttachedSessionSummary>,
}

impl SessionRuntimeState {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            run_status: RunStatus::Idle,
            current_message_id: None,
            usage: None,
            active_stage_id: None,
            active_stage_count: 0,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            pending_steering: Vec::new(),
            interrupt: InterruptRuntimeState::default(),
            attached_sessions: Vec::new(),
        }
    }
}

// ── Supporting types ────────────────────────────────────────────────────────

/// Coarse run-status for the session, derived from lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Idle,
    Running,
    Compacting,
    WaitingOnTool,
    WaitingOnUser,
    Cancelling,
}

impl Default for RunStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// Summary of a currently executing tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveToolSummary {
    pub tool_call_id: String,
    pub tool_name: String,
    /// Monotonic timestamp (epoch millis) when the tool started.
    pub started_at: i64,
}

/// Summary of a pending question awaiting user answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestionSummary {
    pub request_id: String,
    pub questions: serde_json::Value,
}

/// Summary of a pending permission request awaiting user decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPermissionSummary {
    pub permission_id: String,
    pub requested_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// Summary of a pending steering message waiting for the next tool boundary.
/// Constitution §8: pending steering must be observable in runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSteeringMessageSummary {
    pub id: String,
    pub owner_session_id: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    /// Always "next_tool_boundary" in P0.
    pub deliver_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptPhase {
    #[default]
    Idle,
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptTarget {
    Run,
    Stage,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterruptRuntimeState {
    pub phase: InterruptPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<InterruptTarget>,
}

#[derive(Debug, Clone)]
pub enum RuntimeProtocolUpdate {
    PermissionRequested {
        permission_id: String,
        requested_at: i64,
        tool: Option<String>,
    },
    PermissionResolved,
    SteeringEnqueued(PendingSteeringMessageSummary),
    SteeringCleared,
    InterruptRequested {
        requested_at: i64,
        target: InterruptTarget,
    },
    /// P3-G: Unified control input lifecycle transition.
    /// `kind` identifies which control path. `phase` is the new phase.
    /// `at` is epoch-millis of the transition.
    ControlInputTransition {
        kind: ControlInputKind,
        phase: ControlInputPhase,
        at: i64,
    },
}

/// Summary of an attached session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachedSessionSummary {
    pub attached_id: String,
    pub parent_id: String,
    pub context_kind: SessionContextKind,
}

// ── Store ───────────────────────────────────────────────────────────────────

/// Process-wide store of per-session runtime state.
///
/// Uses `tokio::sync::RwLock` for read-heavy access (SSE consumers poll,
/// REST endpoint reads) with infrequent writes (lifecycle hooks).
#[derive(Debug)]
pub struct RuntimeStateStore {
    states: RwLock<HashMap<String, SessionRuntimeState>>,
}

impl RuntimeStateStore {
    pub fn new() -> Self {
        Self {
            states: RwLock::new(HashMap::new()),
        }
    }

    /// Get a cloned snapshot of a session's runtime state.
    pub async fn get(&self, session_id: &str) -> Option<SessionRuntimeState> {
        let guard = self.states.read().await;
        guard.get(session_id).cloned()
    }

    /// Apply a mutation to a session's runtime state.
    ///
    /// If the session does not yet exist in the store, a new default entry
    /// is created before the mutation is applied.
    pub async fn update<F>(&self, session_id: &str, f: F)
    where
        F: FnOnce(&mut SessionRuntimeState),
    {
        let mut guard = self.states.write().await;
        let state = guard
            .entry(session_id.to_string())
            .or_insert_with(|| SessionRuntimeState::new(session_id));
        f(state);
    }

    /// Remove a session's runtime state (e.g. on session delete).
    pub async fn remove(&self, session_id: &str) {
        let mut guard = self.states.write().await;
        guard.remove(session_id);
    }

    pub async fn apply_protocol_update(&self, session_id: &str, update: RuntimeProtocolUpdate) {
        self.update(session_id, |s| match update {
            RuntimeProtocolUpdate::PermissionRequested {
                permission_id,
                requested_at,
                tool,
            } => {
                s.run_status = RunStatus::WaitingOnUser;
                s.pending_permission = Some(PendingPermissionSummary {
                    permission_id,
                    requested_at,
                    tool,
                });
            }
            RuntimeProtocolUpdate::PermissionResolved => {
                s.pending_permission = None;
                if s.run_status == RunStatus::WaitingOnUser && s.pending_question.is_none() {
                    s.run_status = RunStatus::Running;
                }
            }
            RuntimeProtocolUpdate::SteeringEnqueued(summary) => {
                s.pending_steering.push(summary);
            }
            RuntimeProtocolUpdate::SteeringCleared => {
                s.pending_steering.clear();
            }
            RuntimeProtocolUpdate::InterruptRequested {
                requested_at,
                target,
            } => {
                s.run_status = RunStatus::Cancelling;
                s.interrupt.phase = InterruptPhase::Requested;
                s.interrupt.requested_at = Some(requested_at);
                s.interrupt.target = Some(target);
            }
            RuntimeProtocolUpdate::ControlInputTransition { kind, phase, at } => {
                let _ = at;
                match (kind, phase) {
                    (ControlInputKind::Followup, ControlInputPhase::Queued) => {
                        s.pending_followup_count = s.pending_followup_count.saturating_add(1);
                    }
                    (ControlInputKind::Followup, ControlInputPhase::Adopted)
                    | (ControlInputKind::Followup, ControlInputPhase::Consumed)
                    | (ControlInputKind::Followup, ControlInputPhase::Cleared) => {
                        s.pending_followup_count = 0;
                    }
                    _ => {
                        // Other control transitions remain represented by
                        // their dedicated runtime fields.
                    }
                }
            }
        })
        .await;
    }

    // ── Convenience mutators ────────────────────────────────────────────

    /// Mark the session as running with the given message id.
    pub async fn mark_running(&self, session_id: &str, message_id: Option<String>) {
        self.update(session_id, |s| {
            s.run_status = RunStatus::Running;
            s.current_message_id = message_id;
            s.interrupt = InterruptRuntimeState::default();
        })
        .await;
    }

    /// Mark the session as compacting without clearing the surrounding prompt
    /// execution context.
    pub async fn mark_compacting(&self, session_id: &str) {
        self.update(session_id, |s| {
            s.run_status = RunStatus::Compacting;
            s.interrupt = InterruptRuntimeState::default();
        })
        .await;
    }

    /// Mark the session as idle, clearing transient state.
    pub async fn mark_idle(&self, session_id: &str) {
        self.update(session_id, |s| {
            s.run_status = RunStatus::Idle;
            s.current_message_id = None;
            s.active_tools.clear();
            s.active_stage_id = None;
            s.active_stage_count = 0;
            s.pending_question = None;
            s.pending_permission = None;
            s.interrupt = InterruptRuntimeState::default();
            // attached_sessions are NOT cleared here — they persist until
            // explicit detach events.
        })
        .await;
    }

    /// Register a tool call start.
    pub async fn tool_started(&self, session_id: &str, tool_call_id: &str, tool_name: &str) {
        self.update(session_id, |s| {
            s.run_status = RunStatus::WaitingOnTool;
            s.active_tools.push(ActiveToolSummary {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                started_at: chrono::Utc::now().timestamp_millis(),
            });
        })
        .await;
    }

    /// Register a tool call end.
    pub async fn tool_ended(&self, session_id: &str, tool_call_id: &str) {
        self.update(session_id, |s| {
            s.active_tools.retain(|t| t.tool_call_id != tool_call_id);
            // If no more tools are active, revert to Running.
            if s.active_tools.is_empty() && s.run_status == RunStatus::WaitingOnTool {
                s.run_status = RunStatus::Running;
            }
        })
        .await;
    }

    /// Set a pending question.
    pub async fn question_created(
        &self,
        session_id: &str,
        request_id: &str,
        questions: serde_json::Value,
    ) {
        self.update(session_id, |s| {
            s.run_status = RunStatus::WaitingOnUser;
            s.pending_question = Some(PendingQuestionSummary {
                request_id: request_id.to_string(),
                questions,
            });
        })
        .await;
    }

    pub async fn scheduler_stage_started(&self, session_id: &str, stage_id: &str) {
        self.update(session_id, |s| {
            s.active_stage_id = Some(stage_id.to_string());
            s.active_stage_count = s.active_stage_count.saturating_add(1);
        })
        .await;
    }

    pub async fn scheduler_stage_finished(&self, session_id: &str, stage_id: Option<&str>) {
        self.update(session_id, |s| {
            s.active_stage_count = s.active_stage_count.saturating_sub(1);
            if s.active_stage_count == 0 {
                s.active_stage_id = None;
            } else if s.active_stage_id.as_deref() == stage_id {
                s.active_stage_id = None;
            }
        })
        .await;
    }

    pub async fn set_usage(&self, session_id: &str, usage: SessionUsage) {
        self.update(session_id, |s| {
            s.usage = Some(usage);
        })
        .await;
    }

    /// Append a steering message summary to the session's runtime state (Constitution §8).
    pub async fn steering_enqueued(
        &self,
        session_id: &str,
        summary: PendingSteeringMessageSummary,
    ) {
        self.apply_protocol_update(session_id, RuntimeProtocolUpdate::SteeringEnqueued(summary))
            .await;
    }

    /// Clear all pending steering messages for a session.
    pub async fn steering_cleared(&self, session_id: &str) {
        self.apply_protocol_update(session_id, RuntimeProtocolUpdate::SteeringCleared)
            .await;
    }

    /// Clear a pending question.
    pub async fn question_resolved(&self, session_id: &str) {
        self.update(session_id, |s| {
            s.pending_question = None;
            // Revert to Running only if not waiting on something else.
            if s.run_status == RunStatus::WaitingOnUser && s.pending_permission.is_none() {
                s.run_status = RunStatus::Running;
            }
        })
        .await;
    }

    /// Set a pending permission request.
    pub async fn permission_requested(
        &self,
        session_id: &str,
        permission_id: &str,
        requested_at: i64,
        tool: Option<String>,
    ) {
        self.apply_protocol_update(
            session_id,
            RuntimeProtocolUpdate::PermissionRequested {
                permission_id: permission_id.to_string(),
                requested_at,
                tool,
            },
        )
        .await;
    }

    /// Clear a pending permission request.
    pub async fn permission_resolved(&self, session_id: &str) {
        self.apply_protocol_update(session_id, RuntimeProtocolUpdate::PermissionResolved)
            .await;
    }

    pub async fn interrupt_requested(
        &self,
        session_id: &str,
        requested_at: i64,
        target: InterruptTarget,
    ) {
        self.apply_protocol_update(
            session_id,
            RuntimeProtocolUpdate::InterruptRequested {
                requested_at,
                target,
            },
        )
        .await;
    }

    /// Register an attached session.
    pub async fn attached_session_registered(
        &self,
        parent_id: &str,
        attached_id: &str,
        context_kind: SessionContextKind,
    ) {
        self.update(parent_id, |s| {
            // Avoid duplicates.
            if !s
                .attached_sessions
                .iter()
                .any(|c| c.attached_id == attached_id)
            {
                s.attached_sessions.push(AttachedSessionSummary {
                    attached_id: attached_id.to_string(),
                    parent_id: parent_id.to_string(),
                    context_kind,
                });
            }
        })
        .await;
    }

    /// Unregister an attached session.
    pub async fn attached_session_unregistered(&self, parent_id: &str, attached_id: &str) {
        self.update(parent_id, |s| {
            s.attached_sessions.retain(|c| c.attached_id != attached_id);
        })
        .await;
    }
}

impl Default for RuntimeStateStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_session_starts_idle() {
        let store = RuntimeStateStore::new();
        let state = store.get("ses_1").await;
        assert!(state.is_none(), "unknown session returns None");

        store.mark_idle("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Idle);
        assert!(state.active_tools.is_empty());
    }

    #[tokio::test]
    async fn mark_running_then_idle_clears_transient_state() {
        let store = RuntimeStateStore::new();

        store
            .mark_running("ses_1", Some("msg_001".to_string()))
            .await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Running);
        assert_eq!(state.current_message_id.as_deref(), Some("msg_001"));

        store.mark_compacting("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Compacting);
        assert_eq!(state.current_message_id.as_deref(), Some("msg_001"));

        store.tool_started("ses_1", "tc_1", "bash").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::WaitingOnTool);
        assert_eq!(state.active_tools.len(), 1);

        store.mark_idle("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Idle);
        assert!(state.active_tools.is_empty());
        assert!(state.current_message_id.is_none());
    }

    #[tokio::test]
    async fn tool_end_reverts_to_running_when_no_more_tools() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_1", None).await;
        store.tool_started("ses_1", "tc_1", "read").await;
        store.tool_started("ses_1", "tc_2", "write").await;
        assert_eq!(store.get("ses_1").await.unwrap().active_tools.len(), 2);

        store.tool_ended("ses_1", "tc_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.active_tools.len(), 1);
        // Still WaitingOnTool because tc_2 is active.
        assert_eq!(state.run_status, RunStatus::WaitingOnTool);

        store.tool_ended("ses_1", "tc_2").await;
        let state = store.get("ses_1").await.unwrap();
        assert!(state.active_tools.is_empty());
        assert_eq!(state.run_status, RunStatus::Running);
    }

    #[tokio::test]
    async fn question_and_permission_lifecycle() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_1", None).await;

        store
            .question_created("ses_1", "q_1", serde_json::json!([{"question": "ok?"}]))
            .await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::WaitingOnUser);
        assert!(state.pending_question.is_some());

        store.question_resolved("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Running);
        assert!(state.pending_question.is_none());

        store
            .permission_requested("ses_1", "perm_1", 123, Some("bash".to_string()))
            .await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::WaitingOnUser);
        assert_eq!(
            state
                .pending_permission
                .as_ref()
                .map(|pending| pending.tool.as_deref()),
            Some(Some("bash"))
        );

        store.permission_resolved("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Running);
        assert!(state.pending_permission.is_none());
    }

    #[tokio::test]
    async fn attached_session_register_unregister() {
        let store = RuntimeStateStore::new();
        store.mark_running("parent", None).await;

        store
            .attached_session_registered(
                "parent",
                "child_1",
                SessionContextKind::SchedulerStageOutputSession,
            )
            .await;
        store
            .attached_session_registered(
                "parent",
                "child_2",
                SessionContextKind::DelegatedSubsession,
            )
            .await;
        let state = store.get("parent").await.unwrap();
        assert_eq!(state.attached_sessions.len(), 2);
        assert_eq!(
            state.attached_sessions[0].context_kind,
            SessionContextKind::SchedulerStageOutputSession
        );

        // Duplicate attach is idempotent.
        store
            .attached_session_registered(
                "parent",
                "child_1",
                SessionContextKind::SchedulerStageOutputSession,
            )
            .await;
        assert_eq!(
            store.get("parent").await.unwrap().attached_sessions.len(),
            2
        );

        store
            .attached_session_unregistered("parent", "child_1")
            .await;
        let state = store.get("parent").await.unwrap();
        assert_eq!(state.attached_sessions.len(), 1);
        assert_eq!(state.attached_sessions[0].attached_id, "child_2");
    }

    #[tokio::test]
    async fn remove_cleans_up() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_1", None).await;
        assert!(store.get("ses_1").await.is_some());

        store.remove("ses_1").await;
        assert!(store.get("ses_1").await.is_none());
    }

    #[tokio::test]
    async fn concurrent_question_and_permission() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_1", None).await;

        // Both question and permission pending simultaneously.
        store
            .question_created("ses_1", "q_1", serde_json::json!("q"))
            .await;
        store.permission_requested("ses_1", "p_1", 456, None).await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::WaitingOnUser);

        // Resolving question alone should NOT revert to Running
        // because permission is still pending.
        store.question_resolved("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::WaitingOnUser);

        store.permission_resolved("ses_1").await;
        let state = store.get("ses_1").await.unwrap();
        assert_eq!(state.run_status, RunStatus::Running);
    }

    #[tokio::test]
    async fn steering_clear_removes_runtime_observable_pending_messages() {
        let store = RuntimeStateStore::new();
        store
            .steering_enqueued(
                "ses_1",
                PendingSteeringMessageSummary {
                    id: "steer_1".to_string(),
                    owner_session_id: "ses_1".to_string(),
                    created_at: 1,
                    source_session_id: Some("child_1".to_string()),
                    deliver_at: "next_tool_boundary".to_string(),
                },
            )
            .await;
        store
            .steering_enqueued(
                "ses_1",
                PendingSteeringMessageSummary {
                    id: "steer_2".to_string(),
                    owner_session_id: "ses_1".to_string(),
                    created_at: 2,
                    source_session_id: None,
                    deliver_at: "next_tool_boundary".to_string(),
                },
            )
            .await;

        assert_eq!(
            store
                .get("ses_1")
                .await
                .expect("runtime state should exist")
                .pending_steering
                .len(),
            2
        );

        store.steering_cleared("ses_1").await;

        assert!(store
            .get("ses_1")
            .await
            .expect("runtime state should still exist")
            .pending_steering
            .is_empty());
    }

    #[tokio::test]
    async fn interrupt_protocol_is_requested_and_cleared_by_run_lifecycle() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_1", None).await;

        store
            .interrupt_requested("ses_1", 789, InterruptTarget::Run)
            .await;
        let state = store
            .get("ses_1")
            .await
            .expect("runtime state should exist");
        assert_eq!(state.run_status, RunStatus::Cancelling);
        assert_eq!(state.interrupt.phase, InterruptPhase::Requested);
        assert_eq!(state.interrupt.requested_at, Some(789));
        assert_eq!(state.interrupt.target, Some(InterruptTarget::Run));

        store.mark_idle("ses_1").await;
        let state = store
            .get("ses_1")
            .await
            .expect("runtime state should still exist after idle");
        assert_eq!(state.interrupt.phase, InterruptPhase::Idle);
        assert_eq!(state.interrupt.requested_at, None);
        assert_eq!(state.interrupt.target, None);
    }

    /// P3-4: after idle, transient runtime state must be cleared.
    /// mark_idle clears permission, question, active tools, and interrupt.
    /// Steering is NOT cleared (consumed at tool boundaries within a run).
    #[tokio::test]
    async fn mark_idle_clears_permission_and_interrupt_not_steering() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_idle", None).await;

        store
            .permission_requested("ses_idle", "perm_1", 1000, Some("bash".to_string()))
            .await;
        store
            .interrupt_requested("ses_idle", 42, InterruptTarget::Run)
            .await;
        store
            .steering_enqueued(
                "ses_idle",
                PendingSteeringMessageSummary {
                    id: "steer_1".to_string(),
                    owner_session_id: "ses_idle".to_string(),
                    created_at: 1000,
                    deliver_at: "next_tool_boundary".to_string(),
                    source_session_id: None,
                },
            )
            .await;

        store.mark_idle("ses_idle").await;

        let state = store
            .get("ses_idle")
            .await
            .expect("runtime state should exist after idle");
        assert!(
            state.pending_permission.is_none(),
            "permission should be cleared by mark_idle"
        );
        assert_eq!(
            state.interrupt.phase,
            InterruptPhase::Idle,
            "interrupt should reset"
        );
        assert!(
            !state.pending_steering.is_empty(),
            "steering survives idle (consumed at tool boundary)"
        );
    }

    /// P3-4: after interrupt → idle → run, the next turn's runtime state
    /// must not carry any interrupt residue into the new run.
    #[tokio::test]
    async fn interrupt_cleared_before_next_run_does_not_reappear() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_int_2", None).await;
        store
            .interrupt_requested("ses_int_2", 42, InterruptTarget::Run)
            .await;

        // Interrupt consumed: mark idle then start a new run.
        store.mark_idle("ses_int_2").await;
        store.mark_running("ses_int_2", None).await;

        let state = store
            .get("ses_int_2")
            .await
            .expect("runtime state should exist");
        assert_eq!(
            state.interrupt.phase,
            InterruptPhase::Idle,
            "interrupt phase should reset after idle+run"
        );
        assert_eq!(
            state.run_status,
            RunStatus::Running,
            "new run should be Running, not Cancelling"
        );
    }

    #[tokio::test]
    async fn followup_control_transitions_update_pending_count() {
        let store = RuntimeStateStore::new();
        store.mark_running("ses_followup", None).await;

        store
            .apply_protocol_update(
                "ses_followup",
                RuntimeProtocolUpdate::ControlInputTransition {
                    kind: ControlInputKind::Followup,
                    phase: ControlInputPhase::Queued,
                    at: 1,
                },
            )
            .await;
        assert_eq!(
            store
                .get("ses_followup")
                .await
                .expect("runtime state should exist")
                .pending_followup_count,
            1
        );

        store
            .apply_protocol_update(
                "ses_followup",
                RuntimeProtocolUpdate::ControlInputTransition {
                    kind: ControlInputKind::Followup,
                    phase: ControlInputPhase::Consumed,
                    at: 2,
                },
            )
            .await;
        assert_eq!(
            store
                .get("ses_followup")
                .await
                .expect("runtime state should exist")
                .pending_followup_count,
            0
        );
    }
}
