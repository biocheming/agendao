use crate::discovery::{parse_skill_file, read_skill_body};
use crate::{SkillError, SkillMeta, SkillRoot};
use agendao_types::{
    BundledSkillManifest, ManagedSkillRecord, SkillSourceIndexEntry, SkillSourceIndexSnapshot,
    SkillSourceKind, SkillSourceRef, SkillSyncAction, SkillSyncEntry, SkillSyncPlan,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub(crate) struct SkillSyncSourceFile {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillSyncSourceEntry {
    pub skill_name: String,
    pub description: String,
    pub category: Option<String>,
    pub relative_path: String,
    pub content_hash: String,
    pub revision: Option<String>,
    pub markdown_content: String,
    pub body: String,
    pub supporting_files: Vec<SkillSyncSourceFile>,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillSyncSourceSnapshot {
    pub source: SkillSourceRef,
    pub entries: Vec<SkillSyncSourceEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedManagedSkillRecord {
    pub record: ManagedSkillRecord,
    pub current_hash: Option<String>,
}

#[derive(Debug, Default)]
pub struct SkillSyncPlanner;

impl SkillSyncPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn empty_plan_for_source(&self, source: &SkillSourceRef) -> SkillSyncPlan {
        SkillSyncPlan {
            source_id: source.source_id.clone(),
            entries: Vec::new(),
        }
    }

    pub(crate) fn plan_sync(
        &self,
        source_snapshot: &SkillSyncSourceSnapshot,
        managed_records: &[ResolvedManagedSkillRecord],
        catalog: &[SkillMeta],
    ) -> SkillSyncPlan {
        let source_entries = source_snapshot
            .entries
            .iter()
            .map(|entry| (normalize_name(&entry.skill_name), entry))
            .collect::<BTreeMap<_, _>>();

        let managed_by_source = managed_records
            .iter()
            .filter(|record| {
                record
                    .record
                    .source
                    .as_ref()
                    .map(|source| source.source_id == source_snapshot.source.source_id)
                    .unwrap_or(false)
            })
            .map(|record| (normalize_name(&record.record.skill_name), record))
            .collect::<BTreeMap<_, _>>();

        let catalog_by_name = catalog
            .iter()
            .map(|meta| (normalize_name(&meta.name), meta))
            .collect::<BTreeMap<_, _>>();

        let all_names = source_entries
            .keys()
            .chain(managed_by_source.keys())
            .cloned()
            .collect::<BTreeSet<_>>();

        let entries = all_names
            .into_iter()
            .map(|name| {
                build_sync_entry(
                    &name,
                    source_entries.get(&name).copied(),
                    managed_by_source.get(&name).copied(),
                    catalog_by_name.get(&name).copied(),
                )
            })
            .collect();

        SkillSyncPlan {
            source_id: source_snapshot.source.source_id.clone(),
            entries,
        }
    }

    pub(crate) fn refresh_managed_records(
        &self,
        managed_records: &[ManagedSkillRecord],
        catalog: &[SkillMeta],
        source_snapshot: Option<&SkillSyncSourceSnapshot>,
    ) -> Result<Vec<ResolvedManagedSkillRecord>, SkillError> {
        let tracked_names = managed_records
            .iter()
            .map(|record| normalize_name(&record.skill_name))
            .collect::<BTreeSet<_>>();
        let catalog_hashes = catalog
            .iter()
            .filter(|meta| tracked_names.contains(&normalize_name(&meta.name)))
            .map(|meta| Ok((normalize_name(&meta.name), hash_skill_meta(meta)?)))
            .collect::<Result<BTreeMap<_, _>, SkillError>>()?;
        let source_hashes = source_snapshot
            .map(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .map(|entry| {
                        (
                            normalize_name(&entry.skill_name),
                            entry.content_hash.as_str(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        managed_records
            .iter()
            .cloned()
            .map(|mut record| {
                let normalized = normalize_name(&record.skill_name);
                let current_hash = catalog_hashes.get(&normalized).cloned();
                record.deleted_locally = current_hash.is_none();
                record.locally_modified = current_hash
                    .as_deref()
                    .map(|current_hash| {
                        let source_hash = source_hashes.get(&normalized).copied();
                        is_locally_modified(&record, current_hash, source_hash)
                    })
                    .unwrap_or(false);
                Ok(ResolvedManagedSkillRecord {
                    record,
                    current_hash,
                })
            })
            .collect()
    }

    pub(crate) fn build_local_source_snapshot(
        &self,
        source: &SkillSourceRef,
        root: &Path,
    ) -> Result<SkillSyncSourceSnapshot, SkillError> {
        let skill_root = SkillRoot {
            path: root.to_path_buf(),
        };
        let mut entries = iter_skill_markdown_files(root)
            .into_iter()
            .map(|skill_path| load_source_entry(source, root, &skill_root, &skill_path, None))
            .collect::<Result<Vec<_>, SkillError>>()?;
        entries.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        Ok(SkillSyncSourceSnapshot {
            source: source.clone(),
            entries,
        })
    }

    pub(crate) fn build_bundled_source_snapshot(
        &self,
        source: &SkillSourceRef,
        root: &Path,
        manifest: &BundledSkillManifest,
    ) -> Result<SkillSyncSourceSnapshot, SkillError> {
        let skill_root = SkillRoot {
            path: root.to_path_buf(),
        };
        let mut entries = manifest
            .entries
            .iter()
            .map(|manifest_entry| {
                let skill_path = root.join(&manifest_entry.relative_path);
                load_source_entry(
                    source,
                    root,
                    &skill_root,
                    &skill_path,
                    Some(manifest_entry.content_hash.as_str()),
                )
            })
            .collect::<Result<Vec<_>, SkillError>>()?;
        entries.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        Ok(SkillSyncSourceSnapshot {
            source: source.clone(),
            entries,
        })
    }

    pub(crate) fn source_index_snapshot(
        &self,
        source_snapshot: &SkillSyncSourceSnapshot,
    ) -> SkillSourceIndexSnapshot {
        SkillSourceIndexSnapshot {
            source: source_snapshot.source.clone(),
            updated_at: now_unix_timestamp(),
            entries: source_snapshot
                .entries
                .iter()
                .map(|entry| SkillSourceIndexEntry {
                    skill_name: entry.skill_name.clone(),
                    description: Some(entry.description.clone()),
                    category: entry.category.clone(),
                    version: None,
                    revision: entry
                        .revision
                        .clone()
                        .or_else(|| Some(entry.content_hash.clone())),
                    manifest_path: None,
                    checksum: None,
                    ..Default::default()
                })
                .collect(),
        }
    }
}

pub(crate) fn hash_skill_meta(meta: &SkillMeta) -> Result<String, SkillError> {
    let markdown_content =
        fs::read_to_string(&meta.location).map_err(|error| SkillError::ReadFailed {
            path: meta.location.clone(),
            message: error.to_string(),
        })?;
    let supporting_files = meta
        .supporting_files
        .iter()
        .map(|file| {
            let content =
                fs::read_to_string(&file.location).map_err(|error| SkillError::ReadFailed {
                    path: file.location.clone(),
                    message: error.to_string(),
                })?;
            Ok(SkillSyncSourceFile {
                relative_path: file.relative_path.clone(),
                content,
            })
        })
        .collect::<Result<Vec<_>, SkillError>>()?;
    Ok(compute_skill_hash(&markdown_content, &supporting_files))
}

fn build_sync_entry(
    normalized_name: &str,
    source_entry: Option<&SkillSyncSourceEntry>,
    managed_record: Option<&ResolvedManagedSkillRecord>,
    catalog_entry: Option<&SkillMeta>,
) -> SkillSyncEntry {
    let skill_name = source_entry
        .map(|entry| entry.skill_name.clone())
        .or_else(|| managed_record.map(|record| record.record.skill_name.clone()))
        .or_else(|| catalog_entry.map(|meta| meta.name.clone()))
        .unwrap_or_else(|| normalized_name.to_string());

    let (action, reason) = match (source_entry, managed_record) {
        (Some(source_entry), None) => {
            if catalog_entry.is_some() {
                (
                    SkillSyncAction::Noop,
                    "workspace already has a local skill with the same name".to_string(),
                )
            } else {
                (
                    SkillSyncAction::Install,
                    format!(
                        "source `{}` now provides `{}`",
                        source_entry
                            .revision
                            .as_deref()
                            .unwrap_or(source_entry.content_hash.as_str()),
                        source_entry.skill_name
                    ),
                )
            }
        }
        (Some(source_entry), Some(managed_record)) => {
            match managed_record.current_hash.as_deref() {
                None => (
                    SkillSyncAction::SkipDeletedLocally,
                    "workspace deleted the managed skill locally".to_string(),
                ),
                Some(current_hash)
                    if is_locally_modified(
                        &managed_record.record,
                        current_hash,
                        Some(source_entry.content_hash.as_str()),
                    ) =>
                {
                    (
                        SkillSyncAction::SkipLocalModification,
                        "workspace skill diverged from the last synced content".to_string(),
                    )
                }
                Some(current_hash) if current_hash == source_entry.content_hash => (
                    SkillSyncAction::Noop,
                    "workspace already matches source content".to_string(),
                ),
                Some(_) => (
                    SkillSyncAction::Update,
                    format!(
                        "source advanced to `{}`",
                        source_entry
                            .revision
                            .as_deref()
                            .unwrap_or(source_entry.content_hash.as_str())
                    ),
                ),
            }
        }
        (None, Some(_managed_record)) => (
            SkillSyncAction::RemoveManaged,
            "source no longer provides this managed skill".to_string(),
        ),
        (None, None) => (
            SkillSyncAction::Noop,
            "skill is outside the selected source".to_string(),
        ),
    };

    SkillSyncEntry {
        skill_name,
        action,
        reason,
    }
}

fn load_source_entry(
    source: &SkillSourceRef,
    root: &Path,
    skill_root: &SkillRoot,
    skill_path: &Path,
    manifest_hash: Option<&str>,
) -> Result<SkillSyncSourceEntry, SkillError> {
    let meta = parse_skill_file(skill_path, skill_root).ok_or_else(|| SkillError::ReadFailed {
        path: skill_path.to_path_buf(),
        message: "failed to parse skill frontmatter".to_string(),
    })?;
    let markdown_content =
        fs::read_to_string(skill_path).map_err(|error| SkillError::ReadFailed {
            path: skill_path.to_path_buf(),
            message: error.to_string(),
        })?;
    let body = read_skill_body(skill_path).map_err(|error| SkillError::ReadFailed {
        path: skill_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let supporting_files = meta
        .supporting_files
        .iter()
        .map(|file| {
            let content =
                fs::read_to_string(&file.location).map_err(|error| SkillError::ReadFailed {
                    path: file.location.clone(),
                    message: error.to_string(),
                })?;
            Ok(SkillSyncSourceFile {
                relative_path: file.relative_path.clone(),
                content,
            })
        })
        .collect::<Result<Vec<_>, SkillError>>()?;

    let content_hash = compute_skill_hash(&markdown_content, &supporting_files);
    let relative_path = skill_path
        .strip_prefix(root)
        .map_err(|_| SkillError::ReadFailed {
            path: skill_path.to_path_buf(),
            message: "skill path escaped source root".to_string(),
        })?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(SkillSyncSourceEntry {
        skill_name: meta.name,
        description: meta.description,
        category: meta.category,
        relative_path,
        content_hash: content_hash.clone(),
        revision: source
            .revision
            .clone()
            .or_else(|| manifest_hash.map(ToOwned::to_owned))
            .or_else(|| Some(content_hash.clone())),
        markdown_content,
        body,
        supporting_files,
    })
}

fn compute_skill_hash(markdown_content: &str, supporting_files: &[SkillSyncSourceFile]) -> String {
    let mut hasher = Sha256::new();
    hasher.update("SKILL.md");
    hasher.update([0]);
    hasher.update(markdown_content.as_bytes());
    for file in supporting_files {
        hasher.update([0xff]);
        hasher.update(file.relative_path.as_bytes());
        hasher.update([0]);
        hasher.update(file.content.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn is_locally_modified(
    record: &ManagedSkillRecord,
    current_hash: &str,
    source_hash: Option<&str>,
) -> bool {
    if source_hash == Some(current_hash) {
        return false;
    }
    if record.local_hash.as_deref() == Some(current_hash) {
        return false;
    }
    true
}

fn iter_skill_markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut skill_files = WalkDir::new(root)
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
        .collect::<Vec<_>>();
    skill_files.sort();
    skill_files
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn now_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn source_root_kind_supported(source: &SkillSourceRef) -> bool {
    matches!(
        source.source_kind,
        SkillSourceKind::Bundled | SkillSourceKind::LocalPath
    )
}
