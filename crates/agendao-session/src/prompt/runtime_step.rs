use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use agendao_orchestrator::agent_loop::{
    AgentLoopObserver, AssistantTurn, ToolBackend, ToolCall, ToolExecution,
};
use agendao_orchestrator::context::Usage;
use agendao_output_blocks::{
    MessageBlock, MessageRole as OutputMessageRole, OutputBlock, ReasoningBlock, ToolBlock,
};

use crate::tool_result_governance::{
    default_tool_result_artifacts_root, govern_tool_result_output, ToolResultBudget,
};
use crate::Session;

use super::{
    assistant_reasoning_live_identity, assistant_text_live_identity, tool_call_live_identity,
    tool_progress_detail, tool_result_detail, tool_result_live_identity, AskPermissionHook,
    AskQuestionHook, EventBroadcastHook, OutputBlockEvent, OutputBlockHook, PublishBusHook,
    SessionPrompt, SessionUpdateHook, StreamToolResultEntry, StreamToolState,
    STREAM_UPDATE_INTERVAL_MS,
};

fn format_disallowed_tool_message(
    tool_name: &str,
    allowed_tools: &std::collections::HashSet<String>,
) -> String {
    if allowed_tools.contains(agendao_tool::tool_catalog::CAPABILITY_TOOL_ID) {
        format!(
            "Tool `{}` is not directly exposed in this session. Call `capability` with action `search`, optionally action `describe`, then action `call` with the exact result id.",
            tool_name
        )
    } else {
        format!("Tool `{}` is not allowed in this session", tool_name)
    }
}

pub(super) struct SessionToolBackend {
    pub(super) session_id: String,
    pub(super) directory: String,
    pub(super) agent_name: String,
    pub(super) abort_token: CancellationToken,
    pub(super) tool_registry: Arc<agendao_tool::ToolRegistry>,
    pub(super) allowed_tools: Arc<std::collections::HashSet<String>>,
    pub(super) assistant_message_id: String,
    pub(super) ask_question_hook: Option<AskQuestionHook>,
    pub(super) ask_permission_hook: Option<AskPermissionHook>,
    pub(super) publish_bus_hook: Option<PublishBusHook>,
    pub(super) tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    pub(super) config_store: Option<Arc<agendao_config::ConfigStore>>,
    pub(super) runtime_skill_instructions: Option<serde_json::Value>,
    pub(super) todo_manager: Arc<crate::TodoManager>,
    pub(super) file_time_tracker: Arc<agendao_tool::FileTimeTracker>,
}

