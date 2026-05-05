use crate::models::{ModelsData, MODELS_DEV_URL};
use once_cell::sync::Lazy;
use reqwest::header::{CACHE_CONTROL, ETAG, IF_NONE_MATCH, PRAGMA};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

const CATALOG_USER_AGENT: &str = "rocode-rust";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogMetadata {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_refresh_at: Option<i64>,
    #[serde(default)]
    pub last_success_at: Option<i64>,
    #[serde(default)]
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogSnapshot {
    #[serde(default)]
    pub data: ModelsData,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub metadata: CatalogMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogRefreshStatus {
    Updated,
    NotModified,
    FallbackCached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRefreshResult {
    pub snapshot: CatalogSnapshot,
    pub status: CatalogRefreshStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub struct ModelCatalogAuthority {
    snapshot_path: PathBuf,
    metadata_path: PathBuf,
    snapshot: RwLock<Option<CatalogSnapshot>>,
    refresh_lock: Mutex<()>,
    next_generation: AtomicU64,
}

impl ModelCatalogAuthority {
    pub fn new(snapshot_path: PathBuf, metadata_path: PathBuf) -> Self {
        Self {
            snapshot_path,
            metadata_path,
            snapshot: RwLock::new(None),
            refresh_lock: Mutex::new(()),
            next_generation: AtomicU64::new(1),
        }
    }

    pub fn with_snapshot_path(snapshot_path: PathBuf) -> Self {
        let metadata_path = metadata_path_for_snapshot(&snapshot_path);
        Self::new(snapshot_path, metadata_path)
    }

    pub async fn snapshot(&self) -> CatalogSnapshot {
        if let Some(snapshot) = self.snapshot.read().await.clone() {
            return snapshot;
        }

        let _guard = self.refresh_lock.lock().await;
        if let Some(snapshot) = self.snapshot.read().await.clone() {
            return snapshot;
        }

        let loaded = self
            .load_cached_snapshot_sync()
            .unwrap_or_else(|| self.allocate_snapshot(HashMap::new(), CatalogMetadata::default()));
        self.store_snapshot(loaded.clone()).await;

        if loaded.data.is_empty() {
            return self.refresh_locked(Some(loaded), false).await.snapshot;
        }

        loaded
    }

    pub async fn data(&self) -> ModelsData {
        self.snapshot().await.data
    }

    pub async fn refresh(&self, force: bool) -> CatalogSnapshot {
        self.refresh_with_result(force).await.snapshot
    }

    pub async fn refresh_with_result(&self, force: bool) -> CatalogRefreshResult {
        let _guard = self.refresh_lock.lock().await;
        let current = self
            .snapshot
            .read()
            .await
            .clone()
            .or_else(|| self.load_cached_snapshot_sync());
        self.refresh_locked(current, force).await
    }

    pub fn load_cached_data_sync(&self) -> ModelsData {
        self.load_cached_snapshot_sync()
            .map(|snapshot| snapshot.data)
            .unwrap_or_default()
    }

    async fn refresh_locked(
        &self,
        current: Option<CatalogSnapshot>,
        force: bool,
    ) -> CatalogRefreshResult {
        let current = current
            .unwrap_or_else(|| self.allocate_snapshot(HashMap::new(), CatalogMetadata::default()));
        let now = chrono::Utc::now().timestamp_millis();
        let url = format!("{}/api.json", MODELS_DEV_URL);

        let mut request = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", CATALOG_USER_AGENT)
            .timeout(std::time::Duration::from_secs(10));

        if force {
            request = request
                .header(CACHE_CONTROL, "no-cache")
                .header(PRAGMA, "no-cache");
        } else if let Some(etag) = current.metadata.etag.as_deref() {
            request = request.header(IF_NONE_MATCH, etag);
        }

        let result = match request.send().await {
            Ok(response) if response.status() == reqwest::StatusCode::NOT_MODIFIED => {
                let mut snapshot = current.clone();
                snapshot.metadata.last_refresh_at = Some(now);
                if snapshot.metadata.source_url.is_empty() {
                    snapshot.metadata.source_url = url.clone();
                }
                if let Err(error) = self.write_metadata(&snapshot.metadata).await {
                    tracing::debug!(
                        path = %self.metadata_path.display(),
                        error = %error,
                        "Failed to write models catalogue metadata"
                    );
                }
                CatalogRefreshResult {
                    snapshot,
                    status: CatalogRefreshStatus::NotModified,
                    error_message: None,
                }
            }
            Ok(response) if response.status().is_success() => {
                let etag = response
                    .headers()
                    .get(ETAG)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                match response.text().await {
                    Ok(text) => match serde_json::from_str::<ModelsData>(&text) {
                        Ok(parsed) => {
                            let metadata = CatalogMetadata {
                                etag,
                                last_refresh_at: Some(now),
                                last_success_at: Some(now),
                                source_url: url.clone(),
                            };
                            let snapshot = self.allocate_snapshot(parsed, metadata);
                            if let Err(error) = self.write_snapshot_text(&text).await {
                                tracing::debug!(
                                    path = %self.snapshot_path.display(),
                                    error = %error,
                                    "Failed to write models catalogue snapshot"
                                );
                            }
                            if let Err(error) = self.write_metadata(&snapshot.metadata).await {
                                tracing::debug!(
                                    path = %self.metadata_path.display(),
                                    error = %error,
                                    "Failed to write models catalogue metadata"
                                );
                            }
                            CatalogRefreshResult {
                                snapshot,
                                status: CatalogRefreshStatus::Updated,
                                error_message: None,
                            }
                        }
                        Err(error) => {
                            tracing::debug!(%error, "Failed to parse models.dev response");
                            CatalogRefreshResult {
                                snapshot: current.clone(),
                                status: CatalogRefreshStatus::FallbackCached,
                                error_message: Some(format!(
                                    "Failed to parse models.dev response: {}",
                                    error
                                )),
                            }
                        }
                    },
                    Err(error) => {
                        tracing::debug!(%error, "Failed to read models.dev response body");
                        CatalogRefreshResult {
                            snapshot: current.clone(),
                            status: CatalogRefreshStatus::FallbackCached,
                            error_message: Some(format!(
                                "Failed to read models.dev response body: {}",
                                error
                            )),
                        }
                    }
                }
            }
            Ok(response) => {
                tracing::debug!(
                    status = %response.status(),
                    "models.dev request did not return success"
                );
                CatalogRefreshResult {
                    snapshot: current.clone(),
                    status: CatalogRefreshStatus::FallbackCached,
                    error_message: Some(format!("models.dev returned HTTP {}", response.status())),
                }
            }
            Err(error) => {
                tracing::debug!(%error, "Failed to refresh models.dev catalogue");
                CatalogRefreshResult {
                    snapshot: current.clone(),
                    status: CatalogRefreshStatus::FallbackCached,
                    error_message: Some(format!(
                        "Failed to refresh models.dev catalogue: {}",
                        error
                    )),
                }
            }
        };

        self.store_snapshot(result.snapshot.clone()).await;
        result
    }

    async fn store_snapshot(&self, snapshot: CatalogSnapshot) {
        self.next_generation
            .fetch_max(snapshot.generation.saturating_add(1), Ordering::SeqCst);
        *self.snapshot.write().await = Some(snapshot);
    }

    fn load_cached_snapshot_sync(&self) -> Option<CatalogSnapshot> {
        let data = load_catalog_data_from_path_sync(&self.snapshot_path);
        if data.is_empty() {
            return None;
        }

        let metadata = read_metadata_sync(&self.metadata_path).unwrap_or_else(|| CatalogMetadata {
            etag: None,
            last_refresh_at: None,
            last_success_at: None,
            source_url: format!("{}/api.json", MODELS_DEV_URL),
        });

        Some(self.allocate_snapshot(data, metadata))
    }

    fn allocate_snapshot(&self, data: ModelsData, metadata: CatalogMetadata) -> CatalogSnapshot {
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        CatalogSnapshot {
            data,
            generation,
            metadata,
        }
    }

    async fn write_snapshot_text(&self, text: &str) -> std::io::Result<()> {
        if let Some(parent) = self.snapshot_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&self.snapshot_path, text).await
    }

    async fn write_metadata(&self, metadata: &CatalogMetadata) -> std::io::Result<()> {
        if let Some(parent) = self.metadata_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let raw = serde_json::to_vec_pretty(metadata).unwrap_or_default();
        tokio::fs::write(&self.metadata_path, raw).await
    }
}

pub fn default_model_catalog_authority() -> Arc<ModelCatalogAuthority> {
    static AUTHORITY: Lazy<Arc<ModelCatalogAuthority>> = Lazy::new(|| {
        Arc::new(ModelCatalogAuthority::new(
            default_catalog_snapshot_path(),
            default_catalog_metadata_path(),
        ))
    });

    AUTHORITY.clone()
}

pub fn default_catalog_snapshot_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("rocode")
        .join("catalog")
        .join("models.snapshot.json")
}

pub fn default_catalog_metadata_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("rocode")
        .join("catalog")
        .join("models.meta.json")
}

pub fn metadata_path_for_snapshot(snapshot_path: &Path) -> PathBuf {
    let parent = snapshot_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = snapshot_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("models");
    parent.join(format!("{stem}.meta.json"))
}

pub fn load_default_catalog_data_sync() -> ModelsData {
    load_catalog_data_from_path_sync(&default_catalog_snapshot_path())
}

fn read_metadata_sync(path: &Path) -> Option<CatalogMetadata> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<CatalogMetadata>(&raw).ok()
}

