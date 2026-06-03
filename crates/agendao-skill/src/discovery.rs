use crate::{
    SkillConditions, SkillDirectorySignature, SkillFileRef, SkillFileSignature, SkillMeta,
    SkillRoot, SkillRootSignature,
};
use agendao_config::{Config, ConfigStore};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub(crate) fn collect_skill_roots(base: &Path, config: Option<&Config>) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        roots.push(SkillRoot {
            path: config_dir.join("agendao/skill"),
        });
        roots.push(SkillRoot {
            path: config_dir.join("agendao/skills"),
        });
    }

    if let Some(home) = dirs::home_dir() {
        roots.push(SkillRoot {
            path: home.join(".agendao/skill"),
        });
        roots.push(SkillRoot {
            path: home.join(".agendao/skills"),
        });
        roots.push(SkillRoot {
            path: home.join(".agents/skills"),
        });
    }

    roots.push(SkillRoot {
        path: base.join(".agendao/skill"),
    });
    roots.push(SkillRoot {
        path: base.join(".agendao/skills"),
    });
    roots.push(SkillRoot {
        path: base.join(".agents/skills"),
    });

    if let Some(config) = config {
        roots.extend(configured_skill_roots(base, config));
    }

    let mut deduped = Vec::new();
    for root in roots {
        if !deduped
            .iter()
            .any(|existing: &SkillRoot| existing.path == root.path)
        {
            deduped.push(root);
        }
    }
    deduped
}

pub(crate) fn config_for_skill_discovery(
    base: &Path,
    config_store: Option<&ConfigStore>,
) -> Option<Arc<Config>> {
    config_store.map(ConfigStore::config).or_else(|| {
        ConfigStore::from_project_dir(base)
            .ok()
            .map(|store| store.config())
    })
}

pub(crate) fn compute_root_signature(root: &SkillRoot) -> SkillRootSignature {
    let mut directories = vec![current_directory_signature(&root.path)];
    let mut files = Vec::new();
    if root.path.exists() && root.path.is_dir() {
        for entry in WalkDir::new(&root.path)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path().to_path_buf();
            if entry.file_type().is_dir() {
                if path != root.path {
                    directories.push(current_directory_signature(&path));
                }
                continue;
            }

            if entry.file_type().is_file() {
                files.push(current_file_signature(&path));
            }
        }
    }
    directories.sort_by(|a, b| a.path.cmp(&b.path));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    SkillRootSignature {
        root: root.path.clone(),
        directories,
        files,
    }
}

pub(crate) fn root_signature_is_current(signature: &SkillRootSignature) -> bool {
    signature.root
        == signature
            .directories
            .first()
            .map(|dir| dir.path.as_path())
            .unwrap_or(signature.root.as_path())
        && signature
            .directories
            .iter()
            .all(|directory| current_directory_signature(&directory.path) == *directory)
        && signature
            .files
            .iter()
            .all(|file| current_file_signature(&file.path) == *file)
}

pub(crate) fn scan_skill_roots(roots: &[SkillRoot]) -> Vec<SkillMeta> {
    let mut by_name: HashMap<String, SkillMeta> = HashMap::new();
    for root in roots {
        for skill in scan_skill_root(root) {
            by_name.insert(skill.name.clone(), skill);
        }
    }

    let mut skills: Vec<SkillMeta> = by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

pub(crate) fn read_skill_body(path: &Path) -> Result<String, std::io::Error> {
    let raw = fs::read_to_string(path)?;
    let normalized = raw.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Ok(normalized.trim().to_string());
    }

    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
    }

    if !closed {
        return Ok(normalized.trim().to_string());
    }

    Ok(lines.collect::<Vec<_>>().join("\n").trim().to_string())
}

pub(crate) fn parse_skill_file(path: &Path, root: &SkillRoot) -> Option<SkillMeta> {
    let raw = fs::read_to_string(path).ok()?;
    let normalized = raw.replace("\r\n", "\n");
    let mut lines = normalized.lines();

    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut frontmatter_lines = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        frontmatter_lines.push(line);
    }
    if !closed {
        return None;
    }

    let frontmatter = frontmatter_lines.join("\n");
    let name = parse_frontmatter_value(&frontmatter, "name")?;
    let description = parse_frontmatter_value(&frontmatter, "description")?;
    let skill_dir = path.parent()?;

    Some(SkillMeta {
        name,
        description,
        category: derive_category(root, skill_dir),
        location: path.to_path_buf(),
        supporting_files: collect_supporting_files(skill_dir),
        conditions: parse_agendao_conditions(&frontmatter),
    })
}

pub(crate) fn is_valid_relative_skill_path(file_path: &str) -> bool {
    let path = Path::new(file_path);
    !path.is_absolute()
        && !path.as_os_str().is_empty()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_)) || matches!(component, Component::CurDir)
        })
}

fn resolve_skill_path(base: &Path, raw: &str) -> PathBuf {
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

fn configured_skill_roots(base: &Path, config: &Config) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    if let Some(skills) = &config.skills {
        for raw in &skills.paths {
            roots.push(SkillRoot {
                path: resolve_skill_path(base, raw),
            });
        }
    }

    let mut names: Vec<&String> = config.skill_paths.keys().collect();
    names.sort();
    for name in names {
        if let Some(raw) = config.skill_paths.get(name) {
            roots.push(SkillRoot {
                path: resolve_skill_path(base, raw),
            });
        }
    }

    roots
}

