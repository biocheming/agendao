mod architecture_advisor;
mod code_explorer;
mod deep_worker;
mod docs_researcher;
mod media_reader;
mod metis;
mod momus;
mod oracle;
mod sisyphus_junior;

use super::*;
use std::collections::HashMap;

use agendao_permission::{
    build_agent_ruleset, PermissionAction, PermissionEngine, PermissionRule, PermissionRuleset,
};

pub use architecture_advisor::architecture_advisor;
pub use code_explorer::code_explorer;
pub use deep_worker::deep_worker;
pub use docs_researcher::docs_researcher;
pub use media_reader::media_reader;
pub use metis::metis;
pub use momus::momus;
pub use oracle::oracle;
pub use sisyphus_junior::sisyphus_junior;

fn base_agent(name: &str, mode: AgentMode) -> AgentInfo {
    AgentInfo {
        name: name.to_string(),
        description: None,
        mode,
        model: None,
        model_preference: None,
        system_prompt: None,
        temperature: None,
        top_p: None,
        max_tokens: None,
        max_steps: Some(100),
        allowed_tools: Vec::new(),
        options: HashMap::new(),
        permission: build_agent_ruleset(name, &[]),
        hidden: false,
        native: true,
        variant: None,
        color: None,
    }
}

fn base_allowlist_agent(name: &str, mode: AgentMode, allowed_tools: &[&str]) -> AgentInfo {
    let mut agent = base_agent(name, mode);
    agent.allowed_tools = allowed_tools
        .iter()
        .map(|tool| (*tool).to_string())
        .collect();
    agent
}

const READ_SEARCH_TOOLS: &[&str] = &["read", "glob", "grep", "ast_grep_search", "bash"];

const PRIMARY_BUILTIN_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "bash",
    "shell_session",
    "glob",
    "grep",
    "ls",
    "task",
    "task_flow",
    "question",
    "webfetch",
    "websearch",
    "todoread",
    "todowrite",
    "multiedit",
    "apply_patch",
    "skill",
    "lsp",
    "batch",
    "codesearch",
    "ast_grep_search",
    "ast_grep_replace",
    "plan_enter",
    "plan_exit",
];

fn base_read_only_agent(name: &str, mode: AgentMode) -> AgentInfo {
    base_allowlist_agent(name, mode, READ_SEARCH_TOOLS)
}

impl AgentInfo {
    pub fn from_builtin(builtin: BuiltinAgent) -> Self {
        match builtin {
            BuiltinAgent::Build => Self::build(),
            BuiltinAgent::Plan => Self::plan(),
            BuiltinAgent::General => Self::general(),
            BuiltinAgent::Explore => Self::explore(),
            BuiltinAgent::DeepWorker => Self::deep_worker(),
            BuiltinAgent::ArchitectureAdvisor => Self::architecture_advisor(),
            BuiltinAgent::DocsResearcher => Self::docs_researcher(),
            BuiltinAgent::CodeExplorer => Self::code_explorer(),
            BuiltinAgent::MediaReader => Self::media_reader(),
            BuiltinAgent::Metis => Self::metis(),
            BuiltinAgent::Momus => Self::momus(),
            BuiltinAgent::Oracle => Self::oracle(),
            BuiltinAgent::SisyphusJunior => Self::sisyphus_junior(),
            BuiltinAgent::Compaction => Self::compaction(),
            BuiltinAgent::Title => Self::title(),
        }
    }

    pub fn default_agent() -> Self {
        Self::general()
    }

