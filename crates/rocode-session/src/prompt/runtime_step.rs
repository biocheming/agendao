use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use rocode_content::output_blocks::{
    MessageBlock, MessageRole as OutputMessageRole, OutputBlock, ReasoningBlock, ToolBlock,
};
use rocode_orchestrator::runtime::events::{
    LoopError as RuntimeLoopError, LoopEvent, StepBoundary, ToolCallReady as RuntimeToolCallReady,
    ToolResult as RuntimeToolResult,
};
use rocode_orchestrator::runtime::traits::{LoopSink, ToolDispatcher};
use rocode_provider::{Provider, ToolDefinition};

use crate::tool_result_governance::{
    default_tool_result_artifacts_root, govern_tool_result_output, ToolResultBudget,
};
use crate::Session;

use super::{
    assistant_reasoning_live_identity, assistant_text_live_identity, tool_call_live_identity,
    tool_result_live_identity,
    tool_progress_detail, tool_result_detail, AgentLookup, AskPermissionHook, AskQuestionHook,
    EventBroadcastHook, OutputBlockEvent, OutputBlockHook, PersistedSubsession, PublishBusHook,
    SessionPrompt, SessionStepShared, SessionUpdateHook, StreamToolResultEntry, StreamToolState,
    STREAM_UPDATE_INTERVAL_MS,
};

pub(super) struct SessionToolExecutor {
    pub(super) tool_registry: Arc<rocode_tool::ToolRegistry>,
    pub(super) tool_ctx_builder: Arc<dyn Fn() -> rocode_tool::ToolContext + Send + Sync>,
    pub(super) allowed_tools: Option<Arc<std::collections::HashSet<String>>>,
}

#[async_trait]
impl rocode_orchestrator::ToolExecutor for SessionToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        _exec_ctx: &rocode_orchestrator::ExecutionContext,
    ) -> Result<rocode_orchestrator::ToolOutput, rocode_orchestrator::ToolExecError> {
        if let Some(allowed_tools) = self.allowed_tools.as_ref() {
            if !allowed_tools.contains(tool_name) {
                return Err(rocode_orchestrator::ToolExecError::PermissionDenied(
                    format!("Tool `{}` is not allowed in this session", tool_name),
                ));
            }
        }
        let ctx = (self.tool_ctx_builder)();
        let result = self
            .tool_registry
            .execute(tool_name, arguments, ctx)
            .await
            .map_err(|e| match e {
                rocode_tool::ToolError::InvalidArguments(msg) => {
                    rocode_orchestrator::ToolExecError::InvalidArguments(msg)
                }
                rocode_tool::ToolError::PermissionDenied(msg) => {
                    rocode_orchestrator::ToolExecError::PermissionDenied(msg)
                }
                rocode_tool::ToolError::Cancelled => {
                    rocode_orchestrator::ToolExecError::ExecutionError("cancelled".to_string())
                }
                other => rocode_orchestrator::ToolExecError::ExecutionError(other.to_string()),
            })?;
        Ok(rocode_orchestrator::ToolOutput {
            output: result.output,
            is_error: false,
            title: if result.title.is_empty() {
                None
            } else {
                Some(result.title)
            },
            metadata: if result.metadata.is_empty() {
                None
            } else {
                Some(serde_json::to_value(result.metadata).unwrap_or(serde_json::Value::Null))
            },
        })
    }

    async fn list_ids(&self) -> Vec<String> {
        let mut ids = self.tool_registry.list_ids().await;
        if let Some(allowed_tools) = self.allowed_tools.as_ref() {
            ids.retain(|id| allowed_tools.contains(id));
        }
        ids
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &rocode_orchestrator::ExecutionContext,
    ) -> Vec<ToolDefinition> {
        let mut tools: Vec<ToolDefinition> = self
            .tool_registry
            .list_schemas()
            .await
            .into_iter()
            .map(|s| ToolDefinition {
                name: s.name,
                description: Some(s.description),
                parameters: s.parameters,
            })
            .collect();
        if let Some(allowed_tools) = self.allowed_tools.as_ref() {
            tools.retain(|tool| allowed_tools.contains(&tool.name));
        }
        super::prioritize_tool_definitions(&mut tools);
        tools
    }
}

