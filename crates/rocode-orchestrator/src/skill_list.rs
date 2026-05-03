use crate::conversation::OrchestratorConversation;
use crate::error::OrchestratorError;
use crate::output_metadata::{
    append_continuation_target, append_output_usage, continuation_target_from_tool_metadata,
    OutputUsage,
};
use crate::runtime::bridges::{ModelCallerBridge, ToolDispatcherBridge};
use crate::runtime::events::{
    FinishReason, LoopError, LoopEvent, StepBoundary, ToolCallReady, ToolResult,
};
use crate::runtime::loop_impl::run_loop;
use crate::runtime::policy::LoopPolicy;
use crate::runtime::traits::LoopSink;
use crate::tool_runner::ToolRunner;
use crate::traits::{LifecycleHook, Orchestrator};
use crate::types::{AgentDescriptor, ExecutionContext, OrchestratorContext, OrchestratorOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::Instant;

pub struct SkillListOrchestrator {
    agent: AgentDescriptor,
    conversation: OrchestratorConversation,
    tool_runner: ToolRunner,
    loop_policy: LoopPolicy,
    stage_context: Option<(String, u32)>,
}

impl SkillListOrchestrator {
    pub fn new(agent: AgentDescriptor, tool_runner: ToolRunner) -> Self {
        let conversation = if let Some(system_prompt) = agent.system_prompt.as_deref() {
            OrchestratorConversation::with_system_prompt(system_prompt)
        } else {
            OrchestratorConversation::new()
        };
        let max_steps = agent.max_steps;
        Self {
            agent,
            conversation,
            tool_runner,
            loop_policy: LoopPolicy {
                max_steps,
                ..Default::default()
            },
            stage_context: None,
        }
    }

    pub fn load_messages(&mut self, messages: Vec<rocode_provider::Message>) {
        self.conversation.load_messages(messages);
    }

    pub fn conversation(&self) -> &OrchestratorConversation {
        &self.conversation
    }

    pub fn with_loop_policy(mut self, loop_policy: LoopPolicy) -> Self {
        self.loop_policy = loop_policy;
        self
    }

    pub fn take_conversation(self) -> OrchestratorConversation {
        self.conversation
    }

    pub fn set_stage_context(&mut self, stage_name: String, stage_index: u32) {
        self.stage_context = Some((stage_name, stage_index));
    }
}

#[async_trait]
impl Orchestrator for SkillListOrchestrator {
    async fn execute(
        &mut self,
        input: &str,
        ctx: &OrchestratorContext,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        self.conversation.add_user_message(input);

        ctx.lifecycle_hook
            .on_orchestration_start(&self.agent.name, self.loop_policy.max_steps, &ctx.exec_ctx)
            .await;

        // Build bridge adapters: orchestrator traits → runtime traits
        let model = ModelCallerBridge::new(
            ctx.model_resolver.clone(),
            self.agent.model.clone(),
            ctx.exec_ctx.clone(),
        );
        let tools = ToolDispatcherBridge::new(
            self.tool_runner.clone(),
            ctx.tool_executor.clone(),
            ctx.exec_ctx.clone(),
        );

        let policy = self.loop_policy.clone();
        let model_id = self
            .agent
            .model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());

        let mut sink = SkillListSink::new(
            ctx.lifecycle_hook.clone(),
            self.agent.name.clone(),
            model_id,
            ctx.exec_ctx.clone(),
        );
        if let Some((ref stage_name, stage_index)) = self.stage_context {
            sink = sink.with_stage_context(stage_name.clone(), stage_index);
        }

        let messages = self.conversation.messages().to_vec();
        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &policy,
            ctx.cancel_token.as_ref(),
            messages,
        )
        .await
        .map_err(|e| match e {
            LoopError::ModelError(failure) => {
                if failure.is_provider_not_found() {
                    OrchestratorError::NoProvider
                } else {
                    OrchestratorError::ModelError(failure)
                }
            }
            LoopError::ToolDispatchError { tool, error } => {
                OrchestratorError::ToolError { tool, error }
            }
            LoopError::Cancelled => OrchestratorError::Other("cancelled".to_string()),
            LoopError::SinkError(msg) => OrchestratorError::Other(msg),
            LoopError::Other(msg) => OrchestratorError::Other(msg),
        })?;

        let output_metadata = sink.output_metadata().clone();
        // Merge conversation updates from the sink
        self.conversation.extend_messages(sink.into_messages());

        ctx.lifecycle_hook
            .on_orchestration_end(&self.agent.name, outcome.total_steps, &ctx.exec_ctx)
            .await;

        match outcome.finish_reason {
            FinishReason::MaxSteps => Err(OrchestratorError::MaxStepsExceeded(
                self.loop_policy
                    .max_steps
                    .map(|max| format!(": {max}"))
                    .unwrap_or_else(|| " (unbounded policy unexpectedly exhausted)".to_string()),
            )),
            _ => Ok(OrchestratorOutput {
                content: outcome.content,
                steps: outcome.total_steps,
                tool_calls_count: outcome.total_tool_calls,
                metadata: output_metadata,
                finish_reason: outcome.finish_reason,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// SkillListSink – LoopSink for SkillListOrchestrator.
//
// Two responsibilities:
// 1. Fire lifecycle hooks (on_step_start) at step boundaries.
// 2. Reconstruct conversation messages for OrchestratorConversation so that
//    multi-turn continuity is preserved after run_loop returns.
//
// Message reconstruction strategy:
//   - on_event: accumulate text chunks and tool calls per step
//   - on_tool_result: flush assistant-with-tools message (once), add tool result
//   - on_step_boundary(End): if no tool calls, add plain assistant message
// ---------------------------------------------------------------------------

/// Minimum interval between stage content flushes.
const STAGE_CONTENT_FLUSH_INTERVAL_MS: u64 = 200;
/// Minimum buffered characters before an early flush.
const STAGE_CONTENT_FLUSH_MIN_CHARS: usize = 500;

struct StageStreamContext {
    stage_name: String,
    stage_index: u32,
    pending_delta: String,
    last_flush: Instant,
}

struct SkillListSink {
    lifecycle_hook: Arc<dyn LifecycleHook>,
    agent_name: String,
    model_id: String,
    exec_ctx: ExecutionContext,

    // New messages accumulated during run_loop (for post-loop conversation merge).
    messages: Vec<rocode_provider::Message>,

    // Per-step accumulators (reset at each StepBoundary::Start).
    step_text: String,
    step_reasoning: String,
    step_tool_calls: Vec<ToolCallReady>,
    assistant_flushed: bool,

    // Optional stage streaming context for incremental content output.
    stage_ctx: Option<StageStreamContext>,
    output_metadata: HashMap<String, serde_json::Value>,
}

impl SkillListSink {
    fn new(
        lifecycle_hook: Arc<dyn LifecycleHook>,
        agent_name: String,
        model_id: String,
        exec_ctx: ExecutionContext,
    ) -> Self {
        Self {
            lifecycle_hook,
            agent_name,
            model_id,
            exec_ctx,
            messages: Vec::new(),
            step_text: String::new(),
            step_reasoning: String::new(),
            step_tool_calls: Vec::new(),
            assistant_flushed: false,
            stage_ctx: None,
            output_metadata: HashMap::new(),
        }
    }

    fn with_stage_context(mut self, stage_name: String, stage_index: u32) -> Self {
        self.stage_ctx = Some(StageStreamContext {
            stage_name,
            stage_index,
            pending_delta: String::new(),
            last_flush: Instant::now(),
        });
        self
    }

    fn into_messages(self) -> Vec<rocode_provider::Message> {
        self.messages
    }

    fn output_metadata(&self) -> &HashMap<String, serde_json::Value> {
        &self.output_metadata
    }

    /// Flush buffered stage content delta to the lifecycle hook.
    async fn flush_stage_content(&mut self) {
        let Some(ref mut ctx) = self.stage_ctx else {
            return;
        };
        if ctx.pending_delta.is_empty() {
            return;
        }
        let delta = std::mem::take(&mut ctx.pending_delta);
        ctx.last_flush = Instant::now();
        self.lifecycle_hook
            .on_scheduler_stage_content(&ctx.stage_name, ctx.stage_index, &delta, &self.exec_ctx)
            .await;
    }

    /// Buffer a text chunk for stage streaming and flush if thresholds are met.
    async fn maybe_flush_stage_content(&mut self, text: &str) {
        let should_flush = if let Some(ref mut ctx) = self.stage_ctx {
            ctx.pending_delta.push_str(text);
            let elapsed = ctx.last_flush.elapsed();
            ctx.pending_delta.len() >= STAGE_CONTENT_FLUSH_MIN_CHARS
                || elapsed.as_millis() >= STAGE_CONTENT_FLUSH_INTERVAL_MS as u128
        } else {
            false
        };
        if should_flush {
            self.flush_stage_content().await;
        }
    }

    /// Buffer a reasoning chunk for stage streaming and call lifecycle hook.
    async fn maybe_flush_stage_reasoning(&mut self, text: &str) {
        if let Some(ref ctx) = self.stage_ctx {
            self.lifecycle_hook
                .on_scheduler_stage_reasoning(
                    &ctx.stage_name,
                    ctx.stage_index,
                    text,
                    &self.exec_ctx,
                )
                .await;
        } else {
            // Non-scheduler mode: call hook with empty stage name
            self.lifecycle_hook
                .on_scheduler_stage_reasoning("", 0, text, &self.exec_ctx)
                .await;
        }
    }

    /// Flush accumulated text + tool calls as an assistant-with-tools message.
    /// Called once before the first tool result in a step.
    fn flush_assistant_with_tools(&mut self) {
        if self.assistant_flushed {
            return;
        }
        let tool_calls: Vec<rocode_provider::ToolUse> = self
            .step_tool_calls
            .iter()
            .map(|tc| rocode_provider::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.arguments.clone(),
            })
            .collect();
        if let Some(message) = rocode_provider::Message::assistant_turn(
            Some(&self.step_reasoning),
            Some(&self.step_text),
            &tool_calls,
        ) {
            self.messages.push(message);
        }
        self.assistant_flushed = true;
    }
}

#[async_trait]
impl LoopSink for SkillListSink {
    async fn on_event(&mut self, ev: &LoopEvent) -> Result<(), LoopError> {
        match ev {
            LoopEvent::TextChunk(text) => {
                self.step_text.push_str(text);
                self.maybe_flush_stage_content(text).await;
            }
            LoopEvent::ReasoningChunk { text, .. } => {
                self.step_reasoning.push_str(text);
                self.maybe_flush_stage_reasoning(text).await;
            }
            LoopEvent::StepDone { usage, .. } => {
                if let (Some(usage), Some(stage_ctx)) = (usage.as_ref(), self.stage_ctx.as_ref()) {
                    self.lifecycle_hook
                        .on_scheduler_stage_usage(
                            &stage_ctx.stage_name,
                            stage_ctx.stage_index,
                            usage,
                            false,
                            &self.exec_ctx,
                        )
                        .await;
                }
            }
            LoopEvent::ToolCallReady(tc) => {
                self.lifecycle_hook
                    .on_tool_start(
                        &self.agent_name,
                        &tc.id,
                        &tc.name,
                        &tc.arguments,
                        &self.exec_ctx,
                    )
                    .await;
                self.step_tool_calls.push(tc.clone());
            }
            _ => {}
        }
        Ok(())
    }

    async fn on_tool_result(
        &mut self,
        call: &ToolCallReady,
        result: &ToolResult,
    ) -> Result<(), LoopError> {
        // Flush assistant message before the first tool result in this step.
        self.flush_assistant_with_tools();

        // Add tool result message.
        self.messages
            .push(rocode_provider::Message::tool_parts(vec![
                rocode_provider::ContentPart::tool_result(
                    result.tool_call_id.clone(),
                    result.output.clone(),
                    Some(result.is_error),
                ),
            ]));
        if let Some(target) =
            continuation_target_from_tool_metadata(&result.tool_name, result.metadata.as_ref())
        {
            append_continuation_target(&mut self.output_metadata, target);
        }
        let tool_output = crate::ToolOutput {
            output: result.output.clone(),
            is_error: result.is_error,
            title: result.title.clone(),
            metadata: result.metadata.clone(),
        };
        self.lifecycle_hook
            .on_tool_end(
                &self.agent_name,
                &call.id,
                &result.tool_name,
                &tool_output,
                &self.exec_ctx,
            )
            .await;
        Ok(())
    }

    async fn on_step_boundary(&mut self, ctx: &StepBoundary) -> Result<(), LoopError> {
        match ctx {
            StepBoundary::Start { step } => {
                self.lifecycle_hook
                    .on_step_start(&self.agent_name, &self.model_id, *step, &self.exec_ctx)
                    .await;
                // Reset per-step state for the new step.
                self.step_text.clear();
                self.step_reasoning.clear();
                self.step_tool_calls.clear();
                self.assistant_flushed = false;
            }
            StepBoundary::End { usage, .. } => {
                // Flush any remaining stage content delta.
                self.flush_stage_content().await;
                if let Some(usage) = usage {
                    if let Some(stage_ctx) = self.stage_ctx.as_ref() {
                        self.lifecycle_hook
                            .on_scheduler_stage_usage(
                                &stage_ctx.stage_name,
                                stage_ctx.stage_index,
                                usage,
                                true,
                                &self.exec_ctx,
                            )
                            .await;
                    }
                    append_output_usage(&mut self.output_metadata, &OutputUsage::from(usage));
                }
                if !self.assistant_flushed {
                    if let Some(message) = rocode_provider::Message::assistant_turn(
                        Some(&self.step_reasoning),
                        Some(&self.step_text),
                        &[],
                    ) {
                        self.messages.push(message);
                    }
                    self.assistant_flushed = true;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        output_metadata::continuation_targets, AgentDescriptor, AgentResolver, ExecutionContext,
        LifecycleHook, ModelRef, ModelResolver, OrchestratorContext, ToolExecError, ToolExecutor,
        ToolOutput,
    };
    use async_trait::async_trait;
    use futures::stream;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct TestAgentResolver;

    #[async_trait]
    impl AgentResolver for TestAgentResolver {
        fn resolve(&self, _name: &str) -> Option<AgentDescriptor> {
            None
        }
    }

    struct TestModelResolver {
        streams: Mutex<Vec<rocode_provider::StreamResult>>,
    }

    #[async_trait]
    impl ModelResolver for TestModelResolver {
        async fn chat_stream(
            &self,
            _model: Option<&ModelRef>,
            _messages: Vec<rocode_provider::Message>,
            _tools: Vec<rocode_provider::ToolDefinition>,
            _exec_ctx: &ExecutionContext,
        ) -> Result<rocode_provider::StreamResult, OrchestratorError> {
            self.streams
                .lock()
                .await
                .pop()
                .ok_or_else(|| OrchestratorError::Other("missing test stream".to_string()))
        }
    }

    struct TestToolExecutor;

    #[async_trait]
    impl ToolExecutor for TestToolExecutor {
        async fn execute(
            &self,
            tool_name: &str,
            _arguments: serde_json::Value,
            _exec_ctx: &ExecutionContext,
        ) -> Result<ToolOutput, ToolExecError> {
            Ok(ToolOutput {
                output: format!("tool:{}:ok", tool_name),
                is_error: false,
                title: Some("ok".to_string()),
                metadata: Some(json!({
                    "sessionId": "task_echo_123",
                    "agentTaskId": "agent-task-123"
                })),
            })
        }

        async fn list_ids(&self) -> Vec<String> {
            vec!["echo".to_string(), "invalid".to_string()]
        }

        async fn list_definitions(
            &self,
            _exec_ctx: &ExecutionContext,
        ) -> Vec<rocode_provider::ToolDefinition> {
            vec![rocode_provider::ToolDefinition {
                name: "echo".to_string(),
                description: Some("echo input".to_string()),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "value": { "type": "string" }
                    }
                }),
            }]
        }
    }

    struct TestLifecycleHook;

    #[async_trait]
    impl LifecycleHook for TestLifecycleHook {
        async fn on_orchestration_start(
            &self,
            _agent_name: &str,
            _max_steps: Option<u32>,
            _exec_ctx: &ExecutionContext,
        ) {
        }

        async fn on_step_start(
            &self,
            _agent_name: &str,
            _model_id: &str,
            _step: u32,
            _exec_ctx: &ExecutionContext,
        ) {
        }

        async fn on_orchestration_end(
            &self,
            _agent_name: &str,
            _steps: u32,
            _exec_ctx: &ExecutionContext,
        ) {
        }
    }

    fn stream_from(events: Vec<rocode_provider::StreamEvent>) -> rocode_provider::StreamResult {
        Box::pin(stream::iter(
            events
                .into_iter()
                .map(Ok::<_, rocode_provider::ProviderError>),
        ))
    }

    fn test_context(
        streams: Vec<rocode_provider::StreamResult>,
    ) -> (OrchestratorContext, ToolRunner) {
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(TestToolExecutor);
        let context = OrchestratorContext {
            agent_resolver: Arc::new(TestAgentResolver),
            model_resolver: Arc::new(TestModelResolver {
                streams: Mutex::new(streams),
            }),
            tool_executor: tool_executor.clone(),
            lifecycle_hook: Arc::new(TestLifecycleHook),
            cancel_token: Arc::new(crate::runtime::events::NeverCancel),
            exec_ctx: ExecutionContext {
                session_id: "test".to_string(),
                workdir: ".".to_string(),
                agent_name: "test-agent".to_string(),
                metadata: HashMap::new(),
            },
        };
        (context, ToolRunner::new(tool_executor))
    }

    #[tokio::test]
    async fn execute_returns_response_without_tool_calls() {
        let streams = vec![stream_from(vec![
            rocode_provider::StreamEvent::TextDelta("hello".to_string()),
            rocode_provider::StreamEvent::Done,
        ])];
        let (context, runner) = test_context(streams);
        let mut orchestrator = SkillListOrchestrator::new(
            AgentDescriptor {
                name: "test-agent".to_string(),
                system_prompt: None,
                model: Some(ModelRef {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-test".to_string(),
                }),
                max_steps: Some(4),
                temperature: None,
                allowed_tools: Vec::new(),
            },
            runner,
        );

        let output = orchestrator.execute("hi", &context).await.unwrap();
        assert_eq!(output.content, "hello");
        assert_eq!(output.steps, 1);
        assert_eq!(output.tool_calls_count, 0);
    }

    #[tokio::test]
    async fn execute_supports_unbounded_loop_policy() {
        let streams = vec![stream_from(vec![
            rocode_provider::StreamEvent::TextDelta("hello from unbounded".to_string()),
            rocode_provider::StreamEvent::Done,
        ])];
        let (context, runner) = test_context(streams);
        let mut orchestrator = SkillListOrchestrator::new(
            AgentDescriptor {
                name: "test-agent".to_string(),
                system_prompt: None,
                model: Some(ModelRef {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-test".to_string(),
                }),
                max_steps: None,
                temperature: None,
                allowed_tools: Vec::new(),
            },
            runner,
        )
        .with_loop_policy(LoopPolicy {
            max_steps: None,
            ..Default::default()
        });

        let output = orchestrator.execute("hi", &context).await.unwrap();
        assert_eq!(output.content, "hello from unbounded");
        assert_eq!(output.steps, 1);
        assert_eq!(output.tool_calls_count, 0);
    }

    #[tokio::test]
    async fn execute_maps_typed_provider_not_found_to_no_provider() {
        struct MissingProviderResolver;

        #[async_trait]
        impl ModelResolver for MissingProviderResolver {
            async fn chat_stream(
                &self,
                _model: Option<&ModelRef>,
                _messages: Vec<rocode_provider::Message>,
                _tools: Vec<rocode_provider::ToolDefinition>,
                _exec_ctx: &ExecutionContext,
            ) -> Result<rocode_provider::StreamResult, OrchestratorError> {
                Err(OrchestratorError::from_provider_error(
                    "openai",
                    Some("gpt-test"),
                    &rocode_provider::ProviderError::ProviderNotFound("openai".to_string()),
                ))
            }
        }

        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(TestToolExecutor);
        let context = OrchestratorContext {
            agent_resolver: Arc::new(TestAgentResolver),
            model_resolver: Arc::new(MissingProviderResolver),
            tool_executor: tool_executor.clone(),
            lifecycle_hook: Arc::new(TestLifecycleHook),
            cancel_token: Arc::new(crate::runtime::events::NeverCancel),
            exec_ctx: ExecutionContext {
                session_id: "test".to_string(),
                workdir: ".".to_string(),
                agent_name: "test-agent".to_string(),
                metadata: HashMap::new(),
            },
        };
        let mut orchestrator = SkillListOrchestrator::new(
            AgentDescriptor {
                name: "test-agent".to_string(),
                system_prompt: None,
                model: Some(ModelRef {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-test".to_string(),
                }),
                max_steps: Some(4),
                temperature: None,
                allowed_tools: Vec::new(),
            },
            ToolRunner::new(tool_executor),
        );

        let error = orchestrator.execute("hi", &context).await.unwrap_err();

        assert!(matches!(error, OrchestratorError::NoProvider));
    }

    #[tokio::test]
    async fn execute_handles_tool_call_then_finishes_next_step() {
        let streams = vec![
            stream_from(vec![
                rocode_provider::StreamEvent::TextDelta("done".to_string()),
                rocode_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                rocode_provider::StreamEvent::ToolCallEnd {
                    id: "tool-call-1".to_string(),
                    name: "echo".to_string(),
                    input: json!({"value":"x"}),
                },
                rocode_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner) = test_context(streams);
        let mut orchestrator = SkillListOrchestrator::new(
            AgentDescriptor {
                name: "test-agent".to_string(),
                system_prompt: None,
                model: Some(ModelRef {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-test".to_string(),
                }),
                max_steps: Some(4),
                temperature: None,
                allowed_tools: Vec::new(),
            },
            runner,
        );

        let output = orchestrator.execute("hi", &context).await.unwrap();
        assert_eq!(output.content, "done");
        assert_eq!(output.steps, 2);
        assert_eq!(output.tool_calls_count, 1);
        let continuation_targets = continuation_targets(&output.metadata);
        assert_eq!(continuation_targets.len(), 1);
        assert_eq!(continuation_targets[0].session_id, "task_echo_123");
        assert_eq!(
            continuation_targets[0].agent_task_id.as_deref(),
            Some("agent-task-123")
        );
    }

    #[tokio::test]
    async fn execute_replays_reasoning_into_next_orchestration() {
        struct CapturingModelResolver {
            streams: Mutex<Vec<rocode_provider::StreamResult>>,
            requests: Mutex<Vec<Vec<rocode_provider::Message>>>,
        }

        #[async_trait]
        impl ModelResolver for CapturingModelResolver {
            async fn chat_stream(
                &self,
                _model: Option<&ModelRef>,
                messages: Vec<rocode_provider::Message>,
                _tools: Vec<rocode_provider::ToolDefinition>,
                _exec_ctx: &ExecutionContext,
            ) -> Result<rocode_provider::StreamResult, OrchestratorError> {
                self.requests.lock().await.push(messages);
                self.streams
                    .lock()
                    .await
                    .pop()
                    .ok_or_else(|| OrchestratorError::Other("missing test stream".to_string()))
            }
        }

        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(TestToolExecutor);
        let resolver = Arc::new(CapturingModelResolver {
            streams: Mutex::new(vec![
                stream_from(vec![rocode_provider::StreamEvent::Done]),
                stream_from(vec![
                    rocode_provider::StreamEvent::TextDelta("stage two".to_string()),
                    rocode_provider::StreamEvent::Done,
                ]),
                stream_from(vec![
                    rocode_provider::StreamEvent::ReasoningDelta {
                        id: "reasoning-1".to_string(),
                        text: "need tool output first".to_string(),
                    },
                    rocode_provider::StreamEvent::ToolCallEnd {
                        id: "tool-call-1".to_string(),
                        name: "echo".to_string(),
                        input: json!({"value":"x"}),
                    },
                    rocode_provider::StreamEvent::Done,
                ]),
            ]),
            requests: Mutex::new(Vec::new()),
        });
        let context = OrchestratorContext {
            agent_resolver: Arc::new(TestAgentResolver),
            model_resolver: resolver.clone(),
            tool_executor: tool_executor.clone(),
            lifecycle_hook: Arc::new(TestLifecycleHook),
            cancel_token: Arc::new(crate::runtime::events::NeverCancel),
            exec_ctx: ExecutionContext {
                session_id: "test".to_string(),
                workdir: ".".to_string(),
                agent_name: "test-agent".to_string(),
                metadata: HashMap::new(),
            },
        };

        let mut orchestrator = SkillListOrchestrator::new(
            AgentDescriptor {
                name: "test-agent".to_string(),
                system_prompt: None,
                model: Some(ModelRef {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-test".to_string(),
                }),
                max_steps: Some(4),
                temperature: None,
                allowed_tools: Vec::new(),
            },
            ToolRunner::new(tool_executor),
        );

        let first = orchestrator.execute("first run", &context).await.unwrap();
        assert_eq!(first.tool_calls_count, 1);

        let _second = orchestrator.execute("second run", &context).await.unwrap();

        let requests = resolver.requests.lock().await.clone();
        assert!(requests.len() >= 3);
        let second_run_first_request = &requests[2];
        let assistant_with_reasoning = second_run_first_request
            .iter()
            .find(|message| matches!(message.role, rocode_provider::Role::Assistant))
            .expect("second execute should replay assistant history");

        match &assistant_with_reasoning.content {
            rocode_provider::Content::Parts(parts) => {
                assert_eq!(parts[0].content_type, "reasoning");
                assert_eq!(parts[0].text.as_deref(), Some("need tool output first"));
                assert_eq!(parts[1].content_type, "tool_use");
            }
            other => panic!("expected assistant parts replay, got {other:?}"),
        }
    }
}