#[async_trait]
impl ToolBackend for SessionToolBackend {
    async fn execute(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<ToolExecution, String> {
        let tool_name = call.tool.as_str();
        if !self.allowed_tools.contains(tool_name) {
            let catalog_call_allowed = self
                .allowed_tools
                .contains(agendao_tool::tool_catalog::CAPABILITY_TOOL_ID)
                && self.tool_registry.get(tool_name).await.is_some();
            if !catalog_call_allowed {
                return Ok(ToolExecution {
                    output: format_disallowed_tool_message(tool_name, &self.allowed_tools),
                    title: Some("Permission denied".to_string()),
                    metadata: None,
                    is_error: true,
                });
            }
        }

        let mut context = agendao_tool::ToolContext::new(
            self.session_id.clone(),
            self.assistant_message_id.clone(),
            self.directory.clone(),
        )
        .with_agent(self.agent_name.clone())
        .with_tool_runtime_config(self.tool_runtime_config.clone())
        .with_abort(self.abort_token.clone());
        if let Some(config_store) = self.config_store.clone() {
            context = context.with_config_store(config_store);
        }
        if let Some(runtime_skill_instructions) = self.runtime_skill_instructions.clone() {
            context.extra.insert(
                "runtime_skill_instructions".to_string(),
                runtime_skill_instructions,
            );
        }
        context.call_id = Some(call.id.clone());
        if let Some(question_hook) = self.ask_question_hook.clone() {
            let session_id = self.session_id.clone();
            context = context.with_ask_question(move |questions| {
                let question_hook = question_hook.clone();
                let session_id = session_id.clone();
                async move { question_hook(session_id, questions).await }
            });
        }
        if let Some(permission_hook) = self.ask_permission_hook.clone() {
            let session_id = self.session_id.clone();
            context = context.with_ask(move |request| {
                let permission_hook = permission_hook.clone();
                let session_id = session_id.clone();
                async move { permission_hook(session_id, request).await }
            });
        }
        context = context.with_registry(self.tool_registry.clone());
        if let Some(hook) = self.publish_bus_hook.clone() {
            context = context.with_publish_bus(move |event_type, properties| {
                let hook = hook.clone();
                async move { hook(event_type, properties).await }
            });
        }
        {
            let todo_manager = self.todo_manager.clone();
            context = context.with_todo_update(move |session_id, todos| {
                let todo_manager = todo_manager.clone();
                async move {
                    todo_manager
                        .update(
                            &session_id,
                            todos
                                .into_iter()
                                .map(|todo| crate::TodoInfo {
                                    content: todo.content,
                                    status: todo.status,
                                    priority: todo.priority,
                                })
                                .collect(),
                        )
                        .await;
                    Ok(())
                }
            });
        }
        {
            let todo_manager = self.todo_manager.clone();
            context = context.with_todo_get(move |session_id| {
                let todo_manager = todo_manager.clone();
                async move {
                    Ok(todo_manager
                        .get(&session_id)
                        .await
                        .into_iter()
                        .map(|todo| agendao_tool::TodoItemData {
                            content: todo.content,
                            status: todo.status,
                            priority: todo.priority,
                        })
                        .collect())
                }
            });
        }
        {
            let tracker = self.file_time_tracker.clone();
            context = context.with_file_time_read(move |session_id, file_path| {
                let tracker = tracker.clone();
                async move { tracker.record(&session_id, &file_path) }
            });
        }
        {
            let tracker = self.file_time_tracker.clone();
            context = context.with_file_time_assert(move |session_id, file_path| {
                let tracker = tracker.clone();
                async move { tracker.assert_unchanged(&session_id, &file_path) }
            });
        }

        match self
            .tool_registry
            .execute(tool_name, call.arguments.clone(), context)
            .await
        {
            Ok(result) => Ok(ToolExecution {
                output: result.output,
                title: (!result.title.is_empty()).then_some(result.title),
                metadata: (!result.metadata.is_empty())
                    .then(|| serde_json::to_value(result.metadata).unwrap_or_default()),
                is_error: false,
            }),
            Err(error) => Ok(ToolExecution {
                output: error.to_string(),
                title: Some("Tool error".to_string()),
                metadata: None,
                is_error: true,
            }),
        }
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
    pub(super) stream_termination: Option<agendao_provider::StreamTermination>,
}

pub(super) struct SessionAgentObserver<'a> {
    state: Mutex<SessionAgentState<'a>>,
}

struct SessionAgentState<'a> {
    session: &'a mut Session,
    assistant_index: usize,
    update_hook: Option<&'a SessionUpdateHook>,
    event_broadcast: Option<&'a EventBroadcastHook>,
    output_block_hook: Option<&'a OutputBlockHook>,
    last_emit: Instant,
    tool_calls: HashMap<String, StreamToolState>,
    stream_tool_results: Vec<StreamToolResultEntry>,
    finish_reason: Option<String>,
    usage: Usage,
    executed_local_tools: bool,
    assistant_output_started: bool,
    reasoning_output_started: bool,
    tool_result_budget: ToolResultBudget,
}

impl<'a> SessionAgentObserver<'a> {
    pub(super) fn new(
        session: &'a mut Session,
        assistant_index: usize,
        update_hook: Option<&'a SessionUpdateHook>,
        event_broadcast: Option<&'a EventBroadcastHook>,
        output_block_hook: Option<&'a OutputBlockHook>,
        tool_result_budget: ToolResultBudget,
    ) -> Self {
        Self {
            state: Mutex::new(SessionAgentState {
                session,
                assistant_index,
                update_hook,
                event_broadcast,
                output_block_hook,
                last_emit: Instant::now() - Duration::from_millis(STREAM_UPDATE_INTERVAL_MS),
                tool_calls: HashMap::new(),
                stream_tool_results: Vec::new(),
                finish_reason: None,
                usage: Usage::default(),
                executed_local_tools: false,
                assistant_output_started: false,
                reasoning_output_started: false,
                tool_result_budget,
            }),
        }
    }

