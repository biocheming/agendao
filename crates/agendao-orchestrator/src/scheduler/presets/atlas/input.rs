use crate::scheduler::execution_contracts::{
    SHARED_EXECUTION_EVIDENCE_CONTRACT, SHARED_GATE_DECISION_CONTRACT,
    SHARED_RETRY_RECOVERY_CONTRACT, SHARED_VERIFICATION_EVIDENCE_CONTRACT,
};
use crate::scheduler::{
    SchedulerCoordinationGateStageInput, SchedulerCoordinationVerificationStageInput,
    SchedulerExecutionOrchestrationStageInput, SchedulerRetryStageInput,
    SchedulerSynthesisStageInput,
};

use super::build_atlas_dynamic_prompt;

fn push_optional_section(sections: &mut Vec<String>, title: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        sections.push(format!("## {title}\n{value}"));
    }
}

pub fn compose_atlas_execution_orchestration_input(
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
        "Ground Truth Context",
        input.ground_truth_context,
    );
    push_optional_section(
        &mut sections,
        "Skill Tree Context",
        input.skill_tree_context,
    );
    sections.push(
        "## Execution Frame
- This is Atlas coordination-loop orchestration, not a planner-only handoff and not a single autonomous executor.
- Read the current work plan or task list, decompose it into bounded work items, and coordinate the next worker round.
- Delegate one bounded task per worker unless a parallel wave is clearly independent and safe.
- Atlas never writes the implementation itself; it coordinates, verifies, and tracks task completion."
            .to_string(),
    );
    sections.push(
        "## Task Analysis Contract
Before dispatching the next worker round, explicitly determine:
- Total tasks
- Remaining tasks
- Parallel groups
- Sequential dependencies

Keep this compact, task-ledger-first, and grounded in the current plan rather than worker summaries."
            .to_string(),
    );
    sections.push(
        "## Execution Priorities
- Build a parallelization map before dispatching workers.
- Keep explicit task boundaries and terminal status evidence for every item.
- Treat worker completion claims as untrusted until concrete artifacts are checked.
- Use tools, not memory, for current file state, diagnostics, tests, and verification evidence.
- Implement exactly the active plan scope; do not let workers expand task boundaries with extra features.
- Use verification as the QA gate after each delegation round.
- Read notepad context before each delegation and carry forward prior decisions explicitly.
- Store each worker `session_id` and reuse the same session for retries or follow-up fixes.
- Re-read the current plan or active boulder artifact directly before deciding what remains.
- Reach synthesis only when every required task is complete with evidence or a concrete blocker is confirmed."
            .to_string(),
    );
    sections.push(build_atlas_dynamic_prompt(
        input.available_agents,
        input.available_categories,
        input.skill_list,
    ));
    sections.push(SHARED_EXECUTION_EVIDENCE_CONTRACT.to_string());
    sections.join("\n\n")
}

pub fn compose_atlas_synthesis_input(input: SchedulerSynthesisStageInput<'_>) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
synthesis"
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
    push_optional_section(&mut sections, "Route Decision", input.route_decision_json);
    push_optional_section(&mut sections, "Planning Output", input.planning_output);
    push_optional_section(&mut sections, "Delegation Output", input.delegation_output);
    push_optional_section(&mut sections, "Review Output", input.review_output);
    push_optional_section(
        &mut sections,
        "Saved Planning Artifact",
        input.saved_planning_artifact,
    );
    sections.push(
        "## Synthesis Charter
Return the final Atlas delivery in this exact top-level order: `## Delivery Summary` -> `**Task Status**` -> `**Verification**` -> `**Gate Decision**` -> `**Blockers or Risks**` -> `**Next Actions**`. Report by task boundary, prefer reviewed verification over worker claims, and keep explicit evidence in the final answer. The gate decision must say whether Atlas is shipping, continuing another worker round, or blocked."
            .to_string(),
    );
    sections.join("\n\n")
}

pub fn compose_atlas_coordination_gate_input(
    input: SchedulerCoordinationGateStageInput<'_>,
) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
coordination-gate"
            .to_string(),
    );
    sections.push(format!(
        "## Round
{}",
        input.round
    ));
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
        "## Execution Output
{}",
        input.execution_output
    ));
    push_optional_section(
        &mut sections,
        "Verification Output",
        input.verification_output,
    );
    push_optional_section(
        &mut sections,
        "Ground Truth Context",
        input.ground_truth_context,
    );
    sections.push(format!(
        "## Current Plan
{}",
        input.current_plan
    ));
    sections.push(
        "## Coordination Decision Contract
Judge completion by task boundary. Return JSON only. Use `done` only when every required task item is complete with evidence. Use `continue` only when you can name the exact unfinished or weakly-verified task items for the next worker round. Use `blocked` only for a concrete blocker. If execution output already contains a complete structured user-facing delivery, omit `final_response` or reproduce the complete delivery; never replace concrete results with a generic completion sentence. If `final_response` is present, format it as `## Delivery Summary`, `**Task Status**`, `**Verification**`, `**Gate Decision**`, `**Blockers or Risks**`, `**Next Actions**`."
            .to_string(),
    );
    sections.push(SHARED_GATE_DECISION_CONTRACT.to_string());
    sections.push(
        "## Gate Discipline
Before settling the gate, cross-check three authorities: worker output, verification evidence, and the current plan or active boulder state. If verification failed on a worker follow-up, prefer continuing the SAME `session_id` over starting fresh."
            .to_string(),
    );
    sections.join("\n\n")
}

