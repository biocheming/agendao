use crate::audit::{append_audit_event, load_audit_events, DEFAULT_AUDIT_TAIL_LIMIT};
use crate::SkillError;
use rocode_types::{
    BundledSkillManifest, ManagedSkillRecord, SkillArtifactCacheEntry, SkillAuditEvent,
    SkillDistributionRecord, SkillManagedLifecycleRecord, SkillSourceIndexEntry,
    SkillSourceIndexSnapshot, SkillSourceRef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

pub(crate) const SKILL_GOVERNANCE_DIR: &str = ".rocode/state/skill";
const HUB_LOCK_FILENAME: &str = "hub-lock.json";
const BUNDLED_MANIFEST_FILENAME: &str = "bundled-manifest.json";
const AUDIT_LOG_FILENAME: &str = "audit.log";
const ARTIFACT_CACHE_DIRNAME: &str = "artifact-cache";
const ARTIFACT_CACHE_INDEX_FILENAME: &str = "index.json";
const DISTRIBUTION_LOCK_FILENAME: &str = "distribution-lock.json";
const INDEX_CACHE_DIRNAME: &str = "index-cache";
const LIFECYCLE_FILENAME: &str = "lifecycle.json";
const ARTIFACT_CACHE_SCHEMA: &str = "rocode.skill_artifact_cache";
const ARTIFACT_CACHE_VERSION: u32 = 1;
const DISTRIBUTION_LOCK_SCHEMA: &str = "rocode.skill_distribution_lock";
const DISTRIBUTION_LOCK_VERSION: u32 = 1;
const LIFECYCLE_SCHEMA: &str = "rocode.skill_lifecycle";
const LIFECYCLE_VERSION: u32 = 1;
const SOURCE_INDEX_CACHE_SCHEMA: &str = "rocode.skill_source_index_cache";
const SOURCE_INDEX_CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Default)]
pub struct SkillHubSnapshot {
    pub managed_skills: Vec<ManagedSkillRecord>,
    pub artifact_cache: Vec<SkillArtifactCacheEntry>,
    pub distributions: Vec<SkillDistributionRecord>,
    pub lifecycle: Vec<SkillManagedLifecycleRecord>,
    pub source_indices: Vec<SkillSourceIndexSnapshot>,
    pub bundled_manifest: Option<BundledSkillManifest>,
    pub audit_tail: Vec<SkillAuditEvent>,
}

#[derive(Debug)]
pub struct SkillHubStore {
    base_dir: PathBuf,
    cache: Arc<RwLock<SkillHubSnapshot>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RefreshedRemoteSourceIndex {
    pub snapshot: SkillSourceIndexSnapshot,
    pub raw_payload: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredSkillHubState {
    #[serde(default)]
    managed_skills: Vec<ManagedSkillRecord>,
    #[serde(default)]
    source_indices: Vec<SkillSourceIndexSnapshot>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "bundled_manifest",
        rename = "bundled_manifest"
    )]
    legacy_bundled_manifest: Option<BundledSkillManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSourceIndexCacheRecord {
    schema: String,
    version: u32,
    cached_at: i64,
    snapshot: SkillSourceIndexSnapshot,
    raw_payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSkillDistributionLock {
    schema: String,
    version: u32,
    #[serde(default)]
    distributions: Vec<SkillDistributionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSkillLifecycleState {
    schema: String,
    version: u32,
    #[serde(default)]
    lifecycle: Vec<SkillManagedLifecycleRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSkillArtifactCacheState {
    schema: String,
    version: u32,
    #[serde(default)]
    artifact_cache: Vec<SkillArtifactCacheEntry>,
}

impl SkillHubStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        let snapshot = load_snapshot_from_disk(&base_dir).unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to load skill hub snapshot from disk");
            SkillHubSnapshot::default()
        });
        Self {
            base_dir,
            cache: Arc::new(RwLock::new(snapshot)),
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn governance_dir(&self) -> PathBuf {
        governance_dir(&self.base_dir)
    }

    pub fn hub_lock_path(&self) -> PathBuf {
        self.governance_dir().join(HUB_LOCK_FILENAME)
    }

    pub fn audit_log_path(&self) -> PathBuf {
        self.governance_dir().join(AUDIT_LOG_FILENAME)
    }

    pub fn distribution_lock_path(&self) -> PathBuf {
        self.governance_dir().join(DISTRIBUTION_LOCK_FILENAME)
    }

    pub fn artifact_cache_dir(&self) -> PathBuf {
        self.governance_dir().join(ARTIFACT_CACHE_DIRNAME)
    }

    pub fn artifact_cache_index_path(&self) -> PathBuf {
        self.artifact_cache_dir()
            .join(ARTIFACT_CACHE_INDEX_FILENAME)
    }

    pub fn index_cache_dir(&self) -> PathBuf {
        self.governance_dir().join(INDEX_CACHE_DIRNAME)
    }

    pub fn lifecycle_path(&self) -> PathBuf {
        self.governance_dir().join(LIFECYCLE_FILENAME)
    }

    pub fn bundled_manifest_path(&self) -> PathBuf {
        self.governance_dir().join(BUNDLED_MANIFEST_FILENAME)
    }

    pub fn snapshot(&self) -> SkillHubSnapshot {
        self.snapshot_result().unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to read skill hub snapshot from cache");
            SkillHubSnapshot::default()
        })
    }

