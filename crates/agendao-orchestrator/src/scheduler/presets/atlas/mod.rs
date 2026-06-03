use serde_json::json;

use super::{
    default_effect_dispatch, default_resolve_stage_kinds, default_transition_graph,
    passthrough_route_decision,
    runtime_enforcement::{
        validate_runtime_artifact_path, validate_runtime_orchestration_tool, RuntimeArtifactPolicy,
    },
    shared_execution_and_synthesis_effect_protocol, SchedulerPresetBundle,
    SchedulerPresetCapabilityHooks, SchedulerPresetEffectHooks, SchedulerPresetFinalizationHooks,
    SchedulerPresetGateHooks, SchedulerPresetPlatformSpec, SchedulerPresetProjectionHooks,
    SchedulerPresetPromptHooks, SchedulerPresetRouteHooks, SchedulerPresetStageGraphHooks,
    DEFAULT_INTERNAL_STAGE_HOOKS, PLACEMENT_DELEGATED_CLEAR_REVIEWED, PLACEMENT_REVIEWED_KEEP,
    STANDARD_FINAL_OUTPUT_PRIORITY,
};
use crate::scheduler::{
    SchedulerExecutionChildMode, SchedulerExecutionStageDispatch,
    SchedulerExecutionVerificationMode, SchedulerExecutionWorkflowPolicy,
    SchedulerFinalizationMode, SchedulerProfilePlan,
};

pub fn atlas_workflow_todos_payload() -> serde_json::Value {
    json!({
        "todos": [
            { "id": "atlas-1", "content": "Coordinate parallel execution across the selected workers", "status": "pending", "priority": "high" },
            { "id": "atlas-2", "content": "Run verification and settle the coordination gate", "status": "pending", "priority": "high" },
            { "id": "atlas-3", "content": "Synthesize the verified coordinator result", "status": "pending", "priority": "medium" }
        ]
    })
}

const ATLAS_RUNTIME_ORCHESTRATION_TOOLS: &[&str] = &["todowrite"];

fn validate_atlas_runtime_orchestration_tool(tool_name: &str) -> Result<(), String> {
    validate_runtime_orchestration_tool("Atlas", tool_name, ATLAS_RUNTIME_ORCHESTRATION_TOOLS)
}

fn validate_atlas_runtime_artifact_path(
    raw_path: &str,
    exec_ctx: &crate::ExecutionContext,
) -> Result<(), String> {
    validate_runtime_artifact_path(
        "Atlas",
        raw_path,
        exec_ctx,
        RuntimeArtifactPolicy::MarkdownUnder(".sisyphus/plans"),
    )
}

mod input;
mod output;
mod prompt;
mod runtime;

pub use input::*;
pub use output::*;
pub use prompt::*;
use runtime::{resolve_atlas_gate_terminal_content, sync_atlas_runtime_authority};

pub const ATLAS_CAPABILITY_HOOKS: SchedulerPresetCapabilityHooks = SchedulerPresetCapabilityHooks {
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
    validate_runtime_orchestration_tool: Some(validate_atlas_runtime_orchestration_tool),
    validate_runtime_artifact_path: Some(validate_atlas_runtime_artifact_path),
};

pub const ATLAS_STAGE_GRAPH_HOOKS: SchedulerPresetStageGraphHooks =
    SchedulerPresetStageGraphHooks {
        resolve_stage_kinds: default_resolve_stage_kinds,
        stage_tool_policy_override: None,
        stage_session_projection_override: None,
        stage_loop_budget_override: None,
        build_transition_graph: default_transition_graph,
    };

pub const ATLAS_ROUTE_HOOKS: SchedulerPresetRouteHooks = SchedulerPresetRouteHooks {
    route_constraint_note: None,
    constrain_route_decision: passthrough_route_decision,
};

