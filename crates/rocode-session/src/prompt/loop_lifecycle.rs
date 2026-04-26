use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use rocode_execution_types::{session_runtime_request_defaults, CompiledExecutionRequest};
use rocode_content::output_blocks::{OutputBlock, StatusBlock};
use rocode_orchestrator::runtime::events::{
    CancelToken as RuntimeCancelToken, LoopError as RuntimeLoopError,
};
use rocode_orchestrator::runtime::policy::{LoopPolicy, ToolDedupScope};
use rocode_orchestrator::runtime::run_loop;
use rocode_orchestrator::runtime::{SimpleModelCaller, SimpleModelCallerConfig};
use rocode_plugin::{HookContext, HookEvent};
use rocode_provider::transform::{apply_caching, ProviderType};
use rocode_provider::{Provider, ToolDefinition};

use crate::compaction::{run_compaction, CompactionResult};
use crate::message_v2::ModelRef as V2ModelRef;
use crate::{MessageRole, Session, SessionMessage};

use super::runtime_step::{SessionStepRuntimeOutput, SessionStepSink, SessionStepToolDispatcher};
use super::{
    apply_chat_message_hook_outputs, apply_chat_messages_hook_outputs, is_terminal_finish,
    merge_tool_definitions, session_message_hook_payload, skill_reflection, tools_and_output,
    PromptHooks, PromptInput, PromptRequestContext, SessionPrompt, SessionStepShared, MAX_STEPS,
    STREAM_UPDATE_INTERVAL_MS,
};

#[derive(Clone)]
struct SessionStepCancelToken {
    user_cancel: CancellationToken,
    step_complete: Arc<AtomicBool>,
}

