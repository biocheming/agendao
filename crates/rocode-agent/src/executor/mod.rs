#[cfg(test)]
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{AgentInfo, Conversation};
use rocode_orchestrator::runtime::bridges::{ModelCallerBridge, ToolDispatcherBridge};
use rocode_orchestrator::runtime::events::{
    CancelToken as RuntimeCancelToken, FinishReason as RuntimeFinishReason, NeverCancel,
};
use rocode_orchestrator::runtime::policy::{LoopPolicy, ToolDedupScope};
use rocode_orchestrator::runtime::run_loop;
use rocode_orchestrator::{
    ExecutionContext, ModelRef as OrchestratorModelRef, SkillTreeRequestPlan,
    ToolExecutor as OrchestratorToolExecutor, ToolRunner,
};
use rocode_provider::ProviderRegistry;
#[cfg(test)]
use rocode_provider::StreamEvent;
#[cfg(test)]
use rocode_tool::ToolError;
use rocode_tool::ToolRegistry;
use tokio_util::sync::CancellationToken;

mod executor_adapters;
mod executor_helpers;
mod executor_sink;

use executor_adapters::*;
use executor_helpers::*;
use executor_sink::*;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Tool error: {0}")]
    ToolError(String),

    #[error("Max steps exceeded")]
    MaxStepsExceeded,

    #[error("Cancelled")]
    Cancelled,

    #[error("No provider available")]
    NoProvider,

    #[error("Invalid response")]
    InvalidResponse,
}

