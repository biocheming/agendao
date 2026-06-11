//! Server-side single-authority FrontendEvent projector.
//!
//! Constitution Article 4 (土): this is the sole orchestration authority that
//! maps execution-domain ServerEvents into frontend-authority FrontendEvents.
//! No transport (SSE / Unix / Direct) may bypass this projector.
//!
//! ## Commit 2 status (skeleton)
//!
//! The projector is fully functional but NOT yet wired into transport paths.
//! Dead-code warnings are expected until Commands 3-7 connect the transports.
//!
//! ## Design
//!
//! ```text
//! ServerEvent (event_bus)
//!     │
//!     └── project_server_event() ──→ FrontendEvent (frontend_bus)
//!                                         │
//!                      ┌──────────────────┼──────────────────┐
//!                     SSE              Unix Socket         Direct
//! ```
//!
//! The projector reads from authority registries (runtime state store,
//! question registry, stage summaries, topology) to enrich events into
//! full-payload FrontendEvents that frontends can apply without follow-up
//! queries.
//!
//! # Commit 3 scope (authority wiring)
//!
//! - Passthrough: OutputBlock, DiffUpdated (event payload is already complete)
//! - Tool call: ToolCallUpsert with correct phase mapping
//! - Question: QuestionRemoved (removal doesn't need full QuestionInfo lookup)
//! - Permission: PermissionRemoved + PermissionUpsert from info field
//! - Runtime/Projection: full authority projection including usage_books,
//!   compaction, cache semantics, and closure contract from session data
//! - Non-frontend events: Usage, Error, ControlInputTransition, ConfigUpdated → empty

use std::sync::Arc;

use agendao_server_core::frontend_events::FrontendEvent;
use agendao_server_core::runtime_events::{ServerEvent, ToolCallPhase};
use tokio::sync::{broadcast, Mutex};

use crate::session_runtime::projection_authority::build_session_projection_fields;
use crate::session_runtime::telemetry::RuntimeTelemetryAuthority;

// ── Public API ─────────────────────────────────────────────────────────────

/// Serialize and broadcast a FrontendEvent on the frontend event bus.
pub(crate) fn broadcast_frontend_event(bus: &broadcast::Sender<String>, event: &FrontendEvent) {
    match serde_json::to_string(event) {
        Ok(json) => {
            let _ = bus.send(json);
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to serialize FrontendEvent for broadcast");
        }
    }
}

