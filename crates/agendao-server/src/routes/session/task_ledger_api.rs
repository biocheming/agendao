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
    let (ledger, cause) = apply_task_ledger_op(
        &state,
        &id,
        req.expected_revision,
        TaskLedgerOp::AddCheckpoint {
            claim: req.claim,
            verifier: req.verifier,
            coverage: req.coverage,
            covered_criteria: req.covered_criteria,
            evidence_artifact_ids: req.evidence_artifact_ids,
            source_stage_id: req.source_stage_id,
            supersedes: req.supersedes,
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
    let (ledger, cause) = apply_task_ledger_op(
        &state,
        &id,
        req.expected_revision,
        TaskLedgerOp::CloseOpenWithCheckpoint {
            open_id,
            claim: req.claim,
            verifier: req.verifier,
            coverage: req.coverage,
            covered_criteria: req.covered_criteria,
            evidence_artifact_ids: req.evidence_artifact_ids,
            source_stage_id: req.source_stage_id,
        },
    )
    .await?;
    Ok(Json(TaskLedgerWriteResponse {
        ledger,
        cause,
        metadata_key: TASK_LEDGER_METADATA_KEY,
    }))
}
