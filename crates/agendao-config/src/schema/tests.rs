use super::*;
use std::collections::HashMap;

#[test]
fn merges_nested_structs_without_losing_existing_fields() {
    let mut base = Config {
        keybinds: Some(KeybindsConfig {
            submit: Some("enter".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let overlay = Config {
        keybinds: Some(KeybindsConfig {
            interrupt: Some("esc".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    base.merge(overlay);

    let merged = base.keybinds.unwrap();
    assert_eq!(merged.submit, Some("enter".to_string()));
    assert_eq!(merged.interrupt, Some("esc".to_string()));
}

#[test]
fn merges_maps_recursively_for_same_keys() {
    let mut base = Config {
        provider: Some(HashMap::from([(
            "openai".to_string(),
            ProviderConfig {
                base_url: Some("https://old".to_string()),
                models: Some(HashMap::from([(
                    "gpt-4o".to_string(),
                    ModelConfig {
                        api_key: Some("old-key".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };

    let overlay = Config {
        provider: Some(HashMap::from([(
            "openai".to_string(),
            ProviderConfig {
                api_key: Some("new-provider-key".to_string()),
                models: Some(HashMap::from([(
                    "gpt-4o".to_string(),
                    ModelConfig {
                        model: Some("gpt-4o-2026".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };

    base.merge(overlay);

    let provider = base.provider.unwrap().remove("openai").unwrap();
    assert_eq!(provider.base_url, Some("https://old".to_string()));
    assert_eq!(provider.api_key, Some("new-provider-key".to_string()));

    let model = provider.models.unwrap().remove("gpt-4o").unwrap();
    assert_eq!(model.api_key, Some("old-key".to_string()));
    assert_eq!(model.model, Some("gpt-4o-2026".to_string()));
}

#[test]
fn provider_model_merge_preserves_and_updates_extended_model_fields() {
    let mut base = Config {
        provider: Some(HashMap::from([(
            "openai".to_string(),
            ProviderConfig {
                models: Some(HashMap::from([(
                    "gpt-5".to_string(),
                    ModelConfig {
                        reasoning: Some(true),
                        interleaved: Some(ModelInterleavedConfig::Field {
                            field: "reasoning_content".to_string(),
                        }),
                        options: Some(HashMap::from([
                            (
                                "reasoning".to_string(),
                                serde_json::json!({"effort": "medium"}),
                            ),
                            ("verbosity".to_string(), serde_json::json!("low")),
                        ])),
                        cost: Some(ModelCostConfig {
                            input: Some(1.0),
                            context_over_200k: Some(Box::new(ModelCostConfig {
                                output: Some(9.0),
                                ..Default::default()
                            })),
                            ..Default::default()
                        }),
                        limit: Some(ModelLimitConfig {
                            context: Some(128_000),
                            ..Default::default()
                        }),
                        headers: Some(HashMap::from([("x-base".to_string(), "keep".to_string())])),
                        provider: Some(ModelProviderConfig {
                            api: Some("https://base.example".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };

    let overlay = Config {
        provider: Some(HashMap::from([(
            "openai".to_string(),
            ProviderConfig {
                models: Some(HashMap::from([(
                    "gpt-5".to_string(),
                    ModelConfig {
                        attachment: Some(true),
                        modalities: Some(ModelModalities {
                            output: Some(vec!["text".to_string(), "audio".to_string()]),
                            ..Default::default()
                        }),
                        options: Some(HashMap::from([
                            (
                                "reasoning".to_string(),
                                serde_json::json!({"summary": "auto"}),
                            ),
                            ("parallel_tool_calls".to_string(), serde_json::json!(true)),
                        ])),
                        cost: Some(ModelCostConfig {
                            cache_write: Some(3.0),
                            context_over_200k: Some(Box::new(ModelCostConfig {
                                input: Some(7.0),
                                ..Default::default()
                            })),
                            ..Default::default()
                        }),
                        limit: Some(ModelLimitConfig {
                            output: Some(8_192),
                            ..Default::default()
                        }),
                        headers: Some(HashMap::from([
                            ("x-overlay".to_string(), "set".to_string()),
                            ("x-base".to_string(), "override".to_string()),
                        ])),
                        provider: Some(ModelProviderConfig {
                            npm: Some("@ai-sdk/openai".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };

    base.merge(overlay);

    let provider = base.provider.unwrap().remove("openai").unwrap();
    let model = provider.models.unwrap().remove("gpt-5").unwrap();
    assert_eq!(model.reasoning, Some(true));
    assert_eq!(model.attachment, Some(true));
    assert!(matches!(
        model.interleaved,
        Some(ModelInterleavedConfig::Field { ref field }) if field == "reasoning_content"
    ));
    assert_eq!(
        model
            .modalities
            .as_ref()
            .and_then(|modalities| modalities.output.as_ref())
            .cloned(),
        Some(vec!["text".to_string(), "audio".to_string()])
    );

    let options = model.options.as_ref().expect("model options");
    assert_eq!(
        options.get("reasoning"),
        Some(&serde_json::json!({"effort": "medium", "summary": "auto"}))
    );
    assert_eq!(options.get("verbosity"), Some(&serde_json::json!("low")));
    assert_eq!(
        options.get("parallel_tool_calls"),
        Some(&serde_json::json!(true))
    );

    let cost = model.cost.as_ref().expect("model cost");
    assert_eq!(cost.input, Some(1.0));
    assert_eq!(cost.cache_write, Some(3.0));
    let over_200k = cost.context_over_200k.as_ref().expect("nested cost");
    assert_eq!(over_200k.input, Some(7.0));
    assert_eq!(over_200k.output, Some(9.0));

    let limit = model.limit.as_ref().expect("model limit");
    assert_eq!(limit.context, Some(128_000));
    assert_eq!(limit.output, Some(8_192));

    let headers = model.headers.as_ref().expect("headers");
    assert_eq!(headers.get("x-base").map(String::as_str), Some("override"));
    assert_eq!(headers.get("x-overlay").map(String::as_str), Some("set"));

    let provider_cfg = model.provider.as_ref().expect("provider override");
    assert_eq!(provider_cfg.api.as_deref(), Some("https://base.example"));
    assert_eq!(provider_cfg.npm.as_deref(), Some("@ai-sdk/openai"));
}

#[test]
fn external_adapter_config_keeps_secret_values_out_of_config_and_merges_refs() {
    let mut config: Config = serde_json::from_value(serde_json::json!({
        "externalAdapter": {
            "adapters": {
                "generic": {
                    "enabled": true,
                    "source": "generic-webhook",
                    "secretRef": "external-adapter:generic",
                    "defaultWorkspace": "main",
                    "allowSessionRun": true,
                    "allowedWorkspaces": ["main"]
                }
            },
            "replay": {
                "retentionSeconds": 86400,
                "nonceWindowSeconds": 300
            }
        }
    }))
    .unwrap();

    let adapter = config
        .external_adapter
        .take()
        .unwrap()
        .adapters
        .remove("generic")
        .unwrap();
    assert_eq!(
        adapter.secret_ref.as_deref(),
        Some("external-adapter:generic")
    );
    assert_eq!(adapter.allow_session_run, Some(true));
    let serialized = serde_json::to_value(&adapter).unwrap();
    assert!(serialized.get("secret").is_none());
    assert!(serialized.get("signature").is_none());
}

#[test]
fn external_adapter_config_deep_merges_adapter_entries() {
    let mut base = Config {
        external_adapter: Some(ExternalAdapterConfig {
            adapters: HashMap::from([(
                "generic".to_string(),
                ExternalAdapterEntryConfig {
                    enabled: Some(true),
                    source: Some("generic-webhook".to_string()),
                    secret_ref: Some("external-adapter:generic".to_string()),
                    default_workspace: Some("main".to_string()),
                    route_policy_id: None,
                    allow_session_run: Some(false),
                    allowed_workspaces: vec!["main".to_string()],
                },
            )]),
            replay: Some(ExternalAdapterReplayConfig {
                retention_seconds: Some(86_400),
                nonce_window_seconds: None,
            }),
        }),
        ..Default::default()
    };

    base.merge(Config {
        external_adapter: Some(ExternalAdapterConfig {
            adapters: HashMap::from([(
                "generic".to_string(),
                ExternalAdapterEntryConfig {
                    route_policy_id: Some("trusted-webhook".to_string()),
                    allow_session_run: Some(true),
                    allowed_workspaces: vec!["ops".to_string()],
                    ..Default::default()
                },
            )]),
            replay: Some(ExternalAdapterReplayConfig {
                retention_seconds: None,
                nonce_window_seconds: Some(300),
            }),
        }),
        ..Default::default()
    });

    let external = base.external_adapter.unwrap();
    let adapter = external.adapters.get("generic").unwrap();
    assert_eq!(adapter.enabled, Some(true));
    assert_eq!(adapter.source.as_deref(), Some("generic-webhook"));
    assert_eq!(
        adapter.secret_ref.as_deref(),
        Some("external-adapter:generic")
    );
    assert_eq!(adapter.route_policy_id.as_deref(), Some("trusted-webhook"));
    assert_eq!(adapter.allow_session_run, Some(true));
    assert_eq!(adapter.allowed_workspaces, vec!["ops"]);
    let replay = external.replay.unwrap();
    assert_eq!(replay.retention_seconds, Some(86_400));
    assert_eq!(replay.nonce_window_seconds, Some(300));
}

#[test]
fn provider_merge_replaces_identity_and_env_while_preserving_existing_models() {
    let mut base = Config {
        provider: Some(HashMap::from([(
            "custom".to_string(),
            ProviderConfig {
                id: Some("provider-old".to_string()),
                env: Some(vec!["OLD_TOKEN".to_string()]),
                models: Some(HashMap::from([(
                    "baseline".to_string(),
                    ModelConfig {
                        model: Some("baseline-model".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };

    base.merge(Config {
        provider: Some(HashMap::from([(
            "custom".to_string(),
            ProviderConfig {
                id: Some("provider-new".to_string()),
                env: Some(vec!["NEW_TOKEN".to_string(), "FALLBACK_TOKEN".to_string()]),
                models: Some(HashMap::from([(
                    "advanced".to_string(),
                    ModelConfig {
                        model: Some("advanced-model".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )])),
        ..Default::default()
    });

    let provider = base.provider.unwrap().remove("custom").unwrap();
    assert_eq!(provider.id.as_deref(), Some("provider-new"));
    assert_eq!(
        provider.env,
        Some(vec!["NEW_TOKEN".to_string(), "FALLBACK_TOKEN".to_string()])
    );

    let models = provider.models.expect("merged models");
    assert!(models.contains_key("baseline"));
    assert!(models.contains_key("advanced"));
}

#[test]
fn docs_config_merge_replaces_registry_path() {
    let mut base = Config {
        docs: Some(DocsConfig {
            context_docs_registry_path: Some("docs/base-registry.json".to_string()),
        }),
        ..Default::default()
    };

    let overlay = Config {
        docs: Some(DocsConfig {
            context_docs_registry_path: Some("docs/override-registry.json".to_string()),
        }),
        ..Default::default()
    };

    base.merge(overlay);

    assert_eq!(
        base.docs.and_then(|docs| docs.context_docs_registry_path),
        Some("docs/override-registry.json".to_string())
    );
}

#[test]
fn skills_hub_config_deserializes_from_camel_and_snake_case() {
    let camel: Config = serde_json::from_value(serde_json::json!({
        "skills": {
            "hub": {
                "artifactCacheRetentionSeconds": 86400,
                "fetchTimeoutMs": 15000,
                "maxDownloadBytes": 1048576,
                "maxExtractBytes": 2097152
            }
        }
    }))
    .expect("camelCase skills hub config should deserialize");
    let camel_hub = camel
        .skills
        .and_then(|skills| skills.hub)
        .expect("camelCase skills hub config should exist");
    assert_eq!(camel_hub.artifact_cache_retention_seconds, Some(86400));
    assert_eq!(camel_hub.fetch_timeout_ms, Some(15000));
    assert_eq!(camel_hub.max_download_bytes, Some(1048576));
    assert_eq!(camel_hub.max_extract_bytes, Some(2097152));

    let snake: Config = serde_json::from_value(serde_json::json!({
        "skills": {
            "hub": {
                "artifact_cache_retention_seconds": 3600,
                "fetch_timeout_ms": 5000,
                "max_download_bytes": 2048,
                "max_extract_bytes": 4096
            }
        }
    }))
    .expect("snake_case skills hub config should deserialize");
    let snake_hub = snake
        .skills
        .and_then(|skills| skills.hub)
        .expect("snake_case skills hub config should exist");
    assert_eq!(snake_hub.artifact_cache_retention_seconds, Some(3600));
    assert_eq!(snake_hub.fetch_timeout_ms, Some(5000));
    assert_eq!(snake_hub.max_download_bytes, Some(2048));
    assert_eq!(snake_hub.max_extract_bytes, Some(4096));
}

#[test]
fn skills_hub_config_merge_replaces_phase_seven_policy_fields() {
    let mut base = Config {
        skills: Some(SkillsConfig {
            hub: Some(SkillHubConfig {
                artifact_cache_retention_seconds: Some(86400),
                fetch_timeout_ms: Some(10000),
                max_download_bytes: Some(1_000_000),
                max_extract_bytes: None,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let overlay = Config {
        skills: Some(SkillsConfig {
            hub: Some(SkillHubConfig {
                artifact_cache_retention_seconds: Some(600),
                fetch_timeout_ms: None,
                max_download_bytes: None,
                max_extract_bytes: Some(2_000_000),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    base.merge(overlay);
    let hub = base
        .skills
        .and_then(|skills| skills.hub)
        .expect("merged skills hub config should exist");
    assert_eq!(hub.artifact_cache_retention_seconds, Some(600));
    assert_eq!(hub.fetch_timeout_ms, Some(10000));
    assert_eq!(hub.max_download_bytes, Some(1_000_000));
    assert_eq!(hub.max_extract_bytes, Some(2_000_000));
}

#[test]
fn plugin_map_merge_and_instruction_arrays_append_unique() {
    let mut base = Config {
        plugin: HashMap::from([
            (
                "a".to_string(),
                PluginConfig {
                    plugin_type: "npm".to_string(),
                    package: Some("a".to_string()),
                    ..Default::default()
                },
            ),
            (
                "b".to_string(),
                PluginConfig {
                    plugin_type: "npm".to_string(),
                    package: Some("b".to_string()),
                    ..Default::default()
                },
            ),
        ]),
        instructions: vec!["one".to_string(), "two".to_string()],
        ..Default::default()
    };

    let overlay = Config {
        plugin: HashMap::from([
            (
                "b".to_string(),
                PluginConfig {
                    plugin_type: "npm".to_string(),
                    package: Some("b-updated".to_string()),
                    ..Default::default()
                },
            ),
            (
                "c".to_string(),
                PluginConfig {
                    plugin_type: "npm".to_string(),
                    package: Some("c".to_string()),
                    ..Default::default()
                },
            ),
        ]),
        instructions: vec!["two".to_string(), "three".to_string()],
        ..Default::default()
    };

    base.merge(overlay);

    // plugin map: 3 entries, "b" overwritten by overlay
    assert_eq!(base.plugin.len(), 3);
    assert_eq!(base.plugin["b"].package.as_deref(), Some("b-updated"));
    assert!(base.plugin.contains_key("c"));
    assert_eq!(
        base.instructions,
        vec!["one".to_string(), "two".to_string(), "three".to_string()]
    );
}

#[test]
fn provider_lists_follow_replace_semantics_instead_of_union() {
    let mut base = Config {
        disabled_providers: vec!["ethnopic".to_string()],
        enabled_providers: vec!["openai".to_string()],
        ..Default::default()
    };

    let overlay = Config {
        disabled_providers: vec!["google".to_string()],
        ..Default::default()
    };

    base.merge(overlay);

    assert_eq!(base.disabled_providers, vec!["google".to_string()]);
    assert_eq!(base.enabled_providers, vec!["openai".to_string()]);
}

#[test]
fn mcp_enabled_flag_overlay_keeps_existing_full_server_fields() {
    let mut base = Config {
        mcp: Some(HashMap::from([(
            "repo".to_string(),
            McpServerConfig::Full(Box::new(McpServer {
                command: vec!["node".to_string(), "mcp.js".to_string()],
                timeout: Some(3000),
                ..Default::default()
            })),
        )])),
        ..Default::default()
    };

    let overlay = Config {
        mcp: Some(HashMap::from([(
            "repo".to_string(),
            McpServerConfig::Enabled { enabled: false },
        )])),
        ..Default::default()
    };

    base.merge(overlay);

    let server = base.mcp.unwrap().remove("repo").unwrap();
    match server {
        McpServerConfig::Full(server) => {
            assert_eq!(
                server.command,
                vec!["node".to_string(), "mcp.js".to_string()]
            );
            assert_eq!(server.timeout, Some(3000));
            assert_eq!(server.enabled, Some(false));
        }
        McpServerConfig::Enabled { .. } => panic!("expected full MCP server config"),
    }
}

#[test]
fn mcp_full_server_merge_overwrites_maps_and_preserves_unspecified_fields() {
    let mut base = Config {
        mcp: Some(HashMap::from([(
            "repo".to_string(),
            McpServerConfig::Full(Box::new(McpServer {
                server_type: Some("local".to_string()),
                command: vec!["node".to_string(), "server.js".to_string()],
                environment: Some(HashMap::from([("A".to_string(), "1".to_string())])),
                enabled: Some(true),
                timeout: Some(3000),
                headers: Some(HashMap::from([("x-base".to_string(), "keep".to_string())])),
                args: vec!["--stdio".to_string()],
                env: Some(HashMap::from([("LEGACY".to_string(), "base".to_string())])),
                client_id: Some("base-client".to_string()),
                ..Default::default()
            })),
        )])),
        ..Default::default()
    };

    base.merge(Config {
        mcp: Some(HashMap::from([(
            "repo".to_string(),
            McpServerConfig::Full(Box::new(McpServer {
                url: Some("https://mcp.example".to_string()),
                timeout: Some(5000),
                environment: Some(HashMap::from([
                    ("A".to_string(), "2".to_string()),
                    ("B".to_string(), "3".to_string()),
                ])),
                headers: Some(HashMap::from([
                    ("x-base".to_string(), "override".to_string()),
                    ("x-extra".to_string(), "set".to_string()),
                ])),
                env: Some(HashMap::from([(
                    "LEGACY_2".to_string(),
                    "overlay".to_string(),
                )])),
                authorization_url: Some("https://auth.example".to_string()),
                ..Default::default()
            })),
        )])),
        ..Default::default()
    });

    let server = base.mcp.unwrap().remove("repo").unwrap();
    match server {
        McpServerConfig::Full(server) => {
            assert_eq!(server.server_type.as_deref(), Some("local"));
            assert_eq!(
                server.command,
                vec!["node".to_string(), "server.js".to_string()]
            );
            assert_eq!(server.url.as_deref(), Some("https://mcp.example"));
            assert_eq!(server.timeout, Some(5000));
            assert_eq!(server.enabled, Some(true));
            assert_eq!(server.args, vec!["--stdio".to_string()]);
            assert_eq!(server.client_id.as_deref(), Some("base-client"));
            assert_eq!(
                server
                    .environment
                    .as_ref()
                    .and_then(|env| env.get("A"))
                    .map(String::as_str),
                Some("2")
            );
            assert_eq!(
                server
                    .environment
                    .as_ref()
                    .and_then(|env| env.get("B"))
                    .map(String::as_str),
                Some("3")
            );
            assert_eq!(
                server
                    .headers
                    .as_ref()
                    .and_then(|headers| headers.get("x-base"))
                    .map(String::as_str),
                Some("override")
            );
            assert_eq!(
                server
                    .headers
                    .as_ref()
                    .and_then(|headers| headers.get("x-extra"))
                    .map(String::as_str),
                Some("set")
            );
            assert_eq!(
                server
                    .env
                    .as_ref()
                    .and_then(|env| env.get("LEGACY"))
                    .map(String::as_str),
                Some("base")
            );
            assert_eq!(
                server
                    .env
                    .as_ref()
                    .and_then(|env| env.get("LEGACY_2"))
                    .map(String::as_str),
                Some("overlay")
            );
            assert_eq!(
                server.authorization_url.as_deref(),
                Some("https://auth.example")
            );
        }
        McpServerConfig::Enabled { .. } => panic!("expected full MCP server config"),
    }
}

#[test]
fn agent_configs_support_dynamic_keys_and_deep_merge() {
    let mut base = Config {
        agent: Some(AgentConfigs {
            entries: HashMap::from([(
                "reviewer".to_string(),
                AgentConfig {
                    prompt: Some("old prompt".to_string()),
                    options: Some(HashMap::from([("a".to_string(), serde_json::json!(1))])),
                    ..Default::default()
                },
            )]),
        }),
        ..Default::default()
    };

    let overlay = Config {
        agent: Some(AgentConfigs {
            entries: HashMap::from([
                (
                    "reviewer".to_string(),
                    AgentConfig {
                        prompt: Some("new prompt".to_string()),
                        options: Some(HashMap::from([("b".to_string(), serde_json::json!(2))])),
                        ..Default::default()
                    },
                ),
                (
                    "research".to_string(),
                    AgentConfig {
                        mode: Some(AgentMode::Subagent),
                        ..Default::default()
                    },
                ),
            ]),
        }),
        ..Default::default()
    };

    base.merge(overlay);

    let agents = base.agent.unwrap().entries;
    let reviewer = agents.get("reviewer").unwrap();
    assert_eq!(reviewer.prompt.as_deref(), Some("new prompt"));
    let options = reviewer.options.as_ref().unwrap();
    assert_eq!(options.get("a"), Some(&serde_json::json!(1)));
    assert_eq!(options.get("b"), Some(&serde_json::json!(2)));
    assert!(agents.contains_key("research"));
}

#[test]
fn composition_skill_tree_deserializes_from_camel_case() {
    let config: Config = serde_json::from_value(serde_json::json!({
        "composition": {
            "skillTree": {
                "enabled": true,
                "separator": "\n--\n",
                "tokenBudget": 512,
                "truncationStrategy": "tail",
                "root": {
                    "node_id": "root",
                    "markdown_path": "docs/root.md",
                    "children": []
                }
            }
        }
    }))
    .expect("config should deserialize");

    let skill_tree = config
        .composition
        .as_ref()
        .and_then(|c| c.skill_tree.as_ref())
        .expect("composition skill tree should exist");
    assert_eq!(skill_tree.enabled, Some(true));
    assert_eq!(skill_tree.separator.as_deref(), Some("\n--\n"));
    assert_eq!(skill_tree.token_budget, Some(512));
    assert_eq!(skill_tree.truncation_strategy.as_deref(), Some("tail"));
    assert_eq!(
        skill_tree.root.as_ref().map(|root| root.node_id.as_str()),
        Some("root")
    );
}

#[test]
fn composition_skill_tree_merge_replaces_root_and_separator() {
    let mut base = Config {
        composition: Some(CompositionConfig {
            skill_tree: Some(SkillTreeConfig {
                enabled: Some(true),
                separator: Some("old".to_string()),
                token_budget: Some(128),
                truncation_strategy: Some("head".to_string()),
                root: Some(SkillTreeNodeConfig {
                    node_id: "old".to_string(),
                    markdown_path: "docs/old.md".to_string(),
                    children: Vec::new(),
                }),
            }),
        }),
        ..Default::default()
    };

    let overlay = Config {
        composition: Some(CompositionConfig {
            skill_tree: Some(SkillTreeConfig {
                enabled: Some(false),
                separator: Some("new".to_string()),
                token_budget: Some(256),
                truncation_strategy: Some("head-tail".to_string()),
                root: Some(SkillTreeNodeConfig {
                    node_id: "new".to_string(),
                    markdown_path: "docs/new.md".to_string(),
                    children: Vec::new(),
                }),
            }),
        }),
        ..Default::default()
    };

    base.merge(overlay);

    let merged = base
        .composition
        .as_ref()
        .and_then(|c| c.skill_tree.as_ref())
        .expect("merged skill tree should exist");
    assert_eq!(merged.enabled, Some(false));
    assert_eq!(merged.separator.as_deref(), Some("new"));
    assert_eq!(merged.token_budget, Some(256));
    assert_eq!(merged.truncation_strategy.as_deref(), Some("head-tail"));
    assert_eq!(
        merged.root.as_ref().map(|root| root.markdown_path.as_str()),
        Some("docs/new.md")
    );
}

#[test]
fn scheduler_path_deserializes_from_camel_case() {
    let config: Config = serde_json::from_value(serde_json::json!({
        "schedulerPath": "./.agendao/scheduler/sisyphus.jsonc"
    }))
    .expect("config should deserialize");

    assert_eq!(
        config.scheduler_path.as_deref(),
        Some("./.agendao/scheduler/sisyphus.jsonc")
    );
}

#[test]
fn scheduler_path_merge_replaces_previous_value() {
    let mut base = Config {
        scheduler_path: Some("/base/scheduler.jsonc".to_string()),
        ..Default::default()
    };

    let overlay = Config {
        scheduler_path: Some("/override/scheduler.jsonc".to_string()),
        ..Default::default()
    };

    base.merge(overlay);

    assert_eq!(
        base.scheduler_path.as_deref(),
        Some("/override/scheduler.jsonc")
    );
}

#[test]
fn web_search_merge_replaces_previous_base_url() {
    let mut base = Config {
        web_search: Some(WebSearchConfig {
            base_url: Some("https://old.example".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let overlay = Config {
        web_search: Some(WebSearchConfig {
            base_url: Some("https://new.example".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    base.merge(overlay);

    assert_eq!(
        base.web_search
            .as_ref()
            .and_then(|config| config.base_url.as_deref()),
        Some("https://new.example")
    );
}

#[test]
fn web_search_merge_deep_merges_all_fields() {
    let mut base = Config {
        web_search: Some(WebSearchConfig {
            base_url: Some("https://mcp.exa.ai".to_string()),
            method: Some("web_search_exa".to_string()),
            default_search_type: Some("auto".to_string()),
            default_num_results: Some(8),
            options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("livecrawl".to_string(), serde_json::json!("fallback"));
                m.insert("region".to_string(), serde_json::json!("us"));
                m
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let overlay = Config {
        web_search: Some(WebSearchConfig {
            endpoint: Some("/v2/search".to_string()),
            default_search_type: Some("deep".to_string()),
            options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("livecrawl".to_string(), serde_json::json!("preferred"));
                m.insert("language".to_string(), serde_json::json!("zh"));
                m
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    base.merge(overlay);

    let ws = base.web_search.as_ref().unwrap();
    // base_url kept from base (overlay didn't set it)
    assert_eq!(ws.base_url.as_deref(), Some("https://mcp.exa.ai"));
    // endpoint set by overlay
    assert_eq!(ws.endpoint.as_deref(), Some("/v2/search"));
    // method kept from base
    assert_eq!(ws.method.as_deref(), Some("web_search_exa"));
    // default_search_type overridden by overlay
    assert_eq!(ws.default_search_type.as_deref(), Some("deep"));
    // default_num_results kept from base
    assert_eq!(ws.default_num_results, Some(8));
    // options: key-level merge
    let opts = ws.options.as_ref().unwrap();
    assert_eq!(opts.get("livecrawl").unwrap(), "preferred"); // overridden
    assert_eq!(opts.get("region").unwrap(), "us"); // kept from base
    assert_eq!(opts.get("language").unwrap(), "zh"); // added by overlay
}

#[test]
fn formatter_config_merge_deep_merges_entries_and_overwrites_env_by_key() {
    let mut base = Config {
        formatter: Some(FormatterConfig::Enabled(HashMap::from([(
            "rust".to_string(),
            FormatterEntry {
                disabled: Some(false),
                command: vec!["rustfmt".to_string()],
                environment: Some(HashMap::from([("A".to_string(), "1".to_string())])),
                extensions: vec!["rs".to_string()],
            },
        )]))),
        ..Default::default()
    };

    base.merge(Config {
        formatter: Some(FormatterConfig::Enabled(HashMap::from([
            (
                "rust".to_string(),
                FormatterEntry {
                    disabled: Some(true),
                    command: Vec::new(),
                    environment: Some(HashMap::from([
                        ("A".to_string(), "2".to_string()),
                        ("B".to_string(), "3".to_string()),
                    ])),
                    extensions: vec!["rs".to_string(), "rs.in".to_string()],
                },
            ),
            (
                "markdown".to_string(),
                FormatterEntry {
                    command: vec!["prettier".to_string()],
                    extensions: vec!["md".to_string()],
                    ..Default::default()
                },
            ),
        ]))),
        ..Default::default()
    });

    let formatter = match base.formatter.expect("formatter config should exist") {
        FormatterConfig::Enabled(entries) => entries,
        FormatterConfig::Disabled(_) => panic!("expected enabled formatter config"),
    };

    let rust = formatter.get("rust").expect("rust formatter should exist");
    assert_eq!(rust.disabled, Some(true));
    assert_eq!(rust.command, vec!["rustfmt".to_string()]);
    assert_eq!(
        rust.environment
            .as_ref()
            .and_then(|env| env.get("A"))
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        rust.environment
            .as_ref()
            .and_then(|env| env.get("B"))
            .map(String::as_str),
        Some("3")
    );
    assert_eq!(rust.extensions, vec!["rs".to_string(), "rs.in".to_string()]);

    let markdown = formatter
        .get("markdown")
        .expect("markdown formatter should exist");
    assert_eq!(markdown.command, vec!["prettier".to_string()]);
    assert_eq!(markdown.extensions, vec!["md".to_string()]);
}

#[test]
fn lsp_config_merge_deep_merges_server_fields_and_initialization_json() {
    let mut base = Config {
        lsp: Some(LspConfig::Enabled(HashMap::from([(
            "rust-analyzer".to_string(),
            LspServerConfig {
                command: vec!["rust-analyzer".to_string()],
                extensions: vec!["rs".to_string()],
                env: Some(HashMap::from([(
                    "RUSTUP_TOOLCHAIN".to_string(),
                    "stable".to_string(),
                )])),
                initialization: Some(HashMap::from([
                    ("top".to_string(), serde_json::json!("keep")),
                    ("caps".to_string(), serde_json::json!({ "a": 1 })),
                ])),
                ..Default::default()
            },
        )]))),
        ..Default::default()
    };

    base.merge(Config {
        lsp: Some(LspConfig::Enabled(HashMap::from([(
            "rust-analyzer".to_string(),
            LspServerConfig {
                command: Vec::new(),
                extensions: vec!["rs".to_string(), "ron".to_string()],
                disabled: Some(true),
                env: Some(HashMap::from([
                    ("RUSTUP_TOOLCHAIN".to_string(), "nightly".to_string()),
                    ("PROC_MACRO".to_string(), "1".to_string()),
                ])),
                initialization: Some(HashMap::from([
                    ("caps".to_string(), serde_json::json!({ "b": 2 })),
                    ("extra".to_string(), serde_json::json!(true)),
                ])),
            },
        )]))),
        ..Default::default()
    });

    let lsp = match base.lsp.expect("lsp config should exist") {
        LspConfig::Enabled(entries) => entries,
        LspConfig::Disabled(_) => panic!("expected enabled lsp config"),
    };

    let rust = lsp
        .get("rust-analyzer")
        .expect("rust-analyzer config should exist");
    assert_eq!(rust.command, vec!["rust-analyzer".to_string()]);
    assert_eq!(rust.extensions, vec!["rs".to_string(), "ron".to_string()]);
    assert_eq!(rust.disabled, Some(true));
    assert_eq!(
        rust.env
            .as_ref()
            .and_then(|env| env.get("RUSTUP_TOOLCHAIN"))
            .map(String::as_str),
        Some("nightly")
    );
    assert_eq!(
        rust.env
            .as_ref()
            .and_then(|env| env.get("PROC_MACRO"))
            .map(String::as_str),
        Some("1")
    );
    let init = rust
        .initialization
        .as_ref()
        .expect("initialization should exist");
    assert_eq!(init.get("top"), Some(&serde_json::json!("keep")));
    assert_eq!(init.get("extra"), Some(&serde_json::json!(true)));
    assert_eq!(
        init.get("caps"),
        Some(&serde_json::json!({ "a": 1, "b": 2 }))
    );
}

#[test]
fn voice_config_deserializes_from_camel_and_snake_case() {
    let camel: Config = serde_json::from_value(serde_json::json!({
        "voice": {
            "durationSeconds": 20,
            "attachAudio": true,
            "mime": "audio/wav",
            "language": "zh",
            "record": {
                "command": ["ffmpeg", "{file}"]
            }
        }
    }))
    .expect("camelCase voice config should deserialize");
    let camel_voice = camel.voice.expect("camelCase voice config should exist");
    assert_eq!(camel_voice.duration_seconds, Some(20));
    assert_eq!(camel_voice.attach_audio, Some(true));
    assert_eq!(camel_voice.language.as_deref(), Some("zh"));
    assert_eq!(
        camel_voice
            .record
            .as_ref()
            .map(|record| record.command.clone()),
        Some(vec!["ffmpeg".to_string(), "{file}".to_string()])
    );

    let snake: Config = serde_json::from_value(serde_json::json!({
        "voice": {
            "duration_seconds": 8,
            "attach_audio": false,
            "transcribe": {
                "command": ["whisper-cli", "{file}"],
                "env": { "MODEL": "base" }
            }
        }
    }))
    .expect("snake_case voice config should deserialize");
    let snake_voice = snake.voice.expect("snake_case voice config should exist");
    assert_eq!(snake_voice.duration_seconds, Some(8));
    assert_eq!(snake_voice.attach_audio, Some(false));
    assert_eq!(
        snake_voice
            .transcribe
            .as_ref()
            .and_then(|command| command.env.get("MODEL"))
            .map(String::as_str),
        Some("base")
    );
}

#[test]
fn multimodal_config_deserializes_and_nests_voice() {
    let config: Config = serde_json::from_value(serde_json::json!({
        "multimodal": {
            "limits": {
                "maxInputBytes": 4096,
                "max_attachments_per_prompt": 3
            },
            "policy": {
                "allowAudioInput": true,
                "allow_image_input": false,
                "allowFileInput": true
            },
            "voice": {
                "durationSeconds": 18,
                "attachAudio": false
            }
        }
    }))
    .expect("multimodal config should deserialize");

    let multimodal = config.multimodal.expect("multimodal config should exist");
    let limits = multimodal.limits.expect("limits should exist");
    assert_eq!(limits.max_input_bytes, Some(4096));
    assert_eq!(limits.max_attachments_per_prompt, Some(3));

    let policy = multimodal.policy.expect("policy should exist");
    assert_eq!(policy.allow_audio_input, Some(true));
    assert_eq!(policy.allow_image_input, Some(false));
    assert_eq!(policy.allow_file_input, Some(true));

    let voice = multimodal.voice.expect("voice should exist");
    assert_eq!(voice.duration_seconds, Some(18));
    assert_eq!(voice.attach_audio, Some(false));
}

#[test]
fn voice_config_merge_deep_merges_record_and_transcribe() {
    let mut base = Config {
        voice: Some(VoiceConfig {
            duration_seconds: Some(15),
            attach_audio: Some(true),
            mime: Some("audio/wav".to_string()),
            language: Some("zh".to_string()),
            record: Some(VoiceCommandConfig {
                command: vec!["ffmpeg".to_string(), "{file}".to_string()],
                env: HashMap::from([("A".to_string(), "1".to_string())]),
            }),
            transcribe: None,
        }),
        ..Default::default()
    };

    let overlay = Config {
        voice: Some(VoiceConfig {
            duration_seconds: Some(30),
            attach_audio: None,
            mime: None,
            language: Some("en".to_string()),
            record: Some(VoiceCommandConfig {
                command: Vec::new(),
                env: HashMap::from([("B".to_string(), "2".to_string())]),
            }),
            transcribe: Some(VoiceCommandConfig {
                command: vec!["whisper-cli".to_string(), "{file}".to_string()],
                env: HashMap::new(),
            }),
        }),
        ..Default::default()
    };

    base.merge(overlay);

    let voice = base.voice.expect("merged voice config should exist");
    assert_eq!(voice.duration_seconds, Some(30));
    assert_eq!(voice.attach_audio, Some(true));
    assert_eq!(voice.language.as_deref(), Some("en"));
    let record = voice.record.expect("record config should exist");
    assert_eq!(
        record.command,
        vec!["ffmpeg".to_string(), "{file}".to_string()]
    );
    assert_eq!(record.env.get("A").map(String::as_str), Some("1"));
    assert_eq!(record.env.get("B").map(String::as_str), Some("2"));
    assert_eq!(
        voice
            .transcribe
            .as_ref()
            .map(|command| command.command.clone()),
        Some(vec!["whisper-cli".to_string(), "{file}".to_string()])
    );
}

#[test]
fn multimodal_config_merge_deep_merges_limits_policy_and_voice() {
    let mut base = Config {
        multimodal: Some(MultimodalConfig {
            voice: Some(VoiceConfig {
                duration_seconds: Some(15),
                attach_audio: Some(true),
                mime: Some("audio/wav".to_string()),
                language: None,
                record: None,
                transcribe: None,
            }),
            limits: Some(MultimodalLimitsConfig {
                max_input_bytes: Some(2048),
                max_attachments_per_prompt: Some(2),
            }),
            policy: Some(MultimodalAttachmentPolicyConfig {
                allow_audio_input: Some(true),
                allow_image_input: Some(false),
                allow_file_input: Some(true),
            }),
        }),
        ..Default::default()
    };

    base.merge(Config {
        multimodal: Some(MultimodalConfig {
            voice: Some(VoiceConfig {
                duration_seconds: Some(25),
                attach_audio: None,
                mime: None,
                language: Some("en".to_string()),
                record: None,
                transcribe: None,
            }),
            limits: Some(MultimodalLimitsConfig {
                max_input_bytes: None,
                max_attachments_per_prompt: Some(8),
            }),
            policy: Some(MultimodalAttachmentPolicyConfig {
                allow_audio_input: None,
                allow_image_input: Some(true),
                allow_file_input: None,
            }),
        }),
        ..Default::default()
    });

    let multimodal = base
        .multimodal
        .expect("merged multimodal config should exist");
    let voice = multimodal.voice.expect("merged voice should exist");
    assert_eq!(voice.duration_seconds, Some(25));
    assert_eq!(voice.attach_audio, Some(true));
    assert_eq!(voice.language.as_deref(), Some("en"));

    let limits = multimodal.limits.expect("merged limits should exist");
    assert_eq!(limits.max_input_bytes, Some(2048));
    assert_eq!(limits.max_attachments_per_prompt, Some(8));

    let policy = multimodal.policy.expect("merged policy should exist");
    assert_eq!(policy.allow_audio_input, Some(true));
    assert_eq!(policy.allow_image_input, Some(true));
    assert_eq!(policy.allow_file_input, Some(true));
}

#[test]
fn experimental_config_merge_replaces_scalar_flags_and_primary_tools() {
    let mut base = Config {
        experimental: Some(ExperimentalConfig {
            disable_paste_summary: Some(true),
            open_telemetry: Some(false),
            primary_tools: vec!["read".to_string(), "edit".to_string()],
            mcp_timeout: Some(1000),
            ..Default::default()
        }),
        ..Default::default()
    };

    base.merge(Config {
        experimental: Some(ExperimentalConfig {
            batch_tool: Some(true),
            primary_tools: vec!["bash".to_string()],
            continue_loop_on_deny: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    });

    let experimental = base
        .experimental
        .expect("experimental config should exist after merge");
    assert_eq!(experimental.disable_paste_summary, Some(true));
    assert_eq!(experimental.open_telemetry, Some(false));
    assert_eq!(experimental.batch_tool, Some(true));
    assert_eq!(experimental.continue_loop_on_deny, Some(false));
    assert_eq!(experimental.mcp_timeout, Some(1000));
    assert_eq!(experimental.primary_tools, vec!["bash".to_string()]);
}
