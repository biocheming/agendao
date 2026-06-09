use serde_json::json;

use super::{
    default_effect_dispatch, default_resolve_stage_kinds, default_transition_graph,
    passthrough_route_decision,
    runtime_enforcement::{
        validate_runtime_artifact_path, validate_runtime_orchestration_tool, RuntimeArtifactPolicy,
    },
    shared_execution_stage_effect_protocol, SchedulerPresetBundle, SchedulerPresetCapabilityHooks,
    SchedulerPresetEffectHooks, SchedulerPresetFinalizationHooks, SchedulerPresetGateHooks,
    SchedulerPresetPlatformSpec, SchedulerPresetProjectionHooks, SchedulerPresetPromptHooks,
    SchedulerPresetRouteHooks, SchedulerPresetStageGraphHooks, DEFAULT_INTERNAL_STAGE_HOOKS,
    PLACEMENT_DELEGATED_CLEAR_REVIEWED, PLACEMENT_REVIEWED_KEEP, STANDARD_FINAL_OUTPUT_PRIORITY,
};
use crate::scheduler::{
    SchedulerExecutionStageDispatch, SchedulerExecutionWorkflowPolicy, SchedulerFinalizationMode,
    SchedulerProfilePlan,
};

pub fn sisyphus_workflow_todos_payload() -> serde_json::Value {
    json!({
        "todos": [
            { "id": "sisyphus-1", "content": "Classify intent and choose the execution path", "status": "pending", "priority": "high" },
            { "id": "sisyphus-2", "content": "Assess the codebase shape before following patterns", "status": "pending", "priority": "high" },
            { "id": "sisyphus-3", "content": "Explore and research in parallel before committing", "status": "pending", "priority": "high" },
            { "id": "sisyphus-4", "content": "Execute or delegate with explicit task tracking", "status": "pending", "priority": "high" },
            { "id": "sisyphus-5", "content": "Verify evidence and report concrete outcomes", "status": "pending", "priority": "high" }
        ]
    })
}

const SISYPHUS_RUNTIME_ORCHESTRATION_TOOLS: &[&str] = &["todowrite"];

fn validate_sisyphus_runtime_orchestration_tool(tool_name: &str) -> Result<(), String> {
    validate_runtime_orchestration_tool("Sisyphus", tool_name, SISYPHUS_RUNTIME_ORCHESTRATION_TOOLS)
}

fn validate_sisyphus_runtime_artifact_path(
    raw_path: &str,
    exec_ctx: &crate::ExecutionContext,
) -> Result<(), String> {
    validate_runtime_artifact_path(
        "Sisyphus",
        raw_path,
        exec_ctx,
        RuntimeArtifactPolicy::Disabled,
    )
}

mod api;
mod definition;
mod input;
mod output;
mod prompt;
mod runtime;
#[cfg(test)]
mod tests;

pub use api::*;
pub use definition::*;
pub use input::*;
pub use output::*;
pub use prompt::*;
use runtime::resolve_sisyphus_gate_terminal_content;

pub const SISYPHUS_CAPABILITY_HOOKS: SchedulerPresetCapabilityHooks =
    SchedulerPresetCapabilityHooks {
        runtime_update_for_advisory_review: None,
        runtime_update_for_user_choice: None,
        runtime_update_for_approval_review: None,
        runtime_update_for_planned_output: None,
        runtime_update_for_review_gate: None,
        compose_advisory_review_input: None,
        advisory_agent_name: None,
        user_choice_payload: None,
        parse_user_choice: None,
        default_user_choice: None,
        approval_review_agent_name: None,
        max_approval_review_rounds: None,
        approval_review_is_accepted: None,
        planning_artifact_relative_path: None,
        draft_artifact_relative_path: None,
        compose_draft_artifact: None,
        compose_planning_artifact: None,
        validate_runtime_orchestration_tool: Some(validate_sisyphus_runtime_orchestration_tool),
        validate_runtime_artifact_path: Some(validate_sisyphus_runtime_artifact_path),
    };

pub const SISYPHUS_STAGE_GRAPH_HOOKS: SchedulerPresetStageGraphHooks =
    SchedulerPresetStageGraphHooks {
        resolve_stage_kinds: default_resolve_stage_kinds,
        stage_tool_policy_override: None,
        stage_session_projection_override: None,
        stage_loop_budget_override: None,
        build_transition_graph: default_transition_graph,
    };

pub const SISYPHUS_ROUTE_HOOKS: SchedulerPresetRouteHooks = SchedulerPresetRouteHooks {
    route_constraint_note: None,
    constrain_route_decision: passthrough_route_decision,
};

pub const SISYPHUS_GATE_HOOKS: SchedulerPresetGateHooks = SchedulerPresetGateHooks {
    resolve_execution_stage_dispatch: sisyphus_execution_stage_dispatch,
    execution_workflow_policy: SchedulerExecutionWorkflowPolicy::single_pass(),
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
    resolve_gate_terminal_content: Some(resolve_sisyphus_gate_terminal_content),
    resolve_runtime_transition_target: None,
};

