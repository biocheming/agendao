use super::file_ops::{
    migrate_legacy_toml_config, parse_jsonc, resolve_file_references, substitute_env_vars,
};
use super::markdown_parser::{
    fallback_sanitize_yaml, parse_markdown_agent, parse_markdown_command,
    serde_yaml_frontmatter_to_json, split_frontmatter,
};
use super::workspace::{ConfigAuthority, WorkspaceMode};
use super::*;
use crate::{ShareMode, UiPreferencesConfig};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock error")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("failed to create test temp dir");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_parse_jsonc_simple() {
    let content = r#"{"model": "test-model-large"}"#;
    let config: Config = parse_jsonc(content).unwrap();
    assert_eq!(config.model, Some("test-model-large".to_string()));
}

#[test]
fn test_parse_jsonc_with_comments() {
    let content = r#"{
        // This is a comment
        "model": "test-model-large",
        /* Multi-line
            comment */
        "theme": "dark"
    }"#;
    let config: Config = parse_jsonc(content).unwrap();
    assert_eq!(config.model, Some("test-model-large".to_string()));
    assert_eq!(config.theme, Some("dark".to_string()));
}

#[test]
fn test_parse_jsonc_allows_trailing_comma_in_object() {
    let content = r#"{
        "model": "test-model-large",
        "theme": "dark",
    }"#;
    let config: Config = parse_jsonc(content).unwrap();
    assert_eq!(config.model, Some("test-model-large".to_string()));
    assert_eq!(config.theme, Some("dark".to_string()));
}

#[test]
fn test_parse_jsonc_allows_trailing_comma_in_array() {
    let content = r#"{
        "instructions": ["a.md", "b.md",],
        "plugin": [
            "p1",
            "p2",
        ],
    }"#;
    let config: Config = parse_jsonc(content).unwrap();
    assert_eq!(
        config.instructions,
        vec!["a.md".to_string(), "b.md".to_string()]
    );
    // Old array format is backward-compatible: converted to HashMap
    assert_eq!(config.plugin.len(), 2);
    assert!(config.plugin.contains_key("p1"));
    assert!(config.plugin.contains_key("p2"));
}

#[test]
fn test_parse_jsonc_preserves_comment_markers_inside_strings() {
    let content = r#"{
        "provider": {
            "openai": {
                "base_url": "https://example.com/path//not-comment",
                "api_key": "abc/*not-comment*/def"
            }
        }
    }"#;
    let config: Config = parse_jsonc(content).unwrap();
    let provider = config.provider.unwrap();
    let openai = provider.get("openai").unwrap();
    assert_eq!(
        openai.base_url.as_deref(),
        Some("https://example.com/path//not-comment")
    );
    assert_eq!(openai.api_key.as_deref(), Some("abc/*not-comment*/def"));
}

#[test]
fn test_config_merge() {
    let mut config1 = Config {
        model: Some("model1".to_string()),
        instructions: vec!["inst1".to_string()],
        ..Default::default()
    };

    let config2 = Config {
        model: Some("model2".to_string()),
        instructions: vec!["inst2".to_string()],
        ..Default::default()
    };

    config1.merge(config2);

    assert_eq!(config1.model, Some("model2".to_string()));
    assert_eq!(
        config1.instructions,
        vec!["inst1".to_string(), "inst2".to_string()]
    );
}

