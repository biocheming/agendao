use crate::runtime::events::{LoopError, LoopRequest, ModelFailure, ToolCallReady, ToolResult};
use crate::runtime::policy::ModelContextLimits;
use crate::runtime::traits::{ModelCaller, ToolDispatcher};
use crate::tool_runner::{ToolCallInput, ToolRunner};
use crate::traits::{ModelResolver, ToolExecutor};
use crate::types::{ExecutionContext, ModelRef};
use async_trait::async_trait;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// ModelCallerBridge – adapts orchestrator ModelResolver → runtime ModelCaller.
//
// Captures model identity and execution context so that run_loop does not
// need to know about orchestrator-level concerns.
// ---------------------------------------------------------------------------

pub struct ModelCallerBridge {
    model_resolver: Arc<dyn ModelResolver>,
    model: Option<ModelRef>,
    exec_ctx: ExecutionContext,
    context_limits: Option<ModelContextLimits>,
}

impl ModelCallerBridge {
    pub async fn new(
        model_resolver: Arc<dyn ModelResolver>,
        model: Option<ModelRef>,
        exec_ctx: ExecutionContext,
    ) -> Self {
        let context_limits = model_resolver
            .context_limits(model.as_ref(), &exec_ctx)
            .await;
        Self {
            model_resolver,
            model,
            exec_ctx,
            context_limits,
        }
    }

    fn no_provider_failure(&self) -> ModelFailure {
        let provider_id = self
            .model
            .as_ref()
            .map(|model| model.provider_id.clone())
            .unwrap_or_else(|| "unconfigured".to_string());
        let model_id = self.model.as_ref().map(|model| model.model_id.as_str());
        let provider_error = agendao_provider::ProviderError::ProviderNotFound(provider_id.clone());
        ModelFailure::Provider(agendao_provider::summarize_provider_error(
            provider_id.as_str(),
            model_id,
            &provider_error,
        ))
    }

    fn model_failure_from_orchestrator_error(
        &self,
        error: crate::error::OrchestratorError,
    ) -> ModelFailure {
        match error {
            crate::error::OrchestratorError::ModelError(failure) => failure,
            crate::error::OrchestratorError::NoProvider => self.no_provider_failure(),
            other => ModelFailure::Message(other.to_string()),
        }
    }
}

#[async_trait]
impl ModelCaller for ModelCallerBridge {
    async fn call_stream(
        &self,
        req: LoopRequest,
    ) -> Result<agendao_provider::StreamResult, LoopError> {
        self.model_resolver
            .chat_stream(self.model.as_ref(), req.messages, req.tools, &self.exec_ctx)
            .await
            .map_err(|e| LoopError::ModelError(self.model_failure_from_orchestrator_error(e)))
    }

    fn model_failure_from_provider_error(
        &self,
        error: &agendao_provider::ProviderError,
    ) -> ModelFailure {
        let Some(model) = self.model.as_ref() else {
            return ModelFailure::Provider(agendao_provider::summarize_provider_error(
                "unconfigured",
                None,
                error,
            ));
        };

        ModelFailure::Provider(agendao_provider::summarize_provider_error(
            model.provider_id.as_str(),
            Some(model.model_id.as_str()),
            error,
        ))
    }

    fn context_limits(&self) -> Option<ModelContextLimits> {
        self.context_limits.or_else(|| {
            self.model
                .as_ref()
                .map(|model| ModelContextLimits::heuristic_for_model_id(&model.model_id))
        })
    }
}

// ---------------------------------------------------------------------------
// ToolDispatcherBridge – adapts orchestrator ToolRunner → runtime ToolDispatcher.
//
// Preserves ToolRunner's name-repair and "invalid" fallback logic.
// Captures execution context for tool dispatch and definition listing.
// ---------------------------------------------------------------------------

pub struct ToolDispatcherBridge {
    tool_runner: ToolRunner,
    tool_executor: Arc<dyn ToolExecutor>,
    exec_ctx: ExecutionContext,
}

impl ToolDispatcherBridge {
    pub fn new(
        tool_runner: ToolRunner,
        tool_executor: Arc<dyn ToolExecutor>,
        exec_ctx: ExecutionContext,
    ) -> Self {
        Self {
            tool_runner,
            tool_executor,
            exec_ctx,
        }
    }
}

#[async_trait]
impl ToolDispatcher for ToolDispatcherBridge {
    async fn execute(&self, call: &ToolCallReady) -> ToolResult {
        let input = ToolCallInput {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        };
        let mut exec_ctx = self.exec_ctx.clone();
        exec_ctx
            .metadata
            .insert("call_id".to_string(), serde_json::json!(call.id));
        let output = self.tool_runner.execute_tool_call(input, &exec_ctx).await;
        ToolResult {
            tool_call_id: output.tool_call_id,
            tool_name: output.tool_name,
            output: output.content,
            is_error: output.is_error,
            title: output.title,
            metadata: output.metadata,
        }
    }

    async fn list_definitions(&self) -> Vec<agendao_provider::ToolDefinition> {
        self.tool_executor.list_definitions(&self.exec_ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::policy::ModelContextLimits;
    use crate::{ExecutionContext, OrchestratorError};
    use async_trait::async_trait;
    use std::collections::HashMap;

    struct LimitsOnlyResolver {
        limits: Option<ModelContextLimits>,
    }

    #[async_trait]
    impl ModelResolver for LimitsOnlyResolver {
        async fn chat_stream(
            &self,
            _model: Option<&ModelRef>,
            _messages: Vec<agendao_provider::Message>,
            _tools: Vec<agendao_provider::ToolDefinition>,
            _exec_ctx: &ExecutionContext,
        ) -> Result<agendao_provider::StreamResult, OrchestratorError> {
            unreachable!("chat_stream should not be called in this test")
        }

        async fn context_limits(
            &self,
            _model: Option<&ModelRef>,
            _exec_ctx: &ExecutionContext,
        ) -> Option<ModelContextLimits> {
            self.limits
        }
    }

    #[tokio::test]
    async fn model_caller_bridge_prefers_resolver_context_limits() {
        let exact = ModelContextLimits {
            context_window_tokens: Some(1_000_000),
            max_input_tokens: Some(900_000),
            max_output_tokens: Some(32_000),
        };
        let bridge = ModelCallerBridge::new(
            Arc::new(LimitsOnlyResolver {
                limits: Some(exact),
            }),
            Some(ModelRef {
                provider_id: "mock".to_string(),
                model_id: "tiny-model".to_string(),
            }),
            ExecutionContext {
                session_id: "session".to_string(),
                workdir: ".".to_string(),
                agent_name: "agent".to_string(),
                metadata: HashMap::new(),
            },
        )
        .await;

        assert_eq!(bridge.context_limits(), Some(exact));
    }
}
