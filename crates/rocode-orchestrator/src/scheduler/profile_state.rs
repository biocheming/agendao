use super::{
    RouteDecision, SchedulerExecutionOutputSlot, SchedulerExecutionPlacement,
    SchedulerFinalOutputSource, SchedulerPresetRuntimeFields, SchedulerPresetRuntimeUpdate,
};
use crate::agent_tree::AgentTreeNode;
use crate::output_metadata::OutputUsage;
use crate::OrchestratorOutput;
use rocode_types::SessionContinuityPacket;
use serde_json::Value;
use std::collections::HashMap;

pub(super) const SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY: &str =
    "scheduler_session_context_packet";
pub(super) type SchedulerSessionContextPacketMetadata = SessionContinuityPacket;

fn scheduler_session_context_packet_from_metadata(
    metadata: &HashMap<String, Value>,
) -> Option<SchedulerSessionContextPacketMetadata> {
    metadata
        .get(SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY)
        .and_then(SchedulerSessionContextPacketMetadata::from_value)
}

#[derive(Default)]
pub(super) struct SchedulerRouteState {
    pub(super) request_brief: String,
    pub(super) route_decision: Option<RouteDecision>,
    pub(super) direct_response: Option<String>,
    pub(super) interviewed: Option<String>,
    pub(super) routed: Option<String>,
}

#[derive(Default)]
pub(super) struct SchedulerExecutionState {
    pub(super) delegated: Option<OrchestratorOutput>,
    pub(super) reviewed: Option<OrchestratorOutput>,
    pub(super) handed_off: Option<OrchestratorOutput>,
    pub(super) synthesized: Option<OrchestratorOutput>,
}

#[derive(Default)]
pub(super) struct SchedulerPresetRuntimeState {
    pub(super) planned: Option<String>,
    pub(super) planning_artifact_path: Option<String>,
    pub(super) draft_artifact_path: Option<String>,
    pub(super) draft_snapshot: Option<String>,
    pub(super) ground_truth_context: Option<String>,
    pub(super) advisory_review: Option<String>,
    pub(super) approval_review: Option<String>,
    pub(super) user_choice: Option<String>,
    pub(super) review_gate_approved: Option<bool>,
    pub(super) workflow_todos_registered: bool,
    /// Runtime-constructed agent tree parsed from LLM output.
    /// When present, the next execution round uses this tree via
    /// `AgentTreeOrchestrator` for true parallel child dispatch.
    pub(super) dynamic_agent_tree: Option<AgentTreeNode>,
}

#[derive(Default)]
pub(super) struct SchedulerMetricsState {
    pub(super) total_steps: u32,
    pub(super) total_tool_calls: u32,
    pub(super) usage: OutputUsage,
}

#[derive(Default)]
pub(super) struct SchedulerProfileState {
    pub(super) route: SchedulerRouteState,
    pub(super) execution: SchedulerExecutionState,
    pub(super) preset_runtime: SchedulerPresetRuntimeState,
    pub(super) metrics: SchedulerMetricsState,
    pub(super) session_context: Option<String>,
    pub(super) session_context_packet: Option<SchedulerSessionContextPacketMetadata>,
    pub(super) is_cancelled: bool,
}

impl SchedulerProfileState {
    pub(super) fn apply_session_context_metadata(&mut self, metadata: &HashMap<String, Value>) {
        self.session_context_packet = scheduler_session_context_packet_from_metadata(metadata);
        self.session_context = self
            .session_context_packet
            .as_ref()
            .map(SchedulerSessionContextPacketMetadata::render);
    }

    pub(super) fn place_execution_output(
        &mut self,
        placement: SchedulerExecutionPlacement,
        output: OrchestratorOutput,
    ) {
        for slot in placement.clear_slots {
            self.clear_execution_output(*slot);
        }
        self.set_execution_output(placement.slot, output);
    }

    pub(super) fn final_output_content(
        &self,
        source: SchedulerFinalOutputSource,
    ) -> Option<String> {
        match source {
            SchedulerFinalOutputSource::HandedOff => self
                .execution
                .handed_off
                .as_ref()
                .map(|output| output.content.clone()),
            SchedulerFinalOutputSource::Reviewed => self
                .execution
                .reviewed
                .as_ref()
                .map(|output| output.content.clone()),
            SchedulerFinalOutputSource::Planned => self.preset_runtime.planned.clone(),
            SchedulerFinalOutputSource::Synthesized => self
                .execution
                .synthesized
                .as_ref()
                .map(|output| output.content.clone()),
            SchedulerFinalOutputSource::Delegated => self
                .execution
                .delegated
                .as_ref()
                .map(|output| output.content.clone()),
            SchedulerFinalOutputSource::Routed => self.route.routed.clone(),
            SchedulerFinalOutputSource::RequestBrief => Some(self.route.request_brief.clone()),
        }
    }

