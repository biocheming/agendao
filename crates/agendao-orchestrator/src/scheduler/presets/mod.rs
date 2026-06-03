mod atlas;
mod hephaestus;
mod prometheus;
mod runtime_enforcement;
mod sisyphus;
mod verifier;

pub use atlas::*;
pub use hephaestus::*;
pub use prometheus::*;
pub use sisyphus::*;
pub use verifier::*;

use super::super::tool_runner::ToolRunner;
use super::profile_state::SchedulerPresetRuntimeState;
use super::{
    SchedulerAdvisoryReviewInput, SchedulerAutonomousGateStageInput,
    SchedulerAutonomousVerificationStageInput, SchedulerCoordinationGateStageInput,
    SchedulerCoordinationVerificationStageInput, SchedulerDraftArtifactInput,
    SchedulerEffectContext, SchedulerEffectDispatch, SchedulerEffectKind, SchedulerEffectMoment,
    SchedulerEffectProtocol, SchedulerEffectSpec, SchedulerExecutionGateDecision,
    SchedulerExecutionGateStatus, SchedulerExecutionOrchestrationStageInput,
    SchedulerExecutionStageDispatch, SchedulerExecutionWorkflowPolicy, SchedulerFinalizationMode,
    SchedulerFlowDefinition, SchedulerHandoffDecoration, SchedulerHandoffStageInput,
    SchedulerInterviewStageInput, SchedulerLoopBudget, SchedulerPlanStageInput,
    SchedulerPlanningArtifactInput, SchedulerPresetKind, SchedulerPresetMetadata,
    SchedulerPresetRuntimeFields, SchedulerPresetRuntimeUpdate, SchedulerProfileConfig,
    SchedulerProfileOrchestrator, SchedulerProfilePlan, SchedulerRetryStageInput,
    SchedulerReviewStageInput, SchedulerSessionProjection, SchedulerStageGraph, SchedulerStageKind,
    SchedulerStageObservability, SchedulerStagePolicy, SchedulerStageSpec,
    SchedulerSynthesisStageInput, SchedulerTransitionGraph, SchedulerTransitionSpec,
    SchedulerTransitionTarget, SchedulerTransitionTrigger, StageToolPolicy,
};
use crate::ExecutionContext;
use crate::OrchestratorContext;
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn normalize_embedded_delivery_summary(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    let Some((heading_index, canonical_heading)) =
        lines.iter().enumerate().find_map(|(index, line)| {
            embedded_structured_heading(line).map(|heading| (index, heading))
        })
    else {
        return trimmed.to_string();
    };

    let mut normalized = format!("## {canonical_heading}");
    for line in lines.iter().skip(heading_index + 1) {
        normalized.push('\n');
        normalized.push_str(line);
    }
    normalized.trim().to_string()
}

fn embedded_structured_heading(line: &str) -> Option<&'static str> {
    let cleaned = line.trim().replace('`', "");
    let without_leading_hashes = cleaned.trim().trim_start_matches('#').trim();
    let normalized = without_leading_hashes.trim_start_matches('#').trim();

    match normalized {
        "Delivery Summary" => Some("Delivery Summary"),
        "交付摘要" => Some("Delivery Summary"),
        "Plan Summary" => Some("Plan Summary"),
        _ => None,
    }
}

type NormalizeReviewStageOutputFn = for<'a> fn(SchedulerPresetRuntimeFields<'a>, &str) -> String;
type NormalizeFinalOutputFn = fn(&str) -> String;
type RuntimeStringUpdateFn = fn(String) -> SchedulerPresetRuntimeUpdate;
type RuntimeBoolUpdateFn = fn(bool) -> SchedulerPresetRuntimeUpdate;
type ComposeAdvisoryReviewInputFn = for<'a> fn(SchedulerAdvisoryReviewInput<'a>) -> String;
type UserChoicePayloadFn = fn() -> serde_json::Value;
type ParseUserChoiceFn = fn(&str) -> String;
type AgentNameFn = fn() -> &'static str;
type MaxRoundsFn = fn() -> usize;
type ApprovalAcceptedFn = fn(&str) -> bool;
type ArtifactRelativePathFn = fn(&str) -> String;
type ComposeDraftArtifactFn = for<'a> fn(SchedulerDraftArtifactInput<'a>) -> String;
type ComposePlanningArtifactFn = for<'a> fn(SchedulerPlanningArtifactInput<'a>) -> String;
type DecorateFinalOutputFn = fn(String, SchedulerHandoffDecoration) -> String;
type SyncRuntimeAuthorityFn = fn(&mut SchedulerPresetRuntimeState, &OrchestratorContext);
type ResolveGateTerminalContentFn =
    fn(SchedulerExecutionGateStatus, &SchedulerExecutionGateDecision, &str) -> Option<String>;
type StageToolPolicyOverrideFn = fn(SchedulerStageKind) -> Option<StageToolPolicy>;
type SessionProjectionOverrideFn = fn(SchedulerStageKind) -> Option<SchedulerSessionProjection>;
type LoopBudgetOverrideFn = fn(SchedulerStageKind) -> Option<SchedulerLoopBudget>;
type BuildTransitionGraphFn = fn(Vec<SchedulerTransitionSpec>) -> SchedulerTransitionGraph;
type BuildEffectProtocolFn = fn(&[SchedulerStageKind]) -> SchedulerEffectProtocol;
type ResolveEffectDispatchFn =
    fn(SchedulerEffectKind, SchedulerEffectContext) -> SchedulerEffectDispatch;
type ResolveExecutionStageDispatchFn = fn() -> SchedulerExecutionStageDispatch;
type ConstrainRouteDecisionFn =
    fn(crate::scheduler::RouteDecision) -> crate::scheduler::RouteDecision;
