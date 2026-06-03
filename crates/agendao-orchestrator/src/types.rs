use crate::runtime::events::{CancelToken, FinishReason};
use crate::traits::{AgentResolver, LifecycleHook, ModelResolver, ToolExecutor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDescriptor {
    pub name: String,
    #[serde(
        default,
        alias = "systemPrompt",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    #[serde(default, alias = "maxSteps", skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(
        default,
        alias = "temperature",
        skip_serializing_if = "Option::is_none"
    )]
    pub temperature: Option<f32>,
    #[serde(default, alias = "allowedTools", skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRef {
    #[serde(alias = "providerId")]
    pub provider_id: String,
    #[serde(alias = "modelId")]
    pub model_id: String,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub output: String,
    pub is_error: bool,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub session_id: String,
    pub workdir: String,
    pub agent_name: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

pub struct OrchestratorContext {
    pub agent_resolver: Arc<dyn AgentResolver>,
    pub model_resolver: Arc<dyn ModelResolver>,
    pub tool_executor: Arc<dyn ToolExecutor>,
    pub lifecycle_hook: Arc<dyn LifecycleHook>,
    pub cancel_token: Arc<dyn CancelToken>,
    pub exec_ctx: ExecutionContext,
}

#[derive(Debug, Clone)]
pub struct OrchestratorOutput {
    pub content: String,
    pub steps: u32,
    pub tool_calls_count: u32,
    pub metadata: HashMap<String, serde_json::Value>,
    pub finish_reason: FinishReason,
}

impl OrchestratorOutput {
    /// Returns true if the execution was stopped by a cancellation token.
    pub fn is_cancelled(&self) -> bool {
        matches!(self.finish_reason, FinishReason::Cancelled)
    }

    /// Create an empty output, used to close a stage that produced no content.
    pub fn empty() -> Self {
        Self {
            content: String::new(),
            steps: 0,
            tool_calls_count: 0,
            metadata: HashMap::new(),
            finish_reason: FinishReason::EndTurn,
        }
    }
}
