use crate::error::{OrchestratorError, ToolExecError};
use crate::runtime::events::{StepCheckpointDirective, StepCheckpointSnapshot, StepUsage};
use crate::runtime::policy::ModelContextLimits;
use crate::scheduler::SchedulerStageCapabilities;
use crate::types::{
    AgentDescriptor, ExecutionContext, ModelRef, OrchestratorContext, OrchestratorOutput,
    ToolOutput,
};
use async_trait::async_trait;

#[async_trait]
pub trait AgentResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Option<AgentDescriptor>;
}

#[async_trait]
pub trait ModelResolver: Send + Sync {
    async fn chat_stream(
        &self,
        model: Option<&ModelRef>,
        messages: Vec<rocode_provider::Message>,
        tools: Vec<rocode_provider::ToolDefinition>,
        exec_ctx: &ExecutionContext,
    ) -> Result<rocode_provider::StreamResult, OrchestratorError>;

    async fn context_limits(
        &self,
        _model: Option<&ModelRef>,
        _exec_ctx: &ExecutionContext,
    ) -> Option<ModelContextLimits> {
        None
    }
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        exec_ctx: &ExecutionContext,
    ) -> Result<ToolOutput, ToolExecError>;

    async fn list_ids(&self) -> Vec<String>;

    async fn list_definitions(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Vec<rocode_provider::ToolDefinition>;
}

#[async_trait]
pub trait LifecycleHook: Send + Sync {
    async fn on_orchestration_start(
        &self,
        agent_name: &str,
        max_steps: Option<u32>,
        exec_ctx: &ExecutionContext,
    );

    async fn on_step_start(
        &self,
        agent_name: &str,
        model_id: &str,
        step: u32,
        exec_ctx: &ExecutionContext,
    );

    async fn on_tool_start(
        &self,
        _agent_name: &str,
        _tool_call_id: &str,
        _tool_name: &str,
        _tool_args: &serde_json::Value,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional observability hook. Most hook implementations only care about
        // a subset of lifecycle events, so the default remains a true no-op.
    }

    async fn on_tool_end(
        &self,
        _agent_name: &str,
        _tool_call_id: &str,
        _tool_name: &str,
        _tool_output: &ToolOutput,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional observability hook. Concrete implementations should add
        // behavior here instead of changing the default no-op contract.
    }

    async fn on_orchestration_end(&self, agent_name: &str, steps: u32, exec_ctx: &ExecutionContext);

    async fn on_scheduler_stage_start(
        &self,
        _agent_name: &str,
        _stage_name: &str,
        _stage_index: u32,
        _capabilities: Option<&SchedulerStageCapabilities>,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional stage-timeline hook for implementations that surface
        // scheduler progress, usage, or remote control metadata.
    }

    async fn on_scheduler_stage_end(
        &self,
        _agent_name: &str,
        _stage_name: &str,
        _stage_index: u32,
        _stage_total: u32,
        _content: &str,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional stage-timeline hook. Left empty so callers can ignore stage
        // end notifications without extra boilerplate.
    }

    async fn on_scheduler_stage_content(
        &self,
        _stage_name: &str,
        _stage_index: u32,
        _content_delta: &str,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional streaming hook for UIs or telemetry sinks that want
        // incremental stage content.
    }

    async fn on_scheduler_stage_reasoning(
        &self,
        _stage_name: &str,
        _stage_index: u32,
        _reasoning_delta: &str,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional streaming hook for implementations that expose reasoning
        // separately from visible content.
    }

    async fn on_scheduler_stage_usage(
        &self,
        _stage_name: &str,
        _stage_index: u32,
        _usage: &StepUsage,
        _finalized: bool,
        _exec_ctx: &ExecutionContext,
    ) {
        // Optional usage hook. The default no-op keeps the trait ergonomic for
        // callers that do not collect per-stage token accounting.
    }

    async fn on_step_checkpoint(
        &self,
        _agent_name: &str,
        _model_id: &str,
        _step: u32,
        _stage_name: Option<&str>,
        _stage_index: Option<u32>,
        _usage: &StepUsage,
        _request_view: &[rocode_provider::Message],
        _checkpoint: &StepCheckpointSnapshot,
        _default_directive: &StepCheckpointDirective,
        _exec_ctx: &ExecutionContext,
    ) -> Result<Option<StepCheckpointDirective>, OrchestratorError> {
        // Optional governance hook that runs after a step has fully completed
        // and before the next provider request is allowed to start.
        Ok(None)
    }
}

/// Null-object hook for runtimes and tests that do not need lifecycle
/// observability. Keeping this explicit avoids ad hoc empty hook structs at
/// call sites.
pub struct NoopLifecycleHook;

#[async_trait]
impl LifecycleHook for NoopLifecycleHook {
    async fn on_orchestration_start(&self, _: &str, _: Option<u32>, _: &ExecutionContext) {}

    async fn on_step_start(&self, _: &str, _: &str, _: u32, _: &ExecutionContext) {}

    async fn on_orchestration_end(&self, _: &str, _: u32, _: &ExecutionContext) {}
}

#[async_trait]
pub trait Orchestrator: Send + Sync {
    async fn execute(
        &mut self,
        input: &str,
        ctx: &OrchestratorContext,
    ) -> Result<OrchestratorOutput, OrchestratorError>;
}
