use agendao_agent::{AgentInfo, AgentRegistry};
use agendao_execution_types::CompiledExecutionRequest;
use agendao_orchestrator::agent_loop::{
    AgentLoopObserver, AgentObservationContext, CancellationFlag, ModelRoute, ProviderModelBackend,
    ToolCall, ToolExecution,
};
use agendao_orchestrator::blueprint::{
    AgentId, BlueprintName, CapabilityId, EvaluatorId, ExecutionLimits, ModelCapability, NodeSpec,
    OutputContract, OutputFormat, SchedulerBlueprint, SkillId, ToolId, ValidatedBlueprint,
};
use agendao_orchestrator::catalog::{
    AgentCatalogEntry, CapabilityCatalogEntry, CapabilityKind, EffectClass, EvaluatorCatalogEntry,
    EvaluatorKind, PermissionClass, SchedulerCatalog, SkillCatalogEntry, ToolCatalogEntry,
};
use agendao_orchestrator::context::{HandoffPacket, NodeResult, Usage};
use agendao_orchestrator::engine::{RunRequest, SchedulerEngine};
use agendao_orchestrator::events::{EventSink, ExecutionEvent};
use agendao_orchestrator::policy::{PolicyEnvelope, WorkspaceLimits};
use agendao_orchestrator::selector::{
    materialize_generated_agents, AutoSelector, ExplicitSelection, GeneratedAgentSpec,
    LockedSelection, SchedulerChoice, SelectionRequest, SelectionSource, TaskShape,
};
use agendao_orchestrator::templates::TemplateParameters;
use agendao_output_blocks::{OutputBlock, ToolBlock};
use agendao_provider::{Message, Provider, ToolDefinition};
use agendao_server_core::runtime_events::{ServerEvent, ToolCallPhase};
use agendao_skill::{infer_toolsets_from_tools, SkillConditions, SkillRuntimeResolver};
use async_trait::async_trait;
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
use crate::session_runtime::events::broadcast_session_reconcile;
use crate::ServerState;

pub(crate) const BLUEPRINT_LOCK_METADATA_KEY: &str = "scheduler_blueprint";
pub(crate) const BLUEPRINT_FINGERPRINT_METADATA_KEY: &str = "scheduler_blueprint_fingerprint";
pub(crate) const SELECTION_SOURCE_METADATA_KEY: &str = "scheduler_selection_source";
pub(crate) const GENERATED_AGENTS_METADATA_KEY: &str = "scheduler_generated_agents";
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
    pub primary_agent: Option<AgentId>,
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
    pub review: SchedulerReviewSignals,
}

#[derive(Debug, Clone)]
pub struct SchedulerReviewSignals {
    pub tool_call_count: usize,
    pub error_tool_call_count: usize,
    pub skill_write_count: usize,
    pub used_skill_names: Vec<String>,
}

struct SchedulerEventChannel(tokio::sync::mpsc::UnboundedSender<ExecutionEvent>);

struct SchedulerAgentObserver {
    state: Arc<ServerState>,
    session_id: String,
    assistant_message_id: String,
    tool_call_count: AtomicU64,
    error_tool_call_count: AtomicU64,
    skill_write_count: AtomicU64,
    /// Per-step tool tally flushed as ONE ToolBatchCompleted seam at the
    /// step boundary — the server-side batch fact source for the task
    /// ledger on the scheduler path (the session-layer summary is only
    /// written by the direct path).
    step_tallies: std::sync::Mutex<StepToolTallies>,
    run_cancellation: CancellationToken,
    auto_replan: bool,
}

#[derive(Default)]
struct StepToolTally {
    tools_used: Vec<String>,
    success_count: u32,
    error_count: u32,
    error_tools: Vec<String>,
}

#[derive(Default)]
struct StepToolTallies {
    by_node: HashMap<String, StepToolTally>,
}

impl StepToolTallies {
    fn begin(&mut self, node_path: &str) {
        self.by_node
            .insert(node_path.to_string(), StepToolTally::default());
    }

    fn record(&mut self, node_path: &str, tool: &str, is_error: bool) {
        let tally = self.by_node.entry(node_path.to_string()).or_default();
        tally.tools_used.push(tool.to_string());
        if is_error {
            tally.error_count += 1;
            tally.error_tools.push(tool.to_string());
        } else {
            tally.success_count += 1;
        }
    }

    fn finish(&mut self, node_path: &str) -> StepToolTally {
        self.by_node.remove(node_path).unwrap_or_default()
    }
}

#[derive(Clone)]
struct SchedulerEvaluatorScope {
    goal_generation: u64,
    covered_criteria: Vec<String>,
}

