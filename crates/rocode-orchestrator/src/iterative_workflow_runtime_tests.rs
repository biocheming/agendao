use super::*;
use crate::traits::{ModelResolver, ToolExecutor};
use crate::workflow_artifacts::WorkflowModeArtifactEntry;
use crate::{ToolExecError, ToolOutput};
use async_trait::async_trait;
use futures::stream;
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
        verifier: None,
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

fn verify_workflow_config() -> IterativeWorkflowConfig {
    let mut config = workflow_config_with_artifacts();
    config.workflow.mode = crate::iterative_workflow::IterativeWorkflowMode::Verify;
    config.verifier = Some(crate::iterative_workflow::VerifierConfig {
        model: crate::ModelRef {
            provider_id: "openai".to_string(),
            model_id: "gpt-5".to_string(),
        },
        criteria: vec![crate::iterative_workflow::VerifierCriterionDefinition {
            id: "spec".to_string(),
            name: "Spec adherence".to_string(),
            description: "Prefer the candidate that best satisfies the request.".to_string(),
            weight: None,
            aggregation: None,
        }],
        granularity: None,
        repetitions: Some(1),
        selection: Some(crate::iterative_workflow::VerifierSelectionStrategy::RoundRobin),
        max_candidates: Some(3),
        use_logprobs: Some(false),
        trace_format: Some(crate::iterative_workflow::VerifierTraceFormat::Compact),
    });
    config
}

fn verify_workflow_config_with_verifier(
    selection: crate::iterative_workflow::VerifierSelectionStrategy,
    repetitions: u32,
) -> IterativeWorkflowConfig {
    let mut config = verify_workflow_config();
    let verifier = config.verifier.as_mut().expect("verifier should exist");
    verifier.selection = Some(selection);
    verifier.repetitions = Some(repetitions);
    config
}

fn configure_verify_judge_test(
    config: &mut IterativeWorkflowConfig,
    max_iterations: u32,
    required_metric: f64,
) {
    config
        .objective
        .as_mut()
        .expect("objective should exist")
        .satisfied_when = Some(crate::iterative_workflow::SatisfiedWhenDefinition {
        metric_at_least: Some(required_metric),
        metric_at_most: None,
        metric_equals: None,
    });
    config
        .iteration_policy
        .as_mut()
        .expect("iteration policy should exist")
        .max_iterations = Some(max_iterations);
    config
        .decision_policy
        .as_mut()
        .expect("decision policy should exist")
        .keep_conditions = vec![KeepCondition::VerifyPassed];
    config
        .decision_policy
        .as_mut()
        .expect("decision policy should exist")
        .discard_conditions
        .retain(|condition| !matches!(condition, DiscardCondition::MetricRegressed));
}

