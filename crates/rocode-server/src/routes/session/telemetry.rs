use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use rocode_command::stage_protocol::StageSummary;
use rocode_multimodal::PersistedMultimodalExplain;
use rocode_session::{
    load_session_telemetry_snapshot, persist_session_telemetry_snapshot,
    session_last_run_status_label, Session, SessionUsage,
};
use rocode_types::{
    SessionDiagnosticsSidecar, SessionInsightsResponse, SessionMemoryTelemetrySummary,
    SessionMultimodalAttachmentInfo, SessionMultimodalInsight,
};
use serde::{Deserialize, Serialize};

use crate::runtime_control::SessionExecutionTopology;
use crate::session_runtime::state::SessionRuntimeState;
use crate::{Result, ServerState};

use super::cancel::ensure_session_exists;
use super::effective_policy::build_session_effective_policy;
use super::executions::build_session_execution_topology_snapshot;
use super::session_crud::runtime_snapshot_or_default;

#[derive(Debug, Clone, Serialize)]
pub struct SessionTelemetrySnapshot {
    pub runtime: SessionRuntimeState,
    pub stages: Vec<StageSummary>,
    pub topology: SessionExecutionTopology,
    pub usage: SessionUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<SessionMemoryTelemetrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_bust_summary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_runtime_snapshot: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_snapshot_invalidation: Option<PromptSurfaceSnapshotInvalidationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_stabilization: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_preflight_summary: Option<SessionExecutionPreflightSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic_summary: Option<rocode_provider::ProviderDiagnosticSummary>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionExecutionPreflightSource {
    ToolCallState,
    ToolResultPart,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionExecutionPreflightSummary {
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub source: SessionExecutionPreflightSource,
    pub runner: String,
    pub subject: String,
    pub status: rocode_tool::ExecutionPreflightStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<rocode_tool::ExecutionPreflightIssue>,
    #[serde(default)]
    pub attachment_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSurfaceSnapshotInvalidationSummary {
    pub severity: rocode_provider::cache::CacheBustSeverity,
    pub reason: String,
    pub changed_fields: Vec<String>,
}

pub(super) async fn get_session_telemetry(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionTelemetrySnapshot>> {
    ensure_session_exists(&state, &session_id).await?;
    let session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .expect("session existence checked before telemetry load")
    };

    Ok(Json(
        build_session_telemetry_snapshot(&state, &session_id, &session).await?,
    ))
}

pub(super) async fn get_session_insights(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionInsightsResponse>> {
    let session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| crate::ApiError::SessionNotFound(session_id.clone()))?
    };

    let session_record = session.record();
    let memory = match state
        .runtime_memory
        .build_session_memory_insight(&session)
        .await
    {
        Ok(memory) => memory,
        Err(error) => {
            tracing::warn!(
                session_id = %session.id,
                %error,
                "failed to build session memory insight"
            );
            None
        }
    };
    Ok(Json(SessionInsightsResponse {
        id: session_record.id.clone(),
        title: session_record.title.clone(),
        directory: session_record.directory.clone(),
        updated: session_record.time.updated,
        telemetry: load_session_telemetry_snapshot(&session),
        effective_policy: Some(
            build_session_effective_policy(&state, &session, memory.as_ref()).await,
        ),
        memory,
        multimodal: build_session_multimodal_insight(&session),
    }))
}

pub(super) async fn build_session_telemetry_snapshot(
    state: &Arc<ServerState>,
    session_id: &str,
    session: &Session,
) -> Result<SessionTelemetrySnapshot> {
    let mut runtime = runtime_snapshot_or_default(state, session_id).await?;
    let usage = runtime.usage.clone().unwrap_or_else(|| session.get_usage());
    runtime.usage = Some(usage.clone());

    let stages = state
        .runtime_telemetry
        .list_stage_summaries(session_id)
        .await;
    let topology = build_session_execution_topology_snapshot(state, session_id, session).await;
    let memory = build_session_memory_telemetry(state, session).await;
    let diagnostics = SessionDiagnosticsSidecar::derive_from_session(session);
    let cache_bust_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_cache_bust_summary_value);
    let prompt_surface_runtime_snapshot = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::prompt_surface_runtime_snapshot_value);
    let prompt_surface_snapshot_invalidation = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_prompt_surface_snapshot_invalidation_value)
        .and_then(|value| serde_json::from_value(value).ok());
    let ingress_stabilization = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::ingress_stabilization_value);
    let execution_preflight_summary = diagnostics
        .as_ref()
        .and_then(latest_execution_preflight_summary_from_sidecar);
    let provider_diagnostic_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_provider_diagnostic_value)
        .and_then(|value| serde_json::from_value(value).ok());

    Ok(SessionTelemetrySnapshot {
        runtime,
        stages,
        topology,
        usage,
        memory,
        cache_bust_summary,
        prompt_surface_runtime_snapshot,
        prompt_surface_snapshot_invalidation,
        ingress_stabilization,
        execution_preflight_summary,
        provider_diagnostic_summary,
    })
}