    pub(super) fn into_output(self) -> SessionStepRuntimeOutput {
        let state = self.state.into_inner();
        SessionStepRuntimeOutput {
            stream_tool_results: state.stream_tool_results,
            finish_reason: state.finish_reason,
            prompt_tokens: state.usage.input_tokens,
            completion_tokens: state.usage.output_tokens,
            reasoning_tokens: state.usage.reasoning_tokens,
            cache_read_tokens: state.usage.cache_read_tokens,
            cache_miss_tokens: state.usage.cache_miss_tokens,
            cache_write_tokens: state.usage.cache_write_tokens,
            stream_termination: Some(agendao_provider::StreamTermination::Completed),
        }
    }
}

impl SessionAgentState<'_> {
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
    ) -> Option<agendao_types::LiveMessagePartIdentity> {
        match block {
            OutputBlock::Message(message) if message.role == OutputMessageRole::Assistant => {
                let message_id = id?;
                Some(assistant_text_live_identity(
                    message_id,
                    message_phase(message.phase),
                ))
            }
            OutputBlock::Reasoning(reasoning) => {
                let message_id = id?;
                Some(assistant_reasoning_live_identity(
                    message_id,
                    message_phase(reasoning.phase),
                ))
            }
            OutputBlock::Tool(tool) => {
                let tool_call_id = id?;
                let message_id = self.assistant_message_id()?;
                match tool.phase {
                    agendao_output_blocks::ToolPhase::Start => Some(tool_call_live_identity(
                        &message_id,
                        tool_call_id,
                        agendao_types::LivePartPhase::Start,
                    )),
                    agendao_output_blocks::ToolPhase::Running => Some(tool_call_live_identity(
                        &message_id,
                        tool_call_id,
                        agendao_types::LivePartPhase::Append,
                    )),
                    agendao_output_blocks::ToolPhase::Done
                    | agendao_output_blocks::ToolPhase::Error => Some(tool_result_live_identity(
                        &message_id,
                        tool_call_id,
                        agendao_types::LivePartPhase::End,
                    )),
                }
            }
            _ => None,
        }
    }

    async fn emit_output_block(&self, block: OutputBlock, id: Option<String>) {
        if let Some(output_block_hook) = self.output_block_hook {
            output_block_hook(OutputBlockEvent {
                session_id: self.session.id.clone(),
                live_identity: self.live_identity_for_block(&block, id.as_deref()),
                block,
                id,
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
        let message_id = self.assistant_message_id();
        if self.reasoning_output_started {
            self.emit_output_block(
                OutputBlock::Reasoning(ReasoningBlock::end()),
                message_id.clone(),
            )
            .await;
            self.reasoning_output_started = false;
        }
        if self.assistant_output_started {
            let text = self
                .session
                .messages
                .get(self.assistant_index)
                .map(|message| {
                    message
                        .parts
                        .iter()
                        .filter_map(|part| match &part.part_type {
                            crate::PartType::Text { text, ignored, .. }
                                if ignored != &Some(true) =>
                            {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<String>()
                })
                .unwrap_or_default();
            if !text.is_empty() {
                self.emit_output_block(
                    OutputBlock::Message(MessageBlock::full(OutputMessageRole::Assistant, text)),
                    message_id.clone(),
                )
                .await;
            }
            self.emit_output_block(
                OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
                message_id,
            )
            .await;
            self.assistant_output_started = false;
        }
    }

    fn emit_session_update(&mut self, force: bool) {
        self.session.touch();
        SessionPrompt::maybe_emit_session_update(
            self.update_hook,
            self.session,
            &mut self.last_emit,
            force,
        );
    }

    fn tool_entry(&mut self, id: &str) -> &mut StreamToolState {
        self.tool_calls
            .entry(id.to_string())
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
            })
    }

    async fn record_tool_progress(&mut self, id: &str, name: Option<&str>, delta: &str) {
        let (tool_name, input, raw, state, emit_start, detail) = {
            let entry = self.tool_entry(id);
            if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
                entry.name = name.to_string();
            }
            entry.raw_input.push_str(delta);
            if agendao_provider::is_parsable_json(&entry.raw_input) {
                if let Ok(parsed) = serde_json::from_str(&entry.raw_input) {
                    entry.input = parsed;
                }
            }
            entry.state = crate::ToolState::Pending {
                input: entry.input.clone(),
                raw: entry.raw_input.clone(),
            };
            let emit_start = !entry.emitted_output_start && !entry.name.is_empty();
            entry.emitted_output_start |= emit_start;
            let detail = tool_progress_detail(
                &entry.input,
                Some(&entry.raw_input),
                &crate::ToolCallStatus::Pending,
            );
            let emit_detail = detail
                .as_ref()
                .is_some_and(|detail| entry.emitted_output_detail.as_ref() != Some(detail));
            if emit_detail {
                entry.emitted_output_detail = detail.clone();
            }
            (
                entry.name.clone(),
                entry.input.clone(),
                entry.raw_input.clone(),
                entry.state.clone(),
                emit_start,
                emit_detail.then_some(detail).flatten(),
            )
        };
        if emit_start {
            self.emit_output_block(
                OutputBlock::Tool(ToolBlock::start(tool_name.clone())),
                Some(id.to_string()),
            )
            .await;
        }
        if let Some(detail) = detail {
            self.emit_output_block(
                OutputBlock::Tool(ToolBlock::running(tool_name.clone(), detail)),
                Some(id.to_string()),
            )
            .await;
        }
        if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
            SessionPrompt::upsert_tool_call_part(
                assistant,
                id,
                (!tool_name.is_empty()).then_some(tool_name.as_str()),
                Some(input),
                Some(raw),
                Some(crate::ToolCallStatus::Pending),
                Some(state),
            );
        }
        self.emit_session_update(false);
    }

    async fn record_tool_started(&mut self, call: &ToolCall) {
        let name = call.tool.as_str();
        if let Some(broadcast) = self.event_broadcast {
            broadcast(serde_json::json!({
                "type": "tool_call.lifecycle",
                "sessionID": self.session.id,
                "toolCallId": call.id,
                "phase": "start",
                "toolName": name,
            }));
        }
        let raw = serde_json::to_string(&call.arguments).unwrap_or_default();
        let (state, emit_start, detail) = {
            let entry = self.tool_entry(&call.id);
            entry.name = name.to_string();
            entry.input = call.arguments.clone();
            entry.raw_input = raw.clone();
            let emit_start = !entry.emitted_output_start;
            entry.emitted_output_start = true;
            entry.status = crate::ToolCallStatus::Running;
            entry.state = crate::ToolState::Running {
                input: call.arguments.clone(),
                title: None,
                metadata: None,
                time: crate::RunningTime {
                    start: chrono::Utc::now().timestamp_millis(),
                },
            };
            let detail =
                tool_progress_detail(&call.arguments, Some(&raw), &crate::ToolCallStatus::Running);
            (entry.state.clone(), emit_start, detail)
        };
        if emit_start {
            self.emit_output_block(
                OutputBlock::Tool(ToolBlock::start(name)),
                Some(call.id.clone()),
            )
            .await;
        }
        if let Some(detail) = detail {
            self.emit_output_block(
                OutputBlock::Tool(ToolBlock::running(name, detail)),
                Some(call.id.clone()),
            )
            .await;
        }
        if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
            SessionPrompt::upsert_tool_call_part(
                assistant,
                &call.id,
                Some(name),
                Some(call.arguments.clone()),
                Some(raw),
                Some(crate::ToolCallStatus::Running),
                Some(state),
            );
        }
        self.emit_session_update(true);
    }

    async fn record_tool_finished(&mut self, call: &ToolCall, result: &ToolExecution) {
        self.executed_local_tools = true;
        let name = call.tool.as_str();
        let now = chrono::Utc::now().timestamp_millis();
        let mut state_metadata = result
            .metadata
            .clone()
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.into_iter().collect::<HashMap<_, _>>())
            .unwrap_or_default();
        let (_, state_attachments) = SessionPrompt::extract_tool_attachments_from_metadata(
            &mut state_metadata,
            &self.session.id,
            &self.assistant_message_id().unwrap_or_default(),
        );
        let status = if result.is_error {
            crate::ToolCallStatus::Error
        } else {
            crate::ToolCallStatus::Completed
        };
        let state = if result.is_error {
            crate::ToolState::Error {
                input: call.arguments.clone(),
                error: result.output.clone(),
                metadata: (!state_metadata.is_empty()).then_some(state_metadata),
                time: crate::ErrorTime {
                    start: now,
                    end: now,
                },
            }
        } else {
            crate::ToolState::Completed {
                input: call.arguments.clone(),
                output: result.output.clone(),
                title: result
                    .title
                    .clone()
                    .unwrap_or_else(|| "Tool Result".to_string()),
                metadata: state_metadata,
                time: crate::CompletedTime {
                    start: now,
                    end: now,
                    compacted: None,
                },
                attachments: state_attachments,
            }
        };
        if let Some(entry) = self.tool_calls.get_mut(&call.id) {
            entry.status = status.clone();
            entry.state = state.clone();
        }
        if let Some(assistant) = self.session.messages_mut().get_mut(self.assistant_index) {
            SessionPrompt::upsert_tool_call_part(
                assistant,
                &call.id,
                Some(name),
                Some(call.arguments.clone()),
                Some(serde_json::to_string(&call.arguments).unwrap_or_default()),
                Some(status),
                Some(state),
            );
        }

        let mut metadata = result
            .metadata
            .clone()
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.into_iter().collect::<HashMap<_, _>>())
            .unwrap_or_default();
        let governed = govern_tool_result_output(
            &self.session.id,
            &call.id,
            result.output.clone(),
            &mut metadata,
            &default_tool_result_artifacts_root(&self.session.record().directory),
            self.tool_result_budget,
        )
        .await;
        let (attachments, _) = SessionPrompt::extract_tool_attachments_from_metadata(
            &mut metadata,
            &self.session.id,
            &self.assistant_message_id().unwrap_or_default(),
        );
        self.stream_tool_results.push((
            call.id.clone(),
            governed.output.clone(),
            result.is_error,
            result.title.clone(),
            (!metadata.is_empty()).then_some(metadata),
            attachments,
        ));
        let detail = tool_result_detail(result.title.as_deref(), &governed.output);
        let block = if result.is_error {
            OutputBlock::Tool(ToolBlock::error(
                name,
                detail.unwrap_or_else(|| governed.output.clone()),
            ))
        } else {
            OutputBlock::Tool(ToolBlock::done(name, detail))
        };
        self.emit_output_block(block, Some(call.id.clone())).await;
        self.emit_session_update(true);
    }
}

