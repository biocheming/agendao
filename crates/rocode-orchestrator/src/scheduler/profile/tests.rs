use super::super::presets::prometheus_planning_stage_tool_policy;
use super::super::profile_state::{
    SchedulerExecutionState, SchedulerMetricsState, SchedulerPresetRuntimeState,
    SchedulerRouteState,
};
use super::*;
use crate::output_metadata::{append_output_usage, output_usage, OutputUsage};
use crate::runtime::events::FinishReason;
use crate::traits::{AgentResolver, ModelResolver, NoopLifecycleHook, ToolExecutor};
use crate::workflow_mode::{mode_artifacts_from_metadata, WORKFLOW_MODE_ARTIFACTS_METADATA_KEY};
use crate::{
    AgentDescriptor, DirectKind, ExecutionContext, ModelRef, Orchestrator, OrchestratorContext,
    ReviewMode, SchedulerEffectKind, SchedulerEffectMoment, SchedulerEffectSpec,
    SchedulerExecutionChildMode, SchedulerExecutionVerificationMode, SchedulerSessionProjection,
    SchedulerTransitionSpec, SchedulerTransitionTarget, SchedulerTransitionTrigger, ToolExecError,
    ToolOutput,
};
use async_trait::async_trait;
use futures::stream;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

fn planner_only_plan() -> SchedulerProfilePlan {
    SchedulerProfilePlan::new(vec![
        SchedulerStageKind::RequestAnalysis,
        SchedulerStageKind::Route,
        SchedulerStageKind::Interview,
        SchedulerStageKind::Plan,
        SchedulerStageKind::Review,
        SchedulerStageKind::Handoff,
    ])
    .with_orchestrator("prometheus")
}

fn runtime_execution_plan(orchestrator: &str) -> SchedulerProfilePlan {
    SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
        .with_orchestrator(orchestrator)
}

