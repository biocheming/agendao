use crate::runtime::events::{
    LoopError, LoopEvent, LoopRequest, ModelFailure, StepBoundary, ToolCallReady, ToolResult,
};
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
    ) -> Result<rocode_provider::StreamResult, LoopError>;

    fn model_failure_from_provider_error(
        &self,
        error: &rocode_provider::ProviderError,
    ) -> ModelFailure {
        ModelFailure::Provider(rocode_provider::summarize_provider_error(
            "unconfigured",
            None,
            error,
        ))
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
    async fn list_definitions(&self) -> Vec<rocode_provider::ToolDefinition>;
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
}
