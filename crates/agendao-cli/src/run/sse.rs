// P1-2: Converted from include!() to proper module.
// This module lives under run::sse — interaction layer.

use std::io::{self, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use agendao_command::cli_spinner::SpinnerGuard;
use agendao_command::cli_style::CliStyle;
use agendao_command::output_blocks::{
    render_cli_block_rich, MessageBlock, OutputBlock, StatusBlock,
};

use crate::api_client::CliApiClient;

use super::*;

pub(super) fn pending_command_from_session(
    session: &crate::api_client::SessionInfo,
    question_id: &str,
) -> Option<crate::api_client::PendingCommandInvocation> {
    let metadata = session.metadata.as_ref()?;
    let pending = metadata.get("pending_command_invocation")?.clone();
    let pending =
        serde_json::from_value::<crate::api_client::PendingCommandInvocation>(pending).ok()?;
    if pending
        .question_id
        .as_deref()
        .is_some_and(|candidate| candidate != question_id)
    {
        return None;
    }
    Some(pending)
}

pub(super) fn shell_quote_command_value(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '*' | ':'))
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(super) fn split_repeatable_answer(answer: &str) -> Vec<String> {
    answer
        .split(|ch: char| matches!(ch, '\n' | ',' | '\t'))
        .flat_map(|segment| segment.split_whitespace())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn permission_timestamp_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(super) fn cli_summary_thinking_label() -> String {
    "Thinking".to_string()
}

pub(super) fn cli_summary_tool_label(tool_name: &str) -> String {
    let label =
        agendao_command::output_blocks::tool_cli_activity_label(&agendao_command::output_blocks::ToolBlock::start(tool_name));
    format!("Using {label}")
}

pub(super) fn cli_summary_permission_label(tool_name: Option<&str>) -> String {
    match tool_name.map(str::trim).filter(|value| !value.is_empty()) {
        Some(tool_name) => format!("Awaiting permission · {}", cli_summary_tool_label(tool_name)),
        None => "Awaiting permission".to_string(),
    }
}

pub(super) fn cli_summary_waiting_label() -> String {
    "Awaiting user input".to_string()
}

pub(super) fn cli_session_update_finishes_turn(source: Option<&str>) -> bool {
    matches!(
        source,
        Some(
            "turn.final"
                | "prompt.final"
                | "stream.final"
                | "prompt.completed"
                | "prompt.done"
        )
    )
}

pub(super) fn cli_restore_compact_summary(projection: &mut CliFrontendProjection) {
    projection.prompt_lanes.clear_non_error();
    projection.active_label =
        if projection.pending_permission_count > 0 || projection.submitting_permission_count > 0 {
            Some("Awaiting permission".to_string())
        } else {
            match projection.phase {
                CliFrontendPhase::Busy => Some(cli_summary_thinking_label()),
                CliFrontendPhase::Waiting => Some(cli_summary_waiting_label()),
                _ => None,
            }
        };
}

pub(super) fn cli_push_runtime_aux_block(
    runtime: &CliExecutionRuntime,
    block: OutputBlock,
) {
    if let Some(line) = cli_prompt_aux_line(&block) {
        if let Ok(mut projection) = runtime.frontend_projection.lock() {
            projection.prompt_lanes.push_aux_line(line.lane, &line.text);
        }
    }
}

pub(super) fn cli_output_block_updates_transcript_authority(
    block: &OutputBlock,
    live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
) -> bool {
    if let Some(identity) = live_identity {
        return LiveSemanticConsumer::is_transcript_bearing_kind(&identity.part_kind);
    }

    match block {
        OutputBlock::Message(_) | OutputBlock::Reasoning(_) => true,
        OutputBlock::Tool(tool) => matches!(tool.phase, ToolPhase::Done | ToolPhase::Error),
        _ => false,
    }
}

pub(super) fn cli_store_active_tool_label(runtime: &CliExecutionRuntime, tool_call_id: &str, tool_name: &str) {
    if let Ok(mut labels) = runtime.active_tool_labels.lock() {
        labels.insert(tool_call_id.to_string(), tool_name.to_string());
    }
}

pub(super) fn cli_take_active_tool_label(runtime: &CliExecutionRuntime, tool_call_id: &str) -> Option<String> {
    runtime
        .active_tool_labels
        .lock()
        .ok()
        .and_then(|mut labels| labels.remove(tool_call_id))
}

pub(super) fn ingress_stabilization_label(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?.as_object()?;
    let source = value
        .get("source")
        .and_then(|source| {
            source.as_str().map(str::to_string).or_else(|| {
                source
                    .get("source")
                    .and_then(|nested| nested.as_str())
                    .map(str::to_string)
            })
        })
        .unwrap_or_else(|| "unknown".to_string());
    let policy = value
        .get("policy")
        .and_then(|policy| policy.as_str())
        .unwrap_or("metadata_only");
    let batch_count = value
        .get("batch_count")
        .and_then(|count| count.as_u64())
        .unwrap_or(1);
    if batch_count > 1 {
        Some(format!("{source} · {policy} · batch {batch_count}"))
    } else {
        Some(format!("{source} · {policy}"))
    }
}

pub(super) fn provider_diagnostic_label(
    summary: Option<&crate::api_client::ProviderDiagnosticSummary>,
) -> Option<String> {
    let summary = summary?;
    match summary.code.as_str() {
        "thinking_replay_missing" => Some("thinking replay missing".to_string()),
        "thinking_replay_rejected" => Some("thinking replay rejected".to_string()),
        _ => Some(summary.code.replace('_', " ")),
    }
}

pub(super) fn permission_class_label(value: &str) -> Option<String> {
    match value {
        "inspect_read" => Some("Inspect read".to_string()),
        "workspace_write" => Some("Workspace write".to_string()),
        "external_access" => Some("External access".to_string()),
        "dangerous_exec" => Some("Dangerous execution".to_string()),
        other => Some(other.replace('_', " ")),
    }
}

pub(super) fn default_permission_lifetimes(
    permission_class: Option<&str>,
) -> Vec<agendao_permission::PermissionLifetime> {
    match permission_class {
        Some("workspace_write" | "external_access") => vec![
            agendao_permission::PermissionLifetime::Once,
            agendao_permission::PermissionLifetime::Turn,
            agendao_permission::PermissionLifetime::Session,
        ],
        Some("inspect_read" | "dangerous_exec") => {
            vec![agendao_permission::PermissionLifetime::Once]
        }
        Some(_) | None => vec![agendao_permission::PermissionLifetime::Once],
    }
}

pub(super) fn merge_pending_command_arguments(
    pending: &crate::api_client::PendingCommandInvocation,
    answers: &[Vec<String>],
) -> String {
    let mut parts = Vec::new();
    let raw = pending.raw_arguments.trim();
    if !raw.is_empty() {
        parts.push(raw.to_string());
    }

    for (index, field) in pending.missing_fields.iter().enumerate() {
        let answer_values = answers.get(index).cloned().unwrap_or_default();
        let expanded_values = answer_values
            .into_iter()
            .flat_map(|value| {
                if value.contains('\n') || value.contains(',') {
                    split_repeatable_answer(&value)
                } else {
                    vec![value]
                }
            })
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if expanded_values.is_empty() {
            continue;
        }
        parts.push(format!("--{}", field));
        parts.extend(
            expanded_values
                .iter()
                .map(|value| shell_quote_command_value(value)),
        );
    }

    parts.join(" ").trim().to_string()
}

pub(super) fn question_defs_from_info(
    info: &crate::api_client::QuestionInfo,
) -> Vec<agendao_tool::QuestionDef> {
    if !info.items.is_empty() {
        return info
            .items
            .iter()
            .map(|item| agendao_tool::QuestionDef {
                question: item.question.clone(),
                header: item.header.clone(),
                options: item
                    .options
                    .iter()
                    .map(|option| agendao_tool::QuestionOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: item.multiple,
            })
            .collect();
    }

    info.questions
        .iter()
        .enumerate()
        .map(|(index, question)| agendao_tool::QuestionDef {
            question: question.clone(),
            header: None,
            options: info
                .options
                .as_ref()
                .and_then(|all| all.get(index))
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|label| agendao_tool::QuestionOption {
                    label,
                    description: None,
                })
                .collect(),
            multiple: false,
        })
        .collect()
}

pub(super) async fn resolve_prompt_submission(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    session_id: &str,
    style: &CliStyle,
    prompt_response: crate::api_client::PromptResponse,
) -> anyhow::Result<(
    crate::api_client::PromptResponse,
    std::collections::HashSet<String>,
)> {
    let mut response = prompt_response;
    let mut ignored_question_ids = std::collections::HashSet::new();

    loop {
        if response.status != "awaiting_user" {
            return Ok((response, ignored_question_ids));
        }

        let Some(question_id) = response.pending_question_id.clone() else {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(
                    "Command is awaiting user input, but no question id was returned.",
                )),
                style,
            );
            anyhow::bail!("prompt returned awaiting_user without pending_question_id");
        };
        ignored_question_ids.insert(question_id.clone());

        let questions = crate::local_dispatch::list_questions(local_state, api_client)
            .await?
            .into_iter()
            .find(|question| question.id == question_id)
            .map(|question| question_defs_from_info(&question))
            .unwrap_or_default();
        if questions.is_empty() {
            anyhow::bail!(
                "pending question `{}` was not available to answer",
                question_id
            );
        }

        let guard = runtime
            .spinner_guard
            .lock()
            .map(|spinner| spinner.clone())
            .unwrap_or_else(|_| SpinnerGuard::noop());
        let answers = cli_ask_question(
            questions,
            runtime.observed_topology.clone(),
            runtime.frontend_projection.clone(),
            runtime.prompt_session_slot.clone(),
            runtime.terminal_surface.clone(),
            guard,
        )
        .await
        .map_err(|error| anyhow::anyhow!("command question failed: {}", error))?;
        crate::local_dispatch::reply_question(local_state, api_client, &question_id, answers.clone())
            .await?;

        let session = crate::local_dispatch::get_session(local_state, api_client, session_id).await?;
        let Some(pending) = pending_command_from_session(&session, &question_id) else {
            return Ok((response, ignored_question_ids));
        };
        let arguments = merge_pending_command_arguments(&pending, &answers);
        response = crate::local_dispatch::send_command_prompt(
            local_state,
            api_client,
                session_id,
                pending.command.clone(),
                (!arguments.trim().is_empty()).then_some(arguments),
                (runtime.resolved_model_label != "auto")
                    .then(|| runtime.resolved_model_label.clone()),
                None,
                Some("cli".to_string()),
                Some(format!(
                    "cli_command_{}",
                    agendao_core::id::create(agendao_core::id::Prefix::User, true, None)
                )),
                Some(agendao_types::MessageSourceOrigin::Operator),
                Some(agendao_types::MessageSourceSurface::Cli),
            )
            .await?;
    }
}