#[cfg(test)]
fn latest_cache_bust_summary(session: &Session) -> Option<serde_json::Value> {
    diagnostics_sidecar(session).and_then(|sidecar| sidecar.latest_cache_bust_summary_value())
}

#[cfg(test)]
fn prompt_surface_runtime_snapshot(session: &Session) -> Option<serde_json::Value> {
    diagnostics_sidecar(session).and_then(|sidecar| sidecar.prompt_surface_runtime_snapshot_value())
}

#[cfg(test)]
fn latest_prompt_surface_snapshot_invalidation(
    session: &Session,
) -> Option<PromptSurfaceSnapshotInvalidationSummary> {
    diagnostics_sidecar(session)
        .and_then(|sidecar| sidecar.latest_prompt_surface_snapshot_invalidation_value())
        .and_then(|value| serde_json::from_value(value).ok())
}

#[cfg(test)]
fn latest_provider_diagnostic_summary(
    session: &Session,
) -> Option<rocode_provider::ProviderDiagnosticSummary> {
    diagnostics_sidecar(session)
        .and_then(|sidecar| sidecar.latest_provider_diagnostic_value())
        .and_then(|value| serde_json::from_value(value).ok())
}

#[cfg(test)]
fn latest_execution_preflight_summary(
    session: &Session,
) -> Option<SessionExecutionPreflightSummary> {
    diagnostics_sidecar(session)
        .and_then(|sidecar| latest_execution_preflight_summary_from_sidecar(&sidecar))
}

#[cfg(test)]
fn diagnostics_sidecar(session: &Session) -> Option<SessionDiagnosticsSidecar> {
    SessionDiagnosticsSidecar::derive_from_session(session)
}

fn latest_execution_preflight_summary_from_sidecar(
    sidecar: &SessionDiagnosticsSidecar,
) -> Option<SessionExecutionPreflightSummary> {
    let entry = sidecar.latest_execution_preflight_entry()?;
    let preflight: rocode_tool::ExecutionPreflightMetadata = entry.decode_metadata()?;
    Some(SessionExecutionPreflightSummary {
        tool_call_id: entry.tool_call_id,
        tool_name: entry.tool_name,
        source: match entry.source {
            rocode_types::SessionExecutionPreflightMetadataSource::ToolCallState => {
                SessionExecutionPreflightSource::ToolCallState
            }
            rocode_types::SessionExecutionPreflightMetadataSource::ToolResultPart => {
                SessionExecutionPreflightSource::ToolResultPart
            }
        },
        runner: preflight.runner,
        subject: preflight.subject,
        status: preflight.status,
        issues: preflight.issues,
        attachment_count: preflight.attachment_count,
    })
}

pub(super) async fn persist_session_telemetry_metadata(
    state: &Arc<ServerState>,
    session: &mut Session,
) {
    let usage = session.get_usage();
    let last_run_status = session_last_run_status_label(session);
    let session_id = session.record().id.clone();
    let memory = build_session_memory_telemetry(state, session).await;
    let Some(snapshot) = state
        .runtime_telemetry
        .build_persisted_snapshot(&session_id, usage, last_run_status, memory)
        .await
    else {
        return;
    };

    if let Err(error) = persist_session_telemetry_snapshot(session, &snapshot) {
        tracing::warn!(
            session_id = %session.id,
            %error,
            "failed to persist telemetry snapshot into session metadata"
        );
        return;
    }

    state
        .runtime_telemetry
        .emit_telemetry_snapshot_updated_hook(&session_id, &snapshot)
        .await;
}

async fn build_session_memory_telemetry(
    state: &Arc<ServerState>,
    session: &Session,
) -> Option<SessionMemoryTelemetrySummary> {
    match state
        .runtime_memory
        .build_session_memory_telemetry(session)
        .await
    {
        Ok(memory) => memory,
        Err(error) => {
            tracing::warn!(
                session_id = %session.id,
                %error,
                "failed to build session memory telemetry summary"
            );
            None
        }
    }
}

