use agendao_types::{
    tool_call_part_key, tool_result_part_key, LiveMessagePartIdentity, ASSISTANT_TEXT_MAIN_PART_KEY,
};
use serde::Deserialize;
use serde_json::Value;

// ─── Live transcript state fixture ──────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct LiveTranscriptStateFixture {
    pub description: String,
    pub version: u64,
    pub contract_version: String,
    pub canonical_live_stream: CanonicalLiveStreamFixture,
    pub shared_turn_cycles: SharedTurnCyclesFixture,
    pub tool_progress_exclusion: ToolProgressExclusionFixture,
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
        }
    }

    pub fn tool_running_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message.message_id.clone(),
            part_key: tool_call_part_key(&self.tool_running.tool_id),
            part_kind: agendao_types::LiveMessagePartKind::ToolCall,
            phase: agendao_types::LivePartPhase::Snapshot,
        }
    }

    pub fn tool_result_identity(&self) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: self.message.message_id.clone(),
            part_key: tool_result_part_key(&self.tool_result.tool_id),
            part_kind: agendao_types::LiveMessagePartKind::ToolResult,
            phase: agendao_types::LivePartPhase::End,
        }
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
        "../../agendao-command/governance/live_transcript_state_fixture.json"
    ))
    .expect("valid live transcript state fixture")
}
