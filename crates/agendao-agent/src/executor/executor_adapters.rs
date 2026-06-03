use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use agendao_execution_types::ExecutionRequestContext;
use agendao_orchestrator::{
    runtime::policy::ModelContextLimits, ExecutionContext, ModelRef as OrchestratorModelRef,
    ModelResolver as OrchestratorModelResolver, OrchestratorError,
    ToolExecError as OrchestratorToolExecError, ToolExecutor as OrchestratorToolExecutor,
    ToolOutput as OrchestratorToolOutput,
};
use agendao_provider::ProviderRegistry;
use agendao_tool::{ToolContext, ToolRegistry};

use super::{attach_subsession_callbacks, map_tool_error, SubsessionState};
use crate::AgentInfo;

fn preferred_tool_order_key(name: &str) -> (u8, &str) {
    match name {
        "task_flow" => (0, name),
        "task" => (1, name),
        "bash" => (3, name),
        _ => (2, name),
    }
}

fn prioritize_tool_definitions(tools: &mut [agendao_provider::ToolDefinition]) {
    tools.sort_by(|a, b| preferred_tool_order_key(&a.name).cmp(&preferred_tool_order_key(&b.name)));
}

fn map_tool_result(result: agendao_tool::ToolResult) -> OrchestratorToolOutput {
    OrchestratorToolOutput {
        output: result.output,
        is_error: false,
        title: if result.title.is_empty() {
            None
        } else {
            Some(result.title)
        },
        metadata: if result.metadata.is_empty() {
            None
        } else {
            Some(serde_json::to_value(result.metadata).unwrap_or(serde_json::Value::Null))
        },
    }
}

pub(super) struct ToolRegistryAdapter {
    tools: Arc<ToolRegistry>,
    agent: AgentInfo,
    disabled_tools: HashSet<String>,
    providers: Arc<ProviderRegistry>,
    subsessions: Arc<Mutex<HashMap<String, SubsessionState>>>,
    agent_registry: Arc<crate::AgentRegistry>,
    tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    question_callback: Option<agendao_tool::QuestionCallback>,
    ask_callback: Option<agendao_tool::AskCallback>,
}

pub(super) struct ToolRegistryAdapterDeps {
    pub(super) tools: Arc<ToolRegistry>,
    pub(super) disabled_tools: HashSet<String>,
    pub(super) providers: Arc<ProviderRegistry>,
    pub(super) subsessions: Arc<Mutex<HashMap<String, SubsessionState>>>,
    pub(super) agent_registry: Arc<crate::AgentRegistry>,
    pub(super) tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    pub(super) question_callback: Option<agendao_tool::QuestionCallback>,
    pub(super) ask_callback: Option<agendao_tool::AskCallback>,
}

impl ToolRegistryAdapter {
    pub(super) fn new(agent: AgentInfo, deps: ToolRegistryAdapterDeps) -> Self {
        Self {
            tools: deps.tools,
            agent,
            disabled_tools: deps.disabled_tools,
            providers: deps.providers,
            subsessions: deps.subsessions,
            agent_registry: deps.agent_registry,
            tool_runtime_config: deps.tool_runtime_config,
            question_callback: deps.question_callback,
            ask_callback: deps.ask_callback,
        }
    }

    fn ensure_tool_allowed(&self, tool_name: &str) -> Result<(), OrchestratorToolExecError> {
        self.agent
            .ensure_tool_allowed(tool_name)
            .map_err(OrchestratorToolExecError::PermissionDenied)
    }

    fn current_model_string(&self) -> Option<String> {
        if let Some(model) = self.agent.model.as_ref() {
            return Some(format!("{}:{}", model.provider_id, model.model_id));
        }

        let provider = self.providers.list().into_iter().next()?;
        let model_id = provider.models().first().map(|m| m.id.clone())?;
        Some(format!("{}:{}", provider.id(), model_id))
    }

    fn build_tool_context(&self, exec_ctx: &ExecutionContext) -> ToolContext {
        let directory = if exec_ctx.workdir.is_empty() {
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            exec_ctx.workdir.clone()
        };

        let current_model = self.current_model_string();
        let mut base_ctx = ToolContext::new(
            exec_ctx.session_id.clone(),
            "default".to_string(),
            directory,
        )
        .with_agent(self.agent.name.clone())
        .with_tool_runtime_config(self.tool_runtime_config.clone())
        .with_get_last_model({
            let current_model = current_model.clone();
            move |_session_id| {
                let current_model = current_model.clone();
                async move { Ok(current_model) }
            }
        });

        // Attach question callback if available
        if let Some(ref callback) = self.question_callback {
            base_ctx.ask_question = Some(callback.clone());
        }

        // Attach permission approval callback if available
        if let Some(ref callback) = self.ask_callback {
            base_ctx.ask = Some(callback.clone());
        }

        base_ctx.call_id = exec_ctx
            .metadata
            .get("call_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        base_ctx.extra = exec_ctx.metadata.clone();

        attach_subsession_callbacks(
            base_ctx,
            self.subsessions.clone(),
            self.providers.clone(),
            self.tools.clone(),
            self.agent_registry.clone(),
        )
    }
}

#[async_trait::async_trait]
impl OrchestratorToolExecutor for ToolRegistryAdapter {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        exec_ctx: &ExecutionContext,
    ) -> Result<OrchestratorToolOutput, OrchestratorToolExecError> {
        if self.disabled_tools.contains(tool_name) {
            return Err(OrchestratorToolExecError::PermissionDenied(format!(
                "Tool '{}' is disabled for this subagent session",
                tool_name
            )));
        }

        self.ensure_tool_allowed(tool_name)?;
        let ctx = self.build_tool_context(exec_ctx);
        let result = self
            .tools
            .execute(tool_name, arguments, ctx)
            .await
            .map_err(map_tool_error)?;
        Ok(map_tool_result(result))
    }

