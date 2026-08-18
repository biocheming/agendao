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
    AwaitingInteractionRef, SessionTaskLedger, TaskLedgerActor, TaskLedgerOp, TaskLedgerSeamFact,
    TaskLedgerStatus,
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
    use agendao_types::repair::{ToolBatchFollowUpItem, ToolBatchGoalStatus};
    use agendao_types::task_ledger::{TaskGoal, TaskLedgerActor};

    fn snapshot() -> SessionTaskLedger {
        let mut ledger = SessionTaskLedger::empty("ses_reducer");
        ledger
            .apply(
                0,
                TaskLedgerOp::Create {
                    goal: TaskGoal {
                        statement: "ship median".to_string(),
                        acceptance_criteria: vec![],
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
    fn projection_is_typed_and_omitted_for_uncreated_ledgers() {
        assert!(render_ledger_projection(&SessionTaskLedger::empty("x")).is_none());
        let text = render_ledger_projection(&snapshot()).unwrap();
        assert!(text.starts_with("<task-ledger revision=\"1\""));
        assert!(text.contains("Goal: ship median"));
        assert!(text.contains("Next: write tests"));
        assert!(text.ends_with("</task-ledger>"));
    }
}
