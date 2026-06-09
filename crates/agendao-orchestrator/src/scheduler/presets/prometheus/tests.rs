use async_trait::async_trait;

use super::*;
use crate::{
    execute_scheduler_effect_dispatch, OrchestratorError, SchedulerDraftArtifactInput,
    SchedulerEffectContext, SchedulerEffectKind, SchedulerEffectMoment, SchedulerHandoffDecoration,
    SchedulerHandoffStageInput, SchedulerInterviewStageInput, SchedulerPlanStageInput,
    SchedulerPlanningArtifactInput, SchedulerPresetEffectExecutor, SchedulerReviewStageInput,
    SchedulerStageKind, SchedulerTransitionSpec, SchedulerTransitionTarget,
    SchedulerTransitionTrigger,
};

#[test]
fn prometheus_uses_planning_first_stages() {
    assert_eq!(
        prometheus_default_stages(),
        vec![
            SchedulerStageKind::RequestAnalysis,
            SchedulerStageKind::Interview,
            SchedulerStageKind::Plan,
            SchedulerStageKind::Review,
            SchedulerStageKind::Handoff,
        ]
    );
}

#[test]
fn prometheus_plan_sets_orchestrator() {
    let plan = prometheus_plan();
    assert_eq!(plan.orchestrator.as_deref(), Some("prometheus"));
}

#[test]
fn prometheus_workflow_todos_match_omo_phase_shape() {
    let payload = prometheus_workflow_todos_payload();
    let todos = payload["todos"].as_array().expect("todos array");
    assert_eq!(todos.len(), 8);
    assert_eq!(todos[0]["id"], "plan-1");
    assert_eq!(todos[7]["id"], "plan-8");
    assert!(todos[0]["content"].as_str().unwrap().contains("Metis"));
    assert!(todos[6]["content"].as_str().unwrap().contains("Momus"));
    assert!(todos[7]["content"]
        .as_str()
        .unwrap()
        .contains("/start-work"));
}

#[test]
fn prometheus_momus_round_limit_is_effectively_unbounded() {
    assert_eq!(PROMETHEUS_MAX_MOMUS_ROUNDS, usize::MAX);
}