#[derive(Debug, Clone)]
pub struct AgentToolOutput {
    pub output: String,
    pub title: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum AgentRenderEvent {
    AssistantStart,
    AssistantDelta(String),
    AssistantEnd,
    ToolStart {
        id: String,
        name: String,
    },
    ToolProgress {
        id: String,
        name: String,
        input: String,
    },
    ToolEnd {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        output: AgentToolOutput,
    },
    ToolError {
        tool_call_id: String,
        tool_name: String,
        error: String,
        metadata: HashMap<String, serde_json::Value>,
    },
    ReasoningStart,
    ReasoningDelta(String),
    ReasoningEnd,
}

#[derive(Debug, Clone, Default)]
pub struct AgentRenderOutcome {
    pub events: Vec<AgentRenderEvent>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub stream_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentExecutionDiagnostics {
    pub session_id: String,
    pub total_steps: u32,
    pub total_tool_calls: u32,
    pub finish_reason: String,
    pub rendered_events: usize,
    pub tool_results: u32,
    pub tool_errors: u32,
    pub stream_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentExecutionReport {
    pub outcome: AgentRenderOutcome,
    pub diagnostics: AgentExecutionDiagnostics,
}

pub struct AgentExecutor {
    agent: AgentInfo,
    conversation: Conversation,
    providers: Arc<ProviderRegistry>,
    tools: Arc<ToolRegistry>,
    disabled_tools: HashSet<String>,
    subsessions: Arc<Mutex<HashMap<String, SubsessionState>>>,
    max_steps: u32,
    agent_registry: Arc<crate::AgentRegistry>,
    tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    request_skill_tree_plan: Option<SkillTreeRequestPlan>,
    question_callback: Option<rocode_tool::QuestionCallback>,
    ask_callback: Option<rocode_tool::AskCallback>,
}

#[derive(Debug, Clone)]
struct SubsessionState {
    agent: AgentInfo,
    conversation: Conversation,
    disabled_tools: HashSet<String>,
}

#[derive(Clone)]
struct AgentSubsessionCancelToken {
    token: CancellationToken,
}

impl RuntimeCancelToken for AgentSubsessionCancelToken {
    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedSubsessionState {
    pub agent: AgentInfo,
    pub conversation: Conversation,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

impl AgentExecutor {
    pub fn new(
        agent: AgentInfo,
        providers: Arc<ProviderRegistry>,
        tools: Arc<ToolRegistry>,
        agent_registry: Arc<crate::AgentRegistry>,
    ) -> Self {
        let max_steps = agent.max_steps.unwrap_or(100);
        let conversation = Conversation::new();

        Self {
            agent,
            conversation,
            providers,
            tools,
            disabled_tools: HashSet::new(),
            subsessions: Arc::new(Mutex::new(HashMap::new())),
            max_steps,
            agent_registry,
            tool_runtime_config: rocode_tool::ToolRuntimeConfig::default(),
            request_skill_tree_plan: None,
            question_callback: None,
            ask_callback: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.conversation = Conversation::with_system_prompt(prompt);
        self
    }

    pub fn with_request_skill_tree_plan(mut self, plan: SkillTreeRequestPlan) -> Self {
        self.request_skill_tree_plan = Some(plan);
        self
    }

    pub fn with_tool_runtime_config(
        mut self,
        tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    ) -> Self {
        self.tool_runtime_config = tool_runtime_config;
        self
    }

    pub fn with_disabled_tools<I>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        self.disabled_tools = tools.into_iter().collect();
        self
    }

    pub fn with_persisted_subsessions(
        mut self,
        states: HashMap<String, PersistedSubsessionState>,
    ) -> Self {
        let subsessions = states
            .into_iter()
            .map(|(id, state)| {
                (
                    id,
                    SubsessionState {
                        agent: state.agent,
                        conversation: state.conversation,
                        disabled_tools: state.disabled_tools.into_iter().collect(),
                    },
                )
            })
            .collect();
        self.subsessions = Arc::new(Mutex::new(subsessions));
        self
    }

    /// Set a callback for interactive question prompts.
    pub fn with_ask_question<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(Vec<rocode_tool::QuestionDef>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<Vec<String>>, rocode_tool::ToolError>>
            + Send
            + 'static,
    {
        self.question_callback = Some(Arc::new(move |questions| Box::pin(callback(questions))));
        self
    }

    /// Set a callback for interactive permission approval prompts.
    pub fn with_ask_permission<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(rocode_tool::PermissionRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), rocode_tool::ToolError>> + Send + 'static,
    {
        self.ask_callback = Some(Arc::new(move |request| Box::pin(callback(request))));
        self
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    pub fn conversation_mut(&mut self) -> &mut Conversation {
        &mut self.conversation
    }

    fn build_orchestrator_exec_context(&self) -> ExecutionContext {
        ExecutionContext {
            session_id: "default".to_string(),
            workdir: std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            agent_name: self.agent.name.clone(),
            metadata: HashMap::new(),
        }
    }

    fn build_tooling(&self) -> (Arc<dyn OrchestratorToolExecutor>, ToolRunner) {
        let executor: Arc<dyn OrchestratorToolExecutor> = Arc::new(ToolRegistryAdapter::new(
            self.agent.clone(),
            ToolRegistryAdapterDeps {
                tools: self.tools.clone(),
                disabled_tools: self.disabled_tools.clone(),
                providers: self.providers.clone(),
                subsessions: self.subsessions.clone(),
                agent_registry: self.agent_registry.clone(),
                tool_runtime_config: self.tool_runtime_config.clone(),
                question_callback: self.question_callback.clone(),
                ask_callback: self.ask_callback.clone(),
            },
        ));
        let runner = ToolRunner::new(executor.clone());
        (executor, runner)
    }

    fn append_runtime_messages(&mut self, messages: Vec<rocode_provider::Message>) {
        let mut tool_name_by_id = collect_tool_names(&self.conversation);
        for message in &messages {
            append_provider_message(&mut self.conversation, message, &mut tool_name_by_id);
        }
    }

    fn apply_request_skill_tree_plan(
        &self,
        messages: Vec<rocode_provider::Message>,
    ) -> Vec<rocode_provider::Message> {
        match &self.request_skill_tree_plan {
            Some(plan) => plan.apply_to_messages(messages),
            None => messages,
        }
    }

    pub async fn export_subsessions(&self) -> HashMap<String, PersistedSubsessionState> {
        self.subsessions
            .lock()
            .await
            .iter()
            .map(|(id, state)| {
                (
                    id.clone(),
                    PersistedSubsessionState {
                        agent: state.agent.clone(),
                        conversation: state.conversation.clone(),
                        disabled_tools: state.disabled_tools.iter().cloned().collect(),
                    },
                )
            })
            .collect()
    }

    async fn execute_subsession_with_cancel_token(
        &mut self,
        user_message: impl Into<String>,
        cancel_token: CancellationToken,
    ) -> Result<String, AgentError> {
        self.conversation.add_user_message(user_message);
        let base_messages =
            self.apply_request_skill_tree_plan(self.conversation.to_provider_messages());

        let (tool_executor, tool_runner) = self.build_tooling();
        let execution = agent_execution_context(&self.agent);
        let model_ref = execution.model_ref().map(|m| OrchestratorModelRef {
            provider_id: m.provider_id,
            model_id: m.model_id,
        });
        let exec_ctx = self.build_orchestrator_exec_context();

        let model = ModelCallerBridge::new(
            Arc::new(ProviderModelResolver {
                providers: self.providers.clone(),
                execution: execution.clone(),
            }),
            model_ref,
            exec_ctx.clone(),
        );
        let tools = ToolDispatcherBridge::new(tool_runner, tool_executor, exec_ctx);
        let policy = LoopPolicy {
            max_steps: Some(self.max_steps),
            tool_dedup: ToolDedupScope::None,
            ..Default::default()
        };
        let cancel = AgentSubsessionCancelToken {
            token: cancel_token,
        };
        let mut sink = AgentLoopSink::default();

        let outcome = run_loop(&model, &tools, &mut sink, &policy, &cancel, base_messages)
            .await
            .map_err(map_runtime_loop_error)?;

        self.append_runtime_messages(sink.into_messages());

        if matches!(outcome.finish_reason, RuntimeFinishReason::MaxSteps) {
            return Err(AgentError::MaxStepsExceeded);
        }
        if matches!(outcome.finish_reason, RuntimeFinishReason::Cancelled) {
            return Err(AgentError::ProviderError(
                "subagent execution cancelled".to_string(),
            ));
        }

        Ok(outcome.content)
    }

    async fn execute_streaming_impl(
        &mut self,
        user_message: String,
        cancel_token: Option<CancellationToken>,
    ) -> Result<AgentExecutionReport, AgentError> {
        self.conversation.add_user_message(user_message);
        let base_messages =
            self.apply_request_skill_tree_plan(self.conversation.to_provider_messages());

        let (tool_executor, tool_runner) = self.build_tooling();
        let execution = agent_execution_context(&self.agent);
        let model_ref = execution.model_ref().map(|m| OrchestratorModelRef {
            provider_id: m.provider_id,
            model_id: m.model_id,
        });
        let exec_ctx = self.build_orchestrator_exec_context();
        let session_id = exec_ctx.session_id.clone();

        let model = ModelCallerBridge::new(
            Arc::new(ProviderModelResolver {
                providers: self.providers.clone(),
                execution: execution.clone(),
            }),
            model_ref,
            exec_ctx.clone(),
        );
        let tools = ToolDispatcherBridge::new(tool_runner, tool_executor, exec_ctx);
        let policy = LoopPolicy {
            max_steps: Some(self.max_steps),
            tool_dedup: ToolDedupScope::None,
            ..Default::default()
        };
        let cancel_handle = cancel_token.map(|token| AgentSubsessionCancelToken { token });
        let never_cancel = NeverCancel;
        let cancel: &dyn RuntimeCancelToken = match cancel_handle.as_ref() {
            Some(token) => token,
            None => &never_cancel,
        };
        let mut sink = AgentStreamingLoopSink::default();

        let outcome = run_loop(&model, &tools, &mut sink, &policy, cancel, base_messages)
            .await
            .map_err(map_runtime_loop_error)?;
        let (messages, rendered, sink_diag) = sink.into_output();
        self.append_runtime_messages(messages);

        if matches!(outcome.finish_reason, RuntimeFinishReason::MaxSteps) {
            return Err(AgentError::MaxStepsExceeded);
        }
        if matches!(outcome.finish_reason, RuntimeFinishReason::Cancelled) {
            return Err(AgentError::Cancelled);
        }

        let diagnostics = AgentExecutionDiagnostics {
            session_id,
            total_steps: outcome.total_steps,
            total_tool_calls: outcome.total_tool_calls,
            finish_reason: finish_reason_to_string(&outcome.finish_reason),
            rendered_events: rendered.events.len(),
            tool_results: sink_diag.tool_results,
            tool_errors: sink_diag.tool_errors,
            stream_error: rendered.stream_error.clone(),
        };

        Ok(AgentExecutionReport {
            outcome: rendered,
            diagnostics,
        })
    }

    pub async fn execute_reported(
        &mut self,
        user_message: String,
    ) -> Result<AgentExecutionReport, AgentError> {
        self.execute_streaming_impl(user_message, None).await
    }

    pub async fn execute_reported_with_cancel_token(
        &mut self,
        user_message: String,
        cancel_token: CancellationToken,
    ) -> Result<AgentExecutionReport, AgentError> {
        self.execute_streaming_impl(user_message, Some(cancel_token))
            .await
    }

    pub async fn execute_rendered(
        &mut self,
        user_message: String,
    ) -> Result<AgentRenderOutcome, AgentError> {
        let report = self.execute_streaming_impl(user_message, None).await?;
        Ok(report.outcome)
    }

    pub async fn execute_rendered_with_cancel_token(
        &mut self,
        user_message: String,
        cancel_token: CancellationToken,
    ) -> Result<AgentRenderOutcome, AgentError> {
        let report = self
            .execute_streaming_impl(user_message, Some(cancel_token))
            .await?;
        Ok(report.outcome)
    }

    pub async fn execute_text_response(
        &mut self,
        user_message: String,
    ) -> Result<String, AgentError> {
        let report = self.execute_streaming_impl(user_message, None).await?;
        if let Some(stream_error) = &report.outcome.stream_error {
            return Err(AgentError::ProviderError(stream_error.clone()));
        }
        Ok(Self::text_response_from_outcome(&report.outcome))
    }

    pub async fn execute_text_response_with_cancel_token(
        &mut self,
        user_message: String,
        cancel_token: CancellationToken,
    ) -> Result<String, AgentError> {
        let report = self
            .execute_streaming_impl(user_message, Some(cancel_token))
            .await?;
        if let Some(stream_error) = &report.outcome.stream_error {
            return Err(AgentError::ProviderError(stream_error.clone()));
        }
        Ok(Self::text_response_from_outcome(&report.outcome))
    }

    fn text_response_from_outcome(outcome: &AgentRenderOutcome) -> String {
        let mut response = String::new();
        for event in &outcome.events {
            if let AgentRenderEvent::AssistantDelta(delta) = event {
                response.push_str(delta);
            }
        }
        let trimmed = response.trim().to_string();
        if trimmed.is_empty() {
            "(No response generated)".to_string()
        } else {
            trimmed
        }
    }

    #[cfg(test)]
    fn ensure_tool_allowed(&self, tool_name: &str) -> Result<(), ToolError> {
        self.agent
            .ensure_tool_allowed(tool_name)
            .map_err(ToolError::PermissionDenied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;
    use rocode_orchestrator::runtime::events::{
        LoopError as RuntimeLoopError, LoopEvent, StepBoundary,
        ToolCallReady as RuntimeToolCallReady, ToolResult as RuntimeToolResult,
    };
    use rocode_orchestrator::runtime::traits::LoopSink;
    use rocode_permission::{PermissionAction, PermissionRule};

    fn build_executor(agent: AgentInfo) -> AgentExecutor {
        let registry = Arc::new(crate::AgentRegistry::default());
        AgentExecutor::new(
            agent,
            Arc::new(ProviderRegistry::new()),
            Arc::new(ToolRegistry::new()),
            registry,
        )
    }

    #[tokio::test]
    async fn persisted_subsessions_roundtrip() {
        let mut conversation = Conversation::with_system_prompt("subagent prompt");
        conversation.add_user_message("inspect project");
        conversation.add_assistant_message("working on it");

        let mut persisted = HashMap::new();
        persisted.insert(
            "task_explore_1".to_string(),
            PersistedSubsessionState {
                agent: AgentInfo::explore().with_model("gpt-4.1-mini", "openai"),
                conversation: conversation.clone(),
                disabled_tools: vec!["write".to_string(), "edit".to_string()],
            },
        );

        let executor = build_executor(AgentInfo::general()).with_persisted_subsessions(persisted);
        let exported = executor.export_subsessions().await;
        let state = exported
            .get("task_explore_1")
            .expect("expected persisted subsession");

        assert_eq!(state.agent.name, "explore");
        assert_eq!(
            state.conversation.messages.len(),
            conversation.messages.len()
        );

        let mut disabled = state.disabled_tools.clone();
        disabled.sort();
        assert_eq!(disabled, vec!["edit".to_string(), "write".to_string()]);
    }

    #[tokio::test]
    async fn execute_text_response_preserves_user_message_on_no_provider() {
        let mut executor = build_executor(AgentInfo::general());

        let result = executor
            .execute_text_response("trigger missing provider".to_string())
            .await;
        assert!(matches!(result, Err(AgentError::NoProvider)));

        let messages = &executor.conversation().messages;
        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, crate::MessageRole::User));
        assert_eq!(messages[0].content, "trigger missing provider");
    }

    #[tokio::test]
    async fn execute_text_response_maps_typed_missing_provider_to_no_provider() {
        let mut executor =
            build_executor(AgentInfo::general().with_model("gpt-4.1-mini", "missing-provider"));

        let result = executor
            .execute_text_response("trigger missing configured provider".to_string())
            .await;

        assert!(matches!(result, Err(AgentError::NoProvider)));

        let messages = &executor.conversation().messages;
        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, crate::MessageRole::User));
        assert_eq!(messages[0].content, "trigger missing configured provider");
    }

    #[test]
    fn executor_enforces_explore_allowlist() {
        let executor = build_executor(AgentInfo::explore());

        assert!(executor.ensure_tool_allowed("grep").is_ok());

        let denied = executor
            .ensure_tool_allowed("write")
            .expect_err("write should be denied for explore");
        assert!(
            matches!(denied, ToolError::PermissionDenied(_)),
            "expected permission denied, got: {denied}"
        );
    }

    #[test]
    fn executor_blocks_ask_permissions_without_user_approval() {
        let agent = AgentInfo::custom("review").with_permission(vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Ask,
        }]);
        let executor = build_executor(agent);

        let denied = executor
            .ensure_tool_allowed("bash")
            .expect_err("ask should block direct execution");
        assert!(
            matches!(denied, ToolError::PermissionDenied(_)),
            "expected permission denied, got: {denied}"
        );
    }

    #[test]
    fn repair_tool_call_name_fixes_case_when_lower_tool_exists() {
        let available = vec!["read".to_string(), "invalid".to_string()];
        let repaired = ToolRunner::repair_tool_call_name("Read", &available);
        assert_eq!(repaired.as_deref(), Some("read"));
    }

    #[test]
    fn repair_tool_call_name_falls_back_to_invalid_tool() {
        let available = vec!["read".to_string(), "invalid".to_string()];
        let repaired = ToolRunner::repair_tool_call_name("missing_tool", &available);
        assert_eq!(repaired.as_deref(), Some("invalid"));
    }

    /// Build a mock stream from a sequence of StreamEvents.
    fn mock_stream(events: Vec<StreamEvent>) -> rocode_provider::StreamResult {
        let stream = futures::stream::iter(
            events
                .into_iter()
                .map(Ok::<_, rocode_provider::ProviderError>),
        );
        Box::pin(stream)
    }

    async fn process_stream_fixture(
        mut stream: rocode_provider::StreamResult,
    ) -> Result<(String, Vec<ToolCall>), AgentError> {
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        loop {
            let Some(event) = StreamExt::next(&mut stream).await else {
                break;
            };
            match event {
                Ok(StreamEvent::TextDelta(text)) => {
                    content.push_str(&text);
                }
                Ok(StreamEvent::ToolCallEnd { id, name, input }) => {
                    if name.trim().is_empty() {
                        continue;
                    }
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
                Ok(StreamEvent::Done) => break,
                Ok(StreamEvent::Error(e)) => {
                    return Err(AgentError::ProviderError(e));
                }
                Err(e) => {
                    return Err(AgentError::ProviderError(e.to_string()));
                }
                _ => {}
            }
        }

        Ok((content, tool_calls))
    }

    #[tokio::test]
    async fn process_stream_uses_tool_call_end_as_authoritative() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallStart {
                id: "tool-call-0".into(),
                name: "read".into(),
            },
            StreamEvent::ToolCallDelta {
                id: "tool-call-0".into(),
                input: r#"{"partial":true"#.into(),
            },
            StreamEvent::ToolCallEnd {
                id: "tool-call-0".into(),
                name: "read".into(),
                input: serde_json::json!({"file_path": "/tmp/test"}),
            },
            StreamEvent::Done,
        ]);

        let (_, tool_calls) = process_stream_fixture(stream).await.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "read");
        assert_eq!(
            tool_calls[0].arguments,
            serde_json::json!({"file_path": "/tmp/test"})
        );
    }

