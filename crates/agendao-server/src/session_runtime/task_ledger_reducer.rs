// NO indiscriminate open-closing: a passing criterion check proves the
// bound criteria, not that every open question (e.g. a mid-run tool
// failure) was resolved. Questions close only through their own typed
// evidence (CloseOpenWithCheckpoint via the authority API with an
// explicit user confirmation, or a future checker bound to that
// question). Completion therefore stays honestly blocked while any
// question remains open — the delivery gate reports it.

//! Seam reducer: typed execution facts → ledger candidate ops.
//!
//! Candidates exist only inside one `apply_batch` transaction. There is no
//! candidate table, queue, or API; a candidate that fails validation is
//! logged and dropped, and a fact against an uncreated ledger is a no-op
//! (governance is opt-in per session, not imposed on every run).

use std::sync::Arc;

use agendao_types::repair::{ToolBatchGoalStatus, ToolBatchSummary};
use agendao_types::task_ledger::{
    completion_ready, current_checkpoints, missing_acceptance_criteria, AwaitingInteractionKind,
    AwaitingInteractionRef, SessionTaskLedger, TaskLedgerActor, TaskLedgerCause, TaskLedgerOp,
    TaskLedgerSeamFact, TaskLedgerStatus, VerificationCoverage, VerifierRef,
};

use crate::ServerState;

/// Pure candidate generation — same input always yields the same ops.
pub(crate) fn reduce(fact: &TaskLedgerSeamFact, snapshot: &SessionTaskLedger) -> Vec<TaskLedgerOp> {
    // Governance is opt-in: without a created ledger, facts are noise.
    if snapshot.goal.is_none() {
        return Vec::new();
    }
    match fact {
        TaskLedgerSeamFact::RunStarted => {
            // An interrupted run resuming is a state change; a fresh active
            // run is not. Pre-interrupt next keeps its marker until the run
            // commits a new one.
            if snapshot.status == TaskLedgerStatus::Interrupted {
                vec![TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::Active,
                    awaiting: None,
                    blocked_reason: None,
                }]
            } else {
                Vec::new()
            }
        }
        TaskLedgerSeamFact::ToolBatchCompleted { summary } => reduce_tool_batch(summary, snapshot),
        TaskLedgerSeamFact::RecoveryInterrupted => {
            // Idempotent: abort dispatches from the cancel path AND the run
            // tail; the second observation must not bump the revision.
            if snapshot.status == TaskLedgerStatus::Interrupted {
                Vec::new()
            } else {
                vec![TaskLedgerOp::Interrupt]
            }
        }
        TaskLedgerSeamFact::FinalResponseCommitted => {
            // The domain completion gate is the sole definition of readiness.
            // Historical or superseded evidence cannot satisfy it.
            if completion_ready(snapshot) && snapshot.status != TaskLedgerStatus::Completed {
                vec![TaskLedgerOp::Complete {
                    uncovered: Vec::new(),
                }]
            } else {
                Vec::new()
            }
        }
        TaskLedgerSeamFact::EvaluatorGateCompleted {
            node_path,
            passed,
            goal_generation,
        } => {
            if *passed && *goal_generation == snapshot.goal_generation {
                vec![TaskLedgerOp::AddCheckpoint {
                    claim: format!("scheduler evaluation passed at `{node_path}`"),
                    verifier: VerifierRef::Evaluator {
                        name: "scheduler-engine".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: format!("scheduler node evaluation: {node_path}"),
                    },
                    // Model-judge approval is useful review evidence, but it
                    // cannot prove a named acceptance criterion. Criterion
                    // coverage requires a deterministic check or explicit
                    // user confirmation through the authority API.
                    covered_criteria: Vec::new(),
                    evidence_artifact_ids: Vec::new(),
                    source_stage_id: Some(node_path.clone()),
                    supersedes: None,
                }]
            } else {
                Vec::new()
            }
        }
        TaskLedgerSeamFact::InteractionAwaiting {
            kind,
            interaction_id,
        } => {
            let reference = AwaitingInteractionRef {
                kind: *kind,
                interaction_id: interaction_id.clone(),
            };
            if snapshot.awaiting_interactions.contains(&reference) {
                Vec::new()
            } else {
                vec![TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::AwaitingUser,
                    awaiting: Some(reference),
                    blocked_reason: None,
                }]
            }
        }
        TaskLedgerSeamFact::InteractionResolved {
            kind,
            interaction_id,
        } => {
            let tracked = snapshot
                .awaiting_interactions
                .iter()
                .any(|current| current.kind == *kind && current.interaction_id == *interaction_id);
            if tracked {
                vec![TaskLedgerOp::ResolveInteraction {
                    kind: *kind,
                    interaction_id: interaction_id.clone(),
                }]
            } else {
                Vec::new()
            }
        }
    }
}