fn autoresearch_run_workflow() -> IterativeWorkflowConfig {
    IterativeWorkflowConfig {
        workflow: crate::iterative_workflow::WorkflowDescriptor {
            kind: crate::iterative_workflow::IterativeWorkflowKind::Autoresearch,
            mode: crate::iterative_workflow::IterativeWorkflowMode::Run,
        },
        objective: Some(crate::iterative_workflow::ObjectiveDefinition {
            goal: "Improve the score".to_string(),
            scope: crate::iterative_workflow::ScopeDefinition {
                include: vec!["src/**".to_string()],
                exclude: Vec::new(),
            },
            direction: crate::iterative_workflow::ObjectiveDirection::HigherIsBetter,
            metric: crate::iterative_workflow::MetricDefinition {
                kind: crate::iterative_workflow::MetricKind::NumericExtract,
                pattern: Some("score=(\\d+)".to_string()),
                count_pattern: None,
                json_path: None,
                unit: None,
            },
            verify: crate::iterative_workflow::CommandDefinition {
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
        iteration_policy: Some(crate::iterative_workflow::IterationPolicyDefinition {
            mode: crate::iterative_workflow::IterationMode::Bounded,
            max_iterations: Some(5),
            stop_conditions: Vec::new(),
            stuck_threshold: Some(2),
            progress_report_every: None,
        }),
        decision_policy: Some(crate::iterative_workflow::DecisionPolicyDefinition {
            baseline_strategy: Some(
                crate::iterative_workflow::BaselineStrategy::CaptureBeforeFirstIteration,
            ),
            baseline_value: None,
            keep_conditions: vec![
                crate::iterative_workflow::KeepCondition::MetricImproved,
                crate::iterative_workflow::KeepCondition::VerifyPassed,
            ],
            discard_conditions: vec![
                crate::iterative_workflow::DiscardCondition::MetricRegressed,
                crate::iterative_workflow::DiscardCondition::MetricUnchanged,
                crate::iterative_workflow::DiscardCondition::VerifyFailed,
            ],
            rework_policy: None,
            crash_retry_policy: None,
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
        verifier: None,
        ship: None,
    }
}

#[test]
fn atlas_workflow_policy_uses_coordination_loop_with_required_verification() {
    let workflow = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
        .with_orchestrator("atlas")
        .execution_workflow_policy();

    assert_eq!(
        workflow.kind,
        SchedulerExecutionWorkflowKind::CoordinationLoop
    );
    assert_eq!(workflow.child_mode, SchedulerExecutionChildMode::Parallel);
    assert!(workflow.allow_execution_fallback);
    assert_eq!(
        workflow.verification_mode,
        SchedulerExecutionVerificationMode::Required
    );
    assert_eq!(workflow.max_rounds, 3);
}

#[test]
fn sisyphus_workflow_policy_uses_single_pass_scheduler_loop() {
    let workflow = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
        .with_orchestrator("sisyphus")
        .execution_workflow_policy();

    assert_eq!(workflow.kind, SchedulerExecutionWorkflowKind::SinglePass);
    assert_eq!(workflow.max_rounds, 1);
}

#[test]
fn hephaestus_workflow_policy_uses_autonomous_loop_with_fallback() {
    let workflow = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
        .with_orchestrator("hephaestus")
        .execution_workflow_policy();

    assert_eq!(
        workflow.kind,
        SchedulerExecutionWorkflowKind::AutonomousLoop
    );
    assert_eq!(workflow.child_mode, SchedulerExecutionChildMode::Sequential);
    assert!(workflow.allow_execution_fallback);
    assert_eq!(
        workflow.verification_mode,
        SchedulerExecutionVerificationMode::Required
    );
    assert_eq!(workflow.max_rounds, 3);
}

#[test]
fn finalize_output_prefers_handoff_over_review_and_plan() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "brief".to_string(),
            ..Default::default()
        },
        execution: SchedulerExecutionState {
            reviewed: Some(OrchestratorOutput {
                content: "review".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            handed_off: Some(OrchestratorOutput {
                content: "handoff".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        preset_runtime: SchedulerPresetRuntimeState {
            planned: Some("plan".to_string()),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 3,
            total_tool_calls: 2,
            ..Default::default()
        },
        session_context: None,
        session_context_packet: None,
        is_cancelled: false,
    };

    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("## Plan Summary"));
    assert!(output.content.contains("**Recommended Next Step**"));
    assert!(output.content.contains("- handoff"));
    assert_eq!(output.steps, 3);
    assert_eq!(output.tool_calls_count, 2);
}

#[test]
fn finalize_output_does_not_double_count_recorded_stage_usage() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        runtime_execution_plan("sisyphus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let mut stage_metadata = HashMap::new();
    let stage_usage = OutputUsage {
        prompt_tokens: 100,
        completion_tokens: 25,
        context_tokens: 100,
        reasoning_tokens: 7,
        cache_read_tokens: 3,
        cache_miss_tokens: 0,
        cache_write_tokens: 5,
    };
    append_output_usage(&mut stage_metadata, &stage_usage);
    let stage_output = OrchestratorOutput {
        content: "stage output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: stage_metadata,
        finish_reason: FinishReason::EndTurn,
    };
    let mut state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "brief".to_string(),
            ..Default::default()
        },
        execution: SchedulerExecutionState {
            delegated: Some(stage_output.clone()),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    SchedulerProfileOrchestrator::record_output(&mut state, &stage_output);

    let output = orchestrator.finalize_output(state);
    let usage = output_usage(&output.metadata).expect("final output should preserve stage usage");

    assert_eq!(usage.prompt_tokens, 100);
    assert_eq!(usage.completion_tokens, 25);
    assert_eq!(usage.context_tokens, 100);
    assert_eq!(usage.reasoning_tokens, 7);
    assert_eq!(usage.cache_read_tokens, 3);
    assert_eq!(usage.cache_write_tokens, 5);
}

#[test]
fn finalize_output_merges_workflow_mode_artifacts_with_stage_precedence() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        runtime_execution_plan("atlas"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let stage_output = |stage: &str, iteration: u32, status: &str| OrchestratorOutput {
        content: format!("{stage} output"),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::from([(
            WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
            serde_json::json!([{
                "name": "finding-registry",
                "description": format!("{stage} registry"),
                "entries": [{
                    "iteration": iteration,
                    "key": "shared-finding",
                    "status": status,
                    "title": format!("{stage} title"),
                    "detail": format!("{stage} detail"),
                    "evidence": [format!("{stage}-evidence")]
                }]
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            delegated: Some(stage_output("delegated", 1, "open")),
            reviewed: Some(stage_output("reviewed", 2, "triaged")),
            handed_off: Some(stage_output("handed_off", 3, "ready")),
            synthesized: Some(stage_output("synthesized", 4, "verified")),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 4,
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    let artifacts = mode_artifacts_from_metadata(&output.metadata);
    let finding = artifacts
        .iter()
        .find(|artifact| artifact.name == "finding-registry")
        .and_then(|artifact| {
            artifact
                .entries
                .iter()
                .find(|entry| entry.key == "shared-finding")
        })
        .expect("shared finding should exist");

    assert_eq!(finding.iteration, Some(4));
    assert_eq!(finding.status, "verified");
    assert_eq!(finding.title, "synthesized title");
    assert_eq!(finding.detail, "synthesized detail");
    assert!(finding.evidence.contains(&"delegated-evidence".to_string()));
    assert!(finding.evidence.contains(&"reviewed-evidence".to_string()));
    assert!(finding
        .evidence
        .contains(&"handed_off-evidence".to_string()));
    assert!(finding
        .evidence
        .contains(&"synthesized-evidence".to_string()));
}

#[test]
fn finalize_output_normalizes_sisyphus_delivery_shape() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("sisyphus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            delegated: Some(OrchestratorOutput {
                content: "Shipped the change and verified the targeted behavior.".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("## Delivery Summary"));
    assert!(output.content.contains("**Delegation Path**"));
    assert!(output.content.contains("**Execution Outcome**"));
    assert!(output.content.contains("**Verification**"));
}

#[test]
fn sisyphus_final_output_preserves_structured_execution_over_generic_gate_response() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("sisyphus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let execution = OrchestratorOutput {
        content: "## Delivery Summary\nResearch complete.\n\n**Execution Outcome**\n# AlphaFold3 methodology survey\n\n- Protenix\n- Boltz-2\n\n**Verification**\n- Evidence-backed search results collected.".to_string(),
        steps: 1,
        tool_calls_count: 2,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Done,
        summary: "verified".to_string(),
        next_input: None,
        final_response: Some(
            "所有任务已完成。如需深入了解其中任何一个方向的细节，请告诉我。".to_string(),
        ),
    };
    let terminal_output = SchedulerProfileOrchestrator::gate_terminal_output(
        &orchestrator.plan,
        SchedulerExecutionGateStatus::Done,
        &decision,
        &execution,
        None,
    )
    .expect("done gate should produce terminal output");
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            delegated: Some(terminal_output),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 2,
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("AlphaFold3 methodology survey"));
    assert!(output.content.contains("Protenix"));
    assert!(output.content.contains("Boltz-2"));
    assert!(!output.content.contains(
        "所有任务已完成。如需深入了解其中任何一个方向的细节，请告诉我。\n\n**Delegation Path**"
    ));
}

#[test]
fn gate_terminal_output_preserves_structured_execution_for_atlas_generic_gate_response() {
    let plan = runtime_execution_plan("atlas");
    let execution = OrchestratorOutput {
        content: "## Delivery Summary\nResearch complete.\n\n**Task Status**\n- AlphaFold3 method survey finished.\n\n**Verification**\n- Evidence-backed search results collected.".to_string(),
        steps: 1,
        tool_calls_count: 2,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Done,
        summary: "verified".to_string(),
        next_input: None,
        final_response: Some("所有任务已完成。".to_string()),
    };

    let output = SchedulerProfileOrchestrator::gate_terminal_output(
        &plan,
        SchedulerExecutionGateStatus::Done,
        &decision,
        &execution,
        None,
    )
    .expect("done gate should produce terminal output");

    assert!(output.content.contains("AlphaFold3 method survey finished"));
    assert!(!output.content.contains("所有任务已完成。"));
}

#[test]
fn gate_terminal_output_uses_default_preservation_for_custom_scheduler() {
    let plan = runtime_execution_plan("custom-research-scheduler");
    let execution = OrchestratorOutput {
        content: "## Delivery Summary\nResearch complete.\n\n**Execution Outcome**\n- Found concrete AlphaFold3 methodology papers.\n\n**Verification**\n- Checked returned paper metadata.".to_string(),
        steps: 1,
        tool_calls_count: 2,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Done,
        summary: "verified".to_string(),
        next_input: None,
        final_response: Some("Done.".to_string()),
    };

    let output = SchedulerProfileOrchestrator::gate_terminal_output(
        &plan,
        SchedulerExecutionGateStatus::Done,
        &decision,
        &execution,
        None,
    )
    .expect("custom scheduler should use default terminal gate behavior");

    assert!(output
        .content
        .contains("Found concrete AlphaFold3 methodology papers"));
    assert!(!output.content.contains("Done."));
}

#[test]
fn finalize_output_normalizes_atlas_delivery_shape() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::Synthesis]).with_orchestrator("atlas"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            synthesized: Some(OrchestratorOutput {
                content: "Task A done. Task B verified.".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("## Delivery Summary"));
    assert!(output.content.contains("**Task Status**"));
    assert!(output.content.contains("**Verification**"));
    assert!(output.content.contains("**Gate Decision**"));
}

#[test]
fn finalize_output_normalizes_hephaestus_delivery_shape() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("hephaestus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            delegated: Some(OrchestratorOutput {
                content: "Fixed the diagnostics path and ran the targeted check.".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("## Delivery Summary"));
    assert!(output.content.contains("**Completion Status**"));
    assert!(output.content.contains("**What Changed**"));
    assert!(output.content.contains("**Verification**"));
}

#[test]
fn gate_terminal_output_merges_supplemental_workflow_mode_artifacts() {
    let plan = runtime_execution_plan("hephaestus");
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Done,
        summary: "objective satisfied".to_string(),
        next_input: None,
        final_response: None,
    };
    let fallback_output = OrchestratorOutput {
        content: "executor output".to_string(),
        steps: 2,
        tool_calls_count: 1,
        metadata: HashMap::from([(
            WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
            serde_json::json!([{
                "name": "finding-registry",
                "description": "base",
                "entries": [{
                    "iteration": 1,
                    "key": "active-finding",
                    "status": "open",
                    "title": "Open finding",
                    "detail": "base detail",
                    "evidence": ["attack-scenario"]
                }]
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };
    let review_output = OrchestratorOutput {
        content: "verification output".to_string(),
        steps: 3,
        tool_calls_count: 2,
        metadata: HashMap::from([(
            WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
            serde_json::json!([{
                "name": "finding-registry",
                "description": "reviewed",
                "entries": [{
                    "iteration": 2,
                    "key": "active-finding",
                    "status": "verified",
                    "title": "Verified finding",
                    "detail": "review detail",
                    "evidence": ["file-line"]
                }]
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };

    let output = SchedulerProfileOrchestrator::gate_terminal_output(
        &plan,
        SchedulerExecutionGateStatus::Done,
        &decision,
        &fallback_output,
        Some(&review_output),
    )
    .expect("terminal output should resolve");

    let artifacts = output
        .metadata
        .get(WORKFLOW_MODE_ARTIFACTS_METADATA_KEY)
        .expect("workflow mode artifacts should exist")
        .to_string();
    assert!(artifacts.contains("\"status\":\"verified\""));
    assert!(artifacts.contains("attack-scenario"));
    assert!(artifacts.contains("file-line"));
    assert_eq!(output.steps, 5);
    assert_eq!(output.tool_calls_count, 3);
}

#[test]
fn retry_budget_exhausted_output_preserves_review_workflow_mode_artifacts() {
    let plan = runtime_execution_plan("atlas");
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Continue,
        summary: "need more evidence".to_string(),
        next_input: Some("verify the unresolved item".to_string()),
        final_response: None,
    };
    let previous_output = OrchestratorOutput {
        content: "executor output".to_string(),
        steps: 1,
        tool_calls_count: 1,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let review_output = OrchestratorOutput {
        content: "verification output".to_string(),
        steps: 1,
        tool_calls_count: 1,
        metadata: HashMap::from([(
            WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
            serde_json::json!([{
                "name": "ship-checklist",
                "description": "Ship checklist",
                "entries": [{
                    "iteration": 2,
                    "key": "dry-run",
                    "status": "passed",
                    "title": "Dry-run",
                    "detail": "Dry-run succeeded",
                    "evidence": ["verify_passed=true"]
                }]
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };

    let output = SchedulerProfileOrchestrator::retry_budget_exhausted_output(
        &plan,
        3,
        3,
        &decision,
        &previous_output,
        Some(&review_output),
    );

    let artifacts = output
        .metadata
        .get(WORKFLOW_MODE_ARTIFACTS_METADATA_KEY)
        .expect("workflow mode artifacts should exist")
        .to_string();
    assert!(artifacts.contains("ship-checklist"));
    assert!(artifacts.contains("\"status\":\"passed\""));
}

#[derive(Default)]
struct ScriptedBashToolExecutor {
    calls: Mutex<Vec<String>>,
    responses: Mutex<VecDeque<Result<ToolOutput, ToolExecError>>>,
}

#[async_trait]
impl ToolExecutor for ScriptedBashToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        _exec_ctx: &crate::ExecutionContext,
    ) -> Result<ToolOutput, ToolExecError> {
        if tool_name != "bash" {
            return Err(ToolExecError::ExecutionError(format!(
                "unexpected tool call: {tool_name}"
            )));
        }
        if let Some(command) = arguments.get("command").and_then(serde_json::Value::as_str) {
            self.calls.lock().await.push(command.to_string());
        }
        self.responses
            .lock()
            .await
            .pop_front()
            .expect("scripted bash response should exist")
    }

    async fn list_ids(&self) -> Vec<String> {
        vec!["bash".to_string()]
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &crate::ExecutionContext,
    ) -> Vec<rocode_provider::ToolDefinition> {
        Vec::new()
    }
}

#[tokio::test]
async fn hephaestus_workflow_uses_mechanical_gate_without_consuming_gate_model_stream() {
    let workdir = new_temp_workdir();
    let tool_executor = Arc::new(ScriptedBashToolExecutor {
        calls: Mutex::new(Vec::new()),
        responses: Mutex::new(VecDeque::from(vec![
            Ok(ToolOutput {
                output: "score=10".to_string(),
                is_error: false,
                title: None,
                metadata: Some(serde_json::json!({"exit_code": 0})),
            }),
            Ok(ToolOutput {
                output: "score=12".to_string(),
                is_error: false,
                title: None,
                metadata: Some(serde_json::json!({"exit_code": 0})),
            }),
        ])),
    });
    let context = test_context_with_executor(
        &workdir,
        "workflow-gate-session",
        vec![
            stream_from_text("## Verification\n- Mechanical score improved."),
            stream_from_text("Implemented the bounded change and ran the targeted check."),
        ],
        tool_executor.clone(),
    );
    let runner = ToolRunner::new(tool_executor.clone());
    let mut orchestrator = SchedulerProfileOrchestrator::new(
        runtime_execution_plan("hephaestus").with_workflow(autoresearch_run_workflow()),
        runner,
    );

    let output = orchestrator
        .execute("Improve the score", &context)
        .await
        .expect("workflow-backed hephaestus execution should succeed without a gate model stream");

    let calls = tool_executor.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec!["cargo test".to_string(), "cargo test".to_string()]
    );
    assert!(output.content.contains("## Delivery Summary"));
    assert!(output.content.contains("**Completion Status**"));
    assert!(output.content.contains("Objective satisfied"));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[test]
fn atlas_synthesis_input_uses_preset_authority() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::Synthesis]).with_orchestrator("atlas"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Coordinate remaining tasks".to_string(),
            route_decision: Some(RouteDecision {
                mode: RouteMode::Orchestrate,
                direct_kind: None,
                direct_response: None,
                preset: Some("atlas".to_string()),
                insert_plan_stage: None,
                review_mode: None,
                context_append: None,
                rationale_summary: "coordination-heavy task list".to_string(),
            }),
            ..Default::default()
        },
        execution: SchedulerExecutionState {
            delegated: Some(OrchestratorOutput {
                content: "worker claims task A done".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            reviewed: Some(OrchestratorOutput {
                content: "task A verified".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        preset_runtime: SchedulerPresetRuntimeState {
            planned: Some(
                "- task A
- task B"
                    .to_string(),
            ),
            ..Default::default()
        },
        ..Default::default()
    };

    let input = orchestrator.compose_synthesis_input(
        "ship the migration cleanup plan",
        &state,
        &orchestrator.plan,
    );
    assert!(input.contains("## Delivery Summary"));
    assert!(input.contains("prefer reviewed verification over worker claims"));
}

#[test]
fn atlas_coordination_verification_input_uses_preset_authority() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("atlas"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Coordinate remaining tasks".to_string(),
            ..Default::default()
        },
        preset_runtime: SchedulerPresetRuntimeState {
            planned: Some(
                "- task A
- task B"
                    .to_string(),
            ),
            ..Default::default()
        },
        ..Default::default()
    };
    let execution = OrchestratorOutput {
        content: "worker round output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::from([(
            "continuationTargets".to_string(),
            serde_json::json!([{
                "sessionId": "task_build_42",
                "agentTaskId": "agent-task-42",
                "toolName": "task_flow"
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };

    let input = orchestrator.compose_coordination_verification_input(
        "ship the migration cleanup plan",
        &state,
        &orchestrator.plan,
        2,
        &execution,
    );
    assert!(input.contains("Audit each Atlas task item individually"));
    assert!(input.contains("task boundary"));
}

#[test]
fn atlas_coordination_gate_input_uses_preset_authority() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("atlas"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Coordinate remaining tasks".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let execution = OrchestratorOutput {
        content: "worker round output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::from([(
            "continuationTargets".to_string(),
            serde_json::json!([{
                "sessionId": "task_build_42",
                "agentTaskId": "agent-task-42",
                "toolName": "task_flow"
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };
    let review = OrchestratorOutput {
        content: "task A verified, task B weak".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };

    let input = orchestrator.compose_coordination_gate_input(
        "ship the migration cleanup plan",
        &state,
        &orchestrator.plan,
        2,
        &execution,
        Some(&review),
    );
    assert!(input.contains("Judge completion by task boundary"));
    assert!(input.contains("weakly-verified task items"));
}

#[test]
fn atlas_retry_input_uses_preset_authority() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("atlas"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Coordinate remaining tasks".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let execution = OrchestratorOutput {
        content: "worker round output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::from([(
            "continuationTargets".to_string(),
            serde_json::json!([{
                "sessionId": "task_build_42",
                "agentTaskId": "agent-task-42",
                "toolName": "task_flow"
            }]),
        )]),
        finish_reason: FinishReason::EndTurn,
    };
    let review = OrchestratorOutput {
        content: "task A verified, task B weak".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Continue,
        summary: "task B still needs concrete verification".to_string(),
        next_input: Some("continue task B and verify the migration path".to_string()),
        final_response: None,
    };

    let input = orchestrator.compose_retry_input(super::RetryComposeRequest {
        original_input: "ship the migration cleanup plan",
        state: &state,
        plan: &orchestrator.plan,
        round: 2,
        decision: &decision,
        previous_output: &execution,
        review_output: Some(&review),
    });

    assert!(input.contains("## Stage\ncoordination-retry"));
    assert!(input.contains("Continuation Authority"));
    assert!(input.contains("active boulder state"));
    assert!(input.contains("Carry forward inherited notepad decisions"));
    assert!(input.contains("Preferred Continuation"));
    assert!(input.contains("task_build_42"));
    assert!(input.contains("agent-task-42"));
}

#[test]
fn hephaestus_autonomous_verification_input_uses_preset_authority() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("hephaestus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Autonomously fix the diagnostics path".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let execution = OrchestratorOutput {
        content: "fixed the path".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };

    let input = orchestrator.compose_autonomous_verification_input(
        "fix the failing lsp diagnostics path",
        &state,
        &orchestrator.plan,
        1,
        &execution,
    );
    assert!(input.contains("proof of EXPLORE -> PLAN -> DECIDE -> EXECUTE -> VERIFY"));
    assert!(input.contains("changed artifacts"));
}

#[test]
fn hephaestus_autonomous_gate_input_uses_preset_authority() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("hephaestus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Autonomously fix the diagnostics path".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let execution = OrchestratorOutput {
        content: "fixed the path".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let verification = OrchestratorOutput {
        content: "targeted check passed".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };

    let input = orchestrator.compose_autonomous_gate_input(
        "fix the failing lsp diagnostics path",
        &state,
        &orchestrator.plan,
        1,
        &execution,
        Some(&verification),
    );
    assert!(input.contains("proved completion"));
    assert!(input.contains("bounded retry"));
    assert!(input.contains("**What Changed**"));
}

#[test]
fn hephaestus_done_gate_prefers_execution_output_when_final_response_missing() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("hephaestus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let execution = OrchestratorOutput {
        content: "fixed the diagnostics path and ran the targeted check".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Done,
        summary: "verified".to_string(),
        next_input: None,
        final_response: None,
    };

    let output = SchedulerProfileOrchestrator::gate_terminal_output(
        &orchestrator.plan,
        SchedulerExecutionGateStatus::Done,
        &decision,
        &execution,
        None,
    )
    .expect("done gate should resolve execution output");

    assert_eq!(
        output.content,
        "fixed the diagnostics path and ran the targeted check"
    );
}

#[test]
fn sisyphus_done_gate_prefers_execution_output_when_final_response_missing() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("sisyphus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let execution = OrchestratorOutput {
        content: "shipped the change and verified the targeted behavior".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let decision = SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Done,
        summary: "verified".to_string(),
        next_input: None,
        final_response: None,
    };

    let output = SchedulerProfileOrchestrator::gate_terminal_output(
        &orchestrator.plan,
        SchedulerExecutionGateStatus::Done,
        &decision,
        &execution,
        None,
    )
    .expect("done gate should resolve execution output");

    assert_eq!(
        output.content,
        "shipped the change and verified the targeted behavior"
    );
}

#[test]
fn render_stage_sequence_includes_interview_and_handoff() {
    let rendered = render_stage_sequence(&[
        SchedulerStageKind::RequestAnalysis,
        SchedulerStageKind::Route,
        SchedulerStageKind::Interview,
        SchedulerStageKind::Plan,
        SchedulerStageKind::Review,
        SchedulerStageKind::Handoff,
    ]);

    assert_eq!(
        rendered,
        "request-analysis -> route -> interview -> plan -> review -> handoff"
    );
}

#[test]
fn constrain_route_decision_keeps_prometheus_workflow() {
    let decision = RouteDecision {
        mode: RouteMode::Orchestrate,
        direct_kind: None,
        direct_response: None,
        preset: Some("sisyphus".to_string()),
        insert_plan_stage: Some(false),
        review_mode: Some(ReviewMode::Skip),
        context_append: None,
        rationale_summary: "switch presets".to_string(),
    };

    let constrained = SchedulerPresetKind::Prometheus
        .definition()
        .constrain_route_decision(decision);

    assert_eq!(constrained.preset.as_deref(), Some("prometheus"));
    assert_eq!(constrained.mode, RouteMode::Orchestrate);
    assert_eq!(constrained.review_mode, Some(ReviewMode::Normal));
}

#[test]
fn constrain_route_decision_forces_prometheus_direct_reply_back_into_workflow() {
    let decision = RouteDecision {
        mode: RouteMode::Direct,
        direct_kind: Some(DirectKind::Reply),
        direct_response: Some("Hi!".to_string()),
        preset: None,
        insert_plan_stage: None,
        review_mode: None,
        context_append: None,
        rationale_summary: "greeting".to_string(),
    };

    let constrained = SchedulerPresetKind::Prometheus
        .definition()
        .constrain_route_decision(decision);

    assert_eq!(constrained.mode, RouteMode::Orchestrate);
    assert_eq!(constrained.preset.as_deref(), Some("prometheus"));
    assert_eq!(constrained.review_mode, Some(ReviewMode::Normal));
    assert_eq!(constrained.direct_kind, None);
    assert_eq!(constrained.direct_response, None);
}

#[test]
fn request_analysis_input_includes_prometheus_workflow_constraint() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );

    let input = orchestrator.compose_request_analysis_input("Plan the TUI workflow polish");

    assert!(input.contains("## Workflow Constraint"));
    assert!(input.contains("planner-only behavior"));
    assert!(input.contains("Do NOT convert this request into a direct reply"));
}

#[test]
fn request_analysis_input_can_include_session_continuity_context() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );

    let input = orchestrator.compose_request_analysis_input_with_context(
        "把你前面检索的结果写到 markdown 文档中",
        Some("## Session Continuity Context\nprevious assistant result"),
    );

    assert!(input.contains("## Session Continuity Context"));
    assert!(input.contains("previous assistant result"));
    assert!(input.contains("把你前面检索的结果写到 markdown 文档中"));
}

#[test]
fn request_analysis_input_includes_skills_guidance_when_skills_are_available() {
    let mut plan = planner_only_plan();
    plan.skill_list = vec![SchedulerSkillRef {
        name: "docker-debug".to_string(),
        description: "Troubleshoot Docker build and runtime failures".to_string(),
        category: Some("ops".to_string()),
    }];
    let orchestrator =
        SchedulerProfileOrchestrator::new(plan, ToolRunner::new(Arc::new(NoopToolExecutor)));

    let input = orchestrator.compose_request_analysis_input("Debug the container startup issue");

    assert!(input.contains("## Skills"));
    assert!(input.contains("<available_skills compact=\"true\">"));
    assert!(input.contains("After completing a complex task (5+ tool calls)"));
    assert!(input.contains("patch it immediately with `skill_manage`"));
}

#[test]
fn request_analysis_input_includes_omo_phase_zero_taxonomy() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::RequestAnalysis])
            .with_orchestrator("sisyphus"),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );

    let input = orchestrator.compose_request_analysis_input("what do you think about tool search?");

    assert!(input.contains("## Intent Gate Reference"));
    assert!(input.contains("Use the same Phase 0 taxonomy as oh-my-opencode"));
    assert!(input.contains("Research/understanding"));
    assert!(input.contains("Implementation"));
    assert!(input.contains("Investigation"));
    assert!(input.contains("Evaluation"));
    assert!(input.contains("Fix"));
    assert!(input.contains("Open-ended change"));
    assert!(input.contains("Trivial, Explicit, Exploratory, Open-ended, Ambiguous"));
}

#[tokio::test]
async fn request_analysis_short_circuits_obvious_social_before_route() {
    let workdir = new_temp_workdir();
    let context = test_context(&workdir, "direct-social-session", Vec::new());
    let runner = ToolRunner::new(Arc::new(NoopToolExecutor));
    let mut orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![
            SchedulerStageKind::RequestAnalysis,
            SchedulerStageKind::Route,
            SchedulerStageKind::ExecutionOrchestration,
        ])
        .with_orchestrator("sisyphus"),
        runner,
    );

    let output = orchestrator
        .execute("Hi", &context)
        .await
        .expect("obvious social input should not require a route model stream");

    assert!(output.content.contains("Hi"));
    assert_eq!(output.steps, 0);
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[test]
fn request_analysis_respects_route_constraints() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );

    let analysis = orchestrator.analyze_request_stage("Hi", None, &orchestrator.plan);

    assert!(analysis.direct_decision.is_none());
    assert_eq!(analysis.request_type, RequestType::Explicit);
}

#[test]
fn review_input_uses_compact_skill_catalog_without_full_descriptions() {
    let mut plan = planner_only_plan();
    plan.stage_skill_lists.insert(
        SchedulerStageKind::Review,
        vec![SchedulerSkillRef {
            name: "deep-review".to_string(),
            description: "Short review hint. Full private review workflow with long step-by-step details should stay behind skill_view.".to_string(),
            category: Some("review".to_string()),
        }],
    );
    let orchestrator =
        SchedulerProfileOrchestrator::new(plan, ToolRunner::new(Arc::new(NoopToolExecutor)));
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            request_brief: "Review the implementation".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    let input = orchestrator.compose_review_input("Review this patch", &state, &orchestrator.plan);

    assert!(input.contains("<available_skills compact=\"true\">"));
    assert!(input.contains("deep-review: Short review hint"));
    assert!(!input.contains("Full private review workflow"));
    assert!(input.contains("Use `skill_view(name)`"));
}

#[test]
fn prometheus_stage_graph_exposes_planner_workflow_and_handoff_finalization() {
    let plan = planner_only_plan();
    let graph = plan.stage_graph();
    assert_eq!(
        graph.stage_kinds(),
        vec![
            SchedulerStageKind::RequestAnalysis,
            SchedulerStageKind::Route,
            SchedulerStageKind::Interview,
            SchedulerStageKind::Plan,
            SchedulerStageKind::Review,
            SchedulerStageKind::Handoff,
        ]
    );
    let review = graph
        .stage(SchedulerStageKind::Review)
        .expect("review stage spec");
    assert_eq!(
        review.policy.tool_policy,
        prometheus_planning_stage_tool_policy()
    );
    assert_eq!(review.policy.loop_budget, SchedulerLoopBudget::Unbounded);
    assert_eq!(
        plan.finalization_mode(),
        SchedulerFinalizationMode::PlannerHandoff
    );
}

#[test]
fn prometheus_flow_definition_exposes_handoff_loop_back_to_plan() {
    let plan = planner_only_plan();
    let transitions = plan.transition_graph();
    assert!(transitions.transitions.contains(&SchedulerTransitionSpec {
        from: SchedulerStageKind::Interview,
        trigger: SchedulerTransitionTrigger::OnSuccess,
        to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Plan),
    }));
    assert!(transitions.transitions.contains(&SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnUserChoice("High Accuracy Review"),
        to: SchedulerTransitionTarget::Stage(SchedulerStageKind::Plan),
    }));
    assert!(transitions.transitions.contains(&SchedulerTransitionSpec {
        from: SchedulerStageKind::Handoff,
        trigger: SchedulerTransitionTrigger::OnUserChoice("Start Work"),
        to: SchedulerTransitionTarget::Finish,
    }));
}

#[test]
fn prometheus_effect_protocol_exposes_artifact_and_handoff_effects() {
    let plan = planner_only_plan();
    let effects = plan.effect_protocol();
    assert!(effects.effects.contains(&SchedulerEffectSpec {
        stage: SchedulerStageKind::Interview,
        moment: SchedulerEffectMoment::OnSuccess,
        effect: SchedulerEffectKind::SyncDraftArtifact,
    }));
    assert!(effects.effects.contains(&SchedulerEffectSpec {
        stage: SchedulerStageKind::Plan,
        moment: SchedulerEffectMoment::OnEnter,
        effect: SchedulerEffectKind::RequestAdvisoryReview,
    }));
    assert!(effects.effects.contains(&SchedulerEffectSpec {
        stage: SchedulerStageKind::Handoff,
        moment: SchedulerEffectMoment::BeforeTransition,
        effect: SchedulerEffectKind::RunApprovalReviewLoop,
    }));
    assert!(effects.effects.contains(&SchedulerEffectSpec {
        stage: SchedulerStageKind::Handoff,
        moment: SchedulerEffectMoment::OnSuccess,
        effect: SchedulerEffectKind::DecorateFinalOutput,
    }));
}

#[test]
fn prometheus_runtime_transition_finishes_on_start_work_choice() {
    let plan = planner_only_plan();
    let orchestrator = SchedulerProfileOrchestrator::new(
        plan.clone(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let handoff_index = plan
        .stages
        .iter()
        .position(|stage| *stage == SchedulerStageKind::Handoff)
        .expect("handoff stage index");
    let state = SchedulerProfileState {
        preset_runtime: SchedulerPresetRuntimeState {
            user_choice: Some("Start Work".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };

    assert_eq!(
        orchestrator.next_stage_index(SchedulerStageKind::Handoff, handoff_index, &state, &plan,),
        None,
    );
}

#[test]
fn prometheus_runtime_transition_loops_back_to_plan_when_high_accuracy_blocked() {
    let plan = planner_only_plan();
    let orchestrator = SchedulerProfileOrchestrator::new(
        plan.clone(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let handoff_index = plan
        .stages
        .iter()
        .position(|stage| *stage == SchedulerStageKind::Handoff)
        .expect("handoff stage index");
    let plan_index = plan
        .stages
        .iter()
        .position(|stage| *stage == SchedulerStageKind::Plan)
        .expect("plan stage index");
    let state = SchedulerProfileState {
        preset_runtime: SchedulerPresetRuntimeState {
            user_choice: Some("High Accuracy Review".to_string()),
            review_gate_approved: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };

    assert_eq!(
        orchestrator.next_stage_index(SchedulerStageKind::Handoff, handoff_index, &state, &plan,),
        Some(plan_index),
    );
}

#[test]
fn synthesis_stage_is_projected_to_session_for_public_presets() {
    let plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::Synthesis])
        .with_orchestrator("sisyphus");
    let graph = plan.stage_graph();
    let synthesis = graph
        .stage(SchedulerStageKind::Synthesis)
        .expect("synthesis stage spec");
    assert!(synthesis.policy.session_projection.is_visible());
    assert_eq!(synthesis.policy.tool_policy, StageToolPolicy::DisableAll);
    assert_eq!(
        synthesis.policy.loop_budget,
        SchedulerLoopBudget::StepLimit(1)
    );
}

#[test]
fn stage_observability_exposes_policy_matrix_for_public_presets() {
    let prometheus_handoff =
        SchedulerPresetKind::Prometheus.stage_observability(SchedulerStageKind::Handoff);
    assert_eq!(prometheus_handoff.projection, "transcript");
    assert_eq!(
        prometheus_handoff.tool_policy,
        "restricted:prometheus-planning-artifacts"
    );
    assert_eq!(prometheus_handoff.loop_budget, "step-limit:1");

    let atlas_execution =
        SchedulerPresetKind::Atlas.stage_observability(SchedulerStageKind::ExecutionOrchestration);
    assert_eq!(atlas_execution.projection, "transcript");
    assert_eq!(atlas_execution.tool_policy, "allow-all");
    assert_eq!(atlas_execution.loop_budget, "unbounded");
}

#[test]
fn normalize_execution_gate_decision_backfills_bounded_retry_focus() {
    let normalized = normalize_execution_gate_decision(SchedulerExecutionGateDecision {
        status: SchedulerExecutionGateStatus::Continue,
        summary: "verify the remaining diagnostics proof".to_string(),
        next_input: Some("   ".to_string()),
        final_response: Some("  ".to_string()),
    });

    assert_eq!(
        normalized.next_input.as_deref(),
        Some("verify the remaining diagnostics proof")
    );
    assert!(normalized.final_response.is_none());
}

#[test]
fn parse_execution_gate_decision_accepts_legacy_atlas_gate_shape() {
    let output = r#"```json
{
  "gate_decision": "done",
  "reasoning": "All delegated work verified complete.",
  "verification_summary": {
"total_tasks": 8,
"completed": 8
  },
  "task_status": {
"task_1": "done - verified"
  },
  "execution_fidelity": "correct - planner-only workflow preserved",
  "minor_issues": ["Task wording mismatch"],
  "next_actions": ["No further execution needed"]
}
```"#;

    let decision =
        parse_execution_gate_decision(output).expect("legacy gate decision should parse");
    assert_eq!(decision.status, SchedulerExecutionGateStatus::Done);
    assert_eq!(decision.summary, "All delegated work verified complete.");
    let final_response = decision
        .final_response
        .expect("legacy gate should synthesize detail section");
    assert!(final_response.contains("Verification Summary"));
    assert!(final_response.contains("Task Status"));
    assert!(final_response.contains("Execution Fidelity"));
    assert!(final_response.contains("Minor Issues"));
}

#[test]
fn retry_budget_exhausted_output_marks_explicit_terminal_state() {
    let execution = OrchestratorOutput {
        content: "worker result".to_string(),
        steps: 2,
        tool_calls_count: 1,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let verification = OrchestratorOutput {
        content: "verification evidence".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: FinishReason::EndTurn,
    };
    let output = SchedulerProfileOrchestrator::retry_budget_exhausted_output(
        &SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
            .with_orchestrator("atlas"),
        3,
        3,
        &SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Continue,
            summary: "task B still lacks proof".to_string(),
            next_input: None,
            final_response: None,
        },
        &execution,
        Some(&verification),
    );

    assert!(output
        .content
        .contains("exhausted its bounded retry budget"));
    assert!(output.content.contains("task B still lacks proof"));
    assert!(output.content.contains("verification evidence"));
    assert_eq!(
        output
            .metadata
            .get("scheduler_retry_budget_exhausted")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn sisyphus_effect_dispatch_uses_shared_scheduler_protocol() {
    let plan =
        SchedulerProfilePlan::new(vec![SchedulerStageKind::Review]).with_orchestrator("sisyphus");
    let dispatch = plan.effect_dispatch(
        SchedulerEffectKind::RequestAdvisoryReview,
        SchedulerEffectContext {
            planning_artifact_path: None,
            draft_artifact_path: None,
            user_choice: None,
            review_gate_approved: None,
            draft_exists: true,
        },
    );

    assert_eq!(dispatch, SchedulerEffectDispatch::RequestAdvisoryReview);
}

#[test]
fn atlas_effect_dispatch_uses_shared_scheduler_protocol() {
    let plan =
        SchedulerProfilePlan::new(vec![SchedulerStageKind::Review]).with_orchestrator("atlas");
    let dispatch = plan.effect_dispatch(
        SchedulerEffectKind::PersistPlanningArtifact,
        SchedulerEffectContext {
            planning_artifact_path: Some("artifact.md".to_string()),
            draft_artifact_path: None,
            user_choice: None,
            review_gate_approved: None,
            draft_exists: true,
        },
    );

    assert_eq!(dispatch, SchedulerEffectDispatch::PersistPlanningArtifact);
}

#[test]
fn plan_start_work_command_uses_plan_name_when_available() {
    assert_eq!(
        SchedulerProfileOrchestrator::plan_start_work_command(Some(
            ".sisyphus/plans/plan-demo-session.md"
        )),
        "/start-work plan-demo-session"
    );
    assert_eq!(
        SchedulerProfileOrchestrator::plan_start_work_command(None),
        "/start-work"
    );
}

#[test]
fn normalize_prometheus_review_output_preserves_structured_model_output() {
    let state = SchedulerProfileState::default();
    let review_output = "## Plan Generated: plan-demo-session

**Key Decisions Made**
- Keep planner-only flow.

**Scope**
- IN: Use the reviewed plan.
- OUT: Code execution.

**Guardrails Applied**
- None.

**Auto-Resolved**
- None.

**Defaults Applied**
- None.

**Decisions Needed**
- None.

**Handoff Readiness**
- Ready for handoff.

**Review Notes**
- None.";

    let normalized = SchedulerPresetKind::Prometheus
        .definition()
        .normalize_review_stage_output(state.preset_runtime_fields(), review_output)
        .expect("prometheus normalization");

    assert_eq!(normalized, review_output);
}

#[test]
fn normalize_prometheus_review_output_emits_omo_style_sections() {
    let state = SchedulerProfileState {
        route: SchedulerRouteState {
            route_decision: Some(RouteDecision {
                mode: RouteMode::Orchestrate,
                direct_kind: None,
                direct_response: None,
                preset: Some("prometheus".to_string()),
                insert_plan_stage: None,
                review_mode: Some(ReviewMode::Normal),
                context_append: None,
                rationale_summary: "Planner-only handoff stays in Prometheus.".to_string(),
            }),
            interviewed: Some(
                "Confirmed TUI input scope and no execution in this phase.".to_string(),
            ),
            ..Default::default()
        },
        preset_runtime: SchedulerPresetRuntimeState {
            planned: Some(
                "# Plan

- [DECISION NEEDED: pick the final scrollbar style]"
                    .to_string(),
            ),
            planning_artifact_path: Some(".sisyphus/plans/plan-demo-session.md".to_string()),
            advisory_review: Some(
                "- Preserve planner-only boundaries
- Keep handoff explicit"
                    .to_string(),
            ),
            ..Default::default()
        },
        ..Default::default()
    };

    let review = SchedulerPresetKind::Prometheus
        .definition()
        .normalize_review_stage_output(
            state.preset_runtime_fields(),
            "Review found one unresolved choice.",
        )
        .expect("prometheus normalization");

    assert!(review.contains("## Plan Generated: plan-demo-session"));
    assert!(review.contains("**Defaults Applied**"));
    assert!(review.contains("**Decisions Needed**"));
    assert!(review.contains("**Auto-Resolved**"));
    assert!(review.contains("**Handoff Readiness**"));
    assert!(review.contains("DECISION NEEDED"));
}

#[test]
fn finalize_output_appends_prometheus_artifact_note() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        preset_runtime: SchedulerPresetRuntimeState {
            planned: Some(
                "# Plan

- Step 1"
                    .to_string(),
            ),
            planning_artifact_path: Some(".sisyphus/plans/plan-demo-session.md".to_string()),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("# Plan"));
    assert!(output
        .content
        .contains(".sisyphus/plans/plan-demo-session.md"));
}

#[test]
fn finalize_output_normalizes_prometheus_review_into_handoff_delivery() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            reviewed: Some(OrchestratorOutput {
                content: r#"## Plan Generated: scheduler

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
- Plan is consistent."#
                    .to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    assert!(output.content.contains("## Plan Summary"));
    assert!(output.content.contains("**Recommended Next Step**"));
    assert!(output.content.contains("Prometheus remains planner-only"));
}

#[test]
fn finalize_output_emits_precise_prometheus_handoff_command_metadata() {
    let orchestrator = SchedulerProfileOrchestrator::new(
        planner_only_plan(),
        ToolRunner::new(Arc::new(NoopToolExecutor)),
    );
    let state = SchedulerProfileState {
        execution: SchedulerExecutionState {
            handed_off: Some(OrchestratorOutput {
                content: "## Plan Summary\n- Ready.".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: FinishReason::EndTurn,
            }),
            ..Default::default()
        },
        preset_runtime: SchedulerPresetRuntimeState {
            user_choice: Some("Start Work".to_string()),
            planning_artifact_path: Some(".sisyphus/plans/plan-demo-session.md".to_string()),
            ..Default::default()
        },
        metrics: SchedulerMetricsState {
            total_steps: 1,
            total_tool_calls: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    let output = orchestrator.finalize_output(state);
    assert_eq!(
        output
            .metadata
            .get("scheduler_handoff_mode")
            .and_then(|value| value.as_str()),
        Some("atlas")
    );
    assert_eq!(
        output
            .metadata
            .get("scheduler_handoff_command")
            .and_then(|value| value.as_str()),
        Some("/start-work plan-demo-session")
    );
    assert_eq!(
        output
            .metadata
            .get("scheduler_handoff_plan_path")
            .and_then(|value| value.as_str()),
        Some(".sisyphus/plans/plan-demo-session.md")
    );
}

struct TestAgentResolver;

#[async_trait]
impl AgentResolver for TestAgentResolver {
    fn resolve(&self, name: &str) -> Option<AgentDescriptor> {
        match name {
            "metis" | "momus" => Some(AgentDescriptor {
                name: name.to_string(),
                system_prompt: Some(format!("You are {name}.")),
                model: None,
                max_steps: Some(4),
                temperature: Some(0.1),
                allowed_tools: Vec::new(),
            }),
            _ => None,
        }
    }
}

struct TestModelResolver {
    streams: Mutex<Vec<rocode_provider::StreamResult>>,
}

impl TestModelResolver {
    fn new(streams: Vec<rocode_provider::StreamResult>) -> Self {
        Self {
            streams: Mutex::new(streams),
        }
    }
}

#[async_trait]
impl ModelResolver for TestModelResolver {
    async fn chat_stream(
        &self,
        _model: Option<&ModelRef>,
        _messages: Vec<rocode_provider::Message>,
        _tools: Vec<rocode_provider::ToolDefinition>,
        _exec_ctx: &ExecutionContext,
    ) -> Result<rocode_provider::StreamResult, OrchestratorError> {
        self.streams
            .lock()
            .await
            .pop()
            .ok_or_else(|| OrchestratorError::Other("missing test stream".to_string()))
    }
}

fn stream_from_text(text: &str) -> rocode_provider::StreamResult {
    Box::pin(stream::iter(vec![
        Ok::<_, rocode_provider::ProviderError>(rocode_provider::StreamEvent::TextDelta(
            text.to_string(),
        )),
        Ok::<_, rocode_provider::ProviderError>(rocode_provider::StreamEvent::Done),
    ]))
}

fn new_temp_workdir() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("rocode-prometheus-profile-{unique}"));
    std::fs::create_dir_all(&path).expect("temp workdir should create");
    path
}

#[test]
fn atlas_runtime_authority_sync_loads_boulder_plan_snapshot() {
    let workdir = new_temp_workdir();
    let plan_path = workdir.join(".sisyphus/plans/demo.md");
    fs::create_dir_all(plan_path.parent().expect("plan parent")).expect("plan dir");
    fs::write(&plan_path, "- [ ] task A\n- [x] task B\n").expect("plan should write");
    fs::write(
        workdir.join(".sisyphus/boulder.json"),
        format!(
            r#"{{
  "active_plan": "{}",
  "started_at": "2026-03-09T00:00:00Z",
  "session_ids": ["ses-1", "ses-2"],
  "plan_name": "demo",
  "agent": "atlas",
  "worktree_path": "/tmp/demo-worktree"
}}"#,
            plan_path.display()
        ),
    )
    .expect("boulder should write");

    let mut state = SchedulerProfileState::default();
    let ctx = test_context(&workdir, "atlas-session", Vec::new());
    let plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration])
        .with_orchestrator("atlas");
    SchedulerProfileOrchestrator::sync_preset_runtime_authority(&plan, &mut state, &ctx);

    assert_eq!(
        state.preset_runtime.planning_artifact_path.as_deref(),
        Some(".sisyphus/plans/demo.md")
    );
    assert!(state
        .preset_runtime
        .planned
        .as_deref()
        .unwrap_or_default()
        .contains("- [ ] task A"));
    let ground_truth = state
        .preset_runtime
        .ground_truth_context
        .as_deref()
        .unwrap_or_default();
    assert!(ground_truth.contains("boulder_state_path"));
    assert!(ground_truth.contains("tracked_sessions: `2`"));
    assert!(ground_truth.contains("/tmp/demo-worktree"));
}

fn test_context(
    workdir: &Path,
    session_id: &str,
    streams: Vec<rocode_provider::StreamResult>,
) -> OrchestratorContext {
    test_context_with_executor(workdir, session_id, streams, Arc::new(NoopToolExecutor))
}

fn test_context_with_executor(
    workdir: &Path,
    session_id: &str,
    streams: Vec<rocode_provider::StreamResult>,
    tool_executor: Arc<dyn ToolExecutor>,
) -> OrchestratorContext {
    OrchestratorContext {
        agent_resolver: Arc::new(TestAgentResolver),
        model_resolver: Arc::new(TestModelResolver::new(streams)),
        tool_executor: tool_executor.clone(),
        lifecycle_hook: Arc::new(NoopLifecycleHook),
        cancel_token: Arc::new(crate::runtime::events::NeverCancel),
        exec_ctx: ExecutionContext {
            session_id: session_id.to_string(),
            workdir: workdir.display().to_string(),
            agent_name: "prometheus".to_string(),
            metadata: HashMap::new(),
        },
    }
}

#[tokio::test]
async fn prometheus_plan_stage_persists_markdown_artifact() {
    let workdir = new_temp_workdir();
    let session_id = "test-session";
    let expected_relative = SchedulerPresetKind::Prometheus
        .definition()
        .planning_artifact_relative_path(session_id)
        .expect("prometheus plan artifact path");
    let expected_path = workdir.join(&expected_relative);
    let context = test_context(
        &workdir,
        session_id,
        vec![
            stream_from_text("# Plan\n\n- Normalize Ctrl+H\n- Rebind help"),
            stream_from_text("## Metis Review\n- Guardrail: keep help binding reachable"),
        ],
    );
    let runner = ToolRunner::new(Arc::new(NoopToolExecutor));
    let mut orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::Plan]).with_orchestrator("prometheus"),
        runner,
    );

    let output = orchestrator
        .execute("Fix backspace popup", &context)
        .await
        .expect("prometheus plan should succeed");

    let artifact =
        std::fs::read_to_string(&expected_path).expect("prometheus plan artifact should exist");
    assert!(artifact.contains("## TL;DR"));
    assert!(artifact.contains("## Verification Strategy"));
    assert!(artifact.contains("## TODOs"));
    assert!(artifact.contains("Normalize Ctrl+H"));
    assert!(artifact.contains("Rebind help"));
    assert!(output.content.contains("Plan saved to:"));
    assert!(output.content.contains(&expected_relative));
    assert_eq!(output.steps, 2);

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn prometheus_interview_stage_persists_draft_artifact() {
    let workdir = new_temp_workdir();
    let session_id = "draft-session";
    let expected_relative = SchedulerPresetKind::Prometheus
        .definition()
        .draft_artifact_relative_path(session_id)
        .expect("prometheus draft artifact path");
    let expected_path = workdir.join(&expected_relative);
    let context = test_context(
        &workdir,
        session_id,
        vec![stream_from_text(
            "## Interview Brief
- Goal: fix backspace behavior",
        )],
    );
    let runner = ToolRunner::new(Arc::new(NoopToolExecutor));
    let mut orchestrator = SchedulerProfileOrchestrator::new(
        SchedulerProfilePlan::new(vec![SchedulerStageKind::Interview])
            .with_orchestrator("prometheus"),
        runner,
    );

    orchestrator
        .execute("Fix backspace popup", &context)
        .await
        .expect("prometheus interview should succeed");

    let artifact =
        std::fs::read_to_string(&expected_path).expect("prometheus draft artifact should exist");
    assert!(artifact.contains("# Draft:"));
    assert!(artifact.contains("## Requirements (confirmed)"));
    assert!(artifact.contains("## Open Questions"));
    assert!(artifact.contains("fix backspace behavior"));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[derive(Default)]
struct RecordingToolExecutor {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ToolExecutor for RecordingToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        _arguments: serde_json::Value,
        _exec_ctx: &crate::ExecutionContext,
    ) -> Result<ToolOutput, ToolExecError> {
        self.calls.lock().await.push(tool_name.to_string());
        match tool_name {
            "todowrite" => Ok(ToolOutput {
                output: "todos updated".to_string(),
                is_error: false,
                title: None,
                metadata: None,
            }),
            "question" => Ok(ToolOutput {
                output: r#"{"answers":["Start Work"]}"#.to_string(),
                is_error: false,
                title: None,
                metadata: None,
            }),
            other => Err(ToolExecError::ExecutionError(format!(
                "unexpected tool call: {other}"
            ))),
        }
    }

    async fn list_ids(&self) -> Vec<String> {
        Vec::new()
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &crate::ExecutionContext,
    ) -> Vec<rocode_provider::ToolDefinition> {
        Vec::new()
    }
}

#[tokio::test]
async fn prometheus_handoff_registers_todos_and_guides_start_work() {
    let workdir = new_temp_workdir();
    let session_id = "handoff-session";
    let draft_relative = SchedulerPresetKind::Prometheus
        .definition()
        .draft_artifact_relative_path(session_id)
        .expect("prometheus draft path");
    let draft_path = workdir.join(&draft_relative);
    let tool_executor = Arc::new(RecordingToolExecutor::default());
    let context = test_context_with_executor(
        &workdir,
        session_id,
        vec![
            stream_from_text(
                "## Handoff
Ready for execution.",
            ),
            stream_from_text(
                "## Review
Plan looks consistent.",
            ),
            stream_from_text(
                "# Plan

- Fix input handling",
            ),
            stream_from_text(
                "## Metis Review
- Keep scope tight",
            ),
            stream_from_text(
                "## Interview Brief
- Confirm TUI behavior",
            ),
            stream_from_text(
                r#"{"mode":"orchestrate","preset":"prometheus","rationale_summary":"Stay in planner workflow"}"#,
            ),
        ],
        tool_executor.clone(),
    );
    let runner = ToolRunner::new(tool_executor.clone());
    let mut orchestrator = SchedulerProfileOrchestrator::new(planner_only_plan(), runner);

    let output = orchestrator
        .execute("Fix the TUI backspace flow", &context)
        .await
        .expect("prometheus flow should succeed");

    let calls = tool_executor.calls.lock().await.clone();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.as_str() == "todowrite")
            .count(),
        1
    );
    assert!(calls.iter().any(|call| call == "question"));
    assert!(output.content.contains("/start-work"));
    assert!(output.content.contains("Plan saved to:"));
    assert!(!draft_path.exists());

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn prometheus_runtime_rejects_non_planner_orchestration_tools_before_execution() {
    let workdir = new_temp_workdir();
    let tool_executor = Arc::new(RecordingToolExecutor::default());
    let ctx = test_context_with_executor(
        &workdir,
        "runtime-tool-session",
        Vec::new(),
        tool_executor.clone(),
    );
    let mut state = SchedulerProfileState::default();

    let error = SchedulerProfileOrchestrator::execute_orchestration_tool(
        "write",
        serde_json::json!({
            "file_path": ".sisyphus/plans/demo.md",
            "content": "# not allowed"
        }),
        &planner_only_plan(),
        &mut state,
        &ctx,
    )
    .await
    .expect_err("prometheus runtime should reject non-orchestration tools");

    match error {
        OrchestratorError::ToolError { tool, error } => {
            assert_eq!(tool, "write");
            assert!(error.contains("question, todowrite"));
        }
        other => panic!("unexpected error: {other}"),
    }

    assert!(tool_executor.calls.lock().await.is_empty());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn sisyphus_runtime_rejects_non_runtime_tool_before_execution() {
    let workdir = new_temp_workdir();
    let tool_executor = Arc::new(RecordingToolExecutor::default());
    let ctx = test_context_with_executor(
        &workdir,
        "sisyphus-runtime-tool-session",
        Vec::new(),
        tool_executor.clone(),
    );
    let mut state = SchedulerProfileState::default();

    let error = SchedulerProfileOrchestrator::execute_orchestration_tool(
        "question",
        serde_json::json!({"questions": [{"question": "Continue?"}]}),
        &runtime_execution_plan("sisyphus"),
        &mut state,
        &ctx,
    )
    .await
    .expect_err("sisyphus runtime should reject question tool");

    match error {
        OrchestratorError::ToolError { tool, error } => {
            assert_eq!(tool, "question");
            assert!(error.contains("todowrite"));
        }
        other => panic!("unexpected error: {other}"),
    }

    assert!(tool_executor.calls.lock().await.is_empty());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn atlas_runtime_rejects_non_runtime_tool_before_execution() {
    let workdir = new_temp_workdir();
    let tool_executor = Arc::new(RecordingToolExecutor::default());
    let ctx = test_context_with_executor(
        &workdir,
        "atlas-runtime-tool-session",
        Vec::new(),
        tool_executor.clone(),
    );
    let mut state = SchedulerProfileState::default();

    let error = SchedulerProfileOrchestrator::execute_orchestration_tool(
        "question",
        serde_json::json!({"questions": [{"question": "Continue?"}]}),
        &runtime_execution_plan("atlas"),
        &mut state,
        &ctx,
    )
    .await
    .expect_err("atlas runtime should reject question tool");

    match error {
        OrchestratorError::ToolError { tool, error } => {
            assert_eq!(tool, "question");
            assert!(error.contains("todowrite"));
        }
        other => panic!("unexpected error: {other}"),
    }

    assert!(tool_executor.calls.lock().await.is_empty());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn hephaestus_runtime_rejects_non_runtime_tool_before_execution() {
    let workdir = new_temp_workdir();
    let tool_executor = Arc::new(RecordingToolExecutor::default());
    let ctx = test_context_with_executor(
        &workdir,
        "hephaestus-runtime-tool-session",
        Vec::new(),
        tool_executor.clone(),
    );
    let mut state = SchedulerProfileState::default();

    let error = SchedulerProfileOrchestrator::execute_orchestration_tool(
        "question",
        serde_json::json!({"questions": [{"question": "Continue?"}]}),
        &runtime_execution_plan("hephaestus"),
        &mut state,
        &ctx,
    )
    .await
    .expect_err("hephaestus runtime should reject question tool");

    match error {
        OrchestratorError::ToolError { tool, error } => {
            assert_eq!(tool, "question");
            assert!(error.contains("todowrite"));
        }
        other => panic!("unexpected error: {other}"),
    }

    assert!(tool_executor.calls.lock().await.is_empty());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[test]
fn prometheus_runtime_rejects_persisting_artifact_outside_sisyphus_markdown_scope() {
    let workdir = new_temp_workdir();
    let ctx = test_context(&workdir, "persist-guard-session", Vec::new());
    let mut state = SchedulerProfileState::default();
    state.preset_runtime.planning_artifact_path = Some("notes/demo.md".to_string());

    let error = SchedulerProfileOrchestrator::persist_artifact(
        &planner_only_plan(),
        SchedulerArtifactKind::Planning,
        "# invalid plan location",
        &mut state,
        &ctx,
    )
    .expect_err("prometheus runtime should reject invalid artifact location");

    assert!(error
        .to_string()
        .contains("only reference markdown artifacts under .sisyphus"));
    assert!(!workdir.join("notes/demo.md").exists());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[test]
fn sisyphus_runtime_rejects_scheduler_artifact_persistence() {
    let workdir = new_temp_workdir();
    let ctx = test_context(&workdir, "sisyphus-artifact-session", Vec::new());
    let mut state = SchedulerProfileState::default();
    state.preset_runtime.planning_artifact_path = Some(".sisyphus/plans/demo.md".to_string());

    let error = SchedulerProfileOrchestrator::persist_artifact(
        &runtime_execution_plan("sisyphus"),
        SchedulerArtifactKind::Planning,
        "# should not exist",
        &mut state,
        &ctx,
    )
    .expect_err("sisyphus runtime should reject scheduler artifacts");

    assert!(error
        .to_string()
        .contains("does not manage scheduler artifacts"));
    assert!(!workdir.join(".sisyphus/plans/demo.md").exists());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[test]
fn hephaestus_runtime_rejects_scheduler_artifact_persistence() {
    let workdir = new_temp_workdir();
    let ctx = test_context(&workdir, "hephaestus-artifact-session", Vec::new());
    let mut state = SchedulerProfileState::default();
    state.preset_runtime.planning_artifact_path = Some(".sisyphus/plans/demo.md".to_string());

    let error = SchedulerProfileOrchestrator::persist_artifact(
        &runtime_execution_plan("hephaestus"),
        SchedulerArtifactKind::Planning,
        "# should not exist",
        &mut state,
        &ctx,
    )
    .expect_err("hephaestus runtime should reject scheduler artifacts");

    assert!(error
        .to_string()
        .contains("does not manage scheduler artifacts"));
    assert!(!workdir.join(".sisyphus/plans/demo.md").exists());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[test]
fn atlas_runtime_authority_rejects_external_ground_truth_plan_path() {
    let workdir = new_temp_workdir();
    let external_dir = new_temp_workdir();
    let external_plan = external_dir.join("external.md");
    fs::write(&external_plan, "- [ ] outside").expect("external plan should exist");
    fs::create_dir_all(workdir.join(".sisyphus")).expect("boulder dir should exist");
    fs::write(
        workdir.join(".sisyphus/boulder.json"),
        format!(
            r#"{{
  "active_plan": "{}",
  "started_at": "2026-03-09T00:00:00Z",
  "session_ids": ["ses-1"],
  "plan_name": "external",
  "agent": "atlas"
}}"#,
            external_plan.display()
        ),
    )
    .expect("boulder should write");

    let ctx = test_context(&workdir, "atlas-ground-truth-session", Vec::new());
    let mut state = SchedulerProfileState::default();
    SchedulerProfileOrchestrator::sync_preset_runtime_authority(
        &runtime_execution_plan("atlas"),
        &mut state,
        &ctx,
    );

    assert!(state.preset_runtime.planning_artifact_path.is_none());
    assert!(state.preset_runtime.planned.is_none());
    assert!(state.preset_runtime.ground_truth_context.is_none());

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
    std::fs::remove_dir_all(&external_dir).expect("external temp workdir should clean up");
}

#[test]
fn prometheus_runtime_rejects_deleting_artifact_outside_sisyphus_markdown_scope() {
    let workdir = new_temp_workdir();
    let invalid_path = workdir.join("notes/demo.md");
    fs::create_dir_all(invalid_path.parent().expect("invalid path parent"))
        .expect("invalid path dir");
    fs::write(&invalid_path, "draft").expect("invalid draft should exist");

    let ctx = test_context(&workdir, "delete-guard-session", Vec::new());
    let mut state = SchedulerProfileState::default();
    state.preset_runtime.draft_artifact_path = Some("notes/demo.md".to_string());

    let error = SchedulerProfileOrchestrator::delete_artifact(
        &planner_only_plan(),
        SchedulerArtifactKind::Draft,
        &mut state,
        &ctx,
    )
    .expect_err("prometheus runtime should reject invalid artifact deletion");

    assert!(error
        .to_string()
        .contains("only reference markdown artifacts under .sisyphus"));
    assert!(invalid_path.exists());
    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[derive(Default)]
struct NoopToolExecutor;

#[async_trait]
impl ToolExecutor for NoopToolExecutor {
    async fn execute(
        &self,
        _tool_name: &str,
        _arguments: serde_json::Value,
        _exec_ctx: &crate::ExecutionContext,
    ) -> Result<ToolOutput, ToolExecError> {
        Err(ToolExecError::ExecutionError("unused in tests".to_string()))
    }

    async fn list_ids(&self) -> Vec<String> {
        Vec::new()
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &crate::ExecutionContext,
    ) -> Vec<rocode_provider::ToolDefinition> {
        Vec::new()
    }
}

// ── Per-stage override chain tests ──

#[test]
fn stage_policy_without_overrides_uses_preset_defaults() {
    let plan = SchedulerProfilePlan::new(vec![
        SchedulerStageKind::Plan,
        SchedulerStageKind::ExecutionOrchestration,
    ]);
    // Plan default: AllowReadOnly, Unbounded, Transcript
    let plan_policy = plan.stage_policy(SchedulerStageKind::Plan);
    assert_eq!(plan_policy.tool_policy, StageToolPolicy::AllowReadOnly);
    assert_eq!(plan_policy.loop_budget, SchedulerLoopBudget::Unbounded);

    // ExecutionOrchestration default: AllowAll, Unbounded, Transcript, child_session=true
    let exec_policy = plan.stage_policy(SchedulerStageKind::ExecutionOrchestration);
    assert_eq!(exec_policy.tool_policy, StageToolPolicy::AllowAll);
    assert!(exec_policy.child_session);
}

#[test]
fn stage_policy_applies_json_overrides() {
    use crate::scheduler::{SchedulerStageOverride, StageToolPolicyOverride};

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::Plan,
        SchedulerStageOverride {
            kind: SchedulerStageKind::Plan,
            tool_policy: Some(StageToolPolicyOverride::AllowAll),
            loop_budget: Some("step-limit:5".to_string()),
            session_projection: Some("hidden".to_string()),
            agent_tree: None,
            agents: Vec::new(),
            skill_list: Vec::new(),
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::Plan]);
    plan.stage_overrides = overrides;

    let policy = plan.stage_policy(SchedulerStageKind::Plan);
    assert_eq!(policy.tool_policy, StageToolPolicy::AllowAll);
    assert_eq!(policy.loop_budget, SchedulerLoopBudget::StepLimit(5));
    assert_eq!(
        policy.session_projection,
        SchedulerSessionProjection::Hidden
    );
}

#[test]
fn stage_policy_partial_override_keeps_defaults() {
    use crate::scheduler::{SchedulerStageOverride, StageToolPolicyOverride};

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::Review,
        SchedulerStageOverride {
            kind: SchedulerStageKind::Review,
            tool_policy: Some(StageToolPolicyOverride::DisableAll),
            loop_budget: None,        // keep default
            session_projection: None, // keep default
            agent_tree: None,
            agents: Vec::new(),
            skill_list: Vec::new(),
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::Review]);
    plan.stage_overrides = overrides;

    let policy = plan.stage_policy(SchedulerStageKind::Review);
    // Override applied
    assert_eq!(policy.tool_policy, StageToolPolicy::DisableAll);
    // Defaults preserved
    assert_eq!(policy.loop_budget, SchedulerLoopBudget::Unbounded);
    assert_eq!(
        policy.session_projection,
        SchedulerSessionProjection::Transcript
    );
}

#[test]
fn stage_agent_tree_returns_per_stage_tree() {
    use crate::agent_tree::AgentTreeNode;
    use crate::scheduler::config::AgentTreeSource;
    use crate::scheduler::SchedulerStageOverride;
    use crate::AgentDescriptor;

    let tree = AgentTreeNode::new(AgentDescriptor {
        name: "custom-coordinator".to_string(),
        ..Default::default()
    });

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::ExecutionOrchestration,
        SchedulerStageOverride {
            kind: SchedulerStageKind::ExecutionOrchestration,
            tool_policy: None,
            loop_budget: None,
            session_projection: None,
            agent_tree: Some(AgentTreeSource::Inline(tree)),
            agents: Vec::new(),
            skill_list: Vec::new(),
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration]);
    plan.stage_overrides = overrides;

    // Per-stage tree exists
    let tree = plan.stage_agent_tree(SchedulerStageKind::ExecutionOrchestration);
    assert!(tree.is_some());
    assert_eq!(tree.unwrap().agent.name, "custom-coordinator");

    // No override for a different stage
    assert!(plan.stage_agent_tree(SchedulerStageKind::Plan).is_none());
}

#[test]
fn stage_capabilities_override_projects_stage_agents_and_skills() {
    use crate::scheduler::SchedulerStageOverride;

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::Plan,
        SchedulerStageOverride {
            kind: SchedulerStageKind::Plan,
            tool_policy: None,
            loop_budget: None,
            session_projection: None,
            agent_tree: None,
            agents: vec!["planner".to_string(), "researcher".to_string()],
            skill_list: vec!["pubmed".into(), "github-research".into()],
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::Plan]);
    plan.available_agents = vec![
        AvailableAgentMeta {
            name: "planner".to_string(),
            description: String::new(),
            mode: String::new(),
            cost: String::new(),
        },
        AvailableAgentMeta {
            name: "researcher".to_string(),
            description: String::new(),
            mode: String::new(),
            cost: String::new(),
        },
        AvailableAgentMeta {
            name: "extra".to_string(),
            description: String::new(),
            mode: String::new(),
            cost: String::new(),
        },
    ];
    plan.available_categories = vec![AvailableCategoryMeta {
        name: "literature".to_string(),
        description: String::new(),
    }];
    plan.skill_list = vec![
        "pubmed".into(),
        "github-research".into(),
        "irrelevant".into(),
    ];
    plan.stage_overrides = overrides;

    let capabilities = plan
        .stage_graph()
        .stage(SchedulerStageKind::Plan)
        .and_then(|stage| stage.capabilities.clone())
        .expect("stage capabilities should exist");

    assert_eq!(capabilities.agents, vec!["planner", "researcher"]);
    assert_eq!(
        capabilities.skill_list,
        vec!["pubmed".into(), "github-research".into()]
    );
    assert_eq!(capabilities.categories, vec!["literature"]);
}

#[test]
fn effective_skill_list_prefers_stage_scoped_catalog_when_present() {
    let mut plan = planner_only_plan();
    plan.skill_list = vec!["global-skill".into()];
    plan.stage_skill_lists.insert(
        SchedulerStageKind::Review,
        vec!["review-skill".into(), "review-evidence".into()],
    );

    assert_eq!(
        plan.effective_skill_list(Some(SchedulerStageKind::Review)),
        &[
            SchedulerSkillRef::from("review-skill"),
            SchedulerSkillRef::from("review-evidence"),
        ]
    );
    assert_eq!(
        plan.effective_skill_list(Some(SchedulerStageKind::Plan)),
        &[SchedulerSkillRef::from("global-skill")]
    );
}

#[test]
fn parse_loop_budget_variants() {
    assert_eq!(
        super::parse_loop_budget("step-limit:3"),
        SchedulerLoopBudget::StepLimit(3)
    );
    assert_eq!(
        super::parse_loop_budget("step-limit:10"),
        SchedulerLoopBudget::StepLimit(10)
    );
    assert_eq!(
        super::parse_loop_budget("unbounded"),
        SchedulerLoopBudget::Unbounded
    );
    assert_eq!(
        super::parse_loop_budget("garbage"),
        SchedulerLoopBudget::Unbounded
    );
}

#[test]
fn parse_session_projection_variants() {
    assert_eq!(
        super::parse_session_projection("hidden"),
        SchedulerSessionProjection::Hidden
    );
    assert_eq!(
        super::parse_session_projection("transcript"),
        SchedulerSessionProjection::Transcript
    );
    assert_eq!(
        super::parse_session_projection("unknown"),
        SchedulerSessionProjection::Transcript
    );
}

// ── Phase 6: Architecture regression tests (scheduler) ──

/// Invariant: execution fallback respects stage policy overrides.
/// If a user configures tool_policy or loop_budget for ExecutionOrchestration,
/// the fallback path must honor those settings, not hardcode AllowAll/Unbounded.
#[test]
fn fallback_stage_respects_execution_orchestration_policy_override() {
    use crate::scheduler::{SchedulerStageOverride, StageToolPolicyOverride};

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::ExecutionOrchestration,
        SchedulerStageOverride {
            kind: SchedulerStageKind::ExecutionOrchestration,
            tool_policy: Some(StageToolPolicyOverride::AllowReadOnly),
            loop_budget: Some("step-limit:10".to_string()),
            session_projection: None,
            agent_tree: None,
            agents: Vec::new(),
            skill_list: Vec::new(),
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration]);
    plan.stage_overrides = overrides;

    let policy = plan.stage_policy(SchedulerStageKind::ExecutionOrchestration);

    // Fallback uses stage_policy(), so these overrides must be visible.
    assert_eq!(policy.tool_policy, StageToolPolicy::AllowReadOnly);
    assert_eq!(policy.loop_budget, SchedulerLoopBudget::StepLimit(10));
}

/// Invariant: review stage and fallback stage both use stage_agent_from_policy,
/// ensuring the same override chain applies to all stage execution paths.
#[test]
fn stage_agent_from_policy_respects_loop_budget() {
    let unbounded_policy = SchedulerStagePolicy {
        tool_policy: StageToolPolicy::AllowAll,
        loop_budget: SchedulerLoopBudget::Unbounded,
        session_projection: SchedulerSessionProjection::Transcript,
        child_session: false,
    };
    let bounded_policy = SchedulerStagePolicy {
        tool_policy: StageToolPolicy::AllowReadOnly,
        loop_budget: SchedulerLoopBudget::StepLimit(5),
        session_projection: SchedulerSessionProjection::Hidden,
        child_session: true,
    };

    let unbounded_agent = SchedulerProfileOrchestrator::stage_agent_from_policy(
        "test-unbounded",
        "prompt".to_string(),
        unbounded_policy,
    );
    let bounded_agent = SchedulerProfileOrchestrator::stage_agent_from_policy(
        "test-bounded",
        "prompt".to_string(),
        bounded_policy,
    );

    assert!(unbounded_agent.max_steps.is_none());
    assert_eq!(bounded_agent.max_steps, Some(5));
}

/// Invariant: stage_capabilities_override only activates when the override
/// has non-empty agents or skill_list. Empty overrides must not inject
/// spurious capabilities into stages that don't need them.
#[test]
fn stage_capabilities_override_ignores_empty_overrides() {
    use crate::scheduler::SchedulerStageOverride;

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::Review,
        SchedulerStageOverride {
            kind: SchedulerStageKind::Review,
            tool_policy: Some(StageToolPolicyOverride::DisableAll),
            loop_budget: None,
            session_projection: None,
            agent_tree: None,
            agents: Vec::new(),     // empty
            skill_list: Vec::new(), // empty
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::Review]);
    plan.stage_overrides = overrides;

    // Review doesn't need_capabilities(), and override is empty,
    // so stage_graph should NOT inject capabilities.
    let capabilities = plan
        .stage_graph()
        .stage(SchedulerStageKind::Review)
        .and_then(|stage| stage.capabilities.clone());
    assert!(
        capabilities.is_none(),
        "empty override should not inject capabilities"
    );
}

/// Invariant: stage_graph() merges stage_capabilities_override into the
/// preset's stage graph. This ensures the lifecycle hook receives the
/// per-stage narrowed capabilities, not just the plan-level full set.
#[test]
fn stage_graph_merges_capabilities_from_override() {
    use crate::scheduler::SchedulerStageOverride;

    let mut overrides = HashMap::new();
    overrides.insert(
        SchedulerStageKind::ExecutionOrchestration,
        SchedulerStageOverride {
            kind: SchedulerStageKind::ExecutionOrchestration,
            tool_policy: None,
            loop_budget: None,
            session_projection: None,
            agent_tree: None,
            agents: vec!["specialist".to_string()],
            skill_list: vec!["code-review".into()],
        },
    );

    let mut plan = SchedulerProfilePlan::new(vec![SchedulerStageKind::ExecutionOrchestration]);
    plan.available_agents = vec![
        AvailableAgentMeta {
            name: "specialist".to_string(),
            description: String::new(),
            mode: String::new(),
            cost: String::new(),
        },
        AvailableAgentMeta {
            name: "generalist".to_string(),
            description: String::new(),
            mode: String::new(),
            cost: String::new(),
        },
    ];
    plan.skill_list = vec!["code-review".into(), "testing".into()];
    plan.stage_overrides = overrides;

    let capabilities = plan
        .stage_graph()
        .stage(SchedulerStageKind::ExecutionOrchestration)
        .and_then(|stage| stage.capabilities.clone())
        .expect("execution stage should have capabilities");

    // Override narrows to specialist only, not generalist.
    assert_eq!(capabilities.agents, vec!["specialist"]);
    // Override narrows to code-review only, not testing.
    assert_eq!(capabilities.skill_list, vec!["code-review".into()]);
}
