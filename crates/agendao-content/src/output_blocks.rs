use crate::stage_protocol::{parse_step_limit_from_budget, StageStatus, StageSummary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockTone {
    Title,
    Normal,
    Muted,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBlock {
    pub tone: BlockTone,
    pub text: String,
}

impl StatusBlock {
    pub fn title(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Title,
            text: text.into(),
        }
    }

    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Normal,
            text: text.into(),
        }
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Muted,
            text: text.into(),
        }
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Success,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Error,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePhase {
    Start,
    Delta,
    End,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningBlock {
    pub phase: MessagePhase,
    pub text: String,
}

impl ReasoningBlock {
    pub fn start() -> Self {
        Self {
            phase: MessagePhase::Start,
            text: String::new(),
        }
    }

    pub fn delta(text: impl Into<String>) -> Self {
        Self {
            phase: MessagePhase::Delta,
            text: text.into(),
        }
    }

    pub fn end() -> Self {
        Self {
            phase: MessagePhase::End,
            text: String::new(),
        }
    }

    pub fn full(text: impl Into<String>) -> Self {
        Self {
            phase: MessagePhase::Full,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBlock {
    pub role: MessageRole,
    pub phase: MessagePhase,
    pub text: String,
}

impl MessageBlock {
    pub fn start(role: MessageRole) -> Self {
        Self {
            role,
            phase: MessagePhase::Start,
            text: String::new(),
        }
    }

    pub fn delta(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            phase: MessagePhase::Delta,
            text: text.into(),
        }
    }

    pub fn end(role: MessageRole) -> Self {
        Self {
            role,
            phase: MessagePhase::End,
            text: String::new(),
        }
    }

    pub fn full(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            phase: MessagePhase::Full,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPhase {
    Start,
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStructuredDetail {
    FileEdit {
        file_path: String,
        diff_preview: Option<String>,
    },
    FileWrite {
        file_path: String,
        bytes: Option<u64>,
        lines: Option<u64>,
        diff_preview: Option<String>,
    },
    FileRead {
        file_path: String,
        total_lines: Option<u64>,
        truncated: bool,
    },
    BashExec {
        command_preview: String,
        exit_code: Option<i64>,
        output_preview: Option<String>,
        truncated: bool,
    },
    Search {
        pattern: String,
        matches: Option<u64>,
        truncated: bool,
    },
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolBlock {
    pub name: String,
    pub phase: ToolPhase,
    pub detail: Option<String>,
    pub structured: Option<ToolStructuredDetail>,
}

impl ToolBlock {
    pub fn start(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Start,
            detail: None,
            structured: None,
        }
    }

    pub fn running(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Running,
            detail: Some(detail.into()),
            structured: None,
        }
    }

    pub fn done(name: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Done,
            detail,
            structured: None,
        }
    }

    pub fn error(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Error,
            detail: Some(detail.into()),
            structured: None,
        }
    }

    pub fn with_structured(mut self, detail: ToolStructuredDetail) -> Self {
        self.structured = Some(detail);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerDecisionField {
    pub label: String,
    pub value: String,
    pub tone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerDecisionSection {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerDecisionRenderSpec {
    pub version: String,
    pub show_header_divider: bool,
    pub field_order: String,
    pub field_label_emphasis: String,
    pub status_palette: String,
    pub section_spacing: String,
    pub update_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerDecisionBlock {
    pub kind: String,
    pub title: String,
    pub spec: SchedulerDecisionRenderSpec,
    pub fields: Vec<SchedulerDecisionField>,
    pub sections: Vec<SchedulerDecisionSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventField {
    pub label: String,
    pub value: String,
    pub tone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventBlock {
    pub event: String,
    pub title: String,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub fields: Vec<SessionEventField>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItemBlock {
    pub position: usize,
    pub text: String,
}

pub fn default_scheduler_decision_render_spec() -> SchedulerDecisionRenderSpec {
    SchedulerDecisionRenderSpec {
        version: "decision-card/v1".to_string(),
        show_header_divider: true,
        field_order: "as-provided".to_string(),
        field_label_emphasis: "bold".to_string(),
        status_palette: "semantic".to_string(),
        section_spacing: "loose".to_string(),
        update_policy: "stable-shell-live-runtime-append-decision".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerStageBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    pub profile: Option<String>,
    pub stage: String,
    pub title: String,
    pub text: String,
    pub stage_index: Option<u64>,
    pub stage_total: Option<u64>,
    pub step: Option<u64>,
    pub status: Option<String>,
    pub focus: Option<String>,
    pub last_event: Option<String>,
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
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_budget: Option<String>,
    pub available_skill_count: Option<u64>,
    pub available_agent_count: Option<u64>,
    pub available_category_count: Option<u64>,
    pub active_skills: Vec<String>,
    pub active_agents: Vec<String>,
    pub active_categories: Vec<String>,
    #[serde(default)]
    pub done_agent_count: u32,
    #[serde(default)]
    pub total_agent_count: u32,
    pub prompt_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub decision: Option<SchedulerDecisionBlock>,
    pub attached_session_id: Option<String>,
}

impl SchedulerStageBlock {
    pub fn from_metadata(
        text: &str,
        metadata: &HashMap<String, serde_json::Value>,
    ) -> Option<Self> {
        let stage = metadata.get("scheduler_stage")?.as_str()?.to_string();
        let stage_id = metadata
            .get("scheduler_stage_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let profile = metadata
            .get("scheduler_profile")
            .and_then(|v| v.as_str())
            .map(String::from);
        let stage_index = metadata
            .get("scheduler_stage_index")
            .and_then(|v| v.as_u64());
        let stage_total = metadata
            .get("scheduler_stage_total")
            .and_then(|v| v.as_u64());
        let step = metadata
            .get("scheduler_stage_step")
            .and_then(|v| v.as_u64());
        let status = metadata
            .get("scheduler_stage_status")
            .and_then(|v| v.as_str())
            .map(String::from);
        let focus = metadata
            .get("scheduler_stage_focus")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);
        let last_event = metadata
            .get("scheduler_stage_last_event")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);
        let waiting_on = metadata
            .get("scheduler_stage_waiting_on")
            .and_then(|v| v.as_str())
            .map(String::from);
        let estimated_context_tokens = metadata
            .get("scheduler_stage_estimated_context_tokens")
            .and_then(|v| v.as_u64());
        let skill_tree_budget = metadata
            .get("scheduler_stage_skill_tree_budget")
            .and_then(|v| v.as_u64());
        let skill_tree_truncation_strategy = metadata
            .get("scheduler_stage_skill_tree_truncation_strategy")
            .and_then(|v| v.as_str())
            .map(String::from);
        let skill_tree_truncated = metadata
            .get("scheduler_stage_skill_tree_truncated")
            .and_then(|v| v.as_bool());
        let retry_attempt = metadata
            .get("scheduler_stage_retry_attempt")
            .and_then(|v| v.as_u64());
        let activity = metadata
            .get("scheduler_stage_activity")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);
        let loop_budget = metadata
            .get("scheduler_stage_loop_budget")
            .and_then(|v| v.as_str())
            .map(String::from);
        let prompt_tokens = metadata
            .get("scheduler_stage_prompt_tokens")
            .and_then(|v| v.as_u64());
        let context_tokens = metadata
            .get("scheduler_stage_context_tokens")
            .and_then(|v| v.as_u64())
            .or(prompt_tokens);
        let completion_tokens = metadata
            .get("scheduler_stage_completion_tokens")
            .and_then(|v| v.as_u64());
        let reasoning_tokens = metadata
            .get("scheduler_stage_reasoning_tokens")
            .and_then(|v| v.as_u64());
        let cache_read_tokens = metadata
            .get("scheduler_stage_cache_read_tokens")
            .and_then(|v| v.as_u64());
        let cache_miss_tokens = metadata
            .get("scheduler_stage_cache_miss_tokens")
            .and_then(|v| v.as_u64());
        let cache_write_tokens = metadata
            .get("scheduler_stage_cache_write_tokens")
            .and_then(|v| v.as_u64());
        let attached_session_id = metadata
            .get("scheduler_stage_attached_session_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);

        let extract_string_array = |key: &str| -> Vec<String> {
            metadata
                .get(key)
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let active_skills = extract_string_array("scheduler_stage_active_skills");
        let active_agents = extract_string_array("scheduler_stage_active_agents");
        let active_categories = extract_string_array("scheduler_stage_active_categories");

        let available_skill_count = metadata
            .get("scheduler_stage_available_skill_count")
            .and_then(|v| v.as_u64());
        let available_agent_count = metadata
            .get("scheduler_stage_available_agent_count")
            .and_then(|v| v.as_u64());
        let available_category_count = metadata
            .get("scheduler_stage_available_category_count")
            .and_then(|v| v.as_u64());
        let done_agent_count = metadata
            .get("scheduler_stage_done_agent_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let total_agent_count = metadata
            .get("scheduler_stage_total_agent_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let (title, body) = if let Some(rest) = text.trim().strip_prefix("## ") {
            if let Some((heading, after)) = rest.split_once('\n') {
                (heading.trim().to_string(), after.trim_start().to_string())
            } else {
                (rest.trim().to_string(), String::new())
            }
        } else {
            (String::new(), text.to_string())
        };

        Some(Self {
            stage_id,
            profile,
            stage,
            title,
            text: body,
            stage_index,
            stage_total,
            step,
            status,
            focus,
            last_event,
            waiting_on,
            estimated_context_tokens,
            skill_tree_budget,
            skill_tree_truncation_strategy,
            skill_tree_truncated,
            retry_attempt,
            activity,
            loop_budget,
            available_skill_count,
            available_agent_count,
            available_category_count,
            active_skills,
            active_agents,
            active_categories,
            done_agent_count,
            total_agent_count,
            prompt_tokens,
            context_tokens,
            completion_tokens,
            reasoning_tokens,
            cache_read_tokens,
            cache_miss_tokens,
            cache_write_tokens,
            decision: None,
            attached_session_id,
        })
    }

    pub fn to_summary(&self) -> StageSummary {
        StageSummary {
            stage_id: self.stage_id.clone().unwrap_or_default(),
            stage_name: self.stage.clone(),
            index: self.stage_index,
            total: self.stage_total,
            step: self.step,
            step_total: parse_step_limit_from_budget(self.loop_budget.as_deref()),
            status: StageStatus::from_str_lossy(self.status.as_deref()),
            prompt_tokens: self.prompt_tokens,
            context_tokens: self.context_tokens,
            completion_tokens: self.completion_tokens,
            reasoning_tokens: self.reasoning_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_miss_tokens: self.cache_miss_tokens,
            cache_write_tokens: self.cache_write_tokens,
            focus: self.focus.clone(),
            last_event: self.last_event.clone(),
            waiting_on: self.waiting_on.clone(),
            activity: self.activity.clone(),
            estimated_context_tokens: self.estimated_context_tokens,
            skill_tree_budget: self.skill_tree_budget,
            skill_tree_truncation_strategy: self.skill_tree_truncation_strategy.clone(),
            skill_tree_truncated: self.skill_tree_truncated,
            retry_attempt: self.retry_attempt,
            active_agent_count: self.active_agents.len() as u32,
            active_tool_count: 0,
            attached_session_count: if self.attached_session_id.is_some() {
                1
            } else {
                0
            },
            primary_attached_session_id: self.attached_session_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectBlock {
    pub stage_ids: Vec<String>,
    pub events: Vec<InspectEventRow>,
    pub filter_stage_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectEventRow {
    pub ts: i64,
    pub event_type: String,
    pub execution_id: Option<String>,
    pub stage_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputBlock {
    Status(StatusBlock),
    Message(MessageBlock),
    Reasoning(ReasoningBlock),
    Tool(ToolBlock),
    SessionEvent(SessionEventBlock),
    QueueItem(QueueItemBlock),
    SchedulerStage(Box<SchedulerStageBlock>),
    Inspect(InspectBlock),
}
