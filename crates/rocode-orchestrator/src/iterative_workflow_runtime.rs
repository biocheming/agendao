use crate::iterative_workflow::{
    BaselineStrategy, CommandDefinition, DecisionPolicyDefinition, DiscardCondition, IterationMode,
    IterationPolicyDefinition, IterativeWorkflowConfig, IterativeWorkflowKind, KeepCondition,
    MetricKind, ObjectiveDefinition, ObjectiveDirection, SnapshotStrategy,
};
use crate::tool_runner::{ToolCallInput, ToolRunner};
use crate::types::{ExecutionContext, OrchestratorOutput};
use crate::workflow_artifacts::{
    workflow_run_dir, WorkflowArtifactWriter, WorkflowBaselineRecord, WorkflowCommandArtifact,
    WorkflowIterationRecord, WorkflowModeArtifact, WorkflowRunSummaryRecord,
};
use crate::workflow_mode::{
    mode_protocol_for, ModeFinalizeContext, ModeIterationContext, WorkflowModeProtocol,
};
use crate::workflow_workspace::WorkflowWorkspaceService;
use crate::{OrchestratorError, SchedulerExecutionGateDecision, SchedulerExecutionGateStatus};
use glob::Pattern;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const METRIC_EPSILON: f64 = 1e-9;

pub struct WorkflowController {
    config: IterativeWorkflowConfig,
    objective: ObjectiveDefinition,
    runner: VerificationRunner,
    evaluator: ObjectiveEvaluator,
    policy: DecisionPolicy,
    artifacts: WorkflowArtifactWriter,
    mode_protocol: Box<dyn WorkflowModeProtocol>,
    mode_iteration_notes: Vec<String>,
    pending_iteration_brief: Option<String>,
    workspace: WorkflowWorkspaceService,
    snapshot_engine: SnapshotEngine,
    active_checkpoint: Option<WorkspaceCheckpoint>,
    crash_retry_attempts: u32,
    rework_attempts: u32,
    stuck_counter: u32,
    pending_tool_calls: u32,
}

impl WorkflowController {
    pub fn from_config(
        config: IterativeWorkflowConfig,
        tool_runner: ToolRunner,
        exec_ctx: ExecutionContext,
        profile_name: Option<String>,
    ) -> Result<Option<Self>, OrchestratorError> {
        if !Self::supports(&config) {
            return Ok(None);
        }

        let objective = config.objective.clone().ok_or_else(|| {
            OrchestratorError::Other("workflow objective is required".to_string())
        })?;
        let policy = config.decision_policy.clone().ok_or_else(|| {
            OrchestratorError::Other("workflow decisionPolicy is required".to_string())
        })?;

        Ok(Some(Self {
            objective: objective.clone(),
            runner: VerificationRunner::new(tool_runner, exec_ctx.clone()),
            evaluator: ObjectiveEvaluator::new(&objective)?,
            policy: DecisionPolicy::new(policy, config.iteration_policy.clone()),
            artifacts: WorkflowArtifactWriter::new(&config, &exec_ctx, profile_name.as_deref())?,
            mode_protocol: mode_protocol_for(&config),
            mode_iteration_notes: Vec::new(),
            pending_iteration_brief: None,
            workspace: WorkflowWorkspaceService::new(&config, &objective, &exec_ctx)?,
            snapshot_engine: SnapshotEngine::new(&config, &objective, &exec_ctx)?,
            active_checkpoint: None,
            crash_retry_attempts: 0,
            config,
            rework_attempts: 0,
            stuck_counter: 0,
            pending_tool_calls: 0,
        }))
    }

    pub fn supports(config: &IterativeWorkflowConfig) -> bool {
        matches!(config.workflow.kind, IterativeWorkflowKind::Autoresearch)
            && config.objective.is_some()
            && config.decision_policy.is_some()
    }

    pub async fn capture_baseline(&mut self) -> Result<(), OrchestratorError> {
        let objective = self.objective().clone();
        let baseline_strategy = self
            .config
            .decision_policy
            .as_ref()
            .and_then(|policy| policy.baseline_strategy)
            .unwrap_or(BaselineStrategy::CaptureBeforeFirstIteration);

        match baseline_strategy {
            BaselineStrategy::CaptureBeforeFirstIteration => {
                let verify = self
                    .runner
                    .run_command("baseline-verify", 0, &objective.verify, None)
                    .await?;
                self.pending_tool_calls += 1;
                self.evaluator
                    .capture_baseline(verify.clone(), &objective)?;
                self.artifacts.record_baseline(&WorkflowBaselineRecord {
                    source: "capture-before-first-iteration".to_string(),
                    value: self
                        .evaluator
                        .history()
                        .baseline
                        .as_ref()
                        .map(|sample| sample.value),
                    summary: "baseline captured from verify command".to_string(),
                    verify: Some(command_artifact(&verify)),
                })?;
            }
            BaselineStrategy::FromConfig => {
                let baseline_value = self
                    .config
                    .decision_policy
                    .as_ref()
                    .and_then(|policy| policy.baseline_value)
                    .ok_or_else(|| {
                        OrchestratorError::Other(
                            "workflow baselineStrategy 'from-config' requires baselineValue"
                                .to_string(),
                        )
                    })?;
                self.evaluator.capture_configured_baseline(baseline_value);
                self.artifacts.record_baseline(&WorkflowBaselineRecord {
                    source: "from-config".to_string(),
                    value: Some(baseline_value),
                    summary: "baseline loaded from workflow configuration".to_string(),
                    verify: None,
                })?;
            }
            BaselineStrategy::FromLastRun => {
                let manifest = self
                    .artifacts
                    .read_last_run_manifest()?
                    .ok_or_else(|| {
                        OrchestratorError::Other(
                            "workflow baselineStrategy 'from-last-run' could not find a matching prior run"
                                .to_string(),
                        )
                    })?;
                let baseline_value = manifest.best_metric.ok_or_else(|| {
                    OrchestratorError::Other(
                        "workflow baselineStrategy 'from-last-run' found a prior run without best_metric"
                            .to_string(),
                    )
                })?;
                self.evaluator.capture_configured_baseline(baseline_value);
                self.artifacts.record_baseline(&WorkflowBaselineRecord {
                    source: "from-last-run".to_string(),
                    value: Some(baseline_value),
                    summary: format!(
                        "baseline loaded from prior run '{}' (session {})",
                        manifest.objective_fingerprint, manifest.session_id
                    ),
                    verify: None,
                })?;
            }
        }

        Ok(())
    }

    pub fn begin_iteration(&mut self, iteration: u32) -> Result<(), OrchestratorError> {
        if self.active_checkpoint.is_some() {
            return Ok(());
        }

        self.active_checkpoint = Some(self.snapshot_engine.capture(iteration)?);
        self.pending_iteration_brief = self.mode_protocol.iteration_brief(iteration);
        if let Some(note) = self.pending_iteration_brief.as_ref() {
            self.mode_iteration_notes.push(note.clone());
        }
        Ok(())
    }

    pub fn abort_iteration(&mut self, reason: &str) -> Result<(), OrchestratorError> {
        if self.active_checkpoint.is_none() {
            return Ok(());
        }

        self.restore_active_checkpoint()
            .map_err(|err| OrchestratorError::Other(format!("{reason}: {err}")))
    }

    pub fn orphan_active_checkpoint(&mut self) -> Result<(), OrchestratorError> {
        if let Some(checkpoint) = self.active_checkpoint.as_mut() {
            self.snapshot_engine.orphan(checkpoint)?;
        }
        Ok(())
    }

    pub fn execution_context_override(
        &self,
        base_exec_ctx: &ExecutionContext,
    ) -> Option<ExecutionContext> {
        let mut exec_ctx = base_exec_ctx.clone();

        if let Some(workdir) = self
            .active_checkpoint
            .as_ref()
            .and_then(WorkspaceCheckpoint::execution_workdir)
        {
            exec_ctx.workdir = workdir.display().to_string();
        }

        if let Some(checkpoint) = self.active_checkpoint.as_ref() {
            exec_ctx.metadata.insert(
                "workflow_checkpoint_id".to_string(),
                json!(checkpoint.checkpoint_id.as_str()),
            );
        }

        exec_ctx.metadata.insert(
            "workflow_mode".to_string(),
            json!(self.mode_protocol.mode()),
        );

        if let Some(note) = self.pending_iteration_brief.as_deref() {
            exec_ctx
                .metadata
                .insert("workflow_mode_iteration_brief".to_string(), json!(note));
        }

        Some(exec_ctx)
    }

    pub fn handle_execution_error(
        &mut self,
        iteration: u32,
        error: &OrchestratorError,
    ) -> WorkflowGateResult {
        let mut decision = self
            .policy
            .decide_execution_error(error, self.crash_retry_attempts);
        if let Err(restore_err) = self.apply_checkpoint_outcome(&decision) {
            let reason = format!("snapshot action failed during crash recovery: {restore_err}");
            let _ = self.orphan_active_checkpoint();
            decision = IterationDecision::StopBlocked { reason };
        }
        let gate_decision = self.map_crash_gate_decision(iteration, &decision);
        self.apply_iteration_outcome(&decision);
        self.pending_tool_calls = 0;
        let gate_output =
            self.compose_crash_gate_output(iteration, &decision, error, &gate_decision);
        let _ = self.artifacts.append_iteration(&WorkflowIterationRecord {
            iteration,
            phase: "crash".to_string(),
            decision: decision.label().to_string(),
            gate_status: gate_status_label(gate_decision.status).to_string(),
            commit_sha: None,
            metric_value: None,
            baseline_value: self
                .evaluator
                .history()
                .baseline
                .as_ref()
                .map(|sample| sample.value),
            delta_from_baseline: None,
            verify: None,
            guard: None,
            summary: gate_decision.summary.clone(),
        });

        WorkflowGateResult {
            decision: decision.clone(),
            gate_decision: gate_decision.clone(),
            output: gate_output,
        }
    }

    pub(crate) async fn evaluate_round(
        &mut self,
        iteration: u32,
        structured_artifacts: Vec<WorkflowModeArtifact>,
    ) -> Result<WorkflowGateResult, OrchestratorError> {
        let objective = self.objective().clone();
        let workdir_override = self
            .active_checkpoint
            .as_ref()
            .and_then(WorkspaceCheckpoint::execution_workdir);
        let verify = self
            .runner
            .run_command("verify", iteration, &objective.verify, workdir_override)
            .await?;
        let guard = match objective.guard.as_ref() {
            Some(command) => Some(
                self.runner
                    .run_command("guard", iteration, command, workdir_override)
                    .await?,
            ),
            None => None,
        };
        let tool_calls = 1 + u32::from(guard.is_some()) + self.pending_tool_calls;
        let evaluation = self
            .evaluator
            .evaluate_iteration(iteration, &objective, verify, guard)?;

        let mut decision =
            self.policy
                .decide(&evaluation, self.rework_attempts, self.stuck_counter);

        if matches!(decision, IterationDecision::Discard { .. }) {
            let next_stuck = self.stuck_counter + 1;
            if self.policy.stuck_threshold_reached(next_stuck) {
                decision = IterationDecision::StopStalled;
            }
        }

        if let Err(err) = self.apply_checkpoint_outcome(&decision) {
            let reason = format!("snapshot action failed: {err}");
            let _ = self.orphan_active_checkpoint();
            decision = IterationDecision::StopBlocked { reason };
        }

        let mut gate_decision = self.map_to_gate_decision(iteration, &decision, &evaluation);
        self.apply_mode_gate_annotation(
            iteration,
            &decision,
            &evaluation,
            structured_artifacts,
            &mut gate_decision,
        );
        let workspace_update = match decision {
            IterationDecision::Keep | IterationDecision::StopSatisfied => self
                .workspace
                .record_kept_iteration(iteration, decision.label(), &gate_decision.summary)?,
            _ => self.workspace.snapshot(),
        };
        self.apply_iteration_outcome(&decision);
        self.pending_tool_calls = 0;
        let output = self.compose_gate_output(
            iteration,
            tool_calls,
            &decision,
            &evaluation,
            &gate_decision,
        );
        self.artifacts.append_iteration(&WorkflowIterationRecord {
            iteration,
            phase: "iteration".to_string(),
            decision: decision.label().to_string(),
            gate_status: gate_status_label(gate_decision.status).to_string(),
            commit_sha: workspace_update
                .commit
                .as_ref()
                .map(|commit| commit.commit_sha.clone()),
            metric_value: evaluation.metric.as_ref().map(|sample| sample.value),
            baseline_value: evaluation.baseline_value,
            delta_from_baseline: evaluation
                .metric
                .as_ref()
                .zip(evaluation.baseline_value)
                .map(|(metric, baseline)| metric.value - baseline),
            verify: Some(command_artifact(&evaluation.verify)),
            guard: evaluation.guard.as_ref().map(command_artifact),
            summary: gate_decision.summary.clone(),
        })?;

        Ok(WorkflowGateResult {
            decision: decision.clone(),
            gate_decision: gate_decision.clone(),
            output,
        })
    }

    fn objective(&self) -> &ObjectiveDefinition {
        &self.objective
    }

