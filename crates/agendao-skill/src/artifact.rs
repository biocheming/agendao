use crate::discovery::{parse_skill_file, read_skill_body};
use crate::distribution::verify_sha256_checksum;
use crate::hub::governance_dir;
use crate::write::build_skill_document;
use crate::{SkillError, SkillRoot};
use flate2::read::GzDecoder;
use agendao_config::ConfigStore;
use agendao_types::{
    SkillArtifactCacheEntry, SkillArtifactCacheStatus, SkillArtifactKind, SkillArtifactRef,
    SkillHubPolicy,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const ARTIFACT_CACHE_DIRNAME: &str = "artifact-cache";
const DEFAULT_ARTIFACT_CACHE_RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
const DEFAULT_FETCH_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_MAX_EXTRACT_BYTES: u64 = 8 * 1024 * 1024;

pub struct SkillArtifactStore {
    base_dir: PathBuf,
    config_store: Option<Arc<ConfigStore>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SkillArtifactPackage {
    pub skill_name: String,
    pub description: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub markdown_content: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub directory_name: Option<String>,
    #[serde(default)]
    pub supporting_files: Vec<SkillArtifactFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SkillArtifactFile {
    pub relative_path: String,
    pub content: String,
}

impl SkillArtifactStore {
    pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self {
        Self {
            base_dir: base_dir.into(),
            config_store,
        }
    }

    pub fn artifact_cache_dir(&self) -> PathBuf {
        governance_dir(&self.base_dir).join(ARTIFACT_CACHE_DIRNAME)
    }

    pub fn policy(&self) -> SkillHubPolicy {
        let config = self.config_store.as_ref().map(|store| store.config());
        let hub = config
            .as_deref()
            .and_then(|config| config.skills.as_ref())
            .and_then(|skills| skills.hub.as_ref());
        SkillHubPolicy {
            artifact_cache_retention_seconds: hub
                .and_then(|hub| hub.artifact_cache_retention_seconds)
                .unwrap_or(DEFAULT_ARTIFACT_CACHE_RETENTION_SECONDS),
            fetch_timeout_ms: hub
                .and_then(|hub| hub.fetch_timeout_ms)
                .unwrap_or(DEFAULT_FETCH_TIMEOUT_MS),
            max_download_bytes: hub
                .and_then(|hub| hub.max_download_bytes)
                .unwrap_or(DEFAULT_MAX_DOWNLOAD_BYTES),
            max_extract_bytes: hub
                .and_then(|hub| hub.max_extract_bytes)
                .unwrap_or(DEFAULT_MAX_EXTRACT_BYTES),
        }
    }

    pub fn evict_expired_entries(
        &self,
        entries: &[SkillArtifactCacheEntry],
    ) -> Result<Vec<SkillArtifactCacheEntry>, SkillError> {
        let policy = self.policy();
        let now = now_unix_timestamp();
        let mut retained = Vec::with_capacity(entries.len());
        for entry in entries {
            let age_seconds = now.saturating_sub(entry.cached_at);
            if age_seconds > policy.artifact_cache_retention_seconds as i64 {
                remove_cached_artifact_path(Path::new(entry.local_path.trim()))?;
                if let Some(extracted_path) = entry.extracted_path.as_deref() {
                    if extracted_path.trim() != entry.local_path.trim() {
                        remove_cached_artifact_path(Path::new(extracted_path.trim()))?;
                    }
                }
                continue;
            }
            retained.push(entry.clone());
        }
        Ok(retained)
    }

    pub fn fetch_artifact(
        &self,
        artifact: &SkillArtifactRef,
    ) -> Result<SkillArtifactCacheEntry, SkillError> {
        match artifact.kind {
            SkillArtifactKind::RegistryPackage => {
                let source_path = resolve_locator_path(&self.base_dir, &artifact.locator);
                if source_path.is_dir() {
                    self.materialize_directory_artifact(artifact)
                } else {
                    self.fetch_file_artifact(artifact)
                }
            }
            SkillArtifactKind::Archive => self.fetch_archive_artifact(artifact),
            SkillArtifactKind::GitCheckout | SkillArtifactKind::LocalSnapshot => {
                self.materialize_directory_artifact(artifact)
            }
        }
    }

    pub(crate) fn load_package(
        &self,
        cache_entry: &SkillArtifactCacheEntry,
    ) -> Result<SkillArtifactPackage, SkillError> {
        let policy = self.policy();
        match cache_entry.artifact.kind {
            SkillArtifactKind::RegistryPackage => cache_entry
                .extracted_path
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| {
                    let local_path = PathBuf::from(cache_entry.local_path.trim());
                    local_path.is_dir().then_some(local_path)
                })
                .map(|path| load_package_from_skill_root(&path, policy.max_extract_bytes))
                .unwrap_or_else(|| {
                    load_package_from_json_path(Path::new(cache_entry.local_path.trim()))
                }),
            SkillArtifactKind::Archive => {
                if let Some(extracted_path) = cache_entry.extracted_path.as_deref() {
                    return load_package_from_skill_root(
                        Path::new(extracted_path),
                        policy.max_extract_bytes,
                    );
                }
                let path = Path::new(cache_entry.local_path.trim());
                if path.is_dir() {
                    return load_package_from_skill_root(path, policy.max_extract_bytes);
                }
                load_package_from_archive_file(path, policy.max_extract_bytes)
            }
            SkillArtifactKind::GitCheckout | SkillArtifactKind::LocalSnapshot => {
                let path = cache_entry
                    .extracted_path
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(cache_entry.local_path.trim()));
                load_package_from_skill_root(&path, policy.max_extract_bytes)
            }
        }
    }

    fn fetch_file_artifact(
        &self,
        artifact: &SkillArtifactRef,
    ) -> Result<SkillArtifactCacheEntry, SkillError> {
        let policy = self.policy();
        if let Some(size_bytes) = artifact.size_bytes {
            if size_bytes > policy.max_download_bytes {
                return Err(SkillError::ArtifactDownloadSizeExceeded {
                    locator: artifact.locator.clone(),
                    size: size_bytes,
                    limit: policy.max_download_bytes,
                });
            }
        }
        let bytes = load_artifact_payload(&self.base_dir, artifact, &policy)?;
        verify_sha256_checksum(&bytes, artifact.checksum.as_deref())?;

        let cache_path = artifact_cache_path(&self.artifact_cache_dir(), &artifact.artifact_id);
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).map_err(|error| SkillError::WriteFailed {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        fs::write(&cache_path, &bytes).map_err(|error| SkillError::WriteFailed {
            path: cache_path.clone(),
            message: error.to_string(),
        })?;

        Ok(SkillArtifactCacheEntry {
            artifact: artifact.clone(),
            cached_at: now_unix_timestamp(),
            local_path: cache_path.to_string_lossy().to_string(),
            extracted_path: None,
            status: SkillArtifactCacheStatus::Fetched,
            error: None,
        })
    }

    fn fetch_archive_artifact(
        &self,
        artifact: &SkillArtifactRef,
    ) -> Result<SkillArtifactCacheEntry, SkillError> {
        let source_path = resolve_locator_path(&self.base_dir, &artifact.locator);
        if source_path.is_dir() {
            return self.materialize_directory_artifact(artifact);
        }
        self.fetch_file_artifact(artifact)
    }

    fn materialize_directory_artifact(
        &self,
        artifact: &SkillArtifactRef,
    ) -> Result<SkillArtifactCacheEntry, SkillError> {
        let policy = self.policy();
        let source_path = resolve_locator_path(&self.base_dir, &artifact.locator);
        if !source_path.exists() || !source_path.is_dir() {
            return Err(SkillError::ReadFailed {
                path: source_path,
                message: "directory-backed artifact locator was not found".to_string(),
            });
        }

        let materialized_path =
            artifact_materialized_path(&self.artifact_cache_dir(), &artifact.artifact_id);
        if materialized_path.exists() {
            fs::remove_dir_all(&materialized_path).map_err(|error| SkillError::WriteFailed {
                path: materialized_path.clone(),
                message: error.to_string(),
            })?;
        }
        fs::create_dir_all(&materialized_path).map_err(|error| SkillError::WriteFailed {
            path: materialized_path.clone(),
            message: error.to_string(),
        })?;
        copy_dir_recursive(&source_path, &materialized_path, policy.max_extract_bytes)?;

        Ok(SkillArtifactCacheEntry {
            artifact: artifact.clone(),
            cached_at: now_unix_timestamp(),
            local_path: materialized_path.to_string_lossy().to_string(),
            extracted_path: Some(materialized_path.to_string_lossy().to_string()),
            status: SkillArtifactCacheStatus::Extracted,
            error: None,
        })
    }
}

impl SkillArtifactPackage {
    pub(crate) fn markdown_content(&self) -> String {
        if let Some(markdown_content) = self
            .markdown_content
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return markdown_content.to_string();
        }

        build_skill_document(
            &crate::write::build_create_frontmatter(&self.skill_name, &self.description, None)
                .expect("artifact package frontmatter should be valid"),
            self.body.as_deref().unwrap_or_default(),
        )
        .expect("artifact package markdown should render")
    }
}

fn load_artifact_payload(
    base_dir: &Path,
    artifact: &SkillArtifactRef,
    policy: &SkillHubPolicy,
) -> Result<Vec<u8>, SkillError> {
    #[cfg(test)]
    if let Some(delay_ms) = parse_test_timeout_locator(&artifact.locator) {
        std::thread::sleep(Duration::from_millis(delay_ms.min(5)));
        if delay_ms > policy.fetch_timeout_ms {
            return Err(SkillError::ArtifactFetchTimeout {
                locator: artifact.locator.clone(),
                timeout_ms: policy.fetch_timeout_ms,
            });
        }
        return Ok(br#"{"test":"artifact"}"#.to_vec());
    }

    if is_http_locator(&artifact.locator) {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(policy.fetch_timeout_ms))
            .build()
            .map_err(|error| SkillError::ReadFailed {
                path: PathBuf::from(artifact.locator.clone()),
                message: format!("failed to build artifact client: {error}"),
            })?;
        let response = client.get(&artifact.locator).send().map_err(|error| {
            if error.is_timeout() {
                SkillError::ArtifactFetchTimeout {
                    locator: artifact.locator.clone(),
                    timeout_ms: policy.fetch_timeout_ms,
                }
            } else {
                SkillError::ReadFailed {
                    path: PathBuf::from(artifact.locator.clone()),
                    message: format!("failed to fetch artifact: {error}"),
                }
            }
        })?;
        let mut response = response
            .error_for_status()
            .map_err(|error| SkillError::ReadFailed {
                path: PathBuf::from(artifact.locator.clone()),
                message: format!("artifact request failed: {error}"),
            })?;
        if let Some(content_length) = response.content_length() {
            if content_length > policy.max_download_bytes {
                return Err(SkillError::ArtifactDownloadSizeExceeded {
                    locator: artifact.locator.clone(),
                    size: content_length,
                    limit: policy.max_download_bytes,
                });
            }
        }
        read_limited_bytes(&mut response, policy.max_download_bytes, &artifact.locator)
    } else {
        let path = resolve_locator_path(base_dir, &artifact.locator);
        if let Ok(metadata) = fs::metadata(&path) {
            if metadata.len() > policy.max_download_bytes {
                return Err(SkillError::ArtifactDownloadSizeExceeded {
                    locator: path.to_string_lossy().to_string(),
                    size: metadata.len(),
                    limit: policy.max_download_bytes,
                });
            }
        }
        let bytes = fs::read(&path).map_err(|error| SkillError::ReadFailed {
            path,
            message: error.to_string(),
        })?;
        if bytes.len() as u64 > policy.max_download_bytes {
            return Err(SkillError::ArtifactDownloadSizeExceeded {
                locator: artifact.locator.clone(),
                size: bytes.len() as u64,
                limit: policy.max_download_bytes,
            });
        }
        Ok(bytes)
    }
}

fn load_package_from_json_path(path: &Path) -> Result<SkillArtifactPackage, SkillError> {
    let payload = fs::read_to_string(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let package = serde_json::from_str::<SkillArtifactPackage>(&payload).map_err(|error| {
        SkillError::ReadFailed {
            path: path.to_path_buf(),
            message: format!("failed to parse artifact package: {error}"),
        }
    })?;
    normalize_package(package, path)
}

fn load_package_from_archive_file(
    path: &Path,
    max_extract_bytes: u64,
) -> Result<SkillArtifactPackage, SkillError> {
    let bytes = fs::read(path).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let payload = decode_archive_payload(path, &bytes, max_extract_bytes)?;
    let package = serde_json::from_str::<SkillArtifactPackage>(&payload).map_err(|error| {
        SkillError::ReadFailed {
            path: path.to_path_buf(),
            message: format!("failed to parse archive artifact package: {error}"),
        }
    })?;
    normalize_package(package, path)
}

fn decode_archive_payload(
    path: &Path,
    bytes: &[u8],
    max_extract_bytes: u64,
) -> Result<String, SkillError> {
    let is_gzip = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("gz") || extension.eq_ignore_ascii_case("tgz")
        })
        .unwrap_or(false);
    if !is_gzip {
        if bytes.len() as u64 > max_extract_bytes {
            return Err(SkillError::ArtifactExtractSizeExceeded {
                path: path.to_path_buf(),
                size: bytes.len() as u64,
                limit: max_extract_bytes,
            });
        }
        return String::from_utf8(bytes.to_vec()).map_err(|error| SkillError::ReadFailed {
            path: path.to_path_buf(),
            message: format!("archive payload was not valid utf-8: {error}"),
        });
    }

    let decoder = GzDecoder::new(bytes);
    let mut payload = Vec::new();
    decoder
        .take(max_extract_bytes.saturating_add(1))
        .read_to_end(&mut payload)
        .map_err(|error| SkillError::ReadFailed {
            path: path.to_path_buf(),
            message: format!("failed to decode gzip archive payload: {error}"),
        })?;
    if payload.len() as u64 > max_extract_bytes {
        return Err(SkillError::ArtifactExtractSizeExceeded {
            path: path.to_path_buf(),
            size: payload.len() as u64,
            limit: max_extract_bytes,
        });
    }
    String::from_utf8(payload).map_err(|error| SkillError::ReadFailed {
        path: path.to_path_buf(),
        message: format!("archive payload was not valid utf-8: {error}"),
    })
}

