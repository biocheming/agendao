//! Server-side authority for the session task ledger.
//!
//! Single authority rule: the session record's metadata IS the ledger. There
//! is no second registry to invalidate — reads parse the metadata snapshot,
//! writes validate through the domain invariants, bump the revision, persist,
//! and broadcast one canonical replacement event. Fork copies the snapshot
//! (rebound to the new session id, revision preserved); deleting a session
//! deletes its ledger with it.

use std::sync::Arc;

use agendao_types::task_ledger::{
    SessionTaskLedger, TaskLedgerCause, TaskLedgerError, TaskLedgerOp,
};

use crate::error::ApiError;
use crate::ServerState;

pub(crate) use agendao_types::task_ledger::TASK_LEDGER_METADATA_KEY;

pub(crate) fn map_ledger_error(session_id: &str, error: TaskLedgerError) -> ApiError {
    match error {
        TaskLedgerError::RevisionConflict { expected, actual } => ApiError::RevisionConflict {
            resource: format!("task-ledger:{session_id}"),
            expected,
            actual,
        },
        other => ApiError::BadRequest(other.to_string()),
    }
}

/// Read the ledger snapshot. Sessions without structured governance return
/// the empty snapshot (revision 0) — never fabricated state.
pub(crate) async fn task_ledger_snapshot(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Result<SessionTaskLedger, ApiError> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;
    Ok(ledger_snapshot_from_record(
        session_id,
        session.record().metadata.get(TASK_LEDGER_METADATA_KEY),
    ))
}

pub(crate) fn ledger_snapshot_from_record(
    session_id: &str,
    raw: Option<&serde_json::Value>,
) -> SessionTaskLedger {
    match raw {
        Some(value) => match serde_json::from_value::<SessionTaskLedger>(value.clone()) {
            Ok(mut ledger) => {
                // Fork copies the snapshot; the id always reflects the
                // session it was read from.
                ledger.session_id = session_id.to_string();
                ledger
            }
            Err(error) => {
                tracing::warn!(session_id, %error, "task ledger metadata unreadable; treating as empty");
                SessionTaskLedger::empty(session_id)
            }
        },
        None => SessionTaskLedger::empty(session_id),
    }
}

pub(crate) fn cause_for_seam(
    fact: &agendao_types::task_ledger::TaskLedgerSeamFact,
) -> TaskLedgerCause {
    match fact {
        agendao_types::task_ledger::TaskLedgerSeamFact::RunStarted => TaskLedgerCause::Recovery,
        agendao_types::task_ledger::TaskLedgerSeamFact::ToolBatchCompleted { .. } => {
            TaskLedgerCause::StatusChanged
        }
        agendao_types::task_ledger::TaskLedgerSeamFact::RecoveryInterrupted => {
            TaskLedgerCause::StatusChanged
        }
        agendao_types::task_ledger::TaskLedgerSeamFact::FinalResponseCommitted => {
            // The seam itself changes status (conditional Complete); when a
            // checkpoint IS added it arrives as its own seam with its own
            // cause.
            TaskLedgerCause::StatusChanged
        }
        agendao_types::task_ledger::TaskLedgerSeamFact::InteractionAwaiting { .. }
        | agendao_types::task_ledger::TaskLedgerSeamFact::InteractionResolved { .. } => {
            TaskLedgerCause::StatusChanged
        }
        agendao_types::task_ledger::TaskLedgerSeamFact::EvaluatorGateCompleted { .. } => {
            TaskLedgerCause::CheckpointAdded
        }
    }
}

