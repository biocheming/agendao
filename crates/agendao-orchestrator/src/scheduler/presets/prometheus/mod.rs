mod api;
mod artifacts;
mod definition;
mod handoff;
mod input;
mod prompt;
mod review;
mod runtime;
#[cfg(test)]
mod tests;

use super::{
    locked_default_stage_kinds, SchedulerPresetBundle, SchedulerPresetCapabilityHooks,
    SchedulerPresetEffectHooks, SchedulerPresetFinalizationHooks, SchedulerPresetGateHooks,
    SchedulerPresetPlatformSpec, SchedulerPresetProjectionHooks, SchedulerPresetPromptHooks,
    SchedulerPresetRouteHooks, SchedulerPresetRuntimeUpdate, SchedulerPresetStageGraphHooks,
    DEFAULT_INTERNAL_STAGE_HOOKS, PLACEMENT_DELEGATED_CLEAR_REVIEWED, PLACEMENT_REVIEWED_KEEP,
    PLANNER_HANDOFF_FINAL_OUTPUT_PRIORITY,
};
use crate::scheduler::{
    ReviewMode, RouteDecision, RouteMode, SchedulerExecutionStageDispatch,
    SchedulerExecutionWorkflowPolicy, SchedulerFinalizationMode, SchedulerStageKind,
    StageToolPolicy,
};

pub use api::*;
pub use artifacts::*;
pub use definition::*;
pub use handoff::*;
pub use input::*;
pub use prompt::*;
pub use review::*;
pub use runtime::*;

fn prometheus_planning_artifact_relative_path(session_id: &str) -> String {
    build_prometheus_artifact_relative_path(PrometheusArtifactKind::Planning, session_id)
}

fn prometheus_draft_artifact_relative_path(session_id: &str) -> String {
    build_prometheus_artifact_relative_path(PrometheusArtifactKind::Draft, session_id)
}

fn prometheus_resolve_runtime_transition_target(
    transitions: &[&crate::scheduler::SchedulerTransitionSpec],
    user_choice: Option<&str>,
    review_gate_approved: Option<bool>,
) -> Option<crate::scheduler::SchedulerTransitionTarget> {
    resolve_prometheus_transition_target(
        transitions,
        PrometheusTransitionContext {
            user_choice,
            review_gate_approved,
        },
    )
}

fn prometheus_normalize_review_stage_output(
    runtime: crate::scheduler::SchedulerPresetRuntimeFields<'_>,
    output: &str,
) -> String {
    normalize_prometheus_review_stage_output(prometheus_review_state_snapshot(runtime), output)
}

fn extend_prometheus_final_output_metadata(
    state: &crate::scheduler::profile_state::SchedulerProfileState,
    artifact_path: Option<&str>,
    metadata: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    if state.preset_runtime.user_choice.as_deref() != Some("Start Work") {
        return;
    }
    metadata.insert(
        "scheduler_handoff_mode".to_string(),
        serde_json::json!("atlas"),
    );
    metadata.insert(
        "scheduler_handoff_command".to_string(),
        serde_json::json!(crate::scheduler::plan_start_work_command(artifact_path)),
    );
    if let Some(path) = artifact_path {
        metadata.insert(
            "scheduler_handoff_plan_path".to_string(),
            serde_json::json!(path),
        );
    }
}

