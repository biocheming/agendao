use crate::blueprint::{AgentId, SkillId, ToolId};
use crate::context::PromptSurface;
use crate::context::Usage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    pub tool: ToolId,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolExecution {
    pub output: String,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ConversationItem {
    Assistant {
        turn: AssistantTurn,
    },
    ToolResult {
        call_id: String,
        output: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTurn {
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: Usage,
    pub finish_reason: Option<String>,
    pub reasoning_continuation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub agent: AgentId,
    pub skills: BTreeSet<SkillId>,
    pub tools: BTreeSet<ToolId>,
    pub prompt: PromptSurface,
    pub reasoning_continuation: Option<String>,
    pub conversation_seed: Arc<[agendao_provider::Message]>,
}
