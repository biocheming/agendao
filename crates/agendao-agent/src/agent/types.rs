use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use agendao_permission::PermissionRuleset;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedAgentConfig {
    pub identifier: String,
    pub when_to_use: String,
    pub system_prompt: String,
}

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
    CodeExplorer,
    MediaReader,
    Metis,
    Momus,
    Oracle,
    SisyphusJunior,
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
            Self::CodeExplorer => "code-explorer",
            Self::MediaReader => "media-reader",
            Self::Metis => "metis",
            Self::Momus => "momus",
            Self::Oracle => "oracle",
            Self::SisyphusJunior => "sisyphus-junior",
            Self::Compaction => "compaction",
            Self::Title => "title",
        }
    }

    pub const fn all() -> [BuiltinAgent; 15] {
        [
            BuiltinAgent::Build,
            BuiltinAgent::Plan,
            BuiltinAgent::General,
            BuiltinAgent::Explore,
            BuiltinAgent::DeepWorker,
            BuiltinAgent::ArchitectureAdvisor,
            BuiltinAgent::DocsResearcher,
            BuiltinAgent::CodeExplorer,
            BuiltinAgent::MediaReader,
            BuiltinAgent::Metis,
            BuiltinAgent::Momus,
            BuiltinAgent::Oracle,
            BuiltinAgent::SisyphusJunior,
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
    #[serde(default, alias = "permission_ruleset")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    pub content: String,
    pub tool_calls: Vec<ToolCallResult>,
    pub usage: Option<UsageInfo>,
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateInput {
    pub description: String,
    pub model: Option<ModelRef>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Provider error: {0}")]
    ProviderError(#[from] agendao_provider::ProviderError),

    #[error("Failed to parse generated config: {0}")]
    ParseError(String),

    #[error("No default model available")]
    NoDefaultModel,
}