    fn apply_mode_gate_annotation(
        &mut self,
        iteration: u32,
        decision: &IterationDecision,
        evaluation: &ObjectiveEvaluation,
        structured_artifacts: Vec<WorkflowModeArtifact>,
        gate_decision: &mut SchedulerExecutionGateDecision,
    ) {
        let ctx = ModeIterationContext {
            iteration,
            decision: decision.label().to_string(),
            gate_status: gate_status_label(gate_decision.status).to_string(),
            objective_satisfied: evaluation.objective_satisfied,
            metric_value: evaluation.metric.as_ref().map(|sample| sample.value),
            verify_passed: evaluation.verify.passed(),
            guard_passed: evaluation.guard.as_ref().map(CommandRunResult::passed),
            structured_artifacts,
        };
        let annotation = self.mode_protocol.annotate_gate(&ctx);
        if let Some(suffix) = annotation.summary_suffix.as_deref() {
            let trimmed = suffix.trim();
            if !trimmed.is_empty() {
                gate_decision.summary = format!("{} {}", gate_decision.summary.trim(), trimmed)
                    .trim()
                    .to_string();
            }
        }
        if let Some(prefix) = annotation.next_input_prefix.as_deref() {
            let trimmed = prefix.trim();
            if !trimmed.is_empty() {
                match gate_decision.next_input.as_mut() {
                    Some(next_input) if !next_input.trim().is_empty() => {
                        *next_input = format!("{trimmed}\n\n{}", next_input.trim());
                    }
                    _ if gate_decision.status == SchedulerExecutionGateStatus::Continue => {
                        gate_decision.next_input = Some(trimmed.to_string());
                    }
                    _ => {}
                }
            }
        }
        if let Some(note) = annotation.iteration_note {
            let trimmed = note.trim();
            if !trimmed.is_empty() {
                self.mode_iteration_notes.push(trimmed.to_string());
            }
        }
        self.mode_protocol.record_iteration(&ctx);
    }

    pub(crate) fn persist_run_summary(
        &mut self,
        mut summary: WorkflowRunSummaryRecord,
    ) -> Result<(), OrchestratorError> {
        let workspace_update = self.workspace.finalize(
            summary.final_iteration,
            summary.final_decision.as_deref(),
            summary.final_summary.as_deref(),
        )?;
        summary.baseline_metric = self
            .evaluator
            .history()
            .baseline
            .as_ref()
            .map(|sample| sample.value);
        summary.best_metric = self
            .evaluator
            .history()
            .best
            .as_ref()
            .map(|sample| sample.value);
        summary.final_metric = self
            .evaluator
            .history()
            .current
            .as_ref()
            .map(|sample| sample.value);
        let best_iteration = self
            .evaluator
            .history()
            .best
            .as_ref()
            .map(|sample| sample.iteration);
        summary.best_commit = best_iteration.and_then(|iteration| {
            workspace_update
                .kept_commits
                .iter()
                .find(|commit| commit.iteration == iteration)
                .cloned()
        });
        summary.kept_commits = workspace_update.kept_commits;
        summary.squashed_commit = workspace_update.squashed_commit;
        summary.mode_report = Some(self.mode_protocol.finalize_report(
            &ModeFinalizeContext {
                iterations_completed: summary.iterations_completed,
                objective_satisfied: summary.objective_satisfied,
                final_decision: summary.final_decision.clone(),
            },
            &self.mode_iteration_notes,
        ));
        summary.mode_artifacts = self.mode_protocol.export_artifacts();
        self.artifacts.write_summary(&summary)?;
        Ok(())
    }

    fn apply_checkpoint_outcome(
        &mut self,
        decision: &IterationDecision,
    ) -> Result<(), OrchestratorError> {
        match decision {
            IterationDecision::Keep | IterationDecision::StopSatisfied => {
                self.release_active_checkpoint()
            }
            IterationDecision::Discard { .. }
            | IterationDecision::RetryCrash { .. }
            | IterationDecision::StopStalled
            | IterationDecision::StopBlocked { .. } => self.restore_active_checkpoint(),
            IterationDecision::Rework { .. } => Ok(()),
        }
    }

    fn restore_active_checkpoint(&mut self) -> Result<(), OrchestratorError> {
        let Some(mut checkpoint) = self.active_checkpoint.take() else {
            return Err(OrchestratorError::Other(
                "no active checkpoint available to restore".to_string(),
            ));
        };
        self.snapshot_engine.restore(&mut checkpoint)
    }

    fn release_active_checkpoint(&mut self) -> Result<(), OrchestratorError> {
        let Some(mut checkpoint) = self.active_checkpoint.take() else {
            return Ok(());
        };
        self.snapshot_engine.release(&mut checkpoint)
    }

    fn apply_iteration_outcome(&mut self, decision: &IterationDecision) {
        match decision {
            IterationDecision::Keep => {
                self.crash_retry_attempts = 0;
                self.rework_attempts = 0;
                self.stuck_counter = 0;
            }
            IterationDecision::Discard { .. } => {
                self.crash_retry_attempts = 0;
                self.rework_attempts = 0;
                self.stuck_counter += 1;
            }
            IterationDecision::Rework { attempt, .. } => {
                self.rework_attempts = *attempt;
            }
            IterationDecision::RetryCrash { attempt, .. } => {
                self.crash_retry_attempts = *attempt;
            }
            IterationDecision::StopSatisfied
            | IterationDecision::StopStalled
            | IterationDecision::StopBlocked { .. } => {
                self.crash_retry_attempts = 0;
                self.rework_attempts = 0;
            }
        }
    }