/// Project a single ServerEvent into zero or more FrontendEvents.
///
/// This is the single authority entry point: every transport path must
/// consume FrontendEvents produced here. The projector reads from the
/// authoritative registries held by `RuntimeTelemetryAuthority` and the
/// session manager to fill in complete payloads.
pub(crate) async fn project_server_event(
    telemetry: &RuntimeTelemetryAuthority,
    sessions: &Arc<Mutex<agendao_session::SessionManager>>,
    event: &ServerEvent,
) -> Vec<FrontendEvent> {
    match event {
        // ── Passthrough: event payload is already complete ──────────────
        ServerEvent::OutputBlock {
            session_id,
            block,
            id,
            live_identity,
        } => {
            vec![FrontendEvent::OutputBlockAppended {
                session_id: session_id.clone(),
                block: block.clone(),
                id: id.clone(),
                live_identity: live_identity.clone(),
            }]
        }

        ServerEvent::DiffUpdated { session_id, diff } => {
            vec![FrontendEvent::DiffReplaced {
                session_id: session_id.clone(),
                diffs: diff.clone(),
            }]
        }

        // ── Tool lifecycle: upsert into active tool set ─────────────────
        ServerEvent::ToolCallLifecycle {
            session_id,
            tool_call_id,
            tool_name,
            phase,
        } => {
            let frontend_phase = match phase {
                ToolCallPhase::Start => ToolCallPhase::Start,
                ToolCallPhase::Complete => ToolCallPhase::Complete,
            };
            let mut events = vec![FrontendEvent::ToolCallUpsert {
                session_id: session_id.clone(),
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone().unwrap_or_default(),
                phase: frontend_phase,
            }];
            if let Some(rt) = project_runtime_replaced(telemetry, session_id).await {
                events.push(rt);
            }
            events
        }

        // ── Question: upsert / remove ───────────────────────────────────
        ServerEvent::QuestionCreated {
            session_id,
            request_id,
            ..
        } => {
            let mut events =
                project_question_upsert(telemetry, session_id, request_id).await;
            if let Some(rt) = project_runtime_replaced(telemetry, session_id).await {
                events.push(rt);
            }
            events
        }

        ServerEvent::QuestionResolved {
            session_id,
            request_id,
            ..
        } => {
            let mut events = vec![FrontendEvent::QuestionRemoved {
                session_id: session_id.clone(),
                question_id: request_id.clone(),
            }];
            if let Some(rt) = project_runtime_replaced(telemetry, session_id).await {
                events.push(rt);
            }
            events
        }

        // ── Permission: upsert / remove ─────────────────────────────────
        ServerEvent::PermissionRequested {
            session_id,
            permission_id,
            info,
        } => {
            let mut events = project_permission_upsert(session_id, permission_id, info);
            if let Some(rt) = project_runtime_replaced(telemetry, session_id).await {
                events.push(rt);
            }
            events
        }

        ServerEvent::PermissionResolved {
            session_id,
            permission_id,
            reply,
            ..
        } => {
            let mut events = vec![FrontendEvent::PermissionRemoved {
                session_id: session_id.clone(),
                permission_id: permission_id.clone(),
                reply: reply.clone(),
            }];
            if let Some(rt) = project_runtime_replaced(telemetry, session_id).await {
                events.push(rt);
            }
            events
        }

        // ── Session state changes: runtime + projection snapshot ────────
        ServerEvent::SessionStatus { session_id, .. }
        | ServerEvent::SessionUpdated { session_id, .. }
        | ServerEvent::TopologyChanged { session_id, .. }
        | ServerEvent::AttachedSessionAttached { parent_id: session_id, .. }
        | ServerEvent::AttachedSessionDetached { parent_id: session_id, .. } => {
            let mut events = Vec::new();
            if let Some(rt) = project_runtime_replaced(telemetry, session_id).await {
                events.push(rt);
            }
            if let Some(proj) = project_projection_replaced(telemetry, sessions, session_id).await {
                events.push(proj);
            }
            events
        }

        // ── Internal / telemetry-only events (no frontend projection) ────
        ServerEvent::Usage { .. }
        | ServerEvent::Error { .. }
        | ServerEvent::ControlInputTransition { .. }
        | ServerEvent::ConfigUpdated => {
            vec![]
        }
    }
}

/// Spawn a background task that subscribes to the ServerEvent bus,
/// projects every event, and broadcasts FrontendEvents on the frontend bus.
///
/// This is the canonical wiring point: one subscriber, one projector,
/// all transports downstream consume from `frontend_bus`.
pub(crate) fn spawn_frontend_projector(
    event_bus: broadcast::Sender<String>,
    frontend_bus: broadcast::Sender<String>,
    telemetry: Arc<RuntimeTelemetryAuthority>,
    sessions: Arc<Mutex<agendao_session::SessionManager>>,
) -> tokio::task::JoinHandle<()> {
    let mut rx = event_bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(payload) => {
                    let event = match serde_json::from_str::<ServerEvent>(&payload) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let frontend_events =
                        project_server_event(telemetry.as_ref(), &sessions, &event).await;
                    for fe in &frontend_events {
                        broadcast_frontend_event(&frontend_bus, fe);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        skipped = n,
                        "Frontend projector lagged on ServerEvent bus"
                    );
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        tracing::debug!("Frontend projector stopped: event_bus closed");
    })
}

// ── Per-domain projection helpers ──────────────────────────────────────────

/// Project `SessionRuntimeReplaced` from the authoritative runtime state.
async fn project_runtime_replaced(
    telemetry: &RuntimeTelemetryAuthority,
    session_id: &str,
) -> Option<FrontendEvent> {
    let server_runtime = telemetry.get_runtime_snapshot(session_id).await?;
    Some(FrontendEvent::SessionRuntimeReplaced {
        session_id: session_id.to_string(),
        runtime: convert_runtime_state(&server_runtime),
    })
}

