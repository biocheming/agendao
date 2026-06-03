use super::*;
use crate::scheduler::prompt_support::{
    push_session_context_section, render_compact_skill_catalog, SKILLS_GUIDANCE,
};

pub(in crate::scheduler) struct RetryComposeRequest<'a> {
    pub(in crate::scheduler) original_input: &'a str,
    pub(in crate::scheduler) state: &'a SchedulerProfileState,
    pub(in crate::scheduler) plan: &'a SchedulerProfilePlan,
    pub(in crate::scheduler) round: usize,
    pub(in crate::scheduler) decision: &'a SchedulerExecutionGateDecision,
    pub(in crate::scheduler) previous_output: &'a OrchestratorOutput,
    pub(in crate::scheduler) review_output: Option<&'a OrchestratorOutput>,
}

fn render_progressive_skills_index(skill_list: &[SchedulerSkillRef]) -> String {
    format!(
        "{}

Use `skill_view(name)` to inspect a relevant skill before relying on it.

{SKILLS_GUIDANCE}",
        render_compact_skill_catalog(skill_list)
    )
}

impl SchedulerProfileOrchestrator {
    #[cfg(test)]
    pub(super) fn compose_request_analysis_input(&self, input: &str) -> String {
        self.compose_request_analysis_input_with_context(input, None)
    }

    pub(super) fn compose_request_analysis_input_with_context(
        &self,
        input: &str,
        session_context: Option<&str>,
    ) -> String {
        let mut sections = Vec::new();
        sections.push(
            "## Stage
request-analysis"
                .to_string(),
        );
        push_session_context_section(&mut sections, session_context);
        sections.push(format!(
            "## User Request
{input}"
        ));

        if let Some(profile_name) = &self.plan.profile_name {
            sections.push(format!(
                "## Profile Name
{profile_name}"
            ));
        }

        if let Some(description) = &self.plan.description {
            let description = description.trim();
            if !description.is_empty() {
                sections.push(format!(
                    "## Profile Description
{description}"
                ));
            }
        }

        let effective_skill_list = self
            .plan
            .effective_skill_list(Some(SchedulerStageKind::RequestAnalysis));
        if !effective_skill_list.is_empty() {
            sections.push(format!(
                "## Skills
{}

Use `skill_view(name)` to inspect a relevant skill before relying on it.

{SKILLS_GUIDANCE}",
                render_compact_skill_catalog(effective_skill_list),
            ));
        }

        if let Some(skill_tree) = &self.plan.skill_tree {
            let context = skill_tree.context_markdown.trim();
            if !context.is_empty() {
                sections.push(format!(
                    "## Skill Tree Context
{context}"
                ));
            }
        }

        if let Some(route_constraint) = self.plan.route_constraint_note() {
            sections.push(format!(
                "## Workflow Constraint
{route_constraint}"
            ));
        }

        sections.push(
            "## Intent Gate Reference
Use the same Phase 0 taxonomy as oh-my-opencode:
- Step 0: map the surface form to the true intent before choosing a workflow.
- Research/understanding (`explain X`, `how does Y work`) -> explore or inspect, then answer.
- Implementation (`implement X`, `add Y`, `create Z`) -> plan, then delegate or execute.
- Investigation (`look into X`, `check Y`, `investigate`) -> explore, then report findings.
- Evaluation (`what do you think about X?`) -> evaluate and propose; wait for confirmation before changing code.
- Fix (`I'm seeing error X`, `Y is broken`) -> diagnose, then fix minimally.
- Open-ended change (`refactor`, `improve`, `clean up`) -> assess the codebase before proposing an approach.

Classify the request type as one of: Trivial, Explicit, Exploratory, Open-ended, Ambiguous.
Ask only when ambiguity has multiple materially different interpretations or missing critical information."
                .to_string(),
        );

        sections.push(
            "## Orchestrator Intent
Freeze the request context once, then preserve the same semantic goal across the selected workflow's planning, execution, review, or handoff stages. If this profile includes a Route stage, it may choose a request-scoped workflow; otherwise trust the explicitly selected preset. If the active preset is Prometheus, preserve planner-only behavior and keep the session on the reviewed-plan path rather than execution."
                .to_string(),
        );

        sections.join(
            "

",
        )
    }