#[test]
fn test_load_project_finds_and_merges_parent_configs() {
    let temp = TestDir::new("agendao_config_findup");
    let root = temp.path.join("repo");
    let child = root.join("apps/web");
    fs::create_dir_all(&child).unwrap();

    fs::write(root.join("agendao.jsonc"), r#"{ "model": "parent-model" }"#).unwrap();
    fs::write(
        root.join("apps/agendao.jsonc"),
        r#"{ "theme": "dark", "instructions": ["parent.md"] }"#,
    )
    .unwrap();
    fs::write(
        child.join("agendao.jsonc"),
        r#"{ "instructions": ["child.md"] }"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    loader.load_project(&child).unwrap();
    let cfg = loader.config();

    assert_eq!(cfg.model.as_deref(), Some("parent-model"));
    assert_eq!(cfg.theme.as_deref(), Some("dark"));
    assert_eq!(
        cfg.instructions,
        vec!["parent.md".to_string(), "child.md".to_string()]
    );
}

#[test]
fn test_load_project_stops_at_git_root() {
    let temp = TestDir::new("agendao_config_gitroot");
    let outer = temp.path.join("outer");
    let repo = outer.join("repo");
    let child = repo.join("sub");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();

    fs::write(outer.join("agendao.jsonc"), r#"{ "model": "outer-model" }"#).unwrap();
    fs::write(repo.join("agendao.jsonc"), r#"{ "model": "repo-model" }"#).unwrap();
    fs::write(child.join("agendao.jsonc"), r#"{ "theme": "child-theme" }"#).unwrap();

    let mut loader = ConfigLoader::new();
    loader.load_project(&child).unwrap();
    let cfg = loader.config();

    assert_eq!(cfg.model.as_deref(), Some("repo-model"));
    assert_eq!(cfg.theme.as_deref(), Some("child-theme"));
}

#[test]
fn test_resolve_uses_current_directory_dot_agendao_for_isolated_workspace() {
    let temp = TestDir::new("agendao_config_dotdir");
    let root = temp.path.join("repo");
    let child = root.join("service");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join(".agendao")).unwrap();
    fs::create_dir_all(child.join(".agendao")).unwrap();

    fs::write(
        root.join(".agendao/agendao.jsonc"),
        r#"{ "default_agent": "build", "instructions": ["root.md"] }"#,
    )
    .unwrap();
    fs::write(
        child.join(".agendao/agendao.jsonc"),
        r#"{ "default_agent": "reviewer", "instructions": ["child.md"] }"#,
    )
    .unwrap();

    let resolved = ConfigAuthority::resolve(&child).unwrap();
    let cfg = resolved.config;

    assert_eq!(resolved.inputs.mode, WorkspaceMode::Isolated);
    assert_eq!(
        resolved.inputs.identity.config_dir,
        Some(child.join(".agendao"))
    );
    assert_eq!(cfg.default_agent.as_deref(), Some("reviewer"));
    assert_eq!(cfg.instructions, vec!["child.md".to_string()]);
}

#[test]
fn test_isolated_workspace_without_local_agendao_config_still_inherits_global_config() {
    let temp = TestDir::new("agendao_isolated_inherits_global_without_local_config");
    let root = temp.path.join("repo");
    let child = root.join("service");
    let config_home = temp.path.join("config-home");
    let global_agendao_dir = config_home.join("agendao");

    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(child.join(".agendao")).unwrap();
    fs::create_dir_all(&global_agendao_dir).unwrap();
    fs::write(
        global_agendao_dir.join("agendao.json"),
        r#"{ "model": "global-model", "theme": "light" }"#,
    )
    .unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let resolved = ConfigAuthority::resolve(&child).unwrap();
    let cfg = resolved.config;

    assert_eq!(resolved.inputs.mode, WorkspaceMode::Isolated);
    assert_eq!(cfg.model.as_deref(), Some("global-model"));
    assert_eq!(cfg.theme.as_deref(), Some("light"));

    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn test_isolated_workspace_with_local_agendao_config_cuts_off_global_config() {
    let temp = TestDir::new("agendao_isolated_local_config_cuts_global");
    let root = temp.path.join("repo");
    let child = root.join("service");
    let local_agendao_dir = child.join(".agendao");
    let config_home = temp.path.join("config-home");
    let global_agendao_dir = config_home.join("agendao");

    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(&local_agendao_dir).unwrap();
    fs::create_dir_all(&global_agendao_dir).unwrap();
    fs::write(
        global_agendao_dir.join("agendao.json"),
        r#"{ "model": "global-model", "theme": "light" }"#,
    )
    .unwrap();
    fs::write(
        local_agendao_dir.join("agendao.json"),
        r#"{ "default_agent": "reviewer" }"#,
    )
    .unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let resolved = ConfigAuthority::resolve(&child).unwrap();
    let cfg = resolved.config;

    assert_eq!(resolved.inputs.mode, WorkspaceMode::Isolated);
    assert_eq!(cfg.default_agent.as_deref(), Some("reviewer"));
    assert_eq!(cfg.model.as_deref(), None);
    assert_eq!(cfg.theme.as_deref(), None);

    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn test_workspace_mode_ignores_ancestor_dot_agendao() {
    let temp = TestDir::new("agendao_workspace_ancestor_dotdir");
    let root = temp.path.join("repo");
    let child = root.join("service");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join(".agendao")).unwrap();
    fs::create_dir_all(&child).unwrap();

    let resolved = ConfigAuthority::resolve_inputs(&child);

    assert_eq!(resolved.mode, WorkspaceMode::Shared);
    assert_eq!(resolved.identity.workspace_root, child);
    assert_eq!(resolved.identity.config_dir, None);
}

#[test]
fn test_load_project_supports_agendao_top_level_files() {
    let temp = TestDir::new("agendao_config_project_agendao_json");
    let root = temp.path.join("repo");
    let child = root.join("apps/web");
    fs::create_dir_all(&child).unwrap();

    fs::write(
        root.join("agendao.jsonc"),
        r#"{ "model": "parent-model", "instructions": ["root.md"] }"#,
    )
    .unwrap();
    fs::write(
        child.join("agendao.json"),
        r#"{ "theme": "dark", "instructions": ["child.md"] }"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    loader.load_project(&child).unwrap();
    let cfg = loader.config();

    assert_eq!(cfg.model.as_deref(), Some("parent-model"));
    assert_eq!(cfg.theme.as_deref(), Some("dark"));
    assert_eq!(
        cfg.instructions,
        vec!["root.md".to_string(), "child.md".to_string()]
    );
}

#[test]
fn test_load_project_ignores_opencode_files() {
    let temp = TestDir::new("agendao_config_project_opencode_ignored");
    let root = temp.path.join("repo");
    let child = root.join("apps/web");
    fs::create_dir_all(&child).unwrap();

    fs::write(
        root.join("opencode.jsonc"),
        r#"{ "model": "legacy-model", "instructions": ["legacy.md"] }"#,
    )
    .unwrap();
    fs::write(
        child.join("agendao.json"),
        r#"{ "theme": "dark", "instructions": ["current.md"] }"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    loader.load_project(&child).unwrap();
    let cfg = loader.config();

    assert_ne!(cfg.model.as_deref(), Some("legacy-model"));
    assert_eq!(cfg.theme.as_deref(), Some("dark"));
    assert_eq!(cfg.instructions, vec!["current.md".to_string()]);
}

#[test]
fn test_load_from_file_normalizes_scheduler_path_relative_to_config_file() {
    let temp = TestDir::new("agendao_config_scheduler_path");
    let root = temp.path.join("repo");
    let config_dir = root.join(".agendao");
    fs::create_dir_all(&config_dir).unwrap();

    fs::write(
        config_dir.join("agendao.jsonc"),
        r#"{ "schedulerPath": "scheduler/sisyphus.jsonc" }"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    loader
        .load_from_file(config_dir.join("agendao.jsonc"))
        .unwrap();

    assert_eq!(
        loader.config().scheduler_path.as_deref(),
        Some(
            config_dir
                .join("scheduler/sisyphus.jsonc")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn test_load_all_reads_plugins_from_plugin_paths() {
    let temp = TestDir::new("agendao_config_plugin_paths");
    let root = temp.path.join("repo");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join(".opencode/plugins")).unwrap();
    let plugin_path = root.join(".opencode/plugins/legacy-plugin.ts");
    fs::write(&plugin_path, "export default {};\n").unwrap();
    fs::create_dir_all(root.join(".agendao")).unwrap();
    fs::write(
        root.join(".agendao/agendao.json"),
        r#"{
  "plugin_paths": {
"legacy-opencode": ".opencode/plugins"
  }
}"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    let cfg = loader.load_all(&root).unwrap();

    // File plugins are keyed by file stem
    assert!(
        cfg.plugin.contains_key("legacy-plugin"),
        "expected legacy-plugin key in {:?}",
        cfg.plugin
    );
    let plugin_cfg = &cfg.plugin["legacy-plugin"];
    assert_eq!(plugin_cfg.plugin_type, "file");
    assert_eq!(
        plugin_cfg.path.as_deref(),
        Some(plugin_path.to_str().unwrap())
    );
}

#[test]
fn test_load_all_reads_plugins_from_default_agendao_plugin_dir() {
    let temp = TestDir::new("agendao_config_plugin_default_dir");
    let root = temp.path.join("repo");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join(".agendao/plugins")).unwrap();
    let plugin_path = root.join(".agendao/plugins/default-plugin.ts");
    fs::write(&plugin_path, "export default {};\n").unwrap();

    let mut loader = ConfigLoader::new();
    let cfg = loader.load_all(&root).unwrap();

    assert!(
        cfg.plugin.contains_key("default-plugin"),
        "expected default-plugin key in {:?}",
        cfg.plugin
    );
    let plugin_cfg = &cfg.plugin["default-plugin"];
    assert_eq!(plugin_cfg.plugin_type, "file");
    assert_eq!(
        plugin_cfg.path.as_deref(),
        Some(plugin_path.to_str().unwrap())
    );
}

#[test]
fn test_load_all_preserves_explicit_file_plugin() {
    let temp = TestDir::new("agendao_config_plugin_list_preserved");
    let root = temp.path.join("repo");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join("agendao.json"),
        r#"{
  "plugin": ["file:///tmp/should-not-be-loaded.ts"]
}"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    let cfg = loader.load_all(&root).unwrap();

    // Explicitly configured file plugins are preserved in config.
    // The plugin loader handles load failures at runtime.
    assert!(
        cfg.plugin.contains_key("should-not-be-loaded"),
        "expected explicit file plugin to be preserved, got {:?}",
        cfg.plugin
    );
    assert_eq!(cfg.plugin["should-not-be-loaded"].plugin_type, "file");
}

#[test]
fn test_load_all_prefers_higher_precedence_discovered_plugin() {
    let temp = TestDir::new("agendao_config_plugin_precedence");
    let root = temp.path.join("repo");
    let extra_root = root.join("team-plugins");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join(".agendao/plugins")).unwrap();
    fs::create_dir_all(&extra_root).unwrap();

    let workspace_plugin = root.join(".agendao/plugins/shared-plugin.ts");
    let override_plugin = extra_root.join("shared-plugin.ts");
    fs::write(
        &workspace_plugin,
        "export default { source: 'workspace' };\n",
    )
    .unwrap();
    fs::write(&override_plugin, "export default { source: 'team' };\n").unwrap();
    fs::write(
        root.join(".agendao/agendao.json"),
        r#"{
  "plugin_paths": {
    "team": "team-plugins"
  }
}"#,
    )
    .unwrap();

    let mut loader = ConfigLoader::new();
    let cfg = loader.load_all(&root).unwrap();

    assert_eq!(
        cfg.plugin["shared-plugin"].path.as_deref(),
        Some(override_plugin.to_str().unwrap())
    );
}

#[test]
fn test_discover_web_plugins_prefers_later_roots_and_supports_mjs() {
    let temp = TestDir::new("agendao_web_plugin_discovery");
    let low_root = temp.path.join("low");
    let high_root = temp.path.join("high");

    fs::create_dir_all(low_root.join("web/molstar")).unwrap();
    fs::create_dir_all(high_root.join("web/molstar")).unwrap();
    fs::create_dir_all(high_root.join("web")).unwrap();

    fs::write(
        low_root.join("web/molstar/index.js"),
        "export default function() {}\n",
    )
    .unwrap();
    fs::write(
        high_root.join("web/molstar/index.mjs"),
        "export default function() {}\n",
    )
    .unwrap();
    fs::write(
        high_root.join("web/plot.mjs"),
        "export default function() {}\n",
    )
    .unwrap();

    let plugins = discover_web_plugins(&[low_root.clone(), high_root.clone()]);

    let molstar = plugins
        .iter()
        .find(|plugin| plugin.name == "molstar")
        .expect("molstar plugin");
    assert_eq!(molstar.entry(), "index.mjs");
    assert_eq!(molstar.entry_path, high_root.join("web/molstar/index.mjs"));
    assert_eq!(molstar.serve_root, high_root.join("web/molstar"));

    let plot = plugins
        .iter()
        .find(|plugin| plugin.name == "plot")
        .expect("plot plugin");
    assert_eq!(plot.entry(), "plot.mjs");
    assert_eq!(plot.serve_root, high_root.join("web"));
}

#[test]
fn test_substitute_env_vars() {
    std::env::set_var("AGENDAO_TEST_VAR", "test_value");
    let input = r#"{"api_key": "{env:AGENDAO_TEST_VAR}"}"#;
    let result = substitute_env_vars(input);
    assert_eq!(result, r#"{"api_key": "test_value"}"#);
    std::env::remove_var("AGENDAO_TEST_VAR");
}

#[test]
fn test_substitute_env_vars_missing() {
    let input = r#"{"api_key": "{env:NONEXISTENT_VAR_12345}"}"#;
    let result = substitute_env_vars(input);
    assert_eq!(result, r#"{"api_key": ""}"#);
}

#[test]
fn test_resolve_file_references() {
    let temp = TestDir::new("agendao_file_ref");
    let secret_path = temp.path.join("secret.txt");
    fs::write(&secret_path, "my-secret-key").unwrap();

    let input = r#"{"api_key": "{file:secret.txt}"}"#.to_string();
    let result = resolve_file_references(&input, &temp.path).unwrap();
    assert_eq!(result, r#"{"api_key": "my-secret-key"}"#);
}

#[test]
fn test_resolve_file_references_skips_comments() {
    let temp = TestDir::new("agendao_file_ref_comment");
    let input = r#"{
        // "api_key": "{file:secret.txt}"
        "model": "test-model"
    }"#;
    let result = resolve_file_references(input, &temp.path).unwrap();
    assert!(result.contains("{file:secret.txt}"));
}

#[test]
fn test_resolve_file_references_absolute_path() {
    let temp = TestDir::new("agendao_file_ref_abs");
    let secret_path = temp.path.join("abs_secret.txt");
    fs::write(&secret_path, "absolute-secret").unwrap();

    let input = format!(r#"{{"api_key": "{{file:{}}}"}}"#, secret_path.display());
    let result = resolve_file_references(&input, &temp.path).unwrap();
    assert!(result.contains("absolute-secret"));
}

#[test]
fn test_update_config() {
    let temp = TestDir::new("agendao_update_config");

    let patch = Config {
        model: Some("test-model-large".to_string()),
        ..Default::default()
    };

    update_config(&temp.path, &patch).unwrap();

    let content = fs::read_to_string(temp.path.join(".agendao/agendao.json")).unwrap();
    let config: Config = serde_json::from_str(&content).unwrap();
    assert_eq!(config.model, Some("test-model-large".to_string()));
}

#[test]
fn test_update_config_prefers_highest_precedence_existing_project_file() {
    let temp = TestDir::new("agendao_update_config_precedence");
    let config_path = temp.path.join("agendao.jsonc");
    fs::create_dir_all(config_path.parent().expect("config dir")).unwrap();
    fs::write(&config_path, r#"{ "share": "manual" }"#).unwrap();

    let patch = Config {
        ui_preferences: Some(UiPreferencesConfig {
            show_thinking: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    update_config(&temp.path, &patch).unwrap();

    let content = fs::read_to_string(&config_path).unwrap();
    let config: Config = serde_json::from_str(&content).unwrap();
    let ui = config.ui_preferences.expect("ui preferences");
    assert!(matches!(config.share, Some(ShareMode::Manual)));
    assert_eq!(ui.show_thinking, Some(true));
}

#[test]
fn test_load_global_supports_agendao_json() {
    let temp = TestDir::new("agendao_global_json");
    let config_home = temp.path.join("config-home");
    let agendao_dir = config_home.join("agendao");
    fs::create_dir_all(&agendao_dir).unwrap();
    fs::write(
        agendao_dir.join("agendao.json"),
        r#"{ "model": "global-json-model", "theme": "light" }"#,
    )
    .unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let mut loader = ConfigLoader::new();
    loader.load_global().unwrap();
    let cfg = loader.config();

    assert_eq!(cfg.model.as_deref(), Some("global-json-model"));
    assert_eq!(cfg.theme.as_deref(), Some("light"));

    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn test_load_global_prefers_json_over_jsonc_when_both_exist() {
    let temp = TestDir::new("agendao_global_precedence");
    let config_home = temp.path.join("config-home");
    let agendao_dir = config_home.join("agendao");
    fs::create_dir_all(&agendao_dir).unwrap();
    fs::write(
        agendao_dir.join("agendao.jsonc"),
        r#"{ "model": "jsonc-model", "instructions": ["jsonc.md"] }"#,
    )
    .unwrap();
    fs::write(
        agendao_dir.join("agendao.json"),
        r#"{ "model": "json-model", "instructions": ["json.md"] }"#,
    )
    .unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let mut loader = ConfigLoader::new();
    loader.load_global().unwrap();
    let cfg = loader.config();

    assert_eq!(cfg.model.as_deref(), Some("json-model"));
    assert_eq!(
        cfg.instructions,
        vec!["jsonc.md".to_string(), "json.md".to_string()]
    );

    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn test_load_all_does_not_double_load_global_config_files() {
    let temp = TestDir::new("agendao_global_double_load");
    let project = temp.path.join("repo");
    let config_home = temp.path.join("config-home");
    let agendao_dir = config_home.join("agendao");
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::create_dir_all(&agendao_dir).unwrap();
    fs::write(
        agendao_dir.join("agendao.json"),
        r#"{ "instructions": ["global.md"] }"#,
    )
    .unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let mut loader = ConfigLoader::new();
    let cfg = loader.load_all(&project).unwrap();

    assert_eq!(cfg.instructions, vec!["global.md".to_string()]);

    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn test_load_all_ignores_ancestor_dot_agendao_but_keeps_global_config() {
    let temp = TestDir::new("agendao_ancestor_dotdir_ignored");
    let repo = temp.path.join("repo");
    let project_root = repo.join("project");
    let child = project_root.join("lit");
    let config_home = temp.path.join("config-home");
    let agendao_dir = config_home.join("agendao");

    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(project_root.join(".agendao")).unwrap();
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&agendao_dir).unwrap();

    fs::write(
        project_root.join(".agendao/agendao.json"),
        r#"{ "provider": { "sandbox": { "name": "Sandbox Only" } } }"#,
    )
    .unwrap();
    fs::write(
        agendao_dir.join("agendao.json"),
        r#"{ "provider": { "global": { "name": "Global Provider" } } }"#,
    )
    .unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let cfg = load_config(&child).unwrap();
    let providers = cfg.provider.expect("provider map");
    assert!(providers.contains_key("global"));
    assert!(!providers.contains_key("sandbox"));

    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn test_update_global_config_preserves_existing_json_file() {
    let temp = TestDir::new("agendao_update_global_json");
    let config_home = temp.path.join("config-home");
    let agendao_dir = config_home.join("agendao");
    fs::create_dir_all(&agendao_dir).unwrap();
    let config_path = agendao_dir.join("agendao.json");
    fs::write(&config_path, r#"{ "theme": "dark" }"#).unwrap();

    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let patch = Config {
        model: Some("global-model".to_string()),
        ..Default::default()
    };
    update_global_config(&patch).unwrap();

    let content = fs::read_to_string(&config_path).unwrap();
    let config: Config = serde_json::from_str(&content).unwrap();
    assert_eq!(config.theme.as_deref(), Some("dark"));
    assert_eq!(config.model.as_deref(), Some("global-model"));
    assert!(!agendao_dir.join("agendao.jsonc").exists());

    std::env::remove_var("XDG_CONFIG_HOME");
}

// ── YAML frontmatter parsing tests ──────────────────────────────

#[test]
fn test_split_frontmatter_basic() {
    let content = "---\nname: test\ndescription: hello\n---\nBody content here.";
    let (fm, body) = split_frontmatter(content);
    assert!(fm.is_some());
    let fm = fm.unwrap();
    assert!(fm.contains("name: test"));
    assert!(fm.contains("description: hello"));
    assert!(body.contains("Body content here."));
}

#[test]
fn test_split_frontmatter_no_frontmatter() {
    let content = "Just a regular markdown file.";
    let (fm, body) = split_frontmatter(content);
    assert!(fm.is_none());
    assert_eq!(body, content);
}

#[test]
fn test_yaml_frontmatter_flat_key_values() {
    let yaml = "name: reviewer\ndescription: Review code\nmodel: test-model-large";
    let json = serde_yaml_frontmatter_to_json(yaml);
    assert_eq!(json["name"], "reviewer");
    assert_eq!(json["description"], "Review code");
    assert_eq!(json["model"], "test-model-large");
}

#[test]
fn test_yaml_frontmatter_booleans_and_numbers() {
    let yaml = "disable: true\nhidden: false\nsteps: 100\ntemperature: 0.7";
    let json = serde_yaml_frontmatter_to_json(yaml);
    assert_eq!(json["disable"], true);
    assert_eq!(json["hidden"], false);
    assert_eq!(json["steps"], 100);
    assert_eq!(json["temperature"], 0.7);
}

#[test]
fn test_yaml_frontmatter_inline_list() {
    let yaml = "tools: [bash, read, write]";
    let json = serde_yaml_frontmatter_to_json(yaml);
    let tools = json["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[0], "bash");
    assert_eq!(tools[1], "read");
    assert_eq!(tools[2], "write");
}

#[test]
fn test_yaml_frontmatter_dash_list() {
    let yaml = "tools:\n  - bash\n  - read\n  - write";
    let json = serde_yaml_frontmatter_to_json(yaml);
    let tools = json["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[0], "bash");
    assert_eq!(tools[1], "read");
    assert_eq!(tools[2], "write");
}

#[test]
fn test_yaml_frontmatter_nested_object() {
    let yaml = "tools:\n  bash: true\n  read: false";
    let json = serde_yaml_frontmatter_to_json(yaml);
    let tools = json["tools"].as_object().unwrap();
    assert_eq!(tools["bash"], true);
    assert_eq!(tools["read"], false);
}

#[test]
fn test_yaml_frontmatter_block_scalar_literal() {
    let yaml = "prompt: |\n  Line one\n  Line two";
    let json = serde_yaml_frontmatter_to_json(yaml);
    let prompt = json["prompt"].as_str().unwrap();
    assert!(prompt.contains("Line one"));
    assert!(prompt.contains("Line two"));
}

#[test]
fn test_yaml_frontmatter_block_scalar_strip() {
    let yaml = "prompt: |-\n  Line one\n  Line two";
    let json = serde_yaml_frontmatter_to_json(yaml);
    let prompt = json["prompt"].as_str().unwrap();
    assert!(prompt.contains("Line one"));
    assert!(!prompt.ends_with('\n'));
}

#[test]
fn test_yaml_frontmatter_comments_skipped() {
    let yaml = "# This is a comment\nname: test\n# Another comment\ndescription: hello";
    let json = serde_yaml_frontmatter_to_json(yaml);
    assert_eq!(json["name"], "test");
    assert_eq!(json["description"], "hello");
}

#[test]
fn test_yaml_frontmatter_quoted_values() {
    let yaml = "name: \"quoted value\"\ndescription: 'single quoted'";
    let json = serde_yaml_frontmatter_to_json(yaml);
    assert_eq!(json["name"], "quoted value");
    assert_eq!(json["description"], "single quoted");
}

#[test]
fn test_fallback_sanitize_yaml_colon_in_value() {
    let yaml = "description: Use model: test-model for tasks\nname: test";
    let sanitized = fallback_sanitize_yaml(yaml);
    assert!(sanitized.contains("description: |-"));
    assert!(sanitized.contains("  Use model: test-model for tasks"));
    assert!(sanitized.contains("name: test"));
}

#[test]
fn test_fallback_sanitize_yaml_preserves_quoted() {
    let yaml = "description: \"already: quoted\"\nname: test";
    let sanitized = fallback_sanitize_yaml(yaml);
    // Quoted values should not be converted to block scalars
    assert!(sanitized.contains("description: \"already: quoted\""));
}

#[test]
fn test_fallback_sanitize_yaml_preserves_block_scalar() {
    let yaml = "description: |\n  block content\nname: test";
    let sanitized = fallback_sanitize_yaml(yaml);
    assert!(sanitized.contains("description: |"));
}

#[test]
fn test_yaml_frontmatter_value_with_colon_via_fallback() {
    // This YAML has a value with a colon, which would confuse naive parsers.
    // The fallback sanitization should handle it.
    let yaml = "description: Use model: test-model for tasks\nname: test";
    let json = serde_yaml_frontmatter_to_json(yaml);
    // After fallback, description should be preserved
    assert_eq!(json["name"], "test");
    let desc = json["description"].as_str().unwrap();
    assert!(desc.contains("model: test-model"));
}

#[test]
fn test_yaml_frontmatter_inline_map() {
    let yaml = "options: {verbose: true, timeout: 30}";
    let json = serde_yaml_frontmatter_to_json(yaml);
    let options = json["options"].as_object().unwrap();
    assert_eq!(options["verbose"], true);
    assert_eq!(options["timeout"], 30);
}

#[test]
fn test_yaml_frontmatter_empty_value() {
    let yaml = "name:\ndescription: hello";
    let json = serde_yaml_frontmatter_to_json(yaml);
    assert!(json["name"].is_null());
    assert_eq!(json["description"], "hello");
}

#[test]
fn test_parse_markdown_agent_with_frontmatter() {
    let temp = TestDir::new("agendao_md_agent");
    let agent_dir = temp.path.join("agents");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("reviewer.md"),
        "---\ndescription: Reviews code changes\nmode: subagent\nmodel: test-model-large\n---\n\nYou are a code reviewer.\n",
    )
    .unwrap();

    let result = parse_markdown_agent(&agent_dir.join("reviewer.md"), &temp.path);
    assert!(result.is_some());
    let (name, config) = result.unwrap();
    assert_eq!(name, "reviewer");
    assert_eq!(config.description.as_deref(), Some("Reviews code changes"));
    assert_eq!(config.model.as_deref(), Some("test-model-large"));
    assert!(config.prompt.unwrap().contains("You are a code reviewer."));
}

#[test]
fn test_parse_markdown_command_with_frontmatter() {
    let temp = TestDir::new("agendao_md_cmd");
    let cmd_dir = temp.path.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(
        cmd_dir.join("review.md"),
        "---\ndescription: Run a code review\nagent: reviewer\n---\n\nPlease review the changes.\n",
    )
    .unwrap();

    let result = parse_markdown_command(&cmd_dir.join("review.md"), &temp.path);
    assert!(result.is_some());
    let (name, config) = result.unwrap();
    assert_eq!(name, "review");
    assert_eq!(config.description.as_deref(), Some("Run a code review"));
    assert_eq!(config.agent.as_deref(), Some("reviewer"));
    assert!(config
        .template
        .unwrap()
        .contains("Please review the changes."));
}

#[test]
fn test_parse_markdown_agent_with_tools_map() {
    let temp = TestDir::new("agendao_md_agent_tools");
    let agent_dir = temp.path.join("agents");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("safe.md"),
        "---\ndescription: Safe agent\ntools:\n  bash: false\n  read: true\n---\n\nSafe prompt.\n",
    )
    .unwrap();

    let result = parse_markdown_agent(&agent_dir.join("safe.md"), &temp.path);
    assert!(result.is_some());
    let (_name, config) = result.unwrap();
    assert_eq!(config.description.as_deref(), Some("Safe agent"));
    let tools = config.tools.unwrap();
    assert_eq!(tools.get("bash"), Some(&false));
    assert_eq!(tools.get("read"), Some(&true));
}

#[test]
fn test_parse_markdown_agent_colon_in_description_fallback() {
    let temp = TestDir::new("agendao_md_agent_colon");
    let agent_dir = temp.path.join("agents");
    fs::create_dir_all(&agent_dir).unwrap();
    // Description contains a colon -- this is the case the fallback handles
    fs::write(
        agent_dir.join("tricky.md"),
        "---\ndescription: Use model: test-model for tasks\nmode: primary\n---\n\nTricky prompt.\n",
    )
    .unwrap();

    let result = parse_markdown_agent(&agent_dir.join("tricky.md"), &temp.path);
    assert!(result.is_some());
    let (_name, config) = result.unwrap();
    let desc = config.description.unwrap();
    assert!(desc.contains("model: test-model"));
}

#[test]
fn legacy_toml_config_migrates_to_agendao_json() {
    let temp = TestDir::new("agendao_legacy_toml");
    let config_dir = temp.path.join("agendao");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config"),
        r#"
provider = "ethnopic"
model = "test-model-v2"
theme = "dark"
"#,
    )
    .unwrap();

    let mut config = Config::default();
    let migrated = migrate_legacy_toml_config(&config_dir, &mut config);
    assert!(migrated.is_some());
    assert_eq!(config.model.as_deref(), Some("ethnopic/test-model-v2"));
    assert_eq!(config.theme.as_deref(), Some("dark"));
    assert_eq!(
        config.schema.as_deref(),
        Some("https://opencode.ai/config.json") //no agendao.ai domain name now
    );

    let json_path = config_dir.join("agendao.json");
    assert!(json_path.exists());
    assert!(!config_dir.join("config").exists());

    let content = fs::read_to_string(json_path).unwrap();
    let written: Config = serde_json::from_str(&content).unwrap();
    assert_eq!(written.model.as_deref(), Some("ethnopic/test-model-v2"));
    assert_eq!(written.theme.as_deref(), Some("dark"));
}
