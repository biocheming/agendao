mod discovery;
mod file_ops;
mod markdown_parser;
mod transforms;
mod workspace;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use crate::schema::PluginConfig;
use crate::Config;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) use discovery::resolve_configured_path;
pub use transforms::{
    deduplicate_plugins, get_plugin_name, load_config, load_config_with_remote, update_config,
    update_global_config, write_config,
};
pub use workspace::{
    ConfigAuthority, ResolvedConfig, ResolvedConfigInputs, WorkspaceIdentity, WorkspaceMode,
};

use discovery::{
    collect_plugin_roots, collect_rocode_directories, detect_worktree_stop, find_up,
    get_managed_config_dir, load_agents_from_dir, load_commands_from_dir, load_modes_from_dir,
    load_plugins_from_path, normalize_existing_path,
};
pub use discovery::{
    collect_plugin_roots as get_plugin_roots, discover_web_plugins, WebPluginInfo,
};
use file_ops::{
    get_global_config_paths, migrate_legacy_toml_config, parse_jsonc, resolve_file_references,
    substitute_env_vars,
};
use transforms::{apply_post_load_transforms, merge_agent_config};

pub struct ConfigLoader {
    config: Config,
    config_paths: Vec<PathBuf>,
}

const PROJECT_CONFIG_TARGETS: &[&str] = &["rocode.jsonc", "rocode.json"];

const DIRECTORY_CONFIG_FILES: &[&str] = &["rocode.jsonc", "rocode.json"];

