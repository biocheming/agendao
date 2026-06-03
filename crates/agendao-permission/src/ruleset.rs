use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::matching::wildcard_match;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PermissionAction {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "deny")]
    Deny,
    #[serde(rename = "ask")]
    #[default]
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub permission: String,
    pub pattern: String,
    pub action: PermissionAction,
}

pub type PermissionRuleset = Vec<PermissionRule>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Action(PermissionAction),
    Patterns(HashMap<String, PermissionAction>),
}

pub type ConfigPermission = HashMap<String, ConfigValue>;

fn expand(pattern: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if pattern.starts_with("~/") {
            return format!("{}{}", home.display(), &pattern[1..]);
        }
        if pattern == "~" {
            return home.display().to_string();
        }
        if pattern.starts_with("$HOME/") {
            return format!("{}{}", home.display(), &pattern[5..]);
        }
    }
    pattern.to_string()
}

pub fn from_config(permission: &ConfigPermission) -> PermissionRuleset {
    let mut ruleset: PermissionRuleset = Vec::new();

    for (key, value) in permission.iter() {
        match value {
            ConfigValue::Action(action) => {
                ruleset.push(PermissionRule {
                    permission: key.clone(),
                    action: *action,
                    pattern: "*".to_string(),
                });
            }
            ConfigValue::Patterns(patterns) => {
                for (pattern, action) in patterns.iter() {
                    ruleset.push(PermissionRule {
                        permission: key.clone(),
                        pattern: expand(pattern),
                        action: *action,
                    });
                }
            }
        }
    }

    ruleset
}

pub fn merge(rulesets: &[PermissionRuleset]) -> PermissionRuleset {
    rulesets.iter().flat_map(|r| r.clone()).collect()
}

pub fn evaluate(permission: &str, pattern: &str, rulesets: &[PermissionRuleset]) -> PermissionRule {
    let merged = merge(rulesets);

    let matched = merged.iter().rev().find(|rule| {
        wildcard_match(permission, &rule.permission) && wildcard_match(pattern, &rule.pattern)
    });

    matched.cloned().unwrap_or(PermissionRule {
        action: PermissionAction::Ask,
        permission: permission.to_string(),
        pattern: "*".to_string(),
    })
}

pub fn combine_actions(actions: impl IntoIterator<Item = PermissionAction>) -> PermissionAction {
    let mut combined = PermissionAction::Allow;
    for action in actions {
        match action {
            PermissionAction::Deny => return PermissionAction::Deny,
            PermissionAction::Ask => combined = PermissionAction::Ask,
            PermissionAction::Allow => {}
        }
    }
    combined
}

pub fn evaluate_permission_patterns(
    permission: &str,
    patterns: &[String],
    rulesets: &[PermissionRuleset],
) -> PermissionAction {
    if patterns.is_empty() {
        return evaluate(permission, "*", rulesets).action;
    }

    combine_actions(
        patterns
            .iter()
            .map(|pattern| evaluate(permission, pattern, rulesets).action),
    )
}

/// Map a tool name to its permission type.
/// Edit-family tools map to "edit", `ls` maps to "list", others pass through as-is.
pub fn tool_to_permission(tool_name: &str) -> &str {
    match tool_name {
        "write" | "edit" | "multiedit" | "apply_patch" | "patch" => "edit",
        "ls" => "list",
        _ => tool_name,
    }
}

/// Evaluate a tool's permission decision against allowlist and rulesets.
///
/// 1. If `allowed_tools` is non-empty and tool_name is not in the list → Deny.
/// 2. Map tool_name to permission type via `tool_to_permission()`.
/// 3. Evaluate against rulesets; return the matched action (default: Ask).
pub fn evaluate_tool_permission(
    tool_name: &str,
    allowed_tools: &[String],
    rulesets: &[PermissionRuleset],
) -> PermissionAction {
    crate::PermissionEngine::evaluate_tool(tool_name, allowed_tools, rulesets)
}

pub fn evaluate_tool_permission_with_patterns(
    tool_name: &str,
    patterns: &[String],
    allowed_tools: &[String],
    rulesets: &[PermissionRuleset],
) -> PermissionAction {
    crate::PermissionEngine::evaluate_tool_with_patterns(
        tool_name,
        patterns,
        allowed_tools,
        rulesets,
    )
}

pub fn disabled(
    tools: &[String],
    ruleset: &PermissionRuleset,
) -> std::collections::HashSet<String> {
    let mut result = std::collections::HashSet::new();

    for tool in tools {
        let permission = tool_to_permission(tool);

        let rule = ruleset
            .iter()
            .rev()
            .find(|r| wildcard_match(permission, &r.permission));

        if let Some(rule) = rule {
            if rule.pattern == "*" && rule.action == PermissionAction::Deny {
                result.insert(tool.clone());
            }
        }
    }

    result
}

pub fn default_ruleset() -> PermissionRuleset {
    vec![
        PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Allow,
        },
        PermissionRule {
            permission: "doom_loop".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Ask,
        },
        PermissionRule {
            permission: "external_directory".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Ask,
        },
        PermissionRule {
            permission: "question".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Deny,
        },
        PermissionRule {
            permission: "plan_enter".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Deny,
        },
        PermissionRule {
            permission: "plan_exit".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Deny,
        },
        PermissionRule {
            permission: "read".to_string(),
            pattern: "*.env".to_string(),
            action: PermissionAction::Ask,
        },
        PermissionRule {
            permission: "read".to_string(),
            pattern: "*.env.*".to_string(),
            action: PermissionAction::Ask,
        },
        PermissionRule {
            permission: "read".to_string(),
            pattern: "*.env.example".to_string(),
            action: PermissionAction::Allow,
        },
    ]
}

