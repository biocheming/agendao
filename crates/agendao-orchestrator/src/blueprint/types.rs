use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }
    };
}

string_id!(BlueprintName);
string_id!(NodeId);
string_id!(AgentId);
string_id!(SkillId);
string_id!(ToolId);
string_id!(EvaluatorId);
string_id!(CapabilityId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlueprintSchemaVersion {
    V1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerBlueprint {
    pub schema: BlueprintSchemaVersion,
    pub name: BlueprintName,
    pub entry: NodeId,
    pub nodes: BTreeMap<NodeId, NodeSpec>,
    pub limits: ExecutionLimits,
    pub output: OutputContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NodeSpec {
    Agent(AgentNode),
    Parallel(ParallelNode),
    Gate(GateNode),
    Loop(LoopNode),
    End(EndNode),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentNode {
    pub agent: AgentId,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub skills: BTreeSet<SkillId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub tools: BTreeSet<ToolId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_model_capabilities: BTreeSet<ModelCapability>,
    pub max_steps: u32,
    pub next: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelNode {
    pub branches: Vec<NodeId>,
    pub join: NodeId,
    pub max_parallelism: u32,
    pub failure_mode: ParallelFailureMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParallelFailureMode {
    FailFast,
    Collect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateNode {
    pub evaluator: EvaluatorId,
    pub on_pass: NodeId,
    pub on_fail: NodeId,
    pub on_indeterminate: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopNode {
    pub body: Box<SchedulerGraph>,
    pub evaluator: EvaluatorId,
    pub max_iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<CapabilityId>,
    pub on_satisfied: NodeId,
    pub on_exhausted: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerGraph {
    pub entry: NodeId,
    pub nodes: BTreeMap<NodeId, NodeSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndNode {
    pub result: ResultSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResultSource {
    LastNode,
    Named(NodeId),
    Artifact {
        capability: CapabilityId,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputContract {
    pub format: OutputFormat,
    pub include_usage: bool,
    pub include_artifact_refs: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    Text,
    Markdown,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub max_total_tokens: u64,
    pub max_wall_time_ms: u64,
    pub max_parallelism: u32,
    pub max_graph_nodes: u32,
    pub max_graph_depth: u32,
    pub max_loop_iterations: u32,
    pub max_agent_steps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelCapability {
    ToolCalls,
    Reasoning,
    Attachments,
    StructuredOutput,
}