pub const PROMETHEUS_CAPABILITY_HOOKS: SchedulerPresetCapabilityHooks =
    SchedulerPresetCapabilityHooks {
        runtime_update_for_advisory_review: Some(SchedulerPresetRuntimeUpdate::AdvisoryReview),
        runtime_update_for_user_choice: Some(SchedulerPresetRuntimeUpdate::UserChoice),
        runtime_update_for_approval_review: Some(SchedulerPresetRuntimeUpdate::ApprovalReview),
        runtime_update_for_planned_output: Some(SchedulerPresetRuntimeUpdate::Planned),
        runtime_update_for_review_gate: Some(SchedulerPresetRuntimeUpdate::ReviewGateApproved),
        compose_advisory_review_input: Some(compose_prometheus_advisory_review_input),
        advisory_agent_name: Some(prometheus_advisory_agent_name),
        user_choice_payload: Some(prometheus_user_choice_payload),
        parse_user_choice: Some(parse_prometheus_user_choice),
        default_user_choice: Some(prometheus_default_user_choice),
        approval_review_agent_name: Some(prometheus_approval_review_agent_name),
        max_approval_review_rounds: Some(prometheus_max_approval_review_rounds),
        approval_review_is_accepted: Some(prometheus_approval_review_is_accepted),
        planning_artifact_relative_path: Some(prometheus_planning_artifact_relative_path),
        draft_artifact_relative_path: Some(prometheus_draft_artifact_relative_path),
        compose_draft_artifact: Some(compose_prometheus_draft_artifact),
        compose_planning_artifact: Some(compose_prometheus_planning_artifact),
        validate_runtime_orchestration_tool: Some(validate_prometheus_runtime_orchestration_tool),
        validate_runtime_artifact_path: Some(validate_prometheus_runtime_artifact_path),
    };

fn prometheus_stage_tool_policy_override(stage: SchedulerStageKind) -> Option<StageToolPolicy> {
    match stage {
        SchedulerStageKind::Interview
        | SchedulerStageKind::Plan
        | SchedulerStageKind::Review
        | SchedulerStageKind::Handoff => Some(prometheus_planning_stage_tool_policy()),
        _ => None,
    }
}

fn constrain_prometheus_route_decision(decision: RouteDecision) -> RouteDecision {
    let mut constrained = decision;
    constrained.mode = RouteMode::Orchestrate;
    constrained.direct_kind = None;
    constrained.direct_response = None;
    constrained.preset = Some("prometheus".to_string());
    if constrained.review_mode == Some(ReviewMode::Skip) || constrained.review_mode.is_none() {
        constrained.review_mode = Some(ReviewMode::Normal);
    }
    constrained
}

pub const PROMETHEUS_STAGE_GRAPH_HOOKS: SchedulerPresetStageGraphHooks =
    SchedulerPresetStageGraphHooks {
        resolve_stage_kinds: locked_default_stage_kinds,
        stage_tool_policy_override: Some(prometheus_stage_tool_policy_override),
        stage_session_projection_override: None,
        stage_loop_budget_override: None,
        build_transition_graph: prometheus_transition_graph,
    };

pub const PROMETHEUS_ROUTE_HOOKS: SchedulerPresetRouteHooks = SchedulerPresetRouteHooks {
    route_constraint_note: Some(
        "## Route Constraint\nThis session is already running under the explicit Prometheus planner profile. Do NOT convert this request into a direct reply or direct clarification path. Preserve the Prometheus planner workflow, keep the session on interview -> plan -> review -> handoff, and do not reroute this session to Sisyphus, Atlas, Hephaestus, or any other preset.",
    ),
    constrain_route_decision: constrain_prometheus_route_decision,
};

pub const PROMETHEUS_GATE_HOOKS: SchedulerPresetGateHooks = SchedulerPresetGateHooks {
    resolve_execution_stage_dispatch: prometheus_execution_stage_dispatch,
    execution_workflow_policy: SchedulerExecutionWorkflowPolicy::direct(),
    coordination_verification_placement: PLACEMENT_REVIEWED_KEEP,
    coordination_terminal_placement: PLACEMENT_REVIEWED_KEEP,
    autonomous_verification_placement: PLACEMENT_REVIEWED_KEEP,
    autonomous_terminal_placement: PLACEMENT_DELEGATED_CLEAR_REVIEWED,
    retry_exhausted_placement: PLACEMENT_REVIEWED_KEEP,
    coordination_verification_charter: None,
    coordination_gate_contract: None,
    coordination_gate_prompt: None,
    autonomous_verification_charter: None,
    autonomous_gate_contract: None,
    autonomous_gate_prompt: None,
    resolve_gate_terminal_content: None,
    resolve_runtime_transition_target: Some(prometheus_resolve_runtime_transition_target),
};

pub const PROMETHEUS_EFFECT_HOOKS: SchedulerPresetEffectHooks = SchedulerPresetEffectHooks {
    build_effect_protocol: prometheus_effect_protocol,
    resolve_effect_dispatch: resolve_prometheus_effect_dispatch,
    effect_dispatch_is_authoritative: true,
};

