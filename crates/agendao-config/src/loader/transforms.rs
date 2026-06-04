use crate::schema::{
    AgentConfig, AgentConfigs, AgentMode, PermissionAction, PermissionConfig, PermissionRule,
    PluginConfig,
};
use crate::Config;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::file_ops::{get_global_config_paths, parse_jsonc};

fn project_config_write_path(project_dir: &Path) -> PathBuf {
    [
        "agendao.json",
        "agendao.jsonc",
        ".agendao/agendao.json",
        ".agendao/agendao.jsonc",
    ]
    .into_iter()
    .map(|target| project_dir.join(target))
    .find(|path| path.exists())
    .unwrap_or_else(|| project_dir.join(".agendao/agendao.json"))
}

fn write_config_file(config_path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json =
        serde_json::to_string_pretty(config).with_context(|| "Failed to serialize config")?;
    fs::write(config_path, json)
        .with_context(|| format!("Failed to write config to {:?}", config_path))?;

    Ok(())
}

pub(super) fn merge_agent_config(target: &mut AgentConfig, source: AgentConfig) {
    if source.name.is_some() {
        target.name = source.name;
    }
    if source.model.is_some() {
        target.model = source.model;
    }
    if source.variant.is_some() {
        target.variant = source.variant;
    }
    if source.temperature.is_some() {
        target.temperature = source.temperature;
    }
    if source.top_p.is_some() {
        target.top_p = source.top_p;
    }
    if source.prompt.is_some() {
        target.prompt = source.prompt;
    }
    if source.disable.is_some() {
        target.disable = source.disable;
    }
    if source.description.is_some() {
        target.description = source.description;
    }
    if source.mode.is_some() {
        target.mode = source.mode;
    }
    if source.hidden.is_some() {
        target.hidden = source.hidden;
    }
    if source.color.is_some() {
        target.color = source.color;
    }
    if source.steps.is_some() {
        target.steps = source.steps;
    }
    if source.max_steps.is_some() {
        target.max_steps = source.max_steps;
    }
    if source.max_tokens.is_some() {
        target.max_tokens = source.max_tokens;
    }
    if let Some(source_opts) = source.options {
        let target_opts = target.options.get_or_insert_with(HashMap::new);
        for (k, v) in source_opts {
            target_opts.insert(k, v);
        }
    }
    if let Some(source_perm) = source.permission {
        if let Some(target_perm) = &mut target.permission {
            for (k, v) in source_perm.rules {
                target_perm.rules.insert(k, v);
            }
        } else {
            target.permission = Some(source_perm);
        }
    }
    if let Some(source_tools) = source.tools {
        let target_tools = target.tools.get_or_insert_with(HashMap::new);
        for (k, v) in source_tools {
            target_tools.insert(k, v);
        }
    }
}

/// Extract canonical plugin name from a specifier.
/// - For file:// URLs: extracts filename without extension
/// - For npm packages: extracts package name without version
pub fn get_plugin_name(plugin: &str) -> String {
    if plugin.starts_with("file://") {
        return Path::new(plugin.trim_start_matches("file://"))
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| plugin.to_string());
    }
    // For npm packages: strip version after last @
    if let Some(last_at) = plugin.rfind('@') {
        if last_at > 0 {
            return plugin[..last_at].to_string();
        }
    }
    plugin.to_string()
}

/// Deduplicate plugins by name, with later entries (higher priority) winning.
/// Since plugins are added in low-to-high priority order,
/// we reverse, deduplicate (keeping first occurrence), then restore order.
pub fn deduplicate_plugins(
    plugins: HashMap<String, PluginConfig>,
) -> HashMap<String, PluginConfig> {
    // HashMap is inherently deduplicated by key.
    plugins
}