    pub(super) fn compose_route_input(
        &self,
        original_input: &str,
        request_brief: &str,
        plan: &SchedulerProfilePlan,
        session_context: Option<&str>,
    ) -> String {
        let mut sections = Vec::new();
        sections.push("## Stage\nroute".to_string());
        push_session_context_section(&mut sections, session_context);
        sections.push(format!("## Original Request\n{original_input}"));
        sections.push(format!("## Request Brief\n{request_brief}"));
        sections.push(format!("## Current Plan\n{}", render_plan_snapshot(plan)));
        sections.push(
            "## Routing Goal
Choose the best request-scoped orchestration path across AgenDao presets, then return a bounded RouteDecision JSON. Prefer planner-only handoff workflows when the request needs upfront clarification and a reviewed plan instead of execution."
                .to_string(),
        );
        if let Some(route_constraint) = plan.route_constraint_note() {
            sections.push(route_constraint.to_string());
        }
        if let Some(context) = skill_tree_context(plan) {
            sections.push(format!("## Skill Tree Context\n{context}"));
        }
        let capabilities = build_capabilities_summary(
            &plan.available_agents,
            &plan.available_categories,
            plan.effective_skill_list(Some(SchedulerStageKind::Route)),
        );
        if !capabilities.is_empty() {
            sections.push(format!("## System Capabilities\n{capabilities}"));
        }
        sections.join("\n\n")
    }

