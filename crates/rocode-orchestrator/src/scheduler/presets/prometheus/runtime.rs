use crate::scheduler::SchedulerAdvisoryReviewInput;
use serde_json::{json, Value};

use super::super::super::{
    SchedulerEffectContext, SchedulerEffectDispatch, SchedulerEffectKind, SchedulerEffectMoment,
    SchedulerEffectProtocol, SchedulerEffectSpec, SchedulerHandoffDecoration,
    SchedulerPresetRuntimeFields, SchedulerStageKind, SchedulerTransitionGraph,
    SchedulerTransitionSpec, SchedulerTransitionTarget, SchedulerTransitionTrigger,
};
use super::super::normalize_embedded_delivery_summary;
use super::{
    append_handoff_guidance, normalize_prometheus_review_output,
    prometheus_handoff_output_has_required_shape, PrometheusReviewContext,
};

pub const PROMETHEUS_MAX_MOMUS_ROUNDS: usize = usize::MAX;
pub const PROMETHEUS_DEFAULT_HANDOFF_CHOICE: &str = "Start Work";
pub const PROMETHEUS_HIGH_ACCURACY_CHOICE: &str = "High Accuracy Review";

pub fn prometheus_advisory_agent_name() -> &'static str {
    "metis"
}

pub fn prometheus_approval_review_agent_name() -> &'static str {
    "momus"
}

pub fn prometheus_default_user_choice() -> &'static str {
    PROMETHEUS_DEFAULT_HANDOFF_CHOICE
}

pub fn prometheus_max_approval_review_rounds() -> usize {
    PROMETHEUS_MAX_MOMUS_ROUNDS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrometheusReviewStateSnapshot<'a> {
    pub route_rationale_summary: Option<&'a str>,
    pub planning_artifact_path: Option<&'a str>,
    pub interviewed: Option<&'a str>,
    pub planned: Option<&'a str>,
    pub draft_snapshot: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrometheusTransitionContext<'a> {
    pub user_choice: Option<&'a str>,
    pub review_gate_approved: Option<bool>,
}

pub fn prometheus_review_state_snapshot(
    runtime: SchedulerPresetRuntimeFields<'_>,
) -> PrometheusReviewStateSnapshot<'_> {
    PrometheusReviewStateSnapshot {
        route_rationale_summary: runtime.route_rationale_summary,
        planning_artifact_path: runtime.planning_artifact_path,
        interviewed: runtime.interviewed,
        planned: runtime.planned,
        draft_snapshot: runtime.draft_snapshot,
        advisory_review: runtime.advisory_review,
    }
}

pub fn compose_prometheus_advisory_review_input(input: SchedulerAdvisoryReviewInput<'_>) -> String {
    let mut sections = Vec::new();
    sections.push("Review this planning session before I generate the work plan:".to_string());
    sections.push(format!("**User's Goal**: {}", input.goal));
    sections.push(format!(
        "**Original Request**:
{}",
        input.original_request
    ));

    let discussed = input
        .discussed
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("No interview summary captured yet.");
    sections.push(format!(
        "**What We Discussed**:
{discussed}"
    ));

    let understanding = input
        .draft_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Use the current Prometheus draft as the working understanding.");
    sections.push(format!(
        "**My Understanding**:
{understanding}"
    ));

    let research = input
        .research
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("No extra repo research findings were captured yet.");
    sections.push(format!(
        "**Research Findings**:
{research}"
    ));

    sections.push(
        "Please identify:
1. Questions I should have asked but didn't
2. Guardrails that need to be explicitly set
3. Potential scope creep areas to lock down
4. Assumptions I'm making that need validation
5. Missing acceptance criteria
6. Edge cases not addressed

Return actionable guidance for Prometheus only. Focus on planning quality, not implementation."
            .to_string(),
    );
    sections.join(
        "

",
    )
}

pub fn prometheus_user_choice_payload() -> Value {
    json!({
        "questions": [{
            "header": "Next Step",
            "question": "Plan is ready. How would you like to proceed?",
            "options": [
                { "label": "Start Work", "description": "Hand the reviewed plan to Atlas with /start-work and begin tracked execution." },
                { "label": "High Accuracy Review", "description": "Run Momus review before execution." }
            ]
        }]
    })
}