/// Apply post-load transforms: legacy migrations, flag overrides, plugin dedup.
pub(super) fn apply_post_load_transforms(config: &mut Config) {
    // Migrate deprecated `mode` field to `agent` field
    if let Some(mode_configs) = config.mode.take() {
        let agent_configs = config.agent.get_or_insert_with(AgentConfigs::default);
        for (name, mut mode_agent) in mode_configs.entries {
            mode_agent.mode = Some(AgentMode::Primary);
            if let Some(existing) = agent_configs.entries.get_mut(&name) {
                merge_agent_config(existing, mode_agent);
            } else {
                agent_configs.entries.insert(name, mode_agent);
            }
        }
    }

    // AGENDAO_PERMISSION env var override
    if let Ok(perm_json) = env::var("AGENDAO_PERMISSION") {
        if let Ok(perm) = serde_json::from_str::<PermissionConfig>(&perm_json) {
            let target = config
                .permission
                .get_or_insert_with(PermissionConfig::default);
            for (k, v) in perm.rules {
                target.rules.insert(k, v);
            }
        }
    }

    // Backwards compatibility: legacy top-level `tools` config -> permission
    if let Some(tools) = config.tools.take() {
        let mut perms = HashMap::new();
        for (tool, enabled) in tools {
            let action = if enabled {
                PermissionAction::Allow
            } else {
                PermissionAction::Deny
            };
            // write, edit, patch, multiedit all map to "edit" permission
            if tool == "write" || tool == "edit" || tool == "patch" || tool == "multiedit" {
                perms.insert("edit".to_string(), PermissionRule::Action(action));
            } else {
                perms.insert(tool, PermissionRule::Action(action));
            }
        }
        // Legacy tools have lower priority than explicit permission config
        if let Some(existing) = &config.permission {
            for (k, v) in existing.rules.clone() {
                perms.insert(k, v);
            }
        }
        config.permission = Some(PermissionConfig { rules: perms });
    }

    // Set default username from system
    if config.username.is_none() {
        config.username = env::var("USER").or_else(|_| env::var("USERNAME")).ok();
    }

    // Handle migration from autoshare to share field
    if config.autoshare == Some(true) && config.share.is_none() {
        config.share = Some(crate::schema::ShareMode::Auto);
    }

    if let Some(legacy_voice) = config.voice.clone() {
        let multimodal = config
            .multimodal
            .get_or_insert_with(crate::schema::MultimodalConfig::default);
        if let Some(current_voice) = multimodal.voice.as_mut() {
            if current_voice.duration_seconds.is_none() {
                current_voice.duration_seconds = legacy_voice.duration_seconds;
            }
            if current_voice.attach_audio.is_none() {
                current_voice.attach_audio = legacy_voice.attach_audio;
            }
            if current_voice.mime.is_none() {
                current_voice.mime = legacy_voice.mime;
            }
            if current_voice.language.is_none() {
                current_voice.language = legacy_voice.language;
            }
            if current_voice.record.is_none() {
                current_voice.record = legacy_voice.record;
            }
            if current_voice.transcribe.is_none() {
                current_voice.transcribe = legacy_voice.transcribe;
            }
        } else {
            multimodal.voice = Some(legacy_voice);
        }
    }

    // Apply flag overrides for compaction settings
    if env::var("AGENDAO_DISABLE_AUTOCOMPACT").is_ok() {
        let compaction = config.compaction.get_or_insert_with(Default::default);
        compaction.auto = Some(false);
    }
    if env::var("AGENDAO_DISABLE_PRUNE").is_ok() {
        let compaction = config.compaction.get_or_insert_with(Default::default);
        compaction.prune = Some(false);
    }

    // Deduplicate plugins
    let plugins = std::mem::take(&mut config.plugin);
    config.plugin = deduplicate_plugins(plugins);
}

/// Loads config synchronously (without remote wellknown fetching).
pub fn load_config<P: AsRef<Path>>(project_dir: P) -> Result<Config> {
    super::ConfigAuthority::resolve(project_dir.as_ref()).map(|resolved| resolved.config)
}

/// Update project-level config by merging a patch.
pub fn update_config(project_dir: &Path, patch: &Config) -> Result<()> {
    let config_path = project_config_write_path(project_dir);

    let existing = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        parse_jsonc(&content).unwrap_or_default()
    } else {
        Config::default()
    };

    let mut merged = existing;
    merged.merge(patch.clone());
    write_config_file(&config_path, &merged)
}

/// Replace the project-level config with an already-updated full config.
pub fn write_config(project_dir: &Path, config: &Config) -> Result<()> {
    let config_path = project_config_write_path(project_dir);
    write_config_file(&config_path, config)
}

/// Update global config by merging a patch.
pub fn update_global_config(patch: &Config) -> Result<()> {
    let global_paths = get_global_config_paths();

    // Prefer the active highest-precedence existing file; if none exist, default to agendao.json.
    let config_path = global_paths
        .iter()
        .rev()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| {
            global_paths
                .last()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("agendao/agendao.json"))
        });

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        parse_jsonc(&content).unwrap_or_default()
    } else {
        Config::default()
    };

    let mut merged = existing;
    merged.merge(patch.clone());

    let json =
        serde_json::to_string_pretty(&merged).with_context(|| "Failed to serialize config")?;
    fs::write(&config_path, json)
        .with_context(|| format!("Failed to write global config to {:?}", config_path))?;

    Ok(())
}
