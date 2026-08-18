//! HTTP surface for the session task ledger.
//!
//! One generic CAS write (`PATCH`, carrying the typed op) plus the three
//! semantic sub-resources the governance plan names (checkpoint / open /
//! close). All writes go through the single server authority; every committed
//! write broadcasts one canonical `task-ledger.replaced` event.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use agendao_types::task_ledger::{
    SessionTaskLedger, TaskLedgerCause, TaskLedgerOp, VerificationCoverage, VerifierRef,
};

use crate::error::ApiError;
use crate::session_runtime::task_ledger::{
    apply_task_ledger_op, task_ledger_snapshot, TASK_LEDGER_METADATA_KEY,
};
use crate::{Result, ServerState};

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

pub(crate) async fn get_task_ledger(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionTaskLedger>> {
    let snapshot = task_ledger_snapshot(&state, &id).await?;
    Ok(Json(snapshot))
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskLedgerPatchRequest {
    pub expected_revision: u64,
    pub op: TaskLedgerOp,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct TaskLedgerWriteResponse {
    pub ledger: SessionTaskLedger,
    pub cause: TaskLedgerCause,
    pub metadata_key: &'static str,
}

pub(crate) async fn patch_task_ledger(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<TaskLedgerPatchRequest>,
) -> Result<Json<TaskLedgerWriteResponse>> {
    crate::session_runtime::task_ledger::validate_external_op(&req.op)?;
    let (ledger, cause) = apply_task_ledger_op(&state, &id, req.expected_revision, req.op).await?;
    Ok(Json(TaskLedgerWriteResponse {
        ledger,
        cause,
        metadata_key: TASK_LEDGER_METADATA_KEY,
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct CheckpointRequest {
    pub expected_revision: u64,
    pub claim: String,
    pub verifier: VerifierRef,
    pub coverage: VerificationCoverage,
    #[serde(default)]
    pub covered_criteria: Vec<String>,
    #[serde(default)]
    pub evidence_artifact_ids: Vec<String>,
    #[serde(default)]
    pub source_stage_id: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
}

pub(crate) async fn add_task_ledger_checkpoint(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<CheckpointRequest>,
) -> Result<Json<TaskLedgerWriteResponse>> {
    if !non_empty(&req.claim) {
        return Err(ApiError::BadRequest(
            "checkpoint claim must not be empty".into(),
        ));
    }
    let op = TaskLedgerOp::AddCheckpoint {
        claim: req.claim,
        verifier: req.verifier,
        coverage: req.coverage,
        covered_criteria: req.covered_criteria,
        evidence_artifact_ids: req.evidence_artifact_ids,
        source_stage_id: req.source_stage_id,
        supersedes: req.supersedes,
    };
    crate::session_runtime::task_ledger::validate_external_op(&op)?;
    let (ledger, cause) = apply_task_ledger_op(&state, &id, req.expected_revision, op).await?;
    Ok(Json(TaskLedgerWriteResponse {
        ledger,
        cause,
        metadata_key: TASK_LEDGER_METADATA_KEY,
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenQuestionRequest {
    pub expected_revision: u64,
    pub question: String,
    pub settled_by: String,
}

pub(crate) async fn add_task_ledger_open(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<OpenQuestionRequest>,
) -> Result<Json<TaskLedgerWriteResponse>> {
    let (ledger, cause) = apply_task_ledger_op(
        &state,
        &id,
        req.expected_revision,
        TaskLedgerOp::OpenQuestion {
            question: req.question,
            settled_by: req.settled_by,
        },
    )
    .await?;
    Ok(Json(TaskLedgerWriteResponse {
        ledger,
        cause,
        metadata_key: TASK_LEDGER_METADATA_KEY,
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloseOpenRequest {
    pub expected_revision: u64,
    pub claim: String,
    pub verifier: VerifierRef,
    pub coverage: VerificationCoverage,
    #[serde(default)]
    pub covered_criteria: Vec<String>,
    #[serde(default)]
    pub evidence_artifact_ids: Vec<String>,
    #[serde(default)]
    pub source_stage_id: Option<String>,
}

pub(crate) async fn close_task_ledger_open(
    State(state): State<Arc<ServerState>>,
    Path((id, open_id)): Path<(String, String)>,
    Json(req): Json<CloseOpenRequest>,
) -> Result<Json<TaskLedgerWriteResponse>> {
    let op = TaskLedgerOp::CloseOpenWithCheckpoint {
        open_id,
        claim: req.claim,
        verifier: req.verifier,
        coverage: req.coverage,
        covered_criteria: req.covered_criteria,
        evidence_artifact_ids: req.evidence_artifact_ids,
        source_stage_id: req.source_stage_id,
    };
    crate::session_runtime::task_ledger::validate_external_op(&op)?;
    let (ledger, cause) = apply_task_ledger_op(&state, &id, req.expected_revision, op).await?;
    Ok(Json(TaskLedgerWriteResponse {
        ledger,
        cause,
        metadata_key: TASK_LEDGER_METADATA_KEY,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::task_ledger::VerifierRef;

    fn checkpoint_op(verifier: VerifierRef) -> TaskLedgerOp {
        TaskLedgerOp::AddCheckpoint {
            claim: "c".to_string(),
            verifier,
            coverage: VerificationCoverage {
                scope: "s".to_string(),
            },
            covered_criteria: vec![],
            evidence_artifact_ids: vec![],
            source_stage_id: None,
            supersedes: None,
        }
    }

    #[test]
    fn external_transports_cannot_submit_machine_verifiers() {
        // Forging criterion evidence over the wire must be impossible:
        // deterministic-check and evaluator verifiers are seam-internal.
        for verifier in [
            VerifierRef::DeterministicCheck {
                description: "x".to_string(),
            },
            VerifierRef::Evaluator {
                name: "y".to_string(),
            },
        ] {
            let err =
                crate::session_runtime::task_ledger::validate_external_op(&checkpoint_op(verifier))
                    .expect_err("machine verifier must be rejected");
            assert!(err.to_string().contains("cannot be"), "{err}");
        }
        // Explicit user confirmation remains the legitimate external path.
        assert!(
            crate::session_runtime::task_ledger::validate_external_op(&checkpoint_op(
                VerifierRef::UserConfirmation {
                    actor: "user".to_string()
                }
            ))
            .is_ok()
        );
        // Ordinary state writes must also carry honest external provenance.
        assert!(crate::session_runtime::task_ledger::validate_external_op(
            &TaskLedgerOp::SetNext {
                statement: "n".to_string(),
                actor: Some(agendao_types::task_ledger::TaskLedgerActor::User),
            }
        )
        .is_ok());
    }

    #[test]
    fn external_transports_cannot_forge_task_state_provenance() {
        use agendao_types::task_ledger::{
            AwaitingInteractionKind, TaskLedgerActor, TaskLedgerStatus,
        };

        for actor in [
            TaskLedgerActor::Model,
            TaskLedgerActor::Evaluator,
            TaskLedgerActor::System,
        ] {
            let err =
                crate::session_runtime::task_ledger::validate_external_op(&TaskLedgerOp::SetNext {
                    statement: "pretend this came from an internal seam".to_string(),
                    actor: Some(actor),
                })
                .expect_err("external actor provenance must not be forgeable");
            assert!(err.to_string().contains("actor=user"));
        }
        let err = crate::session_runtime::task_ledger::validate_external_op(&checkpoint_op(
            VerifierRef::UserConfirmation {
                actor: "evaluator".to_string(),
            },
        ))
        .expect_err("user confirmation actor is an authority label");
        assert!(err.to_string().contains("must be `user`"));

        for op in [
            TaskLedgerOp::SetStatus {
                status: TaskLedgerStatus::Completed,
                awaiting: None,
                blocked_reason: None,
            },
            TaskLedgerOp::ResolveInteraction {
                kind: AwaitingInteractionKind::Permission,
                interaction_id: "permission-1".to_string(),
            },
            TaskLedgerOp::Interrupt,
            TaskLedgerOp::Complete {
                uncovered: vec!["claim completion without seam authority".to_string()],
            },
        ] {
            let err = crate::session_runtime::task_ledger::validate_external_op(&op)
                .expect_err("external callers cannot forge lifecycle transitions");
            assert!(err.to_string().contains("internal execution seams"));
        }
    }
}