type ResolveStageKindsFn =
    fn(&SchedulerProfileConfig, &'static [SchedulerStageKind]) -> Vec<SchedulerStageKind>;
type ResolveRuntimeTransitionTargetFn = for<'a> fn(
    &[&'a SchedulerTransitionSpec],
    Option<&'a str>,
    Option<bool>,
) -> Option<SchedulerTransitionTarget>;
type WorkflowTodosPayloadFn = fn() -> serde_json::Value;
type SystemPromptPreviewFn = fn() -> &'static str;
type ValidateRuntimeOrchestrationToolFn = fn(&str) -> Result<(), String>;
type ValidateRuntimeArtifactPathFn = fn(&str, &ExecutionContext) -> Result<(), String>;
type DelegationCharterFn = fn(&SchedulerProfilePlan) -> String;
type ExecutionOrchestrationCharterFn = fn(&SchedulerProfilePlan, &str) -> String;
type FinalizationMetadataFn =
    fn(&super::profile_state::SchedulerProfileState, Option<&str>, &mut HashMap<String, Value>);
type StagePromptFn = fn(&str) -> String;
type ComposeInterviewInputFn = for<'a> fn(SchedulerInterviewStageInput<'a>) -> String;
type ComposePlanInputFn = for<'a> fn(SchedulerPlanStageInput<'a>) -> String;
type ComposeExecutionOrchestrationInputFn =
    for<'a> fn(SchedulerExecutionOrchestrationStageInput<'a>) -> String;
type ComposeSynthesisInputFn = for<'a> fn(SchedulerSynthesisStageInput<'a>) -> String;
type ComposeCoordinationVerificationInputFn =
    for<'a> fn(SchedulerCoordinationVerificationStageInput<'a>) -> String;
type ComposeCoordinationGateInputFn = for<'a> fn(SchedulerCoordinationGateStageInput<'a>) -> String;
type ComposeAutonomousVerificationInputFn =
    for<'a> fn(SchedulerAutonomousVerificationStageInput<'a>) -> String;
type ComposeAutonomousGateInputFn = for<'a> fn(SchedulerAutonomousGateStageInput<'a>) -> String;
type ComposeRetryInputFn = for<'a> fn(SchedulerRetryStageInput<'a>) -> String;
type ComposeReviewInputFn = for<'a> fn(SchedulerReviewStageInput<'a>) -> String;
type ComposeHandoffInputFn = for<'a> fn(SchedulerHandoffStageInput<'a>) -> String;
type StaticTextFn = fn() -> &'static str;

