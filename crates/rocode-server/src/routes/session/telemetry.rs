use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use rocode_command::stage_protocol::StageSummary;
use rocode_multimodal::PersistedMultimodalExplain;
use rocode_session::prompt::{explain_session_cache_semantics, explain_session_context};
use rocode_session::{
    load_session_telemetry_snapshot, persist_session_telemetry_snapshot,
    session_last_run_status_label, Session, SessionUsage,
};
use rocode_types::{
    ContextCompactionSummary, ContextPressureGovernanceSummary, PromptSurfaceEvidenceSummary,
    SessionCacheSemanticsSummary, SessionContextClosureContract, SessionContextExplain,
    SessionDiagnosticsSidecar, SessionInsightsResponse, SessionMemoryTelemetrySummary,
    SessionMultimodalAttachmentInfo, SessionMultimodalInsight, SessionOwnershipSummary,
    SessionUsageBooks, WorkflowUsageSummary,
};
use serde::Serialize;

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
    pub usage_books: SessionUsageBooks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<SessionMemoryTelemetrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_evidence: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_explain: Option<SessionContextExplain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<SessionOwnershipSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_compaction_summary: Option<ContextCompactionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pressure_governance_summary: Option<ContextPressureGovernanceSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_semantics: Option<SessionCacheSemanticsSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_closure_contract: Option<SessionContextClosureContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_evidence: Option<PromptSurfaceEvidenceSummary>,
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
    let tree_observation = {
        let sessions = state.sessions.lock().await;
        session_tree_observation_for_session(&sessions, session_id)
    };
    let usage_books = SessionUsageBooks {
        request_context_tokens: session.latest_request_context_tokens(),
        live_context_tokens: usage.live_context_tokens(),
        workflow_cumulative: tree_observation.workflow_cumulative.clone(),
    };

    let stages = state
        .runtime_telemetry
        .list_stage_summaries(session_id)
        .await;
    let topology = build_session_execution_topology_snapshot(state, session_id, session).await;
    let memory = build_session_memory_telemetry(state, session).await;
    let diagnostics = SessionDiagnosticsSidecar::derive_from_session(session);
    let cache_evidence = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_cache_evidence_value);
    let typed_cache_evidence = cache_evidence.clone().and_then(|value| {
        serde_json::from_value::<rocode_provider::cache::CacheEvidenceSummary>(value).ok()
    });
    let context_explain = Some(explain_session_context(
        session,
        Some(usage_books.workflow_cumulative.total_tokens()),
    ));
    let ownership = Some(session.ownership_summary());
    let context_compaction_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_context_compaction_record_value)
        .and_then(|value| serde_json::from_value(value).ok());
    let context_pressure_governance_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::context_pressure_governance_summary_value)
        .and_then(|value| serde_json::from_value(value).ok());
    let prompt_surface_evidence = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_prompt_surface_evidence_value)
        .and_then(|value| serde_json::from_value(value).ok());
    let cache_semantics = context_explain.as_ref().map(|context_explain| {
        explain_session_cache_semantics(
            context_explain,
            context_compaction_summary.as_ref(),
            typed_cache_evidence.as_ref(),
            prompt_surface_evidence.as_ref(),
        )
    });
    let context_closure_contract = build_context_closure_contract(
        context_explain.as_ref(),
        &usage_books,
        cache_semantics.as_ref(),
        context_compaction_summary.as_ref(),
        context_pressure_governance_summary.as_ref(),
        tree_observation.attached_subtree_session_count,
    );
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
        usage_books,
        memory,
        cache_evidence,
        context_explain,
        ownership,
        context_compaction_summary,
        context_pressure_governance_summary,
        cache_semantics,
        context_closure_contract,
        prompt_surface_evidence,
        ingress_stabilization,
        execution_preflight_summary,
        provider_diagnostic_summary,
    })
}

#[derive(Debug, Clone, Default)]
struct SessionTreeObservation {
    workflow_cumulative: WorkflowUsageSummary,
    attached_subtree_session_count: usize,
}

