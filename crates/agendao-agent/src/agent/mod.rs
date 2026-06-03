mod builtin;
mod config_apply;
mod registry;
mod types;

pub use registry::*;
pub use types::*;

#[cfg(test)]
use agendao_config::{
    AgentConfig as LoadedAgentConfig, AgentConfigs as LoadedAgentConfigs,
    AgentMode as LoadedAgentMode, Config as LoadedConfig,
};
#[cfg(test)]
use agendao_permission::PermissionAction;
#[cfg(test)]
use std::collections::HashMap;

const PROMPT_GENERATE: &str = r#"You are an AI agent configuration generator. Given a description of what an agent should do, generate a JSON configuration with:
- identifier: A unique, lowercase, single-word identifier for the agent (use underscores if needed)
- whenToUse: A brief description of when this agent should be used
- systemPrompt: The system prompt that will be given to this agent

The identifier should be descriptive but concise. The system prompt should be detailed enough to guide the agent's behavior."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_agents_have_expected_defaults() {
        let registry = AgentRegistry::new();
        for builtin in BuiltinAgent::all() {
            let agent = registry
                .get(builtin.as_str())
                .unwrap_or_else(|| panic!("missing builtin agent '{}'", builtin.as_str()));
            assert!(agent.native, "builtin agent should be native");
            assert_eq!(agent.name, builtin.as_str());
        }

        assert!(matches!(
            registry.get("build").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("plan").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("general").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("explore").map(|a| a.mode),
            Some(AgentMode::Subagent)
        ));
        assert!(matches!(
            registry.get("deep-worker").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("architecture-advisor").map(|a| a.mode),
            Some(AgentMode::Subagent)
        ));
        assert!(matches!(
            registry.get("docs-researcher").map(|a| a.mode),
            Some(AgentMode::Subagent)
        ));
        assert!(matches!(
            registry.get("code-explorer").map(|a| a.mode),
            Some(AgentMode::Subagent)
        ));
        assert!(matches!(
            registry.get("media-reader").map(|a| a.mode),
            Some(AgentMode::Subagent)
        ));
        assert_eq!(registry.default_agent().name, "general");
    }

    #[test]
    fn explore_agent_permission_is_restricted_to_read_search_and_bash() {
        let explore = AgentInfo::explore();
        assert_eq!(
            explore.tool_permission_decision("grep"),
            PermissionAction::Allow
        );
        assert_eq!(
            explore.tool_permission_decision("glob"),
            PermissionAction::Allow
        );
        assert_eq!(
            explore.tool_permission_decision("read"),
            PermissionAction::Allow
        );
        assert_eq!(
            explore.tool_permission_decision("bash"),
            PermissionAction::Allow
        );
        assert_eq!(
            explore.tool_permission_decision("ast_grep_search"),
            PermissionAction::Allow
        );

        assert_eq!(
            explore.tool_permission_decision("write"),
            PermissionAction::Deny
        );
        assert_eq!(
            explore.tool_permission_decision("ls"),
            PermissionAction::Deny
        );
        assert_eq!(
            explore.tool_permission_decision("websearch"),
            PermissionAction::Deny
        );
        assert_eq!(
            explore.tool_permission_decision("browser_session"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn new_builtin_specialists_have_expected_tool_boundaries() {
        let deep = AgentInfo::deep_worker();
        let architecture = AgentInfo::architecture_advisor();
        let docs = AgentInfo::docs_researcher();
        let code = AgentInfo::code_explorer();
        let media = AgentInfo::media_reader();

        assert_eq!(
            deep.tool_permission_decision("write"),
            PermissionAction::Allow
        );
        assert_eq!(
            deep.tool_permission_decision("ast_grep_search"),
            PermissionAction::Allow
        );
        assert_eq!(
            deep.tool_permission_decision("shell_session"),
            PermissionAction::Allow
        );
        assert_eq!(
            deep.tool_permission_decision("nonexistent_tool"),
            PermissionAction::Deny
        );

        assert_eq!(
            architecture.tool_permission_decision("read"),
            PermissionAction::Allow
        );
        assert_eq!(
            architecture.tool_permission_decision("ast_grep_search"),
            PermissionAction::Allow
        );
        assert_eq!(
            architecture.tool_permission_decision("write"),
            PermissionAction::Deny
        );

        assert_eq!(
            docs.tool_permission_decision("websearch"),
            PermissionAction::Allow
        );
        assert_eq!(
            docs.tool_permission_decision("webfetch"),
            PermissionAction::Allow
        );
        assert_eq!(
            docs.tool_permission_decision("codesearch"),
            PermissionAction::Allow
        );
        assert_eq!(
            docs.tool_permission_decision("context_docs"),
            PermissionAction::Allow
        );
        assert_eq!(
            docs.tool_permission_decision("github_research"),
            PermissionAction::Allow
        );
        assert_eq!(
            docs.tool_permission_decision("browser_session"),
            PermissionAction::Allow
        );
        assert_eq!(
            docs.tool_permission_decision("ast_grep_search"),
            PermissionAction::Deny
        );
        assert_eq!(
            docs.tool_permission_decision("write"),
            PermissionAction::Deny
        );

        assert_eq!(
            code.tool_permission_decision("grep"),
            PermissionAction::Allow
        );
        assert_eq!(
            code.tool_permission_decision("ast_grep_search"),
            PermissionAction::Allow
        );
        assert_eq!(
            code.tool_permission_decision("write"),
            PermissionAction::Deny
        );

        assert_eq!(
            media.tool_permission_decision("read"),
            PermissionAction::Allow
        );
        assert_eq!(
            media.tool_permission_decision("grep"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn specialized_builtin_agent_prompts_match_agendao_runtime() {
        let deep = AgentInfo::deep_worker();
        let architecture = AgentInfo::architecture_advisor();
        let docs = AgentInfo::docs_researcher();
        let code = AgentInfo::code_explorer();
        let media = AgentInfo::media_reader();

        let deep_prompt = deep.system_prompt.as_deref().expect("deep worker prompt");
        assert!(deep_prompt.contains("task_flow"));
        assert!(deep_prompt.contains("shell_session"));
        assert!(deep_prompt.contains("ast_grep_replace"));
        assert!(deep_prompt.contains("task_create"));
        assert!(deep_prompt.contains("task_update"));
        assert!(deep_prompt.contains("lsp_diagnostics"));
        assert!(deep_prompt.contains("Use AgenDao's native tool vocabulary only"));

        let architecture_prompt = architecture
            .system_prompt
            .as_deref()
            .expect("architecture advisor prompt");
        assert!(architecture_prompt.contains("read-only"));
        assert!(architecture_prompt.contains("ast_grep_search"));
        assert!(!architecture_prompt.contains("write"));

        let docs_prompt = docs
            .system_prompt
            .as_deref()
            .expect("docs researcher prompt");
        assert!(docs_prompt.contains("context_docs"));
        assert!(docs_prompt.contains("github_research"));
        assert!(docs_prompt.contains("browser_session"));
        assert!(docs_prompt.contains("Phase 1"));
        assert!(docs_prompt.contains("context7_*"));
        assert!(docs_prompt.contains("grep_app_searchGitHub"));
        assert!(docs_prompt.contains("Do not claim access"));

        let code_prompt = code.system_prompt.as_deref().expect("code explorer prompt");
        assert!(code_prompt.contains("glob"));
        assert!(code_prompt.contains("grep"));
        assert!(code_prompt.contains("ast_grep_search"));
        assert!(code_prompt.contains("read-only"));

        let media_prompt = media.system_prompt.as_deref().expect("media reader prompt");
        assert!(media_prompt.contains("attachment"));
        assert!(media_prompt.contains("read"));
        assert!(media_prompt.contains("preflight"));
    }

    #[test]
    fn general_agent_has_explicit_builtin_tool_allowlist() {
        let general = AgentInfo::general();

        assert_eq!(
            general.tool_permission_decision("ast_grep_search"),
            PermissionAction::Allow
        );
        assert_eq!(
            general.tool_permission_decision("write"),
            PermissionAction::Allow
        );
        assert_eq!(
            general.tool_permission_decision("ast_grep_replace"),
            PermissionAction::Allow
        );
        assert_eq!(
            general.tool_permission_decision("task_flow"),
            PermissionAction::Allow
        );
        assert_eq!(
            general.tool_permission_decision("shell_session"),
            PermissionAction::Allow
        );
        assert_eq!(
            general.tool_permission_decision("nonexistent_tool"),
            PermissionAction::Deny
        );
        assert!(general.allowed_tools.iter().any(|t| t == "ast_grep_search"));
        assert!(general
            .allowed_tools
            .iter()
            .any(|t| t == "ast_grep_replace"));
        assert!(general.allowed_tools.iter().any(|t| t == "task_flow"));
        assert!(general.allowed_tools.iter().any(|t| t == "shell_session"));
    }

    #[test]
    fn build_agent_has_explicit_builtin_tool_allowlist() {
        let build = AgentInfo::build();

        assert_eq!(
            build.tool_permission_decision("ast_grep_search"),
            PermissionAction::Allow
        );
        assert_eq!(
            build.tool_permission_decision("write"),
            PermissionAction::Allow
        );
        assert_eq!(
            build.tool_permission_decision("ast_grep_replace"),
            PermissionAction::Allow
        );
        assert_eq!(
            build.tool_permission_decision("plan_enter"),
            PermissionAction::Allow
        );
        assert_eq!(
            build.tool_permission_decision("task_flow"),
            PermissionAction::Allow
        );
        assert_eq!(
            build.tool_permission_decision("shell_session"),
            PermissionAction::Allow
        );
        assert_eq!(
            build.tool_permission_decision("nonexistent_tool"),
            PermissionAction::Deny
        );
        assert!(build.allowed_tools.iter().any(|t| t == "ast_grep_search"));
        assert!(build.allowed_tools.iter().any(|t| t == "ast_grep_replace"));
        assert!(build.allowed_tools.iter().any(|t| t == "task_flow"));
        assert!(build.allowed_tools.iter().any(|t| t == "shell_session"));
    }

    #[test]
    fn config_can_override_builtin_agent_model() {
        let config = LoadedConfig {
            agent: Some(LoadedAgentConfigs {
                entries: HashMap::from([(
                    "general".to_string(),
                    LoadedAgentConfig {
                        model: Some("openai/gpt-4.1".to_string()),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let registry = AgentRegistry::from_config(&config);
        let general = registry.get("general").expect("general should exist");
        assert_eq!(
            general.model.as_ref().map(|m| m.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(
            general.model.as_ref().map(|m| m.model_id.as_str()),
            Some("gpt-4.1")
        );
        assert_eq!(
            general
                .model_preference
                .as_ref()
                .map(|m| m.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(
            general
                .model_preference
                .as_ref()
                .map(|m| m.model_id.as_str()),
            Some("gpt-4.1")
        );
    }

    #[test]
    fn registry_supports_dynamic_custom_agents_from_config() {
        let config = LoadedConfig {
            agent: Some(LoadedAgentConfigs {
                entries: HashMap::from([(
                    "reviewer".to_string(),
                    LoadedAgentConfig {
                        description: Some("Custom reviewer agent".to_string()),
                        mode: Some(LoadedAgentMode::Subagent),
                        model: Some("openai/gpt-4.1".to_string()),
                        prompt: Some("Review code carefully".to_string()),
                        steps: Some(12),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let registry = AgentRegistry::from_config(&config);
        let reviewer = registry.get("reviewer").expect("reviewer should exist");
        assert_eq!(
            reviewer.description.as_deref(),
            Some("Custom reviewer agent")
        );
        assert!(matches!(reviewer.mode, AgentMode::Subagent));
        assert_eq!(
            reviewer.model.as_ref().map(|m| m.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(
            reviewer.model.as_ref().map(|m| m.model_id.as_str()),
            Some("gpt-4.1")
        );
        assert_eq!(
            reviewer.system_prompt.as_deref(),
            Some("Review code carefully")
        );
        assert_eq!(reviewer.max_steps, Some(12));
        assert!(!reviewer.native);
    }

    #[test]
    fn registry_can_disable_builtin_agent_from_config() {
        let config = LoadedConfig {
            agent: Some(LoadedAgentConfigs {
                entries: HashMap::from([(
                    "build".to_string(),
                    LoadedAgentConfig {
                        disable: Some(true),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let registry = AgentRegistry::from_config(&config);
        assert!(registry.get("build").is_none());
    }

    #[test]
    fn deprecated_mode_config_forces_primary_mode() {
        let config = LoadedConfig {
            mode: Some(LoadedAgentConfigs {
                entries: HashMap::from([(
                    "investigate".to_string(),
                    LoadedAgentConfig {
                        mode: Some(LoadedAgentMode::Subagent),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let registry = AgentRegistry::from_config(&config);
        let agent = registry
            .get("investigate")
            .expect("investigate should be created from deprecated mode config");
        assert!(matches!(agent.mode, AgentMode::Primary));
    }
}