pub fn parse_prometheus_user_choice(output: &str) -> String {
    serde_json::from_str::<Value>(output)
        .ok()
        .and_then(|value| value.get("answers").and_then(|v| v.as_array()).cloned())
        .and_then(|answers| answers.first().cloned())
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| PROMETHEUS_DEFAULT_HANDOFF_CHOICE.to_string())
}

pub fn should_run_high_accuracy_review(choice: &str) -> bool {
    choice.eq_ignore_ascii_case(PROMETHEUS_HIGH_ACCURACY_CHOICE)
}

pub fn recommend_start_work(review_gate_approved: Option<bool>) -> bool {
    !matches!(review_gate_approved, Some(false))
}

pub fn prometheus_transition_graph(
    mut transitions: Vec<SchedulerTransitionSpec>,
) -> SchedulerTransitionGraph {
    transitions.retain(|transition| transition.from != SchedulerStageKind::Handoff);
    transitions.push(SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnUserChoice(PROMETHEUS_DEFAULT_HANDOFF_CHOICE),
        to: SchedulerTransitionTarget::Finish,
    });
    transitions.push(SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnUserChoice(PROMETHEUS_HIGH_ACCURACY_CHOICE),
        to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Plan),
    });
    transitions.push(SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnHighAccuracyApproved,
        to: SchedulerTransitionTarget::Finish,
    });
    transitions.push(SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnHighAccuracyBlocked,
        to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Plan),
    });
    SchedulerTransitionGraph::new(transitions)
}

pub fn prometheus_effect_protocol(stages: &[SchedulerStageKind]) -> SchedulerEffectProtocol {
    let mut effects = Vec::new();

    if stages.contains(&SchedulerStageKind::Interview) {
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Interview,
            moment: SchedulerEffectMoment::OnSuccess,
            effect: SchedulerEffectKind::SyncDraftArtifact,
        });
        if stages.contains(&SchedulerStageKind::Plan) {
            effects.push(SchedulerEffectSpec {
                stage: SchedulerStageKind::Interview,
                moment: SchedulerEffectMoment::OnSuccess,
                effect: SchedulerEffectKind::RegisterWorkflowTodos,
            });
        }
    }

    if stages.contains(&SchedulerStageKind::Plan) {
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Plan,
            moment: SchedulerEffectMoment::OnEnter,
            effect: SchedulerEffectKind::EnsurePlanningArtifactPath,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Plan,
            moment: SchedulerEffectMoment::OnEnter,
            effect: SchedulerEffectKind::RegisterWorkflowTodos,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Plan,
            moment: SchedulerEffectMoment::OnEnter,
            effect: SchedulerEffectKind::RequestAdvisoryReview,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Plan,
            moment: SchedulerEffectMoment::OnEnter,
            effect: SchedulerEffectKind::SyncDraftArtifact,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Plan,
            moment: SchedulerEffectMoment::OnSuccess,
            effect: SchedulerEffectKind::PersistPlanningArtifact,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Plan,
            moment: SchedulerEffectMoment::OnSuccess,
            effect: SchedulerEffectKind::SyncDraftArtifact,
        });
    }

    if stages.contains(&SchedulerStageKind::Handoff) {
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Handoff,
            moment: SchedulerEffectMoment::OnEnter,
            effect: SchedulerEffectKind::RequestUserChoice,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Handoff,
            moment: SchedulerEffectMoment::BeforeTransition,
            effect: SchedulerEffectKind::RunApprovalReviewLoop,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Handoff,
            moment: SchedulerEffectMoment::OnSuccess,
            effect: SchedulerEffectKind::DeleteDraftArtifact,
        });
        effects.push(SchedulerEffectSpec {
            stage: SchedulerStageKind::Handoff,
            moment: SchedulerEffectMoment::OnSuccess,
            effect: SchedulerEffectKind::DecorateFinalOutput,
        });
    }

    SchedulerEffectProtocol::new(effects)
}

pub fn resolve_prometheus_transition_target(
    transitions: &[&SchedulerTransitionSpec],
    context: PrometheusTransitionContext<'_>,
) -> Option<SchedulerTransitionTarget> {
    if context.review_gate_approved == Some(false) {
        if let Some(target) = find_prometheus_transition_target(
            transitions,
            SchedulerTransitionTrigger::OnHighAccuracyBlocked,
        ) {
            return Some(target);
        }
    }

    if context.review_gate_approved == Some(true) {
        if let Some(target) = find_prometheus_transition_target(
            transitions,
            SchedulerTransitionTrigger::OnHighAccuracyApproved,
        ) {
            return Some(target);
        }
    }

    if let Some(choice) = context.user_choice {
        if let Some(transition) = transitions
            .iter()
            .find(|transition| match transition.trigger {
                SchedulerTransitionTrigger::OnUserChoice(expected) => {
                    expected.eq_ignore_ascii_case(choice)
                }
                _ => false,
            })
        {
            return Some(transition.to);
        }
    }

    find_prometheus_transition_target(transitions, SchedulerTransitionTrigger::OnSuccess)
}