    pub(super) fn compose_interview_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        let route_decision_json = state.route.route_decision.as_ref().map(|route_decision| {
            serde_json::to_string_pretty(route_decision)
                .unwrap_or_else(|_| route_decision.rationale_summary.clone())
        });
        if let Some(composed) = plan.compose_interview_stage_input(SchedulerInterviewStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            route_decision_json: route_decision_json.as_deref(),
            draft_artifact_path: state.preset_runtime.draft_artifact_path.as_deref(),
            draft_context: state.preset_runtime.draft_snapshot.as_deref(),
            current_plan: &render_plan_snapshot(plan),
            skill_tree_context: skill_tree_context(plan),
        }) {
            return composed;
        }

        let mut sections = Vec::new();
        sections.push(
            "## Stage
interview"
                .to_string(),
        );
        push_session_context_section(&mut sections, state.session_context.as_deref());
        sections.push(format!(
            "## Original Request
{original_input}"
        ));
        sections.push(format!(
            "## Request Brief
{}",
            state.route.request_brief
        ));
        if let Some(route_decision) = &state.route.route_decision {
            sections.push(format!(
                "## Route Decision
{}",
                serde_json::to_string_pretty(route_decision)
                    .unwrap_or_else(|_| route_decision.rationale_summary.clone())
            ));
        }
        sections.push(format!(
            "## Current Plan
{}",
            render_plan_snapshot(plan)
        ));
        if let Some(context) = skill_tree_context(plan) {
            sections.push(format!(
                "## Skill Tree Context
{context}"
            ));
        }
        sections.push(
            "## Interview Charter
Resolve discoverable unknowns with read-only inspection first. Ask only when a remaining preference or tradeoff materially changes the plan. Return a planning-oriented interview brief."
                .to_string(),
        );
        sections.join(
            "

",
        )
    }

    pub(super) fn compose_plan_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        let route_decision_json = state.route.route_decision.as_ref().map(|route_decision| {
            serde_json::to_string_pretty(route_decision)
                .unwrap_or_else(|_| route_decision.rationale_summary.clone())
        });
        if let Some(composed) = plan.compose_plan_stage_input(SchedulerPlanStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            route_decision_json: route_decision_json.as_deref(),
            route_output: state.route.routed.as_deref(),
            planning_artifact_path: state.preset_runtime.planning_artifact_path.as_deref(),
            draft_artifact_path: state.preset_runtime.draft_artifact_path.as_deref(),
            draft_context: state.preset_runtime.draft_snapshot.as_deref(),
            interview_output: state.route.interviewed.as_deref(),
            advisory_review: state.preset_runtime.advisory_review.as_deref(),
            approval_feedback: state.preset_runtime.approval_review.as_deref(),
            current_plan: &render_plan_snapshot(plan),
            skill_tree_context: skill_tree_context(plan),
            available_agents: &plan.available_agents,
            available_categories: &plan.available_categories,
            skill_list: plan.effective_skill_list(Some(SchedulerStageKind::Plan)),
        }) {
            return composed;
        }

        let mut sections = Vec::new();
        sections.push(
            "## Stage
plan"
                .to_string(),
        );
        push_session_context_section(&mut sections, state.session_context.as_deref());
        sections.push(format!(
            "## Original Request
{original_input}"
        ));
        sections.push(format!(
            "## Request Brief
{}",
            state.route.request_brief
        ));
        if let Some(route_decision) = &state.route.route_decision {
            sections.push(format!(
                "## Route Decision
{}",
                serde_json::to_string_pretty(route_decision)
                    .unwrap_or_else(|_| route_decision.rationale_summary.clone())
            ));
        }
        if let Some(routed) = state.route.routed.as_deref() {
            sections.push(format!(
                "## Route Output
{routed}"
            ));
        }
        sections.push(format!(
            "## Current Plan
{}",
            render_plan_snapshot(plan)
        ));
        if let Some(context) = skill_tree_context(plan) {
            sections.push(format!(
                "## Skill Tree Context
{context}"
            ));
        }
        let capabilities = build_capabilities_summary(
            &plan.available_agents,
            &plan.available_categories,
            plan.effective_skill_list(Some(SchedulerStageKind::Plan)),
        );
        if !capabilities.is_empty() {
            sections.push(format!(
                "## System Capabilities
{capabilities}"
            ));
        }
        sections.push(
            "## Planner Charter
Produce a concrete execution plan only. No file edits, no claims that work is already done. State assumptions, phases, verification, and risks."
                .to_string(),
        );
        sections.join(
            "

",
        )
    }

    pub(super) fn compose_delegation_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        let mut sections = Vec::new();
        sections.push("## Stage\ndelegation".to_string());
        push_session_context_section(&mut sections, state.session_context.as_deref());
        sections.push(format!("## Original Request\n{original_input}"));
        sections.push(format!("## Request Brief\n{}", state.route.request_brief));
        if let Some(route_decision) = &state.route.route_decision {
            sections.push(format!(
                "## Route Summary\n{}",
                route_decision.rationale_summary
            ));
        }
        if let Some(plan_output) = state.preset_runtime.planned.as_deref() {
            sections.push(format!("## Execution Plan\n{plan_output}"));
        }
        if let Some(context) = skill_tree_context(plan) {
            sections.push(format!("## Skill Tree Context\n{context}"));
        }
        let charter = plan.delegation_charter().unwrap_or_else(|| {
            "## Execution Charter
\
                 Execute the task according to the frozen request goal. \
                 Use the execution plan when present, but do not drift from the original request."
                .to_string()
        });
        sections.push(charter);
        sections.join("\n\n")
    }

    pub(in crate::scheduler) fn compose_execution_orchestration_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        scheduler_execution_input::compose_execution_orchestration_input(
            original_input,
            state,
            plan,
        )
    }

    pub(super) fn compose_review_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        let review_skill_list = plan.effective_skill_list(Some(SchedulerStageKind::Review));
        let skills_index_markdown = (!review_skill_list.is_empty())
            .then(|| render_progressive_skills_index(review_skill_list));
        if let Some(composed) = plan.compose_review_stage_input(SchedulerReviewStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            route_summary: state
                .route
                .route_decision
                .as_ref()
                .map(|route_decision| route_decision.rationale_summary.as_str()),
            draft_context: state.preset_runtime.draft_snapshot.as_deref(),
            interview_output: state.route.interviewed.as_deref(),
            execution_plan: state.preset_runtime.planned.as_deref(),
            advisory_review: state.preset_runtime.advisory_review.as_deref(),
            approval_feedback: state.preset_runtime.approval_review.as_deref(),
            saved_planning_artifact: state.preset_runtime.planning_artifact_path.as_deref(),
            skills_index_markdown: skills_index_markdown.as_deref(),
            delegation_output: state
                .execution
                .delegated
                .as_ref()
                .map(|output| output.content.as_str()),
        }) {
            return composed;
        }

        let mut sections = Vec::new();
        sections.push(
            "## Stage
review"
                .to_string(),
        );
        push_session_context_section(&mut sections, state.session_context.as_deref());
        sections.push(format!(
            "## Original Request
{original_input}"
        ));
        sections.push(format!(
            "## Request Brief
{}",
            state.route.request_brief
        ));
        if let Some(route_decision) = &state.route.route_decision {
            sections.push(format!(
                "## Route Summary
{}",
                route_decision.rationale_summary
            ));
        }
        if !review_skill_list.is_empty() {
            sections.push(format!(
                "## Skills
{}

Use `skill_view(name)` to inspect a relevant skill before relying on it.

{SKILLS_GUIDANCE}",
                render_compact_skill_catalog(review_skill_list),
            ));
        }
        if let Some(delegated) = &state.execution.delegated {
            sections.push(format!(
                "## Delegation Output
{}",
                delegated.content
            ));
        }
        sections.push(
            "## Review Charter
Review the delegated result against the original task. Tighten the result without changing the task objective."
                .to_string(),
        );
        sections.join(
            "

",
        )
    }

    pub(super) fn compose_handoff_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        if let Some(composed) = plan.compose_handoff_stage_input(SchedulerHandoffStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            current_plan: &render_plan_snapshot(plan),
            draft_context: state.preset_runtime.draft_snapshot.as_deref(),
            interview_output: state.route.interviewed.as_deref(),
            planning_output: state.preset_runtime.planned.as_deref(),
            review_output: state
                .execution
                .reviewed
                .as_ref()
                .map(|output| output.content.as_str()),
            approval_review: state.preset_runtime.approval_review.as_deref(),
            user_choice: state.preset_runtime.user_choice.as_deref(),
            saved_planning_artifact: state.preset_runtime.planning_artifact_path.as_deref(),
        }) {
            return composed;
        }

        let mut sections = Vec::new();
        sections.push(
            "## Stage
handoff"
                .to_string(),
        );
        push_session_context_section(&mut sections, state.session_context.as_deref());
        sections.push(format!(
            "## Original Request
{original_input}"
        ));
        sections.push(format!(
            "## Request Brief
{}",
            state.route.request_brief
        ));
        sections.push(format!(
            "## Current Plan
{}",
            render_plan_snapshot(plan)
        ));
        sections.push(
            "## Handoff Charter
End this workflow with a reviewed planning handoff. Do not claim code execution was performed. Make the next recommended action explicit."
                .to_string(),
        );
        sections.join(
            "

",
        )
    }

    pub(super) fn compose_synthesis_input(
        &self,
        original_input: &str,
        state: &SchedulerProfileState,
        plan: &SchedulerProfilePlan,
    ) -> String {
        let current_plan = render_plan_snapshot(plan);
        let route_decision_json = state.route.route_decision.as_ref().map(|route_decision| {
            serde_json::to_string_pretty(route_decision)
                .unwrap_or_else(|_| route_decision.rationale_summary.clone())
        });
        if let Some(composed) = plan.compose_synthesis_stage_input(SchedulerSynthesisStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            current_plan: &current_plan,
            route_decision_json: route_decision_json.as_deref(),
            planning_output: state.preset_runtime.planned.as_deref(),
            delegation_output: state
                .execution
                .delegated
                .as_ref()
                .map(|output| output.content.as_str()),
            review_output: state
                .execution
                .reviewed
                .as_ref()
                .map(|output| output.content.as_str()),
            saved_planning_artifact: state.preset_runtime.planning_artifact_path.as_deref(),
        }) {
            return composed;
        }

        let mut sections = Vec::new();
        sections.push(
            "## Stage
synthesis"
                .to_string(),
        );
        push_session_context_section(&mut sections, state.session_context.as_deref());
        sections.push(format!(
            "## Original Request
{original_input}"
        ));
        sections.push(format!(
            "## Request Brief
{}",
            state.route.request_brief
        ));
        sections.push(format!(
            "## Current Plan
{current_plan}"
        ));
        if let Some(route_decision) = &state.route.route_decision {
            sections.push(format!(
                "## Route Decision
{}",
                serde_json::to_string_pretty(route_decision)
                    .unwrap_or_else(|_| route_decision.rationale_summary.clone())
            ));
        }
        if let Some(plan_output) = state.preset_runtime.planned.as_deref() {
            sections.push(format!(
                "## Planning Output
{plan_output}"
            ));
        }
        if let Some(delegated) = &state.execution.delegated {
            sections.push(format!(
                "## Delegation Output
{}",
                delegated.content
            ));
        }
        if let Some(reviewed) = &state.execution.reviewed {
            sections.push(format!(
                "## Review Output
{}",
                reviewed.content
            ));
        }
        if let Some(artifact_path) = state.preset_runtime.planning_artifact_path.as_deref() {
            sections.push(format!(
                "## Saved Planning Artifact
{artifact_path}"
            ));
        }
        sections.push(
            "## Synthesis Charter
Produce the final user-facing answer. Prefer reviewed output when present, otherwise delegated output. Preserve concrete results, unresolved risks, and next actions."
                .to_string(),
        );
        sections.join(
            "

",
        )
    }
}