pub(super) async fn run_server_prompt(
    runtime: &mut CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<agendao_client::FrontendTransport>>,
    input: &str,
    style: &CliStyle,
    update_recovery_base: bool,
) -> anyhow::Result<()> {
    run_server_prompt_with_parts(
        runtime,
        api_client,
        sse_rx,
        local_state,
        transport,
        input,
        input,
        None,
        style,
        update_recovery_base,
    )
    .await
}

pub(super) async fn run_server_prompt_with_parts(
    runtime: &mut CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<agendao_client::FrontendTransport>>,
    input: &str,
    display_input: &str,
    parts: Option<Vec<crate::api_client::PromptPart>>,
    style: &CliStyle,
    update_recovery_base: bool,
) -> anyhow::Result<()> {
    if update_recovery_base {
        runtime.recovery_base_prompt = Some(display_input.to_string());
    }
    if let Ok(mut topology) = runtime.observed_topology.lock() {
        topology.reset_for_run(
            &runtime.resolved_agent_name,
            runtime.scheduler_profile_name.as_deref(),
        );
    }
    if let Ok(mut snapshots) = runtime.scheduler_stage_snapshots.lock() {
        snapshots.clear();
    }
    cli_frontend_set_phase(
        &runtime.frontend_projection,
        CliFrontendPhase::Busy,
        Some(cli_summary_thinking_label()),
    );
    if let Ok(mut active_tool_labels) = runtime.active_tool_labels.lock() {
        active_tool_labels.clear();
    }
    print_block(
        Some(runtime),
        OutputBlock::Message(MessageBlock::full(
            OutputMessageRole::User,
            display_input.to_string(),
        )),
        style,
    )?;
    cli_capture_visible_root_history_transcript(runtime);
    cli_capture_visible_root_transcript(runtime);

    let Some(session_id) = runtime.server_session_id.clone() else {
        anyhow::bail!("CLI server session is not initialized");
    };

    {
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = Some(CliActiveAbortHandle::Server {
            api_client: api_client.clone(),
            session_id: session_id.clone(),
        });
    }

    let prompt_agent = cli_prompt_agent_override(
        &runtime.resolved_agent_name,
        runtime.scheduler_profile_name.as_deref(),
    );

    let prompt_response = match crate::local_dispatch::send_prompt(
        local_state,
        transport,
        api_client,
            &session_id,
            input.to_string(),
            parts,
            prompt_agent,
            runtime.scheduler_profile_name.clone(),
            (runtime.resolved_model_label != "auto").then(|| runtime.resolved_model_label.clone()),
            None,
            Some("cli".to_string()),
            Some(format!(
                "cli_{}",
                agendao_core::id::create(agendao_core::id::Prefix::User, true, None)
            )),
            Some(agendao_types::MessageSourceOrigin::Operator),
            Some(agendao_types::MessageSourceSurface::Cli),
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            cli_frontend_set_phase(
                &runtime.frontend_projection,
                CliFrontendPhase::Failed,
                Some("send prompt failed".to_string()),
            );
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to send prompt: {}",
                    error
                ))),
                style,
            );
            let mut active_abort = runtime.active_abort.lock().await;
            *active_abort = None;
            cli_frontend_clear(runtime);
            return Ok(());
        }
    };

    let (_accepted_response, ignored_question_ids) =
        resolve_prompt_submission(
            runtime,
            api_client,
            local_state,
            &session_id,
            style,
            prompt_response,
        )
        .await?;

    loop {
        match sse_rx.recv().await {
            Some(CliServerEvent::QuestionCreated {
                request_id,
                session_id,
                questions_json,
            }) => {
                if ignored_question_ids.contains(&request_id) {
                    continue;
                }
                if cli_tracks_related_session(runtime, &session_id) {
                    handle_question_from_sse(
                        runtime,
                        api_client,
                        local_state,
                        &request_id,
                        &questions_json,
                    )
                    .await;
                }
            }
            Some(CliServerEvent::QuestionResolved { request_id })
                if ignored_question_ids.contains(&request_id) =>
            {
                continue;
            }
            Some(CliServerEvent::PermissionRequested {
                session_id,
                permission_id,
                info_json,
            }) => {
                if cli_tracks_related_session(runtime, &session_id) {
                    handle_permission_from_sse(
                        runtime,
                        api_client,
                        local_state,
                        &permission_id,
                        &info_json,
                    )
                    .await;
                }
            }
            Some(CliServerEvent::ConfigUpdated) => {
                cli_handle_config_updated_from_sse(runtime, api_client).await;
            }
            // P1-3: session.updated is the RECONCILE FALLBACK, not the primary
            // refresh path. Incremental updates (output blocks, permission events,
            // tool lifecycle) arrive via dedicated SSE event types and update the
            // UI locally. This handler only fires for non-droppable reconcile reasons
            // (turn.final, metadata.change, permission, steering, status.change).
            Some(CliServerEvent::SessionUpdated { session_id, source }) => {
                handle_session_updated_from_sse(
                    runtime,
                    api_client,
                    local_state,
                    &session_id,
                    source.as_deref(),
                    style,
                )
                .await;
            }
            Some(CliServerEvent::SessionIdle {
                session_id: idle_session_id,
            }) => {
                let is_current_session = runtime
                    .server_session_id
                    .as_deref()
                    .is_some_and(|current| current == idle_session_id);
                handle_sse_event(
                    runtime,
                    CliServerEvent::SessionIdle {
                        session_id: idle_session_id,
                    },
                    style,
                );
                if !is_current_session {
                    continue;
                }
                handle_session_updated_from_sse(
                    runtime,
                    api_client,
                    local_state,
                    &session_id,
                    Some("prompt.done"),
                    style,
                )
                .await;
                if let Ok(mut topology) = runtime.observed_topology.lock() {
                    topology.finish_run(Some("Completed".to_string()));
                }
                cli_frontend_clear(runtime);
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::success("Done.")),
                    style,
                );
                break;
            }
            Some(other) => {
                handle_sse_event(runtime, other, style);
            }
            None => break,
        }
    }

    {
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = None;
    }
    Ok(())
}