#[test]
fn prometheus_plan_input_carries_omo_planning_contract() {
    let input = compose_prometheus_plan_input(SchedulerPlanStageInput {
        original_request: "Fix the TUI scroll behavior.",
        session_context: Some("## Session Continuity Context\nprior user preference"),
        request_brief: "Need a planning artifact before execution.",
        route_decision_json: Some(r#"{"preset":"prometheus"}"#),
        route_output: Some("Prometheus planner route selected."),
        planning_artifact_path: Some(".sisyphus/plans/tui-scroll.md"),
        draft_artifact_path: Some(".sisyphus/drafts/tui-scroll.md"),
        draft_context: Some("- Requirement captured"),
        interview_output: Some("Need scrollbar and session preview polish."),
        advisory_review: Some("- Preserve planner-only boundary"),
        approval_feedback: Some("- Tighten QA scenarios"),
        current_plan: "plan snapshot",
        skill_tree_context: None,
        available_agents: &[],
        available_categories: &[],
        skill_list: &[],
    });

    assert!(input.contains("prior user preference"));
    assert!(input.contains("Generate exactly one work plan under `.sisyphus/plans/{name}.md`"));
    assert!(input.contains("[DECISION NEEDED: ...]"));
    assert!(input.contains("Agent-Executed QA Scenarios"));
    assert!(input.contains("Summary Contract"));
}

#[test]
fn prometheus_plan_input_keeps_capabilities_after_planner_charter() {
    let input = compose_prometheus_plan_input(SchedulerPlanStageInput {
        original_request: "Fix the TUI scroll behavior.",
        session_context: None,
        request_brief: "Need a planning artifact before execution.",
        route_decision_json: None,
        route_output: None,
        planning_artifact_path: None,
        draft_artifact_path: None,
        draft_context: None,
        interview_output: None,
        advisory_review: None,
        approval_feedback: None,
        current_plan: "plan snapshot",
        skill_tree_context: None,
        available_agents: &[crate::scheduler::AvailableAgentMeta {
            name: "atlas".to_string(),
            description: "Atlas orchestrator".to_string(),
            mode: "subagent".to_string(),
            cost: "CHEAP".to_string(),
        }],
        available_categories: &[],
        skill_list: &[],
    });

    assert!(
        input.find("## Planner Charter").unwrap()
            < input.find("## Available Execution Resources").unwrap()
    );
}

#[test]
fn prometheus_metis_input_matches_omo_review_shape() {
    let input = compose_prometheus_advisory_review_input(crate::SchedulerAdvisoryReviewInput {
        goal: "Polish the TUI workflow",
        original_request: "Fix backspace popup and improve session scrolling.",
        discussed: Some(
            "- Backspace should not pop a blocking dialog
- Session pane needs better scrolling affordance",
        ),
        draft_context: Some(
            "- Preserve slash palette
- Keep Prometheus planner-only",
        ),
        research: Some("- TUI currently opens a popup on backspace in one path"),
    });

    assert!(input.contains("Review this planning session before I generate the work plan"));
    assert!(input.contains("**User's Goal**"));
    assert!(input.contains("**What We Discussed**"));
    assert!(input.contains("**My Understanding**"));
    assert!(input.contains("**Research Findings**"));
    assert!(input.contains("Questions I should have asked but didn't"));
    assert!(input.contains("Missing acceptance criteria"));
}

#[test]
fn prometheus_review_input_enforces_review_shape() {
    let input = compose_prometheus_review_input(SchedulerReviewStageInput {
        original_request: "Plan the scheduler cleanup.",
        session_context: None,
        request_brief: "Prometheus should only review planning output.",
        route_summary: Some("Keep Prometheus planner-only."),
        draft_context: Some("draft notes"),
        interview_output: Some("interview notes"),
        execution_plan: Some("# Plan"),
        advisory_review: Some("- Guardrail 1"),
        approval_feedback: Some("- Fix missing acceptance criteria"),
        saved_planning_artifact: Some(".sisyphus/plans/scheduler.md"),
        skills_index_markdown: None,
        delegation_output: None,
    });

    assert!(input.contains("Review Delivery Shape"));
    assert!(input.contains("**Auto-Resolved**"));
    assert!(input.contains("**Defaults Applied**"));
    assert!(input.contains("**Decisions Needed**"));
    assert!(input.contains("Do not review it as executed work"));
}

#[test]
fn prometheus_handoff_input_guides_start_work_with_plan_name() {
    let input = compose_prometheus_handoff_input(SchedulerHandoffStageInput {
        original_request: "Prepare the plan and hand it off.",
        session_context: None,
        request_brief: "Planner-only workflow.",
        current_plan: "plan snapshot",
        draft_context: Some("draft notes"),
        interview_output: Some("interview notes"),
        planning_output: Some("# Plan"),
        review_output: Some("## Plan Generated: scheduler"),
        approval_review: Some("OKAY"),
        user_choice: Some("Start Work"),
        saved_planning_artifact: Some(".sisyphus/plans/scheduler.md"),
    });

    assert!(input.contains("Recommended command: `/start-work scheduler`"));
    assert!(input.contains("hands the reviewed plan to Atlas for execution orchestration"));
    assert!(input.contains("Code execution has not been performed here"));
    assert!(input.contains("describe it as the Atlas execution handoff"));
}

#[test]
fn prometheus_handoff_output_normalizes_weak_content_into_structured_delivery() {
    let output = decorate_prometheus_handoff_output(
        "Ready for execution.".to_string(),
        SchedulerHandoffDecoration {
            plan_path: Some(".sisyphus/plans/scheduler.md".to_string()),
            draft_path: Some(".sisyphus/drafts/scheduler.md".to_string()),
            draft_deleted: true,
            recommend_start_work: true,
            review_gate_approved: Some(true),
        },
    );

    assert!(output.contains("## Plan Summary"));
    assert!(output.contains("**Recommended Next Step**"));
    assert!(output.contains("**Execution Status**"));
    assert!(output.contains("/start-work scheduler"));
    assert!(output.contains("hand the reviewed plan to Atlas"));
    assert!(output.contains("Draft cleaned up:"));
}

#[test]
fn prometheus_handoff_output_blocks_start_work_when_momus_not_approved() {
    let output = decorate_prometheus_handoff_output(
        "[DECISION NEEDED: choose the final scrollbar style]".to_string(),
        SchedulerHandoffDecoration {
            plan_path: Some(".sisyphus/plans/scheduler.md".to_string()),
            draft_path: Some(".sisyphus/drafts/scheduler.md".to_string()),
            draft_deleted: false,
            recommend_start_work: false,
            review_gate_approved: Some(false),
        },
    );

    assert!(output.contains("Do not run `/start-work` yet."));
    assert!(output.contains("Momus has not yet approved the plan."));
    assert!(output.contains("DECISION NEEDED"));
}

#[test]
fn prometheus_final_output_normalizes_review_shape_into_handoff_delivery() {
    let output = normalize_prometheus_final_output(
        r#"## Plan Generated: scheduler

**Key Decisions Made**
- Keep Prometheus planner-only.

**Scope**
- IN: planner workflow
- OUT: code execution

**Guardrails Applied**
- Preserve slash palette.

**Auto-Resolved**
- None.

**Defaults Applied**
- Keep review enabled before handoff.

**Decisions Needed**
- None.

**Handoff Readiness**
- Ready for handoff once the reviewed plan is accepted.

**Review Notes**
- Plan is consistent."#,
    );

    assert!(output.contains("## Plan Summary"));
    assert!(output.contains("**Recommended Next Step**"));
    assert!(output.contains("hand the reviewed plan to Atlas"));
    assert!(output.contains("Prometheus remains planner-only"));
}

#[test]
fn prometheus_final_output_preserves_structured_handoff_delivery() {
    let structured = r#"## Plan Summary
- Reviewed planning handoff prepared for Prometheus.

**Recommended Next Step**
- Run `/start-work scheduler` to hand the reviewed plan to Atlas.

**Remaining Decisions or Risks**
- None.

**Execution Status**
- Code execution has not been performed in this workflow."#;

    assert_eq!(normalize_prometheus_final_output(structured), structured);
}

#[test]
fn prometheus_final_output_strips_preface_before_embedded_delivery() {
    let prefaced = "Review complete.\n\n## `## Plan Summary`\n- Reviewed planning handoff prepared for Prometheus.\n\n**Recommended Next Step**\n- Run `/start-work scheduler` to hand the reviewed plan to Atlas.\n\n**Remaining Decisions or Risks**\n- None.\n\n**Execution Status**\n- Code execution has not been performed in this workflow.";
    let normalized = normalize_prometheus_final_output(prefaced);
    assert!(normalized.starts_with("## Plan Summary"));
    assert!(!normalized.contains("Review complete."));
}

#[test]
fn prometheus_interview_input_mentions_draft_memory_and_clearance() {
    let input = compose_prometheus_interview_input(SchedulerInterviewStageInput {
        original_request: "Help me plan a refactor.",
        session_context: None,
        request_brief: "Need a Prometheus interview first.",
        route_decision_json: Some(r#"{"preset":"prometheus"}"#),
        draft_artifact_path: Some(".sisyphus/drafts/refactor.md"),
        draft_context: Some("- Open question"),
        current_plan: "plan snapshot",
        skill_tree_context: None,
    });

    assert!(input.contains("`.sisyphus/drafts/{name}.md`"));
    assert!(input.contains("read-only repo inspection before asking questions"));
    assert!(input.contains("call the `question` tool"));
    assert!(input.contains("Do not leave a blocking question only in the transcript"));
    assert!(input.contains("auto-transition"));
}

#[test]
fn prometheus_draft_artifact_matches_omo_core_sections() {
    let draft = compose_prometheus_draft_artifact(SchedulerDraftArtifactInput {
        original_request: "Fix the TUI backspace popup.",
        request_brief: "Need a planner-only workflow and no direct execution.",
        route_summary: Some("Keep the session in Prometheus planner mode."),
        interview_output: Some(
            "- Scope: TUI input only
- Constraint: preserve slash palette",
        ),
        advisory_review: Some("- Guardrail: avoid changing unrelated keybindings"),
        current_plan: Some("- [DECISION NEEDED: pick the final footer icon]"),
        approval_review: None,
        user_choice: Some("High Accuracy Review"),
        planning_artifact_path: Some(".sisyphus/plans/tui-input.md"),
        draft_artifact_path: Some(".sisyphus/drafts/tui-input.md"),
    });

    assert!(draft.contains("## Requirements (confirmed)"));
    assert!(draft.contains("## Technical Decisions"));
    assert!(draft.contains("## Research Findings"));
    assert!(draft.contains("## Open Questions"));
    assert!(draft.contains("## Scope Boundaries"));
    assert!(draft.contains("DECISION NEEDED"));
}

#[test]
fn prometheus_planning_artifact_wraps_raw_plan_in_omo_shape() {
    let artifact = compose_prometheus_planning_artifact(SchedulerPlanningArtifactInput {
        request_brief: "Align the TUI behavior with OMO while staying planner-only.",
        route_summary: Some("Keep Prometheus active and preserve review before handoff."),
        interview_output: Some(
            "- Need better backspace handling
- Need visible scrollbar in session pane",
        ),
        advisory_review: Some("- Guardrail: avoid changing unrelated TUI flows"),
        planning_output: "# Plan

- Fix backspace popup
- Add session scrollbar",
        planning_artifact_path: Some(".sisyphus/plans/tui-alignment.md"),
    });

    assert!(artifact.contains("## TL;DR"));
    assert!(artifact.contains("## Context"));
    assert!(artifact.contains("## Work Objectives"));
    assert!(artifact.contains("## Verification Strategy"));
    assert!(artifact.contains("## Execution Strategy"));
    assert!(artifact.contains("## TODOs"));
    assert!(artifact.contains("### Dependency Matrix"));
    assert!(artifact.contains("### Agent Dispatch Summary"));
    assert!(artifact.contains("**Recommended Agent Profile**"));
    assert!(artifact.contains("**References** (CRITICAL - Be Exhaustive):"));
    assert!(artifact.contains("**QA Scenarios (MANDATORY)**"));
    assert!(artifact.contains(".sisyphus/evidence/task-1-happy.ext"));
    assert!(artifact.contains("Fix backspace popup"));
    assert!(artifact.contains("Add session scrollbar"));
}

#[test]
fn prometheus_effect_protocol_tracks_plan_and_handoff_timing() {
    let effects = prometheus_effect_protocol(&prometheus_default_stages());

    assert!(effects.effects.iter().any(|effect| {
        effect.stage == SchedulerStageKind::Interview
            && effect.moment == SchedulerEffectMoment::OnSuccess
            && effect.effect == SchedulerEffectKind::RegisterWorkflowTodos
    }));
    assert!(effects.effects.iter().any(|effect| {
        effect.stage == SchedulerStageKind::Plan
            && effect.moment == SchedulerEffectMoment::OnEnter
            && effect.effect == SchedulerEffectKind::RequestAdvisoryReview
    }));
    assert!(effects.effects.iter().any(|effect| {
        effect.stage == SchedulerStageKind::Handoff
            && effect.moment == SchedulerEffectMoment::BeforeTransition
            && effect.effect == SchedulerEffectKind::RunApprovalReviewLoop
    }));
}

#[test]
fn prometheus_transition_resolution_prefers_high_accuracy_block() {
    let graph = prometheus_transition_graph(vec![
        SchedulerTransitionSpec {
            from: SchedulerStageKind::Interview,
            trigger: SchedulerTransitionTrigger::OnSuccess,
            to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Plan),
        },
        SchedulerTransitionSpec {
            from: SchedulerStageKind::Plan,
            trigger: SchedulerTransitionTrigger::OnSuccess,
            to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Review),
        },
        SchedulerTransitionSpec {
            from: SchedulerStageKind::Review,
            trigger: SchedulerTransitionTrigger::OnSuccess,
            to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Handoff),
        },
        SchedulerTransitionSpec {
            from: SchedulerStageKind::Handoff,
            trigger: SchedulerTransitionTrigger::OnSuccess,
            to: SchedulerTransitionTarget::Finish,
        },
    ]);
    let transitions = graph.transitions_from(SchedulerStageKind::Handoff);

    let target = resolve_prometheus_transition_target(
        &transitions,
        PrometheusTransitionContext {
            user_choice: Some(PROMETHEUS_HIGH_ACCURACY_CHOICE),
            review_gate_approved: Some(false),
        },
    );

    assert_eq!(
        target,
        Some(SchedulerTransitionTarget::Stage(SchedulerStageKind::Plan))
    );
}

