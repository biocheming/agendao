use std::future::Future;
use std::path::{Path as FsPath, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use agendao_config::Config as AppConfig;
use agendao_memory::{
    load_last_prefetch_packet, load_persisted_memory_snapshot, render_frozen_snapshot_block,
    render_prefetch_packet_block, PersistedMemorySnapshot, MEMORY_LAST_PREFETCH_METADATA_KEY,
};
use agendao_types::{
    message_latest_compaction_summary, ExternalAdapterResolvedBinding, MemoryRetrievalPacket,
    MemoryRetrievalQuery, MessageRole, PartType as SessionPartType,
    SessionContinuityCompactionSummary, SessionContinuityLedgerEntry, SessionContinuityLedgerKind,
    SessionContinuityLimits, SessionContinuityMemoryAnchor, SessionContinuityPacket,
    SessionContinuityTaskLedger, SessionContinuityTurn, SessionMessage,
};
use agendao_util::util::format::truncate_chars;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::recovery::RecoveryExecutionContext;
use crate::routes::multimodal::resolve_provider_model;
use crate::session_runtime::events::{
    broadcast_session_reconcile, emit_output_block_via_hook, server_output_block_hook,
};
use crate::session_runtime::{assistant_visible_text, ensure_default_session_title, ModelPricing};
use crate::{ApiError, Result, ServerState};
use agendao_command::{
    Command, CommandArgumentField, CommandArgumentKind, CommandContext, InteractivePolicy,
};
use agendao_command_render::output_blocks::{
    MessageBlock, MessageRole as OutputMessageRole, OutputBlock,
};
use agendao_multimodal::{MultimodalAuthority, RuntimeMultimodalExplain, SessionPartAdapter};
use agendao_provider::ReasoningEffort;
use agendao_server_core::runtime_control::SessionRunStatus;
use agendao_server_core::runtime_events::ReconcileReason;
use agendao_session::prompt::assistant_text_live_identity;
use agendao_types::{ControlInputKind, ControlInputPhase, LivePartPhase};

use super::super::{
    apply_plugin_config_hooks, get_plugin_loader, plugin_auth::ensure_plugin_loader_active,
    should_apply_plugin_config_hooks,
};
use super::messages::{
    prompt_display_text, prompt_parts_from_session_parts, prompt_text_from_parts,
};
use super::scheduler::{apply_scheduler_selection_session_metadata, resolve_prompt_request_config};
use super::session_crud::{
    persist_session_if_enabled, persist_session_record_if_enabled, resolved_session_directory,
    set_session_run_status, IdleGuard,
};
use super::telemetry::persist_session_telemetry_metadata;

mod command_args;
mod live_web_ingress;
use command_args::{
    flatten as flatten_argument_values, hydrate as hydrate_scheduler_command_arguments,
    missing_required as missing_required_command_fields,
    normalize_field_key as normalize_command_field_key, parse as parse_command_argument_map,
};
use live_web_ingress::{
    drain as drain_live_web_ingress_batch, resolve as resolve_live_web_ingress_batch,
    stage as stage_live_web_ingress_batch, Stage as LiveWebIngressBatchStage,
    BATCH_WINDOW_MS as LIVE_WEB_INGRESS_BATCH_WINDOW_MS,
};

#[derive(Debug, Clone)]
struct ResolvedPromptPayload {
    display_text: String,
    execution_text: String,
    agent: Option<String>,
    model: Option<String>,
    scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    command: Option<Command>,
    pending_raw_arguments: Option<String>,
}

fn command_goal_statement(resolved: &ResolvedPromptPayload) -> Option<String> {
    let command = resolved
        .command
        .as_ref()
        .filter(|command| command.name == "goal")?;
    let invocation = command.invocation.as_ref()?;
    let parsed = parse_command_argument_map(
        resolved.pending_raw_arguments.as_deref(),
        &invocation.argument_schema,
    );
    parsed
        .get("goal")
        .and_then(|values| values.first())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) const VERIFIED_EXTERNAL_ADAPTER_BINDING_METADATA_KEY: &str =
    "verified_external_adapter_binding";

#[derive(Debug, Clone)]
pub(crate) struct VerifiedSessionIngress {
    pub ingress: agendao_session::prompt::IngressTurnEnvelope,
    pub external_adapter_binding: Option<ExternalAdapterResolvedBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedFollowupPrompt {
    request: SessionPromptRequest,
    apply_plugin_config_hooks: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_continuation_goal_generation: Option<u64>,
}

/// Server-owned state for the cross-run `/goal` continuation loop. This is
/// deliberately stored in session metadata (rather than process memory) so a
/// restart cannot silently forget that a goal has been making no progress.
const AUTO_CONTINUATION_STAGNATION_LIMIT: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TaskLedgerAutoContinuationState {
    goal_generation: u64,
    rounds: u32,
    last_fingerprint: String,
    stagnant_rounds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoContinuationStop {
    Cancelled,
    Completed,
    AwaitingUser,
    Blocked,
    NoGoal,
    NoNext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoContinuationPlan {
    Continue(TaskLedgerAutoContinuationState),
    Stop(AutoContinuationStop),
    Block {
        state: TaskLedgerAutoContinuationState,
        reason: String,
    },
}

fn task_ledger_continuation_fingerprint(
    ledger: &agendao_types::task_ledger::SessionTaskLedger,
) -> String {
    serde_json::to_string(&(
        ledger.goal_generation,
        ledger.status,
        ledger.blocked_reason.as_deref(),
        ledger.next.as_ref().map(|next| next.statement.as_str()),
        ledger
            .open_questions()
            .into_iter()
            .map(|question| question.id.as_str())
            .collect::<Vec<_>>(),
        agendao_types::task_ledger::current_checkpoints(ledger)
            .into_iter()
            .map(|checkpoint| checkpoint.id.as_str())
            .collect::<Vec<_>>(),
    ))
    .unwrap_or_default()
}

fn plan_task_ledger_auto_continuation(
    previous: Option<TaskLedgerAutoContinuationState>,
    ledger: &agendao_types::task_ledger::SessionTaskLedger,
    cancelled: bool,
) -> AutoContinuationPlan {
    if cancelled {
        return AutoContinuationPlan::Stop(AutoContinuationStop::Cancelled);
    }
    if ledger.goal.is_none() {
        return AutoContinuationPlan::Stop(AutoContinuationStop::NoGoal);
    }
    match ledger.status {
        agendao_types::task_ledger::TaskLedgerStatus::Completed => {
            return AutoContinuationPlan::Stop(AutoContinuationStop::Completed)
        }
        agendao_types::task_ledger::TaskLedgerStatus::AwaitingUser => {
            return AutoContinuationPlan::Stop(AutoContinuationStop::AwaitingUser)
        }
        agendao_types::task_ledger::TaskLedgerStatus::Blocked => {
            return AutoContinuationPlan::Stop(AutoContinuationStop::Blocked)
        }
        agendao_types::task_ledger::TaskLedgerStatus::Interrupted => {
            return AutoContinuationPlan::Stop(AutoContinuationStop::Cancelled)
        }
        agendao_types::task_ledger::TaskLedgerStatus::Active => {}
    }
    let Some(next) = ledger
        .next
        .as_ref()
        .filter(|next| !next.statement.trim().is_empty())
    else {
        return AutoContinuationPlan::Stop(AutoContinuationStop::NoNext);
    };
    let fingerprint = task_ledger_continuation_fingerprint(ledger);
    let (rounds, stagnant_rounds) = match previous {
        Some(previous) if previous.goal_generation == ledger.goal_generation => {
            let stagnant = if previous.last_fingerprint == fingerprint {
                previous.stagnant_rounds.saturating_add(1)
            } else {
                0
            };
            (previous.rounds.saturating_add(1), stagnant)
        }
        _ => (1, 0),
    };
    let state = TaskLedgerAutoContinuationState {
        goal_generation: ledger.goal_generation,
        rounds,
        last_fingerprint: fingerprint,
        stagnant_rounds,
    };
    if state.stagnant_rounds >= AUTO_CONTINUATION_STAGNATION_LIMIT {
        return AutoContinuationPlan::Block {
            reason: format!(
                "automatic continuation made no authoritative TaskLedger progress for {} consecutive rounds; next action was `{}`",
                state.stagnant_rounds, next.statement
            ),
            state,
        };
    }
    AutoContinuationPlan::Continue(state)
}

fn load_task_ledger_auto_continuation_state(
    session: &agendao_session::Session,
) -> Option<TaskLedgerAutoContinuationState> {
    serde_json::from_value(
        session
            .record()
            .metadata
            .get(crate::session_runtime::task_ledger::TASK_LEDGER_AUTO_CONTINUATION_METADATA_KEY)?
            .clone(),
    )
    .ok()
}

fn store_task_ledger_auto_continuation_state(
    session: &mut agendao_session::Session,
    state: &TaskLedgerAutoContinuationState,
) {
    if let Ok(value) = serde_json::to_value(state) {
        session.insert_metadata(
            crate::session_runtime::task_ledger::TASK_LEDGER_AUTO_CONTINUATION_METADATA_KEY,
            value,
        );
    }
}

fn clear_task_ledger_auto_continuation_state(session: &mut agendao_session::Session) {
    session.remove_metadata(
        crate::session_runtime::task_ledger::TASK_LEDGER_AUTO_CONTINUATION_METADATA_KEY,
    );
}

fn task_ledger_auto_resume_request(
    provider: &str,
    model: &str,
    variant: Option<String>,
    ledger: &agendao_types::task_ledger::SessionTaskLedger,
) -> SessionPromptRequest {
    let message = task_ledger_auto_resume_message(ledger);
    SessionPromptRequest {
        message: Some(message),
        parts: None,
        idempotency_key: None,
        ingress_source: Some("task-ledger-auto-continuation".to_string()),
        source_origin: Some(agendao_types::MessageSourceOrigin::Scheduler),
        source_surface: Some(agendao_types::MessageSourceSurface::Direct),
        model: Some(format!("{provider}/{model}")),
        variant,
        reasoning_effort: None,
        // Keep SchedulerChoice::Auto authoritative for every continuation;
        // an explicit agent would force the direct path and could skip the
        // verifier needed to earn a completion checkpoint.
        agent: None,
        scheduler: Some(agendao_orchestrator::selector::SchedulerChoice::Auto),
        command: None,
        arguments: None,
        recovery: Some(RecoveryExecutionContext::from_ledger(
            crate::recovery::RecoveryActionKind::Resume,
            ledger,
        )),
        auto_continuation_goal_generation: Some(ledger.goal_generation),
    }
}

fn task_ledger_auto_resume_message(
    ledger: &agendao_types::task_ledger::SessionTaskLedger,
) -> String {
    let goal = ledger
        .goal
        .as_ref()
        .map(|goal| goal.statement.as_str())
        .unwrap_or("the current goal");
    let next = ledger
        .next
        .as_ref()
        .map(|next| next.statement.as_str())
        .unwrap_or("the next concrete action");
    format!(
        "Continue the current server-authoritative TaskLedger goal autonomously.\n\nGoal: {goal}\nNext: {next}\n\nPerform the next concrete work, inspect actual results, and verify completion. Do not stop merely because the previous scheduler turn reached its step, token, or active-time budget. If the goal is not complete, leave a truthful Next action in the Ledger."
    )
}

fn is_task_ledger_auto_continuation(request: &SessionPromptRequest) -> bool {
    request.auto_continuation_goal_generation.is_some()
}

fn parse_request_reasoning_effort(raw: Option<&str>) -> Result<Option<ReasoningEffort>> {
    let Some(raw) = raw.map(str::trim) else {
        return Ok(None);
    };
    if raw.is_empty() || raw.eq_ignore_ascii_case("auto") || raw.eq_ignore_ascii_case("inherit") {
        return Ok(None);
    }
    raw.parse::<ReasoningEffort>().map(Some).map_err(|_| {
        ApiError::BadRequest(format!(
            "invalid reasoning_effort `{raw}` (expected none/minimal/low/medium/high/xhigh/max/ultra)"
        ))
    })
}

async fn prepare_task_ledger_auto_continuation(
    state: &Arc<ServerState>,
    session_id: &str,
    provider: &str,
    model: &str,
    variant: Option<String>,
    cancelled: bool,
) -> Option<QueuedFollowupPrompt> {
    let (plan, ledger) = {
        let mut sessions = state.sessions.lock().await;
        let session = sessions.get_mut(session_id)?;
        let ledger = crate::session_runtime::task_ledger::ledger_snapshot_from_record(
            session_id,
            session
                .record()
                .metadata
                .get(crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY),
        );
        let previous = load_task_ledger_auto_continuation_state(session);
        let plan = plan_task_ledger_auto_continuation(previous, &ledger, cancelled);
        match &plan {
            AutoContinuationPlan::Continue(next_state) => {
                store_task_ledger_auto_continuation_state(session, next_state)
            }
            AutoContinuationPlan::Block { .. } => {
                clear_task_ledger_auto_continuation_state(session)
            }
            AutoContinuationPlan::Stop(
                AutoContinuationStop::Completed
                | AutoContinuationStop::Blocked
                | AutoContinuationStop::Cancelled
                | AutoContinuationStop::NoGoal,
            ) => clear_task_ledger_auto_continuation_state(session),
            AutoContinuationPlan::Stop(
                AutoContinuationStop::AwaitingUser | AutoContinuationStop::NoNext,
            ) => {}
        }
        (plan, ledger)
    };

    match plan {
        AutoContinuationPlan::Continue(_) => {
            persist_session_record_if_enabled(state, session_id).await;
            Some(QueuedFollowupPrompt {
                request: task_ledger_auto_resume_request(provider, model, variant, &ledger),
                // The original run already used the plugin-applied config;
                // reuse that authoritative snapshot for this internal turn.
                apply_plugin_config_hooks: false,
                auto_continuation_goal_generation: Some(ledger.goal_generation),
            })
        }
        AutoContinuationPlan::Block { reason, .. } => {
            let result = crate::session_runtime::task_ledger::apply_task_ledger_op(
                state,
                session_id,
                ledger.revision,
                agendao_types::task_ledger::TaskLedgerOp::SetStatus {
                    status: agendao_types::task_ledger::TaskLedgerStatus::Blocked,
                    awaiting: None,
                    blocked_reason: Some(reason.clone()),
                },
            )
            .await;
            match result {
                Ok(_) => tracing::warn!(
                    session_id,
                    %reason,
                    "stopped TaskLedger auto-continuation after repeated no-progress rounds"
                ),
                Err(error) => tracing::warn!(
                    session_id,
                    %error,
                    "could not commit TaskLedger auto-continuation block; a concurrent ledger update won"
                ),
            }
            None
        }
        AutoContinuationPlan::Stop(_) => {
            persist_session_record_if_enabled(state, session_id).await;
            None
        }
    }
}

fn internal_prompt_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agendao-plugin-internal",
        axum::http::HeaderValue::from_static("1"),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(crate::routes::internal_token()) {
        headers.insert("x-agendao-internal-token", value);
    }
    headers
}

async fn enqueue_followup_prompt(
    state: &Arc<ServerState>,
    session_id: &str,
    queued: QueuedFollowupPrompt,
) -> Result<u64> {
    let queued_count = {
        let mut guard = state.queued_followups.lock().await;
        let queue = guard.entry(session_id.to_string()).or_default();
        queue.push_back(serde_json::to_value(&queued).map_err(|error| {
            ApiError::BadRequest(format!("failed to queue follow-up prompt: {error}"))
        })?);
        queue.len() as u64
    };
    state
        .runtime_telemetry
        .emit_control_input_transition(
            session_id,
            ControlInputKind::Followup,
            ControlInputPhase::Queued,
            chrono::Utc::now().timestamp_millis(),
        )
        .await;
    Ok(queued_count)
}

async fn take_followup_prompt(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Option<QueuedFollowupPrompt> {
    let value = {
        let mut guard = state.queued_followups.lock().await;
        let queue = guard.get_mut(session_id)?;
        // Human input wins over a synthetic Ledger continuation even when it
        // arrived just after the continuation was queued at the run boundary.
        let preferred = queue.iter().position(|value| {
            serde_json::from_value::<QueuedFollowupPrompt>(value.clone())
                .map(|queued| queued.auto_continuation_goal_generation.is_none())
                .unwrap_or(true)
        });
        match preferred {
            Some(index) => queue.remove(index)?,
            None => queue.pop_front()?,
        }
    };
    let mut queued: QueuedFollowupPrompt = match serde_json::from_value(value) {
        Ok(queued) => queued,
        Err(error) => {
            tracing::warn!(session_id, %error, "failed to decode queued follow-up prompt");
            return None;
        }
    };
    queued.request.auto_continuation_goal_generation = queued.auto_continuation_goal_generation;
    state
        .runtime_telemetry
        .emit_control_input_transition(
            session_id,
            ControlInputKind::Followup,
            ControlInputPhase::Adopted,
            chrono::Utc::now().timestamp_millis(),
        )
        .await;
    Some(queued)
}

/// Remove every queued follow-up prompt for the session. Aborting a run drops
/// the prompts queued behind it instead of letting a later run adopt them.
pub(crate) async fn drain_followup_prompts(state: &Arc<ServerState>, session_id: &str) -> usize {
    let dropped = {
        let mut guard = state.queued_followups.lock().await;
        guard.remove(session_id).map_or(0, |queue| queue.len())
    };
    if dropped > 0 {
        state
            .runtime_telemetry
            .emit_control_input_transition(
                session_id,
                ControlInputKind::Followup,
                ControlInputPhase::Cleared,
                chrono::Utc::now().timestamp_millis(),
            )
            .await;
    }
    dropped
}

pub(crate) fn load_verified_external_adapter_binding(
    session: &agendao_session::Session,
) -> Option<ExternalAdapterResolvedBinding> {
    serde_json::from_value(
        session
            .record()
            .metadata
            .get(VERIFIED_EXTERNAL_ADAPTER_BINDING_METADATA_KEY)?
            .clone(),
    )
    .ok()
}

pub(crate) fn persist_verified_external_adapter_binding(
    session: &mut agendao_session::Session,
    binding: &ExternalAdapterResolvedBinding,
) {
    if let Ok(value) = serde_json::to_value(binding) {
        session.insert_metadata(
            VERIFIED_EXTERNAL_ADAPTER_BINDING_METADATA_KEY.to_string(),
            value,
        );
    }
}

async fn resolve_prompt_payload(
    display_text: &str,
    session_id: &str,
    session_directory: &str,
    config: &AppConfig,
) -> Result<ResolvedPromptPayload> {
    let registry = super::super::command_registry_from_config(config);

    let Some(parsed) = registry.parse_invocation(display_text) else {
        return Ok(ResolvedPromptPayload {
            display_text: display_text.to_string(),
            execution_text: display_text.to_string(),
            agent: None,
            model: None,
            scheduler: None,
            command: None,
            pending_raw_arguments: None,
        });
    };

    let command = parsed.command.clone();
    let configured_command = config.command.as_ref().and_then(|commands| {
        commands.get(&command.name).or_else(|| {
            commands.values().find(|configured| {
                configured.name.as_deref().map(str::trim) == Some(command.name.as_str())
            })
        })
    });
    let scheduler = command.invocation.as_ref().and_then(|invocation| {
        if !matches!(
            invocation.mode,
            agendao_command::CommandExecutionMode::Scheduler
        ) || command.name == "goal"
        {
            // `/goal` uses the ordinary request selection path: auto by
            // default, direct when the user explicitly selected an Agent,
            // and an explicit request Scheduler when one was supplied.
            return None;
        }
        let template = if command.name.starts_with("autoresearch") {
            agendao_orchestrator::templates::TemplateId::Autoresearch
        } else {
            agendao_orchestrator::templates::TemplateId::Direct
        };
        Some(agendao_orchestrator::selector::SchedulerChoice::Template { template })
    });
    let raw_arguments_for_hydration = parsed.raw_arguments.clone();
    let raw_arguments_for_pending = parsed.raw_arguments.clone();
    let invocation = command.invocation.as_ref();
    let scheduler_defaults = invocation
        .map(|invocation| {
            hydrate_scheduler_command_arguments(
                &raw_arguments_for_hydration,
                &invocation.argument_schema,
            )
        })
        .transpose()?;
    let hydrated_raw_arguments = scheduler_defaults
        .as_ref()
        .map(|(_, raw)| raw.clone())
        .unwrap_or_else(|| raw_arguments_for_hydration.clone());
    let hydrated_arguments = if let Some((arguments, _)) = scheduler_defaults {
        flatten_argument_values(
            invocation
                .map(|item| item.argument_schema.as_slice())
                .unwrap_or(&[]),
            &arguments,
        )
    } else {
        parsed.arguments.clone()
    };

    let mut ctx =
        CommandContext::new(PathBuf::from(session_directory)).with_arguments(hydrated_arguments);
    let execution_raw_arguments =
        (!hydrated_raw_arguments.trim().is_empty()).then_some(hydrated_raw_arguments);
    let pending_raw_arguments =
        (!raw_arguments_for_pending.trim().is_empty()).then_some(raw_arguments_for_pending);
    if let Some(raw_arguments) = execution_raw_arguments.as_ref() {
        ctx = ctx.with_raw_arguments(raw_arguments.clone());
    }
    ctx = ctx
        .with_variable("SESSION_ID".to_string(), session_id.to_string())
        .with_variable("TIMESTAMP".to_string(), chrono::Utc::now().to_rfc3339());
    let execution_text = registry
        .execute_with_hooks(&command.name, ctx)
        .await
        .map_err(|error| {
            ApiError::BadRequest(format!(
                "Failed to execute command `/{}`: {}",
                command.name, error
            ))
        })?;

    Ok(ResolvedPromptPayload {
        display_text: display_text.to_string(),
        execution_text,
        agent: configured_command.and_then(|configured| configured.agent.clone()),
        model: configured_command.and_then(|configured| configured.model.clone()),
        scheduler,
        command: Some(command.clone()),
        pending_raw_arguments,
    })
}

async fn ensure_memory_frozen_snapshot(
    state: &Arc<ServerState>,
    session: &mut agendao_session::Session,
) -> Option<PersistedMemorySnapshot> {
    if let Some(snapshot) = load_persisted_memory_snapshot(session) {
        return Some(snapshot);
    }

    let packet = match state.runtime_memory.build_frozen_snapshot().await {
        Ok(packet) => packet,
        Err(error) => {
            tracing::warn!(
                session_id = %session.id,
                %error,
                "failed to build frozen memory snapshot"
            );
            return None;
        }
    };

    let snapshot = PersistedMemorySnapshot {
        rendered_block: render_frozen_snapshot_block(&packet),
        packet,
    };

    match serde_json::to_value(&snapshot) {
        Ok(value) => {
            session.insert_metadata(
                agendao_memory::MEMORY_FROZEN_SNAPSHOT_METADATA_KEY.to_string(),
                value,
            );
        }
        Err(error) => {
            tracing::warn!(
                session_id = %session.id,
                %error,
                "failed to serialize frozen memory snapshot"
            );
        }
    }
    Some(snapshot)
}

async fn build_memory_prefetch_packet(
    state: &Arc<ServerState>,
    session_id: &str,
    prompt_text: &str,
) -> Option<MemoryRetrievalPacket> {
    let trimmed = prompt_text.trim();
    let query = MemoryRetrievalQuery {
        query: (!trimmed.is_empty()).then_some(trimmed.to_string()),
        stage: None,
        limit: Some(6),
        kinds: Vec::new(),
        scopes: Vec::new(),
        session_id: Some(session_id.to_string()),
    };

    match state.runtime_memory.build_prefetch_packet(&query).await {
        Ok(packet) => Some(packet),
        Err(error) => {
            tracing::warn!(
                session_id,
                %error,
                "failed to build turn memory prefetch packet"
            );
            None
        }
    }
}

pub(super) async fn resolve_prompt_memory_context(
    state: &Arc<ServerState>,
    session: &mut agendao_session::Session,
    prompt_text: &str,
) -> (
    Option<String>,
    Option<MemoryRetrievalPacket>,
    Option<String>,
) {
    let frozen_snapshot = ensure_memory_frozen_snapshot(state, session).await;
    let prefetch_packet = build_memory_prefetch_packet(state, &session.id, prompt_text).await;

    if let Some(packet) = prefetch_packet.as_ref() {
        match serde_json::to_value(packet) {
            Ok(value) => {
                session.insert_metadata(MEMORY_LAST_PREFETCH_METADATA_KEY.to_string(), value);
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    %error,
                    "failed to serialize last prefetch memory packet"
                );
            }
        }
        if let Err(error) = state
            .runtime_memory
            .record_prefetch_usage(&session.id, packet)
            .await
        {
            tracing::warn!(
                session_id = %session.id,
                %error,
                "failed to persist memory prefetch usage event"
            );
        }
    } else {
        session.remove_metadata(MEMORY_LAST_PREFETCH_METADATA_KEY);
    }

    let frozen_block = frozen_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.rendered_block.clone());
    let prefetch_block = prefetch_packet
        .as_ref()
        .and_then(render_prefetch_packet_block);

    (frozen_block, prefetch_packet, prefetch_block)
}

const SCHEDULER_RECENT_TAIL_MESSAGES: usize = 6;
const SCHEDULER_CONTEXT_TEXT_LIMIT: usize = 4_000;
const SCHEDULER_CONTEXT_TURN_LIMIT: usize = 1_200;
pub(super) const SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY: &str =
    "scheduler_session_context_packet";

type SchedulerSessionContextPacket = SessionContinuityPacket;

pub(super) fn build_scheduler_session_context_packet(
    session: &agendao_session::Session,
) -> Option<SchedulerSessionContextPacket> {
    let exact_recent_tail = collect_scheduler_recent_tail(session);
    let memory_anchors = collect_scheduler_memory_anchors(session);
    let eligible_message_count = count_scheduler_context_messages(session);
    let latest_compaction_summary = latest_compaction_summary(session);
    let working_ledger = build_scheduler_working_ledger(session, &exact_recent_tail);
    let task_ledger = session
        .record()
        .metadata
        .get(agendao_types::task_ledger::TASK_LEDGER_METADATA_KEY)
        .and_then(|value| {
            serde_json::from_value::<agendao_types::task_ledger::SessionTaskLedger>(value.clone())
                .ok()
        })
        .filter(|ledger| ledger.revision > 0)
        .as_ref()
        .map(SessionContinuityTaskLedger::from);

    if exact_recent_tail.is_empty()
        && memory_anchors.is_empty()
        && working_ledger.is_empty()
        && task_ledger.is_none()
        && latest_compaction_summary.is_none()
    {
        return None;
    }

    let exact_recent_tail_count = exact_recent_tail.len();
    Some(SchedulerSessionContextPacket {
        eligible_message_count,
        exact_recent_tail_count,
        omitted_older_turns: eligible_message_count.saturating_sub(exact_recent_tail_count),
        exact_recent_tail,
        memory_anchors,
        working_ledger,
        task_ledger,
        latest_compaction_summary,
        limits: Some(SessionContinuityLimits {
            recent_tail_messages: SCHEDULER_RECENT_TAIL_MESSAGES,
            context_text_chars: SCHEDULER_CONTEXT_TEXT_LIMIT,
            turn_text_chars: SCHEDULER_CONTEXT_TURN_LIMIT,
        }),
        recall_policy: Some(
            "exact_tail_for_recent_followups; ledger_and_compaction_are_lossy; use_scheduler_context_hydrate_for_authorized_source_anchors_when_prior_exact_text_is_needed; use_scheduler_memory_hydrate_for_authorized_memory_anchors_when_exact_memory_detail_is_needed; use_memory_artifacts_or_tools_for_facts_outside_anchors"
                .to_string(),
        ),
        ..SchedulerSessionContextPacket::default()
    })
}

#[cfg(test)]
pub(super) fn build_scheduler_session_context_block(
    session: &agendao_session::Session,
) -> Option<String> {
    build_scheduler_session_context_packet(session).map(|packet| packet.render())
}

pub(super) fn merge_scheduler_prompt_with_memory(
    prompt_text: &str,
    frozen_snapshot_block: Option<&str>,
    prefetch_block: Option<&str>,
) -> String {
    let mut sections = Vec::new();
    if let Some(snapshot) = frozen_snapshot_block
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(snapshot.to_string());
    }
    if let Some(prefetch) = prefetch_block
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(prefetch.to_string());
    }
    sections.push(prompt_text.to_string());
    sections.join("\n\n")
}

