use super::*;
use crate::{
    runtime::events::NeverCancel, SchedulerEffectContext, SchedulerEffectDispatch,
    SchedulerEffectKind, SchedulerStageKind, ToolRunner,
};

use crate::traits::{AgentResolver, LifecycleHook, ModelResolver, Orchestrator, ToolExecutor};
use crate::{
    ExecutionContext, ModelRef, OrchestratorContext, OrchestratorError, ToolExecError, ToolOutput,
};
use async_trait::async_trait;
use futures::stream;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

struct TestAgentResolver;

#[async_trait]
impl AgentResolver for TestAgentResolver {
    fn resolve(&self, _name: &str) -> Option<crate::AgentDescriptor> {
        None
    }
}

struct TestModelResolver {
    streams: Mutex<Vec<rocode_provider::StreamResult>>,
    captured_inputs: Arc<Mutex<Vec<String>>>,
}

impl TestModelResolver {
    fn new(
        streams: Vec<rocode_provider::StreamResult>,
        captured_inputs: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            streams: Mutex::new(streams),
            captured_inputs,
        }
    }

    fn extract_last_user_text(messages: &[rocode_provider::Message]) -> String {
        messages
            .iter()
            .rev()
            .find_map(|m| match (&m.role, &m.content) {
                (rocode_provider::Role::User, rocode_provider::Content::Text(text)) => {
                    Some(text.clone())
                }
                _ => None,
            })
            .unwrap_or_default()
    }
}

#[async_trait]
impl ModelResolver for TestModelResolver {
    async fn chat_stream(
        &self,
        _model: Option<&ModelRef>,
        messages: Vec<rocode_provider::Message>,
        _tools: Vec<rocode_provider::ToolDefinition>,
        _exec_ctx: &ExecutionContext,
    ) -> Result<rocode_provider::StreamResult, OrchestratorError> {
        let input = Self::extract_last_user_text(&messages);
        self.captured_inputs.lock().await.push(input);
        self.streams
            .lock()
            .await
            .pop()
            .ok_or_else(|| OrchestratorError::Other("missing test stream".to_string()))
    }
}

struct TestToolExecutor;

#[async_trait]
impl ToolExecutor for TestToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        _arguments: serde_json::Value,
        _exec_ctx: &ExecutionContext,
    ) -> Result<ToolOutput, ToolExecError> {
        Ok(ToolOutput {
            output: format!("tool:{tool_name}:ok"),
            is_error: false,
            title: Some("ok".to_string()),
            metadata: None,
        })
    }

    async fn list_ids(&self) -> Vec<String> {
        vec!["echo".to_string(), "invalid".to_string()]
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &ExecutionContext,
    ) -> Vec<rocode_provider::ToolDefinition> {
        vec![rocode_provider::ToolDefinition {
            name: "echo".to_string(),
            description: Some("echo input".to_string()),
            parameters: json!({"type": "object"}),
        }]
    }
}

struct TestLifecycleHook;

#[async_trait]
impl LifecycleHook for TestLifecycleHook {
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
}

fn stream_from(events: Vec<rocode_provider::StreamEvent>) -> rocode_provider::StreamResult {
    Box::pin(stream::iter(
        events
            .into_iter()
            .map(Ok::<_, rocode_provider::ProviderError>),
    ))
}

fn test_context(
    streams: Vec<rocode_provider::StreamResult>,
) -> (OrchestratorContext, ToolRunner, Arc<Mutex<Vec<String>>>) {
    let captured_inputs = Arc::new(Mutex::new(Vec::new()));
    let tool_executor: Arc<dyn ToolExecutor> = Arc::new(TestToolExecutor);
    let context = OrchestratorContext {
        agent_resolver: Arc::new(TestAgentResolver),
        model_resolver: Arc::new(TestModelResolver::new(streams, captured_inputs.clone())),
        tool_executor: tool_executor.clone(),
        lifecycle_hook: Arc::new(TestLifecycleHook),
        cancel_token: Arc::new(NeverCancel),
        exec_ctx: ExecutionContext {
            session_id: "test".to_string(),
            workdir: ".".to_string(),
            agent_name: "sisyphus".to_string(),
            metadata: HashMap::new(),
        },
    };
    (context, ToolRunner::new(tool_executor), captured_inputs)
}

#[test]
fn sisyphus_uses_execution_orchestration_stages() {
    assert_eq!(
        sisyphus_default_stages(),
        vec![
            SchedulerStageKind::RequestAnalysis,
            SchedulerStageKind::ExecutionOrchestration,
        ]
    );
}

#[test]
fn sisyphus_plan_sets_orchestrator() {
    let plan = sisyphus_plan();
    assert_eq!(plan.orchestrator.as_deref(), Some("sisyphus"));
}

