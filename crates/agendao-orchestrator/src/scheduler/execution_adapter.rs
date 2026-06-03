use crate::agent_tree::{AgentTreeNode, AgentTreeOrchestrator, ChildExecutionMode};
use crate::skill_graph::{SkillGraphDefinition, SkillGraphOrchestrator};
use crate::traits::Orchestrator;
use crate::ExecutionContext;
use crate::{OrchestratorContext, OrchestratorError, OrchestratorOutput};

use super::{
    clone_context_with_exec_ctx, execute_stage_agent_with_exec_ctx, SchedulerExecutionChildMode,
    SchedulerProfileOrchestrator, SchedulerProfilePlan, SchedulerStageKind,
};

pub(super) struct SchedulerExecutionCapabilityAdapter<'a> {
    orchestrator: &'a SchedulerProfileOrchestrator,
    plan: &'a SchedulerProfilePlan,
    ctx: &'a OrchestratorContext,
    exec_ctx_override: Option<ExecutionContext>,
    /// Runtime-constructed agent tree (parsed from LLM parallel_plan output).
    dynamic_agent_tree: Option<AgentTreeNode>,
}

impl<'a> SchedulerExecutionCapabilityAdapter<'a> {
    pub(super) fn new(
        orchestrator: &'a SchedulerProfileOrchestrator,
        plan: &'a SchedulerProfilePlan,
        ctx: &'a OrchestratorContext,
    ) -> Self {
        Self {
            orchestrator,
            plan,
            ctx,
            exec_ctx_override: None,
            dynamic_agent_tree: None,
        }
    }

    pub(super) fn with_exec_ctx(mut self, exec_ctx: ExecutionContext) -> Self {
        self.exec_ctx_override = Some(exec_ctx);
        self
    }

    pub(super) fn with_dynamic_agent_tree(mut self, tree: AgentTreeNode) -> Self {
        self.dynamic_agent_tree = Some(tree);
        self
    }

    pub(super) async fn execute_agent_tree(
        &self,
        agent_tree: &AgentTreeNode,
        execution_input: &str,
        child_mode: SchedulerExecutionChildMode,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let mut tree =
            AgentTreeOrchestrator::new(agent_tree.clone(), self.orchestrator.tool_runner())
                .with_child_execution_mode(Self::child_execution_mode(child_mode));
        if let Some((stage_name, stage_index)) = stage_context {
            tree.set_stage_context(stage_name, stage_index);
        }
        let ctx = self.effective_context();
        tree.execute(execution_input, &ctx).await
    }

    pub(super) async fn execute_skill_graph(
        &self,
        skill_graph: &SkillGraphDefinition,
        execution_input: &str,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let mut graph =
            SkillGraphOrchestrator::new(skill_graph.clone(), self.orchestrator.tool_runner());
        if let Some((stage_name, stage_index)) = stage_context {
            graph.set_stage_context(stage_name, stage_index);
        }
        let ctx = self.effective_context();
        graph.execute(execution_input, &ctx).await
    }

    pub(super) async fn execute_review_stage(
        &self,
        input: &str,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let profile_suffix =
            super::profile_prompt_suffix(self.plan, Some(SchedulerStageKind::Review));
        let prompt = self
            .plan
            .review_stage_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are the scheduler review layer.                      Audit the current result against the original request and return a tighter, evidence-based review.{}",
                    profile_suffix
                )
            });
        let stage_policy = self.plan.stage_policy(SchedulerStageKind::Review);
        execute_stage_agent_with_exec_ctx(
            input,
            self.ctx,
            SchedulerProfileOrchestrator::stage_agent_from_policy(
                "scheduler-review",
                prompt,
                stage_policy,
            ),
            stage_policy.tool_policy,
            stage_context,
            self.exec_ctx_override.clone(),
        )
        .await
    }

    pub(super) async fn execute_execution_fallback_stage(
        &self,
        input: &str,
        stage_context: Option<(String, u32)>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        let profile_suffix = super::profile_prompt_suffix(
            self.plan,
            Some(SchedulerStageKind::ExecutionOrchestration),
        );
        let prompt = self
            .plan
            .execution_fallback_prompt(&profile_suffix)
            .unwrap_or_else(|| {
                format!(
                    "You are the scheduler execution fallback. \
                     Execute the current request directly with AgenDao tools and return the concrete result.{}",
                    profile_suffix
                )
            });
        let stage_policy = self
            .plan
            .stage_policy(SchedulerStageKind::ExecutionOrchestration);
        execute_stage_agent_with_exec_ctx(
            input,
            self.ctx,
            SchedulerProfileOrchestrator::stage_agent_from_policy(
                "scheduler-execution",
                prompt,
                stage_policy,
            ),
            stage_policy.tool_policy,
            stage_context,
            self.exec_ctx_override.clone(),
        )
        .await
    }

    pub(super) async fn execute_execution_path(
        &self,
        execution_input: &str,
        child_mode: SchedulerExecutionChildMode,
        allow_execution_fallback: bool,
        stage_context: Option<(String, u32)>,
        stage: Option<SchedulerStageKind>,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        // Priority: per-stage agent_tree → profile-level agent_tree →
        //           dynamic_agent_tree → skill_graph → fallback.
        let stage_tree = stage.and_then(|s| self.plan.stage_agent_tree(s));
        if let Some(agent_tree) = stage_tree.or(self.plan.agent_tree.as_ref()) {
            self.execute_agent_tree(agent_tree, execution_input, child_mode, stage_context)
                .await
        } else if let Some(ref dynamic_tree) = self.dynamic_agent_tree {
            self.execute_agent_tree(dynamic_tree, execution_input, child_mode, stage_context)
                .await
        } else if let Some(skill_graph) = &self.plan.skill_graph {
            self.execute_skill_graph(skill_graph, execution_input, stage_context)
                .await
        } else if allow_execution_fallback {
            self.execute_execution_fallback_stage(execution_input, stage_context)
                .await
        } else {
            Err(Self::execution_unavailable_error(self.plan))
        }
    }

    fn child_execution_mode(mode: SchedulerExecutionChildMode) -> ChildExecutionMode {
        match mode {
            SchedulerExecutionChildMode::Parallel => ChildExecutionMode::Parallel,
            SchedulerExecutionChildMode::Sequential => ChildExecutionMode::Sequential,
        }
    }

    fn effective_context(&self) -> OrchestratorContext {
        clone_context_with_exec_ctx(
            self.ctx,
            self.exec_ctx_override
                .clone()
                .unwrap_or_else(|| self.ctx.exec_ctx.clone()),
        )
    }

    fn execution_unavailable_error(plan: &SchedulerProfilePlan) -> OrchestratorError {
        let orchestrator = plan.orchestrator.as_deref().unwrap_or("scheduler");
        OrchestratorError::Other(format!(
            "{orchestrator} execution requires an agent_tree or skill_graph"
        ))
    }
}