fn collect_scheduler_recent_tail(session: &agendao_session::Session) -> Vec<SessionContinuityTurn> {
    let mut turns = session
        .messages
        .iter()
        .rev()
        .filter(|message| is_scheduler_context_message(message))
        .filter_map(|message| {
            let (text, projected) = scheduler_context_text_for_message(message);
            let text = text.trim();
            (!text.is_empty()).then(|| SessionContinuityTurn {
                message_id: message.id.clone(),
                role: role_label(&message.role).to_string(),
                text: text.to_string(),
                projected,
            })
        })
        .take(SCHEDULER_RECENT_TAIL_MESSAGES)
        .collect::<Vec<_>>();
    turns.reverse();
    turns
}

fn collect_scheduler_memory_anchors(
    session: &agendao_session::Session,
) -> Vec<SessionContinuityMemoryAnchor> {
    load_last_prefetch_packet(session)
        .map(|packet| {
            packet
                .items
                .into_iter()
                .map(|item| SessionContinuityMemoryAnchor {
                    record_id: item.card.id.0,
                    title: single_line(&truncate_chars(&item.card.title, 160)),
                    kind: format!("{:?}", item.card.kind),
                    status: format!("{:?}", item.card.status),
                    why_recalled: single_line(&truncate_chars(&item.why_recalled, 240)),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn count_scheduler_context_messages(session: &agendao_session::Session) -> usize {
    session
        .messages
        .iter()
        .filter(|message| is_scheduler_context_message(message))
        .filter(|message| !message.get_text().trim().is_empty())
        .count()
}

fn scheduler_context_text_for_message(message: &SessionMessage) -> (String, bool) {
    if matches!(message.role, MessageRole::Assistant) {
        if let Some(summary) = agendao_session::prompt::sanctioned_model_context_summary(message) {
            return (
                format!(
                    "Projected assistant output for model context. The visible assistant message is preserved in session history; use `scheduler_context_hydrate` with message id `{}` if exact text is required.\n\n{}",
                    message.id, summary
                ),
                true,
            );
        }
    }

    (message.get_text(), false)
}

fn is_scheduler_context_message(message: &SessionMessage) -> bool {
    matches!(message.role, MessageRole::User | MessageRole::Assistant)
}

fn latest_compaction_summary(
    session: &agendao_session::Session,
) -> Option<SessionContinuityCompactionSummary> {
    session.messages.iter().rev().find_map(|message| {
        if !matches!(message.role, MessageRole::Assistant) {
            return None;
        }
        if let Some(summary) =
            message_latest_compaction_summary(&message.metadata, &message.id, None)
        {
            return Some(summary);
        }
        for part in message.parts.iter().rev() {
            if let SessionPartType::Compaction { summary } = &part.part_type {
                let trimmed = summary.trim();
                if !trimmed.is_empty() {
                    return Some(SessionContinuityCompactionSummary {
                        message_id: message.id.clone(),
                        summary: trimmed.to_string(),
                    });
                }
            }
        }
        let is_summary = message
            .metadata
            .get("summary")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if is_summary {
            let text = message.get_text();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(SessionContinuityCompactionSummary {
                    message_id: message.id.clone(),
                    summary: trimmed.to_string(),
                });
            }
        }
        None
    })
}

fn build_scheduler_working_ledger(
    session: &agendao_session::Session,
    recent_tail: &[SessionContinuityTurn],
) -> Vec<SessionContinuityLedgerEntry> {
    let mut ledger = Vec::new();
    let title = session.title.trim();
    if !title.is_empty() && !session.is_default_title() {
        ledger.push(SessionContinuityLedgerEntry::new(
            SessionContinuityLedgerKind::SessionTitle,
            format!("session_title: {}", truncate_chars(title, 160)),
        ));
    }
    if let Some(summary) = session.summary.as_ref() {
        ledger.push(SessionContinuityLedgerEntry::new(
            SessionContinuityLedgerKind::SessionDiff,
            format!(
                "session_diff: files={} additions={} deletions={}",
                summary.files, summary.additions, summary.deletions
            ),
        ));
    }
    if let Some(turn) = recent_tail.iter().rev().find(|turn| turn.role == "user") {
        ledger.push(SessionContinuityLedgerEntry::with_source_id(
            SessionContinuityLedgerKind::LatestUserTurn,
            turn.message_id.clone(),
            format!(
                "latest_user_turn `{}`: {}",
                turn.message_id,
                single_line(&truncate_chars(&turn.text, 240))
            ),
        ));
    }
    if let Some(turn) = recent_tail
        .iter()
        .rev()
        .find(|turn| turn.role == "assistant")
    {
        ledger.push(SessionContinuityLedgerEntry::with_source_id(
            SessionContinuityLedgerKind::LatestAssistantOutcome,
            turn.message_id.clone(),
            format!(
                "latest_assistant_outcome `{}`: {}",
                turn.message_id,
                single_line(&truncate_chars(&turn.text, 360))
            ),
        ));
    }
    if !ledger.is_empty() {
        ledger.push(SessionContinuityLedgerEntry::new(
            SessionContinuityLedgerKind::SourcePolicy,
            "source_policy: use Exact Recent Tail for prior conversation outputs; projected assistant turns are summaries and require scheduler_context_hydrate when exact prior text matters; use tools for current file state, diagnostics, and verification evidence."
                .to_string(),
        ));
    }
    ledger
}

fn role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn command_question_for_field(
    command: &Command,
    field: &CommandArgumentField,
) -> agendao_tool::QuestionDef {
    let template = command.interactive.as_ref().and_then(|interactive| {
        interactive.questions.iter().find(|question| {
            normalize_command_field_key(&question.field_key)
                == normalize_command_field_key(&field.key)
        })
    });

    agendao_tool::QuestionDef {
        question: template
            .map(|question| question.prompt.clone())
            .unwrap_or_else(|| format!("Provide `{}` for `/{}`.", field.label, command.name)),
        header: template
            .map(|question| question.header.clone())
            .or_else(|| Some(field.label.clone())),
        options: template
            .map(|question| {
                question
                    .options
                    .iter()
                    .map(|option| agendao_tool::QuestionOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                field
                    .options
                    .iter()
                    .map(|option| agendao_tool::QuestionOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect()
            }),
        multiple: field.repeatable || matches!(field.kind, CommandArgumentKind::GlobList),
    }
}

async fn create_pending_command_question(
    state: &Arc<ServerState>,
    session_id: &str,
    command: &Command,
    raw_arguments: Option<&str>,
    missing_fields: &[CommandArgumentField],
) -> Result<String> {
    let questions = missing_fields
        .iter()
        .map(|field| command_question_for_field(command, field))
        .collect::<Vec<_>>();
    let (question_info, _) = state
        .runtime_telemetry
        .register_question(session_id.to_string(), questions.clone())
        .await;
    let mut sessions = state.sessions.lock().await;
    let Some(mut session) = sessions.get(session_id).cloned() else {
        return Err(ApiError::SessionNotFound(session_id.to_string()));
    };
    session.insert_metadata(
        "pending_command_invocation",
        serde_json::json!({
            "command": command.name,
            "rawArguments": raw_arguments.unwrap_or_default(),
            "missingFields": missing_fields.iter().map(|field| field.key.clone()).collect::<Vec<_>>(),
            "questionId": question_info.id.clone(),
        }),
    );
    sessions.update(session);

    Ok(question_info.id)
}

fn frontend_smoke_skip_execution_enabled() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::var("AGENDAO_FRONTEND_SMOKE_SKIP_EXECUTION")
            .ok()
            .as_deref()
            == Some("1")
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionPromptRequest {
    pub message: Option<String>,
    #[serde(default)]
    pub parts: Option<Vec<agendao_session::prompt::PartInput>>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub ingress_source: Option<String>,
    /// Canonical message origin (Operator, Scheduler, etc.).
    #[serde(default)]
    pub source_origin: Option<agendao_types::MessageSourceOrigin>,
    /// Which surface/transport the request arrived through.
    #[serde(default)]
    pub source_surface: Option<agendao_types::MessageSourceSurface>,
    pub model: Option<String>,
    pub variant: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    pub agent: Option<String>,
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    pub command: Option<String>,
    pub arguments: Option<String>,
    #[serde(default)]
    pub(super) recovery: Option<RecoveryExecutionContext>,
    /// Server-only marker for a synthetic continuation. The generation guard
    /// prevents a continuation queued for an old Goal from running after a
    /// user replaces the Ledger before the queue is adopted.
    #[serde(skip)]
    pub(super) auto_continuation_goal_generation: Option<u64>,
}

impl SessionPromptRequest {
    pub(super) fn from_command(
        command: String,
        arguments: Option<String>,
        model: Option<String>,
        variant: Option<String>,
        reasoning_effort: Option<String>,
        agent: Option<String>,
        scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    ) -> Self {
        Self {
            message: None,
            parts: None,
            idempotency_key: None,
            ingress_source: Some("web-command".to_string()),
            source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
            source_surface: Some(agendao_types::MessageSourceSurface::Web),
            model,
            variant,
            reasoning_effort,
            agent,
            scheduler,
            command: Some(command),
            arguments,
            recovery: None,
            auto_continuation_goal_generation: None,
        }
    }

    pub(crate) fn from_verified_ingress(
        ingress: &agendao_session::prompt::IngressTurnEnvelope,
    ) -> Self {
        Self {
            message: Some(ingress.user_intent_text.clone()),
            parts: None,
            idempotency_key: ingress.idempotency_key.clone(),
            ingress_source: None,
            source_origin: None,
            source_surface: None,
            model: None,
            variant: None,
            reasoning_effort: None,
            agent: None,
            scheduler: None,
            command: None,
            arguments: None,
            recovery: None,
            auto_continuation_goal_generation: None,
        }
    }
}

fn build_ingress_envelope(
    session_id: &str,
    source: agendao_session::prompt::IngressSource,
    text: &str,
    idempotency_key: Option<String>,
    context_key: Option<String>,
) -> agendao_session::prompt::IngressTurnEnvelope {
    let now = chrono::Utc::now().timestamp_millis();
    let turn_id = idempotency_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("ingress_{}", value.trim()))
        .unwrap_or_else(|| format!("ingress_{}", uuid::Uuid::new_v4().simple()));
    let mut envelope = agendao_session::prompt::IngressTurnEnvelope::new_text(
        session_id.to_string(),
        source,
        turn_id,
        now,
        text.to_string(),
    );
    envelope.context_key = context_key;
    envelope.idempotency_key = idempotency_key.filter(|value| !value.trim().is_empty());
    envelope.stabilization.policy =
        agendao_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY.to_string();
    envelope
}

fn task_ingress_for_prompt(
    session_id: &str,
    display_prompt_text: &str,
    req: &SessionPromptRequest,
    resolved_prompt: &ResolvedPromptPayload,
    verified_ingress: Option<agendao_session::prompt::IngressTurnEnvelope>,
) -> Result<agendao_session::prompt::IngressTurnEnvelope> {
    let mut ingress = if let Some(ingress) = verified_ingress {
        if ingress.session_id != session_id {
            return Err(ApiError::BadRequest(format!(
                "verified ingress session_id `{}` does not match route session `{}`",
                ingress.session_id, session_id
            )));
        }
        ingress
    } else {
        build_ingress_envelope(
            session_id,
            ingress_source_from_request(req.ingress_source.as_deref()),
            display_prompt_text,
            req.idempotency_key.clone(),
            Some("session_prompt".to_string()),
        )
    };

    if ingress.command.is_none() {
        ingress.command = resolved_prompt
            .command
            .as_ref()
            .map(|command| command.name.clone())
            .or_else(|| req.command.clone());
    }

    // Carry canonical source metadata from the request into the ingress
    // so downstream message construction can stamp it on the user message.
    if ingress.source_origin.is_none() {
        ingress.source_origin = req.source_origin;
    }
    if ingress.source_surface.is_none() {
        ingress.source_surface = req.source_surface;
    }

    Ok(ingress)
}

fn ingress_source_from_request(value: Option<&str>) -> agendao_session::prompt::IngressSource {
    agendao_session::prompt::normalize_ingress_source(value)
}

pub(super) struct SchedulerUserMessageContext<'a> {
    pub(super) display_prompt_text: &'a str,
    pub(super) resolved_user_prompt: &'a str,
    pub(super) choice: &'a agendao_orchestrator::selector::SchedulerChoice,
    pub(super) recovery: Option<&'a RecoveryExecutionContext>,
}

pub(super) async fn create_scheduler_user_message(
    prompt_runner: &agendao_session::SessionPrompt,
    session: &mut agendao_session::Session,
    input: &agendao_session::PromptInput,
    ctx: SchedulerUserMessageContext<'_>,
) -> Result<String> {
    prompt_runner
        .create_user_message(input, session)
        .await
        .map_err(|error| {
            ApiError::BadRequest(format!(
                "Failed to create scheduler user message: {}",
                error
            ))
        })?;

    let Some(user_message) = session
        .messages_mut()
        .iter_mut()
        .rfind(|message| matches!(message.role, agendao_session::MessageRole::User))
    else {
        return Err(ApiError::InternalError(
            "Scheduler prompt did not create a user message".to_string(),
        ));
    };

    if prompt_text_from_parts(&input.parts).trim().is_empty()
        && !ctx.display_prompt_text.trim().is_empty()
    {
        if let Some(agendao_session::PartType::Text { text, .. }) = user_message
            .parts
            .iter_mut()
            .find_map(|part| match &mut part.part_type {
                agendao_session::PartType::Text { .. } => Some(&mut part.part_type),
                _ => None,
            })
        {
            *text = ctx.display_prompt_text.to_string();
        }
    }

    user_message.metadata.insert(
        "scheduler".to_string(),
        serde_json::to_value(ctx.choice).map_err(|error| {
            ApiError::InternalError(format!("Failed to serialize scheduler choice: {error}"))
        })?,
    );
    user_message.metadata.insert(
        "resolved_user_prompt".to_string(),
        serde_json::json!(ctx.resolved_user_prompt),
    );

    if let Some(recovery) = ctx.recovery {
        if let Some(action) = recovery.action.as_ref() {
            user_message
                .metadata
                .insert("recovery_action".to_string(), serde_json::json!(action));
        }
        if let Some(revision) = recovery.ledger_revision {
            user_message.metadata.insert(
                "recovery_ledger_revision".to_string(),
                serde_json::json!(revision),
            );
            user_message.metadata.insert(
                "recovery_checkpoint_ids".to_string(),
                serde_json::json!(&recovery.checkpoint_ids),
            );
            user_message.metadata.insert(
                "recovery_open_ids".to_string(),
                serde_json::json!(&recovery.open_ids),
            );
            user_message.metadata.insert(
                "recovery_next_statement".to_string(),
                serde_json::json!(&recovery.next_statement),
            );
        }
    }

    Ok(user_message.id.clone())
}

fn annotate_last_user_message_multimodal_metadata(
    session: &mut agendao_session::Session,
    explain: &RuntimeMultimodalExplain,
) {
    let Some(user_message) = session
        .messages_mut()
        .iter_mut()
        .rfind(|message| matches!(message.role, agendao_session::MessageRole::User))
    else {
        return;
    };

    explain.persist_into_message_metadata(user_message);
}

pub(super) async fn session_prompt(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<SessionPromptRequest>,
) -> Result<Json<serde_json::Value>> {
    session_prompt_inner(state, headers, id, req, None).await
}

pub(crate) async fn session_prompt_with_verified_ingress(
    state: Arc<ServerState>,
    headers: HeaderMap,
    id: String,
    req: SessionPromptRequest,
    verified: VerifiedSessionIngress,
) -> Result<Json<serde_json::Value>> {
    session_prompt_inner(state, headers, id, req, Some(verified)).await
}

fn boxed_session_prompt_inner(
    state: Arc<ServerState>,
    headers: HeaderMap,
    id: String,
    req: SessionPromptRequest,
) -> Pin<Box<dyn Future<Output = Result<Json<serde_json::Value>>> + Send>> {
    Box::pin(session_prompt_inner(state, headers, id, req, None))
}

async fn commit_scheduler_input_and_start_ledger_run(
    state: &Arc<ServerState>,
    session_id: &str,
    session: &mut agendao_session::Session,
) {
    // The session-map value must contain the new user turn before a seam
    // mutates ledger metadata through that same authority. Refreshing from a
    // pre-prompt map value would erase the request recovery needs after abort.
    state.sessions.lock().await.update(session.clone());
    crate::session_runtime::task_ledger_reducer::dispatch_seam(
        state,
        session_id,
        agendao_types::task_ledger::TaskLedgerSeamFact::RunStarted,
    )
    .await;
    // The user turn is now authoritative. Persist one full snapshot at the
    // run boundary; later governance seams write only the session row until
    // the final assistant snapshot is flushed.
    persist_session_if_enabled(state, session_id).await;
    let sessions = state.sessions.lock().await;
    if let Some(fresh) = sessions.get(session_id) {
        *session = fresh.clone();
    }
}

async fn session_prompt_inner(
    state: Arc<ServerState>,
    headers: HeaderMap,
    id: String,
    mut req: SessionPromptRequest,
    verified_ingress: Option<VerifiedSessionIngress>,
) -> Result<Json<serde_json::Value>> {
    let is_auto_continuation_request = is_task_ledger_auto_continuation(&req);
    // 懒加载水合闸门：prompt 需要完整消息上下文，进入执行前先回填。
    state
        .ensure_session_messages_hydrated(&id)
        .await
        .map_err(|e| ApiError::InternalError(e.to_string()))?;
    if let Some(expected_generation) = req.auto_continuation_goal_generation {
        let (ledger, armed) = {
            let sessions = state.sessions.lock().await;
            let session = sessions
                .get(&id)
                .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
            (
                crate::session_runtime::task_ledger::ledger_snapshot_from_record(
                    &id,
                    session
                        .record()
                        .metadata
                        .get(crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY),
                ),
                load_task_ledger_auto_continuation_state(session)
                    .is_some_and(|state| state.goal_generation == expected_generation),
            )
        };
        if ledger.goal_generation != expected_generation
            || ledger.status != agendao_types::task_ledger::TaskLedgerStatus::Active
            || ledger.next.is_none()
            || !armed
        {
            return Ok(Json(serde_json::json!({
                "status": "superseded",
                "ok": true,
                "session_id": id,
                "reason": "task-ledger auto-continuation no longer matches the active goal",
            })));
        }
        req.message = Some(task_ledger_auto_resume_message(&ledger));
        req.recovery = Some(RecoveryExecutionContext::from_ledger(
            crate::recovery::RecoveryActionKind::Resume,
            &ledger,
        ));
    }
    super::super::permission::PERMISSION_ENGINE
        .lock()
        .await
        .clear_turn(&id);

    if req.agent.is_some() && req.scheduler.is_some() {
        return Err(ApiError::BadRequest(
            "`agent` and `scheduler` are mutually exclusive".to_string(),
        ));
    }
    if req.command.is_some() && req.parts.is_some() {
        return Err(ApiError::BadRequest(
            "`command` and `parts` are mutually exclusive".to_string(),
        ));
    }

    let request_parts = req.parts.clone().filter(|parts| !parts.is_empty());
    let display_prompt_text = if let Some(parts) = request_parts.as_ref() {
        prompt_display_text(parts)
    } else if let Some(message) = req.message.as_deref() {
        message.to_string()
    } else if let Some(command) = req.command.as_deref() {
        req.arguments
            .as_deref()
            .map(|args| format!("/{command} {args}"))
            .unwrap_or_else(|| format!("/{command}"))
    } else {
        return Err(ApiError::BadRequest(
            "Either `message`, `parts`, or `command` must be provided".to_string(),
        ));
    };

    let session_directory = {
        let sessions = state.sessions.lock().await;
        let Some(session) = sessions.get(&id) else {
            return Err(ApiError::SessionNotFound(id));
        };
        resolved_session_directory(session.record().directory.as_str(), &state.project_root())
    };
    let _ = ensure_plugin_loader_active(&state).await?;
    let config = if let Some(loader) = get_plugin_loader() {
        if should_apply_plugin_config_hooks(&headers) {
            let mut cfg = (*state.config_store.config()).clone();
            apply_plugin_config_hooks(loader, &mut cfg).await;
            state.config_store.set_plugin_applied(cfg.clone()).await;
            Arc::new(cfg)
        } else {
            // Internal request: use cached plugin-applied config snapshot so that
            // plugin-injected agent configs (model/prompt/permission) are available.
            state
                .config_store
                .plugin_applied()
                .await
                .unwrap_or_else(|| state.config_store.config())
        }
    } else {
        state.config_store.config()
    };

    let resolved_prompt = if let Some(parts) = request_parts.as_ref() {
        ResolvedPromptPayload {
            display_text: prompt_display_text(parts),
            execution_text: prompt_text_from_parts(parts),
            agent: None,
            model: None,
            scheduler: None,
            command: None,
            pending_raw_arguments: None,
        }
    } else {
        resolve_prompt_payload(&display_prompt_text, &id, &session_directory, &config).await?
    };
    let is_goal_command = resolved_prompt
        .command
        .as_ref()
        .is_some_and(|command| command.name == "goal");
    let goal_statement = command_goal_statement(&resolved_prompt);
    if is_goal_command && goal_statement.is_none() {
        return Err(ApiError::BadRequest(
            "Usage: /goal <goal description>".to_string(),
        ));
    }
    if let Some(command) = resolved_prompt.command.as_ref() {
        if let (Some(invocation), Some(interactive)) =
            (command.invocation.as_ref(), command.interactive.as_ref())
        {
            if interactive.when_missing_required != InteractivePolicy::None {
                let parsed_arguments = parse_command_argument_map(
                    resolved_prompt.pending_raw_arguments.as_deref(),
                    &invocation.argument_schema,
                );
                let mut missing_fields =
                    missing_required_command_fields(&invocation.argument_schema, &parsed_arguments);
                if interactive.when_missing_required == InteractivePolicy::AskPerStep {
                    missing_fields.truncate(1);
                }
                if !missing_fields.is_empty() {
                    let question_id = create_pending_command_question(
                        &state,
                        &id,
                        command,
                        resolved_prompt.pending_raw_arguments.as_deref(),
                        &missing_fields,
                    )
                    .await?;
                    broadcast_session_reconcile(
                        state.as_ref(),
                        id.clone(),
                        ReconcileReason::StatusChange,
                    )
                    .await;
                    persist_session_record_if_enabled(&state, &id).await;
                    return Ok(Json(serde_json::json!({
                        "status": "awaiting_user",
                        "session_id": id,
                        "pending_question_id": question_id,
                        "command": command.name,
                        "missing_fields": missing_fields
                            .iter()
                            .map(|field| field.key.clone())
                            .collect::<Vec<_>>(),
                    })));
                }
            }
        }
    }
    if frontend_smoke_skip_execution_enabled() {
        if let Some(statement) = goal_statement.as_deref() {
            crate::session_runtime::task_ledger::start_task_goal(&state, &id, statement).await?;
        }
        let mut pending_command_cleared = false;
        {
            let mut sessions = state.sessions.lock().await;
            if let Some(mut session) = sessions.get(&id).cloned() {
                pending_command_cleared = session
                    .remove_metadata("pending_command_invocation")
                    .is_some();
                if pending_command_cleared {
                    sessions.update(session);
                }
            }
        }
        if pending_command_cleared {
            broadcast_session_reconcile(state.as_ref(), id.clone(), ReconcileReason::StatusChange)
                .await;
        }
        broadcast_session_reconcile(state.as_ref(), id.clone(), ReconcileReason::StatusChange)
            .await;
        persist_session_record_if_enabled(&state, &id).await;
        return Ok(Json(serde_json::json!({
            "status": "accepted",
            "ok": true,
            "session_id": id,
            "smoke_skip_execution": true,
        })));
    }
    let prompt_text = resolved_prompt.execution_text.clone();
    let display_prompt_text = resolved_prompt.display_text.clone();
    let prompt_parts = if let Some(parts) = request_parts.clone() {
        parts
    } else {
        agendao_session::resolve_prompt_parts(&prompt_text, FsPath::new(&session_directory)).await
    };
    let effective_agent = resolved_prompt.agent.clone().or(req.agent.clone());
    let effective_scheduler = super::scheduler::resolve_effective_scheduler_choice(
        resolved_prompt.scheduler.clone(),
        req.scheduler.clone(),
        effective_agent.is_some(),
    );
    let request_config =
        resolve_prompt_request_config(super::scheduler::PromptRequestConfigInput {
            state: &state,
            config: &config,
            session_id: &id,
            requested_agent: effective_agent.as_deref(),
            requested_scheduler: &effective_scheduler,
            request_model: req.model.as_deref().or(resolved_prompt.model.as_deref()),
            request_variant: req.variant.as_deref(),
            request_reasoning_effort: parse_request_reasoning_effort(
                req.reasoning_effort.as_deref(),
            )?,
            route: "session",
        })
        .await?;
    let resolved_agent = request_config.resolved_agent.clone();
    let provider = request_config.provider.clone();
    let provider_id = request_config.provider_id.clone();
    let model_id = request_config.model_id.clone();
    let task_compiled_request = request_config.compiled_request.clone();
    let multimodal_explain = {
        let prompt_input_parts = prompt_parts_from_session_parts(&prompt_parts);
        let multimodal_parts = SessionPartAdapter::from_session_parts(&prompt_input_parts);
        if multimodal_parts.is_empty() {
            None
        } else {
            let authority = MultimodalAuthority::from_config(&config);
            let provider_model = resolve_provider_model(&state, &provider_id, &model_id).await?;
            let capability = authority
                .capability_authority()
                .capability_view(provider_id.clone(), &provider_model);
            let result = authority.capability_authority().preflight(
                &capability,
                &SessionPartAdapter::to_preflight_parts(&multimodal_parts),
            );
            let transport = authority.capability_authority().transport_explain(
                &capability,
                &provider_model,
                &prompt_input_parts,
            );
            if result.hard_block {
                return Err(ApiError::BadRequest(
                    result
                        .warnings
                        .first()
                        .cloned()
                        .or(result.recommended_downgrade.clone())
                        .unwrap_or_else(|| {
                            "Current multimodal policy blocked this input.".to_string()
                        }),
                ));
            }
            Some(RuntimeMultimodalExplain {
                summary: authority.build_display_summary(None, &multimodal_parts),
                capability,
                result,
                transport,
                resolved_model: format!("{}/{}", provider_id, model_id),
            })
        }
    };

    let task_state = state.clone();
    let session_id = id.clone();
    let task_variant = req.variant.clone();
    let task_reasoning_effort = req.reasoning_effort.clone();
    let task_agent = resolved_agent.as_ref().map(|agent| agent.name.clone());
    let task_model = model_id.clone();
    let task_provider_client = provider.clone();
    let task_provider = provider_id.clone();
    let task_scheduler_choice = request_config.scheduler_choice.clone();
    let task_recovery = req.recovery.clone();
    let task_prompt_parts = prompt_parts.clone();
    let task_multimodal_explain = multimodal_explain.clone();
    let task_verified_external_adapter_binding = verified_ingress
        .as_ref()
        .and_then(|verified| verified.external_adapter_binding.clone());
    let task_ingress = task_ingress_for_prompt(
        &session_id,
        &display_prompt_text,
        &req,
        &resolved_prompt,
        verified_ingress
            .as_ref()
            .map(|verified| verified.ingress.clone()),
    )?;
    let live_web_ingress_stage =
        stage_live_web_ingress_batch(&state, &session_id, &task_ingress, &task_prompt_parts)
            .await?;
    if matches!(live_web_ingress_stage, LiveWebIngressBatchStage::Follower) {
        return Ok(Json(serde_json::json!({
            "status": "accepted",
            "ok": true,
            "session_id": id,
            "model": format!("{}/{}", provider_id, model_id),
            "variant": req.variant,
            "command": resolved_prompt.command.as_ref().map(|command| command.name.clone()),
            "batched": true,
        })));
    }
    let auto_continuation_reservation =
        if matches!(live_web_ingress_stage, LiveWebIngressBatchStage::Bypass)
            && is_task_ledger_auto_continuation(&req)
            && !state.prompt_runner.is_running(&session_id).await
        {
            state
                .prompt_runner
                .reserve_session_run(&session_id)
                .await
                .ok()
        } else {
            None
        };
    if matches!(live_web_ingress_stage, LiveWebIngressBatchStage::Bypass)
        && state.prompt_runner.is_running(&session_id).await
        && auto_continuation_reservation.is_none()
    {
        let auto_continuation_goal_generation = req.auto_continuation_goal_generation;
        let queued_count = enqueue_followup_prompt(
            &state,
            &session_id,
            QueuedFollowupPrompt {
                request: req,
                apply_plugin_config_hooks: should_apply_plugin_config_hooks(&headers),
                auto_continuation_goal_generation,
            },
        )
        .await?;
        return Ok(Json(serde_json::json!({
            "status": "queued",
            "ok": true,
            "session_id": id,
            "queued_count": queued_count,
            "model": format!("{}/{}", provider_id, model_id),
            "variant": task_variant,
            "command": resolved_prompt.command.as_ref().map(|command| command.name.clone()),
        })));
    }

    // `/goal <plain language>` activates the server-authoritative TaskLedger
    // before the scheduler snapshots its context. The user never supplies a
    // revision, actor, timestamp or JSON payload.
    if let Some(statement) = goal_statement.as_deref() {
        crate::session_runtime::task_ledger::start_task_goal(&state, &id, statement).await?;
    }

    let mut pending_command_cleared = false;
    let mut persisted_external_adapter_binding = false;
    {
        let mut sessions = state.sessions.lock().await;
        if let Some(mut session) = sessions.get(&id).cloned() {
            if goal_statement.is_some() || !is_auto_continuation_request {
                clear_task_ledger_auto_continuation_state(&mut session);
            }
            pending_command_cleared = session
                .remove_metadata("pending_command_invocation")
                .is_some();
            if let Some(binding) = task_verified_external_adapter_binding.as_ref() {
                persist_verified_external_adapter_binding(&mut session, binding);
                persisted_external_adapter_binding = true;
            }
            sessions.update(session);
        }
    }
    if pending_command_cleared {
        broadcast_session_reconcile(state.as_ref(), id.clone(), ReconcileReason::StatusChange)
            .await;
    }
    if pending_command_cleared || persisted_external_adapter_binding {
        persist_session_record_if_enabled(&state, &id).await;
    }
    let output_block_hook: Option<agendao_session::prompt::OutputBlockHook> =
        Some(server_output_block_hook(task_state.clone()));
    let task_live_batch_owner_turn_id = match &live_web_ingress_stage {
        LiveWebIngressBatchStage::Leader { owner_turn_id, .. } => Some(owner_turn_id.clone()),
        _ => None,
    };
    let task_reserved_run = match live_web_ingress_stage {
        LiveWebIngressBatchStage::Leader { reservation, .. } => Some(reservation),
        _ => auto_continuation_reservation,
    };
    tokio::spawn(async move {
        let (mut session, effective_ingress, effective_parts) = if let Some(owner_turn_id) =
            task_live_batch_owner_turn_id.as_deref()
        {
            tokio::time::sleep(Duration::from_millis(
                LIVE_WEB_INGRESS_BATCH_WINDOW_MS as u64,
            ))
            .await;
            let drained = {
                let mut sessions = task_state.sessions.lock().await;
                match sessions.get(&session_id).cloned() {
                    Some(mut session) => {
                        let resolved = drain_live_web_ingress_batch(&mut session, owner_turn_id)
                            .and_then(resolve_live_web_ingress_batch)
                            .map(|(ingress, parts)| (session.clone(), ingress, parts));
                        sessions.update(session);
                        resolved
                    }
                    None => None,
                }
            };
            match drained {
                Some(values) => values,
                None => {
                    if task_reserved_run.is_some() {
                        task_state
                            .prompt_runner
                            .release_reserved_session_run(&session_id)
                            .await;
                    }
                    return;
                }
            }
        } else {
            let sessions = task_state.sessions.lock().await;
            let Some(session) = sessions.get(&session_id).cloned() else {
                return;
            };
            (session, task_ingress.clone(), task_prompt_parts.clone())
        };
        let normalized_directory = resolved_session_directory(
            session.record().directory.as_str(),
            &task_state.project_root(),
        );
        if session.record().directory != normalized_directory {
            session.set_directory(normalized_directory);
        }
        set_session_run_status(&task_state, &session_id, SessionRunStatus::Busy).await;

        // Safety guard: ensure status is always set to idle when this block
        // exits, mirroring the TS `defer(() => cancel(sessionID))` pattern.
        // This prevents the spinner from getting stuck if anything panics.
        let mut _idle_guard = IdleGuard {
            state: task_state.clone(),
            session_id: Some(session_id.clone()),
        };

        if let Some(variant) = task_variant.as_deref() {
            session.insert_metadata("model_variant", serde_json::json!(variant));
        } else {
            session.remove_metadata("model_variant");
        }
        if let Some(reasoning_effort) = task_reasoning_effort.as_deref() {
            if reasoning_effort.trim().is_empty() {
                session.remove_metadata("model_reasoning_effort");
            } else {
                session.insert_metadata(
                    "model_reasoning_effort",
                    serde_json::json!(reasoning_effort.trim()),
                );
            }
        }
        session.insert_metadata("model_provider", serde_json::json!(&task_provider));
        session.insert_metadata("model_id", serde_json::json!(&task_model));
        if let Some(agent) = task_agent.as_deref() {
            session.insert_metadata("agent", serde_json::json!(agent));
        } else {
            session.remove_metadata("agent");
        }
        session.insert_metadata(
            "scheduler",
            serde_json::to_value(&task_scheduler_choice).unwrap_or(serde_json::Value::Null),
        );
        apply_scheduler_selection_session_metadata(&mut session, &request_config);
        let (memory_frozen_snapshot_block, _memory_prefetch_packet, memory_prefetch_block) =
            resolve_prompt_memory_context(&task_state, &mut session, &prompt_text).await;
        let scheduler_session_context_packet = build_scheduler_session_context_packet(&session);
        let scheduler_execution_prompt = merge_scheduler_prompt_with_memory(
            &prompt_text,
            memory_frozen_snapshot_block.as_deref(),
            memory_prefetch_block.as_deref(),
        );

        {
            let choice = task_scheduler_choice.clone();
            let scheduler_input = agendao_session::PromptInput {
                session_id: session_id.clone(),
                message_id: None,
                model: None,
                agent: None,
                no_reply: false,
                system: None,
                variant: task_variant.clone(),
                parts: effective_parts,
                tools: None,
                ingress: Some(effective_ingress),
            };
            let user_message_id = match create_scheduler_user_message(
                task_state.prompt_runner.as_ref(),
                &mut session,
                &scheduler_input,
                SchedulerUserMessageContext {
                    display_prompt_text: &display_prompt_text,
                    resolved_user_prompt: &prompt_text,
                    choice: &choice,
                    recovery: task_recovery.as_ref(),
                },
            )
            .await
            {
                Ok(message_id) => message_id,
                Err(error) => {
                    let assistant = session.add_assistant_message();
                    assistant.finish = Some("error".to_string());
                    assistant.add_text(format!("Scheduler input error: {error}"));
                    task_state.sessions.lock().await.update(session);
                    if task_reserved_run.is_some() {
                        task_state
                            .prompt_runner
                            .release_reserved_session_run(&session_id)
                            .await;
                    }
                    return;
                }
            };
            if let Some(explain) = task_multimodal_explain.as_ref() {
                annotate_last_user_message_multimodal_metadata(&mut session, explain);
            }
            let mut conversation_seed =
                match agendao_session::prompt::replay_provider_messages(&session.messages) {
                    Ok(messages) => messages,
                    Err(error) => {
                        let assistant = session.add_assistant_message();
                        assistant.finish = Some("error".to_string());
                        assistant.add_text(format!("Scheduler history error: {error}"));
                        task_state.sessions.lock().await.update(session);
                        if task_reserved_run.is_some() {
                            task_state
                                .prompt_runner
                                .release_reserved_session_run(&session_id)
                                .await;
                        }
                        return;
                    }
                };
            // Web batching and synthetic Ledger continuations reserve the
            // session before scheduler registration. Reuse that reservation
            // token as the scheduler cancellation authority so aborts in the
            // boundary gap cannot be lost.
            let cancellation = task_reserved_run
                .as_ref()
                .cloned()
                .unwrap_or_else(CancellationToken::new);
            // Task-governance seam: run started. A resumed (previously
            // interrupted) ledger flips back to active here, before the
            // projection below snapshots the status for this run.
            commit_scheduler_input_and_start_ledger_run(&task_state, &session_id, &mut session)
                .await;
            {
                let sessions = task_state.sessions.lock().await;
                if let Some(session) = sessions.get(&session_id) {
                    if let Some(projection) =
                        crate::session_runtime::task_ledger_reducer::render_ledger_projection(
                            &crate::session_runtime::task_ledger::ledger_snapshot_from_record(
                                &session_id,
                                session.record().metadata.get(
                                    crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY,
                                ),
                            ),
                        )
                    {
                        // Single prompt injection point: the ledger rides the
                        // conversation seed as a typed context block.
                        conversation_seed.push(agendao_provider::Message {
                            role: agendao_provider::Role::User,
                            content: agendao_provider::Content::Text(projection),
                            cache_control: None,
                            provider_options: None,
                        });
                    }
                }
            }
            let assistant_message_id = session.add_assistant_message().id.clone();
            let directory = session.record().directory.clone();
            task_state.sessions.lock().await.update(session.clone());
            broadcast_session_reconcile(
                task_state.as_ref(),
                session_id.clone(),
                ReconcileReason::StatusChange,
            )
            .await;

            task_state
                .runtime_telemetry
                .register_scheduler_run(&session_id, cancellation.clone(), None)
                .await;
            let mut execution_metadata = std::collections::HashMap::from([
                (
                    "message_id".to_string(),
                    serde_json::json!(&assistant_message_id),
                ),
                (
                    "user_message_id".to_string(),
                    serde_json::json!(&user_message_id),
                ),
            ]);
            if let Some(packet) = scheduler_session_context_packet.as_ref() {
                execution_metadata.insert(
                    SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
                    packet.metadata_value(),
                );
            }
            let scheduler_result = crate::scheduler_runner::run_scheduler(
                crate::scheduler_runner::SchedulerRunInput {
                    state: task_state.clone(),
                    session_id: session_id.clone(),
                    assistant_message_id: assistant_message_id.clone(),
                    directory,
                    goal: scheduler_execution_prompt,
                    choice,
                    primary_agent: task_agent
                        .as_deref()
                        .map(agendao_orchestrator::blueprint::AgentId::new),
                    provider: task_provider_client.clone(),
                    request: task_compiled_request.clone(),
                    conversation_seed,
                    execution_metadata,
                    cancellation: cancellation.clone(),
                },
            )
            .await;
            session = task_state
                .sessions
                .lock()
                .await
                .get(&session_id)
                .cloned()
                .unwrap_or(session);
            let scheduler_review = scheduler_result.as_ref().ok().map(|output| {
                let mut nudge = agendao_session::prompt::RuntimeReviewNudge::from_session(
                    &session,
                    output.usage.model_calls as usize,
                );
                nudge.tool_call_count = output.review.tool_call_count;
                nudge.error_tool_call_count = output.review.error_tool_call_count;
                nudge.skill_write_count = output.review.skill_write_count;
                nudge.used_skill_names = output.review.used_skill_names.clone();
                nudge
            });
            let model_pricing = {
                let providers = task_state.providers.read().await;
                providers
                    .find_model(&task_model)
                    .map(|(_, info)| ModelPricing::from_model_info(&info))
            };
            let has_active_task_ledger = {
                let ledger = crate::session_runtime::task_ledger::ledger_snapshot_from_record(
                    &session_id,
                    session
                        .record()
                        .metadata
                        .get(crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY),
                );
                ledger.goal.is_some()
                    && ledger.status == agendao_types::task_ledger::TaskLedgerStatus::Active
                    && ledger.next.is_some()
            };
            if let Some(assistant) = session.get_message_mut(&assistant_message_id) {
                assistant.metadata.insert(
                    "model_provider".to_string(),
                    serde_json::json!(&task_provider),
                );
                assistant
                    .metadata
                    .insert("model_id".to_string(), serde_json::json!(&task_model));
                match scheduler_result {
                    Ok(output) => {
                        assistant.finish = Some("stop".to_string());
                        assistant.metadata.insert(
                            "scheduler_blueprint_fingerprint".to_string(),
                            serde_json::json!(output.fingerprint),
                        );
                        assistant.metadata.insert(
                            "scheduler_blueprint".to_string(),
                            serde_json::to_value(output.blueprint)
                                .unwrap_or(serde_json::Value::Null),
                        );
                        assistant.metadata.insert(
                            "scheduler_selection_source".to_string(),
                            serde_json::json!(crate::scheduler_runner::selection_source_name(
                                output.source
                            )),
                        );
                        assistant.metadata.insert(
                            "scheduler_model_calls".to_string(),
                            serde_json::json!(output.usage.model_calls),
                        );
                        assistant.metadata.insert(
                            "scheduler_tool_calls".to_string(),
                            serde_json::json!(output.usage.tool_calls),
                        );
                        let cost = model_pricing
                            .map(|pricing| {
                                pricing.compute(
                                    output.usage.input_tokens,
                                    output.usage.output_tokens,
                                    output.usage.cache_read_tokens,
                                    output.usage.cache_miss_tokens,
                                    output.usage.cache_write_tokens,
                                )
                            })
                            .unwrap_or(0.0);
                        assistant.usage = Some(agendao_session::MessageUsage {
                            input_tokens: output.usage.input_tokens,
                            output_tokens: output.usage.output_tokens,
                            reasoning_tokens: output.usage.reasoning_tokens,
                            cache_read_tokens: output.usage.cache_read_tokens,
                            cache_miss_tokens: output.usage.cache_miss_tokens,
                            cache_write_tokens: output.usage.cache_write_tokens,
                            context_tokens: output.usage.input_tokens,
                            total_cost: cost,
                        });
                        let text = output
                            .result
                            .output
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or(output.result.summary);
                        assistant.add_text(text);
                    }
                    Err(error) if cancellation.is_cancelled() => {
                        assistant.finish = Some("cancelled".to_string());
                        assistant
                            .metadata
                            .insert("finish_reason".to_string(), serde_json::json!("cancelled"));
                        assistant.add_text("Scheduler cancelled.");
                    }
                    Err(error) => {
                        assistant.finish = Some("error".to_string());
                        assistant
                            .metadata
                            .insert("error".to_string(), serde_json::json!(&error));
                        if error.contains("step limit") {
                            assistant
                                .metadata
                                .insert("scheduler_resumable".to_string(), serde_json::json!(true));
                            let completed = assistant
                                .metadata
                                .get("scheduler_last_completed_step")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0);
                            let maximum = assistant
                                .metadata
                                .get("scheduler_max_steps")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(completed);
                            let continuation = if has_active_task_ledger {
                                "The active TaskLedger goal will automatically start a new scheduler turn from this session's saved context."
                            } else {
                                "Send `continue` to start a new scheduler turn from this session's saved context."
                            };
                            assistant.add_text(format!(
                                "Scheduler paused after step {completed}/{maximum}: {error}\n\n\
                                 The completed steps and tool results above are preserved. {continuation}"
                            ));
                        } else {
                            assistant.add_text(format!("Scheduler error: {error}"));
                        }
                    }
                }
            }
            if let Some(nudge) = scheduler_review.as_ref() {
                let decision = task_state
                    .prompt_runner
                    .maybe_enqueue_background_review(nudge)
                    .await;
                agendao_session::prompt::maybe_append_proposal_notice(&mut session, &decision);
            }
            ensure_default_session_title(&mut session, task_provider_client.clone(), &task_model)
                .await;
            let assistant_text = session
                .get_message(&assistant_message_id)
                .map(assistant_visible_text)
                .unwrap_or_default();
            let session_usage = session.get_usage();
            let _ = task_state
                .runtime_telemetry
                .record_session_usage(&session_id, Some(&assistant_message_id), session_usage)
                .await;
            persist_session_telemetry_metadata(&task_state, &mut session).await;
            task_state.sessions.lock().await.update(session);

            // Task-governance seams at run end run only after the final
            // session update so nothing later overwrites the gate report.
            // ToolBatchCompleted is dispatched by the scheduler observer at
            // each step boundary and must not be replayed here: the run-end
            // seams below own only verify/final-commit/interrupt reporting.
            {
                // Keep the scheduler cancellation token registered until the
                // final assistant message and scheduler step seams are authoritative.
                // Only then verify the workspace and earn completion.
                let verification =
                    crate::session_runtime::task_ledger_reducer::verify_goal_criteria(
                        &task_state,
                        &session_id,
                        cancellation.clone(),
                    )
                    .await;
                if verification.allows_final_commit() && !cancellation.is_cancelled() {
                    crate::session_runtime::task_ledger_reducer::dispatch_seam(
                        &task_state,
                        &session_id,
                        agendao_types::task_ledger::TaskLedgerSeamFact::FinalResponseCommitted,
                    )
                    .await;
                }
                let ledger_snapshot = crate::session_runtime::task_ledger::task_ledger_snapshot(
                    &task_state,
                    &session_id,
                )
                .await
                .unwrap_or_else(|_| {
                    agendao_types::task_ledger::SessionTaskLedger::empty(&session_id)
                });
                if cancellation.is_cancelled() {
                    crate::session_runtime::task_ledger_reducer::dispatch_seam(
                        &task_state,
                        &session_id,
                        agendao_types::task_ledger::TaskLedgerSeamFact::RecoveryInterrupted,
                    )
                    .await;
                } else if ledger_snapshot.revision > 0 {
                    // Typed final-delivery gate: report, never rewrite.
                    let report = crate::session_runtime::task_ledger_reducer::final_delivery_gate(
                        &ledger_snapshot,
                    );
                    let mut sessions = task_state.sessions.lock().await;
                    if let Some(record) = sessions.get_mut(&session_id) {
                        record.insert_metadata(
                            "delivery_gate_report".to_string(),
                            serde_json::json!({
                                "open_questions": report.open_questions_outstanding,
                                "no_verified_checkpoints": report.no_verified_checkpoints,
                                "missing_acceptance_criteria": report.missing_acceptance_criteria,
                                "uncovered_criteria": report.uncovered_criteria,
                                "checked_at": chrono::Utc::now().timestamp_millis(),
                            }),
                        );
                    }
                }
            }

            // Decide and persist the next Ledger boundary while this run's
            // cancellation authority is still registered. An abort racing
            // the boundary can then cancel the run or clear the persisted
            // continuation marker before the synthetic request validates its
            // goal generation/status.
            let pending_auto_continuation = if cancellation.is_cancelled() {
                None
            } else {
                prepare_task_ledger_auto_continuation(
                    &task_state,
                    &session_id,
                    &task_provider,
                    &task_model,
                    task_variant.clone(),
                    false,
                )
                .await
            };

            // The response, scheduler step seams, verifier, completion/interrupt seam,
            // and delivery report are now settled. Retire the cancellation
            // authority only after that full run lifecycle has closed.
            task_state
                .runtime_telemetry
                .finish_scheduler_run(&session_id)
                .await;
            if task_reserved_run.is_some() {
                task_state
                    .prompt_runner
                    .release_reserved_session_run(&session_id)
                    .await;
            }

            broadcast_session_reconcile(
                task_state.as_ref(),
                session_id.clone(),
                ReconcileReason::StatusChange,
            )
            .await;
            if !assistant_text.trim().is_empty() {
                emit_output_block_via_hook(
                    output_block_hook.as_ref(),
                    agendao_session::prompt::OutputBlockEvent {
                        session_id: session_id.clone(),
                        block: OutputBlock::Message(MessageBlock::full(
                            OutputMessageRole::Assistant,
                            assistant_text,
                        )),
                        id: Some(assistant_message_id.clone()),
                        live_identity: Some(assistant_text_live_identity(
                            &assistant_message_id,
                            LivePartPhase::Snapshot,
                        )),
                    },
                )
                .await;
            }
            persist_session_if_enabled(&task_state, &session_id).await;
            _idle_guard.defuse();
            set_session_run_status(&task_state, &session_id, SessionRunStatus::Idle).await;
            // A cancelled run must not silently execute prompts the user
            // queued behind it; abort drains the queue, and a race that
            // re-queues after cancel is still skipped here.
            if !cancellation.is_cancelled() {
                if let Some(queued) = take_followup_prompt(&task_state, &session_id).await {
                    let state = task_state.clone();
                    let session_id_for_followup = session_id.clone();
                    tokio::spawn(async move {
                        state
                            .runtime_telemetry
                            .emit_control_input_transition(
                                &session_id_for_followup,
                                ControlInputKind::Followup,
                                ControlInputPhase::Consumed,
                                chrono::Utc::now().timestamp_millis(),
                            )
                            .await;
                        let headers = if queued.apply_plugin_config_hooks {
                            HeaderMap::new()
                        } else {
                            internal_prompt_headers()
                        };
                        if let Err(error) = boxed_session_prompt_inner(
                            state.clone(),
                            headers,
                            session_id_for_followup.clone(),
                            queued.request,
                        )
                        .await
                        {
                            tracing::error!(session_id = %session_id_for_followup, %error, "failed to adopt queued follow-up prompt");
                        }
                    });
                } else if let Some(auto) = pending_auto_continuation {
                    // The Ledger is the authority for whether another turn
                    // is needed. Route the synthetic request through the
                    // same ingress boundary as a user prompt so the normal
                    // permission, scheduler and cancellation paths remain in
                    // force. If a human prompt wins the race, this request
                    // is queued behind it and the next run adopts it.
                    if let Err(error) =
                        enqueue_followup_prompt(&task_state, &session_id, auto).await
                    {
                        tracing::error!(session_id = %session_id, %error, "failed to queue TaskLedger automatic continuation");
                    } else if let Some(queued) =
                        take_followup_prompt(&task_state, &session_id).await
                    {
                        let state = task_state.clone();
                        let session_id_for_auto = session_id.clone();
                        tokio::spawn(async move {
                            let headers = if queued.apply_plugin_config_hooks {
                                HeaderMap::new()
                            } else {
                                internal_prompt_headers()
                            };
                            if let Err(error) = boxed_session_prompt_inner(
                                state,
                                headers,
                                session_id_for_auto.clone(),
                                queued.request,
                            )
                            .await
                            {
                                tracing::error!(
                                    session_id = %session_id_for_auto,
                                    %error,
                                    "failed to start queued prompt at TaskLedger continuation boundary"
                                );
                            }
                        });
                    }
                }
            }
        }
    });

    Ok(Json(serde_json::json!({
        "status": "accepted",
        "ok": true,
        "session_id": id,
        "model": format!("{}/{}", provider_id, model_id),
        "variant": req.variant,
        "command": resolved_prompt.command.as_ref().map(|command| command.name.clone()),
    })))
}

#[cfg(test)]
#[path = "prompt/tests.rs"]
mod tests;