pub(super) fn cli_prompt_agent_override(
    resolved_agent_name: &str,
    scheduler_profile_name: Option<&str>,
) -> Option<String> {
    if scheduler_profile_name.is_some() {
        None
    } else {
        Some(resolved_agent_name.to_string())
    }
}

pub(super) async fn cli_handle_config_updated_from_sse(
    runtime: &CliExecutionRuntime,
    _api_client: &CliApiClient,
) {
    let _ = runtime;
}

/// Handle an incoming SSE event from the server — update topology,
/// frontend projection, and render output blocks.
pub(super) fn handle_sse_event(
    runtime: &CliExecutionRuntime,
    event: CliServerEvent,
    style: &CliStyle,
) {
    let root_session_id = runtime.server_session_id.as_deref();
    let focused_session_id = cli_focused_session_id(runtime);
    let is_root_session = |event_session_id: &str| {
        root_session_id.is_none_or(|sid| event_session_id.is_empty() || sid == event_session_id)
    };
    let is_related_session =
        |event_session_id: &str| cli_tracks_related_session(runtime, event_session_id);

    match event {
        CliServerEvent::StreamReconnecting { delay_ms } => {
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.run_tail = Some(crate::run::frontend_state_types::CliRunTailState {
                    status: "reconnecting".to_string(),
                    detail: Some(format!(
                        "retrying in {}s",
                        ((delay_ms + 999) / 1000).max(1)
                    )),
                });
            }
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::StreamConnected => {
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                if projection
                    .run_tail
                    .as_ref()
                    .is_some_and(|tail| tail.status == "reconnecting")
                {
                    projection.run_tail = None;
                }
            }
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::ConfigUpdated => {
            tracing::debug!("config.updated reached sync handler");
        }
        CliServerEvent::SessionUpdated { session_id, source } => {
            if !is_root_session(&session_id) {
                return;
            }
            tracing::debug!(session_id, ?source, "session updated");
        }
        CliServerEvent::SessionBusy { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.last_turn_tokens =
                    crate::run::frontend_state_types::CliLastTurnTokenStats::default();
                projection.set_runtime_activity(
                    CliFrontendPhase::Busy,
                    Some(cli_summary_thinking_label()),
                );
                projection.run_tail = None;
            }
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::SessionIdle { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            cli_frontend_set_phase(&runtime.frontend_projection, CliFrontendPhase::Idle, None);
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::SessionRetrying { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.run_tail =
                    Some(crate::run::frontend_state_types::CliRunTailState {
                        status: "retrying".to_string(),
                        detail: Some("Waiting for automatic retry.".to_string()),
                    });
            }
            cli_push_runtime_aux_block(
                runtime,
                OutputBlock::Status(StatusBlock::warning("Retry scheduled.")),
            );
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::QuestionCreated {
            request_id,
            session_id,
            ..
        } => {
            tracing::warn!(
                request_id,
                session_id,
                "question.created reached sync handler — skipping"
            );
        }
        CliServerEvent::QuestionResolved { request_id } => {
            tracing::debug!(request_id, "question resolved");
        }
        CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            ..
        } => {
            tracing::warn!(
                session_id,
                permission_id,
                "permission.requested reached sync handler — skipping"
            );
        }
        CliServerEvent::PermissionResolved {
            session_id,
            permission_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(session_id, permission_id, "permission resolved");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.pending_permission_count = 0;
                projection.submitting_permission_count = 0;
                projection.last_permission_submit_error = None;
                projection.permission_submit_completed_at = Some(permission_timestamp_now());
                cli_restore_compact_summary(&mut projection);
            }
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::ToolCallStarted {
            session_id,
            tool_call_id,
            tool_name,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            if let Ok(mut topology) = runtime.observed_topology.lock() {
                topology.active = true;
            }
            tracing::debug!(tool_call_id, tool_name, "tool call started");
            cli_store_active_tool_label(runtime, &tool_call_id, &tool_name);
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.set_runtime_activity(
                    CliFrontendPhase::Busy,
                    Some(cli_summary_tool_label(&tool_name)),
                );
            }
            cli_push_runtime_aux_block(
                runtime,
                OutputBlock::Status(StatusBlock::title(cli_summary_tool_label(&tool_name))),
            );
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::ToolCallCompleted {
            session_id,
            tool_call_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(tool_call_id, "tool call completed");
            let _ = cli_take_active_tool_label(runtime, &tool_call_id);
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                cli_restore_compact_summary(&mut projection);
            }
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::AttachedSessionAttached {
            parent_id,
            attached_id,
        } => {
            if cli_track_attached_session(runtime, &parent_id, &attached_id) {
                tracing::debug!(parent_id, attached_id, "tracked attached session");
            }
        }
        CliServerEvent::AttachedSessionDetached {
            parent_id,
            attached_id,
        } => {
            if cli_untrack_attached_session(runtime, &parent_id, &attached_id) {
                tracing::debug!(parent_id, attached_id, "untracked attached session");
            }
        }
        CliServerEvent::OutputBlock {
            session_id,
            id,
            live_identity,
            payload,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            let block_payload = payload.get("block").unwrap_or(&payload);
            let Some(block) = parse_output_block(block_payload) else {
                tracing::debug!(?id, payload = %block_payload, "failed to parse output_block");
                return;
            };
            if live_identity.is_none() {
                cli_observe_terminal_stream_block(runtime, &session_id, id.as_deref(), &block);
            }
            if matches!(block, OutputBlock::Reasoning(_))
                && !runtime.show_thinking.load(Ordering::SeqCst)
            {
                return;
            }
            if let Ok(mut topology) = runtime.observed_topology.lock() {
                topology.observe_block(&block);
            }
            if let OutputBlock::SchedulerStage(stage) = &block {
                if let Some(attached_id) = stage.attached_session_id.as_deref() {
                    let _ = cli_track_attached_session(runtime, &session_id, attached_id);
                }
            }
            cli_frontend_observe_block(&runtime.frontend_projection, &block);
            let transcript_bearing_identity = live_identity.as_ref().filter(|identity| {
                LiveSemanticConsumer::is_transcript_bearing_kind(&identity.part_kind)
            });
            let buffered_transcript_identity = transcript_bearing_identity.filter(|identity| {
                matches!(
                    identity.part_kind,
                    agendao_types::LiveMessagePartKind::AssistantText
                        | agendao_types::LiveMessagePartKind::AssistantReasoning
                )
            });
            let block_updates_authority =
                cli_output_block_updates_transcript_authority(&block, live_identity.as_ref());
            let prompt_owned_live_transcript = transcript_bearing_identity.is_some()
                && (runtime.terminal_surface.is_some() || runtime.prompt_session.is_some());
            let skip_secondary_live_render =
                buffered_transcript_identity.is_some() && prompt_owned_live_transcript;
            let sync_live_transcript = transcript_bearing_identity.is_some()
                && (buffered_transcript_identity.is_none()
                    || matches!(
                        live_identity.as_ref().map(|identity| identity.phase),
                        Some(agendao_types::LivePartPhase::End)
                    ));
            let finalize_live_transcript = buffered_transcript_identity.is_some()
                && matches!(
                    live_identity.as_ref().map(|identity| identity.phase),
                    Some(agendao_types::LivePartPhase::End)
                );
            let mut prompt_owned_transcript_refreshed = false;
            if let Some(identity) = transcript_bearing_identity {
                if is_root_session(&session_id) {
                    if let Ok(mut root) = runtime.root_session_transcript.lock() {
                        cli_apply_live_slot_update(&mut root, &block, identity, style);
                        if sync_live_transcript && cli_is_root_focused(runtime) {
                            cli_sync_projection_transcript(runtime, root.clone());
                            if prompt_owned_live_transcript {
                                cli_refresh_prompt(runtime);
                                prompt_owned_transcript_refreshed = true;
                            }
                        }
                    }
                } else if let Ok(mut transcripts) = runtime.attached_session_transcripts.lock() {
                    let transcript = transcripts.entry(session_id.clone()).or_default();
                    cli_apply_live_slot_update(transcript, &block, identity, style);
                    if sync_live_transcript
                        && focused_session_id.as_deref() == Some(session_id.as_str())
                    {
                        cli_sync_projection_transcript(runtime, transcript.clone());
                        if prompt_owned_live_transcript {
                            cli_refresh_prompt(runtime);
                            prompt_owned_transcript_refreshed = true;
                        }
                    }
                }
            }
            if !is_root_session(&session_id) {
                if skip_secondary_live_render {
                    if finalize_live_transcript
                        && focused_session_id.as_deref() == Some(session_id.as_str())
                        && !prompt_owned_transcript_refreshed
                    {
                        cli_refresh_prompt(runtime);
                    }
                    return;
                }
                let rendered = cli_render_session_block(
                    runtime,
                    &session_id,
                    id.as_deref(),
                    &block,
                    live_identity.as_ref(),
                    style,
                );
                if block_updates_authority && transcript_bearing_identity.is_none() {
                    cli_append_session_rendered_transcript(runtime, &session_id, &rendered);
                }
                if focused_session_id.as_deref() == Some(session_id.as_str()) {
                    if let Some(surface) = runtime.terminal_surface.as_ref() {
                        if prompt_owned_live_transcript {
                            if !prompt_owned_transcript_refreshed {
                                cli_refresh_prompt(runtime);
                            }
                        } else if !block_updates_authority {
                            let _ = surface.print_rendered_passthrough(&rendered);
                        } else {
                            let _ = surface.print_rendered_stream(&rendered);
                        }
                    } else if !rendered.is_empty() {
                        if block_updates_authority && transcript_bearing_identity.is_none() {
                            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                                projection.transcript.append_rendered(&rendered);
                                projection.scroll_offset = 0;
                            }
                        }
                        // P0-3: LEGACY fallback — direct stdout when no terminal surface
                        // (pipe/non-interactive mode). In interactive mode, all output must
                        // go through CliTerminalSurface.append_rendered().
                        print!("{}", rendered);
                        let _ = io::stdout().flush();
                    }
                }
                return;
            }
            match &block {
                OutputBlock::SchedulerStage(stage)
                    if !cli_should_emit_scheduler_stage_block(
                        &runtime.scheduler_stage_snapshots,
                        stage,
                    ) => {}
                OutputBlock::SchedulerStage(stage)
                    if !cli_is_terminal_stage_status(stage.status.as_deref()) =>
                {
                    if let Ok(mut projection) = runtime.frontend_projection.lock() {
                        projection.active_stage = Some(stage.as_ref().clone());
                        projection.active_collapsed = false;
                    }
                    cli_refresh_prompt(runtime);
                }
                OutputBlock::SchedulerStage(_) => {
                    if let Ok(mut projection) = runtime.frontend_projection.lock() {
                        projection.active_stage = None;
                        projection.active_collapsed = true;
                    }
                    cli_refresh_prompt(runtime);
                }
                _ => {
                    if skip_secondary_live_render {
                        if finalize_live_transcript
                            && cli_is_root_focused(runtime)
                            && !prompt_owned_transcript_refreshed
                        {
                            cli_refresh_prompt(runtime);
                        }
                        return;
                    }
                    let rendered =
                        cli_render_session_block(
                            runtime,
                            "",
                            id.as_deref(),
                            &block,
                            live_identity.as_ref(),
                            style,
                        );
                    if block_updates_authority && transcript_bearing_identity.is_none() {
                        cli_append_session_rendered_transcript(runtime, "", &rendered);
                    }
                    if cli_is_root_focused(runtime) {
                        if let Some(surface) = runtime.terminal_surface.as_ref() {
                            if prompt_owned_live_transcript {
                                if !prompt_owned_transcript_refreshed {
                                    cli_refresh_prompt(runtime);
                                }
                            } else if !block_updates_authority {
                                let _ = surface.print_rendered_passthrough(&rendered);
                            } else {
                                let _ = surface.print_rendered_stream(&rendered);
                            }
                        } else if !rendered.is_empty() {
                            if block_updates_authority && transcript_bearing_identity.is_none() {
                                if let Ok(mut projection) = runtime.frontend_projection.lock() {
                                    projection.transcript.append_rendered(&rendered);
                                    projection.scroll_offset = 0;
                                }
                            }
                            // P0-3: LEGACY fallback — direct stdout when no terminal surface.
                            print!("{}", rendered);
                            let _ = io::stdout().flush();
                        }
                    }
                }
            }
        }
        CliServerEvent::Error {
            session_id,
            error,
            message_id,
            done,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            if !is_root_session(&session_id) {
                tracing::error!(session_id, error, ?message_id, ?done, "attached session error");
                return;
            }
            tracing::error!(error, ?message_id, ?done, "server error");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.set_runtime_activity(CliFrontendPhase::Failed, None);
                projection.run_tail =
                    Some(crate::run::frontend_state_types::CliRunTailState {
                    status: "error".to_string(),
                    detail: Some(error),
                    });
            }
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::Usage {
            session_id,
            prompt_tokens,
            completion_tokens,
            message_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(prompt_tokens, completion_tokens, ?message_id, "token usage");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.last_turn_tokens.input_tokens = prompt_tokens;
                projection.last_turn_tokens.output_tokens = completion_tokens;
                projection.token_stats.input_tokens = projection
                    .token_stats
                    .input_tokens
                    .saturating_add(prompt_tokens);
                projection.token_stats.output_tokens = projection
                    .token_stats
                    .output_tokens
                    .saturating_add(completion_tokens);
            }
            if !is_root_session(&session_id) {
                return;
            }
            if prompt_tokens > 0 || completion_tokens > 0 {
                if let Ok(mut projection) = runtime.frontend_projection.lock() {
                    projection.run_tail =
                        Some(crate::run::frontend_state_types::CliRunTailState {
                        status: "complete".to_string(),
                        detail: Some(format!(
                            "input {} · output {}",
                            format_token_count(prompt_tokens),
                            format_token_count(completion_tokens)
                        )),
                        });
                }
                cli_refresh_prompt(runtime);
            }
        }
        CliServerEvent::Unknown { event, data } => {
            tracing::trace!("Ignoring unknown SSE event: {} ({})", event, data);
        }
    }
}