fn load_package_from_skill_root(
    root: &Path,
    max_extract_bytes: u64,
) -> Result<SkillArtifactPackage, SkillError> {
    directory_total_size(root, max_extract_bytes)?;
    let skill_file = locate_skill_file(root)?;
    let skill_dir = skill_file.parent().ok_or_else(|| SkillError::ReadFailed {
        path: skill_file.clone(),
        message: "skill file was missing a parent directory".to_string(),
    })?;
    let meta = parse_skill_file(
        &skill_file,
        &SkillRoot {
            path: root.to_path_buf(),
        },
    )
    .ok_or_else(|| SkillError::ArtifactLayoutMismatch {
        path: skill_file.clone(),
        message: format!(
            "materialized artifact `{}` did not contain a valid SKILL.md",
            root.display()
        ),
    })?;
    let markdown_content =
        fs::read_to_string(&skill_file).map_err(|error| SkillError::ReadFailed {
            path: skill_file.clone(),
            message: error.to_string(),
        })?;
    let supporting_files = meta
        .supporting_files
        .iter()
        .map(|file| {
            fs::read_to_string(&file.location)
                .map(|content| SkillArtifactFile {
                    relative_path: file.relative_path.clone(),
                    content,
                })
                .map_err(|error| SkillError::ReadFailed {
                    path: file.location.clone(),
                    message: error.to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    normalize_package(
        SkillArtifactPackage {
            skill_name: meta.name,
            description: meta.description,
            body: Some(
                read_skill_body(&skill_file).map_err(|error| SkillError::ReadFailed {
                    path: skill_file.clone(),
                    message: error.to_string(),
                })?,
            ),
            markdown_content: Some(markdown_content.replace("\r\n", "\n")),
            category: meta.category,
            directory_name: skill_dir
                .file_name()
                .map(|value| value.to_string_lossy().to_string()),
            supporting_files,
        },
        root,
    )
}

fn normalize_package(
    mut package: SkillArtifactPackage,
    path: &Path,
) -> Result<SkillArtifactPackage, SkillError> {
    package.skill_name = package.skill_name.trim().to_string();
    package.description = package.description.trim().to_string();
    package.category = package
        .category
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    package.directory_name = package
        .directory_name
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    for file in &mut package.supporting_files {
        file.relative_path = file.relative_path.replace('\\', "/").trim().to_string();
    }
    package
        .supporting_files
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    if package.skill_name.is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: format!(
                "artifact package `{}` did not contain a valid skill_name",
                path.display()
            ),
        });
    }
    if package.description.is_empty() {
        return Err(SkillError::InvalidSkillDescription {
            name: package.skill_name.clone(),
        });
    }

    Ok(package)
}

fn locate_skill_file(root: &Path) -> Result<PathBuf, SkillError> {
    let direct = root.join("SKILL.md");
    if direct.is_file() {
        return Ok(direct);
    }

    let mut skill_files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            entry
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| *name == "SKILL.md")
                .map(|_| entry.path().to_path_buf())
        })
        .collect::<Vec<_>>();
    skill_files.sort();

    match skill_files.len() {
        1 => Ok(skill_files.remove(0)),
        0 => Err(SkillError::ArtifactLayoutMismatch {
            path: root.to_path_buf(),
            message: format!(
                "materialized artifact `{}` did not contain a SKILL.md file",
                root.display()
            ),
        }),
        _ => Err(SkillError::ArtifactLayoutMismatch {
            path: root.to_path_buf(),
            message: format!(
                "materialized artifact `{}` contained multiple SKILL.md files",
                root.display()
            ),
        }),
    }
}