fn session_tree_observation_for_session(
    sessions: &rocode_session::SessionManager,
    root_session_id: &str,
) -> SessionTreeObservation {
    let mut observation = SessionTreeObservation::default();
    let mut pending = vec![(root_session_id.to_string(), true)];

    while let Some((session_id, is_root)) = pending.pop() {
        let Some(session) = sessions.get(&session_id) else {
            continue;
        };
        observation
            .workflow_cumulative
            .accumulate_session_usage(&session.get_usage());
        let attached_children = sessions
            .attached_sessions(&session_id)
            .into_iter()
            .map(|child| child.record().id.clone())
            .collect::<Vec<_>>();
        if !is_root {
            observation.attached_subtree_session_count += 1;
        }
        pending.extend(
            attached_children
                .into_iter()
                .map(|child_id| (child_id, false)),
        );
    }

    observation
}

fn pressure_percent(tokens: Option<u64>, limit_tokens: Option<u64>) -> Option<u64> {
    let tokens = tokens?;
    let limit = limit_tokens?;
    (limit > 0).then_some(tokens.saturating_mul(100) / limit)
}

fn build_context_closure_contract(
    context_explain: Option<&SessionContextExplain>,
    usage_books: &SessionUsageBooks,
    cache_semantics: Option<&SessionCacheSemanticsSummary>,
    context_compaction_summary: Option<&ContextCompactionSummary>,
    context_pressure_governance_summary: Option<&ContextPressureGovernanceSummary>,
    attached_subtree_session_count: usize,
) -> Option<SessionContextClosureContract> {
    let context_explain = context_explain?;
    let cache_semantics = cache_semantics
        .cloned()
        .unwrap_or(SessionCacheSemanticsSummary {
            basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
            api_view_messages: context_explain.api_view_messages,
            trimmed_model_visible_messages: context_explain
                .raw_model_visible_messages
                .saturating_sub(context_explain.api_view_messages),
            boundary: None,
            cache_evidence: None,
            prompt_surface_evidence: None,
            label: None,
        });

    let prefix_change_detected = cache_semantics
        .boundary
        .as_ref()
        .is_some_and(|boundary| boundary.likely_changed_prefix)
        || cache_semantics
            .cache_evidence
            .as_ref()
            .is_some_and(|summary| summary.severity > rocode_types::SessionCacheSeverity::Stable)
        || cache_semantics
            .prompt_surface_evidence
            .as_ref()
            .is_some_and(|summary| summary.severity > rocode_types::SessionCacheSeverity::Stable);

    let (request_pressure_percent, live_pressure_percent) =
        if let Some(summary) = context_pressure_governance_summary {
            (
                summary.request_pressure_percent.or_else(|| {
                    pressure_percent(summary.request_context_tokens, summary.limit_tokens)
                }),
                summary.live_pressure_percent.or_else(|| {
                    pressure_percent(summary.live_context_tokens, summary.limit_tokens)
                }),
            )
        } else if let Some(summary) = context_compaction_summary {
            (
                pressure_percent(summary.request_context_tokens, summary.limit_tokens),
                pressure_percent(summary.live_context_tokens, summary.limit_tokens),
            )
        } else {
            (None, None)
        };

    let (cache_explainability_source, cache_explainability_severity, cache_explainability_text) =
        if let Some(summary) = cache_semantics.cache_evidence.as_ref().filter(|summary| {
            summary.severity > rocode_types::SessionCacheSeverity::Stable
                && !matches!(summary.status.as_str(), "stable" | "cold_start")
        }) {
            (
                rocode_types::SessionCacheExplainabilitySource::CacheEvidence,
                Some(summary.severity),
                cache_semantics
                    .label
                    .clone()
                    .or_else(|| summary.primary_cause.clone()),
            )
        } else if let Some(summary) = cache_semantics
            .prompt_surface_evidence
            .as_ref()
            .filter(|summary| summary.severity > rocode_types::SessionCacheSeverity::Stable)
        {
            (
                rocode_types::SessionCacheExplainabilitySource::SurfaceEvidence,
                Some(summary.severity),
                cache_semantics
                    .label
                    .clone()
                    .or_else(|| Some(summary.reason.clone())),
            )
        } else if cache_semantics
            .boundary
            .as_ref()
            .is_some_and(|boundary| boundary.possible_cache_evidence)
        {
            (
                rocode_types::SessionCacheExplainabilitySource::BoundaryEvidence,
                Some(rocode_types::SessionCacheSeverity::MediumChange),
                cache_semantics.label.clone().or_else(|| {
                    cache_semantics
                        .boundary
                        .as_ref()
                        .and_then(|boundary| boundary.reason.clone())
                }),
            )
        } else {
            (
                rocode_types::SessionCacheExplainabilitySource::None,
                None,
                cache_semantics.label.clone(),
            )
        };
    let cache_issue_present = !matches!(
        cache_explainability_source,
        rocode_types::SessionCacheExplainabilitySource::None
    );
    let owner_session_cumulative_tokens = context_explain.owner_session_cumulative_tokens;
    let workflow_cumulative_tokens = usage_books.workflow_cumulative.total_tokens();
    let attached_subtree_cumulative_tokens =
        workflow_cumulative_tokens.saturating_sub(owner_session_cumulative_tokens);

    Some(SessionContextClosureContract {
        prefix_stability: rocode_types::SessionPrefixStabilityContract {
            basis: cache_semantics.basis,
            tracked_on_api_view: matches!(
                cache_semantics.basis,
                rocode_types::SessionCacheSemanticsBasis::ApiView
            ),
            api_view_messages: cache_semantics.api_view_messages,
            trimmed_model_visible_messages: cache_semantics.trimmed_model_visible_messages,
            prefix_change_detected,
            explanation: cache_semantics.label.clone(),
        },
        compaction_boundary: rocode_types::SessionCompactionBoundaryContract {
            boundary_recorded: context_compaction_summary.is_some()
                || context_pressure_governance_summary.is_some(),
            phase: context_pressure_governance_summary
                .map(|summary| summary.phase.clone())
                .or_else(|| context_compaction_summary.and_then(|summary| summary.phase.clone())),
            trigger: context_pressure_governance_summary
                .map(|summary| summary.trigger.clone())
                .or_else(|| context_compaction_summary.map(|summary| summary.trigger.clone())),
            reason: context_pressure_governance_summary
                .and_then(|summary| summary.reason.clone())
                .or_else(|| context_compaction_summary.and_then(|summary| summary.reason.clone())),
            governance_status: context_pressure_governance_summary.map(|summary| summary.status),
            request_pressure_percent,
            live_pressure_percent,
            compaction_attempted: context_pressure_governance_summary
                .map(|summary| summary.compaction_attempted)
                .unwrap_or_else(|| context_compaction_summary.is_some()),
            compaction_succeeded: context_pressure_governance_summary
                .map(|summary| summary.compaction_succeeded)
                .unwrap_or_else(|| context_compaction_summary.is_some()),
            blocking: context_pressure_governance_summary
                .map(|summary| summary.blocking)
                .unwrap_or(false),
        },
        cache_explainability: rocode_types::SessionCacheExplainabilityContract {
            issue_present: cache_issue_present,
            explained: !cache_issue_present || cache_explainability_text.is_some(),
            source: cache_explainability_source,
            severity: cache_explainability_severity,
            explanation: cache_explainability_text,
        },
        child_history_isolation: rocode_types::SessionChildHistoryIsolationContract {
            attached_subtree_session_count,
            owner_session_cumulative_tokens,
            workflow_cumulative_tokens,
            attached_subtree_cumulative_tokens,
            owner_live_context_tokens: usage_books.live_context_tokens,
            owner_local_live_prefix: true,
            child_history_in_live_prefix_detected: false,
            explanation: if attached_subtree_session_count > 0 {
                "Attached subtree usage contributes to workflow cumulative only; API view and live prefix remain owner-local."
                    .to_string()
            } else {
                "No attached subtree sessions were observed; the live prefix remains owner-local."
                    .to_string()
            },
        },
    })
}

