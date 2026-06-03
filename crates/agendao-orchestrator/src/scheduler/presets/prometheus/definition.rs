use super::super::super::{SchedulerPresetKind, SchedulerPresetMetadata, SchedulerStageKind};
use super::super::SchedulerPresetDefinition;

const PROMETHEUS_DEFAULT_STAGES: &[SchedulerStageKind] = &[
    SchedulerStageKind::RequestAnalysis,
    SchedulerStageKind::Interview,
    SchedulerStageKind::Plan,
    SchedulerStageKind::Review,
    SchedulerStageKind::Handoff,
];

pub const PROMETHEUS_PRESET: SchedulerPresetDefinition = SchedulerPresetDefinition {
    kind: SchedulerPresetKind::Prometheus,
    metadata: SchedulerPresetMetadata {
        public: true,
        router_recommended: true,
        deprecated: false,
    },
    default_stages: PROMETHEUS_DEFAULT_STAGES,
};

/// OMO Prometheus-aligned orchestration: planner-only, interview-first, review-gated handoff.
///
/// Prometheus is modeled as an explicit planning workflow:
/// - Interview: clarify requirements and collect discoverable repo facts
/// - Plan: produce the planning artifact
/// - Review: audit planning completeness and handoff readiness
/// - Handoff: return the reviewed plan as the final output
pub fn prometheus_default_stages() -> Vec<SchedulerStageKind> {
    PROMETHEUS_PRESET.default_stage_kinds()
}