pub const PROMETHEUS_FINALIZATION_HOOKS: SchedulerPresetFinalizationHooks =
    SchedulerPresetFinalizationHooks {
        finalization_mode: SchedulerFinalizationMode::PlannerHandoff,
        output_priority: PLANNER_HANDOFF_FINAL_OUTPUT_PRIORITY,
        extend_metadata: Some(extend_prometheus_final_output_metadata),
        normalize_review_stage_output: Some(prometheus_normalize_review_stage_output),
        normalize_final_output: Some(normalize_prometheus_final_output),
        decorate_final_output: Some(decorate_prometheus_handoff_output),
    };

pub const PROMETHEUS_PROJECTION_HOOKS: SchedulerPresetProjectionHooks =
    SchedulerPresetProjectionHooks {
        workflow_todos_payload: prometheus_workflow_todos_payload,
        system_prompt_preview: prometheus_system_prompt_preview,
        sync_runtime_authority: None,
    };

pub const PROMETHEUS_PLATFORM: SchedulerPresetPlatformSpec = SchedulerPresetPlatformSpec {
    stage_graph: PROMETHEUS_STAGE_GRAPH_HOOKS,
    route: PROMETHEUS_ROUTE_HOOKS,
    gate: PROMETHEUS_GATE_HOOKS,
    effect: PROMETHEUS_EFFECT_HOOKS,
    internal: DEFAULT_INTERNAL_STAGE_HOOKS,
    finalization: PROMETHEUS_FINALIZATION_HOOKS,
    projection: PROMETHEUS_PROJECTION_HOOKS,
    prompts: PROMETHEUS_PROMPT_HOOKS,
    capabilities: PROMETHEUS_CAPABILITY_HOOKS,
};

fn prometheus_execution_stage_dispatch() -> SchedulerExecutionStageDispatch {
    SchedulerExecutionStageDispatch::Direct
}

fn prometheus_delegation_stage_prompt(profile_suffix: &str) -> String {
    format!(
        "You are Prometheus's execution coordinator. Follow the approved plan precisely, execute with AgenDao tools, and use task/task_flow when delegation materially improves the outcome. Return concrete execution results, not planning prose.{}",
        profile_suffix
    )
}

fn prometheus_synthesis_stage_prompt(profile_suffix: &str) -> String {
    format!(
        "You are the final synthesis layer for AgenDao's scheduler (prometheus mode, OMO-aligned). Merge prior stage outputs into a single final response for the user. Keep the answer faithful to actual stage results. Do not invent edits, tool calls, or conclusions. If there are remaining risks or follow-ups, state them clearly.{}",
        profile_suffix
    )
}

pub const PROMETHEUS_PROMPT_HOOKS: SchedulerPresetPromptHooks = SchedulerPresetPromptHooks {
    delegation_charter: None,
    execution_orchestration_charter: None,
    review_stage_prompt: Some(prometheus_review_prompt),
    interview_stage_prompt: Some(prometheus_interview_prompt),
    plan_stage_prompt: Some(prometheus_plan_prompt),
    handoff_stage_prompt: Some(prometheus_handoff_prompt),
    delegation_stage_prompt: Some(prometheus_delegation_stage_prompt),
    execution_fallback_prompt: None,
    synthesis_stage_prompt: Some(prometheus_synthesis_stage_prompt),
    compose_interview_input: Some(compose_prometheus_interview_input),
    compose_plan_input: Some(compose_prometheus_plan_input),
    compose_execution_orchestration_input: None,
    compose_synthesis_input: None,
    compose_coordination_verification_input: None,
    compose_coordination_gate_input: None,
    compose_autonomous_verification_input: None,
    compose_autonomous_gate_input: None,
    compose_retry_input: None,
    compose_review_input: Some(compose_prometheus_review_input),
    compose_handoff_input: Some(compose_prometheus_handoff_input),
};

pub const PROMETHEUS_PRESET_BUNDLE: SchedulerPresetBundle = SchedulerPresetBundle {
    definition: PROMETHEUS_PRESET,
    platform: PROMETHEUS_PLATFORM,
};