#[test]
fn prometheus_transition_resolution_finishes_on_start_work() {
    let graph = prometheus_transition_graph(vec![SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnSuccess,
        to: SchedulerTransitionTarget::Finish,
    }]);
    let transitions = graph.transitions_from(SchedulerStageKind::Handoff);

    let target = resolve_prometheus_transition_target(
        &transitions,
        PrometheusTransitionContext {
            user_choice: Some(PROMETHEUS_DEFAULT_HANDOFF_CHOICE),
            review_gate_approved: None,
        },
    );

    assert_eq!(target, Some(SchedulerTransitionTarget::Finish));
}

#[derive(Default)]
struct RecordingPrometheusEffectExecutor {
    calls: Vec<&'static str>,
    decorations: Vec<SchedulerHandoffDecoration>,
}

#[async_trait]
impl SchedulerPresetEffectExecutor for RecordingPrometheusEffectExecutor {
    async fn ensure_planning_artifact_path(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("ensure_planning_artifact_path");
        Ok(())
    }

    async fn persist_planning_artifact(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("persist_planning_artifact");
        Ok(())
    }

    async fn sync_draft_artifact(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("sync_draft_artifact");
        Ok(())
    }

    async fn register_workflow_todos(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("register_workflow_todos");
        Ok(())
    }