impl RuntimeCancelToken for SessionStepCancelToken {
    fn is_cancelled(&self) -> bool {
        self.user_cancel.is_cancelled() || self.step_complete.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
struct PromptLoopContext {
    provider: Arc<dyn Provider>,
    model_id: String,
    provider_id: String,
    agent_name: Option<String>,
    system_prompt: Option<String>,
    tools: Vec<ToolDefinition>,
    compiled_request: CompiledExecutionRequest,
    hooks: PromptHooks,
    config_store: Option<Arc<rocode_config::ConfigStore>>,
}

#[derive(Clone)]
struct RuntimeStepContext {
    provider: Arc<dyn Provider>,
    model_id: String,
    provider_id: String,
    agent_name: Option<String>,
    compiled_request: CompiledExecutionRequest,
    hooks: PromptHooks,
    config_store: Option<Arc<rocode_config::ConfigStore>>,
}

struct RuntimeStepInput {
    session_id: String,
    assistant_index: usize,
    chat_messages: Vec<rocode_provider::Message>,
    tool_registry: Arc<rocode_tool::ToolRegistry>,
    step_ctx: RuntimeStepContext,
}

impl SessionPrompt {
    async fn emit_context_compaction_status(
        output_block_hook: Option<&super::OutputBlockHook>,
        session_id: &str,
        block: StatusBlock,
    ) {
        let Some(output_block_hook) = output_block_hook else {
            return;
        };
        output_block_hook(super::OutputBlockEvent {
            session_id: session_id.to_string(),
            block: OutputBlock::Status(block),
            id: None,
        })
        .await;
    }

    pub async fn prompt_with_update_hook(
        &self,
        input: PromptInput,
        session: &mut Session,
        request: PromptRequestContext,
    ) -> anyhow::Result<()> {
        let PromptRequestContext {
            provider,
            system_prompt,
            memory_prefetch,
            tools,
            compiled_request,
            hooks,
        } = request;
        let system_prompt =
            skill_reflection::augment_system_prompt_with_skill_reflection(session, system_prompt);

        self.assert_not_busy(&input.session_id).await?;

        let cancel_token = self.start(&input.session_id).await;
        let token = match cancel_token {
            Some(t) => t,
            None => return Err(anyhow::anyhow!("Session already running")),
        };

        let model_id = input
            .model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let provider_id = input
            .model
            .as_ref()
            .map(|m| m.provider_id.clone())
            .unwrap_or_else(|| "ethnopic".to_string());

        self.create_user_message(&input, session).await?;
        self.apply_runtime_workspace_context(session).await?;
        Self::apply_runtime_memory_prefetch(session, memory_prefetch.as_ref())?;
        Self::annotate_latest_user_message(session, &input, system_prompt.as_deref());

        if session.is_default_title() {
            if let Some(text) = session
                .messages
                .iter()
                .find(|m| matches!(m.role, MessageRole::User))
                .map(|m| m.get_text())
            {
                let immediate = tools_and_output::generate_session_title(
                    &tools_and_output::sanitize_session_title_source(&text),
                );
                if !immediate.is_empty() && immediate != "New Session" {
                    session.set_auto_title(immediate);
                }
            }
        }

        session.touch();
        Self::emit_session_update(hooks.update_hook.as_ref(), session);

        if input.no_reply {
            self.finish_run(&input.session_id).await;
            return Ok(());
        }

        {
            let mut session_state = self.session_state.write().await;
            session_state.set_busy(&input.session_id);
        }

        let session_id = input.session_id.clone();

        let result = self
            .loop_inner(
                session_id.clone(),
                token,
                session,
                PromptLoopContext {
                    provider,
                    model_id,
                    provider_id,
                    agent_name: input.agent.clone(),
                    system_prompt,
                    tools,
                    compiled_request,
                    hooks,
                    config_store: self.config_store.clone(),
                },
            )
            .await;

        self.finish_run(&session_id).await;

        if let Err(e) = result {
            tracing::error!("Prompt loop error for session {}: {}", session_id, e);
            return Err(e);
        }

        Ok(())
    }

    pub async fn resume_session(
        &self,
        session_id: &str,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        compiled_request: CompiledExecutionRequest,
    ) -> anyhow::Result<()> {
        let system_prompt =
            skill_reflection::augment_system_prompt_with_skill_reflection(session, system_prompt);
        let token = self.resume(session_id).await;

        let token = match token {
            Some(t) => t,
            None => {
                return Err(anyhow::anyhow!(
                    "Session {} is not running, cannot resume",
                    session_id
                ));
            }
        };

        let model = session.messages.iter().rev().find_map(|m| match m.role {
            MessageRole::User => session
                .metadata
                .get("model_provider")
                .and_then(|p| p.as_str())
                .zip(session.metadata.get("model_id").and_then(|i| i.as_str()))
                .map(|(provider_id, model_id)| super::ModelRef {
                    provider_id: provider_id.to_string(),
                    model_id: model_id.to_string(),
                }),
            _ => None,
        });

        let model_id = model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let provider_id = model
            .as_ref()
            .map(|m| m.provider_id.clone())
            .unwrap_or_else(|| "ethnopic".to_string());

        let session_id = session_id.to_string();
        let resume_agent = session
            .metadata
            .get("agent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let compiled_request = compiled_request.inherit_missing(&session_runtime_request_defaults(
            session
                .metadata
                .get("model_variant")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        ));

        {
            let mut session_state = self.session_state.write().await;
            session_state.set_busy(&session_id);
        }

        let result = self
            .loop_inner(
                session_id.clone(),
                token,
                session,
                PromptLoopContext {
                    provider,
                    model_id,
                    provider_id,
                    agent_name: resume_agent,
                    system_prompt,
                    tools,
                    compiled_request: compiled_request.clone(),
                    hooks: PromptHooks::default(),
                    config_store: self.config_store.clone(),
                },
            )
            .await;

        self.finish_run(&session_id).await;

        if let Err(e) = result {
            tracing::error!("Resume prompt loop error for session {}: {}", session_id, e);
            return Err(e);
        }

        Ok(())
    }

    async fn run_runtime_step(
        &self,
        token: CancellationToken,
        session: &mut Session,
        resolved_tools: Vec<ToolDefinition>,
        input: RuntimeStepInput,
    ) -> anyhow::Result<SessionStepRuntimeOutput> {
        let assistant_message_id = session
            .messages
            .get(input.assistant_index)
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let shared = Arc::new(Mutex::new(SessionStepShared {
            assistant_message_id: Some(assistant_message_id),
        }));
        let step_complete = Arc::new(AtomicBool::new(false));
        let cancel = SessionStepCancelToken {
            user_cancel: token.clone(),
            step_complete: step_complete.clone(),
        };

        let subsessions = Arc::new(Mutex::new(Self::load_persisted_subsessions(session)));

        let model = SimpleModelCaller {
            provider: input.step_ctx.provider.clone(),
            config: SimpleModelCallerConfig {
                request: input
                    .step_ctx
                    .compiled_request
                    .with_model(input.step_ctx.model_id.clone())
                    .inherit_missing(&session_runtime_request_defaults(None)),
            },
        };
        let allowed_tools = Some(Arc::new(
            resolved_tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<HashSet<_>>(),
        ));

        let tools = SessionStepToolDispatcher {
            session_id: input.session_id.clone(),
            directory: session.directory.clone(),
            agent_name: input.step_ctx.agent_name.clone().unwrap_or_default(),
            abort_token: token.clone(),
            tool_registry: input.tool_registry,
            provider: input.step_ctx.provider.clone(),
            provider_id: input.step_ctx.provider_id.clone(),
            model_id: input.step_ctx.model_id.clone(),
            resolved_tools,
            allowed_tools,
            shared,
            subsessions: subsessions.clone(),
            agent_lookup: input.step_ctx.hooks.agent_lookup.clone(),
            ask_question_hook: input.step_ctx.hooks.ask_question_hook.clone(),
            ask_permission_hook: input.step_ctx.hooks.ask_permission_hook.clone(),
            publish_bus_hook: input.step_ctx.hooks.publish_bus_hook.clone(),
            tool_runtime_config: self.tool_runtime_config.clone(),
            config_store: input.step_ctx.config_store.clone(),
            runtime_skill_instructions: session.metadata.get("runtime_skill_instructions").cloned(),
        };

        let mut sink = SessionStepSink::new(
            session,
            input.assistant_index,
            input.step_ctx.hooks.update_hook.as_ref(),
            input.step_ctx.hooks.event_broadcast.as_ref(),
            input.step_ctx.hooks.output_block_hook.as_ref(),
            step_complete,
        );
        let policy = LoopPolicy {
            max_steps: Some(MAX_STEPS),
            tool_dedup: ToolDedupScope::None,
            ..Default::default()
        };
        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &policy,
            &cancel,
            input.chat_messages,
        )
        .await;
        let output = sink.into_output();

        let persisted = subsessions.lock().await.clone();
        Self::save_persisted_subsessions(session, &persisted);

        match outcome {
            Ok(_) => Ok(output),
            Err(RuntimeLoopError::ModelError(message)) => Err(anyhow::anyhow!("{}", message)),
            Err(RuntimeLoopError::ToolDispatchError { tool, error }) => {
                let lower = error.to_ascii_lowercase();
                if token.is_cancelled()
                    || lower.contains("cancelled")
                    || lower.contains("canceled")
                    || lower.contains("aborted")
                {
                    Ok(output)
                } else {
                    Err(anyhow::anyhow!(
                        "Tool dispatch failed ({}): {}",
                        tool,
                        error
                    ))
                }
            }
            Err(RuntimeLoopError::Cancelled) => Ok(output),
            Err(RuntimeLoopError::SinkError(message)) | Err(RuntimeLoopError::Other(message)) => {
                Err(anyhow::anyhow!("{}", message))
            }
        }
    }

