use super::*;

pub(super) struct SchedulerEffectAdapter<'a, 'o> {
    pub(super) orchestrator: &'a SchedulerProfileOrchestrator,
    pub(super) original_input: &'a str,
    pub(super) state: &'a mut SchedulerProfileState,
    pub(super) plan: &'a SchedulerProfilePlan,
    pub(super) output: &'a mut Option<&'o mut OrchestratorOutput>,
    pub(super) ctx: &'a OrchestratorContext,
    pub(super) stage: SchedulerStageKind,
}

#[async_trait]
impl<'a, 'o> SchedulerPresetEffectExecutor for SchedulerEffectAdapter<'a, 'o> {
    async fn ensure_planning_artifact_path(&mut self) -> Result<(), OrchestratorError> {
        let _ = SchedulerProfileOrchestrator::ensure_planning_artifact_path(
            self.plan, self.state, self.ctx,
        );
        Ok(())
    }

    async fn persist_planning_artifact(&mut self) -> Result<(), OrchestratorError> {
        if let Some(output) = self.output.as_ref() {
            SchedulerProfileOrchestrator::persist_planning_artifact(
                self.plan,
                &output.content,
                self.state,
                self.ctx,
            )?;
        }
        Ok(())
    }

    async fn sync_draft_artifact(&mut self) -> Result<(), OrchestratorError> {
        if let Err(error) = SchedulerProfileOrchestrator::sync_runtime_draft_artifact(
            self.original_input,
            self.plan,
            self.state,
            self.ctx,
        ) {
            tracing::warn!(error = %error, stage = self.stage.as_event_name(), "scheduler effect failed to sync runtime draft artifact");
        }
        Ok(())
    }

    async fn register_workflow_todos(&mut self) -> Result<(), OrchestratorError> {
        self.orchestrator
            .register_scheduler_workflow_todos(self.state, self.plan, self.ctx)
            .await;
        Ok(())
    }

    async fn request_advisory_review(&mut self) -> Result<(), OrchestratorError> {
        self.orchestrator
            .request_capability_advisory_review(
                self.original_input,
                self.state,
                self.plan,
                self.ctx,
            )
            .await;
        Ok(())
    }

    async fn request_user_choice(&mut self) -> Result<(), OrchestratorError> {
        let choice = self
            .orchestrator
            .request_capability_user_choice(self.state, self.plan, self.ctx)
            .await;
        if let Some(update) = self.plan.runtime_update_for_user_choice(choice) {
            self.state.apply_runtime_update(update);
        }
        Ok(())
    }

    async fn run_approval_review_loop(&mut self) -> Result<(), OrchestratorError> {
        let approved = self
            .orchestrator
            .run_capability_approval_review_loop(
                self.original_input,
                self.state,
                self.plan,
                self.ctx,
            )
            .await;
        if let Some(update) = self.plan.runtime_update_for_review_gate(approved) {
            self.state.apply_runtime_update(update);
        }
        Ok(())
    }

    async fn delete_draft_artifact(&mut self) -> Result<(), OrchestratorError> {
        let _ = SchedulerProfileOrchestrator::delete_artifact(
            self.plan,
            SchedulerArtifactKind::Draft,
            self.state,
            self.ctx,
        )?;
        Ok(())
    }

    async fn decorate_final_output(
        &mut self,
        decoration: SchedulerHandoffDecoration,
    ) -> Result<(), OrchestratorError> {
        if let Some(output) = self.output.as_deref_mut() {
            output.content = self
                .plan
                .decorate_final_output(output.content.clone(), decoration);
        }
        Ok(())
    }
}

pub(super) struct SchedulerExecutionStageAdapter<'a> {
    pub(super) orchestrator: &'a SchedulerProfileOrchestrator,
    pub(super) original_input: &'a str,
    pub(super) state: &'a mut SchedulerProfileState,
    pub(super) plan: &'a SchedulerProfilePlan,
    pub(super) ctx: &'a OrchestratorContext,
}

#[async_trait]
impl<'a> SchedulerPresetExecutionStageExecutor for SchedulerExecutionStageAdapter<'a> {
    async fn execute_direct_stage(&mut self) -> Result<(), OrchestratorError> {
        self.orchestrator
            .execute_direct_execution_workflow(self.original_input, self.state, self.plan, self.ctx)
            .await
    }

    async fn execute_single_pass_stage(&mut self) -> Result<(), OrchestratorError> {
        self.orchestrator
            .execute_single_pass_execution_workflow(
                self.original_input,
                self.state,
                self.plan,
                self.ctx,
            )
            .await
    }

    async fn execute_coordination_loop_stage(&mut self) -> Result<(), OrchestratorError> {
        self.orchestrator
            .execute_coordination_execution_workflow(
                self.original_input,
                self.state,
                self.plan,
                self.ctx,
            )
            .await
    }

    async fn execute_autonomous_loop_stage(&mut self) -> Result<(), OrchestratorError> {
        self.orchestrator
            .execute_autonomous_execution_workflow(
                self.original_input,
                self.state,
                self.plan,
                self.ctx,
            )
            .await
    }
}
