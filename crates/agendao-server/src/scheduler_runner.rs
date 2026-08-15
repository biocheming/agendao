use agendao_agent::{AgentInfo, AgentRegistry};
use agendao_execution_types::CompiledExecutionRequest;
use agendao_orchestrator::agent_loop::{CancellationFlag, ModelRoute, ProviderModelBackend};
use agendao_orchestrator::blueprint::{
    AgentId, BlueprintName, CapabilityId, EvaluatorId, ExecutionLimits, ModelCapability,
    OutputContract, OutputFormat, SchedulerBlueprint, SkillId, ToolId, ValidatedBlueprint,
};
use agendao_orchestrator::catalog::{
    AgentCatalogEntry, CapabilityCatalogEntry, CapabilityKind, EffectClass, EvaluatorCatalogEntry,
    EvaluatorKind, PermissionClass, SchedulerCatalog, SkillCatalogEntry, ToolCatalogEntry,
};
use agendao_orchestrator::context::{HandoffPacket, NodeResult, Usage};
use agendao_orchestrator::engine::{RunRequest, SchedulerEngine};
use agendao_orchestrator::events::{EventSink, ExecutionEvent};
use agendao_orchestrator::policy::PolicyEnvelope;
use agendao_orchestrator::selector::{
    AutoSelector, ExplicitSelection, SchedulerChoice, SelectionRequest, SelectionSource, TaskShape,
};
use agendao_orchestrator::templates::TemplateParameters;
use agendao_provider::{Message, Provider, ToolDefinition};
use agendao_skill::{infer_toolsets_from_tools, SkillConditions, SkillRuntimeResolver};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

use crate::routes::{
    scheduler_host_tool_definitions, SessionSchedulerToolExecutor,
    SessionSchedulerToolExecutorInput,
};
use crate::scheduler_backends::{ModelEvaluatorBackend, ModelPlannerBackend};
use crate::scheduler_cache::BoundedLruCache;
use crate::scheduler_capabilities::WorkspaceCapabilityHost;
use crate::ServerState;

pub(crate) const BLUEPRINT_LOCK_METADATA_KEY: &str = "scheduler_blueprint";
pub(crate) const BLUEPRINT_FINGERPRINT_METADATA_KEY: &str = "scheduler_blueprint_fingerprint";
pub(crate) const SELECTION_SOURCE_METADATA_KEY: &str = "scheduler_selection_source";
pub(crate) const REJECTED_BLUEPRINTS_METADATA_KEY: &str =
    "scheduler_rejected_blueprint_fingerprints";

const MAX_HYDRATED_SKILL_CACHE_ENTRIES: usize = 64;
const MAX_HYDRATED_SKILL_CACHE_BYTES: usize = 4 * 1024 * 1024;

type HydratedSkillCache = BoundedLruCache<String, (Arc<str>, String)>;

static HYDRATED_SKILL_CACHE: OnceLock<Mutex<HydratedSkillCache>> = OnceLock::new();
static HYDRATED_SKILL_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static HYDRATED_SKILL_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static HYDRATED_SKILL_CACHE_EVICTIONS: AtomicU64 = AtomicU64::new(0);

pub struct SchedulerRunInput {
    pub state: Arc<ServerState>,
    pub session_id: String,
    pub assistant_message_id: String,
    pub directory: String,
    pub goal: String,
    pub choice: SchedulerChoice,
    pub provider: Arc<dyn Provider>,
    pub request: CompiledExecutionRequest,
    pub conversation_seed: Vec<Message>,
    pub execution_metadata: HashMap<String, serde_json::Value>,
    pub cancellation: CancellationToken,
}

pub struct SchedulerRunOutput {
    pub result: NodeResult,
    pub usage: Usage,
    pub blueprint: SchedulerBlueprint,
    pub fingerprint: String,
    pub source: SelectionSource,
}

struct SchedulerEventChannel(tokio::sync::mpsc::UnboundedSender<ExecutionEvent>);

impl EventSink for SchedulerEventChannel {
    fn emit(&self, event: ExecutionEvent) {
        let _ = self.0.send(event);
    }
}