    #[tokio::test]
    async fn process_stream_ignores_partial_tool_call_without_end() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallStart {
                id: "tool-call-0".into(),
                name: "bash".into(),
            },
            StreamEvent::ToolCallDelta {
                id: "tool-call-0".into(),
                input: r#"{"command":"ls"}"#.into(),
            },
        ]);

        let (_, tool_calls) = process_stream_fixture(stream).await.unwrap();
        assert!(tool_calls.is_empty());
    }

    #[tokio::test]
    async fn process_stream_handles_multiple_tool_call_end_events() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallEnd {
                id: "tool-call-0".into(),
                name: "read".into(),
                input: serde_json::json!({"file_path": "/tmp/a"}),
            },
            StreamEvent::ToolCallEnd {
                id: "tool-call-1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
            StreamEvent::Done,
        ]);

        let (_, tool_calls) = process_stream_fixture(stream).await.unwrap();
        assert_eq!(tool_calls.len(), 2);

        let read_tc = tool_calls.iter().find(|t| t.name == "read").unwrap();
        assert_eq!(
            read_tc.arguments,
            serde_json::json!({"file_path": "/tmp/a"})
        );

        let bash_tc = tool_calls.iter().find(|t| t.name == "bash").unwrap();
        assert_eq!(bash_tc.arguments, serde_json::json!({"command": "ls"}));
    }

    #[tokio::test]
    async fn process_stream_ignores_tool_call_end_with_empty_name() {
        let stream = mock_stream(vec![
            StreamEvent::ToolCallEnd {
                id: "tool-call-empty".into(),
                name: "   ".into(),
                input: serde_json::json!({"command": "ls"}),
            },
            StreamEvent::ToolCallEnd {
                id: "tool-call-1".into(),
                name: "ls".into(),
                input: serde_json::json!({"path": "."}),
            },
            StreamEvent::Done,
        ]);

        let (_, tool_calls) = process_stream_fixture(stream).await.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tool-call-1");
        assert_eq!(tool_calls[0].name, "ls");
        assert_eq!(tool_calls[0].arguments, serde_json::json!({"path": "."}));
    }

    #[tokio::test]
    async fn streaming_sink_maps_runtime_events_to_render_events() {
        let mut sink = AgentStreamingLoopSink::default();
        sink.on_step_boundary(&StepBoundary::Start { step: 1 })
            .await
            .unwrap();
        sink.on_event(&LoopEvent::TextChunk("Hel".to_string()))
            .await
            .unwrap();
        sink.on_event(&LoopEvent::TextChunk("lo".to_string()))
            .await
            .unwrap();
        sink.on_event(&LoopEvent::ToolCallProgress {
            id: "tool-1".to_string(),
            name: Some("read".to_string()),
            partial_input: "{\"path\":\"a\"}".to_string(),
        })
        .await
        .unwrap();

        let call = RuntimeToolCallReady {
            id: "tool-1".to_string(),
            name: "read".to_string(),
            arguments: serde_json::json!({"path":"a"}),
        };
        sink.on_event(&LoopEvent::ToolCallReady(call.clone()))
            .await
            .unwrap();

        sink.on_tool_result(
            &call,
            &RuntimeToolResult {
                tool_call_id: "tool-1".to_string(),
                tool_name: "read".to_string(),
                output: "done".to_string(),
                is_error: false,
                title: None,
                metadata: None,
            },
        )
        .await
        .unwrap();

        sink.on_event(&LoopEvent::StepDone {
            finish_reason: RuntimeFinishReason::ToolUse,
            usage: Some(rocode_orchestrator::runtime::events::StepUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                ..Default::default()
            }),
        })
        .await
        .unwrap();
        sink.on_step_boundary(&StepBoundary::End {
            step: 1,
            finish_reason: RuntimeFinishReason::ToolUse,
            tool_calls_count: 1,
            had_error: false,
            usage: None,
        })
        .await
        .unwrap();

        let (_, outcome, diagnostics) = sink.into_output();
        assert_eq!(outcome.prompt_tokens, 10);
        assert_eq!(outcome.completion_tokens, 20);
        assert!(outcome.stream_error.is_none());
        assert_eq!(diagnostics.tool_results, 1);
        assert_eq!(diagnostics.tool_errors, 0);

        assert!(matches!(
            outcome.events.as_slice(),
            [
                AgentRenderEvent::AssistantStart,
                AgentRenderEvent::AssistantDelta(_),
                AgentRenderEvent::AssistantDelta(_),
                AgentRenderEvent::AssistantEnd,
                AgentRenderEvent::ToolStart { .. },
                AgentRenderEvent::ToolProgress { .. },
                AgentRenderEvent::ToolEnd { .. },
                AgentRenderEvent::ToolResult { .. }
            ]
        ));
    }

    #[tokio::test]
    async fn streaming_sink_closes_assistant_on_stream_error() {
        let mut sink = AgentStreamingLoopSink::default();
        sink.on_step_boundary(&StepBoundary::Start { step: 1 })
            .await
            .unwrap();
        sink.on_event(&LoopEvent::TextChunk("hello".to_string()))
            .await
            .unwrap();

        let err = sink
            .on_event(&LoopEvent::Error("stream broken".to_string()))
            .await
            .expect_err("expected model error");
        assert!(matches!(err, RuntimeLoopError::ModelError(_)));

        let (_, outcome, diagnostics) = sink.into_output();
        assert_eq!(diagnostics.tool_results, 0);
        assert_eq!(diagnostics.tool_errors, 0);
        assert_eq!(outcome.stream_error.as_deref(), Some("stream broken"));
        assert!(matches!(
            outcome.events.as_slice(),
            [
                AgentRenderEvent::AssistantStart,
                AgentRenderEvent::AssistantDelta(_),
                AgentRenderEvent::AssistantEnd
            ]
        ));
    }

    #[test]
    fn text_response_from_outcome_uses_assistant_deltas() {
        let outcome = AgentRenderOutcome {
            events: vec![
                AgentRenderEvent::AssistantStart,
                AgentRenderEvent::AssistantDelta("hello ".to_string()),
                AgentRenderEvent::ToolStart {
                    id: "t1".to_string(),
                    name: "read".to_string(),
                },
                AgentRenderEvent::AssistantDelta("world".to_string()),
                AgentRenderEvent::AssistantEnd,
            ],
            ..Default::default()
        };

        assert_eq!(
            AgentExecutor::text_response_from_outcome(&outcome),
            "hello world"
        );
    }
}