/// External transports may only assert user provenance. Model, evaluator,
/// system, and machine-verifier identities are reserved for internal seams;
/// otherwise any HTTP/RPC caller could forge the labels rendered by clients.
pub(crate) fn validate_external_op(op: &TaskLedgerOp) -> Result<(), ApiError> {
    use agendao_types::task_ledger::{TaskLedgerActor, VerifierRef};

    let validate_actor = |actor: Option<TaskLedgerActor>| {
        if actor == Some(TaskLedgerActor::User) {
            Ok(())
        } else {
            Err(ApiError::BadRequest(
                "external task-ledger writes must use actor=user; model/evaluator/system provenance is reserved for internal seams"
                    .to_string(),
            ))
        }
    };
    match op {
        TaskLedgerOp::Create { goal, .. } | TaskLedgerOp::SetGoal { goal } => {
            validate_actor(Some(goal.set_by))
        }
        TaskLedgerOp::AddCore { actor, .. }
        | TaskLedgerOp::SwapCoreLive { actor, .. }
        | TaskLedgerOp::SetNext { actor, .. } => validate_actor(*actor),
        TaskLedgerOp::AddCheckpoint { verifier, .. }
        | TaskLedgerOp::CloseOpenWithCheckpoint { verifier, .. } => match verifier {
            VerifierRef::UserConfirmation { actor } if actor == "user" => Ok(()),
            VerifierRef::UserConfirmation { .. } => Err(ApiError::BadRequest(
                "external user_confirmation verifier actor must be `user`".to_string(),
            )),
            VerifierRef::DeterministicCheck { .. } | VerifierRef::Evaluator { .. } => {
                Err(ApiError::BadRequest(
                    "machine verifier types (deterministic_check/evaluator) cannot be \
                     submitted over the wire; use user_confirmation or let the server's \
                     seams produce them"
                        .to_string(),
                ))
            }
        },
        TaskLedgerOp::SetStatus { .. }
        | TaskLedgerOp::ResolveInteraction { .. }
        | TaskLedgerOp::Interrupt
        | TaskLedgerOp::Complete { .. } => Err(ApiError::BadRequest(
            "task-ledger lifecycle operations are reserved for internal execution seams"
                .to_string(),
        )),
        _ => Ok(()),
    }
}

pub(crate) fn cause_for_op(op: &TaskLedgerOp) -> TaskLedgerCause {
    match op {
        TaskLedgerOp::Create { .. } => TaskLedgerCause::Created,
        TaskLedgerOp::SetGoal { .. } => TaskLedgerCause::GoalUpdated,
        TaskLedgerOp::AddCore { .. } | TaskLedgerOp::SwapCoreLive { .. } => {
            TaskLedgerCause::CoreUpdated
        }
        TaskLedgerOp::AddCheckpoint { .. } => TaskLedgerCause::CheckpointAdded,
        TaskLedgerOp::OpenQuestion { .. } => TaskLedgerCause::OpenAdded,
        TaskLedgerOp::CloseOpenWithCheckpoint { .. } => TaskLedgerCause::OpenClosed,
        TaskLedgerOp::SetNext { .. } => TaskLedgerCause::NextUpdated,
        TaskLedgerOp::SetStatus { .. }
        | TaskLedgerOp::ResolveInteraction { .. }
        | TaskLedgerOp::Interrupt
        | TaskLedgerOp::Complete { .. } => TaskLedgerCause::StatusChanged,
    }
}

/// Validate + commit one operation under CAS, persist, and broadcast the
/// replacement event. The session write lock serializes concurrent writers;
/// a stale `expected_revision` loses cleanly instead of overwriting.
pub(crate) async fn apply_task_ledger_op(
    state: &Arc<ServerState>,
    session_id: &str,
    expected_revision: u64,
    op: TaskLedgerOp,
) -> Result<(SessionTaskLedger, TaskLedgerCause), ApiError> {
    apply_task_ledger_op_unless_cancelled(state, session_id, expected_revision, op, None)
        .await?
        .ok_or_else(|| ApiError::InternalError("unguarded ledger write was cancelled".to_string()))
}

