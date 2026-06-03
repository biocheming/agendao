use super::profile::SchedulerProfileOrchestrator;
use super::profile_state::SchedulerProfileState;
use super::{SchedulerArtifactKind, SchedulerDraftArtifactInput, SchedulerPlanningArtifactInput};
use crate::scheduler::profile::SchedulerProfilePlan;
use crate::{OrchestratorContext, OrchestratorError};
use std::fs;
use std::path::Path;

impl SchedulerProfileOrchestrator {
    pub(super) fn artifact_path_mut(
        kind: SchedulerArtifactKind,
        state: &mut SchedulerProfileState,
    ) -> &mut Option<String> {
        match kind {
            SchedulerArtifactKind::Planning => &mut state.preset_runtime.planning_artifact_path,
            SchedulerArtifactKind::Draft => &mut state.preset_runtime.draft_artifact_path,
        }
    }

    pub(super) fn artifact_path_ref(
        kind: SchedulerArtifactKind,
        state: &SchedulerProfileState,
    ) -> &Option<String> {
        match kind {
            SchedulerArtifactKind::Planning => &state.preset_runtime.planning_artifact_path,
            SchedulerArtifactKind::Draft => &state.preset_runtime.draft_artifact_path,
        }
    }

    pub(super) fn resolve_artifact_relative_path(
        plan: &SchedulerProfilePlan,
        kind: SchedulerArtifactKind,
        session_id: &str,
    ) -> Option<String> {
        plan.artifact_relative_path(kind, session_id)
    }

    pub(super) fn ensure_artifact_path(
        plan: &SchedulerProfilePlan,
        kind: SchedulerArtifactKind,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Option<String> {
        if let Some(path) = Self::artifact_path_ref(kind, state).clone() {
            return Some(path);
        }

        let path = Self::resolve_artifact_relative_path(plan, kind, &ctx.exec_ctx.session_id)?;
        *Self::artifact_path_mut(kind, state) = Some(path.clone());
        Some(path)
    }

    pub(super) fn load_artifact_snapshot(
        plan: &SchedulerProfilePlan,
        kind: SchedulerArtifactKind,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Option<String> {
        let relative_path = Self::ensure_artifact_path(plan, kind, state, ctx)?;
        let absolute_path = Path::new(&ctx.exec_ctx.workdir).join(relative_path);
        fs::read_to_string(absolute_path)
            .ok()
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty())
    }

    pub(super) fn persist_artifact(
        plan: &SchedulerProfilePlan,
        kind: SchedulerArtifactKind,
        content: &str,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        let body = content.trim();
        if body.is_empty() {
            return Ok(());
        }

        let Some(relative_path) = Self::ensure_artifact_path(plan, kind, state, ctx) else {
            return Ok(());
        };
        plan.validate_runtime_artifact_path(&relative_path, &ctx.exec_ctx)
            .map_err(OrchestratorError::Other)?;
        let absolute_path = Path::new(&ctx.exec_ctx.workdir).join(&relative_path);

        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to create scheduler artifact directory `{}`: {err}",
                    parent.display()
                ))
            })?;
        }

        fs::write(&absolute_path, body).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to persist scheduler artifact `{}`: {err}",
                absolute_path.display()
            ))
        })?;

        Ok(())
    }

    pub(super) fn delete_artifact(
        plan: &SchedulerProfilePlan,
        kind: SchedulerArtifactKind,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Result<bool, OrchestratorError> {
        let Some(relative_path) = Self::artifact_path_ref(kind, state).clone() else {
            return Ok(false);
        };
        plan.validate_runtime_artifact_path(&relative_path, &ctx.exec_ctx)
            .map_err(OrchestratorError::Other)?;
        let absolute_path = Path::new(&ctx.exec_ctx.workdir).join(relative_path);
        if !absolute_path.exists() {
            if matches!(kind, SchedulerArtifactKind::Draft) {
                state.preset_runtime.draft_snapshot = None;
            }
            return Ok(false);
        }

        fs::remove_file(&absolute_path).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to delete scheduler artifact `{}`: {err}",
                absolute_path.display()
            ))
        })?;

        if matches!(kind, SchedulerArtifactKind::Draft) {
            state.preset_runtime.draft_snapshot = None;
        }

        Ok(true)
    }

    pub(super) fn ensure_planning_artifact_path(
        plan: &SchedulerProfilePlan,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Option<String> {
        Self::ensure_artifact_path(plan, SchedulerArtifactKind::Planning, state, ctx)
    }

    pub(super) fn persist_planning_artifact(
        plan: &SchedulerProfilePlan,
        content: &str,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        let normalized = plan
            .compose_planning_artifact(SchedulerPlanningArtifactInput {
                request_brief: &state.route.request_brief,
                route_summary: state
                    .route
                    .route_decision
                    .as_ref()
                    .map(|decision| decision.rationale_summary.as_str()),
                interview_output: state.route.interviewed.as_deref(),
                advisory_review: state.preset_runtime.advisory_review.as_deref(),
                planning_output: content,
                planning_artifact_path: state.preset_runtime.planning_artifact_path.as_deref(),
            })
            .unwrap_or_else(|| content.trim().to_string());
        if !normalized.is_empty() {
            state.preset_runtime.planned = Some(normalized.clone());
        }
        Self::persist_artifact(
            plan,
            SchedulerArtifactKind::Planning,
            &normalized,
            state,
            ctx,
        )
    }

    pub(super) fn render_runtime_draft_artifact(
        original_input: &str,
        plan: &SchedulerProfilePlan,
        state: &SchedulerProfileState,
    ) -> Option<String> {
        plan.compose_draft_artifact(SchedulerDraftArtifactInput {
            original_request: original_input,
            request_brief: &state.route.request_brief,
            route_summary: state
                .route
                .route_decision
                .as_ref()
                .map(|decision| decision.rationale_summary.as_str()),
            interview_output: state.route.interviewed.as_deref(),
            advisory_review: state.preset_runtime.advisory_review.as_deref(),
            current_plan: state.preset_runtime.planned.as_deref(),
            approval_review: state.preset_runtime.approval_review.as_deref(),
            user_choice: state.preset_runtime.user_choice.as_deref(),
            planning_artifact_path: state.preset_runtime.planning_artifact_path.as_deref(),
            draft_artifact_path: state.preset_runtime.draft_artifact_path.as_deref(),
        })
    }

    pub(super) fn sync_runtime_draft_artifact(
        original_input: &str,
        plan: &SchedulerProfilePlan,
        state: &mut SchedulerProfileState,
        ctx: &OrchestratorContext,
    ) -> Result<(), OrchestratorError> {
        let Some(draft) = Self::render_runtime_draft_artifact(original_input, plan, state) else {
            return Ok(());
        };
        Self::persist_artifact(plan, SchedulerArtifactKind::Draft, &draft, state, ctx)?;
        state.preset_runtime.draft_snapshot = Some(draft);
        Ok(())
    }
}
