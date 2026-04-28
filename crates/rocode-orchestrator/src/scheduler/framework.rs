use async_trait::async_trait;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use super::{SchedulerStageKind, StageToolPolicy};
use crate::OrchestratorError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerSkillRef {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
}

impl From<&str> for SchedulerSkillRef {
    fn from(value: &str) -> Self {
        Self {
            name: value.to_string(),
            description: String::new(),
            category: None,
        }
    }
}

impl From<String> for SchedulerSkillRef {
    fn from(value: String) -> Self {
        Self {
            name: value,
            description: String::new(),
            category: None,
        }
    }
}

impl Serialize for SchedulerSkillRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("SchedulerSkillRef", 3)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("description", &self.description)?;
        state.serialize_field("category", &self.category)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for SchedulerSkillRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SchedulerSkillRefVisitor;

        impl<'de> Visitor<'de> for SchedulerSkillRefVisitor {
            type Value = SchedulerSkillRef;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a skill name string or a skill metadata object")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SchedulerSkillRef::from(value))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SchedulerSkillRef::from(value))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut name: Option<String> = None;
                let mut description: Option<String> = None;
                let mut category: Option<Option<String>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => name = Some(map.next_value()?),
                        "description" => description = Some(map.next_value()?),
                        "category" => category = Some(map.next_value()?),
                        _ => {
                            let _: serde_json::Value = map.next_value()?;
                        }
                    }
                }

                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                Ok(SchedulerSkillRef {
                    name,
                    description: description.unwrap_or_default(),
                    category: category.unwrap_or(None),
                })
            }
        }

        deserializer.deserialize_any(SchedulerSkillRefVisitor)
    }
}

/// Per-stage capability descriptor: which skills, agents, and categories
/// this stage has access to. `None` means the stage does not delegate work
/// (e.g. RequestAnalysis, Route, Interview, Review, Synthesis, Handoff).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SchedulerStageCapabilities {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skill_list: Vec<SchedulerSkillRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Whether this stage should create an isolated child session.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub child_session: bool,
}

