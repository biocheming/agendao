use crate::scheduler::execution_contracts::SHARED_EXECUTION_EVIDENCE_CONTRACT;
use crate::scheduler::SchedulerExecutionOrchestrationStageInput;

fn push_optional_section(sections: &mut Vec<String>, title: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        sections.push(format!("## {title}\n{value}"));
    }
}

pub fn compose_sisyphus_execution_orchestration_input(
    input: SchedulerExecutionOrchestrationStageInput<'_>,
) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
execution-orchestration"
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
    push_optional_section(&mut sections, "Planning Output", input.planning_output);
    push_optional_section(
        &mut sections,
        "Skill Tree Context",
        input.skill_tree_context,
    );
    sections.push(
        "## Execution Frame
- This is Sisyphus single-loop execution orchestration, not an interview-first planning workflow.
- Re-state the detected intent class, choose the execution path, and then act.
- Default bias: delegate; work directly only when the task is genuinely super simple.
- If a fitting specialist, category, or skill-backed worker is available, do not become the sole implementer for non-trivial work.
- If you choose to work directly, make the triviality judgment obvious from the result.
- The value here comes from action plus verification, not from a reviewed planning handoff."
            .to_string(),
    );
    sections.push(
        "## Execution Priorities
- Assess codebase shape before following local patterns blindly.
- Run explore/librarian research in parallel before committing on non-trivial repo questions.
- Match delegation to the best specialist, category, and loaded skills.
- Keep explicit task tracking for bounded delegated work.
- Finish only with evidence-backed verification and concrete outcomes."
            .to_string(),
    );
    // Sisyphus single-pass execution already receives the dynamic capability
    // prompt as the stage system prompt. Keep the user input turn-specific so
    // the same large capability catalog is not sent twice.
    sections.push(
        "## Delivery Contract
Return a concrete execution delivery, not a plan. Keep the top-level result anchored in `## Delivery Summary`, `**Delegation Path**`, `**Execution Outcome**`, `**Verification**`, `**Open Risks or Follow-ups**`."
            .to_string(),
    );
    sections.push(SHARED_EXECUTION_EVIDENCE_CONTRACT.to_string());
    sections.join("\n\n")
}
