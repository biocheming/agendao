use chrono::{DateTime, Utc};
use rocode_content::stage_protocol::StageStatus;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

use crate::SessionMemoryTelemetrySummary;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diffs: Option<Vec<FileDiff>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionShare {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRevert {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionRuleset {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTime {
    pub created: i64,
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacting: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<i64>,
}

impl Default for SessionTime {
    fn default() -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            created: now,
            updated: now,
            compacting: None,
            archived: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_miss_tokens: u64,
    /// Latest prompt/context window occupancy for the session.
    /// Unlike the other fields, this is not cumulative; it tracks the most
    /// recent assistant turn's context pressure for UI meters and compaction.
    #[serde(default)]
    pub context_tokens: u64,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SessionStatus {
    #[default]
    Active,
    Completed,
    Archived,
    Compacting,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RunStatus {
    Idle,
    Busy,
    Retrying { attempt: u32 },
}

use crate::message::SessionMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTime,
    pub messages: Vec<SessionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<SessionShare>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert: Option<SessionRevert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionRuleset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
    #[serde(default)]
    pub status: SessionStatus,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing)]
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn touch(&mut self) {
        let now = Utc::now();
        self.time.updated = now.timestamp_millis();
        self.updated_at = now;
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionTelemetrySnapshotVersion {
    #[default]
    V1,
    V2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_scheduler_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionListItem {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionListSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<SessionListHints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_command_invocation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListContract {
    pub filter_query_parameters: Vec<String>,
    pub search_fields: Vec<String>,
    pub non_search_fields: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionListResponse {
    pub items: Vec<SessionListItem>,
    pub contract: SessionListContract,
}

pub type SessionTimeInfo = SessionTime;
pub type SessionSummaryInfo = SessionListSummary;
pub type SessionShareInfo = SessionShare;
pub type SessionRevertInfo = SessionRevert;
pub type PermissionRulesetInfo = PermissionRuleset;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMultimodalAttachmentInfo {
    pub filename: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMultimodalInsight {
    pub user_message_id: String,
    pub attachment_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_downgrade: Option<String>,
    #[serde(default)]
    pub hard_block: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_replaced_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SessionMultimodalAttachmentInfo>,
}

impl SessionMultimodalInsight {
    pub fn display_label(&self) -> Cow<'_, str> {
        self.compact_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| {
                if self.attachment_count == 1 {
                    Cow::Borrowed("attachment-backed input")
                } else {
                    Cow::Owned(format!("{} attachments", self.attachment_count))
                }
            })
    }

    pub fn combined_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for warning in self
            .warnings
            .iter()
            .chain(self.transport_warnings.iter())
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !warnings.iter().any(|existing| existing == warning) {
                warnings.push(warning.to_string());
            }
        }
        warnings
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInsightsResponse {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub updated: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<SessionTelemetrySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<crate::SessionMemoryInsight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multimodal: Option<SessionMultimodalInsight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTimeInfo,
    #[serde(default)]
    pub summary: Option<SessionSummaryInfo>,
    #[serde(default)]
    pub share: Option<SessionShareInfo>,
    #[serde(default)]
    pub revert: Option<SessionRevertInfo>,
    #[serde(default)]
    pub permission: Option<PermissionRulesetInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<SessionTelemetrySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusInfo {
    pub status: String,
    pub idle: bool,
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedStageTelemetrySummary {
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
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
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

impl From<rocode_content::stage_protocol::StageSummary> for PersistedStageTelemetrySummary {
    fn from(value: rocode_content::stage_protocol::StageSummary) -> Self {
        Self {
            stage_id: value.stage_id,
            stage_name: value.stage_name,
            index: value.index,
            total: value.total,
            step: value.step,
            step_total: value.step_total,
            status: value.status,
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            reasoning_tokens: value.reasoning_tokens,
            cache_read_tokens: value.cache_read_tokens,
            cache_write_tokens: value.cache_write_tokens,
            focus: value.focus,
            last_event: value.last_event,
            waiting_on: value.waiting_on,
            estimated_context_tokens: value.estimated_context_tokens,
            skill_tree_budget: value.skill_tree_budget,
            skill_tree_truncation_strategy: value.skill_tree_truncation_strategy,
            skill_tree_truncated: value.skill_tree_truncated,
            retry_attempt: value.retry_attempt,
            active_agent_count: value.active_agent_count,
            active_tool_count: value.active_tool_count,
            child_session_count: value.child_session_count,
            primary_child_session_id: value.primary_child_session_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTelemetrySnapshot {
    #[serde(default)]
    pub version: SessionTelemetrySnapshotVersion,
    pub usage: SessionUsage,
    #[serde(default)]
    pub stage_summaries: Vec<PersistedStageTelemetrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<SessionMemoryTelemetrySummary>,
    pub last_run_status: String,
    pub updated_at: i64,
}