impl SchedulerStageCapabilities {
    pub fn is_empty(&self) -> bool {
        self.skill_list.is_empty()
            && self.agents.is_empty()
            && self.categories.is_empty()
            && !self.child_session
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerSessionProjection {
    Hidden,
    Transcript,
}

impl SchedulerSessionProjection {
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Transcript)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Transcript => "transcript",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerLoopBudget {
    Unbounded,
    StepLimit(u32),
}

impl SchedulerLoopBudget {
    pub fn label(self) -> String {
        match self {
            Self::Unbounded => "unbounded".to_string(),
            Self::StepLimit(limit) => format!("step-limit:{limit}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerStagePolicy {
    pub session_projection: SchedulerSessionProjection,
    pub tool_policy: StageToolPolicy,
    pub loop_budget: SchedulerLoopBudget,
    /// Whether this stage should create an isolated child session for its content.
    pub child_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerStageObservability {
    pub projection: String,
    pub tool_policy: String,
    pub loop_budget: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerStageSpec {
    pub kind: SchedulerStageKind,
    pub policy: SchedulerStagePolicy,
    /// Per-stage capabilities. `None` = stage does not delegate work.
    pub capabilities: Option<SchedulerStageCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerStageGraph {
    pub stages: Vec<SchedulerStageSpec>,
}

impl SchedulerStageGraph {
    pub fn new(stages: Vec<SchedulerStageSpec>) -> Self {
        Self { stages }
    }

    pub fn stage(&self, kind: SchedulerStageKind) -> Option<&SchedulerStageSpec> {
        self.stages.iter().find(|stage| stage.kind == kind)
    }

    pub fn stage_kinds(&self) -> Vec<SchedulerStageKind> {
        self.stages.iter().map(|stage| stage.kind).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerTransitionTrigger {
    OnSuccess,
    OnUserChoice(&'static str),
    OnHighAccuracyApproved,
    OnHighAccuracyBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerTransitionTarget {
    Stage(SchedulerStageKind),
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerTransitionSpec {
    pub from: SchedulerStageKind,
    pub trigger: SchedulerTransitionTrigger,
    pub to: SchedulerTransitionTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerTransitionGraph {
    pub transitions: Vec<SchedulerTransitionSpec>,
}

impl SchedulerTransitionGraph {
    pub fn new(transitions: Vec<SchedulerTransitionSpec>) -> Self {
        Self { transitions }
    }

    pub fn transitions_from(&self, stage: SchedulerStageKind) -> Vec<&SchedulerTransitionSpec> {
        self.transitions
            .iter()
            .filter(|transition| transition.from == stage)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerEffectMoment {
    OnEnter,
    OnSuccess,
    BeforeTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerEffectKind {
    EnsurePlanningArtifactPath,
    PersistPlanningArtifact,
    PersistDraftArtifact,
    SyncDraftArtifact,
    RegisterWorkflowTodos,
    RequestAdvisoryReview,
    RequestUserChoice,
    RunApprovalReviewLoop,
    DeleteDraftArtifact,
    DecorateFinalOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerEffectSpec {
    pub stage: SchedulerStageKind,
    pub moment: SchedulerEffectMoment,
    pub effect: SchedulerEffectKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerEffectProtocol {
    pub effects: Vec<SchedulerEffectSpec>,
}

impl SchedulerEffectProtocol {
    pub fn new(effects: Vec<SchedulerEffectSpec>) -> Self {
        Self { effects }
    }

    pub fn effects_for(
        &self,
        stage: SchedulerStageKind,
        moment: SchedulerEffectMoment,
    ) -> Vec<&SchedulerEffectSpec> {
        self.effects
            .iter()
            .filter(|effect| effect.stage == stage && effect.moment == moment)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerEffectContext {
    pub planning_artifact_path: Option<String>,
    pub draft_artifact_path: Option<String>,
    pub user_choice: Option<String>,
    pub review_gate_approved: Option<bool>,
    pub draft_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerHandoffDecoration {
    pub plan_path: Option<String>,
    pub draft_path: Option<String>,
    pub draft_deleted: bool,
    pub recommend_start_work: bool,
    pub review_gate_approved: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerPresetRuntimeFields<'a> {
    pub route_rationale_summary: Option<&'a str>,
    pub planning_artifact_path: Option<&'a str>,
    pub draft_artifact_path: Option<&'a str>,
    pub interviewed: Option<&'a str>,
    pub planned: Option<&'a str>,
    pub draft_snapshot: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub approval_review: Option<&'a str>,
    pub user_choice: Option<&'a str>,
    pub review_gate_approved: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerPresetRuntimeUpdate {
    Planned(String),
    AdvisoryReview(String),
    ApprovalReview(String),
    UserChoice(String),
    ReviewGateApproved(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerEffectDispatch {
    EnsurePlanningArtifactPath,
    PersistPlanningArtifact,
    SyncDraftArtifact,
    RegisterWorkflowTodos,
    RequestAdvisoryReview,
    RequestUserChoice,
    RunApprovalReviewLoop,
    DeleteDraftArtifact,
    DecorateFinalOutput(SchedulerHandoffDecoration),
    Skip,
}

#[async_trait]
pub trait SchedulerPresetEffectExecutor {
    async fn ensure_planning_artifact_path(&mut self) -> Result<(), OrchestratorError>;
    async fn persist_planning_artifact(&mut self) -> Result<(), OrchestratorError>;
    async fn sync_draft_artifact(&mut self) -> Result<(), OrchestratorError>;
    async fn register_workflow_todos(&mut self) -> Result<(), OrchestratorError>;
    async fn request_advisory_review(&mut self) -> Result<(), OrchestratorError>;
    async fn request_user_choice(&mut self) -> Result<(), OrchestratorError>;
    async fn run_approval_review_loop(&mut self) -> Result<(), OrchestratorError>;
    async fn delete_draft_artifact(&mut self) -> Result<(), OrchestratorError>;
    async fn decorate_final_output(
        &mut self,
        decoration: SchedulerHandoffDecoration,
    ) -> Result<(), OrchestratorError>;
}

pub async fn execute_scheduler_effect_dispatch<E: SchedulerPresetEffectExecutor>(
    dispatch: SchedulerEffectDispatch,
    executor: &mut E,
) -> Result<(), OrchestratorError> {
    match dispatch {
        SchedulerEffectDispatch::EnsurePlanningArtifactPath => {
            executor.ensure_planning_artifact_path().await
        }
        SchedulerEffectDispatch::PersistPlanningArtifact => {
            executor.persist_planning_artifact().await
        }
        SchedulerEffectDispatch::SyncDraftArtifact => executor.sync_draft_artifact().await,
        SchedulerEffectDispatch::RegisterWorkflowTodos => executor.register_workflow_todos().await,
        SchedulerEffectDispatch::RequestAdvisoryReview => executor.request_advisory_review().await,
        SchedulerEffectDispatch::RequestUserChoice => executor.request_user_choice().await,
        SchedulerEffectDispatch::RunApprovalReviewLoop => executor.run_approval_review_loop().await,
        SchedulerEffectDispatch::DeleteDraftArtifact => executor.delete_draft_artifact().await,
        SchedulerEffectDispatch::DecorateFinalOutput(decoration) => {
            executor.decorate_final_output(decoration).await
        }
        SchedulerEffectDispatch::Skip => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerExecutionStageDispatch {
    Direct,
    SinglePass,
    CoordinationLoop,
    AutonomousLoop,
}

#[async_trait]
pub trait SchedulerPresetExecutionStageExecutor {
    async fn execute_direct_stage(&mut self) -> Result<(), OrchestratorError>;
    async fn execute_single_pass_stage(&mut self) -> Result<(), OrchestratorError>;
    async fn execute_coordination_loop_stage(&mut self) -> Result<(), OrchestratorError>;
    async fn execute_autonomous_loop_stage(&mut self) -> Result<(), OrchestratorError>;
}

pub async fn execute_scheduler_execution_stage_dispatch<
    E: SchedulerPresetExecutionStageExecutor,
>(
    dispatch: SchedulerExecutionStageDispatch,
    executor: &mut E,
) -> Result<(), OrchestratorError> {
    match dispatch {
        SchedulerExecutionStageDispatch::Direct => executor.execute_direct_stage().await,
        SchedulerExecutionStageDispatch::SinglePass => executor.execute_single_pass_stage().await,
        SchedulerExecutionStageDispatch::CoordinationLoop => {
            executor.execute_coordination_loop_stage().await
        }
        SchedulerExecutionStageDispatch::AutonomousLoop => {
            executor.execute_autonomous_loop_stage().await
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerExecutionWorkflowKind {
    Direct,
    SinglePass,
    CoordinationLoop,
    AutonomousLoop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerExecutionChildMode {
    Parallel,
    Sequential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerExecutionVerificationMode {
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerExecutionGateStatus {
    Done,
    Continue,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SchedulerExecutionGateDecision {
    pub status: SchedulerExecutionGateStatus,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub next_input: Option<String>,
    #[serde(default)]
    pub final_response: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerExecutionWorkflowPolicy {
    pub kind: SchedulerExecutionWorkflowKind,
    pub child_mode: SchedulerExecutionChildMode,
    pub allow_execution_fallback: bool,
    pub verification_mode: SchedulerExecutionVerificationMode,
    pub max_rounds: u32,
}

impl SchedulerExecutionWorkflowPolicy {
    pub const fn direct() -> Self {
        Self {
            kind: SchedulerExecutionWorkflowKind::Direct,
            child_mode: SchedulerExecutionChildMode::Parallel,
            allow_execution_fallback: true,
            verification_mode: SchedulerExecutionVerificationMode::Optional,
            max_rounds: 1,
        }
    }

    pub const fn single_pass() -> Self {
        Self {
            kind: SchedulerExecutionWorkflowKind::SinglePass,
            child_mode: SchedulerExecutionChildMode::Sequential,
            allow_execution_fallback: false,
            verification_mode: SchedulerExecutionVerificationMode::Optional,
            max_rounds: 1,
        }
    }

    pub const fn coordination_loop(
        child_mode: SchedulerExecutionChildMode,
        allow_execution_fallback: bool,
        verification_mode: SchedulerExecutionVerificationMode,
        max_rounds: u32,
    ) -> Self {
        Self {
            kind: SchedulerExecutionWorkflowKind::CoordinationLoop,
            child_mode,
            allow_execution_fallback,
            verification_mode,
            max_rounds,
        }
    }

    pub const fn autonomous_loop(
        child_mode: SchedulerExecutionChildMode,
        allow_execution_fallback: bool,
        verification_mode: SchedulerExecutionVerificationMode,
        max_rounds: u32,
    ) -> Self {
        Self {
            kind: SchedulerExecutionWorkflowKind::AutonomousLoop,
            child_mode,
            allow_execution_fallback,
            verification_mode,
            max_rounds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerFinalizationMode {
    StandardSynthesis,
    PlannerHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerFlowDefinition {
    pub stage_graph: SchedulerStageGraph,
    pub transition_graph: SchedulerTransitionGraph,
    pub effect_protocol: SchedulerEffectProtocol,
    pub execution_workflow_policy: SchedulerExecutionWorkflowPolicy,
    pub finalization_mode: SchedulerFinalizationMode,
}

/// Maximum number of children allowed in a single dynamic parallel plan.
pub const DYNAMIC_AGENT_TREE_MAX_CHILDREN: usize = 5;

/// LLM-emitted dynamic agent tree declaration.
///
/// During `ExecutionOrchestration` the coordinating agent may output a
/// `<parallel_plan>` JSON block that defines a runtime-constructed
/// `AgentTreeNode`. The scheduler parses, validates, and stores it in
/// `SchedulerProfileState` so the *next* coordination round can execute
/// it via `AgentTreeOrchestrator` with true parallel dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicAgentTreeDeclaration {
    /// Description of the coordination / parent task.
    pub root_task: String,
    /// Parallel child agents.
    pub children: Vec<DynamicChildAgent>,
    /// How to aggregate child outputs (defaults to synthesis).
    #[serde(default)]
    pub aggregation: Option<AggregationStrategy>,
}

/// A single child agent in a dynamic parallel plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicChildAgent {
    /// Unique identifier for the child (used in output aggregation).
    pub name: String,
    /// System prompt / task description for the child.
    pub task: String,
    /// Tools the child is allowed to use (must be a subset of parent tools).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Optional model override ("provider:model" format).
    #[serde(default)]
    pub model: Option<String>,
}

/// Aggregation strategy for merging parallel child outputs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationStrategy {
    /// Parent agent synthesizes child outputs into a unified result (default).
    #[default]
    Synthesize,
    /// Concatenate child outputs verbatim.
    Concatenate,
}
