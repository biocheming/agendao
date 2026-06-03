use super::{
    RouteDecision, SchedulerAdvisoryReviewInput, SchedulerArtifactKind,
    SchedulerAutonomousGateStageInput, SchedulerAutonomousVerificationStageInput,
    SchedulerCoordinationGateStageInput, SchedulerCoordinationVerificationStageInput,
    SchedulerDraftArtifactInput, SchedulerExecutionGateDecision, SchedulerExecutionGateStatus,
    SchedulerExecutionOrchestrationStageInput, SchedulerExecutionPlacement,
    SchedulerFinalOutputSource, SchedulerHandoffDecoration, SchedulerHandoffStageInput,
    SchedulerInternalStageSpec, SchedulerInterviewStageInput, SchedulerPlanStageInput,
    SchedulerPlanningArtifactInput, SchedulerPresetKind, SchedulerPresetRuntimeFields,
    SchedulerPresetRuntimeUpdate, SchedulerRetryStageInput, SchedulerReviewStageInput,
    SchedulerSynthesisStageInput, SchedulerTransitionSpec, SchedulerTransitionTarget,
    SchedulerVerificationFailurePolicy, SchedulerVerificationFallback,
};
use crate::scheduler::profile::SchedulerProfilePlan;
use crate::scheduler::profile_state::SchedulerPresetRuntimeState;

