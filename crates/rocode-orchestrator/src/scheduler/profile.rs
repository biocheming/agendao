mod adapters;
mod finalization;
mod inputs;
mod parsing;
pub(super) mod presentation;

pub(super) use parsing::parse_dynamic_agent_tree;

use crate::agent_tree::{AgentTreeNode, AgentTreeOrchestrator};
use crate::iterative_workflow::IterativeWorkflowConfig;
use crate::output_metadata::{
    append_output_usage, continuation_targets, merge_output_metadata, output_usage,
    ContinuationTarget,
};
use crate::skill_graph::{SkillGraphDefinition, SkillGraphOrchestrator};
use crate::skill_tree::SkillTreeRequestPlan;
use crate::tool_runner::ToolRunner;
use crate::traits::Orchestrator;
use crate::{
    ModelRef, OrchestratorContext, OrchestratorError, OrchestratorOutput, SchedulerProfileConfig,
    SchedulerSkillRef, SchedulerStageOverride, StageToolPolicyOverride,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use self::adapters::{SchedulerEffectAdapter, SchedulerExecutionStageAdapter};
pub(in crate::scheduler) use self::inputs::RetryComposeRequest;
pub use self::parsing::{normalize_execution_gate_decision, parse_execution_gate_decision};
#[cfg(test)]
use self::presentation::render_stage_sequence;
use self::presentation::{
    parse_loop_budget, parse_session_projection, profile_prompt_suffix, render_plan_snapshot,
    skill_tree_context,
};

use super::execution::SchedulerExecutionService;
use super::execution_adapter::SchedulerExecutionCapabilityAdapter;
use super::execution_input as scheduler_execution_input;
use super::profile_state::SchedulerProfileState;
use super::prompt_support::build_capabilities_summary;

#[cfg(test)]
use super::SchedulerFinalizationMode;
use super::{
    apply_route_decision, execute_scheduler_effect_dispatch,
    execute_scheduler_execution_stage_dispatch, execute_stage_agent, parse_route_decision,
    route_system_prompt, stage_agent, stage_agent_unbounded, validate_route_decision,
    AvailableAgentMeta, AvailableCategoryMeta, RouteDecision, RouteMode,
    SchedulerAdvisoryReviewInput, SchedulerEffectContext, SchedulerEffectDispatch,
    SchedulerEffectKind, SchedulerEffectMoment, SchedulerEffectProtocol,
    SchedulerExecutionGateDecision, SchedulerExecutionGateStatus, SchedulerExecutionStageDispatch,
    SchedulerExecutionWorkflowKind, SchedulerExecutionWorkflowPolicy, SchedulerFlowDefinition,
    SchedulerHandoffDecoration, SchedulerHandoffStageInput, SchedulerInterviewStageInput,
    SchedulerLoopBudget, SchedulerPlanStageInput, SchedulerPresetDefinition,
    SchedulerPresetEffectExecutor, SchedulerPresetExecutionStageExecutor, SchedulerPresetKind,
    SchedulerReviewStageInput, SchedulerStageCapabilities, SchedulerStageGraph,
    SchedulerStagePolicy, SchedulerSynthesisStageInput, SchedulerTransitionGraph,
    SchedulerTransitionTarget, SchedulerTransitionTrigger, StageToolPolicy,
};

#[cfg(test)]
mod tests;

struct StageEffectsRequest<'a> {
    stage: SchedulerStageKind,
    moment: SchedulerEffectMoment,
    original_input: &'a str,
    state: &'a mut SchedulerProfileState,
    plan: &'a SchedulerProfilePlan,
    output: Option<&'a mut OrchestratorOutput>,
    ctx: &'a OrchestratorContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerStageKind {
    RequestAnalysis,
    Route,
    Interview,
    Plan,
    Delegation,
    Review,
    ExecutionOrchestration,
    Synthesis,
    Handoff,
}

impl SchedulerStageKind {
    fn as_event_name(self) -> &'static str {
        match self {
            SchedulerStageKind::RequestAnalysis => "request-analysis",
            SchedulerStageKind::Route => "route",
            SchedulerStageKind::Interview => "interview",
            SchedulerStageKind::Plan => "plan",
            SchedulerStageKind::Delegation => "delegation",
            SchedulerStageKind::Review => "review",
            SchedulerStageKind::ExecutionOrchestration => "execution-orchestration",
            SchedulerStageKind::Synthesis => "synthesis",
            SchedulerStageKind::Handoff => "handoff",
        }
    }

    pub fn event_name(self) -> &'static str {
        self.as_event_name()
    }

    pub fn from_event_name(value: &str) -> Option<Self> {
        match value {
            "request-analysis" => Some(Self::RequestAnalysis),
            "route" => Some(Self::Route),
            "interview" => Some(Self::Interview),
            "plan" => Some(Self::Plan),
            "delegation" => Some(Self::Delegation),
            "review" => Some(Self::Review),
            "execution-orchestration" => Some(Self::ExecutionOrchestration),
            "synthesis" => Some(Self::Synthesis),
            "handoff" => Some(Self::Handoff),
            _ => None,
        }
    }

    /// Whether this stage kind inherently delegates work and thus needs
    /// awareness of available skills, agents, and categories.
    pub fn needs_capabilities(self) -> bool {
        matches!(
            self,
            Self::Plan | Self::ExecutionOrchestration | Self::Delegation
        )
    }
}

#[derive(Debug, Clone)]
pub struct SchedulerProfilePlan {
    pub profile_name: Option<String>,
    pub orchestrator: Option<String>,
    pub description: Option<String>,
    pub model: Option<ModelRef>,
    pub stages: Vec<SchedulerStageKind>,
    pub skill_list: Vec<SchedulerSkillRef>,
    pub agent_tree: Option<AgentTreeNode>,
    pub skill_graph: Option<SkillGraphDefinition>,
    pub skill_tree: Option<SkillTreeRequestPlan>,
    pub workflow: Option<IterativeWorkflowConfig>,
    pub available_agents: Vec<AvailableAgentMeta>,
    pub available_categories: Vec<AvailableCategoryMeta>,
    /// Stage-scoped skill catalog views resolved outside the static profile JSON.
    pub stage_skill_lists: HashMap<SchedulerStageKind, Vec<SchedulerSkillRef>>,
    /// Per-stage policy overrides from JSON config.
    pub stage_overrides: HashMap<SchedulerStageKind, SchedulerStageOverride>,
}

impl SchedulerProfilePlan {
    pub fn new(stages: Vec<SchedulerStageKind>) -> Self {
        Self {
            profile_name: None,
            orchestrator: None,
            description: None,
            model: None,
            stages,
            skill_list: Vec::new(),
            agent_tree: None,
            skill_graph: None,
            skill_tree: None,
            workflow: None,
            available_agents: Vec::new(),
            available_categories: Vec::new(),
            stage_skill_lists: HashMap::new(),
            stage_overrides: HashMap::new(),
        }
    }

