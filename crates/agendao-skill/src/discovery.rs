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

    // 用户级 agendao 目录经唯一权威入口解析（尊重 AGENDAO_HOME 覆盖）。
    let agendao_home = agendao_util::agendao_home();
    roots.push(SkillRoot {
        path: agendao_home.join("skill"),
    });
    roots.push(SkillRoot {
        path: agendao_home.join("skills"),
    });

    // ~/.agents/skills 是跨工具共享目录（非 agendao 自有），仍取真实用户主目录。
    if let Some(home) = dirs::home_dir() {
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
    let document = crate::write::parse_skill_document(&normalized).ok()?;
    let frontmatter = crate::write::parse_skill_frontmatter(&document).ok()?;
    let skill_dir = path.parent()?;
    let conditions = frontmatter
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.agendao.as_ref())
        .map(|metadata| SkillConditions {
            requires_tools: metadata.requires_tools.clone(),
            fallback_for_tools: metadata.fallback_for_tools.clone(),
            requires_toolsets: metadata.requires_toolsets.clone(),
            fallback_for_toolsets: metadata.fallback_for_toolsets.clone(),
        })
        .unwrap_or_default();

    Some(SkillMeta {
        name: frontmatter.name,
        description: frontmatter.description,
        category: derive_category(root, skill_dir),
        location: path.to_path_buf(),
        supporting_files: collect_supporting_files(skill_dir),
        conditions,
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
    // 展开用户配置里手写的 `~/`，要的是真实用户主目录，不经 agendao_home。
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