impl SchedulerProfilePlan {
    pub(super) fn route_constraint_note(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.route_constraint_note())
    }

    pub(super) fn interview_stage_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.interview_stage_prompt(profile_suffix))
    }

    pub(super) fn plan_stage_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.plan_stage_prompt(profile_suffix))
    }

    pub(super) fn delegation_stage_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.delegation_stage_prompt(profile_suffix))
    }

    pub(super) fn delegation_charter(&self) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.delegation_charter(self))
    }

    pub(super) fn review_stage_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.review_stage_prompt(profile_suffix))
    }

    pub(super) fn handoff_stage_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.handoff_stage_prompt(profile_suffix))
    }

    pub(super) fn synthesis_stage_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.synthesis_stage_prompt(profile_suffix))
    }

    pub(super) fn execution_fallback_prompt(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.execution_fallback_prompt(profile_suffix))
    }

    pub(super) fn coordination_verification_placement(&self) -> SchedulerExecutionPlacement {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_verification_placement()
    }

    pub(super) fn coordination_terminal_placement(&self) -> SchedulerExecutionPlacement {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_terminal_placement()
    }

    pub(super) fn autonomous_verification_placement(&self) -> SchedulerExecutionPlacement {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_verification_placement()
    }

    pub(super) fn autonomous_terminal_placement(&self) -> SchedulerExecutionPlacement {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_terminal_placement()
    }

    pub(super) fn retry_exhausted_placement(&self) -> SchedulerExecutionPlacement {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .retry_exhausted_placement()
    }

    pub(super) fn final_output_priority(&self) -> &'static [SchedulerFinalOutputSource] {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .final_output_priority()
    }

    pub(super) fn single_pass_executor_stage(&self) -> SchedulerInternalStageSpec {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .single_pass_executor_stage()
    }

    pub(super) fn coordination_verification_stage(&self) -> SchedulerInternalStageSpec {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_verification_stage()
    }

    pub(super) fn coordination_gate_stage(&self) -> SchedulerInternalStageSpec {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_gate_stage()
    }

    pub(super) fn coordination_retry_event(&self) -> &'static str {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_retry_event()
    }

    pub(super) fn autonomous_verification_stage(&self) -> SchedulerInternalStageSpec {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_verification_stage()
    }

    pub(super) fn autonomous_gate_stage(&self) -> SchedulerInternalStageSpec {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_gate_stage()
    }

    pub(super) fn autonomous_retry_event(&self) -> &'static str {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_retry_event()
    }

    pub(super) fn coordination_verification_fallback(&self) -> SchedulerVerificationFallback {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_verification_fallback()
    }

    pub(super) fn autonomous_verification_failure_policy(
        &self,
    ) -> SchedulerVerificationFailurePolicy {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_verification_failure_policy()
    }

    pub(super) fn coordination_semantics_error(&self) -> &'static str {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .coordination_semantics_error()
    }

    pub(super) fn autonomous_semantics_error(&self) -> &'static str {
        self.preset_definition()
            .unwrap_or(crate::scheduler::SchedulerPresetKind::Sisyphus.definition())
            .autonomous_semantics_error()
    }

    pub(super) fn execution_orchestration_charter(&self, profile_suffix: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.execution_orchestration_charter(self, profile_suffix))
    }

    pub(super) fn coordination_verification_charter(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.coordination_verification_charter())
    }

    pub(super) fn coordination_gate_contract(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.coordination_gate_contract())
    }

    pub(super) fn coordination_gate_prompt(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.coordination_gate_prompt())
    }

    pub(super) fn autonomous_verification_charter(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.autonomous_verification_charter())
    }

    pub(super) fn autonomous_gate_contract(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.autonomous_gate_contract())
    }

    pub(super) fn autonomous_gate_prompt(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.autonomous_gate_prompt())
    }

    pub(super) fn compose_interview_stage_input(
        &self,
        input: SchedulerInterviewStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_interview_input(input))
    }

    pub(super) fn compose_plan_stage_input(
        &self,
        input: SchedulerPlanStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_plan_input(input))
    }

    pub(super) fn compose_execution_orchestration_stage_input(
        &self,
        input: SchedulerExecutionOrchestrationStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_execution_orchestration_input(input))
    }

    pub(super) fn compose_synthesis_stage_input(
        &self,
        input: SchedulerSynthesisStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_synthesis_input(input))
    }

    pub(super) fn compose_coordination_verification_stage_input(
        &self,
        input: SchedulerCoordinationVerificationStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_coordination_verification_input(input))
    }

    pub(super) fn compose_coordination_gate_stage_input(
        &self,
        input: SchedulerCoordinationGateStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_coordination_gate_input(input))
    }

    pub(super) fn compose_autonomous_verification_stage_input(
        &self,
        input: SchedulerAutonomousVerificationStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_autonomous_verification_input(input))
    }

    pub(super) fn compose_autonomous_gate_stage_input(
        &self,
        input: SchedulerAutonomousGateStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_autonomous_gate_input(input))
    }

    pub(super) fn compose_review_stage_input(
        &self,
        input: SchedulerReviewStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_review_input(input))
    }

    pub(super) fn compose_retry_stage_input(
        &self,
        input: SchedulerRetryStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_retry_input(input))
    }

    pub(super) fn compose_handoff_stage_input(
        &self,
        input: SchedulerHandoffStageInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_handoff_input(input))
    }

    pub(super) fn compose_advisory_review_input(
        &self,
        input: SchedulerAdvisoryReviewInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_advisory_review_input(input))
    }

    pub(super) fn compose_draft_artifact(
        &self,
        input: SchedulerDraftArtifactInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_draft_artifact(input))
    }

    pub(super) fn compose_planning_artifact(
        &self,
        input: SchedulerPlanningArtifactInput<'_>,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.compose_planning_artifact(input))
    }

    pub(super) fn decorate_final_output(
        &self,
        content: String,
        decoration: SchedulerHandoffDecoration,
    ) -> String {
        self.preset_definition()
            .map(|definition| definition.decorate_final_output(content.clone(), decoration))
            .unwrap_or(content)
    }

    pub(super) fn extend_final_output_metadata(
        &self,
        state: &crate::scheduler::profile_state::SchedulerProfileState,
        artifact_path: Option<&str>,
        metadata: &mut std::collections::HashMap<String, serde_json::Value>,
    ) {
        if let Some(definition) = self.preset_definition() {
            definition.extend_final_output_metadata(state, artifact_path, metadata);
        }
    }

    pub(super) fn validate_runtime_orchestration_tool(
        &self,
        tool_name: &str,
    ) -> Result<(), String> {
        self.preset_definition()
            .map(|definition| definition.validate_runtime_orchestration_tool(tool_name))
            .unwrap_or(Ok(()))
    }

    pub(super) fn validate_runtime_artifact_path(
        &self,
        raw_path: &str,
        exec_ctx: &crate::ExecutionContext,
    ) -> Result<(), String> {
        self.preset_definition()
            .map(|definition| definition.validate_runtime_artifact_path(raw_path, exec_ctx))
            .unwrap_or(Ok(()))
    }

    pub(super) fn workflow_todos_payload(&self) -> Option<serde_json::Value> {
        self.preset_definition()
            .map(|definition| definition.workflow_todos_payload())
    }

    pub(super) fn advisory_agent_name(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.advisory_agent_name())
    }

    pub(super) fn runtime_update_for_advisory_review(
        &self,
        content: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.preset_definition()
            .and_then(|definition| definition.runtime_update_for_advisory_review(content))
    }

    pub(super) fn user_choice_payload(&self) -> Option<serde_json::Value> {
        self.preset_definition()
            .and_then(|definition| definition.user_choice_payload())
    }

    pub(super) fn default_user_choice(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.default_user_choice())
    }

    pub(super) fn parse_user_choice(&self, output: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.parse_user_choice(output))
    }

    pub(super) fn runtime_update_for_user_choice(
        &self,
        choice: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.preset_definition()
            .and_then(|definition| definition.runtime_update_for_user_choice(choice))
    }

    pub(super) fn approval_review_agent_name(&self) -> Option<&'static str> {
        self.preset_definition()
            .and_then(|definition| definition.approval_review_agent_name())
    }

    pub(super) fn max_approval_review_rounds(&self) -> Option<usize> {
        self.preset_definition()
            .and_then(|definition| definition.max_approval_review_rounds())
    }

    pub(super) fn approval_review_is_accepted(&self, output: &str) -> bool {
        self.preset_definition()
            .map(|definition| definition.approval_review_is_accepted(output))
            .unwrap_or(false)
    }

    pub(super) fn runtime_update_for_approval_review(
        &self,
        content: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.preset_definition()
            .and_then(|definition| definition.runtime_update_for_approval_review(content))
    }

    pub(super) fn runtime_update_for_planned_output(
        &self,
        content: String,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.preset_definition()
            .and_then(|definition| definition.runtime_update_for_planned_output(content))
    }

    pub(super) fn runtime_update_for_review_gate(
        &self,
        approved: bool,
    ) -> Option<SchedulerPresetRuntimeUpdate> {
        self.preset_definition()
            .and_then(|definition| definition.runtime_update_for_review_gate(approved))
    }

    pub(super) fn resolve_runtime_transition_target(
        &self,
        transitions: &[&SchedulerTransitionSpec],
        user_choice: Option<&str>,
        review_gate_approved: Option<bool>,
    ) -> Option<SchedulerTransitionTarget> {
        self.preset_definition().and_then(|definition| {
            definition.resolve_runtime_transition_target(
                transitions,
                user_choice,
                review_gate_approved,
            )
        })
    }

    pub(super) fn normalize_final_output(&self, output: &str) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.normalize_final_output(output))
    }

    pub(super) fn sync_runtime_authority(
        &self,
        runtime: &mut SchedulerPresetRuntimeState,
        ctx: &crate::OrchestratorContext,
    ) {
        if let Some(definition) = self.preset_definition() {
            definition.sync_runtime_authority(runtime, ctx);
        }
    }

    pub(super) fn normalize_review_stage_output(
        &self,
        runtime: SchedulerPresetRuntimeFields<'_>,
        output: &str,
    ) -> Option<String> {
        self.preset_definition()
            .and_then(|definition| definition.normalize_review_stage_output(runtime, output))
    }

    pub(super) fn resolve_gate_terminal_content(
        &self,
        status: SchedulerExecutionGateStatus,
        decision: &SchedulerExecutionGateDecision,
        fallback_content: &str,
    ) -> Option<String> {
        self.preset_definition()
            .unwrap_or(SchedulerPresetKind::Sisyphus.definition())
            .resolve_gate_terminal_content(status, decision, fallback_content)
    }

    pub(super) fn effect_dispatch_is_authoritative(&self) -> bool {
        self.preset_definition()
            .map(|definition| definition.effect_dispatch_is_authoritative())
            .unwrap_or(false)
    }

    pub(super) fn constrain_route_decision(&self, decision: RouteDecision) -> RouteDecision {
        self.preset_definition()
            .map(|definition| definition.constrain_route_decision(decision.clone()))
            .unwrap_or(decision)
    }
}

impl SchedulerProfilePlan {
    pub(super) fn artifact_relative_path(
        &self,
        kind: SchedulerArtifactKind,
        session_id: &str,
    ) -> Option<String> {
        self.preset_definition().and_then(|definition| match kind {
            SchedulerArtifactKind::Planning => {
                definition.planning_artifact_relative_path(session_id)
            }
            SchedulerArtifactKind::Draft => definition.draft_artifact_relative_path(session_id),
        })
    }
}