fn copy_dir_recursive(
    source: &Path,
    destination: &Path,
    max_extract_bytes: u64,
) -> Result<(), SkillError> {
    let mut copied_bytes = 0_u64;
    for entry in WalkDir::new(source).follow_links(false).into_iter() {
        let entry = entry.map_err(|error| SkillError::ReadFailed {
            path: source.to_path_buf(),
            message: error.to_string(),
        })?;
        let relative =
            entry
                .path()
                .strip_prefix(source)
                .map_err(|error| SkillError::ReadFailed {
                    path: entry.path().to_path_buf(),
                    message: error.to_string(),
                })?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|error| SkillError::WriteFailed {
                path: target,
                message: error.to_string(),
            })?;
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| SkillError::WriteFailed {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        let file_size = entry
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| SkillError::ReadFailed {
                path: entry.path().to_path_buf(),
                message: error.to_string(),
            })?;
        copied_bytes = copied_bytes.saturating_add(file_size);
        if copied_bytes > max_extract_bytes {
            return Err(SkillError::ArtifactExtractSizeExceeded {
                path: source.to_path_buf(),
                size: copied_bytes,
                limit: max_extract_bytes,
            });
        }
        fs::copy(entry.path(), &target).map_err(|error| SkillError::WriteFailed {
            path: target,
            message: error.to_string(),
        })?;
    }

    Ok(())
}