    pub fn from_profile_config(
        profile_name: Option<String>,
        default_stages: Vec<SchedulerStageKind>,
        profile: &SchedulerProfileConfig,
    ) -> Self {
        let stages = if profile.stages.is_empty() {
            default_stages
        } else {
            profile.stage_kinds()
        };

        let stage_overrides = profile
            .stage_overrides()
            .into_iter()
            .map(|(kind, o)| (kind, o.clone()))
            .collect();

        Self {
            profile_name,
            orchestrator: profile.orchestrator.clone(),
            description: profile.description.clone(),
            model: profile.model.clone(),
            stages,
            skill_list: profile.skill_list.clone(),
            agent_tree: profile
                .agent_tree
                .as_ref()
                .and_then(|s| s.as_inline())
                .cloned(),
            skill_graph: profile.skill_graph.clone(),
            skill_tree: profile.skill_tree.clone(),
            workflow: profile.workflow().cloned(),
            available_agents: profile.available_agents.clone(),
            available_categories: profile.available_categories.clone(),
            stage_skill_lists: HashMap::new(),
            stage_overrides,
        }
    }

    pub fn with_orchestrator(mut self, orchestrator: impl Into<String>) -> Self {
        self.orchestrator = Some(orchestrator.into());
        self
    }

    pub fn with_description(mut self, description: Option<String>) -> Self {
        self.description = description;
        self
    }

    pub fn with_skill_list(mut self, skill_list: Vec<SchedulerSkillRef>) -> Self {
        self.skill_list = skill_list;
        self
    }

    pub fn with_agent_tree(mut self, agent_tree: AgentTreeNode) -> Self {
        self.agent_tree = Some(agent_tree);
        self
    }

    pub fn with_skill_graph(mut self, skill_graph: SkillGraphDefinition) -> Self {
        self.skill_graph = Some(skill_graph);
        self
    }

    pub fn with_skill_tree(mut self, skill_tree: SkillTreeRequestPlan) -> Self {
        self.skill_tree = Some(skill_tree);
        self
    }

    pub fn with_workflow(mut self, workflow: IterativeWorkflowConfig) -> Self {
        self.workflow = Some(workflow);
        self
    }

    /// Resolve capabilities from plan-level config. Used as fallback when
    /// a stage that needs capabilities has no per-stage override.
    pub fn resolve_capabilities(&self) -> SchedulerStageCapabilities {
        SchedulerStageCapabilities {
            skill_list: self.skill_list.clone(),
            agents: self
                .available_agents
                .iter()
                .map(|a| a.name.clone())
                .collect(),
            categories: self
                .available_categories
                .iter()
                .map(|c| c.name.clone())
                .collect(),
            child_session: false,
        }
    }

    pub(super) fn stage_capabilities_override(
        &self,
        stage: SchedulerStageKind,
    ) -> Option<SchedulerStageCapabilities> {
        let overrides = self.stage_overrides.get(&stage)?;
        if overrides.agents.is_empty() && overrides.skill_list.is_empty() {
            return None;
        }

        let mut capabilities = self.resolve_capabilities();
        if !overrides.agents.is_empty() {
            capabilities.agents = overrides.agents.clone();
        }
        if !overrides.skill_list.is_empty() {
            capabilities.skill_list = overrides.skill_list.clone();
        }
        Some(capabilities)
    }

    pub fn effective_skill_list(&self, stage: Option<SchedulerStageKind>) -> &[SchedulerSkillRef] {
        if let Some(stage) = stage {
            if let Some(skill_list) = self.stage_skill_lists.get(&stage) {
                return skill_list.as_slice();
            }
            if let Some(overrides) = self.stage_overrides.get(&stage) {
                if !overrides.skill_list.is_empty() {
                    return overrides.skill_list.as_slice();
                }
            }
        }

        self.skill_list.as_slice()
    }

    pub fn has_execution_path(&self) -> bool {
        self.agent_tree.is_some()
            || self.skill_graph.is_some()
            || self
                .stages
                .iter()
                .any(|stage| !matches!(stage, SchedulerStageKind::RequestAnalysis))
    }

    pub(super) fn preset_definition(&self) -> Option<SchedulerPresetDefinition> {
        self.orchestrator
            .as_deref()
            .and_then(|value| value.parse::<SchedulerPresetKind>().ok())
            .map(SchedulerPresetKind::definition)
    }

    pub fn execution_workflow_policy(&self) -> SchedulerExecutionWorkflowPolicy {
        self.flow_definition().execution_workflow_policy
    }

    fn execution_stage_dispatch(&self) -> SchedulerExecutionStageDispatch {
        self.preset_definition()
            .unwrap_or(SchedulerPresetKind::Sisyphus.definition())
            .execution_stage_dispatch()
    }

    pub(super) fn stage_execution_semantics(&self) -> Option<SchedulerExecutionWorkflowPolicy> {
        let workflow = self.execution_workflow_policy();
        match workflow.kind {
            SchedulerExecutionWorkflowKind::CoordinationLoop
            | SchedulerExecutionWorkflowKind::AutonomousLoop => Some(workflow),
            SchedulerExecutionWorkflowKind::Direct | SchedulerExecutionWorkflowKind::SinglePass => {
                None
            }
        }
    }

    pub fn stage_policy(&self, stage: SchedulerStageKind) -> SchedulerStagePolicy {
        // Start from preset → hardcoded default chain.
        let mut policy = self
            .preset_definition()
            .map(|definition| definition.stage_policy(stage))
            .unwrap_or(
                SchedulerPresetKind::Sisyphus
                    .definition()
                    .stage_policy(stage),
            );

        // Apply per-stage JSON overrides on top.
        if let Some(overrides) = self.stage_overrides.get(&stage) {
            if let Some(tp) = overrides.tool_policy {
                policy.tool_policy = match tp {
                    StageToolPolicyOverride::AllowAll => StageToolPolicy::AllowAll,
                    StageToolPolicyOverride::AllowReadOnly => StageToolPolicy::AllowReadOnly,
                    StageToolPolicyOverride::DisableAll => StageToolPolicy::DisableAll,
                };
            }
            if let Some(ref budget_str) = overrides.loop_budget {
                policy.loop_budget = parse_loop_budget(budget_str);
            }
            if let Some(ref proj_str) = overrides.session_projection {
                policy.session_projection = parse_session_projection(proj_str);
            }
        }

        policy
    }

    /// Look up the per-stage agent tree override for a given stage.
    /// Returns `None` if no per-stage override exists.
    pub(super) fn stage_agent_tree(&self, stage: SchedulerStageKind) -> Option<&AgentTreeNode> {
        self.stage_overrides
            .get(&stage)
            .and_then(|o| o.agent_tree.as_ref())
            .and_then(|s| s.as_inline())
    }

    pub fn flow_definition(&self) -> SchedulerFlowDefinition {
        self.preset_definition()
            .unwrap_or(SchedulerPresetKind::Sisyphus.definition())
            .flow_definition(&self.stages)
    }

    fn stage_graph(&self) -> SchedulerStageGraph {
        let mut stage_graph = self.flow_definition().stage_graph;
        for stage in &mut stage_graph.stages {
            if let Some(capabilities) = self.stage_capabilities_override(stage.kind) {
                stage.capabilities = Some(capabilities);
            }
        }
        stage_graph
    }

    pub fn transition_graph(&self) -> SchedulerTransitionGraph {
        self.flow_definition().transition_graph
    }

    pub fn effect_protocol(&self) -> SchedulerEffectProtocol {
        self.flow_definition().effect_protocol
    }