pub(super) struct SessionStepToolDispatcher {
    pub(super) session_id: String,
    pub(super) directory: String,
    pub(super) agent_name: String,
    pub(super) abort_token: CancellationToken,
    pub(super) tool_registry: Arc<rocode_tool::ToolRegistry>,
    pub(super) provider: Arc<dyn Provider>,
    pub(super) provider_id: String,
    pub(super) model_id: String,
    pub(super) resolved_tools: Vec<ToolDefinition>,
    pub(super) allowed_tools: Option<Arc<std::collections::HashSet<String>>>,
    pub(super) shared: Arc<Mutex<SessionStepShared>>,
    pub(super) subsessions: Arc<Mutex<HashMap<String, PersistedSubsession>>>,
    pub(super) agent_lookup: Option<AgentLookup>,
    pub(super) ask_question_hook: Option<AskQuestionHook>,
    pub(super) ask_permission_hook: Option<AskPermissionHook>,
    pub(super) publish_bus_hook: Option<PublishBusHook>,
    pub(super) tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    pub(super) config_store: Option<Arc<rocode_config::ConfigStore>>,
    pub(super) runtime_skill_instructions: Option<serde_json::Value>,
}

#[async_trait]
impl ToolDispatcher for SessionStepToolDispatcher {
    async fn execute(&self, call: &RuntimeToolCallReady) -> RuntimeToolResult {
        let message_id = {
            let shared = self.shared.lock().await;
            shared
                .assistant_message_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string())
        };

        let default_model = format!("{}:{}", self.provider_id, self.model_id);
        let session_id = self.session_id.clone();
        let directory = self.directory.clone();
        let agent_name = self.agent_name.clone();
        let abort_token = self.abort_token.clone();
        let subsessions = self.subsessions.clone();
        let provider = self.provider.clone();
        let tool_registry = self.tool_registry.clone();
        let agent_lookup = self.agent_lookup.clone();
        let ask_question_hook = self.ask_question_hook.clone();
        let ask_permission_hook = self.ask_permission_hook.clone();
        let publish_bus_hook = self.publish_bus_hook.clone();
        let call_id = call.id.clone();
        let tool_runtime_config = self.tool_runtime_config.clone();
        let config_store = self.config_store.clone();
        let runtime_skill_instructions = self.runtime_skill_instructions.clone();

        let tool_ctx_builder = Arc::new(move || {
            let mut base_ctx = rocode_tool::ToolContext::new(
                session_id.clone(),
                message_id.clone(),
                directory.clone(),
            )
            .with_agent(agent_name.clone())
            .with_tool_runtime_config(tool_runtime_config.clone())
            .with_abort(abort_token.clone());
            if let Some(config_store) = config_store.clone() {
                base_ctx = base_ctx.with_config_store(config_store);
            }
            if let Some(runtime_skill_instructions) = runtime_skill_instructions.clone() {
                base_ctx.extra.insert(
                    "runtime_skill_instructions".to_string(),
                    runtime_skill_instructions,
                );
            }
            base_ctx.call_id = Some(call_id.clone());
            let ctx = SessionPrompt::with_persistent_subsession_callbacks(
                base_ctx,
                subsessions.clone(),
                provider.clone(),
                tool_registry.clone(),
                default_model.clone(),
                agent_lookup.clone(),
                ask_question_hook.clone(),
                ask_permission_hook.clone(),
            )
            .with_registry(tool_registry.clone());
            if let Some(ref hook) = publish_bus_hook {
                let hook = hook.clone();
                ctx.with_publish_bus(move |event_type, properties| {
                    let hook = hook.clone();
                    async move { hook(event_type, properties).await }
                })
            } else {
                ctx
            }
        });

