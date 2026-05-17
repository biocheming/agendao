use crate::schema::{AgentConfig, AgentMode, CommandConfig};
use anyhow::Result;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::file_ops::get_global_config_paths;
use super::markdown_parser::{parse_markdown_agent, parse_markdown_command};

pub(super) fn normalize_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn detect_worktree_stop(start: &Path) -> PathBuf {
    let mut current = normalize_existing_path(start);
    let mut topmost = current.clone();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        let Some(parent) = current.parent() else {
            return topmost;
        };
        if parent == current {
            return topmost;
        }
        topmost = parent.to_path_buf();
        current = parent.to_path_buf();
    }
}

pub(super) fn find_up(target: &str, start: &Path, stop: &Path) -> Vec<PathBuf> {
    let mut current = normalize_existing_path(start);
    let stop = normalize_existing_path(stop);
    let mut result = Vec::new();

    loop {
        let candidate = current.join(target);
        if candidate.exists() {
            result.push(candidate);
        }
        if current == stop {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }

    result
}

/// Get the managed config directory for enterprise deployments.
pub(super) fn get_managed_config_dir() -> PathBuf {
    if let Ok(test_dir) = env::var("ROCODE_TEST_MANAGED_CONFIG_DIR") {
        return PathBuf::from(test_dir);
    }
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/rocode")
    } else if cfg!(target_os = "windows") {
        let program_data =
            env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        PathBuf::from(program_data).join("rocode")
    } else {
        PathBuf::from("/etc/rocode")
    }
}

/// Collect .rocode directories from project hierarchy and global config.
pub(super) fn collect_rocode_directories(project_dir: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();

    // Global config directory
    for global_config in get_global_config_paths() {
        if let Some(global_config_dir) = global_config.parent() {
            let global_config_dir = global_config_dir.to_path_buf();
            if global_config_dir.exists() {
                directories.push(global_config_dir);
            }
        }
    }

    // Only the current workspace-local `.rocode` directory participates in
    // config/command/agent discovery. Ancestor `.rocode` directories do not.
    let start_dir = normalize_existing_path(project_dir);
    let local_rocode_dir = start_dir.join(".rocode");
    if local_rocode_dir.is_dir() {
        directories.push(local_rocode_dir);
    }

    // Home directory .rocode
    if let Some(home) = dirs::home_dir() {
        let home_rocode = home.join(".rocode");
        if home_rocode.exists() && !directories.contains(&home_rocode) {
            directories.push(home_rocode);
        }
    }

    // ROCODE_CONFIG_DIR overrides
    if let Ok(config_dir) = env::var("ROCODE_CONFIG_DIR") {
        let dir = PathBuf::from(config_dir);
        if !directories.contains(&dir) {
            directories.push(dir);
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    directories.retain(|d| seen.insert(d.clone()));

    directories
}

/// Load command definitions from markdown files in {command,commands}/**/*.md
pub(super) fn load_commands_from_dir(dir: &Path) -> HashMap<String, CommandConfig> {
    let mut result = HashMap::new();

    for subdir_name in &["command", "commands"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = glob_md_files(&subdir) {
            for entry in entries {
                if let Some((name, content)) = parse_markdown_command(&entry, dir) {
                    result.insert(name, content);
                }
            }
        }
    }

    result
}

/// Load agent definitions from markdown files in {agent,agents}/**/*.md
pub(super) fn load_agents_from_dir(dir: &Path) -> HashMap<String, AgentConfig> {
    let mut result = HashMap::new();

    for subdir_name in &["agent", "agents"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = glob_md_files(&subdir) {
            for entry in entries {
                if let Some((name, config)) = parse_markdown_agent(&entry, dir) {
                    result.insert(name, config);
                }
            }
        }
    }

    result
}

/// Load mode definitions from markdown files in {mode,modes}/*.md
pub(super) fn load_modes_from_dir(dir: &Path) -> HashMap<String, AgentConfig> {
    let mut result = HashMap::new();

    for subdir_name in &["mode", "modes"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = glob_md_files(&subdir) {
            for entry in entries {
                if let Some((name, mut config)) = parse_markdown_agent(&entry, dir) {
                    // Modes are always primary agents
                    config.mode = Some(AgentMode::Primary);
                    result.insert(name, config);
                }
            }
        }
    }

    result
}

pub(crate) fn resolve_configured_path(base: &Path, raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

pub fn collect_plugin_roots(
    project_dir: &Path,
    plugin_paths: &HashMap<String, String>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(config_dir) = dirs::config_dir() {
        roots.push(config_dir.join("rocode/plugins"));
        roots.push(config_dir.join("rocode/plugin"));
    }

    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".rocode/plugins"));
        roots.push(home.join(".rocode/plugin"));
    }

    let start_dir = normalize_existing_path(project_dir);
    let stop_dir = detect_worktree_stop(&start_dir);
    let found = find_up(".rocode", &start_dir, &stop_dir);
    for path in found.into_iter().rev() {
        roots.push(path.join("plugins"));
        roots.push(path.join("plugin"));
    }

    let mut names: Vec<&String> = plugin_paths.keys().collect();
    names.sort();
    for name in names {
        if let Some(raw) = plugin_paths.get(name) {
            roots.push(resolve_configured_path(project_dir, raw));
        }
    }

    let mut deduped = Vec::new();
    for root in roots {
        if !deduped.contains(&root) {
            deduped.push(root);
        }
    }
    deduped
}

