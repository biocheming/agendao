use super::{AssistantTurn, ConversationItem, ModelRequest, ToolCall, ToolExecution};
use crate::blueprint::{AgentNode, ExecutionLimits};
use crate::context::{
    build_prompt_surface, HandoffPacket, NodeResult, PromptAuthority, PromptSurfaceInput, Usage,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[async_trait]
pub trait ModelBackend: Send + Sync {
    async fn invoke(
        &self,
        request: ModelRequest,
        context: &AgentObservationContext<'_>,
        observer: &dyn AgentLoopObserver,
    ) -> Result<AssistantTurn, ModelBackendError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelBackendError {
    #[error("{message}", message = .0.message)]
    Provider(Box<agendao_provider::ProviderErrorSummary>),
    #[error("{0}")]
    Message(String),
}

impl ModelBackendError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

#[async_trait]
pub trait ToolBackend: Send + Sync {
    async fn execute(
        &self,
        context: &AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<ToolExecution, String>;
}

#[async_trait]
pub trait AgentLoopObserver: Send + Sync {
    async fn step_started(
        &self,
        _context: &AgentObservationContext<'_>,
        _step: u32,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn take_boundary_inputs(
        &self,
        _context: &AgentObservationContext<'_>,
    ) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    async fn assistant_turn(
        &self,
        _context: &AgentObservationContext<'_>,
        _turn: &AssistantTurn,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn text_delta(
        &self,
        _context: &AgentObservationContext<'_>,
        _text: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn reasoning_delta(
        &self,
        _context: &AgentObservationContext<'_>,
        _id: &str,
        _text: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn tool_input_delta(
        &self,
        _context: &AgentObservationContext<'_>,
        _id: &str,
        _tool: Option<&str>,
        _delta: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn tool_started(
        &self,
        _context: &AgentObservationContext<'_>,
        _call: &ToolCall,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn tool_finished(
        &self,
        _context: &AgentObservationContext<'_>,
        _call: &ToolCall,
        _result: &ToolExecution,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn step_finished(
        &self,
        _context: &AgentObservationContext<'_>,
        _step: u32,
        _usage: &Usage,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AgentObservationContext<'a> {
    pub node_path: &'a str,
    pub agent: &'a crate::blueprint::AgentId,
}

pub(crate) struct NoopAgentLoopObserver;

#[async_trait]
impl AgentLoopObserver for NoopAgentLoopObserver {}

pub(crate) static NOOP_AGENT_LOOP_OBSERVER: NoopAgentLoopObserver = NoopAgentLoopObserver;

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Debug, Clone, Default)]
pub struct CancellationFlag(Arc<CancellationState>);

impl CancellationFlag {
    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            self.0.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.0.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutionBudget {
    limits: ExecutionLimits,
    started: Instant,
    usage: Arc<Mutex<Usage>>,
}

impl ExecutionBudget {
    pub(crate) fn new(limits: ExecutionLimits) -> Self {
        Self {
            limits,
            started: Instant::now(),
            usage: Arc::new(Mutex::new(Usage::default())),
        }
    }

    pub(crate) fn check_time(&self) -> Result<(), AgentLoopError> {
        if self.started.elapsed() >= Duration::from_millis(self.limits.max_wall_time_ms) {
            return Err(AgentLoopError::DeadlineExceeded);
        }
        Ok(())
    }

    pub(crate) fn remaining(&self) -> Result<Duration, AgentLoopError> {
        Duration::from_millis(self.limits.max_wall_time_ms)
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(AgentLoopError::DeadlineExceeded)
    }

    pub(crate) fn reserve_model_call(&self) -> Result<(), AgentLoopError> {
        self.update(|usage| {
            if usage.model_calls >= self.limits.max_model_calls {
                return Err(AgentLoopError::ModelCallBudgetExceeded);
            }
            usage.model_calls += 1;
            Ok(())
        })
    }

    pub(crate) fn reserve_tool_call(&self) -> Result<(), AgentLoopError> {
        self.update(|usage| {
            if usage.tool_calls >= self.limits.max_tool_calls {
                return Err(AgentLoopError::ToolCallBudgetExceeded);
            }
            usage.tool_calls += 1;
            Ok(())
        })
    }

    pub(crate) fn record_tokens(&self, usage: &Usage) -> Result<(), AgentLoopError> {
        self.update(|total| {
            total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
            total.reasoning_tokens = total
                .reasoning_tokens
                .saturating_add(usage.reasoning_tokens);
            total.cache_read_tokens = total
                .cache_read_tokens
                .saturating_add(usage.cache_read_tokens);
            total.cache_miss_tokens = total
                .cache_miss_tokens
                .saturating_add(usage.cache_miss_tokens);
            total.cache_write_tokens = total
                .cache_write_tokens
                .saturating_add(usage.cache_write_tokens);
            if total.total_tokens() > self.limits.max_total_tokens {
                return Err(AgentLoopError::TokenBudgetExceeded);
            }
            Ok(())
        })
    }

    pub(crate) fn snapshot(&self) -> Usage {
        self.usage.lock().expect("budget mutex poisoned").clone()
    }

    fn update<T>(
        &self,
        update: impl FnOnce(&mut Usage) -> Result<T, AgentLoopError>,
    ) -> Result<T, AgentLoopError> {
        self.check_time()?;
        update(&mut self.usage.lock().expect("budget mutex poisoned"))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentLoopError {
    #[error("execution was cancelled")]
    Cancelled,
    #[error("execution deadline exceeded")]
    DeadlineExceeded,
    #[error("model call budget exceeded")]
    ModelCallBudgetExceeded,
    #[error("tool call budget exceeded")]
    ToolCallBudgetExceeded,
    #[error("token budget exceeded")]
    TokenBudgetExceeded,
    #[error("agent exhausted its {steps} steps without a final response")]
    StepLimitExceeded { steps: u32 },
    #[error("model invocation failed: {0}")]
    Model(#[source] ModelBackendError),
    #[error("tool '{tool}' failed: {message}")]
    Tool { tool: String, message: String },
    #[error("model requested undeclared tool '{0}'")]
    UndeclaredTool(String),
    #[error("prompt surface construction failed: {0}")]
    Prompt(String),
    #[error("agent loop observer failed: {0}")]
    Observer(String),
}

pub(crate) struct AgentLoop<'a> {
    model: &'a dyn ModelBackend,
    tools: &'a dyn ToolBackend,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentRunOutcome {
    pub result: NodeResult,
    pub conversation: Vec<ConversationItem>,
}

pub(crate) struct AgentRunContext<'a> {
    pub(crate) prompt_authority: &'a PromptAuthority<'a>,
    pub(crate) workspace_summary: &'a str,
    pub(crate) progress_summary: &'a str,
    pub(crate) budget: &'a ExecutionBudget,
    pub(crate) cancellation: &'a CancellationFlag,
    pub(crate) observer: &'a dyn AgentLoopObserver,
}

impl<'a> AgentLoop<'a> {
    pub(crate) fn new(model: &'a dyn ModelBackend, tools: &'a dyn ToolBackend) -> Self {
        Self { model, tools }
    }

    pub(crate) async fn run(
        &self,
        node: &AgentNode,
        handoff: HandoffPacket,
        conversation_seed: Vec<agendao_provider::Message>,
        context: AgentRunContext<'_>,
    ) -> Result<AgentRunOutcome, AgentLoopError> {
        let observation = AgentObservationContext {
            node_path: context.progress_summary,
            agent: &node.agent,
        };
        let conversation_seed: Arc<[agendao_provider::Message]> = conversation_seed.into();
        let mut conversation = Arc::new(Vec::new());
        let mut continuation = None;
        let mut previous_fingerprints = None;
        let mut local_usage = Usage::default();
        for step in 1..=node.max_steps {
            check_cancelled(context.cancellation)?;
            context
                .observer
                .step_started(&observation, step)
                .await
                .map_err(AgentLoopError::Observer)?;
            let boundary_inputs = context
                .observer
                .take_boundary_inputs(&observation)
                .await
                .map_err(AgentLoopError::Observer)?;
            Arc::make_mut(&mut conversation).extend(
                boundary_inputs
                    .into_iter()
                    .map(|content| ConversationItem::User { content }),
            );
            context.budget.reserve_model_call()?;
            local_usage.model_calls += 1;
            let prompt = build_prompt_surface(
                context.prompt_authority,
                node,
                PromptSurfaceInput {
                    workspace_summary: context.workspace_summary.to_string(),
                    progress_summary: context.progress_summary.to_string(),
                    handoff: handoff.clone(),
                    history_tail: Arc::clone(&conversation),
                    reasoning_continuation: continuation.as_deref(),
                },
            )
            .map_err(|error| AgentLoopError::Prompt(error.to_string()))?;
            let cache_diagnostic = prompt.fingerprints.compare(previous_fingerprints.as_ref());
            tracing::debug!(
                node_path = observation.node_path,
                agent = observation.agent.as_str(),
                stable_prefix_fingerprint = %prompt.fingerprints.agent_surface,
                cache_diagnostic = ?cache_diagnostic,
                "scheduler prompt cache diagnostic"
            );
            previous_fingerprints = Some(prompt.fingerprints.clone());
            let model_request = ModelRequest {
                agent: node.agent.clone(),
                skills: node.skills.clone(),
                tools: node.tools.clone(),
                prompt,
                reasoning_continuation: continuation.clone(),
                conversation_seed: conversation_seed.clone(),
            };
            let turn = tokio::select! {
                _ = context.cancellation.cancelled() => return Err(AgentLoopError::Cancelled),
                result = tokio::time::timeout(
                    context.budget.remaining()?,
                    self.model.invoke(model_request, &observation, context.observer),
                ) => result
                    .map_err(|_| AgentLoopError::DeadlineExceeded)?
                    .map_err(AgentLoopError::Model)?,
            };
            check_cancelled(context.cancellation)?;
            context.budget.record_tokens(&turn.usage)?;
            local_usage.input_tokens = local_usage
                .input_tokens
                .saturating_add(turn.usage.input_tokens);
            local_usage.output_tokens = local_usage
                .output_tokens
                .saturating_add(turn.usage.output_tokens);
            local_usage.reasoning_tokens = local_usage
                .reasoning_tokens
                .saturating_add(turn.usage.reasoning_tokens);
            local_usage.cache_read_tokens = local_usage
                .cache_read_tokens
                .saturating_add(turn.usage.cache_read_tokens);
            local_usage.cache_miss_tokens = local_usage
                .cache_miss_tokens
                .saturating_add(turn.usage.cache_miss_tokens);
            local_usage.cache_write_tokens = local_usage
                .cache_write_tokens
                .saturating_add(turn.usage.cache_write_tokens);
            continuation = turn.reasoning_continuation.clone();
            context
                .observer
                .assistant_turn(&observation, &turn)
                .await
                .map_err(AgentLoopError::Observer)?;

            if turn.tool_calls.is_empty() {
                context
                    .observer
                    .step_finished(&observation, step, &local_usage)
                    .await
                    .map_err(AgentLoopError::Observer)?;
                let output = turn.content.clone().unwrap_or_default();
                Arc::make_mut(&mut conversation).push(ConversationItem::Assistant { turn });
                return Ok(AgentRunOutcome {
                    result: NodeResult {
                        summary: output.clone(),
                        output: Some(output),
                        usage: local_usage,
                        ..NodeResult::default()
                    },
                    conversation: Arc::try_unwrap(conversation)
                        .unwrap_or_else(|shared| shared.as_ref().clone()),
                });
            }

            let mut tool_results = Vec::with_capacity(turn.tool_calls.len());
            for call in &turn.tool_calls {
                check_cancelled(context.cancellation)?;
                if !node.tools.contains(&call.tool) {
                    return Err(AgentLoopError::UndeclaredTool(
                        call.tool.as_str().to_string(),
                    ));
                }
                context.budget.reserve_tool_call()?;
                local_usage.tool_calls += 1;
                context
                    .observer
                    .tool_started(&observation, call)
                    .await
                    .map_err(AgentLoopError::Observer)?;
                let output = tokio::select! {
                    _ = context.cancellation.cancelled() => {
                        return Err(AgentLoopError::Cancelled);
                    }
                    result = tokio::time::timeout(
                        context.budget.remaining()?,
                        self.tools.execute(&observation, call),
                    ) => result
                        .map_err(|_| AgentLoopError::DeadlineExceeded)?
                        .map_err(|message| AgentLoopError::Tool {
                            tool: call.tool.as_str().to_string(),
                            message,
                        })?,
                };
                context
                    .observer
                    .tool_finished(&observation, call, &output)
                    .await
                    .map_err(AgentLoopError::Observer)?;
                tool_results.push(ConversationItem::ToolResult {
                    call_id: call.id.clone(),
                    output: output.output,
                    is_error: output.is_error,
                });
            }
            let conversation = Arc::make_mut(&mut conversation);
            conversation.push(ConversationItem::Assistant { turn });
            conversation.extend(tool_results);
            context
                .observer
                .step_finished(&observation, step, &local_usage)
                .await
                .map_err(AgentLoopError::Observer)?;
        }
        Err(AgentLoopError::StepLimitExceeded {
            steps: node.max_steps,
        })
    }
}

fn check_cancelled(cancellation: &CancellationFlag) -> Result<(), AgentLoopError> {
    if cancellation.is_cancelled() {
        Err(AgentLoopError::Cancelled)
    } else {
        Ok(())
    }
}