#[cfg(test)]
fn latest_cache_evidence(session: &Session) -> Option<serde_json::Value> {
    diagnostics_sidecar(session).and_then(|sidecar| sidecar.latest_cache_evidence_value())
}

#[cfg(test)]
fn latest_context_compaction_summary(session: &Session) -> Option<ContextCompactionSummary> {
    diagnostics_sidecar(session)
        .and_then(|sidecar| sidecar.latest_context_compaction_record_value())
        .and_then(|value| serde_json::from_value(value).ok())
}

#[cfg(test)]
fn latest_prompt_surface_evidence(session: &Session) -> Option<PromptSurfaceEvidenceSummary> {
    diagnostics_sidecar(session)
        .and_then(|sidecar| sidecar.latest_prompt_surface_evidence_value())
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
    fn latest_cache_evidence_reads_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_provider::cache::CACHE_EVIDENCE_METADATA_KEY.to_string(),
            serde_json::json!({
                "status": "degraded",
                "severity": "MediumChange",
                "primary_cause": "prefix changed before the stable boundary",
                "change_count": 1,
            }),
        );

        let summary = latest_cache_evidence(&session).expect("summary");

        assert_eq!(summary["status"], "degraded");
        assert_eq!(summary["severity"], "MediumChange");
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
    fn latest_prompt_surface_evidence_reads_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_session::prompt::PROMPT_SURFACE_EVIDENCE_METADATA_KEY.to_string(),
            serde_json::json!({
                "severity": "MediumChange",
                "reason": "surface changed: outputProjectionPolicyHash",
                "changed_fields": ["outputProjectionPolicyHash"],
            }),
        );

        let evidence = latest_prompt_surface_evidence(&session).expect("evidence");

        assert_eq!(
            evidence.severity,
            rocode_types::SessionCacheSeverity::MediumChange
        );
        assert_eq!(
            evidence.reason,
            "surface changed: outputProjectionPolicyHash"
        );
        assert_eq!(
            evidence.changed_fields,
            vec!["outputProjectionPolicyHash".to_string()]
        );
    }

    #[test]
    fn latest_prompt_surface_evidence_falls_back_to_snapshot_payload() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        session.insert_metadata(
            rocode_session::prompt::PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::json!({
                "generation": 7,
                "evidence": {
                    "severity": "LowChange",
                    "reason": "surface changed: ingressPolicyHash",
                    "changed_fields": ["ingressPolicyHash"]
                }
            }),
        );

        let evidence = latest_prompt_surface_evidence(&session).expect("evidence");

        assert_eq!(
            evidence.severity,
            rocode_types::SessionCacheSeverity::LowChange
        );
        assert_eq!(
            evidence.changed_fields,
            vec!["ingressPolicyHash".to_string()]
        );
    }

    #[test]
    fn context_closure_contract_tracks_compaction_and_cache_explainability() {
        let usage_books = SessionUsageBooks {
            request_context_tokens: Some(88_000),
            live_context_tokens: Some(82_000),
            workflow_cumulative: WorkflowUsageSummary {
                input_tokens: 120_000,
                output_tokens: 18_000,
                reasoning_tokens: 5_000,
                cache_write_tokens: 2_000,
                cache_read_tokens: 34_000,
                cache_miss_tokens: 7_000,
                total_cost: 1.60,
            },
        };
        let context_explain = SessionContextExplain {
            resolved_model: Some("openai/gpt-4o".to_string()),
            fork: None,
            raw_history_messages: 18,
            raw_model_visible_messages: 15,
            api_view_messages: 8,
            api_view_estimated_input_tokens: Some(92_000),
            api_view_body_chars: Some(360_000),
            live_context_tokens: Some(82_000),
            last_request_context_tokens: Some(88_000),
            owner_session_cumulative_tokens: 104_000,
            workflow_cumulative_tokens: usage_books.workflow_cumulative.total_tokens(),
        };
        let context_compaction_summary = ContextCompactionSummary {
            trigger: "auto_preflight".to_string(),
            phase: Some("prompt.pre_request".to_string()),
            reason: Some("request_view_threshold".to_string()),
            forced: false,
            request_context_tokens: Some(92_000),
            live_context_tokens: Some(82_000),
            limit_tokens: Some(100_000),
            body_chars: Some(360_000),
            message_count_before: Some(15),
            compacted_message_count: Some(7),
            kept_message_count: Some(8),
            summary: Some("Compacted 7 messages.".to_string()),
        };
        let cache_semantics = SessionCacheSemanticsSummary {
            basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
            api_view_messages: 8,
            trimmed_model_visible_messages: 7,
            boundary: Some(rocode_types::SessionCacheBoundarySummary {
                kind: rocode_types::SessionCacheBoundaryKind::Compaction,
                trigger: "auto_preflight".to_string(),
                phase: Some("prompt.pre_request".to_string()),
                reason: Some("request_view_threshold".to_string()),
                message_count_before: Some(15),
                compacted_message_count: Some(7),
                kept_message_count: Some(8),
                trimmed_model_visible_messages: 7,
                likely_changed_prefix: true,
                possible_cache_evidence: true,
            }),
            cache_evidence: Some(rocode_types::SessionCacheEvidenceExplain {
                status: "degraded".to_string(),
                severity: rocode_types::SessionCacheSeverity::MediumChange,
                primary_cause: Some("prefix changed before the stable boundary".to_string()),
                change_count: 1,
            }),
            prompt_surface_evidence: Some(PromptSurfaceEvidenceSummary {
                severity: rocode_types::SessionCacheSeverity::LowChange,
                reason: "surface changed: ingressPolicyHash".to_string(),
                changed_fields: vec!["ingressPolicyHash".to_string()],
            }),
            label: Some("boundary recorded · prefix changed".to_string()),
        };
        let governance_summary = ContextPressureGovernanceSummary {
            trigger: "step_checkpoint_gate".to_string(),
            phase: "scheduler.step_checkpoint".to_string(),
            status: rocode_types::ContextPressureGovernanceStatus::Compacted,
            reason: Some("request view exceeded safe checkpoint limit".to_string()),
            request_context_tokens: Some(95_000),
            live_context_tokens: Some(82_000),
            limit_tokens: Some(100_000),
            body_chars: Some(360_000),
            request_pressure_percent: Some(95),
            live_pressure_percent: Some(82),
            compaction_attempted: true,
            compaction_succeeded: true,
            blocking: false,
        };

        let contract = build_context_closure_contract(
            Some(&context_explain),
            &usage_books,
            Some(&cache_semantics),
            Some(&context_compaction_summary),
            Some(&governance_summary),
            2,
        )
        .expect("contract should build");

        assert!(contract.prefix_stability.tracked_on_api_view);
        assert!(contract.prefix_stability.prefix_change_detected);
        assert_eq!(
            contract.prefix_stability.explanation.as_deref(),
            Some("boundary recorded · prefix changed")
        );
        assert!(contract.compaction_boundary.boundary_recorded);
        assert_eq!(
            contract.compaction_boundary.phase.as_deref(),
            Some("scheduler.step_checkpoint")
        );
        assert_eq!(
            contract.compaction_boundary.request_pressure_percent,
            Some(95)
        );
        assert!(contract.compaction_boundary.compaction_attempted);
        assert!(contract.compaction_boundary.compaction_succeeded);
        assert!(!contract.compaction_boundary.blocking);
        assert!(contract.cache_explainability.issue_present);
        assert!(contract.cache_explainability.explained);
        assert_eq!(
            contract.cache_explainability.source,
            rocode_types::SessionCacheExplainabilitySource::CacheEvidence
        );
        assert_eq!(
            contract.cache_explainability.severity,
            Some(rocode_types::SessionCacheSeverity::MediumChange)
        );
        assert_eq!(
            contract
                .child_history_isolation
                .attached_subtree_session_count,
            2
        );
        assert_eq!(
            contract
                .child_history_isolation
                .attached_subtree_cumulative_tokens,
            usage_books.workflow_cumulative.total_tokens()
                - context_explain.owner_session_cumulative_tokens
        );
        assert!(contract.child_history_isolation.owner_local_live_prefix);
        assert!(
            !contract
                .child_history_isolation
                .child_history_in_live_prefix_detected
        );
    }

    #[test]
    fn latest_context_compaction_summary_reads_assistant_metadata() {
        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            rocode_session::prompt::CONTEXT_COMPACTION_RECORD_METADATA_KEY.to_string(),
            serde_json::json!({
                "trigger": "overflow_recovery",
                "phase": "prompt.provider_overflow",
                "reason": "provider_overflow",
                "forced": true,
                "request_context_tokens": 120000_u64,
                "limit_tokens": 100000_u64,
                "body_chars": 480000,
                "message_count_before": 6,
                "compacted_message_count": 3,
                "kept_message_count": 3,
                "summary": "Compacted 3 messages."
            }),
        );

        let summary = latest_context_compaction_summary(&session).expect("summary");

        assert_eq!(summary.trigger, "overflow_recovery");
        assert_eq!(summary.phase.as_deref(), Some("prompt.provider_overflow"));
        assert_eq!(summary.reason.as_deref(), Some("provider_overflow"));
        assert!(summary.forced);
        assert_eq!(summary.compacted_message_count, Some(3));
        assert_eq!(summary.summary.as_deref(), Some("Compacted 3 messages."));
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
        let runtime_usage = SessionUsage {
            input_tokens: 10,
            output_tokens: 20,
            reasoning_tokens: 3,
            cache_write_tokens: 4,
            cache_read_tokens: 5,
            cache_miss_tokens: 0,
            context_tokens: 0,
            total_cost: 0.12,
        };
        runtime.usage = Some(runtime_usage.clone());

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
                attached_session_count: 0,
                primary_attached_session_id: None,
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
            usage_books: SessionUsageBooks {
                request_context_tokens: Some(10),
                live_context_tokens: None,
                workflow_cumulative: runtime_usage.workflow_usage_summary(),
            },
            memory: None,
            cache_evidence: None,
            context_explain: Some(SessionContextExplain {
                resolved_model: Some("openai/gpt-4o".to_string()),
                fork: None,
                raw_history_messages: 18,
                raw_model_visible_messages: 15,
                api_view_messages: 8,
                api_view_estimated_input_tokens: Some(92_000),
                api_view_body_chars: Some(360_000),
                live_context_tokens: None,
                last_request_context_tokens: Some(10),
                owner_session_cumulative_tokens: runtime_usage.session_cumulative_tokens(),
                workflow_cumulative_tokens: runtime_usage.workflow_usage_summary().total_tokens(),
            }),
            ownership: Some(SessionOwnershipSummary {
                context_kind: rocode_types::SessionContextKind::RootSessionContinuity,
                handoff_mode: rocode_types::SessionHandoffMode::SelfContinuity,
                owns_prompt_continuity: true,
                compact_owner: true,
                provider_model_role: rocode_types::SessionProviderModelRole::RequestShapeOnly,
                workflow_usage_role: rocode_types::SessionWorkflowUsageRole::ObservationOnly,
            }),
            context_compaction_summary: Some(ContextCompactionSummary {
                trigger: "auto_preflight".to_string(),
                phase: Some("prompt.pre_request".to_string()),
                reason: Some("request_view_threshold".to_string()),
                forced: false,
                request_context_tokens: Some(92_000),
                live_context_tokens: None,
                limit_tokens: Some(100_000),
                body_chars: Some(360_000),
                message_count_before: Some(14),
                compacted_message_count: Some(7),
                kept_message_count: Some(7),
                summary: Some("Compacted 7 messages.".to_string()),
            }),
            context_pressure_governance_summary: None,
            cache_semantics: Some(SessionCacheSemanticsSummary {
                basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
                api_view_messages: 8,
                trimmed_model_visible_messages: 7,
                boundary: Some(rocode_types::SessionCacheBoundarySummary {
                    kind: rocode_types::SessionCacheBoundaryKind::Compaction,
                    trigger: "auto_preflight".to_string(),
                    phase: Some("prompt.pre_request".to_string()),
                    reason: Some("request_view_threshold".to_string()),
                    message_count_before: Some(14),
                    compacted_message_count: Some(7),
                    kept_message_count: Some(7),
                    trimmed_model_visible_messages: 7,
                    likely_changed_prefix: true,
                    possible_cache_evidence: true,
                }),
                cache_evidence: Some(rocode_types::SessionCacheEvidenceExplain {
                    status: "degraded".to_string(),
                    severity: rocode_types::SessionCacheSeverity::MediumChange,
                    primary_cause: Some(
                        "prefix changed before the stable boundary"
                            .to_string(),
                    ),
                    change_count: 1,
                }),
                prompt_surface_evidence: Some(PromptSurfaceEvidenceSummary {
                    severity: rocode_types::SessionCacheSeverity::HighChange,
                    reason: "surface changed: toolSurfaceHash".to_string(),
                    changed_fields: vec!["toolSurfaceHash".to_string()],
                }),
                label: Some(
                    "boundary recorded · prefix changed"
                        .to_string(),
                ),
            }),
            context_closure_contract: Some(rocode_types::SessionContextClosureContract {
                prefix_stability: rocode_types::SessionPrefixStabilityContract {
                    basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
                    tracked_on_api_view: true,
                    api_view_messages: 8,
                    trimmed_model_visible_messages: 7,
                    prefix_change_detected: true,
                    explanation: Some(
                        "boundary recorded · prefix changed"
                            .to_string(),
                    ),
                },
                compaction_boundary: rocode_types::SessionCompactionBoundaryContract {
                    boundary_recorded: true,
                    phase: Some("prompt.pre_request".to_string()),
                    trigger: Some("auto_preflight".to_string()),
                    reason: Some("request_view_threshold".to_string()),
                    governance_status: None,
                    request_pressure_percent: Some(92),
                    live_pressure_percent: None,
                    compaction_attempted: true,
                    compaction_succeeded: true,
                    blocking: false,
                },
                cache_explainability: rocode_types::SessionCacheExplainabilityContract {
                    issue_present: true,
                    explained: true,
                    source: rocode_types::SessionCacheExplainabilitySource::CacheEvidence,
                    severity: Some(rocode_types::SessionCacheSeverity::MediumChange),
                    explanation: Some(
                        "boundary recorded · prefix changed"
                            .to_string(),
                    ),
                },
                child_history_isolation: rocode_types::SessionChildHistoryIsolationContract {
                    attached_subtree_session_count: 0,
                    owner_session_cumulative_tokens: runtime_usage.session_cumulative_tokens(),
                    workflow_cumulative_tokens: runtime_usage
                        .workflow_usage_summary()
                        .total_tokens(),
                    attached_subtree_cumulative_tokens: 0,
                    owner_live_context_tokens: None,
                    owner_local_live_prefix: true,
                    child_history_in_live_prefix_detected: false,
                    explanation:
                        "No attached subtree sessions were observed; the live prefix remains owner-local."
                            .to_string(),
                },
            }),
            prompt_surface_evidence: Some(PromptSurfaceEvidenceSummary {
                severity: rocode_types::SessionCacheSeverity::HighChange,
                reason: "surface changed: toolSurfaceHash".to_string(),
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
        assert_eq!(value["context_explain"]["api_view_messages"], 8);
        assert_eq!(
            value["ownership"]["context_kind"],
            "root_session_continuity"
        );
        assert_eq!(value["ownership"]["compact_owner"], true);
        assert_eq!(
            value["context_explain"]["workflow_cumulative_tokens"],
            runtime_usage.workflow_usage_summary().total_tokens()
        );
        assert_eq!(
            value["context_compaction_summary"]["reason"],
            "request_view_threshold"
        );
        assert_eq!(
            value["context_compaction_summary"]["compacted_message_count"],
            7
        );
        assert_eq!(value["cache_semantics"]["basis"], "api_view");
        assert_eq!(
            value["cache_semantics"]["label"],
            "boundary recorded · prefix changed"
        );
        assert_eq!(
            value["context_closure_contract"]["prefix_stability"]["prefix_change_detected"],
            true
        );
        assert_eq!(
            value["context_closure_contract"]["compaction_boundary"]["request_pressure_percent"],
            92
        );
        assert_eq!(
            value["context_closure_contract"]["cache_explainability"]["source"],
            "cache_evidence"
        );
        assert_eq!(
            value["context_closure_contract"]["child_history_isolation"]["owner_local_live_prefix"],
            true
        );
        assert_eq!(
            value["prompt_surface_evidence"]["changed_fields"][0],
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

    #[tokio::test]
    async fn telemetry_snapshot_usage_books_keep_owner_local_context_and_subtree_cumulative() {
        let state = Arc::new(ServerState::new());

        let (root_id, root_session, root_usage, child_usage) = {
            let mut sessions = state.sessions.lock().await;

            let mut root = sessions.create("project", "/tmp/project");
            let root_usage = MessageUsage {
                input_tokens: 120,
                output_tokens: 30,
                reasoning_tokens: 9,
                cache_write_tokens: 7,
                cache_read_tokens: 40,
                cache_miss_tokens: 5,
                context_tokens: 180,
                total_cost: 0.75,
            };
            let assistant = root.add_assistant_message();
            assistant.usage = Some(root_usage.clone());
            sessions.update(root.clone());

            let mut child = rocode_session::Session::attached_with_context_kind(
                &root,
                rocode_types::SessionContextKind::DelegatedSubsession,
            );
            let child_usage = MessageUsage {
                input_tokens: 60,
                output_tokens: 15,
                reasoning_tokens: 4,
                cache_write_tokens: 3,
                cache_read_tokens: 12,
                cache_miss_tokens: 2,
                context_tokens: 90,
                total_cost: 0.33,
            };
            let child_assistant = child.add_assistant_message();
            child_assistant.usage = Some(child_usage.clone());
            sessions.update(child);

            (root.id.clone(), root, root_usage, child_usage)
        };

        let snapshot = build_session_telemetry_snapshot(&state, &root_id, &root_session)
            .await
            .expect("snapshot should build");

        assert_eq!(
            snapshot.usage_books.request_context_tokens,
            root_usage.request_context_tokens()
        );
        assert_eq!(
            snapshot.usage_books.live_context_tokens,
            root_usage.live_context_tokens()
        );
        assert_eq!(
            snapshot.usage_books.workflow_cumulative.input_tokens,
            root_usage.input_tokens + child_usage.input_tokens
        );
        assert_eq!(
            snapshot.usage_books.workflow_cumulative.output_tokens,
            root_usage.output_tokens + child_usage.output_tokens
        );
        assert_eq!(
            snapshot.usage_books.workflow_cumulative.reasoning_tokens,
            root_usage.reasoning_tokens + child_usage.reasoning_tokens
        );
        assert_eq!(
            snapshot.usage_books.workflow_cumulative.cache_read_tokens,
            root_usage.cache_read_tokens + child_usage.cache_read_tokens
        );
        assert_eq!(
            snapshot.usage_books.workflow_cumulative.cache_miss_tokens,
            root_usage.cache_miss_tokens + child_usage.cache_miss_tokens
        );
        assert_eq!(
            snapshot.usage_books.workflow_cumulative.total_tokens(),
            root_usage.input_tokens
                + root_usage.output_tokens
                + root_usage.reasoning_tokens
                + child_usage.input_tokens
                + child_usage.output_tokens
                + child_usage.reasoning_tokens
        );
        assert!(
            (snapshot.usage_books.workflow_cumulative.total_cost
                - (root_usage.total_cost + child_usage.total_cost))
                .abs()
                < f64::EPSILON
        );
        let explain = snapshot
            .context_explain
            .as_ref()
            .expect("context explain should be present");
        assert_eq!(explain.raw_history_messages, 1);
        assert_eq!(explain.raw_model_visible_messages, 1);
        assert_eq!(explain.api_view_messages, 0);
        assert_eq!(explain.api_view_estimated_input_tokens, None);
        assert_eq!(
            explain.live_context_tokens,
            root_usage.live_context_tokens()
        );
        assert_eq!(
            explain.last_request_context_tokens,
            root_usage.request_context_tokens()
        );
        assert_eq!(
            explain.owner_session_cumulative_tokens,
            root_usage.input_tokens + root_usage.output_tokens + root_usage.reasoning_tokens
        );
        assert_eq!(
            explain.workflow_cumulative_tokens,
            snapshot.usage_books.workflow_cumulative.total_tokens()
        );
        let contract = snapshot
            .context_closure_contract
            .as_ref()
            .expect("context closure contract should be present");
        assert_eq!(
            contract
                .child_history_isolation
                .attached_subtree_session_count,
            1
        );
        assert_eq!(
            contract
                .child_history_isolation
                .attached_subtree_cumulative_tokens,
            child_usage.input_tokens + child_usage.output_tokens + child_usage.reasoning_tokens
        );
        assert!(contract.child_history_isolation.owner_local_live_prefix);
        assert!(
            !contract
                .child_history_isolation
                .child_history_in_live_prefix_detected
        );
        assert!(!contract.prefix_stability.prefix_change_detected);
    }
}