fn find_prometheus_transition_target(
    transitions: &[&SchedulerTransitionSpec],
    trigger: SchedulerTransitionTrigger,
) -> Option<SchedulerTransitionTarget> {
    transitions
        .iter()
        .find(|transition| transition.trigger == trigger)
        .map(|transition| transition.to)
}

pub fn resolve_prometheus_effect_dispatch(
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
            if context
                .user_choice
                .as_deref()
                .map(should_run_high_accuracy_review)
                .unwrap_or(false)
            {
                SchedulerEffectDispatch::RunApprovalReviewLoop
            } else {
                SchedulerEffectDispatch::Skip
            }
        }
        SchedulerEffectKind::DeleteDraftArtifact => {
            if resolve_prometheus_handoff_decoration(context).recommend_start_work {
                SchedulerEffectDispatch::DeleteDraftArtifact
            } else {
                SchedulerEffectDispatch::Skip
            }
        }
        SchedulerEffectKind::DecorateFinalOutput => SchedulerEffectDispatch::DecorateFinalOutput(
            resolve_prometheus_handoff_decoration(context),
        ),
    }
}

pub fn resolve_prometheus_handoff_decoration(
    context: SchedulerEffectContext,
) -> SchedulerHandoffDecoration {
    let recommend_start_work = recommend_start_work(context.review_gate_approved);
    SchedulerHandoffDecoration {
        plan_path: context.planning_artifact_path,
        draft_path: context.draft_artifact_path,
        draft_deleted: recommend_start_work && !context.draft_exists,
        recommend_start_work,
        review_gate_approved: context.review_gate_approved,
    }
}

pub fn decorate_prometheus_handoff_output(
    content: String,
    decoration: SchedulerHandoffDecoration,
) -> String {
    append_handoff_guidance(
        content,
        decoration.plan_path.as_deref(),
        decoration.draft_path.as_deref(),
        decoration.draft_deleted,
        decoration.recommend_start_work,
        decoration.review_gate_approved,
    )
}

fn prometheus_content_has_blocking_decisions(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("[decision needed:")
        || lower.contains("decision needed")
        || lower.contains("blocked pending the decisions listed below")
}

fn infer_prometheus_review_gate(content: &str) -> Option<bool> {
    let lower = content.to_ascii_lowercase();
    if lower.contains("momus has not yet approved")
        || lower.contains("high accuracy review: still blocked")
        || lower.contains("do not run `/start-work` yet")
    {
        Some(false)
    } else if lower.contains("approved by momus")
        || lower.contains("high accuracy review: approved")
    {
        Some(true)
    } else {
        None
    }
}

pub fn normalize_prometheus_final_output(output: &str) -> String {
    let normalized_input = normalize_embedded_delivery_summary(output);
    let trimmed = normalized_input.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if prometheus_handoff_output_has_required_shape(trimmed) {
        return trimmed.to_string();
    }

    let review_gate_approved = infer_prometheus_review_gate(trimmed);
    let recommend_start_work =
        review_gate_approved != Some(false) && !prometheus_content_has_blocking_decisions(trimmed);

    append_handoff_guidance(
        trimmed.to_string(),
        None,
        None,
        false,
        recommend_start_work,
        review_gate_approved,
    )
}

pub fn normalize_prometheus_review_stage_output(
    snapshot: PrometheusReviewStateSnapshot<'_>,
    review_output: &str,
) -> String {
    normalize_prometheus_review_output(
        PrometheusReviewContext {
            route_rationale_summary: snapshot.route_rationale_summary,
            planning_artifact_path: snapshot.planning_artifact_path,
            interviewed: snapshot.interviewed,
            planned: snapshot.planned,
            draft_snapshot: snapshot.draft_snapshot,
            advisory_review: snapshot.advisory_review,
        },
        review_output,
    )
}
