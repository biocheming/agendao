use crate::workflow_mode::mode_artifact_contract;
use crate::OrchestratorOutput;

use super::execution_contracts::{
    normalize_retry_focus, SHARED_EXECUTION_EVIDENCE_CONTRACT, SHARED_GATE_DECISION_CONTRACT,
    SHARED_RETRY_RECOVERY_CONTRACT, SHARED_VERIFICATION_EVIDENCE_CONTRACT,
};
use super::profile_state::SchedulerProfileState;
use super::prompt_support::push_session_context_section;
use super::{
    render_plan_snapshot, skill_tree_context, SchedulerAutonomousGateStageInput,
    SchedulerAutonomousVerificationStageInput, SchedulerCoordinationGateStageInput,
    SchedulerCoordinationVerificationStageInput, SchedulerExecutionGateDecision,
    SchedulerExecutionOrchestrationStageInput, SchedulerProfileOrchestrator, SchedulerProfilePlan,
    SchedulerRetryStageInput,
};

pub(super) fn compose_execution_orchestration_input(
    original_input: &str,
    state: &SchedulerProfileState,
    plan: &SchedulerProfilePlan,
) -> String {
    let skill_tree_context_value = skill_tree_context(plan);
    if let Some(composed) = plan.compose_execution_orchestration_stage_input(
        SchedulerExecutionOrchestrationStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            route_summary: state
                .route
                .route_decision
                .as_ref()
                .map(|route_decision| route_decision.rationale_summary.as_str()),
            planning_output: state.preset_runtime.planned.as_deref(),
            ground_truth_context: state.preset_runtime.ground_truth_context.as_deref(),
            skill_tree_context: skill_tree_context_value,
            available_agents: &plan.available_agents,
            available_categories: &plan.available_categories,
            skill_list: plan
                .effective_skill_list(Some(super::SchedulerStageKind::ExecutionOrchestration)),
        },
    ) {
        return composed;
    }

    let mut sections = Vec::new();
    sections.push("## Stage\nexecution-orchestration".to_string());
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
        sections.push(format!("## Planning Output\n{plan_output}"));
    }
    if let Some(ground_truth) = state.preset_runtime.ground_truth_context.as_deref() {
        sections.push(format!("## Ground Truth Context\n{ground_truth}"));
    }
    if let Some(context) = skill_tree_context_value {
        sections.push(format!("## Skill Tree Context\n{context}"));
    }
    let profile_suffix = super::profile_prompt_suffix(
        plan,
        Some(super::SchedulerStageKind::ExecutionOrchestration),
    );
    let charter = plan
        .execution_orchestration_charter(&profile_suffix)
        .unwrap_or_else(|| {
            "## Coordination Charter\n\
             Coordinate the execution graph or worker tree, preserve task boundaries, \
             and aggregate a single execution result."
                .to_string()
        });
    sections.push(charter);
    sections.push(SHARED_EXECUTION_EVIDENCE_CONTRACT.to_string());
    if let Some(workflow) = plan.workflow.as_ref() {
        sections.push(mode_artifact_contract(workflow.workflow.mode));
    }
    sections.join("\n\n")
}

pub(super) fn compose_coordination_verification_input(
    original_input: &str,
    state: &SchedulerProfileState,
    plan: &SchedulerProfilePlan,
    round: usize,
    execution_output: &OrchestratorOutput,
) -> String {
    let skill_tree_context_value = skill_tree_context(plan);
    if let Some(composed) = plan.compose_coordination_verification_stage_input(
        SchedulerCoordinationVerificationStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            round,
            execution_output: execution_output.content.as_str(),
            planning_output: state.preset_runtime.planned.as_deref(),
            ground_truth_context: state.preset_runtime.ground_truth_context.as_deref(),
            skill_tree_context: skill_tree_context_value,
        },
    ) {
        return composed;
    }

    let mut sections = Vec::new();
    sections.push("## Stage\ncoordination-verification".to_string());
    sections.push(format!("## Round\n{round}"));
    push_session_context_section(&mut sections, state.session_context.as_deref());
    sections.push(format!("## Original Request\n{original_input}"));
    sections.push(format!("## Request Brief\n{}", state.route.request_brief));
    sections.push(format!("## Execution Output\n{}", execution_output.content));
    if let Some(plan_output) = state.preset_runtime.planned.as_deref() {
        sections.push(format!("## Planning Output\n{plan_output}"));
    }
    if let Some(ground_truth) = state.preset_runtime.ground_truth_context.as_deref() {
        sections.push(format!("## Ground Truth Context\n{ground_truth}"));
    }
    if let Some(context) = skill_tree_context_value {
        sections.push(format!("## Skill Tree Context\n{context}"));
    }
    sections.push(
        plan.coordination_verification_charter()
            .map(str::to_string)
            .unwrap_or_else(|| {
                "## Verification Charter\n\
                 Verify worker outputs against the original request. \
                 Confirm completion, identify missing work, and surface blockers \
                 without redoing the implementation."
                    .to_string()
            }),
    );
    sections.push(SHARED_VERIFICATION_EVIDENCE_CONTRACT.to_string());
    if let Some(workflow) = plan.workflow.as_ref() {
        sections.push(mode_artifact_contract(workflow.workflow.mode));
    }
    sections.join("\n\n")
}

