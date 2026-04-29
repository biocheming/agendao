// Tool execution + subsession methods for SessionPrompt

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use rocode_orchestrator::inline_subtask_request_defaults;
use rocode_provider::{Provider, ToolDefinition};

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
        Self::execute_tool_calls_with_hook(
            session,
            tool_registry,
            ctx,
            provider,
            ToolExecutionOptions {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                hooks: PromptHooks::default(),
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

        let tool_calls: Vec<(String, String, serde_json::Value)> = session.messages
            [last_assistant_index]
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
                    .map(|args| (id.clone(), name.clone(), args))
                }
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            return Ok(0);
        }

        if let Some(assistant_msg) = session.messages_mut().get_mut(last_assistant_index) {
            for (call_id, tool_name, input) in &tool_calls {
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
            for (call_id, tool_name, input) in tool_calls {
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
                let mut effective_tool_name = repaired_tool_name.clone();
                let mut effective_input =
                    if repaired_tool_name == "invalid" && tool_name != "invalid" {
                        Self::invalid_tool_payload(
                            &tool_name,
                            &format!("Unknown tool requested by model: {}", tool_name),
                        )
                    } else {
                        input
                    };
                effective_input =
                    rocode_tool::normalize_tool_arguments(&effective_tool_name, effective_input);
                if effective_tool_name != "invalid" {
                    if let Some(payload) =
                        Self::prevalidate_tool_arguments(&effective_tool_name, &effective_input)
                    {
                        tracing::warn!(
                            tool_name = %tool_name,
                            normalized_tool = %effective_tool_name,
                            "tool arguments failed prevalidation, routing to invalid tool"
                        );
                        effective_tool_name = "invalid".to_string();
                        effective_input = payload;
                    }
                }

                let mut execution = tool_registry
                    .execute(
                        &effective_tool_name,
                        effective_input.clone(),
                        tool_ctx.clone(),
                    )
                    .await;

                if effective_tool_name != "invalid"
                    && available_tool_ids.contains("invalid")
                    && matches!(&execution, Err(rocode_tool::ToolError::InvalidArguments(_)))
                {
                    let validation_error = execution
                        .as_ref()
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Invalid arguments".to_string());
                    tracing::info!(
                        tool_name = %tool_name,
                        error = %validation_error,
                        "tool call validation failed, routing to invalid tool"
                    );
                    effective_tool_name = "invalid".to_string();
                    effective_input = Self::invalid_tool_payload(&tool_name, &validation_error);
                    effective_input = rocode_tool::normalize_tool_arguments(
                        &effective_tool_name,
                        effective_input,
                    );
                    execution = tool_registry
                        .execute(
                            &effective_tool_name,
                            effective_input.clone(),
                            tool_ctx.clone(),
                        )
                        .await;
                }

                let (content, is_error, title, metadata, attachments, state_attachments) =
                    match execution {
                        Ok(result) => {
                            let mut metadata = result.metadata;
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
                        Err(e) => (
                            format!("Error: {}", e),
                            true,
                            Some("Tool Error".to_string()),
                            None,
                            None,
                            None,
                        ),
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

                Self::push_tool_result_part(
                    &mut msg,
                    call_id.clone(),
                    content.clone(),
                    is_error,
                    title.clone(),
                    metadata.clone(),
                    attachments.clone(),
                );
                executed_calls += 1;

                if let Some(assistant_msg) = session.messages_mut().get_mut(last_assistant_index) {
                    let now = chrono::Utc::now().timestamp_millis();
                    let next_state = if is_error {
                        crate::ToolState::Error {
                            input: history_input.clone(),
                            error: content.clone(),
                            metadata: None,
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
        Ok(executed_calls)
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

        if available_tool_ids.contains("invalid") {
            tracing::warn!(
                tool_name = tool_name,
                "unknown tool call, routing to invalid tool"
            );
            return "invalid".to_string();
        }

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

        ctx.with_prompt_subsession(move |session_id, prompt| {
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
                    &prompt,
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
                        prompt,
                        output: output.clone(),
                    });
                }
                Ok(output)
            }
        })
    }

    pub(super) async fn execute_persisted_subsession_prompt(
        subsession: &PersistedSubsession,
        prompt: &str,
        provider: Arc<dyn Provider>,
        tool_registry: Arc<rocode_tool::ToolRegistry>,
        options: PersistedSubsessionPromptOptions,
    ) -> anyhow::Result<String> {
        let model = Self::resolve_subsession_model(
            subsession.model.as_deref(),
            &options.default_model,
            provider.id(),
        );

        let composed_prompt = Self::compose_subsession_prompt(&subsession.history, prompt);
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

        executor
            .execute_inline(provider, &tool_registry, &subsession.disabled_tools)
            .await
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
        prompt: &str,
    ) -> String {
        if history.is_empty() {
            return prompt.to_string();
        }

        let history_text = history
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|turn| format!("User:\n{}\n\nAssistant:\n{}", turn.prompt, turn.output))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        format!(
            "Continue this subtask session.\n\nPrevious conversation:\n{}\n\nNew request:\n{}",
            history_text, prompt
        )
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

    #[test]
    fn persisted_subsessions_roundtrip_via_session_metadata() {
        let mut session = Session::new("proj", ".");
        let mut map = HashMap::new();
        map.insert(
            "task_explore_1".to_string(),
            PersistedSubsession {
                agent: "explore".to_string(),
                model: Some("ethnopic:test-model".to_string()),
                directory: Some("/tmp/project".to_string()),
                disabled_tools: vec!["task".to_string()],
                history: vec![PersistedSubsessionTurn {
                    prompt: "Inspect src".to_string(),
                    output: "Done".to_string(),
                }],
            },
        );

        SessionPrompt::save_persisted_subsessions(&mut session, &map);
        let loaded = SessionPrompt::load_persisted_subsessions(&session);
        assert_eq!(loaded.len(), 1);
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
            prompt: "Find files".to_string(),
            output: "Found 10 files".to_string(),
        }];
        let composed = SessionPrompt::compose_subsession_prompt(&history, "Continue");
        assert!(composed.contains("Previous conversation"));
        assert!(composed.contains("Find files"));
        assert!(composed.contains("Continue"));
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
    fn repair_tool_call_name_routes_unknown_to_invalid() {
        let tools = HashSet::from([
            "read".to_string(),
            "glob".to_string(),
            "invalid".to_string(),
        ]);
        assert_eq!(
            SessionPrompt::repair_tool_call_name("read_html_file", &tools),
            "invalid"
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
}
