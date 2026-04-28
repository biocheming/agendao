use crate::SkillError;
use rocode_types::{
    SkillArtifactKind, SkillArtifactRef, SkillDistributionRecord, SkillDistributionRelease,
    SkillDistributionResolution, SkillDistributionResolverKind, SkillManagedLifecycleState,
    SkillSourceIndexEntry, SkillSourceIndexSnapshot, SkillSourceKind, SkillSourceRef,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Default)]
pub struct SkillDistributionResolver;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedSkillDistribution {
    pub record: SkillDistributionRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SkillInstallManifest {
    pub skill_name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub published_at: Option<i64>,
    pub artifact: SkillInstallManifestArtifact,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SkillInstallManifestArtifact {
    #[serde(default)]
    pub artifact_id: Option<String>,
    pub locator: String,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

impl SkillDistributionResolver {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn resolve_distribution(
        &self,
        base_dir: &Path,
        source: &SkillSourceRef,
        source_index: &SkillSourceIndexSnapshot,
        skill_name: &str,
        fetch_timeout_ms: u64,
    ) -> Result<ResolvedSkillDistribution, SkillError> {
        let (resolver_kind, artifact_kind) = match source.source_kind {
            SkillSourceKind::Registry => (
                SkillDistributionResolverKind::RegistryManifest,
                SkillArtifactKind::RegistryPackage,
            ),
            SkillSourceKind::Archive => (
                SkillDistributionResolverKind::ArchiveManifest,
                SkillArtifactKind::Archive,
            ),
            SkillSourceKind::Git => (
                SkillDistributionResolverKind::GitCheckout,
                SkillArtifactKind::GitCheckout,
            ),
            _ => {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                    "remote distribution resolve requires registry/archive/git source, got {:?}",
                    source.source_kind
                ),
                })
            }
        };

        self.resolve_manifest_distribution(
            base_dir,
            source,
            source_index,
            skill_name,
            resolver_kind,
            artifact_kind,
            fetch_timeout_ms,
        )
    }

