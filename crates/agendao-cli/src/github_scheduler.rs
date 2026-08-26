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
    // Cancellation lifecycle: a CI run must honor SIGINT instead of being
    // SIGKILLed at job teardown. The flag races the engine run; the ctrl-c
    // listener is aborted as soon as the run settles either way.
    let cancellation = CancellationFlag::default();
    let interrupt_flag = cancellation.clone();
    let interrupt_listener = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupt_flag.cancel();
        }
    });
    let outcome = run_github_prompt_with_cancellation(
        prompt,
        agent,
        providers,
        tools,
        runtime_config,
        directory,
        cancellation,
    )
    .await;
    interrupt_listener.abort();
    outcome
}

/// Headless composition core with an injectable cancellation flag — the
/// test seam that proves SIGINT semantics (cancel promptly, stay quiet)
/// without sending a real signal to the test process. Production callers
/// reach it through `run_github_prompt`, which owns the SIGINT bridge.
async fn run_github_prompt_with_cancellation(
    prompt: &str,
    agent: AgentInfo,
    providers: Arc<ProviderRegistry>,
    tools: Arc<ToolRegistry>,
    runtime_config: ToolRuntimeConfig,
    directory: String,
    cancellation: CancellationFlag,
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
                max_steps: limits.max_agent_steps,
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
            primary_agent: agent_id.clone(),
            planning_agent: None,
            collaborators: Vec::new(),
            agent_skills: BTreeMap::new(),
            agent_tools: BTreeMap::from([(agent_id.clone(), tool_ids)]),
            agent_max_steps: BTreeMap::from([(agent_id.clone(), limits.max_agent_steps)]),
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
    let model = ProviderModelBackend::from_definitions(provider, request, definitions);
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
        cancellation,
    )
    .await;
    let outcome = outcome?;
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

/// Headless isolation contract tests（docs/execution-authorities.md 第六节）。
///
/// github path 与 session path 共享内核（SchedulerEngine/AgentLoop）、
/// model backend 折叠点（`ProviderModelBackend::from_definitions`）与取消
/// 语义；它特化的是 composition 面（单 agent、无 evaluator/observer、无
/// 事件投影）。这些测试 pin 住特化的边界，使"headless 特化"成为被证明的
/// 产品契约而不是实现巧合：
/// 1. 成功路径返回脚本化文本（经真实 catalog/policy/blueprint 组装）；
/// 2. provider 失败沿 Err 完整传播（与 session 路径同一错误分类来源）；
/// 3. 取消 flag 触发后 promptly 返回归类为取消的 Err，且 provider 静止
///    （SIGINT 语义的注入式证明，与 scheduler 契约测试同一硬断言）；
/// 4. agent allowlist 是工具隔离的双重执行点：definitions 过滤 +
///    execute 拒绝。
#[cfg(test)]
mod github_scheduler_tests {
    use super::*;
    use agendao_agent::{AgentMode, ModelRef};
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // ── scripted provider（CLI 本地窄口径版）──────────────────────────
    // resolve_provider 经 agent.model 的 ModelRef 直查 registry，不走
    // get_model，因此无需 ModelInfo。

    enum ScriptedTurn {
        Events(Vec<agendao_provider::StreamEvent>),
        Fail(agendao_provider::ProviderError),
    }

    struct ScriptedProvider {
        calls: AtomicUsize,
        hang: AtomicBool,
        script: std::sync::Mutex<VecDeque<ScriptedTurn>>,
    }

    impl ScriptedProvider {
        fn text_turns(text: &str) -> VecDeque<ScriptedTurn> {
            VecDeque::from([ScriptedTurn::Events(vec![
                agendao_provider::StreamEvent::Start,
                agendao_provider::StreamEvent::TextStart,
                agendao_provider::StreamEvent::TextDelta(text.to_string()),
                agendao_provider::StreamEvent::TextEnd,
                agendao_provider::StreamEvent::FinishStep {
                    finish_reason: Some("stop".to_string()),
                    usage: agendao_provider::StreamUsage::default(),
                    provider_metadata: None,
                },
            ])])
        }

        fn new(turns: VecDeque<ScriptedTurn>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                hang: AtomicBool::new(false),
                script: std::sync::Mutex::new(turns),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }

        fn hang(&self) {
            self.hang.store(true, Ordering::Relaxed);
        }
    }