        let executor = Arc::new(SessionToolExecutor {
            tool_registry: self.tool_registry.clone(),
            tool_ctx_builder,
            allowed_tools: self.allowed_tools.clone(),
        });
        let tool_runner = rocode_orchestrator::ToolRunner::new(executor);
        let exec_ctx = rocode_orchestrator::ExecutionContext {
            session_id: self.session_id.clone(),
            workdir: self.directory.clone(),
            agent_name: self.agent_name.clone(),
            metadata: std::collections::HashMap::new(),
        };

        let input = rocode_orchestrator::tool_runner::ToolCallInput {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        };
        let output = tool_runner.execute_tool_call(input, &exec_ctx).await;

        RuntimeToolResult {
            tool_call_id: output.tool_call_id,
            tool_name: output.tool_name,
            output: output.content,
            is_error: output.is_error,
            title: output.title,
            metadata: output.metadata,
        }
    }

    async fn list_definitions(&self) -> Vec<ToolDefinition> {
        self.resolved_tools.clone()
    }
}

pub(super) struct SessionStepRuntimeOutput {
    pub(super) stream_tool_results: Vec<StreamToolResultEntry>,
    pub(super) finish_reason: Option<String>,
    pub(super) prompt_tokens: u64,
    pub(super) completion_tokens: u64,
    pub(super) reasoning_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) cache_miss_tokens: u64,
    pub(super) cache_write_tokens: u64,
    pub(super) executed_local_tools_this_step: bool,
    /// P3-F: Why the provider stream terminated. Used by the server to
    /// decide whether a backfill reconcile is needed.
    pub(super) stream_termination: Option<rocode_provider::StreamTermination>,
}

pub(super) struct SessionStepSink<'a> {
    pub(super) session: &'a mut Session,
    pub(super) assistant_index: usize,
    pub(super) update_hook: Option<&'a SessionUpdateHook>,
    pub(super) event_broadcast: Option<&'a EventBroadcastHook>,
    pub(super) output_block_hook: Option<&'a OutputBlockHook>,
    pub(super) last_emit: Instant,
    pub(super) tool_calls: HashMap<String, StreamToolState>,
    pub(super) stream_tool_results: Vec<StreamToolResultEntry>,
    pub(super) finish_reason: Option<String>,
    pub(super) prompt_tokens: u64,
    pub(super) completion_tokens: u64,
    pub(super) reasoning_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) cache_miss_tokens: u64,
    pub(super) cache_write_tokens: u64,
    pub(super) executed_local_tools_this_step: bool,
    pub(super) stream_termination: Option<rocode_provider::StreamTermination>,
    pub(super) step_complete: Arc<AtomicBool>,
    pub(super) assistant_output_started: bool,
    pub(super) reasoning_output_started: bool,
    pub(super) tool_result_budget: ToolResultBudget,
}

impl<'a> SessionStepSink<'a> {
    pub(super) fn new(
        session: &'a mut Session,
        assistant_index: usize,
        update_hook: Option<&'a SessionUpdateHook>,
        event_broadcast: Option<&'a EventBroadcastHook>,
        output_block_hook: Option<&'a OutputBlockHook>,
        step_complete: Arc<AtomicBool>,
        tool_result_budget: ToolResultBudget,
    ) -> Self {
        Self {
            session,
            assistant_index,
            update_hook,
            event_broadcast,
            output_block_hook,
            last_emit: Instant::now() - Duration::from_millis(STREAM_UPDATE_INTERVAL_MS),
            tool_calls: HashMap::new(),
            stream_tool_results: Vec::new(),
            finish_reason: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_miss_tokens: 0,
            cache_write_tokens: 0,
            executed_local_tools_this_step: false,
            stream_termination: None,
            step_complete,
            assistant_output_started: false,
            reasoning_output_started: false,
            tool_result_budget,
        }
    }

