use crate::scheduler::prompt_context::{AvailableAgentMeta, AvailableCategoryMeta};
use crate::scheduler::SchedulerSkillRef;

pub struct SchedulerDraftArtifactInput<'a> {
    pub original_request: &'a str,
    pub request_brief: &'a str,
    pub route_summary: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub current_plan: Option<&'a str>,
    pub approval_review: Option<&'a str>,
    pub user_choice: Option<&'a str>,
    pub planning_artifact_path: Option<&'a str>,
    pub draft_artifact_path: Option<&'a str>,
}

pub struct SchedulerPlanningArtifactInput<'a> {
    pub request_brief: &'a str,
    pub route_summary: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub planning_output: &'a str,
    pub planning_artifact_path: Option<&'a str>,
}

pub struct SchedulerInterviewStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub route_decision_json: Option<&'a str>,
    pub draft_artifact_path: Option<&'a str>,
    pub draft_context: Option<&'a str>,
    pub current_plan: &'a str,
    pub skill_tree_context: Option<&'a str>,
}

pub struct SchedulerPlanStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub route_decision_json: Option<&'a str>,
    pub route_output: Option<&'a str>,
    pub planning_artifact_path: Option<&'a str>,
    pub draft_artifact_path: Option<&'a str>,
    pub draft_context: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub approval_feedback: Option<&'a str>,
    pub current_plan: &'a str,
    pub skill_tree_context: Option<&'a str>,
    pub available_agents: &'a [AvailableAgentMeta],
    pub available_categories: &'a [AvailableCategoryMeta],
    pub skill_list: &'a [SchedulerSkillRef],
}

pub struct SchedulerExecutionOrchestrationStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub route_summary: Option<&'a str>,
    pub planning_output: Option<&'a str>,
    pub ground_truth_context: Option<&'a str>,
    pub skill_tree_context: Option<&'a str>,
    pub available_agents: &'a [AvailableAgentMeta],
    pub available_categories: &'a [AvailableCategoryMeta],
    pub skill_list: &'a [SchedulerSkillRef],
}

pub struct SchedulerSynthesisStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub current_plan: &'a str,
    pub route_decision_json: Option<&'a str>,
    pub planning_output: Option<&'a str>,
    pub delegation_output: Option<&'a str>,
    pub review_output: Option<&'a str>,
    pub saved_planning_artifact: Option<&'a str>,
}

pub struct SchedulerCoordinationVerificationStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub round: usize,
    pub execution_output: &'a str,
    pub planning_output: Option<&'a str>,
    pub ground_truth_context: Option<&'a str>,
    pub skill_tree_context: Option<&'a str>,
}

pub struct SchedulerAutonomousVerificationStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub current_plan: &'a str,
    pub round: usize,
    pub execution_output: &'a str,
}

pub struct SchedulerCoordinationGateStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub current_plan: &'a str,
    pub round: usize,
    pub execution_output: &'a str,
    pub verification_output: Option<&'a str>,
    pub ground_truth_context: Option<&'a str>,
}

pub struct SchedulerAutonomousGateStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub current_plan: &'a str,
    pub round: usize,
    pub execution_output: &'a str,
    pub verification_output: Option<&'a str>,
}

pub struct SchedulerRetryStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub current_plan: &'a str,
    pub round: usize,
    pub previous_output: &'a str,
    pub verification_output: Option<&'a str>,
    pub retry_summary: &'a str,
    pub next_input: Option<&'a str>,
    pub ground_truth_context: Option<&'a str>,
    pub preferred_continuation_session_id: Option<&'a str>,
    pub preferred_continuation_agent_task_id: Option<&'a str>,
    pub continuation_candidates: Option<&'a str>,
}

pub struct SchedulerReviewStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub route_summary: Option<&'a str>,
    pub draft_context: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub execution_plan: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub approval_feedback: Option<&'a str>,
    pub saved_planning_artifact: Option<&'a str>,
    pub skills_index_markdown: Option<&'a str>,
    pub delegation_output: Option<&'a str>,
}

pub struct SchedulerHandoffStageInput<'a> {
    pub original_request: &'a str,
    pub session_context: Option<&'a str>,
    pub request_brief: &'a str,
    pub current_plan: &'a str,
    pub draft_context: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub planning_output: Option<&'a str>,
    pub review_output: Option<&'a str>,
    pub approval_review: Option<&'a str>,
    pub user_choice: Option<&'a str>,
    pub saved_planning_artifact: Option<&'a str>,
}

pub struct SchedulerAdvisoryReviewInput<'a> {
    pub goal: &'a str,
    pub original_request: &'a str,
    pub discussed: Option<&'a str>,
    pub draft_context: Option<&'a str>,
    pub research: Option<&'a str>,
}