fn scheduler_evaluator_prompt(goal: &str, criteria: &[String]) -> String {
    let criteria_text = if criteria.is_empty() {
        "(no explicit acceptance criteria)".to_string()
    } else {
        criteria
            .iter()
            .map(|criterion| format!("- {criterion}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "Judge whether the candidate fully satisfies the original goal and every listed acceptance criterion.\n\nOriginal goal:\n{goal}\n\nAcceptance criteria:\n{criteria_text}"
    )
}

#[async_trait]
impl AgentLoopObserver for SchedulerAgentObserver {
    async fn tool_started(
        &self,
        _context: &AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().timestamp_millis();
        {
            let mut sessions = self.state.sessions.lock().await;
            let session = sessions
                .get_mut(&self.session_id)
                .ok_or_else(|| format!("scheduler session '{}' is unavailable", self.session_id))?;
            let assistant = session
                .get_message_mut(&self.assistant_message_id)
                .ok_or_else(|| {
                    format!(
                        "scheduler assistant message '{}' is unavailable",
                        self.assistant_message_id
                    )
                })?;
            if !assistant.parts.iter().any(|part| {
                matches!(
                    &part.part_type,
                    agendao_session::PartType::ToolCall { id, .. } if id == &call.id
                )
            }) {
                assistant.add_tool_call(&call.id, call.tool.as_str(), call.arguments.clone());
            }
            if let Some(part) = assistant.parts.iter_mut().find(|part| {
                matches!(
                    &part.part_type,
                    agendao_session::PartType::ToolCall { id, .. } if id == &call.id
                )
            }) {
                if let agendao_session::PartType::ToolCall { status, state, .. } =
                    &mut part.part_type
                {
                    *status = agendao_session::ToolCallStatus::Running;
                    *state = Some(agendao_session::ToolState::Running {
                        input: call.arguments.clone(),
                        title: None,
                        metadata: None,
                        time: agendao_session::RunningTime { start: now },
                    });
                }
            }
        }

        self.state
            .runtime_telemetry
            .runtime_state()
            .tool_started(&self.session_id, &call.id, call.tool.as_str())
            .await;
        crate::session_runtime::events::broadcast_server_event(
            self.state.as_ref(),
            &ServerEvent::ToolCallLifecycle {
                session_id: self.session_id.clone(),
                tool_call_id: call.id.clone(),
                phase: ToolCallPhase::Start,
                tool_name: Some(call.tool.as_str().to_string()),
            },
        );
        let start = OutputBlock::Tool(ToolBlock::start(call.tool.as_str()));
        crate::session_runtime::events::broadcast_server_event(
            self.state.as_ref(),
            &ServerEvent::output_block(
                self.session_id.clone(),
                &start,
                Some(&call.id),
                Some(agendao_session::prompt::tool_call_live_identity(
                    &self.assistant_message_id,
                    &call.id,
                    agendao_types::LivePartPhase::Start,
                )),
            ),
        );
        let arguments = serde_json::to_string(&call.arguments).unwrap_or_default();
        let running = OutputBlock::Tool(ToolBlock::running(call.tool.as_str(), arguments));
        crate::session_runtime::events::broadcast_server_event(
            self.state.as_ref(),
            &ServerEvent::output_block(
                self.session_id.clone(),
                &running,
                Some(&call.id),
                Some(agendao_session::prompt::tool_call_live_identity(
                    &self.assistant_message_id,
                    &call.id,
                    agendao_types::LivePartPhase::Append,
                )),
            ),
        );
        Ok(())
    }

    async fn tool_finished(
        &self,
        context: &AgentObservationContext<'_>,
        call: &ToolCall,
        result: &ToolExecution,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().timestamp_millis();
        {
            let mut tallies = self
                .step_tallies
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tallies.record(context.node_path, call.tool.as_str(), result.is_error);
        }
        let metadata = result
            .metadata
            .clone()
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.into_iter().collect::<HashMap<_, _>>())
            .unwrap_or_default();
        {
            let mut sessions = self.state.sessions.lock().await;
            let session = sessions
                .get_mut(&self.session_id)
                .ok_or_else(|| format!("scheduler session '{}' is unavailable", self.session_id))?;
            let assistant = session
                .get_message_mut(&self.assistant_message_id)
                .ok_or_else(|| {
                    format!(
                        "scheduler assistant message '{}' is unavailable",
                        self.assistant_message_id
                    )
                })?;
            if let Some(part) = assistant.parts.iter_mut().find(|part| {
                matches!(
                    &part.part_type,
                    agendao_session::PartType::ToolCall { id, .. } if id == &call.id
                )
            }) {
                if let agendao_session::PartType::ToolCall { status, state, .. } =
                    &mut part.part_type
                {
                    *status = if result.is_error {
                        agendao_session::ToolCallStatus::Error
                    } else {
                        agendao_session::ToolCallStatus::Completed
                    };
                    *state = Some(if result.is_error {
                        agendao_session::ToolState::Error {
                            input: call.arguments.clone(),
                            error: result.output.clone(),
                            metadata: (!metadata.is_empty()).then_some(metadata.clone()),
                            time: agendao_session::ErrorTime {
                                start: now,
                                end: now,
                            },
                        }
                    } else {
                        agendao_session::ToolState::Completed {
                            input: call.arguments.clone(),
                            output: result.output.clone(),
                            title: result
                                .title
                                .clone()
                                .unwrap_or_else(|| "Tool Result".to_string()),
                            metadata: metadata.clone(),
                            time: agendao_session::CompletedTime {
                                start: now,
                                end: now,
                                compacted: None,
                            },
                            attachments: None,
                        }
                    });
                }
            }
            assistant.add_tool_result(&call.id, &result.output, result.is_error);
            if let Some(agendao_session::MessagePart {
                part_type:
                    agendao_session::PartType::ToolResult {
                        title,
                        metadata: part_metadata,
                        ..
                    },
                ..
            }) = assistant.parts.last_mut()
            {
                *title = result.title.clone();
                *part_metadata = (!metadata.is_empty()).then_some(metadata.clone());
            }
        }

        self.state
            .runtime_telemetry
            .runtime_state()
            .tool_ended(&self.session_id, &call.id)
            .await;
        crate::session_runtime::events::broadcast_server_event(
            self.state.as_ref(),
            &ServerEvent::ToolCallLifecycle {
                session_id: self.session_id.clone(),
                tool_call_id: call.id.clone(),
                phase: ToolCallPhase::Complete,
                tool_name: Some(call.tool.as_str().to_string()),
            },
        );
        let detail = match result.title.as_deref() {
            Some(title) if !title.trim().is_empty() => format!("{title}\n{}", result.output),
            _ => result.output.clone(),
        };
        let block = if result.is_error {
            OutputBlock::Tool(ToolBlock::error(call.tool.as_str(), detail))
        } else {
            OutputBlock::Tool(ToolBlock::done(call.tool.as_str(), Some(detail)))
        };
        crate::session_runtime::events::broadcast_server_event(
            self.state.as_ref(),
            &ServerEvent::output_block(
                self.session_id.clone(),
                &block,
                Some(&call.id),
                Some(agendao_session::prompt::tool_result_live_identity(
                    &self.assistant_message_id,
                    &call.id,
                    agendao_types::LivePartPhase::End,
                )),
            ),
        );

        self.tool_call_count.fetch_add(1, Ordering::Relaxed);
        if result.is_error {
            self.error_tool_call_count.fetch_add(1, Ordering::Relaxed);
        }
        let memory = self.state.runtime_memory.memory_authority();
        if let Err(error) = memory
            .ingest_tool_result_observation(&agendao_memory::ToolMemoryObservation {
                session_id: &self.session_id,
                tool_call_id: &call.id,
                tool_name: call.tool.as_str(),
                stage_id: Some(context.node_path),
                output: &result.output,
                is_error: result.is_error,
            })
            .await
        {
            tracing::warn!(
                %error,
                session_id = %self.session_id,
                tool_call_id = %call.id,
                tool = %call.tool.as_str(),
                stage_id = %context.node_path,
                "failed to ingest scheduler tool result into memory"
            );
        }

        if call.tool.as_str() == "skill_manage" && !result.is_error {
            self.skill_write_count.fetch_add(1, Ordering::Relaxed);
            if let Some((action, name, location, supporting_file, guard_report)) =
                scheduler_skill_write_metadata(result)
            {
                if let Err(error) = memory
                    .ingest_skill_write_observation(&agendao_memory::SkillWriteObservation {
                        session_id: &self.session_id,
                        tool_call_id: Some(&call.id),
                        skill_name: name,
                        action,
                        location,
                        supporting_file,
                        guard_report: guard_report.as_ref(),
                    })
                    .await
                {
                    tracing::warn!(
                        %error,
                        session_id = %self.session_id,
                        tool_call_id = %call.id,
                        skill = %name,
                        "failed to link scheduler skill write to memory"
                    );
                }
            } else {
                tracing::warn!(
                    session_id = %self.session_id,
                    tool_call_id = %call.id,
                    "scheduler skill_manage result omitted required write metadata"
                );
            }
        }
        Ok(())
    }

    async fn step_started(
        &self,
        context: &AgentObservationContext<'_>,
        _step: u32,
    ) -> Result<(), String> {
        let mut tallies = self
            .step_tallies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tallies.begin(context.node_path);
        Ok(())
    }

    async fn step_finished(
        &self,
        context: &AgentObservationContext<'_>,
        _step: u32,
        _usage: &Usage,
    ) -> Result<(), String> {
        let tally = {
            let mut tallies = self
                .step_tallies
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tallies.finish(context.node_path)
        };
        if tally.tools_used.is_empty() {
            return Ok(());
        }
        let goal_status = if tally.error_count == 0 {
            agendao_types::repair::ToolBatchGoalStatus::Advanced
        } else if tally.success_count == 0 {
            agendao_types::repair::ToolBatchGoalStatus::Blocked
        } else {
            agendao_types::repair::ToolBatchGoalStatus::Mixed
        };
        crate::session_runtime::task_ledger_reducer::dispatch_run_seam(
            &self.state,
            &self.session_id,
            agendao_types::task_ledger::TaskLedgerSeamFact::ToolBatchCompleted {
                summary: agendao_types::repair::ToolBatchSummary {
                    tools_used: tally.tools_used,
                    success_count: tally.success_count,
                    error_count: tally.error_count,
                    error_kinds: tally
                        .error_tools
                        .into_iter()
                        .map(|tool| format!("{tool}:error"))
                        .collect(),
                    goal_status,
                    blocked_by: Vec::new(),
                    artifacts_created: Vec::new(),
                    pending_follow_up: Vec::new(),
                    // A failed call is an observation, not proof that a
                    // question remains unresolved after the next model step.
                    unresolved_items: Vec::new(),
                    recommended_next_step: None,
                    repair_events: Vec::new(),
                },
            },
            self.auto_replan,
            &self.run_cancellation,
        )
        .await;
        Ok(())
    }

    async fn take_boundary_inputs(
        &self,
        _context: &AgentObservationContext<'_>,
    ) -> Result<Vec<String>, String> {
        let steering = self
            .state
            .steering_store
            .lock()
            .await
            .drain(&self.session_id);
        if steering.is_empty() {
            return Ok(Vec::new());
        }

        let now = chrono::Utc::now().timestamp_millis();
        let last_source = steering
            .iter()
            .rev()
            .find_map(|message| message.source_session_id.clone());
        let last_latency_ms = steering.iter().rev().find_map(|message| {
            (message.created_at > 0).then_some(now.saturating_sub(message.created_at) as u64)
        });
        let inputs = steering
            .iter()
            .map(|message| message.text.clone())
            .collect::<Vec<_>>();

        {
            let mut sessions = self.state.sessions.lock().await;
            let session = sessions
                .get_mut(&self.session_id)
                .ok_or_else(|| format!("scheduler session '{}' is unavailable", self.session_id))?;
            for (index, message) in steering.iter().enumerate() {
                let mut record = agendao_session::SessionMessage::user(
                    self.session_id.clone(),
                    message.text.clone(),
                );
                record.metadata.insert(
                    "steering_mode".to_string(),
                    serde_json::json!("next_tool_boundary"),
                );
                record
                    .metadata
                    .insert("steering_status".to_string(), serde_json::json!("consumed"));
                record
                    .metadata
                    .insert("steering_index".to_string(), serde_json::json!(index));
                record
                    .metadata
                    .insert("steering_injected_at".to_string(), serde_json::json!(now));
                record.metadata.insert(
                    "steering_owner_session_id".to_string(),
                    serde_json::json!(&self.session_id),
                );
                record.metadata.insert(
                    "steering_injected_during_active_run".to_string(),
                    serde_json::json!(true),
                );
                if let Some(source) = message.source_session_id.as_ref() {
                    record.metadata.insert(
                        "steering_source_session_id".to_string(),
                        serde_json::json!(source),
                    );
                }
                let (admission, authority) = agendao_types::origin_to_admission_authority(
                    agendao_types::MessageSourceOrigin::System,
                );
                agendao_types::apply_message_source_metadata(
                    &mut record.metadata,
                    agendao_types::MessageSourceOrigin::System,
                    agendao_types::MessageSourceSurface::HttpApi,
                );
                agendao_types::apply_message_admission_metadata(
                    &mut record.metadata,
                    admission,
                    authority,
                );
                session.push_message(record);
            }
            let consumed = session
                .record()
                .metadata
                .get("consumed_steering_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                .saturating_add(steering.len() as u64);
            session.insert_metadata("consumed_steering_count", serde_json::json!(consumed));
            session.insert_metadata("last_steering_injected_at", serde_json::json!(now));
            session.insert_metadata(
                "last_steering_source_session_id",
                serde_json::json!(last_source),
            );
            session.insert_metadata(
                "last_steering_latency_ms",
                serde_json::json!(last_latency_ms),
            );
        }

        self.state
            .runtime_telemetry
            .emit_control_input_transition(
                &self.session_id,
                agendao_types::ControlInputKind::Steering,
                agendao_types::ControlInputPhase::Adopted,
                now,
            )
            .await;
        self.state
            .runtime_telemetry
            .emit_control_input_transition(
                &self.session_id,
                agendao_types::ControlInputKind::Steering,
                agendao_types::ControlInputPhase::Consumed,
                now,
            )
            .await;
        self.state
            .runtime_telemetry
            .runtime_state()
            .steering_cleared(&self.session_id)
            .await;
        self.state
            .runtime_telemetry
            .emit_control_input_transition(
                &self.session_id,
                agendao_types::ControlInputKind::Steering,
                agendao_types::ControlInputPhase::Cleared,
                now,
            )
            .await;
        broadcast_session_reconcile(
            self.state.as_ref(),
            self.session_id.clone(),
            agendao_server_core::runtime_events::ReconcileReason::StatusChange,
        )
        .await;
        Ok(inputs)
    }
}

type SchedulerSkillWriteMetadata<'a> = (
    &'static str,
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
    Option<agendao_types::SkillGuardReport>,
);

fn scheduler_skill_write_metadata(
    result: &ToolExecution,
) -> Option<SchedulerSkillWriteMetadata<'_>> {
    let metadata = result.metadata.as_ref()?.as_object()?;
    let action = match metadata.get("action")?.as_str()? {
        "created" | "create" => "create",
        "patched" | "patch" => "patch",
        "edited" | "edit" => "edit",
        "supporting_file_written" | "write_file" => "write_file",
        "supporting_file_removed" | "remove_file" => "remove_file",
        "deleted" | "delete" => "delete",
        _ => return None,
    };
    let name = metadata.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let location = metadata.get("location").and_then(serde_json::Value::as_str);
    let supporting_file = metadata
        .get("file_path")
        .and_then(serde_json::Value::as_str);
    let guard_report = metadata
        .get("guard_report")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    Some((action, name, location, supporting_file, guard_report))
}

