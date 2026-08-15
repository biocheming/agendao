// Tool execution methods for SessionPrompt.

use std::collections::HashSet;
use std::sync::Arc;

use agendao_provider::ToolDefinition;
use agendao_types::{RepairEvent, ToolBatchSummary};

use crate::{MessageRole, PartType, Session, SessionMessage};

use super::{PromptHooks, SessionPrompt};

#[derive(Clone)]
struct ToolExecutionOptions {
    hooks: PromptHooks,
    repair_policy: agendao_types::RepairPolicy,
    tool_result_budget: crate::tool_result_governance::ToolResultBudget,
}

// ── P2.2: Tool batch fact extraction ────────────────────────────────────

struct ToolCallBatchFact {
    tool_name: String,
    is_error: bool,
    error_kind: Option<String>,
    block_reason: Option<agendao_types::ToolBatchBlockReason>,
    artifacts_created: Vec<String>,
    repair_events: Vec<RepairEvent>,
    suggested_follow_up: Vec<agendao_types::ToolBatchFollowUpItem>,
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
                        } => Some(agendao_tool::repair_events(metadata)),
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
    Vec<agendao_types::ToolBatchFollowUpItem>,
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
                        follow_up.push(agendao_types::ToolBatchFollowUpItem {
                            kind: "fix_args".into(),
                            text: format!("{name}: fix arguments and retry"),
                        });
                    }
                    Some("permission_denied") => {
                        follow_up.push(agendao_types::ToolBatchFollowUpItem {
                            kind: "ask_permission".into(),
                            text: format!("{name}: request permission or use alternative"),
                        });
                    }
                    Some("timeout") => {
                        follow_up.push(agendao_types::ToolBatchFollowUpItem {
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
) -> Option<agendao_types::ToolBatchBlockReason> {
    let from_error = match error_kind {
        Some("invalid_arguments") => Some(agendao_types::ToolBatchBlockReason::InvalidArguments),
        Some("permission_denied") => Some(agendao_types::ToolBatchBlockReason::PermissionDenied),
        Some("timeout") => Some(agendao_types::ToolBatchBlockReason::Timeout),
        Some("provider_rejected") => Some(agendao_types::ToolBatchBlockReason::ProviderRejected),
        Some("user_input_required") => Some(agendao_types::ToolBatchBlockReason::UserInputRequired),
        Some("execution_error") => Some(agendao_types::ToolBatchBlockReason::ToolExecutionError),
        Some(_) => Some(agendao_types::ToolBatchBlockReason::Unknown),
        None => None,
    };
    if from_error.is_some() {
        return from_error;
    }
    // When a tool call was permissively rerouted, infer the block reason
    // from repair events that carry the original error classification.
    for e in repair_events {
        let Some(kind) = agendao_types::RepairKind::parse(&e.repair_kind) else {
            continue;
        };
        match kind {
            agendao_types::RepairKind::ArgumentPrevalidationFallback => {
                return Some(agendao_types::ToolBatchBlockReason::InvalidArguments);
            }
            agendao_types::RepairKind::InvalidToolReroute => {
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

fn derive_goal_status(facts: &[ToolCallBatchFact]) -> agendao_types::ToolBatchGoalStatus {
    let has_error = facts.iter().any(|f| f.is_error);
    let has_blocker = facts.iter().any(|f| f.block_reason.is_some());
    // Count calls that actually succeeded (no error AND no blocker).
    let real_success_count = facts
        .iter()
        .filter(|f| !f.is_error && f.block_reason.is_none())
        .count();
    let total = facts.len();

    if real_success_count == total {
        return agendao_types::ToolBatchGoalStatus::Advanced;
    }
    if real_success_count > 0 && has_error {
        return agendao_types::ToolBatchGoalStatus::Mixed;
    }
    if real_success_count > 0 && has_blocker {
        return agendao_types::ToolBatchGoalStatus::Mixed;
    }
    if has_blocker {
        return agendao_types::ToolBatchGoalStatus::Blocked;
    }
    if has_error {
        return agendao_types::ToolBatchGoalStatus::NoProgress;
    }
    agendao_types::ToolBatchGoalStatus::NoProgress
}

fn derive_recommended_next_step(facts: &[ToolCallBatchFact]) -> Option<String> {
    let has_success = facts.iter().any(|f| !f.is_error);
    let has_artifact = facts.iter().any(|f| !f.artifacts_created.is_empty());
    let all_have_blockers = facts.iter().all(|f| f.block_reason.is_some());
    let has_invalid_args = facts
        .iter()
        .any(|f| f.block_reason == Some(agendao_types::ToolBatchBlockReason::InvalidArguments));
    let has_permission = facts
        .iter()
        .any(|f| f.block_reason == Some(agendao_types::ToolBatchBlockReason::PermissionDenied));
    let has_timeout = facts
        .iter()
        .any(|f| f.block_reason == Some(agendao_types::ToolBatchBlockReason::Timeout));

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

impl SessionPrompt {
    pub async fn execute_tool_calls(
        session: &mut Session,
        tool_registry: Arc<agendao_tool::ToolRegistry>,
        ctx: agendao_tool::ToolContext,
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
            ToolExecutionOptions {
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
        tool_registry: Arc<agendao_tool::ToolRegistry>,
        ctx: agendao_tool::ToolContext,
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

        let ctx = ctx.with_registry(tool_registry.clone());
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
                let mut repair_metadata = agendao_tool::Metadata::new();
                if repaired_tool_name != tool_name {
                    let event = agendao_tool::repair_event_builder(
                        agendao_types::RepairKind::ToolNameRepair.as_str(),
                        "session_prompt",
                        &repaired_tool_name,
                    )
                    .reason("case-insensitive exact tool name match")
                    .raw_shape(serde_json::json!(tool_name))
                    .normalized_shape(serde_json::json!(repaired_tool_name))
                    .build();
                    agendao_tool::append_repair_event(&mut repair_metadata, event);
                }
                let mut effective_tool_name = repaired_tool_name.clone();
                let mut effective_input = input.clone();
                let mut strict_prevalidation_error: Option<String> = None;
                if let Some(payload) =
                    Self::prevalidate_tool_arguments(&effective_tool_name, &effective_input)
                {
                    let is_strict =
                        matches!(options.repair_policy, agendao_types::RepairPolicy::Strict);
                    tracing::warn!(
                        tool_name = %tool_name,
                        normalized_tool = %effective_tool_name,
                        policy = %options.repair_policy.label(),
                        "tool arguments failed prevalidation"
                    );
                    let mut event = agendao_tool::repair_event_builder(
                        agendao_types::RepairKind::ArgumentPrevalidationFallback.as_str(),
                        "session_prompt",
                        &effective_tool_name,
                    )
                    .raw_shape(raw_shape.clone())
                    .normalized_shape(payload.clone())
                    .strict_mode_would_fail(is_strict);
                    if let Some(reason) = payload.get("error").and_then(|value| value.as_str()) {
                        event = event.reason(reason);
                    }
                    agendao_tool::append_repair_event(&mut repair_metadata, event.build());
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
                            (!agendao_tool::repair_events(&repair_metadata).is_empty())
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
                                    agendao_tool::merge_repair_telemetry(
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
                                        agendao_types::RepairPolicy::Strict
                                    );
                                    // Permissive: reroute to invalid for machine-readable errors.
                                    // Strict: return the raw error, don't silently rewrite.
                                    if !is_strict && available_tool_ids.contains("invalid") {
                                        // Record the reroute as a repair event for telemetry.
                                        let error_text = format!("Error: {}", e);
                                        let original_kind = classify_error_kind(&error_text);
                                        let reroute_event = agendao_tool::repair_event_builder(
                                            agendao_types::RepairKind::InvalidToolReroute.as_str(),
                                            "session_prompt",
                                            &effective_tool_name,
                                        )
                                        .reason(error_text)
                                        .original_error_kind(original_kind)
                                        .build();
                                        agendao_tool::append_repair_event(
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
                                                agendao_tool::merge_repair_telemetry(
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
                                                (!agendao_tool::repair_events(&repair_metadata)
                                                    .is_empty())
                                                .then_some(repair_metadata.clone()),
                                                None,
                                                None,
                                            ),
                                        }
                                    } else {
                                        // Strict mode (or no invalid tool): return the raw error.
                                        if is_strict {
                                            let event = agendao_tool::repair_event_builder(
                                                agendao_types::RepairKind::ExecutionErrorNoReroute
                                                    .as_str(),
                                                "session_prompt",
                                                &effective_tool_name,
                                            )
                                            .reason(format!("Error: {}", e))
                                            .raw_shape(raw_shape.clone())
                                            .normalized_shape(effective_input.clone())
                                            .strict_mode_would_fail(true)
                                            .build();
                                            agendao_tool::append_repair_event(
                                                &mut repair_metadata,
                                                event,
                                            );
                                        }
                                        (
                                            format!("Error: {}", e),
                                            true,
                                            Some("Tool Error".to_string()),
                                            (!agendao_tool::repair_events(&repair_metadata)
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
                )
                .await;

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

        // Build and persist a tool batch summary for telemetry / compaction.
        if executed_calls > 0 {
            let summary = session
                .messages
                .get(last_assistant_index)
                .and_then(|msg| Self::build_tool_batch_summary(msg, &[]));
            if let Some(summary) = summary {
                session.insert_metadata(
                    "latest_tool_batch_summary".to_string(),
                    serde_json::to_value(&summary).unwrap_or_default(),
                );
            }
        }

        Ok(executed_calls)
    }

    /// Build a structured `ToolBatchSummary` from the completed tool calls.
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
        let blocked_by: Vec<agendao_types::ToolBatchBlockReason> = {
            let mut reasons: Vec<agendao_types::ToolBatchBlockReason> =
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

        let pending_follow_up: Vec<agendao_types::ToolBatchFollowUpItem> = facts
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
        chat_messages: &mut Vec<agendao_provider::Message>,
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
        chat_messages.push(agendao_provider::Message {
            role: agendao_provider::Role::User,
            content: agendao_provider::Content::Text(context_block),
            cache_control: None,
            provider_options: None,
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use agendao_tool::{Tool, ToolContext, ToolError, ToolResult};
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::Arc;

    struct EchoTool;
    struct AlwaysFailTool;
    struct ExecutionErrorTool;

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
    ) -> Vec<agendao_types::RepairEvent> {
        session.messages[assistant_index]
            .parts
            .iter()
            .find_map(|part| match &part.part_type {
                PartType::ToolCall {
                    state: Some(crate::ToolState::Completed { metadata, .. }),
                    ..
                } => Some(agendao_tool::repair_events(metadata)),
                PartType::ToolCall {
                    state:
                        Some(crate::ToolState::Error {
                            metadata: Some(metadata),
                            ..
                        }),
                    ..
                } => Some(agendao_tool::repair_events(metadata)),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn execute_tool_calls_records_tool_name_repair_only() {
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
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
            serde_json::json!({"value": "hello"}),
        );
        session.messages_mut().push(assistant);
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
            .await
            .expect("execute_tool_calls should succeed");

        let repair_events = tool_state_repair_events(&session, 1);
        assert!(repair_events.iter().any(|event| {
            event.repair_kind == "tool_name_repair"
                && event.raw_shape.as_ref().and_then(|value| value.as_str()) == Some("ECHO_TOOL")
                && event
                    .normalized_shape
                    .as_ref()
                    .and_then(|value| value.as_str())
                    == Some("echo_tool")
        }));
        assert!(!repair_events
            .iter()
            .any(|event| event.repair_kind == "argument_normalization"));
    }

    #[tokio::test]
    async fn execute_tool_calls_persists_prompt_layer_repair_telemetry_on_error() {
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_fail", "FAIL_TOOL", serde_json::json!({}));
        session.messages_mut().push(assistant);
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
            .await
            .expect("execute_tool_calls should complete despite tool failure");

        let repair_events = tool_state_repair_events(&session, 1);
        assert!(repair_events.iter().any(|event| {
            event.repair_kind == "tool_name_repair"
                && event
                    .normalized_shape
                    .as_ref()
                    .and_then(|value| value.as_str())
                    == Some("fail_tool")
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
            goal_status: agendao_types::ToolBatchGoalStatus::Advanced,
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

        let mut chat_messages = vec![agendao_provider::Message {
            role: agendao_provider::Role::User,
            content: agendao_provider::Content::Text("original user request".to_string()),
            cache_control: None,
            provider_options: None,
        }];

        SessionPrompt::inject_latest_tool_batch_summary(&mut session, &mut chat_messages);

        assert_eq!(chat_messages.len(), 2);
        let injected = match &chat_messages[1].content {
            agendao_provider::Content::Text(text) => text.clone(),
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
            agendao_types::ToolBatchGoalStatus::Blocked
        );
        assert_eq!(
            summary.blocked_by,
            vec![agendao_types::ToolBatchBlockReason::ProviderRejected]
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
            agendao_types::ToolBatchGoalStatus::NoProgress
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
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;
        tool_registry
            .register(agendao_tool::invalid::InvalidTool)
            .await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_fail", "fail_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);

        // Strict policy via config store.
        let config = agendao_config::Config {
            repair_policy: Some(agendao_types::RepairPolicy::Strict),
            ..Default::default()
        };
        let config_store = Arc::new(agendao_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
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
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
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

        let config = agendao_config::Config {
            runtime_budget: Some(agendao_config::RuntimeBudgetConfig {
                tool_result_max_chars: 128,
                tool_result_preview_chars: 32,
                ..agendao_config::RuntimeBudgetConfig::default()
            }),
            ..Default::default()
        };
        let config_store = Arc::new(agendao_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
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
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
        tool_registry
            .register(agendao_tool::write::WriteTool::new())
            .await;
        tool_registry
            .register(agendao_tool::invalid::InvalidTool)
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

        let config = agendao_config::Config {
            repair_policy: Some(agendao_types::RepairPolicy::Strict),
            ..Default::default()
        };
        let config_store = Arc::new(agendao_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
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
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;
        tool_registry
            .register(agendao_tool::invalid::InvalidTool)
            .await;

        // ── Permissive ────────────────────────────────────────────
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_fail_p", "fail_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);

        let config = agendao_config::Config {
            repair_policy: Some(agendao_types::RepairPolicy::Permissive),
            ..Default::default()
        };
        let config_store = Arc::new(agendao_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(&mut session, tool_registry.clone(), ctx)
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

        let config2 = agendao_config::Config {
            repair_policy: Some(agendao_types::RepairPolicy::Strict),
            ..Default::default()
        };
        let config_store2 = Arc::new(agendao_config::ConfigStore::new(config2));
        let ctx2 = ToolContext::new(session2.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store2);

        SessionPrompt::execute_tool_calls(&mut session2, tool_registry, ctx2)
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
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
        tool_registry.register(AlwaysFailTool).await;
        // Register the invalid tool so permissive reroute can happen.
        tool_registry
            .register(agendao_tool::invalid::InvalidTool)
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
        // Permissive policy so the reroute happens.
        let config = agendao_config::Config {
            repair_policy: Some(agendao_types::RepairPolicy::Permissive),
            ..Default::default()
        };
        let config_store = Arc::new(agendao_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
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
                .contains(&agendao_types::ToolBatchBlockReason::InvalidArguments),
            "blocked_by should contain InvalidArguments, got {:?}",
            summary.blocked_by
        );
        assert_eq!(
            summary.recommended_next_step,
            Some("fix tool arguments before retrying".to_string())
        );
    }

    // P2.3: an ordinary execution error permissive reroute must NOT be
    // misclassified as invalid_arguments in the batch summary.
    #[tokio::test]
    async fn p23_execution_error_reroute_is_not_classified_as_invalid_arguments() {
        let tool_registry = Arc::new(agendao_tool::ToolRegistry::new());
        // AlwaysFailTool.execute() returns ExecutionError("boom"), which has
        // no validate() — this is a genuine execution error, not bad args.
        tool_registry.register(ExecutionErrorTool).await;
        tool_registry
            .register(agendao_tool::invalid::InvalidTool)
            .await;

        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "run failing tool"));
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_tool_call("call_exec_err", "exec_err_tool", serde_json::json!({}));
        session.messages_mut().push(assistant);
        let config = agendao_config::Config {
            repair_policy: Some(agendao_types::RepairPolicy::Permissive),
            ..Default::default()
        };
        let config_store = Arc::new(agendao_config::ConfigStore::new(config));
        let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string())
            .with_config_store(config_store);

        SessionPrompt::execute_tool_calls(&mut session, tool_registry, ctx)
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
                .contains(&agendao_types::ToolBatchBlockReason::InvalidArguments),
            "execution error reroute must not be misclassified as invalid_arguments"
        );
        // Must be classified as a tool execution error.
        assert!(
            summary
                .blocked_by
                .contains(&agendao_types::ToolBatchBlockReason::ToolExecutionError),
            "execution error reroute should be classified as tool_execution_error"
        );
    }
}