impl ConfigLoader {
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            config_paths: Vec::new(),
        }
    }

    pub fn load_from_str(&mut self, content: &str) -> Result<()> {
        let config: Config =
            parse_jsonc(content).with_context(|| "Failed to parse config content")?;
        self.config.merge(config);
        Ok(())
    }

    pub fn into_config(self) -> Config {
        self.config
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        // Apply {env:VAR} substitution
        let content = substitute_env_vars(&content);

        // Apply {file:path} substitution
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let content = resolve_file_references(&content, base_dir)
            .with_context(|| format!("Failed to resolve file references in: {:?}", path))?;

        let mut config: Config = parse_jsonc(&content)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;
        normalize_config_paths(&mut config, base_dir);

        self.config.merge(config);
        self.config_paths.push(path.to_path_buf());
        Ok(())
    }

    pub fn load_global(&mut self) -> Result<()> {
        let global_config_paths = get_global_config_paths();

        for global_config_path in &global_config_paths {
            self.load_from_file(global_config_path)?;
        }

        if let Some(global_config_dir) = global_config_paths.first().and_then(|path| path.parent())
        {
            if let Some(migrated_path) =
                migrate_legacy_toml_config(global_config_dir, &mut self.config)
            {
                if !self.config_paths.contains(&migrated_path) {
                    self.config_paths.push(migrated_path);
                }
            }
        }

        Ok(())
    }

    pub fn load_project<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<()> {
        let input = project_dir.as_ref();
        let start_dir = if input.is_dir() {
            input.to_path_buf()
        } else {
            input
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| input.to_path_buf())
        };
        let start_dir = normalize_existing_path(&start_dir);
        let stop_dir = detect_worktree_stop(&start_dir);

        // TS parity: findUp per target, then load from ancestor -> descendant.
        for target in PROJECT_CONFIG_TARGETS {
            let found = find_up(target, &start_dir, &stop_dir);
            for path in found.into_iter().rev() {
                self.load_from_file(path)?;
            }
        }

        Ok(())
    }

    pub fn load_from_env(&mut self) -> Result<()> {
        if let Ok(config_path) = env::var("ROCODE_CONFIG") {
            self.load_from_file(&config_path)?;
        }

        Ok(())
    }

    /// Load inline config content from ROCODE_CONFIG_CONTENT env var.
    /// Per TS parity, this is applied after project config but before managed config.
    pub fn load_from_env_content(&mut self) -> Result<()> {
        if let Ok(config_content) = env::var("ROCODE_CONFIG_CONTENT") {
            self.load_from_str(&config_content)?;
        }

        Ok(())
    }

    /// Loads all config sources synchronously (without remote wellknown).
    /// Merge order (TS parity):
    /// 1. Global config (~/.config/rocode/rocode.json{,c})
    /// 2. Custom config (ROCODE_CONFIG)
    /// 3. Project config (rocode json{,c})
    /// 4. .rocode directories (agents, commands, modes, config)
    /// 5. Inline config (ROCODE_CONFIG_CONTENT)
    /// 6. Managed config directory (enterprise, highest priority)
    ///
    /// Then: plugin_paths/default plugin dir scan, legacy migrations, flag overrides, plugin dedup
    pub fn load_all<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<Config> {
        let project_dir = project_dir.as_ref();

        self.load_global()?;
        self.load_from_env()?;
        self.load_project(project_dir)?;

        // Scan .rocode directories
        let directories = collect_rocode_directories(project_dir);
        let global_config_dirs: HashSet<PathBuf> = get_global_config_paths()
            .into_iter()
            .filter_map(|path| path.parent().map(normalize_existing_path))
            .collect();
        for dir in &directories {
            // Global config files are already loaded by load_global(); this pass only
            // adds markdown sidecars and project-local .rocode config files.
            if !global_config_dirs.contains(&normalize_existing_path(dir)) {
                for file_name in DIRECTORY_CONFIG_FILES {
                    let path = dir.join(file_name);
                    self.load_from_file(&path)?;
                }
            }

            // Load commands, agents, modes from markdown files
            let commands = load_commands_from_dir(dir);
            if !commands.is_empty() {
                let mut cmd_map = self.config.command.take().unwrap_or_default();
                for (name, cmd) in commands {
                    cmd_map.insert(name, cmd);
                }
                self.config.command = Some(cmd_map);
            }

            let agents = load_agents_from_dir(dir);
            if !agents.is_empty() {
                let mut agent_configs = self.config.agent.take().unwrap_or_default();
                for (name, agent) in agents {
                    if let Some(existing) = agent_configs.entries.get_mut(&name) {
                        // Deep merge
                        merge_agent_config(existing, agent);
                    } else {
                        agent_configs.entries.insert(name, agent);
                    }
                }
                self.config.agent = Some(agent_configs);
            }

            let modes = load_modes_from_dir(dir);
            if !modes.is_empty() {
                let mut agent_configs = self.config.agent.take().unwrap_or_default();
                for (name, agent) in modes {
                    if let Some(existing) = agent_configs.entries.get_mut(&name) {
                        merge_agent_config(existing, agent);
                    } else {
                        agent_configs.entries.insert(name, agent);
                    }
                }
                self.config.agent = Some(agent_configs);
            }
        }

        // Plugin discovery is path-driven:
        // - rocode default plugin directories
        // - configured `plugin_paths`
        // Auto-discovered file plugins are merged; explicitly configured plugins
        // (from config files) are preserved via entry().or_insert().
        let mut discovered_plugins = std::collections::HashMap::new();
        for dir in collect_plugin_roots(project_dir, &self.config.plugin_paths) {
            let plugins = load_plugins_from_path(&dir);
            for plugin_spec in plugins {
                let (key, config) = PluginConfig::from_file_spec(&plugin_spec);
                discovered_plugins.insert(key, config);
            }
        }
        for (key, config) in discovered_plugins {
            self.config.plugin.entry(key).or_insert(config);
        }

        // Inline config content overrides all non-managed config sources
        self.load_from_env_content()?;

        // Load managed config (enterprise, highest priority)
        self.load_managed_config()?;

        // Apply legacy migrations and flag overrides
        apply_post_load_transforms(&mut self.config);

        Ok(self.config.clone())
    }

    /// Loads all config sources including remote `.well-known/opencode` endpoints.
    /// Merge order (low -> high precedence):
    /// 1. Remote .well-known/opencode (org defaults) -- lowest priority
    /// 2. Global config (~/.config/rocode/rocode.json{,c})
    /// 3. Custom config (ROCODE_CONFIG)
    /// 4. Project config (rocode json{,c})
    /// 5. .rocode directories + plugin_paths plugin dirs
    /// 6. Inline config (ROCODE_CONFIG_CONTENT)
    /// 7. Managed config directory (enterprise, highest priority)
    pub async fn load_all_with_remote<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<Config> {
        let wellknown_config = crate::wellknown::load_wellknown().await;
        self.config.merge(wellknown_config);

        // Delegate to load_all which handles everything else
        self.load_all(project_dir)
    }

    /// Load managed config files from enterprise directory (highest priority).
    fn load_managed_config(&mut self) -> Result<()> {
        let managed_dir = get_managed_config_dir();
        if managed_dir.exists() {
            for file_name in DIRECTORY_CONFIG_FILES {
                let path = managed_dir.join(file_name);
                self.load_from_file(&path)?;
            }
        }
        Ok(())
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_paths(&self) -> &[PathBuf] {
        &self.config_paths
    }
}

fn normalize_config_paths(config: &mut Config, base_dir: &Path) {
    if let Some(path) = config.scheduler_path.as_deref().map(str::trim) {
        if !path.is_empty() {
            config.scheduler_path = Some(
                resolve_configured_path(base_dir, path)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    if let Some(path) = config.task_category_path.as_deref().map(str::trim) {
        if !path.is_empty() {
            config.task_category_path = Some(
                resolve_configured_path(base_dir, path)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}
