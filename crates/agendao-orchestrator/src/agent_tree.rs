use crate::output_metadata::merge_output_metadata;
use crate::skill_list::SkillListOrchestrator;
use crate::tool_runner::ToolRunner;
use crate::traits::Orchestrator;
use crate::types::{AgentDescriptor, OrchestratorContext, OrchestratorOutput};
use crate::OrchestratorError;
use async_trait::async_trait;
use futures::future::{try_join_all, BoxFuture};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTreeNode {
    pub agent: AgentDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AgentTreeNode>,
}

impl AgentTreeNode {
    pub fn new(agent: AgentDescriptor) -> Self {
        Self {
            agent,
            prompt: None,
            children: Vec::new(),
        }
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    pub fn with_children(mut self, children: Vec<AgentTreeNode>) -> Self {
        self.children = children;
        self
    }
}

pub struct AgentTreeOrchestrator {
    root: AgentTreeNode,
    tool_runner: ToolRunner,
    child_execution_mode: ChildExecutionMode,
    stage_context: Option<(String, u32)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ChildExecutionMode {
    Sequential,
    #[default]
    Parallel,
}

impl AgentTreeOrchestrator {
    pub fn new(root: AgentTreeNode, tool_runner: ToolRunner) -> Self {
        Self {
            root,
            tool_runner,
            child_execution_mode: ChildExecutionMode::default(),
            stage_context: None,
        }
    }

    pub fn with_child_execution_mode(mut self, mode: ChildExecutionMode) -> Self {
        self.child_execution_mode = mode;
        self
    }

    pub fn set_stage_context(&mut self, stage_name: String, stage_index: u32) {
        self.stage_context = Some((stage_name, stage_index));
    }

    fn compose_node_task(input: &str, role_prompt: Option<&str>) -> String {
        match role_prompt {
            Some(role_prompt) => format!("Task:\n{input}\n\nRole:\n{role_prompt}"),
            None => input.to_string(),
        }
    }

    fn compose_child_task(parent_task: &str, parent_output: &str, child_name: &str) -> String {
        format!(
            "Parent Task:\n{parent_task}\n\nParent Draft:\n{parent_output}\n\nDelegated Child Agent:\n{child_name}"
        )
    }

    fn compose_aggregation_task(
        original_task: &str,
        parent_draft: &str,
        child_outputs: &[(String, String)],
    ) -> String {
        let child_summary = child_outputs
            .iter()
            .map(|(name, content)| format!("- {name}: {content}"))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "Original Task:\n{original_task}\n\nYour Previous Draft:\n{parent_draft}\n\nChild Outputs:\n{child_summary}\n\nSynthesize a single final answer."
        )
    }

    fn execute_node<'a>(
        &'a self,
        node: &'a AgentTreeNode,
        input: String,
        ctx: &'a OrchestratorContext,
    ) -> BoxFuture<'a, Result<OrchestratorOutput, OrchestratorError>> {
        Box::pin(async move {
            let node_task = Self::compose_node_task(&input, node.prompt.as_deref());
            let mut orchestrator =
                SkillListOrchestrator::new(node.agent.clone(), self.tool_runner.clone());
            if let Some((ref stage_name, stage_index)) = self.stage_context {
                orchestrator.set_stage_context(stage_name.clone(), stage_index);
            }

            let first_output = orchestrator.execute(&node_task, ctx).await?;
            if node.children.is_empty() {
                return Ok(first_output);
            }

            let mut total_steps = first_output.steps;
            let mut total_tool_calls = first_output.tool_calls_count;
            let mut metadata = first_output.metadata.clone();
            let mut child_outputs: Vec<(String, String)> = Vec::with_capacity(node.children.len());
            let child_results = self
                .execute_children(node, &input, &first_output.content, ctx)
                .await?;

            for (name, child_output) in child_results {
                total_steps += child_output.steps;
                total_tool_calls += child_output.tool_calls_count;
                merge_output_metadata(&mut metadata, &child_output.metadata);
                child_outputs.push((name, child_output.content));
            }

            let aggregate_task =
                Self::compose_aggregation_task(&input, &first_output.content, &child_outputs);
            let aggregate_output = orchestrator.execute(&aggregate_task, ctx).await?;
            merge_output_metadata(&mut metadata, &aggregate_output.metadata);

            Ok(OrchestratorOutput {
                content: aggregate_output.content,
                steps: total_steps + aggregate_output.steps,
                tool_calls_count: total_tool_calls + aggregate_output.tool_calls_count,
                metadata,
                finish_reason: aggregate_output.finish_reason,
            })
        })
    }

    fn execute_children<'a>(
        &'a self,
        node: &'a AgentTreeNode,
        original_input: &'a str,
        parent_output: &'a str,
        ctx: &'a OrchestratorContext,
    ) -> BoxFuture<'a, Result<Vec<(String, OrchestratorOutput)>, OrchestratorError>> {
        Box::pin(async move {
            match self.child_execution_mode {
                ChildExecutionMode::Sequential => {
                    let mut outputs = Vec::with_capacity(node.children.len());
                    for child in &node.children {
                        let child_task = Self::compose_child_task(
                            original_input,
                            parent_output,
                            &child.agent.name,
                        );
                        let child_output = self.execute_node(child, child_task, ctx).await?;
                        outputs.push((child.agent.name.clone(), child_output));
                    }
                    Ok(outputs)
                }
                ChildExecutionMode::Parallel => {
                    let futures = node.children.iter().map(|child| {
                        let child_task = Self::compose_child_task(
                            original_input,
                            parent_output,
                            &child.agent.name,
                        );
                        async move {
                            let output = self.execute_node(child, child_task, ctx).await?;
                            Ok::<(String, OrchestratorOutput), OrchestratorError>((
                                child.agent.name.clone(),
                                output,
                            ))
                        }
                    });
                    try_join_all(futures).await
                }
            }
        })
    }
}

