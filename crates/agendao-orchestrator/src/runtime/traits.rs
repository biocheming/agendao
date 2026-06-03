use crate::runtime::events::{
    LoopError, LoopEvent, LoopRequest, ModelFailure, StepBoundary, StepCheckpointDirective,
    StepCheckpointSnapshot, ToolCallReady, ToolResult,
};
use crate::runtime::policy::ModelContextLimits;
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// ModelCaller – abstracts the LLM provider.
// Implementation owns model config (id, temperature, max_tokens, etc.).
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ModelCaller: Send + Sync {
    async fn call_stream(
        &self,
        req: LoopRequest,
    ) -> Result<agendao_provider::StreamResult, LoopError>;

    fn model_failure_from_provider_error(
        &self,
        error: &agendao_provider::ProviderError,
    ) -> ModelFailure {
        ModelFailure::Provider(agendao_provider::summarize_provider_error(
            "unconfigured",
            None,
            error,
        ))
    }

    fn context_limits(&self) -> Option<ModelContextLimits> {
        None
    }
}

// ---------------------------------------------------------------------------
// ToolDispatcher – abstracts tool execution.
// Implementation owns the tool registry, permission checks, etc.
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    /// Execute a fully-assembled tool call.
    async fn execute(&self, call: &ToolCallReady) -> ToolResult;

    /// List available tool definitions for the model.
    async fn list_definitions(&self) -> Vec<agendao_provider::ToolDefinition>;
}

// ---------------------------------------------------------------------------
// LoopSink – receives normalized events and tool results.
// Session implements this with persistence + UI push.
// Orchestrator implements this as lightweight in-memory accumulator.
// ---------------------------------------------------------------------------

#[async_trait]
pub trait LoopSink: Send {
    /// Called for each normalized event from the model stream.
    async fn on_event(&mut self, ev: &LoopEvent) -> Result<(), LoopError>;

    /// Called after a tool has been executed.
    async fn on_tool_result(
        &mut self,
        call: &ToolCallReady,
        result: &ToolResult,
    ) -> Result<(), LoopError>;

    /// Called at step boundaries. End variant includes finish_reason,
    /// tool_calls_count, and had_error so the Sink does not need to
    /// infer these from the event stream.
    async fn on_step_boundary(&mut self, ctx: &StepBoundary) -> Result<(), LoopError>;

    /// Called after a step boundary has finalized and before the next request
    /// is allowed to reuse the in-flight request view.
    async fn on_step_checkpoint(
        &mut self,
        _ctx: &StepBoundary,
        _request_view: &[agendao_provider::Message],
        _checkpoint: &StepCheckpointSnapshot,
        _default_directive: &StepCheckpointDirective,
    ) -> Result<Option<StepCheckpointDirective>, LoopError> {
        Ok(None)
    }
}