    pub(super) fn into_output(self) -> SessionStepRuntimeOutput {
        SessionStepRuntimeOutput {
            stream_tool_results: self.stream_tool_results,
            finish_reason: self.finish_reason,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            reasoning_tokens: self.reasoning_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_miss_tokens: self.cache_miss_tokens,
            cache_write_tokens: self.cache_write_tokens,
            executed_local_tools_this_step: self.executed_local_tools_this_step,
            stream_termination: self.stream_termination,
        }
    }

    fn assistant_message_id(&self) -> Option<String> {
        self.session
            .messages
            .get(self.assistant_index)
            .map(|message| message.id.clone())
    }

    fn live_identity_for_block(
        &self,
        block: &OutputBlock,
        id: Option<&str>,
    ) -> Option<rocode_types::LiveMessagePartIdentity> {
        match block {
            OutputBlock::Message(message) if message.role == OutputMessageRole::Assistant => {
                let message_id = id?;
                let phase = match message.phase {
                    rocode_content::output_blocks::MessagePhase::Start => {
                        rocode_types::LivePartPhase::Start
                    }
                    rocode_content::output_blocks::MessagePhase::Delta => {
                        rocode_types::LivePartPhase::Append
                    }
                    rocode_content::output_blocks::MessagePhase::Full => {
                        rocode_types::LivePartPhase::Snapshot
                    }
                    rocode_content::output_blocks::MessagePhase::End => {
                        rocode_types::LivePartPhase::End
                    }
                };
                Some(assistant_text_live_identity(
                    message_id,
                    Some(message_id.to_string()),
                    phase,
                ))
            }
            OutputBlock::Reasoning(reasoning) => {
                let message_id = id?;
                let phase = match reasoning.phase {
                    rocode_content::output_blocks::MessagePhase::Start => {
                        rocode_types::LivePartPhase::Start
                    }
                    rocode_content::output_blocks::MessagePhase::Delta => {
                        rocode_types::LivePartPhase::Append
                    }
                    rocode_content::output_blocks::MessagePhase::Full => {
                        rocode_types::LivePartPhase::Snapshot
                    }
                    rocode_content::output_blocks::MessagePhase::End => {
                        rocode_types::LivePartPhase::End
                    }
                };
                Some(assistant_reasoning_live_identity(
                    message_id,
                    Some(message_id.to_string()),
                    phase,
                ))
            }
            OutputBlock::Tool(tool) => {
                let tool_call_id = id?;
                let message_id = self.assistant_message_id()?;
                let phase = match tool.phase {
                    rocode_content::output_blocks::ToolPhase::Start => {
                        rocode_types::LivePartPhase::Start
                    }
                    rocode_content::output_blocks::ToolPhase::Running => {
                        rocode_types::LivePartPhase::Append
                    }
                    rocode_content::output_blocks::ToolPhase::Done
                    | rocode_content::output_blocks::ToolPhase::Error => {
                        rocode_types::LivePartPhase::End
                    }
                };
                let identity = match tool.phase {
                    rocode_content::output_blocks::ToolPhase::Done
                    | rocode_content::output_blocks::ToolPhase::Error => {
                        tool_result_live_identity(&message_id, tool_call_id, phase)
                    }
                    _ => tool_call_live_identity(&message_id, tool_call_id, phase),
                };
                Some(identity)
            }
            _ => None,
        }
    }

    async fn emit_output_block(&self, block: OutputBlock, id: Option<String>) {
        if let Some(output_block_hook) = self.output_block_hook {
            let live_identity = self.live_identity_for_block(&block, id.as_deref());
            output_block_hook(OutputBlockEvent {
                session_id: self.session.id.clone(),
                block,
                id,
                live_identity,
            })
            .await;
        }
    }

    async fn ensure_assistant_output_started(&mut self) {
        if self.assistant_output_started {
            return;
        }
        self.emit_output_block(
            OutputBlock::Message(MessageBlock::start(OutputMessageRole::Assistant)),
            self.assistant_message_id(),
        )
        .await;
        self.assistant_output_started = true;
    }