    pub fn effect_dispatch(
        &self,
        effect: SchedulerEffectKind,
        context: SchedulerEffectContext,
    ) -> SchedulerEffectDispatch {
        self.preset_definition()
            .unwrap_or(SchedulerPresetKind::Sisyphus.definition())
            .resolve_effect_dispatch(effect, context)
    }

    #[cfg(test)]
    fn finalization_mode(&self) -> SchedulerFinalizationMode {
        self.flow_definition().finalization_mode
    }
}

pub struct SchedulerProfileOrchestrator {
    plan: SchedulerProfilePlan,
    tool_runner: ToolRunner,
}

impl SchedulerProfileOrchestrator {
    pub fn new(plan: SchedulerProfilePlan, tool_runner: ToolRunner) -> Self {
        Self { plan, tool_runner }
    }

    pub(super) fn tool_runner(&self) -> ToolRunner {
        self.tool_runner.clone()
    }

    async fn execute_delegation_stage(
        &self,
        input: &str,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let profile_suffix = profile_prompt_suffix(plan, Some(SchedulerStageKind::Delegation));
        let prompt = plan
            .delegation_stage_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are the scheduler delegation executor. \
                     Execute the frozen request goal faithfully. \
                     Use ROCode tools directly, and delegate only when it clearly helps. \
                     Return concrete execution results only.{}",
                    profile_suffix
                )
            });
        let stage_policy = plan.stage_policy(SchedulerStageKind::Delegation);
        execute_stage_agent(
            input,
            ctx,
            Self::stage_agent_from_policy("scheduler-delegation", prompt, stage_policy),
            stage_policy.tool_policy,
            stage_context,
        )
        .await
    }

    pub(super) async fn execute_review_stage(
        &self,
        input: &str,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        SchedulerExecutionCapabilityAdapter::new(self, plan, ctx)
            .execute_review_stage(input, stage_context)
            .await
    }

    async fn execute_direct_execution_workflow(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        SchedulerExecutionService::new(self, original_input, state, plan, ctx)
            .run_direct()
            .await
    }

    pub(super) fn compose_coordination_verification_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        round: usize,
        execution_output: &OrchestratorOutput,
    ) -> String {
        scheduler_execution_input::compose_coordination_verification_input(
            original_input,
            state,
            plan,
            round,
            execution_output,
        )
    }

    pub(super) fn compose_coordination_gate_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        round: usize,
        execution_output: &OrchestratorOutput,
        review_output: Option<&OrchestratorOutput>,
    ) -> String {
        scheduler_execution_input::compose_coordination_gate_input(
            original_input,
            state,
            plan,
            round,
            execution_output,
            review_output,
        )
    }

    pub(super) fn compose_autonomous_verification_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        round: usize,
        execution_output: &OrchestratorOutput,
    ) -> String {
        scheduler_execution_input::compose_autonomous_verification_input(
            original_input,
            state,
            plan,
            round,
            execution_output,
        )
    }

    pub(super) fn compose_autonomous_gate_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        round: usize,
        execution_output: &OrchestratorOutput,
        verification_output: Option<&OrchestratorOutput>,
    ) -> String {
        scheduler_execution_input::compose_autonomous_gate_input(
            original_input,
            state,
            plan,
            round,
            execution_output,
            verification_output,
        )
    }

    pub(super) fn compose_retry_input(&self, input: RetryComposeRequest<'_>) -> String {
        let RetryComposeRequest {
            original_input,
            state,
            plan,
            round,
            decision,
            previous_output,
            review_output,
        } = input;
        scheduler_execution_input::compose_retry_input(
            original_input,
            state,
            plan,
            round,
            decision,
            previous_output,
            review_output,
        )
    }

    async fn execute_coordination_execution_workflow(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        SchedulerExecutionService::new(self, original_input, state, plan, ctx)
            .run_coordination_loop()
            .await
    }

    async fn execute_autonomous_execution_workflow(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        SchedulerExecutionService::new(self, original_input, state, plan, ctx)
            .run_autonomous_loop()
            .await
    }

    pub(super) fn stage_agent_from_policy(
        name: &str,
        system_prompt: String,
        policy: SchedulerStagePolicy,
    ) -> crate::AgentDescriptor {
        match policy.loop_budget {
            SchedulerLoopBudget::Unbounded => stage_agent_unbounded(name, system_prompt),
            SchedulerLoopBudget::StepLimit(max_steps) => {
                stage_agent(name, system_prompt, max_steps)
            }
        }
    }

    async fn execute_single_pass_execution_workflow(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        SchedulerExecutionService::new(self, original_input, state, plan, ctx)
            .run_single_pass()
            .await
    }

    async fn execute_execution_stage(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        let mut adapter = SchedulerExecutionStageAdapter {
            orchestrator: self,
            original_input,
            state,
            plan,
            ctx,
        };
        execute_scheduler_execution_stage_dispatch(plan.execution_stage_dispatch(), &mut adapter)
            .await
    }

    async fn emit_stage_start(
        plan: &SchedulerProfilePlan,
        stage: SchedulerStageKind,
        stage_index: u32,
        ctx: &OrchestratorContext,
    ) {
        if !plan
            .stage_graph()
            .stage(stage)
            .map(|spec| spec.policy.session_projection.is_visible())
            .unwrap_or(false)
        {
            return;
        }

        // Resolve per-stage capabilities:
        // 1. If the stage spec has explicit capabilities, use those.
        // 2. Otherwise, for stages that delegate work (Plan, ExecutionOrchestration,
        //    Delegation), inherit from plan-level config.
        // 3. For stages that don't delegate (RequestAnalysis, Route, Interview,
        //    Review, Synthesis, Handoff), capabilities is None.
        let mut capabilities = plan
            .stage_graph()
            .stage(stage)
            .and_then(|spec| spec.capabilities.clone())
            .or_else(|| {
                if stage.needs_capabilities() {
                    Some(plan.resolve_capabilities())
                } else {
                    None
                }
            });

        // Propagate child_session policy into capabilities so the lifecycle hook
        // can create an isolated child session for this stage.
        let child_session = plan
            .stage_graph()
            .stage(stage)
            .map(|spec| spec.policy.child_session)
            .unwrap_or(false);
        if child_session {
            capabilities
                .get_or_insert_with(SchedulerStageCapabilities::default)
                .child_session = true;
        }

        ctx.lifecycle_hook
            .on_scheduler_stage_start(
                &ctx.exec_ctx.agent_name,
                stage.as_event_name(),
                stage_index,
                capabilities.as_ref(),
                &ctx.exec_ctx,
            )
            .await;
    }

    async fn emit_stage_end(
        plan: &SchedulerProfilePlan,
        stage: SchedulerStageKind,
        stage_index: u32,
        output: &OrchestratorOutput,
        ctx: &OrchestratorContext,
    ) {
        if !plan
            .stage_graph()
            .stage(stage)
            .map(|spec| spec.policy.session_projection.is_visible())
            .unwrap_or(false)
        {
            return;
        }
        let stage_total = plan
            .stage_graph()
            .stages
            .iter()
            .filter(|spec| spec.policy.session_projection.is_visible())
            .count() as u32;
        ctx.lifecycle_hook
            .on_scheduler_stage_end(
                &ctx.exec_ctx.agent_name,
                stage.as_event_name(),
                stage_index,
                stage_total,
                &output.content,
                &ctx.exec_ctx,
            )
            .await;
    }

    fn execution_stage_output(state: &SchedulerProfileState) -> Option<&OrchestratorOutput> {
        state
            .execution
            .delegated
            .as_ref()
            .or(state.execution.reviewed.as_ref())
    }

    pub(super) fn retry_budget_exhausted_output(
        plan: &SchedulerProfilePlan,
        round: usize,
        max_rounds: usize,
        decision: &SchedulerExecutionGateDecision,
        previous_output: &OrchestratorOutput,
        review_output: Option<&OrchestratorOutput>,
    ) -> OrchestratorOutput {
        let retry_focus = decision
            .next_input
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                let summary = decision.summary.trim();
                if summary.is_empty() {
                    "collect the missing proof for the unresolved gap"
                } else {
                    summary
                }
            });
        let orchestrator = plan.orchestrator.as_deref().unwrap_or("scheduler");
        let mut content = format!(
            "## Delivery Summary\n- {orchestrator} exhausted its bounded retry budget after round {round}/{max_rounds}.\n\n**Retry Status**\n- Retry budget exhausted.\n\n**Retry Focus**\n- {retry_focus}"
        );
        if !decision.summary.trim().is_empty() {
            content.push_str(&format!(
                "\n\n**Blockers or Risks**\n- {}",
                decision.summary.trim()
            ));
        }
        if let Some(review_output) = review_output {
            if !review_output.content.trim().is_empty() {
                content.push_str(&format!(
                    "\n\n**Verification**\n{}",
                    review_output.content.trim()
                ));
            }
        } else if !previous_output.content.trim().is_empty() {
            content.push_str(&format!(
                "\n\n**Verification**\n- Last execution output is preserved below.\n\n{}",
                previous_output.content.trim()
            ));
        }

        let mut metadata = previous_output.metadata.clone();
        if let Some(review_output) = review_output {
            merge_output_metadata(&mut metadata, &review_output.metadata);
        }
        metadata.insert(
            "scheduler_retry_budget_exhausted".to_string(),
            serde_json::json!(true),
        );
        metadata.insert(
            "scheduler_retry_round".to_string(),
            serde_json::json!(round),
        );
        metadata.insert(
            "scheduler_retry_limit".to_string(),
            serde_json::json!(max_rounds),
        );

        OrchestratorOutput {
            content,
            steps: previous_output.steps + review_output.map(|output| output.steps).unwrap_or(0),
            tool_calls_count: previous_output.tool_calls_count
                + review_output
                    .map(|output| output.tool_calls_count)
                    .unwrap_or(0),
            metadata,
            finish_reason: crate::runtime::events::FinishReason::EndTurn,
        }
    }

    pub(super) fn gate_terminal_output(
        plan: &SchedulerProfilePlan,
        status: SchedulerExecutionGateStatus,
        decision: &SchedulerExecutionGateDecision,
        fallback_output: &OrchestratorOutput,
        supplemental_output: Option<&OrchestratorOutput>,
    ) -> Option<OrchestratorOutput> {
        plan.resolve_gate_terminal_content(status, decision, &fallback_output.content)
            .map(|content| {
                let mut output = fallback_output.clone();
                output.content = content;
                if let Some(supplemental_output) = supplemental_output {
                    merge_output_metadata(&mut output.metadata, &supplemental_output.metadata);
                    output.steps += supplemental_output.steps;
                    output.tool_calls_count += supplemental_output.tool_calls_count;
                }
                output
            })
    }

    pub(super) fn record_output(state: &mut SchedulerProfileState, output: &OrchestratorOutput) {
        state.metrics.total_steps += output.steps;
        state.metrics.total_tool_calls += output.tool_calls_count;
        if let Some(usage) = output_usage(&output.metadata) {
            state.metrics.usage.accumulate(&usage);
        }
        if output.is_cancelled() {
            state.is_cancelled = true;
        }
    }

    pub(super) fn sync_preset_runtime_authority(
        plan: &SchedulerProfilePlan,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) {
        plan.sync_runtime_authority(&mut state.preset_runtime, ctx);
        Self::sanitize_runtime_artifact_paths(plan, state, ctx);
    }

    fn sanitize_runtime_artifact_paths(
        plan: &SchedulerProfilePlan,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) {
        if let Some(path) = state.preset_runtime.planning_artifact_path.clone() {
            if let Err(error) = plan.validate_runtime_artifact_path(&path, &ctx.exec_ctx) {
                tracing::warn!(error = %error, path = %path, orchestrator = ?plan.orchestrator, "scheduler planning artifact path rejected by runtime authority");
                state.preset_runtime.planning_artifact_path = None;
                state.preset_runtime.planned = None;
                state.preset_runtime.ground_truth_context = None;
            }
        }

        if let Some(path) = state.preset_runtime.draft_artifact_path.clone() {
            if let Err(error) = plan.validate_runtime_artifact_path(&path, &ctx.exec_ctx) {
                tracing::warn!(error = %error, path = %path, orchestrator = ?plan.orchestrator, "scheduler draft artifact path rejected by runtime authority");
                state.preset_runtime.draft_artifact_path = None;
                state.preset_runtime.draft_snapshot = None;
            }
        }
    }

    pub(super) fn retry_continuation_targets(
        previous_output: &OrchestratorOutput,
        review_output: Option<&OrchestratorOutput>,
    ) -> Vec<ContinuationTarget> {
        let mut metadata = previous_output.metadata.clone();
        if let Some(review_output) = review_output {
            merge_output_metadata(&mut metadata, &review_output.metadata);
        }
        continuation_targets(&metadata)
    }

    pub(super) fn render_retry_continuation_candidates(
        targets: &[ContinuationTarget],
    ) -> Option<String> {
        let rendered = targets
            .iter()
            .map(|target| {
                let mut parts = vec![format!("session_id: `{}`", target.session_id)];
                if let Some(agent_task_id) = target.agent_task_id.as_deref() {
                    parts.push(format!("agent_task_id: `{agent_task_id}`"));
                }
                if let Some(tool_name) = target.tool_name.as_deref() {
                    parts.push(format!("tool: `{tool_name}`"));
                }
                format!("- {}", parts.join(" | "))
            })
            .collect::<Vec<_>>()
            .join("\n");
        (!rendered.is_empty()).then_some(rendered)
    }

    async fn execute_orchestration_tool(
        tool_name: &str,
        arguments: serde_json::Value,
        plan: &SchedulerProfilePlan,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Result<crate::ToolOutput, OrchestratorError> {
        plan.validate_runtime_orchestration_tool(tool_name)
            .map_err(|error| OrchestratorError::ToolError {
                tool: tool_name.to_string(),
                error,
            })?;
        let output = ctx
            .tool_executor
            .execute(tool_name, arguments, &ctx.exec_ctx)
            .await
            .map_err(|error| OrchestratorError::ToolError {
                tool: tool_name.to_string(),
                error: error.to_string(),
            })?;
        state.metrics.total_tool_calls += 1;

        if output.is_error {
            return Err(OrchestratorError::ToolError {
                tool: tool_name.to_string(),
                error: output.output.clone(),
            });
        }

        Ok(output)
    }

    #[cfg(test)]
    fn plan_start_work_command(plan_path: Option<&str>) -> String {
        crate::scheduler::plan_start_work_command(plan_path)
    }

    async fn execute_resolved_agent(
        &self,
        name: &str,
        input: &str,
        ctx: &OrchestratorContext,
        policy: StageToolPolicy,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let agent = ctx
            .agent_resolver
            .resolve(name)
            .ok_or_else(|| OrchestratorError::AgentNotFound(name.to_string()))?;
        execute_stage_agent(input, ctx, agent, policy, stage_context).await
    }

    async fn register_scheduler_workflow_todos(
        &self,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) {
        if state.preset_runtime.workflow_todos_registered {
            return;
        }

        let Some(payload) = plan.workflow_todos_payload() else {
            return;
        };

        match Self::execute_orchestration_tool("todowrite", payload, plan, state, ctx).await {
            Ok(_) => {
                state.preset_runtime.workflow_todos_registered = true;
            }
            Err(error) => {
                tracing::warn!(error = %error, orchestrator = ?plan.orchestrator, "scheduler workflow todo registration failed; continuing");
            }
        }
    }

    async fn request_capability_advisory_review(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) {
        let Some(agent_name) = plan.advisory_agent_name() else {
            return;
        };
        let Some(advisory_input) =
            plan.compose_advisory_review_input(SchedulerAdvisoryReviewInput {
                goal: &state.route.request_brief,
                original_request: original_input,
                discussed: state.route.interviewed.as_deref(),
                draft_context: state.preset_runtime.draft_snapshot.as_deref(),
                research: state.route.routed.as_deref(),
            })
        else {
            return;
        };
        match self
            .execute_resolved_agent(
                agent_name,
                &advisory_input,
                ctx,
                StageToolPolicy::AllowReadOnly,
                None,
            )
            .await
        {
            Ok(output) => {
                Self::record_output(state, &output);
                if let Some(update) =
                    plan.runtime_update_for_advisory_review(output.content.clone())
                {
                    state.apply_runtime_update(update);
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, orchestrator = ?plan.orchestrator, "preset advisory review failed; continuing without advisory feedback");
            }
        }
    }

    async fn request_capability_user_choice(
        &self,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> String {
        let Some(payload) = plan.user_choice_payload() else {
            return String::new();
        };
        let default_choice = plan.default_user_choice().unwrap_or("").to_string();

        let answer = match Self::execute_orchestration_tool("question", payload, plan, state, ctx)
            .await
        {
            Ok(output) => plan
                .parse_user_choice(&output.output)
                .unwrap_or_else(|| default_choice.clone()),
            Err(error) => {
                tracing::warn!(error = %error, orchestrator = ?plan.orchestrator, "preset user choice prompt failed; defaulting to configured choice");
                default_choice
            }
        };

        if let Some(update) = plan.runtime_update_for_user_choice(answer.clone()) {
            state.apply_runtime_update(update);
        }
        answer
    }

    async fn run_capability_approval_review_loop(
        &self,
        original_input: &str,
        state: &mut SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
    ) -> bool {
        let Some(plan_path) = state.preset_runtime.planning_artifact_path.clone() else {
            tracing::warn!(orchestrator = ?plan.orchestrator, "preset approval review loop skipped because no plan artifact path was available");
            return false;
        };
        let Some(agent_name) = plan.approval_review_agent_name() else {
            return false;
        };
        let Some(max_rounds) = plan.max_approval_review_rounds() else {
            return false;
        };

        for round in 1..=max_rounds {
            let review = match self
                .execute_resolved_agent(
                    agent_name,
                    &plan_path,
                    ctx,
                    StageToolPolicy::AllowReadOnly,
                    None,
                )
                .await
            {
                Ok(output) => output,
                Err(error) => {
                    tracing::warn!(error = %error, round, orchestrator = ?plan.orchestrator, "preset approval review agent failed");
                    return false;
                }
            };
            Self::record_output(state, &review);
            if let Some(update) = plan.runtime_update_for_approval_review(review.content.clone()) {
                state.apply_runtime_update(update);
            }
            if let Err(error) = Self::sync_runtime_draft_artifact(original_input, plan, state, ctx)
            {
                tracing::warn!(error = %error, round, orchestrator = ?plan.orchestrator, "failed to sync draft after approval review loop");
            }

            if plan.approval_review_is_accepted(&review.content) {
                return true;
            }

            if round == max_rounds {
                break;
            }

            match self
                .execute_plan_stage(original_input, state, plan, ctx, None)
                .await
            {
                Ok(output) => {
                    Self::record_output(state, &output);
                    if let Some(update) =
                        plan.runtime_update_for_planned_output(output.content.clone())
                    {
                        state.apply_runtime_update(update);
                    }
                    if let Err(error) =
                        Self::persist_planning_artifact(plan, &output.content, state, ctx)
                    {
                        tracing::warn!(error = %error, round, orchestrator = ?plan.orchestrator, "failed to persist regenerated plan after approval review loop");
                    }
                    if let Err(error) =
                        Self::sync_runtime_draft_artifact(original_input, plan, state, ctx)
                    {
                        tracing::warn!(error = %error, round, orchestrator = ?plan.orchestrator, "failed to sync draft after plan regeneration");
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, round, orchestrator = ?plan.orchestrator, "preset approval review loop failed to regenerate plan");
                    return false;
                }
            }
        }

        false
    }

    fn scheduler_effect_context(
        state: &SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> SchedulerEffectContext {
        let draft_exists = state
            .preset_runtime
            .draft_artifact_path
            .as_deref()
            .map(|path| Path::new(&ctx.exec_ctx.workdir).join(path).exists())
            .unwrap_or(false);
        SchedulerEffectContext {
            planning_artifact_path: state.preset_runtime.planning_artifact_path.clone(),
            draft_artifact_path: state.preset_runtime.draft_artifact_path.clone(),
            user_choice: state.preset_runtime.user_choice.clone(),
            review_gate_approved: state.preset_runtime.review_gate_approved,
            draft_exists,
        }
    }

    async fn run_stage_effects(
        &self,
        input: StageEffectsRequest<'_>,
    ) -> Result<(), OrchestratorError> {
        let StageEffectsRequest {
            stage,
            moment,
            original_input,
            state,
            plan,
            mut output,
            ctx,
        } = input;
        for effect in plan.effect_protocol().effects_for(stage, moment) {
            let dispatch =
                plan.effect_dispatch(effect.effect, Self::scheduler_effect_context(state, ctx));
            if dispatch != SchedulerEffectDispatch::Skip || plan.effect_dispatch_is_authoritative()
            {
                {
                    let mut adapter = SchedulerEffectAdapter {
                        orchestrator: self,
                        original_input,
                        state,
                        plan,
                        output: &mut output,
                        ctx,
                        stage,
                    };
                    execute_scheduler_effect_dispatch(dispatch, &mut adapter).await?;
                }
                continue;
            }

            match effect.effect {
                SchedulerEffectKind::EnsurePlanningArtifactPath => {
                    let _ = Self::ensure_planning_artifact_path(plan, state, ctx);
                }
                SchedulerEffectKind::PersistPlanningArtifact => {
                    if let Some(output) = output.as_ref() {
                        Self::persist_planning_artifact(plan, &output.content, state, ctx)?;
                    }
                }
                SchedulerEffectKind::PersistDraftArtifact
                | SchedulerEffectKind::SyncDraftArtifact => {
                    if let Err(error) =
                        Self::sync_runtime_draft_artifact(original_input, plan, state, ctx)
                    {
                        tracing::warn!(error = %error, stage = stage.as_event_name(), "scheduler effect failed to sync preset draft artifact");
                    }
                }
                SchedulerEffectKind::RegisterWorkflowTodos => {
                    self.register_scheduler_workflow_todos(state, plan, ctx)
                        .await;
                }
                SchedulerEffectKind::RequestAdvisoryReview => {
                    self.request_capability_advisory_review(original_input, state, plan, ctx)
                        .await;
                }
                SchedulerEffectKind::RequestUserChoice
                | SchedulerEffectKind::RunApprovalReviewLoop
                | SchedulerEffectKind::DeleteDraftArtifact
                | SchedulerEffectKind::DecorateFinalOutput => {}
            }
        }
        Ok(())
    }

    fn resolve_transition_target(
        &self,
        stage: SchedulerStageKind,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> SchedulerTransitionTarget {
        let transition_graph = plan.transition_graph();
        let transitions = transition_graph.transitions_from(stage);

        if let Some(target) = plan.resolve_runtime_transition_target(
            &transitions,
            state.preset_runtime.user_choice.as_deref(),
            state.preset_runtime.review_gate_approved,
        ) {
            return target;
        }

        transitions
            .iter()
            .find(|transition| transition.trigger == SchedulerTransitionTrigger::OnSuccess)
            .map(|transition| transition.to)
            .unwrap_or(SchedulerTransitionTarget::Finish)
    }

    fn next_stage_index(
        &self,
        stage: SchedulerStageKind,
        current_index: usize,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> Option<usize> {
        match self.resolve_transition_target(stage, state, plan) {
            SchedulerTransitionTarget::Finish => None,
            SchedulerTransitionTarget::Stage(target) => plan
                .stages
                .iter()
                .position(|candidate| *candidate == target)
                .or_else(|| {
                    let fallback = current_index + 1;
                    (fallback < plan.stages.len()).then_some(fallback)
                }),
        }
    }

    async fn execute_route_stage(
        &self,
        original_input: &str,
        request_brief: &str,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        session_context: Option<&str>,
        stage_context: Option<(String, u32)>,
    ) -> Result<(OrchestratorOutput, RouteDecision), OrchestratorError> {
        let input = self.compose_route_input(original_input, request_brief, plan, session_context);
        let stage_policy = plan.stage_policy(SchedulerStageKind::Route);
        let output = execute_stage_agent(
            &input,
            ctx,
            Self::stage_agent_from_policy(
                "scheduler-route",
                route_system_prompt().to_string(),
                stage_policy,
            ),
            stage_policy.tool_policy,
            stage_context,
        )
        .await?;
        let decision = parse_route_decision(&output.content).ok_or_else(|| {
            OrchestratorError::Other(
                "route stage did not return a valid RouteDecision JSON".to_string(),
            )
        })?;
        validate_route_decision(&decision).map_err(|error| {
            OrchestratorError::Other(format!(
                "route stage returned invalid RouteDecision: {error}"
            ))
        })?;
        Ok((output, decision))
    }

    async fn execute_interview_stage(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let input = self.compose_interview_input(original_input, state, plan);
        let profile_suffix = profile_prompt_suffix(plan, Some(SchedulerStageKind::Interview));
        let prompt = plan
            .interview_stage_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are the scheduler interview layer. Clarify the request enough for planning with read-only inspection first, then return a concise planning brief.{}",
                    profile_suffix
                )
            });
        let stage_policy = plan.stage_policy(SchedulerStageKind::Interview);
        execute_stage_agent(
            &input,
            ctx,
            Self::stage_agent_from_policy("scheduler-interview", prompt, stage_policy),
            stage_policy.tool_policy,
            stage_context,
        )
        .await
    }

    async fn execute_plan_stage(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let input = self.compose_plan_input(original_input, state, plan);
        let profile_suffix = profile_prompt_suffix(plan, Some(SchedulerStageKind::Plan));
        let prompt = plan
            .plan_stage_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are ROCode's planning stage. \
                     Ask the planning questions internally, inspect the codebase with read-only tools when needed, \
                     and return a concrete execution plan. Keep the output practical: assumptions, ordered steps, \
                     verification, and risks. Never claim the task is already implemented.{}",
                    profile_suffix
                )
            });
        let stage_policy = plan.stage_policy(SchedulerStageKind::Plan);
        execute_stage_agent(
            &input,
            ctx,
            Self::stage_agent_from_policy("scheduler-plan", prompt, stage_policy),
            stage_policy.tool_policy,
            stage_context,
        )
        .await
    }

    async fn execute_handoff_stage(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let input = self.compose_handoff_input(original_input, state, plan);
        let profile_suffix = profile_prompt_suffix(plan, Some(SchedulerStageKind::Handoff));
        let prompt = plan
            .handoff_stage_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are the scheduler handoff layer. Produce a concise next-step handoff without claiming work was executed beyond the evidence in prior stages.{}",
                    profile_suffix
                )
            });
        let stage_policy = plan.stage_policy(SchedulerStageKind::Handoff);
        execute_stage_agent(
            &input,
            ctx,
            Self::stage_agent_from_policy("scheduler-handoff", prompt, stage_policy),
            stage_policy.tool_policy,
            stage_context,
        )
        .await
    }

    async fn execute_synthesis_stage(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
        ctx: &OrchestratorContext,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let input = self.compose_synthesis_input(original_input, state, plan);
        let profile_suffix = profile_prompt_suffix(plan, Some(SchedulerStageKind::Synthesis));
        let prompt = plan
            .synthesis_stage_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are the final synthesis layer for ROCode's scheduler. \
                     Merge prior stage outputs into a single final response for the user. \
                     Keep the answer faithful to actual stage results. \
                     Do not invent edits, tool calls, or conclusions. \
                     If there are remaining risks or follow-ups, state them clearly.{}",
                    profile_suffix
                )
            });
        let stage_policy = plan.stage_policy(SchedulerStageKind::Synthesis);
        execute_stage_agent(
            &input,
            ctx,
            Self::stage_agent_from_policy("scheduler-synthesis", prompt, stage_policy),
            stage_policy.tool_policy,
            stage_context,
        )
        .await
    }
}