pub const ATLAS_GATE_HOOKS: SchedulerPresetGateHooks = SchedulerPresetGateHooks {
    resolve_execution_stage_dispatch: atlas_execution_stage_dispatch,
    execution_workflow_policy: SchedulerExecutionWorkflowPolicy::coordination_loop(
        SchedulerExecutionChildMode::Parallel,
        true,
        SchedulerExecutionVerificationMode::Required,
        3,
    ),
    coordination_verification_placement: PLACEMENT_REVIEWED_KEEP,
    coordination_terminal_placement: PLACEMENT_REVIEWED_KEEP,
    autonomous_verification_placement: PLACEMENT_REVIEWED_KEEP,
    autonomous_terminal_placement: PLACEMENT_DELEGATED_CLEAR_REVIEWED,
    retry_exhausted_placement: PLACEMENT_REVIEWED_KEEP,
    coordination_verification_charter: Some(atlas_verification_charter),
    coordination_gate_contract: Some(atlas_gate_contract),
    coordination_gate_prompt: Some(atlas_gate_prompt),
    autonomous_verification_charter: None,
    autonomous_gate_contract: None,
    autonomous_gate_prompt: None,
    resolve_gate_terminal_content: Some(resolve_atlas_gate_terminal_content),
    resolve_runtime_transition_target: None,
};

pub const ATLAS_EFFECT_HOOKS: SchedulerPresetEffectHooks = SchedulerPresetEffectHooks {
    build_effect_protocol: shared_execution_and_synthesis_effect_protocol,
    resolve_effect_dispatch: default_effect_dispatch,
    effect_dispatch_is_authoritative: false,
};

pub const ATLAS_FINALIZATION_HOOKS: SchedulerPresetFinalizationHooks =
    SchedulerPresetFinalizationHooks {
        finalization_mode: SchedulerFinalizationMode::StandardSynthesis,
        output_priority: STANDARD_FINAL_OUTPUT_PRIORITY,
        extend_metadata: None,
        normalize_review_stage_output: None,
        normalize_final_output: Some(normalize_atlas_final_output),
        decorate_final_output: None,
    };

pub const ATLAS_PROJECTION_HOOKS: SchedulerPresetProjectionHooks = SchedulerPresetProjectionHooks {
    workflow_todos_payload: atlas_workflow_todos_payload,
    system_prompt_preview: atlas_system_prompt_preview,
    sync_runtime_authority: Some(sync_atlas_runtime_authority),
};

pub const ATLAS_PLATFORM: SchedulerPresetPlatformSpec = SchedulerPresetPlatformSpec {
    stage_graph: ATLAS_STAGE_GRAPH_HOOKS,
    route: ATLAS_ROUTE_HOOKS,
    gate: ATLAS_GATE_HOOKS,
    effect: ATLAS_EFFECT_HOOKS,
    internal: DEFAULT_INTERNAL_STAGE_HOOKS,
    finalization: ATLAS_FINALIZATION_HOOKS,
    projection: ATLAS_PROJECTION_HOOKS,
    prompts: ATLAS_PROMPT_HOOKS,
    capabilities: ATLAS_CAPABILITY_HOOKS,
};

fn atlas_execution_stage_dispatch() -> SchedulerExecutionStageDispatch {
    SchedulerExecutionStageDispatch::CoordinationLoop
}

fn atlas_execution_orchestration_charter(
    plan: &SchedulerProfilePlan,
    profile_suffix: &str,
) -> String {
    format!(
        "{}{}",
        build_atlas_dynamic_prompt(
            &plan.available_agents,
            &plan.available_categories,
            plan.effective_skill_list(Some(
                crate::scheduler::SchedulerStageKind::ExecutionOrchestration,
            )),
        ),
        profile_suffix,
    )
}

fn atlas_review_stage_prompt(profile_suffix: &str) -> String {
    format!(
        "You are the scheduler review layer for atlas mode. Audit the delegated result against the original request, tighten weak claims, and keep the answer faithful to evidence. Use read-only tools only when they materially improve verification.{}",
        profile_suffix
    )
}

fn atlas_execution_fallback_prompt(profile_suffix: &str) -> String {
    format!(
        "You are Atlas's execution fallback. Work from a task-list mindset: decompose, track, verify, and only claim completion with evidence. Use task/task_flow when helpful and preserve per-task status in your output.{}",
        profile_suffix
    )
}

pub const ATLAS_PROMPT_HOOKS: SchedulerPresetPromptHooks = SchedulerPresetPromptHooks {
    delegation_charter: None,
    execution_orchestration_charter: Some(atlas_execution_orchestration_charter),
    review_stage_prompt: Some(atlas_review_stage_prompt),
    interview_stage_prompt: None,
    plan_stage_prompt: None,
    handoff_stage_prompt: None,
    delegation_stage_prompt: None,
    execution_fallback_prompt: Some(atlas_execution_fallback_prompt),
    synthesis_stage_prompt: Some(atlas_synthesis_prompt),
    compose_interview_input: None,
    compose_plan_input: None,
    compose_execution_orchestration_input: Some(compose_atlas_execution_orchestration_input),
    compose_synthesis_input: Some(compose_atlas_synthesis_input),
    compose_coordination_verification_input: Some(compose_atlas_coordination_verification_input),
    compose_coordination_gate_input: Some(compose_atlas_coordination_gate_input),
    compose_autonomous_verification_input: None,
    compose_autonomous_gate_input: None,
    compose_retry_input: Some(compose_atlas_retry_input),
    compose_review_input: None,
    compose_handoff_input: None,
};

