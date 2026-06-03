use std::collections::HashMap;
use std::path::Path;

use super::*;
use agendao_config::{Config as LoadedConfig, ConfigStore};

pub struct AgentRegistry {
    pub(crate) agents: HashMap<String, AgentInfo>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        let mut agents = HashMap::new();
        for builtin in BuiltinAgent::all() {
            let agent = AgentInfo::from_builtin(builtin);
            agents.insert(builtin.as_str().to_string(), agent);
        }
        agents.insert("summary".to_string(), AgentInfo::summary());
        Self { agents }
    }

    pub fn from_config(config: &LoadedConfig) -> Self {
        let mut registry = Self::new();
        registry.apply_config(config);
        registry
    }

    pub fn from_optional_config(config: Option<&LoadedConfig>) -> Self {
        if let Some(config) = config {
            return Self::from_config(config);
        }
        Self::new()
    }

    pub fn from_project_dir(project_dir: impl AsRef<Path>) -> Self {
        ConfigStore::from_project_dir(project_dir.as_ref())
            .ok()
            .map(|store| Self::from_config(&store.config()))
            .unwrap_or_else(Self::new)
    }

    pub fn get(&self, name: &str) -> Option<&AgentInfo> {
        self.agents.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut AgentInfo> {
        self.agents.get_mut(name)
    }

    pub fn register(&mut self, agent: AgentInfo) {
        self.agents.insert(agent.name.clone(), agent);
    }

    pub fn list(&self) -> Vec<&AgentInfo> {
        let mut agents: Vec<&AgentInfo> = self.agents.values().filter(|a| !a.hidden).collect();
        agents.sort_by(|a, b| {
            let a_is_build = a.name == "build";
            let b_is_build = b.name == "build";
            if a_is_build {
                return std::cmp::Ordering::Less;
            }
            if b_is_build {
                return std::cmp::Ordering::Greater;
            }
            a.name.cmp(&b.name)
        });
        agents
    }

    pub fn list_all(&self) -> Vec<&AgentInfo> {
        self.agents.values().collect()
    }

    pub fn list_primary(&self) -> Vec<&AgentInfo> {
        let mut agents: Vec<&AgentInfo> = self
            .agents
            .values()
            .filter(|a| matches!(a.mode, AgentMode::Primary) && !a.hidden)
            .collect();
        agents.sort_by(|a, b| {
            let a_is_build = a.name == "build";
            let b_is_build = b.name == "build";
            if a_is_build {
                return std::cmp::Ordering::Less;
            }
            if b_is_build {
                return std::cmp::Ordering::Greater;
            }
            a.name.cmp(&b.name)
        });
        agents
    }

    pub fn list_subagents(&self) -> Vec<&AgentInfo> {
        let mut agents: Vec<&AgentInfo> = self
            .agents
            .values()
            .filter(|a| matches!(a.mode, AgentMode::Subagent) && !a.hidden)
            .collect();
        agents.sort_by(|a, b| a.name.cmp(&b.name));
        agents
    }

    pub fn default_agent(&self) -> &AgentInfo {
        if let Some(general) = self.get(BuiltinAgent::General.as_str()) {
            return general;
        }

        if let Some(primary) = self
            .agents
            .values()
            .find(|a| !a.hidden && !matches!(a.mode, AgentMode::Subagent))
        {
            return primary;
        }

        self.agents
            .values()
            .next()
            .expect("Agent registry is empty; expected at least one agent")
    }

    pub async fn generate(
        &self,
        input: GenerateInput,
        provider_registry: &agendao_provider::ProviderRegistry,
    ) -> Result<GeneratedAgentConfig, AgentError> {
        let model_ref = input.model.clone().ok_or(AgentError::NoDefaultModel)?;

        let provider = provider_registry
            .get(&model_ref.provider_id)
            .ok_or(AgentError::NoDefaultModel)?;

        let existing_names: Vec<&str> = self.agents.keys().map(|s| s.as_str()).collect();
        let existing_list = existing_names.join(", ");

        let user_content = format!(
            "Create an agent configuration based on this request: \"{}\".\n\n\
             IMPORTANT: The following identifiers already exist and must NOT be used: {}\n\
             Return ONLY the JSON object, no other text, do not wrap in backticks",
            input.description, existing_list
        );

        let messages = vec![
            agendao_provider::Message::system(PROMPT_GENERATE),
            agendao_provider::Message::user(&user_content),
        ];

        let request = agendao_orchestrator::agent_generation_request(model_ref.model_id.clone())
            .to_chat_request(messages, vec![], false);

        let response = provider.chat(request).await?;

        let content = response
            .choices
            .first()
            .and_then(|c| match &c.message.content {
                agendao_provider::Content::Text(text) => Some(text.clone()),
                agendao_provider::Content::Parts(parts) => {
                    parts.first().and_then(|p| p.text.clone())
                }
            })
            .unwrap_or_default();

        let cleaned = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        serde_json::from_str(cleaned)
            .map_err(|e| AgentError::ParseError(format!("{}: {}", e, cleaned)))
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
