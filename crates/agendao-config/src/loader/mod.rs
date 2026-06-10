mod discovery;
mod file_ops;
mod markdown_parser;
mod transforms;
mod workspace;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use crate::schema::PluginConfig;
use crate::{Config, ExternalToolCatalogFile, ExternalToolConfig, ExternalToolExecutionKind};
use agendao_types::ToolCatalogMetadata;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) use discovery::resolve_configured_path;
pub use transforms::{
    deduplicate_plugins, get_plugin_name, load_config, update_config, update_global_config,
    write_config,
};
pub use workspace::{
    ConfigAuthority, ResolvedConfig, ResolvedConfigInputs, WorkspaceIdentity, WorkspaceMode,
};

use discovery::{
    collect_agendao_directories, collect_plugin_roots, detect_worktree_stop, find_up,
    get_managed_config_dir, load_agents_from_dir, load_commands_from_dir, load_modes_from_dir,
    load_plugins_from_path, normalize_existing_path,
};
pub use discovery::{
    collect_plugin_roots as get_plugin_roots, discover_web_plugins, WebPluginInfo,
};
use file_ops::{
    get_global_config_paths, migrate_legacy_toml_config, parse_external_tool_catalog_jsonc,
    parse_jsonc, resolve_file_references, substitute_env_vars,
};
use transforms::{apply_post_load_transforms, merge_agent_config};

pub struct ConfigLoader {
    config: Config,
    config_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExternalToolCatalog {
    pub source_path: PathBuf,
    pub tools: HashMap<String, ExternalToolConfig>,
}

const PROJECT_CONFIG_TARGETS: &[&str] = &["agendao.jsonc", "agendao.json"];

const DIRECTORY_CONFIG_FILES: &[&str] = &["agendao.jsonc", "agendao.json"];

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

    pub fn load_external_tool_catalogs(&self) -> Result<Vec<ResolvedExternalToolCatalog>> {
        let mut catalogs = Vec::new();
        let mut seen_imports = HashSet::new();

        for config_path in &self.config_paths {
            let Some(base_dir) = config_path.parent() else {
                continue;
            };

            for import in &self.config.tool_imports {
                let resolved = normalize_path_lexically(resolve_configured_path(base_dir, import));
                if !seen_imports.insert(resolved.clone()) {
                    continue;
                }
                if !resolved.exists() || resolved.is_dir() {
                    continue;
                }
                let content = fs::read_to_string(&resolved).with_context(|| {
                    format!(
                        "Failed to read external tool catalog: {}",
                        resolved.display()
                    )
                })?;
                let catalog = parse_external_tool_catalog_jsonc(&content).with_context(|| {
                    format!(
                        "Failed to parse external tool catalog file: {}",
                        resolved.display()
                    )
                })?;
                let catalog_base_dir = resolved.parent().unwrap_or(base_dir).to_path_buf();
                let normalized = normalize_external_tool_catalog_paths(catalog, &catalog_base_dir);
                validate_external_tool_catalog(&normalized, &resolved)?;
                catalogs.push(ResolvedExternalToolCatalog {
                    source_path: resolved,
                    tools: normalized.tools,
                });
            }
        }

        Ok(catalogs)
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
        if let Ok(config_path) = env::var("AGENDAO_CONFIG") {
            self.load_from_file(&config_path)?;
        }

        Ok(())
    }

    /// Load inline config content from AGENDAO_CONFIG_CONTENT env var.
    /// Per TS parity, this is applied after project config but before managed config.
    pub fn load_from_env_content(&mut self) -> Result<()> {
        if let Ok(config_content) = env::var("AGENDAO_CONFIG_CONTENT") {
            self.load_from_str(&config_content)?;
        }

        Ok(())
    }