    async fn list_ids(&self) -> Vec<String> {
        self.tools.list_ids().await
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &ExecutionContext,
    ) -> Vec<agendao_provider::ToolDefinition> {
        let mut tools: Vec<agendao_provider::ToolDefinition> = self
            .tools
            .list_schemas()
            .await
            .into_iter()
            .filter(|schema| !self.disabled_tools.contains(&schema.name))
            .map(|schema| agendao_provider::ToolDefinition {
                name: schema.name,
                description: Some(schema.description),
                parameters: schema.parameters,
            })
            .collect();
        prioritize_tool_definitions(&mut tools);
        tools
    }
}

pub(super) struct ProviderModelResolver {
    pub(super) providers: Arc<ProviderRegistry>,
    pub(super) execution: ExecutionRequestContext,
}

#[async_trait::async_trait]
impl OrchestratorModelResolver for ProviderModelResolver {
    async fn chat_stream(
        &self,
        model: Option<&OrchestratorModelRef>,
        messages: Vec<agendao_provider::Message>,
        tools: Vec<agendao_provider::ToolDefinition>,
        _exec_ctx: &ExecutionContext,
    ) -> Result<agendao_provider::StreamResult, OrchestratorError> {
        let (provider, model_id) = if let Some(model) = model {
            let provider = self.providers.get(&model.provider_id).ok_or_else(|| {
                OrchestratorError::from_provider_error(
                    &model.provider_id,
                    Some(&model.model_id),
                    &agendao_provider::ProviderError::ProviderNotFound(model.provider_id.clone()),
                )
            })?;
            (provider, model.model_id.clone())
        } else if let Some(model) = self.execution.model_ref() {
            let provider = self.providers.get(&model.provider_id).ok_or_else(|| {
                OrchestratorError::from_provider_error(
                    &model.provider_id,
                    Some(&model.model_id),
                    &agendao_provider::ProviderError::ProviderNotFound(model.provider_id.clone()),
                )
            })?;
            (provider, model.model_id)
        } else {
            let provider = self
                .providers
                .list()
                .into_iter()
                .next()
                .ok_or(OrchestratorError::NoProvider)?;
            let model_id = provider
                .models()
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_default();
            (provider, model_id)
        };

        let request = self
            .execution
            .compile_with_model(model_id.clone())
            .to_chat_request(messages, tools, true);
        provider.chat_stream(request).await.map_err(|error| {
            OrchestratorError::from_provider_error(provider.id(), Some(&model_id), &error)
        })
    }

    async fn context_limits(
        &self,
        model: Option<&OrchestratorModelRef>,
        _exec_ctx: &ExecutionContext,
    ) -> Option<ModelContextLimits> {
        let (provider_id, model_id) = if let Some(model) = model {
            (model.provider_id.as_str(), model.model_id.as_str())
        } else {
            let model = self.execution.model_ref()?;
            let model_id = model.model_id;
            let provider_id = model.provider_id;
            return self
                .providers
                .get(provider_id.as_str())
                .and_then(|provider| {
                    provider
                        .get_model(model_id.as_str())
                        .map(ModelContextLimits::from_model_info)
                });
        };

        self.providers.get(provider_id).and_then(|provider| {
            provider
                .get_model(model_id)
                .map(ModelContextLimits::from_model_info)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prioritize_tool_definitions_prefers_task_flow_over_task() {
        let mut tools = vec![
            agendao_provider::ToolDefinition {
                name: "task".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            agendao_provider::ToolDefinition {
                name: "websearch".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            agendao_provider::ToolDefinition {
                name: "task_flow".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];

        prioritize_tool_definitions(&mut tools);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, vec!["task_flow", "task", "websearch"]);
    }

    #[test]
    fn prioritize_tool_definitions_pushes_bash_after_other_tools() {
        let mut tools = vec![
            agendao_provider::ToolDefinition {
                name: "bash".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            agendao_provider::ToolDefinition {
                name: "read".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            agendao_provider::ToolDefinition {
                name: "task".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];

        prioritize_tool_definitions(&mut tools);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, vec!["task", "read", "bash"]);
    }
}