    fn map_to_gate_decision(
        &self,
        iteration: u32,
        decision: &IterationDecision,
        evaluation: &ObjectiveEvaluation,
    ) -> SchedulerExecutionGateDecision {
        let mut gate = match decision {
            IterationDecision::Keep => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Continue,
                summary: self.keep_summary(evaluation),
                next_input: Some(self.keep_next_input(evaluation)),
                final_response: None,
            },
            IterationDecision::Discard { reason } => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Continue,
                summary: format!("iteration discarded: {}", reason.as_str()),
                next_input: Some(self.discard_next_input(reason, evaluation)),
                final_response: None,
            },
            IterationDecision::Rework {
                attempt,
                guard_output,
            } => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Continue,
                summary: format!("guard failed; rework attempt {attempt}"),
                next_input: Some(self.rework_next_input(*attempt, guard_output)),
                final_response: None,
            },
            IterationDecision::RetryCrash { attempt, error } => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Continue,
                summary: format!("crash recovery attempt {attempt}"),
                next_input: Some(format!(
                    "Address the runtime failure before the next attempt: {}",
                    first_meaningful_line(error)
                )),
                final_response: None,
            },
            IterationDecision::StopSatisfied => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Done,
                summary: "objective satisfied".to_string(),
                next_input: None,
                final_response: Some(self.done_response(evaluation)),
            },
            IterationDecision::StopStalled => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Blocked,
                summary: "progress stalled".to_string(),
                next_input: None,
                final_response: Some(self.blocked_response(
                    "progress stalled before the objective was satisfied",
                    evaluation,
                )),
            },
            IterationDecision::StopBlocked { reason } => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Blocked,
                summary: format!("blocked: {reason}"),
                next_input: None,
                final_response: Some(self.blocked_response(reason, evaluation)),
            },
        };

        if gate.status == SchedulerExecutionGateStatus::Continue
            && self.policy.max_iterations_reached(iteration)
        {
            gate.status = SchedulerExecutionGateStatus::Done;
            gate.summary = format!("max iterations reached at {iteration}");
            gate.next_input = None;
            gate.final_response = Some(self.done_response(evaluation));
        }

        gate
    }

    fn map_crash_gate_decision(
        &self,
        iteration: u32,
        decision: &IterationDecision,
    ) -> SchedulerExecutionGateDecision {
        let synthetic = ObjectiveEvaluation::synthetic_execution_failure();
        let mut gate = match decision {
            IterationDecision::RetryCrash { attempt, error } => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Continue,
                summary: format!("crash recovery attempt {attempt}"),
                next_input: Some(format!(
                    "The previous execution round crashed. Repair that failure first: {}",
                    first_meaningful_line(error)
                )),
                final_response: None,
            },
            IterationDecision::StopBlocked { reason } => SchedulerExecutionGateDecision {
                status: SchedulerExecutionGateStatus::Blocked,
                summary: format!("blocked: {reason}"),
                next_input: None,
                final_response: Some(self.blocked_response(reason, &synthetic)),
            },
            other => self.map_to_gate_decision(iteration, other, &synthetic),
        };

        if gate.status == SchedulerExecutionGateStatus::Continue
            && self.policy.max_iterations_reached(iteration)
        {
            gate.status = SchedulerExecutionGateStatus::Blocked;
            gate.summary = format!("max iterations reached during crash recovery at {iteration}");
            gate.next_input = None;
            gate.final_response = Some(
                self.blocked_response("crash recovery exhausted the iteration budget", &synthetic),
            );
        }

        gate
    }

    fn keep_summary(&self, evaluation: &ObjectiveEvaluation) -> String {
        if let Some(metric) = &evaluation.metric {
            let baseline = self
                .evaluator
                .history()
                .baseline
                .as_ref()
                .map(|sample| format!("{:.4}", sample.value))
                .unwrap_or_else(|| "n/a".to_string());
            format!(
                "iteration kept; metric is {:.4} (baseline {baseline})",
                metric.value
            )
        } else {
            "iteration kept; verification passed".to_string()
        }
    }

    fn keep_next_input(&self, evaluation: &ObjectiveEvaluation) -> String {
        if let Some(metric) = &evaluation.metric {
            format!(
                "Continue improving the objective from the current best metric {:.4}. Preserve passing verification and avoid undoing the validated change.",
                metric.value
            )
        } else {
            "Continue improving the objective while preserving the passing verification state."
                .to_string()
        }
    }

    fn discard_next_input(
        &self,
        reason: &DiscardReason,
        evaluation: &ObjectiveEvaluation,
    ) -> String {
        let metric_note = evaluation
            .metric
            .as_ref()
            .map(|sample| format!("Current metric {:.4}. ", sample.value))
            .unwrap_or_default();
        format!(
            "{}Retry with a narrower change-set focused on avoiding {}.",
            metric_note,
            reason.as_str()
        )
    }

    fn rework_next_input(&self, attempt: u32, guard_output: &str) -> String {
        format!(
            "Rework attempt {attempt}. Fix the guard failure first: {}",
            first_meaningful_line(guard_output)
        )
    }

    fn done_response(&self, evaluation: &ObjectiveEvaluation) -> String {
        let metric_line = evaluation
            .metric
            .as_ref()
            .map(|sample| format!("- Final metric: {:.4}", sample.value))
            .unwrap_or_else(|| "- Final metric: not extracted".to_string());
        format!(
            "## Delivery Summary\nObjective satisfied.\n\n**Completion Status**\n- The configured objective threshold is satisfied.\n\n**What Changed**\n- The autonomous loop kept the current workspace state as the best verified candidate.\n\n**Verification**\n{}\n{}\n\n**Risks or Follow-ups**\n- `patch-file` checkpoint restore is active. Other snapshot strategies are still pending runtime support.",
            self.verify_bullet(evaluation),
            metric_line
        )
    }

    fn blocked_response(&self, reason: &str, evaluation: &ObjectiveEvaluation) -> String {
        format!(
            "## Delivery Summary\nWorkflow blocked.\n\n**Completion Status**\n- {reason}\n\n**What Changed**\n- The workflow stopped before it could produce another kept iteration.\n\n**Verification**\n{}\n\n**Risks or Follow-ups**\n- Check the checkpoint and snapshot strategy support if the block came from workspace restore or release.",
            self.verify_bullet(evaluation)
        )
    }

    fn verify_bullet(&self, evaluation: &ObjectiveEvaluation) -> String {
        let verify = &evaluation.verify;
        let exit_code = verify.exit_code.unwrap_or(-1);
        if verify.timed_out {
            "- Verify command timed out.".to_string()
        } else if let Some(error) = &verify.runtime_error {
            format!(
                "- Verify command failed to execute: {}",
                first_meaningful_line(error)
            )
        } else {
            format!(
                "- Verify exit code: {exit_code}. {}",
                first_meaningful_line(&verify.output)
            )
        }
    }

    fn compose_gate_output(
        &self,
        iteration: u32,
        tool_calls: u32,
        decision: &IterationDecision,
        evaluation: &ObjectiveEvaluation,
        gate_decision: &SchedulerExecutionGateDecision,
    ) -> OrchestratorOutput {
        let metric_section = evaluation
            .metric
            .as_ref()
            .map(|sample| {
                format!(
                    "- current: {:.4}\n- baseline: {}\n- improved over baseline: {}\n- new best: {}",
                    sample.value,
                    self.evaluator
                        .history()
                        .baseline
                        .as_ref()
                        .map(|baseline| format!("{:.4}", baseline.value))
                        .unwrap_or_else(|| "n/a".to_string()),
                    yes_no(evaluation.improved_over_baseline),
                    yes_no(evaluation.improved_over_best),
                )
            })
            .unwrap_or_else(|| "- metric unavailable".to_string());
        let guard_section = evaluation
            .guard
            .as_ref()
            .map(|guard| {
                format!(
                    "- exit code: {}\n- {}",
                    guard.exit_code.unwrap_or(-1),
                    first_meaningful_line(&guard.output)
                )
            })
            .unwrap_or_else(|| "- not configured".to_string());

        OrchestratorOutput {
            content: format!(
                "## Workflow Gate\n- Iteration: {iteration}\n- Domain Decision: {}\n- Scheduler Projection: {}\n\n**Summary**\n- {}\n\n**Metric**\n{}\n\n**Verification**\n{}\n\n**Guard**\n{}",
                decision.label(),
                gate_status_label(gate_decision.status),
                gate_decision.summary,
                metric_section,
                self.verify_bullet(evaluation),
                guard_section,
            ),
            steps: 0,
            tool_calls_count: tool_calls,
            metadata: Default::default(),
            finish_reason: crate::runtime::events::FinishReason::EndTurn,
        }
    }

    fn compose_crash_gate_output(
        &self,
        iteration: u32,
        decision: &IterationDecision,
        error: &OrchestratorError,
        gate_decision: &SchedulerExecutionGateDecision,
    ) -> OrchestratorOutput {
        OrchestratorOutput {
            content: format!(
                "## Workflow Gate\n- Iteration: {iteration}\n- Domain Decision: {}\n- Scheduler Projection: {}\n\n**Summary**\n- {}\n\n**Crash Context**\n- {}",
                decision.label(),
                gate_status_label(gate_decision.status),
                gate_decision.summary,
                first_meaningful_line(&error.to_string()),
            ),
            steps: 0,
            tool_calls_count: 0,
            metadata: Default::default(),
            finish_reason: crate::runtime::events::FinishReason::EndTurn,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowGateResult {
    pub decision: IterationDecision,
    pub gate_decision: SchedulerExecutionGateDecision,
    pub output: OrchestratorOutput,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IterationDecision {
    Keep,
    Discard { reason: DiscardReason },
    Rework { attempt: u32, guard_output: String },
    RetryCrash { attempt: u32, error: String },
    StopSatisfied,
    StopStalled,
    StopBlocked { reason: String },
}

impl IterationDecision {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Discard { .. } => "discard",
            Self::Rework { .. } => "rework",
            Self::RetryCrash { .. } => "retry-crash",
            Self::StopSatisfied => "stop-satisfied",
            Self::StopStalled => "stop-stalled",
            Self::StopBlocked { .. } => "stop-blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscardReason {
    MetricRegressed,
    MetricUnchanged,
    VerifyFailed,
    VerifyTimeout,
    GuardFailedAfterRework,
    CrashUnrecoverable,
    SimplicityOverride,
}

impl DiscardReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::MetricRegressed => "metric-regressed",
            Self::MetricUnchanged => "metric-unchanged",
            Self::VerifyFailed => "verify-failed",
            Self::VerifyTimeout => "verify-timeout",
            Self::GuardFailedAfterRework => "guard-failed-after-rework",
            Self::CrashUnrecoverable => "crash-unrecoverable",
            Self::SimplicityOverride => "simplicity-override",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObjectiveEvaluator {
    history: MetricHistory,
}

impl ObjectiveEvaluator {
    pub fn new(objective: &ObjectiveDefinition) -> Result<Self, OrchestratorError> {
        Ok(Self {
            history: MetricHistory::new(objective.direction),
        })
    }

    pub fn history(&self) -> &MetricHistory {
        &self.history
    }

    pub fn capture_baseline(
        &mut self,
        verify: CommandRunResult,
        objective: &ObjectiveDefinition,
    ) -> Result<(), OrchestratorError> {
        if verify.timed_out {
            return Err(OrchestratorError::Other(
                "workflow baseline verification timed out".to_string(),
            ));
        }
        if let Some(error) = verify.runtime_error.as_ref() {
            return Err(OrchestratorError::Other(format!(
                "workflow baseline verification failed to execute: {}",
                first_meaningful_line(error)
            )));
        }
        let sample = self.sample_from_verify(0, &verify, objective)?;
        self.history.record_baseline(sample);
        Ok(())
    }

    pub fn capture_configured_baseline(&mut self, value: f64) {
        self.history.record_baseline(MetricSample {
            value,
            iteration: 0,
            captured_at: SystemTime::now(),
            command_output: "configured baseline".to_string(),
            exit_code: 0,
        });
    }

    pub fn evaluate_iteration(
        &mut self,
        iteration: u32,
        objective: &ObjectiveDefinition,
        verify: CommandRunResult,
        guard: Option<CommandRunResult>,
    ) -> Result<ObjectiveEvaluation, OrchestratorError> {
        if let Some(error) = verify.runtime_error.clone() {
            return Ok(ObjectiveEvaluation {
                verify,
                guard,
                metric: None,
                improved_over_baseline: false,
                improved_over_best: false,
                metric_regressed: false,
                metric_unchanged: false,
                objective_satisfied: false,
                runtime_error: Some(error),
                baseline_value: self.history.baseline.as_ref().map(|sample| sample.value),
            });
        }
        if verify.timed_out {
            return Ok(ObjectiveEvaluation {
                verify,
                guard,
                metric: None,
                improved_over_baseline: false,
                improved_over_best: false,
                metric_regressed: false,
                metric_unchanged: false,
                objective_satisfied: false,
                runtime_error: None,
                baseline_value: self.history.baseline.as_ref().map(|sample| sample.value),
            });
        }

        let sample = self.sample_from_verify(iteration, &verify, objective)?;
        let improved_over_baseline = self.history.improved_over_baseline(&sample);
        let improved_over_best = self.history.is_new_best(&sample);
        let metric_regressed = self.history.regressed_against_best(&sample);
        let metric_unchanged = self.history.unchanged_against_best(&sample);
        let objective_satisfied = objective_satisfied(&sample, objective);

        self.history.record_sample(sample.clone());

        Ok(ObjectiveEvaluation {
            verify,
            guard,
            metric: Some(sample),
            improved_over_baseline,
            improved_over_best,
            metric_regressed,
            metric_unchanged,
            objective_satisfied,
            runtime_error: None,
            baseline_value: self.history.baseline.as_ref().map(|sample| sample.value),
        })
    }

    fn sample_from_verify(
        &self,
        iteration: u32,
        verify: &CommandRunResult,
        objective: &ObjectiveDefinition,
    ) -> Result<MetricSample, OrchestratorError> {
        let exit_code = verify
            .exit_code
            .unwrap_or(if verify.passed() { 0 } else { 1 });
        let value = extract_metric_value(&objective.metric, &verify.output, exit_code)?;
        Ok(MetricSample {
            value,
            iteration,
            captured_at: SystemTime::now(),
            command_output: verify.output.clone(),
            exit_code,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DecisionPolicy {
    definition: DecisionPolicyDefinition,
    iteration_policy: Option<IterationPolicyDefinition>,
}

impl DecisionPolicy {
    pub fn new(
        definition: DecisionPolicyDefinition,
        iteration_policy: Option<IterationPolicyDefinition>,
    ) -> Self {
        Self {
            definition,
            iteration_policy,
        }
    }

    pub fn decide(
        &self,
        evaluation: &ObjectiveEvaluation,
        current_rework_attempts: u32,
        _stuck_counter: u32,
    ) -> IterationDecision {
        if let Some(error) = &evaluation.runtime_error {
            return IterationDecision::StopBlocked {
                reason: format!(
                    "verify command failed to execute: {}",
                    first_meaningful_line(error)
                ),
            };
        }

        if evaluation.verify.timed_out && self.should_discard(DiscardCondition::VerifyFailed) {
            return IterationDecision::Discard {
                reason: DiscardReason::VerifyTimeout,
            };
        }

        if !evaluation.verify.passed() && self.should_discard(DiscardCondition::VerifyFailed) {
            return IterationDecision::Discard {
                reason: DiscardReason::VerifyFailed,
            };
        }

        if let Some(guard) = &evaluation.guard {
            if let Some(error) = &guard.runtime_error {
                return IterationDecision::StopBlocked {
                    reason: format!(
                        "guard command failed to execute: {}",
                        first_meaningful_line(error)
                    ),
                };
            }
            if guard.timed_out {
                return IterationDecision::StopBlocked {
                    reason: "guard command timed out".to_string(),
                };
            }
            if !guard.passed() {
                let next_attempt = current_rework_attempts + 1;
                let max_attempts = self
                    .definition
                    .rework_policy
                    .as_ref()
                    .and_then(|policy| policy.max_attempts)
                    .unwrap_or(0);
                if next_attempt <= max_attempts {
                    return IterationDecision::Rework {
                        attempt: next_attempt,
                        guard_output: guard.output.clone(),
                    };
                }
                return IterationDecision::Discard {
                    reason: DiscardReason::GuardFailedAfterRework,
                };
            }
        }

        if evaluation.objective_satisfied {
            return IterationDecision::StopSatisfied;
        }

        if evaluation.metric_regressed && self.should_discard(DiscardCondition::MetricRegressed) {
            return IterationDecision::Discard {
                reason: DiscardReason::MetricRegressed,
            };
        }

        if evaluation.metric_unchanged {
            if self.trips_simplicity_override(evaluation) {
                return IterationDecision::Discard {
                    reason: DiscardReason::SimplicityOverride,
                };
            }
            if self.should_discard(DiscardCondition::MetricUnchanged) {
                return IterationDecision::Discard {
                    reason: DiscardReason::MetricUnchanged,
                };
            }
        }

        if self.keep_conditions_satisfied(evaluation) {
            return IterationDecision::Keep;
        }

        IterationDecision::StopBlocked {
            reason: "decision policy conditions were not satisfied for a keep or stop outcome"
                .to_string(),
        }
    }

    pub fn decide_execution_error(
        &self,
        error: &OrchestratorError,
        current_crash_retries: u32,
    ) -> IterationDecision {
        let next_attempt = current_crash_retries + 1;
        let max_attempts = self
            .definition
            .crash_retry_policy
            .as_ref()
            .and_then(|policy| policy.max_attempts)
            .unwrap_or(0);
        if next_attempt <= max_attempts {
            IterationDecision::RetryCrash {
                attempt: next_attempt,
                error: error.to_string(),
            }
        } else {
            IterationDecision::StopBlocked {
                reason: format!(
                    "crash recovery exhausted after {current_crash_retries} retries: {}",
                    first_meaningful_line(&error.to_string())
                ),
            }
        }
    }

    pub fn stuck_threshold_reached(&self, next_stuck_counter: u32) -> bool {
        self.iteration_policy
            .as_ref()
            .and_then(|policy| policy.stuck_threshold)
            .map(|threshold| threshold > 0 && next_stuck_counter >= threshold)
            .unwrap_or(false)
    }

    pub fn max_iterations_reached(&self, iteration: u32) -> bool {
        self.iteration_policy
            .as_ref()
            .filter(|policy| matches!(policy.mode, IterationMode::Bounded))
            .and_then(|policy| policy.max_iterations)
            .map(|limit| iteration >= limit)
            .unwrap_or(false)
    }

    fn should_discard(&self, condition: DiscardCondition) -> bool {
        if self.definition.discard_conditions.is_empty() {
            return matches!(
                condition,
                DiscardCondition::MetricRegressed | DiscardCondition::VerifyFailed
            );
        }
        self.definition.discard_conditions.contains(&condition)
    }

    fn keep_conditions_satisfied(&self, evaluation: &ObjectiveEvaluation) -> bool {
        if self.definition.keep_conditions.is_empty() {
            return evaluation.verify.passed() && !evaluation.metric_regressed;
        }

        self.definition
            .keep_conditions
            .iter()
            .all(|condition| match condition {
                KeepCondition::MetricImproved => evaluation.improved_over_best,
                KeepCondition::MetricUnchangedButSimpler => false,
                KeepCondition::VerifyPassed => evaluation.verify.passed(),
                KeepCondition::GuardPassed => evaluation
                    .guard
                    .as_ref()
                    .map(CommandRunResult::passed)
                    .unwrap_or(true),
            })
    }

    fn trips_simplicity_override(&self, evaluation: &ObjectiveEvaluation) -> bool {
        let Some(override_config) = &self.definition.simplicity_override else {
            return false;
        };
        if override_config.enabled == Some(false) {
            return false;
        }
        let Some(min_percent) = override_config.min_improvement_percent else {
            return false;
        };
        let Some(metric) = &evaluation.metric else {
            return false;
        };
        let Some(baseline) = evaluation
            .metric
            .as_ref()
            .and_then(|_| evaluation_metric_baseline(evaluation))
        else {
            return false;
        };
        let delta = (metric.value - baseline).abs();
        let denominator = baseline.abs().max(METRIC_EPSILON);
        let percent = (delta / denominator) * 100.0;
        percent < min_percent
    }
}

fn evaluation_metric_baseline(evaluation: &ObjectiveEvaluation) -> Option<f64> {
    evaluation.baseline_value
}

#[derive(Debug, Clone)]
pub struct ObjectiveEvaluation {
    pub verify: CommandRunResult,
    pub guard: Option<CommandRunResult>,
    pub metric: Option<MetricSample>,
    pub improved_over_baseline: bool,
    pub improved_over_best: bool,
    pub metric_regressed: bool,
    pub metric_unchanged: bool,
    pub objective_satisfied: bool,
    pub runtime_error: Option<String>,
    baseline_value: Option<f64>,
}

impl ObjectiveEvaluation {
    fn synthetic_execution_failure() -> Self {
        Self {
            verify: CommandRunResult {
                output: "verify not run".to_string(),
                exit_code: None,
                timed_out: false,
                runtime_error: Some("verify not run".to_string()),
            },
            guard: None,
            metric: None,
            improved_over_baseline: false,
            improved_over_best: false,
            metric_regressed: false,
            metric_unchanged: false,
            objective_satisfied: false,
            runtime_error: Some("execution failed before verify".to_string()),
            baseline_value: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSample {
    pub value: f64,
    pub iteration: u32,
    pub captured_at: SystemTime,
    pub command_output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone)]
pub struct MetricHistory {
    pub baseline: Option<MetricSample>,
    pub best: Option<MetricSample>,
    pub current: Option<MetricSample>,
    pub samples: Vec<MetricSample>,
    direction: ObjectiveDirection,
}

impl MetricHistory {
    fn new(direction: ObjectiveDirection) -> Self {
        Self {
            baseline: None,
            best: None,
            current: None,
            samples: Vec::new(),
            direction,
        }
    }

    fn record_baseline(&mut self, sample: MetricSample) {
        self.baseline = Some(sample.clone());
        self.best = Some(sample.clone());
        self.current = Some(sample.clone());
        self.samples.push(sample);
    }

    fn record_sample(&mut self, sample: MetricSample) {
        if self
            .best
            .as_ref()
            .map(|best| self.is_better(sample.value, best.value))
            .unwrap_or(true)
        {
            self.best = Some(sample.clone());
        }
        self.current = Some(sample.clone());
        self.samples.push(sample);
    }

    fn improved_over_baseline(&self, sample: &MetricSample) -> bool {
        self.baseline
            .as_ref()
            .map(|baseline| self.is_better(sample.value, baseline.value))
            .unwrap_or(false)
    }

    fn is_new_best(&self, sample: &MetricSample) -> bool {
        self.best
            .as_ref()
            .map(|best| self.is_better(sample.value, best.value))
            .unwrap_or(true)
    }

    fn regressed_against_best(&self, sample: &MetricSample) -> bool {
        self.best
            .as_ref()
            .map(|best| self.is_worse(sample.value, best.value))
            .unwrap_or(false)
    }

    fn unchanged_against_best(&self, sample: &MetricSample) -> bool {
        self.best
            .as_ref()
            .map(|best| float_eq(sample.value, best.value))
            .unwrap_or(false)
    }

    fn is_better(&self, candidate: f64, reference: f64) -> bool {
        match self.direction {
            ObjectiveDirection::HigherIsBetter => candidate > reference + METRIC_EPSILON,
            ObjectiveDirection::LowerIsBetter => candidate + METRIC_EPSILON < reference,
        }
    }

    fn is_worse(&self, candidate: f64, reference: f64) -> bool {
        match self.direction {
            ObjectiveDirection::HigherIsBetter => candidate + METRIC_EPSILON < reference,
            ObjectiveDirection::LowerIsBetter => candidate > reference + METRIC_EPSILON,
        }
    }
}

#[derive(Clone)]
pub struct VerificationRunner {
    tool_runner: ToolRunner,
    exec_ctx: ExecutionContext,
}

impl VerificationRunner {
    pub fn new(tool_runner: ToolRunner, exec_ctx: ExecutionContext) -> Self {
        Self {
            tool_runner,
            exec_ctx,
        }
    }

    pub async fn run_command(
        &self,
        label: &str,
        iteration: u32,
        command: &CommandDefinition,
        workdir_override: Option<&Path>,
    ) -> Result<CommandRunResult, OrchestratorError> {
        let workdir = resolve_command_workdir(
            &self.exec_ctx.workdir,
            workdir_override,
            command.working_directory.as_deref(),
        );
        let call_id = format!("workflow-{label}-{iteration}");
        let arguments = json!({
            "command": compose_shell_command(command)?,
            "description": format!("Run workflow {label}"),
            "workdir": workdir,
            "timeout": command.timeout_ms.unwrap_or(300_000),
        });
        let mut exec_ctx = self.exec_ctx.clone();
        exec_ctx
            .metadata
            .insert("call_id".to_string(), json!(call_id.clone()));

        let output = self
            .tool_runner
            .execute_tool_call(
                ToolCallInput {
                    id: call_id,
                    name: "bash".to_string(),
                    arguments,
                },
                &exec_ctx,
            )
            .await;

        let exit_code = output
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("exit_code"))
            .and_then(Value::as_i64)
            .map(|code| code as i32)
            .or_else(|| (!output.is_error).then_some(0));
        let runtime_error =
            (output.is_error && exit_code.is_none()).then_some(output.content.clone());
        let timed_out = runtime_error
            .as_deref()
            .map(|message| {
                let lower = message.to_ascii_lowercase();
                lower.contains("timed out") || lower.contains("timeout")
            })
            .unwrap_or(false);

        Ok(CommandRunResult {
            output: output.content,
            exit_code,
            timed_out,
            runtime_error,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CommandRunResult {
    pub output: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub runtime_error: Option<String>,
}

impl CommandRunResult {
    pub fn passed(&self) -> bool {
        !self.timed_out && self.runtime_error.is_none() && self.exit_code.unwrap_or(0) == 0
    }
}

fn extract_metric_value(
    metric: &crate::iterative_workflow::MetricDefinition,
    output: &str,
    exit_code: i32,
) -> Result<f64, OrchestratorError> {
    match metric.kind {
        MetricKind::NumericExtract => {
            let pattern = metric.pattern.as_deref().ok_or_else(|| {
                OrchestratorError::Other("numeric-extract metric requires pattern".to_string())
            })?;
            let regex = Regex::new(pattern).map_err(|err| {
                OrchestratorError::Other(format!("invalid metric regex pattern: {err}"))
            })?;
            let captures = regex.captures(output).ok_or_else(|| {
                OrchestratorError::Other(format!(
                    "metric pattern did not match verification output: {pattern}"
                ))
            })?;
            let raw = captures
                .iter()
                .skip(1)
                .flatten()
                .next()
                .map(|m| m.as_str())
                .or_else(|| captures.get(0).map(|m| m.as_str()))
                .ok_or_else(|| {
                    OrchestratorError::Other(
                        "metric pattern matched but no numeric capture was available".to_string(),
                    )
                })?;
            parse_metric_number(raw)
        }
        MetricKind::CountLines => {
            let pattern = metric.count_pattern.as_deref().ok_or_else(|| {
                OrchestratorError::Other("count-lines metric requires countPattern".to_string())
            })?;
            let regex = Regex::new(pattern).map_err(|err| {
                OrchestratorError::Other(format!("invalid countPattern regex: {err}"))
            })?;
            Ok(output.lines().filter(|line| regex.is_match(line)).count() as f64)
        }
        MetricKind::ExitCode => Ok(exit_code as f64),
        MetricKind::JsonPath => {
            let path = metric.json_path.as_deref().ok_or_else(|| {
                OrchestratorError::Other("json-path metric requires jsonPath".to_string())
            })?;
            let value: Value = serde_json::from_str(output).map_err(|err| {
                OrchestratorError::Other(format!("verification output was not valid JSON: {err}"))
            })?;
            let selected = select_json_path(&value, path)?;
            match selected {
                Value::Number(number) => number.as_f64().ok_or_else(|| {
                    OrchestratorError::Other("jsonPath metric did not resolve to f64".to_string())
                }),
                Value::String(text) => parse_metric_number(text),
                other => Err(OrchestratorError::Other(format!(
                    "jsonPath metric resolved to unsupported value: {other}"
                ))),
            }
        }
    }
}

fn compose_shell_command(command: &CommandDefinition) -> Result<String, OrchestratorError> {
    if command.env.is_empty() {
        return Ok(command.command.clone());
    }

    let mut prefix = Vec::new();
    for (key, value) in &command.env {
        if !valid_env_key(key) {
            return Err(OrchestratorError::Other(format!(
                "invalid environment variable name in workflow command: {key}"
            )));
        }
        prefix.push(format!("{key}={}", shell_quote(value)));
    }

    Ok(format!("{} {}", prefix.join(" "), command.command))
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn select_json_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value, OrchestratorError> {
    let mut remaining = path.trim();
    if remaining.is_empty() {
        return Err(OrchestratorError::Other(
            "jsonPath cannot be empty".to_string(),
        ));
    }
    if let Some(stripped) = remaining.strip_prefix('$') {
        remaining = stripped;
    }

    let mut current = value;
    while !remaining.is_empty() {
        if let Some(stripped) = remaining.strip_prefix('.') {
            remaining = stripped;
            let key_len = remaining
                .find(|ch: char| ch == '.' || ch == '[')
                .unwrap_or(remaining.len());
            let key = &remaining[..key_len];
            if key.is_empty() {
                return Err(OrchestratorError::Other(format!(
                    "unsupported jsonPath syntax: {path}"
                )));
            }
            current = current.get(key).ok_or_else(|| {
                OrchestratorError::Other(format!("jsonPath segment not found: {key}"))
            })?;
            remaining = &remaining[key_len..];
            continue;
        }

        if let Some(stripped) = remaining.strip_prefix('[') {
            let end = stripped.find(']').ok_or_else(|| {
                OrchestratorError::Other(format!("unterminated jsonPath index: {path}"))
            })?;
            let index_text = &stripped[..end];
            let index: usize = index_text.parse().map_err(|_| {
                OrchestratorError::Other(format!("invalid jsonPath index: {index_text}"))
            })?;
            current = current.get(index).ok_or_else(|| {
                OrchestratorError::Other(format!("jsonPath index out of bounds: {index}"))
            })?;
            remaining = &stripped[end + 1..];
            continue;
        }

        return Err(OrchestratorError::Other(format!(
            "unsupported jsonPath syntax: {path}"
        )));
    }

    Ok(current)
}

fn objective_satisfied(sample: &MetricSample, objective: &ObjectiveDefinition) -> bool {
    let Some(satisfied_when) = objective.satisfied_when.as_ref() else {
        return false;
    };

    if let Some(target) = satisfied_when.metric_equals {
        return float_eq(sample.value, target);
    }
    if let Some(target) = satisfied_when.metric_at_least {
        if sample.value + METRIC_EPSILON < target {
            return false;
        }
    }
    if let Some(target) = satisfied_when.metric_at_most {
        if sample.value > target + METRIC_EPSILON {
            return false;
        }
    }

    satisfied_when.metric_at_least.is_some() || satisfied_when.metric_at_most.is_some()
}

fn parse_metric_number(raw: &str) -> Result<f64, OrchestratorError> {
    let normalized = raw.trim().replace(',', "");
    normalized.parse::<f64>().map_err(|err| {
        OrchestratorError::Other(format!("failed to parse metric '{raw}' as number: {err}"))
    })
}

fn float_eq(left: f64, right: f64) -> bool {
    (left - right).abs() <= METRIC_EPSILON
}

fn first_meaningful_line(content: &str) -> &str {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("No details provided.")
}

fn gate_status_label(status: SchedulerExecutionGateStatus) -> &'static str {
    match status {
        SchedulerExecutionGateStatus::Done => "done",
        SchedulerExecutionGateStatus::Continue => "continue",
        SchedulerExecutionGateStatus::Blocked => "blocked",
    }
}

fn command_artifact(result: &CommandRunResult) -> WorkflowCommandArtifact {
    WorkflowCommandArtifact {
        exit_code: result.exit_code,
        passed: result.passed(),
        timed_out: result.timed_out,
        runtime_error: result.runtime_error.clone(),
        output_excerpt: first_meaningful_line(&result.output).to_string(),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointStatus {
    Active,
    Restored,
    Released,
    Orphaned,
}

#[derive(Debug, Clone)]
pub struct WorkspaceCheckpoint {
    pub checkpoint_id: String,
    pub iteration: u32,
    pub strategy: SnapshotStrategy,
    pub created_at: SystemTime,
    pub status: CheckpointStatus,
    ref_data: CheckpointRefData,
}

impl WorkspaceCheckpoint {
    fn execution_workdir(&self) -> Option<&Path> {
        self.ref_data.execution_workdir()
    }
}

#[derive(Debug, Clone)]
struct SnapshotEntry {
    relative_path: String,
    backup_path: PathBuf,
}

#[derive(Debug, Clone)]
enum CheckpointRefData {
    PatchFile {
        root: PathBuf,
        entries: Vec<SnapshotEntry>,
    },
    GitBranch {
        worktree_root: PathBuf,
        execution_root: PathBuf,
        branch_name: String,
    },
    GitStash {
        root: PathBuf,
        entries: Vec<SnapshotEntry>,
        stash_commit: Option<String>,
        captured_paths: Vec<String>,
    },
    Worktree {
        worktree_root: PathBuf,
        execution_root: PathBuf,
    },
}

impl CheckpointRefData {
    fn execution_workdir(&self) -> Option<&Path> {
        match self {
            Self::GitBranch { execution_root, .. } | Self::Worktree { execution_root, .. } => {
                Some(execution_root.as_path())
            }
            Self::PatchFile { .. } | Self::GitStash { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotEngine {
    workdir: PathBuf,
    runtime_root: PathBuf,
    checkpoint_root: PathBuf,
    worktree_root: PathBuf,
    strategy: SnapshotStrategy,
    session_id: String,
    runtime_relative: Option<PathBuf>,
    include: Vec<Pattern>,
    exclude: Vec<Pattern>,
}

impl SnapshotEngine {
    pub fn new(
        config: &IterativeWorkflowConfig,
        objective: &ObjectiveDefinition,
        exec_ctx: &ExecutionContext,
    ) -> Result<Self, OrchestratorError> {
        let workspace_policy = config.workspace_policy.as_ref().ok_or_else(|| {
            OrchestratorError::Other("workflow workspacePolicy is required".to_string())
        })?;
        let workdir = PathBuf::from(&exec_ctx.workdir);
        let runtime_root = workflow_run_dir(config, &workdir, &exec_ctx.session_id);
        let checkpoint_root = runtime_root.join("checkpoints");
        let worktree_root = resolve_worktree_root(&workdir, &runtime_root, &exec_ctx.session_id);
        let include = compile_patterns(&objective.scope.include, "include")?;
        let exclude = compile_patterns(&objective.scope.exclude, "exclude")?;
        let runtime_relative = runtime_root.strip_prefix(&workdir).ok().map(PathBuf::from);

        fs::create_dir_all(&checkpoint_root).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create workflow checkpoint root '{}': {err}",
                checkpoint_root.display()
            ))
        })?;
        fs::create_dir_all(&worktree_root).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create workflow worktree root '{}': {err}",
                worktree_root.display()
            ))
        })?;

        Ok(Self {
            workdir,
            runtime_root,
            checkpoint_root,
            worktree_root,
            strategy: workspace_policy.snapshot_strategy,
            session_id: exec_ctx.session_id.clone(),
            runtime_relative,
            include,
            exclude,
        })
    }

    pub fn capture(&self, iteration: u32) -> Result<WorkspaceCheckpoint, OrchestratorError> {
        match self.strategy {
            SnapshotStrategy::PatchFile => self.capture_patch_snapshot(iteration),
            SnapshotStrategy::GitBranchPerIteration => self.capture_git_branch_snapshot(iteration),
            SnapshotStrategy::GitStashStack => self.capture_git_stash_snapshot(iteration),
            SnapshotStrategy::WorktreeFork => self.capture_worktree_snapshot(iteration),
        }
    }

    pub fn restore(&self, checkpoint: &mut WorkspaceCheckpoint) -> Result<(), OrchestratorError> {
        if checkpoint.status != CheckpointStatus::Active {
            return Err(OrchestratorError::Other(format!(
                "checkpoint '{}' is not active",
                checkpoint.checkpoint_id
            )));
        }
        match &checkpoint.ref_data {
            CheckpointRefData::PatchFile { entries, .. } => {
                self.restore_patch_snapshot(&checkpoint.checkpoint_id, entries)
            }
            CheckpointRefData::GitBranch {
                worktree_root,
                branch_name,
                ..
            } => self.cleanup_git_worktree(worktree_root, Some(branch_name)),
            CheckpointRefData::GitStash {
                entries,
                stash_commit,
                captured_paths,
                ..
            } => self.restore_git_stash_snapshot(stash_commit.as_deref(), captured_paths, entries),

            CheckpointRefData::Worktree { worktree_root, .. } => {
                self.cleanup_git_worktree(worktree_root, None)
            }
        }?;
        checkpoint.status = CheckpointStatus::Restored;
        self.cleanup_checkpoint(checkpoint)
    }

    pub fn release(&self, checkpoint: &mut WorkspaceCheckpoint) -> Result<(), OrchestratorError> {
        if checkpoint.status != CheckpointStatus::Active {
            return Err(OrchestratorError::Other(format!(
                "checkpoint '{}' is not active",
                checkpoint.checkpoint_id
            )));
        }
        match &checkpoint.ref_data {
            CheckpointRefData::PatchFile { .. } => {}
            CheckpointRefData::GitBranch {
                worktree_root,
                execution_root,
                branch_name,
                ..
            } => {
                self.sync_scoped_state(execution_root, &self.workdir)?;
                self.cleanup_git_worktree(worktree_root, Some(branch_name))?;
            }
            CheckpointRefData::GitStash { stash_commit, .. } => {
                self.release_git_stash_snapshot(stash_commit.as_deref())?;
            }
            CheckpointRefData::Worktree {
                worktree_root,
                execution_root,
                ..
            } => {
                self.sync_scoped_state(execution_root, &self.workdir)?;
                self.cleanup_git_worktree(worktree_root, None)?;
            }
        }
        checkpoint.status = CheckpointStatus::Released;
        self.cleanup_checkpoint(checkpoint)
    }

    pub fn orphan(&self, checkpoint: &mut WorkspaceCheckpoint) -> Result<(), OrchestratorError> {
        if checkpoint.status == CheckpointStatus::Active {
            checkpoint.status = CheckpointStatus::Orphaned;
        }
        Ok(())
    }

    fn capture_patch_snapshot(
        &self,
        iteration: u32,
    ) -> Result<WorkspaceCheckpoint, OrchestratorError> {
        let checkpoint_id = format!("iter-{iteration}-{}", now_nanos());
        let scoped_paths = self.current_scoped_files()?;
        let (root, entries) = self.create_file_backups(&checkpoint_id, &scoped_paths)?;

        Ok(WorkspaceCheckpoint {
            checkpoint_id,
            iteration,
            strategy: self.strategy,
            created_at: SystemTime::now(),
            status: CheckpointStatus::Active,
            ref_data: CheckpointRefData::PatchFile { root, entries },
        })
    }

    fn create_file_backups(
        &self,
        checkpoint_id: &str,
        scoped_paths: &[String],
    ) -> Result<(PathBuf, Vec<SnapshotEntry>), OrchestratorError> {
        let root = self.checkpoint_root.join(checkpoint_id);
        let files_root = root.join("files");
        fs::create_dir_all(&files_root).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create checkpoint directory '{}': {err}",
                files_root.display()
            ))
        })?;

        let mut entries = Vec::new();
        for relative_path in scoped_paths {
            let source_path = self.workdir.join(Path::new(relative_path));
            let backup_path = files_root.join(Path::new(relative_path));
            if let Some(parent) = backup_path.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    OrchestratorError::Other(format!(
                        "failed to create checkpoint parent '{}' for '{}': {err}",
                        parent.display(),
                        relative_path
                    ))
                })?;
            }
            fs::copy(&source_path, &backup_path).map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to snapshot '{}' into '{}': {err}",
                    source_path.display(),
                    backup_path.display()
                ))
            })?;
            entries.push(SnapshotEntry {
                relative_path: relative_path.clone(),
                backup_path,
            });
        }

        Ok((root, entries))
    }

    fn capture_git_branch_snapshot(
        &self,
        iteration: u32,
    ) -> Result<WorkspaceCheckpoint, OrchestratorError> {
        let checkpoint_id = format!("iter-{iteration}-{}", now_nanos());
        let repo_root = self.git_repo_root()?;
        let head_sha = self.git_head_sha()?;
        let branch_name = iteration_branch_name(&self.session_id, iteration, &checkpoint_id);
        let worktree_root = self.worktree_root.join(&checkpoint_id);
        let execution_root = self.worktree_execution_root(&repo_root, &worktree_root)?;

        let result = self.create_git_worktree(
            &worktree_root,
            Some(branch_name.as_str()),
            &head_sha,
            &format!("branch checkpoint '{}'", checkpoint_id),
        );
        if let Err(err) = result {
            if let Err(cleanup_error) =
                self.cleanup_git_worktree(&worktree_root, Some(&branch_name))
            {
                tracing::warn!(
                    session_id = %self.session_id,
                    checkpoint_id,
                    worktree_root = %worktree_root.display(),
                    branch_name,
                    error = %cleanup_error,
                    "Failed to cleanup git worktree after branch checkpoint creation error"
                );
            }
            return Err(err);
        }
        if let Err(err) = self.sync_scoped_state(&self.workdir, &execution_root) {
            if let Err(cleanup_error) =
                self.cleanup_git_worktree(&worktree_root, Some(&branch_name))
            {
                tracing::warn!(
                    session_id = %self.session_id,
                    checkpoint_id,
                    worktree_root = %worktree_root.display(),
                    branch_name,
                    error = %cleanup_error,
                    "Failed to cleanup git worktree after branch checkpoint sync error"
                );
            }
            return Err(err);
        }

        Ok(WorkspaceCheckpoint {
            checkpoint_id,
            iteration,
            strategy: self.strategy,
            created_at: SystemTime::now(),
            status: CheckpointStatus::Active,
            ref_data: CheckpointRefData::GitBranch {
                worktree_root,
                execution_root,
                branch_name,
            },
        })
    }

    fn capture_git_stash_snapshot(
        &self,
        iteration: u32,
    ) -> Result<WorkspaceCheckpoint, OrchestratorError> {
        self.git_repo_root()?;
        let checkpoint_id = format!("iter-{iteration}-{}", now_nanos());
        let snapshot_message = format!("rocode-autoresearch/{}/{}", self.session_id, checkpoint_id);
        let scoped_paths = self.current_scoped_files()?;
        let (root, entries) = self.create_file_backups(&checkpoint_id, &scoped_paths)?;
        let stash_commit = self.capture_stash_snapshot(&snapshot_message, &scoped_paths)?;
        for entry in &entries {
            let target_path = self.workdir.join(Path::new(&entry.relative_path));
            copy_file_to_target(&entry.backup_path, &target_path, &entry.relative_path)?;
        }

        Ok(WorkspaceCheckpoint {
            checkpoint_id,
            iteration,
            strategy: self.strategy,
            created_at: SystemTime::now(),
            status: CheckpointStatus::Active,
            ref_data: CheckpointRefData::GitStash {
                root,
                entries,
                stash_commit,
                captured_paths: scoped_paths,
            },
        })
    }

    fn capture_worktree_snapshot(
        &self,
        iteration: u32,
    ) -> Result<WorkspaceCheckpoint, OrchestratorError> {
        let checkpoint_id = format!("iter-{iteration}-{}", now_nanos());
        let repo_root = self.git_repo_root()?;
        let head_sha = self.git_head_sha()?;
        let worktree_root = self.worktree_root.join(&checkpoint_id);
        let execution_root = self.worktree_execution_root(&repo_root, &worktree_root)?;

        let result = self.create_git_worktree(
            &worktree_root,
            None,
            &head_sha,
            &format!("worktree checkpoint '{}'", checkpoint_id),
        );
        if let Err(err) = result {
            if let Err(cleanup_error) = self.cleanup_git_worktree(&worktree_root, None) {
                tracing::warn!(
                    session_id = %self.session_id,
                    checkpoint_id,
                    worktree_root = %worktree_root.display(),
                    error = %cleanup_error,
                    "Failed to cleanup git worktree after worktree checkpoint creation error"
                );
            }
            return Err(err);
        }
        if let Err(err) = self.sync_scoped_state(&self.workdir, &execution_root) {
            if let Err(cleanup_error) = self.cleanup_git_worktree(&worktree_root, None) {
                tracing::warn!(
                    session_id = %self.session_id,
                    checkpoint_id,
                    worktree_root = %worktree_root.display(),
                    error = %cleanup_error,
                    "Failed to cleanup git worktree after worktree checkpoint sync error"
                );
            }
            return Err(err);
        }

        Ok(WorkspaceCheckpoint {
            checkpoint_id,
            iteration,
            strategy: self.strategy,
            created_at: SystemTime::now(),
            status: CheckpointStatus::Active,
            ref_data: CheckpointRefData::Worktree {
                worktree_root,
                execution_root,
            },
        })
    }

    fn restore_patch_snapshot(
        &self,
        checkpoint_id: &str,
        entries: &[SnapshotEntry],
    ) -> Result<(), OrchestratorError> {
        let expected_paths: HashSet<&str> = entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        let scoped_paths = self.current_scoped_files()?;
        for relative_path in scoped_paths {
            if expected_paths.contains(relative_path.as_str()) {
                continue;
            }
            let absolute_path = self.workdir.join(Path::new(&relative_path));
            if absolute_path.exists() {
                remove_path(
                    &absolute_path,
                    &self.workdir,
                    self.runtime_root_for(&self.workdir).as_deref(),
                )?;
            }
        }

        for entry in entries {
            let target_path = self.workdir.join(Path::new(&entry.relative_path));
            copy_file_to_target(&entry.backup_path, &target_path, checkpoint_id)?;
        }

        Ok(())
    }

    fn restore_git_stash_snapshot(
        &self,
        stash_commit: Option<&str>,
        captured_paths: &[String],
        entries: &[SnapshotEntry],
    ) -> Result<(), OrchestratorError> {
        let current_paths = self.current_scoped_files()?;
        let discard_message = format!(
            "rocode-autoresearch/{}/discard-{}",
            self.session_id,
            now_nanos()
        );
        let discard_stash = self.capture_stash_snapshot(&discard_message, &current_paths)?;
        if let Some(stash_commit) = stash_commit {
            self.apply_stash_snapshot(stash_commit)?;
            self.restore_untracked_from_stash_parent(stash_commit, captured_paths)?;
            self.drop_stash_snapshot(stash_commit)?;
        } else {
            self.clear_scoped_state(&self.workdir)?;
        }
        self.restore_patch_snapshot("git-stash", entries)?;
        if let Some(discard_stash) = discard_stash.as_deref() {
            self.drop_stash_snapshot(discard_stash)?;
        }
        Ok(())
    }

    fn release_git_stash_snapshot(
        &self,
        stash_commit: Option<&str>,
    ) -> Result<(), OrchestratorError> {
        if let Some(stash_commit) = stash_commit {
            self.drop_stash_snapshot(stash_commit)?;
        }
        Ok(())
    }

    fn current_scoped_files(&self) -> Result<Vec<String>, OrchestratorError> {
        self.scoped_files_in(&self.workdir)
    }

    fn scoped_files_in(&self, root: &Path) -> Result<Vec<String>, OrchestratorError> {
        let mut files = Vec::new();
        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(|entry| self.should_descend(root, entry.path()))
        {
            let entry = entry.map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to walk workflow workdir '{}': {err}",
                    root.display()
                ))
            })?;
            let file_type = entry.file_type();
            if !file_type.is_file() {
                continue;
            }
            if file_type.is_symlink() {
                return Err(OrchestratorError::Other(format!(
                    "workflow snapshot does not support symlinks: '{}'",
                    entry.path().display()
                )));
            }

            let relative = entry.path().strip_prefix(root).map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to compute scoped path under '{}': {err}",
                    root.display()
                ))
            })?;
            let normalized = normalize_relative_path(relative);
            if self.matches_scope(&normalized) {
                files.push(normalized);
            }
        }
        files.sort();
        Ok(files)
    }

    fn should_descend(&self, root: &Path, path: &Path) -> bool {
        if path == root {
            return true;
        }
        !self.is_internal_path(root, path)
    }

    fn matches_scope(&self, relative_path: &str) -> bool {
        let included = if self.include.is_empty() {
            true
        } else {
            self.include
                .iter()
                .any(|pattern| pattern.matches(relative_path))
        };
        included
            && !self
                .exclude
                .iter()
                .any(|pattern| pattern.matches(relative_path))
    }

    fn is_internal_path(&self, root: &Path, path: &Path) -> bool {
        path.starts_with(root.join(".git"))
            || self
                .runtime_root_for(root)
                .map(|runtime_root| path.starts_with(runtime_root))
                .unwrap_or(false)
    }

    fn runtime_root_for(&self, root: &Path) -> Option<PathBuf> {
        if root == self.workdir {
            return Some(self.runtime_root.clone());
        }
        self.runtime_relative
            .as_ref()
            .map(|relative| root.join(relative))
    }

    fn sync_scoped_state(
        &self,
        source_root: &Path,
        target_root: &Path,
    ) -> Result<(), OrchestratorError> {
        let source_files = self.scoped_files_in(source_root)?;
        let source_set: HashSet<&str> = source_files.iter().map(String::as_str).collect();
        let target_files = self.scoped_files_in(target_root)?;

        for relative_path in target_files {
            if source_set.contains(relative_path.as_str()) {
                continue;
            }
            let absolute_path = target_root.join(Path::new(&relative_path));
            if absolute_path.exists() {
                remove_path(
                    &absolute_path,
                    target_root,
                    self.runtime_root_for(target_root).as_deref(),
                )?;
            }
        }

        for relative_path in source_files {
            let source_path = source_root.join(Path::new(&relative_path));
            let target_path = target_root.join(Path::new(&relative_path));
            copy_file_to_target(&source_path, &target_path, &relative_path)?;
        }

        Ok(())
    }

    fn clear_scoped_state(&self, root: &Path) -> Result<(), OrchestratorError> {
        for relative_path in self.scoped_files_in(root)? {
            let absolute_path = root.join(Path::new(&relative_path));
            if absolute_path.exists() {
                remove_path(&absolute_path, root, self.runtime_root_for(root).as_deref())?;
            }
        }
        Ok(())
    }

    fn git_repo_root(&self) -> Result<PathBuf, OrchestratorError> {
        let output = run_git(&self.workdir, ["rev-parse", "--show-toplevel"])?;
        Ok(PathBuf::from(output.trim()))
    }

    fn git_head_sha(&self) -> Result<String, OrchestratorError> {
        run_git(&self.workdir, ["rev-parse", "HEAD"])
    }

    fn worktree_execution_root(
        &self,
        repo_root: &Path,
        worktree_root: &Path,
    ) -> Result<PathBuf, OrchestratorError> {
        let relative = self.workdir.strip_prefix(repo_root).map_err(|err| {
            OrchestratorError::Other(format!(
                "workflow workdir '{}' is not inside git repo '{}': {err}",
                self.workdir.display(),
                repo_root.display()
            ))
        })?;
        Ok(if relative.as_os_str().is_empty() {
            worktree_root.to_path_buf()
        } else {
            worktree_root.join(relative)
        })
    }

    fn create_git_worktree(
        &self,
        worktree_root: &Path,
        branch_name: Option<&str>,
        head_sha: &str,
        label: &str,
    ) -> Result<(), OrchestratorError> {
        let mut args = vec!["worktree".to_string(), "add".to_string()];
        if let Some(branch_name) = branch_name {
            args.push("-b".to_string());
            args.push(branch_name.to_string());
        } else {
            args.push("--detach".to_string());
        }
        args.push(worktree_root.display().to_string());
        args.push(head_sha.to_string());
        run_git(&self.workdir, args).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create {label} at '{}': {err}",
                worktree_root.display()
            ))
        })?;
        Ok(())
    }

    fn cleanup_git_worktree(
        &self,
        worktree_root: &Path,
        branch_name: Option<&str>,
    ) -> Result<(), OrchestratorError> {
        if worktree_root.exists() {
            run_git(
                &self.workdir,
                [
                    "worktree",
                    "remove",
                    "--force",
                    worktree_root.to_string_lossy().as_ref(),
                ],
            )
            .map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to remove workflow worktree '{}': {err}",
                    worktree_root.display()
                ))
            })?;
        }

        if let Some(branch_name) = branch_name {
            run_git(&self.workdir, ["branch", "-D", branch_name]).map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to delete workflow branch '{branch_name}': {err}"
                ))
            })?;
        }

        if let Err(error) = run_git(&self.workdir, ["worktree", "prune"]) {
            tracing::debug!(
                session_id = %self.session_id,
                worktree_root = %worktree_root.display(),
                error = %error,
                "Failed to prune git worktrees during cleanup"
            );
        }
        Ok(())
    }

    fn capture_stash_snapshot(
        &self,
        message: &str,
        scoped_paths: &[String],
    ) -> Result<Option<String>, OrchestratorError> {
        if scoped_paths.is_empty() {
            return Ok(None);
        }

        let mut args = vec![
            "stash".to_string(),
            "push".to_string(),
            "--include-untracked".to_string(),
            "--message".to_string(),
            message.to_string(),
            "--".to_string(),
        ];
        args.extend(scoped_paths.iter().cloned());

        let output = run_git(&self.workdir, args)?;
        if output.contains("No local changes to save") {
            return Ok(None);
        }

        let stash_commit = find_stash_commit_by_message(&self.workdir, message)?;
        if let Some(stash_commit) = stash_commit.as_deref() {
            self.apply_stash_snapshot(stash_commit)?;
            self.restore_untracked_from_stash_parent(stash_commit, scoped_paths)?;
        }
        Ok(stash_commit)
    }

    fn apply_stash_snapshot(&self, stash_commit: &str) -> Result<(), OrchestratorError> {
        run_git(&self.workdir, ["stash", "apply", "--index", stash_commit]).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to apply workflow stash '{stash_commit}': {err}"
            ))
        })?;
        Ok(())
    }

    fn drop_stash_snapshot(&self, stash_commit: &str) -> Result<(), OrchestratorError> {
        let Some(stash_ref) = find_stash_ref_by_commit(&self.workdir, stash_commit)? else {
            return Ok(());
        };
        run_git(&self.workdir, ["stash", "drop", stash_ref.as_str()]).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to drop workflow stash '{stash_ref}': {err}"
            ))
        })?;
        Ok(())
    }

    fn restore_untracked_from_stash_parent(
        &self,
        stash_commit: &str,
        captured_paths: &[String],
    ) -> Result<(), OrchestratorError> {
        if captured_paths.is_empty() {
            return Ok(());
        }
        let Some(untracked_parent) = try_run_git(
            &self.workdir,
            ["rev-parse", "--verify", &format!("{stash_commit}^3")],
        )?
        else {
            return Ok(());
        };

        let mut list_args = vec![
            "ls-tree".to_string(),
            "-r".to_string(),
            "--name-only".to_string(),
            untracked_parent.clone(),
            "--".to_string(),
        ];
        list_args.extend(captured_paths.iter().cloned());
        let untracked_files = run_git(&self.workdir, list_args)?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if untracked_files.is_empty() {
            return Ok(());
        }

        let mut checkout_args = vec!["checkout".to_string(), untracked_parent, "--".to_string()];
        checkout_args.extend(untracked_files);
        run_git(&self.workdir, checkout_args).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to restore workflow untracked files from stash '{stash_commit}': {err}"
            ))
        })?;
        Ok(())
    }

    fn cleanup_checkpoint(
        &self,
        checkpoint: &WorkspaceCheckpoint,
    ) -> Result<(), OrchestratorError> {
        let root = match &checkpoint.ref_data {
            CheckpointRefData::PatchFile { root, .. }
            | CheckpointRefData::GitStash { root, .. } => Some(root),
            CheckpointRefData::GitBranch { .. } | CheckpointRefData::Worktree { .. } => None,
        };
        if let Some(root) = root {
            if root.exists() {
                fs::remove_dir_all(root).map_err(|err| {
                    OrchestratorError::Other(format!(
                        "failed to clean up checkpoint '{}': {err}",
                        root.display()
                    ))
                })?;
            }
        }
        Ok(())
    }
}