impl EventSink for SchedulerEventChannel {
    fn emit(&self, event: ExecutionEvent) {
        let _ = self.0.send(event);
    }
}

pub async fn run_scheduler(input: SchedulerRunInput) -> Result<SchedulerRunOutput, String> {
    let config = input.state.config_store.config();
    let agents = Arc::new(AgentRegistry::from_config(&config));
    let tool_definitions = scheduler_tool_definitions(&input.state).await;
    let mut catalog = build_catalog(&input.state, &config, &agents, &tool_definitions)?;
    let runtime_budget = agendao_config::RuntimeBudgetConfig::from_config(Some(&config));
    let limits = execution_limits(&input.request, &runtime_budget);
    let policy = build_policy(&config, &catalog, limits.clone(), &runtime_budget);
    let task = classify_task(&input.goal);
    let primary = primary_agent(&agents, input.primary_agent.as_ref())?;
    let parameters = template_parameters(
        &catalog,
        &agents,
        primary,
        limits,
        &input.goal,
        &task,
        &tool_definitions,
    );
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
            task,
            default_parameters: parameters,
            goal: input.goal.clone(),
            workspace_summary: workspace_summary.clone(),
            rejected_blueprint_fingerprints,
        })
        .await
        .map_err(|error| error.to_string())?;

    catalog = materialize_generated_agents(&catalog, &selection.generated_agents)
        .map_err(|error| error.to_string())?;
    let mut selected_blueprint = selection.blueprint.blueprint().clone();
    bound_blueprint_tool_surfaces(
        &mut selected_blueprint,
        &catalog,
        &input.goal,
        &tool_definitions,
    );
    hydrate_selected_skills(&input.state, &selected_blueprint, &mut catalog)?;
    let blueprint = ValidatedBlueprint::new(selected_blueprint, &catalog, &policy)
        .map_err(|error| error.to_string())?;
    persist_blueprint_lock(
        &input.state,
        &input.session_id,
        &blueprint,
        selection.source,
        &selection.generated_agents,
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
    .with_routes(
        model_routes(
            &input.state,
            &agents,
            &selection.generated_agents,
            &input.request,
        )
        .await?,
    );
    let tool_backend = SessionSchedulerToolExecutor::new(
        input.state.clone(),
        SessionSchedulerToolExecutorInput {
            session_id: input.session_id.clone(),
            message_id: input.assistant_message_id.clone(),
            directory: input.directory.clone(),
            abort_token: input.cancellation.clone(),
            tool_runtime_config: agendao_tool::ToolRuntimeConfig::from_config(&config),
            execution_metadata: input.execution_metadata,
            capability_allowed_tools_by_agent: catalog
                .agents
                .iter()
                .map(|(agent_id, agent)| {
                    (
                        agent_id.as_str().to_string(),
                        agent
                            .available_tools
                            .iter()
                            .map(|tool| tool.as_str().to_string())
                            .collect(),
                    )
                })
                .collect(),
        },
    );
    let evaluator_ledger =
        crate::session_runtime::task_ledger::task_ledger_snapshot(&input.state, &input.session_id)
            .await
            .unwrap_or_else(|_| {
                agendao_types::task_ledger::SessionTaskLedger::empty(&input.session_id)
            });
    let evaluator_scope = SchedulerEvaluatorScope {
        goal_generation: evaluator_ledger.goal_generation,
        covered_criteria: evaluator_ledger
            .goal
            .as_ref()
            .map(|goal| goal.acceptance_criteria.clone())
            .unwrap_or_default(),
    };
    // The evaluator reviews the goal the LEDGER carries — the same source
    // the checkpoint's generation/criteria come from. Reviewing input.goal
    // while stamping ledger generations produces evidence about a goal the
    // ledger may no longer hold (preset or mid-run rewritten).
    let evaluator_goal_text = evaluator_ledger
        .goal
        .as_ref()
        .map(|goal| goal.statement.clone())
        .unwrap_or_else(|| input.goal.clone());
    let evaluator_prompt =
        scheduler_evaluator_prompt(&evaluator_goal_text, &evaluator_scope.covered_criteria);
    let evaluator = ModelEvaluatorBackend::new(
        input.provider,
        input.request,
        BTreeMap::from([(EvaluatorId::from("quality"), evaluator_prompt)]),
    );
    let capabilities = WorkspaceCapabilityHost::new(input.directory.clone().into())?;
    let cancellation = CancellationFlag::default();
    let cancellation_signal = cancellation.clone();
    let run_cancellation = input.cancellation.clone();
    let cancellation_token = input.cancellation;
    let cancellation_task = tokio::spawn(async move {
        cancellation_token.cancelled().await;
        cancellation_signal.cancel();
    });
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_sink = SchedulerEventChannel(event_tx);
    let auto_replan = blueprint.blueprint().nodes.len() > 2
        || blueprint.blueprint().nodes.values().any(|node| {
            matches!(
                node,
                NodeSpec::Loop(_) | NodeSpec::Parallel(_) | NodeSpec::Gate(_)
            )
        });
    let agent_observer = SchedulerAgentObserver {
        state: input.state.clone(),
        session_id: input.session_id.clone(),
        assistant_message_id: input.assistant_message_id.clone(),
        tool_call_count: AtomicU64::new(0),
        error_tool_call_count: AtomicU64::new(0),
        skill_write_count: AtomicU64::new(0),
        step_tallies: std::sync::Mutex::new(StepToolTallies::default()),
        run_cancellation,
        auto_replan,
    };
    let used_skill_names = selected_skill_tool_surfaces(blueprint.blueprint())
        .keys()
        .map(|skill| skill.0.clone())
        .collect::<Vec<_>>();
    let projection_task = tokio::spawn(project_scheduler_events(
        input.state.clone(),
        input.session_id.clone(),
        event_rx,
        evaluator_scope,
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
    .with_agent_observer(&agent_observer)
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
        review: SchedulerReviewSignals {
            tool_call_count: agent_observer.tool_call_count.load(Ordering::Relaxed) as usize,
            error_tool_call_count: agent_observer.error_tool_call_count.load(Ordering::Relaxed)
                as usize,
            skill_write_count: agent_observer.skill_write_count.load(Ordering::Relaxed) as usize,
            used_skill_names,
        },
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
    let mut catalog = build_catalog(state, &config, &agents, &tools)?;
    let runtime_budget = agendao_config::RuntimeBudgetConfig::from_config(Some(&config));
    let limits = execution_limits(&CompiledExecutionRequest::default(), &runtime_budget);
    let policy = build_policy(&config, &catalog, limits, &runtime_budget);
    hydrate_selected_skills(state, &blueprint, &mut catalog)?;
    ValidatedBlueprint::new(blueprint, &catalog, &policy).map_err(|error| error.to_string())
}

async fn project_scheduler_events(
    state: Arc<ServerState>,
    session_id: String,
    mut events: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
    evaluator_scope: SchedulerEvaluatorScope,
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
                // Task-governance seam: a passed evaluation gate is
                // current-generation evidence (criterion mapping stays
                // explicit — the evaluator validates the node, not named
                // acceptance criteria).
                crate::session_runtime::task_ledger_reducer::dispatch_seam(
                    &state,
                    &session_id,
                    agendao_types::task_ledger::TaskLedgerSeamFact::EvaluatorGateCompleted {
                        node_path: path.clone(),
                        // `outcome` is already the &str projection above.
                        passed: outcome == "pass",
                        goal_generation: evaluator_scope.goal_generation,
                    },
                )
                .await;
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
    config: &agendao_config::Config,
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
                    permission: global_tool_permission_class(config, &tool.name),
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
                    requires_tools: skill
                        .conditions
                        .requires_tools
                        .into_iter()
                        .map(|tool| ToolId::new(tool.trim().to_ascii_lowercase()))
                        .collect(),
                    fallback_for_tools: skill
                        .conditions
                        .fallback_for_tools
                        .into_iter()
                        .map(|tool| ToolId::new(tool.trim().to_ascii_lowercase()))
                        .collect(),
                    requires_toolsets: skill
                        .conditions
                        .requires_toolsets
                        .into_iter()
                        .map(|toolset| toolset.trim().to_ascii_lowercase())
                        .collect(),
                    fallback_for_toolsets: skill
                        .conditions
                        .fallback_for_toolsets
                        .into_iter()
                        .map(|toolset| toolset.trim().to_ascii_lowercase())
                        .collect(),
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
                .filter(|tool| {
                    agent.is_tool_allowed(tool.as_str())
                        && tool_entries[*tool].permission != PermissionClass::DenyByDefault
                })
                .cloned()
                .collect();
            (
                id.clone(),
                AgentCatalogEntry {
                    id,
                    system_policy: agent.resolved_system_prompt().unwrap_or_default(),
                    max_steps: agent.max_steps.unwrap_or(1),
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

fn execution_limits(
    request: &CompiledExecutionRequest,
    budget: &agendao_config::RuntimeBudgetConfig,
) -> ExecutionLimits {
    ExecutionLimits {
        max_model_calls: budget.scheduler_max_model_calls,
        max_tool_calls: budget.scheduler_max_tool_calls,
        max_total_tokens: request
            .max_tokens_or(8_192)
            .saturating_mul(u64::from(budget.scheduler_max_model_calls))
            .min(budget.scheduler_max_total_tokens),
        max_wall_time_ms: request
            .timeout_secs
            .unwrap_or(budget.scheduler_max_wall_time_ms / 1_000)
            .saturating_mul(1_000)
            .min(budget.scheduler_max_wall_time_ms),
        max_parallelism: budget.scheduler_max_parallelism,
        max_graph_nodes: budget.scheduler_max_graph_nodes,
        max_graph_depth: budget.scheduler_max_graph_depth,
        max_loop_iterations: budget.scheduler_max_loop_iterations,
        max_agent_steps: budget.scheduler_max_agent_steps,
    }
}

fn build_policy(
    config: &agendao_config::Config,
    catalog: &SchedulerCatalog,
    hard_limits: ExecutionLimits,
    budget: &agendao_config::RuntimeBudgetConfig,
) -> PolicyEnvelope {
    let allowed_tools = catalog
        .tools
        .iter()
        .filter(|(_, tool)| tool.permission != PermissionClass::DenyByDefault)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    // Ask permits governed scheduler capabilities: the tools that produce
    // mutations still pass through their normal interactive permission gate,
    // while checkpoint restore remains the safety rollback for those writes.
    let allowed_capabilities = catalog
        .capabilities
        .iter()
        .filter(|(_, capability)| capability_effect_is_allowed(config, capability.effect))
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let allowed_effects = catalog
        .tools
        .iter()
        .filter(|(id, _)| allowed_tools.contains(*id))
        .map(|(_, tool)| tool.effect)
        .chain(
            catalog
                .capabilities
                .iter()
                .filter(|(id, _)| allowed_capabilities.contains(*id))
                .map(|(_, capability)| capability.effect),
        )
        .collect();
    PolicyEnvelope {
        hard_limits,
        allowed_tools,
        allowed_effects,
        allowed_capabilities,
        workspace_limits: WorkspaceLimits {
            max_files: budget.scheduler_workspace_max_files,
            max_total_bytes: budget.scheduler_workspace_max_total_bytes,
            min_free_disk_bytes: budget.scheduler_workspace_min_free_disk_bytes,
            operation_timeout_ms: budget.scheduler_workspace_operation_timeout_ms,
        },
    }
}

fn global_tool_permission_class(config: &agendao_config::Config, tool: &str) -> PermissionClass {
    match configured_tool_permission(config, tool) {
        agendao_permission::PermissionAction::Allow => PermissionClass::Automatic,
        agendao_permission::PermissionAction::Ask => PermissionClass::Ask,
        agendao_permission::PermissionAction::Deny => PermissionClass::DenyByDefault,
    }
}

fn configured_tool_permission(
    config: &agendao_config::Config,
    tool: &str,
) -> agendao_permission::PermissionAction {
    let Some(permission) = config.permission.as_ref() else {
        return agendao_permission::PermissionAction::Allow;
    };
    let permission_name = agendao_permission::tool_to_permission(tool);
    let Some(rule) = permission
        .rules
        .get(permission_name)
        .or_else(|| permission.rules.get("*"))
    else {
        return agendao_permission::PermissionAction::Allow;
    };
    match rule {
        agendao_config::PermissionRule::Action(action) => map_permission_action(action),
        agendao_config::PermissionRule::Object(patterns) => {
            agendao_permission::combine_actions(patterns.values().map(map_permission_action))
        }
    }
}

fn map_permission_action(
    action: &agendao_config::PermissionAction,
) -> agendao_permission::PermissionAction {
    match action {
        agendao_config::PermissionAction::Allow => agendao_permission::PermissionAction::Allow,
        agendao_config::PermissionAction::Ask => agendao_permission::PermissionAction::Ask,
        agendao_config::PermissionAction::Deny => agendao_permission::PermissionAction::Deny,
    }
}

fn capability_effect_is_allowed(config: &agendao_config::Config, effect: EffectClass) -> bool {
    let representative_tool = match effect {
        EffectClass::ReadOnly => "read",
        EffectClass::WorkspaceMutation => "write",
        EffectClass::ProcessExecution => "bash",
        EffectClass::Network => "webfetch",
        EffectClass::ExternalMutation => "question",
    };
    configured_tool_permission(config, representative_tool)
        != agendao_permission::PermissionAction::Deny
}

fn primary_agent(agents: &AgentRegistry, requested: Option<&AgentId>) -> Result<AgentId, String> {
    if let Some(requested) = requested {
        return agents
            .get(requested.as_str())
            .filter(|agent| !agent.hidden)
            .map(|agent| AgentId::new(agent.name.clone()))
            .ok_or_else(|| {
                format!(
                    "scheduler primary agent '{}' is unavailable",
                    requested.as_str()
                )
            });
    }
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
    goal: &str,
    task: &TaskShape,
    tool_definitions: &[ToolDefinition],
) -> TemplateParameters {
    let planning_agent = agents
        .get("plan")
        .filter(|agent| {
            !agent.hidden
                && catalog
                    .agents
                    .contains_key(&AgentId::new(agent.name.clone()))
        })
        .map(|agent| AgentId::new(agent.name.clone()));
    let collaborators = semantic_collaborators(agents, goal, task);
    let full_agent_tools = std::iter::once(primary_agent.clone())
        .chain(planning_agent.iter().cloned())
        .chain(collaborators.iter().cloned())
        .filter_map(|id| {
            catalog
                .agents
                .get(&id)
                .map(|agent| (id, agent.available_tools.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let agent_skills = semantic_skills(catalog, goal, task, &full_agent_tools);
    let agent_tools = full_agent_tools
        .iter()
        .map(|(agent, tools)| {
            let skills = agent_skills.get(agent).cloned().unwrap_or_default();
            (
                agent.clone(),
                progressive_scheduler_tool_surface(
                    catalog,
                    tools,
                    &skills,
                    goal,
                    tool_definitions,
                    None,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let agent_max_steps = agent_tools
        .keys()
        .filter_map(|id| {
            catalog
                .agents
                .get(id)
                .map(|agent| (id.clone(), agent.max_steps))
        })
        .collect();
    TemplateParameters {
        name: BlueprintName::from("session-scheduler"),
        primary_agent,
        planning_agent,
        collaborators,
        agent_skills,
        agent_tools,
        agent_max_steps,
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

fn progressive_scheduler_tool_surface(
    catalog: &SchedulerCatalog,
    allowed: &BTreeSet<ToolId>,
    skills: &BTreeSet<SkillId>,
    goal: &str,
    definitions: &[ToolDefinition],
    pinned: Option<&BTreeSet<ToolId>>,
) -> BTreeSet<ToolId> {
    const MAX_TOOLS: usize = 16;
    const MAX_SCHEMA_BYTES: usize = 32 * 1024;
    const CORE: &[&str] = &["capability", "bash", "read", "apply_patch", "grep"];
    const COMMON: &[&str] = &[
        "glob",
        "ls",
        "edit",
        "write",
        "lsp",
        "todoread",
        "todowrite",
        "batch",
        "question",
        "websearch",
        "webfetch",
        "skill_view",
    ];

    let schema_bytes = definitions
        .iter()
        .map(|definition| {
            (
                definition.name.as_str(),
                definition.name.len()
                    + definition
                        .description
                        .as_deref()
                        .map(str::len)
                        .unwrap_or_default()
                    + serde_json::to_vec(&definition.parameters)
                        .map(|value| value.len())
                        .unwrap_or_default(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut required = skills
        .iter()
        .filter_map(|skill| catalog.skills.get(skill))
        .flat_map(|skill| skill.requires_tools.iter().cloned())
        .filter(|tool| allowed.contains(tool))
        .collect::<BTreeSet<_>>();
    let normalized_goal = goal.to_ascii_lowercase();
    let goal_terms = semantic_terms(&normalized_goal);
    let mut ranked = allowed
        .iter()
        .filter(|tool| !CORE.contains(&tool.as_str()) && !required.contains(*tool))
        .map(|tool| {
            let name = tool.as_str().to_ascii_lowercase();
            let score = i32::from(normalized_goal.contains(&name)) * 20
                + goal_terms
                    .iter()
                    .filter(|term| name.contains(term.as_str()) || term.contains(&name))
                    .count() as i32
                    * 4
                + COMMON
                    .iter()
                    .position(|common| *common == name)
                    .map(|index| 12_i32.saturating_sub(index as i32))
                    .unwrap_or_default();
            (score, tool.clone())
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let mut candidates = CORE
        .iter()
        .map(|name| ToolId::from(*name))
        .filter(|tool| allowed.contains(tool))
        .collect::<Vec<_>>();
    candidates.extend(std::mem::take(&mut required));
    candidates.extend(
        pinned
            .into_iter()
            .flat_map(|tools| tools.iter().filter(|tool| allowed.contains(*tool)).cloned()),
    );
    candidates.extend(ranked.into_iter().map(|(_, tool)| tool));

    let mut selected = BTreeSet::new();
    let mut bytes = 0usize;
    for tool in candidates {
        if selected.contains(&tool) || selected.len() >= MAX_TOOLS {
            continue;
        }
        let next = schema_bytes.get(tool.as_str()).copied().unwrap_or_default();
        if !selected.is_empty() && bytes.saturating_add(next) > MAX_SCHEMA_BYTES {
            continue;
        }
        bytes = bytes.saturating_add(next);
        selected.insert(tool);
    }
    selected
}

fn bound_blueprint_tool_surfaces(
    blueprint: &mut SchedulerBlueprint,
    catalog: &SchedulerCatalog,
    goal: &str,
    definitions: &[ToolDefinition],
) {
    fn visit(
        nodes: &mut BTreeMap<
            agendao_orchestrator::blueprint::NodeId,
            agendao_orchestrator::blueprint::NodeSpec,
        >,
        catalog: &SchedulerCatalog,
        goal: &str,
        definitions: &[ToolDefinition],
    ) {
        for node in nodes.values_mut() {
            match node {
                agendao_orchestrator::blueprint::NodeSpec::Agent(agent) => {
                    let Some(agent_catalog) = catalog.agents.get(&agent.agent) else {
                        continue;
                    };
                    agent.tools = progressive_scheduler_tool_surface(
                        catalog,
                        &agent_catalog.available_tools,
                        &agent.skills,
                        goal,
                        definitions,
                        Some(&agent.tools),
                    );
                }
                agendao_orchestrator::blueprint::NodeSpec::Loop(loop_node) => {
                    visit(&mut loop_node.body.nodes, catalog, goal, definitions);
                }
                _ => {}
            }
        }
    }

    visit(&mut blueprint.nodes, catalog, goal, definitions);
}

fn semantic_skills(
    catalog: &SchedulerCatalog,
    goal: &str,
    task: &TaskShape,
    agent_tools: &BTreeMap<AgentId, BTreeSet<ToolId>>,
) -> BTreeMap<AgentId, BTreeSet<SkillId>> {
    let normalized_goal = goal.to_lowercase();
    let goal_terms = semantic_terms(&normalized_goal);
    agent_tools
        .iter()
        .filter_map(|(agent_id, tools)| {
            let agent = catalog.agents.get(agent_id)?;
            let mut ranked = catalog
                .skills
                .iter()
                .filter(|(skill_id, skill)| {
                    agent.available_skills.contains(*skill_id)
                        && skill_supports_tool_surface(skill, tools)
                })
                .filter_map(|(skill_id, skill)| {
                    let score =
                        skill_relevance(skill_id, skill, &normalized_goal, &goal_terms, task);
                    (score > 0).then_some((score, skill_id.clone()))
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
            let selected = ranked
                .into_iter()
                .take(3)
                .map(|(_, skill)| skill)
                .collect::<BTreeSet<_>>();
            (!selected.is_empty()).then_some((agent_id.clone(), selected))
        })
        .collect()
}

fn semantic_terms(value: &str) -> BTreeSet<String> {
    const STOP_WORDS: &[&str] = &[
        "about", "all", "and", "entire", "from", "into", "project", "that", "the", "this", "with",
        "一个", "全部", "整个", "以及", "进行", "这个", "项目",
    ];
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 3 && !STOP_WORDS.contains(term))
        .map(ToString::to_string)
        .collect()
}

fn skill_relevance(
    skill_id: &SkillId,
    skill: &SkillCatalogEntry,
    goal: &str,
    goal_terms: &BTreeSet<String>,
    task: &TaskShape,
) -> i32 {
    let id = skill_id.as_str().to_lowercase();
    let searchable = format!(
        "{} {} {}",
        id,
        skill.summary.to_lowercase(),
        skill
            .capability_tags
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    );
    let mut score = if goal.contains(&id) || goal.contains(&id.replace('-', " ")) {
        20
    } else {
        0
    };
    score += goal_terms
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count() as i32
        * 3;
    if task.requires_verification
        && contains_any(
            &searchable,
            &["audit", "review", "verify", "validation", "test", "quality"],
        )
    {
        score += 8;
    }
    if task.iterative_research
        && contains_any(
            &searchable,
            &["research", "experiment", "benchmark", "evaluation"],
        )
    {
        score += 8;
    }
    if task.benefits_from_parallelism
        && contains_any(&searchable, &["compare", "analysis", "review", "research"])
    {
        score += 3;
    }
    score
}

fn skill_supports_tool_surface(skill: &SkillCatalogEntry, tools: &BTreeSet<ToolId>) -> bool {
    let available_tools = tools
        .iter()
        .map(|tool| tool.as_str().trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let available_toolsets = infer_toolsets_from_tools(available_tools.iter().map(String::as_str));
    skill
        .requires_tools
        .iter()
        .all(|tool| available_tools.contains(tool.as_str()))
        && skill
            .fallback_for_tools
            .iter()
            .all(|tool| !available_tools.contains(tool.as_str()))
        && skill
            .requires_toolsets
            .iter()
            .all(|toolset| available_toolsets.contains(toolset))
        && skill
            .fallback_for_toolsets
            .iter()
            .all(|toolset| !available_toolsets.contains(toolset))
}

fn semantic_collaborators(agents: &AgentRegistry, goal: &str, task: &TaskShape) -> Vec<AgentId> {
    let normalized = goal.to_ascii_lowercase();
    let architecture_work = contains_any(
        &normalized,
        &[
            "architecture",
            "design",
            "refactor",
            "performance",
            "security",
            "架构",
            "设计",
            "重构",
            "性能",
            "安全",
        ],
    );
    let external_research = contains_any(
        &normalized,
        &[
            "documentation",
            "docs",
            "external",
            "provider",
            "protocol",
            "文档",
            "外部",
            "协议",
            "供应商",
        ],
    );
    let media_work = contains_any(
        &normalized,
        &[
            "pdf",
            "image",
            "screenshot",
            "diagram",
            "attachment",
            "图片",
            "截图",
            "图表",
            "附件",
        ],
    );
    let code_work = contains_any(
        &normalized,
        &[
            "code",
            "repository",
            "project",
            "debug",
            "audit",
            "代码",
            "仓库",
            "项目",
            "调试",
            "审计",
        ],
    );

    let mut ranked = agents
        .list_subagents()
        .into_iter()
        .map(|agent| {
            let role = agent.name.as_str();
            let mut score = semantic_description_overlap(agent, &normalized);
            if task.iterative_research {
                score += match role {
                    "docs-researcher" => 8,
                    "explore" => 5,
                    "architecture-advisor" => 2,
                    _ => 0,
                };
            }
            if task.requires_verification {
                score += match role {
                    "explore" => 7,
                    "architecture-advisor" => 6,
                    "docs-researcher" => 1,
                    _ => 0,
                };
            }
            if task.benefits_from_parallelism {
                score += 1;
            }
            if architecture_work && role == "architecture-advisor" {
                score += 8;
            }
            if external_research && role == "docs-researcher" {
                score += 8;
            }
            if media_work && role == "media-reader" {
                score += 10;
            }
            if code_work && role == "explore" {
                score += 8;
            }
            (score, collaborator_role_rank(role), agent.name.clone())
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    ranked
        .into_iter()
        .take(3)
        .map(|(_, _, name)| AgentId::new(name))
        .collect()
}

fn semantic_description_overlap(agent: &AgentInfo, goal: &str) -> i32 {
    agent
        .description
        .as_deref()
        .unwrap_or_default()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| word.len() >= 4)
        .map(str::to_ascii_lowercase)
        .filter(|word| goal.contains(word))
        .count() as i32
}

fn collaborator_role_rank(role: &str) -> u8 {
    match role {
        "explore" => 0,
        "architecture-advisor" => 1,
        "docs-researcher" => 2,
        "media-reader" => 3,
        _ => 4,
    }
}

fn classify_task(goal: &str) -> TaskShape {
    let normalized = goal.to_ascii_lowercase();
    let iterative_research = contains_any(
        &normalized,
        &[
            "autoresearch",
            "iterative research",
            "research loop",
            "迭代研究",
            "循环实验",
            "持续研究",
            "多轮研究",
        ],
    );
    let requires_verification = contains_any(
        &normalized,
        &[
            "verify",
            "validation",
            "audit",
            "review",
            "验证",
            "核验",
            "审计",
            "复查",
            "测试",
        ],
    );
    let benefits_from_parallelism = contains_any(
        &normalized,
        &[
            "parallel",
            "compare",
            "comparison",
            "comprehensive",
            "entire project",
            "并行",
            "对比",
            "比较",
            "全面",
            "整个项目",
            "全项目",
            "逐项",
        ],
    );
    let complex = contains_any(
        &normalized,
        &[
            "architecture",
            "refactor",
            "redesign",
            "migration",
            "security",
            "permission",
            "performance",
            "optimize",
            "dead code",
            "架构",
            "重构",
            "重新设计",
            "迁移",
            "安全",
            "权限",
            "性能",
            "优化",
            "死代码",
            "治理",
        ],
    );
    let explicitly_small = contains_any(
        &normalized,
        &[
            "typo",
            "fix ",
            "rename",
            "format this",
            "explain",
            "show me",
            "single file",
            "one file",
            "错别字",
            "修复",
            "重命名",
            "格式化",
            "解释",
            "查看",
            "列出",
            "单个文件",
            "一处",
        ],
    );
    TaskShape {
        simple: !iterative_research
            && !requires_verification
            && !benefits_from_parallelism
            && !complex
            && goal.chars().count() <= 160
            && explicitly_small,
        iterative_research,
        requires_verification,
        benefits_from_parallelism,
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
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
    generated_agents: &[GeneratedAgentSpec],
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
    for generated in generated_agents {
        if let Some(route) = routes.get(&generated.base_agent).cloned() {
            routes.insert(generated.id.clone(), route);
        }
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
) -> Result<Option<LockedSelection>, String> {
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
    let generated_agents = session
        .record()
        .metadata
        .get(GENERATED_AGENTS_METADATA_KEY)
        .ok_or_else(|| "stored scheduler Blueprint has no generated-agent manifest".to_string())
        .and_then(|value| {
            serde_json::from_value::<Vec<GeneratedAgentSpec>>(value.clone())
                .map_err(|error| format!("stored generated-agent manifest is invalid: {error}"))
        })?;
    let source = session
        .record()
        .metadata
        .get(SELECTION_SOURCE_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "stored scheduler Blueprint has no selection source".to_string())
        .and_then(parse_selection_source)?;
    let extended = materialize_generated_agents(catalog, &generated_agents)
        .map_err(|error| format!("stored generated-agent manifest no longer validates: {error}"))?;
    ValidatedBlueprint::new(blueprint, &extended, policy)
        .map(|blueprint| {
            Some(LockedSelection {
                blueprint,
                source,
                generated_agents,
            })
        })
        .map_err(|error| format!("stored scheduler Blueprint no longer validates: {error}"))
}

async fn persist_blueprint_lock(
    state: &ServerState,
    session_id: &str,
    blueprint: &ValidatedBlueprint,
    source: SelectionSource,
    generated_agents: &[GeneratedAgentSpec],
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
    session.insert_metadata(
        GENERATED_AGENTS_METADATA_KEY,
        serde_json::to_value(generated_agents).map_err(|error| error.to_string())?,
    );
    sessions.update(session);
    Ok(())
}

pub(crate) fn selection_source_name(source: SelectionSource) -> &'static str {
    match source {
        SelectionSource::User => "user",
        SelectionSource::Heuristic => "heuristic",
        SelectionSource::Planner => "planner",
    }
}

fn parse_selection_source(source: &str) -> Result<SelectionSource, String> {
    match source {
        "user" => Ok(SelectionSource::User),
        "heuristic" => Ok(SelectionSource::Heuristic),
        "planner" => Ok(SelectionSource::Planner),
        other => Err(format!(
            "stored scheduler selection source '{other}' is invalid"
        )),
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

    #[test]
    fn scheduler_step_tallies_are_isolated_by_node_path() {
        let mut tallies = StepToolTallies::default();
        tallies.begin("root/left");
        tallies.begin("root/right");
        tallies.record("root/left", "bash", true);
        tallies.record("root/right", "read", false);

        let left = tallies.finish("root/left");
        assert_eq!(left.tools_used, vec!["bash"]);
        assert_eq!(left.error_count, 1);
        let right = tallies.finish("root/right");
        assert_eq!(right.tools_used, vec!["read"]);
        assert_eq!(right.success_count, 1);
    }

    #[test]
    fn scheduler_evaluator_prompt_names_goal_and_every_criterion() {
        let prompt = scheduler_evaluator_prompt(
            "ship median",
            &[
                "three tests pass".to_string(),
                "empty input errors".to_string(),
            ],
        );
        assert!(prompt.contains("Original goal:\nship median"));
        assert!(prompt.contains("- three tests pass"));
        assert!(prompt.contains("- empty input errors"));
    }
    use crate::session_runtime::memory::RuntimeMemoryAuthority;
    use agendao_config::ConfigStore;
    use agendao_memory::MemoryAuthority;
    use agendao_orchestrator::blueprint::{
        AgentNode, BlueprintSchemaVersion, EndNode, NodeId, NodeSpec,
    };
    use agendao_runtime_context::ResolvedWorkspaceContextAuthority;
    use agendao_state::UserStateAuthority;
    use agendao_storage::{Database, MemoryRepository};
    use tempfile::tempdir;

    #[tokio::test]
    async fn scheduler_tool_completion_is_persisted_as_memory() {
        crate::isolate_test_config_home();
        let workspace = tempdir().expect("workspace");
        let config_store = Arc::new(
            ConfigStore::from_project_dir(workspace.path()).expect("workspace config store"),
        );
        let user_state = Arc::new(UserStateAuthority::from_config_store(&config_store));
        let resolved_context = Arc::new(ResolvedWorkspaceContextAuthority::new(
            config_store.clone(),
            user_state.clone(),
        ));
        let database = Database::in_memory().await.expect("in-memory database");
        let repository = Arc::new(MemoryRepository::new(database.pool().clone()));
        let mut state = ServerState::new();
        state.workspace_root = workspace.path().to_path_buf();
        state.config_store = config_store;
        state.user_state = user_state.clone();
        state.resolved_context_authority = resolved_context.clone();
        state.runtime_memory = Arc::new(RuntimeMemoryAuthority::new(Arc::new(
            MemoryAuthority::new(user_state, resolved_context).with_repository(repository),
        )));
        let state = Arc::new(state);
        let (session_id, assistant_message_id) = {
            let mut sessions = state.sessions.lock().await;
            let session = sessions.create(
                "scheduler-memory-project",
                workspace.path().to_string_lossy(),
            );
            let session_id = session.id.clone();
            let assistant_message_id = sessions
                .get_mut(&session_id)
                .expect("created scheduler session")
                .add_assistant_message()
                .id
                .clone();
            (session_id, assistant_message_id)
        };
        let observer = SchedulerAgentObserver {
            state: state.clone(),
            session_id: session_id.clone(),
            assistant_message_id: assistant_message_id.clone(),
            tool_call_count: AtomicU64::new(0),
            error_tool_call_count: AtomicU64::new(0),
            skill_write_count: AtomicU64::new(0),
            step_tallies: std::sync::Mutex::new(StepToolTallies::default()),
            run_cancellation: CancellationToken::new(),
            auto_replan: false,
        };
        let agent = AgentId::from("worker");
        let context = AgentObservationContext {
            node_path: "execute",
            agent: &agent,
        };
        let call = ToolCall {
            id: "tool-call-1".to_string(),
            tool: ToolId::from("skill_manage"),
            arguments: serde_json::json!({"action": "patch", "name": "e2e-skill"}),
        };
        let result = ToolExecution {
            output: "SCHEDULER_MEMORY_EVIDENCE".to_string(),
            title: None,
            metadata: Some(serde_json::json!({
                "action": "patched",
                "name": "e2e-skill",
                "location": workspace.path().join(".agendao/skills/e2e-skill/SKILL.md"),
            })),
            is_error: false,
        };

        observer
            .tool_started(&context, &call)
            .await
            .expect("tool start must be persisted");
        observer
            .tool_finished(&context, &call, &result)
            .await
            .expect("memory ingestion must not fail the observer");

        let records = state
            .runtime_memory
            .list_memory(None)
            .await
            .expect("memory query");
        assert!(records.iter().any(|record| {
            record.title == "Methodology candidate linked to skill e2e-skill"
                && record.summary.contains("e2e-skill")
        }));
        assert_eq!(observer.tool_call_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.skill_write_count.load(Ordering::Relaxed), 1);
        let sessions = state.sessions.lock().await;
        let assistant = sessions
            .get(&session_id)
            .and_then(|session| session.get_message(&assistant_message_id))
            .expect("scheduler assistant message");
        assert!(assistant.parts.iter().any(|part| matches!(
            &part.part_type,
            agendao_session::PartType::ToolCall {
                id,
                status: agendao_session::ToolCallStatus::Completed,
                ..
            } if id == "tool-call-1"
        )));
        assert!(assistant.parts.iter().any(|part| matches!(
            &part.part_type,
            agendao_session::PartType::ToolResult {
                tool_call_id,
                content,
                ..
            } if tool_call_id == "tool-call-1" && content == "SCHEDULER_MEMORY_EVIDENCE"
        )));
    }

    #[test]
    fn classification_keeps_simple_work_on_direct_path() {
        assert!(classify_task("Explain this function").simple);
        let audit = classify_task("全面审计并验证整个项目");
        assert!(!audit.simple);
        assert!(audit.requires_verification);
        assert!(audit.benefits_from_parallelism);
        assert!(classify_task("run autoresearch").iterative_research);
        assert!(!classify_task("redesign the provider architecture").simple);
        assert_eq!(
            classify_task("implement a credential rotation workflow"),
            TaskShape::default()
        );
    }

    #[test]
    fn progressive_scheduler_surface_bounds_hundreds_of_tools_and_keeps_core() {
        let state = ServerState::new();
        let config = agendao_config::Config::default();
        let agents = AgentRegistry::from_config(&config);
        let catalog = build_catalog(&state, &config, &agents, &[]).unwrap();
        let mut names = vec!["capability", "bash", "read", "apply_patch", "grep"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        names.extend((0..300).map(|index| format!("mcp_tool_{index:03}")));
        let allowed = names.iter().cloned().map(ToolId::new).collect();
        let definitions = names
            .iter()
            .map(|name| ToolDefinition {
                name: name.clone(),
                description: Some("synthetic MCP capability".to_string()),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}}
                }),
            })
            .collect::<Vec<_>>();

        let selected = progressive_scheduler_tool_surface(
            &catalog,
            &allowed,
            &BTreeSet::new(),
            "use mcp_tool_299",
            &definitions,
            None,
        );

        assert!(selected.len() <= 16);
        for core in ["capability", "bash", "read", "apply_patch", "grep"] {
            assert!(
                selected.contains(&ToolId::from(core)),
                "missing core {core}"
            );
        }
        assert!(selected.contains(&ToolId::from("mcp_tool_299")));
    }

    #[test]
    fn collaborator_selection_matches_task_semantics() {
        let agents = AgentRegistry::new();
        let audit = classify_task("全面审计项目架构、性能和权限并逐项验证");
        assert_eq!(
            semantic_collaborators(&agents, "全面审计项目架构、性能和权限并逐项验证", &audit),
            vec![
                AgentId::from("explore"),
                AgentId::from("architecture-advisor"),
                AgentId::from("docs-researcher"),
            ]
        );

        let media = classify_task("compare the attached screenshots");
        assert_eq!(
            semantic_collaborators(&agents, "compare the attached screenshots", &media)[0],
            AgentId::from("media-reader")
        );
    }

    #[test]
    fn requested_agent_becomes_the_direct_blueprint_primary_leaf() {
        let state = ServerState::new();
        let config = agendao_config::Config::default();
        let agents = AgentRegistry::from_config(&config);
        let requested = AgentId::from("deep-worker");
        let primary = primary_agent(&agents, Some(&requested)).expect("visible requested agent");
        assert_eq!(primary, requested);

        let definitions = ["read", "grep", "bash"]
            .into_iter()
            .map(|name| ToolDefinition {
                name: name.to_string(),
                description: None,
                parameters: serde_json::json!({"type": "object"}),
            })
            .collect::<Vec<_>>();
        let catalog = build_catalog(&state, &config, &agents, &definitions).unwrap();
        let budget = agendao_config::RuntimeBudgetConfig::default();
        let limits = execution_limits(&CompiledExecutionRequest::default(), &budget);
        let parameters = template_parameters(
            &catalog,
            &agents,
            primary,
            limits,
            "perform a deep repository analysis",
            &TaskShape {
                simple: true,
                ..TaskShape::default()
            },
            &definitions,
        );
        let blueprint = agendao_orchestrator::templates::build_template(
            agendao_orchestrator::templates::TemplateId::Direct,
            &parameters,
        )
        .expect("direct template");
        let NodeSpec::Agent(execute) = &blueprint.nodes[&NodeId::from("execute")] else {
            panic!("direct execute node must be an agent");
        };
        assert_eq!(execute.agent, AgentId::from("deep-worker"));
    }

    #[tokio::test]
    async fn generated_agent_manifest_round_trips_with_the_blueprint_lock() {
        let state = ServerState::new();
        let session = agendao_session::Session::new("project", ".");
        let session_id = session.id.clone();
        state.sessions.lock().await.update(session);

        let config = agendao_config::Config::default();
        let agents = AgentRegistry::from_config(&config);
        let catalog = build_catalog(&state, &config, &agents, &[]).unwrap();
        let budget = agendao_config::RuntimeBudgetConfig::default();
        let limits = execution_limits(&CompiledExecutionRequest::default(), &budget);
        let policy = build_policy(&config, &catalog, limits.clone(), &budget);
        let base = primary_agent(&agents, None).unwrap();
        let parameters = template_parameters(
            &catalog,
            &agents,
            base.clone(),
            limits,
            "inspect authentication boundaries",
            &TaskShape {
                simple: true,
                ..TaskShape::default()
            },
            &[],
        );
        let generated = GeneratedAgentSpec {
            id: AgentId::from("auth-reviewer"),
            base_agent: base,
            system_policy: "Focus on authentication boundaries and cite evidence.".to_string(),
        };
        let mut blueprint = agendao_orchestrator::templates::build_template(
            agendao_orchestrator::templates::TemplateId::Direct,
            &parameters,
        )
        .unwrap();
        let NodeSpec::Agent(execute) = blueprint
            .nodes
            .get_mut(&NodeId::from("execute"))
            .expect("execute node")
        else {
            panic!("execute node must be an agent");
        };
        execute.agent = generated.id.clone();
        let extended = materialize_generated_agents(&catalog, std::slice::from_ref(&generated))
            .expect("materialized generated agent");
        let validated = ValidatedBlueprint::new(blueprint.clone(), &extended, &policy)
            .expect("generated-agent Blueprint");

        persist_blueprint_lock(
            &state,
            &session_id,
            &validated,
            SelectionSource::Planner,
            std::slice::from_ref(&generated),
        )
        .await
        .expect("persist Blueprint lock");
        let loaded = load_locked_blueprint(&state, &session_id, &catalog, &policy)
            .await
            .expect("load Blueprint lock")
            .expect("stored lock");

        assert_eq!(loaded.source, SelectionSource::Planner);
        assert_eq!(loaded.generated_agents, vec![generated]);
        assert_eq!(loaded.blueprint.blueprint(), &blueprint);
    }

    #[test]
    fn semantic_skill_selection_is_agent_scoped_and_tool_aware() {
        let review = SkillId::from("code-review");
        let research = SkillId::from("docs-research");
        let blocked = SkillId::from("write-audit");
        let skill = |id: SkillId, summary: &str, required: &str| SkillCatalogEntry {
            id,
            summary: summary.to_string(),
            content_fingerprint: format!("skill-{summary}"),
            capability_tags: BTreeSet::new(),
            requires_tools: BTreeSet::from([ToolId::from(required)]),
            fallback_for_tools: BTreeSet::new(),
            requires_toolsets: BTreeSet::new(),
            fallback_for_toolsets: BTreeSet::new(),
            hydrated_prompt: None,
        };
        let reviewer = AgentId::from("reviewer");
        let researcher = AgentId::from("researcher");
        let catalog = SchedulerCatalog {
            revision: "semantic-skills-test".to_string(),
            agents: BTreeMap::from([
                (
                    reviewer.clone(),
                    AgentCatalogEntry {
                        id: reviewer.clone(),
                        system_policy: String::new(),
                        max_steps: 4,
                        available_skills: BTreeSet::from([review.clone(), blocked.clone()]),
                        available_tools: BTreeSet::from([ToolId::from("read")]),
                        model_capabilities: BTreeSet::new(),
                    },
                ),
                (
                    researcher.clone(),
                    AgentCatalogEntry {
                        id: researcher.clone(),
                        system_policy: String::new(),
                        max_steps: 4,
                        available_skills: BTreeSet::from([research.clone(), blocked.clone()]),
                        available_tools: BTreeSet::from([ToolId::from("webfetch")]),
                        model_capabilities: BTreeSet::new(),
                    },
                ),
            ]),
            skills: BTreeMap::from([
                (
                    review.clone(),
                    skill(review.clone(), "Review and verify source code", "read"),
                ),
                (
                    research.clone(),
                    skill(
                        research.clone(),
                        "Research and verify external documentation",
                        "webfetch",
                    ),
                ),
                (
                    blocked.clone(),
                    skill(blocked.clone(), "Audit and verify written output", "write"),
                ),
            ]),
            tools: BTreeMap::new(),
            evaluators: BTreeMap::new(),
            capabilities: BTreeMap::new(),
        };
        let agent_tools = BTreeMap::from([
            (reviewer.clone(), BTreeSet::from([ToolId::from("read")])),
            (
                researcher.clone(),
                BTreeSet::from([ToolId::from("webfetch")]),
            ),
        ]);

        let selected = semantic_skills(
            &catalog,
            "review code and research external documentation, then verify it",
            &TaskShape {
                requires_verification: true,
                ..TaskShape::default()
            },
            &agent_tools,
        );

        assert_eq!(selected[&reviewer], BTreeSet::from([review]));
        assert_eq!(selected[&researcher], BTreeSet::from([research]));
        assert!(selected.values().all(|skills| !skills.contains(&blocked)));
    }

    #[test]
    fn production_policy_intersects_permissions_and_runtime_budget() {
        let config = agendao_config::Config {
            permission: Some(agendao_config::PermissionConfig {
                rules: HashMap::from([
                    (
                        "bash".to_string(),
                        agendao_config::PermissionRule::Action(
                            agendao_config::PermissionAction::Deny,
                        ),
                    ),
                    (
                        "edit".to_string(),
                        agendao_config::PermissionRule::Action(
                            agendao_config::PermissionAction::Ask,
                        ),
                    ),
                ]),
            }),
            ..Default::default()
        };
        let tools = [
            ("read", EffectClass::ReadOnly),
            ("write", EffectClass::WorkspaceMutation),
            ("bash", EffectClass::ProcessExecution),
        ]
        .into_iter()
        .map(|(name, effect)| {
            let id = ToolId::from(name);
            (
                id.clone(),
                ToolCatalogEntry {
                    id,
                    effect,
                    permission: global_tool_permission_class(&config, name),
                },
            )
        })
        .collect();
        let catalog = SchedulerCatalog {
            revision: "policy-test".to_string(),
            agents: BTreeMap::new(),
            skills: BTreeMap::new(),
            tools,
            evaluators: BTreeMap::new(),
            capabilities: BTreeMap::from([(
                CapabilityId::from("workspace-checkpoint"),
                CapabilityCatalogEntry {
                    id: CapabilityId::from("workspace-checkpoint"),
                    kind: CapabilityKind::WorkspaceCheckpoint,
                    effect: EffectClass::WorkspaceMutation,
                },
            )]),
        };
        let budget = agendao_config::RuntimeBudgetConfig {
            scheduler_max_model_calls: 3,
            scheduler_max_tool_calls: 7,
            scheduler_max_total_tokens: 12_000,
            scheduler_max_wall_time_ms: 9_000,
            scheduler_workspace_max_files: 12,
            scheduler_workspace_max_total_bytes: 34_000,
            scheduler_workspace_min_free_disk_bytes: 56_000,
            scheduler_workspace_operation_timeout_ms: 7_000,
            ..Default::default()
        };
        let request = CompiledExecutionRequest {
            max_tokens: Some(8_000),
            timeout_secs: Some(30),
            ..Default::default()
        };
        let limits = execution_limits(&request, &budget);
        let policy = build_policy(&config, &catalog, limits.clone(), &budget);

        assert_eq!(limits.max_model_calls, 3);
        assert_eq!(limits.max_tool_calls, 7);
        assert_eq!(limits.max_total_tokens, 12_000);
        assert_eq!(limits.max_wall_time_ms, 9_000);
        assert!(policy.allowed_tools.contains(&ToolId::from("read")));
        assert!(policy.allowed_tools.contains(&ToolId::from("write")));
        assert!(!policy.allowed_tools.contains(&ToolId::from("bash")));
        assert!(!policy
            .allowed_effects
            .contains(&EffectClass::ProcessExecution));
        assert!(policy
            .allowed_capabilities
            .contains(&CapabilityId::from("workspace-checkpoint")));
        assert_eq!(policy.workspace_limits.max_files, 12);
        assert_eq!(policy.workspace_limits.max_total_bytes, 34_000);
        assert_eq!(policy.workspace_limits.min_free_disk_bytes, 56_000);
        assert_eq!(policy.workspace_limits.operation_timeout_ms, 7_000);

        let mut denied_config = config.clone();
        denied_config
            .permission
            .as_mut()
            .expect("permission config")
            .rules
            .insert(
                "edit".to_string(),
                agendao_config::PermissionRule::Action(agendao_config::PermissionAction::Deny),
            );
        let denied_policy = build_policy(&denied_config, &catalog, limits, &budget);
        assert!(denied_policy.allowed_capabilities.is_empty());
    }

    #[test]
    fn production_templates_use_each_agents_own_tool_surface() {
        let state = ServerState::new();
        let config = agendao_config::Config::default();
        let agents = AgentRegistry::from_config(&config);
        let definitions = ["read", "grep", "write", "bash", "websearch"]
            .into_iter()
            .map(|name| ToolDefinition {
                name: name.to_string(),
                description: None,
                parameters: serde_json::json!({"type": "object"}),
            })
            .collect::<Vec<_>>();
        let catalog = build_catalog(&state, &config, &agents, &definitions).unwrap();
        let budget = agendao_config::RuntimeBudgetConfig::default();
        let limits = execution_limits(&CompiledExecutionRequest::default(), &budget);
        let policy = build_policy(&config, &catalog, limits.clone(), &budget);
        let task = classify_task("全面并行审计项目架构和性能");
        let parameters = template_parameters(
            &catalog,
            &agents,
            primary_agent(&agents, None).unwrap(),
            limits,
            "全面并行审计项目架构和性能",
            &task,
            &definitions,
        );

        for template in [
            agendao_orchestrator::templates::TemplateId::Plan,
            agendao_orchestrator::templates::TemplateId::Coordinate,
        ] {
            let blueprint = agendao_orchestrator::templates::build_template(template, &parameters)
                .expect("template");
            ValidatedBlueprint::new(blueprint, &catalog, &policy)
                .unwrap_or_else(|error| panic!("{template:?} failed: {error}"));
        }
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
            limits: execution_limits(
                &CompiledExecutionRequest::default(),
                &agendao_config::RuntimeBudgetConfig::default(),
            ),
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
