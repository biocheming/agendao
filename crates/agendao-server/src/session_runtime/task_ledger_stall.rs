//! Task-level stall observation (Phase 4 v1: observe and report only).
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
//! v1 never auto-replans: first hits become telemetry observations. Acting
//! on them (budgeted re-planning) is gated on Phase 6 evidence.

use std::collections::VecDeque;
use std::sync::Arc;

use agendao_types::task_ledger::{SessionTaskLedger, TaskLedgerStatus};

use crate::ServerState;

const WINDOW: usize = 3;

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
    windows: std::sync::Mutex<std::collections::HashMap<String, StallWindow>>,
}

impl StallWindows {
    /// Record a committed seam. `run_started` implements the resume-window
    /// reset. Returns the observations worth surfacing (possibly empty).
    pub fn record(
        &self,
        session_id: &str,
        ledger: &SessionTaskLedger,
        run_started: bool,
    ) -> Vec<StallObservation> {
        if ledger.revision == 0 {
            return Vec::new();
        }
        let mut windows = match self.windows.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let window = windows.entry(session_id.to_string()).or_default();
        if run_started {
            window.record(ledger)
        } else {
            window.push_frame(ledger)
        }
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
) {
    let observations = state
        .stall_windows
        .record(&ledger.session_id, ledger, run_started);
    if observations.is_empty() {
        return;
    }
    // v1 surfaces facts only — with named rules, never a vibe.
    tracing::info!(
        session_id = %ledger.session_id,
        observations = ?observations,
        "task stall observation (facts only, no action taken)"
    );
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(&ledger.session_id) {
        session.insert_metadata(
            "stall_observation".to_string(),
            serde_json::to_value(&observations).unwrap_or_default(),
        );
    }
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
}
