use crate::runtime::events::ModelFailure;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("model error: {0}")]
    ModelError(Box<ModelFailure>),

    #[error("tool execution failed: {tool} - {error}")]
    ToolError { tool: String, error: String },

    #[error("max steps exceeded{0}")]
    MaxStepsExceeded(String),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("no provider available")]
    NoProvider,

    #[error("orchestrator error: {0}")]
    Other(String),
}

impl OrchestratorError {
    pub fn from_provider_error(
        provider_id: &str,
        model_id: Option<&str>,
        error: &agendao_provider::ProviderError,
    ) -> Self {
        Self::ModelError(Box::new(ModelFailure::Provider(
            agendao_provider::summarize_provider_error(provider_id, model_id, error),
        )))
    }

    pub fn is_no_provider(&self) -> bool {
        match self {
            Self::NoProvider => true,
            Self::ModelError(failure) => failure.is_provider_not_found(),
            Self::ToolError { .. }
            | Self::MaxStepsExceeded(_)
            | Self::AgentNotFound(_)
            | Self::Other(_) => false,
        }
    }

    pub fn model_failure(&self) -> Option<&ModelFailure> {
        match self {
            Self::ModelError(failure) => Some(failure.as_ref()),
            Self::ToolError { .. }
            | Self::MaxStepsExceeded(_)
            | Self::AgentNotFound(_)
            | Self::NoProvider
            | Self::Other(_) => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolExecError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("execution error: {0}")]
    ExecutionError(String),
}