    pub fn build() -> Self {
        Self {
            name: "build".to_string(),
            description: Some(
                "The default agent. Executes tools based on configured permissions.".to_string(),
            ),
            mode: AgentMode::Primary,
            model: None,
            model_preference: None,
            system_prompt: None,
            temperature: None,
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(100),
            allowed_tools: PRIMARY_BUILTIN_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            options: HashMap::new(),
            permission: build_agent_ruleset("build", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn plan() -> Self {
        Self {
            name: "plan".to_string(),
            description: Some("Plan mode. Disallows all edit tools.".to_string()),
            mode: AgentMode::Primary,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a planning assistant. Analyze the task and create a detailed plan before execution.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(50),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: build_agent_ruleset("plan", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn general() -> Self {
        Self {
            name: "general".to_string(),
            description: Some("Default general-purpose agent.".to_string()),
            mode: AgentMode::Primary,
            model: None,
            model_preference: None,
            system_prompt: Some(
                "You are a helpful assistant. Complete the task given to you.".to_string(),
            ),
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(20),
            allowed_tools: PRIMARY_BUILTIN_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            options: HashMap::new(),
            permission: build_agent_ruleset("general", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn explore() -> Self {
        Self {
            name: "explore".to_string(),
            description: Some("Exploration subagent for searching and reading code.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are an exploration assistant. Search and read code to answer questions. Focus on read-only operations.".to_string()),
            temperature: Some(0.5),
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(30),
            allowed_tools: READ_SEARCH_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            options: HashMap::new(),
            permission: build_agent_ruleset("explore", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn deep_worker() -> Self {
        deep_worker()
    }

    pub fn architecture_advisor() -> Self {
        architecture_advisor()
    }

    pub fn docs_researcher() -> Self {
        docs_researcher()
    }

    pub fn code_explorer() -> Self {
        code_explorer()
    }

    pub fn media_reader() -> Self {
        media_reader()
    }

    pub fn metis() -> Self {
        metis()
    }

    pub fn momus() -> Self {
        momus()
    }

    pub fn oracle() -> Self {
        oracle()
    }

    pub fn sisyphus_junior() -> Self {
        sisyphus_junior()
    }

    pub fn title() -> Self {
        Self {
            name: "title".to_string(),
            description: Some("Generates concise session titles.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a title generator. Generate a concise 3-5 word title that summarizes the conversation. Return only the title, nothing else.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(1024),
            max_steps: Some(1),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            }],
            hidden: true,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn summary() -> Self {
        Self {
            name: "summary".to_string(),
            description: Some("Generates conversation summaries.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a summary generator. Create a concise summary of the conversation. Focus on key decisions and outcomes.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(1024),
            max_steps: Some(1),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            }],
            hidden: true,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn compaction() -> Self {
        Self {
            name: "compaction".to_string(),
            description: Some("Compacts conversation history while preserving context.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a context compaction assistant. Summarize the conversation while preserving all important context for future interactions.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(1024),
            max_steps: Some(1),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            }],
            hidden: true,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn custom(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            description: None,
            mode: AgentMode::All,
            model: None,
            model_preference: None,
            system_prompt: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_steps: Some(100),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: build_agent_ruleset(&name, &[]),
            hidden: false,
            native: false,
            variant: None,
            color: None,
        }
    }

    pub fn with_model(
        mut self,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Self {
        let model = ModelRef {
            model_id: model_id.into(),
            provider_id: provider_id.into(),
        };
        self.model_preference = Some(model.clone());
        self.model = Some(model);
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }

    pub fn with_max_steps(mut self, steps: u32) -> Self {
        self.max_steps = Some(steps);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_permission(mut self, permission: PermissionRuleset) -> Self {
        self.permission = permission;
        self
    }

    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn tool_permission_decision(&self, tool_name: &str) -> PermissionAction {
        PermissionEngine::evaluate_tool(
            tool_name,
            &self.allowed_tools,
            std::slice::from_ref(&self.permission),
        )
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        matches!(
            self.tool_permission_decision(tool_name),
            PermissionAction::Allow
        )
    }

    pub fn ensure_tool_allowed(&self, tool_name: &str) -> Result<(), String> {
        match self.tool_permission_decision(tool_name) {
            PermissionAction::Allow => Ok(()),
            PermissionAction::Ask => Err(format!(
                "Tool '{}' requires explicit approval for agent '{}'",
                tool_name, self.name
            )),
            PermissionAction::Deny => Err(format!(
                "Tool '{}' is denied by agent '{}' permission rules",
                tool_name, self.name
            )),
        }
    }

    pub fn resolved_system_prompt(&self) -> Option<String> {
        self.system_prompt
            .clone()
            .filter(|prompt| !prompt.trim().is_empty())
    }
}