    /// Loads all config sources synchronously (without remote wellknown).
    /// Merge order (TS parity):
    /// 1. Global config (~/.config/agendao/agendao.json{,c})
    /// 2. Custom config (AGENDAO_CONFIG)
    /// 3. Project config (agendao json{,c})
    /// 4. .agendao directories (agents, commands, modes, config)
    /// 5. Inline config (AGENDAO_CONFIG_CONTENT)
    /// 6. Managed config directory (enterprise, highest priority)
    ///
    /// Then: plugin_paths/default plugin dir scan, legacy migrations, flag overrides, plugin dedup
    pub fn load_all<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<Config> {
        let project_dir = project_dir.as_ref();

        self.load_global()?;
        self.load_from_env()?;
        self.load_project(project_dir)?;

        // Scan .agendao directories
        let directories = collect_agendao_directories(project_dir);
        let global_config_dirs: HashSet<PathBuf> = get_global_config_paths()
            .into_iter()
            .filter_map(|path| path.parent().map(normalize_existing_path))
            .collect();
        for dir in &directories {
            // Global config files are already loaded by load_global(); this pass only
            // adds markdown sidecars and project-local .agendao config files.
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
        // - agendao default plugin directories
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
    for tool_import in &mut config.tool_imports {
        let trimmed = tool_import.trim();
        if trimmed.is_empty() {
            continue;
        }
        *tool_import = resolve_configured_path(base_dir, trimmed)
            .to_string_lossy()
            .to_string();
    }
}

fn normalize_external_tool_catalog_paths(
    mut catalog: ExternalToolCatalogFile,
    base_dir: &Path,
) -> ExternalToolCatalogFile {
    for config in catalog.tools.values_mut() {
        if let Some(source) = config.source.as_mut() {
            if let Some(path) = source
                .path
                .as_deref()
                .map(str::trim)
                .filter(|v: &&str| !v.is_empty())
            {
                source.path = Some(
                    normalize_path_lexically(resolve_configured_path(base_dir, path))
                        .to_string_lossy()
                        .to_string(),
                );
            }
            if let Some(manifest) = source
                .manifest
                .as_deref()
                .map(str::trim)
                .filter(|v: &&str| !v.is_empty())
            {
                source.manifest = Some(
                    normalize_path_lexically(resolve_configured_path(base_dir, manifest))
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }

        if let Some(execution) = config.execution.as_mut() {
            if let Some(entry) = execution
                .entry
                .as_deref()
                .map(str::trim)
                .filter(|v: &&str| !v.is_empty())
            {
                execution.entry = Some(
                    normalize_path_lexically(resolve_configured_path(base_dir, entry))
                        .to_string_lossy()
                        .to_string(),
                );
            }
            if let Some(arguments_schema_ref) = execution
                .arguments_schema_ref
                .as_deref()
                .map(str::trim)
                .filter(|v: &&str| !v.is_empty())
            {
                execution.arguments_schema_ref = Some(
                    normalize_path_lexically(resolve_configured_path(
                        base_dir,
                        arguments_schema_ref,
                    ))
                    .to_string_lossy()
                    .to_string(),
                );
            }
        }

        if let Some(catalog_meta) = config.catalog.as_mut() {
            if catalog_meta.domain.is_none() || catalog_meta.family.is_none() {
                if let Some(source_path) = config
                    .source
                    .as_ref()
                    .and_then(|source| source.path.as_deref())
                {
                    apply_catalog_path_defaults(catalog_meta, Path::new(source_path));
                }
            }
        }
    }
    catalog
}

fn validate_external_tool_catalog(
    catalog: &ExternalToolCatalogFile,
    source_path: &Path,
) -> Result<()> {
    let mut names = HashSet::new();
    for (tool_name, config) in &catalog.tools {
        if !names.insert(tool_name) {
            anyhow::bail!(
                "duplicate external tool name `{}` in {}",
                tool_name,
                source_path.display()
            );
        }

        if let Some(execution) = config.execution.as_ref() {
            match execution.kind {
                ExternalToolExecutionKind::ScriptRunner => {
                    let entry = execution
                        .entry
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    if entry.is_none() {
                        anyhow::bail!(
                            "external tool `{}` in {} declares executable mode but is missing execution.entry",
                            tool_name,
                            source_path.display()
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

fn apply_catalog_path_defaults(catalog: &mut ToolCatalogMetadata, path: &Path) {
    let components = path
        .parent()
        .into_iter()
        .flat_map(|parent| parent.components())
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    if catalog.domain.is_none() {
        if let Some(idx) = components.iter().rposition(|segment| segment == "tools") {
            catalog.domain = components.get(idx + 1).cloned();
            catalog.family = catalog
                .family
                .clone()
                .or_else(|| components.get(idx + 2).cloned());
            catalog.subfamily = catalog
                .subfamily
                .clone()
                .or_else(|| components.get(idx + 3).cloned());
        }
    }
}

fn normalize_path_lexically(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve and load external tool catalog files for a project/workspace using
/// the same configuration authority path as the main config loader.
pub fn load_external_tool_catalogs_for_project<P: AsRef<Path>>(
    project_dir: P,
) -> Result<Vec<ResolvedExternalToolCatalog>> {
    let inputs = ConfigAuthority::resolve_inputs(project_dir.as_ref());
    let mut loader = ConfigLoader::new();
    loader.load_with_inputs(&inputs)?;
    loader.load_external_tool_catalogs()
}
