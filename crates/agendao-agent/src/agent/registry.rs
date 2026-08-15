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
            .unwrap_or_default()
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
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