pub const ATLAS_PRESET_BUNDLE: SchedulerPresetBundle = SchedulerPresetBundle {
    definition: ATLAS_PRESET,
    platform: ATLAS_PLATFORM,
};

use super::super::{
    SchedulerPresetKind, SchedulerPresetMetadata, SchedulerProfileConfig,
    SchedulerProfileOrchestrator, SchedulerStageKind,
};
use super::{orchestrator_from_definition, plan_from_definition, SchedulerPresetDefinition};
use crate::tool_runner::ToolRunner;

const ATLAS_DEFAULT_STAGES: &[SchedulerStageKind] = &[
    SchedulerStageKind::RequestAnalysis,
    SchedulerStageKind::ExecutionOrchestration,
    SchedulerStageKind::Synthesis,
];

pub const ATLAS_PRESET: SchedulerPresetDefinition = SchedulerPresetDefinition {
    kind: SchedulerPresetKind::Atlas,
    metadata: SchedulerPresetMetadata {
        public: true,
        router_recommended: true,
        deprecated: false,
    },
    default_stages: ATLAS_DEFAULT_STAGES,
};

/// OMO Atlas-aligned orchestration: todo-list-driven parallel coordination.
///
/// Atlas keeps the shared coordination-workflow stage topology, but its runtime semantics are
/// stricter: coordination results must be verified before the gate can declare
/// completion, and verification can fall back to the scheduler review layer
/// when no verification graph is configured.
pub fn atlas_default_stages() -> Vec<SchedulerStageKind> {
    ATLAS_PRESET.default_stage_kinds()
}

pub type AtlasPlan = SchedulerProfilePlan;
pub type AtlasOrchestrator = SchedulerProfileOrchestrator;

pub fn atlas_plan() -> AtlasPlan {
    SchedulerProfilePlan::new(atlas_default_stages()).with_orchestrator("atlas")
}

pub fn atlas_plan_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
) -> AtlasPlan {
    plan_from_definition(profile_name, profile, ATLAS_PRESET)
}

pub fn atlas_orchestrator_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
) -> AtlasOrchestrator {
    orchestrator_from_definition(profile_name, profile, tool_runner, ATLAS_PRESET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SchedulerEffectContext, SchedulerEffectDispatch, SchedulerEffectKind};

    #[test]
    fn atlas_uses_coordination_stages() {
        assert_eq!(
            atlas_default_stages(),
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
            ]
        );
    }

    #[test]
    fn atlas_plan_sets_orchestrator() {
        let plan = atlas_plan();
        assert_eq!(plan.orchestrator.as_deref(), Some("atlas"));
    }

    #[test]
    fn atlas_effect_protocol_registers_workflow_todos_for_execution_and_synthesis() {
        let effects = atlas_plan().effect_protocol();
        assert!(effects.effects.iter().any(|effect| {
            effect.stage == SchedulerStageKind::ExecutionOrchestration
                && effect.moment == crate::SchedulerEffectMoment::OnEnter
                && effect.effect == SchedulerEffectKind::RegisterWorkflowTodos
        }));
        assert!(effects.effects.iter().any(|effect| {
            effect.stage == SchedulerStageKind::Synthesis
                && effect.moment == crate::SchedulerEffectMoment::OnEnter
                && effect.effect == SchedulerEffectKind::RegisterWorkflowTodos
        }));
    }

    #[test]
    fn atlas_uses_shared_effect_dispatch_framework() {
        let dispatch = atlas_plan().effect_dispatch(
            SchedulerEffectKind::PersistPlanningArtifact,
            SchedulerEffectContext {
                planning_artifact_path: Some("artifact.md".to_string()),
                draft_artifact_path: None,
                user_choice: None,
                review_gate_approved: None,
                draft_exists: true,
            },
        );

        assert_eq!(dispatch, SchedulerEffectDispatch::PersistPlanningArtifact);
    }
}