/// Project `SessionProjectionReplaced` from telemetry authorities.
///
/// # Topology guard
///
/// Topology is `None` when no execution records exist — the projector never
/// fabricates a fake authority. This matches the FrontendEvent contract where
/// `topology: Option<SessionExecutionTopology>` is the canonical shape.
///
/// # Fields not yet wired (Commit 3+)
///
/// `usage_books`, `context_compaction_summary`, `cache_semantics`, and
/// `context_closure_contract` are built by `build_persisted_snapshot()` in
/// `routes/session/telemetry.rs` from session-scoped data. The
/// `RuntimeTelemetryAuthority` does not currently cache a full telemetry
/// snapshot. These fields will be populated when the telemetry authority
/// gains a snapshot cache (Commit 3–5).
async fn project_projection_replaced(
    telemetry: &RuntimeTelemetryAuthority,
    sessions: &Arc<Mutex<agendao_session::SessionManager>>,
    session_id: &str,
) -> Option<FrontendEvent> {
    let runtime = telemetry.get_runtime_snapshot(session_id).await?;
    let stages = telemetry.list_stage_summaries(session_id).await;
    let topology = telemetry
        .build_session_execution_topology(session_id.to_string(), vec![])
        .await;

    // Guard: only emit Some(topology) when there are actual execution records.
    let has_execution_records =
        topology.active_count > 0 || topology.done_count > 0 || !topology.roots.is_empty();
    let topology = if has_execution_records {
        Some(convert_execution_topology(&topology))
    } else {
        None
    };

    // ── Read projection fields from session authority ──────────────────
    let projection_fields = {
        let sessions_guard = sessions.lock().await;
        let session = sessions_guard.get(session_id)?;
        build_session_projection_fields(
            &session,
            session_id,
            runtime.usage.as_ref(),
            &sessions_guard,
        )
    };

    Some(FrontendEvent::SessionProjectionReplaced {
        session_id: session_id.to_string(),
        topology,
        stages,
        attached_sessions: runtime
            .attached_sessions
            .iter()
            .map(|a| agendao_api::AttachedSessionSummary {
                attached_id: a.attached_id.clone(),
                parent_id: a.parent_id.clone(),
                context_kind: Some(a.context_kind),
            })
            .collect(),
        usage: runtime.usage.clone(),
        usage_books: Some(projection_fields.usage_books),
        context_compaction_summary: projection_fields.context_compaction_summary,
        context_compaction_lifecycle_summary: projection_fields.context_compaction_lifecycle_summary,
        cache_semantics: projection_fields.cache_semantics,
        context_closure_contract: projection_fields.context_closure_contract,
    })
}

/// Project `QuestionUpsert` by looking up the question in the registry.
///
/// Authority contract: only the question whose `id` matches `request_id`
/// may be projected as a `QuestionUpsert`. If no match is found, the
/// projector emits an empty vec and logs a debug event — it never falls
/// back to "send something close." The frontend must not receive a
/// question it did not ask about.
async fn project_question_upsert(
    telemetry: &RuntimeTelemetryAuthority,
    session_id: &str,
    request_id: &str,
) -> Vec<FrontendEvent> {
    let questions = telemetry.list_questions_for_session(session_id).await;
    let matched = questions.iter().find(|q| q.id == request_id);
    match matched {
        Some(q) => {
            vec![FrontendEvent::QuestionUpsert {
                session_id: session_id.to_string(),
                question: convert_question_info(q),
            }]
        }
        None => {
            tracing::debug!(
                session_id = %session_id,
                request_id = %request_id,
                available = questions.len(),
                "QuestionUpsert: request_id not found in question registry — no FrontendEvent emitted"
            );
            vec![]
        }
    }
}