#[async_trait]
impl Orchestrator for AgentTreeOrchestrator {
    async fn execute(
        &mut self,
        input: &str,
        ctx: &OrchestratorContext,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        self.execute_node(&self.root, input.to_string(), ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        output_metadata::continuation_targets, AgentResolver, ExecutionContext, LifecycleHook,
        ModelRef, ModelResolver, OrchestratorContext, ToolExecError, ToolExecutor, ToolOutput,
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
        fn resolve(&self, _name: &str) -> Option<AgentDescriptor> {
            None
        }
    }

    struct TestModelResolver {
        streams: Mutex<Vec<agendao_provider::StreamResult>>,
        captured_inputs: Arc<Mutex<Vec<String>>>,
    }

    impl TestModelResolver {
        fn new(
            streams: Vec<agendao_provider::StreamResult>,
            captured_inputs: Arc<Mutex<Vec<String>>>,
        ) -> Self {
            Self {
                streams: Mutex::new(streams),
                captured_inputs,
            }
        }

        fn extract_last_user_text(messages: &[agendao_provider::Message]) -> String {
            messages
                .iter()
                .rev()
                .find_map(|m| match (&m.role, &m.content) {
                    (agendao_provider::Role::User, agendao_provider::Content::Text(text)) => {
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
            messages: Vec<agendao_provider::Message>,
            _tools: Vec<agendao_provider::ToolDefinition>,
            _exec_ctx: &ExecutionContext,
        ) -> Result<agendao_provider::StreamResult, OrchestratorError> {
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
                metadata: Some(json!({
                    "sessionId": "task_echo_tree_123",
                    "agentTaskId": "agent-tree-123"
                })),
            })
        }

        async fn list_ids(&self) -> Vec<String> {
            vec!["echo".to_string(), "invalid".to_string()]
        }

        async fn list_definitions(
            &self,
            _exec_ctx: &ExecutionContext,
        ) -> Vec<agendao_provider::ToolDefinition> {
            vec![agendao_provider::ToolDefinition {
                name: "echo".to_string(),
                description: Some("echo input".to_string()),
                parameters: json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } }
                }),
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

    fn stream_from(events: Vec<agendao_provider::StreamEvent>) -> agendao_provider::StreamResult {
        Box::pin(stream::iter(
            events
                .into_iter()
                .map(Ok::<_, agendao_provider::ProviderError>),
        ))
    }

    fn test_context(
        streams: Vec<agendao_provider::StreamResult>,
    ) -> (OrchestratorContext, ToolRunner, Arc<Mutex<Vec<String>>>) {
        let captured_inputs = Arc::new(Mutex::new(Vec::new()));
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(TestToolExecutor);
        let context = OrchestratorContext {
            agent_resolver: Arc::new(TestAgentResolver),
            model_resolver: Arc::new(TestModelResolver::new(streams, captured_inputs.clone())),
            tool_executor: tool_executor.clone(),
            lifecycle_hook: Arc::new(TestLifecycleHook),
            cancel_token: Arc::new(crate::runtime::events::NeverCancel),
            exec_ctx: ExecutionContext {
                session_id: "test".to_string(),
                workdir: ".".to_string(),
                agent_name: "test-agent".to_string(),
                metadata: HashMap::new(),
            },
        };
        (context, ToolRunner::new(tool_executor), captured_inputs)
    }

    fn test_agent(name: &str) -> AgentDescriptor {
        AgentDescriptor {
            name: name.to_string(),
            system_prompt: None,
            model: Some(ModelRef {
                provider_id: "openai".to_string(),
                model_id: "gpt-test".to_string(),
            }),
            max_steps: Some(4),
            temperature: None,
            allowed_tools: Vec::new(),
        }
    }

    #[tokio::test]
    async fn execute_leaf_node_returns_single_result() {
        let streams = vec![stream_from(vec![
            agendao_provider::StreamEvent::TextDelta("leaf done".to_string()),
            agendao_provider::StreamEvent::Done,
        ])];
        let (context, runner, captured_inputs) = test_context(streams);

        let root = AgentTreeNode::new(test_agent("root"));
        let mut orchestrator = AgentTreeOrchestrator::new(root, runner);
        let output = orchestrator.execute("solve task", &context).await.unwrap();

        assert_eq!(output.content, "leaf done");
        assert_eq!(output.steps, 1);
        assert_eq!(output.tool_calls_count, 0);

        let inputs = captured_inputs.lock().await.clone();
        assert_eq!(inputs, vec!["solve task".to_string()]);
    }

    #[tokio::test]
    async fn execute_parent_then_child_then_parent_aggregate() {
        // pop() order: root first -> child -> root aggregate
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("root final synthesis".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("child analysis".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("root draft".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner, captured_inputs) = test_context(streams);

        let child = AgentTreeNode::new(test_agent("child")).with_prompt("Focus on edge cases.");
        let root = AgentTreeNode::new(test_agent("root"))
            .with_prompt("Plan before synthesis.")
            .with_children(vec![child]);
        let mut orchestrator = AgentTreeOrchestrator::new(root, runner);

        let output = orchestrator
            .execute("fix flaky tests", &context)
            .await
            .unwrap();
        assert_eq!(output.content, "root final synthesis");
        assert_eq!(output.steps, 3);
        assert_eq!(output.tool_calls_count, 0);

        let inputs = captured_inputs.lock().await.clone();
        assert_eq!(inputs.len(), 3);
        assert!(inputs[0].contains("fix flaky tests"));
        assert!(inputs[0].contains("Plan before synthesis."));
        assert!(inputs[1].contains("root draft"));
        assert!(inputs[1].contains("Focus on edge cases."));
        assert!(inputs[2].contains("root draft"));
        assert!(inputs[2].contains("child analysis"));
    }

    #[tokio::test]
    async fn execute_parent_two_children_parallel_then_parent_aggregate() {
        // pop() order: root first -> child(unknown order) x2 -> root aggregate
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("root final synthesis".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("child B analysis".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("child A analysis".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("root draft".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner, captured_inputs) = test_context(streams);

        let child_a = AgentTreeNode::new(test_agent("child-a")).with_prompt("Focus on API.");
        let child_b = AgentTreeNode::new(test_agent("child-b")).with_prompt("Focus on tests.");
        let root = AgentTreeNode::new(test_agent("root"))
            .with_prompt("Plan before synthesis.")
            .with_children(vec![child_a, child_b]);
        let mut orchestrator = AgentTreeOrchestrator::new(root, runner)
            .with_child_execution_mode(ChildExecutionMode::Parallel);

        let output = orchestrator
            .execute("stabilize refactor", &context)
            .await
            .unwrap();
        assert_eq!(output.content, "root final synthesis");
        assert_eq!(output.steps, 4);
        assert_eq!(output.tool_calls_count, 0);

        let inputs = captured_inputs.lock().await.clone();
        assert_eq!(inputs.len(), 4);
        assert!(inputs[0].contains("stabilize refactor"));
        assert!(inputs[0].contains("Plan before synthesis."));

        let child_inputs = &inputs[1..3];
        assert!(child_inputs.iter().any(|s| s.contains("child-a")));
        assert!(child_inputs.iter().any(|s| s.contains("Focus on API.")));
        assert!(child_inputs.iter().any(|s| s.contains("child-b")));
        assert!(child_inputs.iter().any(|s| s.contains("Focus on tests.")));

        assert!(inputs[3].contains("root draft"));
        assert!(inputs[3].contains("child"));
    }

    #[tokio::test]
    async fn execute_aggregate_preserves_child_continuation_metadata() {
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("root final synthesis".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("child done".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::ToolCallEnd {
                    id: "tool-call-1".to_string(),
                    name: "echo".to_string(),
                    input: json!({"value":"x"}),
                },
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("root draft".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner, _captured_inputs) = test_context(streams);

        let child = AgentTreeNode::new(test_agent("child"));
        let root = AgentTreeNode::new(test_agent("root")).with_children(vec![child]);
        let mut orchestrator = AgentTreeOrchestrator::new(root, runner);
        let output = orchestrator.execute("solve task", &context).await.unwrap();

        let targets = continuation_targets(&output.metadata);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].session_id, "task_echo_tree_123");
        assert_eq!(targets[0].agent_task_id.as_deref(), Some("agent-tree-123"));
    }

    // ── Phase 6: Architecture regression test ──

    /// Invariant: AgentTreeOrchestrator supports set_stage_context so that
    /// stage/event/tool activity can be attributed to the current scheduler
    /// stage, matching the observability contract of SkillGraphOrchestrator.
    #[test]
    fn agent_tree_orchestrator_accepts_stage_context() {
        let root = AgentTreeNode::new(test_agent("root"));
        let runner = ToolRunner::new(Arc::new(TestToolExecutor) as Arc<dyn ToolExecutor>);
        let mut orchestrator = AgentTreeOrchestrator::new(root, runner);

        // Must compile and not panic — the stage context is stored internally
        // and forwarded to SkillListOrchestrator during execute_node.
        orchestrator.set_stage_context("execution-orchestration".to_string(), 3);
    }
}