fn scan_skill_root(root: &SkillRoot) -> Vec<SkillMeta> {
    if !root.path.exists() || !root.path.is_dir() {
        return Vec::new();
    }

    iter_skill_files(&root.path)
        .into_iter()
        .filter_map(|path| parse_skill_file(&path, root))
        .collect()
}

fn parse_frontmatter_value(frontmatter: &str, key: &str) -> Option<String> {
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix(&format!("{key}:")) {
            let value = value.trim();
            if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                return Some(value[1..value.len() - 1].to_string());
            }
            return Some(value.to_string());
        }
    }
    None
}

fn parse_agendao_conditions(frontmatter: &str) -> SkillConditions {
    SkillConditions {
        requires_tools: parse_scoped_frontmatter_list(frontmatter, "requires_tools"),
        fallback_for_tools: parse_scoped_frontmatter_list(frontmatter, "fallback_for_tools"),
        requires_toolsets: parse_scoped_frontmatter_list(frontmatter, "requires_toolsets"),
        fallback_for_toolsets: parse_scoped_frontmatter_list(frontmatter, "fallback_for_toolsets"),
        stage_filter: parse_scoped_frontmatter_list(frontmatter, "stage_filter"),
    }
}

fn parse_scoped_frontmatter_list(frontmatter: &str, key: &str) -> Vec<String> {
    let lines = frontmatter.lines().collect::<Vec<_>>();
    let mut in_metadata = false;
    let mut metadata_indent = 0usize;
    let mut in_agendao = false;
    let mut agendao_indent = 0usize;

    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        let indent = line.len().saturating_sub(line.trim_start().len());

        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }

        if in_agendao && indent <= agendao_indent && !trimmed.starts_with('-') {
            in_agendao = false;
        }
        if in_metadata && indent <= metadata_indent && !trimmed.starts_with("metadata:") {
            in_metadata = false;
            in_agendao = false;
        }

        if trimmed == "metadata:" {
            in_metadata = true;
            metadata_indent = indent;
            in_agendao = false;
            index += 1;
            continue;
        }

        if in_metadata && trimmed == "agendao:" {
            in_agendao = true;
            agendao_indent = indent;
            index += 1;
            continue;
        }

        if in_agendao {
            let prefix = format!("{key}:");
            if let Some(value) = trimmed.strip_prefix(&prefix) {
                let key_indent = indent;
                let value = value.trim();
                if !value.is_empty() {
                    return parse_inline_yaml_list(value);
                }

                let mut items = Vec::new();
                let mut cursor = index + 1;
                while cursor < lines.len() {
                    let next = lines[cursor];
                    let next_trimmed = next.trim();
                    let next_indent = next.len().saturating_sub(next.trim_start().len());
                    if next_trimmed.is_empty() || next_trimmed.starts_with('#') {
                        cursor += 1;
                        continue;
                    }
                    if next_indent <= key_indent {
                        break;
                    }
                    if let Some(item) = next_trimmed.strip_prefix('-') {
                        let item = normalize_yaml_scalar(item.trim());
                        if !item.is_empty() {
                            items.push(item);
                        }
                    }
                    cursor += 1;
                }
                return items;
            }
        }

        index += 1;
    }

    Vec::new()
}

fn parse_inline_yaml_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return trimmed[1..trimmed.len() - 1]
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(normalize_yaml_scalar)
            .filter(|item| !item.is_empty())
            .collect();
    }

    let scalar = normalize_yaml_scalar(trimmed);
    if scalar.is_empty() {
        Vec::new()
    } else {
        vec![scalar]
    }
}

fn normalize_yaml_scalar(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].trim().to_string();
    }
    trimmed.to_string()
}

fn iter_skill_files(root: &Path) -> Vec<PathBuf> {
    let mut skill_files: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "SKILL.md")
                .unwrap_or(false)
        })
        .collect();
    skill_files.sort();
    skill_files
}

fn current_directory_signature(path: &Path) -> SkillDirectorySignature {
    match fs::metadata(path) {
        Ok(metadata) => SkillDirectorySignature {
            path: path.to_path_buf(),
            exists: true,
            is_dir: metadata.is_dir(),
            modified_ns: metadata_modified_ns(&metadata),
        },
        Err(_) => SkillDirectorySignature {
            path: path.to_path_buf(),
            exists: false,
            is_dir: false,
            modified_ns: 0,
        },
    }
}

fn current_file_signature(path: &Path) -> SkillFileSignature {
    match fs::metadata(path) {
        Ok(metadata) => SkillFileSignature {
            path: path.to_path_buf(),
            exists: true,
            is_file: metadata.is_file(),
            modified_ns: metadata_modified_ns(&metadata),
            size: metadata.len(),
        },
        Err(_) => SkillFileSignature {
            path: path.to_path_buf(),
            exists: false,
            is_file: false,
            modified_ns: 0,
            size: 0,
        },
    }
}

fn metadata_modified_ns(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn derive_category(root: &SkillRoot, skill_dir: &Path) -> Option<String> {
    let relative = skill_dir.strip_prefix(&root.path).ok()?;
    let mut parts: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();

    if parts.len() <= 1 {
        None
    } else {
        parts.pop();
        Some(parts.join("/"))
    }
}

fn collect_supporting_files(skill_dir: &Path) -> Vec<SkillFileRef> {
    let mut files: Vec<SkillFileRef> = WalkDir::new(skill_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name != "SKILL.md")
                .unwrap_or(false)
        })
        .filter_map(|location| {
            let relative_path = location
                .strip_prefix(skill_dir)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some(SkillFileRef {
                relative_path,
                location,
            })
        })
        .collect();
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files
}