fn workflow_config_with_commit_policy(squash_on_completion: bool) -> IterativeWorkflowConfig {
    let mut config = workflow_config_with_artifacts();
    config
        .workspace_policy
        .as_mut()
        .expect("workspace policy should exist")
        .commit_policy = Some(crate::iterative_workflow::CommitPolicyDefinition {
        commit_kept_iterations: Some(true),
        message_template: Some("autoresearch iter {iteration}: {decision} {summary}".to_string()),
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

struct ScriptedModelResolver {
    streams: Mutex<Vec<rocode_provider::StreamResult>>,
    captured_inputs: Arc<Mutex<Vec<String>>>,
    captured_metadata: Arc<Mutex<Vec<HashMap<String, serde_json::Value>>>>,
}

impl ScriptedModelResolver {
    fn new(streams: Vec<rocode_provider::StreamResult>) -> Self {
        Self {
            streams: Mutex::new(streams),
            captured_inputs: Arc::new(Mutex::new(Vec::new())),
            captured_metadata: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured_inputs(&self) -> Arc<Mutex<Vec<String>>> {
        self.captured_inputs.clone()
    }

    fn captured_metadata(&self) -> Arc<Mutex<Vec<HashMap<String, serde_json::Value>>>> {
        self.captured_metadata.clone()
    }

    fn extract_last_user_text(messages: &[rocode_provider::Message]) -> String {
        messages
            .iter()
            .rev()
            .find_map(|message| match (&message.role, &message.content) {
                (rocode_provider::Role::User, rocode_provider::Content::Text(text)) => {
                    Some(text.clone())
                }
                _ => None,
            })
            .unwrap_or_default()
    }
}

#[async_trait]
impl ModelResolver for ScriptedModelResolver {
    async fn chat_stream(
        &self,
        _model: Option<&crate::ModelRef>,
        messages: Vec<rocode_provider::Message>,
        _tools: Vec<rocode_provider::ToolDefinition>,
        exec_ctx: &ExecutionContext,
    ) -> Result<rocode_provider::StreamResult, OrchestratorError> {
        self.captured_inputs
            .lock()
            .await
            .push(Self::extract_last_user_text(&messages));
        self.captured_metadata
            .lock()
            .await
            .push(exec_ctx.metadata.clone());

        self.streams
            .lock()
            .await
            .pop()
            .ok_or_else(|| OrchestratorError::Other("missing scripted model stream".to_string()))
    }
}

struct CapturingLifecycleHook {
    events: Arc<Mutex<Vec<String>>>,
}

impl CapturingLifecycleHook {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn events(&self) -> Arc<Mutex<Vec<String>>> {
        self.events.clone()
    }
}

#[async_trait]
impl crate::traits::LifecycleHook for CapturingLifecycleHook {
    async fn on_orchestration_start(
        &self,
        _agent_name: &str,
        _max_steps: Option<u32>,
        _exec_ctx: &ExecutionContext,
    ) {
    }

    async fn on_step_start(
        &self,
        _agent_name: &str,
        _model_id: &str,
        _step: u32,
        _exec_ctx: &ExecutionContext,
    ) {
    }

    async fn on_orchestration_end(
        &self,
        _agent_name: &str,
        _steps: u32,
        _exec_ctx: &ExecutionContext,
    ) {
    }

    async fn on_scheduler_stage_start(
        &self,
        _agent_name: &str,
        stage_name: &str,
        _stage_index: u32,
        _capabilities: Option<&crate::scheduler::SchedulerStageCapabilities>,
        exec_ctx: &ExecutionContext,
    ) {
        let job_key = exec_ctx
            .metadata
            .get("workflow_verifier_score_job_key")
            .and_then(|value| value.as_str())
            .unwrap_or("missing");
        self.events
            .lock()
            .await
            .push(format!("start:{stage_name}:{job_key}"));
    }

    async fn on_scheduler_stage_end(
        &self,
        _agent_name: &str,
        stage_name: &str,
        _stage_index: u32,
        _stage_total: u32,
        content: &str,
        exec_ctx: &ExecutionContext,
    ) {
        let job_key = exec_ctx
            .metadata
            .get("workflow_verifier_score_job_key")
            .and_then(|value| value.as_str())
            .unwrap_or("missing");
        self.events
            .lock()
            .await
            .push(format!("end:{stage_name}:{job_key}:{content}"));
    }
}

struct ImmediateCancel;

impl crate::runtime::events::CancelToken for ImmediateCancel {
    fn is_cancelled(&self) -> bool {
        true
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

fn stream_from_logprob_scores() -> rocode_provider::StreamResult {
    let metadata = json!({
        "logprobs": [[
            {"token":"<score_A>","logprob":0.0,"top_logprobs":[]},
            {"token":"T","logprob":0.9f64.ln(),"top_logprobs":[{"token":"A","logprob":0.1f64.ln()}]},
            {"token":"</score_A>\n<score_B>","logprob":0.0,"top_logprobs":[]},
            {"token":"A","logprob":0.9f64.ln(),"top_logprobs":[{"token":"T","logprob":0.1f64.ln()}]},
            {"token":"</score_B>","logprob":0.0,"top_logprobs":[]}
        ]]
    });
    Box::pin(stream::iter(vec![
        Ok::<_, rocode_provider::ProviderError>(rocode_provider::StreamEvent::TextDelta(
            "<score_A>T</score_A>\n<score_B>A</score_B>".to_string(),
        )),
        Ok::<_, rocode_provider::ProviderError>(rocode_provider::StreamEvent::FinishStep {
            finish_reason: Some("stop".to_string()),
            usage: rocode_provider::StreamUsage::default(),
            provider_metadata: Some(metadata),
        }),
        Ok::<_, rocode_provider::ProviderError>(rocode_provider::StreamEvent::Done),
    ]))
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
        .evaluate_round(1, &OrchestratorOutput::empty(), Vec::new())
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
async fn verify_mode_persists_selected_candidate_summary() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config();
    configure_verify_judge_test(&mut config, 2, 13.0);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![stream_from_text(
        r#"{"winner":"cand-002","rationale":"Candidate two better satisfies the request.","scores":[{"criterion_id":"spec","winner":"cand-002","score_a":3.0,"score_b":5.0,"explanation":"Candidate two is more aligned."}]}"#,
    )]));
    let captured_inputs = judge.captured_inputs();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-run".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller
        .begin_iteration(1)
        .expect("first verify iteration should begin");
    let first_output = OrchestratorOutput {
        content: "candidate one execution output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: crate::runtime::events::FinishReason::EndTurn,
    };
    let first = controller
        .evaluate_round(1, &first_output, Vec::new())
        .await
        .expect("first verify round should succeed");
    assert_eq!(first.decision, IterationDecision::Keep);

    controller
        .begin_iteration(2)
        .expect("second verify iteration should begin");
    let second_output = OrchestratorOutput {
        content: "candidate two execution output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: crate::runtime::events::FinishReason::EndTurn,
    };
    let second = controller
        .evaluate_round(2, &second_output, Vec::new())
        .await
        .expect("second verify round should succeed");
    assert_eq!(second.decision, IterationDecision::Keep);
    assert_eq!(
        second.gate_decision.status,
        SchedulerExecutionGateStatus::Done
    );
    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate two execution output")
    );

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
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("verify summary should persist");

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("run-manifest.json"),
        )
        .expect("manifest should read"),
    )
    .expect("manifest should parse");
    assert_eq!(
        manifest["verifier"]["selected_candidate_id"].as_str(),
        Some("cand-002")
    );
    assert_eq!(
        manifest["verifier"]["candidates_considered"].as_u64(),
        Some(2)
    );
    assert_eq!(manifest["verifier"]["used_judge"].as_bool(), Some(true));
    assert_eq!(
        manifest["verifier"]["judge_rationale"].as_str(),
        Some("Candidate two better satisfies the request.")
    );
    assert_eq!(
        manifest["verifier"]["criterion_scores"][0]["criterion_id"].as_str(),
        Some("spec")
    );
    assert_eq!(
        manifest["verifier"]["criterion_scores"][0]["winner_candidate_id"].as_str(),
        Some("cand-002")
    );
    assert_eq!(
        manifest["verifier"]["criterion_scores"][0]["score_a"].as_str(),
        Some("3.0000")
    );
    assert_eq!(
        manifest["verifier"]["criterion_scores"][0]["score_b"].as_str(),
        Some("5.0000")
    );

    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("summary.json"),
        )
        .expect("summary should read"),
    )
    .expect("summary should parse");
    assert_eq!(
        summary["final_response"].as_str(),
        Some("candidate two execution output")
    );
    assert_eq!(
        summary["verifier"]["judge_rationale"].as_str(),
        Some("Candidate two better satisfies the request.")
    );
    assert_eq!(
        summary["verifier"]["criterion_scores"][0]["criterion_id"].as_str(),
        Some("spec")
    );
    assert!(summary["mode_report"]["final_notes"]
        .as_array()
        .expect("final notes should be an array")
        .iter()
        .any(|note| note
            .as_str()
            .is_some_and(|text| text
                .contains("Verifier rationale: Candidate two better satisfies the request."))));
    assert!(captured_inputs
        .lock()
        .await
        .iter()
        .any(|input| input.contains("cand-001") && input.contains("cand-002")));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_judge_prompt_includes_patch_and_artifact_trace() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config();
    configure_verify_judge_test(&mut config, 2, 13.0);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![stream_from_text(
        r#"{"winner":"cand-002"}"#,
    )]));
    let captured_inputs = judge.captured_inputs();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-trace-evidence".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller
        .begin_iteration(1)
        .expect("first verify iteration should begin");
    std::fs::write(workdir.join("src/lib.rs"), "candidate one\n")
        .expect("first candidate mutation should write");
    controller
        .evaluate_round(
            1,
            &OrchestratorOutput {
                content: "candidate one execution output".to_string(),
                steps: 4,
                tool_calls_count: 2,
                metadata: HashMap::from([(
                    "tool_window".to_string(),
                    json!("edit src/lib.rs; run cargo test"),
                )]),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("first verify round should succeed");

    controller
        .begin_iteration(2)
        .expect("second verify iteration should begin");
    std::fs::write(workdir.join("src/lib.rs"), "candidate two\n")
        .expect("second candidate mutation should write");
    controller
        .evaluate_round(
            2,
            &OrchestratorOutput {
                content: "candidate two execution output".to_string(),
                steps: 5,
                tool_calls_count: 3,
                metadata: HashMap::from([(
                    "tool_window".to_string(),
                    json!("edit src/lib.rs; rerun cargo test"),
                )]),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            vec![WorkflowModeArtifact {
                name: "candidate-registry".to_string(),
                description: "Verifier candidates".to_string(),
                entries: vec![WorkflowModeArtifactEntry {
                    iteration: Some(2),
                    key: "cand-002".to_string(),
                    title: "Candidate two".to_string(),
                    status: "kept".to_string(),
                    detail: "contains updated implementation evidence".to_string(),
                    evidence: vec!["src/lib.rs changed".to_string()],
                }],
            }],
        )
        .await
        .expect("second verify round should succeed");

    let inputs = captured_inputs.lock().await.clone();
    assert_eq!(inputs.len(), 1);
    let prompt = inputs.first().expect("judge prompt should be captured");
    assert!(prompt.contains("workspace_changes:"));
    assert!(prompt.contains("modified src/lib.rs"));
    assert!(prompt.contains("trajectory_fingerprint:"));
    assert!(prompt.contains("execution_summary:"));
    assert!(prompt.contains("steps: 5"));
    assert!(prompt.contains("tool_calls: 3"));
    assert!(prompt.contains("execution_metadata:"));
    assert!(prompt.contains("tool_window"));
    assert!(prompt.contains("structured_artifacts:"));
    assert!(prompt.contains("candidate-registry:cand-002"));
    assert!(prompt.contains("evidence=src/lib.rs changed"));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_uses_logprob_expected_reward_when_enabled() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config();
    configure_verify_judge_test(&mut config, 2, 13.0);
    config
        .verifier
        .as_mut()
        .expect("verifier should exist")
        .use_logprobs = Some(true);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![
        stream_from_logprob_scores(),
    ]));
    let captured_metadata = judge.captured_metadata();
    let lifecycle = Arc::new(CapturingLifecycleHook::new());
    let lifecycle_events = lifecycle.events();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-logprobs".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        lifecycle,
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller.begin_iteration(1).expect("first iteration");
    let first_output = OrchestratorOutput {
        content: "candidate one execution output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: crate::runtime::events::FinishReason::EndTurn,
    };
    controller
        .evaluate_round(1, &first_output, Vec::new())
        .await
        .expect("first round should evaluate");
    controller.begin_iteration(2).expect("second iteration");
    let second_output = OrchestratorOutput {
        content: "candidate two execution output".to_string(),
        steps: 1,
        tool_calls_count: 0,
        metadata: HashMap::new(),
        finish_reason: crate::runtime::events::FinishReason::EndTurn,
    };
    let second = controller
        .evaluate_round(2, &second_output, Vec::new())
        .await
        .expect("second round should evaluate");

    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate two execution output")
    );

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
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("verify summary should persist");

    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("summary.json"),
        )
        .expect("summary should read"),
    )
    .expect("summary should parse");
    assert_eq!(
        summary["verifier"]["selected_candidate_id"].as_str(),
        Some("cand-002")
    );
    assert_eq!(summary["verifier"]["used_logprobs"].as_bool(), Some(true));
    assert_eq!(
        summary["verifier"]["criterion_scores"][0]["winner_candidate_id"].as_str(),
        Some("cand-002")
    );
    assert_eq!(summary["verifier"]["judge_calls"].as_u64(), Some(1));
    let mode_artifacts = std::fs::read_to_string(
        workdir
            .join(".rocode")
            .join("autoresearch")
            .join("workflow-test")
            .join("mode-artifacts.json"),
    )
    .expect("mode artifacts should read");
    assert!(mode_artifacts.contains("\"name\": \"score-job-matrix\""));
    assert!(mode_artifacts.contains("\"name\": \"round-robin-win-counts\""));
    assert!(mode_artifacts.contains("requested_logprobs=true"));
    assert!(mode_artifacts.contains("requested_top_logprobs=20"));
    assert!(mode_artifacts.contains("logprob_status=requested-usable"));
    assert!(mode_artifacts.contains("fallback=none"));
    assert!(mode_artifacts.contains("formula_mean"));
    let metadata = captured_metadata.lock().await.clone();
    assert_eq!(
        metadata[0]["workflow_verifier_stage"].as_str(),
        Some("score-job")
    );
    assert_eq!(
        metadata[0]["workflow_verifier_criterion_id"].as_str(),
        Some("spec")
    );
    assert_eq!(
        metadata[0]["workflow_verifier_requested_top_logprobs"].as_u64(),
        Some(20)
    );
    let events = lifecycle_events.lock().await.clone();
    assert!(events.iter().any(|event| {
        event.starts_with("start:verifier-score-job:") && event.contains("criterion=")
    }));
    assert!(events.iter().any(|event| {
        event.starts_with("end:verifier-score-job:") && event.contains("used_logprobs=true")
    }));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_reports_logprob_capability_fallback_reason() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config();
    configure_verify_judge_test(&mut config, 2, 13.0);
    config
        .verifier
        .as_mut()
        .expect("verifier should exist")
        .use_logprobs = Some(true);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![stream_from_text(
        "<score_A>T</score_A>\n<score_B>A</score_B>",
    )]));
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-logprob-fallback".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller.begin_iteration(1).expect("first iteration");
    controller
        .evaluate_round(
            1,
            &OrchestratorOutput {
                content: "candidate one execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("first round should evaluate");
    controller.begin_iteration(2).expect("second iteration");
    let second = controller
        .evaluate_round(
            2,
            &OrchestratorOutput {
                content: "candidate two execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("second round should evaluate");

    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate two execution output")
    );
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
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("verify summary should persist");
    let mode_artifacts = std::fs::read_to_string(
        workdir
            .join(".rocode")
            .join("autoresearch")
            .join("workflow-test")
            .join("mode-artifacts.json"),
    )
    .expect("mode artifacts should read");
    assert!(mode_artifacts.contains("used_logprobs=false"));
    assert!(mode_artifacts.contains("fallback=text-tag"));
    assert!(mode_artifacts.contains("logprob_status=requested-missing-provider-metadata"));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_score_job_respects_cancellation_before_model_call() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config();
    configure_verify_judge_test(&mut config, 2, 13.0);
    config
        .verifier
        .as_mut()
        .expect("verifier should exist")
        .use_logprobs = Some(true);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![
        stream_from_logprob_scores(),
    ]));
    let captured_inputs = judge.captured_inputs();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-cancel-score-job".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(ImmediateCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller.begin_iteration(1).expect("first iteration");
    controller
        .evaluate_round(
            1,
            &OrchestratorOutput {
                content: "candidate one execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("first round should evaluate");
    controller.begin_iteration(2).expect("second iteration");
    let second = controller
        .evaluate_round(
            2,
            &OrchestratorOutput {
                content: "candidate two execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("second round should evaluate");

    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate one execution output")
    );
    assert!(captured_inputs.lock().await.is_empty());

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
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("verify summary should persist");
    let mode_artifacts = std::fs::read_to_string(
        workdir
            .join(".rocode")
            .join("autoresearch")
            .join("workflow-test")
            .join("mode-artifacts.json"),
    )
    .expect("mode artifacts should read");
    assert!(mode_artifacts.contains("fallback=error"));
    assert!(mode_artifacts.contains("cancelled before model call"));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_reuses_persistent_pairwise_cache_across_runs() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config();
    configure_verify_judge_test(&mut config, 2, 13.0);

    let run_once = |streams: Vec<rocode_provider::StreamResult>| {
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
                Ok(ToolOutput {
                    output: "score=11".to_string(),
                    is_error: false,
                    title: None,
                    metadata: Some(json!({"exit_code": 0})),
                }),
            ])),
        });
        let judge = Arc::new(ScriptedModelResolver::new(streams));
        (executor, judge)
    };

    let (executor, judge) = run_once(vec![stream_from_text(r#"{"winner":"cand-002"}"#)]);
    let mut first_controller = WorkflowController::from_config(
        config.clone(),
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-persistent-cache".to_string()),
    )
    .expect("first controller construction should succeed")
    .expect("first workflow should activate controller");
    first_controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );
    first_controller
        .capture_baseline()
        .await
        .expect("first baseline should capture");
    for iteration in 1..=2 {
        first_controller
            .begin_iteration(iteration)
            .expect("first run iteration should begin");
        let result = first_controller
            .evaluate_round(
                iteration,
                &OrchestratorOutput {
                    content: format!("candidate {iteration} execution output"),
                    steps: 1,
                    tool_calls_count: 0,
                    metadata: HashMap::new(),
                    finish_reason: crate::runtime::events::FinishReason::EndTurn,
                },
                Vec::new(),
            )
            .await
            .expect("first run should evaluate");
        if iteration == 2 {
            first_controller
                .persist_run_summary(WorkflowRunSummaryRecord {
                    iterations_completed: 2,
                    final_iteration: Some(2),
                    baseline_metric: None,
                    best_metric: None,
                    final_metric: None,
                    kept_commits: Vec::new(),
                    best_commit: None,
                    squashed_commit: None,
                    final_decision: Some(result.decision.label().to_string()),
                    final_gate_status: Some(
                        gate_status_label(result.gate_decision.status).to_string(),
                    ),
                    final_summary: Some(result.gate_decision.summary.clone()),
                    final_response: result.gate_decision.final_response.clone(),
                    verifier: None,
                    mode_report: None,
                    mode_artifacts: Vec::new(),
                    objective_satisfied: true,
                    cancelled: false,
                    exhausted_budget: false,
                })
                .expect("first summary should persist");
        }
    }

    let (executor, judge) = run_once(Vec::new());
    let captured_inputs = judge.captured_inputs();
    let mut second_controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-persistent-cache".to_string()),
    )
    .expect("second controller construction should succeed")
    .expect("second workflow should activate controller");
    second_controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );
    second_controller
        .capture_baseline()
        .await
        .expect("second baseline should capture");
    let mut second_result = None;
    for iteration in 1..=2 {
        second_controller
            .begin_iteration(iteration)
            .expect("second run iteration should begin");
        let result = second_controller
            .evaluate_round(
                iteration,
                &OrchestratorOutput {
                    content: format!("candidate {iteration} execution output"),
                    steps: 1,
                    tool_calls_count: 0,
                    metadata: HashMap::new(),
                    finish_reason: crate::runtime::events::FinishReason::EndTurn,
                },
                Vec::new(),
            )
            .await
            .expect("second run should evaluate from persistent cache");
        if iteration == 2 {
            second_result = Some(result);
        }
    }
    assert!(
        captured_inputs.lock().await.is_empty(),
        "persistent cache should avoid second-run judge calls"
    );
    let second_result = second_result.expect("second result should exist");
    second_controller
        .persist_run_summary(WorkflowRunSummaryRecord {
            iterations_completed: 2,
            final_iteration: Some(2),
            baseline_metric: None,
            best_metric: None,
            final_metric: None,
            kept_commits: Vec::new(),
            best_commit: None,
            squashed_commit: None,
            final_decision: Some(second_result.decision.label().to_string()),
            final_gate_status: Some(
                gate_status_label(second_result.gate_decision.status).to_string(),
            ),
            final_summary: Some(second_result.gate_decision.summary.clone()),
            final_response: second_result.gate_decision.final_response.clone(),
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("second summary should persist");

    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("summary.json"),
        )
        .expect("summary should read"),
    )
    .expect("summary should parse");
    assert_eq!(summary["verifier"]["judge_calls"].as_u64(), Some(0));
    assert_eq!(summary["verifier"]["cache_hits"].as_u64(), Some(1));
    assert_eq!(
        summary["verifier"]["selected_candidate_id"].as_str(),
        Some("cand-002")
    );

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_round_robin_uses_majority_vote() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config_with_verifier(
        crate::iterative_workflow::VerifierSelectionStrategy::RoundRobin,
        3,
    );
    configure_verify_judge_test(&mut config, 2, 13.0);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![
        stream_from_text(r#"{"winner":"cand-002"}"#),
        stream_from_text(r#"{"winner":"cand-002"}"#),
        stream_from_text(r#"{"winner":"cand-001"}"#),
    ]));
    let captured_inputs = judge.captured_inputs();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-round-robin".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller
        .begin_iteration(1)
        .expect("first verify iteration should begin");
    controller
        .evaluate_round(
            1,
            &OrchestratorOutput {
                content: "candidate one execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("first verify round should succeed");

    controller
        .begin_iteration(2)
        .expect("second verify iteration should begin");
    let second = controller
        .evaluate_round(
            2,
            &OrchestratorOutput {
                content: "candidate two execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("second verify round should succeed");

    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate two execution output")
    );
    let inputs = captured_inputs.lock().await.clone();
    assert_eq!(inputs.len(), 3);
    assert!(inputs.iter().any(|input| input.contains("Comparison: 1/3")));
    assert!(inputs.iter().any(|input| input.contains("Comparison: 2/3")));
    assert!(inputs.iter().any(|input| input.contains("Comparison: 3/3")));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_round_robin_scores_all_candidate_pairs() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config_with_verifier(
        crate::iterative_workflow::VerifierSelectionStrategy::RoundRobin,
        1,
    );
    configure_verify_judge_test(&mut config, 3, 20.0);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
            Ok(ToolOutput {
                output: "score=10".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![
        stream_from_text(r#"{"winner":"cand-003"}"#),
        stream_from_text(r#"{"winner":"cand-003"}"#),
        stream_from_text(r#"{"winner":"cand-002"}"#),
    ]));
    let captured_inputs = judge.captured_inputs();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-round-robin-all-pairs".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    let mut final_result = None;
    for iteration in 1..=3 {
        controller
            .begin_iteration(iteration)
            .expect("verify iteration should begin");
        let result = controller
            .evaluate_round(
                iteration,
                &OrchestratorOutput {
                    content: format!("candidate {iteration} execution output"),
                    steps: 1,
                    tool_calls_count: 0,
                    metadata: HashMap::new(),
                    finish_reason: crate::runtime::events::FinishReason::EndTurn,
                },
                Vec::new(),
            )
            .await
            .expect("verify round should succeed");
        if iteration == 3 {
            assert_eq!(
                result.gate_decision.final_response.as_deref(),
                Some("candidate 3 execution output")
            );
            final_result = Some(result);
        }
    }

    let inputs = captured_inputs.lock().await.clone();
    assert_eq!(inputs.len(), 3);
    assert!(inputs
        .iter()
        .any(|input| { input.contains("id: cand-001") && input.contains("id: cand-002") }));
    let final_round_inputs = &inputs[1..];
    assert!(final_round_inputs
        .iter()
        .any(|input| { input.contains("id: cand-001") && input.contains("id: cand-003") }));
    assert!(final_round_inputs
        .iter()
        .any(|input| { input.contains("id: cand-002") && input.contains("id: cand-003") }));

    let final_result = final_result.expect("final result should be captured");
    controller
        .persist_run_summary(WorkflowRunSummaryRecord {
            iterations_completed: 3,
            final_iteration: Some(3),
            baseline_metric: None,
            best_metric: None,
            final_metric: None,
            kept_commits: Vec::new(),
            best_commit: None,
            squashed_commit: None,
            final_decision: Some(final_result.decision.label().to_string()),
            final_gate_status: Some(
                gate_status_label(final_result.gate_decision.status).to_string(),
            ),
            final_summary: Some(final_result.gate_decision.summary.clone()),
            final_response: final_result.gate_decision.final_response.clone(),
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("verify summary should persist");

    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("summary.json"),
        )
        .expect("summary should read"),
    )
    .expect("summary should parse");
    assert_eq!(
        summary["verifier"]["pairwise_comparisons"].as_u64(),
        Some(4)
    );
    assert_eq!(summary["verifier"]["judge_calls"].as_u64(), Some(3));
    assert_eq!(summary["verifier"]["cache_hits"].as_u64(), Some(1));
    let mode_artifacts = std::fs::read_to_string(
        workdir
            .join(".rocode")
            .join("autoresearch")
            .join("workflow-test")
            .join("mode-artifacts.json"),
    )
    .expect("mode artifacts should read");
    assert!(mode_artifacts.contains("\"name\": \"pairwise-score-matrix\""));
    assert!(mode_artifacts.contains("\"name\": \"selection-report\""));
    assert!(mode_artifacts.contains("\"status\": \"cached\""));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_scores_can_override_top_level_winner_votes() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config_with_verifier(
        crate::iterative_workflow::VerifierSelectionStrategy::RoundRobin,
        3,
    );
    configure_verify_judge_test(&mut config, 2, 13.0);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![
        stream_from_text(
            r#"{"winner":"cand-001","scores":[{"criterion_id":"spec","winner":"cand-002","score_a":3.0,"score_b":5.0,"explanation":"candidate two better follows the request"}]}"#,
        ),
        stream_from_text(
            r#"{"winner":"cand-001","scores":[{"criterion_id":"spec","winner":"cand-002","score_a":2.0,"score_b":4.0,"explanation":"candidate two stays more aligned"}]}"#,
        ),
        stream_from_text(
            r#"{"winner":"cand-002","scores":[{"criterion_id":"spec","winner":"cand-002","score_a":1.0,"score_b":5.0,"explanation":"candidate two is clearly stronger"}]}"#,
        ),
    ]));
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-score-override".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller
        .begin_iteration(1)
        .expect("first verify iteration should begin");
    controller
        .evaluate_round(
            1,
            &OrchestratorOutput {
                content: "candidate one execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("first verify round should succeed");

    controller
        .begin_iteration(2)
        .expect("second verify iteration should begin");
    let second = controller
        .evaluate_round(
            2,
            &OrchestratorOutput {
                content: "candidate two execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("second verify round should succeed");

    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate two execution output")
    );

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
            verifier: None,
            mode_report: None,
            mode_artifacts: Vec::new(),
            objective_satisfied: true,
            cancelled: false,
            exhausted_budget: false,
        })
        .expect("verify summary should persist");

    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            workdir
                .join(".rocode")
                .join("autoresearch")
                .join("workflow-test")
                .join("summary.json"),
        )
        .expect("summary should read"),
    )
    .expect("summary should parse");
    let notes = summary["mode_report"]["final_notes"]
        .as_array()
        .expect("final notes should be an array")
        .iter()
        .filter_map(|note| note.as_str())
        .collect::<Vec<_>>();
    assert!(notes
        .iter()
        .any(|note| note.contains("spec -> cand-002 (2.0/4.7)")));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_criterion_weight_can_control_selection() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config_with_verifier(
        crate::iterative_workflow::VerifierSelectionStrategy::RoundRobin,
        1,
    );
    configure_verify_judge_test(&mut config, 2, 13.0);
    config
        .verifier
        .as_mut()
        .expect("verifier should exist")
        .criteria = vec![
        crate::iterative_workflow::VerifierCriterionDefinition {
            id: "spec".to_string(),
            name: "Spec adherence".to_string(),
            description: "Base request satisfaction.".to_string(),
            weight: Some(1.0),
            aggregation: Some(crate::iterative_workflow::VerifierCriterionAggregation::WinnerVote),
        },
        crate::iterative_workflow::VerifierCriterionDefinition {
            id: "safety".to_string(),
            name: "Safety".to_string(),
            description: "Prefer the safer answer when there is tradeoff.".to_string(),
            weight: Some(5.0),
            aggregation: Some(crate::iterative_workflow::VerifierCriterionAggregation::WinnerVote),
        },
    ];
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![stream_from_text(
        r#"{"winner":"cand-001","scores":[{"criterion_id":"spec","winner":"cand-001","explanation":"candidate one is slightly more polished"},{"criterion_id":"safety","winner":"cand-002","explanation":"candidate two is materially safer"}]}"#,
    )]));
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-weighted-criteria".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    controller
        .begin_iteration(1)
        .expect("first verify iteration should begin");
    controller
        .evaluate_round(
            1,
            &OrchestratorOutput {
                content: "candidate one execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("first verify round should succeed");

    controller
        .begin_iteration(2)
        .expect("second verify iteration should begin");
    let second = controller
        .evaluate_round(
            2,
            &OrchestratorOutput {
                content: "candidate two execution output".to_string(),
                steps: 1,
                tool_calls_count: 0,
                metadata: HashMap::new(),
                finish_reason: crate::runtime::events::FinishReason::EndTurn,
            },
            Vec::new(),
        )
        .await
        .expect("second verify round should succeed");

    assert_eq!(
        second.gate_decision.final_response.as_deref(),
        Some("candidate two execution output")
    );

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn verify_mode_tournament_re_evaluates_candidate_pool() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
    let mut config = verify_workflow_config_with_verifier(
        crate::iterative_workflow::VerifierSelectionStrategy::Tournament,
        1,
    );
    configure_verify_judge_test(&mut config, 3, 20.0);
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
            Ok(ToolOutput {
                output: "score=11".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
            Ok(ToolOutput {
                output: "score=10".to_string(),
                is_error: false,
                title: None,
                metadata: Some(json!({"exit_code": 0})),
            }),
        ])),
    });
    let judge = Arc::new(ScriptedModelResolver::new(vec![
        stream_from_text(r#"{"winner":"cand-003"}"#),
        stream_from_text(r#"{"winner":"cand-001"}"#),
    ]));
    let captured_inputs = judge.captured_inputs();
    let mut controller = WorkflowController::from_config(
        config,
        ToolRunner::new(executor),
        test_exec_ctx(&workdir),
        Some("verify-tournament".to_string()),
    )
    .expect("controller construction should succeed")
    .expect("workflow should activate controller");
    controller.bind_verifier_judge(
        judge,
        Arc::new(crate::traits::NoopLifecycleHook),
        Arc::new(crate::runtime::events::NeverCancel),
        test_exec_ctx(&workdir),
    );

    controller
        .capture_baseline()
        .await
        .expect("baseline should capture");
    for iteration in 1..=3 {
        controller
            .begin_iteration(iteration)
            .expect("verify iteration should begin");
        let result = controller
            .evaluate_round(
                iteration,
                &OrchestratorOutput {
                    content: format!("candidate {iteration} execution output"),
                    steps: 1,
                    tool_calls_count: 0,
                    metadata: HashMap::new(),
                    finish_reason: crate::runtime::events::FinishReason::EndTurn,
                },
                Vec::new(),
            )
            .await
            .expect("verify round should succeed");
        if iteration == 3 {
            assert_eq!(
                result.gate_decision.status,
                SchedulerExecutionGateStatus::Done
            );
            assert_eq!(
                result.gate_decision.final_response.as_deref(),
                Some("candidate 3 execution output")
            );
        }
    }

    let inputs = captured_inputs.lock().await.clone();
    assert_eq!(inputs.len(), 2);
    assert!(inputs
        .iter()
        .all(|input| input.contains("Selection strategy: tournament")));
    assert!(inputs
        .iter()
        .any(|input| { input.contains("id: cand-001") && input.contains("id: cand-003") }));

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn workflow_controller_restores_snapshot_and_continues_on_discard() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
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
        .evaluate_round(1, &OrchestratorOutput::empty(), Vec::new())
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
            &OrchestratorOutput::empty(),
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
            verifier: None,
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
    let manifest = std::fs::read_to_string(run_root.join("run-manifest.json")).expect("manifest");
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
        .evaluate_round(1, &OrchestratorOutput::empty(), Vec::new())
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
            verifier: None,
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
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
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
        .evaluate_round(1, &OrchestratorOutput::empty(), Vec::new())
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
            verifier: None,
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
    let status = run_git(&workdir, ["status", "--short", "--", "src/lib.rs"]).expect("git status");
    assert!(status.trim().is_empty());

    std::fs::remove_dir_all(&workdir).expect("temp workdir should clean up");
}

#[tokio::test]
async fn workflow_controller_squashes_kept_iteration_commits_on_completion() {
    let workdir = new_temp_workdir();
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
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
        .evaluate_round(1, &OrchestratorOutput::empty(), Vec::new())
        .await
        .expect("first workflow evaluation should succeed");
    assert_eq!(first.decision, IterationDecision::Keep);

    controller
        .begin_iteration(2)
        .expect("iteration two should begin");
    std::fs::write(workdir.join("src/lib.rs"), "candidate-two\n")
        .expect("second candidate mutation should write");
    let second = controller
        .evaluate_round(2, &OrchestratorOutput::empty(), Vec::new())
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
            verifier: None,
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
            &OrchestratorOutput::empty(),
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
            verifier: None,
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
    let retry = controller.handle_execution_error(1, &OrchestratorError::Other("boom".to_string()));

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
    let second =
        controller.handle_execution_error(2, &OrchestratorError::Other("still boom".to_string()));
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
    let blocked =
        controller.handle_execution_error(3, &OrchestratorError::Other("final boom".to_string()));
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
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
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
        .evaluate_round(1, &OrchestratorOutput::empty(), Vec::new())
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
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
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
    std::fs::write(workdir.join("src/lib.rs"), "baseline\n").expect("baseline file should write");
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
    std::fs::remove_file(workdir.join("src/local.rs")).expect("iteration removal should succeed");
    std::fs::write(workdir.join("src/new.rs"), "iteration new file\n")
        .expect("iteration new file should write");

    engine
        .restore(&mut checkpoint)
        .expect("stash checkpoint restore should succeed");

    assert_eq!(
        std::fs::read_to_string(workdir.join("src/lib.rs")).expect("tracked file should restore"),
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
