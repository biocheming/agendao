use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use agendao_permission::PermissionRuleset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinAgent {
    Build,
    Plan,
    General,
    Explore,
    DeepWorker,
    ArchitectureAdvisor,
    DocsResearcher,
    MediaReader,
    Compaction,
    Title,
}

impl BuiltinAgent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::General => "general",
            Self::Explore => "explore",
            Self::DeepWorker => "deep-worker",
            Self::ArchitectureAdvisor => "architecture-advisor",
            Self::DocsResearcher => "docs-researcher",
            Self::MediaReader => "media-reader",
            Self::Compaction => "compaction",
            Self::Title => "title",
        }
    }

    pub const fn all() -> [BuiltinAgent; 10] {
        [
            BuiltinAgent::Build,
            BuiltinAgent::Plan,
            BuiltinAgent::General,
            BuiltinAgent::Explore,
            BuiltinAgent::DeepWorker,
            BuiltinAgent::ArchitectureAdvisor,
            BuiltinAgent::DocsResearcher,
            BuiltinAgent::MediaReader,
            BuiltinAgent::Compaction,
            BuiltinAgent::Title,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub description: Option<String>,
    pub mode: AgentMode,
    pub model: Option<ModelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_preference: Option<ModelRef>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u64>,
    pub max_steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    pub options: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub permission: PermissionRuleset,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    #[default]
    Primary,
    Subagent,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    pub model_id: String,
    pub provider_id: String,
}