    pub fn managed_skills(&self) -> Vec<ManagedSkillRecord> {
        self.snapshot().managed_skills
    }

    pub fn distributions(&self) -> Vec<SkillDistributionRecord> {
        self.snapshot().distributions
    }

    pub fn artifact_cache(&self) -> Vec<SkillArtifactCacheEntry> {
        self.snapshot().artifact_cache
    }

    pub fn lifecycle(&self) -> Vec<SkillManagedLifecycleRecord> {
        self.snapshot().lifecycle
    }

    pub fn managed_skill(&self, skill_name: &str) -> Option<ManagedSkillRecord> {
        self.snapshot()
            .managed_skills
            .into_iter()
            .find(|record| record.skill_name.eq_ignore_ascii_case(skill_name))
    }

    pub fn audit_tail(&self) -> Vec<SkillAuditEvent> {
        self.snapshot().audit_tail
    }

    pub fn bundled_manifest(&self) -> Option<BundledSkillManifest> {
        self.snapshot().bundled_manifest
    }

    pub fn upsert_managed_skill(&self, record: ManagedSkillRecord) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        if let Some(existing) = snapshot
            .managed_skills
            .iter_mut()
            .find(|entry| entry.skill_name.eq_ignore_ascii_case(&record.skill_name))
        {
            *existing = record;
        } else {
            snapshot.managed_skills.push(record);
            snapshot
                .managed_skills
                .sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        }
        self.persist_snapshot(snapshot)
    }

    pub fn replace_managed_skills(
        &self,
        mut managed_skills: Vec<ManagedSkillRecord>,
    ) -> Result<(), SkillError> {
        managed_skills.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        let mut snapshot = self.snapshot_result()?;
        snapshot.managed_skills = managed_skills;
        self.persist_snapshot(snapshot)
    }

    pub fn replace_distributions(
        &self,
        mut distributions: Vec<SkillDistributionRecord>,
    ) -> Result<(), SkillError> {
        distributions.sort_by(|left, right| left.distribution_id.cmp(&right.distribution_id));
        let mut snapshot = self.snapshot_result()?;
        snapshot.distributions = distributions;
        self.persist_snapshot(snapshot)
    }

    pub fn replace_artifact_cache(
        &self,
        mut artifact_cache: Vec<SkillArtifactCacheEntry>,
    ) -> Result<(), SkillError> {
        artifact_cache
            .sort_by(|left, right| left.artifact.artifact_id.cmp(&right.artifact.artifact_id));
        let mut snapshot = self.snapshot_result()?;
        snapshot.artifact_cache = artifact_cache;
        self.persist_snapshot(snapshot)
    }

    pub fn upsert_artifact_cache_entry(
        &self,
        entry: SkillArtifactCacheEntry,
    ) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        if let Some(existing) = snapshot
            .artifact_cache
            .iter_mut()
            .find(|cached| cached.artifact.artifact_id == entry.artifact.artifact_id)
        {
            *existing = entry;
        } else {
            snapshot.artifact_cache.push(entry);
            snapshot
                .artifact_cache
                .sort_by(|left, right| left.artifact.artifact_id.cmp(&right.artifact.artifact_id));
        }
        self.persist_snapshot(snapshot)
    }

    pub fn upsert_distribution(
        &self,
        distribution: SkillDistributionRecord,
    ) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        if let Some(existing) = snapshot
            .distributions
            .iter_mut()
            .find(|entry| entry.distribution_id == distribution.distribution_id)
        {
            *existing = distribution;
        } else {
            snapshot.distributions.push(distribution);
            snapshot
                .distributions
                .sort_by(|left, right| left.distribution_id.cmp(&right.distribution_id));
        }
        self.persist_snapshot(snapshot)
    }

    pub fn replace_lifecycle(
        &self,
        mut lifecycle: Vec<SkillManagedLifecycleRecord>,
    ) -> Result<(), SkillError> {
        lifecycle.sort_by(|left, right| left.distribution_id.cmp(&right.distribution_id));
        let mut snapshot = self.snapshot_result()?;
        snapshot.lifecycle = lifecycle;
        self.persist_snapshot(snapshot)
    }

    pub fn upsert_lifecycle_record(
        &self,
        record: SkillManagedLifecycleRecord,
    ) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        if let Some(existing) = snapshot
            .lifecycle
            .iter_mut()
            .find(|entry| entry.distribution_id == record.distribution_id)
        {
            *existing = record;
        } else {
            snapshot.lifecycle.push(record);
            snapshot
                .lifecycle
                .sort_by(|left, right| left.distribution_id.cmp(&right.distribution_id));
        }
        self.persist_snapshot(snapshot)
    }

    pub fn remove_managed_skill(
        &self,
        skill_name: &str,
    ) -> Result<Option<ManagedSkillRecord>, SkillError> {
        let mut snapshot = self.snapshot_result()?;
        let original_len = snapshot.managed_skills.len();
        let mut removed = None;
        snapshot.managed_skills.retain(|record| {
            let matches = record.skill_name.eq_ignore_ascii_case(skill_name);
            if matches {
                removed = Some(record.clone());
            }
            !matches
        });
        if snapshot.managed_skills.len() == original_len {
            return Ok(None);
        }
        self.persist_snapshot(snapshot)?;
        Ok(removed)
    }

    pub fn replace_source_indices(
        &self,
        source_indices: Vec<SkillSourceIndexSnapshot>,
    ) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        snapshot.source_indices = source_indices;
        self.persist_snapshot(snapshot)
    }

    pub fn upsert_source_index(
        &self,
        source_index: SkillSourceIndexSnapshot,
    ) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        if let Some(existing) = snapshot
            .source_indices
            .iter_mut()
            .find(|entry| entry.source.source_id == source_index.source.source_id)
        {
            *existing = source_index;
        } else {
            snapshot.source_indices.push(source_index);
            snapshot
                .source_indices
                .sort_by(|left, right| left.source.source_id.cmp(&right.source.source_id));
        }
        self.persist_snapshot(snapshot)
    }

    pub(crate) fn upsert_remote_source_index(
        &self,
        refreshed: RefreshedRemoteSourceIndex,
    ) -> Result<SkillSourceIndexSnapshot, SkillError> {
        persist_source_index_cache(
            &source_index_cache_path(
                &self.index_cache_dir(),
                &refreshed.snapshot.source.source_id,
            ),
            &StoredSourceIndexCacheRecord {
                schema: SOURCE_INDEX_CACHE_SCHEMA.to_string(),
                version: SOURCE_INDEX_CACHE_VERSION,
                cached_at: now_unix_timestamp(),
                snapshot: refreshed.snapshot.clone(),
                raw_payload: refreshed.raw_payload,
            },
        )?;
        self.upsert_source_index(refreshed.snapshot.clone())?;
        Ok(refreshed.snapshot)
    }

    pub fn source_index(&self, source_id: &str) -> Option<SkillSourceIndexSnapshot> {
        self.snapshot()
            .source_indices
            .into_iter()
            .find(|snapshot| snapshot.source.source_id == source_id)
    }

    pub fn replace_bundled_manifest(
        &self,
        bundled_manifest: Option<BundledSkillManifest>,
    ) -> Result<(), SkillError> {
        let mut snapshot = self.snapshot_result()?;
        snapshot.bundled_manifest = bundled_manifest;
        self.persist_snapshot(snapshot)
    }

    pub fn append_audit_event(&self, event: SkillAuditEvent) -> Result<(), SkillError> {
        append_audit_event(&self.audit_log_path(), &event)?;
        let mut snapshot = self.write_cache()?;
        snapshot.audit_tail.push(event);
        if snapshot.audit_tail.len() > DEFAULT_AUDIT_TAIL_LIMIT {
            let overflow = snapshot.audit_tail.len() - DEFAULT_AUDIT_TAIL_LIMIT;
            snapshot.audit_tail.drain(0..overflow);
        }
        Ok(())
    }

    pub fn refresh(&self) -> Result<SkillHubSnapshot, SkillError> {
        let snapshot = load_snapshot_from_disk(&self.base_dir)?;
        *self.write_cache()? = snapshot.clone();
        Ok(snapshot)
    }

    fn persist_snapshot(&self, snapshot: SkillHubSnapshot) -> Result<(), SkillError> {
        persist_hub_state(&self.base_dir, &snapshot)?;
        *self.write_cache()? = snapshot;
        Ok(())
    }

    fn snapshot_result(&self) -> Result<SkillHubSnapshot, SkillError> {
        Ok(self.read_cache()?.clone())
    }

    fn read_cache(&self) -> Result<RwLockReadGuard<'_, SkillHubSnapshot>, SkillError> {
        self.cache.read().map_err(|_| SkillError::CachePoisoned {
            resource: "skill hub cache",
        })
    }

    fn write_cache(&self) -> Result<RwLockWriteGuard<'_, SkillHubSnapshot>, SkillError> {
        self.cache.write().map_err(|_| SkillError::CachePoisoned {
            resource: "skill hub cache",
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RemoteSkillIndexDocument {
    Snapshot(RemoteSkillIndexEntriesDocument),
    SkillsObject(RemoteSkillIndexSkillsDocument),
    EntryList(Vec<RemoteSkillIndexEntry>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteSkillIndexEntriesDocument {
    #[serde(default)]
    entries: Vec<RemoteSkillIndexEntry>,
    #[serde(default)]
    updated_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteSkillIndexSkillsDocument {
    #[serde(default)]
    skills: Vec<RemoteSkillIndexEntry>,
    #[serde(default)]
    updated_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteSkillIndexEntry {
    #[serde(alias = "name")]
    skill_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    manifest_path: Option<String>,
    #[serde(default)]
    checksum: Option<String>,
}

pub(crate) fn governance_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(SKILL_GOVERNANCE_DIR)
}

pub(crate) fn refresh_remote_source_index(
    base_dir: &Path,
    source: &SkillSourceRef,
    fetch_timeout_ms: u64,
) -> Result<RefreshedRemoteSourceIndex, SkillError> {
    let payload = load_remote_index_payload(base_dir, source, fetch_timeout_ms)?;
    let parsed = serde_json::from_str::<RemoteSkillIndexDocument>(&payload).map_err(|error| {
        SkillError::ReadFailed {
            path: remote_index_virtual_path(base_dir, source),
            message: format!("failed to parse remote skill index payload: {error}"),
        }
    })?;
    Ok(RefreshedRemoteSourceIndex {
        snapshot: remote_index_document_to_snapshot(source, parsed),
        raw_payload: payload,
    })
}

fn load_remote_index_payload(
    base_dir: &Path,
    source: &SkillSourceRef,
    fetch_timeout_ms: u64,
) -> Result<String, SkillError> {
    if is_http_locator(&source.locator) {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(fetch_timeout_ms))
            .build()
            .map_err(|error| SkillError::ReadFailed {
                path: remote_index_virtual_path(base_dir, source),
                message: format!("failed to build remote skill index client: {error}"),
            })?;
        let response = client.get(&source.locator).send().map_err(|error| {
            if error.is_timeout() {
                SkillError::ArtifactFetchTimeout {
                    locator: source.locator.clone(),
                    timeout_ms: fetch_timeout_ms,
                }
            } else {
                SkillError::ReadFailed {
                    path: remote_index_virtual_path(base_dir, source),
                    message: format!("failed to fetch remote skill index: {error}"),
                }
            }
        })?;
        let response = response
            .error_for_status()
            .map_err(|error| SkillError::ReadFailed {
                path: remote_index_virtual_path(base_dir, source),
                message: format!("remote skill index request failed: {error}"),
            })?;
        response.text().map_err(|error| {
            if error.is_timeout() {
                SkillError::ArtifactFetchTimeout {
                    locator: source.locator.clone(),
                    timeout_ms: fetch_timeout_ms,
                }
            } else {
                SkillError::ReadFailed {
                    path: remote_index_virtual_path(base_dir, source),
                    message: format!("failed to read remote skill index body: {error}"),
                }
            }
        })
    } else {
        let path = resolve_index_locator_path(base_dir, &source.locator);
        fs::read_to_string(&path).map_err(|error| SkillError::ReadFailed {
            path,
            message: error.to_string(),
        })
    }
}

fn remote_index_document_to_snapshot(
    source: &SkillSourceRef,
    document: RemoteSkillIndexDocument,
) -> SkillSourceIndexSnapshot {
    let (entries, updated_at) = match document {
        RemoteSkillIndexDocument::Snapshot(document) => (document.entries, document.updated_at),
        RemoteSkillIndexDocument::SkillsObject(document) => (document.skills, document.updated_at),
        RemoteSkillIndexDocument::EntryList(entries) => (entries, None),
    };

    let mut normalized_entries = entries
        .into_iter()
        .filter_map(|entry| {
            let skill_name = entry.skill_name.trim().to_string();
            if skill_name.is_empty() {
                return None;
            }
            Some(SkillSourceIndexEntry {
                skill_name,
                description: entry
                    .description
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                category: entry
                    .category
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                version: entry
                    .version
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                revision: entry.revision.or_else(|| source.revision.clone()),
                manifest_path: entry
                    .manifest_path
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                checksum: entry
                    .checksum
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
            })
        })
        .collect::<Vec<_>>();
    normalized_entries.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
    normalized_entries
        .dedup_by(|left, right| left.skill_name.eq_ignore_ascii_case(&right.skill_name));

    SkillSourceIndexSnapshot {
        source: source.clone(),
        updated_at: updated_at.unwrap_or_else(now_unix_timestamp),
        entries: normalized_entries,
    }
}

fn resolve_index_locator_path(base_dir: &Path, locator: &str) -> PathBuf {
    let path = PathBuf::from(locator);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn remote_index_virtual_path(base_dir: &Path, source: &SkillSourceRef) -> PathBuf {
    if is_http_locator(&source.locator) {
        PathBuf::from(source.locator.clone())
    } else {
        resolve_index_locator_path(base_dir, &source.locator)
    }
}

fn is_http_locator(locator: &str) -> bool {
    locator.starts_with("http://") || locator.starts_with("https://")
}

fn now_unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn load_snapshot_from_disk(base_dir: &Path) -> Result<SkillHubSnapshot, SkillError> {
    let governance_dir = governance_dir(base_dir);
    let hub_lock_path = governance_dir.join(HUB_LOCK_FILENAME);
    let bundled_manifest_path = governance_dir.join(BUNDLED_MANIFEST_FILENAME);
    let audit_log_path = governance_dir.join(AUDIT_LOG_FILENAME);
    let artifact_cache_index_path = governance_dir
        .join(ARTIFACT_CACHE_DIRNAME)
        .join(ARTIFACT_CACHE_INDEX_FILENAME);
    let distribution_lock_path = governance_dir.join(DISTRIBUTION_LOCK_FILENAME);
    let index_cache_dir = governance_dir.join(INDEX_CACHE_DIRNAME);
    let lifecycle_path = governance_dir.join(LIFECYCLE_FILENAME);

    let state = match load_hub_state(&hub_lock_path) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(%error, path=%hub_lock_path.display(), "failed to load skill hub state; continuing with empty state");
            StoredSkillHubState::default()
        }
    };
    let bundled_manifest = match load_bundled_manifest(&bundled_manifest_path) {
        Ok(manifest) => manifest.or(state.legacy_bundled_manifest),
        Err(error) => {
            tracing::warn!(%error, path=%bundled_manifest_path.display(), "failed to load bundled skill manifest");
            state.legacy_bundled_manifest
        }
    };
    let artifact_cache = match load_artifact_cache_state(&artifact_cache_index_path) {
        Ok(artifact_cache) => artifact_cache,
        Err(error) => {
            tracing::warn!(%error, path=%artifact_cache_index_path.display(), "failed to load skill artifact cache state");
            Vec::new()
        }
    };
    let distributions = match load_distribution_lock(&distribution_lock_path) {
        Ok(distributions) => distributions,
        Err(error) => {
            tracing::warn!(%error, path=%distribution_lock_path.display(), "failed to load skill distribution lock");
            Vec::new()
        }
    };
    let source_indices = merge_cached_remote_source_indices(state.source_indices, &index_cache_dir);
    let lifecycle = match load_lifecycle_state(&lifecycle_path) {
        Ok(lifecycle) => lifecycle,
        Err(error) => {
            tracing::warn!(%error, path=%lifecycle_path.display(), "failed to load skill lifecycle state");
            Vec::new()
        }
    };
    let audit_tail = match load_audit_events(&audit_log_path, DEFAULT_AUDIT_TAIL_LIMIT) {
        Ok(audit_tail) => audit_tail,
        Err(error) => {
            tracing::warn!(%error, path=%audit_log_path.display(), "failed to load skill audit log");
            Vec::new()
        }
    };
    Ok(SkillHubSnapshot {
        managed_skills: state.managed_skills,
        artifact_cache,
        distributions,
        lifecycle,
        source_indices,
        bundled_manifest,
        audit_tail,
    })
}

fn load_hub_state(path: &Path) -> Result<StoredSkillHubState, SkillError> {
    if !path.exists() {
        return Ok(StoredSkillHubState::default());
    }
    let content = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    serde_json::from_str(&content).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn persist_hub_state(base_dir: &Path, snapshot: &SkillHubSnapshot) -> Result<(), SkillError> {
    let governance_dir = governance_dir(base_dir);
    fs::create_dir_all(&governance_dir).map_err(|error| SkillError::WriteFailed {
        path: governance_dir.clone(),
        message: error.to_string(),
    })?;

    let state = StoredSkillHubState {
        managed_skills: snapshot.managed_skills.clone(),
        source_indices: snapshot.source_indices.clone(),
        legacy_bundled_manifest: None,
    };
    let hub_lock_path = governance_dir.join(HUB_LOCK_FILENAME);
    let payload =
        serde_json::to_string_pretty(&state).map_err(|error| SkillError::WriteFailed {
            path: hub_lock_path.clone(),
            message: error.to_string(),
        })?;
    fs::write(&hub_lock_path, payload).map_err(|error| SkillError::WriteFailed {
        path: hub_lock_path,
        message: error.to_string(),
    })?;

    persist_bundled_manifest(
        &governance_dir.join(BUNDLED_MANIFEST_FILENAME),
        snapshot.bundled_manifest.as_ref(),
    )?;
    persist_artifact_cache_state(
        &governance_dir
            .join(ARTIFACT_CACHE_DIRNAME)
            .join(ARTIFACT_CACHE_INDEX_FILENAME),
        &snapshot.artifact_cache,
    )?;
    persist_distribution_lock(
        &governance_dir.join(DISTRIBUTION_LOCK_FILENAME),
        &snapshot.distributions,
    )?;
    persist_lifecycle_state(
        &governance_dir.join(LIFECYCLE_FILENAME),
        &snapshot.lifecycle,
    )
}

fn load_bundled_manifest(path: &Path) -> Result<Option<BundledSkillManifest>, SkillError> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    serde_json::from_str(&content).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn persist_bundled_manifest(
    path: &Path,
    bundled_manifest: Option<&BundledSkillManifest>,
) -> Result<(), SkillError> {
    match bundled_manifest {
        Some(bundled_manifest) => {
            let payload = serde_json::to_string_pretty(bundled_manifest).map_err(|error| {
                SkillError::WriteFailed {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                }
            })?;
            fs::write(path, payload).map_err(|error| SkillError::WriteFailed {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
        None => {
            if path.exists() {
                fs::remove_file(path).map_err(|error| SkillError::WriteFailed {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })?;
            }
            Ok(())
        }
    }
}

fn load_distribution_lock(path: &Path) -> Result<Vec<SkillDistributionRecord>, SkillError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let stored =
        serde_json::from_str::<StoredSkillDistributionLock>(&content).map_err(|error| {
            SkillError::ReadFailed {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    if stored.schema != DISTRIBUTION_LOCK_SCHEMA || stored.version != DISTRIBUTION_LOCK_VERSION {
        return Ok(Vec::new());
    }
    Ok(stored.distributions)
}

fn load_artifact_cache_state(path: &Path) -> Result<Vec<SkillArtifactCacheEntry>, SkillError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let stored =
        serde_json::from_str::<StoredSkillArtifactCacheState>(&content).map_err(|error| {
            SkillError::ReadFailed {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    if stored.schema != ARTIFACT_CACHE_SCHEMA || stored.version != ARTIFACT_CACHE_VERSION {
        return Ok(Vec::new());
    }
    Ok(stored.artifact_cache)
}

fn persist_distribution_lock(
    path: &Path,
    distributions: &[SkillDistributionRecord],
) -> Result<(), SkillError> {
    let payload = serde_json::to_string_pretty(&StoredSkillDistributionLock {
        schema: DISTRIBUTION_LOCK_SCHEMA.to_string(),
        version: DISTRIBUTION_LOCK_VERSION,
        distributions: distributions.to_vec(),
    })
    .map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    fs::write(path, payload).map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn persist_artifact_cache_state(
    path: &Path,
    artifact_cache: &[SkillArtifactCacheEntry],
) -> Result<(), SkillError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillError::WriteFailed {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    let payload = serde_json::to_string_pretty(&StoredSkillArtifactCacheState {
        schema: ARTIFACT_CACHE_SCHEMA.to_string(),
        version: ARTIFACT_CACHE_VERSION,
        artifact_cache: artifact_cache.to_vec(),
    })
    .map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    fs::write(path, payload).map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn load_lifecycle_state(path: &Path) -> Result<Vec<SkillManagedLifecycleRecord>, SkillError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let stored = serde_json::from_str::<StoredSkillLifecycleState>(&content).map_err(|error| {
        SkillError::ReadFailed {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if stored.schema != LIFECYCLE_SCHEMA || stored.version != LIFECYCLE_VERSION {
        return Ok(Vec::new());
    }
    Ok(stored.lifecycle)
}

fn persist_lifecycle_state(
    path: &Path,
    lifecycle: &[SkillManagedLifecycleRecord],
) -> Result<(), SkillError> {
    let payload = serde_json::to_string_pretty(&StoredSkillLifecycleState {
        schema: LIFECYCLE_SCHEMA.to_string(),
        version: LIFECYCLE_VERSION,
        lifecycle: lifecycle.to_vec(),
    })
    .map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    fs::write(path, payload).map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn merge_cached_remote_source_indices(
    source_indices: Vec<SkillSourceIndexSnapshot>,
    index_cache_dir: &Path,
) -> Vec<SkillSourceIndexSnapshot> {
    let mut merged = source_indices
        .into_iter()
        .map(|snapshot| (snapshot.source.source_id.clone(), snapshot))
        .collect::<std::collections::BTreeMap<_, _>>();

    for cached in load_cached_remote_source_indices(index_cache_dir) {
        merged.insert(cached.source.source_id.clone(), cached);
    }

    merged.into_values().collect()
}

fn load_cached_remote_source_indices(index_cache_dir: &Path) -> Vec<SkillSourceIndexSnapshot> {
    let Ok(entries) = fs::read_dir(index_cache_dir) else {
        return Vec::new();
    };

    let mut snapshots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        match load_source_index_cache_record(&path) {
            Ok(Some(record)) => snapshots.push(record.snapshot),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, path=%path.display(), "failed to load cached remote source index");
            }
        }
    }
    snapshots.sort_by(|left, right| left.source.source_id.cmp(&right.source.source_id));
    snapshots
}

fn load_source_index_cache_record(
    path: &Path,
) -> Result<Option<StoredSourceIndexCacheRecord>, SkillError> {
    let content = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let record =
        serde_json::from_str::<StoredSourceIndexCacheRecord>(&content).map_err(|error| {
            SkillError::ReadFailed {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    if record.schema != SOURCE_INDEX_CACHE_SCHEMA || record.version != SOURCE_INDEX_CACHE_VERSION {
        return Ok(None);
    }
    Ok(Some(record))
}

fn persist_source_index_cache(
    path: &Path,
    record: &StoredSourceIndexCacheRecord,
) -> Result<(), SkillError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillError::WriteFailed {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    let payload =
        serde_json::to_string_pretty(record).map_err(|error| SkillError::WriteFailed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    fs::write(path, payload).map_err(|error| SkillError::WriteFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn source_index_cache_path(index_cache_dir: &Path, source_id: &str) -> PathBuf {
    index_cache_dir.join(format!("{}.json", source_index_cache_key(source_id)))
}

fn source_index_cache_key(source_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_id.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}
