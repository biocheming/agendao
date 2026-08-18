//! Task-level stall detection and bounded replanning.
//!
//! The detector consumes ledger frames — typed snapshots committed at seams
//! — never hidden reasoning or word frequencies. Suppression rules from the
//! governance plan:
//! - `awaiting_user` frames are silent: waiting on a permission or question
//!   is not reasoning stalling, and no wall-clock or seam window accrues.
//! - `interrupted` frames are silent for the same reason.
//! - `RunStarted` resets the window: after a resume the first seams only
//!   establish a new baseline; a pre-interrupt `Next` that is still current
//!   is not evidence of a stall.
//!
//! Native governance remains opt-in per session. For a governed structured or
//! loop run, a repeated typed stall can enqueue a bounded system steering at
//! the existing next-tool boundary. It never bypasses scheduler/tool policy.

use std::collections::VecDeque;
use std::sync::Arc;

use agendao_types::task_ledger::{
    SessionTaskLedger, TaskLedgerActor, TaskLedgerOp, TaskLedgerStatus,
};

use crate::ServerState;

const WINDOW: usize = 3;
const MAX_REPLAN_ATTEMPTS: u32 = 2;
const REPLAN_DEADLINE_MS: i64 = 120_000;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StallFrame {
    pub revision: u64,
    pub status: TaskLedgerStatus,
    pub next_statement: Option<String>,
    pub verified_count: usize,
    pub open_count: usize,
}