pub(super) async fn handle_question_from_sse(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    request_id: &str,
    questions_json: &serde_json::Value,
) {
    if let Ok(mut projection) = runtime.frontend_projection.lock() {
        projection.set_runtime_activity(
            CliFrontendPhase::Waiting,
            Some(cli_summary_waiting_label()),
        );
    }
    cli_push_runtime_aux_block(
        runtime,
        OutputBlock::Status(StatusBlock::warning(cli_summary_waiting_label())),
    );
    cli_refresh_prompt(runtime);

    let questions: Vec<agendao_tool::QuestionDef> =
        match serde_json::from_value(questions_json.clone()) {
            Ok(questions) => questions,
            Err(error) => {
                tracing::warn!("Failed to parse questions from SSE: {}", error);
                if let Err(reject_error) =
                    crate::local_dispatch::reject_question(local_state, api_client, request_id)
                        .await
                {
                    tracing::warn!(
                        request_id,
                        error = %reject_error,
                        "Failed to reject malformed question request"
                    );
                }
                return;
            }
        };

    if questions.is_empty() {
        tracing::debug!("Empty question list from SSE — rejecting");
        if let Err(error) =
            crate::local_dispatch::reject_question(local_state, api_client, request_id).await
        {
            tracing::warn!(
                request_id,
                error = %error,
                "Failed to reject empty question request"
            );
        }
        return;
    }

    let guard = runtime
        .spinner_guard
        .lock()
        .map(|spinner| spinner.clone())
        .unwrap_or_else(|_| SpinnerGuard::noop());
    let result = cli_ask_question(
        questions,
        runtime.observed_topology.clone(),
        runtime.frontend_projection.clone(),
        runtime.prompt_session_slot.clone(),
        runtime.terminal_surface.clone(),
        guard,
    )
    .await;

    match result {
        Ok(answers) => {
            if let Err(error) =
                crate::local_dispatch::reply_question(local_state, api_client, request_id, answers)
                    .await
            {
                tracing::error!("Failed to reply question `{}`: {}", request_id, error);
            }
        }
        Err(_) => {
            if let Err(error) =
                crate::local_dispatch::reject_question(local_state, api_client, request_id).await
            {
                tracing::error!("Failed to reject question `{}`: {}", request_id, error);
            }
        }
    }
}

