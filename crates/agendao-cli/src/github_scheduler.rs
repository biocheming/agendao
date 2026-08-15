use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agendao_agent::AgentInfo;
use agendao_execution_types::CompiledExecutionRequest;
use agendao_orchestrator::agent_loop::{
    AgentObservationContext, CancellationFlag, ProviderModelBackend, ToolBackend, ToolCall,
    ToolExecution,
};
use agendao_orchestrator::blueprint::{
    AgentId, BlueprintName, ExecutionLimits, ModelCapability, OutputContract, OutputFormat, ToolId,
    ValidatedBlueprint,
};
use agendao_orchestrator::catalog::{
    AgentCatalogEntry, EffectClass, PermissionClass, SchedulerCatalog, ToolCatalogEntry,
};
use agendao_orchestrator::context::{HandoffPacket, NodeResult};
use agendao_orchestrator::engine::{
    ArtifactRequest, CapabilityBackend, CheckpointHandle, CheckpointRequest, EvaluationOutcome,
    EvaluatorBackend, RestoreRequest, RunDisposition, RunRequest, SchedulerEngine,
};
use agendao_orchestrator::policy::PolicyEnvelope;
use agendao_orchestrator::templates::{build_template, TemplateId, TemplateParameters};
use agendao_provider::{Provider, ProviderRegistry, ToolDefinition};
use agendao_tool::{ToolContext, ToolRegistry, ToolRuntimeConfig};

struct GithubToolBackend {
    registry: Arc<ToolRegistry>,
    agent: AgentInfo,
    context: ToolContext,
}

impl GithubToolBackend {
    async fn new(
        registry: Arc<ToolRegistry>,
        agent: AgentInfo,
        directory: String,
        runtime_config: ToolRuntimeConfig,
    ) -> Self {
        let context = ToolContext::new("github".to_string(), "github".to_string(), directory)
            .with_agent(agent.name.clone())
            .with_tool_runtime_config(runtime_config);
        Self {
            registry,
            agent,
            context,
        }
    }

    async fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self
            .registry
            .list_schemas()
            .await
            .into_iter()
            .filter(|schema| self.agent.is_tool_allowed(&schema.name))
            .map(|schema| ToolDefinition {
                name: schema.name,
                description: Some(schema.description),
                parameters: schema.parameters,
            })
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }
}

#[async_trait::async_trait]
impl ToolBackend for GithubToolBackend {
    async fn execute(
        &self,
        _observation: &AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<ToolExecution, String> {
        if let Err(error) = self.agent.ensure_tool_allowed(call.tool.as_str()) {
            return Ok(ToolExecution {
                output: error,
                title: Some("Tool denied".to_string()),
                metadata: None,
                is_error: true,
            });
        }
        let mut context = self.context.clone();
        context.call_id = Some(call.id.clone());
        Ok(
            match self
                .registry
                .execute(call.tool.as_str(), call.arguments.clone(), context)
                .await
            {
                Ok(result) => ToolExecution {
                    output: result.output,
                    title: (!result.title.is_empty()).then_some(result.title),
                    metadata: (!result.metadata.is_empty())
                        .then(|| serde_json::to_value(result.metadata).unwrap_or_default()),
                    is_error: false,
                },
                Err(error) => ToolExecution {
                    output: error.to_string(),
                    title: Some("Tool error".to_string()),
                    metadata: None,
                    is_error: true,
                },
            },
        )
    }
}

struct DirectRunHost;

#[async_trait::async_trait]
impl EvaluatorBackend for DirectRunHost {
    async fn evaluate(
        &self,
        _evaluator: &agendao_orchestrator::blueprint::EvaluatorId,
        _candidate: &NodeResult,
    ) -> Result<EvaluationOutcome, String> {
        Err("direct scheduler does not provide an evaluator".to_string())
    }
}

#[async_trait::async_trait]
impl CapabilityBackend for DirectRunHost {
    async fn checkpoint(&self, _request: &CheckpointRequest) -> Result<CheckpointHandle, String> {
        Err("direct scheduler does not provide checkpoint capability".to_string())
    }

    async fn restore(&self, _request: &RestoreRequest) -> Result<(), String> {
        Err("direct scheduler does not provide checkpoint restore".to_string())
    }

    async fn store_artifact(&self, _request: &ArtifactRequest) -> Result<String, String> {
        Err("direct scheduler does not provide artifact storage".to_string())
    }