/// Per-session observation window. Server memory only; rebuilt from the
/// ledger after a restart (an empty window simply re-baselines).
#[derive(Default)]
pub struct StallWindow {
    frames: VecDeque<StallFrame>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StallPhase {
    #[default]
    Healthy,
    Suspected,
    Stalled,
    Replanning,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub(crate) enum StallAction {
    None,
    Replan {
        attempt: u32,
        next_statement: String,
        steering: String,
    },
    Block {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct StallDecision {
    pub phase: StallPhase,
    pub observations: Vec<StallObservation>,
    pub replan_attempts: u32,
    pub deadline_at_ms: i64,
    pub action: StallAction,
}

#[derive(Default)]
struct SessionStallControl {
    window: StallWindow,
    phase: StallPhase,
    consecutive_hits: u32,
    replan_attempts: u32,
    deadline_at_ms: i64,
}

impl SessionStallControl {
    fn evaluate(
        &mut self,
        ledger: &SessionTaskLedger,
        run_started: bool,
        now_ms: i64,
        auto_replan: bool,
        cancelled: bool,
    ) -> StallDecision {
        if run_started {
            self.phase = StallPhase::Healthy;
            self.consecutive_hits = 0;
            self.replan_attempts = 0;
            self.deadline_at_ms = now_ms.saturating_add(REPLAN_DEADLINE_MS);
            let observations = self.window.record(ledger);
            return self.decision(observations, StallAction::None);
        }

        // Fast/direct runs do not participate in the detector at all. This
        // is stronger than merely suppressing actions: they must not surface
        // suspected/stalled telemetry after ordinary tool calls.
        if !auto_replan {
            self.window.frames.clear();
            self.phase = StallPhase::Healthy;
            self.consecutive_hits = 0;
            return self.decision(Vec::new(), StallAction::None);
        }

        let observations = self.window.push_frame(ledger);
        if matches!(
            ledger.status,
            TaskLedgerStatus::AwaitingUser | TaskLedgerStatus::Interrupted
        ) {
            return self.decision(observations, StallAction::None);
        }
        if ledger.status == TaskLedgerStatus::Blocked {
            self.phase = StallPhase::Blocked;
            return self.decision(observations, StallAction::None);
        }

        let no_new_verification = observations
            .iter()
            .any(|observation| matches!(observation, StallObservation::NoNewVerification { .. }));
        let repeated_direction = observations.iter().any(|observation| {
            matches!(
                observation,
                StallObservation::NextUnchanged { .. } | StallObservation::OpenCountGrowing { .. }
            )
        });
        let stall_evidence = no_new_verification && repeated_direction;
        if !stall_evidence {
            self.phase = StallPhase::Healthy;
            self.consecutive_hits = 0;
            return self.decision(observations, StallAction::None);
        }

        self.consecutive_hits = self.consecutive_hits.saturating_add(1);
        if self.consecutive_hits == 1 {
            self.phase = StallPhase::Suspected;
            return self.decision(observations, StallAction::None);
        }

        self.phase = StallPhase::Stalled;
        if !auto_replan || cancelled {
            return self.decision(observations, StallAction::None);
        }
        if now_ms >= self.deadline_at_ms {
            self.phase = StallPhase::Blocked;
            return self.decision(
                observations,
                StallAction::Block {
                    reason: "automatic replanning deadline exhausted".to_string(),
                },
            );
        }
        if self.replan_attempts >= MAX_REPLAN_ATTEMPTS {
            self.phase = StallPhase::Blocked;
            return self.decision(
                observations,
                StallAction::Block {
                    reason: format!(
                        "automatic replanning budget exhausted after {} attempts",
                        MAX_REPLAN_ATTEMPTS
                    ),
                },
            );
        }

        self.replan_attempts += 1;
        self.phase = StallPhase::Replanning;
        let attempt = self.replan_attempts;
        let next_statement = format!(
            "Replan attempt {attempt}: run one different discriminating check before repeating the prior action"
        );
        let steering = format!(
            "Task governance detected repeated typed stall evidence. Replan attempt {attempt}/{MAX_REPLAN_ATTEMPTS}: change strategy now. Run a different discriminating check; then either make concrete progress or name a specific blocker. Do not repeat the previous action unchanged."
        );
        self.decision(
            observations,
            StallAction::Replan {
                attempt,
                next_statement,
                steering,
            },
        )
    }

    fn decision(&self, observations: Vec<StallObservation>, action: StallAction) -> StallDecision {
        StallDecision {
            phase: self.phase,
            observations,
            replan_attempts: self.replan_attempts,
            deadline_at_ms: self.deadline_at_ms,
            action,
        }
    }

    fn rebaseline(&mut self, ledger: &SessionTaskLedger) {
        self.window.frames.clear();
        let _ = self.window.push_frame(ledger);
        self.consecutive_hits = 0;
    }
}

impl StallWindow {
    pub fn record(&mut self, ledger: &SessionTaskLedger) -> Vec<StallObservation> {
        // Resume-window reset: a fresh run wipes prior evidence.
        self.frames.clear();
        self.push_frame(ledger)
    }

    pub fn push_frame(&mut self, ledger: &SessionTaskLedger) -> Vec<StallObservation> {
        // Suppression: awaiting/interrupted frames neither count nor accrue.
        if matches!(
            ledger.status,
            TaskLedgerStatus::AwaitingUser | TaskLedgerStatus::Interrupted
        ) {
            return Vec::new();
        }
        self.frames.push_back(StallFrame {
            revision: ledger.revision,
            status: ledger.status,
            next_statement: ledger.next.as_ref().map(|next| next.statement.clone()),
            verified_count: ledger.verified.len(),
            open_count: ledger.open_questions().len(),
        });
        if self.frames.len() > WINDOW {
            self.frames.pop_front();
        }
        observe(&self.frames)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "observation")]
pub(crate) enum StallObservation {
    NextUnchanged { seams: usize },
    NoNewVerification { seams: usize },
    OpenCountGrowing { seams: usize },
    VerifiedGrowingButNextUnchanged { seams: usize },
}

/// Pure rule evaluation over the window. All rules need a FULL window of
/// non-suppressed frames; partial windows assert nothing (fast tasks with
/// three ordinary tool calls must not be flagged — plan exit criteria).
pub(crate) fn observe(frames: &VecDeque<StallFrame>) -> Vec<StallObservation> {
    if frames.len() < WINDOW {
        return Vec::new();
    }
    let mut found = Vec::new();
    let nexts: Vec<&Option<String>> = frames.iter().map(|frame| &frame.next_statement).collect();
    let next_unchanged =
        nexts[0].is_some() && nexts.iter().collect::<std::collections::HashSet<_>>().len() == 1;
    let verified_flat =
        frames.front().unwrap().verified_count == frames.back().unwrap().verified_count;
    let opens: Vec<usize> = frames.iter().map(|frame| frame.open_count).collect();
    let opens_growing = opens.windows(2).all(|pair| pair[1] > pair[0]);

    if next_unchanged {
        found.push(StallObservation::NextUnchanged { seams: WINDOW });
    }
    if verified_flat {
        found.push(StallObservation::NoNewVerification { seams: WINDOW });
    }
    if opens_growing {
        found.push(StallObservation::OpenCountGrowing { seams: WINDOW });
    }
    if next_unchanged && !verified_flat {
        found.push(StallObservation::VerifiedGrowingButNextUnchanged { seams: WINDOW });
    }
    found
}

/// Registry of per-session windows on the server.
#[derive(Default)]
pub struct StallWindows {
    windows: std::sync::Mutex<std::collections::HashMap<String, SessionStallControl>>,
}

impl StallWindows {
    /// Record a committed seam. `run_started` implements the resume-window
    /// reset. Returns the observations worth surfacing (possibly empty).
    pub(crate) fn record(
        &self,
        session_id: &str,
        ledger: &SessionTaskLedger,
        run_started: bool,
        now_ms: i64,
        auto_replan: bool,
        cancelled: bool,
    ) -> StallDecision {
        if ledger.revision == 0 {
            return StallDecision {
                phase: StallPhase::Healthy,
                observations: Vec::new(),
                replan_attempts: 0,
                deadline_at_ms: 0,
                action: StallAction::None,
            };
        }
        let mut windows = match self.windows.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        windows.entry(session_id.to_string()).or_default().evaluate(
            ledger,
            run_started,
            now_ms,
            auto_replan,
            cancelled,
        )
    }

    pub(crate) fn rebaseline(&self, session_id: &str, ledger: &SessionTaskLedger) {
        let mut windows = match self.windows.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        windows
            .entry(session_id.to_string())
            .or_default()
            .rebaseline(ledger);
    }

    /// Test-only: how many frames the session's window currently holds.
    #[cfg(test)]
    pub fn frame_count(&self, session_id: &str) -> usize {
        self.windows
            .lock()
            .map(|windows| {
                windows
                    .get(session_id)
                    .map(|control| control.window.frames.len())
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }

    pub fn forget(&self, session_id: &str) {
        if let Ok(mut windows) = self.windows.lock() {
            windows.remove(session_id);
        }
    }
}

/// Called from the seam dispatcher after every committed revision.
pub(crate) async fn record_stall_frame(
    state: &Arc<ServerState>,
    ledger: &SessionTaskLedger,
    run_started: bool,
    auto_replan: bool,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let cancelled = cancellation.is_some_and(|token| token.is_cancelled());
    let decision = state.stall_windows.record(
        &ledger.session_id,
        ledger,
        run_started,
        now_ms,
        auto_replan,
        cancelled,
    );
    if decision.observations.is_empty() && matches!(decision.action, StallAction::None) {
        return;
    }
    tracing::info!(
        session_id = %ledger.session_id,
        decision = ?decision,
        "task stall control decision"
    );

    match &decision.action {
        StallAction::Replan {
            next_statement,
            steering,
            ..
        } if !cancellation.is_some_and(|token| token.is_cancelled()) => {
            if let Ok(Some((snapshot, _))) =
                super::task_ledger::apply_task_ledger_op_unless_cancelled(
                    state,
                    &ledger.session_id,
                    ledger.revision,
                    TaskLedgerOp::SetNext {
                        statement: next_statement.clone(),
                        actor: Some(TaskLedgerActor::System),
                    },
                    cancellation,
                )
                .await
            {
                state
                    .stall_windows
                    .rebaseline(&ledger.session_id, &snapshot);
                if !cancellation.is_some_and(|token| token.is_cancelled()) {
                    super::steering::enqueue_system_steering(
                        state,
                        &ledger.session_id,
                        steering.clone(),
                        "steer_replan",
                    )
                    .await;
                }
            }
        }
        StallAction::Block { reason }
            if !cancellation.is_some_and(|token| token.is_cancelled()) =>
        {
            if let Ok(Some((_snapshot, _))) =
                super::task_ledger::apply_task_ledger_op_unless_cancelled(
                    state,
                    &ledger.session_id,
                    ledger.revision,
                    TaskLedgerOp::SetStatus {
                        status: TaskLedgerStatus::Blocked,
                        awaiting: None,
                        blocked_reason: Some(reason.clone()),
                    },
                    cancellation,
                )
                .await
            {}
        }
        _ => {}
    }

    {
        let mut sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&ledger.session_id) {
            session.insert_metadata(
                "stall_observation".to_string(),
                serde_json::to_value(&decision).unwrap_or_default(),
            );
        }
    }
    // Persistence helpers re-acquire the sessions mutex.
    crate::routes::session::session_crud::persist_session_if_enabled(state, &ledger.session_id)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger_with(
        status: TaskLedgerStatus,
        next: &str,
        verified: usize,
        open: usize,
    ) -> SessionTaskLedger {
        let mut ledger = SessionTaskLedger::empty("ses_stall");
        ledger.revision = 1;
        ledger.status = status;
        ledger.next = Some(agendao_types::task_ledger::NextAction {
            statement: next.to_string(),
            provenance: agendao_types::task_ledger::NextActionProvenance {
                actor: agendao_types::task_ledger::TaskLedgerActor::Model,
                pre_interrupt: false,
                set_at: 0,
            },
        });
        ledger.verified = (0..verified)
            .map(|i| agendao_types::task_ledger::VerifiedCheckpoint {
                id: format!("chk-{i:02}"),
                claim: "c".into(),
                verifier: agendao_types::task_ledger::VerifierRef::DeterministicCheck {
                    description: "t".into(),
                },
                coverage: agendao_types::task_ledger::VerificationCoverage { scope: "s".into() },
                goal_generation: ledger.goal_generation,
                covered_criteria: vec![],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
                superseded_by: None,
                created_at: 0,
            })
            .collect();
        ledger.open = (0..open)
            .map(|i| agendao_types::task_ledger::OpenQuestion {
                id: format!("open-{i:02}"),
                question: "q".into(),
                settled_by: "t".into(),
                opened_at: 0,
                closed_by_checkpoint_id: None,
            })
            .collect();
        ledger
    }

    #[test]
    fn next_unchanged_across_full_window_is_observed() {
        let mut window = StallWindow::default();
        let mut observations = Vec::new();
        for _ in 0..WINDOW {
            observations =
                window.push_frame(&ledger_with(TaskLedgerStatus::Active, "same next", 1, 0));
        }
        assert!(observations
            .iter()
            .any(|observation| matches!(observation, StallObservation::NextUnchanged { .. })));
        assert!(observations
            .iter()
            .any(|observation| matches!(observation, StallObservation::NoNewVerification { .. })));
    }

    #[test]
    fn partial_window_asserts_nothing() {
        let mut two = StallWindow::default();
        let first = two.push_frame(&ledger_with(TaskLedgerStatus::Active, "n", 0, 0));
        let second = two.push_frame(&ledger_with(TaskLedgerStatus::Active, "n", 0, 0));
        // Two frames only: no rule may fire even though both look stalled.
        assert!(first.is_empty());
        assert!(
            second.is_empty(),
            "window shorter than {WINDOW} observes nothing"
        );
    }

    #[test]
    fn awaiting_user_and_interrupted_are_silent_and_do_not_accrue() {
        let mut window = StallWindow::default();
        window.push_frame(&ledger_with(TaskLedgerStatus::Active, "n", 0, 0));
        window.push_frame(&ledger_with(TaskLedgerStatus::Active, "n", 0, 0));
        // Suppressed statuses neither fire nor enter the window.
        assert!(window
            .push_frame(&ledger_with(TaskLedgerStatus::AwaitingUser, "wait", 0, 0))
            .is_empty());
        assert!(window
            .push_frame(&ledger_with(TaskLedgerStatus::Interrupted, "n", 0, 0))
            .is_empty());
        // The third ACTIVE frame completes a full-active window (the two
        // suppressed frames did NOT accrue toward it) — with a changed next
        // so only the flat-verified rule can fire.
        let observations =
            window.push_frame(&ledger_with(TaskLedgerStatus::Active, "different", 0, 0));
        assert!(observations
            .iter()
            .all(|observation| !matches!(observation, StallObservation::NextUnchanged { .. })));
    }

    #[test]
    fn noop_seam_frames_record_without_revision() {
        // The scheduler success batch (Advanced, nothing unresolved, no
        // recommended next) reduces to zero ops; the observation window
        // must still see it — "nothing changed" IS the observation.
        let mut window = StallWindow::default();
        let ledger = ledger_with(TaskLedgerStatus::Active, "n", 1, 0);
        for _ in 0..2 {
            let out = window.push_frame(&ledger);
            assert!(out.is_empty(), "partial window observes nothing");
        }
        let out = window.push_frame(&ledger);
        assert!(out
            .iter()
            .any(|observation| matches!(observation, StallObservation::NextUnchanged { .. })));
    }

    #[test]
    fn run_started_resets_the_window() {
        let mut window = StallWindow::default();
        window.push_frame(&ledger_with(TaskLedgerStatus::Active, "old", 0, 0));
        window.push_frame(&ledger_with(TaskLedgerStatus::Active, "old", 0, 0));
        // Resume: fresh baseline, prior evidence wiped. `record` contributes
        // the first post-resume frame, so one more push leaves only two.
        let out = window.record(&ledger_with(TaskLedgerStatus::Active, "new", 0, 0));
        assert!(out.is_empty());
        let out = window.push_frame(&ledger_with(TaskLedgerStatus::Active, "new", 0, 0));
        assert!(out.is_empty(), "only two frames since resume");
    }

    #[test]
    fn verified_growing_with_same_next_is_flagged_separately() {
        let mut window = StallWindow::default();
        let mut out = Vec::new();
        for verified in 0..WINDOW {
            out = window.push_frame(&ledger_with(TaskLedgerStatus::Active, "same", verified, 0));
        }
        assert!(out.iter().any(|observation| matches!(
            observation,
            StallObservation::VerifiedGrowingButNextUnchanged { .. }
        )));
        // …and the depth-task protection: no NoNewVerification false positive.
        assert!(!out
            .iter()
            .any(|observation| matches!(observation, StallObservation::NoNewVerification { .. })));
    }

    #[test]
    fn automatic_replanning_is_bounded_and_blocks_after_budget() {
        let ledger = ledger_with(TaskLedgerStatus::Active, "same", 0, 0);
        let mut control = SessionStallControl::default();
        let _ = control.evaluate(&ledger, true, 0, true, false);
        let _ = control.evaluate(&ledger, false, 1, true, false);
        let suspected = control.evaluate(&ledger, false, 2, true, false);
        assert_eq!(suspected.phase, StallPhase::Suspected);
        let first = control.evaluate(&ledger, false, 3, true, false);
        assert!(matches!(
            first.action,
            StallAction::Replan { attempt: 1, .. }
        ));

        let mut replanned = ledger.clone();
        replanned.next.as_mut().unwrap().statement = "replan one".to_string();
        control.rebaseline(&replanned);
        for now in 4..6 {
            let _ = control.evaluate(&replanned, false, now, true, false);
        }
        let second = control.evaluate(&replanned, false, 6, true, false);
        assert!(matches!(
            second.action,
            StallAction::Replan { attempt: 2, .. }
        ));

        replanned.next.as_mut().unwrap().statement = "replan two".to_string();
        control.rebaseline(&replanned);
        for now in 8..10 {
            let _ = control.evaluate(&replanned, false, now, true, false);
        }
        let exhausted = control.evaluate(&replanned, false, 10, true, false);
        assert_eq!(exhausted.phase, StallPhase::Blocked);
        assert!(matches!(exhausted.action, StallAction::Block { .. }));
    }

    #[test]
    fn replan_action_respects_enablement_deadline_and_cancellation() {
        let ledger = ledger_with(TaskLedgerStatus::Active, "same", 0, 0);
        for (enabled, cancelled, now, expected_action) in [
            (false, false, 3, "none"),
            (true, true, 3, "none"),
            (true, false, REPLAN_DEADLINE_MS, "block"),
        ] {
            let mut control = SessionStallControl::default();
            let _ = control.evaluate(&ledger, true, 0, enabled, cancelled);
            let _ = control.evaluate(&ledger, false, 1, enabled, cancelled);
            let _ = control.evaluate(&ledger, false, 2, enabled, cancelled);
            let decision = control.evaluate(&ledger, false, now, enabled, cancelled);
            match expected_action {
                "none" => assert!(matches!(decision.action, StallAction::None)),
                "block" => assert!(matches!(decision.action, StallAction::Block { .. })),
                _ => unreachable!(),
            }
        }
    }

    async fn state_with_governed_session() -> (Arc<ServerState>, String, SessionTaskLedger) {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/stall-replan").id.clone()
        };
        let (ledger, _) = super::super::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            TaskLedgerOp::Create {
                goal: agendao_types::task_ledger::TaskGoal {
                    statement: "escape stall".to_string(),
                    acceptance_criteria: vec![],
                    criterion_checks: vec![],
                    set_by: TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "same next".to_string(),
            },
        )
        .await
        .expect("create ledger");
        (state, session_id, ledger)
    }

    #[tokio::test]
    async fn repeated_stall_commits_new_next_and_enqueues_one_boundary_replan() {
        let (state, session_id, ledger) = state_with_governed_session().await;
        let token = tokio_util::sync::CancellationToken::new();
        record_stall_frame(&state, &ledger, true, true, Some(&token)).await;
        for _ in 0..3 {
            record_stall_frame(&state, &ledger, false, true, Some(&token)).await;
        }
        let updated = super::super::task_ledger::task_ledger_snapshot(&state, &session_id)
            .await
            .expect("updated ledger");
        assert!(updated
            .next
            .as_ref()
            .expect("next")
            .statement
            .starts_with("Replan attempt 1:"));
        assert_eq!(
            state.steering_store.lock().await.pending_count(&session_id),
            1
        );
    }

    #[tokio::test]
    async fn cancelled_run_never_commits_or_enqueues_replan() {
        let (state, session_id, ledger) = state_with_governed_session().await;
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        record_stall_frame(&state, &ledger, true, true, Some(&token)).await;
        for _ in 0..3 {
            record_stall_frame(&state, &ledger, false, true, Some(&token)).await;
        }
        let unchanged = super::super::task_ledger::task_ledger_snapshot(&state, &session_id)
            .await
            .expect("ledger");
        assert_eq!(unchanged.revision, ledger.revision);
        assert_eq!(
            state.steering_store.lock().await.pending_count(&session_id),
            0
        );
    }

    #[tokio::test]
    async fn cancellation_while_waiting_for_session_lock_prevents_replan_commit() {
        let (state, session_id, ledger) = state_with_governed_session().await;
        let token = tokio_util::sync::CancellationToken::new();
        record_stall_frame(&state, &ledger, true, true, Some(&token)).await;
        record_stall_frame(&state, &ledger, false, true, Some(&token)).await;
        record_stall_frame(&state, &ledger, false, true, Some(&token)).await;

        let guard = state.sessions.lock().await;
        let task_state = Arc::clone(&state);
        let task_ledger = ledger.clone();
        let task_token = token.clone();
        let task = tokio::spawn(async move {
            record_stall_frame(&task_state, &task_ledger, false, true, Some(&task_token)).await;
        });
        tokio::task::yield_now().await;
        token.cancel();
        drop(guard);
        task.await.expect("stall task");

        let unchanged = super::super::task_ledger::task_ledger_snapshot(&state, &session_id)
            .await
            .expect("ledger");
        assert_eq!(unchanged.revision, ledger.revision);
        assert_eq!(
            state.steering_store.lock().await.pending_count(&session_id),
            0
        );
    }
}