    #[async_trait::async_trait]
    impl Provider for ScriptedProvider {
        fn id(&self) -> &str {
            "scripted"
        }

        fn name(&self) -> &str {
            "Scripted"
        }

        fn provider_profile_fingerprint(
            &self,
        ) -> Option<agendao_provider::ProviderProfileFingerprint> {
            None
        }

        fn models(&self) -> Vec<agendao_provider::ModelInfo> {
            Vec::new()
        }

        fn get_model(&self, _id: &str) -> Option<&agendao_provider::ModelInfo> {
            None
        }

        async fn chat(
            &self,
            _request: agendao_provider::ChatRequest,
        ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
            Err(agendao_provider::ProviderError::InvalidRequest(
                "scripted provider only supports chat_stream".to_string(),
            ))
        }

        async fn chat_stream(
            &self,
            _request: agendao_provider::ChatRequest,
        ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.hang.load(Ordering::Relaxed) {
                return Ok(Box::pin(futures::stream::pending()));
            }
            let next = self.script.lock().expect("script lock").pop_front();
            match next {
                Some(ScriptedTurn::Events(events)) => {
                    Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
                }
                Some(ScriptedTurn::Fail(error)) => Err(error),
                None => Ok(Box::pin(futures::stream::iter(vec![Ok(
                    agendao_provider::StreamEvent::FinishStep {
                        finish_reason: Some("stop".to_string()),
                        usage: agendao_provider::StreamUsage::default(),
                        provider_metadata: None,
                    },
                )]))),
            }
        }
    }

    // ── stub tool（allowlist 隔离测试）────────────────────────────────

    struct NamedStubTool(String);

    #[async_trait::async_trait]
    impl agendao_tool::Tool for NamedStubTool {
        fn id(&self) -> &str {
            &self.0
        }

        fn description(&self) -> &str {
            "contract stub tool"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: agendao_tool::ToolContext,
        ) -> Result<agendao_tool::ToolResult, agendao_tool::ToolError> {
            Ok(agendao_tool::ToolResult::simple("stub", "stub executed"))
        }
    }

    // ── 装配 helpers ──────────────────────────────────────────────────