pub async fn run_scheduler(input: SchedulerRunInput) -> Result<SchedulerRunOutput, String> {
    let config = input.state.config_store.config();
    let agents = Arc::new(AgentRegistry::from_config(&config));
    let tool_definitions = scheduler_tool_definitions(&input.state).await;
    let mut catalog = build_catalog(&input.state, &agents, &tool_definitions)?;
    let limits = execution_limits(&input.request);
    let policy = PolicyEnvelope::allow_catalog(limits.clone(), &catalog);
    let primary = primary_agent(&agents)?;
    let parameters = template_parameters(&catalog, &agents, primary, limits);
    let workspace_summary = workspace_summary(&input.state, &input.directory).await?;
    let rejected_blueprint_fingerprints =
        load_rejected_blueprints(&input.state, &input.session_id).await?;
    let locked = if matches!(input.choice, SchedulerChoice::Auto) {
        load_locked_blueprint(&input.state, &input.session_id, &catalog, &policy).await?
    } else {
        None
    };
    let explicit = match input.choice.clone() {
        SchedulerChoice::Auto => None,
        SchedulerChoice::Template { template } => Some(ExplicitSelection::Template {
            id: template,
            parameters: parameters.clone(),
        }),
        SchedulerChoice::Blueprint { blueprint } => Some(ExplicitSelection::Blueprint(blueprint)),
    };
    let planner = ModelPlannerBackend::new(input.provider.clone(), input.request.clone());
    let selection = AutoSelector::new(&planner, &catalog, &policy)
        .select(SelectionRequest {
            explicit,
            locked,
            task: classify_task(&input.goal),
            default_parameters: parameters,
            goal: input.goal.clone(),
            workspace_summary: workspace_summary.clone(),
            rejected_blueprint_fingerprints,
        })
        .await
        .map_err(|error| error.to_string())?;

    hydrate_selected_skills(&input.state, selection.blueprint.blueprint(), &mut catalog)?;
    let blueprint =
        ValidatedBlueprint::new(selection.blueprint.blueprint().clone(), &catalog, &policy)
            .map_err(|error| error.to_string())?;
    persist_blueprint_lock(
        &input.state,
        &input.session_id,
        &blueprint,
        selection.source,
    )
    .await?;

    let model = ProviderModelBackend::new(
        input.provider.clone(),
        input.request.clone(),
        tool_definitions
            .iter()
            .cloned()
            .map(|definition| (ToolId::new(definition.name.clone()), definition)),
    )
    .with_routes(model_routes(&input.state, &agents, &input.request).await?);
    let tool_backend = SessionSchedulerToolExecutor::new(
        input.state.clone(),
        SessionSchedulerToolExecutorInput {
            session_id: input.session_id.clone(),
            message_id: input.assistant_message_id,
            directory: input.directory.clone(),
            abort_token: input.cancellation.clone(),
            tool_runtime_config: agendao_tool::ToolRuntimeConfig::from_config(&config),
            execution_metadata: input.execution_metadata,
        },
    );
    let evaluator = ModelEvaluatorBackend::new(
        input.provider,
        input.request,
        BTreeMap::from([(
            EvaluatorId::from("quality"),
            "Judge whether the candidate fully satisfies the original goal and its constraints."
                .to_string(),
        )]),
    );
    let capabilities = WorkspaceCapabilityHost::new(input.directory.clone().into())?;
    let cancellation = CancellationFlag::default();
    let cancellation_signal = cancellation.clone();
    let cancellation_token = input.cancellation;
    let cancellation_task = tokio::spawn(async move {
        cancellation_token.cancelled().await;
        cancellation_signal.cancel();
    });
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_sink = SchedulerEventChannel(event_tx);
    let projection_task = tokio::spawn(project_scheduler_events(
        input.state.clone(),
        input.session_id.clone(),
        event_rx,
    ));
    let execution = SchedulerEngine::new(
        &model,
        &tool_backend,
        &evaluator,
        &capabilities,
        &catalog,
        &policy,
        "You are operating inside AgenDao's governed harness. Follow the selected agent policy, use only declared tools and skills, respect workspace authority, and return concise evidence-backed results.",
    )
    .with_events(&event_sink)
    .run(
        &blueprint,
        RunRequest {
            handoff: HandoffPacket {
                goal: input.goal,
                ..HandoffPacket::default()
            },
            conversation_seed: input.conversation_seed,
            workspace_root: input.directory.clone(),
            workspace_summary,
        },
        cancellation,
    )
    .await;
    drop(event_sink);
    projection_task
        .await
        .map_err(|error| format!("scheduler event projection failed: {error}"))?;
    cancellation_task.abort();
    let outcome = execution.map_err(|error| error.to_string())?;
    let usage = outcome.usage;
    Ok(SchedulerRunOutput {
        result: outcome.result,
        usage,
        blueprint: blueprint.blueprint().clone(),
        fingerprint: blueprint.fingerprint().to_string(),
        source: selection.source,
    })
}