fn build_session_multimodal_insight(session: &Session) -> Option<SessionMultimodalInsight> {
    let message = session
        .record()
        .messages
        .iter()
        .rev()
        .find(|message| PersistedMultimodalExplain::has_message_signal(message))?;
    let explain = PersistedMultimodalExplain::from_message(message)?;

    Some(SessionMultimodalInsight {
        user_message_id: message.id.clone(),
        attachment_count: explain.attachment_count,
        kinds: explain.kinds,
        badges: explain.badges,
        compact_label: explain.compact_label,
        resolved_model: explain.resolved_model,
        warnings: explain.warnings,
        unsupported_parts: explain.unsupported_parts,
        recommended_downgrade: explain.recommended_downgrade,
        hard_block: explain.hard_block,
        transport_replaced_parts: explain.transport_replaced_parts,
        transport_warnings: explain.transport_warnings,
        attachments: explain
            .attachments
            .into_iter()
            .map(|attachment| SessionMultimodalAttachmentInfo {
                filename: attachment.filename,
                mime: attachment.mime,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_control::SessionExecutionTopology;
    use crate::session_runtime::state::SessionRuntimeState;
    use crate::session_runtime::{emit_scheduler_stage_message, SchedulerStageMessageInput};
    use crate::ServerState;
    use rocode_command::stage_protocol::{StageStatus, StageSummary};
    use rocode_memory::PersistedMemorySnapshot;
    use rocode_orchestrator::ExecutionContext;
    use rocode_plugin::{global, Hook, HookEvent};
    use rocode_session::{
        persist_session_telemetry_snapshot, MessageUsage, SessionTelemetrySnapshotVersion,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    fn sample_execution_preflight_metadata(
        status: rocode_tool::ExecutionPreflightStatus,
        issues: Vec<rocode_tool::ExecutionPreflightIssue>,
        attachment_count: usize,
    ) -> HashMap<String, serde_json::Value> {
        let mut metadata = HashMap::new();
        metadata.insert(
            rocode_tool::EXECUTION_PREFLIGHT_METADATA_KEY.to_string(),
            serde_json::to_value(rocode_tool::ExecutionPreflightMetadata {
                runner: "read".to_string(),
                subject: "/tmp/sample.pdf".to_string(),
                status,
                issues,
                output: "PDF read successfully".to_string(),
                metadata: HashMap::new(),
                attachment_count,
            })
            .expect("execution preflight metadata should serialize"),
        );
        metadata
    }

    #[test]
    fn telemetry_snapshot_syncs_runtime_usage_from_session_when_missing() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.usage = Some(rocode_session::MessageUsage {
            input_tokens: 12,
            output_tokens: 8,
            reasoning_tokens: 3,
            cache_write_tokens: 2,
            cache_read_tokens: 1,
            cache_miss_tokens: 0,
            context_tokens: 0,
            total_cost: 0.42,
        });

        let mut runtime = SessionRuntimeState::new("session-1");
        let usage = runtime.usage.clone().unwrap_or_else(|| session.get_usage());
        runtime.usage = Some(usage.clone());

        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 8);
        assert_eq!(runtime.usage.as_ref().map(|v| v.total_cost), Some(0.42));
    }

    #[test]
    fn latest_cache_bust_summary_reads_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_provider::cache::CACHE_BUST_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!({
                "status": "degraded",
                "severity": "LikelyBust",
                "primary_cause": "messagePrefixHash changed: message prefix changed before the stable boundary",
                "change_count": 1,
            }),
        );

        let summary = latest_cache_bust_summary(&session).expect("summary");

        assert_eq!(summary["status"], "degraded");
        assert_eq!(summary["severity"], "LikelyBust");
    }

    #[test]
    fn latest_provider_diagnostic_summary_reads_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::RequestValidation,
            code: "thinking_replay_missing".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-reasoner".to_string()),
            message: "missing replay".to_string(),
        }
        .attach_to_metadata(&mut assistant.metadata);

        let summary = latest_provider_diagnostic_summary(&session).expect("summary");

        assert_eq!(summary.code, "thinking_replay_missing");
        assert_eq!(summary.provider_id, "deepseek");
        assert_eq!(summary.model_id.as_deref(), Some("deepseek-reasoner"));
    }

    #[test]
    fn prompt_surface_runtime_snapshot_prefers_session_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        session.insert_metadata(
            rocode_session::prompt::PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::json!({
                "generation": 2,
                "source": "session",
            }),
        );
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_session::prompt::PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::json!({
                "generation": 1,
                "source": "assistant",
            }),
        );

        let snapshot = prompt_surface_runtime_snapshot(&session).expect("snapshot");

        assert_eq!(snapshot["generation"], 2);
        assert_eq!(snapshot["source"], "session");
    }

    #[test]
    fn prompt_surface_runtime_snapshot_falls_back_to_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_session::prompt::PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::json!({
                "generation": 3,
                "source": "assistant",
            }),
        );

        let snapshot = prompt_surface_runtime_snapshot(&session).expect("snapshot");

        assert_eq!(snapshot["generation"], 3);
        assert_eq!(snapshot["source"], "assistant");
    }

    #[test]
    fn latest_prompt_surface_snapshot_invalidation_reads_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_session::prompt::PROMPT_SURFACE_SNAPSHOT_INVALIDATION_METADATA_KEY.to_string(),
            serde_json::json!({
                "severity": "LikelyBust",
                "reason": "prompt surface runtime changed: outputProjectionPolicyHash",
                "changed_fields": ["outputProjectionPolicyHash"],
            }),
        );

        let invalidation =
            latest_prompt_surface_snapshot_invalidation(&session).expect("invalidation");

        assert_eq!(
            invalidation.severity,
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        assert_eq!(
            invalidation.reason,
            "prompt surface runtime changed: outputProjectionPolicyHash"
        );
        assert_eq!(
            invalidation.changed_fields,
            vec!["outputProjectionPolicyHash".to_string()]
        );
    }

    #[test]
    fn latest_prompt_surface_snapshot_invalidation_falls_back_to_snapshot_payload() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        session.insert_metadata(
            rocode_session::prompt::PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::json!({
                "generation": 7,
                "invalidation": {
                    "severity": "SoftDegradation",
                    "reason": "prompt surface runtime changed: ingressPolicyHash",
                    "changed_fields": ["ingressPolicyHash"]
                }
            }),
        );

        let invalidation =
            latest_prompt_surface_snapshot_invalidation(&session).expect("invalidation");

        assert_eq!(
            invalidation.severity,
            rocode_provider::cache::CacheBustSeverity::SoftDegradation
        );
        assert_eq!(
            invalidation.changed_fields,
            vec!["ingressPolicyHash".to_string()]
        );
    }

    #[test]
    fn latest_execution_preflight_summary_prefers_tool_call_state_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let call_id = "call-1";

        let mut assistant = rocode_session::SessionMessage::assistant(session.id.clone());
        assistant.add_tool_call(
            call_id,
            "media_inspect",
            serde_json::json!({ "file_path": "/tmp/sample.pdf" }),
        );
        if let Some(rocode_session::MessagePart {
            part_type: rocode_session::PartType::ToolCall { status, state, .. },
            ..
        }) = assistant.parts.last_mut()
        {
            *status = rocode_session::ToolCallStatus::Completed;
            *state = Some(rocode_session::ToolState::Completed {
                input: serde_json::json!({ "file_path": "/tmp/sample.pdf" }),
                output: "ok".to_string(),
                title: "Media Inspect".to_string(),
                metadata: sample_execution_preflight_metadata(
                    rocode_tool::ExecutionPreflightStatus::Ready,
                    Vec::new(),
                    1,
                ),
                time: rocode_session::CompletedTime {
                    start: 1,
                    end: 2,
                    compacted: None,
                },
                attachments: None,
            });
        }
        session.push_message(assistant);

        let mut tool = rocode_session::SessionMessage::tool(session.id.clone());
        tool.add_tool_result(call_id, "delegated result", false);
        if let Some(rocode_session::MessagePart {
            part_type: rocode_session::PartType::ToolResult { metadata, .. },
            ..
        }) = tool.parts.last_mut()
        {
            *metadata = Some(sample_execution_preflight_metadata(
                rocode_tool::ExecutionPreflightStatus::SoftWarn,
                vec![rocode_tool::ExecutionPreflightIssue {
                    severity: rocode_tool::ExecutionPreflightSeverity::SoftWarn,
                    code: "attachment_missing".to_string(),
                    message: "attachment payload missing".to_string(),
                }],
                0,
            ));
        }
        session.push_message(tool);

        let summary = latest_execution_preflight_summary(&session).expect("summary");

        assert_eq!(summary.tool_call_id, call_id);
        assert_eq!(summary.tool_name.as_deref(), Some("media_inspect"));
        assert_eq!(
            summary.source,
            SessionExecutionPreflightSource::ToolCallState
        );
        assert_eq!(summary.status, rocode_tool::ExecutionPreflightStatus::Ready);
        assert_eq!(summary.attachment_count, 1);
        assert!(summary.issues.is_empty());
    }

    #[test]
    fn latest_execution_preflight_summary_falls_back_to_tool_result_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let call_id = "call-2";

        let mut assistant = rocode_session::SessionMessage::assistant(session.id.clone());
        assistant.add_tool_call(
            call_id,
            "media_inspect",
            serde_json::json!({ "file_path": "/tmp/sample.pdf" }),
        );
        session.push_message(assistant);

        let mut tool = rocode_session::SessionMessage::tool(session.id.clone());
        tool.add_tool_result(call_id, "delegated result", false);
        if let Some(rocode_session::MessagePart {
            part_type: rocode_session::PartType::ToolResult { metadata, .. },
            ..
        }) = tool.parts.last_mut()
        {
            *metadata = Some(sample_execution_preflight_metadata(
                rocode_tool::ExecutionPreflightStatus::SoftWarn,
                vec![rocode_tool::ExecutionPreflightIssue {
                    severity: rocode_tool::ExecutionPreflightSeverity::SoftWarn,
                    code: "attachment_missing".to_string(),
                    message: "attachment payload missing".to_string(),
                }],
                0,
            ));
        }
        session.push_message(tool);

        let summary = latest_execution_preflight_summary(&session).expect("summary");

        assert_eq!(summary.tool_call_id, call_id);
        assert_eq!(summary.tool_name.as_deref(), Some("media_inspect"));
        assert_eq!(
            summary.source,
            SessionExecutionPreflightSource::ToolResultPart
        );
        assert_eq!(
            summary.status,
            rocode_tool::ExecutionPreflightStatus::SoftWarn
        );
        assert_eq!(summary.issues.len(), 1);
    }

    #[test]
    fn telemetry_snapshot_serializes_authority_contract_fields() {
        let mut runtime = SessionRuntimeState::new("session-1");
        runtime.active_stage_id = Some("stage-1".to_string());
        runtime.active_stage_count = 1;
        runtime.usage = Some(SessionUsage {
            input_tokens: 10,
            output_tokens: 20,
            reasoning_tokens: 3,
            cache_write_tokens: 4,
            cache_read_tokens: 5,
            cache_miss_tokens: 0,
            context_tokens: 0,
            total_cost: 0.12,
        });

        let snapshot = SessionTelemetrySnapshot {
            runtime,
            stages: vec![StageSummary {
                stage_id: "stage-1".to_string(),
                stage_name: "Plan".to_string(),
                index: Some(1),
                total: Some(2),
                step: Some(1),
                step_total: Some(3),
                status: StageStatus::Waiting,
                prompt_tokens: Some(11),
                context_tokens: None,
                completion_tokens: Some(7),
                reasoning_tokens: Some(5),
                cache_read_tokens: Some(2),
                cache_miss_tokens: Some(0),
                cache_write_tokens: Some(1),
                focus: Some("inspect scheduler".to_string()),
                last_event: Some("scheduler.stage.waiting".to_string()),
                waiting_on: Some("tool".to_string()),
                estimated_context_tokens: Some(99),
                skill_tree_budget: Some(512),
                skill_tree_truncation_strategy: Some("head".to_string()),
                skill_tree_truncated: Some(true),
                retry_attempt: Some(2),
                active_agent_count: 1,
                active_tool_count: 2,
                child_session_count: 0,
                primary_child_session_id: None,
            }],
            topology: SessionExecutionTopology {
                session_id: "session-1".to_string(),
                active_count: 1,
                done_count: 0,
                running_count: 0,
                waiting_count: 1,
                cancelling_count: 0,
                retry_count: 0,
                updated_at: Some(123),
                roots: Vec::new(),
            },
            usage: SessionUsage {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 3,
                cache_write_tokens: 4,
                cache_read_tokens: 5,
                cache_miss_tokens: 0,
                context_tokens: 0,
                total_cost: 0.12,
            },
            memory: None,
            cache_bust_summary: None,
            prompt_surface_runtime_snapshot: Some(serde_json::json!({
                "generation": 7,
                "invalidation": {
                    "reason": "prompt surface runtime changed: toolSurfaceHash"
                },
            })),
            prompt_surface_snapshot_invalidation: Some(PromptSurfaceSnapshotInvalidationSummary {
                severity: rocode_provider::cache::CacheBustSeverity::HardBust,
                reason: "prompt surface runtime changed: toolSurfaceHash".to_string(),
                changed_fields: vec!["toolSurfaceHash".to_string()],
            }),
            ingress_stabilization: Some(serde_json::json!({
                "source": "web",
                "policy": rocode_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY,
                "batch_count": 1,
            })),
            execution_preflight_summary: Some(SessionExecutionPreflightSummary {
                tool_call_id: "call-1".to_string(),
                tool_name: Some("media_inspect".to_string()),
                source: SessionExecutionPreflightSource::ToolCallState,
                runner: "read".to_string(),
                subject: "/tmp/sample.pdf".to_string(),
                status: rocode_tool::ExecutionPreflightStatus::Ready,
                issues: Vec::new(),
                attachment_count: 1,
            }),
            provider_diagnostic_summary: Some(rocode_provider::ProviderDiagnosticSummary {
                severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
                source: rocode_provider::ProviderDiagnosticSource::ApiErrorRewrite,
                code: "thinking_replay_rejected".to_string(),
                provider_id: "deepseek".to_string(),
                model_id: Some("deepseek-reasoner".to_string()),
                message: "rejected replay".to_string(),
            }),
        };

        let value = serde_json::to_value(&snapshot).expect("snapshot should serialize");

        assert!(value.get("runtime").is_some());
        assert!(value.get("stages").is_some());
        assert!(value.get("topology").is_some());
        assert!(value.get("usage").is_some());
        assert_eq!(value["runtime"]["active_stage_id"], "stage-1");
        assert_eq!(value["stages"][0]["status"], "waiting");
        assert_eq!(value["stages"][0]["skill_tree_truncated"], true);
        assert_eq!(value["topology"]["waiting_count"], 1);
        assert_eq!(value["usage"]["total_cost"], 0.12);
        assert_eq!(value["prompt_surface_runtime_snapshot"]["generation"], 7);
        assert_eq!(
            value["prompt_surface_snapshot_invalidation"]["changed_fields"][0],
            "toolSurfaceHash"
        );
        assert_eq!(
            value["ingress_stabilization"]["policy"],
            rocode_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY
        );
        assert_eq!(value["execution_preflight_summary"]["runner"], "read");
        assert_eq!(
            value["execution_preflight_summary"]["source"],
            "tool_call_state"
        );
        assert_eq!(
            value["provider_diagnostic_summary"]["code"],
            "thinking_replay_rejected"
        );
        assert_eq!(
            value["provider_diagnostic_summary"]["provider_id"],
            "deepseek"
        );
    }

    #[test]
    fn persisted_telemetry_snapshot_defaults_version_when_missing() {
        let value = serde_json::json!({
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "reasoning_tokens": 3,
                "cache_write_tokens": 4,
                "cache_read_tokens": 5,
                "total_cost": 0.5
            },
            "stage_summaries": [],
            "last_run_status": "completed",
            "updated_at": 123
        });

        let parsed = serde_json::from_value::<rocode_session::SessionTelemetrySnapshot>(value)
            .expect("snapshot should deserialize with default version");

        assert_eq!(
            parsed.version,
            rocode_session::SessionTelemetrySnapshotVersion::V1
        );
    }

    #[test]
    fn session_insights_builds_multimodal_detail_from_last_user_message() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let user = session.add_user_message("[audio input]");
        user.metadata
            .insert("multimodal_kinds".to_string(), serde_json::json!(["audio"]));
        user.metadata.insert(
            "multimodal_badges".to_string(),
            serde_json::json!(["audio"]),
        );
        user.metadata.insert(
            "multimodal_compact_label".to_string(),
            serde_json::json!("[audio input]"),
        );
        user.metadata.insert(
            "multimodal_resolved_model".to_string(),
            serde_json::json!("openai/gpt-audio"),
        );
        user.metadata.insert(
            "multimodal_preflight".to_string(),
            serde_json::json!({
                "warnings": ["Audio accepted."],
                "unsupported_parts": [],
                "recommended_downgrade": null,
                "hard_block": false
            }),
        );
        user.metadata.insert(
            "multimodal_transport".to_string(),
            serde_json::json!({
                "replaced_parts": ["voice.wav"],
                "warnings": [
                    "ERROR: Cannot read \"voice.wav\" (this model does not support audio input). Inform the user."
                ]
            }),
        );
        user.add_file(
            "data:audio/wav;base64,UklGRg==".to_string(),
            "voice.wav".to_string(),
            "audio/wav".to_string(),
        );
        let user_id = user.id.clone();

        let insight = build_session_multimodal_insight(&session).expect("multimodal insight");
        assert_eq!(insight.user_message_id, user_id);
        assert_eq!(insight.attachment_count, 1);
        assert_eq!(insight.kinds, vec!["audio".to_string()]);
        assert_eq!(insight.badges, vec!["audio".to_string()]);
        assert_eq!(insight.resolved_model.as_deref(), Some("openai/gpt-audio"));
        assert_eq!(insight.attachments.len(), 1);
        assert_eq!(insight.attachments[0].filename, "voice.wav");
        assert_eq!(insight.attachments[0].mime, "audio/wav");
        assert_eq!(insight.warnings, vec!["Audio accepted.".to_string()]);
        assert_eq!(
            insight.transport_replaced_parts,
            vec!["voice.wav".to_string()]
        );
        assert_eq!(insight.transport_warnings.len(), 1);
        assert!(insight.transport_warnings[0].contains("does not support audio input"));
        assert!(!insight.hard_block);
    }

    #[tokio::test]
    async fn session_insights_returns_persisted_snapshot() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions.create("project", "/tmp/project");
            session.set_title("Telemetry Session");
            let user = session.add_user_message("[audio input]");
            user.metadata
                .insert("multimodal_kinds".to_string(), serde_json::json!(["audio"]));
            user.metadata.insert(
                "multimodal_compact_label".to_string(),
                serde_json::json!("[audio input]"),
            );
            user.metadata.insert(
                "multimodal_resolved_model".to_string(),
                serde_json::json!("openai/gpt-audio"),
            );
            user.metadata.insert(
                "multimodal_preflight".to_string(),
                serde_json::json!({
                    "warnings": ["Audio accepted."],
                    "unsupported_parts": [],
                    "recommended_downgrade": null,
                    "hard_block": false
                }),
            );
            user.metadata.insert(
                "multimodal_transport".to_string(),
                serde_json::json!({
                    "replaced_parts": ["voice.wav"],
                    "warnings": [
                        "ERROR: Cannot read \"voice.wav\" (this model does not support audio input). Inform the user."
                    ]
                }),
            );
            user.add_file(
                "data:audio/wav;base64,UklGRg==".to_string(),
                "voice.wav".to_string(),
                "audio/wav".to_string(),
            );
            persist_session_telemetry_snapshot(
                &mut session,
                &rocode_session::SessionTelemetrySnapshot {
                    version: SessionTelemetrySnapshotVersion::V1,
                    memory: None,
                    usage: rocode_types::SessionUsage {
                        input_tokens: 10,
                        output_tokens: 20,
                        reasoning_tokens: 3,
                        cache_write_tokens: 4,
                        cache_read_tokens: 5,
                        cache_miss_tokens: 0,
                        context_tokens: 0,
                        total_cost: 0.25,
                    },
                    stage_summaries: vec![],
                    last_run_status: "completed".to_string(),
                    updated_at: 123,
                },
            )
            .expect("snapshot should persist");
            session.insert_metadata(
                rocode_memory::MEMORY_FROZEN_SNAPSHOT_METADATA_KEY.to_string(),
                serde_json::to_value(PersistedMemorySnapshot {
                    packet: rocode_types::MemoryRetrievalPacket {
                        generated_at: 200,
                        snapshot: true,
                        query: None,
                        scopes: vec![rocode_types::MemoryScope::WorkspaceShared],
                        items: vec![],
                        note: Some("frozen".to_string()),
                        budget_limit: Some(8),
                    },
                    rendered_block: Some("memory block".to_string()),
                })
                .expect("frozen memory snapshot should serialize"),
            );
            session.insert_metadata(
                rocode_memory::MEMORY_LAST_PREFETCH_METADATA_KEY.to_string(),
                serde_json::to_value(rocode_types::MemoryRetrievalPacket {
                    generated_at: 250,
                    snapshot: false,
                    query: Some("latest prompt".to_string()),
                    scopes: vec![rocode_types::MemoryScope::WorkspaceShared],
                    items: vec![],
                    note: Some("prefetch".to_string()),
                    budget_limit: Some(6),
                })
                .expect("prefetch packet should serialize"),
            );
            let id = session.id.clone();
            sessions.update(session);
            id
        };

        let Json(response) = get_session_insights(State(state), Path(session_id.clone()))
            .await
            .expect("insights route should succeed");

        assert_eq!(response.id, session_id);
        assert_eq!(response.title, "Telemetry Session");
        assert_eq!(response.directory, "/tmp/project");
        assert_eq!(
            response
                .telemetry
                .as_ref()
                .map(|snapshot| snapshot.last_run_status.as_str()),
            Some("completed")
        );
        assert_eq!(
            response
                .memory
                .as_ref()
                .map(|memory| memory.summary.last_prefetch_query.as_deref()),
            Some(Some("latest prompt"))
        );
        assert_eq!(
            response
                .multimodal
                .as_ref()
                .and_then(|multimodal| multimodal.resolved_model.as_deref()),
            Some("openai/gpt-audio")
        );
        assert_eq!(
            response
                .multimodal
                .as_ref()
                .map(|multimodal| multimodal.attachment_count),
            Some(1)
        );
        assert_eq!(
            response
                .multimodal
                .as_ref()
                .map(|multimodal| multimodal.transport_replaced_parts.clone()),
            Some(vec!["voice.wav".to_string()])
        );
        assert!(response.effective_policy.is_some());
        assert_eq!(
            response
                .effective_policy
                .as_ref()
                .map(|policy| policy.session_id.as_str()),
            Some(session_id.as_str())
        );
    }

    #[tokio::test]
    async fn persist_session_telemetry_metadata_emits_snapshot_hook() {
        let state = Arc::new(ServerState::new());
        let hook_name = format!(
            "telemetry-snapshot-updated-{}",
            uuid::Uuid::new_v4().simple()
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        global()
            .register(Hook::new(
                &hook_name,
                HookEvent::TelemetrySnapshotUpdated,
                move |ctx| {
                    let tx = tx.clone();
                    async move {
                        let _ = tx.send(ctx);
                        Ok(())
                    }
                },
            ))
            .await;

        let mut session = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project")
        };
        let session_id = session.id.clone();
        let assistant = session.add_assistant_message();
        assistant.usage = Some(MessageUsage {
            input_tokens: 10,
            output_tokens: 20,
            reasoning_tokens: 3,
            cache_write_tokens: 4,
            cache_read_tokens: 5,
            cache_miss_tokens: 0,
            context_tokens: 0,
            total_cost: 0.25,
        });

        let exec_ctx = ExecutionContext {
            session_id: session_id.clone(),
            workdir: "/tmp/project".to_string(),
            agent_name: "test-agent".to_string(),
            metadata: HashMap::new(),
        };

        emit_scheduler_stage_message(SchedulerStageMessageInput {
            state: &state,
            session_id: &session_id,
            scheduler_profile: "prometheus",
            stage_name: "plan",
            stage_index: 1,
            stage_total: 1,
            content: "## Plan\n\n- summarize runtime",
            exec_ctx: &exec_ctx,
            output_hook: None,
        })
        .await;

        state
            .runtime_telemetry
            .record_session_usage(
                &session_id,
                None,
                SessionUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                    reasoning_tokens: 3,
                    cache_write_tokens: 4,
                    cache_read_tokens: 5,
                    cache_miss_tokens: 0,
                    context_tokens: 0,
                    total_cost: 0.25,
                },
            )
            .await;

        persist_session_telemetry_metadata(&state, &mut session).await;

        let hook_ctx = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("hook should fire")
            .expect("hook payload should arrive");
        assert_eq!(hook_ctx.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(
            hook_ctx.get("sessionID"),
            Some(&serde_json::json!(session_id))
        );
        assert_eq!(
            hook_ctx
                .get("snapshot")
                .and_then(|value| value.get("usage"))
                .and_then(|value| value.get("input_tokens")),
            Some(&serde_json::json!(10))
        );
        assert_eq!(
            hook_ctx
                .get("snapshot")
                .and_then(|value| value.get("stage_summaries"))
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(1)
        );

        let _ = global()
            .remove(&HookEvent::TelemetrySnapshotUpdated, &hook_name)
            .await;
    }
}
