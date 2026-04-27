use serde_json::json;

use super::hephaestus::{
    HEPHAESTUS_CAPABILITY_HOOKS, HEPHAESTUS_EFFECT_HOOKS, HEPHAESTUS_FINALIZATION_HOOKS,
    HEPHAESTUS_GATE_HOOKS, HEPHAESTUS_PROMPT_HOOKS, HEPHAESTUS_ROUTE_HOOKS,
    HEPHAESTUS_STAGE_GRAPH_HOOKS,
};
use super::{
    orchestrator_from_definition, plan_from_definition, SchedulerPresetBundle,
    SchedulerPresetPlatformSpec, SchedulerPresetProjectionHooks,
};
use crate::scheduler::{
    SchedulerPresetDefinition, SchedulerPresetKind, SchedulerPresetMetadata,
    SchedulerProfileConfig, SchedulerProfileOrchestrator, SchedulerProfilePlan, SchedulerStageKind,
};
use crate::tool_runner::ToolRunner;

const VERIFIER_DEFAULT_STAGES: &[SchedulerStageKind] = &[
    SchedulerStageKind::RequestAnalysis,
    SchedulerStageKind::ExecutionOrchestration,
];

pub fn verifier_defaults_payload() -> serde_json::Value {
    json!({
        "selection": "round-robin",
        "repetitions": 3,
        "use_logprobs": true,
        "granularity": 20,
        "trace_format": "compact",
        "criterion_defaults": {
            "weight": 1.0,
            "aggregation": "score-margin"
        },
        "recommended_criteria": [
            {
                "id": "spec",
                "name": "Spec adherence",
                "description": "Prefer the candidate that most directly satisfies the request.",
                "weight": 1.0,
                "aggregation": "score-margin"
            },
            {
                "id": "safety",
                "name": "Safety and regression control",
                "description": "Prefer the candidate with lower regression and safety risk when scores are otherwise close.",
                "weight": 1.5,
                "aggregation": "winner-vote"
            }
        ]
    })
}

pub fn verifier_workflow_todos_payload() -> serde_json::Value {
    json!({
        "todos": [
            { "id": "verifier-1", "content": "Generate a candidate with strong empirical verification evidence", "status": "pending", "priority": "high" },
            { "id": "verifier-2", "content": "Preserve trace quality so the candidate can survive later verifier comparison", "status": "pending", "priority": "high" },
            { "id": "verifier-3", "content": "Return the selected candidate result rather than assuming the last iteration wins", "status": "pending", "priority": "high" }
        ],
        "verifier_defaults": verifier_defaults_payload()
    })
}

fn verifier_system_prompt_preview() -> &'static str {
    "You are Verifier, a workflow-backed candidate selection preset over the autonomous deep-worker topology. Use it when explicit multi-candidate comparison is worth the extra judge cost. Preserve trajectory evidence for canonical score-job verification: pair, criterion, repetition, A-T score-token expected reward, and selected-candidate finalization."
}

pub const VERIFIER_PRESET: SchedulerPresetDefinition = SchedulerPresetDefinition {
    kind: SchedulerPresetKind::Verifier,
    metadata: SchedulerPresetMetadata {
        public: true,
        router_recommended: true,
        deprecated: false,
    },
    default_stages: VERIFIER_DEFAULT_STAGES,
};

pub const VERIFIER_PROJECTION_HOOKS: SchedulerPresetProjectionHooks =
    SchedulerPresetProjectionHooks {
        workflow_todos_payload: verifier_workflow_todos_payload,
        system_prompt_preview: verifier_system_prompt_preview,
        sync_runtime_authority: None,
    };

pub const VERIFIER_PLATFORM: SchedulerPresetPlatformSpec = SchedulerPresetPlatformSpec {
    stage_graph: HEPHAESTUS_STAGE_GRAPH_HOOKS,
    route: HEPHAESTUS_ROUTE_HOOKS,
    gate: HEPHAESTUS_GATE_HOOKS,
    effect: HEPHAESTUS_EFFECT_HOOKS,
    internal: super::DEFAULT_INTERNAL_STAGE_HOOKS,
    finalization: HEPHAESTUS_FINALIZATION_HOOKS,
    projection: VERIFIER_PROJECTION_HOOKS,
    prompts: HEPHAESTUS_PROMPT_HOOKS,
    capabilities: HEPHAESTUS_CAPABILITY_HOOKS,
};

pub const VERIFIER_PRESET_BUNDLE: SchedulerPresetBundle = SchedulerPresetBundle {
    definition: VERIFIER_PRESET,
    platform: VERIFIER_PLATFORM,
};

pub type VerifierPlan = SchedulerProfilePlan;
pub type VerifierOrchestrator = SchedulerProfileOrchestrator;

pub fn verifier_default_stages() -> Vec<SchedulerStageKind> {
    VERIFIER_PRESET.default_stage_kinds()
}

pub fn verifier_plan() -> VerifierPlan {
    SchedulerProfilePlan::new(verifier_default_stages()).with_orchestrator("verifier")
}

pub fn verifier_plan_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
) -> VerifierPlan {
    plan_from_definition(profile_name, profile, VERIFIER_PRESET)
}

pub fn verifier_orchestrator_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
) -> VerifierOrchestrator {
    orchestrator_from_definition(profile_name, profile, tool_runner, VERIFIER_PRESET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_workflow_todos_payload_exposes_weighted_defaults() {
        let payload = verifier_workflow_todos_payload();
        assert_eq!(
            payload["verifier_defaults"]["selection"].as_str(),
            Some("round-robin")
        );
        assert_eq!(
            payload["verifier_defaults"]["criterion_defaults"]["aggregation"].as_str(),
            Some("score-margin")
        );
        assert_eq!(
            payload["verifier_defaults"]["use_logprobs"].as_bool(),
            Some(true)
        );
        assert_eq!(
            payload["verifier_defaults"]["granularity"].as_u64(),
            Some(20)
        );
        assert_eq!(
            payload["verifier_defaults"]["recommended_criteria"][1]["aggregation"].as_str(),
            Some("winner-vote")
        );
    }

    #[test]
    fn verifier_system_prompt_preview_mentions_score_job_defaults() {
        let preview = verifier_system_prompt_preview();
        assert!(preview.contains("workflow-backed candidate selection"));
        assert!(preview.contains("score-job verification"));
        assert!(preview.contains("A-T score-token expected reward"));
    }
}
