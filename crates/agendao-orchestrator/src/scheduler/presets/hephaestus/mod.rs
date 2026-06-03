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
    SchedulerExecutionChildMode, SchedulerExecutionStageDispatch,
    SchedulerExecutionVerificationMode, SchedulerExecutionWorkflowPolicy,
    SchedulerFinalizationMode, SchedulerProfilePlan,
};

pub fn hephaestus_workflow_todos_payload() -> serde_json::Value {
    json!({
        "todos": [
            { "id": "hephaestus-1", "content": "Run the autonomous execution loop to completion", "status": "pending", "priority": "high" },
            { "id": "hephaestus-2", "content": "Verify the deep worker result before finish gate", "status": "pending", "priority": "high" },
            { "id": "hephaestus-3", "content": "Return the finalized executor result", "status": "pending", "priority": "medium" }
        ]
    })
}

const HEPHAESTUS_RUNTIME_ORCHESTRATION_TOOLS: &[&str] = &["todowrite"];

fn validate_hephaestus_runtime_orchestration_tool(tool_name: &str) -> Result<(), String> {
    validate_runtime_orchestration_tool(
        "Hephaestus",
        tool_name,
        HEPHAESTUS_RUNTIME_ORCHESTRATION_TOOLS,
    )
}

fn validate_hephaestus_runtime_artifact_path(
    raw_path: &str,
    exec_ctx: &crate::ExecutionContext,
) -> Result<(), String> {
    validate_runtime_artifact_path(
        "Hephaestus",
        raw_path,
        exec_ctx,
        RuntimeArtifactPolicy::Disabled,
    )
}

mod input;
mod output;
mod prompt;
mod runtime;

pub use input::*;
pub use output::*;
pub use prompt::*;
use runtime::resolve_hephaestus_gate_terminal_content;

pub const HEPHAESTUS_CAPABILITY_HOOKS: SchedulerPresetCapabilityHooks =
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
        validate_runtime_orchestration_tool: Some(validate_hephaestus_runtime_orchestration_tool),
        validate_runtime_artifact_path: Some(validate_hephaestus_runtime_artifact_path),
    };

pub const HEPHAESTUS_STAGE_GRAPH_HOOKS: SchedulerPresetStageGraphHooks =
    SchedulerPresetStageGraphHooks {
        resolve_stage_kinds: default_resolve_stage_kinds,
        stage_tool_policy_override: None,
        stage_session_projection_override: None,
        stage_loop_budget_override: None,
        build_transition_graph: default_transition_graph,
    };

pub const HEPHAESTUS_ROUTE_HOOKS: SchedulerPresetRouteHooks = SchedulerPresetRouteHooks {
    route_constraint_note: None,
    constrain_route_decision: passthrough_route_decision,
};

pub const HEPHAESTUS_GATE_HOOKS: SchedulerPresetGateHooks = SchedulerPresetGateHooks {
    resolve_execution_stage_dispatch: hephaestus_execution_stage_dispatch,
    execution_workflow_policy: SchedulerExecutionWorkflowPolicy::autonomous_loop(
        SchedulerExecutionChildMode::Sequential,
        true,
        SchedulerExecutionVerificationMode::Required,
        3,
    ),
    coordination_verification_placement: PLACEMENT_REVIEWED_KEEP,
    coordination_terminal_placement: PLACEMENT_REVIEWED_KEEP,
    autonomous_verification_placement: PLACEMENT_REVIEWED_KEEP,
    autonomous_terminal_placement: PLACEMENT_DELEGATED_CLEAR_REVIEWED,
    retry_exhausted_placement: PLACEMENT_REVIEWED_KEEP,
    coordination_verification_charter: None,
    coordination_gate_contract: None,
    coordination_gate_prompt: None,
    autonomous_verification_charter: Some(hephaestus_verification_charter),
    autonomous_gate_contract: Some(hephaestus_gate_contract),
    autonomous_gate_prompt: Some(hephaestus_gate_prompt),
    resolve_gate_terminal_content: Some(resolve_hephaestus_gate_terminal_content),
    resolve_runtime_transition_target: None,
};

