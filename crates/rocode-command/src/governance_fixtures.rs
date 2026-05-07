use crate::output_blocks::SchedulerStageBlock;
use crate::stage_protocol::StageEvent;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerStageGovernanceFixture {
    pub block: SchedulerStageBlock,
    pub payload: Value,
    pub metadata: HashMap<String, Value>,
    pub message_text: String,
}

pub fn canonical_scheduler_stage_fixture() -> SchedulerStageGovernanceFixture {
    serde_json::from_str(include_str!("../governance/scheduler_stage_fixture.json"))
        .expect("valid canonical scheduler stage governance fixture")
}

// ─── Multi-agent replay fixture ──────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct MultiAgentReplayFixture {
    pub description: String,
    pub stages: Vec<StageFixtureEntry>,
    pub session_id: String,
    pub expected: ExpectedAggregates,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageFixtureEntry {
    pub block: SchedulerStageBlock,
    pub metadata: HashMap<String, Value>,
    pub message_text: String,
    pub execution_records: Vec<ExecutionRecordFixture>,
    pub events: Vec<StageEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionRecordFixture {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub status: String,
    pub label: Option<String>,
    pub parent_id: Option<String>,
    pub stage_id: Option<String>,
    pub waiting_on: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedAggregates {
    pub total_stages: usize,
    pub total_execution_records: usize,
    pub total_events: usize,
    pub distinct_stage_ids: Vec<String>,
    pub distinct_agent_labels: Vec<String>,
    pub distinct_tool_labels: Vec<String>,
    pub question_count: usize,
    pub stages_with_attached_sessions: usize,
    pub aggregate_prompt_tokens: u64,
    pub aggregate_completion_tokens: u64,
    pub aggregate_reasoning_tokens: u64,
}

pub fn multi_agent_replay_fixture() -> MultiAgentReplayFixture {
    serde_json::from_str(include_str!(
        "../governance/multi_agent_replay_fixture.json"
    ))
    .expect("valid multi-agent replay governance fixture")
}