/// Fire-and-forget interaction lifecycle dispatch used by the permission
/// and question authorities.
pub(crate) async fn dispatch_interaction(
    state: &Arc<ServerState>,
    session_id: &str,
    kind: AwaitingInteractionKind,
    interaction_id: &str,
    resolved: bool,
) {
    let fact = if resolved {
        TaskLedgerSeamFact::InteractionResolved {
            kind,
            interaction_id: interaction_id.to_string(),
        }
    } else {
        TaskLedgerSeamFact::InteractionAwaiting {
            kind,
            interaction_id: interaction_id.to_string(),
        }
    };
    dispatch_seam(state, session_id, fact).await;
}

fn reduce_tool_batch(
    summary: &ToolBatchSummary,
    snapshot: &SessionTaskLedger,
) -> Vec<TaskLedgerOp> {
    let mut ops = Vec::new();

    // Unresolved items become open questions — deduped against what is
    // already open so a persistent blocker does not pile up duplicates.
    for unresolved in &summary.unresolved_items {
        let already_open = snapshot
            .open_questions()
            .iter()
            .any(|question| question.question == *unresolved);
        if already_open {
            continue;
        }
        let settled_by = summary
            .pending_follow_up
            .first()
            .map(|follow_up| follow_up.text.clone())
            .unwrap_or_else(|| "next tool batch resolving this item".to_string());
        ops.push(TaskLedgerOp::OpenQuestion {
            question: unresolved.clone(),
            settled_by,
        });
    }

    if let Some(next_step) = summary.recommended_next_step.as_deref() {
        if !next_step.trim().is_empty() {
            ops.push(TaskLedgerOp::SetNext {
                statement: next_step.to_string(),
                actor: Some(TaskLedgerActor::Model),
            });
        }
    }

    // A fully blocked batch moves the ledger to blocked — but only once the
    // batch offered a next action (invariant 4), which is why SetNext runs
    // first above.
    if summary.goal_status == ToolBatchGoalStatus::Blocked && !summary.blocked_by.is_empty() {
        let reason = summary
            .blocked_by
            .first()
            .map(|reason| format!("{reason:?}"))
            .unwrap_or_else(|| "tool batch blocked".to_string());
        ops.push(TaskLedgerOp::SetStatus {
            status: TaskLedgerStatus::Blocked,
            awaiting: None,
            blocked_reason: Some(reason),
        });
    }

    ops
}

/// Commit one seam: reduce → single-transaction apply (one revision, one
/// canonical event via the authority's op path when the batch is empty).
/// Returns the new snapshot when a revision was committed.
pub(crate) async fn dispatch_seam(
    state: &Arc<ServerState>,
    session_id: &str,
    fact: TaskLedgerSeamFact,
) -> Option<SessionTaskLedger> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    let snapshot = super::task_ledger::ledger_snapshot_from_record(
        session_id,
        session
            .record()
            .metadata
            .get(super::task_ledger::TASK_LEDGER_METADATA_KEY),
    );
    let ops = reduce(&fact, &snapshot);
    if ops.is_empty() {
        // Only execution-progress facts feed the stall window when they are
        // no-ops: "the batch changed nothing" IS the observation. Other
        // no-op facts (stale evaluator, unknown interaction resolution, a
        // not-yet-earned completion, repeated interrupt) must NOT — a run
        // of those would fabricate stall evidence.
        let feeds_window = matches!(
            fact,
            TaskLedgerSeamFact::ToolBatchCompleted { .. } | TaskLedgerSeamFact::RunStarted
        );
        if feeds_window {
            let run_started = matches!(fact, TaskLedgerSeamFact::RunStarted);
            let frame_ledger = snapshot.clone();
            drop(sessions);
            super::task_ledger_stall::record_stall_frame(state, &frame_ledger, run_started).await;
        }
        return None;
    }
    let expected = snapshot.revision;
    let mut staged = snapshot.clone();
    if let Err(error) = staged.apply_batch(expected, ops, now_ms) {
        tracing::warn!(
            session_id,
            fact = ?fact,
            error = %error,
            "task ledger seam candidates rejected; nothing committed"
        );
        return None;
    }
    let value = serde_json::to_value(&staged).ok()?;
    session.insert_metadata(
        super::task_ledger::TASK_LEDGER_METADATA_KEY.to_string(),
        value,
    );
    // Broadcast under the write lock (see authority): revision order on the
    // bus matches commit order.
    let cause = super::task_ledger::cause_for_seam(&fact);
    crate::session_runtime::events::broadcast_server_event(
        state,
        &agendao_server_core::runtime_events::ServerEvent::TaskLedgerReplaced {
            session_id: session_id.to_string(),
            ledger: staged.clone(),
            cause,
        },
    );
    drop(sessions);

    crate::routes::session::session_crud::persist_session_if_enabled(state, session_id).await;
    let run_started = matches!(
        fact,
        agendao_types::task_ledger::TaskLedgerSeamFact::RunStarted
    );
    super::task_ledger_stall::record_stall_frame(state, &staged, run_started).await;
    Some(staged)
}

/// Typed final-delivery gate. Checks reference integrity only — never
/// rewrites the user's answer — and reports what it found.
pub(crate) struct DeliveryGateReport {
    pub open_questions_outstanding: Vec<String>,
    pub no_verified_checkpoints: bool,
    pub missing_acceptance_criteria: Vec<String>,
    pub uncovered_criteria: Vec<String>,
}