pub const SISYPHUS_EFFECT_HOOKS: SchedulerPresetEffectHooks = SchedulerPresetEffectHooks {
    build_effect_protocol: shared_execution_stage_effect_protocol,
    resolve_effect_dispatch: default_effect_dispatch,
    effect_dispatch_is_authoritative: false,
};

pub const SISYPHUS_FINALIZATION_HOOKS: SchedulerPresetFinalizationHooks =
    SchedulerPresetFinalizationHooks {
        finalization_mode: SchedulerFinalizationMode::StandardSynthesis,
        output_priority: STANDARD_FINAL_OUTPUT_PRIORITY,
        extend_metadata: None,
        normalize_review_stage_output: None,
        normalize_final_output: Some(normalize_sisyphus_final_output),
        decorate_final_output: None,
    };

pub const SISYPHUS_PROJECTION_HOOKS: SchedulerPresetProjectionHooks =
    SchedulerPresetProjectionHooks {
        workflow_todos_payload: sisyphus_workflow_todos_payload,
        system_prompt_preview: sisyphus_system_prompt_preview,
        sync_runtime_authority: None,
    };

pub const SISYPHUS_PLATFORM: SchedulerPresetPlatformSpec = SchedulerPresetPlatformSpec {
    stage_graph: SISYPHUS_STAGE_GRAPH_HOOKS,
    route: SISYPHUS_ROUTE_HOOKS,
    gate: SISYPHUS_GATE_HOOKS,
    effect: SISYPHUS_EFFECT_HOOKS,
    internal: DEFAULT_INTERNAL_STAGE_HOOKS,
    finalization: SISYPHUS_FINALIZATION_HOOKS,
    projection: SISYPHUS_PROJECTION_HOOKS,
    prompts: SISYPHUS_PROMPT_HOOKS,
    capabilities: SISYPHUS_CAPABILITY_HOOKS,
};

fn sisyphus_execution_stage_dispatch() -> SchedulerExecutionStageDispatch {
    SchedulerExecutionStageDispatch::SinglePass
}

fn sisyphus_dynamic_charter(plan: &SchedulerProfilePlan) -> String {
    let ext = sisyphus_prompt_extension(
        &plan.available_agents,
        &plan.available_categories,
        plan.effective_skill_list(Some(
            crate::scheduler::SchedulerStageKind::ExecutionOrchestration,
        )),
    );
    crate::scheduler::preset::render_preset_prompt_extension(&ext)
}

fn sisyphus_execution_orchestration_charter(
    plan: &SchedulerProfilePlan,
    _profile_suffix: &str,
) -> String {
    sisyphus_dynamic_charter(plan)
}

fn sisyphus_review_stage_prompt(profile_suffix: &str) -> String {
    format!(
        "You are the scheduler review layer for sisyphus mode. Audit the delegated result against the original request, tighten weak claims, and keep the answer faithful to evidence. Use read-only tools only when they materially improve verification.{}",
        profile_suffix
    )
}

fn sisyphus_delegation_stage_prompt(profile_suffix: &str) -> String {
    format!(
        "You are Sisyphus's delegation executor. Prefer delegating non-trivial work through AgenDao's task tools, but finish genuinely trivial work directly. Return only concrete execution results, not a fresh plan.{}",
        profile_suffix
    )
}

fn sisyphus_synthesis_stage_prompt(profile_suffix: &str) -> String {
    format!(
        "You are the final synthesis layer for AgenDao's scheduler (sisyphus mode, OMO-aligned). Merge prior stage outputs into a single final response for the user. Keep the answer faithful to actual stage results. Do not invent edits, tool calls, or conclusions. If there are remaining risks or follow-ups, state them clearly.{}",
        profile_suffix
    )
}

pub const SISYPHUS_PROMPT_HOOKS: SchedulerPresetPromptHooks = SchedulerPresetPromptHooks {
    delegation_charter: Some(sisyphus_dynamic_charter),
    execution_orchestration_charter: Some(sisyphus_execution_orchestration_charter),
    review_stage_prompt: Some(sisyphus_review_stage_prompt),
    interview_stage_prompt: None,
    plan_stage_prompt: None,
    handoff_stage_prompt: None,
    delegation_stage_prompt: Some(sisyphus_delegation_stage_prompt),
    execution_fallback_prompt: None,
    synthesis_stage_prompt: Some(sisyphus_synthesis_stage_prompt),
    compose_interview_input: None,
    compose_plan_input: None,
    compose_execution_orchestration_input: Some(compose_sisyphus_execution_orchestration_input),
    compose_synthesis_input: None,
    compose_coordination_verification_input: None,
    compose_coordination_gate_input: None,
    compose_autonomous_verification_input: None,
    compose_autonomous_gate_input: None,
    compose_retry_input: None,
    compose_review_input: None,
    compose_handoff_input: None,
};

pub const SISYPHUS_PRESET_BUNDLE: SchedulerPresetBundle = SchedulerPresetBundle {
    definition: SISYPHUS_PRESET,
    platform: SISYPHUS_PLATFORM,
};
