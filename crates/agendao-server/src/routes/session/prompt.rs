use std::path::{Path as FsPath, PathBuf};
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
    persist_session_if_enabled, resolved_session_directory, set_session_run_status, IdleGuard,
};
use super::telemetry::persist_session_telemetry_metadata;

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

const LIVE_WEB_INGRESS_BATCH_METADATA_KEY: &str = "live_web_ingress_batch";
const LIVE_WEB_INGRESS_BATCH_WINDOW_MS: i64 = 250;
pub(crate) const VERIFIED_EXTERNAL_ADAPTER_BINDING_METADATA_KEY: &str =
    "verified_external_adapter_binding";

#[derive(Debug, Clone)]
pub(crate) struct VerifiedSessionIngress {
    pub ingress: agendao_session::prompt::IngressTurnEnvelope,
    pub external_adapter_binding: Option<ExternalAdapterResolvedBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveWebIngressBatch {
    owner_turn_id: String,
    opened_at_ms: i64,
    items: Vec<LiveWebIngressBatchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveWebIngressBatchItem {
    ingress: agendao_session::prompt::IngressTurnEnvelope,
    parts: Vec<agendao_session::prompt::PartInput>,
}

enum LiveWebIngressBatchStage {
    Bypass,
    Leader {
        owner_turn_id: String,
        reservation: CancellationToken,
    },
    Follower,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedFollowupPrompt {
    request: SessionPromptRequest,
    apply_plugin_config_hooks: bool,
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
        guard.get_mut(session_id)?.pop_front()?
    };
    let queued = match serde_json::from_value(value) {
        Ok(queued) => queued,
        Err(error) => {
            tracing::warn!(session_id, %error, "failed to decode queued follow-up prompt");
            return None;
        }
    };
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
        matches!(
            invocation.mode,
            agendao_command::CommandExecutionMode::Scheduler
        )
        .then(|| {
            let template = if command.name.starts_with("autoresearch") {
                agendao_orchestrator::templates::TemplateId::Autoresearch
            } else {
                agendao_orchestrator::templates::TemplateId::Direct
            };
            agendao_orchestrator::selector::SchedulerChoice::Template { template }
        })
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

fn normalize_command_field_key(key: &str) -> String {
    key.trim()
        .trim_start_matches('-')
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn tokenize_command_arguments(raw_arguments: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;

    for ch in raw_arguments.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }

        match ch {
            '\\' => escape = true,
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            _ if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn shell_quote_command_value(value: &str) -> String {
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

fn parse_command_argument_map(
    raw_arguments: Option<&str>,
    fields: &[CommandArgumentField],
) -> std::collections::HashMap<String, Vec<String>> {
    let mut values = std::collections::HashMap::<String, Vec<String>>::new();
    let Some(raw_arguments) = raw_arguments.filter(|value| !value.trim().is_empty()) else {
        return values;
    };

    let field_map = fields
        .iter()
        .map(|field| (normalize_command_field_key(&field.key), field))
        .collect::<std::collections::HashMap<_, _>>();
    let tokens = tokenize_command_arguments(raw_arguments);
    let mut index = 0;

    while index < tokens.len() {
        let token = &tokens[index];
        if !token.starts_with("--") {
            index += 1;
            continue;
        }

        let key = normalize_command_field_key(token.trim_start_matches("--"));
        let Some(field) = field_map.get(&key) else {
            index += 1;
            continue;
        };

        let mut captured = Vec::new();
        let mut cursor = index + 1;

        while cursor < tokens.len() && !tokens[cursor].starts_with("--") {
            captured.push(tokens[cursor].clone());
            cursor += 1;
            if !field.repeatable && !matches!(field.kind, CommandArgumentKind::GlobList) {
                break;
            }
        }

        if matches!(field.kind, CommandArgumentKind::Boolean) && captured.is_empty() {
            captured.push("true".to_string());
        }

        if !captured.is_empty() {
            values.entry(key).or_default().extend(captured);
        }
        index = cursor.max(index + 1);
    }

    values
}

fn flatten_argument_values(
    fields: &[CommandArgumentField],
    arguments: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut result = Vec::new();
    for field in fields {
        let key = normalize_command_field_key(&field.key);
        if let Some(values) = arguments.get(&key) {
            result.extend(values.iter().cloned());
        }
    }
    result
}

fn build_raw_arguments_from_map(
    fields: &[CommandArgumentField],
    arguments: &std::collections::HashMap<String, Vec<String>>,
) -> String {
    let mut parts = Vec::new();

    for field in fields {
        let key = normalize_command_field_key(&field.key);
        let Some(values) = arguments.get(&key) else {
            continue;
        };
        let values = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if values.is_empty() {
            continue;
        }
        parts.push(format!("--{}", field.key));
        parts.extend(values.into_iter().map(shell_quote_command_value));
    }

    parts.join(" ")
}

fn hydrate_scheduler_command_arguments(
    raw_arguments: &str,
    fields: &[CommandArgumentField],
) -> Result<(std::collections::HashMap<String, Vec<String>>, String)> {
    let parsed_arguments = parse_command_argument_map(Some(raw_arguments), fields);
    let hydrated_raw = build_raw_arguments_from_map(fields, &parsed_arguments);
    Ok((parsed_arguments, hydrated_raw))
}

fn missing_required_command_fields(
    fields: &[CommandArgumentField],
    parsed_arguments: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<CommandArgumentField> {
    fields
        .iter()
        .filter(|field| field.required)
        .filter(|field| {
            let key = normalize_command_field_key(&field.key);
            parsed_arguments
                .get(&key)
                .is_none_or(|values| values.iter().all(|value| value.trim().is_empty()))
        })
        .cloned()
        .collect()
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
    pub agent: Option<String>,
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    pub command: Option<String>,
    pub arguments: Option<String>,
    #[serde(default)]
    pub(super) recovery: Option<RecoveryExecutionContext>,
}

impl SessionPromptRequest {
    pub(super) fn from_command(
        command: String,
        arguments: Option<String>,
        model: Option<String>,
        variant: Option<String>,
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
            agent,
            scheduler,
            command: Some(command),
            arguments,
            recovery: None,
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
            agent: None,
            scheduler: None,
            command: None,
            arguments: None,
            recovery: None,
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

fn supports_live_web_ingress_batch(ingress: &agendao_session::prompt::IngressTurnEnvelope) -> bool {
    matches!(ingress.source, agendao_session::prompt::IngressSource::Web)
        && ingress.context_key.as_deref() == Some("session_prompt")
        && ingress.command.is_none()
}

fn load_live_web_ingress_batch(session: &agendao_session::Session) -> Option<LiveWebIngressBatch> {
    session
        .metadata
        .get(LIVE_WEB_INGRESS_BATCH_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn store_live_web_ingress_batch(
    session: &mut agendao_session::Session,
    batch: &LiveWebIngressBatch,
) -> bool {
    match serde_json::to_value(batch) {
        Ok(value) => {
            session.insert_metadata(LIVE_WEB_INGRESS_BATCH_METADATA_KEY.to_string(), value);
            true
        }
        Err(error) => {
            tracing::warn!(%error, "failed to serialize live web ingress batch");
            false
        }
    }
}

fn clear_live_web_ingress_batch(session: &mut agendao_session::Session) {
    session.remove_metadata(LIVE_WEB_INGRESS_BATCH_METADATA_KEY);
}

fn stale_live_web_ingress_batch(batch: &LiveWebIngressBatch, now_ms: i64) -> bool {
    now_ms.saturating_sub(batch.opened_at_ms) > LIVE_WEB_INGRESS_BATCH_WINDOW_MS
}

fn matching_live_web_ingress_batch(
    batch: &LiveWebIngressBatch,
    ingress: &agendao_session::prompt::IngressTurnEnvelope,
) -> bool {
    batch
        .items
        .first()
        .map(|first| {
            first.ingress.session_id == ingress.session_id
                && first.ingress.source == ingress.source
                && first.ingress.context_key == ingress.context_key
                && first.ingress.command == ingress.command
        })
        .unwrap_or(false)
}

fn append_live_web_ingress_batch_if_present(
    session: &mut agendao_session::Session,
    ingress: agendao_session::prompt::IngressTurnEnvelope,
    parts: Vec<agendao_session::prompt::PartInput>,
    now_ms: i64,
) -> bool {
    if !supports_live_web_ingress_batch(&ingress) {
        return false;
    }

    let item = LiveWebIngressBatchItem { ingress, parts };
    let batch = load_live_web_ingress_batch(session)
        .filter(|batch| !stale_live_web_ingress_batch(batch, now_ms));
    if batch.is_none() {
        clear_live_web_ingress_batch(session);
    }

    if let Some(mut batch) = batch {
        if matching_live_web_ingress_batch(&batch, &item.ingress) {
            batch.items.push(item);
            return store_live_web_ingress_batch(session, &batch);
        }
        clear_live_web_ingress_batch(session);
    }

    false
}

fn open_live_web_ingress_batch(
    session: &mut agendao_session::Session,
    ingress: agendao_session::prompt::IngressTurnEnvelope,
    parts: Vec<agendao_session::prompt::PartInput>,
    now_ms: i64,
) -> Option<String> {
    if !supports_live_web_ingress_batch(&ingress) {
        return None;
    }

    let item = LiveWebIngressBatchItem { ingress, parts };
    clear_live_web_ingress_batch(session);

    let owner_turn_id = item.ingress.turn_id.clone();
    let batch = LiveWebIngressBatch {
        owner_turn_id: owner_turn_id.clone(),
        opened_at_ms: now_ms,
        items: vec![item],
    };
    if store_live_web_ingress_batch(session, &batch) {
        Some(owner_turn_id)
    } else {
        None
    }
}

fn drain_live_web_ingress_batch(
    session: &mut agendao_session::Session,
    owner_turn_id: &str,
) -> Option<LiveWebIngressBatch> {
    let batch = load_live_web_ingress_batch(session)?;
    if batch.owner_turn_id != owner_turn_id {
        return None;
    }
    clear_live_web_ingress_batch(session);
    Some(batch)
}

fn resolve_live_web_ingress_batch(
    batch: LiveWebIngressBatch,
) -> Option<(
    agendao_session::prompt::IngressTurnEnvelope,
    Vec<agendao_session::prompt::PartInput>,
)> {
    let mut items = batch.items;
    items.sort_by(|left, right| {
        left.ingress
            .received_at_ms
            .cmp(&right.ingress.received_at_ms)
            .then_with(|| left.ingress.turn_id.cmp(&right.ingress.turn_id))
    });

    // `stabilize_ingress_turns()` only owns ingress-local merge semantics
    // (shadow text, metadata, dedupe markers). Authoritative prompt content is
    // rebuilt from `PartInput` below, not from `user_intent_text`.
    let stabilized = agendao_session::prompt::stabilize_ingress_turns(
        items.iter().map(|item| item.ingress.clone()).collect(),
    );
    if stabilized.len() != 1 {
        tracing::warn!(
            item_count = items.len(),
            stabilized_count = stabilized.len(),
            "live web ingress batch did not stabilize to a single turn"
        );
        return None;
    }

    let mut seen_idempotency_keys = std::collections::HashSet::new();
    let mut merged_parts = Vec::new();
    for item in items {
        let duplicate = item
            .ingress
            .idempotency_key
            .as_deref()
            .map(|key| {
                let scoped = format!(
                    "{}:{:?}:{}",
                    item.ingress.session_id, item.ingress.source, key
                );
                !seen_idempotency_keys.insert(scoped)
            })
            .unwrap_or(false);
        if duplicate {
            continue;
        }
        merged_parts.extend(item.parts);
    }

    stabilized
        .into_iter()
        .next()
        .map(|ingress| (ingress, merged_parts))
}

async fn stage_live_web_ingress_batch(
    state: &Arc<ServerState>,
    session_id: &str,
    ingress: &agendao_session::prompt::IngressTurnEnvelope,
    parts: &[agendao_session::prompt::PartInput],
) -> Result<LiveWebIngressBatchStage> {
    if !supports_live_web_ingress_batch(ingress) {
        return Ok(LiveWebIngressBatchStage::Bypass);
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    {
        let mut sessions = state.sessions.lock().await;
        let Some(mut session) = sessions.get(session_id).cloned() else {
            return Err(ApiError::SessionNotFound(session_id.to_string()));
        };
        if append_live_web_ingress_batch_if_present(
            &mut session,
            ingress.clone(),
            parts.to_vec(),
            now_ms,
        ) {
            sessions.update(session);
            return Ok(LiveWebIngressBatchStage::Follower);
        }
        sessions.update(session);
    }

    let reservation = match state.prompt_runner.reserve_session_run(session_id).await {
        Ok(token) => token,
        Err(error) => {
            let mut sessions = state.sessions.lock().await;
            let Some(mut session) = sessions.get(session_id).cloned() else {
                return Err(ApiError::SessionNotFound(session_id.to_string()));
            };
            if append_live_web_ingress_batch_if_present(
                &mut session,
                ingress.clone(),
                parts.to_vec(),
                now_ms,
            ) {
                sessions.update(session);
                return Ok(LiveWebIngressBatchStage::Follower);
            }
            return Err(ApiError::BadRequest(error.to_string()));
        }
    };

    let mut sessions = state.sessions.lock().await;
    let Some(mut session) = sessions.get(session_id).cloned() else {
        drop(sessions);
        state
            .prompt_runner
            .release_reserved_session_run(session_id)
            .await;
        return Err(ApiError::SessionNotFound(session_id.to_string()));
    };

    if append_live_web_ingress_batch_if_present(
        &mut session,
        ingress.clone(),
        parts.to_vec(),
        now_ms,
    ) {
        sessions.update(session);
        drop(sessions);
        state
            .prompt_runner
            .release_reserved_session_run(session_id)
            .await;
        return Ok(LiveWebIngressBatchStage::Follower);
    }

    let Some(owner_turn_id) =
        open_live_web_ingress_batch(&mut session, ingress.clone(), parts.to_vec(), now_ms)
    else {
        sessions.update(session);
        drop(sessions);
        state
            .prompt_runner
            .release_reserved_session_run(session_id)
            .await;
        return Ok(LiveWebIngressBatchStage::Bypass);
    };

    sessions.update(session);
    Ok(LiveWebIngressBatchStage::Leader {
        owner_turn_id,
        reservation,
    })
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
    let sessions = state.sessions.lock().await;
    if let Some(fresh) = sessions.get(session_id) {
        *session = fresh.clone();
    }
}

async fn session_prompt_inner(
    state: Arc<ServerState>,
    headers: HeaderMap,
    id: String,
    req: SessionPromptRequest,
    verified_ingress: Option<VerifiedSessionIngress>,
) -> Result<Json<serde_json::Value>> {
    // 懒加载水合闸门：prompt 需要完整消息上下文，进入执行前先回填。
    state
        .ensure_session_messages_hydrated(&id)
        .await
        .map_err(|e| ApiError::InternalError(e.to_string()))?;
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
                    persist_session_if_enabled(&state, &id).await;
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
        persist_session_if_enabled(&state, &id).await;
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
    if matches!(live_web_ingress_stage, LiveWebIngressBatchStage::Bypass)
        && state.prompt_runner.is_running(&session_id).await
    {
        let queued_count = enqueue_followup_prompt(
            &state,
            &session_id,
            QueuedFollowupPrompt {
                request: req,
                apply_plugin_config_hooks: should_apply_plugin_config_hooks(&headers),
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
    let mut pending_command_cleared = false;
    let mut persisted_external_adapter_binding = false;
    {
        let mut sessions = state.sessions.lock().await;
        if let Some(mut session) = sessions.get(&id).cloned() {
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
        persist_session_if_enabled(&state, &id).await;
    }
    if persisted_external_adapter_binding {
        persist_session_if_enabled(&state, &id).await;
    }
    let output_block_hook: Option<agendao_session::prompt::OutputBlockHook> =
        Some(server_output_block_hook(task_state.clone()));
    let task_live_batch_owner_turn_id = match &live_web_ingress_stage {
        LiveWebIngressBatchStage::Leader { owner_turn_id, .. } => Some(owner_turn_id.clone()),
        _ => None,
    };
    let task_reserved_run = match live_web_ingress_stage {
        LiveWebIngressBatchStage::Leader { reservation, .. } => Some(reservation),
        _ => None,
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
        if let Some(recovery) = task_recovery.as_ref() {
            if let Some(action) = recovery.action.as_ref() {
                session.insert_metadata("last_recovery_action", serde_json::json!(action));
            }
        }

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

            let cancellation = CancellationToken::new();
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
                        assistant.add_text(format!("Scheduler error: {error}"));
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

            // Task-governance seams at run end — run against the CURRENT
            // session record (the scheduler wrote through the map during the
            // run; the local above is stale for metadata), and only after the
            // final update so nothing later overwrites the gate report.
            {
                let last_batch = {
                    let sessions = task_state.sessions.lock().await;
                    sessions.get(&session_id).and_then(|record| {
                        record
                            .record()
                            .metadata
                            .get("latest_tool_batch_summary")
                            .cloned()
                    })
                };
                if let Some(value) = last_batch {
                    if let Ok(summary) =
                        serde_json::from_value::<agendao_types::repair::ToolBatchSummary>(value)
                    {
                        crate::session_runtime::task_ledger_reducer::dispatch_seam(
                            &task_state,
                            &session_id,
                            agendao_types::task_ledger::TaskLedgerSeamFact::ToolBatchCompleted {
                                summary,
                            },
                        )
                        .await;
                    }
                }
                // Keep the scheduler cancellation token registered until the
                // final assistant message and last batch are authoritative.
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

            // The response, final batch, verifier, completion/interrupt seam,
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
                    let handle = tokio::runtime::Handle::current();
                    tokio::task::spawn_blocking(move || {
                        handle.block_on(async move {
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
                            if let Err(error) = session_prompt_inner(
                                state.clone(),
                                headers,
                                session_id_for_followup.clone(),
                                queued.request,
                                None,
                            )
                            .await
                            {
                                tracing::error!(session_id = %session_id_for_followup, %error, "failed to adopt queued follow-up prompt");
                            }
                        });
                    });
                }
            }
            if let Err(error) = task_state.flush_session_to_storage(&session_id).await {
                tracing::error!(session_id = %session_id, %error, "failed to flush session to storage");
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
mod tests {
    use super::*;
    use agendao_command::{CommandArgumentOption, CommandRegistry, CommandSource};
    use agendao_multimodal::{
        ModalityPreflightResult, ModalitySupportView, ModalityTransportResult,
        MultimodalDisplaySummary, PreflightCapabilityView, RuntimeMultimodalExplain,
    };
    use agendao_orchestrator::output_projection::{
        SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY, SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY,
        SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY,
    };
    use agendao_session::{IngressSource, PartType, Session, SessionStateManager};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn test_prompt_runner() -> agendao_session::SessionPrompt {
        agendao_session::SessionPrompt::new(Arc::new(RwLock::new(SessionStateManager::new())))
    }

    fn text_parts(message: &agendao_session::SessionMessage) -> Vec<&str> {
        message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn run_started_refresh_preserves_the_new_user_message_for_recovery() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            agendao_types::task_ledger::TaskLedgerOp::Create {
                goal: agendao_types::task_ledger::TaskGoal {
                    statement: "finish after recovery".to_string(),
                    acceptance_criteria: vec![],
                    criterion_checks: vec![],
                    set_by: agendao_types::task_ledger::TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "run the task".to_string(),
            },
        )
        .await
        .expect("create ledger");
        crate::session_runtime::task_ledger_reducer::dispatch_seam(
            &state,
            &session_id,
            agendao_types::task_ledger::TaskLedgerSeamFact::RecoveryInterrupted,
        )
        .await;

        let mut local = state
            .sessions
            .lock()
            .await
            .get(&session_id)
            .expect("session")
            .clone();
        local.add_user_message("original request that must survive abort");

        commit_scheduler_input_and_start_ledger_run(&state, &session_id, &mut local).await;

        let stored = state
            .sessions
            .lock()
            .await
            .get(&session_id)
            .expect("session")
            .clone();
        assert_eq!(
            crate::recovery::latest_user_prompt(&stored).as_deref(),
            Some("original request that must survive abort")
        );
        let ledger = crate::session_runtime::task_ledger::ledger_snapshot_from_record(
            &session_id,
            stored
                .record()
                .metadata
                .get(crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY),
        );
        assert_eq!(
            ledger.status,
            agendao_types::task_ledger::TaskLedgerStatus::Active
        );
        assert!(local
            .messages
            .iter()
            .any(|message| message.get_text() == "original request that must survive abort"));
    }

    #[test]
    fn scheduler_choice_defaults_to_auto_without_an_explicit_agent() {
        assert_eq!(
            super::super::scheduler::resolve_effective_scheduler_choice(None, None, false),
            agendao_orchestrator::selector::SchedulerChoice::Auto
        );
    }

    #[test]
    fn explicit_agent_uses_direct_scheduler_and_explicit_scheduler_is_preserved() {
        assert_eq!(
            super::super::scheduler::resolve_effective_scheduler_choice(None, None, true),
            agendao_orchestrator::selector::SchedulerChoice::Template {
                template: agendao_orchestrator::templates::TemplateId::Direct,
            }
        );

        let explicit = agendao_orchestrator::selector::SchedulerChoice::Template {
            template: agendao_orchestrator::templates::TemplateId::Verify,
        };
        assert_eq!(
            super::super::scheduler::resolve_effective_scheduler_choice(
                None,
                Some(explicit.clone()),
                false,
            ),
            explicit
        );
    }

    #[test]
    fn session_prompt_ingress_source_defaults_to_api_and_preserves_known_sources() {
        use agendao_session::prompt::IngressSource;

        assert_eq!(ingress_source_from_request(None), IngressSource::Api);
        assert_eq!(ingress_source_from_request(Some("")), IngressSource::Api);
        assert_eq!(ingress_source_from_request(Some("cli")), IngressSource::Cli);
        assert_eq!(ingress_source_from_request(Some("TUI")), IngressSource::Tui);
        assert_eq!(ingress_source_from_request(Some("web")), IngressSource::Web);
        assert_eq!(
            ingress_source_from_request(Some("scheduler")),
            IngressSource::Scheduler
        );
        assert_eq!(
            ingress_source_from_request(Some("feishu")),
            IngressSource::Other("feishu".to_string())
        );
    }

    #[test]
    fn build_ingress_envelope_uses_entry_metadata_contract() {
        let ingress = build_ingress_envelope(
            "ses_1",
            ingress_source_from_request(None),
            "hello",
            Some("idem_1".to_string()),
            Some("session_prompt".to_string()),
        );

        assert_eq!(ingress.source, agendao_session::prompt::IngressSource::Api);
        assert_eq!(ingress.context_key.as_deref(), Some("session_prompt"));
        assert_eq!(ingress.idempotency_key.as_deref(), Some("idem_1"));
        assert_eq!(
            ingress.stabilization.policy,
            agendao_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY
        );
    }

    fn unresolved_prompt_payload(text: &str) -> ResolvedPromptPayload {
        ResolvedPromptPayload {
            display_text: text.to_string(),
            execution_text: text.to_string(),
            agent: None,
            model: None,
            scheduler: None,
            command: None,
            pending_raw_arguments: None,
        }
    }

    fn prompt_request_message(text: &str) -> SessionPromptRequest {
        SessionPromptRequest {
            message: Some(text.to_string()),
            parts: None,
            idempotency_key: None,
            ingress_source: None,
            model: None,
            variant: None,
            agent: None,
            scheduler: None,
            command: None,
            arguments: None,
            recovery: None,
            source_origin: None,
            source_surface: None,
        }
    }

    fn sample_external_ingress(session_id: &str) -> agendao_session::prompt::IngressTurnEnvelope {
        let event = agendao_types::ExternalAdapterEvent {
            adapter_id: "generic".to_string(),
            source: agendao_types::ExternalAdapterSource::GenericWebhook,
            external_event_id: "evt_1".to_string(),
            external_user_id: "user_1".to_string(),
            external_conversation_id: "chat_1".to_string(),
            external_thread_id: None,
            received_at_ms: 1_714_000_000_000,
            text: "hello from webhook".to_string(),
            attachments: Vec::new(),
            idempotency_key: None,
            reply_target: None,
            raw_event_ref: None,
        };
        agendao_session::prompt::external_adapter_event_to_ingress_turn(session_id, &event)
            .expect("external adapter event should map to ingress")
    }

    #[test]
    fn task_ingress_for_prompt_preserves_verified_external_adapter_ingress() {
        let verified_ingress = sample_external_ingress("ses_1");
        let request = prompt_request_message("hello from webhook");
        let resolved = unresolved_prompt_payload("hello from webhook");

        let ingress = task_ingress_for_prompt(
            "ses_1",
            "hello from webhook",
            &request,
            &resolved,
            Some(verified_ingress),
        )
        .unwrap();

        assert_eq!(
            ingress.source,
            agendao_session::prompt::IngressSource::Other(
                "external:generic-webhook:generic".to_string()
            )
        );
        assert_eq!(
            ingress.stabilization.policy,
            agendao_session::prompt::INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY
        );
        assert!(ingress.external_adapter.is_some());
        assert_ne!(ingress.context_key.as_deref(), Some("session_prompt"));
    }

    #[test]
    fn task_ingress_for_prompt_rejects_verified_ingress_for_other_session() {
        let verified_ingress = sample_external_ingress("ses_other");
        let request = prompt_request_message("hello from webhook");
        let resolved = unresolved_prompt_payload("hello from webhook");

        let error = task_ingress_for_prompt(
            "ses_1",
            "hello from webhook",
            &request,
            &resolved,
            Some(verified_ingress),
        )
        .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[test]
    fn task_ingress_for_prompt_still_builds_http_entry_ingress_when_unset() {
        let mut request = prompt_request_message("hello");
        request.idempotency_key = Some("idem_1".to_string());
        request.ingress_source = Some("api".to_string());
        let resolved = unresolved_prompt_payload("hello");

        let ingress = task_ingress_for_prompt("ses_1", "hello", &request, &resolved, None).unwrap();

        assert_eq!(ingress.source, agendao_session::prompt::IngressSource::Api);
        assert_eq!(ingress.context_key.as_deref(), Some("session_prompt"));
        assert_eq!(ingress.idempotency_key.as_deref(), Some("idem_1"));
        assert!(ingress.external_adapter.is_none());
    }

    #[tokio::test]
    async fn followup_queue_preserves_fifo_order_and_tracks_runtime_count() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };

        for (index, message) in ["first", "second", "third"].iter().enumerate() {
            let queued_count = enqueue_followup_prompt(
                &state,
                &session_id,
                QueuedFollowupPrompt {
                    request: prompt_request_message(message),
                    apply_plugin_config_hooks: true,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{message} should queue: {error}"));
            assert_eq!(queued_count, index as u64 + 1);
        }
        assert_eq!(
            state
                .runtime_telemetry
                .runtime_state()
                .get(&session_id)
                .await
                .expect("runtime state should exist")
                .pending_followup_count,
            3
        );

        let adopted = take_followup_prompt(&state, &session_id)
            .await
            .expect("first queued follow-up should be adoptable");
        assert_eq!(adopted.request.message.as_deref(), Some("first"));
        assert_eq!(
            state
                .runtime_telemetry
                .runtime_state()
                .get(&session_id)
                .await
                .expect("runtime state should exist")
                .pending_followup_count,
            2
        );

        let second = take_followup_prompt(&state, &session_id)
            .await
            .expect("second queued follow-up should be adoptable");
        assert_eq!(second.request.message.as_deref(), Some("second"));

        let dropped = drain_followup_prompts(&state, &session_id).await;
        assert_eq!(dropped, 1);
        assert_eq!(
            state
                .runtime_telemetry
                .runtime_state()
                .get(&session_id)
                .await
                .expect("runtime state should exist")
                .pending_followup_count,
            0
        );
        assert!(take_followup_prompt(&state, &session_id).await.is_none());
    }

    #[tokio::test]
    async fn aborting_a_session_drops_queued_followups() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };

        for message in ["queued-one", "queued-two"] {
            enqueue_followup_prompt(
                &state,
                &session_id,
                QueuedFollowupPrompt {
                    request: prompt_request_message(message),
                    apply_plugin_config_hooks: true,
                },
            )
            .await
            .expect("follow-up should queue");
        }

        let response = super::super::cancel::abort_session_execution(&state, &session_id).await;
        assert_eq!(response["dropped_queued_prompts"], serde_json::json!(2));
        assert!(take_followup_prompt(&state, &session_id).await.is_none());
        assert_eq!(
            state
                .runtime_telemetry
                .runtime_state()
                .get(&session_id)
                .await
                .expect("runtime state should exist")
                .pending_followup_count,
            0
        );
    }

    #[tokio::test]
    async fn abort_clears_pending_question_and_awaiting_ledger_after_run_token_retires() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };
        crate::session_runtime::task_ledger::apply_task_ledger_op(
            &state,
            &session_id,
            0,
            agendao_types::task_ledger::TaskLedgerOp::Create {
                goal: agendao_types::task_ledger::TaskGoal {
                    statement: "answer then resume".to_string(),
                    acceptance_criteria: vec![],
                    criterion_checks: vec![],
                    set_by: agendao_types::task_ledger::TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "wait for answer".to_string(),
            },
        )
        .await
        .expect("create ledger");
        let (question, _waiter) = state
            .runtime_telemetry
            .register_question(
                session_id.clone(),
                vec![agendao_tool::QuestionDef {
                    question: "continue?".to_string(),
                    header: None,
                    options: vec![],
                    multiple: false,
                }],
            )
            .await;
        crate::session_runtime::task_ledger_reducer::dispatch_seam(
            &state,
            &session_id,
            agendao_types::task_ledger::TaskLedgerSeamFact::InteractionAwaiting {
                kind: agendao_types::task_ledger::AwaitingInteractionKind::Question,
                interaction_id: question.id,
            },
        )
        .await;

        let response = super::super::cancel::abort_session_execution(&state, &session_id).await;
        assert_eq!(response["aborted"], serde_json::json!(true));
        assert_eq!(
            response["cancelled_pending_questions"],
            serde_json::json!(1)
        );
        assert!(state
            .runtime_telemetry
            .list_questions_for_session(&session_id)
            .await
            .is_empty());
        let ledger = crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
            .await
            .expect("ledger");
        assert_eq!(
            ledger.status,
            agendao_types::task_ledger::TaskLedgerStatus::Interrupted
        );
        assert!(ledger.awaiting_interactions.is_empty());
        assert!(
            ledger
                .next
                .as_ref()
                .expect("pre-interrupt next")
                .provenance
                .pre_interrupt
        );
    }

    #[test]
    fn live_web_ingress_batch_merges_parts_and_uses_stabilized_ingress() {
        let mut session = Session::new("project", "/tmp");
        let now_ms = 1_000;
        let mut first = build_ingress_envelope(
            &session.id,
            IngressSource::Web,
            "first",
            Some("web_1".to_string()),
            Some("session_prompt".to_string()),
        );
        first.received_at_ms = now_ms;
        first.stabilized_at_ms = now_ms;

        let mut second = build_ingress_envelope(
            &session.id,
            IngressSource::Web,
            "second",
            Some("web_2".to_string()),
            Some("session_prompt".to_string()),
        );
        second.received_at_ms = now_ms + 10;
        second.stabilized_at_ms = now_ms + 10;

        let owner = open_live_web_ingress_batch(
            &mut session,
            first,
            vec![agendao_session::prompt::PartInput::Text {
                text: "first".to_string(),
            }],
            now_ms,
        )
        .expect("leader batch should open");
        assert!(append_live_web_ingress_batch_if_present(
            &mut session,
            second,
            vec![agendao_session::prompt::PartInput::Text {
                text: "second".to_string(),
            }],
            now_ms + 10,
        ));

        let batch = drain_live_web_ingress_batch(&mut session, &owner).expect("batch should drain");
        let (ingress, parts) =
            resolve_live_web_ingress_batch(batch).expect("batch should resolve to one turn");

        assert_eq!(
            ingress.stabilization.policy,
            agendao_session::prompt::INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH
        );
        let rendered = parts
            .iter()
            .filter_map(|part| match part {
                agendao_session::prompt::PartInput::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["first", "second"]);
    }

    #[test]
    fn live_web_ingress_batch_does_not_accept_command_turns() {
        let mut session = Session::new("project", "/tmp");
        let now_ms = 1_000;
        let mut ingress = build_ingress_envelope(
            &session.id,
            IngressSource::Web,
            "/new",
            Some("web_cmd".to_string()),
            Some("session_prompt".to_string()),
        );
        ingress.command = Some("new".to_string());

        assert!(!append_live_web_ingress_batch_if_present(
            &mut session,
            ingress.clone(),
            vec![agendao_session::prompt::PartInput::Text {
                text: "/new".to_string(),
            }],
            now_ms,
        ));
        assert!(open_live_web_ingress_batch(
            &mut session,
            ingress,
            vec![agendao_session::prompt::PartInput::Text {
                text: "/new".to_string(),
            }],
            now_ms,
        )
        .is_none());
    }

    #[test]
    fn scheduler_session_context_carries_recent_turns() {
        let mut session = Session::new("project", "/tmp");
        session.set_title("Martini3 antibody formulation research");
        session.add_user_message("检索近年来 martini3 在抗体制剂开发中的研究");
        {
            let assistant = session.add_assistant_message();
            assistant.add_text("Found papers A, B, and C with notes about antibody formulation.");
        }
        let block = build_scheduler_session_context_block(&session)
            .expect("same-session scheduler context should render");

        assert!(block.contains("## Session Continuity Context"));
        assert!(block.contains("## Context Coverage"));
        assert!(block.contains("## Hydration Guidance"));
        assert!(block.contains("scheduler_context_hydrate"));
        assert!(block.contains("Martini3 antibody formulation research"));
        assert!(block.contains("Found papers A, B, and C"));
        assert!(block.contains("exact_tail_message_ids"));
    }

    #[test]
    fn scheduler_session_context_uses_projection_summary_for_projected_assistant_output() {
        let mut session = Session::new("project", "/tmp");
        session.add_user_message("检索 AlphaFold3 方法学研究");
        let assistant_id = {
            let assistant = session.add_assistant_message();
            assistant.add_text("full report body that should not be placed in scheduler context");
            assistant.metadata.insert(
                SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
                serde_json::json!("OnDemandArtifact"),
            );
            assistant.metadata.insert(
                SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
                serde_json::json!(
                    "Large assistant output stored as artifact `art_assistant_test`. Summary:\nAlphaFold3 methodology survey summary"
                ),
            );
            assistant.metadata.insert(
                SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY.to_string(),
                serde_json::json!([{"id": "art_assistant_test"}]),
            );
            assistant.id.clone()
        };

        let block = build_scheduler_session_context_block(&session)
            .expect("same-session scheduler context should render");
        let packet = build_scheduler_session_context_packet(&session)
            .expect("same-session scheduler context packet should render");
        let metadata = packet.metadata_value();

        assert!(block.contains("Projected assistant output for model context"));
        assert!(block.contains("AlphaFold3 methodology survey summary"));
        assert!(block.contains(&assistant_id));
        assert!(!block.contains("full report body that should not be placed"));
        assert_eq!(
            metadata["exact_recent_tail"][1]["projected"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn scheduler_session_context_rejects_unsanctioned_full_projection_policy() {
        let mut session = Session::new("project", "/tmp");
        session.add_user_message("总结这次调查");
        let assistant_id = {
            let assistant = session.add_assistant_message();
            assistant.add_text("full report body should remain the scheduler context source");
            assistant.metadata.insert(
                SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
                serde_json::json!("Full"),
            );
            assistant.metadata.insert(
                SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
                serde_json::json!("summary must not override full projection"),
            );
            assistant.id.clone()
        };

        let block = build_scheduler_session_context_block(&session)
            .expect("same-session scheduler context should render");
        let packet = build_scheduler_session_context_packet(&session)
            .expect("same-session scheduler context packet should render");
        let metadata = packet.metadata_value();

        assert!(!block.contains("Projected assistant output for model context"));
        assert!(block.contains("full report body should remain the scheduler context source"));
        assert!(!block.contains("summary must not override full projection"));
        assert_eq!(
            metadata["exact_recent_tail"][1]["projected"],
            serde_json::json!(false)
        );
        assert_eq!(
            metadata["exact_recent_tail"][1]["message_id"].as_str(),
            Some(assistant_id.as_str())
        );
    }

    #[test]
    fn scheduler_session_context_reports_recent_tail_coverage() {
        let mut session = Session::new("project", "/tmp");
        for index in 0..8 {
            session.add_user_message(format!("turn {index}"));
        }

        let block = build_scheduler_session_context_block(&session)
            .expect("same-session scheduler context should render");

        assert!(block.contains("exact_recent_tail: last 6 of 8 eligible"));
        assert!(block.contains("omitted_older_turns: 2"));
        assert!(!block.contains("turn 0"));
        assert!(!block.contains("turn 1"));
        assert!(block.contains("turn 7"));
    }

    #[test]
    fn scheduler_session_context_anchors_compaction_summary() {
        let mut session = Session::new("project", "/tmp");
        session.add_user_message("earlier research request");
        let compaction_id = {
            let summary = session.add_assistant_message();
            summary
                .metadata
                .insert("summary".to_string(), serde_json::json!(true));
            summary.add_text("Compacted research findings about Martini3 antibodies.");
            summary.id.clone()
        };

        let block = build_scheduler_session_context_block(&session)
            .expect("same-session scheduler context should render");

        assert!(block.contains("## Latest Compaction Summary"));
        assert!(block.contains(&format!("source: assistant `{compaction_id}`")));
        assert!(block.contains(&format!("compaction_summary_message_id: `{compaction_id}`")));
    }

    #[test]
    fn scheduler_session_context_packet_metadata_names_hydration_policy() {
        let mut session = Session::new("project", "/tmp");
        session.add_user_message("first request");

        let packet = build_scheduler_session_context_packet(&session)
            .expect("same-session scheduler context packet should render");
        let metadata = packet.metadata_value();

        assert_eq!(metadata["version"], serde_json::json!(1));
        assert!(metadata["recall_policy"]
            .as_str()
            .expect("recall policy should be present")
            .contains("use_scheduler_context_hydrate"));
    }

    #[test]
    fn scheduler_session_context_carries_memory_anchors_from_last_prefetch() {
        let mut session = Session::new("project", "/tmp");
        session.insert_metadata(
            MEMORY_LAST_PREFETCH_METADATA_KEY.to_string(),
            serde_json::to_value(MemoryRetrievalPacket {
                generated_at: 42,
                snapshot: false,
                query: Some("follow up".to_string()),
                scopes: vec![agendao_types::MemoryScope::SessionEphemeral],
                items: vec![agendao_types::MemoryRecallView {
                    card: agendao_types::MemoryCardView {
                        id: agendao_types::MemoryRecordId("mem_123".to_string()),
                        kind: agendao_types::MemoryKind::Lesson,
                        scope: agendao_types::MemoryScope::SessionEphemeral,
                        status: agendao_types::MemoryStatus::Validated,
                        title: "Prior Martini3 bibliography decision".to_string(),
                        summary: "Use the saved paper shortlist.".to_string(),
                        derived_skill_name: None,
                        linked_skill_name: None,
                        confidence: Some(0.9),
                        validation_status: agendao_types::MemoryValidationStatus::Passed,
                        last_validated_at: None,
                    },
                    why_recalled: "query matched Martini3 follow-up".to_string(),
                    evidence_summary: None,
                }],
                note: None,
                budget_limit: Some(6),
            })
            .expect("memory packet should serialize"),
        );

        let packet = build_scheduler_session_context_packet(&session)
            .expect("memory anchors alone should render scheduler context");
        let block = packet.render();
        let metadata = packet.metadata_value();

        assert!(block.contains("## Memory Anchors"));
        assert!(block.contains("mem_123"));
        assert!(block.contains("Prior Martini3 bibliography decision"));
        assert_eq!(metadata["memory_anchors"][0]["record_id"], "mem_123");
        assert_eq!(metadata["memory_anchors"][0]["status"], "Validated");
    }

    #[test]
    fn scheduler_session_context_packet_metadata_is_typed_authority() {
        let mut session = Session::new("project", "/tmp");
        let first_id = session.add_user_message("first request").id.clone();
        let second_id = {
            let message = session.add_assistant_message();
            message.add_text("first answer body that should stay in the continuity packet");
            message.id.clone()
        };

        let packet = build_scheduler_session_context_packet(&session)
            .expect("same-session scheduler context packet should render");
        let metadata = packet.metadata_value();
        let restored = SessionContinuityPacket::from_value(&metadata)
            .expect("typed continuity packet should deserialize");

        assert_eq!(metadata["version"], serde_json::json!(1));
        assert_eq!(metadata["eligible_message_count"], serde_json::json!(2));
        assert_eq!(metadata["omitted_older_turns"], serde_json::json!(0));
        assert_eq!(metadata["exact_recent_tail"][0]["message_id"], first_id);
        assert_eq!(metadata["exact_recent_tail"][0]["role"], "user");
        assert_eq!(metadata["exact_recent_tail"][0]["text"], "first request");
        assert_eq!(metadata["exact_recent_tail"][1]["message_id"], second_id);
        assert_eq!(metadata["exact_recent_tail"][1]["role"], "assistant");
        assert_eq!(
            metadata["exact_recent_tail"][1]["text"],
            "first answer body that should stay in the continuity packet"
        );
        assert!(!metadata["working_ledger"]
            .as_array()
            .expect("working ledger should serialize")
            .is_empty());
        assert_eq!(restored.render(), packet.render());
    }

    #[test]
    fn scheduler_continuity_packet_carries_task_ledger_without_second_prompt_projection() {
        let mut session = Session::new("project", "/tmp");
        let mut ledger = agendao_types::task_ledger::SessionTaskLedger::empty(&session.id);
        ledger
            .apply(
                0,
                agendao_types::task_ledger::TaskLedgerOp::Create {
                    goal: agendao_types::task_ledger::TaskGoal {
                        statement: "resume the governed task".to_string(),
                        acceptance_criteria: vec!["check passes".to_string()],
                        criterion_checks: vec![],
                        set_by: agendao_types::task_ledger::TaskLedgerActor::User,
                        set_at: 1,
                    },
                    next_statement: "run the remaining check".to_string(),
                },
                2,
            )
            .expect("create ledger");
        session.insert_metadata(
            agendao_types::task_ledger::TASK_LEDGER_METADATA_KEY.to_string(),
            serde_json::to_value(&ledger).expect("serialize ledger"),
        );

        let packet = build_scheduler_session_context_packet(&session)
            .expect("task ledger alone should produce a continuity packet");
        let projected = packet
            .task_ledger
            .as_ref()
            .expect("typed task ledger continuity");
        assert_eq!(projected.revision, ledger.revision);
        assert_eq!(
            projected.next.as_ref().map(|next| next.statement.as_str()),
            Some("run the remaining check")
        );
        assert!(
            !packet.render().contains("resume the governed task"),
            "the continuity packet is audit metadata; live task-ledger projection has one separate injection point"
        );
    }

    #[test]
    fn scheduler_session_context_keeps_source_anchors_when_truncated() {
        let mut session = Session::new("project", "/tmp");
        let mut latest_message_id = String::new();
        for index in 0..6 {
            let message = session.add_user_message(format!("turn {index} {}", "x".repeat(2_000)));
            latest_message_id = message.id.clone();
        }

        let block = build_scheduler_session_context_block(&session)
            .expect("same-session scheduler context should render");

        assert!(block.contains("## Source Anchors"));
        assert!(block.contains("## Hydration Guidance"));
        assert!(block.contains(&format!("`{latest_message_id}`")));
        assert!(block.contains("scheduler_context_hydrate"));
        assert!(block.contains("...[truncated]..."));
        assert!(block.chars().count() <= SCHEDULER_CONTEXT_TEXT_LIMIT);
    }

    #[test]
    fn scheduler_prompt_merge_keeps_memory_before_current_prompt() {
        let merged = merge_scheduler_prompt_with_memory(
            "把你前面检索的结果写到 markdown 文档中",
            Some("Frozen Memory Snapshot:\n- preference"),
            Some("Turn Memory Recall:\n- related method"),
        );

        assert!(merged.contains("Frozen Memory Snapshot"));
        assert!(merged.contains("Turn Memory Recall"));
        assert!(merged.ends_with("把你前面检索的结果写到 markdown 文档中"));
    }

    #[tokio::test]
    async fn scheduler_user_message_preserves_attachment_only_parts() {
        let prompt_runner = test_prompt_runner();
        let mut session = Session::new("project", "/tmp");
        let input = agendao_session::PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            parts: vec![agendao_session::PartInput::File {
                url: "data:text/plain;base64,SGVsbG8=".to_string(),
                filename: Some("note.txt".to_string()),
                mime: Some("text/plain".to_string()),
            }],
            tools: None,
            ingress: None,
        };

        let message_id = create_scheduler_user_message(
            &prompt_runner,
            &mut session,
            &input,
            SchedulerUserMessageContext {
                display_prompt_text: "[1 attachment]",
                resolved_user_prompt: "",
                choice: &agendao_orchestrator::selector::SchedulerChoice::Auto,
                recovery: None,
            },
        )
        .await
        .expect("scheduler attachment-only user message should be created");

        let message = session
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("user message should exist");
        assert!(
            text_parts(message).contains(&"[1 attachment]"),
            "attachment-only scheduler prompt should retain a visible summary text part"
        );
        assert!(message.parts.iter().any(|part| matches!(
            &part.part_type,
            PartType::File { filename, mime, .. }
            if filename == "note.txt" && mime == "text/plain"
        )));
        assert_eq!(
            message.metadata.get("scheduler"),
            Some(&serde_json::json!({ "kind": "auto" }))
        );
    }

    #[tokio::test]
    async fn scheduler_user_message_keeps_text_and_file_parts_together() {
        let prompt_runner = test_prompt_runner();
        let mut session = Session::new("project", "/tmp");
        let input = agendao_session::PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            parts: vec![
                agendao_session::PartInput::Text {
                    text: "Inspect @note.txt".to_string(),
                },
                agendao_session::PartInput::File {
                    url: "data:text/plain;base64,SGVsbG8=".to_string(),
                    filename: Some("note.txt".to_string()),
                    mime: Some("text/plain".to_string()),
                },
            ],
            tools: None,
            ingress: None,
        };

        let message_id = create_scheduler_user_message(
            &prompt_runner,
            &mut session,
            &input,
            SchedulerUserMessageContext {
                display_prompt_text: "Inspect @note.txt",
                resolved_user_prompt: "Inspect @note.txt",
                choice: &agendao_orchestrator::selector::SchedulerChoice::Auto,
                recovery: None,
            },
        )
        .await
        .expect("scheduler text+attachment user message should be created");

        let message = session
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("user message should exist");
        assert!(
            text_parts(message).contains(&"Inspect @note.txt"),
            "scheduler prompt text should remain visible alongside attachment parts"
        );
        assert!(message.parts.iter().any(|part| matches!(
            &part.part_type,
            PartType::File { filename, .. } if filename == "note.txt"
        )));
        assert_eq!(
            message.metadata.get("resolved_user_prompt"),
            Some(&serde_json::json!("Inspect @note.txt"))
        );
    }

    #[test]
    fn annotate_last_user_message_multimodal_metadata_persists_explain_fields() {
        let mut session = Session::new("project", "/tmp");
        session.add_user_message("[audio input]");

        annotate_last_user_message_multimodal_metadata(
            &mut session,
            &RuntimeMultimodalExplain {
                summary: MultimodalDisplaySummary {
                    primary_text: String::new(),
                    attachment_count: 1,
                    badges: vec!["audio".to_string()],
                    compact_label: "[audio input]".to_string(),
                    kinds: vec!["audio".to_string()],
                },
                capability: PreflightCapabilityView {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-audio".to_string(),
                    attachment: true,
                    tool_call: false,
                    reasoning: false,
                    temperature: true,
                    input: ModalitySupportView {
                        text: true,
                        audio: true,
                        image: false,
                        video: false,
                        pdf: false,
                    },
                    output: ModalitySupportView {
                        text: true,
                        audio: false,
                        image: false,
                        video: false,
                        pdf: false,
                    },
                },
                result: ModalityPreflightResult {
                    warnings: vec!["Audio accepted.".to_string()],
                    unsupported_parts: Vec::new(),
                    recommended_downgrade: None,
                    hard_block: false,
                },
                transport: ModalityTransportResult {
                    replaced_parts: vec!["voice.wav".to_string()],
                    warnings: vec![
                        "ERROR: Cannot read \"voice.wav\" (this model does not support audio input). Inform the user.".to_string(),
                    ],
                },
                resolved_model: "openai/gpt-audio".to_string(),
            },
        );

        let message = session
            .messages
            .iter()
            .rfind(|message| matches!(message.role, agendao_session::MessageRole::User))
            .expect("user message should exist");

        assert_eq!(
            message
                .metadata
                .get("multimodal_resolved_model")
                .and_then(|value| value.as_str()),
            Some("openai/gpt-audio")
        );
        assert_eq!(
            message
                .metadata
                .get("multimodal_compact_label")
                .and_then(|value| value.as_str()),
            Some("[audio input]")
        );
        assert_eq!(
            message
                .metadata
                .get("multimodal_attachment_count")
                .and_then(|value| value.as_u64()),
            Some(1)
        );
        assert!(message.metadata.contains_key("multimodal_preflight"));
        assert_eq!(
            message
                .metadata
                .get("multimodal_transport")
                .and_then(|value| value.get("replaced_parts"))
                .and_then(|value| value.as_array())
                .map(|value| value.len()),
            Some(1)
        );
    }

    #[test]
    fn parse_command_argument_map_preserves_quoted_values() {
        let fields = vec![
            CommandArgumentField {
                key: "goal".to_string(),
                label: "Goal".to_string(),
                required: true,
                kind: CommandArgumentKind::LongText,
                repeatable: false,
                options: Vec::new(),
            },
            CommandArgumentField {
                key: "scope".to_string(),
                label: "Scope".to_string(),
                required: true,
                kind: CommandArgumentKind::GlobList,
                repeatable: true,
                options: Vec::new(),
            },
            CommandArgumentField {
                key: "ship".to_string(),
                label: "Ship".to_string(),
                required: false,
                kind: CommandArgumentKind::Boolean,
                repeatable: false,
                options: vec![CommandArgumentOption {
                    label: "true".to_string(),
                    description: None,
                }],
            },
        ];

        let parsed = parse_command_argument_map(
            Some("--goal \"reduce test flakes\" --scope src/** tests/** --ship"),
            &fields,
        );

        assert_eq!(
            parsed.get("goal"),
            Some(&vec!["reduce test flakes".to_string()])
        );
        assert_eq!(
            parsed.get("scope"),
            Some(&vec!["src/**".to_string(), "tests/**".to_string()])
        );
        assert_eq!(parsed.get("ship"), Some(&vec!["true".to_string()]));
    }

    #[test]
    fn missing_required_command_fields_only_returns_unset_fields() {
        let fields = vec![
            CommandArgumentField {
                key: "goal".to_string(),
                label: "Goal".to_string(),
                required: true,
                kind: CommandArgumentKind::LongText,
                repeatable: false,
                options: Vec::new(),
            },
            CommandArgumentField {
                key: "verify".to_string(),
                label: "Verify".to_string(),
                required: true,
                kind: CommandArgumentKind::CommandLine,
                repeatable: false,
                options: Vec::new(),
            },
        ];

        let parsed = parse_command_argument_map(Some("--goal improve-docs"), &fields);
        let missing = missing_required_command_fields(&fields, &parsed);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].key, "verify");
    }

    #[test]
    fn hydrate_scheduler_command_arguments_does_not_inject_hidden_defaults() {
        let registry = CommandRegistry::new();
        let command = registry.get("autoresearch").expect("autoresearch command");
        let invocation = command
            .invocation
            .as_ref()
            .expect("autoresearch invocation");

        let (arguments, raw_arguments) =
            hydrate_scheduler_command_arguments("", &invocation.argument_schema)
                .expect("empty arguments should parse");

        assert!(arguments.is_empty());
        assert!(raw_arguments.is_empty());
    }

    #[test]
    fn hydrate_scheduler_command_arguments_preserves_explicit_user_values() {
        let registry = CommandRegistry::new();
        let command = registry.get("autoresearch").expect("autoresearch command");
        let invocation = command
            .invocation
            .as_ref()
            .expect("autoresearch invocation");

        let (arguments, raw_arguments) = hydrate_scheduler_command_arguments(
            "--goal \"teacher demo goal\" --verify ./custom-verify.sh",
            &invocation.argument_schema,
        )
        .expect("explicit arguments should parse");

        assert_eq!(
            arguments.get("goal"),
            Some(&vec!["teacher demo goal".to_string()])
        );
        assert_eq!(
            arguments.get("verify"),
            Some(&vec!["./custom-verify.sh".to_string()])
        );
        assert!(raw_arguments.contains("--goal \"teacher demo goal\""));
        assert!(raw_arguments.contains("--verify ./custom-verify.sh"));
        assert!(!raw_arguments.contains("--guard"));
        assert!(!raw_arguments.contains("--iterations"));
    }

    #[tokio::test]
    async fn configured_command_uses_merged_template_agent_and_model() {
        let config = AppConfig {
            command: Some(std::collections::HashMap::from([(
                "inherited".to_string(),
                agendao_config::CommandConfig {
                    description: Some("Inherited command".to_string()),
                    template: Some("Inspect $ARGUMENTS".to_string()),
                    agent: Some("global-agent".to_string()),
                    model: Some("deepseek-v4-flash".to_string()),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        let resolved = resolve_prompt_payload(
            "/inherited exact marker",
            "session-command",
            "/workspace",
            &config,
        )
        .await
        .expect("configured command should resolve");

        assert_eq!(resolved.execution_text, "Inspect exact marker");
        assert_eq!(resolved.agent.as_deref(), Some("global-agent"));
        assert_eq!(resolved.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(
            resolved.command.as_ref().map(|command| &command.source),
            Some(&CommandSource::Config)
        );
    }
}