    /// 测试 fixture 落在 CARGO_TARGET_DIR 之下（与 server 侧
    /// test_support::target_fixture_root 同一约束：不留仓库内或 /tmp 产物）。
    fn target_fixture(test: &str) -> String {
        let configured = std::env::var("CARGO_TARGET_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                panic!("CARGO_TARGET_DIR is not set; run tests with CARGO_TARGET_DIR=../target")
            });
        let fixture = std::path::Path::new(&configured)
            .join("agendao-cli-unit-tests")
            .join(test)
            .join(format!("{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&fixture)
            .unwrap_or_else(|error| panic!("create fixture {}: {error}", fixture.display()));
        fixture.to_string_lossy().into_owned()
    }

    fn probe_agent(allowed_tools: &[&str]) -> AgentInfo {
        AgentInfo {
            name: "github-probe".to_string(),
            description: None,
            mode: AgentMode::Primary,
            model: Some(ModelRef {
                model_id: "probe-1".to_string(),
                provider_id: "scripted".to_string(),
            }),
            model_preference: None,
            system_prompt: Some("You are a headless contract probe.".to_string()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_steps: None,
            allowed_tools: allowed_tools
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            options: HashMap::new(),
            permission: agendao_agent::AgentInfo::from_builtin(
                agendao_agent::BuiltinAgent::General,
            )
            .permission,
            hidden: false,
            native: false,
            variant: None,
            color: None,
        }
    }

    fn registry_with(provider: Arc<ScriptedProvider>) -> Arc<ProviderRegistry> {
        let mut providers = ProviderRegistry::new();
        providers.register_arc(provider);
        Arc::new(providers)
    }

    fn empty_tool_registry() -> Arc<ToolRegistry> {
        Arc::new(ToolRegistry::new())
    }

    // ── 契约 1：成功路径 ──────────────────────────────────────────────

    #[tokio::test]
    async fn github_run_completes_and_returns_scripted_text() {
        let provider = ScriptedProvider::new(ScriptedProvider::text_turns("github contract: done"));
        let outcome = run_github_prompt_with_cancellation(
            "contract: finish the assigned work",
            probe_agent(&[]),
            registry_with(provider.clone()),
            empty_tool_registry(),
            ToolRuntimeConfig::default(),
            target_fixture("github-success"),
            CancellationFlag::default(),
        )
        .await
        .expect("scripted headless run must succeed");
        assert_eq!(outcome, "github contract: done");
        assert_eq!(provider.calls(), 1);
    }

    // ── 契约 2：provider 失败沿 Err 完整传播 ──────────────────────────

    #[tokio::test]
    async fn github_run_propagates_provider_failure() {
        let provider = ScriptedProvider::new(VecDeque::from([ScriptedTurn::Fail(
            agendao_provider::ProviderError::InvalidRequest("github: provider refused".to_string()),
        )]));
        let error = run_github_prompt_with_cancellation(
            "contract: finish the assigned work",
            probe_agent(&[]),
            registry_with(provider),
            empty_tool_registry(),
            ToolRuntimeConfig::default(),
            target_fixture("github-failure"),
            CancellationFlag::default(),
        )
        .await
        .expect_err("provider failure must surface as Err");
        assert!(
            error.to_string().contains("github: provider refused"),
            "error classification must preserve the provider failure cause, got: {error}"
        );
    }

    // ── 契约 3：取消 promptly 返回且 provider 静止 ─────────────────────

    #[tokio::test]
    async fn github_run_cancellation_returns_promptly_and_stays_quiet() {
        let provider = ScriptedProvider::new(ScriptedProvider::text_turns("never finishing"));
        provider.hang();
        let cancellation = CancellationFlag::default();

        let run = tokio::spawn(run_github_prompt_with_cancellation(
            "contract: finish the assigned work",
            probe_agent(&[]),
            registry_with(provider.clone()),
            empty_tool_registry(),
            ToolRuntimeConfig::default(),
            target_fixture("github-cancel"),
            cancellation.clone(),
        ));

        // 等模型调用真正 in-flight，再取消 —— 取消必须作用于运行中执行体。
        let mut in_flight = false;
        for _ in 0..250 {
            if provider.calls() >= 1 {
                in_flight = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(in_flight, "scripted model call must become in-flight");

        cancellation.cancel();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(10), run)
            .await
            .expect("cancelled headless run must return promptly")
            .expect("run task must join cleanly after cancellation");

        let error = joined.expect_err("cancelled run must surface as Err");
        assert!(
            error.to_string().to_lowercase().contains("cancel"),
            "error must be classified as cancellation, got: {error}"
        );

        // 取消静止窗口：headless 路径与 session 路径同一取消契约。
        let calls_at_cancel = provider.calls();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            provider.calls(),
            calls_at_cancel,
            "provider must stay quiet after cancellation returns"
        );
    }

    // ── 契约 4：agent allowlist 是工具隔离的双重执行点 ─────────────────

    #[tokio::test]
    async fn github_tool_surface_is_bounded_by_agent_allowlist() {
        let tools = ToolRegistry::new();
        tools
            .register(NamedStubTool("probe-tool".to_string()))
            .await;
        tools
            .register(NamedStubTool("secret-tool".to_string()))
            .await;
        let tools = Arc::new(tools);

        let backend = GithubToolBackend::new(
            tools.clone(),
            probe_agent(&["probe-tool"]),
            target_fixture("github-allowlist"),
            ToolRuntimeConfig::default(),
        )
        .await;

        // 执行点一：definitions 只暴露 allowlist 内的工具。
        let definitions = backend.definitions().await;
        let names: Vec<&str> = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert_eq!(names, vec!["probe-tool"]);

        // 执行点二：即便越权直呼 execute，allowlist 也拒绝（headless 无
        // permission 弹面，拒绝是唯一屏障）。
        let agent_id = AgentId::from("github-probe");
        let observation = AgentObservationContext {
            node_path: "direct",
            agent: &agent_id,
            step: 1,
            max_steps: 16,
        };
        let denied = backend
            .execute(
                &observation,
                &ToolCall {
                    id: "call-1".to_string(),
                    tool: ToolId::from("secret-tool"),
                    arguments: serde_json::json!({}),
                },
            )
            .await
            .expect("denied tool still returns a structured ToolExecution");
        assert!(denied.is_error, "out-of-allowlist call must be denied");
        assert!(
            denied.output.contains("secret-tool"),
            "denial must name the rejected tool, got: {}",
            denied.output
        );
    }
}
