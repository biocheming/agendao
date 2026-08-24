use crate::agent_loop::{
    AgentLoop, AgentLoopError, AgentLoopObserver, AgentRunContext, CancellationFlag,
    ExecutionBudget, InteractionClock, ModelBackend, ToolBackend, NOOP_AGENT_LOOP_OBSERVER,
};
use crate::blueprint::{
    CapabilityId, EvaluatorId, NodeId, NodeSpec, ParallelFailureMode, ResultSource, SchedulerGraph,
    ValidatedBlueprint,
};
use crate::catalog::{EvaluatorKind, SchedulerCatalog};
use crate::context::PromptAuthority;
use crate::context::{HandoffPacket, NodeResult, Usage};
use crate::events::{EventSink, ExecutionEvent, NOOP_EVENT_SINK};
use crate::policy::{PolicyEnvelope, WorkspaceLimits};
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evaluation {
    Pass,
    Fail,
    Indeterminate,
}

#[async_trait]
pub trait EvaluatorBackend: Send + Sync {
    async fn evaluate(
        &self,
        evaluator: &EvaluatorId,
        candidate: &NodeResult,
    ) -> Result<EvaluationOutcome, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationOutcome {
    pub evaluation: Evaluation,
    pub usage: Usage,
}

#[async_trait]
pub trait CapabilityBackend: Send + Sync {
    async fn checkpoint(&self, request: &CheckpointRequest) -> Result<CheckpointHandle, String>;
    async fn restore(&self, request: &RestoreRequest) -> Result<(), String>;
    async fn store_artifact(&self, request: &ArtifactRequest) -> Result<String, String>;
    async fn finalize(&self, disposition: RunDisposition) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunDisposition {
    Commit,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRequest {
    pub capability: CapabilityId,
    pub workspace_root: String,
    pub scope: String,
    pub iteration: u32,
    pub limits: WorkspaceLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointHandle {
    pub capability: CapabilityId,
    pub workspace_root: String,
    pub id: String,
    pub iteration: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRequest {
    pub checkpoint: CheckpointHandle,
    pub limits: WorkspaceLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRequest {
    pub capability: CapabilityId,
    pub workspace_root: String,
    pub name: String,
    pub content: String,
    pub limits: WorkspaceLimits,
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub handoff: HandoffPacket,
    pub conversation_seed: Vec<agendao_provider::Message>,
    pub workspace_root: String,
    pub workspace_summary: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EngineError {
    #[error(transparent)]
    Agent(#[from] AgentLoopError),
    #[error("evaluator '{evaluator}' failed: {message}")]
    Evaluator { evaluator: String, message: String },
    #[error("capability '{capability}' failed: {message}")]
    Capability { capability: String, message: String },
    #[error("execution reached missing node '{0}'")]
    MissingNode(String),
    #[error("parallel branch '{branch}' failed: {message}")]
    ParallelBranch { branch: String, message: String },
    #[error("end node requests unavailable result from '{0}'")]
    MissingResult(String),
    #[error("catalog fingerprint failed: {0}")]
    CatalogFingerprint(String),
    #[error("capability cleanup failed after {execution}: {message}")]
    Cleanup { execution: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub result: NodeResult,
    pub node_results: BTreeMap<String, NodeResult>,
    pub usage: Usage,
}

pub struct SchedulerEngine<'a> {
    agent_loop: AgentLoop<'a>,
    evaluator: &'a dyn EvaluatorBackend,
    capabilities: &'a dyn CapabilityBackend,
    catalog: &'a SchedulerCatalog,
    policy: &'a PolicyEnvelope,
    harness_policy: &'a str,
    events: &'a dyn EventSink,
    agent_observer: &'a dyn AgentLoopObserver,
    interaction_clock: InteractionClock,
}

impl<'a> SchedulerEngine<'a> {
    pub fn new(
        model: &'a dyn ModelBackend,
        tools: &'a dyn ToolBackend,
        evaluator: &'a dyn EvaluatorBackend,
        capabilities: &'a dyn CapabilityBackend,
        catalog: &'a SchedulerCatalog,
        policy: &'a PolicyEnvelope,
        harness_policy: &'a str,
    ) -> Self {
        Self {
            agent_loop: AgentLoop::new(model, tools),
            evaluator,
            capabilities,
            catalog,
            policy,
            harness_policy,
            events: &NOOP_EVENT_SINK,
            agent_observer: &NOOP_AGENT_LOOP_OBSERVER,
            interaction_clock: InteractionClock::default(),
        }
    }

    pub fn with_events(mut self, events: &'a dyn EventSink) -> Self {
        self.events = events;
        self
    }

    pub fn with_agent_observer(mut self, observer: &'a dyn AgentLoopObserver) -> Self {
        self.agent_observer = observer;
        self
    }

    pub fn with_interaction_clock(mut self, interaction_clock: InteractionClock) -> Self {
        self.interaction_clock = interaction_clock;
        self
    }

    pub async fn run(
        &self,
        blueprint: &ValidatedBlueprint,
        request: RunRequest,
        cancellation: CancellationFlag,
    ) -> Result<RunOutcome, EngineError> {
        let specification = blueprint.blueprint();
        let graph = SchedulerGraph {
            entry: specification.entry.clone(),
            nodes: specification.nodes.clone(),
        };
        let budget =
            ExecutionBudget::new(specification.limits.clone(), self.interaction_clock.clone());
        let prompt_authority = PromptAuthority::new(
            blueprint.fingerprint(),
            self.catalog
                .fingerprint()
                .map_err(|error| EngineError::CatalogFingerprint(error.to_string()))?,
            self.catalog,
            self.policy,
            self.harness_policy,
        );
        let control = RunControl {
            budget: &budget,
            cancellation: &cancellation,
            prompt_authority: &prompt_authority,
            workspace_root: &request.workspace_root,
            workspace_summary: &request.workspace_summary,
        };
        let state = BranchState::new(request.handoff, request.conversation_seed);
        self.events.emit(ExecutionEvent::RunStarted);
        let execution = self
            .execute_graph(
                &graph,
                graph.entry.clone(),
                None,
                "root".to_string(),
                state,
                control,
            )
            .await;
        let state = match execution {
            Ok(state) => state,
            Err(error) => {
                if let Err(message) = self.capabilities.finalize(RunDisposition::Rollback).await {
                    let cleanup = EngineError::Cleanup {
                        execution: error.to_string(),
                        message,
                    };
                    self.events.emit(ExecutionEvent::RunFailed {
                        message: cleanup.to_string(),
                    });
                    return Err(cleanup);
                }
                self.events.emit(ExecutionEvent::RunFailed {
                    message: error.to_string(),
                });
                return Err(error);
            }
        };
        if let Err(message) = self.capabilities.finalize(RunDisposition::Commit).await {
            let error = EngineError::Cleanup {
                execution: "successful execution".to_string(),
                message,
            };
            self.events.emit(ExecutionEvent::RunFailed {
                message: error.to_string(),
            });
            return Err(error);
        }
        let result = state.last.unwrap_or_default();
        let usage = budget.snapshot();
        self.events.emit(ExecutionEvent::RunCompleted {
            usage: usage.clone(),
        });
        Ok(RunOutcome {
            result,
            node_results: state.results,
            usage,
        })
    }

    fn execute_graph<'b>(
        &'b self,
        graph: &'b SchedulerGraph,
        start: NodeId,
        stop_before: Option<&'b NodeId>,
        scope: String,
        mut state: BranchState,
        control: RunControl<'b>,
    ) -> BoxFuture<'b, Result<BranchState, EngineError>> {
        Box::pin(async move {
            let budget = control.budget;
            let cancellation = control.cancellation;
            let mut current = start;
            loop {
                if stop_before == Some(&current) {
                    return Ok(state);
                }
                budget.check_time()?;
                if cancellation.is_cancelled() {
                    return Err(AgentLoopError::Cancelled.into());
                }
                let node = graph
                    .nodes
                    .get(&current)
                    .ok_or_else(|| EngineError::MissingNode(current.as_str().to_string()))?;
                let node_path = format!("{scope}/{}", current.as_str());
                self.events.emit(ExecutionEvent::NodeStarted {
                    path: node_path.clone(),
                });
                match node {
                    NodeSpec::Agent(agent) => {
                        let outcome = self
                            .agent_loop
                            .run(
                                agent,
                                state.handoff.clone(),
                                std::mem::take(&mut state.conversation_seed),
                                AgentRunContext {
                                    prompt_authority: control.prompt_authority,
                                    workspace_summary: control.workspace_summary,
                                    progress_summary: &node_path,
                                    budget,
                                    cancellation,
                                    observer: self.agent_observer,
                                },
                            )
                            .await?;
                        state.record(node_path.clone(), outcome.result);
                        self.events
                            .emit(ExecutionEvent::NodeCompleted { path: node_path });
                        current = agent.next.clone();
                    }
                    NodeSpec::Parallel(parallel) => {
                        let mut branches =
                            stream::iter(parallel.branches.clone().into_iter().map(|branch| {
                                let branch_state = state.fork_for(&branch);
                                let branch_scope = scope.clone();
                                async move {
                                    let result = self
                                        .execute_graph(
                                            graph,
                                            branch.clone(),
                                            Some(&parallel.join),
                                            branch_scope,
                                            branch_state,
                                            control,
                                        )
                                        .await;
                                    (branch, result)
                                }
                            }))
                            .buffer_unordered(parallel.max_parallelism as usize);
                        let mut successes = BTreeMap::new();
                        let mut failures = Vec::new();
                        while let Some((branch, result)) = branches.next().await {
                            match result {
                                Ok(branch_state) => {
                                    successes.insert(branch, branch_state);
                                }
                                Err(error)
                                    if parallel.failure_mode == ParallelFailureMode::FailFast =>
                                {
                                    return Err(EngineError::ParallelBranch {
                                        branch: branch.as_str().to_string(),
                                        message: error.to_string(),
                                    });
                                }
                                Err(error) => failures.push((branch, error)),
                            }
                        }
                        drop(branches);
                        failures.sort_by(|left, right| left.0.cmp(&right.0));
                        let mut branch_results: Vec<NodeResult> = successes
                            .values()
                            .filter_map(|branch| branch.last.clone())
                            .collect();
                        branch_results.extend(failures.into_iter().map(|(branch, error)| {
                            NodeResult {
                                summary: format!("branch {} failed: {error}", branch.as_str()),
                                ..NodeResult::default()
                            }
                        }));
                        for branch_state in successes.into_values() {
                            state.results.extend(branch_state.results);
                        }
                        let combined = NodeResult::combine(branch_results);
                        state.record(node_path.clone(), combined.clone());
                        self.events
                            .emit(ExecutionEvent::NodeCompleted { path: node_path });
                        state.handoff = handoff_from_result(&state.handoff, &combined);
                        current = parallel.join.clone();
                    }
                    NodeSpec::Gate(gate) => {
                        let candidate = state.last.clone().unwrap_or_default();
                        let evaluation = self
                            .evaluate_with_deadline(
                                &gate.evaluator,
                                &candidate,
                                budget,
                                cancellation,
                            )
                            .await?;
                        self.events.emit(ExecutionEvent::Evaluated {
                            path: node_path.clone(),
                            outcome: evaluation,
                        });
                        self.events
                            .emit(ExecutionEvent::NodeCompleted { path: node_path });
                        current = match evaluation {
                            Evaluation::Pass => gate.on_pass.clone(),
                            Evaluation::Fail => gate.on_fail.clone(),
                            Evaluation::Indeterminate => gate.on_indeterminate.clone(),
                        };
                    }
                    NodeSpec::Loop(loop_node) => {
                        let mut satisfied = false;
                        for iteration in 0..loop_node.max_iterations {
                            self.events.emit(ExecutionEvent::LoopIteration {
                                path: node_path.clone(),
                                iteration: iteration + 1,
                            });
                            let checkpoint = if let Some(capability) = &loop_node.checkpoint {
                                if cancellation.is_cancelled() {
                                    return Err(AgentLoopError::Cancelled.into());
                                }
                                let request = CheckpointRequest {
                                    capability: capability.clone(),
                                    workspace_root: control.workspace_root.to_string(),
                                    scope: node_path.clone(),
                                    iteration: iteration + 1,
                                    limits: self.policy.workspace_limits.clone(),
                                };
                                let checkpoint =
                                    self.capabilities.checkpoint(&request).await.map_err(
                                        |message| EngineError::Capability {
                                            capability: capability.as_str().to_string(),
                                            message,
                                        },
                                    )?;
                                if cancellation.is_cancelled() {
                                    return Err(AgentLoopError::Cancelled.into());
                                }
                                budget.check_time()?;
                                Some(checkpoint)
                            } else {
                                None
                            };
                            let loop_state = BranchState::new(
                                loop_handoff(&state.handoff, state.last.as_ref(), iteration),
                                Vec::new(),
                            );
                            let completed = self
                                .execute_graph(
                                    &loop_node.body,
                                    loop_node.body.entry.clone(),
                                    None,
                                    format!("{node_path}/iteration-{}", iteration + 1),
                                    loop_state,
                                    control,
                                )
                                .await;
                            let completed = match completed {
                                Ok(completed) => completed,
                                Err(error) => {
                                    if let Some(checkpoint) = checkpoint.as_ref() {
                                        self.restore_checkpoint(checkpoint).await?;
                                    }
                                    return Err(error);
                                }
                            };
                            let candidate = completed.last.clone().unwrap_or_default();
                            state.results.extend(completed.results);
                            state.record(node_path.clone(), candidate.clone());
                            let evaluation = self
                                .evaluate_with_deadline(
                                    &loop_node.evaluator,
                                    &candidate,
                                    budget,
                                    cancellation,
                                )
                                .await;
                            let evaluation = match evaluation {
                                Ok(evaluation) => evaluation,
                                Err(error) => {
                                    if let Some(checkpoint) = checkpoint.as_ref() {
                                        self.restore_checkpoint(checkpoint).await?;
                                    }
                                    return Err(error);
                                }
                            };
                            self.events.emit(ExecutionEvent::Evaluated {
                                path: node_path.clone(),
                                outcome: evaluation,
                            });
                            match evaluation {
                                Evaluation::Pass => {
                                    satisfied = true;
                                    break;
                                }
                                Evaluation::Fail | Evaluation::Indeterminate => {
                                    if let Some(checkpoint) = checkpoint.as_ref() {
                                        self.restore_checkpoint(checkpoint).await?;
                                    }
                                    state.handoff = handoff_from_result(&state.handoff, &candidate);
                                }
                            }
                        }
                        current = if satisfied {
                            loop_node.on_satisfied.clone()
                        } else {
                            loop_node.on_exhausted.clone()
                        };
                        self.events
                            .emit(ExecutionEvent::NodeCompleted { path: node_path });
                    }
                    NodeSpec::End(end) => {
                        let result = match &end.result {
                            ResultSource::LastNode => state.last.clone().unwrap_or_default(),
                            ResultSource::Named(source) => {
                                let source_path = format!("{scope}/{}", source.as_str());
                                state.results.get(&source_path).cloned().ok_or_else(|| {
                                    EngineError::MissingResult(source.as_str().to_string())
                                })?
                            }
                            ResultSource::Artifact { capability, name } => {
                                let candidate = state.last.clone().unwrap_or_default();
                                let content = candidate
                                    .output
                                    .clone()
                                    .unwrap_or_else(|| candidate.summary.clone());
                                if cancellation.is_cancelled() {
                                    return Err(AgentLoopError::Cancelled.into());
                                }
                                let artifact_id = self
                                    .capabilities
                                    .store_artifact(&ArtifactRequest {
                                        capability: capability.clone(),
                                        workspace_root: control.workspace_root.to_string(),
                                        name: name.clone(),
                                        content,
                                        limits: self.policy.workspace_limits.clone(),
                                    })
                                    .await
                                    .map_err(|message| EngineError::Capability {
                                        capability: capability.as_str().to_string(),
                                        message,
                                    })?;
                                if cancellation.is_cancelled() {
                                    return Err(AgentLoopError::Cancelled.into());
                                }
                                budget.check_time()?;
                                NodeResult {
                                    summary: format!("artifact:{artifact_id}"),
                                    artifact_refs: vec![crate::context::ArtifactRef {
                                        id: artifact_id,
                                        media_type: "text/plain".to_string(),
                                    }],
                                    ..NodeResult::default()
                                }
                            }
                        };
                        state.record(node_path.clone(), result);
                        self.events
                            .emit(ExecutionEvent::NodeCompleted { path: node_path });
                        return Ok(state);
                    }
                }
            }
        })
    }

    async fn restore_checkpoint(&self, checkpoint: &CheckpointHandle) -> Result<(), EngineError> {
        self.capabilities
            .restore(&RestoreRequest {
                checkpoint: checkpoint.clone(),
                limits: self.policy.workspace_limits.clone(),
            })
            .await
            .map_err(|message| EngineError::Capability {
                capability: checkpoint.capability.as_str().to_string(),
                message,
            })
    }

    async fn evaluate_with_deadline(
        &self,
        evaluator: &EvaluatorId,
        candidate: &NodeResult,
        budget: &ExecutionBudget,
        cancellation: &CancellationFlag,
    ) -> Result<Evaluation, EngineError> {
        let is_model_judge = self
            .catalog
            .evaluators
            .get(evaluator)
            .is_some_and(|entry| entry.kind == EvaluatorKind::ModelJudge);
        if is_model_judge {
            budget.reserve_model_call()?;
        }
        let outcome = tokio::select! {
            _ = cancellation.cancelled() => return Err(AgentLoopError::Cancelled.into()),
            _ = budget.deadline() => return Err(AgentLoopError::DeadlineExceeded.into()),
            result = self.evaluator.evaluate(evaluator, candidate) => result
                .map_err(|message| EngineError::Evaluator {
                    evaluator: evaluator.as_str().to_string(),
                    message,
                })?,
        };
        budget.record_tokens(&outcome.usage)?;
        Ok(outcome.evaluation)
    }
}

#[derive(Clone, Copy)]
struct RunControl<'a> {
    budget: &'a ExecutionBudget,
    cancellation: &'a CancellationFlag,
    prompt_authority: &'a PromptAuthority<'a>,
    workspace_root: &'a str,
    workspace_summary: &'a str,
}

#[derive(Debug, Clone)]
struct BranchState {
    handoff: HandoffPacket,
    conversation_seed: Vec<agendao_provider::Message>,
    last: Option<NodeResult>,
    results: BTreeMap<String, NodeResult>,
}

impl BranchState {
    fn new(handoff: HandoffPacket, conversation_seed: Vec<agendao_provider::Message>) -> Self {
        Self {
            handoff,
            conversation_seed,
            last: None,
            results: BTreeMap::new(),
        }
    }

    fn record(&mut self, node: String, result: NodeResult) {
        self.handoff = handoff_from_result(&self.handoff, &result);
        self.last = Some(result.clone());
        self.results.insert(node, result);
    }

    fn fork_for(&self, branch: &NodeId) -> Self {
        let mut handoff = self.handoff.clone();
        handoff
            .inputs
            .insert("scheduler.branch".to_string(), branch.as_str().to_string());
        Self::new(handoff, Vec::new())
    }
}

fn handoff_from_result(previous: &HandoffPacket, result: &NodeResult) -> HandoffPacket {
    let mut next = previous.clone();
    next.inputs
        .insert("previous.summary".to_string(), result.summary.clone());
    next.artifact_refs = result.artifact_refs.clone();
    next
}

fn loop_handoff(
    previous: &HandoffPacket,
    result: Option<&NodeResult>,
    iteration: u32,
) -> HandoffPacket {
    let mut next = previous.clone();
    next.inputs.insert(
        "scheduler.iteration".to_string(),
        (iteration + 1).to_string(),
    );
    if let Some(result) = result {
        next.inputs
            .insert("previous.summary".to_string(), result.summary.clone());
        next.artifact_refs = result.artifact_refs.clone();
    }
    next
}