/// Project `PermissionUpsert` from the event's info payload.
fn project_permission_upsert(
    session_id: &str,
    permission_id: &str,
    info: &serde_json::Value,
) -> Vec<FrontendEvent> {
    match serde_json::from_value::<agendao_api::PermissionRequestInfo>(info.clone()) {
        Ok(permission) => {
            vec![FrontendEvent::PermissionUpsert {
                session_id: session_id.to_string(),
                permission,
            }]
        }
        Err(_) => {
            tracing::debug!(
                session_id = %session_id,
                permission_id = %permission_id,
                "Failed to deserialize PermissionRequestInfo from ServerEvent info field"
            );
            vec![]
        }
    }
}

// ── Type conversion helpers ────────────────────────────────────────────────

/// Convert server-internal `SessionRuntimeState` to the API-facing type.
fn convert_runtime_state(
    server: &agendao_server_core::runtime_state::SessionRuntimeState,
) -> agendao_api::SessionRuntimeState {
    use agendao_server_core::runtime_state::RunStatus;

    agendao_api::SessionRuntimeState {
        session_id: server.session_id.clone(),
        run_status: match server.run_status {
            RunStatus::Idle => agendao_api::SessionRunStatusKind::Idle,
            RunStatus::Running => agendao_api::SessionRunStatusKind::Running,
            RunStatus::Compacting => agendao_api::SessionRunStatusKind::Compacting,
            RunStatus::WaitingOnTool => agendao_api::SessionRunStatusKind::WaitingOnTool,
            RunStatus::WaitingOnUser => agendao_api::SessionRunStatusKind::WaitingOnUser,
            RunStatus::Cancelling => agendao_api::SessionRunStatusKind::Cancelling,
            RunStatus::Blocked => agendao_api::SessionRunStatusKind::Blocked,
            RunStatus::Sleeping => agendao_api::SessionRunStatusKind::Sleeping,
        },
        current_message_id: server.current_message_id.clone(),
        usage: server.usage.clone(),
        active_stage_id: server.active_stage_id.clone(),
        active_stage_count: server.active_stage_count,
        active_tools: server
            .active_tools
            .iter()
            .map(|t| agendao_api::ActiveToolSummary {
                tool_call_id: t.tool_call_id.clone(),
                tool_name: t.tool_name.clone(),
                started_at: t.started_at,
            })
            .collect(),
        pending_question: server.pending_question.as_ref().map(|q| {
            agendao_api::PendingQuestionSummary {
                request_id: q.request_id.clone(),
                questions: q.questions.clone(),
            }
        }),
        pending_permission: server.pending_permission.as_ref().map(|p| {
            agendao_api::PendingPermissionSummary {
                permission_id: p.permission_id.clone(),
                requested_at: p.requested_at,
                tool: p.tool.clone(),
            }
        }),
        pending_followup_count: server.pending_followup_count,
        attached_sessions: server
            .attached_sessions
            .iter()
            .map(|a| agendao_api::AttachedSessionSummary {
                attached_id: a.attached_id.clone(),
                parent_id: a.parent_id.clone(),
                context_kind: Some(a.context_kind),
            })
            .collect(),
    }
}

/// Convert server-internal execution topology to the API-facing type.
///
/// Uses serde roundtrip as a pragmatic bridge (Commit 2 skeleton).
/// The two types have identical serde shapes; a manual conversion will
/// be substituted in Commit 3 when topology is first-classed.
fn convert_execution_topology(
    server: &agendao_server_core::runtime_control::SessionExecutionTopology,
) -> agendao_api::SessionExecutionTopology {
    serde_json::from_value(serde_json::to_value(server).unwrap_or_default())
        .unwrap_or_else(|_| agendao_api::SessionExecutionTopology {
            session_id: server.session_id.clone(),
            active_count: server.active_count,
            done_count: server.done_count,
            running_count: server.running_count,
            waiting_count: server.waiting_count,
            cancelling_count: server.cancelling_count,
            retry_count: server.retry_count,
            updated_at: server.updated_at,
            roots: vec![],
        })
}