fn resolve_worktree_root(workdir: &Path, runtime_root: &Path, session_id: &str) -> PathBuf {
    if runtime_root.starts_with(workdir) {
        workdir
            .parent()
            .unwrap_or(workdir)
            .join(format!(".rocode-autoresearch-worktrees-{session_id}"))
    } else {
        runtime_root.join("worktrees")
    }
}

fn resolve_path(value: &str, base: &Path) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn resolve_command_workdir(
    default_workdir: &str,
    workdir_override: Option<&Path>,
    configured_workdir: Option<&str>,
) -> String {
    let base = workdir_override.unwrap_or_else(|| Path::new(default_workdir));
    match configured_workdir {
        Some(workdir) => resolve_path(workdir, base).display().to_string(),
        None => base.display().to_string(),
    }
}

fn run_git<I, S>(cwd: &Path, args: I) -> Result<String, OrchestratorError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args_vec: Vec<_> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(cwd)
        .output()
        .map_err(|err| {
            OrchestratorError::Other(format!("failed to run git in '{}': {err}", cwd.display()))
        })?;

    if !output.status.success() {
        let rendered = args_vec
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(OrchestratorError::Other(format!(
            "git {} failed: {}",
            rendered,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn try_run_git<I, S>(cwd: &Path, args: I) -> Result<Option<String>, OrchestratorError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args_vec: Vec<_> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(cwd)
        .output()
        .map_err(|err| {
            OrchestratorError::Other(format!("failed to run git in '{}': {err}", cwd.display()))
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn find_stash_commit_by_message(
    workdir: &Path,
    message: &str,
) -> Result<Option<String>, OrchestratorError> {
    let output = run_git(workdir, ["stash", "list", "--format=%H%x00%gs"])?;
    Ok(output.lines().find_map(|line| {
        let mut parts = line.splitn(2, '\0');
        let commit = parts.next()?.trim();
        let subject = parts.next()?.trim();
        (subject == message && !commit.is_empty()).then(|| commit.to_string())
    }))
}

fn find_stash_ref_by_commit(
    workdir: &Path,
    commit: &str,
) -> Result<Option<String>, OrchestratorError> {
    let output = run_git(workdir, ["stash", "list", "--format=%H%x00%gd"])?;
    Ok(output.lines().find_map(|line| {
        let mut parts = line.splitn(2, '\0');
        let sha = parts.next()?.trim();
        let reference = parts.next()?.trim();
        (sha == commit && !reference.is_empty()).then(|| reference.to_string())
    }))
}

fn compile_patterns(patterns: &[String], label: &str) -> Result<Vec<Pattern>, OrchestratorError> {
    patterns
        .iter()
        .map(|pattern| {
            Pattern::new(pattern).map_err(|err| {
                OrchestratorError::Other(format!(
                    "invalid workflow {label} glob '{pattern}': {err}"
                ))
            })
        })
        .collect()
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn iteration_branch_name(session_id: &str, iteration: u32, checkpoint_id: &str) -> String {
    let sanitized = session_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '/' => ch,
            _ => '-',
        })
        .collect::<String>();
    format!("autoresearch/{sanitized}/iter-{iteration}-{checkpoint_id}")
}

fn copy_file_to_target(
    source_path: &Path,
    target_path: &Path,
    label: &str,
) -> Result<(), OrchestratorError> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create directory '{}' for '{}': {err}",
                parent.display(),
                label
            ))
        })?;
    }
    if target_path.is_dir() {
        fs::remove_dir_all(target_path).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to replace directory '{}' while syncing '{}': {err}",
                target_path.display(),
                label
            ))
        })?;
    }
    fs::copy(source_path, target_path).map_err(|err| {
        OrchestratorError::Other(format!(
            "failed to copy '{}' into '{}': {err}",
            source_path.display(),
            target_path.display()
        ))
    })?;
    Ok(())
}