#[derive(Clone, Copy)]
pub struct SchedulerPresetStageGraphHooks {
    pub resolve_stage_kinds: ResolveStageKindsFn,
    pub stage_tool_policy_override: Option<StageToolPolicyOverrideFn>,
    pub stage_session_projection_override: Option<SessionProjectionOverrideFn>,
    pub stage_loop_budget_override: Option<LoopBudgetOverrideFn>,
    pub build_transition_graph: BuildTransitionGraphFn,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetRouteHooks {
    pub route_constraint_note: Option<&'static str>,
    pub constrain_route_decision: ConstrainRouteDecisionFn,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetGateHooks {
    pub resolve_execution_stage_dispatch: ResolveExecutionStageDispatchFn,
    pub execution_workflow_policy: SchedulerExecutionWorkflowPolicy,
    pub coordination_verification_placement: SchedulerExecutionPlacement,
    pub coordination_terminal_placement: SchedulerExecutionPlacement,
    pub autonomous_verification_placement: SchedulerExecutionPlacement,
    pub autonomous_terminal_placement: SchedulerExecutionPlacement,
    pub retry_exhausted_placement: SchedulerExecutionPlacement,
    pub coordination_verification_charter: Option<StaticTextFn>,
    pub coordination_gate_contract: Option<StaticTextFn>,
    pub coordination_gate_prompt: Option<StaticTextFn>,
    pub autonomous_verification_charter: Option<StaticTextFn>,
    pub autonomous_gate_contract: Option<StaticTextFn>,
    pub autonomous_gate_prompt: Option<StaticTextFn>,
    pub resolve_gate_terminal_content: Option<ResolveGateTerminalContentFn>,
    pub resolve_runtime_transition_target: Option<ResolveRuntimeTransitionTargetFn>,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetEffectHooks {
    pub build_effect_protocol: BuildEffectProtocolFn,
    pub resolve_effect_dispatch: ResolveEffectDispatchFn,
    pub effect_dispatch_is_authoritative: bool,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetInternalStageHooks {
    pub single_pass_executor: SchedulerInternalStageSpec,
    pub coordination_verification: SchedulerInternalStageSpec,
    pub coordination_gate: SchedulerInternalStageSpec,
    pub coordination_retry_event: &'static str,
    pub autonomous_verification: SchedulerInternalStageSpec,
    pub autonomous_gate: SchedulerInternalStageSpec,
    pub autonomous_retry_event: &'static str,
    pub coordination_verification_fallback: SchedulerVerificationFallback,
    pub autonomous_verification_failure: SchedulerVerificationFailurePolicy,
    pub coordination_semantics_error: &'static str,
    pub autonomous_semantics_error: &'static str,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetFinalizationHooks {
    pub finalization_mode: SchedulerFinalizationMode,
    pub output_priority: &'static [SchedulerFinalOutputSource],
    pub(super) extend_metadata: Option<FinalizationMetadataFn>,
    pub normalize_review_stage_output: Option<NormalizeReviewStageOutputFn>,
    pub normalize_final_output: Option<NormalizeFinalOutputFn>,
    pub decorate_final_output: Option<DecorateFinalOutputFn>,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetProjectionHooks {
    pub workflow_todos_payload: WorkflowTodosPayloadFn,
    pub system_prompt_preview: SystemPromptPreviewFn,
    pub(super) sync_runtime_authority: Option<SyncRuntimeAuthorityFn>,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetCapabilityHooks {
    pub runtime_update_for_advisory_review: Option<RuntimeStringUpdateFn>,
    pub runtime_update_for_user_choice: Option<RuntimeStringUpdateFn>,
    pub runtime_update_for_approval_review: Option<RuntimeStringUpdateFn>,
    pub runtime_update_for_planned_output: Option<RuntimeStringUpdateFn>,
    pub runtime_update_for_review_gate: Option<RuntimeBoolUpdateFn>,
    pub compose_advisory_review_input: Option<ComposeAdvisoryReviewInputFn>,
    pub advisory_agent_name: Option<AgentNameFn>,
    pub user_choice_payload: Option<UserChoicePayloadFn>,
    pub parse_user_choice: Option<ParseUserChoiceFn>,
    pub default_user_choice: Option<AgentNameFn>,
    pub approval_review_agent_name: Option<AgentNameFn>,
    pub max_approval_review_rounds: Option<MaxRoundsFn>,
    pub approval_review_is_accepted: Option<ApprovalAcceptedFn>,
    pub planning_artifact_relative_path: Option<ArtifactRelativePathFn>,
    pub draft_artifact_relative_path: Option<ArtifactRelativePathFn>,
    pub compose_draft_artifact: Option<ComposeDraftArtifactFn>,
    pub compose_planning_artifact: Option<ComposePlanningArtifactFn>,
    pub validate_runtime_orchestration_tool: Option<ValidateRuntimeOrchestrationToolFn>,
    pub validate_runtime_artifact_path: Option<ValidateRuntimeArtifactPathFn>,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetPromptHooks {
    pub delegation_charter: Option<DelegationCharterFn>,
    pub execution_orchestration_charter: Option<ExecutionOrchestrationCharterFn>,
    pub review_stage_prompt: Option<StagePromptFn>,
    pub interview_stage_prompt: Option<StagePromptFn>,
    pub plan_stage_prompt: Option<StagePromptFn>,
    pub handoff_stage_prompt: Option<StagePromptFn>,
    pub delegation_stage_prompt: Option<StagePromptFn>,
    pub execution_fallback_prompt: Option<StagePromptFn>,
    pub synthesis_stage_prompt: Option<StagePromptFn>,
    pub compose_interview_input: Option<ComposeInterviewInputFn>,
    pub compose_plan_input: Option<ComposePlanInputFn>,
    pub compose_execution_orchestration_input: Option<ComposeExecutionOrchestrationInputFn>,
    pub compose_synthesis_input: Option<ComposeSynthesisInputFn>,
    pub compose_coordination_verification_input: Option<ComposeCoordinationVerificationInputFn>,
    pub compose_coordination_gate_input: Option<ComposeCoordinationGateInputFn>,
    pub compose_autonomous_verification_input: Option<ComposeAutonomousVerificationInputFn>,
    pub compose_autonomous_gate_input: Option<ComposeAutonomousGateInputFn>,
    pub compose_retry_input: Option<ComposeRetryInputFn>,
    pub compose_review_input: Option<ComposeReviewInputFn>,
    pub compose_handoff_input: Option<ComposeHandoffInputFn>,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetPlatformSpec {
    pub stage_graph: SchedulerPresetStageGraphHooks,
    pub route: SchedulerPresetRouteHooks,
    pub gate: SchedulerPresetGateHooks,
    pub effect: SchedulerPresetEffectHooks,
    pub internal: SchedulerPresetInternalStageHooks,
    pub finalization: SchedulerPresetFinalizationHooks,
    pub projection: SchedulerPresetProjectionHooks,
    pub prompts: SchedulerPresetPromptHooks,
    pub capabilities: SchedulerPresetCapabilityHooks,
}

#[derive(Clone, Copy)]
pub struct SchedulerPresetBundle {
    pub definition: SchedulerPresetDefinition,
    pub platform: SchedulerPresetPlatformSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerExecutionOutputSlot {
    Delegated,
    Reviewed,
    HandedOff,
    Synthesized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerExecutionPlacement {
    pub slot: SchedulerExecutionOutputSlot,
    pub clear_slots: &'static [SchedulerExecutionOutputSlot],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerFinalOutputSource {
    HandedOff,
    Reviewed,
    Planned,
    Synthesized,
    Delegated,
    Routed,
    RequestBrief,
}

#[derive(Debug, Clone, Copy)]
pub struct SchedulerInternalStageSpec {
    pub event_name: &'static str,
    pub agent_name: &'static str,
    pub tool_policy: StageToolPolicy,
    pub fallback_prompt: StagePromptFn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerVerificationFallback {
    Skip,
    ReviewStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerVerificationFailurePolicy {
    Continue,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerPresetDefinition {
    pub kind: SchedulerPresetKind,
    pub metadata: SchedulerPresetMetadata,
    pub default_stages: &'static [SchedulerStageKind],
}

impl SchedulerPresetDefinition {
    fn bundle(self) -> &'static SchedulerPresetBundle {
        match self.kind {
            SchedulerPresetKind::Sisyphus => &SISYPHUS_PRESET_BUNDLE,
            SchedulerPresetKind::Prometheus => &PROMETHEUS_PRESET_BUNDLE,
            SchedulerPresetKind::Atlas => &ATLAS_PRESET_BUNDLE,
            SchedulerPresetKind::Hephaestus => &HEPHAESTUS_PRESET_BUNDLE,
            SchedulerPresetKind::Verifier => &VERIFIER_PRESET_BUNDLE,
        }
    }

    pub fn default_stage_kinds(self) -> Vec<SchedulerStageKind> {
        self.default_stages.to_vec()
    }

    pub fn resolved_stage_kinds(self, profile: &SchedulerProfileConfig) -> Vec<SchedulerStageKind> {
        (self.bundle().platform.stage_graph.resolve_stage_kinds)(profile, self.default_stages)
    }

    pub fn post_route_stage_kinds(self) -> Vec<SchedulerStageKind> {
        self.default_stages
            .iter()
            .copied()
            .filter(|stage| {
                !matches!(
                    stage,
                    SchedulerStageKind::RequestAnalysis | SchedulerStageKind::Route
                )
            })
            .collect()
    }

    pub fn stage_policy(self, stage: SchedulerStageKind) -> SchedulerStagePolicy {
        let session_projection = self
            .bundle()
            .platform
            .stage_graph
            .stage_session_projection_override
            .and_then(|override_projection| override_projection(stage))
            .unwrap_or_else(|| default_stage_session_projection(stage));

        let tool_policy = self
            .bundle()
            .platform
            .stage_graph
            .stage_tool_policy_override
            .and_then(|override_policy| override_policy(stage))
            .unwrap_or_else(|| default_stage_tool_policy(stage));
        let loop_budget = self
            .bundle()
            .platform
            .stage_graph
            .stage_loop_budget_override
            .and_then(|override_budget| override_budget(stage))
            .unwrap_or_else(|| default_stage_loop_budget(stage));

        SchedulerStagePolicy {
            session_projection,
            tool_policy,
            loop_budget,
            attached_session: default_stage_attached_session(stage),
        }
    }

    pub fn stage_graph(self, stages: &[SchedulerStageKind]) -> SchedulerStageGraph {
        SchedulerStageGraph::new(
            stages
                .iter()
                .copied()
                .map(|kind| SchedulerStageSpec {
                    kind,
                    policy: self.stage_policy(kind),
                    capabilities: None,
                })
                .collect(),
        )
    }

    pub fn transition_graph(self, stages: &[SchedulerStageKind]) -> SchedulerTransitionGraph {
        let mut transitions = stages
            .windows(2)
            .map(|window| SchedulerTransitionSpec {
                from: window[0],
                trigger: SchedulerTransitionTrigger::OnSuccess,
                to: SchedulerTransitionTarget::Stage(window[1]),
            })
            .collect::<Vec<_>>();

        if let Some(last) = stages.last().copied() {
            transitions.push(SchedulerTransitionSpec {
                from: last,
                trigger: SchedulerTransitionTrigger::OnSuccess,
                to: SchedulerTransitionTarget::Finish,
            });
        }

        (self.bundle().platform.stage_graph.build_transition_graph)(transitions)
    }

    pub fn effect_protocol(self, stages: &[SchedulerStageKind]) -> SchedulerEffectProtocol {
        (self.bundle().platform.effect.build_effect_protocol)(stages)
    }

    pub fn resolve_effect_dispatch(
        self,
        effect: SchedulerEffectKind,
        context: SchedulerEffectContext,
    ) -> SchedulerEffectDispatch {
        (self.bundle().platform.effect.resolve_effect_dispatch)(effect, context)
    }

    pub fn effect_dispatch_is_authoritative(self) -> bool {
        self.bundle()
            .platform
            .effect
            .effect_dispatch_is_authoritative
    }

    pub fn route_constraint_note(self) -> Option<&'static str> {
        self.bundle().platform.route.route_constraint_note
    }

    pub fn constrain_route_decision(
        self,
        decision: crate::scheduler::RouteDecision,
    ) -> crate::scheduler::RouteDecision {
        (self.bundle().platform.route.constrain_route_decision)(decision)
    }

    pub fn normalize_review_stage_output(
        self,
        runtime: SchedulerPresetRuntimeFields<'_>,
        output: &str,
    ) -> Option<String> {
        self.bundle()
            .platform
            .finalization
            .normalize_review_stage_output
            .map(|normalize| normalize(runtime, output))
    }

    pub fn normalize_final_output(self, output: &str) -> Option<String> {
        self.bundle()
            .platform
            .finalization
            .normalize_final_output
            .map(|normalize| normalize(output))
    }

    pub fn runtime_update_for_advisory_review(
        self,
        content: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.bundle()
            .platform
            .capabilities
            .runtime_update_for_advisory_review
            .map(|update| update(content))
    }

    pub fn runtime_update_for_user_choice(
        self,
        choice: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.bundle()
            .platform
            .capabilities
            .runtime_update_for_user_choice
            .map(|update| update(choice))
    }

    pub fn runtime_update_for_approval_review(
        self,
        content: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.bundle()
            .platform
            .capabilities
            .runtime_update_for_approval_review
            .map(|update| update(content))
    }

    pub fn runtime_update_for_planned_output(
        self,
        content: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.bundle()
            .platform
            .capabilities
            .runtime_update_for_planned_output
            .map(|update| update(content))
    }

    pub fn runtime_update_for_review_gate(
        self,
        approved: bool,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.bundle()
            .platform
            .capabilities
            .runtime_update_for_review_gate
            .map(|update| update(approved))
    }

    pub fn execution_workflow_policy(self) -> SchedulerExecutionWorkflowPolicy {
        self.bundle().platform.gate.execution_workflow_policy
    }

    pub fn execution_stage_dispatch(self) -> SchedulerExecutionStageDispatch {
        (self.bundle().platform.gate.resolve_execution_stage_dispatch)()
    }

    pub fn coordination_verification_placement(self) -> SchedulerExecutionPlacement {
        self.bundle()
            .platform
            .gate
            .coordination_verification_placement
    }

    pub fn coordination_terminal_placement(self) -> SchedulerExecutionPlacement {
        self.bundle().platform.gate.coordination_terminal_placement
    }

    pub fn autonomous_verification_placement(self) -> SchedulerExecutionPlacement {
        self.bundle()
            .platform
            .gate
            .autonomous_verification_placement
    }

    pub fn autonomous_terminal_placement(self) -> SchedulerExecutionPlacement {
        self.bundle().platform.gate.autonomous_terminal_placement
    }

    pub fn retry_exhausted_placement(self) -> SchedulerExecutionPlacement {
        self.bundle().platform.gate.retry_exhausted_placement
    }

    pub fn flow_definition(self, stages: &[SchedulerStageKind]) -> SchedulerFlowDefinition {
        SchedulerFlowDefinition {
            stage_graph: self.stage_graph(stages),
            transition_graph: self.transition_graph(stages),
            effect_protocol: self.effect_protocol(stages),
            execution_workflow_policy: self.execution_workflow_policy(),
            finalization_mode: self.finalization_mode(),
        }
    }

    pub fn finalization_mode(self) -> SchedulerFinalizationMode {
        self.bundle().platform.finalization.finalization_mode
    }

    pub fn final_output_priority(self) -> &'static [SchedulerFinalOutputSource] {
        self.bundle().platform.finalization.output_priority
    }

    pub fn single_pass_executor_stage(self) -> SchedulerInternalStageSpec {
        self.bundle().platform.internal.single_pass_executor
    }

    pub fn coordination_verification_stage(self) -> SchedulerInternalStageSpec {
        self.bundle().platform.internal.coordination_verification
    }

    pub fn coordination_gate_stage(self) -> SchedulerInternalStageSpec {
        self.bundle().platform.internal.coordination_gate
    }

    pub fn coordination_retry_event(self) -> &'static str {
        self.bundle().platform.internal.coordination_retry_event
    }

    pub fn autonomous_verification_stage(self) -> SchedulerInternalStageSpec {
        self.bundle().platform.internal.autonomous_verification
    }

    pub fn autonomous_gate_stage(self) -> SchedulerInternalStageSpec {
        self.bundle().platform.internal.autonomous_gate
    }

    pub fn autonomous_retry_event(self) -> &'static str {
        self.bundle().platform.internal.autonomous_retry_event
    }

    pub fn coordination_verification_fallback(self) -> SchedulerVerificationFallback {
        self.bundle()
            .platform
            .internal
            .coordination_verification_fallback
    }

    pub fn autonomous_verification_failure_policy(self) -> SchedulerVerificationFailurePolicy {
        self.bundle()
            .platform
            .internal
            .autonomous_verification_failure
    }

    pub fn coordination_semantics_error(self) -> &'static str {
        self.bundle().platform.internal.coordination_semantics_error
    }

    pub fn autonomous_semantics_error(self) -> &'static str {
        self.bundle().platform.internal.autonomous_semantics_error
    }

    pub(super) fn extend_final_output_metadata(
        self,
        state: &super::profile_state::SchedulerProfileState,
        artifact_path: Option<&str>,
        metadata: &mut HashMap<String, Value>,
    ) {
        if let Some(extend) = self.bundle().platform.finalization.extend_metadata {
            extend(state, artifact_path, metadata);
        }
    }

    pub fn delegation_charter(self, plan: &SchedulerProfilePlan) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .delegation_charter
            .map(|build| build(plan))
    }

    pub fn execution_orchestration_charter(
        self,
        plan: &SchedulerProfilePlan,
        profile_suffix: &str,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .execution_orchestration_charter
            .map(|build| build(plan, profile_suffix))
    }

    pub fn review_stage_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .review_stage_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn interview_stage_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .interview_stage_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn plan_stage_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .plan_stage_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn handoff_stage_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .handoff_stage_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn compose_interview_input(
        self,
        input: SchedulerInterviewStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_interview_input
            .map(|compose| compose(input))
    }

    pub fn compose_plan_input(self, input: SchedulerPlanStageInput<'_>) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_plan_input
            .map(|compose| compose(input))
    }

    pub fn compose_execution_orchestration_input(
        self,
        input: SchedulerExecutionOrchestrationStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_execution_orchestration_input
            .map(|compose| compose(input))
    }

    pub fn compose_synthesis_input(
        self,
        input: SchedulerSynthesisStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_synthesis_input
            .map(|compose| compose(input))
    }

    pub fn compose_coordination_verification_input(
        self,
        input: SchedulerCoordinationVerificationStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_coordination_verification_input
            .map(|compose| compose(input))
    }

    pub fn compose_coordination_gate_input(
        self,
        input: SchedulerCoordinationGateStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_coordination_gate_input
            .map(|compose| compose(input))
    }

    pub fn compose_autonomous_verification_input(
        self,
        input: SchedulerAutonomousVerificationStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_autonomous_verification_input
            .map(|compose| compose(input))
    }

    pub fn compose_autonomous_gate_input(
        self,
        input: SchedulerAutonomousGateStageInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_autonomous_gate_input
            .map(|compose| compose(input))
    }

    pub fn compose_review_input(self, input: SchedulerReviewStageInput<'_>) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_review_input
            .map(|compose| compose(input))
    }

    pub fn compose_retry_input(self, input: SchedulerRetryStageInput<'_>) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_retry_input
            .map(|compose| compose(input))
    }

    pub fn compose_handoff_input(self, input: SchedulerHandoffStageInput<'_>) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .compose_handoff_input
            .map(|compose| compose(input))
    }

    pub fn compose_advisory_review_input(
        self,
        input: SchedulerAdvisoryReviewInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .capabilities
            .compose_advisory_review_input
            .map(|compose| compose(input))
    }

    pub fn advisory_agent_name(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .capabilities
            .advisory_agent_name
            .map(|name| name())
    }

    pub fn user_choice_payload(self) -> Option<serde_json::Value> {
        self.bundle()
            .platform
            .capabilities
            .user_choice_payload
            .map(|payload| payload())
    }

    pub fn parse_user_choice(self, output: &str) -> Option<String> {
        self.bundle()
            .platform
            .capabilities
            .parse_user_choice
            .map(|parse| parse(output))
    }

    pub fn default_user_choice(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .capabilities
            .default_user_choice
            .map(|choice| choice())
    }

    pub fn approval_review_agent_name(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .capabilities
            .approval_review_agent_name
            .map(|name| name())
    }

    pub fn max_approval_review_rounds(self) -> Option<usize> {
        self.bundle()
            .platform
            .capabilities
            .max_approval_review_rounds
            .map(|max_rounds| max_rounds())
    }

    pub fn approval_review_is_accepted(self, output: &str) -> bool {
        self.bundle()
            .platform
            .capabilities
            .approval_review_is_accepted
            .map(|accepts| accepts(output))
            .unwrap_or(false)
    }

    pub fn delegation_stage_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .delegation_stage_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn execution_fallback_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .execution_fallback_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn synthesis_stage_prompt(self, profile_suffix: &str) -> Option<String> {
        self.bundle()
            .platform
            .prompts
            .synthesis_stage_prompt
            .map(|prompt| prompt(profile_suffix))
    }

    pub fn coordination_verification_charter(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .gate
            .coordination_verification_charter
            .map(|prompt| prompt())
    }

    pub fn coordination_gate_contract(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .gate
            .coordination_gate_contract
            .map(|prompt| prompt())
    }

    pub fn coordination_gate_prompt(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .gate
            .coordination_gate_prompt
            .map(|prompt| prompt())
    }

    pub fn autonomous_verification_charter(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .gate
            .autonomous_verification_charter
            .map(|prompt| prompt())
    }

    pub fn autonomous_gate_contract(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .gate
            .autonomous_gate_contract
            .map(|prompt| prompt())
    }

    pub fn autonomous_gate_prompt(self) -> Option<&'static str> {
        self.bundle()
            .platform
            .gate
            .autonomous_gate_prompt
            .map(|prompt| prompt())
    }

    pub fn workflow_todos_payload(self) -> serde_json::Value {
        (self.bundle().platform.projection.workflow_todos_payload)()
    }

    pub fn planning_artifact_relative_path(self, session_id: &str) -> Option<String> {
        self.bundle()
            .platform
            .capabilities
            .planning_artifact_relative_path
            .map(|build| build(session_id))
    }

    pub fn draft_artifact_relative_path(self, session_id: &str) -> Option<String> {
        self.bundle()
            .platform
            .capabilities
            .draft_artifact_relative_path
            .map(|build| build(session_id))
    }

    pub fn compose_draft_artifact(self, input: SchedulerDraftArtifactInput<'_>) -> Option<String> {
        self.bundle()
            .platform
            .capabilities
            .compose_draft_artifact
            .map(|compose| compose(input))
    }

    pub fn compose_planning_artifact(
        self,
        input: SchedulerPlanningArtifactInput<'_>,
    ) -> Option<String> {
        self.bundle()
            .platform
            .capabilities
            .compose_planning_artifact
            .map(|compose| compose(input))
    }

    pub fn decorate_final_output(
        self,
        content: String,
        decoration: SchedulerHandoffDecoration,
    ) -> String {
        self.bundle()
            .platform
            .finalization
            .decorate_final_output
            .map(|decorate| decorate(content.clone(), decoration))
            .unwrap_or(content)
    }

    pub fn validate_runtime_orchestration_tool(self, tool_name: &str) -> Result<(), String> {
        self.bundle()
            .platform
            .capabilities
            .validate_runtime_orchestration_tool
            .map(|validate| validate(tool_name))
            .unwrap_or(Ok(()))
    }

    pub fn validate_runtime_artifact_path(
        self,
        raw_path: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), String> {
        self.bundle()
            .platform
            .capabilities
            .validate_runtime_artifact_path
            .map(|validate| validate(raw_path, exec_ctx))
            .unwrap_or(Ok(()))
    }

    pub(super) fn sync_runtime_authority(
        self,
        runtime: &mut SchedulerPresetRuntimeState,
        ctx: &OrchestratorContext,
    ) {
        if let Some(sync) = self.bundle().platform.projection.sync_runtime_authority {
            sync(runtime, ctx);
        }
    }

    pub fn resolve_gate_terminal_content(
        self,
        status: SchedulerExecutionGateStatus,
        decision: &SchedulerExecutionGateDecision,
        fallback_content: &str,
    ) -> Option<String> {
        self.bundle()
            .platform
            .gate
            .resolve_gate_terminal_content
            .and_then(|resolve| resolve(status, decision, fallback_content))
            .or_else(|| default_gate_terminal_content(status, decision, fallback_content))
    }

    pub fn resolve_runtime_transition_target(
        self,
        transitions: &[&SchedulerTransitionSpec],
        user_choice: Option<&str>,
        review_gate_approved: Option<bool>,
    ) -> Option<SchedulerTransitionTarget> {
        self.bundle()
            .platform
            .gate
            .resolve_runtime_transition_target
            .and_then(|resolve| resolve(transitions, user_choice, review_gate_approved))
    }

    pub fn system_prompt_preview(self) -> &'static str {
        (self.bundle().platform.projection.system_prompt_preview)()
    }

    pub fn stage_observability(self, stage: SchedulerStageKind) -> SchedulerStageObservability {
        let policy = self.stage_policy(stage);
        SchedulerStageObservability {
            projection: policy.session_projection.label().to_string(),
            tool_policy: policy.tool_policy.label(),
            loop_budget: policy.loop_budget.label(),
        }
    }
}

fn shared_execution_workflow_effect_protocol(
    stages: &[SchedulerStageKind],
    workflow_stages: &[SchedulerStageKind],
) -> SchedulerEffectProtocol {
    let effects = workflow_stages
        .iter()
        .copied()
        .filter(|stage| stages.contains(stage))
        .map(|stage| SchedulerEffectSpec {
            stage,
            moment: SchedulerEffectMoment::OnEnter,
            effect: SchedulerEffectKind::RegisterWorkflowTodos,
        })
        .collect();
    SchedulerEffectProtocol::new(effects)
}

pub(super) fn default_effect_dispatch(
    effect: SchedulerEffectKind,
    context: SchedulerEffectContext,
) -> SchedulerEffectDispatch {
    match effect {
        SchedulerEffectKind::EnsurePlanningArtifactPath => {
            SchedulerEffectDispatch::EnsurePlanningArtifactPath
        }
        SchedulerEffectKind::PersistPlanningArtifact => {
            SchedulerEffectDispatch::PersistPlanningArtifact
        }
        SchedulerEffectKind::PersistDraftArtifact | SchedulerEffectKind::SyncDraftArtifact => {
            SchedulerEffectDispatch::SyncDraftArtifact
        }
        SchedulerEffectKind::RegisterWorkflowTodos => {
            SchedulerEffectDispatch::RegisterWorkflowTodos
        }
        SchedulerEffectKind::RequestAdvisoryReview => {
            SchedulerEffectDispatch::RequestAdvisoryReview
        }
        SchedulerEffectKind::RequestUserChoice => SchedulerEffectDispatch::RequestUserChoice,
        SchedulerEffectKind::RunApprovalReviewLoop => {
            SchedulerEffectDispatch::RunApprovalReviewLoop
        }
        SchedulerEffectKind::DeleteDraftArtifact => SchedulerEffectDispatch::DeleteDraftArtifact,
        SchedulerEffectKind::DecorateFinalOutput => {
            SchedulerEffectDispatch::DecorateFinalOutput(SchedulerHandoffDecoration {
                plan_path: context.planning_artifact_path,
                draft_path: context.draft_artifact_path,
                draft_deleted: !context.draft_exists,
                recommend_start_work: true,
                review_gate_approved: context.review_gate_approved,
            })
        }
    }
}

pub(super) fn default_gate_terminal_content(
    status: SchedulerExecutionGateStatus,
    decision: &SchedulerExecutionGateDecision,
    _fallback_content: &str,
) -> Option<String> {
    match status {
        SchedulerExecutionGateStatus::Done => decision
            .final_response
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        SchedulerExecutionGateStatus::Blocked => {
            let blocked = decision
                .final_response
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| decision.summary.clone());
            (!blocked.trim().is_empty()).then_some(blocked)
        }
        SchedulerExecutionGateStatus::Continue => None,
    }
}

pub(super) fn plan_from_definition(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    definition: SchedulerPresetDefinition,
) -> SchedulerProfilePlan {
    let mut plan = SchedulerProfilePlan::from_profile_config(
        profile_name,
        definition.default_stage_kinds(),
        profile,
    );
    plan.stages = definition.resolved_stage_kinds(profile);
    plan
}

pub(super) fn orchestrator_from_definition(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
    definition: SchedulerPresetDefinition,
) -> SchedulerProfileOrchestrator {
    SchedulerProfileOrchestrator::new(
        plan_from_definition(profile_name, profile, definition),
        tool_runner,
    )
}

pub fn scheduler_preset_definition(kind: SchedulerPresetKind) -> SchedulerPresetDefinition {
    match kind {
        SchedulerPresetKind::Sisyphus => SISYPHUS_PRESET_BUNDLE.definition,
        SchedulerPresetKind::Prometheus => PROMETHEUS_PRESET_BUNDLE.definition,
        SchedulerPresetKind::Atlas => ATLAS_PRESET_BUNDLE.definition,
        SchedulerPresetKind::Hephaestus => HEPHAESTUS_PRESET_BUNDLE.definition,
        SchedulerPresetKind::Verifier => VERIFIER_PRESET_BUNDLE.definition,
    }
}

pub(super) fn default_resolve_stage_kinds(
    profile: &SchedulerProfileConfig,
    default_stages: &'static [SchedulerStageKind],
) -> Vec<SchedulerStageKind> {
    if profile.stages.is_empty() {
        default_stages.to_vec()
    } else {
        profile.stage_kinds()
    }
}

pub(super) fn locked_default_stage_kinds(
    _profile: &SchedulerProfileConfig,
    default_stages: &'static [SchedulerStageKind],
) -> Vec<SchedulerStageKind> {
    default_stages.to_vec()
}

pub(super) const KEEP_EXISTING_OUTPUTS: &[SchedulerExecutionOutputSlot] = &[];
pub(super) const CLEAR_REVIEWED_OUTPUT: &[SchedulerExecutionOutputSlot] =
    &[SchedulerExecutionOutputSlot::Reviewed];

pub(super) const PLACEMENT_REVIEWED_KEEP: SchedulerExecutionPlacement =
    SchedulerExecutionPlacement {
        slot: SchedulerExecutionOutputSlot::Reviewed,
        clear_slots: KEEP_EXISTING_OUTPUTS,
    };

pub(super) const PLACEMENT_DELEGATED_CLEAR_REVIEWED: SchedulerExecutionPlacement =
    SchedulerExecutionPlacement {
        slot: SchedulerExecutionOutputSlot::Delegated,
        clear_slots: CLEAR_REVIEWED_OUTPUT,
    };

pub(super) const STANDARD_FINAL_OUTPUT_PRIORITY: &[SchedulerFinalOutputSource] = &[
    SchedulerFinalOutputSource::Synthesized,
    SchedulerFinalOutputSource::Reviewed,
    SchedulerFinalOutputSource::Delegated,
    SchedulerFinalOutputSource::HandedOff,
    SchedulerFinalOutputSource::Planned,
    SchedulerFinalOutputSource::Routed,
    SchedulerFinalOutputSource::RequestBrief,
];

pub(super) const PLANNER_HANDOFF_FINAL_OUTPUT_PRIORITY: &[SchedulerFinalOutputSource] = &[
    SchedulerFinalOutputSource::HandedOff,
    SchedulerFinalOutputSource::Reviewed,
    SchedulerFinalOutputSource::Planned,
    SchedulerFinalOutputSource::Synthesized,
    SchedulerFinalOutputSource::Delegated,
    SchedulerFinalOutputSource::Routed,
    SchedulerFinalOutputSource::RequestBrief,
];

fn default_single_pass_executor_prompt(_profile_suffix: &str) -> String {
    "You are the scheduler execution orchestrator. Execute the task faithfully and return concrete results only.".to_string()
}

fn default_coordination_gate_prompt(_profile_suffix: &str) -> String {
    "You are the coordination gate. Decide whether the coordinator is done, needs another worker round, or is blocked. Return JSON only, never prose outside JSON.".to_string()
}

fn default_autonomous_verification_prompt(_profile_suffix: &str) -> String {
    "You are the verification layer. Audit the executor result with read-only reasoning and return a concise verification note: completed evidence, missing evidence, and residual risks.".to_string()
}

fn default_autonomous_gate_prompt(_profile_suffix: &str) -> String {
    "You are the finish gate. Judge whether the executor output is complete, needs one more bounded retry, or is blocked. Return JSON only, never prose outside JSON.".to_string()
}

pub(super) const DEFAULT_SINGLE_PASS_EXECUTOR_STAGE: SchedulerInternalStageSpec =
    SchedulerInternalStageSpec {
        event_name: "single-pass-executor",
        agent_name: "scheduler-single-pass-executor",
        tool_policy: StageToolPolicy::AllowAll,
        fallback_prompt: default_single_pass_executor_prompt,
    };

pub(super) const DEFAULT_COORDINATION_VERIFICATION_STAGE: SchedulerInternalStageSpec =
    SchedulerInternalStageSpec {
        event_name: "coordination-verification",
        agent_name: "scheduler-coordination-verification",
        tool_policy: StageToolPolicy::AllowReadOnly,
        fallback_prompt: default_autonomous_verification_prompt,
    };

pub(super) const DEFAULT_COORDINATION_GATE_STAGE: SchedulerInternalStageSpec =
    SchedulerInternalStageSpec {
        event_name: "coordination-gate",
        agent_name: "scheduler-coordination-gate",
        tool_policy: StageToolPolicy::DisableAll,
        fallback_prompt: default_coordination_gate_prompt,
    };

pub(super) const DEFAULT_AUTONOMOUS_VERIFICATION_STAGE: SchedulerInternalStageSpec =
    SchedulerInternalStageSpec {
        event_name: "autonomous-verification",
        agent_name: "scheduler-autonomous-verification",
        tool_policy: StageToolPolicy::AllowReadOnly,
        fallback_prompt: default_autonomous_verification_prompt,
    };

pub(super) const DEFAULT_AUTONOMOUS_GATE_STAGE: SchedulerInternalStageSpec =
    SchedulerInternalStageSpec {
        event_name: "autonomous-gate",
        agent_name: "scheduler-autonomous-gate",
        tool_policy: StageToolPolicy::DisableAll,
        fallback_prompt: default_autonomous_gate_prompt,
    };

pub(super) const DEFAULT_INTERNAL_STAGE_HOOKS: SchedulerPresetInternalStageHooks =
    SchedulerPresetInternalStageHooks {
        single_pass_executor: DEFAULT_SINGLE_PASS_EXECUTOR_STAGE,
        coordination_verification: DEFAULT_COORDINATION_VERIFICATION_STAGE,
        coordination_gate: DEFAULT_COORDINATION_GATE_STAGE,
        coordination_retry_event: "coordination-retry",
        autonomous_verification: DEFAULT_AUTONOMOUS_VERIFICATION_STAGE,
        autonomous_gate: DEFAULT_AUTONOMOUS_GATE_STAGE,
        autonomous_retry_event: "autonomous-retry",
        coordination_verification_fallback: SchedulerVerificationFallback::ReviewStage,
        autonomous_verification_failure: SchedulerVerificationFailurePolicy::Error,
        coordination_semantics_error: "coordination execution semantics unavailable",
        autonomous_semantics_error: "executor execution semantics unavailable",
    };

fn default_stage_tool_policy(stage: SchedulerStageKind) -> StageToolPolicy {
    match stage {
        SchedulerStageKind::RequestAnalysis => StageToolPolicy::DisableAll,
        SchedulerStageKind::Route => StageToolPolicy::AllowReadOnly,
        SchedulerStageKind::Interview => StageToolPolicy::AllowReadOnly,
        SchedulerStageKind::Plan => StageToolPolicy::AllowReadOnly,
        SchedulerStageKind::Delegation => StageToolPolicy::AllowAll,
        SchedulerStageKind::Review => StageToolPolicy::AllowReadOnly,
        SchedulerStageKind::ExecutionOrchestration => StageToolPolicy::AllowAll,
        SchedulerStageKind::Synthesis => StageToolPolicy::DisableAll,
        SchedulerStageKind::Handoff => StageToolPolicy::DisableAll,
    }
}

fn default_stage_session_projection(stage: SchedulerStageKind) -> SchedulerSessionProjection {
    match stage {
        SchedulerStageKind::RequestAnalysis => SchedulerSessionProjection::Hidden,
        _ => SchedulerSessionProjection::Transcript,
    }
}

fn default_stage_loop_budget(stage: SchedulerStageKind) -> SchedulerLoopBudget {
    match stage {
        SchedulerStageKind::RequestAnalysis
        | SchedulerStageKind::Route
        | SchedulerStageKind::Synthesis
        | SchedulerStageKind::Handoff => SchedulerLoopBudget::StepLimit(1),
        SchedulerStageKind::Interview
        | SchedulerStageKind::Plan
        | SchedulerStageKind::Delegation
        | SchedulerStageKind::Review
        | SchedulerStageKind::ExecutionOrchestration => SchedulerLoopBudget::Unbounded,
    }
}

fn default_stage_attached_session(stage: SchedulerStageKind) -> bool {
    matches!(
        stage,
        SchedulerStageKind::ExecutionOrchestration | SchedulerStageKind::Delegation
    )
}

pub(super) fn default_transition_graph(
    transitions: Vec<SchedulerTransitionSpec>,
) -> SchedulerTransitionGraph {
    SchedulerTransitionGraph::new(transitions)
}

pub(super) fn passthrough_route_decision(
    decision: crate::scheduler::RouteDecision,
) -> crate::scheduler::RouteDecision {
    decision
}

pub(super) fn shared_execution_stage_effect_protocol(
    stages: &[SchedulerStageKind],
) -> SchedulerEffectProtocol {
    shared_execution_workflow_effect_protocol(stages, &[SchedulerStageKind::ExecutionOrchestration])
}

pub(super) fn shared_execution_and_synthesis_effect_protocol(
    stages: &[SchedulerStageKind],
) -> SchedulerEffectProtocol {
    shared_execution_workflow_effect_protocol(
        stages,
        &[
            SchedulerStageKind::ExecutionOrchestration,
            SchedulerStageKind::Synthesis,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_PRESETS: [SchedulerPresetKind; 5] = [
        SchedulerPresetKind::Sisyphus,
        SchedulerPresetKind::Prometheus,
        SchedulerPresetKind::Atlas,
        SchedulerPresetKind::Hephaestus,
        SchedulerPresetKind::Verifier,
    ];

    #[test]
    fn all_preset_previews_follow_structural_contract() {
        // Every preset preview must have:
        // 1. "You are <Name>" — third-person role introduction
        // 2. "Bias:" — operational bias declaration
        // 3. "Boundary:" — constraint declaration
        for kind in ALL_PRESETS {
            let def = scheduler_preset_definition(kind);
            let preview = def.system_prompt_preview();
            let name = format!("{kind:?}");
            assert!(
                preview.starts_with("You are"),
                "{name}: preview must start with 'You are', got: {preview}"
            );
            assert!(
                preview.contains("Bias:"),
                "{name}: preview must contain 'Bias:'"
            );
            assert!(
                preview.contains("Boundary:"),
                "{name}: preview must contain 'Boundary:'"
            );
        }
    }

    #[test]
    fn preset_previews_use_no_first_person() {
        for kind in ALL_PRESETS {
            let def = scheduler_preset_definition(kind);
            let preview = def.system_prompt_preview();
            let name = format!("{kind:?}");
            assert!(
                !preview.contains(&format!("I'm {name}")),
                "{name}: preview must not use first-person 'I'm'"
            );
        }
    }

    #[test]
    fn prometheus_preview_declares_planner_only_boundary() {
        let def = scheduler_preset_definition(SchedulerPresetKind::Prometheus);
        let preview = def.system_prompt_preview();
        assert!(
            preview.contains("planner-only"),
            "Prometheus boundary must declare planner-only constraint"
        );
    }

    #[test]
    fn atlas_preview_declares_conductor_boundary() {
        let def = scheduler_preset_definition(SchedulerPresetKind::Atlas);
        let preview = def.system_prompt_preview();
        assert!(
            preview.contains("never write code yourself"),
            "Atlas boundary must prohibit direct code authoring"
        );
    }

    #[test]
    fn normalize_embedded_delivery_summary_drops_preface_and_repairs_heading() {
        let raw = "Preface line.\n\n---\n\n## `## Delivery Summary`\n\nActual body\n\n**Verification**\n- Checked.";
        let normalized = normalize_embedded_delivery_summary(raw);
        assert!(normalized.starts_with("## Delivery Summary"));
        assert!(!normalized.contains("Preface line."));
        assert_eq!(normalized.matches("## Delivery Summary").count(), 1);
    }

    #[test]
    fn normalize_embedded_delivery_summary_recognizes_chinese_delivery_heading() {
        let raw = "现在我对结果有了全面了解。\n\n## 交付摘要\n\n以下是整理后的最终答案。";
        let normalized = normalize_embedded_delivery_summary(raw);
        assert!(normalized.starts_with("## Delivery Summary"));
        assert!(!normalized.contains("现在我对结果有了全面了解。"));
        assert!(normalized.contains("以下是整理后的最终答案。"));
    }

    #[test]
    fn normalize_embedded_delivery_summary_keeps_non_delivery_text() {
        let raw = "Plain final answer without structured heading.";
        assert_eq!(normalize_embedded_delivery_summary(raw), raw);
    }
}
