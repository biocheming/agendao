// Tool execution + subsession methods for SessionPrompt

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use rocode_orchestrator::inline_subtask_request_defaults;
use rocode_provider::{Provider, ToolDefinition};
use rocode_types::{
    RepairEvent, SubsessionHandoffFieldKind, SubsessionHandoffPacket, SubsessionHandoffRichness,
    SubsessionResultEnvelope, ToolBatchSummary,
};

use crate::{FilePart, MessageRole, PartType, Session, SessionMessage};

use super::subtask::SubtaskExecutor;
use super::{
    AgentLookup, AgentParams, AskPermissionHook, AskQuestionHook, ModelRef, PersistedSubsession,
    PersistedSubsessionTurn, PromptHooks, SessionPrompt,
};

#[derive(Debug, Clone)]
struct PendingSyntheticMessage {
    agent: Option<String>,
    text: String,
    attachments: Vec<rocode_tool::SyntheticAttachment>,
}

#[derive(Clone)]
struct ToolExecutionOptions {
    provider_id: String,
    model_id: String,
    hooks: PromptHooks,
    repair_policy: rocode_types::RepairPolicy,
    tool_result_budget: crate::tool_result_governance::ToolResultBudget,
}

const MAX_PERSISTED_SUBSESSION_HISTORY_TURNS: usize = 8;
const MAX_SUBSESSION_HANDOFF_TAIL_FIELDS: usize = 3;
const MAX_SUBSESSION_FIELD_CHARS: usize = 4_000;
const MAX_SUBSESSION_TAIL_FIELD_CHARS: usize = 1_200;

// ── P2.2: Tool batch fact extraction ────────────────────────────────────

struct ToolCallBatchFact {
    tool_name: String,
    is_error: bool,
    error_kind: Option<String>,
    block_reason: Option<rocode_types::ToolBatchBlockReason>,
    artifacts_created: Vec<String>,
    repair_events: Vec<RepairEvent>,
    suggested_follow_up: Vec<rocode_types::ToolBatchFollowUpItem>,
    unresolved_items: Vec<String>,
}

fn collect_tool_batch_facts(assistant_msg: &SessionMessage) -> Vec<ToolCallBatchFact> {
    assistant_msg
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::ToolCall {
                name,
                status,
                state,
                ..
            } => {
                let is_error = matches!(status, crate::ToolCallStatus::Error);
                let error_kind = if is_error {
                    state.as_ref().and_then(|s| match s {
                        crate::ToolState::Error { error, .. } => Some(classify_error_kind(error)),
                        _ => None,
                    })
                } else {
                    None
                };
                let repair_events = state
                    .as_ref()
                    .and_then(|s| match s {
                        crate::ToolState::Completed { metadata, .. }
                        | crate::ToolState::Error {
                            metadata: Some(metadata),
                            ..
                        } => Some(rocode_tool::structured_repair_events(metadata)),
                        _ => None,
                    })
                    .unwrap_or_default();
                let block_reason = classify_block_reason(error_kind.as_deref(), &repair_events);
                let (artifacts_created, suggested_follow_up, unresolved_items) =
                    extract_tool_fact_extras(name, is_error, error_kind.as_deref(), state.as_ref());
                Some(ToolCallBatchFact {
                    tool_name: name.clone(),
                    is_error,
                    error_kind,
                    block_reason,
                    artifacts_created,
                    repair_events,
                    suggested_follow_up,
                    unresolved_items,
                })
            }
            _ => None,
        })
        .collect()
}