fn remove_path(
    target_path: &Path,
    workdir: &Path,
    runtime_root: Option<&Path>,
) -> Result<(), OrchestratorError> {
    if target_path.is_dir() {
        fs::remove_dir_all(target_path).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to remove directory '{}': {err}",
                target_path.display()
            ))
        })?;
    } else {
        fs::remove_file(target_path).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to remove file '{}': {err}",
                target_path.display()
            ))
        })?;
    }

    if let Some(parent) = target_path.parent() {
        prune_empty_directories(parent, workdir, runtime_root.unwrap_or(workdir))?;
    }
    Ok(())
}

fn prune_empty_directories(
    mut current: &Path,
    workdir: &Path,
    runtime_root: &Path,
) -> Result<(), OrchestratorError> {
    while current.starts_with(workdir) && current != workdir && !current.starts_with(runtime_root) {
        match fs::remove_dir(current) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => {
                return Err(OrchestratorError::Other(format!(
                    "failed to prune empty directory '{}': {err}",
                    current.display()
                )));
            }
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    Ok(())
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ToolExecutor;
    use crate::workflow_artifacts::WorkflowModeArtifactEntry;
    use crate::{ToolExecError, ToolOutput};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn run_workflow_config() -> IterativeWorkflowConfig {
        IterativeWorkflowConfig {
            workflow: crate::iterative_workflow::WorkflowDescriptor {
                kind: IterativeWorkflowKind::Autoresearch,
                mode: crate::iterative_workflow::IterativeWorkflowMode::Run,
            },
            objective: Some(ObjectiveDefinition {
                goal: "Improve passing tests".to_string(),
                scope: crate::iterative_workflow::ScopeDefinition {
                    include: vec!["src/**".to_string()],
                    exclude: Vec::new(),
                },
                direction: ObjectiveDirection::HigherIsBetter,
                metric: crate::iterative_workflow::MetricDefinition {
                    kind: MetricKind::NumericExtract,
                    pattern: Some("score=(\\d+)".to_string()),
                    count_pattern: None,
                    json_path: None,
                    unit: None,
                },
                verify: CommandDefinition {
                    command: "cargo test".to_string(),
                    timeout_ms: Some(5_000),
                    env: HashMap::new(),
                    working_directory: None,
                },
                guard: None,
                satisfied_when: Some(crate::iterative_workflow::SatisfiedWhenDefinition {
                    metric_at_least: Some(12.0),
                    metric_at_most: None,
                    metric_equals: None,
                }),
            }),
            iteration_policy: Some(IterationPolicyDefinition {
                mode: IterationMode::Bounded,
                max_iterations: Some(5),
                stop_conditions: Vec::new(),
                stuck_threshold: Some(2),
                progress_report_every: None,
            }),
            decision_policy: Some(DecisionPolicyDefinition {
                baseline_strategy: Some(BaselineStrategy::CaptureBeforeFirstIteration),
                baseline_value: None,
                keep_conditions: vec![KeepCondition::MetricImproved, KeepCondition::VerifyPassed],
                discard_conditions: vec![
                    DiscardCondition::MetricRegressed,
                    DiscardCondition::MetricUnchanged,
                    DiscardCondition::VerifyFailed,
                ],
                rework_policy: None,
                crash_retry_policy: Some(crate::iterative_workflow::AttemptPolicy {
                    max_attempts: Some(2),
                }),
                simplicity_override: None,
            }),
            workspace_policy: Some(crate::iterative_workflow::WorkspacePolicyDefinition {
                mutation_mode: None,
                protected_paths: Vec::new(),
                snapshot_strategy: crate::iterative_workflow::SnapshotStrategy::PatchFile,
                commit_policy: None,
            }),
            artifacts: None,
            approval_policy: None,
            security: None,
            debug: None,
            fix: None,
            ship: None,
        }
    }

    fn workflow_config_with_strategy(strategy: SnapshotStrategy) -> IterativeWorkflowConfig {
        let mut config = run_workflow_config();
        config
            .workspace_policy
            .as_mut()
            .expect("workspace policy should exist")
            .snapshot_strategy = strategy;
        config
    }

    fn workflow_config_with_artifacts() -> IterativeWorkflowConfig {
        let mut config = run_workflow_config();
        config.artifacts = Some(crate::iterative_workflow::ArtifactDefinition {
            root_dir: None,
            run_dir: None,
            iteration_log: Some(crate::iterative_workflow::ArtifactFileDefinition {
                format: Some("tsv".to_string()),
                filename: Some("iterations.tsv".to_string()),
            }),
            summary: Some(crate::iterative_workflow::ArtifactFileDefinition {
                format: Some("json".to_string()),
                filename: Some("summary.json".to_string()),
            }),
        });
        config
    }

    fn workflow_config_with_commit_policy(squash_on_completion: bool) -> IterativeWorkflowConfig {
        let mut config = workflow_config_with_artifacts();
        config
            .workspace_policy
            .as_mut()
            .expect("workspace policy should exist")
            .commit_policy = Some(crate::iterative_workflow::CommitPolicyDefinition {
            commit_kept_iterations: Some(true),
            message_template: Some(
                "autoresearch iter {iteration}: {decision} {summary}".to_string(),
            ),
            squash_on_completion: Some(squash_on_completion),
        });
        config
    }

    fn workflow_config_for_security_mode() -> IterativeWorkflowConfig {
        let mut config = workflow_config_with_artifacts();
        config.workflow.mode = crate::iterative_workflow::IterativeWorkflowMode::Security;
        config.security = Some(crate::iterative_workflow::SecurityConfig {
            coverage_targets: vec![
                crate::iterative_workflow::SecurityCoverageTarget::OwaspTop10,
                crate::iterative_workflow::SecurityCoverageTarget::Stride,
            ],
            fail_on_severity: Some(crate::iterative_workflow::SeverityLevel::High),
            diff_mode: Some(true),
            auto_fix: Some(false),
            required_evidence: vec![
                crate::iterative_workflow::SecurityEvidenceRequirement::FileLine,
                crate::iterative_workflow::SecurityEvidenceRequirement::SeverityJustification,
            ],
        });
        config
    }

    #[derive(Default)]
    struct ScriptedToolExecutor {
        responses: Mutex<VecDeque<Result<ToolOutput, ToolExecError>>>,
    }

    #[async_trait]
    impl ToolExecutor for ScriptedToolExecutor {
        async fn execute(
            &self,
            _tool_name: &str,
            _arguments: Value,
            _exec_ctx: &ExecutionContext,
        ) -> Result<ToolOutput, ToolExecError> {
            self.responses
                .lock()
                .await
                .pop_front()
                .expect("scripted tool response should exist")
        }

        async fn list_ids(&self) -> Vec<String> {
            vec!["bash".to_string()]
        }

        async fn list_definitions(
            &self,
            _exec_ctx: &ExecutionContext,
        ) -> Vec<rocode_provider::ToolDefinition> {
            Vec::new()
        }
    }

    fn new_temp_workdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("rocode-workflow-runtime-{}", now_nanos()));
        std::fs::create_dir_all(path.join("src")).expect("temp workdir should create");
        path
    }

    fn init_git_repo(workdir: &Path) {
        run_git(workdir, ["init", "-q"]).expect("git init should succeed");
        run_git(
            workdir,
            ["config", "user.email", "workflow-test@example.com"],
        )
        .expect("git email config should succeed");
        run_git(workdir, ["config", "user.name", "Workflow Test"])
            .expect("git name config should succeed");
        run_git(workdir, ["add", "-A"]).expect("git add should succeed");
        run_git(workdir, ["commit", "-qm", "initial"]).expect("git commit should succeed");
    }

    fn test_exec_ctx(workdir: &Path) -> ExecutionContext {
        ExecutionContext {
            session_id: "workflow-test".to_string(),
            workdir: workdir.display().to_string(),
            agent_name: "hephaestus".to_string(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn workflow_controller_stops_when_objective_is_satisfied() {
        let workdir = new_temp_workdir();
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=12".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let mut controller = WorkflowController::from_config(
            run_workflow_config(),
            ToolRunner::new(executor),
            test_exec_ctx(&workdir),
            None,
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        let result = controller
            .evaluate_round(1, Vec::new())
            .await
            .expect("workflow evaluation should succeed");

        assert_eq!(result.decision, IterationDecision::StopSatisfied);
        assert_eq!(
            result.gate_decision.status,
            SchedulerExecutionGateStatus::Done
        );
        assert!(result
            .output
            .content
            .contains("Domain Decision: stop-satisfied"));

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn workflow_controller_restores_snapshot_and_continues_on_discard() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "baseline\n")
            .expect("baseline file should write");
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=9".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let mut controller = WorkflowController::from_config(
            run_workflow_config(),
            ToolRunner::new(executor),
            test_exec_ctx(&workdir),
            None,
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        controller
            .begin_iteration(1)
            .expect("snapshot capture should succeed");
        std::fs::write(workdir.join("src/lib.rs"), "regressed\n")
            .expect("iteration mutation should write");
        std::fs::write(workdir.join("src/new.rs"), "new file\n").expect("new file should write");
        let result = controller
            .evaluate_round(1, Vec::new())
            .await
            .expect("workflow evaluation should succeed");

        assert_eq!(
            result.decision,
            IterationDecision::Discard {
                reason: DiscardReason::MetricRegressed
            }
        );
        assert_eq!(
            result.gate_decision.status,
            SchedulerExecutionGateStatus::Continue
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join("src/lib.rs")).expect("restored file should read"),
            "baseline\n"
        );
        assert!(!workdir.join("src/new.rs").exists());

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn workflow_controller_persists_iteration_log_summary_and_manifest() {
        let workdir = new_temp_workdir();
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=12".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let exec_ctx = test_exec_ctx(&workdir);
        let session_id = exec_ctx.session_id.clone();
        let mut controller = WorkflowController::from_config(
            workflow_config_with_artifacts(),
            ToolRunner::new(executor),
            exec_ctx,
            Some("autoresearch-run".to_string()),
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        let result = controller
            .evaluate_round(
                1,
                vec![WorkflowModeArtifact {
                    name: "finding-registry".to_string(),
                    description: "Security findings".to_string(),
                    entries: vec![WorkflowModeArtifactEntry {
                        iteration: Some(1),
                        key: "active-finding".to_string(),
                        status: "verified".to_string(),
                        title: "Structured finding".to_string(),
                        detail: "Imported from scheduler execution output.".to_string(),
                        evidence: vec!["structured".to_string(), "file-line".to_string()],
                    }],
                }],
            )
            .await
            .expect("workflow evaluation should succeed");
        controller
            .persist_run_summary(WorkflowRunSummaryRecord {
                iterations_completed: 1,
                final_iteration: Some(1),
                baseline_metric: None,
                best_metric: None,
                final_metric: None,
                kept_commits: Vec::new(),
                best_commit: None,
                squashed_commit: None,
                final_decision: Some(result.decision.label().to_string()),
                final_gate_status: Some(gate_status_label(result.gate_decision.status).to_string()),
                final_summary: Some(result.gate_decision.summary.clone()),
                final_response: result.gate_decision.final_response.clone(),
                mode_report: None,
                mode_artifacts: Vec::new(),
                objective_satisfied: true,
                cancelled: false,
                exhausted_budget: false,
            })
            .expect("summary should persist");

        let run_root = workdir
            .join(".rocode")
            .join("autoresearch")
            .join(session_id);
        let iteration_log =
            std::fs::read_to_string(run_root.join("iterations.tsv")).expect("iteration log");
        assert!(iteration_log.contains("baseline"));
        assert!(iteration_log.contains("stop-satisfied"));
        let summary = std::fs::read_to_string(run_root.join("summary.json")).expect("summary");
        assert!(summary.contains("\"iterations_completed\": 1"));
        let manifest =
            std::fs::read_to_string(run_root.join("run-manifest.json")).expect("manifest");
        assert!(manifest.contains("\"best_metric\": 12.0"));
        assert!(manifest.contains("\"best_commit\""));
        assert!(manifest.contains("\"mode_report\""));
        let mode_artifacts =
            std::fs::read_to_string(run_root.join("mode-artifacts.json")).expect("mode artifacts");
        assert!(mode_artifacts.contains("objective-log"));
        let objective_index_dir = workdir
            .join(".rocode")
            .join("autoresearch")
            .join("objectives")
            .join("autoresearch-run");
        let latest = std::fs::read_to_string(
            std::fs::read_dir(&objective_index_dir)
                .expect("objective index dir")
                .next()
                .expect("objective index file")
                .expect("objective index entry")
                .path(),
        )
        .expect("objective index");
        assert!(latest.contains("\"best_metric\": 12.0"));

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn workflow_controller_loads_baseline_from_last_run_manifest() {
        let workdir = new_temp_workdir();
        let first_executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=12".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let mut first_config = workflow_config_with_artifacts();
        let first_exec_ctx = test_exec_ctx(&workdir);
        let mut first_controller = WorkflowController::from_config(
            first_config.clone(),
            ToolRunner::new(first_executor),
            first_exec_ctx,
            Some("autoresearch-run".to_string()),
        )
        .expect("first controller construction should succeed")
        .expect("workflow should activate controller");
        first_controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        let first_result = first_controller
            .evaluate_round(1, Vec::new())
            .await
            .expect("first workflow evaluation should succeed");
        first_controller
            .persist_run_summary(WorkflowRunSummaryRecord {
                iterations_completed: 1,
                final_iteration: Some(1),
                baseline_metric: None,
                best_metric: None,
                final_metric: None,
                kept_commits: Vec::new(),
                best_commit: None,
                squashed_commit: None,
                final_decision: Some(first_result.decision.label().to_string()),
                final_gate_status: Some(
                    gate_status_label(first_result.gate_decision.status).to_string(),
                ),
                final_summary: Some(first_result.gate_decision.summary.clone()),
                final_response: first_result.gate_decision.final_response.clone(),
                mode_report: None,
                mode_artifacts: Vec::new(),
                objective_satisfied: true,
                cancelled: false,
                exhausted_budget: false,
            })
            .expect("first summary should persist");

        first_config
            .decision_policy
            .as_mut()
            .expect("decision policy")
            .baseline_strategy = Some(BaselineStrategy::FromLastRun);
        let second_executor = Arc::new(ScriptedToolExecutor::default());
        let mut second_controller = WorkflowController::from_config(
            first_config,
            ToolRunner::new(second_executor),
            test_exec_ctx(&workdir),
            Some("autoresearch-run".to_string()),
        )
        .expect("second controller construction should succeed")
        .expect("workflow should activate controller");
        second_controller
            .capture_baseline()
            .await
            .expect("from-last-run baseline should load");

        assert_eq!(
            second_controller
                .evaluator
                .history()
                .baseline
                .as_ref()
                .map(|sample| sample.value),
            Some(12.0)
        );

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn workflow_controller_commits_kept_iterations_via_workspace_service() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "baseline\n")
            .expect("baseline file should write");
        init_git_repo(&workdir);
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=12".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let mut controller = WorkflowController::from_config(
            workflow_config_with_commit_policy(false),
            ToolRunner::new(executor),
            test_exec_ctx(&workdir),
            Some("autoresearch-run".to_string()),
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        controller
            .begin_iteration(1)
            .expect("iteration should begin");
        std::fs::write(workdir.join("src/lib.rs"), "candidate\n")
            .expect("candidate mutation should write");
        let result = controller
            .evaluate_round(1, Vec::new())
            .await
            .expect("workflow evaluation should succeed");
        controller
            .persist_run_summary(WorkflowRunSummaryRecord {
                iterations_completed: 1,
                final_iteration: Some(1),
                baseline_metric: None,
                best_metric: None,
                final_metric: None,
                kept_commits: Vec::new(),
                best_commit: None,
                squashed_commit: None,
                final_decision: Some(result.decision.label().to_string()),
                final_gate_status: Some(gate_status_label(result.gate_decision.status).to_string()),
                final_summary: Some(result.gate_decision.summary.clone()),
                final_response: result.gate_decision.final_response.clone(),
                mode_report: None,
                mode_artifacts: Vec::new(),
                objective_satisfied: true,
                cancelled: false,
                exhausted_budget: false,
            })
            .expect("summary should persist");

        let head_subject =
            run_git(&workdir, ["log", "-1", "--pretty=%s"]).expect("git log should succeed");
        assert!(head_subject.contains("autoresearch iter 1: stop-satisfied"));
        assert_eq!(
            run_git(&workdir, ["rev-list", "--count", "HEAD"]).expect("commit count"),
            "2"
        );
        let status =
            run_git(&workdir, ["status", "--short", "--", "src/lib.rs"]).expect("git status");
        assert!(status.trim().is_empty());

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn workflow_controller_squashes_kept_iteration_commits_on_completion() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "baseline\n")
            .expect("baseline file should write");
        init_git_repo(&workdir);
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=11".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=12".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let mut controller = WorkflowController::from_config(
            workflow_config_with_commit_policy(true),
            ToolRunner::new(executor),
            test_exec_ctx(&workdir),
            Some("autoresearch-run".to_string()),
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        controller
            .begin_iteration(1)
            .expect("iteration one should begin");
        std::fs::write(workdir.join("src/lib.rs"), "candidate-one\n")
            .expect("first candidate mutation should write");
        let first = controller
            .evaluate_round(1, Vec::new())
            .await
            .expect("first workflow evaluation should succeed");
        assert_eq!(first.decision, IterationDecision::Keep);

        controller
            .begin_iteration(2)
            .expect("iteration two should begin");
        std::fs::write(workdir.join("src/lib.rs"), "candidate-two\n")
            .expect("second candidate mutation should write");
        let second = controller
            .evaluate_round(2, Vec::new())
            .await
            .expect("second workflow evaluation should succeed");
        assert_eq!(second.decision, IterationDecision::StopSatisfied);

        controller
            .persist_run_summary(WorkflowRunSummaryRecord {
                iterations_completed: 2,
                final_iteration: Some(2),
                baseline_metric: None,
                best_metric: None,
                final_metric: None,
                kept_commits: Vec::new(),
                best_commit: None,
                squashed_commit: None,
                final_decision: Some(second.decision.label().to_string()),
                final_gate_status: Some(gate_status_label(second.gate_decision.status).to_string()),
                final_summary: Some(second.gate_decision.summary.clone()),
                final_response: second.gate_decision.final_response.clone(),
                mode_report: None,
                mode_artifacts: Vec::new(),
                objective_satisfied: true,
                cancelled: false,
                exhausted_budget: false,
            })
            .expect("summary should persist");

        assert_eq!(
            run_git(&workdir, ["rev-list", "--count", "HEAD"]).expect("commit count"),
            "2"
        );
        let head_subject =
            run_git(&workdir, ["log", "-1", "--pretty=%s"]).expect("git log should succeed");
        assert!(head_subject.contains("autoresearch iter 2: stop-satisfied"));
        let manifest = std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("run-manifest.json"),
        )
        .expect("manifest should read");
        assert!(manifest.contains("\"squashed_commit\""));

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn security_mode_protocol_annotates_gate_and_retry_input() {
        let workdir = new_temp_workdir();
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=11".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let mut controller = WorkflowController::from_config(
            workflow_config_for_security_mode(),
            ToolRunner::new(executor),
            test_exec_ctx(&workdir),
            Some("autoresearch-security".to_string()),
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        let result = controller
            .evaluate_round(
                1,
                vec![WorkflowModeArtifact {
                    name: "finding-registry".to_string(),
                    description: "Security findings".to_string(),
                    entries: vec![WorkflowModeArtifactEntry {
                        iteration: Some(1),
                        key: "active-finding".to_string(),
                        status: "verified".to_string(),
                        title: "Structured finding".to_string(),
                        detail: "Imported from scheduler execution output.".to_string(),
                        evidence: vec!["structured".to_string(), "file-line".to_string()],
                    }],
                }],
            )
            .await
            .expect("workflow evaluation should succeed");

        assert!(result
            .gate_decision
            .summary
            .contains("Security protocol requires evidence-backed findings"));
        assert!(result
            .gate_decision
            .next_input
            .as_deref()
            .unwrap_or_default()
            .contains("finding registry"));

        controller
            .persist_run_summary(WorkflowRunSummaryRecord {
                iterations_completed: 1,
                final_iteration: Some(1),
                baseline_metric: None,
                best_metric: None,
                final_metric: None,
                kept_commits: Vec::new(),
                best_commit: None,
                squashed_commit: None,
                final_decision: Some(result.decision.label().to_string()),
                final_gate_status: Some(gate_status_label(result.gate_decision.status).to_string()),
                final_summary: Some(result.gate_decision.summary.clone()),
                final_response: result.gate_decision.final_response.clone(),
                mode_report: None,
                mode_artifacts: Vec::new(),
                objective_satisfied: false,
                cancelled: false,
                exhausted_budget: false,
            })
            .expect("summary should persist");

        let mode_artifacts = std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("mode-artifacts.json"),
        )
        .expect("mode artifacts should read");
        assert!(mode_artifacts.contains("\"name\": \"finding-registry\""));
        assert!(mode_artifacts.contains("\"key\": \"active-finding\""));
        assert!(
            mode_artifacts.contains("\"status\": \"verified\""),
            "{mode_artifacts}"
        );
        assert!(
            mode_artifacts.contains("\"title\": \"Structured finding\""),
            "{mode_artifacts}"
        );

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[test]
    fn snapshot_engine_restores_files_and_removes_created_paths() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "before\n").expect("baseline file should write");
        let config = run_workflow_config();
        let objective = config.objective.as_ref().expect("objective should exist");
        let engine = SnapshotEngine::new(&config, objective, &test_exec_ctx(&workdir))
            .expect("snapshot engine should construct");

        let mut checkpoint = engine.capture(1).expect("capture should succeed");
        std::fs::write(workdir.join("src/lib.rs"), "after\n").expect("mutated file should write");
        std::fs::write(workdir.join("src/extra.rs"), "extra\n").expect("created file should write");

        engine
            .restore(&mut checkpoint)
            .expect("restore should succeed");

        assert_eq!(
            std::fs::read_to_string(workdir.join("src/lib.rs")).expect("restored file should read"),
            "before\n"
        );
        assert!(!workdir.join("src/extra.rs").exists());

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[test]
    fn workflow_controller_retries_crash_before_blocking() {
        let workdir = new_temp_workdir();
        let mut controller = WorkflowController::from_config(
            run_workflow_config(),
            ToolRunner::new(Arc::new(ScriptedToolExecutor::default())),
            test_exec_ctx(&workdir),
            None,
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .begin_iteration(1)
            .expect("snapshot capture should succeed");
        let retry =
            controller.handle_execution_error(1, &OrchestratorError::Other("boom".to_string()));

        assert_eq!(
            retry.decision,
            IterationDecision::RetryCrash {
                attempt: 1,
                error: "orchestrator error: boom".to_string()
            }
        );
        assert_eq!(
            retry.gate_decision.status,
            SchedulerExecutionGateStatus::Continue
        );

        controller
            .begin_iteration(2)
            .expect("second snapshot capture should succeed");
        let second = controller
            .handle_execution_error(2, &OrchestratorError::Other("still boom".to_string()));
        assert_eq!(
            second.decision,
            IterationDecision::RetryCrash {
                attempt: 2,
                error: "orchestrator error: still boom".to_string()
            }
        );
        assert_eq!(
            second.gate_decision.status,
            SchedulerExecutionGateStatus::Continue
        );

        controller
            .begin_iteration(3)
            .expect("third snapshot capture should succeed");
        let blocked = controller
            .handle_execution_error(3, &OrchestratorError::Other("final boom".to_string()));
        assert!(matches!(
            blocked.decision,
            IterationDecision::StopBlocked { .. }
        ));
        assert_eq!(
            blocked.gate_decision.status,
            SchedulerExecutionGateStatus::Blocked
        );

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[tokio::test]
    async fn patch_file_execution_context_override_still_exposes_workflow_metadata() {
        let workdir = new_temp_workdir();
        let executor = Arc::new(ScriptedToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![Ok(ToolOutput {
                output: "score=10".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            })])),
        });
        let exec_ctx = test_exec_ctx(&workdir);
        let mut controller = WorkflowController::from_config(
            workflow_config_for_security_mode(),
            ToolRunner::new(executor),
            exec_ctx.clone(),
            None,
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        controller
            .begin_iteration(1)
            .expect("iteration should begin");

        let override_ctx = controller
            .execution_context_override(&exec_ctx)
            .expect("patch-file mode should still expose workflow metadata");

        assert_eq!(override_ctx.workdir, exec_ctx.workdir);
        assert_eq!(
            override_ctx
                .metadata
                .get("workflow_mode")
                .and_then(Value::as_str),
            Some("security")
        );
        assert!(override_ctx
            .metadata
            .contains_key("workflow_mode_iteration_brief"));

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[derive(Default)]
    struct RecordingToolExecutor {
        responses: Mutex<VecDeque<Result<ToolOutput, ToolExecError>>>,
        workdirs: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolExecutor for RecordingToolExecutor {
        async fn execute(
            &self,
            tool_name: &str,
            arguments: Value,
            _exec_ctx: &ExecutionContext,
        ) -> Result<ToolOutput, ToolExecError> {
            if tool_name == "bash" {
                if let Some(workdir) = arguments.get("workdir").and_then(Value::as_str) {
                    self.workdirs.lock().await.push(workdir.to_string());
                }
            }
            self.responses
                .lock()
                .await
                .pop_front()
                .expect("scripted tool response should exist")
        }

        async fn list_ids(&self) -> Vec<String> {
            vec!["bash".to_string()]
        }

        async fn list_definitions(
            &self,
            _exec_ctx: &ExecutionContext,
        ) -> Vec<rocode_provider::ToolDefinition> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn worktree_fork_uses_overridden_verify_workdir_and_promotes_kept_changes() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "baseline\n")
            .expect("baseline file should write");
        init_git_repo(&workdir);

        let executor = Arc::new(RecordingToolExecutor {
            responses: Mutex::new(VecDeque::from(vec![
                Ok(ToolOutput {
                    output: "score=10".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
                Ok(ToolOutput {
                    output: "score=12".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
            workdirs: Mutex::new(Vec::new()),
        });
        let mut controller = WorkflowController::from_config(
            workflow_config_with_strategy(SnapshotStrategy::WorktreeFork),
            ToolRunner::new(executor.clone()),
            test_exec_ctx(&workdir),
            None,
        )
        .expect("controller construction should succeed")
        .expect("workflow should activate controller");

        controller
            .capture_baseline()
            .await
            .expect("baseline should capture");
        controller
            .begin_iteration(1)
            .expect("worktree checkpoint should capture");
        let override_ctx = controller
            .execution_context_override(&test_exec_ctx(&workdir))
            .expect("worktree checkpoint should expose override context");
        let worktree_exec_root = PathBuf::from(&override_ctx.workdir);
        assert_ne!(worktree_exec_root, workdir);
        assert_eq!(
            std::fs::read_to_string(worktree_exec_root.join("src/lib.rs"))
                .expect("forked worktree file should read"),
            "baseline\n"
        );

        std::fs::write(worktree_exec_root.join("src/lib.rs"), "candidate\n")
            .expect("candidate change should write");
        std::fs::write(worktree_exec_root.join("src/new.rs"), "new file\n")
            .expect("new candidate file should write");

        let result = controller
            .evaluate_round(1, Vec::new())
            .await
            .expect("workflow evaluation should succeed");

        assert_eq!(result.decision, IterationDecision::StopSatisfied);
        assert_eq!(
            std::fs::read_to_string(workdir.join("src/lib.rs")).expect("promoted file should read"),
            "candidate\n"
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join("src/new.rs")).expect("promoted file should read"),
            "new file\n"
        );
        let recorded_workdirs = executor.workdirs.lock().await.clone();
        assert_eq!(recorded_workdirs.len(), 2);
        assert_eq!(recorded_workdirs[0], workdir.display().to_string());
        assert_eq!(
            recorded_workdirs[1],
            worktree_exec_root.display().to_string()
        );
        assert!(!worktree_exec_root.exists());

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[test]
    fn git_branch_snapshot_discards_isolated_changes_and_cleans_branch() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "baseline\n")
            .expect("baseline file should write");
        init_git_repo(&workdir);

        let config = workflow_config_with_strategy(SnapshotStrategy::GitBranchPerIteration);
        let objective = config.objective.as_ref().expect("objective should exist");
        let engine = SnapshotEngine::new(&config, objective, &test_exec_ctx(&workdir))
            .expect("snapshot engine should construct");

        let mut checkpoint = engine.capture(1).expect("branch checkpoint should capture");
        let branch_exec_root = checkpoint
            .execution_workdir()
            .expect("git branch checkpoint should override execution root")
            .to_path_buf();
        std::fs::write(branch_exec_root.join("src/lib.rs"), "branch-candidate\n")
            .expect("branch candidate should write");
        std::fs::write(branch_exec_root.join("src/branch_only.rs"), "branch only\n")
            .expect("branch-only file should write");

        engine
            .restore(&mut checkpoint)
            .expect("branch checkpoint restore should succeed");

        assert_eq!(
            std::fs::read_to_string(workdir.join("src/lib.rs"))
                .expect("authoritative file should read"),
            "baseline\n"
        );
        assert!(!workdir.join("src/branch_only.rs").exists());
        assert!(!branch_exec_root.exists());
        let branches = run_git(
            &workdir,
            ["branch", "--list", "autoresearch/workflow-test/*"],
        )
        .expect("branch listing should succeed");
        assert!(branches.trim().is_empty());

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }

    #[test]
    fn git_stash_snapshot_restores_dirty_authoritative_workspace() {
        let workdir = new_temp_workdir();
        std::fs::write(workdir.join("src/lib.rs"), "baseline\n")
            .expect("baseline file should write");
        init_git_repo(&workdir);
        std::fs::write(workdir.join("src/lib.rs"), "authoritative dirty\n")
            .expect("authoritative tracked change should write");
        std::fs::write(workdir.join("src/local.rs"), "authoritative untracked\n")
            .expect("authoritative untracked file should write");

        let config = workflow_config_with_strategy(SnapshotStrategy::GitStashStack);
        let objective = config.objective.as_ref().expect("objective should exist");
        let engine = SnapshotEngine::new(&config, objective, &test_exec_ctx(&workdir))
            .expect("snapshot engine should construct");

        let mut checkpoint = engine.capture(1).expect("stash checkpoint should capture");
        std::fs::write(workdir.join("src/lib.rs"), "discard me\n")
            .expect("iteration tracked change should write");
        std::fs::remove_file(workdir.join("src/local.rs"))
            .expect("iteration removal should succeed");
        std::fs::write(workdir.join("src/new.rs"), "iteration new file\n")
            .expect("iteration new file should write");

        engine
            .restore(&mut checkpoint)
            .expect("stash checkpoint restore should succeed");

        assert_eq!(
            std::fs::read_to_string(workdir.join("src/lib.rs"))
                .expect("tracked file should restore"),
            "authoritative dirty\n"
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join("src/local.rs"))
                .expect("untracked file should restore"),
            "authoritative untracked\n"
        );
        assert!(!workdir.join("src/new.rs").exists());

        std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    }
}
