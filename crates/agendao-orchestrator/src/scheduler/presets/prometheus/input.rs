use crate::scheduler::prompt_support::build_capabilities_summary;
use crate::scheduler::{
    SchedulerHandoffStageInput, SchedulerInterviewStageInput, SchedulerPlanStageInput,
    SchedulerReviewStageInput,
};

use super::plan_start_work_command;

fn push_optional_section(sections: &mut Vec<String>, title: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        sections.push(format!(
            "## {title}
{value}"
        ));
    }
}

pub fn compose_prometheus_interview_input(input: SchedulerInterviewStageInput<'_>) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
interview"
            .to_string(),
    );
    push_optional_section(
        &mut sections,
        "Session Continuity Context",
        input.session_context,
    );
    sections.push(format!(
        "## Original Request
{}",
        input.original_request
    ));
    sections.push(format!(
        "## Request Brief
{}",
        input.request_brief
    ));
    push_optional_section(&mut sections, "Route Decision", input.route_decision_json);
    push_optional_section(
        &mut sections,
        "Draft Artifact Path",
        input.draft_artifact_path,
    );
    push_optional_section(&mut sections, "Draft Context", input.draft_context);
    sections.push(format!(
        "## Current Plan
{}",
        input.current_plan
    ));
    push_optional_section(
        &mut sections,
        "Skill Tree Context",
        input.skill_tree_context,
    );
    sections.push(
        "## Interview Focus
- Classify the request intent before planning
- Prefer read-only repo inspection before asking questions
- Keep `.sisyphus/drafts/{name}.md` updated as external memory
- If a user answer is required to continue, call the `question` tool rather than asking only in plain markdown
- End with either a `question` tool call for the next material blocker or a clear signal that requirements are ready for auto-transition"
            .to_string(),
    );
    sections.push(
        "## Interview Charter
Stay in Phase 1 interview mode. Resolve discoverable unknowns with read-only inspection first. Ask only when a remaining preference, tradeoff, or requirement ambiguity materially changes the work plan. If that ambiguity blocks plan generation, you MUST use the `question` tool. Do not leave a blocking question only in the transcript. Tell the user the draft exists and return markdown with: Request Understanding, Discoverable Facts, Open Decisions, Constraints, Recommended Planning Focus."
            .to_string(),
    );
    sections.join(
        "

",
    )
}

pub fn compose_prometheus_plan_input(input: SchedulerPlanStageInput<'_>) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
plan"
            .to_string(),
    );
    push_optional_section(
        &mut sections,
        "Session Continuity Context",
        input.session_context,
    );
    sections.push(format!(
        "## Original Request
{}",
        input.original_request
    ));
    sections.push(format!(
        "## Request Brief
{}",
        input.request_brief
    ));
    push_optional_section(&mut sections, "Route Decision", input.route_decision_json);
    push_optional_section(&mut sections, "Route Output", input.route_output);
    push_optional_section(
        &mut sections,
        "Planning Artifact Path",
        input.planning_artifact_path,
    );
    push_optional_section(
        &mut sections,
        "Draft Artifact Path",
        input.draft_artifact_path,
    );
    push_optional_section(&mut sections, "Draft Context", input.draft_context);
    push_optional_section(&mut sections, "Interview Output", input.interview_output);
    push_optional_section(&mut sections, "Metis Review", input.advisory_review);
    push_optional_section(&mut sections, "Momus Feedback", input.approval_feedback);
    sections.push(format!(
        "## Current Plan
{}",
        input.current_plan
    ));
    push_optional_section(
        &mut sections,
        "Skill Tree Context",
        input.skill_tree_context,
    );
    let capabilities = build_capabilities_summary(
        input.available_agents,
        input.available_categories,
        input.skill_list,
    );
    sections.push(
        "## Plan Generation Focus
- Incorporate Metis findings silently before writing the plan
- Generate exactly one work plan under `.sisyphus/plans/{name}.md`
- Preserve the single-plan mandate and maximize parallel execution where the scope supports it
- Every task must include concrete acceptance criteria and Agent-Executed QA Scenarios
- If a critical question remains unresolved, keep planning moving with `[DECISION NEEDED: ...]`
- If Momus feedback is present, address EVERY issue before considering the plan ready"
            .to_string(),
    );
    sections.push(
        "## Summary Contract
When the plan is ready, the downstream review/handoff must be able to present: `## Plan Generated: {name}`, `**Key Decisions Made**`, `**Scope**`, `**Guardrails Applied**`, `**Auto-Resolved**`, `**Defaults Applied**`, `**Decisions Needed**`, and the saved plan path."
            .to_string(),
    );
    sections.push(
        "## Planner Charter
Generate the Prometheus work plan only. Do not implement, do not edit non-markdown files, and do not claim execution is complete. Produce a concrete plan for `.sisyphus/plans/{name}.md` with assumptions, guardrails, verification strategy, parallelization, and risks."
            .to_string(),
    );
    // Keep runtime capability catalogs after stable planner-governance text
    // so provider-side prompt caching sees a more reusable prefix.
    if !capabilities.is_empty() {
        sections.push(format!(
            "## Available Execution Resources\n{capabilities}\n\nWhen generating the work plan, reference these agents and categories in the Agent Dispatch Summary and per-task Recommended Agent Profile sections."
        ));
    }
    sections.join(
        "

",
    )
}

