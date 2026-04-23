use std::path::{Path as FsPath, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use rocode_config::Config as AppConfig;
use rocode_memory::{
    load_persisted_memory_snapshot, render_frozen_snapshot_block, render_prefetch_packet_block,
    PersistedMemorySnapshot, MEMORY_LAST_PREFETCH_METADATA_KEY,
};
use rocode_types::{MemoryRetrievalPacket, MemoryRetrievalQuery};
use serde::Deserialize;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::recovery::RecoveryExecutionContext;
use crate::routes::multimodal::resolve_provider_model;
use crate::routes::permission::request_permission;
use crate::routes::skill_catalog::enrich_scheduler_plan_skills;
use crate::runtime_control::SessionRunStatus;
use crate::session_runtime::events::{
    broadcast_session_updated, emit_output_block_via_hook, server_output_block_hook, ServerEvent,
};
use crate::session_runtime::{
    assistant_visible_text, ensure_default_session_title,
    finalize_active_scheduler_stage_cancelled, first_user_message_text,
    visible_assistant_text_from_orchestrator_output, ModelPricing, SessionSchedulerLifecycleHook,
};
use crate::{ApiError, Result, ServerState};
use rocode_agent::{AgentMode, AgentRegistry};
use rocode_command::{
    output_blocks::{MessageBlock, MessageRole as OutputMessageRole, OutputBlock},
    Command, CommandArgumentField, CommandArgumentKind, CommandContext, CommandRegistry,
    InteractivePolicy,
};
use rocode_multimodal::{MultimodalAuthority, RuntimeMultimodalExplain, SessionPartAdapter};
use rocode_orchestrator::output_metadata::output_usage;
use rocode_orchestrator::{
    scheduler_orchestrator_from_plan, scheduler_plan_from_profile, AvailableAgentMeta,
    AvailableCategoryMeta, CommandDefinition as WorkflowCommandDefinition, DebugConfig,
    ExecutionContext as OrchestratorExecutionContext, IterationPolicyDefinition, MetricDefinition,
    ModelResolver, ObjectiveDefinition, Orchestrator, OrchestratorContext, ScopeDefinition,
    ToolExecutor as OrchestratorToolExecutor, ToolRunner,
};

use super::super::tui::request_question_answers;
use super::super::{
    apply_plugin_config_hooks, get_plugin_loader, plugin_auth::ensure_plugin_loader_active,
    should_apply_plugin_config_hooks,
};
use super::autoresearch_target::{
    resolve_autoresearch_command, AutoresearchProfileOverrideRecord,
    AUTORESEARCH_PROFILE_OVERRIDE_METADATA_KEY,
};
use super::cancel::is_scheduler_cancellation_error;
use super::messages::{prompt_display_text, prompt_text_from_parts};
use super::scheduler::{
    apply_skill_tree_telemetry_metadata, resolve_prompt_request_config,
    resolve_scheduler_profile_config, scheduler_mode_kind, scheduler_system_prompt_preview,
    to_task_agent_info, SchedulerAgentResolver, SchedulerRunCancelToken,
    SessionSchedulerModelResolver, SessionSchedulerToolExecutor,
};
use super::session_crud::{
    persist_sessions_if_enabled, resolved_session_directory, set_session_run_status, IdleGuard,
};
use super::telemetry::persist_session_telemetry_metadata;

#[derive(Debug, Clone)]
struct ResolvedPromptPayload {
    display_text: String,
    execution_text: String,
    agent: Option<String>,
    scheduler_profile: Option<String>,
    scheduler_profile_override: Option<(String, rocode_orchestrator::SchedulerProfileConfig)>,
    autoresearch_profile_override_record: Option<AutoresearchProfileOverrideRecord>,
    command: Option<Command>,
    pending_raw_arguments: Option<String>,
}

fn set_autoresearch_override_metadata(
    session: &mut rocode_session::Session,
    record: Option<&AutoresearchProfileOverrideRecord>,
) {
    if let Some(record) = record {
        if let Ok(value) = serde_json::to_value(record) {
            session.insert_metadata(
                AUTORESEARCH_PROFILE_OVERRIDE_METADATA_KEY.to_string(),
                value,
            );
            return;
        }
    }
    session.remove_metadata(AUTORESEARCH_PROFILE_OVERRIDE_METADATA_KEY);
}

async fn resolve_prompt_payload(
    display_text: &str,
    session_id: &str,
    session_directory: &str,
    config: &AppConfig,
) -> Result<ResolvedPromptPayload> {
    let mut registry = CommandRegistry::new();
    registry
        .load_from_directory(&PathBuf::from(session_directory))
        .map_err(|error| ApiError::BadRequest(format!("Failed to load commands: {}", error)))?;

    let Some(parsed) = registry.parse_invocation(display_text) else {
        return Ok(ResolvedPromptPayload {
            display_text: display_text.to_string(),
            execution_text: display_text.to_string(),
            agent: None,
            scheduler_profile: None,
            scheduler_profile_override: None,
            autoresearch_profile_override_record: None,
            command: None,
            pending_raw_arguments: None,
        });
    };

    let command = parsed.command.clone();
    let mut scheduler_profile = command.scheduler_profile.clone();
    let mut scheduler_profile_override = None;
    let mut autoresearch_profile_override_record = None;
    let mut raw_arguments_for_hydration = parsed.raw_arguments.clone();
    let mut raw_arguments_for_pending = parsed.raw_arguments.clone();
    if command.name == "autoresearch" {
        let resolved =
            resolve_autoresearch_command(config, session_directory, &parsed.raw_arguments)
                .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        scheduler_profile = Some(resolved.scheduler_profile_name.clone());
        raw_arguments_for_hydration = resolved.raw_arguments_for_execution;
        raw_arguments_for_pending = resolved.raw_arguments_for_pending;
        autoresearch_profile_override_record = resolved.profile_override.clone();
        scheduler_profile_override = resolved
            .profile_override
            .map(|record| (record.profile_name.clone(), record.profile));
    }
    let invocation = command.invocation.as_ref();
    let scheduler_defaults = invocation
        .map(|invocation| {
            hydrate_scheduler_command_arguments(
                config,
                &command,
                scheduler_profile_override
                    .as_ref()
                    .map(|(_, profile)| profile),
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
            &invocation
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
        agent: None,
        scheduler_profile,
        scheduler_profile_override,
        autoresearch_profile_override_record,
        command: Some(command.clone()),
        pending_raw_arguments,
    })
}

async fn ensure_memory_frozen_snapshot(
    state: &Arc<ServerState>,
    session: &mut rocode_session::Session,
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
                rocode_memory::MEMORY_FROZEN_SNAPSHOT_METADATA_KEY.to_string(),
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
    session: &mut rocode_session::Session,
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

pub(super) fn merge_system_prompt_with_memory_snapshot(
    base: Option<String>,
    frozen_snapshot_block: Option<&str>,
) -> Option<String> {
    match (
        base.map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        frozen_snapshot_block
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        (Some(base), Some(snapshot)) => Some(format!("{base}\n\n{snapshot}")),
        (Some(base), None) => Some(base),
        (None, Some(snapshot)) => Some(snapshot.to_string()),
        (None, None) => None,
    }
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

fn workflow_command_value(def: &WorkflowCommandDefinition) -> String {
    def.command.trim().to_string()
}

fn workflow_scope_values(scope: &ScopeDefinition) -> Vec<String> {
    scope
        .include
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn workflow_metric_value(metric: &MetricDefinition) -> String {
    serde_json::to_string(metric).unwrap_or_else(|_| "metric".to_string())
}

fn workflow_debug_symptom(debug: &DebugConfig) -> String {
    debug.symptom.trim().to_string()
}

fn workflow_iteration_value(iteration_policy: &IterationPolicyDefinition) -> Option<String> {
    iteration_policy
        .max_iterations
        .map(|value| value.to_string())
}

fn populate_objective_defaults(
    defaults: &mut std::collections::HashMap<String, Vec<String>>,
    objective: &ObjectiveDefinition,
) {
    let goal = objective.goal.trim();
    if !goal.is_empty() {
        defaults.insert("goal".to_string(), vec![goal.to_string()]);
    }

    let scope = workflow_scope_values(&objective.scope);
    if !scope.is_empty() {
        defaults.insert("scope".to_string(), scope);
    }

    let metric = workflow_metric_value(&objective.metric);
    if !metric.trim().is_empty() {
        defaults.insert("metric".to_string(), vec![metric]);
    }

    let verify = workflow_command_value(&objective.verify);
    if !verify.is_empty() {
        defaults.insert("verify".to_string(), vec![verify]);
    }

    if let Some(guard) = objective.guard.as_ref() {
        let guard = workflow_command_value(guard);
        if !guard.is_empty() {
            defaults.insert("guard".to_string(), vec![guard]);
        }
    }
}

fn workflow_command_defaults(
    config: &AppConfig,
    command: &Command,
    scheduler_profile_override: Option<&rocode_orchestrator::SchedulerProfileConfig>,
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let profile = if let Some(profile) = scheduler_profile_override {
        profile.clone()
    } else {
        let Some(profile_name) = command.scheduler_profile.as_deref() else {
            return Ok(std::collections::HashMap::new());
        };
        let Some((_, profile)) = resolve_scheduler_profile_config(config, Some(profile_name))
        else {
            return Ok(std::collections::HashMap::new());
        };
        profile
    };
    let Some(workflow) = profile.workflow() else {
        return Ok(std::collections::HashMap::new());
    };

    let mut defaults = std::collections::HashMap::new();

    if let Some(objective) = workflow.objective.as_ref() {
        populate_objective_defaults(&mut defaults, objective);
    }
    if let Some(iteration_policy) = workflow.iteration_policy.as_ref() {
        if let Some(iterations) = workflow_iteration_value(iteration_policy) {
            defaults.insert("iterations".to_string(), vec![iterations]);
        }
    }
    if let Some(debug) = workflow.debug.as_ref() {
        let symptom = workflow_debug_symptom(debug);
        if !symptom.is_empty() {
            defaults.insert("symptom".to_string(), vec![symptom]);
        }
    }
    if let Some(ship) = workflow.ship.as_ref() {
        defaults.entry("target".to_string()).or_insert_with(|| {
            vec![format!(
                "ship {}",
                serde_json::to_string(&ship.ship_type).unwrap_or_else(|_| "target".to_string())
            )]
        });
    }

    Ok(defaults)
}

fn hydrate_scheduler_command_arguments(
    config: &AppConfig,
    command: &Command,
    scheduler_profile_override: Option<&rocode_orchestrator::SchedulerProfileConfig>,
    raw_arguments: &str,
    fields: &[CommandArgumentField],
) -> Result<(std::collections::HashMap<String, Vec<String>>, String)> {
    let mut parsed_arguments = parse_command_argument_map(Some(raw_arguments), fields);
    let defaults = workflow_command_defaults(config, command, scheduler_profile_override)?;

    for field in fields {
        let key = normalize_command_field_key(&field.key);
        let has_value = parsed_arguments
            .get(&key)
            .is_some_and(|values| values.iter().any(|value| !value.trim().is_empty()));
        if has_value {
            continue;
        }
        let Some(default_values) = defaults.get(&key) else {
            continue;
        };
        if default_values.is_empty() {
            continue;
        }
        parsed_arguments.insert(key, default_values.clone());
    }

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
) -> rocode_tool::QuestionDef {
    let template = command.interactive.as_ref().and_then(|interactive| {
        interactive.questions.iter().find(|question| {
            normalize_command_field_key(&question.field_key)
                == normalize_command_field_key(&field.key)
        })
    });

    rocode_tool::QuestionDef {
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
                    .map(|option| rocode_tool::QuestionOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                field
                    .options
                    .iter()
                    .map(|option| rocode_tool::QuestionOption {
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
    autoresearch_override_record: Option<&AutoresearchProfileOverrideRecord>,
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
            "schedulerProfile": command.scheduler_profile.clone(),
            "questionId": question_info.id.clone(),
        }),
    );
    set_autoresearch_override_metadata(&mut session, autoresearch_override_record);
    sessions.update(session);

    Ok(question_info.id)
}

fn frontend_smoke_skip_execution_enabled() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::var("ROCODE_FRONTEND_SMOKE_SKIP_EXECUTION")
            .ok()
            .as_deref()
            == Some("1")
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct SessionPromptRequest {
    pub message: Option<String>,
    #[serde(default)]
    pub parts: Option<Vec<rocode_session::prompt::PartInput>>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub agent: Option<String>,
    pub scheduler_profile: Option<String>,
    pub command: Option<String>,
    pub arguments: Option<String>,
    #[serde(default)]
    pub(super) recovery: Option<RecoveryExecutionContext>,
}

pub(super) struct SchedulerUserMessageContext<'a> {
    pub(super) display_prompt_text: &'a str,
    pub(super) resolved_user_prompt: &'a str,
    pub(super) profile_name: &'a str,
    pub(super) mode_kind: &'a str,
    pub(super) resolved_system_prompt: &'a str,
    pub(super) recovery: Option<&'a RecoveryExecutionContext>,
}

pub(super) async fn create_scheduler_user_message(
    prompt_runner: &rocode_session::SessionPrompt,
    session: &mut rocode_session::Session,
    input: &rocode_session::PromptInput,
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
        .rfind(|message| matches!(message.role, rocode_session::MessageRole::User))
    else {
        return Err(ApiError::InternalError(
            "Scheduler prompt did not create a user message".to_string(),
        ));
    };

    if prompt_text_from_parts(&input.parts).trim().is_empty()
        && !ctx.display_prompt_text.trim().is_empty()
    {
        if let Some(rocode_session::PartType::Text { text, .. }) = user_message
            .parts
            .iter_mut()
            .find_map(|part| match &mut part.part_type {
                rocode_session::PartType::Text { .. } => Some(&mut part.part_type),
                _ => None,
            })
        {
            *text = ctx.display_prompt_text.to_string();
        }
    }

    user_message.metadata.insert(
        "resolved_scheduler_profile".to_string(),
        serde_json::json!(ctx.profile_name),
    );
    user_message.metadata.insert(
        "resolved_execution_mode_kind".to_string(),
        serde_json::json!(ctx.mode_kind),
    );
    user_message.metadata.insert(
        "resolved_system_prompt".to_string(),
        serde_json::json!(ctx.resolved_system_prompt),
    );
    user_message.metadata.insert(
        "resolved_system_prompt_preview".to_string(),
        serde_json::json!(ctx.resolved_system_prompt),
    );
    user_message.metadata.insert(
        "resolved_system_prompt_applied".to_string(),
        serde_json::json!(true),
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
        if let Some(target_id) = recovery.target_id.as_deref() {
            user_message.metadata.insert(
                "recovery_target_id".to_string(),
                serde_json::json!(target_id),
            );
        }
        if let Some(target_kind) = recovery.target_kind.as_deref() {
            user_message.metadata.insert(
                "recovery_target_kind".to_string(),
                serde_json::json!(target_kind),
            );
        }
        if let Some(target_label) = recovery.target_label.as_deref() {
            user_message.metadata.insert(
                "recovery_target_label".to_string(),
                serde_json::json!(target_label),
            );
        }
    }

    Ok(user_message.id.clone())
}

fn annotate_last_user_message_multimodal_metadata(
    session: &mut rocode_session::Session,
    explain: &RuntimeMultimodalExplain,
) {
    let Some(user_message) = session
        .messages_mut()
        .iter_mut()
        .rfind(|message| matches!(message.role, rocode_session::MessageRole::User))
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
    if req.agent.is_some() && req.scheduler_profile.is_some() {
        return Err(ApiError::BadRequest(
            "`agent` and `scheduler_profile` are mutually exclusive".to_string(),
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
    let known_agents = AgentRegistry::from_config(&config)
        .list_all()
        .into_iter()
        .map(|agent| agent.name.clone())
        .collect::<Vec<_>>();

    let resolved_prompt = if let Some(parts) = request_parts.as_ref() {
        ResolvedPromptPayload {
            display_text: prompt_display_text(parts),
            execution_text: prompt_text_from_parts(parts),
            agent: None,
            scheduler_profile: None,
            scheduler_profile_override: None,
            autoresearch_profile_override_record: None,
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
                        resolved_prompt
                            .autoresearch_profile_override_record
                            .as_ref(),
                    )
                    .await?;
                    broadcast_session_updated(
                        state.as_ref(),
                        id.clone(),
                        "prompt.command.awaiting_user",
                    );
                    persist_sessions_if_enabled(&state).await;
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
            broadcast_session_updated(state.as_ref(), id.clone(), "prompt.command.accepted");
        }
        broadcast_session_updated(state.as_ref(), id.clone(), "prompt.smoke.accepted");
        persist_sessions_if_enabled(&state).await;
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
        rocode_session::resolve_prompt_parts(
            &prompt_text,
            FsPath::new(&session_directory),
            &known_agents,
        )
        .await
    };
    let effective_agent = resolved_prompt.agent.clone().or(req.agent.clone());
    let effective_scheduler_profile = resolved_prompt
        .scheduler_profile
        .clone()
        .or(req.scheduler_profile.clone());

    let request_config =
        resolve_prompt_request_config(super::scheduler::PromptRequestConfigInput {
            state: &state,
            config: &config,
            session_id: &id,
            requested_agent: effective_agent.as_deref(),
            requested_scheduler_profile: effective_scheduler_profile.as_deref(),
            scheduler_profile_override: resolved_prompt.scheduler_profile_override.clone(),
            request_model: req.model.as_deref(),
            request_variant: req.variant.as_deref(),
            route: "session",
        })
        .await?;
    let scheduler_applied = request_config.scheduler_applied;
    let scheduler_profile_name = request_config.scheduler_profile_name.clone();
    let scheduler_root_agent = request_config.scheduler_root_agent.clone();
    let scheduler_skill_tree_applied = request_config.scheduler_skill_tree_applied;
    let request_skill_tree_plan = request_config.request_skill_tree_plan.clone();
    let resolved_agent = request_config.resolved_agent.clone();
    let provider = request_config.provider.clone();
    let provider_id = request_config.provider_id.clone();
    let model_id = request_config.model_id.clone();
    let agent_system_prompt = request_config.agent_system_prompt.clone();
    let task_compiled_request = request_config.compiled_request.clone();
    let multimodal_explain = {
        let multimodal_parts = SessionPartAdapter::from_session_parts(&prompt_parts);
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
                &prompt_parts,
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
    let task_system_prompt = agent_system_prompt.clone();
    let task_scheduler_applied = scheduler_applied;
    let task_scheduler_profile_name = scheduler_profile_name.clone();
    let task_scheduler_root_agent = scheduler_root_agent.clone();
    let task_scheduler_skill_tree_applied = scheduler_skill_tree_applied;
    let task_request_skill_tree_plan = request_skill_tree_plan.clone();
    let task_config = config.clone();
    let task_recovery = req.recovery.clone();
    let task_prompt_parts = prompt_parts.clone();
    let task_multimodal_explain = multimodal_explain.clone();
    let task_scheduler_profile_config = request_config.scheduler_profile_config.clone();
    let task_autoresearch_override_record =
        resolved_prompt.autoresearch_profile_override_record.clone();
    let mut pending_command_cleared = false;
    {
        let mut sessions = state.sessions.lock().await;
        if let Some(mut session) = sessions.get(&id).cloned() {
            pending_command_cleared = session
                .remove_metadata("pending_command_invocation")
                .is_some();
            set_autoresearch_override_metadata(
                &mut session,
                task_autoresearch_override_record.as_ref(),
            );
            sessions.update(session);
        }
    }
    if pending_command_cleared {
        broadcast_session_updated(state.as_ref(), id.clone(), "prompt.command.accepted");
        persist_sessions_if_enabled(&state).await;
    }
    let output_block_hook: Option<rocode_session::prompt::OutputBlockHook> =
        Some(server_output_block_hook(task_state.clone()));
    tokio::spawn(async move {
        let mut session = {
            let sessions = task_state.sessions.lock().await;
            let Some(session) = sessions.get(&session_id).cloned() else {
                return;
            };
            session
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
            "scheduler_applied",
            serde_json::json!(task_scheduler_applied),
        );
        session.insert_metadata(
            "scheduler_skill_tree_applied",
            serde_json::json!(task_scheduler_skill_tree_applied),
        );
        if let Some(profile) = task_scheduler_profile_name.as_deref() {
            session.insert_metadata("scheduler_profile", serde_json::json!(profile));
        } else {
            session.remove_metadata("scheduler_profile");
        }
        if let Some(root_agent) = task_scheduler_root_agent.as_deref() {
            session.insert_metadata("scheduler_root_agent", serde_json::json!(root_agent));
        } else {
            session.remove_metadata("scheduler_root_agent");
        }
        if let Some(recovery) = task_recovery.as_ref() {
            if let Some(action) = recovery.action.as_ref() {
                session.insert_metadata("last_recovery_action", serde_json::json!(action));
            }
            if let Some(target_id) = recovery.target_id.as_deref() {
                session.insert_metadata("last_recovery_target_id", serde_json::json!(target_id));
            } else {
                session.remove_metadata("last_recovery_target_id");
            }
            if let Some(target_kind) = recovery.target_kind.as_deref() {
                session
                    .insert_metadata("last_recovery_target_kind", serde_json::json!(target_kind));
            } else {
                session.remove_metadata("last_recovery_target_kind");
            }
            if let Some(target_label) = recovery.target_label.as_deref() {
                session.insert_metadata(
                    "last_recovery_target_label",
                    serde_json::json!(target_label),
                );
            } else {
                session.remove_metadata("last_recovery_target_label");
            }
        }

        let (memory_frozen_snapshot_block, memory_prefetch_packet, memory_prefetch_block) =
            resolve_prompt_memory_context(&task_state, &mut session, &prompt_text).await;
        let task_system_prompt = merge_system_prompt_with_memory_snapshot(
            task_system_prompt.clone(),
            memory_frozen_snapshot_block.as_deref(),
        );
        let scheduler_execution_prompt = merge_scheduler_prompt_with_memory(
            &prompt_text,
            memory_frozen_snapshot_block.as_deref(),
            memory_prefetch_block.as_deref(),
        );

        if let (Some(profile_name), Some(profile_config)) = (
            task_scheduler_profile_name.clone(),
            task_scheduler_profile_config.clone(),
        ) {
            let mode_kind = scheduler_mode_kind(&profile_name);
            let resolved_system_prompt =
                scheduler_system_prompt_preview(&profile_name, &profile_config);
            let scheduler_input = rocode_session::PromptInput {
                session_id: session_id.clone(),
                message_id: None,
                model: None,
                agent: None,
                no_reply: false,
                system: None,
                variant: task_variant.clone(),
                parts: task_prompt_parts.clone(),
                tools: None,
            };
            let user_message_id = match create_scheduler_user_message(
                task_state.prompt_runner.as_ref(),
                &mut session,
                &scheduler_input,
                SchedulerUserMessageContext {
                    display_prompt_text: &display_prompt_text,
                    resolved_user_prompt: &prompt_text,
                    profile_name: &profile_name,
                    mode_kind,
                    resolved_system_prompt: &resolved_system_prompt,
                    recovery: task_recovery.as_ref(),
                },
            )
            .await
            {
                Ok(message_id) => message_id,
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        scheduler_profile = %profile_name,
                        %error,
                        "failed to create scheduler user message"
                    );
                    let assistant = session.add_assistant_message();
                    assistant.finish = Some("error".to_string());
                    assistant
                        .metadata
                        .insert("error".to_string(), serde_json::json!(error.to_string()));
                    assistant.add_text(format!("Scheduler input error: {}", error));
                    session.touch();
                    {
                        let mut sessions = task_state.sessions.lock().await;
                        sessions.update(session.clone());
                    }
                    broadcast_session_updated(
                        task_state.as_ref(),
                        session_id.clone(),
                        "prompt.scheduler.error",
                    );
                    persist_sessions_if_enabled(&task_state).await;
                    return;
                }
            };
            let assistant_message_id = session.add_assistant_message().id.clone();

            // Set an immediate title from the user message when the title is
            // still the auto-generated default, so frontends see a meaningful
            // label right away.  The LLM-generated title replaces it later.
            if session.is_default_title() {
                if let Some(first_text) = first_user_message_text(&session) {
                    let immediate = rocode_session::generate_session_title(&first_text);
                    if !immediate.is_empty() && immediate != "New Session" {
                        session.set_auto_title(immediate);
                    }
                }
            }

            {
                let mut sessions = task_state.sessions.lock().await;
                sessions.update(session.clone());
            }
            broadcast_session_updated(
                task_state.as_ref(),
                session_id.clone(),
                "prompt.scheduler.pending",
            );

            let agent_registry = Arc::new(AgentRegistry::from_config(&task_config));

            // Inject runtime metadata into profile_config for dynamic prompt building
            let mut profile_config = profile_config;
            if profile_config.available_agents.is_empty() {
                profile_config.available_agents = agent_registry
                    .list()
                    .iter()
                    .filter(|a| !a.hidden && matches!(a.mode, AgentMode::Subagent | AgentMode::All))
                    .map(|a| AvailableAgentMeta {
                        name: a.name.clone(),
                        description: a.description.clone().unwrap_or_default(),
                        mode: match a.mode {
                            AgentMode::Primary => "primary".to_string(),
                            AgentMode::Subagent => "subagent".to_string(),
                            AgentMode::All => "all".to_string(),
                        },
                        cost: if a.name == "oracle" {
                            "EXPENSIVE".to_string()
                        } else {
                            "CHEAP".to_string()
                        },
                    })
                    .collect();
            }
            if profile_config.available_categories.is_empty() {
                profile_config.available_categories = task_state
                    .category_registry
                    .category_descriptions()
                    .into_iter()
                    .map(|(name, description)| AvailableCategoryMeta { name, description })
                    .collect();
            }

            let current_model = Some(format!("{}:{}", task_provider, task_model));
            let scheduler_abort_token = CancellationToken::new();
            task_state
                .runtime_telemetry
                .register_scheduler_run(
                    &session_id,
                    scheduler_abort_token.clone(),
                    Some(profile_name.clone()),
                )
                .await;
            let tool_executor: Arc<dyn OrchestratorToolExecutor> =
                Arc::new(SessionSchedulerToolExecutor {
                    state: task_state.clone(),
                    session_id: session_id.clone(),
                    message_id: assistant_message_id.clone(),
                    directory: session.record().directory.clone(),
                    abort_token: scheduler_abort_token.clone(),
                    current_model,
                    tool_runtime_config: rocode_tool::ToolRuntimeConfig::from_config(&task_config),
                    agent_registry: agent_registry.clone(),
                });
            let tool_runner = ToolRunner::new(tool_executor.clone());
            let model_resolver: Arc<dyn ModelResolver> = Arc::new(SessionSchedulerModelResolver {
                state: task_state.clone(),
                fallback_provider_id: task_provider.clone(),
                fallback_model_id: task_model.clone(),
                fallback_request: task_compiled_request.clone(),
            });
            let mut exec_metadata = std::collections::HashMap::from([
                (
                    "message_id".to_string(),
                    serde_json::json!(assistant_message_id.clone()),
                ),
                (
                    "user_message_id".to_string(),
                    serde_json::json!(user_message_id.clone()),
                ),
                (
                    "scheduler_profile".to_string(),
                    serde_json::json!(profile_name.clone()),
                ),
            ]);
            apply_skill_tree_telemetry_metadata(
                &mut exec_metadata,
                task_request_skill_tree_plan.as_ref(),
            );
            let exec_ctx = OrchestratorExecutionContext {
                session_id: session_id.clone(),
                workdir: session.record().directory.clone(),
                agent_name: profile_name.clone(),
                metadata: exec_metadata,
            };
            let task_model_pricing = {
                let providers = task_state.providers.read().await;
                providers
                    .find_model(&task_model)
                    .map(|(_, info)| ModelPricing::from_model_info(&info))
            };
            let lifecycle_hook = Arc::new(
                SessionSchedulerLifecycleHook::new(
                    task_state.clone(),
                    session_id.clone(),
                    profile_name.clone(),
                )
                .with_model_pricing(task_model_pricing)
                .with_output_hook(output_block_hook.clone()),
            );
            let ctx = OrchestratorContext {
                agent_resolver: Arc::new(SchedulerAgentResolver {
                    registry: agent_registry.clone(),
                }),
                model_resolver,
                tool_executor,
                lifecycle_hook,
                cancel_token: Arc::new(SchedulerRunCancelToken {
                    token: scheduler_abort_token.clone(),
                }),
                exec_ctx,
            };
            let orchestrator_result =
                match scheduler_plan_from_profile(Some(profile_name.clone()), &profile_config) {
                    Ok(mut plan) => {
                        match enrich_scheduler_plan_skills(&task_state, &mut plan).await {
                            Ok(()) => {
                                scheduler_orchestrator_from_plan(plan, tool_runner)
                                    .execute(&scheduler_execution_prompt, &ctx)
                                    .await
                            }
                            Err(error) => Err(rocode_orchestrator::OrchestratorError::Other(
                                error.to_string(),
                            )),
                        }
                    }
                    Err(error) => Err(rocode_orchestrator::OrchestratorError::Other(
                        error.to_string(),
                    )),
                };
            task_state
                .runtime_telemetry
                .finish_scheduler_run(&session_id)
                .await;

            session = {
                let sessions = task_state.sessions.lock().await;
                sessions.get(&session_id).cloned().unwrap_or(session)
            };

            // Extract handoff metadata before borrowing session mutably.
            let handoff_entries: Vec<(String, serde_json::Value)> =
                if let Ok(ref output) = orchestrator_result {
                    [
                        "scheduler_handoff_mode",
                        "scheduler_handoff_plan_path",
                        "scheduler_handoff_command",
                    ]
                    .iter()
                    .filter_map(|key| {
                        output
                            .metadata
                            .get(*key)
                            .map(|v| (key.to_string(), v.clone()))
                    })
                    .collect()
                } else {
                    Vec::new()
                };

            if let Some(assistant) = session.get_message_mut(&assistant_message_id) {
                assistant.metadata.insert(
                    "model_provider".to_string(),
                    serde_json::json!(&task_provider),
                );
                assistant
                    .metadata
                    .insert("model_id".to_string(), serde_json::json!(&task_model));
                assistant.metadata.insert(
                    "scheduler_profile".to_string(),
                    serde_json::json!(profile_name.clone()),
                );
                assistant.metadata.insert(
                    "resolved_scheduler_profile".to_string(),
                    serde_json::json!(profile_name.clone()),
                );
                assistant.metadata.insert(
                    "resolved_execution_mode_kind".to_string(),
                    serde_json::json!(mode_kind),
                );
                assistant
                    .metadata
                    .insert("mode".to_string(), serde_json::json!(profile_name.clone()));
                assistant.metadata.insert(
                    "scheduler_applied".to_string(),
                    serde_json::json!(task_scheduler_applied),
                );
                match orchestrator_result {
                    Ok(output) => {
                        if output.is_cancelled() {
                            let _ =
                                finalize_active_scheduler_stage_cancelled(&task_state, &session_id)
                                    .await;
                            assistant.finish = Some("cancelled".to_string());
                            assistant.metadata.insert(
                                "finish_reason".to_string(),
                                serde_json::json!("cancelled"),
                            );
                        } else {
                            assistant.finish = Some("stop".to_string());
                        }
                        assistant.metadata.insert(
                            "scheduler_steps".to_string(),
                            serde_json::json!(output.steps),
                        );
                        assistant.metadata.insert(
                            "scheduler_tool_calls".to_string(),
                            serde_json::json!(output.tool_calls_count),
                        );
                        if let Some(usage) = output_usage(&output.metadata) {
                            let cost = task_model_pricing
                                .map(|p| {
                                    p.compute(
                                        usage.prompt_tokens,
                                        usage.completion_tokens,
                                        usage.cache_read_tokens,
                                        usage.cache_write_tokens,
                                    )
                                })
                                .unwrap_or(0.0);
                            assistant.usage = Some(rocode_session::MessageUsage {
                                input_tokens: usage.prompt_tokens,
                                output_tokens: usage.completion_tokens,
                                reasoning_tokens: usage.reasoning_tokens,
                                cache_read_tokens: usage.cache_read_tokens,
                                cache_write_tokens: usage.cache_write_tokens,
                                total_cost: cost,
                            });
                        }
                        assistant.add_text(visible_assistant_text_from_orchestrator_output(
                            &output.content,
                        ));
                    }
                    Err(error) => {
                        if is_scheduler_cancellation_error(&error) {
                            let _ =
                                finalize_active_scheduler_stage_cancelled(&task_state, &session_id)
                                    .await;
                            assistant.finish = Some("cancelled".to_string());
                            assistant.metadata.insert(
                                "finish_reason".to_string(),
                                serde_json::json!("cancelled"),
                            );
                            assistant.add_text("Scheduler cancelled.");
                        } else {
                            tracing::error!(
                                session_id = %session_id,
                                scheduler_profile = %profile_name,
                                %error,
                                "scheduler prompt failed"
                            );
                            assistant.finish = Some("error".to_string());
                            assistant
                                .metadata
                                .insert("error".to_string(), serde_json::json!(error.to_string()));
                            assistant.add_text(format!("Scheduler error: {}", error));
                        }
                    }
                }
            }
            ensure_default_session_title(&mut session, task_provider_client.clone(), &task_model)
                .await;
            // Propagate handoff metadata to session (outside message borrow).
            for (key, value) in handoff_entries {
                session.insert_metadata(key, value);
            }
            let session_usage = session.get_usage();
            session.touch();
            {
                let mut sessions = task_state.sessions.lock().await;
                sessions.update(session.clone());
            }
            let _ = task_state
                .runtime_telemetry
                .record_session_usage(&session_id, Some(&assistant_message_id), session_usage)
                .await;
            let assistant_text = session
                .get_message(&assistant_message_id)
                .map(assistant_visible_text)
                .unwrap_or_default();
            broadcast_session_updated(
                task_state.as_ref(),
                session_id.clone(),
                "prompt.scheduler.completed",
            );
            if let Some(output_hook) = output_block_hook.clone() {
                if !assistant_text.trim().is_empty() {
                    emit_output_block_via_hook(
                        Some(&output_hook),
                        rocode_session::prompt::OutputBlockEvent {
                            session_id: session_id.clone(),
                            block: OutputBlock::Message(MessageBlock::full(
                                OutputMessageRole::Assistant,
                                assistant_text,
                            )),
                            id: Some(assistant_message_id.clone()),
                        },
                    )
                    .await;
                }
            }
            persist_sessions_if_enabled(&task_state).await;
            return;
        }

        let (update_tx, mut update_rx) =
            tokio::sync::mpsc::unbounded_channel::<rocode_session::Session>();
        let update_state = task_state.clone();
        let update_session_repo = task_state.session_repo.clone();
        let update_message_repo = task_state.message_repo.clone();

        // Coalescing persistence worker — only persists the latest snapshot, not every tick.
        let persist_latest: Arc<tokio::sync::Mutex<Option<rocode_session::Session>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let persist_notify = Arc::new(Notify::new());
        let persist_worker = {
            let latest = persist_latest.clone();
            let notify = persist_notify.clone();
            let s_repo = update_session_repo.clone();
            let m_repo = update_message_repo.clone();
            tokio::spawn(async move {
                loop {
                    notify.notified().await;
                    // Drain: grab the latest snapshot, leaving None.
                    let snapshot = latest.lock().await.take();
                    let Some(snapshot) = snapshot else { continue };
                    if let (Some(s_repo), Some(m_repo)) = (&s_repo, &m_repo) {
                        match serde_json::to_value(&snapshot) {
                            Ok(val) => match serde_json::from_value::<rocode_types::Session>(val) {
                                Ok(mut stored) => {
                                    let messages = std::mem::take(&mut stored.messages);
                                    if let Err(e) = s_repo.upsert(&stored).await {
                                        tracing::warn!(session_id = %stored.id, %e, "incremental session upsert failed");
                                    }
                                    for msg in messages {
                                        if let Err(e) = m_repo.upsert(&msg).await {
                                            tracing::warn!(message_id = %msg.id, %e, "incremental message upsert failed");
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(session_id = %snapshot.id, %e, "incremental persist: failed to deserialize session snapshot");
                                }
                            },
                            Err(e) => {
                                tracing::warn!(session_id = %snapshot.id, %e, "incremental persist: failed to serialize session snapshot");
                            }
                        }
                    }
                }
            })
        };

        let mut update_task = tokio::spawn(async move {
            while let Some(snapshot) = update_rx.recv().await {
                {
                    let mut sessions = update_state.sessions.lock().await;
                    sessions.update(snapshot.clone());
                }

                *persist_latest.lock().await = Some(snapshot);
                persist_notify.notify_one();
            }
            persist_notify.notify_one();
        });
        // Keep persist_worker handle at this scope so the outer timeout path can abort it.
        let persist_worker_handle = persist_worker;
        let update_hook: rocode_session::SessionUpdateHook = Arc::new(move |snapshot| {
            let _ = update_tx.send(snapshot.clone());
        });

        let prompt_runner = task_state.prompt_runner.clone();
        let tool_defs = rocode_session::resolve_tools(task_state.tool_registry.as_ref()).await;
        let input = rocode_session::PromptInput {
            session_id: session_id.clone(),
            message_id: None,
            model: Some(rocode_session::prompt::ModelRef {
                provider_id: task_provider.clone(),
                model_id: task_model.clone(),
            }),
            agent: task_agent.clone(),
            no_reply: false,
            system: None,
            variant: task_variant.clone(),
            parts: task_prompt_parts.clone(),
            tools: None,
        };

        let agent_registry = AgentRegistry::from_config(&config);
        let agent_lookup: Option<rocode_session::prompt::AgentLookup> = {
            Some(Arc::new(move |name: &str| {
                agent_registry.get(name).map(to_task_agent_info)
            }))
        };

        let ask_question_hook: Option<rocode_session::prompt::AskQuestionHook> = {
            let state = task_state.clone();
            Some(Arc::new(move |session_id, questions| {
                let state = state.clone();
                Box::pin(
                    async move { request_question_answers(state, session_id, questions).await },
                )
            }))
        };
        let ask_permission_hook: Option<rocode_session::prompt::AskPermissionHook> = {
            let state = task_state.clone();
            Some(Arc::new(move |session_id, request| {
                let state = state.clone();
                Box::pin(async move { request_permission(state, session_id, request).await })
            }))
        };

        let event_broadcast: Option<rocode_session::prompt::EventBroadcastHook> = {
            let state = task_state.clone();
            Some(Arc::new(move |event| {
                if let Ok(server_event) = serde_json::from_value::<ServerEvent>(event) {
                    if let Some(payload) = server_event.to_json_string() {
                        state.broadcast(&payload);
                    } else {
                        tracing::warn!(
                            "failed to serialize ServerEvent from prompt event_broadcast"
                        );
                    }
                } else {
                    tracing::warn!("ignored non-ServerEvent payload in prompt event_broadcast");
                }
            }))
        };
        let publish_bus_hook: Option<rocode_session::prompt::PublishBusHook> = {
            let state = task_state.clone();
            let session_id = session_id.clone();
            Some(Arc::new(
                move |event_type: String, properties: serde_json::Value| {
                    let state = state.clone();
                    let session_id = session_id.clone();
                    Box::pin(async move {
                        match event_type.as_str() {
                            "agent_task.registered" => {
                                let task_id = properties["task_id"].as_str().unwrap_or_default();
                                let agent_name =
                                    properties["agent_name"].as_str().unwrap_or_default();
                                let parent_tool_call_id = properties["parent_tool_call_id"]
                                .as_str()
                                .map(
                                    crate::runtime_control::RuntimeControlRegistry::tool_call_execution_id,
                                );
                                let stage_id = if let Some(ref pid) = parent_tool_call_id {
                                    state.runtime_telemetry.resolve_stage_id(pid).await
                                } else {
                                    None
                                };
                                state
                                    .runtime_telemetry
                                    .register_agent_task(
                                        task_id,
                                        &session_id,
                                        agent_name,
                                        parent_tool_call_id,
                                        stage_id,
                                    )
                                    .await;
                            }
                            "agent_task.completed" => {
                                let task_id = properties["task_id"].as_str().unwrap_or_default();
                                state.runtime_telemetry.finish_agent_task(task_id).await;
                            }
                            _ => {}
                        }
                    }) as Pin<Box<dyn std::future::Future<Output = ()> + Send>>
                },
            ))
        };

        if let Err(error) = prompt_runner
            .prompt_with_update_hook(
                input,
                &mut session,
                rocode_session::prompt::PromptRequestContext {
                    provider,
                    system_prompt: task_system_prompt.clone(),
                    memory_prefetch: memory_prefetch_packet.clone(),
                    tools: tool_defs,
                    compiled_request: task_compiled_request.clone(),
                    hooks: rocode_session::prompt::PromptHooks {
                        update_hook: Some(update_hook),
                        event_broadcast,
                        output_block_hook,
                        agent_lookup,
                        ask_question_hook,
                        ask_permission_hook,
                        publish_bus_hook,
                    },
                },
            )
            .await
        {
            tracing::error!(
                session_id = %session_id,
                provider_id = %task_provider,
                model_id = %task_model,
                %error,
                "session prompt failed"
            );
            let assistant = session.add_assistant_message();
            assistant.finish = Some("error".to_string());
            assistant
                .metadata
                .insert("error".to_string(), serde_json::json!(error.to_string()));
            assistant
                .metadata
                .insert("finish_reason".to_string(), serde_json::json!("error"));
            assistant.metadata.insert(
                "model_provider".to_string(),
                serde_json::json!(&task_provider),
            );
            assistant
                .metadata
                .insert("model_id".to_string(), serde_json::json!(&task_model));
            if let Some(agent) = task_agent.as_deref() {
                assistant
                    .metadata
                    .insert("agent".to_string(), serde_json::json!(agent));
            }
            assistant.add_text(format!("Provider error: {}", error));
        }
        match tokio::time::timeout(Duration::from_secs(1), &mut update_task).await {
            Ok(joined) => {
                let _ = joined;
            }
            Err(_) => {
                update_task.abort();
                tracing::warn!(
                    session_id = %session_id,
                    "timed out waiting for prompt update task shutdown; aborted task"
                );
            }
        }
        // Always clean up the persist worker — it may still be alive if update_task was aborted.
        // Give it a brief window to flush the last queued snapshot, then abort.
        if !persist_worker_handle.is_finished() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        persist_worker_handle.abort();

        let latest_assistant_message_id = session
            .messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, rocode_session::MessageRole::Assistant))
            .map(|message| message.id.clone());
        if let Some(explain) = task_multimodal_explain.as_ref() {
            annotate_last_user_message_multimodal_metadata(&mut session, explain);
        }
        let _ = task_state
            .runtime_telemetry
            .record_session_usage(
                &session_id,
                latest_assistant_message_id.as_deref(),
                session.get_usage(),
            )
            .await;
        persist_session_telemetry_metadata(&task_state, &mut session).await;
        {
            let mut sessions = task_state.sessions.lock().await;
            sessions.update(session.clone());
        }
        broadcast_session_updated(task_state.as_ref(), session_id.clone(), "prompt.final");
        // Normal path reached — defuse the guard so we handle cleanup explicitly.
        _idle_guard.defuse();
        set_session_run_status(&task_state, &session_id, SessionRunStatus::Idle).await;
        // Only flush the current session — full sync is deferred to shutdown/startup.
        if let Err(err) = task_state.flush_session_to_storage(&session_id).await {
            tracing::error!(session_id = %session_id, %err, "failed to flush session to storage");
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
    use rocode_command::{CommandArgumentOption, CommandRegistry};
    use rocode_config::Config as AppConfig;
    use rocode_multimodal::{
        ModalityPreflightResult, ModalitySupportView, ModalityTransportResult,
        MultimodalDisplaySummary, PreflightCapabilityView, RuntimeMultimodalExplain,
    };
    use rocode_session::{PartType, Session, SessionStateManager};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn test_prompt_runner() -> rocode_session::SessionPrompt {
        rocode_session::SessionPrompt::new(Arc::new(RwLock::new(SessionStateManager::new())))
    }

    fn text_parts(message: &rocode_session::SessionMessage) -> Vec<&str> {
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
    async fn scheduler_user_message_preserves_attachment_only_parts() {
        let prompt_runner = test_prompt_runner();
        let mut session = Session::new("project", "/tmp");
        let input = rocode_session::PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            parts: vec![rocode_session::PartInput::File {
                url: "data:text/plain;base64,SGVsbG8=".to_string(),
                filename: Some("note.txt".to_string()),
                mime: Some("text/plain".to_string()),
            }],
            tools: None,
        };

        let message_id = create_scheduler_user_message(
            &prompt_runner,
            &mut session,
            &input,
            SchedulerUserMessageContext {
                display_prompt_text: "[1 attachment]",
                resolved_user_prompt: "",
                profile_name: "atlas",
                mode_kind: "preset",
                resolved_system_prompt: "You are Atlas.",
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
            message.metadata.get("resolved_scheduler_profile"),
            Some(&serde_json::json!("atlas"))
        );
    }

    #[tokio::test]
    async fn scheduler_user_message_keeps_text_and_file_parts_together() {
        let prompt_runner = test_prompt_runner();
        let mut session = Session::new("project", "/tmp");
        let input = rocode_session::PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            parts: vec![
                rocode_session::PartInput::Text {
                    text: "Inspect @note.txt".to_string(),
                },
                rocode_session::PartInput::File {
                    url: "data:text/plain;base64,SGVsbG8=".to_string(),
                    filename: Some("note.txt".to_string()),
                    mime: Some("text/plain".to_string()),
                },
            ],
            tools: None,
        };

        let message_id = create_scheduler_user_message(
            &prompt_runner,
            &mut session,
            &input,
            SchedulerUserMessageContext {
                display_prompt_text: "Inspect @note.txt",
                resolved_user_prompt: "Inspect @note.txt",
                profile_name: "atlas",
                mode_kind: "preset",
                resolved_system_prompt: "You are Atlas.",
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
            .rfind(|message| matches!(message.role, rocode_session::MessageRole::User))
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
    fn hydrate_scheduler_command_arguments_uses_workflow_defaults_for_autoresearch() {
        let registry = CommandRegistry::new();
        let command = registry.get("autoresearch").expect("autoresearch command");
        let invocation = command
            .invocation
            .as_ref()
            .expect("autoresearch invocation");

        let (arguments, raw_arguments) = hydrate_scheduler_command_arguments(
            &AppConfig::default(),
            command,
            None,
            "",
            &invocation.argument_schema,
        )
        .expect("workflow defaults should hydrate autoresearch command");

        assert_eq!(
            arguments.get("verify"),
            Some(&vec!["bash ./scripts/verify-autoresearch.sh".to_string()])
        );
        assert_eq!(arguments.get("iterations"), Some(&vec!["6".to_string()]));
        assert_eq!(
            arguments.get("scope"),
            Some(&vec![
                "crates/**".to_string(),
                "scripts/**".to_string(),
                "Cargo.toml".to_string(),
                "Cargo.lock".to_string(),
            ])
        );
        assert!(
            arguments
                .get("goal")
                .and_then(|values| values.first())
                .is_some_and(|value| value.contains("Increase the curated regression score")),
            "workflow goal should hydrate command defaults"
        );
        assert!(
            arguments
                .get("metric")
                .and_then(|values| values.first())
                .is_some_and(|value| value.contains("\"kind\":\"numeric-extract\"")),
            "workflow metric should hydrate command defaults"
        );
        assert!(raw_arguments.contains("--verify \"bash ./scripts/verify-autoresearch.sh\""));
        assert!(raw_arguments.contains("--iterations 6"));
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
            &AppConfig::default(),
            command,
            None,
            "--goal \"teacher demo goal\" --verify ./custom-verify.sh",
            &invocation.argument_schema,
        )
        .expect("workflow defaults should merge with explicit arguments");

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
        assert!(raw_arguments.contains("--guard"));
        assert!(raw_arguments.contains("--iterations 6"));
    }
}