pub const HEPHAESTUS_EFFECT_HOOKS: SchedulerPresetEffectHooks = SchedulerPresetEffectHooks {
    build_effect_protocol: shared_execution_stage_effect_protocol,
    resolve_effect_dispatch: default_effect_dispatch,
    effect_dispatch_is_authoritative: false,
};

pub const HEPHAESTUS_FINALIZATION_HOOKS: SchedulerPresetFinalizationHooks =
    SchedulerPresetFinalizationHooks {
        finalization_mode: SchedulerFinalizationMode::StandardSynthesis,
        output_priority: STANDARD_FINAL_OUTPUT_PRIORITY,
        extend_metadata: None,
        normalize_review_stage_output: None,
        normalize_final_output: Some(normalize_hephaestus_final_output),
        decorate_final_output: None,
    };

pub const HEPHAESTUS_PROJECTION_HOOKS: SchedulerPresetProjectionHooks =
    SchedulerPresetProjectionHooks {
        workflow_todos_payload: hephaestus_workflow_todos_payload,
        system_prompt_preview: hephaestus_system_prompt_preview,
        sync_runtime_authority: None,
    };

pub const HEPHAESTUS_PLATFORM: SchedulerPresetPlatformSpec = SchedulerPresetPlatformSpec {
    stage_graph: HEPHAESTUS_STAGE_GRAPH_HOOKS,
    route: HEPHAESTUS_ROUTE_HOOKS,
    gate: HEPHAESTUS_GATE_HOOKS,
    effect: HEPHAESTUS_EFFECT_HOOKS,
    internal: DEFAULT_INTERNAL_STAGE_HOOKS,
    finalization: HEPHAESTUS_FINALIZATION_HOOKS,
    projection: HEPHAESTUS_PROJECTION_HOOKS,
    prompts: HEPHAESTUS_PROMPT_HOOKS,
    capabilities: HEPHAESTUS_CAPABILITY_HOOKS,
};

fn hephaestus_execution_stage_dispatch() -> SchedulerExecutionStageDispatch {
    SchedulerExecutionStageDispatch::AutonomousLoop
}

fn hephaestus_execution_orchestration_charter(
    plan: &SchedulerProfilePlan,
    profile_suffix: &str,
) -> String {
    format!(
        "{}{}",
        build_hephaestus_dynamic_prompt(
            &plan.available_agents,
            &plan.available_categories,
            plan.effective_skill_list(Some(
                crate::scheduler::SchedulerStageKind::ExecutionOrchestration,
            )),
        ),
        profile_suffix,
    )
}

fn hephaestus_execution_fallback_prompt(profile_suffix: &str) -> String {
    format!(
        "You are Hephaestus's autonomous execution layer. Run the full explore -> plan -> decide -> execute -> verify loop yourself. Do not stop at partial progress when further verified action is possible.{}",
        profile_suffix
    )
}

pub const HEPHAESTUS_PROMPT_HOOKS: SchedulerPresetPromptHooks = SchedulerPresetPromptHooks {
    delegation_charter: None,
    execution_orchestration_charter: Some(hephaestus_execution_orchestration_charter),
    review_stage_prompt: None,
    interview_stage_prompt: None,
    plan_stage_prompt: None,
    handoff_stage_prompt: None,
    delegation_stage_prompt: None,
    execution_fallback_prompt: Some(hephaestus_execution_fallback_prompt),
    synthesis_stage_prompt: None,
    compose_interview_input: None,
    compose_plan_input: None,
    compose_execution_orchestration_input: Some(compose_hephaestus_execution_orchestration_input),
    compose_synthesis_input: None,
    compose_coordination_verification_input: None,
    compose_coordination_gate_input: None,
    compose_autonomous_verification_input: Some(compose_hephaestus_autonomous_verification_input),
    compose_autonomous_gate_input: Some(compose_hephaestus_autonomous_gate_input),
    compose_retry_input: Some(compose_hephaestus_retry_input),
    compose_review_input: None,
    compose_handoff_input: None,
};