    async fn maybe_compact_context(
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        session: &mut Session,
        provider: &Arc<dyn Provider>,
        filtered_messages: &[SessionMessage],
        compiled_request: &CompiledExecutionRequest,
        config_store: Option<&rocode_config::ConfigStore>,
        output_block_hook: Option<&super::OutputBlockHook>,
    ) {
        let compaction_config = Self::runtime_compaction_config(config_store);
        if !Self::should_compact(
            filtered_messages,
            provider.as_ref(),
            model_id,
            compiled_request.max_tokens,
            &compaction_config,
        ) {
            return;
        }

        tracing::info!(
            "Context overflow detected, triggering compaction for session {}",
            session_id
        );
        Self::emit_context_compaction_status(
            output_block_hook,
            session_id,
            StatusBlock::warning("Auto-compacting context to stay within the model window..."),
        )
        .await;

        let parent_id = filtered_messages
            .last()
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let compaction_messages =
            Self::build_chat_messages(filtered_messages, None).unwrap_or_default();
        let model_ref = V2ModelRef {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        };

        match run_compaction::<crate::compaction::NoopSessionOps>(
            session_id,
            &parent_id,
            compaction_messages,
            model_ref,
            provider.clone(),
            crate::compaction::RunCompactionOptions {
                abort: CancellationToken::new(),
                auto: true,
                config: Some(compaction_config.clone()),
                session_ops: None,
            },
        )
        .await
        {
            Ok(CompactionResult::Continue) => {
                tracing::info!(
                    "LLM compaction complete for session {}, continuing",
                    session_id
                );
                Self::emit_context_compaction_status(
                    output_block_hook,
                    session_id,
                    StatusBlock::success("Context compacted. Continuing the session."),
                )
                .await;
            }
            Ok(CompactionResult::Stop) => {
                tracing::warn!(
                    "LLM compaction returned stop for session {}, falling back to simple compaction",
                    session_id
                );
                if let Some(summary) = Self::trigger_compaction(session, filtered_messages, None) {
                    tracing::info!("Fallback compaction (from stop) complete: {}", summary);
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::success("Context compacted. Continuing the session."),
                    )
                    .await;
                } else {
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::warning(
                            "Auto-compaction ran, but the context could not be reduced.",
                        ),
                    )
                    .await;
                }
            }
            Err(e) => {
                tracing::warn!(
                    "LLM compaction failed for session {}: {}, falling back to simple compaction",
                    session_id,
                    e
                );
                if let Some(summary) = Self::trigger_compaction(session, filtered_messages, None) {
                    tracing::info!("Fallback compaction complete: {}", summary);
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::success("Context compacted. Continuing the session."),
                    )
                    .await;
                } else {
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::warning(
                            "Auto-compaction failed, and the context could not be reduced.",
                        ),
                    )
                    .await;
                }
            }
        }
    }

    async fn prepare_chat_messages(
        session_id: &str,
        agent_name: Option<&str>,
        system_prompt: Option<&str>,
        mut filtered_messages: Vec<SessionMessage>,
        provider_type: ProviderType,
    ) -> anyhow::Result<Vec<rocode_provider::Message>> {
        if rocode_plugin::should_trigger_script_hooks(HookEvent::ChatMessagesTransform, agent_name)
            .await
        {
            let hook_messages = serde_json::Value::Array(
                filtered_messages
                    .iter()
                    .map(session_message_hook_payload)
                    .collect(),
            );
            let message_hook_outputs = rocode_plugin::trigger_collect(
                HookContext::new(HookEvent::ChatMessagesTransform)
                    .with_session(session_id)
                    .with_data("message_count", serde_json::json!(filtered_messages.len()))
                    .with_data("messages", hook_messages),
            )
            .await;
            apply_chat_messages_hook_outputs(&mut filtered_messages, message_hook_outputs);
        }

        let mut prompt_messages = filtered_messages;
        if let Some(agent) = agent_name {
            let was_plan = super::was_plan_agent(&prompt_messages);
            prompt_messages = super::insert_reminders(&prompt_messages, agent, was_plan);
        }

        let mut chat_messages = Self::build_chat_messages(&prompt_messages, system_prompt)?;
        apply_caching(&mut chat_messages, provider_type);
        Ok(chat_messages)
    }

    fn finalize_assistant_message(
        session: &mut Session,
        assistant_index: usize,
        step_output: &SessionStepRuntimeOutput,
    ) {
        if let Some(assistant_msg) = session.messages_mut().get_mut(assistant_index) {
            if let Some(reason) = step_output.finish_reason.clone() {
                assistant_msg
                    .metadata
                    .insert("finish_reason".to_string(), serde_json::json!(reason));
            }
            assistant_msg.metadata.insert(
                "completed_at".to_string(),
                serde_json::json!(chrono::Utc::now().timestamp_millis()),
            );
            assistant_msg.metadata.insert(
                "usage".to_string(),
                serde_json::json!({
                    "prompt_tokens": step_output.prompt_tokens,
                    "completion_tokens": step_output.completion_tokens,
                    "reasoning_tokens": step_output.reasoning_tokens,
                    "cache_read_tokens": step_output.cache_read_tokens,
                    "cache_write_tokens": step_output.cache_write_tokens,
                }),
            );
            assistant_msg.metadata.insert(
                "tokens_input".to_string(),
                serde_json::json!(step_output.prompt_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_output".to_string(),
                serde_json::json!(step_output.completion_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_reasoning".to_string(),
                serde_json::json!(step_output.reasoning_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_cache_read".to_string(),
                serde_json::json!(step_output.cache_read_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_cache_write".to_string(),
                serde_json::json!(step_output.cache_write_tokens),
            );
            assistant_msg.usage = Some(crate::message::MessageUsage {
                input_tokens: step_output.prompt_tokens,
                output_tokens: step_output.completion_tokens,
                reasoning_tokens: step_output.reasoning_tokens,
                cache_read_tokens: step_output.cache_read_tokens,
                cache_write_tokens: step_output.cache_write_tokens,
                ..Default::default()
            });
        }
    }

    async fn run_chat_message_hook(
        session: &mut Session,
        session_id: &str,
        assistant_index: usize,
        agent_name: Option<&str>,
        provider: &Arc<dyn Provider>,
        model_id: &str,
        has_tool_calls: bool,
    ) {
        if !rocode_plugin::should_trigger_script_hooks(HookEvent::ChatMessage, agent_name).await {
            return;
        }
        let Some(assistant_msg) = session.messages.get(assistant_index).cloned() else {
            return;
        };

        let mut hook_ctx = HookContext::new(HookEvent::ChatMessage)
            .with_session(session_id)
            .with_data("message_id", serde_json::json!(&assistant_msg.id))
            .with_data("message", session_message_hook_payload(&assistant_msg))
            .with_data("parts", serde_json::json!(&assistant_msg.parts))
            .with_data("has_tool_calls", serde_json::json!(has_tool_calls));

        if let Some(model) = provider.get_model(model_id) {
            hook_ctx = hook_ctx.with_data(
                "model",
                serde_json::json!({
                    "id": model.id,
                    "name": model.name,
                    "provider": model.provider,
                }),
            );
        } else {
            hook_ctx = hook_ctx.with_data("model_id", serde_json::json!(model_id));
        }
        hook_ctx = hook_ctx.with_data("sessionID", serde_json::json!(session_id));
        if let Some(agent) = agent_name {
            hook_ctx = hook_ctx.with_data("agent", serde_json::json!(agent));
        }

        let hook_outputs = rocode_plugin::trigger_collect(hook_ctx).await;
        if let Some(current_assistant) = session.messages_mut().get_mut(assistant_index) {
            apply_chat_message_hook_outputs(current_assistant, hook_outputs);
        }
    }

    async fn loop_inner(
        &self,
        session_id: String,
        token: CancellationToken,
        session: &mut Session,
        prompt_ctx: PromptLoopContext,
    ) -> anyhow::Result<()> {
        let mut step = 0u32;
        let provider_type = ProviderType::from_provider_id(&prompt_ctx.provider_id);
        let mut post_first_step_ran = false;
        let turn_start_index = session.messages.len().saturating_sub(1);

        loop {
            if token.is_cancelled() {
                tracing::info!("Prompt loop cancelled for session {}", session_id);
                break;
            }

            let filtered_messages = Self::filter_compacted_messages(&session.messages);

            let last_user_idx = filtered_messages
                .iter()
                .rposition(|m| matches!(m.role, MessageRole::User));

            let last_assistant_idx = filtered_messages
                .iter()
                .rposition(|m| matches!(m.role, MessageRole::Assistant));

            let last_user_idx = match last_user_idx {
                Some(idx) => idx,
                None => return Err(anyhow::anyhow!("No user message found")),
            };

            if self
                .process_pending_subtasks(
                    session,
                    prompt_ctx.provider.clone(),
                    &prompt_ctx.provider_id,
                    &prompt_ctx.model_id,
                    &prompt_ctx.hooks,
                )
                .await?
            {
                tracing::info!("Processed pending subtask parts for session {}", session_id);
                continue;
            }

            if let Some(assistant_idx) = last_assistant_idx {
                let assistant = &filtered_messages[assistant_idx];
                if is_terminal_finish(assistant.finish.as_deref()) && last_user_idx < assistant_idx
                {
                    tracing::info!(
                        finish = ?assistant.finish,
                        "Prompt loop complete for session {}", session_id
                    );
                    break;
                }
            }

            step += 1;
            if step > MAX_STEPS {
                tracing::warn!("Max steps reached for session {}", session_id);
                break;
            }

            Self::maybe_compact_context(
                &session_id,
                &prompt_ctx.provider_id,
                &prompt_ctx.model_id,
                session,
                &prompt_ctx.provider,
                &filtered_messages,
                &prompt_ctx.compiled_request,
                prompt_ctx.config_store.as_deref(),
                prompt_ctx.hooks.output_block_hook.as_ref(),
            )
            .await;

            tracing::info!(
                step = step,
                session_id = %session_id,
                message_count = filtered_messages.len(),
                "prompt loop step start"
            );

            let chat_messages = Self::prepare_chat_messages(
                &session_id,
                prompt_ctx.agent_name.as_deref(),
                prompt_ctx.system_prompt.as_deref(),
                filtered_messages,
                provider_type,
            )
            .await?;
            let resolved_tools = merge_tool_definitions(
                prompt_ctx.tools.clone(),
                Self::mcp_tools_from_session(session),
            );

            let tool_registry = Arc::new(rocode_tool::create_default_registry().await);

            let assistant_index = session.messages.len();
            let assistant_message_id =
                rocode_core::id::create(rocode_core::id::Prefix::Message, true, None);
            let mut assistant_metadata = HashMap::new();
            assistant_metadata.insert(
                "model_provider".to_string(),
                serde_json::json!(&prompt_ctx.provider_id),
            );
            assistant_metadata.insert(
                "model_id".to_string(),
                serde_json::json!(&prompt_ctx.model_id),
            );
            if let Some(agent) = prompt_ctx.agent_name.as_deref() {
                assistant_metadata.insert("agent".to_string(), serde_json::json!(agent));
                assistant_metadata.insert("mode".to_string(), serde_json::json!(agent));
            }
            session.messages_mut().push(SessionMessage {
                id: assistant_message_id,
                session_id: session_id.clone(),
                role: MessageRole::Assistant,
                parts: Vec::new(),
                created_at: chrono::Utc::now(),
                metadata: assistant_metadata,
                usage: None,
                finish: None,
            });
            session.touch();
            Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);

            let step_output = self
                .run_runtime_step(
                    token.clone(),
                    session,
                    resolved_tools,
                    RuntimeStepInput {
                        session_id: session_id.clone(),
                        assistant_index,
                        chat_messages,
                        tool_registry: tool_registry.clone(),
                        step_ctx: RuntimeStepContext {
                            provider: prompt_ctx.provider.clone(),
                            model_id: prompt_ctx.model_id.clone(),
                            provider_id: prompt_ctx.provider_id.clone(),
                            agent_name: prompt_ctx.agent_name.clone(),
                            compiled_request: prompt_ctx.compiled_request.clone(),
                            hooks: prompt_ctx.hooks.clone(),
                            config_store: prompt_ctx.config_store.clone(),
                        },
                    },
                )
                .await?;

            let finish_reason = step_output.finish_reason.clone();
            let executed_local_tools_this_step = step_output.executed_local_tools_this_step;

            Self::finalize_assistant_message(session, assistant_index, &step_output);

            if !step_output.stream_tool_results.is_empty() {
                let mut tool_msg = SessionMessage::tool(session_id.clone());
                for (tool_call_id, content, is_error, title, metadata, attachments) in
                    step_output.stream_tool_results
                {
                    Self::push_tool_result_part(
                        &mut tool_msg,
                        tool_call_id,
                        content,
                        is_error,
                        title,
                        metadata,
                        attachments,
                    );
                }
                session.messages_mut().push(tool_msg);
            }

            let has_tool_calls = session
                .messages
                .get(assistant_index)
                .map(Self::has_unresolved_tool_calls)
                .unwrap_or(false);

            session.touch();
            Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);

            Self::run_chat_message_hook(
                session,
                &session_id,
                assistant_index,
                prompt_ctx.agent_name.as_deref(),
                &prompt_ctx.provider,
                &prompt_ctx.model_id,
                has_tool_calls,
            )
            .await;

            if executed_local_tools_this_step {
                continue;
            }

            if !post_first_step_ran {
                Self::ensure_title(session, prompt_ctx.provider.clone(), &prompt_ctx.model_id)
                    .await;
                let _ = Self::summarize_session(
                    session,
                    &session_id,
                    &prompt_ctx.provider_id,
                    &prompt_ctx.model_id,
                    prompt_ctx.provider.as_ref(),
                )
                .await;
                post_first_step_ran = true;
            }

            if is_terminal_finish(finish_reason.as_deref()) {
                Self::maybe_append_runtime_skill_save_suggestion(session, turn_start_index);
                skill_reflection::update_skill_reflection_metadata(
                    self.config_store.clone(),
                    session,
                );
                Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);
                tracing::info!(
                    "Prompt loop complete for session {} with finish: {:?}",
                    session_id,
                    finish_reason
                );
                break;
            }
        }

        if token.is_cancelled() {
            Self::abort_pending_tool_calls(session);
        }

        let compaction_config = Self::runtime_compaction_config(prompt_ctx.config_store.as_deref());
        Self::prune_after_loop(session, &compaction_config);
        session.touch();
        Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);

        Ok(())
    }

    pub(super) fn emit_session_update(
        update_hook: Option<&super::SessionUpdateHook>,
        session: &Session,
    ) {
        if let Some(hook) = update_hook {
            hook(session);
        }
    }

    pub(super) fn maybe_emit_session_update(
        update_hook: Option<&super::SessionUpdateHook>,
        session: &Session,
        last_emit: &mut Instant,
        force: bool,
    ) {
        let elapsed = last_emit.elapsed();
        if force || elapsed >= Duration::from_millis(STREAM_UPDATE_INTERVAL_MS) {
            Self::emit_session_update(update_hook, session);
            *last_emit = Instant::now();
        }
    }
}