    async fn ensure_reasoning_output_started(&mut self) {
        self.ensure_assistant_output_started().await;
        if self.reasoning_output_started {
            return;
        }
        self.emit_output_block(
            OutputBlock::Reasoning(ReasoningBlock::start()),
            self.assistant_message_id(),
        )
        .await;
        self.reasoning_output_started = true;
    }

    async fn finish_output_blocks(&mut self) {
        let assistant_message_id = self.assistant_message_id();
        if self.reasoning_output_started {
            self.emit_output_block(
                OutputBlock::Reasoning(ReasoningBlock::end()),
                assistant_message_id.clone(),
            )
            .await;
            self.reasoning_output_started = false;
        }
        if self.assistant_output_started {
            self.emit_output_block(
                OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
                assistant_message_id,
            )
            .await;
            self.assistant_output_started = false;
        }
    }
}

#[async_trait]
impl<'a> LoopSink for SessionStepSink<'a> {
    async fn on_event(&mut self, event: &LoopEvent) -> std::result::Result<(), RuntimeLoopError> {
        match event {
            LoopEvent::TextChunk(text) => {
                self.ensure_assistant_output_started().await;
                if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
                    SessionPrompt::append_delta_part(assistant, false, text);
                }
                self.emit_output_block(
                    OutputBlock::Message(MessageBlock::delta(
                        OutputMessageRole::Assistant,
                        text.clone(),
                    )),
                    self.assistant_message_id(),
                )
                .await;
                self.session.touch();
                SessionPrompt::maybe_emit_session_update(
                    self.update_hook,
                    self.session,
                    &mut self.last_emit,
                    false,
                );
            }
            LoopEvent::ReasoningChunk { text, .. } => {
                self.ensure_reasoning_output_started().await;
                if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
                    SessionPrompt::append_delta_part(assistant, true, text);
                }
                self.emit_output_block(
                    OutputBlock::Reasoning(ReasoningBlock::delta(text.clone())),
                    self.assistant_message_id(),
                )
                .await;
                self.session.touch();
                SessionPrompt::maybe_emit_session_update(
                    self.update_hook,
                    self.session,
                    &mut self.last_emit,
                    false,
                );
            }
            LoopEvent::ToolCallProgress {
                id,
                name,
                partial_input,
            } => {
                if let Some(next_name) = name {
                    if next_name.trim().is_empty() {
                        return Ok(());
                    }

                    if let Some(broadcast) = &self.event_broadcast {
                        let event = serde_json::json!({
                            "type": "tool_call.lifecycle",
                            "sessionID": self.session.id,
                            "toolCallId": id,
                            "phase": "start",
                            "toolName": next_name,
                        });
                        broadcast(event);
                    }

                    let (tool_input, tool_raw, tool_state, should_emit_start) = {
                        let entry =
                            self.tool_calls
                                .entry(id.clone())
                                .or_insert_with(|| StreamToolState {
                                    name: String::new(),
                                    raw_input: String::new(),
                                    input: serde_json::json!({}),
                                    status: crate::ToolCallStatus::Pending,
                                    state: crate::ToolState::Pending {
                                        input: serde_json::json!({}),
                                        raw: String::new(),
                                    },
                                    emitted_output_start: false,
                                    emitted_output_detail: None,
                                });
                        if entry.name.is_empty() {
                            entry.name = next_name.clone();
                        }
                        let should_emit_start = !entry.emitted_output_start;
                        if should_emit_start {
                            entry.emitted_output_start = true;
                        }
                        entry.status = crate::ToolCallStatus::Pending;
                        entry.state = crate::ToolState::Pending {
                            input: entry.input.clone(),
                            raw: entry.raw_input.clone(),
                        };
                        (
                            entry.input.clone(),
                            entry.raw_input.clone(),
                            entry.state.clone(),
                            should_emit_start,
                        )
                    };
                    if should_emit_start {
                        self.ensure_assistant_output_started().await;
                        self.emit_output_block(
                            OutputBlock::Tool(ToolBlock::start(next_name.clone())),
                            Some(id.clone()),
                        )
                        .await;
                    }
                    if let Some(assistant) =
                        self.session.messages_mut().get_mut(self.assistant_index)
                    {
                        SessionPrompt::upsert_tool_call_part(
                            assistant,
                            id,
                            Some(next_name),
                            Some(tool_input),
                            Some(tool_raw),
                            Some(crate::ToolCallStatus::Pending),
                            Some(tool_state),
                        );
                    }
                }
                if !partial_input.is_empty() {
                    let (tool_input, tool_raw, tool_state, tool_name, detail) = {
                        let entry =
                            self.tool_calls
                                .entry(id.clone())
                                .or_insert_with(|| StreamToolState {
                                    name: String::new(),
                                    raw_input: String::new(),
                                    input: serde_json::json!({}),
                                    status: crate::ToolCallStatus::Pending,
                                    state: crate::ToolState::Pending {
                                        input: serde_json::json!({}),
                                        raw: String::new(),
                                    },
                                    emitted_output_start: false,
                                    emitted_output_detail: None,
                                });
                        entry.raw_input.push_str(partial_input);
                        if rocode_provider::is_parsable_json(&entry.raw_input) {
                            if let Ok(parsed) = serde_json::from_str(&entry.raw_input) {
                                entry.input = parsed;
                            }
                        }
                        entry.state = crate::ToolState::Pending {
                            input: entry.input.clone(),
                            raw: entry.raw_input.clone(),
                        };
                        let detail = tool_progress_detail(
                            &entry.input,
                            Some(entry.raw_input.as_str()),
                            &crate::ToolCallStatus::Pending,
                        );
                        let tool_name = if entry.name.trim().is_empty() {
                            id.clone()
                        } else {
                            entry.name.clone()
                        };
                        let should_emit_detail = detail.as_ref().is_some_and(|detail| {
                            entry.emitted_output_detail.as_ref() != Some(detail)
                        });
                        if should_emit_detail {
                            entry.emitted_output_detail = detail.clone();
                        }
                        (
                            entry.input.clone(),
                            entry.raw_input.clone(),
                            entry.state.clone(),
                            tool_name,
                            if should_emit_detail { detail } else { None },
                        )
                    };
                    if let Some(detail) = detail {
                        self.ensure_assistant_output_started().await;
                        self.emit_output_block(
                            OutputBlock::Tool(ToolBlock::running(tool_name, detail)),
                            Some(id.clone()),
                        )
                        .await;
                    }
                    if let Some(assistant) =
                        self.session.messages_mut().get_mut(self.assistant_index)
                    {
                        SessionPrompt::upsert_tool_call_part(
                            assistant,
                            id,
                            None,
                            Some(tool_input),
                            Some(tool_raw),
                            Some(crate::ToolCallStatus::Pending),
                            Some(tool_state),
                        );
                    }
                }
            }
            LoopEvent::ToolCallReady(call) => {
                if call.name.trim().is_empty() {
                    return Ok(());
                }

                if let Some(broadcast) = &self.event_broadcast {
                    let event = serde_json::json!({
                        "type": "tool_call.lifecycle",
                        "sessionID": self.session.id,
                        "toolCallId": &call.id,
                        "phase": "complete",
                        "toolName": &call.name,
                    });
                    broadcast(event);
                }

                let (tool_input, tool_raw, tool_state, should_emit_start, detail) = {
                    let entry =
                        self.tool_calls
                            .entry(call.id.clone())
                            .or_insert_with(|| StreamToolState {
                                name: String::new(),
                                raw_input: String::new(),
                                input: serde_json::json!({}),
                                status: crate::ToolCallStatus::Pending,
                                state: crate::ToolState::Pending {
                                    input: serde_json::json!({}),
                                    raw: String::new(),
                                },
                                emitted_output_start: false,
                                emitted_output_detail: None,
                            });
                    entry.name = call.name.clone();
                    entry.input = call.arguments.clone();
                    entry.raw_input = serde_json::to_string(&call.arguments).unwrap_or_default();
                    let should_emit_start = !entry.emitted_output_start;
                    if should_emit_start {
                        entry.emitted_output_start = true;
                    }
                    let detail = tool_progress_detail(
                        &entry.input,
                        Some(entry.raw_input.as_str()),
                        &crate::ToolCallStatus::Running,
                    );
                    let should_emit_detail = detail
                        .as_ref()
                        .is_some_and(|detail| entry.emitted_output_detail.as_ref() != Some(detail));
                    if should_emit_detail {
                        entry.emitted_output_detail = detail.clone();
                    }
                    entry.status = crate::ToolCallStatus::Running;
                    entry.state = crate::ToolState::Running {
                        input: entry.input.clone(),
                        title: None,
                        metadata: None,
                        time: crate::RunningTime {
                            start: chrono::Utc::now().timestamp_millis(),
                        },
                    };
                    (
                        entry.input.clone(),
                        entry.raw_input.clone(),
                        entry.state.clone(),
                        should_emit_start,
                        if should_emit_detail { detail } else { None },
                    )
                };
                self.ensure_assistant_output_started().await;
                if should_emit_start {
                    self.emit_output_block(
                        OutputBlock::Tool(ToolBlock::start(call.name.clone())),
                        Some(call.id.clone()),
                    )
                    .await;
                }
                if let Some(detail) = detail {
                    self.emit_output_block(
                        OutputBlock::Tool(ToolBlock::running(call.name.clone(), detail)),
                        Some(call.id.clone()),
                    )
                    .await;
                }
                if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
                    SessionPrompt::upsert_tool_call_part(
                        assistant,
                        &call.id,
                        Some(&call.name),
                        Some(tool_input),
                        Some(tool_raw),
                        Some(crate::ToolCallStatus::Running),
                        Some(tool_state),
                    );
                }
                self.session.touch();
                SessionPrompt::maybe_emit_session_update(
                    self.update_hook,
                    self.session,
                    &mut self.last_emit,
                    true,
                );
            }
            LoopEvent::StepDone {
                finish_reason,
                usage,
            } => {
                self.finish_reason = Some(match finish_reason {
                    rocode_orchestrator::runtime::events::FinishReason::ToolUse => {
                        "tool-calls".to_string()
                    }
                    rocode_orchestrator::runtime::events::FinishReason::EndTurn => {
                        "stop".to_string()
                    }
                    rocode_orchestrator::runtime::events::FinishReason::Provider(reason) => {
                        reason.clone()
                    }
                    rocode_orchestrator::runtime::events::FinishReason::MaxSteps => {
                        "max_steps".to_string()
                    }
                    rocode_orchestrator::runtime::events::FinishReason::Cancelled => {
                        "cancelled".to_string()
                    }
                    rocode_orchestrator::runtime::events::FinishReason::Error(message) => {
                        format!("error:{}", message)
                    }
                });
                if let Some(usage) = usage {
                    self.prompt_tokens = self.prompt_tokens.max(usage.prompt_tokens);
                    self.completion_tokens = self.completion_tokens.max(usage.completion_tokens);
                    self.reasoning_tokens = self.reasoning_tokens.max(usage.reasoning_tokens);
                    self.cache_read_tokens = self.cache_read_tokens.max(usage.cache_read_tokens);
                    self.cache_miss_tokens = self.cache_miss_tokens.max(usage.cache_miss_tokens);
                    self.cache_write_tokens = self.cache_write_tokens.max(usage.cache_write_tokens);
                }
                self.finish_output_blocks().await;
            }
            LoopEvent::Error(msg) => {
                self.finish_output_blocks().await;
                return Err(RuntimeLoopError::ModelError(
                    rocode_orchestrator::runtime::events::ModelFailure::Message(msg.clone()),
                ));
            }
        }
        Ok(())
    }

    async fn on_tool_result(
        &mut self,
        call: &RuntimeToolCallReady,
        result: &RuntimeToolResult,
    ) -> std::result::Result<(), RuntimeLoopError> {
        self.executed_local_tools_this_step = true;

        if let Some(entry) = self.tool_calls.get_mut(&call.id) {
            entry.input = call.arguments.clone();
            entry.name = result.tool_name.clone();
            entry.status = if result.is_error {
                crate::ToolCallStatus::Error
            } else {
                crate::ToolCallStatus::Completed
            };
            let now = chrono::Utc::now().timestamp_millis();
            entry.state = if result.is_error {
                crate::ToolState::Error {
                    input: call.arguments.clone(),
                    error: result.output.clone(),
                    metadata: None,
                    time: crate::ErrorTime {
                        start: now,
                        end: now,
                    },
                }
            } else {
                let mut metadata = result
                    .metadata
                    .clone()
                    .and_then(|value| value.as_object().cloned())
                    .map(|obj| obj.into_iter().collect::<HashMap<_, _>>())
                    .unwrap_or_default();
                let (_, state_attachments) = SessionPrompt::extract_tool_attachments_from_metadata(
                    &mut metadata,
                    &self.session.id,
                    &self
                        .session
                        .messages
                        .get(self.assistant_index)
                        .map(|m| m.id.clone())
                        .unwrap_or_default(),
                );
                crate::ToolState::Completed {
                    input: call.arguments.clone(),
                    output: result.output.clone(),
                    title: result
                        .title
                        .clone()
                        .unwrap_or_else(|| "Tool Result".to_string()),
                    metadata,
                    time: crate::CompletedTime {
                        start: now,
                        end: now,
                        compacted: None,
                    },
                    attachments: state_attachments,
                }
            };
            if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
                SessionPrompt::upsert_tool_call_part(
                    assistant,
                    &call.id,
                    Some(&result.tool_name),
                    Some(call.arguments.clone()),
                    Some(serde_json::to_string(&call.arguments).unwrap_or_default()),
                    Some(entry.status.clone()),
                    Some(entry.state.clone()),
                );
            }
        }

        let mut metadata_map = result
            .metadata
            .clone()
            .and_then(|value| value.as_object().cloned())
            .map(|obj| obj.into_iter().collect::<HashMap<_, _>>())
            .unwrap_or_default();
        let artifacts_root = default_tool_result_artifacts_root(&self.session.record().directory);
        let governed = govern_tool_result_output(
            &self.session.id,
            &call.id,
            result.output.clone(),
            &mut metadata_map,
            &artifacts_root,
            self.tool_result_budget,
        );
        let (attachments, _) = SessionPrompt::extract_tool_attachments_from_metadata(
            &mut metadata_map,
            &self.session.id,
            &self
                .session
                .messages
                .get(self.assistant_index)
                .map(|m| m.id.clone())
                .unwrap_or_default(),
        );
        self.stream_tool_results.push((
            call.id.clone(),
            governed.output.clone(),
            result.is_error,
            result.title.clone(),
            if metadata_map.is_empty() {
                None
            } else {
                Some(metadata_map)
            },
            attachments,
        ));

        let detail = tool_result_detail(result.title.as_deref(), &governed.output);
        let block = if result.is_error {
            OutputBlock::Tool(ToolBlock::error(
                result.tool_name.clone(),
                detail.unwrap_or_else(|| governed.output.clone()),
            ))
        } else {
            OutputBlock::Tool(ToolBlock::done(result.tool_name.clone(), detail))
        };
        self.emit_output_block(block, Some(call.id.clone())).await;

        self.session.touch();
        SessionPrompt::maybe_emit_session_update(
            self.update_hook,
            self.session,
            &mut self.last_emit,
            true,
        );
        Ok(())
    }

    async fn on_step_boundary(
        &mut self,
        ctx: &StepBoundary,
    ) -> std::result::Result<(), RuntimeLoopError> {
        if let StepBoundary::End { .. } = ctx {
            self.step_complete.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}