#[test]
fn sisyphus_effect_protocol_registers_workflow_todos() {
    let effects = sisyphus_plan().effect_protocol();
    assert!(effects.effects.iter().any(|effect| {
        effect.stage == SchedulerStageKind::ExecutionOrchestration
            && effect.moment == crate::SchedulerEffectMoment::OnEnter
            && effect.effect == SchedulerEffectKind::RegisterWorkflowTodos
    }));
}

#[test]
fn sisyphus_workflow_todos_match_omo_execution_shape() {
    let payload = sisyphus_workflow_todos_payload();
    let todos = payload["todos"].as_array().expect("todos array");
    assert_eq!(todos.len(), 5);
    assert!(todos[0]["content"].as_str().unwrap().contains("intent"));
    assert!(todos[2]["content"].as_str().unwrap().contains("parallel"));
    assert!(todos[3]["content"]
        .as_str()
        .unwrap()
        .contains("task tracking"));
    assert!(todos[4]["content"]
        .as_str()
        .unwrap()
        .contains("Verify evidence"));
}

#[test]
fn sisyphus_uses_shared_effect_dispatch_framework() {
    let dispatch = sisyphus_plan().effect_dispatch(
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

#[tokio::test]
async fn sisyphus_runs_single_execution_orchestration_without_review_or_synthesis() {
    let streams = vec![stream_from(vec![
        rocode_provider::StreamEvent::TextDelta("sisyphus shipped the change".to_string()),
        rocode_provider::StreamEvent::Done,
    ])];
    let (context, runner, captured_inputs) = test_context(streams);

    let mut plan = sisyphus_plan().with_description(Some(
        "OMO-style single-loop delegation-first orchestrator".to_string(),
    ));
    plan.skill_list = vec!["review-pr".into(), "simplify".into()];
    plan.available_agents = vec![
        crate::scheduler::AvailableAgentMeta {
            name: "explore".to_string(),
            description: "Exploration subagent for searching code.".to_string(),
            mode: "subagent".to_string(),
            cost: "CHEAP".to_string(),
        },
        crate::scheduler::AvailableAgentMeta {
            name: "oracle".to_string(),
            description: "High-IQ reasoning specialist.".to_string(),
            mode: "subagent".to_string(),
            cost: "EXPENSIVE".to_string(),
        },
    ];
    plan.available_categories = vec![crate::scheduler::AvailableCategoryMeta {
        name: "rust".to_string(),
        description: "Rust implementation and debugging tasks".to_string(),
    }];
    let mut orchestrator = SisyphusOrchestrator::new(plan, runner);
    let output = orchestrator
        .execute("fix the flaky migration test", &context)
        .await
        .unwrap();

    assert!(output.content.contains("## Delivery Summary"));
    assert!(output.content.contains("**Delegation Path**"));
    assert!(output.content.contains("**Execution Outcome**"));
    assert!(output.content.contains("**Verification**"));
    assert!(output.content.contains("sisyphus shipped the change"));
    assert_eq!(output.steps, 1);

    let inputs = captured_inputs.lock().await.clone();
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].contains("## Stage\nexecution-orchestration"));
    assert!(inputs[0].contains("fix the flaky migration test"));
    assert!(inputs[0].contains("## Execution Frame"));
    assert!(inputs[0].contains("single-loop execution orchestration"));
    assert!(inputs[0].contains("not an interview-first planning workflow"));
    assert!(inputs[0].contains("do not become the sole implementer for non-trivial work"));
    assert!(inputs[0].contains("make the triviality judgment obvious from the result"));
    assert!(inputs[0].contains("## Execution Priorities"));
    assert!(inputs[0].contains("explore/librarian research in parallel"));
    assert!(inputs[0].contains("evidence-backed verification"));
    assert!(inputs[0].contains("## Delivery Contract"));
    assert!(!inputs[0].contains("## Stage\nroute"));
    assert!(!inputs[0].contains("Phase 0 - Intent Gate"));
    assert!(!inputs[0].contains("Phase 3 - Completion"));
    assert!(!inputs[0].contains("<available_skills compact=\"true\">"));
    assert!(!inputs[0].contains("`explore` agent — **CHEAP**"));
    assert!(!inputs[0].contains("`oracle` agent — **EXPENSIVE**"));
    assert!(!inputs[0].contains("Explore Agent = Contextual Grep"));
    assert!(!inputs[0].contains("Oracle_Usage"));
    assert!(!inputs[0].contains("`rust` — Rust implementation and debugging tasks"));
    assert!(!inputs[0].contains("general: review-pr, simplify"));
    assert!(!inputs[0].contains("## Stage\nreview"));
    assert!(!inputs[0].contains("## Stage\nsynthesis"));
    assert!(!inputs[0].contains("## Stage\ndelegation"));
}