/// Deterministic criterion verifier (Phase 3 completion piece).
///
/// Runs the goal's `criterion_checks` commands in the session workspace at
/// the final-response seam. Exit 0 on a named criterion is the ONLY
/// automated path that produces criterion-covering evidence; a model judge
/// can never cover a named criterion (falsified 2026-08-18: an agent that
/// completed the task in the wrong workspace still got a model PASS).
///
/// When every bound check passes, the resulting checkpoints cover only their
/// named criteria. Open questions remain open until their own typed evidence
/// closes them. Any failure or cancellation leaves the ledger untouched; the
/// delivery gate reports the missing evidence honestly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CriterionVerificationOutcome {
    /// There is no ledger/check to run, or the ledger is already complete.
    NotRequired,
    /// Every bound check passed and its checkpoints were committed.
    Passed,
    /// A check failed, timed out, could not run, or could not commit atomically.
    Failed,
    /// The owning run was cancelled before evidence could be committed.
    Cancelled,
}

impl CriterionVerificationOutcome {
    pub(crate) fn allows_final_commit(&self) -> bool {
        matches!(self, Self::NotRequired | Self::Passed)
    }
}

pub(crate) async fn verify_goal_criteria(
    state: &Arc<ServerState>,
    session_id: &str,
    cancellation: tokio_util::sync::CancellationToken,
) -> CriterionVerificationOutcome {
    if cancellation.is_cancelled() {
        return CriterionVerificationOutcome::Cancelled;
    }
    let (goal, workspace_dir, revision) = {
        let sessions = state.sessions.lock().await;
        let Some(session) = sessions.get(session_id) else {
            return CriterionVerificationOutcome::NotRequired;
        };
        let snapshot = super::task_ledger::ledger_snapshot_from_record(
            session_id,
            session
                .record()
                .metadata
                .get(super::task_ledger::TASK_LEDGER_METADATA_KEY),
        );
        let Some(goal) = snapshot.goal.clone() else {
            return CriterionVerificationOutcome::NotRequired;
        };
        if goal.criterion_checks.is_empty() || snapshot.status == TaskLedgerStatus::Completed {
            return CriterionVerificationOutcome::NotRequired;
        }
        (goal, session.record().directory.clone(), snapshot.revision)
    };

    let mut ops: Vec<TaskLedgerOp> = Vec::new();
    for check in &goal.criterion_checks {
        // Execution model: the command was bound to the criterion by the
        // user through the authority API — that binding IS the explicit
        // authorization. The verifier lives INSIDE the run lifecycle: the
        // scheduler cancellation token participates, so Stop cancels a
        // running check exactly like it cancels the run. Timeout and cancel
        // both use the platform termination path and reap the immediate child;
        // Unix additionally terminates the whole process group.
        let mut child = match spawn_criterion_check(
            &check.command,
            std::path::Path::new(&workspace_dir),
        ) {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(criterion = %check.criterion, %error, "criterion check failed to spawn");
                return CriterionVerificationOutcome::Failed;
            }
        };
        let passed = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                tracing::info!(criterion = %check.criterion, "criterion check cancelled by run abort");
                terminate_criterion_check(&mut child).await;
                return CriterionVerificationOutcome::Cancelled;
            }
            status = child.wait() => match status {
                Ok(status) => status.success(),
                Err(error) => {
                    tracing::warn!(criterion = %check.criterion, %error, "criterion check wait failed");
                    return CriterionVerificationOutcome::Failed;
                }
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                tracing::warn!(criterion = %check.criterion, "criterion check timed out; terminating process tree");
                terminate_criterion_check(&mut child).await;
                false
            }
        };
        tracing::info!(
            session_id,
            criterion = %check.criterion,
            command = %check.command,
            passed,
            "deterministic criterion check executed"
        );
        if !passed {
            // One failed check means the evidence chain stops here: no
            // completion, no open-closing. The gate reports the gap.
            return CriterionVerificationOutcome::Failed;
        }
        ops.push(TaskLedgerOp::AddCheckpoint {
            claim: format!("deterministic check passed: {}", check.command),
            verifier: VerifierRef::DeterministicCheck {
                description: check.command.clone(),
            },
            coverage: VerificationCoverage {
                scope: format!("exit 0 in {}", workspace_dir),
            },
            covered_criteria: vec![check.criterion.clone()],
            evidence_artifact_ids: Vec::new(),
            source_stage_id: None,
            supersedes: None,
        });
    }

    // Cancellation may arrive after the final child exits but before the
    // evidence transaction starts. Do not turn that race into a checkpoint.
    if cancellation.is_cancelled() {
        return CriterionVerificationOutcome::Cancelled;
    }

    // NO indiscriminate open-closing: a passing criterion check proves the
    // bound criteria, not that every open question (e.g. a mid-run tool
    // failure) was resolved. Questions close only through their own typed
    // evidence (CloseOpenWithCheckpoint with an explicit user confirmation
    // via the authority API). Completion stays honestly blocked while any
    // question remains open — the delivery gate reports it.

    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut sessions = state.sessions.lock().await;
    if cancellation.is_cancelled() {
        return CriterionVerificationOutcome::Cancelled;
    }
    let Some(session) = sessions.get_mut(session_id) else {
        return CriterionVerificationOutcome::Failed;
    };
    let mut staged = super::task_ledger::ledger_snapshot_from_record(
        session_id,
        session
            .record()
            .metadata
            .get(super::task_ledger::TASK_LEDGER_METADATA_KEY),
    );
    if staged.apply_batch(revision, ops, now_ms).is_err() {
        return CriterionVerificationOutcome::Failed;
    }
    let Ok(value) = serde_json::to_value(&staged) else {
        return CriterionVerificationOutcome::Failed;
    };
    session.insert_metadata(
        super::task_ledger::TASK_LEDGER_METADATA_KEY.to_string(),
        value,
    );
    // Broadcast under the lock (revision order == event order), then drop
    // the guard BEFORE persisting: flush_session_to_storage re-acquires the
    // sessions mutex, so persisting under the lock self-deadlocks the
    // server (bit once in production; invisible in unit tests because
    // storage-less ServerState::new() early-returns before the lock).
    crate::session_runtime::events::broadcast_server_event(
        state,
        &agendao_server_core::runtime_events::ServerEvent::TaskLedgerReplaced {
            session_id: session_id.to_string(),
            ledger: staged.clone(),
            cause: TaskLedgerCause::CheckpointAdded,
        },
    );
    drop(sessions);
    crate::routes::session::session_crud::persist_session_if_enabled(state, session_id).await;
    CriterionVerificationOutcome::Passed
}