pub(super) fn compose_coordination_gate_input(
    original_input: &str,
    state: &SchedulerProfileState,
    plan: &SchedulerProfilePlan,
    round: usize,
    execution_output: &OrchestratorOutput,
    review_output: Option<&OrchestratorOutput>,
) -> String {
    let current_plan = render_plan_snapshot(plan);
    if let Some(composed) =
        plan.compose_coordination_gate_stage_input(SchedulerCoordinationGateStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            current_plan: &current_plan,
            round,
            execution_output: execution_output.content.as_str(),
            verification_output: review_output.map(|output| output.content.as_str()),
            ground_truth_context: state.preset_runtime.ground_truth_context.as_deref(),
        })
    {
        return composed;
    }

    let mut sections = Vec::new();
    sections.push("## Stage\ncoordination-gate".to_string());
    sections.push(format!("## Round\n{round}"));
    push_session_context_section(&mut sections, state.session_context.as_deref());
    sections.push(format!("## Original Request\n{original_input}"));
    sections.push(format!("## Request Brief\n{}", state.route.request_brief));
    sections.push(format!("## Execution Output\n{}", execution_output.content));
    if let Some(review_output) = review_output {
        sections.push(format!("## Verification Output\n{}", review_output.content));
    }
    if let Some(ground_truth) = state.preset_runtime.ground_truth_context.as_deref() {
        sections.push(format!("## Ground Truth Context\n{ground_truth}"));
    }
    sections.push(format!("## Current Plan\n{current_plan}"));
    sections.push(
        plan.coordination_gate_contract()
            .map(str::to_string)
            .unwrap_or_else(|| {
                r#"## Coordination Decision Contract
Return JSON only: {"status":"done|continue|blocked","summary":"short summary","next_input":"optional next round task","final_response":"optional final coordinator response"}.
Use continue only when there is concrete unfinished work for another worker round."#
                    .to_string()
            }),
    );
    sections.push(SHARED_GATE_DECISION_CONTRACT.to_string());
    sections.join("\n\n")
}

pub(super) fn compose_autonomous_verification_input(
    original_input: &str,
    state: &SchedulerProfileState,
    plan: &SchedulerProfilePlan,
    round: usize,
    execution_output: &OrchestratorOutput,
) -> String {
    let current_plan = render_plan_snapshot(plan);
    if let Some(composed) = plan.compose_autonomous_verification_stage_input(
        SchedulerAutonomousVerificationStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            current_plan: &current_plan,
            round,
            execution_output: execution_output.content.as_str(),
        },
    ) {
        return composed;
    }

    let mut sections = Vec::new();
    sections.push("## Stage\nautonomous-verification".to_string());
    sections.push(format!("## Round\n{round}"));
    push_session_context_section(&mut sections, state.session_context.as_deref());
    sections.push(format!("## Original Request\n{original_input}"));
    sections.push(format!("## Request Brief\n{}", state.route.request_brief));
    sections.push(format!("## Executor Output\n{}", execution_output.content));
    sections.push(format!("## Current Plan\n{current_plan}"));
    sections.push(
        plan.autonomous_verification_charter()
            .map(str::to_string)
            .unwrap_or_else(|| {
                "## Verification Charter\n\
                 Audit the executor output before completion. Confirm what is done, \
                 what evidence is present, and what remains uncertain. \
                 Prefer concrete verification notes over stylistic critique."
                    .to_string()
            }),
    );
    sections.push(SHARED_VERIFICATION_EVIDENCE_CONTRACT.to_string());
    if let Some(workflow) = plan.workflow.as_ref() {
        sections.push(mode_artifact_contract(workflow.workflow.mode));
    }
    sections.join("\n\n")
}

