use crate::output_metadata::merge_output_metadata;
use crate::skill_list::SkillListOrchestrator;
use crate::tool_runner::ToolRunner;
use crate::traits::Orchestrator;
use crate::types::{AgentDescriptor, OrchestratorContext, OrchestratorOutput};
use crate::OrchestratorError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGraphNode {
    pub id: String,
    pub agent: AgentDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl SkillGraphNode {
    pub fn new(id: impl Into<String>, agent: AgentDescriptor) -> Self {
        Self {
            id: id.into(),
            agent,
            prompt: None,
        }
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGraphEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub condition: EdgeCondition,
}

impl SkillGraphEdge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, condition: EdgeCondition) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            condition,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum EdgeCondition {
    #[default]
    Always,
    OutputContains(String),
    OutputNotContains(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGraphDefinition {
    #[serde(alias = "entryNodeId")]
    pub entry_node_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<SkillGraphNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<SkillGraphEdge>,
    #[serde(default = "default_max_hops", alias = "maxHops")]
    pub max_hops: u32,
}

impl SkillGraphDefinition {
    pub fn new(entry_node_id: impl Into<String>) -> Self {
        Self {
            entry_node_id: entry_node_id.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            max_hops: 20,
        }
    }

    pub fn with_nodes(mut self, nodes: Vec<SkillGraphNode>) -> Self {
        self.nodes = nodes;
        self
    }

    pub fn with_edges(mut self, edges: Vec<SkillGraphEdge>) -> Self {
        self.edges = edges;
        self
    }

    pub fn with_max_hops(mut self, max_hops: u32) -> Self {
        self.max_hops = max_hops;
        self
    }
}

fn default_max_hops() -> u32 {
    20
}

pub struct SkillGraphOrchestrator {
    graph: SkillGraphDefinition,
    tool_runner: ToolRunner,
    stage_context: Option<(String, u32)>,
}

impl SkillGraphOrchestrator {
    pub fn new(graph: SkillGraphDefinition, tool_runner: ToolRunner) -> Self {
        Self {
            graph,
            tool_runner,
            stage_context: None,
        }
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

    fn compose_transition_input(
        original_input: &str,
        from_node: &str,
        from_output: &str,
        to_node: &str,
    ) -> String {
        format!(
            "Original Task:\n{original_input}\n\nPrevious Node:\n{from_node}\n\nPrevious Output:\n{from_output}\n\nNext Node:\n{to_node}"
        )
    }

    fn evaluate_condition(condition: &EdgeCondition, node_output: &str) -> bool {
        match condition {
            EdgeCondition::Always => true,
            EdgeCondition::OutputContains(needle) => node_output.contains(needle),
            EdgeCondition::OutputNotContains(needle) => !node_output.contains(needle),
        }
    }
}

#[async_trait]
impl Orchestrator for SkillGraphOrchestrator {
    async fn execute(
        &mut self,
        input: &str,
        ctx: &OrchestratorContext,
    ) -> Result<OrchestratorOutput, OrchestratorError> {
        if self.graph.max_hops == 0 {
            return Err(OrchestratorError::Other(
                "skill graph max_hops must be > 0".to_string(),
            ));
        }

        let nodes_by_id: HashMap<&str, &SkillGraphNode> = self
            .graph
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();

        let mut current_node_id = self.graph.entry_node_id.clone();
        let mut current_input = input.to_string();
        let mut total_steps = 0_u32;
        let mut total_tool_calls = 0_u32;
        let mut hops = 0_u32;
        let mut metadata = HashMap::new();

        loop {
            hops += 1;
            if hops > self.graph.max_hops {
                return Err(OrchestratorError::Other(format!(
                    "skill graph exceeded max_hops: {}",
                    self.graph.max_hops
                )));
            }

            let node = nodes_by_id.get(current_node_id.as_str()).ok_or_else(|| {
                OrchestratorError::Other(format!("skill graph node not found: {current_node_id}"))
            })?;

            let node_task = Self::compose_node_task(&current_input, node.prompt.as_deref());
            let mut node_orchestrator =
                SkillListOrchestrator::new(node.agent.clone(), self.tool_runner.clone());
            if let Some((stage_name, stage_index)) = self.stage_context.as_ref() {
                node_orchestrator.set_stage_context(stage_name.clone(), *stage_index);
            }
            let node_output = node_orchestrator.execute(&node_task, ctx).await?;

            total_steps += node_output.steps;
            total_tool_calls += node_output.tool_calls_count;
            merge_output_metadata(&mut metadata, &node_output.metadata);

            let mut next_node_id: Option<String> = None;
            for edge in &self.graph.edges {
                if edge.from != current_node_id {
                    continue;
                }
                if Self::evaluate_condition(&edge.condition, &node_output.content) {
                    next_node_id = Some(edge.to.clone());
                    break;
                }
            }

            match next_node_id {
                Some(next) => {
                    current_input = Self::compose_transition_input(
                        input,
                        &current_node_id,
                        &node_output.content,
                        &next,
                    );
                    current_node_id = next;
                }
                None => {
                    return Ok(OrchestratorOutput {
                        content: node_output.content,
                        steps: total_steps,
                        tool_calls_count: total_tool_calls,
                        metadata,
                        finish_reason: node_output.finish_reason,
                    });
                }
            }
        }
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
                    "sessionId": "task_echo_graph_123",
                    "agentTaskId": "agent-graph-123"
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
    async fn execute_single_node_without_edges() {
        let streams = vec![stream_from(vec![
            agendao_provider::StreamEvent::TextDelta("node-a done".to_string()),
            agendao_provider::StreamEvent::Done,
        ])];
        let (context, runner, _captured) = test_context(streams);

        let graph = SkillGraphDefinition::new("a")
            .with_nodes(vec![SkillGraphNode::new("a", test_agent("agent-a"))]);
        let mut orchestrator = SkillGraphOrchestrator::new(graph, runner);
        let output = orchestrator
            .execute("compile project", &context)
            .await
            .unwrap();

        assert_eq!(output.content, "node-a done");
        assert_eq!(output.steps, 1);
        assert_eq!(output.tool_calls_count, 0);
    }

    #[tokio::test]
    async fn execute_two_nodes_with_always_edge() {
        // pop() order: node-a first, node-b second
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("node-b done".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("node-a draft".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner, captured_inputs) = test_context(streams);

        let graph = SkillGraphDefinition::new("a")
            .with_nodes(vec![
                SkillGraphNode::new("a", test_agent("agent-a")).with_prompt("Analyze first."),
                SkillGraphNode::new("b", test_agent("agent-b")).with_prompt("Finalize."),
            ])
            .with_edges(vec![SkillGraphEdge::new("a", "b", EdgeCondition::Always)]);
        let mut orchestrator = SkillGraphOrchestrator::new(graph, runner);
        let output = orchestrator.execute("fix build", &context).await.unwrap();

        assert_eq!(output.content, "node-b done");
        assert_eq!(output.steps, 2);
        assert_eq!(output.tool_calls_count, 0);

        let inputs = captured_inputs.lock().await.clone();
        assert_eq!(inputs.len(), 2);
        assert!(inputs[1].contains("Previous Node:\na"));
        assert!(inputs[1].contains("Previous Output:\nnode-a draft"));
    }

    #[tokio::test]
    async fn execute_branch_selects_first_matching_edge() {
        // pop() order: node-a first, node-b second
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("node-b done".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("go to b".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner, _captured) = test_context(streams);

        let graph = SkillGraphDefinition::new("a")
            .with_nodes(vec![
                SkillGraphNode::new("a", test_agent("agent-a")),
                SkillGraphNode::new("b", test_agent("agent-b")),
                SkillGraphNode::new("c", test_agent("agent-c")),
            ])
            .with_edges(vec![
                SkillGraphEdge::new(
                    "a",
                    "b",
                    EdgeCondition::OutputContains("go to b".to_string()),
                ),
                SkillGraphEdge::new("a", "c", EdgeCondition::Always),
            ]);
        let mut orchestrator = SkillGraphOrchestrator::new(graph, runner);
        let output = orchestrator.execute("route", &context).await.unwrap();

        assert_eq!(output.content, "node-b done");
        assert_eq!(output.steps, 2);
    }

    #[tokio::test]
    async fn execute_self_loop_hits_max_hops() {
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("retry".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("retry".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
        ];
        let (context, runner, _captured) = test_context(streams);

        let graph = SkillGraphDefinition::new("a")
            .with_max_hops(1)
            .with_nodes(vec![SkillGraphNode::new("a", test_agent("agent-a"))])
            .with_edges(vec![SkillGraphEdge::new("a", "a", EdgeCondition::Always)]);
        let mut orchestrator = SkillGraphOrchestrator::new(graph, runner);
        let err = orchestrator.execute("loop", &context).await.unwrap_err();

        match err {
            OrchestratorError::Other(msg) => assert!(msg.contains("max_hops")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_graph_preserves_continuation_metadata_across_nodes() {
        let streams = vec![
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("node-b done".to_string()),
                agendao_provider::StreamEvent::Done,
            ]),
            stream_from(vec![
                agendao_provider::StreamEvent::TextDelta("node-a done".to_string()),
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
        ];
        let (context, runner, _captured) = test_context(streams);

        let graph = SkillGraphDefinition::new("a")
            .with_nodes(vec![
                SkillGraphNode::new("a", test_agent("agent-a")),
                SkillGraphNode::new("b", test_agent("agent-b")),
            ])
            .with_edges(vec![SkillGraphEdge::new("a", "b", EdgeCondition::Always)]);
        let mut orchestrator = SkillGraphOrchestrator::new(graph, runner);
        let output = orchestrator.execute("fix build", &context).await.unwrap();

        let targets = continuation_targets(&output.metadata);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].session_id, "task_echo_graph_123");
        assert_eq!(targets[0].agent_task_id.as_deref(), Some("agent-graph-123"));
    }
}