pub(super) async fn handle_permission_from_sse(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    permission_id: &str,
    info_json: &serde_json::Value,
) {
    let permission_label =
        serde_json::from_value::<crate::api_client::PermissionRequestInfo>(info_json.clone())
            .ok()
            .map(|info| cli_summary_permission_label(Some(info.tool.as_str())))
            .unwrap_or_else(|| cli_summary_permission_label(None));
    if let Ok(mut projection) = runtime.frontend_projection.lock() {
        projection.set_runtime_activity(CliFrontendPhase::Waiting, Some(permission_label.clone()));
        projection.pending_permission_count = 1;
        projection.submitting_permission_count = 0;
        projection.last_permission_submit_error = None;
    }
    cli_push_runtime_aux_block(
        runtime,
        OutputBlock::Status(StatusBlock::warning(permission_label)),
    );
    cli_refresh_prompt(runtime);

    let info: crate::api_client::PermissionRequestInfo =
        match serde_json::from_value(info_json.clone()) {
            Ok(info) => info,
            Err(error) => {
                tracing::warn!(permission_id, %error, "failed to parse permission info from SSE");
                let _ = crate::local_dispatch::reply_permission(
                    local_state,
                    api_client,
                    permission_id,
                    "reject",
                    Some("Invalid permission request payload".to_string()),
                )
                .await;
                return;
            }
        };

    let input = info.input.as_object().cloned().unwrap_or_default();
    let permission = input
        .get("permission")
        .and_then(|value| value.as_str())
        .unwrap_or(info.tool.as_str())
        .to_string();
    let patterns = input
        .get("patterns")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let metadata = input
        .get("metadata")
        .and_then(|value| value.as_object())
        .map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let permission_class = info.permission_class.as_deref().and_then(permission_class_label);
    let scope_key = info.scope_key.clone();
    let scope_label = info.scope_label.clone();
    let lifetimes = info
        .supported_lifetimes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let lifetimes = if lifetimes.is_empty() {
        input.get("supported_lifetimes")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        lifetimes
    };
    let lifetimes = lifetimes
        .iter()
        .filter_map(|value| match *value {
            "once" => Some(agendao_permission::PermissionLifetime::Once),
            "turn" => Some(agendao_permission::PermissionLifetime::Turn),
            "session" | "always" => Some(agendao_permission::PermissionLifetime::Session),
            _ => None,
        })
        .collect::<Vec<_>>();
    let lifetimes = if lifetimes.is_empty() {
        default_permission_lifetimes(info.permission_class.as_deref())
    } else {
        lifetimes
    };
    let guard = runtime
        .spinner_guard
        .lock()
        .map(|spinner| spinner.clone())
        .unwrap_or_else(|_| SpinnerGuard::noop());
    guard.pause();
    let prompt_session = runtime
        .prompt_session_slot
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    let suspended_by_surface = match runtime.terminal_surface.as_ref() {
        Some(surface) => surface.suspend_modal_prompt().unwrap_or(false),
        None => false,
    };
    let suspended_directly = !suspended_by_surface && prompt_session.is_some();
    if suspended_directly {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.suspend();
        }
    }

    {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(stdout, crossterm::cursor::Show);
        let _ = stdout.flush();
    }

    let decision = {
        let permission = permission.clone();
        let patterns = patterns.clone();
        let metadata = metadata.clone();
        let permission_class = permission_class.clone();
        let scope_key = scope_key.clone();
        let scope_label = scope_label.clone();
        let matcher_label = info.matcher_label.clone();
        let grant_target_summary = info.grant_target_summary.clone();
        let risk_tags = info.risk_tags.clone();
        let lifetimes = lifetimes.clone();
        tokio::task::spawn_blocking(move || {
            let style = CliStyle::detect();
            prompt_permission(
                &permission,
                permission_class.as_deref(),
                scope_key.as_deref(),
                scope_label.as_deref(),
                matcher_label.as_deref(),
                grant_target_summary.as_deref(),
                &patterns,
                &metadata,
                &lifetimes,
                &risk_tags,
                &style,
            )
        })
        .await
    };

    guard.resume();

    let decision = match decision {
        Ok(Ok(decision)) => decision,
        Ok(Err(error)) => {
            tracing::error!(permission_id, %error, "permission prompt IO error");
            eprintln!("Permission prompt IO error for {permission_id}: {error}");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.submitting_permission_count = 0;
                projection.last_permission_submit_error = Some(error.to_string());
                projection.permission_submit_completed_at = Some(permission_timestamp_now());
            }
            if let Some(surface) = runtime.terminal_surface.as_ref() {
                let _ = surface.resume_modal_prompt(suspended_by_surface);
            } else if suspended_directly {
                if let Some(prompt_session) = prompt_session.as_ref() {
                    let _ = prompt_session.resume();
                }
            }
            cli_refresh_prompt(runtime);
            let _ = crate::local_dispatch::reply_permission(
                local_state,
                api_client,
                permission_id,
                "reject",
                Some(format!("Permission prompt IO error: {}", error)),
            )
            .await;
            return;
        }
        Err(error) => {
            tracing::error!(permission_id, %error, "permission prompt task failed");
            eprintln!("Permission prompt failed for {permission_id}: {error}");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.submitting_permission_count = 0;
                projection.last_permission_submit_error = Some(error.to_string());
                projection.permission_submit_completed_at = Some(permission_timestamp_now());
            }
            if let Some(surface) = runtime.terminal_surface.as_ref() {
                let _ = surface.resume_modal_prompt(suspended_by_surface);
            } else if suspended_directly {
                if let Some(prompt_session) = prompt_session.as_ref() {
                    let _ = prompt_session.resume();
                }
            }
            cli_refresh_prompt(runtime);
            let _ = crate::local_dispatch::reply_permission(
                local_state,
                api_client,
                permission_id,
                "reject",
                Some(format!("Permission prompt failed: {}", error)),
            )
            .await;
            return;
        }
    };

    if let Some(surface) = runtime.terminal_surface.as_ref() {
        let _ = surface.resume_modal_prompt(suspended_by_surface);
    } else if suspended_directly {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.resume();
        }
    }

    let (reply, message) = match decision {
        PermissionDecision::Allow => ("once", Some("approved".to_string())),
        PermissionDecision::AllowTurn => ("turn", Some("approved for turn".to_string())),
        PermissionDecision::AllowSession => {
            ("session", Some("approved for session".to_string()))
        }
        PermissionDecision::Deny => ("reject", Some("rejected".to_string())),
    };

    if let Ok(mut projection) = runtime.frontend_projection.lock() {
        projection.submitting_permission_count = 1;
        projection.last_permission_submit_error = None;
        projection.permission_submit_started_at = Some(permission_timestamp_now());
    }
    cli_refresh_prompt(runtime);

    if let Err(error) = crate::local_dispatch::reply_permission(
        local_state,
        api_client,
        permission_id,
        reply,
        message,
    )
    .await
    {
        tracing::error!(permission_id, %error, "failed to reply permission");
        if let Ok(mut projection) = runtime.frontend_projection.lock() {
            projection.submitting_permission_count = 0;
            projection.last_permission_submit_error = Some(error.to_string());
            projection.permission_submit_completed_at = Some(permission_timestamp_now());
        }
        cli_refresh_prompt(runtime);
        eprintln!(
            "Failed to submit permission reply for {permission_id}: {error}. Server may still be waiting."
        );
    } else {
        if let Ok(mut projection) = runtime.frontend_projection.lock() {
            projection.permission_submit_completed_at = Some(permission_timestamp_now());
        }
        cli_refresh_prompt(runtime);
    }
}