    async fn request_advisory_review(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("request_advisory_review");
        Ok(())
    }

    async fn request_user_choice(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("request_user_choice");
        Ok(())
    }

    async fn run_approval_review_loop(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("run_approval_review_loop");
        Ok(())
    }

    async fn delete_draft_artifact(&mut self) -> Result<(), OrchestratorError> {
        self.calls.push("delete_draft_artifact");
        Ok(())
    }

    async fn decorate_final_output(
        &mut self,
        decoration: SchedulerHandoffDecoration,
    ) -> Result<(), OrchestratorError> {
        self.calls.push("decorate_final_output");
        self.decorations.push(decoration);
        Ok(())
    }
}

#[tokio::test]
async fn prometheus_effect_executor_runs_momus_for_high_accuracy_choice() {
    let dispatch = resolve_prometheus_effect_dispatch(
        SchedulerEffectKind::RunApprovalReviewLoop,
        SchedulerEffectContext {
            planning_artifact_path: None,
            draft_artifact_path: None,
            user_choice: Some(PROMETHEUS_HIGH_ACCURACY_CHOICE.to_string()),
            review_gate_approved: None,
            draft_exists: true,
        },
    );
    let mut executor = RecordingPrometheusEffectExecutor::default();

    execute_scheduler_effect_dispatch(dispatch, &mut executor)
        .await
        .expect("dispatch should execute");

    assert_eq!(executor.calls, vec!["run_approval_review_loop"]);
}

