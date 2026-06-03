use crate::output_blocks::SchedulerStageBlock;
use crate::stage_protocol::StageEvent;
use agendao_types::{
    scheduler_stage_part_key, tool_call_part_key, tool_result_part_key, LiveMessagePartIdentity,
    ASSISTANT_TEXT_MAIN_PART_KEY,
};
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

// ─── Live transcript state fixture ──────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct LiveTranscriptStateFixture {
    pub description: String,
    pub version: u64,
    pub contract_version: String,
    pub canonical_live_stream: CanonicalLiveStreamFixture,
    pub shared_turn_cycles: SharedTurnCyclesFixture,
    pub tool_progress_exclusion: ToolProgressExclusionFixture,
    pub scheduler_stage_exclusion: SchedulerStageExclusionFixture,
    pub run_tail_contract: RunTailContractFixture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanonicalLiveStreamFixture {
    pub description: String,
    pub events: Vec<CanonicalLiveEventFixture>,
    pub expected: CanonicalLiveExpectedFixture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanonicalLiveEventFixture {
    pub kind: String,
    pub phase: String,
    pub role: Option<String>,
    pub text: Option<String>,
    pub detail: Option<String>,
    pub name: Option<String>,
    pub id: Option<String>,
    pub live_identity: Option<LiveMessagePartIdentity>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanonicalLiveExpectedFixture {
    pub transcript_blocks: CanonicalTranscriptExpectation,
    pub activity_blocks: CanonicalActivityExpectation,
    pub no_duplicate_headers: bool,
    pub no_replay_on_history_reload: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanonicalTranscriptExpectation {
    pub order: Vec<String>,
    pub assistant_count: usize,
    pub thinking_count: usize,
    pub tool_count: usize,
    pub tool_running_in_transcript: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanonicalActivityExpectation {
    pub tool_running_visible: bool,
    pub tool_running_count: usize,
    pub scheduler_stage_in_activity: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedTurnCyclesFixture {
    pub entries: Vec<SharedTurnCycleEntry>,
    pub expected: SharedTurnCyclesExpected,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedTurnCyclesExpected {
    pub assistant_message_count: usize,
    pub tool_result_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedTurnCycleEntry {
    pub message_id: String,
    pub message_text: String,
    pub tool: Option<SharedTurnCycleTool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedTurnCycleTool {
    pub tool_id: String,
    pub tool_name: String,
    pub tool_detail: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolProgressExclusionFixture {
    pub message: ToolProgressMessageFixture,
    pub tool_running: ToolProgressToolFixture,
    pub tool_result: ToolProgressToolFixture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolProgressMessageFixture {
    pub message_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolProgressToolFixture {
    pub tool_id: String,
    pub tool_name: String,
    pub tool_detail: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerStageExclusionFixture {
    pub message_id: String,
    pub stage_id: String,
    pub stage: String,
    pub title: String,
    pub text: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunTailContractFixture {
    pub completed_status: String,
    pub completed_usage: RunTailUsageFixture,
    pub error_status: String,
    pub error_message: String,
    pub awaiting_user_status: String,
    pub awaiting_user_detail: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunTailUsageFixture {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_cost: f64,
}

impl SharedTurnCycleEntry {
    pub fn assistant_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message_id.clone(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: agendao_types::LiveMessagePartKind::AssistantText,
            phase: agendao_types::LivePartPhase::Snapshot,
            legacy_block_id: Some(self.message_id.clone()),
        }
    }
}

impl SharedTurnCycleTool {
    pub fn tool_result_identity(&self, message_id: &str) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: message_id.to_string(),
            part_key: tool_result_part_key(&self.tool_id),
            part_kind: agendao_types::LiveMessagePartKind::ToolResult,
            phase: agendao_types::LivePartPhase::End,
            legacy_block_id: Some(self.tool_id.clone()),
        }
    }
}

impl ToolProgressExclusionFixture {
    pub fn message_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message.message_id.clone(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: agendao_types::LiveMessagePartKind::AssistantText,
            phase: agendao_types::LivePartPhase::Snapshot,
            legacy_block_id: Some(self.message.message_id.clone()),
        }
    }

    pub fn tool_running_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message.message_id.clone(),
            part_key: tool_call_part_key(&self.tool_running.tool_id),
            part_kind: agendao_types::LiveMessagePartKind::ToolCall,
            phase: agendao_types::LivePartPhase::Snapshot,
            legacy_block_id: Some(self.tool_running.tool_id.clone()),
        }
    }

    pub fn tool_result_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message.message_id.clone(),
            part_key: tool_result_part_key(&self.tool_result.tool_id),
            part_kind: agendao_types::LiveMessagePartKind::ToolResult,
            phase: agendao_types::LivePartPhase::End,
            legacy_block_id: Some(self.tool_result.tool_id.clone()),
        }
    }
}

impl SchedulerStageExclusionFixture {
    pub fn scheduler_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message_id.clone(),
            part_key: scheduler_stage_part_key(&self.stage_id),
            part_kind: agendao_types::LiveMessagePartKind::SchedulerStage,
            phase: agendao_types::LivePartPhase::Snapshot,
            legacy_block_id: None,
        }
    }

    pub fn payload(&self) -> Value {
        serde_json::json!({
            "kind": "scheduler_stage",
            "stage_id": self.stage_id,
            "profile": "atlas",
            "stage": self.stage,
            "title": self.title,
            "text": self.text,
            "stage_index": 1,
            "stage_total": 1,
            "status": self.status,
            "active_agents": [],
            "active_skills": [],
            "active_categories": [],
            "done_agent_count": 0,
            "total_agent_count": 0
        })
    }
}

impl CanonicalLiveEventFixture {
    pub fn payload(&self) -> Value {
        let mut payload = serde_json::Map::new();
        payload.insert("kind".to_string(), Value::String(self.kind.clone()));
        payload.insert("phase".to_string(), Value::String(self.phase.clone()));
        if let Some(role) = &self.role {
            payload.insert("role".to_string(), Value::String(role.clone()));
        }
        if let Some(text) = &self.text {
            payload.insert("text".to_string(), Value::String(text.clone()));
        }
        if let Some(detail) = &self.detail {
            payload.insert("detail".to_string(), Value::String(detail.clone()));
        }
        if let Some(name) = &self.name {
            payload.insert("name".to_string(), Value::String(name.clone()));
        }
        Value::Object(payload)
    }
}

pub fn live_transcript_state_fixture() -> LiveTranscriptStateFixture {
    serde_json::from_str(include_str!(
        "../governance/live_transcript_state_fixture.json"
    ))
    .expect("valid live transcript state fixture")
}