pub(super) async fn cli_refresh_server_info(
    api_client: &CliApiClient,
    projection: &Arc<Mutex<CliFrontendProjection>>,
    server_session_id: Option<&str>,
) {
    match api_client.get_all_providers().await {
        Ok(response) => {
            let mut model_catalog = std::collections::HashMap::new();
            for provider in response.all {
                for model in provider.models {
                    model_catalog.insert(
                        format!("{}/{}", provider.id, model.id),
                        CliModelCatalogEntry::from_provider_model(
                            model.context_window,
                            #[cfg(test)]
                            model.cost_per_million_input,
                            #[cfg(test)]
                            model.cost_per_million_output,
                        ),
                    );
                }
            }
            if let Ok(mut projection) = projection.lock() {
                projection.model_catalog = model_catalog;
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh provider catalogue: {}", error);
        }
    }

    match api_client.get_mcp_status().await {
        Ok(servers) => {
            let statuses: Vec<CliMcpServerStatus> = servers.into_iter().map(Into::into).collect();
            if let Ok(mut projection) = projection.lock() {
                projection.mcp_servers = statuses;
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh MCP status: {}", error);
        }
    }

    match api_client.get_lsp_servers().await {
        Ok(servers) => {
            if let Ok(mut projection) = projection.lock() {
                projection.lsp_servers = servers;
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh LSP status: {}", error);
        }
    }

    if let Some(session_id) = server_session_id {
        cli_refresh_session_telemetry(api_client, projection, session_id).await;
    }
}

pub(super) async fn cli_refresh_session_telemetry(
    api_client: &CliApiClient,
    projection: &Arc<Mutex<CliFrontendProjection>>,
    session_id: &str,
) {
    match api_client.get_session_telemetry(session_id).await {
        Ok(telemetry) => {
            if let Ok(mut projection) = projection.lock() {
                projection.session_runtime = Some(telemetry.runtime.clone());
                projection.stage_summaries = telemetry.stages.clone();
                projection.telemetry_topology = Some(telemetry.topology.clone());
                if matches!(
                    telemetry.runtime.run_status,
                    crate::api_client::SessionRunStatusKind::Compacting
                ) {
                    projection.run_tail =
                        Some(crate::run::frontend_state_types::CliRunTailState {
                            status: "compacting".to_string(),
                            detail: Some("Preparing a smaller context window.".to_string()),
                        });
                } else if projection
                    .run_tail
                    .as_ref()
                    .is_some_and(|tail| tail.status == "compacting")
                {
                    projection.run_tail = None;
                }
                projection.sync_usage_from_snapshot(&telemetry.usage, Some(&telemetry.usage_books));
                projection.cache_diagnostic =
                    cli_context_closure_cache_diagnostic_label(
                        telemetry.context_closure_contract.as_ref(),
                    )
                    .or_else(|| {
                        telemetry
                            .cache_evidence
                            .as_ref()
                            .cloned()
                            .and_then(|value| serde_json::from_value(value).ok())
                            .and_then(|summary| cli_cache_evidence_status_label(&summary))
                    })
                    .or_else(|| {
                        telemetry
                            .cache_semantics
                            .as_ref()
                            .and_then(|summary| summary.label.clone())
                    });
                projection.ingress_diagnostic =
                    ingress_stabilization_label(telemetry.ingress_stabilization.as_ref());
                projection.provider_diagnostic =
                    provider_diagnostic_label(telemetry.provider_diagnostic_summary.as_ref());
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh session telemetry: {}", error);
        }
    }
}

pub(super) fn cli_history_message_role(role: &str) -> Option<OutputMessageRole> {
    match role {
        "user" => Some(OutputMessageRole::User),
        "assistant" => Some(OutputMessageRole::Assistant),
        _ => None,
    }
}

pub(super) fn cli_append_history_text_block(
    transcript: &mut CliVisibleTranscript,
    role: Option<OutputMessageRole>,
    text: &mut String,
    style: &CliStyle,
) {
    let Some(role) = role else {
        text.clear();
        return;
    };
    if text.trim().is_empty() {
        text.clear();
        return;
    }
    let rendered = render_cli_block_rich(
        &OutputBlock::Message(MessageBlock::full(role, std::mem::take(text))),
        style,
    );
    transcript.append_committed(&rendered);
}

pub(super) fn cli_append_history_tool_result_block(
    transcript: &mut CliVisibleTranscript,
    tool_result: &crate::api_client::ToolResult,
    style: &CliStyle,
) {
    let tool_name = tool_result
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&tool_result.tool_call_id)
        .to_string();
    let detail = tool_result
        .content
        .trim()
        .is_empty()
        .then_some(String::new())
        .and_then(|_| None)
        .or_else(|| Some(tool_result.content.clone()));
    let block = if tool_result.is_error {
        OutputBlock::Tool(agendao_command::output_blocks::ToolBlock::error(
            tool_name,
            detail.unwrap_or_else(|| "tool failed".to_string()),
        ))
    } else {
        OutputBlock::Tool(agendao_command::output_blocks::ToolBlock::done(tool_name, detail))
    };
    transcript.append_committed(&render_cli_block_rich(&block, style));
}

pub(super) fn cli_append_history_reasoning_block(
    transcript: &mut CliVisibleTranscript,
    text: &mut String,
    style: &CliStyle,
) {
    if text.trim().is_empty() {
        text.clear();
        return;
    }
    let rendered = render_cli_block_rich(
        &OutputBlock::Reasoning(ReasoningBlock::full(std::mem::take(text))),
        style,
    );
    transcript.append_committed(&rendered);
}

pub(super) fn cli_transcript_from_history(
    messages: &[crate::api_client::MessageInfo],
    style: &CliStyle,
) -> CliVisibleTranscript {
    let mut transcript = CliVisibleTranscript::default();

    for message in messages {
        let role = cli_history_message_role(&message.role);
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();

        for part in &message.parts {
            if part.ignored == Some(true) {
                continue;
            }

            if part.part_type == "text" {
                cli_append_history_reasoning_block(&mut transcript, &mut pending_reasoning, style);
                if let Some(text) = part.text.as_deref() {
                    pending_text.push_str(text);
                }
                continue;
            }

            if part.part_type == "reasoning" {
                cli_append_history_text_block(&mut transcript, role, &mut pending_text, style);
                if let Some(text) = part.text.as_deref() {
                    pending_reasoning.push_str(text);
                }
                continue;
            }

            if let Some(tool_result) = part.tool_result.as_ref() {
                cli_append_history_text_block(&mut transcript, role, &mut pending_text, style);
                cli_append_history_reasoning_block(&mut transcript, &mut pending_reasoning, style);
                cli_append_history_tool_result_block(&mut transcript, tool_result, style);
            }
        }

        cli_append_history_reasoning_block(&mut transcript, &mut pending_reasoning, style);
        cli_append_history_text_block(&mut transcript, role, &mut pending_text, style);
    }

    transcript
}

pub(super) async fn cli_refresh_session_transcript_from_history(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    session_id: &str,
    style: &CliStyle,
) -> Option<CliVisibleTranscript> {
    match crate::local_dispatch::list_messages(local_state, api_client, session_id).await {
        Ok(messages) => {
            let transcript = cli_transcript_from_history(&messages, style);
            cli_replace_root_history_transcript(runtime, transcript);
            runtime
                .root_history_transcript
                .lock()
                .ok()
                .map(|history| history.clone())
        }
        Err(error) => {
            tracing::debug!("Failed to rebuild CLI transcript from history: {}", error);
            None
        }
    }
}

pub(super) async fn handle_session_updated_from_sse(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    session_id: &str,
    source: Option<&str>,
    style: &CliStyle,
) {
    let server_session_id = match runtime.server_session_id.as_deref() {
        Some(server_session_id) if server_session_id == session_id => server_session_id,
        _ => return,
    };
    if !cli_session_update_requires_refresh(source) {
        return;
    }
    let previous_visible_transcript = runtime
        .root_session_transcript
        .lock()
        .ok()
        .map(|transcript| transcript.clone());
    match crate::local_dispatch::get_session(local_state, api_client, server_session_id).await {
        Ok(session) => {
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.session_title = Some(session.title);
            }
        }
        Err(error) => {
            tracing::debug!(
                "Failed to refresh session title after session.updated: {}",
                error
            );
        }
    }
    cli_refresh_server_info(
        api_client,
        &runtime.frontend_projection,
        Some(server_session_id),
    )
    .await;
    let refreshed_history = cli_refresh_session_transcript_from_history(
        runtime,
        api_client,
        local_state,
        server_session_id,
        style,
    )
    .await;
    if let Some(transcript) = refreshed_history.clone() {
        cli_replace_root_session_transcript(runtime, transcript.clone());
        if cli_is_root_focused(runtime) {
            if cli_session_update_finishes_turn(source) {
                if let Some(previous_visible_transcript) = previous_visible_transcript.as_ref() {
                    if let Some(suffix) =
                        cli_history_transcript_suffix(previous_visible_transcript, &transcript)
                    {
                        if let Some(surface) = runtime.terminal_surface.as_ref() {
                            let _ = surface.print_rendered_stream(&suffix);
                        } else if !suffix.is_empty() {
                            print!("{suffix}");
                            let _ = io::stdout().flush();
                        }
                    }
                }
            }
            cli_sync_projection_transcript(runtime, transcript);
        }
    }
    if cli_session_update_finishes_turn(source) {
        if let Ok(mut active_tool_labels) = runtime.active_tool_labels.lock() {
            active_tool_labels.clear();
        }
    }
}

#[cfg(test)]
mod cli_history_transcript_tests {
    use super::cli_transcript_from_history;
    use crate::api_client::{MessageInfo, MessagePart, MessageTokensInfo, ToolResult};
    use agendao_command::cli_style::CliStyle;

    fn text_part(id: &str, text: &str) -> MessagePart {
        MessagePart {
            id: id.to_string(),
            part_type: "text".to_string(),
            text: Some(text.to_string()),
            file: None,
            tool_call: None,
            tool_result: None,
            synthetic: None,
            ignored: None,
        }
    }

    fn tool_result_part(id: &str, title: &str, content: &str) -> MessagePart {
        MessagePart {
            id: id.to_string(),
            part_type: "tool_result".to_string(),
            text: None,
            file: None,
            tool_call: None,
            tool_result: Some(ToolResult {
                tool_call_id: "tool-call-1".to_string(),
                content: content.to_string(),
                is_error: false,
                title: Some(title.to_string()),
                metadata: None,
                attachments: None,
            }),
            synthetic: None,
            ignored: None,
        }
    }

    fn reasoning_part(id: &str, text: &str) -> MessagePart {
        MessagePart {
            id: id.to_string(),
            part_type: "reasoning".to_string(),
            text: Some(text.to_string()),
            file: None,
            tool_call: None,
            tool_result: None,
            synthetic: None,
            ignored: None,
        }
    }

    fn ignored_text_part(id: &str, text: &str) -> MessagePart {
        MessagePart {
            ignored: Some(true),
            ..text_part(id, text)
        }
    }

    fn message(id: &str, role: &str, parts: Vec<MessagePart>) -> MessageInfo {
        MessageInfo {
            id: id.to_string(),
            session_id: "session-1".to_string(),
            role: role.to_string(),
            created_at: 0,
            completed_at: Some(1),
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            cost: 0.0,
            tokens: MessageTokensInfo::default(),
            parts,
            metadata: None,
            multimodal: None,
        }
    }

    #[test]
    fn history_transcript_rebuilds_final_user_assistant_and_tool_result_blocks() {
        let style = CliStyle::plain();
        let transcript = cli_transcript_from_history(
            &[
                message("msg-user", "user", vec![text_part("u1", "research x")]),
                message(
                    "msg-assistant",
                    "assistant",
                    vec![
                        reasoning_part("r1", "Thinking about the result.\n"),
                        text_part("a1", "Found two papers.\n"),
                        tool_result_part("tr1", "SkillsList", "Available skills: <available_skills>"),
                        ignored_text_part("a2", "internal-only"),
                        text_part("a3", "Next I will summarize them."),
                    ],
                ),
            ],
            &style,
        );

        let rendered = agendao_util::util::color::strip_ansi(&transcript.rendered_text());
        assert!(rendered.contains("research x"));
        assert!(rendered.contains("Found two papers."));
        assert!(rendered.contains("Thinking about the result."));
        assert!(rendered.contains("SkillsList"));
        assert!(rendered.contains("Available skills: <available_skills>"));
        assert!(rendered.contains("Next I will summarize them."));
        assert!(!rendered.contains("internal-only"));
    }
}