/// Platform-specific criterion-check process management.
///
/// Unix: dedicated process group, so termination can SIGKILL the whole tree
/// (bash -c children included). Windows: `cmd /C` with kill-on-drop; full
/// tree governance would require Job Objects (documented follow-up in the
/// governance plan).
#[cfg(unix)]
fn spawn_criterion_check(
    command: &str,
    workspace_dir: &std::path::Path,
) -> std::io::Result<tokio::process::Child> {
    tokio::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .current_dir(workspace_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
}

#[cfg(windows)]
fn spawn_criterion_check(
    command: &str,
    workspace_dir: &std::path::Path,
) -> std::io::Result<tokio::process::Child> {
    // Exit code is the only signal; output goes to null so a chatty check
    // can never flood memory on either platform.
    tokio::process::Command::new("cmd")
        .args(["/C", command])
        .current_dir(workspace_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// Terminate the whole check process tree and reap it — used by BOTH the
/// timeout and the cancellation path so no path leaks an unreaped child.
#[cfg(unix)]
async fn terminate_criterion_check(child: &mut tokio::process::Child) {
    if let Some(pgid) = child.id() {
        // Negative pid = the whole process group (bash children included).
        unsafe {
            libc::kill(-(pgid as i32), libc::SIGKILL);
        }
    }
    let _ = child.wait().await;
}

#[cfg(windows)]
async fn terminate_criterion_check(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

pub(crate) fn final_delivery_gate(snapshot: &SessionTaskLedger) -> DeliveryGateReport {
    DeliveryGateReport {
        open_questions_outstanding: snapshot
            .open_questions()
            .iter()
            .map(|question| format!("{}: {}", question.id, question.question))
            .collect(),
        no_verified_checkpoints: current_checkpoints(snapshot).is_empty(),
        missing_acceptance_criteria: missing_acceptance_criteria(
            snapshot,
            &snapshot.uncovered_criteria,
        ),
        uncovered_criteria: snapshot.uncovered_criteria.clone(),
    }
}

/// Model-visible projection: fixed, compact, typed. This is the ONLY shape
/// in which the ledger enters a prompt, and it is injected at exactly one
/// place (the scheduler conversation seed).
pub(crate) fn render_ledger_projection(snapshot: &SessionTaskLedger) -> Option<String> {
    if snapshot.revision == 0 || snapshot.goal.is_none() {
        return None;
    }
    let mut lines = vec![format!(
        "<task-ledger revision=\"{}\" status=\"{:?}\">",
        snapshot.revision, snapshot.status
    )];
    if let Some(goal) = &snapshot.goal {
        lines.push(format!("Goal: {}", goal.statement));
        if !goal.acceptance_criteria.is_empty() {
            lines.push(format!("Accept: {}", goal.acceptance_criteria.join(" | ")));
        }
    }
    for entry in snapshot.live_core() {
        lines.push(format!("Core(live): {}", entry.statement));
    }
    if let Some(latest) = current_checkpoints(snapshot).last() {
        lines.push(format!(
            "Verified[{}]: {} (by {}, covered: {})",
            latest.id,
            latest.claim,
            latest.verifier.describe(),
            latest.coverage.scope
        ));
    }
    for question in snapshot.open_questions().iter().take(3) {
        lines.push(format!(
            "Open[{}]: {} — settled by: {}",
            question.id, question.question, question.settled_by
        ));
    }
    if let Some(next) = &snapshot.next {
        lines.push(format!("Next: {}", next.statement));
    }
    lines.push("</task-ledger>".to_string());
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::task_ledger::TaskGoal;

    async fn async_state_with_session() -> Arc<ServerState> {
        let state = Arc::new(ServerState::new());
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp");
        drop(sessions);
        state
    }

    async fn async_first_session_id(state: &Arc<ServerState>) -> String {
        let sessions = state.sessions.lock().await;
        sessions
            .list()
            .first()
            .map(|session| session.record().id.clone())
            .expect("session")
    }

    #[cfg(unix)]
    fn passing_check_command() -> &'static str {
        "true"
    }

    #[cfg(windows)]
    fn passing_check_command() -> &'static str {
        "exit /B 0"
    }

    #[cfg(unix)]
    fn failing_check_command() -> &'static str {
        "false"
    }

    #[cfg(windows)]
    fn failing_check_command() -> &'static str {
        "exit /B 1"
    }

    #[cfg(unix)]
    fn long_running_check_command() -> &'static str {
        "sleep 60"
    }

    #[cfg(windows)]
    fn long_running_check_command() -> &'static str {
        // `for` and `ver` are cmd builtins, so killing cmd does not leave a
        // helper process behind while Windows Job Object support is pending.
        "for /L %i in (1,1,2147483647) do @ver >NUL"
    }

    use agendao_types::repair::{ToolBatchFollowUpItem, ToolBatchGoalStatus};
    use agendao_types::task_ledger::TaskLedgerActor;

    fn snapshot() -> SessionTaskLedger {
        let mut ledger = SessionTaskLedger::empty("ses_reducer");
        ledger
            .apply(
                0,
                TaskLedgerOp::Create {
                    goal: TaskGoal {
                        statement: "ship median".to_string(),
                        acceptance_criteria: vec![],
                        criterion_checks: vec![],
                        set_by: TaskLedgerActor::User,
                        set_at: 1,
                    },
                    next_statement: "write tests".to_string(),
                },
                1,
            )
            .unwrap();
        ledger
    }

    fn batch(
        goal_status: ToolBatchGoalStatus,
        unresolved: Vec<&str>,
        next: Option<&str>,
    ) -> TaskLedgerSeamFact {
        TaskLedgerSeamFact::ToolBatchCompleted {
            summary: ToolBatchSummary {
                tools_used: vec!["bash".to_string()],
                success_count: 1,
                error_count: 0,
                error_kinds: vec![],
                goal_status,
                blocked_by: vec![],
                artifacts_created: vec![],
                pending_follow_up: vec![ToolBatchFollowUpItem {
                    kind: "retry".to_string(),
                    text: "rerun with diagnosis".to_string(),
                }],
                unresolved_items: unresolved.into_iter().map(String::from).collect(),
                recommended_next_step: next.map(String::from),
                repair_events: vec![],
            },
        }
    }

    #[test]
    fn facts_against_uncreated_ledger_are_noops() {
        let empty = SessionTaskLedger::empty("ses_x");
        assert!(reduce(
            &batch(ToolBatchGoalStatus::Blocked, vec!["x"], None),
            &empty
        )
        .is_empty());
    }

    #[test]
    fn unresolved_becomes_open_and_next_rides_model_provenance() {
        let snap = snapshot();
        let ops = reduce(
            &batch(
                ToolBatchGoalStatus::Mixed,
                vec!["flaky test"],
                Some("rerun tests"),
            ),
            &snap,
        );
        assert_eq!(ops.len(), 2);
        assert!(
            matches!(&ops[0], TaskLedgerOp::OpenQuestion { question, .. } if question == "flaky test")
        );
        assert!(matches!(
            &ops[1],
            TaskLedgerOp::SetNext { statement, actor: Some(TaskLedgerActor::Model) }
                if statement == "rerun tests"
        ));
    }

    #[test]
    fn duplicate_unresolved_does_not_pile_up() {
        let mut snap = snapshot();
        snap.apply_batch(
            1,
            vec![TaskLedgerOp::OpenQuestion {
                question: "flaky test".to_string(),
                settled_by: "x".to_string(),
            }],
            2,
        )
        .unwrap();
        let ops = reduce(
            &batch(ToolBatchGoalStatus::Mixed, vec!["flaky test"], None),
            &snap,
        );
        assert!(ops.is_empty(), "already open, nothing new");
    }

    #[test]
    fn interaction_awaiting_and_resolved_round_trip() {
        use agendao_types::task_ledger::AwaitingInteractionKind;
        let mut snap = snapshot();
        let awaiting = TaskLedgerSeamFact::InteractionAwaiting {
            kind: AwaitingInteractionKind::Permission,
            interaction_id: "permission_1".to_string(),
        };
        let ops = reduce(&awaiting, &snap);
        assert!(matches!(
            &ops[0],
            TaskLedgerOp::SetStatus {
                status: TaskLedgerStatus::AwaitingUser,
                ..
            }
        ));
        snap.apply_batch(1, ops, 2).unwrap();
        assert_eq!(snap.status, TaskLedgerStatus::AwaitingUser);
        // Same interaction again: idempotent, no revision churn.
        assert!(reduce(&awaiting, &snap).is_empty());
        // Resolving flips back to active only for the matching reference.
        let wrong = TaskLedgerSeamFact::InteractionResolved {
            kind: AwaitingInteractionKind::Question,
            interaction_id: "permission_1".to_string(),
        };
        assert!(reduce(&wrong, &snap).is_empty());
        let right = TaskLedgerSeamFact::InteractionResolved {
            kind: AwaitingInteractionKind::Permission,
            interaction_id: "permission_1".to_string(),
        };
        let ops = reduce(&right, &snap);
        snap.apply_batch(2, ops, 3).unwrap();
        assert_eq!(snap.status, TaskLedgerStatus::Active);
        assert!(snap.awaiting_interactions.is_empty());
    }

    #[test]
    fn interrupt_is_idempotent_and_final_commit_needs_clean_evidence() {
        let mut snap = snapshot();
        let interrupt = TaskLedgerSeamFact::RecoveryInterrupted;
        let ops = reduce(&interrupt, &snap);
        snap.apply_batch(1, ops, 2).unwrap();
        assert_eq!(snap.status, TaskLedgerStatus::Interrupted);
        // Second dispatch (run tail after cancel path) must be a no-op.
        assert!(reduce(&interrupt, &snap).is_empty());
        // Final commit without checkpoints: no auto-complete.
        assert!(reduce(&TaskLedgerSeamFact::FinalResponseCommitted, &snap).is_empty());
        // With a checkpoint and nothing open: completes.
        snap.apply_batch(
            2,
            vec![TaskLedgerOp::AddCheckpoint {
                claim: "done".to_string(),
                verifier: agendao_types::task_ledger::VerifierRef::DeterministicCheck {
                    description: "unittest".to_string(),
                },
                coverage: agendao_types::task_ledger::VerificationCoverage {
                    scope: "all cases".to_string(),
                },
                covered_criteria: vec![],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
            }],
            3,
        )
        .unwrap();
        let ops = reduce(&TaskLedgerSeamFact::FinalResponseCommitted, &snap);
        assert!(ops.len() == 1);
        assert!(matches!(&ops[0], TaskLedgerOp::Complete { uncovered } if uncovered.is_empty()));
    }

    #[test]
    fn final_commit_and_delivery_report_use_current_criterion_coverage() {
        let mut snap = snapshot();
        snap.apply(
            1,
            TaskLedgerOp::SetGoal {
                goal: TaskGoal {
                    statement: "ship median".to_string(),
                    acceptance_criteria: vec!["all tests pass".to_string()],
                    criterion_checks: vec![],
                    set_by: TaskLedgerActor::User,
                    set_at: 2,
                },
            },
            2,
        )
        .unwrap();
        snap.apply(
            2,
            TaskLedgerOp::AddCheckpoint {
                claim: "formatting passed".to_string(),
                verifier: agendao_types::task_ledger::VerifierRef::DeterministicCheck {
                    description: "cargo fmt --check".to_string(),
                },
                coverage: agendao_types::task_ledger::VerificationCoverage {
                    scope: "Rust formatting".to_string(),
                },
                covered_criteria: vec![],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
            },
            3,
        )
        .unwrap();

        assert!(reduce(&TaskLedgerSeamFact::FinalResponseCommitted, &snap).is_empty());
        let report = final_delivery_gate(&snap);
        assert!(!report.no_verified_checkpoints);
        assert_eq!(
            report.missing_acceptance_criteria,
            vec!["all tests pass".to_string()]
        );
        assert!(report.uncovered_criteria.is_empty());
    }

    #[test]
    fn evaluator_pass_creates_checkpoint_without_criterion_mapping() {
        let snap = snapshot();
        let pass = TaskLedgerSeamFact::EvaluatorGateCompleted {
            node_path: "verify/main".to_string(),
            passed: true,
            goal_generation: snap.goal_generation,
        };
        let ops = reduce(&pass, &snap);
        assert!(matches!(
            &ops[0],
            TaskLedgerOp::AddCheckpoint {
                verifier: agendao_types::task_ledger::VerifierRef::Evaluator { .. },
                covered_criteria,
                source_stage_id,
                ..
            } if covered_criteria.is_empty() && source_stage_id.as_deref() == Some("verify/main")
        ));
        // A failed gate produces nothing.
        let fail = TaskLedgerSeamFact::EvaluatorGateCompleted {
            node_path: "verify/main".to_string(),
            passed: false,
            goal_generation: snap.goal_generation,
        };
        assert!(reduce(&fail, &snap).is_empty());

        let stale = TaskLedgerSeamFact::EvaluatorGateCompleted {
            node_path: "verify/main".to_string(),
            passed: true,
            goal_generation: snap.goal_generation + 1,
        };
        assert!(reduce(&stale, &snap).is_empty());
    }

    #[test]
    fn evaluator_pass_is_current_generation_review_but_not_completion_evidence() {
        let mut snap = snapshot();
        snap.apply(
            1,
            TaskLedgerOp::SetGoal {
                goal: TaskGoal {
                    statement: "ship median".to_string(),
                    acceptance_criteria: vec!["unittest passes".to_string()],
                    criterion_checks: vec![],
                    set_by: TaskLedgerActor::User,
                    set_at: 2,
                },
            },
            2,
        )
        .unwrap();
        let fact = TaskLedgerSeamFact::EvaluatorGateCompleted {
            node_path: "verify/main".to_string(),
            passed: true,
            goal_generation: snap.goal_generation,
        };
        let ops = reduce(&fact, &snap);
        snap.apply_batch(2, ops, 3).unwrap();
        assert_eq!(current_checkpoints(&snap).len(), 1);
        assert!(current_checkpoints(&snap)[0].covered_criteria.is_empty());
        assert!(!completion_ready(&snap));

        let stale = TaskLedgerSeamFact::EvaluatorGateCompleted {
            node_path: "verify/old".to_string(),
            passed: true,
            goal_generation: snap.goal_generation - 1,
        };
        assert!(reduce(&stale, &snap).is_empty());
    }

    #[test]
    fn scheduler_tool_batch_fact_maps_to_opens_and_status() {
        let mut snap = snapshot();
        let fact = TaskLedgerSeamFact::ToolBatchCompleted {
            summary: agendao_types::repair::ToolBatchSummary {
                tools_used: vec!["bash".to_string()],
                success_count: 1,
                error_count: 1,
                error_kinds: vec![],
                goal_status: agendao_types::repair::ToolBatchGoalStatus::Mixed,
                blocked_by: vec![],
                artifacts_created: vec![],
                pending_follow_up: vec![],
                unresolved_items: vec!["a tool call failed; diagnose before retrying".to_string()],
                recommended_next_step: Some("rerun with diagnosis".to_string()),
                repair_events: vec![],
            },
        };
        let ops = reduce(&fact, &snap);
        assert!(ops.len() >= 2);
        snap.apply_batch(1, ops, 2).unwrap();
        assert_eq!(snap.open_questions().len(), 1);
        assert_eq!(
            snap.next.as_ref().unwrap().provenance.actor,
            TaskLedgerActor::Model
        );
    }

    #[tokio::test]
    async fn criterion_verifier_files_covered_evidence_and_completes() {
        let state = async_state_with_session().await;
        let session_id = async_first_session_id(&state).await;
        // Goal with a bound check that trivially passes.
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            TaskLedgerOp::Create {
                goal: TaskGoal {
                    statement: "ship".to_string(),
                    acceptance_criteria: vec!["check passes".to_string()],
                    criterion_checks: vec![agendao_types::task_ledger::CriterionCheck {
                        criterion: "check passes".to_string(),
                        command: passing_check_command().to_string(),
                    }],
                    set_by: TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "work".to_string(),
            },
        )
        .await
        .expect("create");
        // An open tool-failure question the verification should settle.
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            1,
            TaskLedgerOp::OpenQuestion {
                question: "a tool call failed".to_string(),
                settled_by: "final verification".to_string(),
            },
        )
        .await
        .expect("open");

        let outcome = verify_goal_criteria(
            &state,
            &session_id,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        assert_eq!(outcome, CriterionVerificationOutcome::Passed);
        let staged = crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
            .await
            .expect("committed verifier snapshot");
        assert!(staged
            .verified
            .iter()
            .any(|checkpoint| checkpoint.covered_criteria == vec!["check passes".to_string()]));
        // P1-3 regression: a passing criterion check does NOT settle the
        // unrelated open question; completion stays blocked until the user
        // closes it explicitly with their own confirmation.
        assert_eq!(
            staged.open_questions().len(),
            1,
            "opens are NOT auto-closed by criterion verification"
        );
        assert!(!completion_ready(&staged));
        let ops = reduce(&TaskLedgerSeamFact::FinalResponseCommitted, &staged);
        assert!(ops.is_empty(), "completion blocked while an open remains");
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            staged.revision,
            TaskLedgerOp::CloseOpenWithCheckpoint {
                open_id: staged.open_questions()[0].id.clone(),
                claim: "failure was transient; user confirmed end state".to_string(),
                verifier: agendao_types::task_ledger::VerifierRef::UserConfirmation {
                    actor: "test-user".to_string(),
                },
                coverage: VerificationCoverage {
                    scope: "manual review of the failed call".to_string(),
                },
                covered_criteria: Vec::new(),
                evidence_artifact_ids: Vec::new(),
                source_stage_id: None,
            },
        )
        .await
        .expect("explicit close");
        let closed = crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
            .await
            .unwrap();
        assert!(completion_ready(&closed));
        // The completion seam can now finish the ledger.
        let ops = reduce(&TaskLedgerSeamFact::FinalResponseCommitted, &closed);
        assert!(matches!(ops.as_slice(), [TaskLedgerOp::Complete { .. }]));
    }

    #[tokio::test]
    async fn cancelled_run_stops_criterion_verification_immediately() {
        let state = async_state_with_session().await;
        let session_id = async_first_session_id(&state).await;
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            TaskLedgerOp::Create {
                goal: TaskGoal {
                    statement: "ship".to_string(),
                    acceptance_criteria: vec!["check passes".to_string()],
                    criterion_checks: vec![agendao_types::task_ledger::CriterionCheck {
                        criterion: "check passes".to_string(),
                        command: long_running_check_command().to_string(),
                    }],
                    set_by: TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "work".to_string(),
            },
        )
        .await
        .expect("create");
        let token = tokio_util::sync::CancellationToken::new();
        let task_state = state.clone();
        let task_session_id = session_id.clone();
        let task_token = token.clone();
        let task = tokio::spawn(async move {
            verify_goal_criteria(&task_state, &task_session_id, task_token).await
        });
        // Exercise cancellation while the child is actually running; a
        // pre-cancelled token would miss the post-spawn lifecycle entirely.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let started = std::time::Instant::now();
        token.cancel();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("cancel must preempt the long-running command")
            .expect("verifier task must not panic");
        assert_eq!(outcome, CriterionVerificationOutcome::Cancelled);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "cancel must preempt the 60s command"
        );
        let snapshot =
            crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
                .await
                .unwrap();
        assert!(snapshot.verified.is_empty(), "no checkpoint written");
    }

    #[tokio::test]
    async fn criterion_verifier_failure_blocks_the_chain() {
        let state = async_state_with_session().await;
        let session_id = async_first_session_id(&state).await;
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            TaskLedgerOp::Create {
                goal: TaskGoal {
                    statement: "ship".to_string(),
                    acceptance_criteria: vec!["check passes".to_string()],
                    criterion_checks: vec![agendao_types::task_ledger::CriterionCheck {
                        criterion: "check passes".to_string(),
                        command: failing_check_command().to_string(),
                    }],
                    set_by: TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "work".to_string(),
            },
        )
        .await
        .expect("create");
        assert_eq!(
            verify_goal_criteria(
                &state,
                &session_id,
                tokio_util::sync::CancellationToken::new(),
            )
            .await,
            CriterionVerificationOutcome::Failed,
            "a failed check commits nothing",
        );
        let snapshot =
            crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
                .await
                .unwrap();
        assert!(snapshot.verified.is_empty());
        assert!(!completion_ready(&snapshot));
    }

    #[tokio::test]
    async fn noop_fact_routing_feeds_window_only_for_execution_progress() {
        let state = async_state_with_session().await;
        let session_id = async_first_session_id(&state).await;
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            TaskLedgerOp::Create {
                goal: TaskGoal {
                    statement: "ship".to_string(),
                    acceptance_criteria: vec![],
                    criterion_checks: vec![],
                    set_by: TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "work".to_string(),
            },
        )
        .await
        .expect("create");

        // A success tool batch (no unresolved, no next) is a no-op whose
        // frame MUST land in the window.
        let success_batch = TaskLedgerSeamFact::ToolBatchCompleted {
            summary: agendao_types::repair::ToolBatchSummary {
                tools_used: vec!["read".to_string()],
                success_count: 1,
                error_count: 0,
                error_kinds: vec![],
                goal_status: agendao_types::repair::ToolBatchGoalStatus::Advanced,
                blocked_by: vec![],
                artifacts_created: vec![],
                pending_follow_up: vec![],
                unresolved_items: vec![],
                recommended_next_step: None,
                repair_events: vec![],
            },
        };
        dispatch_seam(&state, &session_id, success_batch.clone()).await;
        assert_eq!(state.stall_windows.frame_count(&session_id), 1);
        dispatch_seam(&state, &session_id, success_batch).await;
        assert_eq!(state.stall_windows.frame_count(&session_id), 2);

        // Non-progress no-ops must NOT feed the window: an unearned
        // completion, a failed evaluator, an unknown interaction resolution.
        for fact in [
            TaskLedgerSeamFact::FinalResponseCommitted,
            TaskLedgerSeamFact::EvaluatorGateCompleted {
                node_path: "v".to_string(),
                passed: false,
                goal_generation: 1,
            },
            TaskLedgerSeamFact::InteractionResolved {
                kind: AwaitingInteractionKind::Permission,
                interaction_id: "unknown".to_string(),
            },
        ] {
            dispatch_seam(&state, &session_id, fact).await;
        }
        assert_eq!(
            state.stall_windows.frame_count(&session_id),
            2,
            "non-progress no-ops fabricate no stall frames"
        );
    }

    #[test]
    fn projection_is_typed_and_omitted_for_uncreated_ledgers() {
        assert!(render_ledger_projection(&SessionTaskLedger::empty("x")).is_none());
        let text = render_ledger_projection(&snapshot()).unwrap();
        assert!(text.starts_with("<task-ledger revision=\"1\""));
        assert!(text.contains("Goal: ship median"));
        assert!(text.contains("Next: write tests"));
        assert!(text.ends_with("</task-ledger>"));
    }
}