fn collect_plugins_in_dir(dir: &Path, plugins: &mut Vec<String>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "ts" || ext == "js" || ext == "mjs" {
                        plugins.push(format!("file://{}", path.display()));
                    }
                }
            } else if path.is_dir() {
                if path.file_name().map_or(false, |n| n == "web") {
                    continue;
                }
                for entry_name in ["index.ts", "index.js", "index.mjs"] {
                    let entry_path = path.join(entry_name);
                    if entry_path.is_file() {
                        plugins.push(format!("file://{}", entry_path.display()));
                        break;
                    }
                }
            }
        }
    }
}

/// Load plugin paths from a directory.
/// - Direct files in `path`
/// - Compatibility subdirectories `path/plugin` and `path/plugins`
pub(super) fn load_plugins_from_path(path: &Path) -> Vec<String> {
    let mut plugins = Vec::new();
    collect_plugins_in_dir(path, &mut plugins);
    collect_plugins_in_dir(&path.join("plugin"), &mut plugins);
    collect_plugins_in_dir(&path.join("plugins"), &mut plugins);
    plugins
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WebPluginInfo {
    pub name: String,
    pub entry_path: PathBuf,
    pub serve_root: PathBuf,
}

impl WebPluginInfo {
    pub fn entry(&self) -> String {
        self.entry_path
            .strip_prefix(&self.serve_root)
            .ok()
            .and_then(|path| path.to_str())
            .map(|path| path.replace('\\', "/"))
            .unwrap_or_else(|| {
                self.entry_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
    }
}

/// Discover web plugins from `web/` subdirectories under plugin roots.
/// Web plugins must be `.js` files (browser-loadable).
pub fn discover_web_plugins(plugin_roots: &[PathBuf]) -> Vec<WebPluginInfo> {
    let mut web_plugins = Vec::new();
    for root in plugin_roots {
        for sub in ["", "plugin", "plugins"] {
            let web_dir = if sub.is_empty() {
                root.join("web")
            } else {
                root.join(sub).join("web")
            };
            if !web_dir.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&web_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if matches!(
                            path.extension().and_then(|e| e.to_str()),
                            Some("js" | "mjs")
                        ) {
                            let name = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            upsert_web_plugin(
                                &mut web_plugins,
                                WebPluginInfo {
                                    name,
                                    entry_path: path,
                                    serve_root: web_dir.clone(),
                                },
                            );
                        }
                    } else if path.is_dir() {
                        let mut entry_path = None;
                        for entry_name in ["index.js", "index.mjs"] {
                            let candidate = path.join(entry_name);
                            if candidate.is_file() {
                                entry_path = Some(candidate);
                                break;
                            }
                        }
                        if let Some(entry_path) = entry_path {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            upsert_web_plugin(
                                &mut web_plugins,
                                WebPluginInfo {
                                    name,
                                    entry_path,
                                    serve_root: path,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
    web_plugins
}

fn upsert_web_plugin(web_plugins: &mut Vec<WebPluginInfo>, plugin: WebPluginInfo) {
    if let Some(existing) = web_plugins
        .iter_mut()
        .find(|entry| entry.name == plugin.name)
    {
        *existing = plugin;
    } else {
        web_plugins.push(plugin);
    }
}

/// Recursively find all .md files in a directory.
fn glob_md_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    glob_md_files_recursive(dir, &mut results)?;
    Ok(results)
}

fn glob_md_files_recursive(dir: &Path, results: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            glob_md_files_recursive(&path, results)?;
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            results.push(path);
        }
    }
    Ok(())
}