pub fn load_catalog_data_from_path_sync(path: &Path) -> ModelsData {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    parse_models_data(&raw).unwrap_or_default()
}

fn parse_models_data(raw: &str) -> Option<ModelsData> {
    if let Ok(parsed) = serde_json::from_str::<ModelsData>(raw) {
        return Some(parsed);
    }

    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let map = value.as_object()?;

    let mut data = HashMap::new();
    for (provider_id, provider_value) in map {
        match serde_json::from_value::<crate::models::ProviderInfo>(provider_value.clone()) {
            Ok(mut provider) => {
                if provider.id.trim().is_empty() {
                    provider.id = provider_id.clone();
                }
                data.insert(provider_id.clone(), provider);
            }
            Err(error) => {
                tracing::debug!(
                    provider = provider_id,
                    %error,
                    "Skipping invalid provider entry from models catalogue snapshot"
                );
            }
        }
    }

    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_snapshot_path(filename: &str) -> PathBuf {
        let unique = format!(
            "rocode-provider-catalog-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(filename)
    }

    #[test]
    fn load_catalog_data_from_path_sync_returns_empty_for_missing_file() {
        let path = temp_snapshot_path("missing.snapshot.json");
        let data = load_catalog_data_from_path_sync(&path);
        assert!(data.is_empty());
    }

    #[test]
    fn load_catalog_data_from_path_sync_reads_snapshot_from_primary_path() {
        let path = temp_snapshot_path("models.snapshot.json");
        fs::write(
            &path,
            r#"{
                "openai": {
                    "id": "openai",
                    "name": "OpenAI",
                    "env": [],
                    "models": {}
                }
            }"#,
        )
        .expect("write snapshot");

        let data = load_catalog_data_from_path_sync(&path);
        assert!(data.contains_key("openai"));
    }
}