pub const HEPHAESTUS_PRESET_BUNDLE: SchedulerPresetBundle = SchedulerPresetBundle {
    definition: HEPHAESTUS_PRESET,
    platform: HEPHAESTUS_PLATFORM,
};

use super::super::{
    SchedulerPresetKind, SchedulerPresetMetadata, SchedulerProfileConfig,
    SchedulerProfileOrchestrator, SchedulerStageKind,
};
use super::{orchestrator_from_definition, plan_from_definition, SchedulerPresetDefinition};
use crate::tool_runner::ToolRunner;

const HEPHAESTUS_DEFAULT_STAGES: &[SchedulerStageKind] = &[
    SchedulerStageKind::RequestAnalysis,
    SchedulerStageKind::ExecutionOrchestration,
];

pub const HEPHAESTUS_PRESET: SchedulerPresetDefinition = SchedulerPresetDefinition {
    kind: SchedulerPresetKind::Hephaestus,
    metadata: SchedulerPresetMetadata {
        public: true,
        router_recommended: true,
        deprecated: false,
    },
    default_stages: HEPHAESTUS_DEFAULT_STAGES,
};

/// OMO Hephaestus-aligned orchestration: autonomous deep worker.
///
/// Hephaestus keeps the shared autonomous-workflow low-overhead topology, but the scheduler
/// now treats autonomous fallback execution as a first-class path and still
/// requires verification before the finish gate can settle the result.
pub fn hephaestus_default_stages() -> Vec<SchedulerStageKind> {
    HEPHAESTUS_PRESET.default_stage_kinds()
}

pub type HephaestusPlan = SchedulerProfilePlan;
pub type HephaestusOrchestrator = SchedulerProfileOrchestrator;

pub fn hephaestus_plan() -> HephaestusPlan {
    SchedulerProfilePlan::new(hephaestus_default_stages()).with_orchestrator("hephaestus")
}

pub fn hephaestus_plan_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
) -> HephaestusPlan {
    plan_from_definition(profile_name, profile, HEPHAESTUS_PRESET)
}

pub fn hephaestus_orchestrator_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
) -> HephaestusOrchestrator {
    orchestrator_from_definition(profile_name, profile, tool_runner, HEPHAESTUS_PRESET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SchedulerEffectContext, SchedulerEffectDispatch, SchedulerEffectKind};

    #[test]
    fn hephaestus_uses_low_overhead_stages() {
        assert_eq!(
            hephaestus_default_stages(),
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn hephaestus_plan_sets_orchestrator() {
        let plan = hephaestus_plan();
        assert_eq!(plan.orchestrator.as_deref(), Some("hephaestus"));
    }

    #[test]
    fn hephaestus_effect_protocol_registers_workflow_todos() {
        let effects = hephaestus_plan().effect_protocol();
        assert!(effects.effects.iter().any(|effect| {
            effect.stage == SchedulerStageKind::ExecutionOrchestration
                && effect.moment == crate::SchedulerEffectMoment::OnEnter
                && effect.effect == SchedulerEffectKind::RegisterWorkflowTodos
        }));
    }

    #[test]
    fn hephaestus_uses_shared_effect_dispatch_framework() {
        let dispatch = hephaestus_plan().effect_dispatch(
            SchedulerEffectKind::RequestAdvisoryReview,
            SchedulerEffectContext {
                planning_artifact_path: None,
                draft_artifact_path: None,
                user_choice: None,
                review_gate_approved: None,
                draft_exists: true,
            },
        );

        assert_eq!(dispatch, SchedulerEffectDispatch::RequestAdvisoryReview);
    }
}