    fn resolve_manifest_distribution(
        &self,
        base_dir: &Path,
        source: &SkillSourceRef,
        source_index: &SkillSourceIndexSnapshot,
        skill_name: &str,
        resolver_kind: SkillDistributionResolverKind,
        artifact_kind: SkillArtifactKind,
        fetch_timeout_ms: u64,
    ) -> Result<ResolvedSkillDistribution, SkillError> {
        let entry = source_index
            .entries
            .iter()
            .find(|entry| entry.skill_name.eq_ignore_ascii_case(skill_name))
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{skill_name}` was not found in source index `{}`",
                    source.source_id
                ),
            })?;

        let manifest_locator = remote_manifest_locator(base_dir, source, entry);
        let manifest = load_install_manifest(base_dir, &manifest_locator, fetch_timeout_ms)?;
        let now = now_unix_timestamp();
        let normalized_skill_name = manifest.skill_name.trim().to_string();
        if normalized_skill_name.is_empty() {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "install manifest `{manifest_locator}` did not contain a skill_name"
                ),
            });
        }
        if !normalized_skill_name.eq_ignore_ascii_case(skill_name) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "install manifest resolved `{}` but requested `{skill_name}`",
                    normalized_skill_name
                ),
            });
        }

        let artifact_locator = remote_artifact_locator(base_dir, &manifest_locator, &manifest)?;
        let artifact =
            SkillArtifactRef {
                artifact_id: manifest.artifact.artifact_id.clone().unwrap_or_else(|| {
                    default_artifact_id(source, &normalized_skill_name, &manifest)
                }),
                kind: artifact_kind,
                locator: artifact_locator,
                checksum: manifest
                    .artifact
                    .checksum
                    .clone()
                    .or_else(|| manifest.checksum.clone())
                    .or_else(|| entry.checksum.clone()),
                size_bytes: manifest.artifact.size_bytes,
            };
        let release = SkillDistributionRelease {
            version: manifest.version.clone().or_else(|| entry.version.clone()),
            revision: manifest
                .revision
                .clone()
                .or_else(|| entry.revision.clone())
                .or_else(|| source.revision.clone()),
            checksum: manifest.checksum.clone().or_else(|| entry.checksum.clone()),
            manifest_path: Some(manifest_locator.clone()),
            published_at: manifest.published_at,
        };
        let record = SkillDistributionRecord {
            distribution_id: distribution_id(source, &normalized_skill_name, &release),
            source: source.clone(),
            skill_name: normalized_skill_name,
            release,
            resolution: SkillDistributionResolution {
                resolved_at: now,
                resolver_kind,
                artifact,
            },
            installed: None,
            lifecycle: SkillManagedLifecycleState::Resolved,
        };

        Ok(ResolvedSkillDistribution { record })
    }
}

fn load_install_manifest(
    base_dir: &Path,
    locator: &str,
    fetch_timeout_ms: u64,
) -> Result<SkillInstallManifest, SkillError> {
    let payload = if is_http_locator(locator) {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(fetch_timeout_ms))
            .build()
            .map_err(|error| SkillError::ReadFailed {
                path: PathBuf::from(locator),
                message: format!("failed to build install manifest client: {error}"),
            })?;
        let response = client.get(locator).send().map_err(|error| {
            if error.is_timeout() {
                SkillError::ArtifactFetchTimeout {
                    locator: locator.to_string(),
                    timeout_ms: fetch_timeout_ms,
                }
            } else {
                SkillError::ReadFailed {
                    path: PathBuf::from(locator),
                    message: format!("failed to fetch install manifest: {error}"),
                }
            }
        })?;
        let response = response
            .error_for_status()
            .map_err(|error| SkillError::ReadFailed {
                path: PathBuf::from(locator),
                message: format!("install manifest request failed: {error}"),
            })?;
        response.text().map_err(|error| {
            if error.is_timeout() {
                SkillError::ArtifactFetchTimeout {
                    locator: locator.to_string(),
                    timeout_ms: fetch_timeout_ms,
                }
            } else {
                SkillError::ReadFailed {
                    path: PathBuf::from(locator),
                    message: format!("failed to read install manifest body: {error}"),
                }
            }
        })?
    } else {
        let path = resolve_locator_path(base_dir, locator);
        fs::read_to_string(&path).map_err(|error| SkillError::ReadFailed {
            path,
            message: error.to_string(),
        })?
    };

    serde_json::from_str::<SkillInstallManifest>(&payload).map_err(|error| SkillError::ReadFailed {
        path: PathBuf::from(locator),
        message: format!("failed to parse install manifest: {error}"),
    })
}

fn remote_manifest_locator(
    base_dir: &Path,
    source: &SkillSourceRef,
    entry: &SkillSourceIndexEntry,
) -> String {
    if let Some(manifest_path) = entry
        .manifest_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if is_http_locator(manifest_path) || PathBuf::from(manifest_path).is_absolute() {
            return manifest_path.to_string();
        }
        if is_http_locator(&source.locator) {
            return join_url_path_like(&source.locator, manifest_path);
        }
        return derive_relative_locator(base_dir, &source.locator, manifest_path);
    }

    let default_name = format!("{}.json", normalize_component(&entry.skill_name));
    if is_http_locator(&source.locator) {
        join_url_path_like(&source.locator, &format!("manifests/{default_name}"))
    } else {
        derive_relative_locator(
            base_dir,
            &source.locator,
            &format!("manifests/{default_name}"),
        )
    }
}

fn remote_artifact_locator(
    base_dir: &Path,
    manifest_locator: &str,
    manifest: &SkillInstallManifest,
) -> Result<String, SkillError> {
    let locator = manifest.artifact.locator.trim();
    if locator.is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: format!(
                "install manifest `{manifest_locator}` did not contain an artifact locator"
            ),
        });
    }

    if is_http_locator(locator)
        || PathBuf::from(locator).is_absolute()
        || is_test_passthrough_locator(locator)
    {
        return Ok(locator.to_string());
    }

    if is_http_locator(manifest_locator) {
        Ok(join_url_path_like(manifest_locator, locator))
    } else {
        Ok(
            resolve_relative_to_file(base_dir, manifest_locator, locator)
                .to_string_lossy()
                .to_string(),
        )
    }
}

fn distribution_id(
    source: &SkillSourceRef,
    skill_name: &str,
    release: &SkillDistributionRelease,
) -> String {
    let release_hint = release
        .version
        .as_deref()
        .or(release.revision.as_deref())
        .unwrap_or("unversioned");
    format!(
        "dist:{}:{}:{}",
        normalize_component(&source.source_id),
        normalize_component(skill_name),
        normalize_component(release_hint)
    )
}

fn default_artifact_id(
    source: &SkillSourceRef,
    skill_name: &str,
    manifest: &SkillInstallManifest,
) -> String {
    let release_hint = manifest
        .version
        .as_deref()
        .or(manifest.revision.as_deref())
        .unwrap_or("artifact");
    format!(
        "artifact:{}:{}:{}",
        normalize_component(&source.source_id),
        normalize_component(skill_name),
        normalize_component(release_hint)
    )
}

fn is_test_passthrough_locator(locator: &str) -> bool {
    #[cfg(test)]
    {
        locator.trim().starts_with("test+timeout://")
    }
    #[cfg(not(test))]
    {
        let _ = locator;
        false
    }
}

fn normalize_component(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn derive_relative_locator(base_dir: &Path, source_locator: &str, suffix: &str) -> String {
    let root = resolve_registry_root(base_dir, source_locator);
    root.join(suffix).to_string_lossy().to_string()
}

fn resolve_relative_to_file(base_dir: &Path, base_locator: &str, relative: &str) -> PathBuf {
    let base_path = resolve_locator_path(base_dir, base_locator);
    let parent = if base_path.is_dir() {
        base_path
    } else {
        base_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| base_dir.to_path_buf())
    };
    parent.join(relative)
}

fn resolve_registry_root(base_dir: &Path, source_locator: &str) -> PathBuf {
    let path = resolve_locator_path(base_dir, source_locator);
    if path.is_dir() {
        path
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| base_dir.to_path_buf())
    }
}

fn resolve_locator_path(base_dir: &Path, locator: &str) -> PathBuf {
    let path = PathBuf::from(locator);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn join_url_path_like(base: &str, relative: &str) -> String {
    let trimmed_relative = relative.trim_start_matches('/');
    let base_without_query = base
        .split_once('?')
        .map(|(prefix, _)| prefix)
        .unwrap_or(base)
        .trim_end_matches('/');

    let base_prefix = if base_without_query.ends_with(".json")
        || base_without_query.ends_with(".tgz")
        || base_without_query.ends_with(".tar")
        || base_without_query.ends_with(".zip")
    {
        base_without_query
            .rsplit_once('/')
            .map(|(prefix, _)| prefix)
            .unwrap_or(base_without_query)
    } else {
        base_without_query
    };
    format!("{base_prefix}/{trimmed_relative}")
}

fn is_http_locator(locator: &str) -> bool {
    locator.starts_with("http://") || locator.starts_with("https://")
}

fn now_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn verify_sha256_checksum(
    bytes: &[u8],
    checksum: Option<&str>,
) -> Result<(), SkillError> {
    let Some(checksum) = checksum.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let expected = checksum
        .strip_prefix("sha256:")
        .unwrap_or(checksum)
        .trim()
        .to_ascii_lowercase();
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(SkillError::ArtifactChecksumMismatch { expected, actual });
    }
    Ok(())
}