async fn load_rejected_blueprints(
    state: &ServerState,
    session_id: &str,
) -> Result<BTreeSet<String>, String> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| format!("session '{session_id}' not found"))?;
    let Some(value) = session
        .record()
        .metadata
        .get(REJECTED_BLUEPRINTS_METADATA_KEY)
    else {
        return Ok(BTreeSet::new());
    };
    serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid rejected Blueprint metadata: {error}"))
}

pub(crate) async fn validate_user_blueprint(
    state: &ServerState,
    blueprint: SchedulerBlueprint,
) -> Result<ValidatedBlueprint, String> {
    let config = state.config_store.config();
    let agents = AgentRegistry::from_config(&config);
    let tools = scheduler_tool_definitions(state).await;
    let mut catalog = build_catalog(state, &agents, &tools)?;
    let policy = PolicyEnvelope::allow_catalog(
        execution_limits(&CompiledExecutionRequest::default()),
        &catalog,
    );
    hydrate_selected_skills(state, &blueprint, &mut catalog)?;
    ValidatedBlueprint::new(blueprint, &catalog, &policy).map_err(|error| error.to_string())
}

async fn project_scheduler_events(
    state: Arc<ServerState>,
    session_id: String,
    mut events: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
) {
    use agendao_server_core::runtime_control::{ExecutionPatch, FieldUpdate};

    let mut node_metadata = BTreeMap::<String, serde_json::Map<String, serde_json::Value>>::new();
    while let Some(event) = events.recv().await {
        match event {
            ExecutionEvent::RunStarted => {
                state
                    .runtime_telemetry
                    .update_scheduler_run(
                        &session_id,
                        ExecutionPatch {
                            recent_event: FieldUpdate::Set("Scheduler run started".to_string()),
                            waiting_on: FieldUpdate::Set("model".to_string()),
                            ..ExecutionPatch::default()
                        },
                    )
                    .await;
            }
            ExecutionEvent::NodeStarted { path } => {
                node_metadata
                    .entry(path.clone())
                    .or_default()
                    .insert("path".to_string(), serde_json::json!(&path));
                state
                    .runtime_telemetry
                    .register_scheduler_node(&session_id, &path)
                    .await;
            }
            ExecutionEvent::NodeCompleted { path } => {
                state
                    .runtime_telemetry
                    .finish_scheduler_node(&session_id, &path)
                    .await;
            }
            ExecutionEvent::LoopIteration { path, iteration } => {
                let metadata = node_metadata.entry(path.clone()).or_default();
                metadata.insert("path".to_string(), serde_json::json!(&path));
                metadata.insert("iteration".to_string(), serde_json::json!(iteration));
                state
                    .runtime_telemetry
                    .update_scheduler_node(
                        &session_id,
                        &path,
                        ExecutionPatch {
                            recent_event: FieldUpdate::Set(format!("Loop iteration {iteration}")),
                            metadata: FieldUpdate::Set(serde_json::Value::Object(metadata.clone())),
                            ..ExecutionPatch::default()
                        },
                    )
                    .await;
            }
            ExecutionEvent::Evaluated { path, outcome } => {
                let outcome = match outcome {
                    agendao_orchestrator::engine::Evaluation::Pass => "pass",
                    agendao_orchestrator::engine::Evaluation::Fail => "fail",
                    agendao_orchestrator::engine::Evaluation::Indeterminate => "indeterminate",
                };
                let metadata = node_metadata.entry(path.clone()).or_default();
                metadata.insert("path".to_string(), serde_json::json!(&path));
                metadata.insert("evaluation".to_string(), serde_json::json!(outcome));
                state
                    .runtime_telemetry
                    .update_scheduler_node(
                        &session_id,
                        &path,
                        ExecutionPatch {
                            recent_event: FieldUpdate::Set(format!("Evaluation: {outcome}")),
                            metadata: FieldUpdate::Set(serde_json::Value::Object(metadata.clone())),
                            ..ExecutionPatch::default()
                        },
                    )
                    .await;
            }
            ExecutionEvent::RunCompleted { usage } => {
                state
                    .runtime_telemetry
                    .update_scheduler_run(
                        &session_id,
                        ExecutionPatch {
                            waiting_on: FieldUpdate::Clear,
                            recent_event: FieldUpdate::Set("Scheduler run completed".to_string()),
                            metadata: FieldUpdate::Set(serde_json::json!({
                                "usage": {
                                    "model_calls": usage.model_calls,
                                    "tool_calls": usage.tool_calls,
                                    "input_tokens": usage.input_tokens,
                                    "output_tokens": usage.output_tokens,
                                    "reasoning_tokens": usage.reasoning_tokens,
                                    "cache_read_tokens": usage.cache_read_tokens,
                                    "cache_miss_tokens": usage.cache_miss_tokens,
                                    "cache_write_tokens": usage.cache_write_tokens,
                                }
                            })),
                            ..ExecutionPatch::default()
                        },
                    )
                    .await;
            }
            ExecutionEvent::RunFailed { message } => {
                state
                    .runtime_telemetry
                    .update_scheduler_run(
                        &session_id,
                        ExecutionPatch {
                            waiting_on: FieldUpdate::Clear,
                            recent_event: FieldUpdate::Set(format!(
                                "Scheduler run failed: {message}"
                            )),
                            metadata: FieldUpdate::Set(serde_json::json!({ "error": message })),
                            ..ExecutionPatch::default()
                        },
                    )
                    .await;
            }
        }
    }
}