pub fn compose_prometheus_review_input(input: SchedulerReviewStageInput<'_>) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
review"
            .to_string(),
    );
    push_optional_section(
        &mut sections,
        "Session Continuity Context",
        input.session_context,
    );
    sections.push(format!(
        "## Original Request
{}",
        input.original_request
    ));
    sections.push(format!(
        "## Request Brief
{}",
        input.request_brief
    ));
    push_optional_section(&mut sections, "Route Summary", input.route_summary);
    push_optional_section(&mut sections, "Draft Context", input.draft_context);
    push_optional_section(&mut sections, "Interview Output", input.interview_output);
    push_optional_section(&mut sections, "Execution Plan", input.execution_plan);
    push_optional_section(&mut sections, "Metis Review", input.advisory_review);
    push_optional_section(&mut sections, "Momus Feedback", input.approval_feedback);
    push_optional_section(
        &mut sections,
        "Saved Planning Artifact",
        input.saved_planning_artifact,
    );
    push_optional_section(&mut sections, "Skills Index", input.skills_index_markdown);
    push_optional_section(&mut sections, "Additional Context", input.delegation_output);
    sections.push(
        "## Review Delivery Shape
Return markdown in this exact top-level order: `## Plan Generated: {name}` -> `**Key Decisions Made**` -> `**Scope**` -> `**Guardrails Applied**` -> `**Auto-Resolved**` -> `**Defaults Applied**` -> `**Decisions Needed**` -> `**Handoff Readiness**` -> `**Review Notes**`. If a section is empty, write `- None.`"
            .to_string(),
    );
    sections.push(
        "## Review Charter
Review the generated Prometheus plan against the original request, interview context, Metis guardrails, and any Momus feedback. Do not review it as executed work. Tighten the planning handoff, classify gaps as Auto-Resolved / Defaults Applied / Decisions Needed, and preserve Prometheus as planner-only."
            .to_string(),
    );
    sections.join(
        "

",
    )
}

pub fn compose_prometheus_handoff_input(input: SchedulerHandoffStageInput<'_>) -> String {
    let mut sections = Vec::new();
    let suggested_start_work = plan_start_work_command(input.saved_planning_artifact);
    sections.push(
        "## Stage
handoff"
            .to_string(),
    );
    push_optional_section(
        &mut sections,
        "Session Continuity Context",
        input.session_context,
    );
    sections.push(format!(
        "## Original Request
{}",
        input.original_request
    ));
    sections.push(format!(
        "## Request Brief
{}",
        input.request_brief
    ));
    sections.push(format!(
        "## Current Plan
{}",
        input.current_plan
    ));
    push_optional_section(&mut sections, "Draft Context", input.draft_context);
    push_optional_section(&mut sections, "Interview Output", input.interview_output);
    push_optional_section(&mut sections, "Planning Output", input.planning_output);
    push_optional_section(&mut sections, "Review Output", input.review_output);
    push_optional_section(&mut sections, "Momus Review", input.approval_review);
    push_optional_section(&mut sections, "User Choice", input.user_choice);
    push_optional_section(
        &mut sections,
        "Saved Planning Artifact",
        input.saved_planning_artifact,
    );
    sections.push(format!(
        "## Handoff State
- Recommended command: `{suggested_start_work}`
- Prometheus remains planner-only in this workflow
- `/start-work` hands the reviewed plan to Atlas for execution orchestration
- Code execution has not been performed here"
    ));
    sections.push(
        "## Handoff Charter
End this workflow with a reviewed planning handoff. If the plan is approved, make `/start-work` the explicit next action and describe it as the Atlas execution handoff. If Momus still blocks the plan or decisions remain unresolved, say so clearly and do not imply execution is ready."
            .to_string(),
    );
    sections.join(
        "

",
    )
}