#[async_trait]
impl Orchestrator for SchedulerProfileOrchestrator {
    async fn execute(
        &mut self,
        input: &str,
        ctx: &OrchestratorContext,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        if !self.plan.has_execution_path() {
            return Err(OrchestratorError::Other(
                "scheduler profile requires at least one execution dimension".to_string(),
            ));
        }

        let mut resolved_plan = self.plan.clone();
        let mut state = SchedulerProfileState::default();
        state.apply_session_context_metadata(&ctx.exec_ctx.metadata);
        if Self::resolve_artifact_relative_path(
            &resolved_plan,
            SchedulerArtifactKind::Draft,
            &ctx.exec_ctx.session_id,
        )
        .is_some()
        {
            let _ = Self::ensure_artifact_path(
                &resolved_plan,
                SchedulerArtifactKind::Draft,
                &mut state,
                ctx,
            );
            state.preset_runtime.draft_snapshot = Self::load_artifact_snapshot(
                &resolved_plan,
                SchedulerArtifactKind::Draft,
                &mut state,
                ctx,
            );
        }
        Self::sync_preset_runtime_authority(&self.plan, &mut state, ctx);
        let mut stage_idx = 0usize;

        while stage_idx < resolved_plan.stages.len() {
            let stage = resolved_plan.stages[stage_idx];
            let stage_ordinal = stage_idx as u32 + 1;
            Self::emit_stage_start(&resolved_plan, stage, stage_ordinal, ctx).await;
            match stage {
                SchedulerStageKind::RequestAnalysis => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    Self::emit_stage_end(
                        &resolved_plan,
                        stage,
                        stage_ordinal,
                        &OrchestratorOutput::empty(),
                        ctx,
                    )
                    .await;
                }
                SchedulerStageKind::Route => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    match self
                        .execute_route_stage(
                            input,
                            &state.route.request_brief,
                            &resolved_plan,
                            ctx,
                            state.session_context.as_deref(),
                            Some((
                                SchedulerStageKind::Route.as_event_name().to_string(),
                                stage_ordinal,
                            )),
                        )
                        .await
                    {
                        Ok((output, decision)) => {
                            Self::record_output(&mut state, &output);
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &output,
                                ctx,
                            )
                            .await;
                            state.route.routed = Some(output.content.clone());

                            tracing::info!(
                                mode = ?decision.mode,
                                preset = decision.preset.as_deref().unwrap_or("<inherit>"),
                                rationale = %decision.rationale_summary,
                                "route stage resolved request-scoped plan"
                            );

                            let decision = resolved_plan.constrain_route_decision(decision);

                            match decision.mode {
                                RouteMode::Direct => {
                                    let reply = decision
                                        .direct_response
                                        .clone()
                                        .filter(|s| !s.trim().is_empty())
                                        .unwrap_or_else(|| output.content.clone());

                                    state.route.route_decision = Some(decision);
                                    state.route.direct_response = Some(reply.clone());

                                    return Ok(OrchestratorOutput {
                                        content: reply,
                                        steps: state.metrics.total_steps,
                                        tool_calls_count: state.metrics.total_tool_calls,
                                        metadata: {
                                            let mut metadata = HashMap::new();
                                            if !state.metrics.usage.is_zero() {
                                                append_output_usage(
                                                    &mut metadata,
                                                    &state.metrics.usage,
                                                );
                                            }
                                            metadata
                                        },
                                        finish_reason:
                                            crate::runtime::events::FinishReason::EndTurn,
                                    });
                                }
                                RouteMode::Orchestrate => {
                                    apply_route_decision(&mut resolved_plan, stage_idx, &decision);
                                    state.route.route_decision = Some(decision);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "route stage failed; keeping original plan");
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &OrchestratorOutput::empty(),
                                ctx,
                            )
                            .await;
                        }
                    }
                }
                SchedulerStageKind::Interview => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    let mut interview_output: Option<OrchestratorOutput> = None;
                    let interview_result: Result<(), _> = async {
                        self.run_stage_effects(StageEffectsRequest {
                            stage,
                            moment: SchedulerEffectMoment::OnEnter,
                            original_input: input,
                            state: &mut state,
                            plan: &resolved_plan,
                            output: None,
                            ctx,
                        })
                        .await?;
                        match self
                            .execute_interview_stage(
                                input,
                                &state,
                                &resolved_plan,
                                ctx,
                                Some((
                                    SchedulerStageKind::Interview.as_event_name().to_string(),
                                    stage_ordinal,
                                )),
                            )
                            .await
                        {
                            Ok(output) => {
                                Self::record_output(&mut state, &output);
                                state.route.interviewed = Some(output.content.clone());
                                self.run_stage_effects(StageEffectsRequest {
                                    stage,
                                    moment: SchedulerEffectMoment::OnSuccess,
                                    original_input: input,
                                    state: &mut state,
                                    plan: &resolved_plan,
                                    output: None,
                                    ctx,
                                })
                                .await?;
                                interview_output = Some(output);
                            }
                            Err(err) => {
                                tracing::warn!(error = %err, "interview stage failed; continuing without explicit interview brief");
                            }
                        }
                        Ok(())
                    }.await;
                    let empty_fallback = OrchestratorOutput::empty();
                    let stage_out = interview_output.as_ref().unwrap_or(&empty_fallback);
                    Self::emit_stage_end(&resolved_plan, stage, stage_ordinal, stage_out, ctx)
                        .await;
                    interview_result?;
                }
                SchedulerStageKind::Plan => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    let mut plan_output: Option<OrchestratorOutput> = None;
                    let plan_result: Result<(), _> = async {
                        self.run_stage_effects(StageEffectsRequest {
                            stage,
                            moment: SchedulerEffectMoment::OnEnter,
                            original_input: input,
                            state: &mut state,
                            plan: &resolved_plan,
                            output: None,
                            ctx,
                        })
                        .await?;
                        match self
                            .execute_plan_stage(
                                input,
                                &state,
                                &resolved_plan,
                                ctx,
                                Some((
                                    SchedulerStageKind::Plan.as_event_name().to_string(),
                                    stage_ordinal,
                                )),
                            )
                            .await
                        {
                            Ok(output) => {
                                Self::record_output(&mut state, &output);
                                if let Some(update) = resolved_plan
                                    .runtime_update_for_planned_output(output.content.clone())
                                {
                                    state.apply_runtime_update(update);
                                }
                                let mut effect_output = output.clone();
                                self.run_stage_effects(StageEffectsRequest {
                                    stage,
                                    moment: SchedulerEffectMoment::OnSuccess,
                                    original_input: input,
                                    state: &mut state,
                                    plan: &resolved_plan,
                                    output: Some(&mut effect_output),
                                    ctx,
                                })
                                .await?;
                                plan_output = Some(effect_output);
                            }
                            Err(err) => {
                                tracing::warn!(error = %err, "plan stage failed; continuing without explicit plan");
                            }
                        }
                        Ok(())
                    }.await;
                    let empty_fallback = OrchestratorOutput::empty();
                    let stage_out = plan_output.as_ref().unwrap_or(&empty_fallback);
                    Self::emit_stage_end(&resolved_plan, stage, stage_ordinal, stage_out, ctx)
                        .await;
                    plan_result?;
                }
                SchedulerStageKind::Delegation => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    let delegation_input =
                        self.compose_delegation_input(input, &state, &resolved_plan);
                    let delegation_result: Result<OrchestratorOutput, _> =
                        if let Some(agent_tree) = &resolved_plan.agent_tree {
                            let mut tree = AgentTreeOrchestrator::new(
                                agent_tree.clone(),
                                self.tool_runner.clone(),
                            );
                            tree.execute(&delegation_input, ctx).await
                        } else {
                            self.execute_delegation_stage(
                                &delegation_input,
                                &resolved_plan,
                                ctx,
                                Some((
                                    SchedulerStageKind::Delegation.as_event_name().to_string(),
                                    stage_ordinal,
                                )),
                            )
                            .await
                        };
                    match delegation_result {
                        Ok(output) => {
                            Self::record_output(&mut state, &output);
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &output,
                                ctx,
                            )
                            .await;
                            state.execution.delegated = Some(output);
                        }
                        Err(err) => {
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &OrchestratorOutput::empty(),
                                ctx,
                            )
                            .await;
                            return Err(err);
                        }
                    }
                }
                SchedulerStageKind::ExecutionOrchestration => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }

                    match self
                        .execute_execution_stage(input, &mut state, &resolved_plan, ctx)
                        .await
                    {
                        Ok(()) => {
                            let output = Self::execution_stage_output(&state)
                                .cloned()
                                .unwrap_or_else(OrchestratorOutput::empty);
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &output,
                                ctx,
                            )
                            .await;
                        }
                        Err(err) => {
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &OrchestratorOutput::empty(),
                                ctx,
                            )
                            .await;
                            return Err(err);
                        }
                    }
                }
                SchedulerStageKind::Review => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    let review_input = self.compose_review_input(input, &state, &resolved_plan);
                    let review_result: Result<Option<OrchestratorOutput>, _> =
                        if Self::resolve_artifact_relative_path(
                            &resolved_plan,
                            SchedulerArtifactKind::Draft,
                            &ctx.exec_ctx.session_id,
                        )
                        .is_some()
                        {
                            let review_stage_context = Some((
                                SchedulerStageKind::Review.as_event_name().to_string(),
                                stage_ordinal,
                            ));
                            match self
                                .execute_review_stage(
                                    &review_input,
                                    &resolved_plan,
                                    ctx,
                                    review_stage_context,
                                )
                                .await
                            {
                                Ok(output) => {
                                    Self::record_output(&mut state, &output);
                                    let mut normalized = output.clone();
                                    normalized.content = resolved_plan
                                        .normalize_review_stage_output(
                                            state.preset_runtime_fields(),
                                            &output.content,
                                        )
                                        .unwrap_or_else(|| output.content.clone());
                                    state.execution.reviewed = Some(normalized.clone());
                                    Ok(Some(normalized))
                                }
                                Err(e) => Err(e),
                            }
                        } else if let Some(skill_graph) = &resolved_plan.skill_graph {
                            let mut graph = SkillGraphOrchestrator::new(
                                skill_graph.clone(),
                                self.tool_runner.clone(),
                            );
                            match graph.execute(&review_input, ctx).await {
                                Ok(output) => {
                                    Self::record_output(&mut state, &output);
                                    state.execution.reviewed = Some(output.clone());
                                    Ok(Some(output))
                                }
                                Err(e) => Err(e),
                            }
                        } else if state.execution.delegated.is_some() {
                            match self
                                .execute_review_stage(
                                    &review_input,
                                    &resolved_plan,
                                    ctx,
                                    Some((
                                        SchedulerStageKind::Review.as_event_name().to_string(),
                                        stage_ordinal,
                                    )),
                                )
                                .await
                            {
                                Ok(output) => {
                                    Self::record_output(&mut state, &output);
                                    state.execution.reviewed = Some(output.clone());
                                    Ok(Some(output))
                                }
                                Err(e) => Err(e),
                            }
                        } else {
                            Ok(None) // no review path matched
                        };
                    match review_result {
                        Ok(Some(output)) => {
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &output,
                                ctx,
                            )
                            .await;
                        }
                        Ok(None) => {
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &OrchestratorOutput::empty(),
                                ctx,
                            )
                            .await;
                        }
                        Err(err) => {
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &OrchestratorOutput::empty(),
                                ctx,
                            )
                            .await;
                            return Err(err);
                        }
                    }
                }
                SchedulerStageKind::Synthesis => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    match self
                        .execute_synthesis_stage(
                            input,
                            &state,
                            &resolved_plan,
                            ctx,
                            Some((
                                SchedulerStageKind::Synthesis.as_event_name().to_string(),
                                stage_ordinal,
                            )),
                        )
                        .await
                    {
                        Ok(output) => {
                            Self::record_output(&mut state, &output);
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &output,
                                ctx,
                            )
                            .await;
                            state.execution.synthesized = Some(output);
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "synthesis stage failed; falling back to prior stage output");
                            Self::emit_stage_end(
                                &resolved_plan,
                                stage,
                                stage_ordinal,
                                &OrchestratorOutput::empty(),
                                ctx,
                            )
                            .await;
                        }
                    }
                }
                SchedulerStageKind::Handoff => {
                    if state.route.request_brief.is_empty() {
                        state.route.request_brief = self
                            .compose_request_analysis_input_with_context(
                                input,
                                state.session_context.as_deref(),
                            );
                    }
                    let handoff_result: Result<(), _> = async {
                        self.run_stage_effects(StageEffectsRequest {
                            stage,
                            moment: SchedulerEffectMoment::OnEnter,
                            original_input: input,
                            state: &mut state,
                            plan: &resolved_plan,
                            output: None,
                            ctx,
                        })
                        .await?;
                        self.run_stage_effects(StageEffectsRequest {
                            stage,
                            moment: SchedulerEffectMoment::BeforeTransition,
                            original_input: input,
                            state: &mut state,
                            plan: &resolved_plan,
                            output: None,
                            ctx,
                        })
                        .await?;
                        match self
                            .execute_handoff_stage(
                                input,
                                &state,
                                &resolved_plan,
                                ctx,
                                Some((
                                    SchedulerStageKind::Handoff.as_event_name().to_string(),
                                    stage_ordinal,
                                )),
                            )
                            .await
                        {
                            Ok(output) => {
                                Self::record_output(&mut state, &output);
                                let mut effect_output = output.clone();
                                self.run_stage_effects(StageEffectsRequest {
                                    stage,
                                    moment: SchedulerEffectMoment::OnSuccess,
                                    original_input: input,
                                    state: &mut state,
                                    plan: &resolved_plan,
                                    output: Some(&mut effect_output),
                                    ctx,
                                })
                                .await?;
                                state.execution.handed_off = Some(effect_output);
                            }
                            Err(err) => {
                                tracing::warn!(error = %err, "handoff stage failed; falling back to prior stage output");
                            }
                        }
                        Ok(())
                    }.await;
                    let stage_out = state
                        .execution
                        .handed_off
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(OrchestratorOutput::empty);
                    Self::emit_stage_end(&resolved_plan, stage, stage_ordinal, &stage_out, ctx)
                        .await;
                    handoff_result?;
                }
            }
            // ── Cancellation check: if any stage was cancelled, terminate scheduler ──
            if state.is_cancelled {
                tracing::info!(
                    stage = ?stage,
                    stage_idx,
                    "scheduler cancelled during stage; terminating"
                );
                break;
            }
            match self.next_stage_index(stage, stage_idx, &state, &resolved_plan) {
                Some(next_stage) => stage_idx = next_stage,
                None => break,
            }
        }

        Ok(self.finalize_output(state))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SchedulerArtifactKind {
    Planning,
    Draft,
}