/// Convert server-internal QuestionInfo to the API-facing type.
///
/// Uses serde roundtrip as a pragmatic bridge (Commit 2 skeleton).
/// The two types have identical serde shapes.
fn convert_question_info(
    server: &agendao_server_core::runtime_control::QuestionInfo,
) -> agendao_api::QuestionInfo {
    serde_json::from_value(serde_json::to_value(server).unwrap_or_default())
        .unwrap_or_else(|_| agendao_api::QuestionInfo {
            id: server.id.clone(),
            session_id: server.session_id.clone(),
            questions: server.questions.clone(),
            options: server.options.clone(),
            items: vec![],
        })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_server_core::runtime_events::{
        QuestionResolutionKind, ServerEvent, ToolCallPhase as ServerToolPhase,
    };

    fn test_telemetry() -> Arc<RuntimeTelemetryAuthority> {
        let (tx, _rx) = broadcast::channel(16);
        RuntimeTelemetryAuthority::new(tx, None).into()
    }

    fn test_sessions() -> Arc<Mutex<agendao_session::SessionManager>> {
        Arc::new(Mutex::new(agendao_session::SessionManager::new()))
    }

    // ── OutputBlock passthrough ────────────────────────────────────────

    #[tokio::test]
    async fn output_block_passthrough() {
        let telemetry = test_telemetry();
        let event = ServerEvent::OutputBlock {
            session_id: "ses_1".into(),
            block: serde_json::json!({"kind": "message", "text": "hello"}),
            id: Some("msg_1".into()),
            live_identity: None,
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        assert_eq!(result.len(), 1);
        match &result[0] {
            FrontendEvent::OutputBlockAppended {
                session_id,
                block,
                id,
                ..
            } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(block["text"], "hello");
                assert_eq!(id.as_deref(), Some("msg_1"));
            }
            other => panic!("expected OutputBlockAppended, got {:?}", other),
        }
    }

    // ── Diff passthrough ───────────────────────────────────────────────

    #[tokio::test]
    async fn diff_updated_passthrough() {
        let telemetry = test_telemetry();
        let event = ServerEvent::DiffUpdated {
            session_id: "ses_1".into(),
            diff: vec![agendao_server_core::runtime_events::DiffEntry {
                path: "src/main.rs".into(),
                additions: 3,
                deletions: 1,
            }],
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        assert_eq!(result.len(), 1);
        match &result[0] {
            FrontendEvent::DiffReplaced { session_id, diffs } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(diffs.len(), 1);
                assert_eq!(diffs[0].path, "src/main.rs");
            }
            other => panic!("expected DiffReplaced, got {:?}", other),
        }
    }

    // ── Tool lifecycle ─────────────────────────────────────────────────

    #[tokio::test]
    async fn tool_call_start_produces_upsert() {
        let telemetry = test_telemetry();
        let event = ServerEvent::ToolCallLifecycle {
            session_id: "ses_1".into(),
            tool_call_id: "tc_1".into(),
            tool_name: Some("bash".into()),
            phase: ServerToolPhase::Start,
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        let upsert = result
            .iter()
            .find(|e| matches!(e, FrontendEvent::ToolCallUpsert { .. }))
            .expect("ToolCallUpsert should be present");
        match upsert {
            FrontendEvent::ToolCallUpsert {
                tool_call_id,
                tool_name,
                phase,
                ..
            } => {
                assert_eq!(tool_call_id, "tc_1");
                assert_eq!(tool_name, "bash");
                assert_eq!(*phase, agendao_server_core::ToolCallPhase::Start);
            }
            _ => unreachable!(),
        }
    }

    // ── Question resolved → QuestionRemoved ────────────────────────────

    #[tokio::test]
    async fn question_resolved_produces_removed() {
        let telemetry = test_telemetry();
        let event = ServerEvent::QuestionResolved {
            session_id: "ses_1".into(),
            request_id: "q_1".into(),
            resolution: Some(QuestionResolutionKind::Answered),
            answers: None,
            reason: None,
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        let removed = result
            .iter()
            .find(|e| matches!(e, FrontendEvent::QuestionRemoved { .. }))
            .expect("QuestionRemoved should be present");
        match removed {
            FrontendEvent::QuestionRemoved {
                session_id,
                question_id,
            } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(question_id, "q_1");
            }
            _ => unreachable!(),
        }
    }

    // ── Permission resolved → PermissionRemoved ────────────────────────

    #[tokio::test]
    async fn permission_resolved_produces_removed() {
        let telemetry = test_telemetry();
        let event = ServerEvent::PermissionResolved {
            session_id: "ses_1".into(),
            permission_id: "p_1".into(),
            reply: "once".into(),
            message: None,
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        let removed = result
            .iter()
            .find(|e| matches!(e, FrontendEvent::PermissionRemoved { .. }))
            .expect("PermissionRemoved should be present");
        match removed {
            FrontendEvent::PermissionRemoved {
                session_id,
                permission_id,
                reply,
            } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(permission_id, "p_1");
                assert_eq!(reply, "once");
            }
            _ => unreachable!(),
        }
    }

    // ── Non-frontend events produce empty vec ──────────────────────────

    #[tokio::test]
    async fn config_updated_produces_no_frontend_events() {
        let telemetry = test_telemetry();
        let result =
            project_server_event(&telemetry, &test_sessions(), &ServerEvent::ConfigUpdated).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn usage_event_produces_no_frontend_events() {
        let telemetry = test_telemetry();
        let event = ServerEvent::Usage {
            session_id: Some("ses_1".into()),
            prompt_tokens: 10,
            completion_tokens: 20,
            message_id: None,
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        assert!(result.is_empty());
    }

    // ── Permission upsert from info field ──────────────────────────────

    #[tokio::test]
    async fn permission_upsert_deserializes_from_info() {
        let telemetry = test_telemetry();
        let info = serde_json::json!({
            "id": "p_1",
            "session_id": "ses_1",
            "tool": "bash",
            "input": {"command": "cargo test"},
            "message": "Allow cargo test?",
            "permission_class": null,
            "scope_key": null,
            "scope_label": null,
            "origin_tool": null,
            "supported_lifetimes": [],
            "matcher_kind": null,
            "matcher_key": null,
            "matcher_label": null,
            "grant_target_summary": null,
            "risk_tags": []
        });
        let event = ServerEvent::PermissionRequested {
            session_id: "ses_1".into(),
            permission_id: "p_1".into(),
            info,
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;
        let upsert = result
            .iter()
            .find(|e| matches!(e, FrontendEvent::PermissionUpsert { .. }))
            .expect("PermissionUpsert should be present");
        match upsert {
            FrontendEvent::PermissionUpsert {
                session_id,
                permission,
            } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(permission.id, "p_1");
                assert_eq!(permission.tool, "bash");
            }
            _ => unreachable!(),
        }
    }

    // ── Roundtrip: every frontend event must serialize with a type field ─

    #[test]
    fn all_projected_events_have_type_field() {
        let events: Vec<FrontendEvent> = vec![
            FrontendEvent::SessionRuntimeReplaced {
                session_id: "s".into(),
                runtime: agendao_api::SessionRuntimeState {
                    session_id: "s".into(),
                    run_status: agendao_api::SessionRunStatusKind::Idle,
                    current_message_id: None,
                    usage: None,
                    active_stage_id: None,
                    active_stage_count: 0,
                    active_tools: vec![],
                    pending_question: None,
                    pending_permission: None,
                    pending_followup_count: 0,
                    attached_sessions: vec![],
                },
            },
            FrontendEvent::QuestionRemoved {
                session_id: "s".into(),
                question_id: "q".into(),
            },
            FrontendEvent::ToolCallUpsert {
                session_id: "s".into(),
                tool_call_id: "tc".into(),
                tool_name: "bash".into(),
                phase: agendao_server_core::ToolCallPhase::Start,
            },
            FrontendEvent::OutputBlockAppended {
                session_id: "s".into(),
                block: serde_json::json!({"text": "ok"}),
                id: None,
                live_identity: None,
            },
        ];
        for event in &events {
            let json =
                serde_json::to_value(event).expect("FrontendEvent must serialize");
            assert!(
                json.get("type").and_then(|v| v.as_str()).is_some(),
                "missing 'type' field in: {}",
                json
            );
        }
    }

    // ── Authority guards ──────────────────────────────────────────────

    /// When no execution records exist, `SessionProjectionReplaced.topology`
    /// MUST be `None` — the projector never fabricates a fake authority.
    #[tokio::test]
    async fn topology_none_when_no_execution_records() {
        let telemetry = test_telemetry();
        let sessions = test_sessions();
        // Create a session and get its auto-generated ID.
        let sid = {
            let mut guard = sessions.lock().await;
            let session = agendao_session::Session::new("test_project", "/tmp/test");
            let id = session.record().id.clone();
            guard.update(session);
            id
        };
        // Initialize runtime state.
        use agendao_server_core::runtime_control::SessionRunStatus;
        telemetry
            .set_session_run_status(&sid, SessionRunStatus::Busy)
            .await;
        telemetry
            .set_session_run_status(&sid, SessionRunStatus::Idle)
            .await;

        let event = ServerEvent::SessionStatus {
            session_id: sid.clone(),
            status: serde_json::json!({"runStatus": "idle"}),
        };
        let result = project_server_event(&telemetry, &sessions, &event).await;

        let proj = result
            .iter()
            .find(|e| matches!(e, FrontendEvent::SessionProjectionReplaced { .. }))
            .expect("SessionProjectionReplaced should be present");
        match proj {
            FrontendEvent::SessionProjectionReplaced { topology, .. } => {
                assert!(
                    topology.is_none(),
                    "topology must be None when no execution records exist, got {:?}",
                    topology
                );
            }
            _ => unreachable!(),
        }
    }

    /// When the question request_id does not match any question in the
    /// registry, `project_question_upsert` MUST return an empty vec —
    /// it never falls back to "send something close."
    #[tokio::test]
    async fn question_upsert_empty_when_request_id_not_found() {
        let telemetry = test_telemetry();
        // Register a question with a DIFFERENT id than the one in the event.
        use agendao_tool::QuestionDef;
        let _ = telemetry
            .register_question(
                "ses_1".to_string(),
                vec![QuestionDef {
                    question: "Proceed?".to_string(),
                    header: Some("Confirm".to_string()),
                    options: vec![],
                    multiple: false,
                }],
            )
            .await;

        // Fire QuestionCreated with a request_id that does NOT match the
        // registered question's id.
        let event = ServerEvent::QuestionCreated {
            session_id: "ses_1".into(),
            request_id: "no_such_id".into(),
            questions: serde_json::json!([]),
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;

        let upsert_count = result
            .iter()
            .filter(|e| matches!(e, FrontendEvent::QuestionUpsert { .. }))
            .count();
        assert_eq!(
            upsert_count, 0,
            "QuestionUpsert must be empty when request_id is not in registry"
        );
    }

    /// When a request_id DOES match a registered question, the projector
    /// emits exactly one QuestionUpsert with the matching question.
    #[tokio::test]
    async fn question_upsert_succeeds_when_request_id_matches() {
        let telemetry = test_telemetry();
        use agendao_tool::QuestionDef;
        let (info, _rx) = telemetry
            .register_question(
                "ses_1".to_string(),
                vec![QuestionDef {
                    question: "Proceed?".to_string(),
                    header: Some("Confirm".to_string()),
                    options: vec![],
                    multiple: false,
                }],
            )
            .await;

        let event = ServerEvent::QuestionCreated {
            session_id: "ses_1".into(),
            request_id: info.id.clone(),
            questions: serde_json::json!([]),
        };
        let result = project_server_event(&telemetry, &test_sessions(), &event).await;

        let upsert = result
            .iter()
            .find(|e| matches!(e, FrontendEvent::QuestionUpsert { .. }))
            .expect("QuestionUpsert should be present when request_id matches");
        match upsert {
            FrontendEvent::QuestionUpsert {
                session_id,
                question,
            } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(question.id, info.id);
            }
            _ => unreachable!(),
        }
    }
}