fn extract_tool_fact_extras(
    name: &str,
    _is_error: bool,
    error_kind: Option<&str>,
    state: Option<&crate::ToolState>,
) -> (
    Vec<String>,
    Vec<rocode_types::ToolBatchFollowUpItem>,
    Vec<String>,
) {
    let mut artifacts = Vec::new();
    let mut follow_up = Vec::new();
    let mut unresolved = Vec::new();

    if let Some(state) = state {
        match state {
            crate::ToolState::Completed {
                output,
                attachments,
                ..
            } => {
                if let Some(attachments) = attachments {
                    for a in attachments {
                        if let Some(ref filename) = a.filename {
                            artifacts.push(filename.clone());
                        } else if !a.url.is_empty() {
                            artifacts.push(a.url.clone());
                        }
                    }
                }
                let trimmed = output.trim();
                if trimmed.starts_with('/') && !trimmed.contains('\n') && trimmed.len() < 256 {
                    artifacts.push(trimmed.to_string());
                }
            }
            crate::ToolState::Error { error, .. } => {
                unresolved.push(format!("{name} call failed: {error}"));
                match error_kind {
                    Some("invalid_arguments") => {
                        follow_up.push(rocode_types::ToolBatchFollowUpItem {
                            kind: "fix_args".into(),
                            text: format!("{name}: fix arguments and retry"),
                        });
                    }
                    Some("permission_denied") => {
                        follow_up.push(rocode_types::ToolBatchFollowUpItem {
                            kind: "ask_permission".into(),
                            text: format!("{name}: request permission or use alternative"),
                        });
                    }
                    Some("timeout") => {
                        follow_up.push(rocode_types::ToolBatchFollowUpItem {
                            kind: "retry_narrower".into(),
                            text: format!("{name}: retry with a narrower operation"),
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    (artifacts, follow_up, unresolved)
}

fn classify_block_reason(
    error_kind: Option<&str>,
    repair_events: &[RepairEvent],
) -> Option<rocode_types::ToolBatchBlockReason> {
    let from_error = match error_kind {
        Some("invalid_arguments") => Some(rocode_types::ToolBatchBlockReason::InvalidArguments),
        Some("permission_denied") => Some(rocode_types::ToolBatchBlockReason::PermissionDenied),
        Some("timeout") => Some(rocode_types::ToolBatchBlockReason::Timeout),
        Some("provider_rejected") => Some(rocode_types::ToolBatchBlockReason::ProviderRejected),
        Some("user_input_required") => Some(rocode_types::ToolBatchBlockReason::UserInputRequired),
        Some("execution_error") => Some(rocode_types::ToolBatchBlockReason::ToolExecutionError),
        Some(_) => Some(rocode_types::ToolBatchBlockReason::Unknown),
        None => None,
    };
    if from_error.is_some() {
        return from_error;
    }
    // When a tool call was permissively rerouted, infer the block reason
    // from repair events that carry the original error classification.
    for e in repair_events {
        let Some(kind) = rocode_types::RepairKind::from_legacy_str(&e.repair_kind) else {
            continue;
        };
        match kind {
            rocode_types::RepairKind::ArgumentPrevalidationFallback => {
                return Some(rocode_types::ToolBatchBlockReason::InvalidArguments);
            }
            rocode_types::RepairKind::InvalidToolReroute => {
                // Read the structured original_error_kind field directly.
                let error_kind = e
                    .original_error_kind
                    .as_deref()
                    .unwrap_or("execution_error");
                return classify_block_reason(Some(error_kind), &[]);
            }
            _ => {}
        }
    }
    None
}

fn derive_goal_status(facts: &[ToolCallBatchFact]) -> rocode_types::ToolBatchGoalStatus {
    let has_error = facts.iter().any(|f| f.is_error);
    let has_blocker = facts.iter().any(|f| f.block_reason.is_some());
    // Count calls that actually succeeded (no error AND no blocker).
    let real_success_count = facts
        .iter()
        .filter(|f| !f.is_error && f.block_reason.is_none())
        .count();
    let total = facts.len();

    if real_success_count == total {
        return rocode_types::ToolBatchGoalStatus::Advanced;
    }
    if real_success_count > 0 && has_error {
        return rocode_types::ToolBatchGoalStatus::Mixed;
    }
    if real_success_count > 0 && has_blocker {
        return rocode_types::ToolBatchGoalStatus::Mixed;
    }
    if has_blocker {
        return rocode_types::ToolBatchGoalStatus::Blocked;
    }
    if has_error {
        return rocode_types::ToolBatchGoalStatus::NoProgress;
    }
    rocode_types::ToolBatchGoalStatus::NoProgress
}

fn derive_recommended_next_step(facts: &[ToolCallBatchFact]) -> Option<String> {
    let has_success = facts.iter().any(|f| !f.is_error);
    let has_artifact = facts.iter().any(|f| !f.artifacts_created.is_empty());
    let all_have_blockers = facts.iter().all(|f| f.block_reason.is_some());
    let has_invalid_args = facts
        .iter()
        .any(|f| f.block_reason == Some(rocode_types::ToolBatchBlockReason::InvalidArguments));
    let has_permission = facts
        .iter()
        .any(|f| f.block_reason == Some(rocode_types::ToolBatchBlockReason::PermissionDenied));
    let has_timeout = facts
        .iter()
        .any(|f| f.block_reason == Some(rocode_types::ToolBatchBlockReason::Timeout));

    // All calls are blocked (even if permissively rerouted): give blocker-specific advice.
    if all_have_blockers {
        if has_invalid_args {
            return Some("fix tool arguments before retrying".into());
        }
        if has_permission {
            return Some("request permission or choose a non-privileged path".into());
        }
        if has_timeout {
            return Some("retry with a narrower or cheaper operation".into());
        }
    }
    if has_success && has_artifact && !has_invalid_args {
        return Some("continue from successful outputs".into());
    }
    if has_success && has_invalid_args {
        return Some("continue from successful outputs and fix the failed calls".into());
    }
    None
}

#[derive(Clone)]
pub(super) struct PersistedSubsessionPromptOptions {
    pub(super) default_model: String,
    pub(super) fallback_directory: Option<String>,
    pub(super) hooks: PromptHooks,
    pub(super) question_session_id: Option<String>,
    pub(super) abort: Option<CancellationToken>,
    pub(super) tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    pub(super) config_store: Option<Arc<rocode_config::ConfigStore>>,
}

impl SessionPrompt {
    pub async fn execute_tool_calls(
        session: &mut Session,
        tool_registry: Arc<rocode_tool::ToolRegistry>,
        ctx: rocode_tool::ToolContext,
        provider: Arc<dyn Provider>,
        provider_id: &str,
        model_id: &str,
    ) -> anyhow::Result<()> {
        let repair_policy = crate::compaction::effective_repair_policy(ctx.config_store.as_deref());
        let tool_result_budget = crate::tool_result_governance::tool_result_budget(
            ctx.config_store
                .as_ref()
                .map(|store| store.config())
                .as_deref()
                .and_then(|cfg| cfg.runtime_budget.as_ref()),
        );
        Self::execute_tool_calls_with_hook(
            session,
            tool_registry,
            ctx,
            provider,
            ToolExecutionOptions {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                hooks: PromptHooks::default(),
                repair_policy,
                tool_result_budget,
            },
        )
        .await?;
        Ok(())
    }

    async fn execute_tool_calls_with_hook(
        session: &mut Session,
        tool_registry: Arc<rocode_tool::ToolRegistry>,
        ctx: rocode_tool::ToolContext,
        provider: Arc<dyn Provider>,
        options: ToolExecutionOptions,
    ) -> anyhow::Result<usize> {
        let Some(last_assistant_index) = session
            .messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::Assistant))
        else {
            return Ok(0);
        };

        let resolved_call_ids: HashSet<String> = session
            .messages
            .iter()
            .skip(last_assistant_index + 1)
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .collect();

        let tool_calls: Vec<(String, String, serde_json::Value, serde_json::Value)> = session
            .messages[last_assistant_index]
            .parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::ToolCall {
                    id,
                    name,
                    input,
                    status,
                    raw,
                    state,
                    ..
                } if !resolved_call_ids.contains(id) && !name.trim().is_empty() => {
                    Self::tool_call_input_for_execution(
                        status,
                        input,
                        raw.as_deref(),
                        state.as_ref(),
                    )
                    .map(|args| {
                        (
                            id.clone(),
                            name.clone(),
                            args,
                            Self::tool_call_raw_shape_for_execution(
                                input,
                                raw.as_deref(),
                                state.as_ref(),
                            ),
                        )
                    })
                }
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            return Ok(0);
        }

        if let Some(assistant_msg) = session.messages_mut().get_mut(last_assistant_index) {
            for (call_id, tool_name, input, _) in &tool_calls {
                Self::upsert_tool_call_part(
                    assistant_msg,
                    call_id,
                    Some(tool_name),
                    Some(input.clone()),
                    None,
                    Some(crate::ToolCallStatus::Running),
                    Some(crate::ToolState::Running {
                        input: input.clone(),
                        title: None,
                        metadata: None,
                        time: crate::RunningTime {
                            start: chrono::Utc::now().timestamp_millis(),
                        },
                    }),
                );
            }
        }

        // Emit update so TUI shows tools in "Running" state immediately.
        Self::emit_session_update(options.hooks.update_hook.as_ref(), session);

        let subsessions = Arc::new(Mutex::new(Self::load_persisted_subsessions(session)));
        let pending_synthetic_messages =
            Arc::new(Mutex::new(Vec::<PendingSyntheticMessage>::new()));
        let default_model = format!("{}:{}", options.provider_id, options.model_id);
        let ctx = Self::with_persistent_subsession_callbacks(
            ctx,
            subsessions.clone(),
            provider,
            tool_registry.clone(),
            default_model,
            options.hooks.agent_lookup.clone(),
            options.hooks.ask_question_hook.clone(),
            options.hooks.ask_permission_hook.clone(),
        )
        .with_create_synthetic_message({
            let pending_synthetic_messages = pending_synthetic_messages.clone();
            move |_session_id, agent, text, attachments| {
                let pending_synthetic_messages = pending_synthetic_messages.clone();
                async move {
                    pending_synthetic_messages
                        .lock()
                        .await
                        .push(PendingSyntheticMessage {
                            agent,
                            text,
                            attachments,
                        });
                    Ok(())
                }
            }
        })
        .with_registry(tool_registry.clone());
        let available_tool_ids: HashSet<String> =
            tool_registry.list_ids().await.into_iter().collect();

        let mut executed_calls = 0usize;
        let tool_results_msg = {
            let mut msg = SessionMessage::tool(ctx.session_id.clone());
            for (call_id, tool_name, input, raw_shape) in tool_calls {
                tracing::info!(
                    tool_call_id = %call_id,
                    tool_name = %tool_name,
                    input_type = %if input.is_object() { "object" } else if input.is_string() { "string" } else { "other" },
                    input_keys = %if input.is_object() {
                        input.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(",")).unwrap_or_default()
                    } else {
                        input.to_string().chars().take(120).collect::<String>()
                    },
                    "[DIAG] executing tool call"
                );
                let mut tool_ctx = ctx.clone();
                tool_ctx.call_id = Some(call_id.clone());
                let repaired_tool_name =
                    Self::repair_tool_call_name(&tool_name, &available_tool_ids);
                let mut repair_metadata = rocode_tool::Metadata::new();
                if repaired_tool_name != tool_name {
                    let mut event = rocode_tool::tool_repair_event(
                        rocode_types::RepairKind::ToolNameRepair.as_str(),
                        "session_prompt",
                        &repaired_tool_name,
                    );
                    event.insert("from".to_string(), serde_json::json!(tool_name));
                    event.insert("to".to_string(), serde_json::json!(repaired_tool_name));
                    event.insert(
                        "reason".to_string(),
                        serde_json::json!("case_insensitive_exact_match"),
                    );
                    rocode_tool::append_tool_repair_event_map(&mut repair_metadata, event);
                }
                let mut effective_tool_name = repaired_tool_name.clone();
                let mut effective_input = input.clone();
                let (normalized_input, normalization_telemetry) =
                    rocode_tool::normalize_tool_arguments_with_telemetry(
                        &effective_tool_name,
                        effective_input,
                    );
                effective_input = normalized_input;
                if !normalization_telemetry.is_empty() {
                    let mut event = rocode_tool::tool_repair_event(
                        rocode_types::RepairKind::ArgumentNormalization.as_str(),
                        "session_prompt",
                        &effective_tool_name,
                    );
                    event.insert(
                        "modes".to_string(),
                        serde_json::json!(normalization_telemetry.modes),
                    );
                    // P1.1: Record the raw model output and the normalized execution args.
                    event.insert("raw_shape".to_string(), raw_shape.clone());
                    event.insert("normalized_shape".to_string(), effective_input.clone());
                    rocode_tool::append_tool_repair_event_map(&mut repair_metadata, event);
                }
                let mut strict_prevalidation_error: Option<String> = None;
                if let Some(payload) =
                    Self::prevalidate_tool_arguments(&effective_tool_name, &effective_input)
                {
                    let is_strict =
                        matches!(options.repair_policy, rocode_types::RepairPolicy::Strict);
                    tracing::warn!(
                        tool_name = %tool_name,
                        normalized_tool = %effective_tool_name,
                        policy = %options.repair_policy.label(),
                        "tool arguments failed prevalidation"
                    );
                    let mut event = rocode_tool::tool_repair_event(
                        rocode_types::RepairKind::ArgumentPrevalidationFallback.as_str(),
                        "session_prompt",
                        &effective_tool_name,
                    );
                    if let Some(reason) = payload.get("error").and_then(|value| value.as_str()) {
                        event.insert("reason".to_string(), serde_json::json!(reason));
                    }
                    if let Some(received_args) = payload.get("receivedArgs") {
                        event.insert("receivedArgs".to_string(), received_args.clone());
                    }
                    // P1.1: record the raw model output and the replacement payload.
                    event.insert("raw_shape".to_string(), raw_shape.clone());
                    event.insert("normalized_shape".to_string(), payload.clone());
                    if is_strict {
                        event.insert(
                            "strict_mode_would_fail".to_string(),
                            serde_json::json!(true),
                        );
                    }
                    rocode_tool::append_tool_repair_event_map(&mut repair_metadata, event);
                    if is_strict {
                        // Strict: do not rewrite the execution input or reroute
                        // through the invalid tool. Record the failure and stop
                        // before executing the tool.
                        strict_prevalidation_error = payload
                            .get("error")
                            .and_then(|value| value.as_str())
                            .map(ToOwned::to_owned)
                            .or_else(|| Some("Tool arguments failed prevalidation".to_string()));
                    } else {
                        // Permissive: reroute to invalid tool for a helpful error message.
                        effective_tool_name = "invalid".to_string();
                        effective_input = payload;
                    }
                }

                let (content, is_error, title, metadata, attachments, state_attachments) =
                    match strict_prevalidation_error {
                        Some(error) => (
                            format!("Invalid arguments: {}", error),
                            true,
                            Some("Tool Error".to_string()),
                            (!rocode_tool::tool_repair_events(&repair_metadata).is_empty())
                                .then_some(repair_metadata.clone()),
                            None,
                            None,
                        ),
                        None => {
                            let execution = tool_registry
                                .execute(
                                    &effective_tool_name,
                                    effective_input.clone(),
                                    tool_ctx.clone(),
                                )
                                .await;
                            match execution {
                                Ok(result) => {
                                    let mut metadata = result.metadata;
                                    rocode_tool::merge_tool_repair_telemetry(
                                        &mut metadata,
                                        &repair_metadata,
                                    );
                                    let (attachments, state_attachments) =
                                        Self::extract_tool_attachments_from_metadata(
                                            &mut metadata,
                                            &ctx.session_id,
                                            &ctx.message_id,
                                        );
                                    (
                                        result.output,
                                        false,
                                        Some(result.title),
                                        Some(metadata),
                                        attachments,
                                        state_attachments,
                                    )
                                }
                                Err(e) => {
                                    let is_strict = matches!(
                                        options.repair_policy,
                                        rocode_types::RepairPolicy::Strict
                                    );
                                    // Permissive: reroute to invalid for machine-readable errors.
                                    // Strict: return the raw error, don't silently rewrite.
                                    if !is_strict && available_tool_ids.contains("invalid") {
                                        // Record the reroute as a repair event for telemetry.
                                        let error_text = format!("Error: {}", e);
                                        let original_kind = classify_error_kind(&error_text);
                                        let mut reroute_event = rocode_tool::tool_repair_event(
                                            rocode_types::RepairKind::InvalidToolReroute.as_str(),
                                            "session_prompt",
                                            &effective_tool_name,
                                        );
                                        reroute_event.insert(
                                            "reason".to_string(),
                                            serde_json::json!(error_text),
                                        );
                                        reroute_event.insert(
                                            "original_error_kind".to_string(),
                                            serde_json::json!(original_kind),
                                        );
                                        rocode_tool::append_tool_repair_event_map(
                                            &mut repair_metadata,
                                            reroute_event,
                                        );
                                        let invalid_input = Self::invalid_tool_payload(
                                            &tool_name,
                                            &format!("Error: {}", e),
                                        );
                                        let fallback_execution = tool_registry
                                            .execute(
                                                "invalid",
                                                invalid_input.clone(),
                                                tool_ctx.clone(),
                                            )
                                            .await;
                                        match fallback_execution {
                                            Ok(result) => {
                                                effective_tool_name = "invalid".to_string();
                                                effective_input = invalid_input;
                                                let mut metadata = result.metadata;
                                                rocode_tool::merge_tool_repair_telemetry(
                                                    &mut metadata,
                                                    &repair_metadata,
                                                );
                                                let (attachments, state_attachments) =
                                                    Self::extract_tool_attachments_from_metadata(
                                                        &mut metadata,
                                                        &ctx.session_id,
                                                        &ctx.message_id,
                                                    );
                                                (
                                                    result.output,
                                                    false,
                                                    Some(result.title),
                                                    Some(metadata),
                                                    attachments,
                                                    state_attachments,
                                                )
                                            }
                                            Err(fallback_err) => (
                                                format!(
                                            "Tool {} failed: {}. Invalid fallback also failed: {}",
                                            tool_name, e, fallback_err
                                        ),
                                                true,
                                                Some("Tool Error".to_string()),
                                                (!rocode_tool::tool_repair_events(
                                                    &repair_metadata,
                                                )
                                                .is_empty())
                                                .then_some(repair_metadata.clone()),
                                                None,
                                                None,
                                            ),
                                        }
                                    } else {
                                        // Strict mode (or no invalid tool): return the raw error.
                                        if is_strict {
                                            let mut event = rocode_tool::tool_repair_event(
                                                rocode_types::RepairKind::ExecutionErrorNoReroute
                                                    .as_str(),
                                                "session_prompt",
                                                &effective_tool_name,
                                            );
                                            event.insert(
                                                "reason".to_string(),
                                                serde_json::json!(format!("Error: {}", e)),
                                            );
                                            // P1.1: record the failing input shape.
                                            event
                                                .insert("raw_shape".to_string(), raw_shape.clone());
                                            event.insert(
                                                "normalized_shape".to_string(),
                                                effective_input.clone(),
                                            );
                                            event.insert(
                                                "strict_mode_would_fail".to_string(),
                                                serde_json::json!(true),
                                            );
                                            rocode_tool::append_tool_repair_event_map(
                                                &mut repair_metadata,
                                                event,
                                            );
                                        }
                                        (
                                            format!("Error: {}", e),
                                            true,
                                            Some("Tool Error".to_string()),
                                            (!rocode_tool::tool_repair_events(&repair_metadata)
                                                .is_empty())
                                            .then_some(repair_metadata.clone()),
                                            None,
                                            None,
                                        )
                                    }
                                }
                            }
                        }
                    };
                let history_input = Self::sanitize_tool_call_input_for_history(
                    &effective_tool_name,
                    &effective_input,
                    if is_error {
                        Some(content.as_str())
                    } else {
                        None
                    },
                );

                // P2-4: govern large tool results before they enter the transcript.
                // Raw full content is artifact-backed; the transcript holds a governed preview.
                let artifacts_root =
                    crate::tool_result_governance::default_tool_result_artifacts_root(
                        &session.record().directory,
                    );
                let mut gov_metadata = metadata.clone().unwrap_or_default();
                let governed = crate::tool_result_governance::govern_tool_result_output(
                    &session.id,
                    &call_id,
                    content.clone(),
                    &mut gov_metadata,
                    &artifacts_root,
                    options.tool_result_budget,
                );

                Self::push_tool_result_part(
                    &mut msg,
                    call_id.clone(),
                    governed.output,
                    is_error,
                    title.clone(),
                    Some(gov_metadata),
                    attachments.clone(),
                );
                executed_calls += 1;

                if let Some(assistant_msg) = session.messages_mut().get_mut(last_assistant_index) {
                    let now = chrono::Utc::now().timestamp_millis();
                    let next_state = if is_error {
                        crate::ToolState::Error {
                            input: history_input.clone(),
                            error: content.clone(),
                            metadata: metadata.clone(),
                            time: crate::ErrorTime {
                                start: now,
                                end: now,
                            },
                        }
                    } else {
                        crate::ToolState::Completed {
                            input: history_input.clone(),
                            output: content.clone(),
                            title: title.clone().unwrap_or_else(|| "Tool Result".to_string()),
                            metadata: metadata.clone().unwrap_or_default(),
                            time: crate::CompletedTime {
                                start: now,
                                end: now,
                                compacted: None,
                            },
                            attachments: state_attachments.clone(),
                        }
                    };
                    Self::upsert_tool_call_part(
                        assistant_msg,
                        &call_id,
                        Some(&effective_tool_name),
                        Some(history_input),
                        None,
                        Some(if is_error {
                            crate::ToolCallStatus::Error
                        } else {
                            crate::ToolCallStatus::Completed
                        }),
                        Some(next_state),
                    );
                }

                // Emit update after each tool completes so TUI renders results incrementally.
                Self::emit_session_update(options.hooks.update_hook.as_ref(), session);
            }
            msg
        };

        if !tool_results_msg.parts.is_empty() {
            session.push_message(tool_results_msg);
        }

        // Capture synthetic attachment filenames before the messages are consumed.
        let synthetic_artifacts: Vec<String> = {
            let pending = pending_synthetic_messages.lock().await;
            pending
                .iter()
                .flat_map(|m| {
                    m.attachments.iter().filter_map(|a| {
                        a.filename.clone().or_else(|| {
                            if a.url.is_empty() {
                                None
                            } else {
                                Some(a.url.clone())
                            }
                        })
                    })
                })
                .collect()
        };

        let pending_synthetic_messages = {
            let mut pending = pending_synthetic_messages.lock().await;
            std::mem::take(&mut *pending)
        };
        if !pending_synthetic_messages.is_empty() {
            for message in pending_synthetic_messages {
                Self::append_synthetic_user_message(session, message);
            }
            Self::emit_session_update(options.hooks.update_hook.as_ref(), session);
        }

        let persisted = subsessions.lock().await.clone();
        Self::save_persisted_subsessions(session, &persisted);

        // Build and persist a tool batch summary for telemetry / compaction.
        if executed_calls > 0 {
            let summary = session
                .messages
                .get(last_assistant_index)
                .and_then(|msg| Self::build_tool_batch_summary(msg, &synthetic_artifacts));
            if let Some(summary) = summary {
                session.insert_metadata(
                    "latest_tool_batch_summary".to_string(),
                    serde_json::to_value(&summary).unwrap_or_default(),
                );
            }
        }

        Ok(executed_calls)
    }

    /// Build a structured `ToolBatchSummary` from the completed tool calls in
    /// an assistant message, enriched with pending synthetic attachment info.
    pub(super) fn build_tool_batch_summary(
        assistant_msg: &SessionMessage,
        synthetic_artifacts: &[String],
    ) -> Option<ToolBatchSummary> {
        let facts = collect_tool_batch_facts(assistant_msg);
        if facts.is_empty() {
            return None;
        }

        let success_count = facts.iter().filter(|f| !f.is_error).count() as u32;
        let error_count = facts.iter().filter(|f| f.is_error).count() as u32;
        let tools_used = {
            let mut names: Vec<String> = facts.iter().map(|f| f.tool_name.clone()).collect();
            names.sort();
            names.dedup();
            names
        };
        let error_kinds = {
            let mut kinds: Vec<String> =
                facts.iter().filter_map(|f| f.error_kind.clone()).collect();
            kinds.sort();
            kinds.dedup();
            kinds
        };
        let blocked_by: Vec<rocode_types::ToolBatchBlockReason> = {
            let mut reasons: Vec<rocode_types::ToolBatchBlockReason> =
                facts.iter().filter_map(|f| f.block_reason).collect();
            reasons.sort_by_key(|r| r.as_str());
            reasons.dedup_by_key(|r| r.as_str());
            reasons
        };
        let repair_events: Vec<RepairEvent> =
            facts.iter().flat_map(|f| f.repair_events.clone()).collect();
        let goal_status = derive_goal_status(&facts);
        let recommended_next_step = derive_recommended_next_step(&facts);

        let mut artifacts_created: Vec<String> = facts
            .iter()
            .flat_map(|f| f.artifacts_created.clone())
            .collect();
        artifacts_created.extend(synthetic_artifacts.iter().cloned());
        artifacts_created.sort();
        artifacts_created.dedup();

        let pending_follow_up: Vec<rocode_types::ToolBatchFollowUpItem> = facts
            .iter()
            .flat_map(|f| f.suggested_follow_up.clone())
            .collect();

        let unresolved_items: Vec<String> = facts
            .iter()
            .flat_map(|f| f.unresolved_items.clone())
            .collect();

        Some(ToolBatchSummary {
            tools_used,
            success_count,
            error_count,
            error_kinds,
            goal_status,
            blocked_by,
            artifacts_created,
            pending_follow_up,
            unresolved_items,
            recommended_next_step,
            repair_events,
        })
    }

    /// Read the latest tool batch summary from session metadata and inject it
    /// into the chat messages as a compact model-visible context block (P0.4).
    pub(super) fn inject_latest_tool_batch_summary(
        session: &mut Session,
        chat_messages: &mut Vec<rocode_provider::Message>,
    ) {
        let Some(summary_value) = session.remove_metadata("latest_tool_batch_summary") else {
            return;
        };
        let Ok(summary) = serde_json::from_value::<ToolBatchSummary>(summary_value) else {
            return;
        };
        if summary.tools_used.is_empty() {
            return;
        }

        let context_block = summary.format_for_context();
        // Append as a user message so the model sees it as task context.
        chat_messages.push(rocode_provider::Message {
            role: rocode_provider::Role::User,
            content: rocode_provider::Content::Text(context_block),
            cache_control: None,
            provider_options: None,
        });
    }

    fn append_synthetic_user_message(session: &mut Session, message: PendingSyntheticMessage) {
        let attachments = message
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| FilePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                session_id: session.id.clone(),
                message_id: String::new(),
                mime: attachment.mime.clone(),
                url: attachment.url.clone(),
                filename: Some(
                    attachment
                        .filename
                        .clone()
                        .unwrap_or_else(|| synthetic_attachment_filename(attachment, index)),
                ),
                source: None,
            })
            .collect::<Vec<_>>();

        let text = if message.text.trim().is_empty() && !attachments.is_empty() {
            " ".to_string()
        } else {
            message.text
        };
        let msg = session.add_synthetic_user_message(text, &attachments);
        if let Some(agent) = message.agent {
            msg.metadata
                .insert("synthetic_agent".to_string(), serde_json::json!(agent));
        }
    }

    pub(super) fn repair_tool_call_name(
        tool_name: &str,
        available_tool_ids: &HashSet<String>,
    ) -> String {
        if available_tool_ids.contains(tool_name) {
            return tool_name.to_string();
        }

        let lower = tool_name.to_ascii_lowercase();
        if lower != tool_name && available_tool_ids.contains(&lower) {
            tracing::info!(
                original = tool_name,
                repaired = %lower,
                "repairing tool call name via lowercase match"
            );
            return lower;
        }

        tracing::warn!(
            tool_name = tool_name,
            "unknown tool call; preserving original name for error reporting"
        );
        tool_name.to_string()
    }

    pub(super) fn mcp_tools_from_session(session: &Session) -> Vec<ToolDefinition> {
        session
            .metadata
            .get("mcp_tools")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let name = item.get("name").and_then(|v| v.as_str())?.to_string();
                        let description = item
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let parameters = item
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({"type":"object"}));
                        Some(ToolDefinition {
                            name,
                            description,
                            parameters,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn load_persisted_subsessions(
        session: &Session,
    ) -> HashMap<String, PersistedSubsession> {
        session
            .metadata
            .get("subsessions")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    pub(super) fn save_persisted_subsessions(
        session: &mut Session,
        subsessions: &HashMap<String, PersistedSubsession>,
    ) {
        if subsessions.is_empty() {
            session.remove_metadata("subsessions");
            return;
        }
        if let Ok(value) = serde_json::to_value(subsessions) {
            session.insert_metadata("subsessions".to_string(), value);
        }
    }

    pub(super) fn with_persistent_subsession_callbacks(
        ctx: rocode_tool::ToolContext,
        subsessions: Arc<Mutex<HashMap<String, PersistedSubsession>>>,
        provider: Arc<dyn Provider>,
        tool_registry: Arc<rocode_tool::ToolRegistry>,
        default_model: String,
        agent_lookup: Option<AgentLookup>,
        ask_question_hook: Option<AskQuestionHook>,
        ask_permission_hook: Option<AskPermissionHook>,
    ) -> rocode_tool::ToolContext {
        let parent_directory = ctx.directory.clone();
        let agent_lookup_for_subsessions = agent_lookup.clone();
        let ctx = if let Some(ref lookup) = agent_lookup {
            let lookup = lookup.clone();
            ctx.with_get_agent_info(move |name| {
                let lookup = lookup.clone();
                async move { Ok(lookup(&name)) }
            })
        } else {
            ctx
        };

        let ctx = if let Some(ref question_hook) = ask_question_hook {
            let session_id = ctx.session_id.clone();
            let question_hook = question_hook.clone();
            ctx.with_ask_question(move |questions| {
                let question_hook = question_hook.clone();
                let session_id = session_id.clone();
                async move { question_hook(session_id, questions).await }
            })
        } else {
            ctx
        };

        let ctx = if let Some(ref permission_hook) = ask_permission_hook {
            let session_id = ctx.session_id.clone();
            let permission_hook = permission_hook.clone();
            ctx.with_ask(move |request| {
                let permission_hook = permission_hook.clone();
                let session_id = session_id.clone();
                async move { permission_hook(session_id, request).await }
            })
        } else {
            ctx
        };

        let ctx = ctx.with_get_last_model({
            let default_model = default_model.clone();
            move |_session_id| {
                let default_model = default_model.clone();
                async move { Ok(Some(default_model)) }
            }
        });

        let ctx = ctx.with_create_subsession({
            let subsessions = subsessions.clone();
            let parent_directory = parent_directory.clone();
            move |agent, _title, model, disabled_tools| {
                let subsessions = subsessions.clone();
                let parent_directory = parent_directory.clone();
                async move {
                    let session_id = format!("task_{}_{}", agent, uuid::Uuid::new_v4().simple());
                    let mut state = subsessions.lock().await;
                    state.insert(
                        session_id.clone(),
                        PersistedSubsession {
                            kind: rocode_types::SessionContextKind::DelegatedSubsession,
                            agent,
                            model,
                            directory: Some(parent_directory),
                            disabled_tools,
                            history: Vec::new(),
                        },
                    );
                    Ok(session_id)
                }
            }
        });

        let abort_token = ctx.abort.clone();
        let tool_runtime_config = ctx.runtime_config.clone();
        let config_store = ctx.config_store.clone();

        ctx.with_prompt_subsession(move |session_id, handoff| {
            let subsessions = subsessions.clone();
            let provider = provider.clone();
            let tool_registry = tool_registry.clone();
            let default_model = default_model.clone();
            let parent_directory = parent_directory.clone();
            let ask_question_hook = ask_question_hook.clone();
            let agent_lookup = agent_lookup_for_subsessions.clone();
            let abort_token = abort_token.clone();
            let tool_runtime_config = tool_runtime_config.clone();
            let config_store = config_store.clone();

            async move {
                let current = {
                    let state = subsessions.lock().await;
                    state.get(&session_id).cloned()
                }
                .ok_or_else(|| {
                    rocode_tool::ToolError::ExecutionError(format!(
                        "Unknown subagent session: {}. Start without task_id first.",
                        session_id
                    ))
                })?;

                let output = Self::execute_persisted_subsession_prompt(
                    &current,
                    &handoff,
                    provider,
                    tool_registry,
                    PersistedSubsessionPromptOptions {
                        default_model: default_model.clone(),
                        fallback_directory: Some(parent_directory.clone()),
                        hooks: PromptHooks {
                            agent_lookup: agent_lookup.clone(),
                            ask_question_hook: ask_question_hook.clone(),
                            ..Default::default()
                        },
                        question_session_id: Some(session_id.clone()),
                        abort: Some(abort_token),
                        tool_runtime_config: tool_runtime_config.clone(),
                        config_store,
                    },
                )
                .await
                .map_err(|e| rocode_tool::ToolError::ExecutionError(e.to_string()))?;

                let mut state = subsessions.lock().await;
                if let Some(existing) = state.get_mut(&session_id) {
                    existing.history.push(PersistedSubsessionTurn {
                        handoff: Some(handoff),
                        result: Some(output.clone()),
                        prompt: None,
                        output: None,
                    });
                }
                Ok(output)
            }
        })
    }

    pub(super) async fn execute_persisted_subsession_prompt(
        subsession: &PersistedSubsession,
        handoff: &SubsessionHandoffPacket,
        provider: Arc<dyn Provider>,
        tool_registry: Arc<rocode_tool::ToolRegistry>,
        options: PersistedSubsessionPromptOptions,
    ) -> anyhow::Result<SubsessionResultEnvelope> {
        let model = Self::resolve_subsession_model(
            subsession.model.as_deref(),
            &options.default_model,
            provider.id(),
        );

        // Cross-session handoff stays bounded: only the delegated subsession's
        // own history and the new explicit prompt cross this boundary.
        let composed_prompt = Self::compose_subsession_prompt(&subsession.history, handoff);
        let working_directory = subsession
            .directory
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .or_else(|| {
                options
                    .fallback_directory
                    .as_deref()
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
            });
        let mut executor = SubtaskExecutor::new(&subsession.agent, &composed_prompt)
            .with_model(model)
            .with_tool_runtime_config(options.tool_runtime_config.clone());
        if let Some(config_store) = options.config_store.clone() {
            executor = executor.with_config_store(config_store);
        }
        if let Some(directory) = working_directory {
            executor = executor.with_working_directory(directory);
        }
        if let Some(question_hook) = options.hooks.ask_question_hook.clone() {
            let session_id = options
                .question_session_id
                .clone()
                .unwrap_or_else(|| "subtask".to_string());
            executor = executor.with_ask_question_hook(question_hook, session_id);
        }
        if let Some(permission_hook) = options.hooks.ask_permission_hook.clone() {
            executor = executor.with_ask_permission_hook(permission_hook);
        }
        if let Some(token) = options.abort.clone() {
            executor = executor.with_abort(token);
        }
        let agent_info = options
            .hooks
            .agent_lookup
            .as_ref()
            .and_then(|lookup| lookup(&subsession.agent));
        let request_defaults = inline_subtask_request_defaults(
            agent_info.as_ref().and_then(|info| info.variant.clone()),
        );
        executor = executor.with_max_steps(agent_info.as_ref().and_then(|info| info.steps));
        executor = executor
            .with_execution_context(agent_info.as_ref().and_then(|info| info.execution.clone()));
        executor = executor.with_variant(
            agent_info
                .as_ref()
                .and_then(|info| info.variant.clone())
                .or_else(|| request_defaults.variant.clone()),
        );
        executor.agent_params = AgentParams {
            max_tokens: agent_info
                .as_ref()
                .and_then(|info| info.max_tokens)
                .or(request_defaults.max_tokens),
            temperature: agent_info
                .as_ref()
                .and_then(|info| info.temperature)
                .or(request_defaults.temperature),
            top_p: agent_info
                .as_ref()
                .and_then(|info| info.top_p)
                .or(request_defaults.top_p),
        };

        let output = executor
            .execute_inline(provider, &tool_registry, &subsession.disabled_tools)
            .await?;
        Ok(SubsessionResultEnvelope::summary(output))
    }

    pub(super) fn resolve_subsession_model(
        requested_model: Option<&str>,
        default_model: &str,
        current_provider_id: &str,
    ) -> ModelRef {
        let mut model = Self::parse_model_string(requested_model.unwrap_or(default_model));
        if model.provider_id == "default" && model.model_id == "default" {
            model = Self::parse_model_string(default_model);
        }

        // Subsession execution reuses the parent provider object.
        // If a subagent model comes from another provider namespace (for example
        // plugin config like "opencode/big-pickle"), running it against the
        // current provider causes model-not-found errors. Fallback to the
        // parent's default model in that mismatch case.
        if model.provider_id != "default" && model.provider_id != current_provider_id {
            tracing::warn!(
                requested_provider = %model.provider_id,
                requested_model = %model.model_id,
                current_provider = %current_provider_id,
                fallback_model = %default_model,
                "subsession model provider differs from current provider; falling back to default model"
            );
            return Self::parse_model_string(default_model);
        }

        model
    }

    pub(super) fn parse_model_string(raw: &str) -> ModelRef {
        if let Some((provider_id, model_id)) = raw.split_once(':').or_else(|| raw.split_once('/')) {
            return ModelRef {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
            };
        }
        if raw.is_empty() {
            return ModelRef {
                provider_id: "default".to_string(),
                model_id: "default".to_string(),
            };
        }
        ModelRef {
            provider_id: "default".to_string(),
            model_id: raw.to_string(),
        }
    }

    pub(super) fn compose_subsession_prompt(
        history: &[PersistedSubsessionTurn],
        handoff: &SubsessionHandoffPacket,
    ) -> String {
        let rendered_handoff = Self::render_subsession_handoff(handoff);
        if history.is_empty() {
            return rendered_handoff;
        }

        let history_text = history
            .iter()
            .rev()
            .take(MAX_PERSISTED_SUBSESSION_HISTORY_TURNS)
            .rev()
            .map(Self::render_persisted_subsession_turn)
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        format!(
            "Continue this delegated subsession.\n\nPrevious delegated work:\n{}\n\nNew handoff:\n{}",
            history_text, rendered_handoff
        )
    }

    fn render_persisted_subsession_turn(turn: &PersistedSubsessionTurn) -> String {
        let handoff = turn.handoff.clone().unwrap_or_else(|| {
            SubsessionHandoffPacket::bounded_goal(turn.prompt.clone().unwrap_or_default())
        });
        let result = turn.result.clone().unwrap_or_else(|| {
            SubsessionResultEnvelope::summary(turn.output.clone().unwrap_or_default())
        });

        format!(
            "Delegated handoff:\n{}\n\nRecovered result ({}):\n{}",
            Self::indent_block(&Self::render_subsession_handoff(&handoff)),
            match result.absorb_mode {
                rocode_types::SubsessionResultAbsorbMode::SummaryOnly => "summary only",
            },
            Self::indent_block(&Self::truncate_subsession_field(
                &result.text,
                MAX_SUBSESSION_FIELD_CHARS
            ))
        )
    }

    fn render_subsession_handoff(handoff: &SubsessionHandoffPacket) -> String {
        let mut lines = vec![format!(
            "Delegated handoff mode: {}",
            match handoff.effective_richness() {
                SubsessionHandoffRichness::Bounded => "bounded",
                SubsessionHandoffRichness::Enriched => "enriched",
            }
        )];

        let mut sanctioned_tail_count = 0usize;
        for field in &handoff.fields {
            let trimmed = field.text.trim();
            if trimmed.is_empty() {
                continue;
            }

            let limit = if matches!(field.kind, SubsessionHandoffFieldKind::SanctionedRecentTail) {
                if sanctioned_tail_count >= MAX_SUBSESSION_HANDOFF_TAIL_FIELDS {
                    continue;
                }
                sanctioned_tail_count += 1;
                MAX_SUBSESSION_TAIL_FIELD_CHARS
            } else {
                MAX_SUBSESSION_FIELD_CHARS
            };
            let text = Self::truncate_subsession_field(trimmed, limit);
            let label = Self::subsession_handoff_field_label(field.kind);
            let title = field
                .title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let header = match title {
                Some(title) => format!("## {}: {}", label, title),
                None => format!("## {}", label),
            };
            lines.push(header);
            lines.push(text);
        }

        lines.join("\n\n")
    }

    fn subsession_handoff_field_label(kind: SubsessionHandoffFieldKind) -> &'static str {
        match kind {
            SubsessionHandoffFieldKind::Goal => "Goal",
            SubsessionHandoffFieldKind::Constraint => "Constraints",
            SubsessionHandoffFieldKind::Fact => "Facts",
            SubsessionHandoffFieldKind::RequiredPath => "Required Paths",
            SubsessionHandoffFieldKind::SupportingContext => "Supporting Context",
            SubsessionHandoffFieldKind::PreflightContext => "Preflight Context",
            SubsessionHandoffFieldKind::RecentConclusion => "Recent Conclusions",
            SubsessionHandoffFieldKind::SanctionedRecentTail => "Sanctioned Recent Tail",
        }
    }

    fn truncate_subsession_field(text: &str, max_chars: usize) -> String {
        let normalized = text.trim();
        let truncated = normalized.chars().take(max_chars).collect::<String>();
        if normalized.chars().count() > max_chars {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }

    fn indent_block(text: &str) -> String {
        text.lines()
            .map(|line| format!("  {}", line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn classify_error_kind(error: &str) -> String {
    let lower = error.trim().to_ascii_lowercase();
    if lower.starts_with("permission denied:") || lower.contains("permission denied") {
        "permission_denied".to_string()
    } else if lower.starts_with("provider error:")
        || lower.starts_with("invalid request:")
        || lower.contains("provider rejected")
    {
        "provider_rejected".to_string()
    } else if lower.contains("user input required")
        || lower.contains("question required")
        || lower.contains("approval required")
    {
        "user_input_required".to_string()
    } else if lower.starts_with("file not found:") || lower.contains("file not found") {
        "file_not_found".to_string()
    } else if lower.starts_with("timeout:")
        || lower.contains("timeout:")
        || lower.contains("timed out")
    {
        "timeout".to_string()
    } else if lower.starts_with("invalid arguments:")
        || lower.contains("invalid arguments:")
        || lower.starts_with("validation error:")
        || lower.contains("validation error:")
    {
        "invalid_arguments".to_string()
    } else if lower == "cancelled" || lower.contains("cancelled") || lower.contains("canceled") {
        "cancelled".to_string()
    } else {
        "execution_error".to_string()
    }
}

fn synthetic_attachment_filename(
    attachment: &rocode_tool::SyntheticAttachment,
    index: usize,
) -> String {
    if let Some(filename) = attachment
        .filename
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return filename.clone();
    }

    let ext = match attachment.mime.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        _ => "bin",
    };
    format!("attachment-{}.{}", index + 1, ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use async_trait::async_trait;
    use futures::stream;
    use rocode_provider::{
        ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamResult,
    };
    use rocode_tool::{Tool, ToolContext, ToolError, ToolResult};
    use std::collections::HashSet;
    use std::sync::Arc;

    struct StaticModelProvider {
        model: Option<ModelInfo>,
    }

    impl StaticModelProvider {
        fn with_model(model_id: &str, context_window: u64, max_output_tokens: u64) -> Self {
            Self {
                model: Some(ModelInfo {
                    id: model_id.to_string(),
                    name: "Static Model".to_string(),
                    provider: "mock".to_string(),
                    context_window,
                    max_input_tokens: None,
                    max_output_tokens,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.0,
                    cost_per_million_output: 0.0,
                    cost_per_million_cache_read: None,
                    cost_per_million_cache_write: None,
                }),
            }
        }
    }

    #[async_trait]
    impl Provider for StaticModelProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            self.model.clone().into_iter().collect()
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            self.model.as_ref().filter(|model| model.id == id)
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "chat() not used in this test".to_string(),
            ))
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    struct SyntheticAttachmentTool;
    struct EchoTool;
    struct AlwaysFailTool;
    struct ExecutionErrorTool;

    #[async_trait]
    impl Tool for SyntheticAttachmentTool {
        fn id(&self) -> &str {
            "synthetic_attachment"
        }

        fn description(&self) -> &str {
            "Emits a synthetic attachment message for tests"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            ctx.do_create_synthetic_message_with_attachments(
                Some("docs-researcher".to_string()),
                String::new(),
                vec![rocode_tool::SyntheticAttachment {
                    url: "file:///tmp/artifact.png".to_string(),
                    mime: "image/png".to_string(),
                    filename: Some("artifact.png".to_string()),
                }],
            )
            .await?;

            Ok(ToolResult::simple("Synthetic Attachment", "queued"))
        }
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn id(&self) -> &str {
            "echo_tool"
        }

        fn description(&self) -> &str {
            "Echo tool for telemetry tests"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(
            &self,
            args: serde_json::Value,
            _ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::simple("Echo", args.to_string()))
        }
    }

    #[async_trait]
    impl Tool for AlwaysFailTool {
        fn id(&self) -> &str {
            "fail_tool"
        }

        fn description(&self) -> &str {
            "Always fails for telemetry tests"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "required_arg": { "type": "string" }
                },
                "required": ["required_arg"]
            })
        }

        fn validate(&self, _args: &serde_json::Value) -> Result<(), ToolError> {
            Err(ToolError::InvalidArguments(
                "Invalid arguments: required_arg is required".to_string(),
            ))
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError::ExecutionError("boom".to_string()))
        }
    }

    #[async_trait]
    impl Tool for ExecutionErrorTool {
        fn id(&self) -> &str {
            "exec_err_tool"
        }

        fn description(&self) -> &str {
            "Always fails with ExecutionError for testing"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError::ExecutionError("boom".to_string()))
        }
    }

    fn tool_state_repair_events(
        session: &Session,
        assistant_index: usize,
    ) -> Vec<serde_json::Value> {
        session.messages[assistant_index]
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::ToolCall {
                    state: Some(crate::ToolState::Completed { metadata, .. }),
                    ..
                } => Some(rocode_tool::tool_repair_events(metadata)),
                PartType::ToolCall {
                    state:
                        Some(crate::ToolState::Error {
                            metadata: Some(metadata),
                            ..
                        }),
                    ..
                } => Some(rocode_tool::tool_repair_events(metadata)),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn persisted_subsessions_roundtrip_via_session_metadata() {
        let mut session = Session::new("proj", ".");
        let mut map = HashMap::new();
        map.insert(
            "task_explore_1".to_string(),
            PersistedSubsession {
                kind: rocode_types::SessionContextKind::DelegatedSubsession,
                agent: "explore".to_string(),
                model: Some("ethnopic:test-model".to_string()),
                directory: Some("/tmp/project".to_string()),
                disabled_tools: vec!["task".to_string()],
                history: vec![PersistedSubsessionTurn {
                    handoff: Some(SubsessionHandoffPacket::bounded_goal("Inspect src")),
                    result: Some(SubsessionResultEnvelope::summary("Done")),
                    prompt: None,
                    output: None,
                }],
            },
        );

        SessionPrompt::save_persisted_subsessions(&mut session, &map);
        let loaded = SessionPrompt::load_persisted_subsessions(&session);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded["task_explore_1"].kind,
            rocode_types::SessionContextKind::DelegatedSubsession
        );
        assert_eq!(loaded["task_explore_1"].agent, "explore");
        assert_eq!(loaded["task_explore_1"].history.len(), 1);
    }

    #[test]
    fn parse_model_string_supports_provider_prefix() {
        let model = SessionPrompt::parse_model_string("openai:gpt-4o");
        assert_eq!(model.provider_id, "openai");
        assert_eq!(model.model_id, "gpt-4o");
    }

    #[test]
    fn resolve_subsession_model_falls_back_on_provider_mismatch() {
        let model = SessionPrompt::resolve_subsession_model(
            Some("opencode:big-pickle"),
            "zhipuai-coding-plan:glm-4.6",
            "zhipuai-coding-plan",
        );
        assert_eq!(model.provider_id, "zhipuai-coding-plan");
        assert_eq!(model.model_id, "glm-4.6");
    }

    #[test]
    fn resolve_subsession_model_keeps_same_provider_model() {
        let model = SessionPrompt::resolve_subsession_model(
            Some("zhipuai-coding-plan:GLM-5"),
            "zhipuai-coding-plan:glm-4.6",
            "zhipuai-coding-plan",
        );
        assert_eq!(model.provider_id, "zhipuai-coding-plan");
        assert_eq!(model.model_id, "GLM-5");
    }

    #[test]
    fn compose_subsession_prompt_includes_recent_history() {
        let history = vec![PersistedSubsessionTurn {
            handoff: Some(SubsessionHandoffPacket::bounded_goal("Find files")),
            result: Some(SubsessionResultEnvelope::summary("Found 10 files")),
            prompt: None,
            output: None,
        }];
        let composed = SessionPrompt::compose_subsession_prompt(
            &history,
            &SubsessionHandoffPacket::bounded_goal("Continue"),
        );
        assert!(composed.contains("Previous delegated work"));
        assert!(composed.contains("Find files"));
        assert!(composed.contains("Continue"));
    }

    #[test]
    fn compose_subsession_prompt_limits_sanctioned_recent_tail_fields() {
        let mut handoff = SubsessionHandoffPacket::bounded_goal("Continue");
        handoff.push_text(SubsessionHandoffFieldKind::SanctionedRecentTail, "tail one");
        handoff.push_text(SubsessionHandoffFieldKind::SanctionedRecentTail, "tail two");
        handoff.push_text(
            SubsessionHandoffFieldKind::SanctionedRecentTail,
            "tail three",
        );
        handoff.push_text(
            SubsessionHandoffFieldKind::SanctionedRecentTail,
            "tail four",
        );

        let composed = SessionPrompt::compose_subsession_prompt(&[], &handoff);

        assert!(composed.contains("tail one"));
        assert!(composed.contains("tail two"));
        assert!(composed.contains("tail three"));
        assert!(!composed.contains("tail four"));
    }

    #[tokio::test]
    async fn execute_tool_calls_appends_synthetic_attachment_message() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(SyntheticAttachmentTool).await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session.messages_mut().push(SessionMessage::user(
            sid.clone(),
            "run synthetic attachment",
        ));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call(
            "call_synthetic",
            "synthetic_attachment",
            serde_json::json!({}),
        );
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should succeed");

        let synthetic_msg = session
            .messages
            .last()
            .expect("synthetic user message should be appended");
        assert!(matches!(synthetic_msg.role, MessageRole::User));
        assert_eq!(
            synthetic_msg
                .metadata
                .get("synthetic_agent")
                .and_then(|value| value.as_str()),
            Some("docs-researcher")
        );

        let text_part = synthetic_msg
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::Text {
                    text, synthetic, ..
                } => Some((text.as_str(), *synthetic)),
                _ => None,
            })
            .expect("synthetic text part should exist");
        assert_eq!(text_part.0, " ");
        assert_eq!(text_part.1, Some(true));

        let file_part = synthetic_msg
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::File {
                    url,
                    filename,
                    mime,
                } => Some((url.as_str(), filename.as_str(), mime.as_str())),
                _ => None,
            })
            .expect("synthetic file part should exist");
        assert_eq!(file_part.0, "file:///tmp/artifact.png");
        assert_eq!(file_part.1, "artifact.png");
        assert_eq!(file_part.2, "image/png");
    }

    #[tokio::test]
    async fn execute_tool_calls_persists_prompt_layer_repair_telemetry_on_success() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(EchoTool).await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run echo"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call(
            "call_echo",
            "ECHO_TOOL",
            serde_json::json!("{\"value\":\"hello\"}"),
        );
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should succeed");

        let repair_events = tool_state_repair_events(&session, 1);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("tool_name_repair")
                && event.get("from").and_then(|value| value.as_str()) == Some("ECHO_TOOL")
                && event.get("to").and_then(|value| value.as_str()) == Some("echo_tool")
        }));
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("argument_normalization")
                && event
                    .get("modes")
                    .and_then(|value| value.as_array())
                    .is_some_and(|modes| {
                        modes
                            .iter()
                            .any(|value| value.as_str() == Some("robust_json_object_parse"))
                    })
        }));
        // P1.1: verify the three-layer arg contract — raw_shape and normalized_shape
        // are populated on the argument_normalization repair event.
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("argument_normalization")
                && event.get("raw_shape").is_some()
                && event.get("normalized_shape").is_some()
        }));
    }

    #[tokio::test]
    async fn execute_tool_calls_persists_prompt_layer_repair_telemetry_on_error() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_fail", "FAIL_TOOL", serde_json::json!({}));
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should complete despite tool failure");

        let repair_events = tool_state_repair_events(&session, 1);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("tool_name_repair")
                && event.get("to").and_then(|value| value.as_str()) == Some("fail_tool")
        }));
    }

    #[test]
    fn inject_latest_tool_batch_summary_consumes_metadata_once() {
        let mut session = Session::new("proj", ".");
        let summary = ToolBatchSummary {
            tools_used: vec!["read".to_string(), "edit".to_string()],
            success_count: 2,
            error_count: 0,
            error_kinds: Vec::new(),
            goal_status: rocode_types::ToolBatchGoalStatus::Advanced,
            blocked_by: Vec::new(),
            artifacts_created: Vec::new(),
            pending_follow_up: Vec::new(),
            unresolved_items: Vec::new(),
            recommended_next_step: Some("continue with implementation".to_string()),
            repair_events: Vec::new(),
        };
        session.insert_metadata(
            "latest_tool_batch_summary".to_string(),
            serde_json::to_value(&summary).expect("summary should serialize"),
        );

        let mut chat_messages = vec![rocode_provider::Message {
            role: rocode_provider::Role::User,
            content: rocode_provider::Content::Text("original user request".to_string()),
            cache_control: None,
            provider_options: None,
        }];

        SessionPrompt::inject_latest_tool_batch_summary(&mut session, &mut chat_messages);

        assert_eq!(chat_messages.len(), 2);
        let injected = match &chat_messages[1].content {
            rocode_provider::Content::Text(text) => text.clone(),
            other => panic!("expected text summary, got {other:?}"),
        };
        assert!(injected.contains("<tool-batch-summary>"));
        assert!(injected.contains("tools: edit, read") || injected.contains("tools: read, edit"));
        assert_eq!(session.metadata.get("latest_tool_batch_summary"), None);

        SessionPrompt::inject_latest_tool_batch_summary(&mut session, &mut chat_messages);
        assert_eq!(chat_messages.len(), 2);
    }

    #[test]
    fn inject_latest_tool_batch_summary_skips_invalid_payload_and_clears_it() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            "latest_tool_batch_summary".to_string(),
            serde_json::json!({"bad": "shape"}),
        );

        let mut chat_messages = Vec::new();
        SessionPrompt::inject_latest_tool_batch_summary(&mut session, &mut chat_messages);

        assert!(chat_messages.is_empty());
        assert_eq!(session.metadata.get("latest_tool_batch_summary"), None);
    }

    #[test]
    fn build_tool_batch_summary_marks_provider_rejected_as_blocked() {
        let mut assistant = SessionMessage::assistant("sess".to_string());
        assistant.add_tool_call("call_provider", "websearch", serde_json::json!({}));
        if let Some(part) = assistant.parts.get_mut(0) {
            part.part_type = PartType::ToolCall {
                id: "call_provider".to_string(),
                name: "websearch".to_string(),
                input: serde_json::json!({}),
                status: crate::ToolCallStatus::Error,
                raw: None,
                state: Some(crate::ToolState::Error {
                    input: serde_json::json!({}),
                    error: "Provider error: Invalid request".to_string(),
                    metadata: None,
                    time: crate::ErrorTime { start: 0, end: 1 },
                }),
            };
        }

        let summary = SessionPrompt::build_tool_batch_summary(&assistant, &[])
            .expect("summary should be built");
        assert_eq!(
            summary.goal_status,
            rocode_types::ToolBatchGoalStatus::Blocked
        );
        assert_eq!(
            summary.blocked_by,
            vec![rocode_types::ToolBatchBlockReason::ProviderRejected]
        );
        assert_eq!(
            summary.recommended_next_step, None,
            "provider-rejected path should not pretend normal execution can continue"
        );
    }

    #[test]
    fn build_tool_batch_summary_uses_no_progress_when_failure_has_no_blocker() {
        let mut assistant = SessionMessage::assistant("sess".to_string());
        assistant.add_tool_call("call_unknown", "read", serde_json::json!({}));
        if let Some(part) = assistant.parts.get_mut(0) {
            part.part_type = PartType::ToolCall {
                id: "call_unknown".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({}),
                status: crate::ToolCallStatus::Error,
                raw: None,
                state: None,
            };
        }

        let summary = SessionPrompt::build_tool_batch_summary(&assistant, &[])
            .expect("summary should be built");
        assert_eq!(
            summary.goal_status,
            rocode_types::ToolBatchGoalStatus::NoProgress
        );
        assert!(
            summary.blocked_by.is_empty(),
            "missing blocker classification should not be upgraded to blocked"
        );
    }

    #[test]
    fn repair_tool_call_name_keeps_exact_match() {
        let tools = HashSet::from([
            "read".to_string(),
            "glob".to_string(),
            "invalid".to_string(),
        ]);
        assert_eq!(SessionPrompt::repair_tool_call_name("read", &tools), "read");
    }

    #[test]
    fn repair_tool_call_name_repairs_case_mismatch() {
        let tools = HashSet::from([
            "read".to_string(),
            "glob".to_string(),
            "invalid".to_string(),
        ]);
        assert_eq!(SessionPrompt::repair_tool_call_name("Read", &tools), "read");
    }

    #[test]
    fn repair_tool_call_name_preserves_unknown_name() {
        let tools = HashSet::from([
            "read".to_string(),
            "glob".to_string(),
            "invalid".to_string(),
        ]);
        assert_eq!(
            SessionPrompt::repair_tool_call_name("read_html_file", &tools),
            "read_html_file"
        );
    }

    #[test]
    fn mcp_tools_from_session_reads_runtime_metadata() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            "mcp_tools".to_string(),
            serde_json::json!([{
                "name": "repo_search",
                "description": "Search repository",
                "parameters": {"type":"object","properties":{"q":{"type":"string"}}}
            }]),
        );

        let tools = SessionPrompt::mcp_tools_from_session(&session);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "repo_search");
    }

    #[tokio::test]
    // P2.3: strict/permissive split must not drift — strict preserves raw errors.
    async fn strict_tool_execution_does_not_reroute_invalid_args_to_invalid_tool() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;
        tool_registry
            .register(rocode_tool::invalid::InvalidTool)
            .await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_fail", "fail_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));

        // Strict policy via config store.
        let mut config = rocode_config::Config::default();
        config.repair_policy = Some(rocode_types::RepairPolicy::Strict);
        let config_store = Arc::new(rocode_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should complete");

        // In strict mode, the tool call name should NOT be changed to "invalid".
        let assistant_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .expect("assistant message should exist");
        let tool_name = assistant_msg
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::ToolCall { id, name, .. } if id == "call_fail" => Some(name.clone()),
                _ => None,
            })
            .expect("tool call should exist");
        assert_eq!(
            tool_name, "fail_tool",
            "strict mode should preserve original tool name, not reroute to invalid"
        );
    }

    #[tokio::test]
    async fn execute_tool_calls_reads_runtime_budget_from_config_store() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(EchoTool).await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run large echo"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call(
            "call_echo",
            "echo_tool",
            serde_json::json!({ "value": "Q".repeat(600) }),
        );
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));

        let mut config = rocode_config::Config::default();
        config.runtime_budget = Some(rocode_config::RuntimeBudgetConfig {
            tool_result_max_chars: 128,
            tool_result_preview_chars: 32,
            ..rocode_config::RuntimeBudgetConfig::default()
        });
        let config_store = Arc::new(rocode_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should succeed");

        let tool_message = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Tool))
            .expect("tool message should exist");
        let tool_result = tool_message
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::ToolResult {
                    content, metadata, ..
                } => Some((content.as_str(), metadata.as_ref())),
                _ => None,
            })
            .expect("tool result should exist");

        assert!(tool_result
            .0
            .contains("[tool result governed: output too large]"));
        assert!(tool_result.0.contains("preview_chars: 32"));
        assert_eq!(
            tool_result.1.and_then(|m| m.get("tool_result_governed")),
            Some(&serde_json::json!(true))
        );
    }

    #[tokio::test]
    async fn strict_prevalidation_preserves_original_write_input_and_avoids_invalid_payload() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry
            .register(rocode_tool::write::WriteTool::new())
            .await;
        tool_registry
            .register(rocode_tool::invalid::InvalidTool)
            .await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "write file"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call(
            "call_write",
            "write",
            serde_json::json!({
                "file_path": "demo.txt"
            }),
        );
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));

        let mut config = rocode_config::Config::default();
        config.repair_policy = Some(rocode_types::RepairPolicy::Strict);
        let config_store = Arc::new(rocode_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should complete");

        let assistant_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .expect("assistant message should exist");
        let (tool_name, tool_input) = assistant_msg
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::ToolCall {
                    id, name, input, ..
                } if id == "call_write" => Some((name.clone(), input.clone())),
                _ => None,
            })
            .expect("tool call should exist");

        assert_eq!(tool_name, "write");
        assert_eq!(tool_input, serde_json::json!({ "file_path": "demo.txt" }));

        let tool_result = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Tool))
            .and_then(|message| {
                message.parts.iter().find_map(|part| match &part.part_type {
                    PartType::ToolResult {
                        tool_call_id,
                        content,
                        ..
                    } if tool_call_id == "call_write" => Some(content.clone()),
                    _ => None,
                })
            })
            .expect("tool result should exist");
        assert!(
            tool_result.contains("Invalid arguments:"),
            "strict prevalidation should stop with an argument error"
        );
    }

    #[tokio::test]
    async fn permissive_repair_preserves_invalid_reroute_strict_does_not() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;
        tool_registry
            .register(rocode_tool::invalid::InvalidTool)
            .await;

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));

        // ── Permissive ────────────────────────────────────────────
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_fail_p", "fail_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);

        let mut config = rocode_config::Config::default();
        config.repair_policy = Some(rocode_types::RepairPolicy::Permissive);
        let config_store = Arc::new(rocode_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry.clone(),
            ctx,
            provider.clone(),
            "mock",
            "test-model",
        )
        .await
        .expect("permissive should succeed");

        let p_name = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .and_then(|m| {
                m.parts.iter().find_map(|part| match &part.part_type {
                    PartType::ToolCall { id, name, .. } if id == "call_fail_p" => {
                        Some(name.clone())
                    }
                    _ => None,
                })
            })
            .unwrap_or_default();
        assert_eq!(p_name, "invalid", "permissive should reroute to invalid");

        // ── Strict ────────────────────────────────────────────────
        let mut session2 = Session::new("proj2", ".");
        let sid2 = session2.id.clone();
        session2
            .messages_mut()
            .push(SessionMessage::user(sid2.clone(), "run failing tool"));
        let mut assistant2 = SessionMessage::assistant(sid2);
        assistant2.add_tool_call("call_fail_s", "fail_tool", serde_json::json!({}));
        session2.messages_mut().push(assistant2);

        let mut config2 = rocode_config::Config::default();
        config2.repair_policy = Some(rocode_types::RepairPolicy::Strict);
        let config_store2 = Arc::new(rocode_config::ConfigStore::new(config2));
        let ctx2 = ToolContext::new(session2.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store2);

        SessionPrompt::execute_tool_calls(
            &mut session2,
            tool_registry,
            ctx2,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("strict should complete");

        let s_name = session2
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .and_then(|m| {
                m.parts.iter().find_map(|part| match &part.part_type {
                    PartType::ToolCall { id, name, .. } if id == "call_fail_s" => {
                        Some(name.clone())
                    }
                    _ => None,
                })
            })
            .unwrap_or_default();
        assert_eq!(
            s_name, "fail_tool",
            "strict should preserve original tool name"
        );
    }

    // P2.3: invalid args from real execution must end up as blocked_by in the
    // persisted batch summary, with machine-readable next_step.
    #[tokio::test]
    async fn p23_tool_batch_summary_marks_invalid_arguments_as_blocked_with_fix_args_next_step() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;
        // Register the invalid tool so permissive reroute can happen.
        tool_registry
            .register(rocode_tool::invalid::InvalidTool)
            .await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        // AlwaysFailTool's validate() returns Err, which triggers permissive
        // reroute to invalid. The batch summary must reflect the block reason.
        assistant.add_tool_call("call_fail", "fail_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
        // Permissive policy so the reroute happens.
        let mut config = rocode_config::Config::default();
        config.repair_policy = Some(rocode_types::RepairPolicy::Permissive);
        let config_store = Arc::new(rocode_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should succeed");

        // Read the persisted summary from session metadata.
        let summary_value = session
            .record()
            .metadata
            .get("latest_tool_batch_summary")
            .expect("batch summary should be persisted");
        let summary: ToolBatchSummary =
            serde_json::from_value(summary_value.clone()).expect("should deserialize");

        assert!(
            summary
                .blocked_by
                .contains(&rocode_types::ToolBatchBlockReason::InvalidArguments),
            "blocked_by should contain InvalidArguments, got {:?}",
            summary.blocked_by
        );
        assert_eq!(
            summary.recommended_next_step,
            Some("fix tool arguments before retrying".to_string())
        );
    }

    // P2.3: synthetic attachment names captured from the real
    // execute_tool_calls → pending_synthetic_messages pipeline must survive
    // into the persisted batch summary.
    #[tokio::test]
    async fn p23_tool_batch_summary_preserves_synthetic_artifact_names() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        tool_registry.register(SyntheticAttachmentTool).await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session.messages_mut().push(SessionMessage::user(
            sid.clone(),
            "run synthetic attachment",
        ));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call(
            "call_synthetic",
            "synthetic_attachment",
            serde_json::json!({}),
        );
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should succeed");

        // Read from session metadata — the path that execute_tool_calls writes.
        let summary_value = session
            .record()
            .metadata
            .get("latest_tool_batch_summary")
            .expect("batch summary should be persisted");
        let summary: ToolBatchSummary =
            serde_json::from_value(summary_value.clone()).expect("should deserialize");

        // SyntheticAttachmentTool emits "artifact.png" as an attachment filename.
        assert!(
            summary
                .artifacts_created
                .iter()
                .any(|a| a == "artifact.png"),
            "synthetic artifact name should survive into persisted summary"
        );
        let formatted = summary.format_for_context();
        assert!(formatted.contains("artifacts: artifact.png"));
    }

    // P2.3: an ordinary execution error permissive reroute must NOT be
    // misclassified as invalid_arguments in the batch summary.
    #[tokio::test]
    async fn p23_execution_error_reroute_is_not_classified_as_invalid_arguments() {
        let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
        // AlwaysFailTool.execute() returns ExecutionError("boom"), which has
        // no validate() — this is a genuine execution error, not bad args.
        tool_registry.register(ExecutionErrorTool).await;
        tool_registry
            .register(rocode_tool::invalid::InvalidTool)
            .await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_exec_err", "exec_err_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);

        let provider: Arc<dyn Provider> =
            Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
        let mut config = rocode_config::Config::default();
        config.repair_policy = Some(rocode_types::RepairPolicy::Permissive);
        let config_store = Arc::new(rocode_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(
            &mut session,
            tool_registry,
            ctx,
            provider,
            "mock",
            "test-model",
        )
        .await
        .expect("execute_tool_calls should succeed");

        let summary_value = session
            .record()
            .metadata
            .get("latest_tool_batch_summary")
            .expect("batch summary should be persisted");
        let summary: ToolBatchSummary =
            serde_json::from_value(summary_value.clone()).expect("should deserialize");

        // Must NOT be classified as invalid_arguments.
        assert!(
            !summary
                .blocked_by
                .contains(&rocode_types::ToolBatchBlockReason::InvalidArguments),
            "execution error reroute must not be misclassified as invalid_arguments"
        );
        // Must be classified as a tool execution error.
        assert!(
            summary
                .blocked_by
                .contains(&rocode_types::ToolBatchBlockReason::ToolExecutionError),
            "execution error reroute should be classified as tool_execution_error"
        );
    }
}