    async fn finalize(&self, _disposition: RunDisposition) -> Result<(), String> {
        Ok(())
    }
}

pub(super) async fn run_github_prompt(
    prompt: &str,
    agent: AgentInfo,
    providers: Arc<ProviderRegistry>,
    tools: Arc<ToolRegistry>,
    runtime_config: ToolRuntimeConfig,
    directory: String,
) -> anyhow::Result<String> {
    let (provider, model_id) = resolve_provider(&agent, &providers)?;
    let request = compiled_request(&agent, model_id.clone());
    let tool_backend =
        GithubToolBackend::new(tools, agent.clone(), directory.clone(), runtime_config).await;
    let definitions = tool_backend.definitions().await;
    let tool_ids = definitions
        .iter()
        .map(|definition| ToolId::new(definition.name.clone()))
        .collect::<BTreeSet<_>>();
    let agent_id = AgentId::new(agent.name.clone());
    let limits = execution_limits(&agent);
    let catalog = SchedulerCatalog {
        revision: "github-direct-v1".to_string(),
        agents: BTreeMap::from([(
            agent_id.clone(),
            AgentCatalogEntry {
                id: agent_id.clone(),
                system_policy: agent.resolved_system_prompt().unwrap_or_default(),
                available_skills: BTreeSet::new(),
                available_tools: tool_ids.clone(),
                model_capabilities: BTreeSet::from([
                    ModelCapability::ToolCalls,
                    ModelCapability::Reasoning,
                    ModelCapability::Attachments,
                    ModelCapability::StructuredOutput,
                ]),
            },
        )]),
        skills: BTreeMap::new(),
        tools: tool_ids
            .iter()
            .cloned()
            .map(|id| {
                (
                    id.clone(),
                    ToolCatalogEntry {
                        id,
                        effect: EffectClass::WorkspaceMutation,
                        permission: PermissionClass::Ask,
                    },
                )
            })
            .collect(),
        evaluators: BTreeMap::new(),
        capabilities: BTreeMap::new(),
    };
    let policy = PolicyEnvelope::allow_catalog(limits.clone(), &catalog);
    let blueprint = build_template(
        TemplateId::Direct,
        &TemplateParameters {
            name: BlueprintName::from("github-direct"),
            primary_agent: agent_id,
            collaborators: Vec::new(),
            skills: BTreeSet::new(),
            tools: tool_ids,
            evaluator: None,
            checkpoint: None,
            limits,
            output: OutputContract {
                format: OutputFormat::Text,
                include_usage: false,
                include_artifact_refs: false,
            },
        },
    )?;
    let blueprint = ValidatedBlueprint::new(blueprint, &catalog, &policy)?;
    let model = ProviderModelBackend::new(
        provider,
        request,
        definitions
            .into_iter()
            .map(|definition| (ToolId::new(definition.name.clone()), definition)),
    );
    let outcome = SchedulerEngine::new(
        &model,
        &tool_backend,
        &DirectRunHost,
        &DirectRunHost,
        &catalog,
        &policy,
        "Operate inside AgenDao's governed harness and use only declared tools.",
    )
    .run(
        &blueprint,
        RunRequest {
            handoff: HandoffPacket {
                goal: prompt.to_string(),
                ..HandoffPacket::default()
            },
            conversation_seed: Vec::new(),
            workspace_root: directory.clone(),
            workspace_summary: directory,
        },
        CancellationFlag::default(),
    )
    .await?;
    let text = outcome.result.output.unwrap_or(outcome.result.summary);
    let text = text.trim();
    Ok(if text.is_empty() {
        "(No response generated)".to_string()
    } else {
        text.to_string()
    })
}

fn resolve_provider(
    agent: &AgentInfo,
    providers: &ProviderRegistry,
) -> anyhow::Result<(Arc<dyn Provider>, String)> {
    if let Some(model) = &agent.model {
        let provider = providers
            .get(&model.provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' is not configured", model.provider_id))?;
        return Ok((provider, model.model_id.clone()));
    }
    let provider = providers
        .list()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No providers configured for GitHub run."))?;
    let model_id = provider
        .models()
        .first()
        .map(|model| model.id.clone())
        .ok_or_else(|| anyhow::anyhow!("Provider '{}' has no models", provider.id()))?;
    Ok((provider, model_id))
}

fn compiled_request(agent: &AgentInfo, model_id: String) -> CompiledExecutionRequest {
    CompiledExecutionRequest {
        model_id,
        max_tokens: agent.max_tokens,
        temperature: agent.temperature,
        top_p: agent.top_p,
        variant: agent.variant.clone(),
        provider_options: (!agent.options.is_empty()).then(|| agent.options.clone()),
        ..CompiledExecutionRequest::default()
    }
}

fn execution_limits(agent: &AgentInfo) -> ExecutionLimits {
    let max_steps = agent.max_steps.unwrap_or(16);
    ExecutionLimits {
        max_model_calls: max_steps,
        max_tool_calls: max_steps.saturating_mul(8),
        max_total_tokens: agent
            .max_tokens
            .unwrap_or(8_192)
            .saturating_mul(max_steps as u64),
        max_wall_time_ms: 1_800_000,
        max_parallelism: 1,
        max_graph_nodes: 2,
        max_graph_depth: 2,
        max_loop_iterations: 1,
        max_agent_steps: max_steps,
    }
}