/// Apply an internal run action only if its cancellation authority is still
/// live at the write-lock linearization point. This closes the race where a
/// stall action observes a live token, waits for the session mutex, and then
/// commits after Stop has already cancelled the run.
pub(crate) async fn apply_task_ledger_op_unless_cancelled(
    state: &Arc<ServerState>,
    session_id: &str,
    expected_revision: u64,
    op: TaskLedgerOp,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Option<(SessionTaskLedger, TaskLedgerCause)>, ApiError> {
    let cause = cause_for_op(&op);
    let now_ms = chrono::Utc::now().timestamp_millis();

    let mut sessions = state.sessions.lock().await;
    if cancellation.is_some_and(tokio_util::sync::CancellationToken::is_cancelled) {
        return Ok(None);
    }
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;
    let mut ledger = ledger_snapshot_from_record(
        session_id,
        session.record().metadata.get(TASK_LEDGER_METADATA_KEY),
    );
    if let Err(error) = ledger.apply(expected_revision, op, now_ms) {
        if matches!(error, TaskLedgerError::RevisionConflict { .. }) {
            let count = session
                .record()
                .metadata
                .get("task_ledger_revision_conflict_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                .saturating_add(1);
            session.insert_metadata(
                "task_ledger_revision_conflict_count".to_string(),
                serde_json::json!(count),
            );
            drop(sessions);
            crate::routes::session::session_crud::persist_session_if_enabled(state, session_id)
                .await;
        }
        return Err(map_ledger_error(session_id, error));
    }
    let snapshot = ledger.clone();
    session.insert_metadata(
        TASK_LEDGER_METADATA_KEY.to_string(),
        serde_json::to_value(&snapshot).map_err(|error| {
            ApiError::InternalError(format!("failed to serialize task ledger: {error}"))
        })?,
    );
    // Broadcast BEFORE releasing the lock and before the (slower) persist:
    // with the write lock held, concurrent applies serialize, so events
    // leave in revision order and no rev2 can overtake rev1 on the bus.
    crate::session_runtime::events::broadcast_server_event(
        state,
        &agendao_server_core::runtime_events::ServerEvent::TaskLedgerReplaced {
            session_id: session_id.to_string(),
            ledger: snapshot.clone(),
            cause: cause.clone(),
        },
    );
    drop(sessions);

    crate::routes::session::session_crud::persist_session_if_enabled(state, session_id).await;
    Ok(Some((snapshot, cause)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::task_ledger::{
        TaskGoal, TaskLedgerActor, TaskLedgerStatus, VerificationCoverage, VerifierRef,
    };
    use std::collections::HashMap;

    async fn state_with_session() -> Arc<ServerState> {
        let state = Arc::new(ServerState::new());
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/ledger-test");
        drop(sessions);
        state
    }

    async fn first_session_id(state: &Arc<ServerState>) -> String {
        let sessions = state.sessions.lock().await;
        sessions
            .list()
            .first()
            .map(|session| session.record().id.clone())
            .expect("session")
    }

    fn goal(statement: &str) -> TaskGoal {
        TaskGoal {
            statement: statement.to_string(),
            acceptance_criteria: vec!["tests pass".to_string()],
            criterion_checks: vec![],
            set_by: TaskLedgerActor::User,
            set_at: 1_000,
        }
    }

    fn create_op() -> TaskLedgerOp {
        TaskLedgerOp::Create {
            goal: goal("ship median"),
            next_statement: "write median".to_string(),
        }
    }

    #[tokio::test]
    async fn snapshot_for_session_without_ledger_is_empty_not_missing() {
        let state = state_with_session().await;
        let session_id = first_session_id(&state).await;
        let snapshot = task_ledger_snapshot(&state, &session_id).await.unwrap();
        assert_eq!(snapshot.revision, 0);
        assert!(snapshot.goal.is_none());
    }

    #[tokio::test]
    async fn apply_persists_to_metadata_and_roundtrips() {
        let state = state_with_session().await;
        let session_id = first_session_id(&state).await;
        let (snapshot, cause) = apply_task_ledger_op(&state, &session_id, 0, create_op())
            .await
            .unwrap();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(cause, TaskLedgerCause::Created);

        let reread = task_ledger_snapshot(&state, &session_id).await.unwrap();
        assert_eq!(reread, snapshot);
        assert_eq!(reread.status, TaskLedgerStatus::Active);
    }

    #[tokio::test]
    async fn stale_revision_returns_conflict_and_keeps_state() {
        let state = state_with_session().await;
        let session_id = first_session_id(&state).await;
        apply_task_ledger_op(&state, &session_id, 0, create_op())
            .await
            .unwrap();
        let err = apply_task_ledger_op(&state, &session_id, 0, create_op())
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::RevisionConflict { .. }));
        let snapshot = task_ledger_snapshot(&state, &session_id).await.unwrap();
        assert_eq!(snapshot.revision, 1, "failed write changed nothing");
        let sessions = state.sessions.lock().await;
        assert_eq!(
            sessions
                .get(&session_id)
                .and_then(|session| {
                    session
                        .record()
                        .metadata
                        .get("task_ledger_revision_conflict_count")
                })
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[tokio::test]
    async fn checkpoint_via_authority_requires_coverage() {
        let state = state_with_session().await;
        let session_id = first_session_id(&state).await;
        apply_task_ledger_op(&state, &session_id, 0, create_op())
            .await
            .unwrap();
        let err = apply_task_ledger_op(
            &state,
            &session_id,
            1,
            TaskLedgerOp::AddCheckpoint {
                claim: "works".to_string(),
                verifier: VerifierRef::DeterministicCheck {
                    description: "unittest".to_string(),
                },
                coverage: VerificationCoverage {
                    scope: String::new(),
                },
                covered_criteria: vec![],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("coverage"));
    }

    #[tokio::test]
    async fn fork_copies_snapshot_rebound_to_new_session() {
        let state = state_with_session().await;
        let session_id = first_session_id(&state).await;
        apply_task_ledger_op(&state, &session_id, 0, create_op())
            .await
            .unwrap();
        apply_task_ledger_op(
            &state,
            &session_id,
            1,
            TaskLedgerOp::AddCheckpoint {
                claim: "median verified".to_string(),
                verifier: VerifierRef::DeterministicCheck {
                    description: "unittest".to_string(),
                },
                coverage: VerificationCoverage {
                    scope: "3 cases".to_string(),
                },
                covered_criteria: vec![],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
            },
        )
        .await
        .unwrap();

        // Governance decision: fork copies the snapshot (history and
        // revision preserved) and rebinds it to the child session.
        let forked_id = {
            let mut sessions = state.sessions.lock().await;
            sessions
                .fork(
                    &session_id,
                    agendao_session::SessionForkSpec {
                        message_id: None,
                        history_mode: agendao_types::SessionForkHistoryMode::All,
                        history_message_limit: None,
                    },
                )
                .expect("fork")
                .record()
                .id
                .clone()
        };
        let forked = task_ledger_snapshot(&state, &forked_id).await.unwrap();
        assert_eq!(forked.session_id, forked_id);
        assert_eq!(forked.revision, 2, "snapshot revision preserved");
        assert_eq!(forked.verified.len(), 1);
        // The two ledgers evolve independently afterwards.
        apply_task_ledger_op(
            &state,
            &forked_id,
            2,
            TaskLedgerOp::SetNext {
                statement: "child-only next".to_string(),
                actor: None,
            },
        )
        .await
        .unwrap();
        let parent = task_ledger_snapshot(&state, &session_id).await.unwrap();
        assert_ne!(parent.next.as_ref().unwrap().statement, "child-only next");
    }

    #[tokio::test]
    async fn deleting_session_deletes_its_ledger() {
        let state = state_with_session().await;
        let session_id = first_session_id(&state).await;
        apply_task_ledger_op(&state, &session_id, 0, create_op())
            .await
            .unwrap();
        {
            let mut sessions = state.sessions.lock().await;
            sessions.delete(&session_id);
        }
        let err = task_ledger_snapshot(&state, &session_id).await.unwrap_err();
        assert!(matches!(err, ApiError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn unknown_session_is_not_found() {
        let state = state_with_session().await;
        let err = task_ledger_snapshot(&state, "ses_missing")
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::SessionNotFound(_)));
        let _unused: HashMap<String, String> = HashMap::new();
    }
}
