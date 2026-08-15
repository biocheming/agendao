use crate::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::discovery::{load_plugins_from_path, normalize_existing_path};
use super::file_ops::get_global_config_paths;
use super::transforms::apply_post_load_transforms;
use super::{ConfigLoader, DIRECTORY_CONFIG_FILES};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    #[default]
    Shared,
    Isolated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceIdentity {
    pub requested_dir: PathBuf,
    pub workspace_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<PathBuf>,
    pub workspace_key: String,
}

impl WorkspaceIdentity {
    pub fn fallback(project_dir: &Path) -> Self {
        let requested_dir = normalize_existing_path(project_dir);
        let workspace_root = requested_dir.clone();
        let workspace_key = workspace_root.to_string_lossy().to_string();
        Self {
            requested_dir,
            workspace_root,
            config_dir: None,
            workspace_key,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedConfigInputs {
    pub identity: WorkspaceIdentity,
    pub mode: WorkspaceMode,
    #[serde(default)]
    pub config_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub config: Config,
    pub inputs: ResolvedConfigInputs,
}

#[derive(Debug, Default, Clone)]
pub struct ConfigAuthority;

impl ConfigAuthority {
    pub fn resolve<P: AsRef<Path>>(project_dir: P) -> Result<ResolvedConfig> {
        let inputs = Self::resolve_inputs(project_dir.as_ref());
        let mut loader = ConfigLoader::new();
        let config = loader.load_with_inputs(&inputs)?;
        Ok(ResolvedConfig {
            config,
            inputs: ResolvedConfigInputs {
                config_paths: loader.config_paths().to_vec(),
                ..inputs
            },
        })
    }

    pub fn resolve_inputs(project_dir: &Path) -> ResolvedConfigInputs {
        let requested_dir = if project_dir.is_dir() {
            normalize_existing_path(project_dir)
        } else {
            project_dir
                .parent()
                .map(normalize_existing_path)
                .unwrap_or_else(|| normalize_existing_path(project_dir))
        };
        let isolated_candidate = requested_dir.join(".agendao");
        let isolated_dir = if fs::metadata(&isolated_candidate)
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            Some(isolated_candidate)
        } else {
            None
        };
        let mode = if isolated_dir.is_some() {
            WorkspaceMode::Isolated
        } else {
            WorkspaceMode::Shared
        };
        let workspace_root = isolated_dir
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| requested_dir.clone());
        let workspace_key = workspace_root.to_string_lossy().to_string();

        ResolvedConfigInputs {
            identity: WorkspaceIdentity {
                requested_dir,
                workspace_root,
                config_dir: isolated_dir,
                workspace_key,
            },
            mode,
            config_paths: Vec::new(),
        }
    }
}

impl ConfigLoader {
    pub(super) fn load_with_inputs(&mut self, inputs: &ResolvedConfigInputs) -> Result<Config> {
        match inputs.mode {
            WorkspaceMode::Shared => self.load_all(&inputs.identity.workspace_root),
            WorkspaceMode::Isolated => self.load_isolated(&inputs.identity),
        }
    }

    fn load_isolated(&mut self, identity: &WorkspaceIdentity) -> Result<Config> {
        let Some(config_dir) = identity.config_dir.as_ref() else {
            return self.load_all(&identity.workspace_root);
        };

        // Isolated controls workspace-scoped data authority; it does not create
        // a second configuration root. Global config remains the base layer and
        // the workspace-local config only overrides fields it explicitly owns.
        self.load_global()?;
        for file_name in DIRECTORY_CONFIG_FILES {
            let path = config_dir.join(file_name);
            self.load_from_file(&path)?;
        }

        let mut loaded_sidecar_dirs = std::collections::HashSet::new();
        for global_dir in get_global_config_paths()
            .into_iter()
            .filter_map(|path| path.parent().map(normalize_existing_path))
        {
            if loaded_sidecar_dirs.insert(global_dir.clone()) {
                self.load_directory_sidecars(&global_dir)?;
            }
        }
        let local_dir = normalize_existing_path(config_dir);
        if loaded_sidecar_dirs.insert(local_dir.clone()) {
            self.load_directory_sidecars(&local_dir)?;
        }
        let mut discovered_plugins = std::collections::HashMap::new();
        for plugin_dir in [
            config_dir.join("plugins"),
            identity.workspace_root.join(".agendao/plugins"),
        ] {
            let plugins = load_plugins_from_path(&plugin_dir);
            for plugin_spec in plugins {
                let (key, config) = crate::schema::PluginConfig::from_file_spec(&plugin_spec);
                discovered_plugins.insert(key, config);
            }
        }
        for (key, config) in discovered_plugins {
            self.config.plugin.entry(key).or_insert(config);
        }

        apply_post_load_transforms(&mut self.config);
        self.config_paths.extend(
            DIRECTORY_CONFIG_FILES
                .iter()
                .map(|name| config_dir.join(name))
                .filter(|path| path.exists()),
        );
        Ok(self.config.clone())
    }
}