    pub(super) fn preset_runtime_fields(&self) -> SchedulerPresetRuntimeFields<'_> {
        SchedulerPresetRuntimeFields {
            route_rationale_summary: self
                .route
                .route_decision
                .as_ref()
                .map(|decision| decision.rationale_summary.as_str()),
            planning_artifact_path: self.preset_runtime.planning_artifact_path.as_deref(),
            draft_artifact_path: self.preset_runtime.draft_artifact_path.as_deref(),
            interviewed: self.route.interviewed.as_deref(),
            planned: self.preset_runtime.planned.as_deref(),
            draft_snapshot: self.preset_runtime.draft_snapshot.as_deref(),
            advisory_review: self.preset_runtime.advisory_review.as_deref(),
            approval_review: self.preset_runtime.approval_review.as_deref(),
            user_choice: self.preset_runtime.user_choice.as_deref(),
            review_gate_approved: self.preset_runtime.review_gate_approved,
        }
    }

    pub(super) fn apply_runtime_update(&mut self, update: SchedulerPresetRuntimeUpdate) {
        match update {
            SchedulerPresetRuntimeUpdate::Planned(content) => {
                self.preset_runtime.planned = Some(content)
            }
            SchedulerPresetRuntimeUpdate::AdvisoryReview(content) => {
                self.preset_runtime.advisory_review = Some(content)
            }
            SchedulerPresetRuntimeUpdate::ApprovalReview(content) => {
                self.preset_runtime.approval_review = Some(content)
            }
            SchedulerPresetRuntimeUpdate::UserChoice(choice) => {
                self.preset_runtime.user_choice = Some(choice)
            }
            SchedulerPresetRuntimeUpdate::ReviewGateApproved(approved) => {
                self.preset_runtime.review_gate_approved = Some(approved)
            }
        }
    }

    fn set_execution_output(
        &mut self,
        slot: SchedulerExecutionOutputSlot,
        output: OrchestratorOutput,
    ) {
        match slot {
            SchedulerExecutionOutputSlot::Delegated => self.execution.delegated = Some(output),
            SchedulerExecutionOutputSlot::Reviewed => self.execution.reviewed = Some(output),
            SchedulerExecutionOutputSlot::HandedOff => self.execution.handed_off = Some(output),
            SchedulerExecutionOutputSlot::Synthesized => self.execution.synthesized = Some(output),
        }
    }

    fn clear_execution_output(&mut self, slot: SchedulerExecutionOutputSlot) {
        match slot {
            SchedulerExecutionOutputSlot::Delegated => self.execution.delegated = None,
            SchedulerExecutionOutputSlot::Reviewed => self.execution.reviewed = None,
            SchedulerExecutionOutputSlot::HandedOff => self.execution.handed_off = None,
            SchedulerExecutionOutputSlot::Synthesized => self.execution.synthesized = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_context_packet_metadata_parses_anchor_contract() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 8,
                "exact_recent_tail_count": 6,
                "omitted_older_turns": 2,
                "exact_recent_tail": [
                    {"message_id": "msg_user", "role": "user"},
                    {"message_id": "msg_assistant", "role": "assistant"}
                ],
                "memory_anchors": [
                    {
                        "record_id": "mem_123",
                        "title": "Prior Martini3 decision",
                        "kind": "Lesson",
                        "status": "Validated"
                    }
                ],
                "latest_compaction_summary": {"message_id": "msg_compaction"},
                "limits": {
                    "recent_tail_messages": 6,
                    "context_text_chars": 4000,
                    "turn_text_chars": 1200
                },
                "recall_policy": "exact_tail_for_recent_followups"
            }),
        )]);

        let packet = scheduler_session_context_packet_from_metadata(&metadata)
            .expect("version 1 packet should parse");

        assert_eq!(packet.omitted_older_turns, 2);
        assert_eq!(packet.exact_recent_tail[0].message_id, "msg_user");
        assert_eq!(packet.exact_recent_tail[1].message_id, "msg_assistant");
        assert_eq!(
            packet
                .latest_compaction_summary
                .as_ref()
                .map(|anchor| anchor.message_id.as_str()),
            Some("msg_compaction")
        );
        assert_eq!(packet.exact_recent_tail_count, 6);
        assert_eq!(packet.memory_anchors[0].record_id, "mem_123");
        assert_eq!(packet.memory_anchors[0].status, "Validated");
    }

    #[test]
    fn session_context_packet_metadata_rejects_unknown_version() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 99,
                "exact_recent_tail": []
            }),
        )]);

        assert!(scheduler_session_context_packet_from_metadata(&metadata).is_none());
    }

    #[test]
    fn scheduler_profile_state_loads_session_context_contract_from_metadata() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 1,
                "exact_recent_tail_count": 1,
                "exact_recent_tail": [{"message_id": "msg_1", "role": "user", "text": "prior context"}]
            }),
        )]);
        let mut state = SchedulerProfileState::default();

        state.apply_session_context_metadata(&metadata);

        let rendered = state
            .session_context
            .as_deref()
            .expect("rendered continuity should derive from packet");
        assert!(rendered.contains("## Session Continuity Context"));
        assert!(rendered.contains("prior context"));
        assert!(rendered.contains("msg_1"));
        assert_eq!(
            state
                .session_context_packet
                .as_ref()
                .and_then(|packet| packet.exact_recent_tail.first())
                .map(|anchor| anchor.message_id.as_str()),
            Some("msg_1")
        );
    }
}
