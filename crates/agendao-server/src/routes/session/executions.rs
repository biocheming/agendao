use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use agendao_core::agent_task_registry::{global_task_registry, AgentTask, AgentTaskStatus};
use agendao_session::{PartType, Session, ToolCallStatus};

use crate::runtime_control::SessionExecutionTopology;
use crate::{ApiError, Result, ServerState};

use super::cancel::ensure_session_exists;

pub(super) async fn get_session_executions(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionExecutionTopology>> {
    ensure_session_exists(&state, &session_id).await?;
    let session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?
    };
    Ok(Json(
        build_session_execution_topology_snapshot(&state, &session_id, &session).await,
    ))
}

pub(super) async fn build_session_execution_topology_snapshot(
    state: &Arc<ServerState>,
    session_id: &str,
    session: &Session,
) -> SessionExecutionTopology {
    let base_records = state
        .runtime_telemetry
        .list_session_execution_records(session_id)
        .await;
    let mut extra_records = collect_active_tool_execution_records(session, &base_records);
    extra_records.extend(collect_active_agent_task_execution_records(
        session_id,
        &base_records,
    ));
    state
        .runtime_telemetry
        .build_session_execution_topology(session_id.to_string(), extra_records)
        .await
}

/// Global enumeration: list all active execution records across all sessions.
pub(super) async fn list_all_executions(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<serde_json::Value>> {
    let records = state.runtime_telemetry.list_all_executions().await;
    let session_ids = state.runtime_telemetry.list_active_session_ids().await;
    Ok(Json(serde_json::json!({
        "active_count": records.len(),
        "active_session_ids": session_ids,
        "executions": records,
    })))
}

pub(super) async fn cancel_session_execution(
    State(state): State<Arc<ServerState>>,
    Path((_session_id, execution_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let result = state
        .runtime_telemetry
        .cancel_execution(&execution_id)
        .await;
    match result {
        Some(kind) => {
            // For AgentTask, also cancel via the global task registry.
            if matches!(kind, crate::runtime_control::ExecutionKind::AgentTask) {
                if let Some(task_id) = execution_id.strip_prefix("agent_task:") {
                    let _ = global_task_registry().cancel(task_id);
                }
            }
            Ok(Json(serde_json::json!({
                "cancelled": true,
                "kind": kind,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "cancelled": false,
            "error": "execution not found",
        }))),
    }
}

pub(super) fn collect_active_tool_execution_records(
    session: &Session,
    existing_records: &[crate::runtime_control::ExecutionRecord],
) -> Vec<crate::runtime_control::ExecutionRecord> {
    let session_record = session.record();
    let parent_id = select_active_tool_parent_id(existing_records);
    // Resolve stage_id from the parent record.
    let stage_id = parent_id.as_ref().and_then(|pid| {
        existing_records
            .iter()
            .find(|r| r.id == *pid)
            .and_then(|r| r.stage_id.clone())
    });

    // Build a set of tool_call IDs already present in the registry to avoid
    // double-counting when the lifecycle hook has already registered them.
    let registered_ids: std::collections::HashSet<&str> = existing_records
        .iter()
        .filter(|r| matches!(r.kind, crate::runtime_control::ExecutionKind::ToolCall))
        .map(|r| r.id.as_str())
        .collect();

    let mut records = Vec::new();

    for message in &session_record.messages {
        for part in &message.parts {
            let PartType::ToolCall {
                id,
                name,
                input,
                status,
                ..
            } = &part.part_type
            else {
                continue;
            };

            if !matches!(status, ToolCallStatus::Pending | ToolCallStatus::Running) {
                continue;
            }

            // Skip if this tool call is already registered via the lifecycle hook.
            let candidate_id = format!("tool_call:{id}");
            if registered_ids.contains(candidate_id.as_str()) {
                continue;
            }

            let execution_status = match status {
                ToolCallStatus::Pending => crate::runtime_control::ExecutionStatus::Waiting,
                ToolCallStatus::Running => crate::runtime_control::ExecutionStatus::Running,
                ToolCallStatus::Completed | ToolCallStatus::Error => continue,
            };

            let (waiting_on, recent_event) = match status {
                ToolCallStatus::Pending => ("dispatch".to_string(), format!("{name} queued")),
                ToolCallStatus::Running => ("tool".to_string(), format!("{name} running")),
                ToolCallStatus::Completed | ToolCallStatus::Error => {
                    debug_assert!(
                        false,
                        "completed/error tool calls should have been filtered before record creation"
                    );
                    continue;
                }
            };

            records.push(crate::runtime_control::ExecutionRecord {
                id: format!("tool_call:{id}"),
                session_id: session_record.id.clone(),
                kind: crate::runtime_control::ExecutionKind::ToolCall,
                status: execution_status,
                label: Some(format!("Tool: {name}")),
                parent_id: parent_id.clone(),
                stage_id: stage_id.clone(),
                waiting_on: Some(waiting_on),
                recent_event: Some(recent_event),
                started_at: part.created_at.timestamp_millis(),
                updated_at: part.created_at.timestamp_millis(),
                metadata: Some(serde_json::json!({
                    "tool_call_id": id,
                    "tool_name": name,
                    "input": input,
                    "message_id": message.id,
                    "status": match status {
                        ToolCallStatus::Pending => "pending",
                        ToolCallStatus::Running => "running",
                        ToolCallStatus::Completed => "completed",
                        ToolCallStatus::Error => "error",
                    },
                })),
            });
        }
    }

    records
}

pub(super) fn collect_active_agent_task_execution_records(
    session_id: &str,
    existing_records: &[crate::runtime_control::ExecutionRecord],
) -> Vec<crate::runtime_control::ExecutionRecord> {
    let parent_id = select_active_agent_task_parent_id(existing_records);
    let stage_id = parent_id.as_ref().and_then(|pid| {
        existing_records
            .iter()
            .find(|r| r.id == *pid)
            .and_then(|r| r.stage_id.clone())
    });
    global_task_registry()
        .list()
        .into_iter()
        .filter(|task| task.session_id.as_deref() == Some(session_id))
        .filter(|task| !task.status.is_terminal())
        .map(|task| {
            agent_task_execution_record(task, session_id, parent_id.clone(), stage_id.clone())
        })
        .collect()
}

fn agent_task_execution_record(
    task: AgentTask,
    session_id: &str,
    parent_id: Option<String>,
    stage_id: Option<String>,
) -> crate::runtime_control::ExecutionRecord {
    let (status, waiting_on, recent_event, step) = match &task.status {
        AgentTaskStatus::Pending => (
            crate::runtime_control::ExecutionStatus::Waiting,
            Some("agent".to_string()),
            Some("Agent task queued".to_string()),
            None,
        ),
        AgentTaskStatus::Running { step } => (
            crate::runtime_control::ExecutionStatus::Running,
            Some("agent".to_string()),
            Some(match task.max_steps {
                Some(max_steps) => format!("Step {} / {}", step, max_steps),
                None => format!("Step {}", step),
            }),
            Some(*step),
        ),
        AgentTaskStatus::Completed { .. }
        | AgentTaskStatus::Cancelled
        | AgentTaskStatus::Failed { .. } => (
            crate::runtime_control::ExecutionStatus::Running,
            None,
            None,
            None,
        ),
    };

    crate::runtime_control::ExecutionRecord {
        id: format!("agent_task:{}", task.id),
        session_id: session_id.to_string(),
        kind: crate::runtime_control::ExecutionKind::AgentTask,
        status,
        label: Some(format!("Agent task: {}", task.agent_name)),
        parent_id,
        stage_id,
        waiting_on,
        recent_event,
        started_at: task.started_at.saturating_mul(1000),
        updated_at: chrono::Utc::now().timestamp_millis(),
        metadata: Some(serde_json::json!({
            "task_id": task.id,
            "agent_name": task.agent_name,
            "prompt": task.prompt,
            "max_steps": task.max_steps,
            "step": step,
            "output_tail": task.output_tail,
        })),
    }
}

fn select_active_tool_parent_id(
    records: &[crate::runtime_control::ExecutionRecord],
) -> Option<String> {
    select_preferred_execution_parent_id(records)
}

fn select_active_agent_task_parent_id(
    records: &[crate::runtime_control::ExecutionRecord],
) -> Option<String> {
    records
        .iter()
        .filter(|record| matches!(record.kind, crate::runtime_control::ExecutionKind::ToolCall))
        .filter(|record| {
            record
                .metadata
                .as_ref()
                .and_then(|value| value.get("tool_name"))
                .and_then(|value| value.as_str())
                .map(|name| matches!(name, "task" | "task_flow"))
                .unwrap_or(false)
        })
        .max_by_key(|record| record.updated_at)
        .map(|record| record.id.clone())
        .or_else(|| select_preferred_execution_parent_id(records))
}

fn select_preferred_execution_parent_id(
    records: &[crate::runtime_control::ExecutionRecord],
) -> Option<String> {
    records
        .iter()
        .filter(|record| {
            matches!(
                record.kind,
                crate::runtime_control::ExecutionKind::PromptRun
                    | crate::runtime_control::ExecutionKind::SchedulerRun
                    | crate::runtime_control::ExecutionKind::SchedulerStage
            )
        })
        .max_by_key(|record| {
            (
                execution_parent_rank(&record.kind),
                record.updated_at,
                record.started_at,
            )
        })
        .map(|record| record.id.clone())
}

fn execution_parent_rank(kind: &crate::runtime_control::ExecutionKind) -> u8 {
    match kind {
        crate::runtime_control::ExecutionKind::PromptRun => 0,
        crate::runtime_control::ExecutionKind::SchedulerRun => 1,
        crate::runtime_control::ExecutionKind::SchedulerStage => 2,
        crate::runtime_control::ExecutionKind::ToolCall
        | crate::runtime_control::ExecutionKind::AgentTask
        | crate::runtime_control::ExecutionKind::Question => 0,
    }
}
