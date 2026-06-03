use std::collections::{HashMap, HashSet};

use super::*;
use agendao_config::{
    AgentConfig as LoadedAgentConfig, AgentConfigs as LoadedAgentConfigs,
    AgentMode as LoadedAgentMode, Config as LoadedConfig,
    PermissionAction as LoadedPermissionAction, PermissionConfig as LoadedPermissionConfig,
    PermissionRule as LoadedPermissionRule,
};
use agendao_permission::{
    build_agent_ruleset, tool_to_permission, PermissionAction, PermissionRule, PermissionRuleset,
};

impl AgentRegistry {
    pub(crate) fn apply_config(&mut self, config: &LoadedConfig) {
        if let Some(mode_configs) = &config.mode {
            self.apply_agent_configs(mode_configs, Some(AgentMode::Primary));
        }
        if let Some(agent_configs) = &config.agent {
            self.apply_agent_configs(agent_configs, None);
        }
    }

    fn apply_agent_configs(
        &mut self,
        configs: &LoadedAgentConfigs,
        forced_mode: Option<AgentMode>,
    ) {
        for (key, cfg) in &configs.entries {
            self.apply_agent_config(key, cfg, forced_mode);
        }
    }

    fn apply_agent_config(
        &mut self,
        key: &str,
        cfg: &LoadedAgentConfig,
        forced_mode: Option<AgentMode>,
    ) {
        if cfg.disable.unwrap_or(false) {
            self.agents.remove(key);
            return;
        }

        let mut agent = self
            .agents
            .get(key)
            .cloned()
            .unwrap_or_else(|| AgentInfo::custom(key.to_string()));

        if let Some(name) = &cfg.name {
            agent.name = name.clone();
        }
        if let Some(description) = &cfg.description {
            agent.description = Some(description.clone());
        }
        if let Some(prompt) = &cfg.prompt {
            agent.system_prompt = Some(prompt.clone());
        }
        if let Some(variant) = &cfg.variant {
            agent.variant = Some(variant.clone());
        }
        if let Some(temperature) = cfg.temperature {
            agent.temperature = Some(temperature);
        }
        if let Some(top_p) = cfg.top_p {
            agent.top_p = Some(top_p);
        }
        if let Some(color) = &cfg.color {
            agent.color = Some(color.clone());
        }
        if let Some(hidden) = cfg.hidden {
            agent.hidden = hidden;
        }
        if let Some(mode) = forced_mode {
            agent.mode = mode;
        } else if let Some(mode) = cfg.mode.clone() {
            agent.mode = map_loaded_agent_mode(mode);
        }
        if let Some(steps) = cfg.steps.or(cfg.max_steps) {
            agent.max_steps = Some(steps);
        }
        if let Some(max_tokens) = cfg.max_tokens {
            agent.max_tokens = Some(max_tokens);
        }
        if let Some(model) = cfg.model.as_deref().and_then(parse_model_ref) {
            agent.model_preference = Some(model.clone());
            agent.model = Some(model);
        }
        if let Some(options) = &cfg.options {
            for (key, value) in options {
                if let Some(existing) = agent.options.get_mut(key) {
                    merge_json_value(existing, value.clone());
                } else {
                    agent.options.insert(key.clone(), value.clone());
                }
            }
        }
        if let Some(tool_overrides) = &cfg.tools {
            if !agent.allowed_tools.is_empty() {
                let mut merged: HashSet<String> = agent.allowed_tools.into_iter().collect();
                for (tool, enabled) in tool_overrides {
                    if *enabled {
                        merged.insert(tool.clone());
                    } else {
                        merged.remove(tool);
                    }
                }
                let mut out: Vec<String> = merged.into_iter().collect();
                out.sort();
                agent.allowed_tools = out;
            }
        }

        if cfg.permission.is_some() || cfg.tools.is_some() {
            let mut user_rules: PermissionRuleset = Vec::new();
            if let Some(permission_cfg) = &cfg.permission {
                user_rules.extend(permission_rules_from_config(permission_cfg));
            }
            if let Some(tool_overrides) = &cfg.tools {
                user_rules.extend(permission_rules_from_tools(tool_overrides));
            }
            agent.permission = build_agent_ruleset(key, &user_rules);
        }

        self.agents.insert(key.to_string(), agent);
    }
}

fn map_loaded_permission_action(action: &LoadedPermissionAction) -> PermissionAction {
    match action {
        LoadedPermissionAction::Ask => PermissionAction::Ask,
        LoadedPermissionAction::Allow => PermissionAction::Allow,
        LoadedPermissionAction::Deny => PermissionAction::Deny,
    }
}

fn permission_rules_from_config(permission: &LoadedPermissionConfig) -> PermissionRuleset {
    let mut rules = Vec::new();
    for (permission_name, rule) in &permission.rules {
        match rule {
            LoadedPermissionRule::Action(action) => rules.push(PermissionRule {
                permission: permission_name.clone(),
                pattern: "*".to_string(),
                action: map_loaded_permission_action(action),
            }),
            LoadedPermissionRule::Object(patterns) => {
                for (pattern, action) in patterns {
                    rules.push(PermissionRule {
                        permission: permission_name.clone(),
                        pattern: pattern.clone(),
                        action: map_loaded_permission_action(action),
                    });
                }
            }
        }
    }
    rules
}

fn permission_rules_from_tools(tool_overrides: &HashMap<String, bool>) -> PermissionRuleset {
    tool_overrides
        .iter()
        .map(|(tool, enabled)| PermissionRule {
            permission: tool_to_permission(tool).to_string(),
            pattern: "*".to_string(),
            action: if *enabled {
                PermissionAction::Allow
            } else {
                PermissionAction::Deny
            },
        })
        .collect()
}

fn map_loaded_agent_mode(mode: LoadedAgentMode) -> AgentMode {
    match mode {
        LoadedAgentMode::Primary => AgentMode::Primary,
        LoadedAgentMode::Subagent => AgentMode::Subagent,
        LoadedAgentMode::All => AgentMode::All,
    }
}

fn parse_model_ref(raw: &str) -> Option<ModelRef> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (provider, model) = raw.split_once(':').or_else(|| raw.split_once('/'))?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(ModelRef {
        provider_id: provider.to_string(),
        model_id: model.to_string(),
    })
}

fn merge_json_value(target: &mut serde_json::Value, source: serde_json::Value) {
    match (target, source) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(source_map)) => {
            for (key, source_value) in source_map {
                if let Some(target_value) = target_map.get_mut(&key) {
                    merge_json_value(target_value, source_value);
                } else {
                    target_map.insert(key, source_value);
                }
            }
        }
        (target, source) => *target = source,
    }
}
