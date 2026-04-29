use super::*;
use crate::scheduler::append_artifact_note;

impl SchedulerProfileOrchestrator {
    pub(super) fn finalize_output(&self, state: SchedulerProfileState) -> OrchestratorOutput {
        let artifact_path = state.preset_runtime.planning_artifact_path.clone();
        let content = self
            .plan
            .final_output_priority()
            .iter()
            .find_map(|source| state.final_output_content(*source))
            .unwrap_or_else(|| state.route.request_brief.clone());
        let content = self
            .plan
            .normalize_final_output(&content)
            .unwrap_or(content);
        let content = append_artifact_note(content, artifact_path.as_deref());

        let mut metadata = HashMap::new();

        // Final-output metadata only preserves the structured fields that
        // `merge_output_metadata` knows how to reconcile. For workflow mode
        // artifacts, later stages have higher precedence for the same
        // artifact-name/entry-key pair:
        // delegated < reviewed < handed_off < synthesized.
        for output in [
            state.execution.delegated.as_ref(),
            state.execution.reviewed.as_ref(),
            state.execution.handed_off.as_ref(),
            state.execution.synthesized.as_ref(),
        ] {
            if let Some(output) = output {
                merge_output_metadata(&mut metadata, &output.metadata);
            }
        }

        // Preset-specific finalization metadata is applied after stage merges,
        // so it can publish final handoff commands or other terminal metadata.
        self.plan
            .extend_final_output_metadata(&state, artifact_path.as_deref(), &mut metadata);

        OrchestratorOutput {
            content,
            steps: state.metrics.total_steps,
            tool_calls_count: state.metrics.total_tool_calls,
            metadata,
            finish_reason: if state.is_cancelled {
                crate::runtime::events::FinishReason::Cancelled
            } else {
                crate::runtime::events::FinishReason::EndTurn
            },
        }
    }
}
