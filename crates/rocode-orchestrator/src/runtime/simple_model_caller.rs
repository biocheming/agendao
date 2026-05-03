//! A generic `ModelCaller` implementation that wraps any `rocode_provider::Provider`.
//!
//! This eliminates the need for each consumer (session, server, compaction) to
//! implement its own near-identical ModelCaller. Per Constitution Article 1,
//! the execution kernel's adapter types should be written once.

use std::sync::Arc;

use crate::request_execution::CompiledExecutionRequest;
use crate::runtime::events::{LoopError as RuntimeLoopError, LoopRequest, ModelFailure};
use crate::runtime::traits::ModelCaller;
use rocode_provider::{summarize_provider_error, Provider, ProviderError, StreamResult};

/// Configuration for building `ChatRequest` from `LoopRequest`.
#[derive(Clone)]
pub struct SimpleModelCallerConfig {
    pub request: CompiledExecutionRequest,
}

/// A reusable `ModelCaller` that translates `LoopRequest` → `ChatRequest` using
/// a `Provider` and `SimpleModelCallerConfig`. Covers the common case shared by
/// session, server, and compaction callers.
pub struct SimpleModelCaller {
    pub provider: Arc<dyn Provider>,
    pub config: SimpleModelCallerConfig,
}

#[async_trait::async_trait]
impl ModelCaller for SimpleModelCaller {
    async fn call_stream(
        &self,
        req: LoopRequest,
    ) -> std::result::Result<StreamResult, RuntimeLoopError> {
        let request = self
            .config
            .request
            .to_chat_request(req.messages, req.tools, true);
        self.provider.chat_stream(request).await.map_err(|error| {
            RuntimeLoopError::ModelError(self.model_failure_from_provider_error(&error))
        })
    }

    fn model_failure_from_provider_error(&self, error: &ProviderError) -> ModelFailure {
        ModelFailure::Provider(summarize_provider_error(
            self.provider.id(),
            Some(self.config.request.model_id.as_str()),
            error,
        ))
    }
}