pub fn build_agent_ruleset(agent_name: &str, user_ruleset: &[PermissionRule]) -> PermissionRuleset {
    let defaults = default_ruleset();
    let user = user_ruleset.to_vec();

    match agent_name {
        "build" => {
            let build_specific = vec![
                PermissionRule {
                    permission: "question".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "plan_enter".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
            ];
            merge(&[defaults, build_specific, user])
        }
        "plan" => {
            let plan_specific = vec![
                PermissionRule {
                    permission: "question".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "plan_exit".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "edit".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Deny,
                },
            ];
            merge(&[defaults, plan_specific, user])
        }
        "explore" => {
            let explore_specific = vec![
                PermissionRule {
                    permission: "*".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Deny,
                },
                PermissionRule {
                    permission: "grep".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "glob".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "list".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "bash".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "webfetch".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "websearch".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "codesearch".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "ast_grep_search".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "read".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
            ];
            merge(&[explore_specific, user])
        }
        _ => merge(&[defaults, user]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config() {
        let mut config = HashMap::new();
        config.insert(
            "bash".to_string(),
            ConfigValue::Action(PermissionAction::Allow),
        );

        let ruleset = from_config(&config);
        assert_eq!(ruleset.len(), 1);
        assert_eq!(ruleset[0].permission, "bash");
        assert_eq!(ruleset[0].action, PermissionAction::Allow);
    }

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("foo", "*"));
        assert!(wildcard_match("foo/bar", "foo/*"));
        assert!(wildcard_match("foo/bar/baz", "*/baz"));
        assert!(wildcard_match("foo/bar/baz", "*bar*"));
        assert!(!wildcard_match("foo", "bar"));
    }

    #[test]
    fn test_disabled() {
        let ruleset = vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Deny,
        }];

        let tools = vec!["bash".to_string(), "read".to_string()];
        let disabled_tools = disabled(&tools, &ruleset);

        assert!(disabled_tools.contains("bash"));
        assert!(!disabled_tools.contains("read"));
    }

    #[test]
    fn tool_to_permission_maps_edit_tools() {
        assert_eq!(tool_to_permission("write"), "edit");
        assert_eq!(tool_to_permission("edit"), "edit");
        assert_eq!(tool_to_permission("multiedit"), "edit");
        assert_eq!(tool_to_permission("apply_patch"), "edit");
        assert_eq!(tool_to_permission("patch"), "edit");
    }

    #[test]
    fn tool_to_permission_maps_ls() {
        assert_eq!(tool_to_permission("ls"), "list");
    }

    #[test]
    fn tool_to_permission_passes_through_unknown() {
        assert_eq!(tool_to_permission("bash"), "bash");
        assert_eq!(tool_to_permission("grep"), "grep");
        assert_eq!(tool_to_permission("read"), "read");
    }

    #[test]
    fn evaluate_tool_permission_allows_tool_in_allowlist() {
        let ruleset = vec![PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Deny,
        }];
        // Tool is in allowlist — even with deny-all ruleset, check proceeds to ruleset
        let result = evaluate_tool_permission("grep", &["grep".to_string()], &[ruleset]);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn evaluate_tool_permission_denies_tool_not_in_allowlist() {
        let ruleset = vec![PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Allow,
        }];
        // Tool NOT in non-empty allowlist → Deny regardless of ruleset
        let result = evaluate_tool_permission("write", &["grep".to_string()], &[ruleset]);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn evaluate_tool_permission_empty_allowlist_means_no_filter() {
        let ruleset = vec![PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Allow,
        }];
        // Empty allowlist → no allowlist filter, proceed to ruleset
        let result = evaluate_tool_permission("write", &[], &[ruleset]);
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn evaluate_tool_permission_maps_tool_name_to_permission() {
        let ruleset = vec![PermissionRule {
            permission: "edit".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Ask,
        }];
        // "write" maps to "edit" permission via tool_to_permission
        let result = evaluate_tool_permission("write", &[], &[ruleset]);
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn evaluate_tool_permission_defaults_to_ask() {
        // No matching rules → default Ask
        let result = evaluate_tool_permission("unknown_tool", &[], &[]);
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn combine_actions_uses_deny_over_ask_over_allow() {
        assert_eq!(
            combine_actions([PermissionAction::Allow, PermissionAction::Allow]),
            PermissionAction::Allow
        );
        assert_eq!(
            combine_actions([PermissionAction::Allow, PermissionAction::Ask]),
            PermissionAction::Ask
        );
        assert_eq!(
            combine_actions([PermissionAction::Ask, PermissionAction::Deny]),
            PermissionAction::Deny
        );
    }

    #[test]
    fn evaluate_permission_patterns_folds_multiple_patterns() {
        let ruleset = vec![
            PermissionRule {
                permission: "read".to_string(),
                pattern: "allowed/*".to_string(),
                action: PermissionAction::Allow,
            },
            PermissionRule {
                permission: "read".to_string(),
                pattern: "secret/*".to_string(),
                action: PermissionAction::Ask,
            },
        ];

        let result = evaluate_permission_patterns(
            "read",
            &[
                "allowed/file.txt".to_string(),
                "secret/file.txt".to_string(),
            ],
            &[ruleset],
        );
        assert_eq!(result, PermissionAction::Ask);
    }
}