async fn scheduler_tool_definitions(state: &ServerState) -> Vec<ToolDefinition> {
    let mut definitions = state
        .tool_registry
        .list_schemas()
        .await
        .into_iter()
        .map(|schema| ToolDefinition {
            name: schema.name,
            description: Some(schema.description),
            parameters: schema.parameters,
        })
        .collect::<Vec<_>>();
    definitions.extend(scheduler_host_tool_definitions());
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    definitions
}

fn build_catalog(
    state: &ServerState,
    agents: &AgentRegistry,
    tools: &[ToolDefinition],
) -> Result<SchedulerCatalog, String> {
    let tool_entries = tools
        .iter()
        .map(|tool| {
            let id = ToolId::new(tool.name.clone());
            (
                id.clone(),
                ToolCatalogEntry {
                    id,
                    effect: tool_effect(&tool.name),
                    permission: PermissionClass::Ask,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let skill_resolver =
        SkillRuntimeResolver::new(state.project_root(), Some(state.config_store.clone()));
    let skill_entries = skill_resolver
        .list_skill_catalog(None)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|skill| {
            let id = SkillId::new(skill.name);
            let fingerprint = format!(
                "meta:{:x}",
                Sha256::digest(format!(
                    "{}\0{}",
                    skill.description,
                    skill.location.display()
                ))
            );
            (
                id.clone(),
                SkillCatalogEntry {
                    id,
                    summary: skill.description,
                    content_fingerprint: fingerprint,
                    capability_tags: skill.category.into_iter().collect(),
                    hydrated_prompt: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let all_skills = skill_entries.keys().cloned().collect::<BTreeSet<_>>();
    let agent_entries = agents
        .list_all()
        .into_iter()
        .filter(|agent| !agent.hidden)
        .map(|agent| {
            let id = AgentId::new(agent.name.clone());
            let available_tools = tool_entries
                .keys()
                .filter(|tool| agent.is_tool_allowed(tool.as_str()))
                .cloned()
                .collect();
            (
                id.clone(),
                AgentCatalogEntry {
                    id,
                    system_policy: agent.resolved_system_prompt().unwrap_or_default(),
                    available_skills: all_skills.clone(),
                    available_tools,
                    model_capabilities: BTreeSet::from([
                        ModelCapability::ToolCalls,
                        ModelCapability::Reasoning,
                        ModelCapability::Attachments,
                        ModelCapability::StructuredOutput,
                    ]),
                },
            )
        })
        .collect();
    Ok(SchedulerCatalog {
        revision: "scheduler-catalog-v1".to_string(),
        agents: agent_entries,
        skills: skill_entries,
        tools: tool_entries,
        evaluators: BTreeMap::from([(
            EvaluatorId::from("quality"),
            EvaluatorCatalogEntry {
                id: EvaluatorId::from("quality"),
                kind: EvaluatorKind::ModelJudge,
            },
        )]),
        capabilities: BTreeMap::from([(
            CapabilityId::from("workspace-checkpoint"),
            CapabilityCatalogEntry {
                id: CapabilityId::from("workspace-checkpoint"),
                kind: CapabilityKind::WorkspaceCheckpoint,
                effect: EffectClass::WorkspaceMutation,
            },
        )]),
    })
}

fn execution_limits(request: &CompiledExecutionRequest) -> ExecutionLimits {
    ExecutionLimits {
        max_model_calls: 32,
        max_tool_calls: 96,
        max_total_tokens: request.max_tokens_or(8_192).saturating_mul(32),
        max_wall_time_ms: request.timeout_secs.unwrap_or(1_800).saturating_mul(1_000),
        max_parallelism: 4,
        max_graph_nodes: 48,
        max_graph_depth: 16,
        max_loop_iterations: 6,
        max_agent_steps: 16,
    }
}

fn primary_agent(agents: &AgentRegistry) -> Result<AgentId, String> {
    agents
        .get("build")
        .or_else(|| agents.list_primary().first().copied())
        .map(|agent| AgentId::new(agent.name.clone()))
        .ok_or_else(|| "scheduler catalog has no primary agent".to_string())
}

fn template_parameters(
    catalog: &SchedulerCatalog,
    agents: &AgentRegistry,
    primary_agent: AgentId,
    limits: ExecutionLimits,
) -> TemplateParameters {
    let collaborators = agents
        .list_subagents()
        .into_iter()
        .take(3)
        .map(|agent| AgentId::new(agent.name.clone()))
        .collect();
    let tools = catalog
        .agents
        .get(&primary_agent)
        .map(|agent| agent.available_tools.clone())
        .unwrap_or_default();
    TemplateParameters {
        name: BlueprintName::from("session-scheduler"),
        primary_agent,
        collaborators,
        skills: BTreeSet::new(),
        tools,
        evaluator: Some(EvaluatorId::from("quality")),
        checkpoint: Some(CapabilityId::from("workspace-checkpoint")),
        limits,
        output: OutputContract {
            format: OutputFormat::Markdown,
            include_usage: true,
            include_artifact_refs: true,
        },
    }
}

fn classify_task(goal: &str) -> TaskShape {
    let normalized = goal.to_ascii_lowercase();
    TaskShape {
        simple: goal.chars().count() <= 160
            && !normalized.contains("audit")
            && !normalized.contains("research")
            && !normalized.contains("verify"),
        iterative_research: normalized.contains("autoresearch")
            || normalized.contains("iterative research"),
        requires_verification: normalized.contains("verify")
            || normalized.contains("验证")
            || normalized.contains("审计"),
        benefits_from_parallelism: normalized.contains("parallel")
            || normalized.contains("compare")
            || normalized.contains("全面"),
    }
}

fn hydrate_selected_skills(
    state: &ServerState,
    blueprint: &SchedulerBlueprint,
    catalog: &mut SchedulerCatalog,
) -> Result<(), String> {
    let selected_surfaces = selected_skill_tool_surfaces(blueprint);
    if selected_surfaces.is_empty() {
        return Ok(());
    }
    let resolver =
        SkillRuntimeResolver::new(state.project_root(), Some(state.config_store.clone()));
    let selected_names = selected_surfaces
        .keys()
        .map(|id| id.as_str().to_string())
        .collect::<Vec<_>>();
    for (id, tool_surfaces) in selected_surfaces {
        let (prompt, fingerprint) =
            hydrate_skill_prompt(state, &resolver, &id, &selected_names, &tool_surfaces)?;
        let entry = catalog
            .skills
            .get_mut(&id)
            .ok_or_else(|| format!("selected skill '{}' is unavailable", id.as_str()))?;
        entry.content_fingerprint = fingerprint;
        entry.hydrated_prompt = Some(prompt);
    }
    Ok(())
}

fn hydrate_skill_prompt(
    state: &ServerState,
    resolver: &SkillRuntimeResolver,
    id: &SkillId,
    selected_names: &[String],
    tool_surfaces: &BTreeSet<BTreeSet<ToolId>>,
) -> Result<(Arc<str>, String), String> {
    let loaded = resolver
        .load_skill(id.as_str(), None)
        .map_err(|error| error.to_string())?;
    let detail = resolver
        .load_skill_detail(id.as_str(), None)
        .map_err(|error| error.to_string())?;
    if detail.setup_needed {
        return Err(format!(
            "selected skill '{}' is not runtime-ready",
            id.as_str()
        ));
    }
    for tools in tool_surfaces {
        validate_skill_tool_surface(id, &loaded.meta.conditions, tools)?;
    }

    let source_fingerprint = format!("sha256:{:x}", Sha256::digest(loaded.content.as_bytes()));
    let key_material = serde_json::to_vec(&serde_json::json!({
        "workspace": state.project_root(),
        "config_revision": state.config_store.revision(),
        "skill": id,
        "selected": selected_names,
        "tool_surfaces": tool_surfaces,
        "content": source_fingerprint,
    }))
    .map_err(|error| error.to_string())?;
    let key = format!("{:x}", Sha256::digest(key_material));
    let cache = HYDRATED_SKILL_CACHE.get_or_init(|| {
        Mutex::new(HydratedSkillCache::new(
            MAX_HYDRATED_SKILL_CACHE_ENTRIES,
            MAX_HYDRATED_SKILL_CACHE_BYTES,
        ))
    });
    {
        let mut cache = cache
            .lock()
            .map_err(|_| "hydrated skill cache is poisoned".to_string())?;
        if let Some((prompt, fingerprint)) = cache.get(&key) {
            let hits = HYDRATED_SKILL_CACHE_HITS.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::debug!(
                skill = id.as_str(),
                hits,
                misses = HYDRATED_SKILL_CACHE_MISSES.load(Ordering::Relaxed),
                evictions = HYDRATED_SKILL_CACHE_EVICTIONS.load(Ordering::Relaxed),
                "hydrated skill cache hit"
            );
            return Ok((prompt, fingerprint));
        }
    }

    HYDRATED_SKILL_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let packet =
        resolver.build_prompt_packet_for_loaded_skill(&loaded, &detail, Some(selected_names));
    let prompt: Arc<str> = Arc::from(packet.render_prompt_block(None, None));
    let fingerprint = format!("sha256:{:x}", Sha256::digest(prompt.as_bytes()));
    let cache_bytes = key
        .len()
        .saturating_add(prompt.len())
        .saturating_add(fingerprint.len());
    let evictions = cache
        .lock()
        .map_err(|_| "hydrated skill cache is poisoned".to_string())?
        .insert(key, (prompt.clone(), fingerprint.clone()), cache_bytes);
    HYDRATED_SKILL_CACHE_EVICTIONS.fetch_add(evictions as u64, Ordering::Relaxed);
    tracing::debug!(
        skill = id.as_str(),
        hits = HYDRATED_SKILL_CACHE_HITS.load(Ordering::Relaxed),
        misses = HYDRATED_SKILL_CACHE_MISSES.load(Ordering::Relaxed),
        evictions = HYDRATED_SKILL_CACHE_EVICTIONS.load(Ordering::Relaxed),
        "hydrated skill cache miss"
    );
    Ok((prompt, fingerprint))
}

fn validate_skill_tool_surface(
    id: &SkillId,
    conditions: &SkillConditions,
    tools: &BTreeSet<ToolId>,
) -> Result<(), String> {
    let available_tools = tools
        .iter()
        .map(|tool| tool.as_str().trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let available_toolsets = infer_toolsets_from_tools(available_tools.iter().map(String::as_str));

    for required in &conditions.requires_tools {
        let required = required.trim().to_ascii_lowercase();
        if !available_tools.contains(&required) {
            return Err(format!(
                "selected skill '{}' requires undeclared tool '{}'",
                id.as_str(),
                required
            ));
        }
    }
    for primary in &conditions.fallback_for_tools {
        let primary = primary.trim().to_ascii_lowercase();
        if available_tools.contains(&primary) {
            return Err(format!(
                "selected fallback skill '{}' conflicts with available tool '{}'",
                id.as_str(),
                primary
            ));
        }
    }
    for required in &conditions.requires_toolsets {
        let required = required.trim().to_ascii_lowercase();
        if !available_toolsets.contains(&required) {
            return Err(format!(
                "selected skill '{}' requires unavailable toolset '{}'",
                id.as_str(),
                required
            ));
        }
    }
    for primary in &conditions.fallback_for_toolsets {
        let primary = primary.trim().to_ascii_lowercase();
        if available_toolsets.contains(&primary) {
            return Err(format!(
                "selected fallback skill '{}' conflicts with available toolset '{}'",
                id.as_str(),
                primary
            ));
        }
    }
    Ok(())
}

fn selected_skill_tool_surfaces(
    blueprint: &SchedulerBlueprint,
) -> BTreeMap<SkillId, BTreeSet<BTreeSet<ToolId>>> {
    fn collect(
        nodes: &BTreeMap<
            agendao_orchestrator::blueprint::NodeId,
            agendao_orchestrator::blueprint::NodeSpec,
        >,
        selected: &mut BTreeMap<SkillId, BTreeSet<BTreeSet<ToolId>>>,
    ) {
        for node in nodes.values() {
            match node {
                agendao_orchestrator::blueprint::NodeSpec::Agent(agent) => {
                    for skill in &agent.skills {
                        selected
                            .entry(skill.clone())
                            .or_default()
                            .insert(agent.tools.clone());
                    }
                }
                agendao_orchestrator::blueprint::NodeSpec::Loop(loop_node) => {
                    collect(&loop_node.body.nodes, selected);
                }
                _ => {}
            }
        }
    }

    let mut selected = BTreeMap::new();
    collect(&blueprint.nodes, &mut selected);
    selected
}

async fn workspace_summary(state: &ServerState, directory: &str) -> Result<String, String> {
    let context = state.resolved_context.read().await;
    serde_json::to_string(&serde_json::json!({
        "directory": directory,
        "project_root": state.project_root(),
        "identity": &context.identity,
        "mode": &context.mode,
    }))
    .map_err(|error| format!("workspace summary serialization failed: {error}"))
}

async fn model_routes(
    state: &ServerState,
    agents: &AgentRegistry,
    request_defaults: &CompiledExecutionRequest,
) -> Result<BTreeMap<AgentId, ModelRoute>, String> {
    let providers = state.providers.read().await;
    let mut routes = BTreeMap::new();
    for agent in agents.list_all().into_iter().filter(|agent| !agent.hidden) {
        let Some(model) = agent.model.as_ref() else {
            continue;
        };
        let provider = providers
            .get_provider(&model.provider_id)
            .map_err(|error| error.to_string())?;
        routes.insert(
            AgentId::new(agent.name.clone()),
            ModelRoute {
                provider,
                request: agent_request(request_defaults, agent, &model.model_id),
            },
        );
    }
    Ok(routes)
}

fn agent_request(
    request_defaults: &CompiledExecutionRequest,
    agent: &AgentInfo,
    model_id: &str,
) -> CompiledExecutionRequest {
    let mut request = request_defaults.with_model(model_id);
    request.max_tokens = agent.max_tokens.or(request.max_tokens);
    request.temperature = agent.temperature.or(request.temperature);
    request.top_p = agent.top_p.or(request.top_p);
    request.variant = agent.variant.clone().or(request.variant);
    if !agent.options.is_empty() {
        request.provider_options = Some(agent.options.clone());
    }
    request
}

async fn load_locked_blueprint(
    state: &ServerState,
    session_id: &str,
    catalog: &SchedulerCatalog,
    policy: &PolicyEnvelope,
) -> Result<Option<ValidatedBlueprint>, String> {
    let sessions = state.sessions.lock().await;
    let Some(session) = sessions.get(session_id) else {
        return Ok(None);
    };
    let Some(value) = session.record().metadata.get(BLUEPRINT_LOCK_METADATA_KEY) else {
        return Ok(None);
    };
    let blueprint = serde_json::from_value(value.clone()).map_err(|error| {
        format!("stored scheduler Blueprint is invalid and cannot be reused: {error}")
    })?;
    ValidatedBlueprint::new(blueprint, catalog, policy)
        .map(Some)
        .map_err(|error| format!("stored scheduler Blueprint no longer validates: {error}"))
}

async fn persist_blueprint_lock(
    state: &ServerState,
    session_id: &str,
    blueprint: &ValidatedBlueprint,
    source: SelectionSource,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().await;
    let mut session = sessions
        .get(session_id)
        .cloned()
        .ok_or_else(|| "scheduler session is unavailable".to_string())?;
    session.insert_metadata(
        BLUEPRINT_LOCK_METADATA_KEY,
        serde_json::to_value(blueprint.blueprint()).map_err(|error| error.to_string())?,
    );
    session.insert_metadata(
        BLUEPRINT_FINGERPRINT_METADATA_KEY,
        serde_json::json!(blueprint.fingerprint().to_string()),
    );
    session.insert_metadata(
        SELECTION_SOURCE_METADATA_KEY,
        serde_json::json!(selection_source_name(source)),
    );
    sessions.update(session);
    Ok(())
}

pub(crate) fn selection_source_name(source: SelectionSource) -> &'static str {
    match source {
        SelectionSource::User => "user",
        SelectionSource::SessionLock => "session-lock",
        SelectionSource::Heuristic => "heuristic",
        SelectionSource::Planner => "planner",
    }
}

fn tool_effect(tool: &str) -> EffectClass {
    match tool {
        "read" | "glob" | "grep" | "ast_grep_search" | "lsp_diagnostics" => EffectClass::ReadOnly,
        "bash" | "shell_session" => EffectClass::ProcessExecution,
        "webfetch" | "websearch" | "browser_session" => EffectClass::Network,
        "question" => EffectClass::ExternalMutation,
        _ => EffectClass::WorkspaceMutation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_orchestrator::blueprint::{
        AgentNode, BlueprintSchemaVersion, EndNode, NodeId, NodeSpec,
    };

    #[test]
    fn classification_keeps_simple_work_on_direct_path() {
        assert!(classify_task("Explain this function").simple);
        assert!(classify_task("全面审计并验证整个项目").requires_verification);
        assert!(classify_task("run autoresearch").iterative_research);
    }

    #[test]
    fn skill_conditions_are_revalidated_against_each_node_tool_surface() {
        let skill = SkillId::from("conditional");
        let tools = BTreeSet::from([ToolId::from("read"), ToolId::from("webfetch")]);
        let conditions = SkillConditions {
            requires_tools: vec!["read".to_string()],
            fallback_for_tools: vec!["bash".to_string()],
            requires_toolsets: vec!["search".to_string(), "web".to_string()],
            fallback_for_toolsets: vec!["browser".to_string()],
        };
        validate_skill_tool_surface(&skill, &conditions, &tools).expect("matching surface");

        let mut missing_tool = conditions.clone();
        missing_tool.requires_tools.push("write".to_string());
        assert!(validate_skill_tool_surface(&skill, &missing_tool, &tools)
            .unwrap_err()
            .contains("requires undeclared tool 'write'"));

        let mut primary_present = conditions.clone();
        primary_present.fallback_for_tools = vec!["read".to_string()];
        assert!(
            validate_skill_tool_surface(&skill, &primary_present, &tools)
                .unwrap_err()
                .contains("conflicts with available tool 'read'")
        );

        let mut missing_toolset = conditions.clone();
        missing_toolset
            .requires_toolsets
            .push("browser".to_string());
        assert!(
            validate_skill_tool_surface(&skill, &missing_toolset, &tools)
                .unwrap_err()
                .contains("requires unavailable toolset 'browser'")
        );

        let mut primary_toolset_present = conditions;
        primary_toolset_present.fallback_for_toolsets = vec!["web".to_string()];
        assert!(
            validate_skill_tool_surface(&skill, &primary_toolset_present, &tools)
                .unwrap_err()
                .contains("conflicts with available toolset 'web'")
        );
    }

    #[test]
    fn repeated_skill_keeps_distinct_node_tool_surfaces() {
        let skill = SkillId::from("conditional");
        let agent_node = |tool: &str, next: &str| {
            NodeSpec::Agent(AgentNode {
                agent: AgentId::from("build"),
                skills: BTreeSet::from([skill.clone()]),
                tools: BTreeSet::from([ToolId::new(tool)]),
                required_model_capabilities: BTreeSet::new(),
                max_steps: 1,
                next: NodeId::from(next),
            })
        };
        let blueprint = SchedulerBlueprint {
            schema: BlueprintSchemaVersion::V1,
            name: BlueprintName::from("skill-surfaces"),
            entry: NodeId::from("first"),
            nodes: BTreeMap::from([
                (NodeId::from("first"), agent_node("read", "second")),
                (NodeId::from("second"), agent_node("write", "done")),
                (
                    NodeId::from("done"),
                    NodeSpec::End(EndNode {
                        result: agendao_orchestrator::blueprint::ResultSource::LastNode,
                    }),
                ),
            ]),
            limits: execution_limits(&CompiledExecutionRequest::default()),
            output: OutputContract {
                format: OutputFormat::Markdown,
                include_usage: true,
                include_artifact_refs: true,
            },
        };

        let surfaces = selected_skill_tool_surfaces(&blueprint);
        assert_eq!(
            surfaces[&skill],
            BTreeSet::from([
                BTreeSet::from([ToolId::from("read")]),
                BTreeSet::from([ToolId::from("write")]),
            ])
        );
    }
}