fn message_phase(phase: agendao_output_blocks::MessagePhase) -> agendao_types::LivePartPhase {
    match phase {
        agendao_output_blocks::MessagePhase::Start => agendao_types::LivePartPhase::Start,
        agendao_output_blocks::MessagePhase::Delta => agendao_types::LivePartPhase::Append,
        agendao_output_blocks::MessagePhase::Full => agendao_types::LivePartPhase::Snapshot,
        agendao_output_blocks::MessagePhase::End => agendao_types::LivePartPhase::End,
    }
}

#[async_trait]
impl AgentLoopObserver for SessionAgentObserver<'_> {
    async fn assistant_turn(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        turn: &AssistantTurn,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        state.finish_reason = turn.finish_reason.clone();
        state.usage.merge(&turn.usage);
        state.finish_output_blocks().await;
        Ok(())
    }

    async fn text_delta(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        text: &str,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        state.ensure_assistant_output_started().await;
        let assistant_index = state.assistant_index;
        if let Some(assistant) = state.session.messages_mut().get_mut(assistant_index) {
            SessionPrompt::append_delta_part(assistant, false, text);
        }
        state
            .emit_output_block(
                OutputBlock::Message(MessageBlock::delta(OutputMessageRole::Assistant, text)),
                state.assistant_message_id(),
            )
            .await;
        state.emit_session_update(false);
        Ok(())
    }

    async fn reasoning_delta(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        _id: &str,
        text: &str,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        state.ensure_reasoning_output_started().await;
        let assistant_index = state.assistant_index;
        if let Some(assistant) = state.session.messages_mut().get_mut(assistant_index) {
            SessionPrompt::append_delta_part(assistant, true, text);
        }
        state
            .emit_output_block(
                OutputBlock::Reasoning(ReasoningBlock::delta(text)),
                state.assistant_message_id(),
            )
            .await;
        state.emit_session_update(false);
        Ok(())
    }

    async fn tool_input_delta(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        id: &str,
        tool: Option<&str>,
        delta: &str,
    ) -> Result<(), String> {
        self.state
            .lock()
            .await
            .record_tool_progress(id, tool, delta)
            .await;
        Ok(())
    }

    async fn tool_started(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<(), String> {
        self.state.lock().await.record_tool_started(call).await;
        Ok(())
    }

    async fn tool_finished(
        &self,
        _context: &agendao_orchestrator::agent_loop::AgentObservationContext<'_>,
        call: &ToolCall,
        result: &ToolExecution,
    ) -> Result<(), String> {
        self.state
            .lock()
            .await
            .record_tool_finished(call, result)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{format_disallowed_tool_message, SessionAgentObserver};
    use agendao_orchestrator::agent_loop::{AgentLoopObserver, AssistantTurn, ToolCall};
    use agendao_orchestrator::blueprint::AgentId;
    use agendao_orchestrator::context::Usage;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn disallowed_tool_message_points_progressive_sessions_back_to_capability_flow() {
        let allowed = std::collections::HashSet::from([
            agendao_tool::tool_catalog::CAPABILITY_TOOL_ID.to_string(),
        ]);

        let message = format_disallowed_tool_message("bash", &allowed);
        assert!(message.contains("not directly exposed"));
        assert!(message.contains("`capability`"));
        assert!(message.contains("action `call`"));
    }

    #[test]
    fn disallowed_tool_message_stays_generic_without_catalog_facade() {
        let allowed = std::collections::HashSet::from(["read".to_string(), "write".to_string()]);
        let message = format_disallowed_tool_message("bash", &allowed);
        assert_eq!(message, "Tool `bash` is not allowed in this session");
    }

    #[tokio::test]
    async fn assistant_turn_emits_final_text_snapshot_before_end() {
        let mut session = crate::Session::new("ses_1", ".");
        let assistant_index = session.messages.len();
        session.add_assistant_message().add_text("final answer");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_hook = captured.clone();
        let output_hook: super::OutputBlockHook = Arc::new(move |event| {
            let captured = captured_hook.clone();
            Box::pin(async move {
                captured.lock().await.push(event.block);
            })
        });
        let observer = SessionAgentObserver::new(
            &mut session,
            assistant_index,
            None,
            None,
            Some(&output_hook),
            crate::tool_result_governance::ToolResultBudget::default(),
        );
        observer.state.lock().await.assistant_output_started = true;
        let agent = AgentId::new("agent");

        observer
            .assistant_turn(
                &agendao_orchestrator::agent_loop::AgentObservationContext {
                    node_path: "root",
                    agent: &agent,
                },
                &AssistantTurn {
                    content: Some("final answer".to_string()),
                    reasoning: None,
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".to_string()),
                    usage: Usage::default(),
                    reasoning_continuation: None,
                },
            )
            .await
            .expect("finish assistant turn");

        let blocks = captured.lock().await;
        assert!(matches!(
            blocks.as_slice(),
            [
                agendao_output_blocks::OutputBlock::Message(full),
                agendao_output_blocks::OutputBlock::Message(end)
            ] if full.phase == agendao_output_blocks::MessagePhase::Full
                && full.text == "final answer"
                && end.phase == agendao_output_blocks::MessagePhase::End
        ));
    }

    #[tokio::test]
    async fn tool_started_broadcasts_start_lifecycle_phase() {
        let mut session = crate::Session::new("ses_1", ".");
        let assistant_index = session.messages.len();
        session.add_assistant_message();
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_hook = captured.clone();
        let event_hook: super::EventBroadcastHook = Arc::new(move |event| {
            captured_hook.lock().expect("capture event").push(event);
        });
        let observer = SessionAgentObserver::new(
            &mut session,
            assistant_index,
            None,
            Some(&event_hook),
            None,
            crate::tool_result_governance::ToolResultBudget::default(),
        );
        let agent = AgentId::new("agent");

        observer
            .tool_started(
                &agendao_orchestrator::agent_loop::AgentObservationContext {
                    node_path: "root",
                    agent: &agent,
                },
                &ToolCall {
                    id: "call-1".to_string(),
                    tool: agendao_orchestrator::blueprint::ToolId::new("bash"),
                    arguments: serde_json::json!({"command": "cargo test"}),
                },
            )
            .await
            .expect("record tool start");

        let events = captured.lock().expect("read events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "tool_call.lifecycle");
        assert_eq!(events[0]["phase"], "start");
    }
}