#[tokio::test]
async fn prometheus_effect_executor_decorates_start_work_handoff() {
    let dispatch = resolve_prometheus_effect_dispatch(
        SchedulerEffectKind::DecorateFinalOutput,
        SchedulerEffectContext {
            planning_artifact_path: Some(".sisyphus/plans/plan-demo.md".to_string()),
            draft_artifact_path: Some(".sisyphus/drafts/draft-demo.md".to_string()),
            user_choice: Some(PROMETHEUS_DEFAULT_HANDOFF_CHOICE.to_string()),
            review_gate_approved: Some(true),
            draft_exists: false,
        },
    );
    let mut executor = RecordingPrometheusEffectExecutor::default();

    execute_scheduler_effect_dispatch(dispatch, &mut executor)
        .await
        .expect("dispatch should execute");

    assert_eq!(executor.calls, vec!["decorate_final_output"]);
    assert_eq!(executor.decorations.len(), 1);
    let decoration = &executor.decorations[0];
    assert_eq!(
        decoration.plan_path.as_deref(),
        Some(".sisyphus/plans/plan-demo.md")
    );
    assert_eq!(
        decoration.draft_path.as_deref(),
        Some(".sisyphus/drafts/draft-demo.md")
    );
    assert!(decoration.draft_deleted);
    assert!(decoration.recommend_start_work);
    assert_eq!(decoration.review_gate_approved, Some(true));
}