pub(super) fn compose_autonomous_gate_input(
    original_input: &str,
    state: &SchedulerProfileState,
    plan: &SchedulerProfilePlan,
    round: usize,
    execution_output: &OrchestratorOutput,
    verification_output: Option<&OrchestratorOutput>,
) -> String {
    let current_plan = render_plan_snapshot(plan);
    if let Some(composed) =
        plan.compose_autonomous_gate_stage_input(SchedulerAutonomousGateStageInput {
            original_request: original_input,
            session_context: state.session_context.as_deref(),
            request_brief: &state.route.request_brief,
            current_plan: &current_plan,
            round,
            execution_output: execution_output.content.as_str(),
            verification_output: verification_output.map(|output| output.content.as_str()),
        })
    {
        return composed;
    }

    let mut sections = Vec::new();
    sections.push("## Stage\nautonomous-finish-gate".to_string());
    sections.push(format!("## Round\n{round}"));
    push_session_context_section(&mut sections, state.session_context.as_deref());
    sections.push(format!("## Original Request\n{original_input}"));
    sections.push(format!("## Request Brief\n{}", state.route.request_brief));
    sections.push(format!("## Executor Output\n{}", execution_output.content));
    if let Some(verification) = verification_output {
        sections.push(format!("## Verification Output\n{}", verification.content));
    }
    sections.push(format!("## Current Plan\n{current_plan}"));
    sections.push(
        plan.autonomous_gate_contract()
            .map(str::to_string)
            .unwrap_or_else(|| {
                r#"## Finish Gate Contract
Return JSON only: {"status":"done|continue|blocked","summary":"short summary","next_input":"optional retry brief","final_response":"optional final response"}.
Prefer done when the output already satisfies the request.
Use continue only when a bounded retry should materially improve the result."#
                    .to_string()
            }),
    );
    sections.push(SHARED_GATE_DECISION_CONTRACT.to_string());
    sections.join("\n\n")
}

pub(super) fn compose_retry_input(
    original_input: &str,
    state: &SchedulerProfileState,
    plan: &SchedulerProfilePlan,
    round: usize,
    decision: &SchedulerExecutionGateDecision,
    previous_output: &OrchestratorOutput,
    review_output: Option<&OrchestratorOutput>,
) -> String {
    let current_plan = render_plan_snapshot(plan);
    let continuation_targets =
        SchedulerProfileOrchestrator::retry_continuation_targets(previous_output, review_output);
    let preferred_continuation = continuation_targets.last();
    let continuation_candidates =
        SchedulerProfileOrchestrator::render_retry_continuation_candidates(&continuation_targets);
    if let Some(composed) = plan.compose_retry_stage_input(SchedulerRetryStageInput {
        original_request: original_input,
        session_context: state.session_context.as_deref(),
        request_brief: &state.route.request_brief,
        current_plan: &current_plan,
        round,
        previous_output: previous_output.content.as_str(),
        verification_output: review_output.map(|output| output.content.as_str()),
        retry_summary: &decision.summary,
        next_input: decision.next_input.as_deref(),
        ground_truth_context: state.preset_runtime.ground_truth_context.as_deref(),
        preferred_continuation_session_id: preferred_continuation
            .map(|target| target.session_id.as_str()),
        preferred_continuation_agent_task_id: preferred_continuation
            .and_then(|target| target.agent_task_id.as_deref()),
        continuation_candidates: continuation_candidates.as_deref(),
    }) {
        return composed;
    }

    let mut sections = Vec::new();
    sections.push("## Retry Request".to_string());
    push_session_context_section(&mut sections, state.session_context.as_deref());
    sections.push(format!("## Original Request\n{original_input}"));
    sections.push(format!("## Request Brief\n{}", state.route.request_brief));
    sections.push(format!("## Current Plan\n{current_plan}"));
    sections.push(format!("## Previous Attempt\n{}", previous_output.content));
    if let Some(review_output) = review_output {
        sections.push(format!("## Verification Notes\n{}", review_output.content));
    }
    sections.push(format!("## Retry Summary\n{}", decision.summary));
    let retry_focus = normalize_retry_focus(&decision.summary, decision.next_input.as_deref());
    if !retry_focus.is_empty() {
        sections.push(format!("## Retry Focus\n{retry_focus}"));
    }
    if let Some(ground_truth) = state.preset_runtime.ground_truth_context.as_deref() {
        sections.push(format!("## Ground Truth Context\n{ground_truth}"));
    }
    if let Some(preferred) = preferred_continuation {
        let mut preferred_parts = vec![format!("session_id: {}", preferred.session_id)];
        if let Some(agent_task_id) = preferred.agent_task_id.as_deref() {
            preferred_parts.push(format!("agent_task_id: {agent_task_id}"));
        }
        sections.push(format!(
            "## Preferred Continuation\n{}",
            preferred_parts.join(" | ")
        ));
    }
    sections.push(SHARED_RETRY_RECOVERY_CONTRACT.to_string());
    if let Some(candidates) = continuation_candidates {
        sections.push(format!("## Continuation Candidates\n{candidates}"));
    }
    sections.join("\n\n")
}