pub fn compose_atlas_coordination_verification_input(
    input: SchedulerCoordinationVerificationStageInput<'_>,
) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
coordination-verification"
            .to_string(),
    );
    sections.push(format!(
        "## Round
{}",
        input.round
    ));
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
        "## Execution Output
{}",
        input.execution_output
    ));
    push_optional_section(&mut sections, "Planning Output", input.planning_output);
    push_optional_section(
        &mut sections,
        "Ground Truth Context",
        input.ground_truth_context,
    );
    push_optional_section(
        &mut sections,
        "Skill Tree Context",
        input.skill_tree_context,
    );
    sections.push(
        "## Verification Charter
Audit each Atlas task item individually against execution evidence. Mark items complete only when the worker output proves the required task boundary. Re-read the current plan or active boulder artifact as ground truth before settling status. Surface incomplete, conflicting, and blocked items explicitly, and do not rewrite implementation here."
            .to_string(),
    );
    sections.push(SHARED_VERIFICATION_EVIDENCE_CONTRACT.to_string());
    sections.join("\n\n")
}

pub fn compose_atlas_retry_input(input: SchedulerRetryStageInput<'_>) -> String {
    let mut sections = Vec::new();
    sections.push(
        "## Stage
coordination-retry"
            .to_string(),
    );
    sections.push(format!(
        "## Round
{}",
        input.round
    ));
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
    sections.push(format!(
        "## Previous Attempt
{}",
        input.previous_output
    ));
    push_optional_section(
        &mut sections,
        "Verification Output",
        input.verification_output,
    );
    push_optional_section(
        &mut sections,
        "Ground Truth Context",
        input.ground_truth_context,
    );
    sections.push(format!(
        "## Retry Summary
{}",
        input.retry_summary
    ));
    push_optional_section(&mut sections, "Retry Focus", input.next_input);
    if input.preferred_continuation_session_id.is_some()
        || input.preferred_continuation_agent_task_id.is_some()
    {
        let mut preferred = Vec::new();
        if let Some(session_id) = input.preferred_continuation_session_id {
            preferred.push(format!("session_id: `{session_id}`"));
        }
        if let Some(agent_task_id) = input.preferred_continuation_agent_task_id {
            preferred.push(format!("agent_task_id: `{agent_task_id}`"));
        }
        sections.push(format!(
            "## Preferred Continuation
{}",
            preferred.join(" | ")
        ));
    }
    push_optional_section(
        &mut sections,
        "Continuation Candidates",
        input.continuation_candidates,
    );
    sections.push(
        "## Continuation Authority
Atlas retry rounds are not fresh starts. Continue from the same task boundary, carry forward prior verification findings, and preserve all previously established constraints. Re-read the current plan or active boulder state before changing task status. If downstream execution can continue the same worker session, prefer continuation over starting a brand-new worker context."
            .to_string(),
    );
    sections.push(
        "## Retry Priorities
- Fix only the named weak or unfinished task items from the gate decision.
- Preserve the existing task ledger and update status by task boundary.
- Carry forward inherited notepad decisions and do not rediscover settled context.
- Prefer concrete evidence over re-explaining prior work."
            .to_string(),
    );
    sections.push(SHARED_RETRY_RECOVERY_CONTRACT.to_string());
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{AvailableAgentMeta, AvailableCategoryMeta};

    #[test]
    fn atlas_execution_input_carries_coordination_loop_semantics() {
        let input = SchedulerExecutionOrchestrationStageInput {
            original_request: "ship the migration cleanup plan",
            session_context: Some("## Session Continuity Context\nprior task ledger"),
            request_brief: "Coordinate the remaining migration tasks",
            route_summary: Some("coordination-heavy task list"),
            planning_output: Some("1. update schema\n2. verify migration"),
            ground_truth_context: Some(
                "authoritative_plan_path: `.sisyphus/plans/demo.md`\nboulder_state_path: `.sisyphus/boulder.json`",
            ),
            skill_tree_context: Some("inherits rust + db context"),
            available_agents: &[AvailableAgentMeta {
                name: "oracle".into(),
                description: "High-IQ reasoning specialist.".into(),
                mode: "subagent".into(),
                cost: "EXPENSIVE".into(),
            }],
            available_categories: &[AvailableCategoryMeta {
                name: "rust".into(),
                description: "Rust implementation and debugging tasks".into(),
            }],
            skill_list: &["review-pr".into()],
        };

        let composed = compose_atlas_execution_orchestration_input(input);
        assert!(composed.contains("prior task ledger"));
        assert!(composed.contains("Atlas coordination-loop orchestration"));
        assert!(composed.contains("decompose it into bounded work items"));
        assert!(composed.contains("parallelization map"));
        assert!(composed.contains("Task Analysis Contract"));
        assert!(composed.contains("Sequential dependencies"));
        assert!(composed.contains("worker completion claims as untrusted"));
        assert!(composed.contains("Use tools, not memory"));
        assert!(composed.contains("Implement exactly the active plan scope"));
        assert!(composed.contains("Ground Truth Context"));
        assert!(composed.contains(".sisyphus/boulder.json"));
        assert!(composed.contains("Read notepad context before each delegation"));
        assert!(composed.contains("Store each worker `session_id`"));
        assert!(composed.contains("active boulder artifact"));
        assert!(composed.contains("6-Section Prompt Structure"));
        assert!(composed.contains("`rust` — Rust implementation and debugging tasks"));
        assert!(composed.contains("Oracle_Usage"));
    }

    #[test]
    fn atlas_synthesis_input_carries_structured_delivery_contract() {
        let composed = compose_atlas_synthesis_input(SchedulerSynthesisStageInput {
            original_request: "ship the migration cleanup plan",
            session_context: None,
            request_brief: "Coordinate remaining migration tasks",
            current_plan: "request-analysis -> execution-orchestration -> synthesis",
            route_decision_json: Some("{\"preset\":\"atlas\"}"),
            planning_output: Some("- task A\n- task B"),
            delegation_output: Some("worker claims task A done"),
            review_output: Some("task A verified"),
            saved_planning_artifact: Some("artifact.md"),
        });
        assert!(composed.contains("## Stage\nsynthesis"));
        assert!(composed.contains("## Delivery Summary"));
        assert!(composed.contains("**Task Status**"));
        assert!(composed.contains("**Gate Decision**"));
        assert!(composed.contains("prefer reviewed verification over worker claims"));
    }

    #[test]
    fn atlas_coordination_verification_input_carries_task_level_verification_contract() {
        let composed = compose_atlas_coordination_verification_input(
            SchedulerCoordinationVerificationStageInput {
                original_request: "ship the migration cleanup plan",
                session_context: None,
                request_brief: "Coordinate remaining migration tasks",
                round: 2,
                execution_output: "worker round output",
                planning_output: Some("- task A\n- task B"),
                ground_truth_context: Some(
                    "authoritative_plan_path: `.sisyphus/plans/demo.md`\nauthoritative_plan_snapshot:\n- [ ] task A",
                ),
                skill_tree_context: Some("inherits rust + db context"),
            },
        );
        assert!(composed.contains("## Stage\ncoordination-verification"));
        assert!(composed.contains("Audit each Atlas task item individually"));
        assert!(composed.contains("task boundary"));
        assert!(composed.contains("active boulder artifact as ground truth"));
        assert!(composed.contains("Ground Truth Context"));
    }

    #[test]
    fn atlas_coordination_gate_input_carries_task_ledger_contract() {
        let composed = compose_atlas_coordination_gate_input(SchedulerCoordinationGateStageInput {
            original_request: "ship the migration cleanup plan",
            session_context: None,
            request_brief: "Coordinate remaining migration tasks",
            current_plan: "request-analysis -> execution-orchestration -> synthesis",
            round: 2,
            execution_output: "worker round output",
            verification_output: Some("task A verified, task B weak"),
            ground_truth_context: Some("authoritative_plan_path: `.sisyphus/plans/demo.md`"),
        });
        assert!(composed.contains("## Stage\ncoordination-gate"));
        assert!(composed.contains("Judge completion by task boundary"));
        assert!(composed.contains("weakly-verified task items"));
        assert!(composed.contains("**Task Status**"));
        assert!(composed.contains("**Gate Decision**"));
        assert!(composed.contains("prefer continuing the SAME `session_id`"));
        assert!(composed.contains("Ground Truth Context"));
    }

    #[test]
    fn atlas_retry_input_carries_continuation_authority() {
        let composed = compose_atlas_retry_input(SchedulerRetryStageInput {
            original_request: "ship the migration cleanup plan",
            session_context: None,
            request_brief: "Coordinate remaining migration tasks",
            current_plan: "request-analysis -> execution-orchestration -> synthesis",
            round: 2,
            previous_output: "worker round output",
            verification_output: Some("task A verified, task B weak"),
            retry_summary: "task B still needs concrete verification",
            next_input: Some("continue task B and verify the migration path"),
            ground_truth_context: Some("authoritative_plan_path: `.sisyphus/plans/demo.md`"),
            preferred_continuation_session_id: Some("task_build_42"),
            preferred_continuation_agent_task_id: Some("agent-task-42"),
            continuation_candidates: Some(
                "- session_id: `task_build_42` | agent_task_id: `agent-task-42` | tool: `task_flow`",
            ),
        });
        assert!(composed.contains("## Stage\ncoordination-retry"));
        assert!(composed.contains("Continuation Authority"));
        assert!(composed.contains("active boulder state"));
        assert!(composed.contains("prefer continuation over starting a brand-new worker context"));
        assert!(composed.contains("Carry forward inherited notepad decisions"));
        assert!(composed.contains("Ground Truth Context"));
        assert!(composed.contains("task_build_42"));
        assert!(composed.contains("agent-task-42"));
    }
}