fn artifact_cache_path(cache_dir: &Path, artifact_id: &str) -> PathBuf {
    let key = artifact_cache_key(artifact_id);
    cache_dir.join(format!("{key}.artifact"))
}

fn artifact_materialized_path(cache_dir: &Path, artifact_id: &str) -> PathBuf {
    cache_dir.join(format!("{}.extracted", artifact_cache_key(artifact_id)))
}

fn artifact_cache_key(artifact_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(artifact_id.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn resolve_locator_path(base_dir: &Path, locator: &str) -> PathBuf {
    let path = PathBuf::from(locator);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn is_http_locator(locator: &str) -> bool {
    locator.starts_with("http://") || locator.starts_with("https://")
}

#[cfg(test)]
fn parse_test_timeout_locator(locator: &str) -> Option<u64> {
    locator
        .trim()
        .strip_prefix("test+timeout://")
        .and_then(|value| value.parse::<u64>().ok())
}

fn now_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn read_limited_bytes<R: Read>(
    reader: &mut R,
    limit: u64,
    locator: &str,
) -> Result<Vec<u8>, SkillError> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| SkillError::ReadFailed {
            path: PathBuf::from(locator),
            message: format!("failed to read artifact body: {error}"),
        })?;
    if bytes.len() as u64 > limit {
        return Err(SkillError::ArtifactDownloadSizeExceeded {
            locator: locator.to_string(),
            size: bytes.len() as u64,
            limit,
        });
    }
    Ok(bytes)
}

fn directory_total_size(root: &Path, limit: u64) -> Result<u64, SkillError> {
    let mut total = 0_u64;
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = entry.map_err(|error| SkillError::ReadFailed {
            path: root.to_path_buf(),
            message: error.to_string(),
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let size = entry
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| SkillError::ReadFailed {
                path: entry.path().to_path_buf(),
                message: error.to_string(),
            })?;
        total = total.saturating_add(size);
        if total > limit {
            return Err(SkillError::ArtifactExtractSizeExceeded {
                path: root.to_path_buf(),
                size: total,
                limit,
            });
        }
    }
    Ok(total)
}

fn remove_cached_artifact_path(path: &Path) -> Result<(), SkillError> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| SkillError::WriteFailed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    } else {
        fs::remove_file(path).map_err(|error| SkillError::WriteFailed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    Ok(())
}
