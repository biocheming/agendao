//! Stage Protocol — canonical protocol types for the three-layer stage architecture.
//!
//! Three orthogonal layers, each with a single authority:
//!
//! | Layer              | Struct            | Purpose                                |
//! |--------------------|-------------------|----------------------------------------|
//! | Stage Summary      | [`StageSummary`]  | Stable card the user sees (aggregated) |
//! | Execution Topology | [`ExecutionNode`] | Active tree: stage→agent→tool/question |
//! | Raw SSE            | [`StageEvent`]    | Real-time event stream & history replay|

use serde::{Deserialize, Serialize};

pub mod telemetry_event_names {
    pub const SESSION_UPDATED: &str = "session.updated";
    pub const SESSION_STATUS: &str = "session.status";
    pub const SESSION_USAGE: &str = "session.usage";
    pub const SESSION_ERROR: &str = "session.error";
    pub const QUESTION_CREATED: &str = "question.created";
    pub const QUESTION_RESOLVED: &str = "question.resolved";
    pub const PERMISSION_REQUESTED: &str = "permission.requested";
    pub const PERMISSION_RESOLVED: &str = "permission.resolved";
    pub const TOOL_STARTED: &str = "tool.started";
    pub const TOOL_COMPLETED: &str = "tool.completed";
    pub const EXECUTION_TOPOLOGY_CHANGED: &str = "execution.topology.changed";
    pub const DIFF_UPDATED: &str = "diff.updated";
    pub const CHILD_SESSION_ATTACHED: &str = "child_session.attached";
    pub const CHILD_SESSION_DETACHED: &str = "child_session.detached";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageSummary {
    pub stage_id: String,
    pub stage_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_total: Option<u64>,
    pub status: StageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_miss_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree_budget: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree_truncation_strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_attempt: Option<u64>,
    pub active_agent_count: u32,
    pub active_tool_count: u32,
    pub child_session_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_child_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Running,
    Waiting,
    Done,
    Cancelled,
    Cancelling,
    Blocked,
    Retrying,
}

impl StageStatus {
    pub fn from_str_lossy(s: Option<&str>) -> Self {
        match s {
            Some("done") => Self::Done,
            Some("cancelled") => Self::Cancelled,
            Some("cancelling") => Self::Cancelling,
            Some("waiting") => Self::Waiting,
            Some("blocked") => Self::Blocked,
            Some("retrying") => Self::Retrying,
            Some("running") => Self::Running,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionNode {
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    pub kind: ExecutionNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: ExecutionNodeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionNodeKind {
    Stage,
    Agent,
    Tool,
    Question,
    Subsession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionNodeStatus {
    Running,
    Waiting,
    Cancelling,
    Retry,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageEvent {
    pub event_id: String,
    pub scope: EventScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    pub event_type: String,
    pub ts: i64,
    pub payload: serde_json::Value,
}

impl StageEvent {
    pub fn new(
        scope: EventScope,
        stage_id: Option<String>,
        execution_id: Option<String>,
        event_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_id: format!("evt_{}", uuid::Uuid::new_v4().simple()),
            scope,
            stage_id,
            execution_id,
            event_type: event_type.into(),
            ts: chrono::Utc::now().timestamp_millis(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventScope {
    Session,
    Stage,
    Agent,
}

pub fn parse_step_limit_from_budget(budget: Option<&str>) -> Option<u64> {
    let s = budget?;
    let rest = s.strip_prefix("step-limit:")?;
    rest.trim().parse::<u64>().ok()
}
