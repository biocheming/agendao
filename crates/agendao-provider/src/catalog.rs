use crate::models::{
    ModelInfo as ModelsDataModelInfo, ModelLimit, ModelsData,
    ProviderInfo as ModelsProviderInfo, MODELS_DEV_URL,
};
use once_cell::sync::Lazy;
use reqwest::header::{CACHE_CONTROL, ETAG, IF_NONE_MATCH, PRAGMA};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

const CATALOG_USER_AGENT: &str = "agendao-rust";
pub const DEFAULT_CATALOG_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

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
    pub snapshot: Arc<CatalogSnapshot>,
    pub status: CatalogRefreshStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub struct ModelCatalogAuthority {
    snapshot_path: PathBuf,
    metadata_path: PathBuf,
    client: reqwest::Client,
    snapshot: RwLock<Option<Arc<CatalogSnapshot>>>,
    refresh_lock: Mutex<()>,
    next_generation: AtomicU64,
}

impl ModelCatalogAuthority {
    pub fn new(snapshot_path: PathBuf, metadata_path: PathBuf) -> Self {
        Self {
            snapshot_path,
            metadata_path,
            client: reqwest::Client::new(),
            snapshot: RwLock::new(None),
            refresh_lock: Mutex::new(()),
            next_generation: AtomicU64::new(1),
        }
    }

    pub fn with_snapshot_path(snapshot_path: PathBuf) -> Self {
        let metadata_path = metadata_path_for_snapshot(&snapshot_path);
        Self::new(snapshot_path, metadata_path)
    }

    /// Shared snapshot handle — clones only the `Arc`, never the multi-MB
    /// catalogue table. Hot read path for every provider-facing request.
    pub async fn snapshot(&self) -> Arc<CatalogSnapshot> {
        if let Some(snapshot) = self.snapshot.read().await.clone() {
            return snapshot;
        }

        let _guard = self.refresh_lock.lock().await;
        if let Some(snapshot) = self.snapshot.read().await.clone() {
            return snapshot;
        }

        let loaded = self
            .load_cached_snapshot()
            .await
            .unwrap_or_else(|| self.allocate_snapshot(HashMap::new(), CatalogMetadata::default()));
        let loaded = Arc::new(loaded);
        self.store_snapshot(Arc::clone(&loaded)).await;

        if loaded.data.is_empty() {
            return self.refresh_locked(Some(loaded), false).await.snapshot;
        }

        loaded
    }

    /// Full-table copy. Only for cold paths (CLI one-shot listings); hot
    /// paths should use [`Self::snapshot`] (shared handle) or
    /// [`Self::map_snapshot`] (borrow under read lock).
    pub async fn data(&self) -> ModelsData {
        self.snapshot().await.data.clone()
    }

    /// Run `f` against the cached snapshot under a read lock, loading the
    /// snapshot first if necessary. Lets callers clone just the entries they
    /// need instead of the whole (multi-MB) catalogue.
    pub async fn map_snapshot<T>(&self, f: impl FnOnce(&CatalogSnapshot) -> T) -> T {
        if self.snapshot.read().await.is_none() {
            // Cold path: populate the cache via the normal load/refresh flow.
            let _ = self.snapshot().await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .expect("catalog snapshot must be populated after load");
        f(snapshot)
    }

    pub async fn refresh(&self, force: bool) -> Arc<CatalogSnapshot> {
        self.refresh_with_result(force).await.snapshot
    }

    pub async fn refresh_with_result(&self, force: bool) -> CatalogRefreshResult {
        let _guard = self.refresh_lock.lock().await;
        let current = match self.snapshot.read().await.clone() {
            Some(snapshot) => Some(snapshot),
            None => self.load_cached_snapshot().await.map(Arc::new),
        };
        self.refresh_locked(current, force).await
    }

    /// Refresh the catalogue when its last attempt is at least 24 hours old.
    ///
    /// `None` means the cached snapshot is still inside the TTL. Failed
    /// attempts retain the cached data and advance `last_refresh_at`, avoiding
    /// a network retry on every provider-facing request while offline.
    pub async fn refresh_if_stale(&self) -> Option<CatalogRefreshResult> {
        let now = chrono::Utc::now().timestamp_millis();
        if let Some(current) = self.snapshot.read().await.as_ref() {
            if !catalog_refresh_due(&current.metadata, now, DEFAULT_CATALOG_REFRESH_INTERVAL) {
                return None;
            }
        }

        let _guard = self.refresh_lock.lock().await;
        let current = match self.snapshot.read().await.clone() {
            Some(snapshot) => Some(snapshot),
            None => self.load_cached_snapshot().await.map(Arc::new),
        };
        if current.as_ref().is_some_and(|snapshot| {
            !catalog_refresh_due(&snapshot.metadata, now, DEFAULT_CATALOG_REFRESH_INTERVAL)
        }) {
            return None;
        }

        Some(self.refresh_locked(current, false).await)
    }

    async fn refresh_locked(
        &self,
        current: Option<Arc<CatalogSnapshot>>,
        force: bool,
    ) -> CatalogRefreshResult {
        let current = current.unwrap_or_else(|| {
            Arc::new(self.allocate_snapshot(HashMap::new(), CatalogMetadata::default()))
        });
        let now = chrono::Utc::now().timestamp_millis();
        let url = format!("{}/api.json", MODELS_DEV_URL);

        let mut request = self
            .client
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
                // Only the metadata changes on 304; the table copy here is the
                // one accepted full clone (refreshes are rare, user-triggered).
                let mut snapshot = (*current).clone();
                snapshot.metadata.last_refresh_at = Some(now);
                snapshot.metadata.last_success_at = Some(now);
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
                    snapshot: Arc::new(snapshot),
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
                            let snapshot = Arc::new(self.allocate_snapshot(parsed, metadata));
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

        let mut result = result;
        if result.status == CatalogRefreshStatus::FallbackCached {
            let mut snapshot = (*result.snapshot).clone();
            snapshot.metadata.last_refresh_at = Some(now);
            if snapshot.metadata.source_url.is_empty() {
                snapshot.metadata.source_url = url;
            }
            if let Err(error) = self.write_metadata(&snapshot.metadata).await {
                tracing::debug!(
                    path = %self.metadata_path.display(),
                    error = %error,
                    "Failed to write models catalogue refresh-attempt metadata"
                );
            }
            result.snapshot = Arc::new(snapshot);
        }

        self.store_snapshot(result.snapshot.clone()).await;
        result
    }

    async fn store_snapshot(&self, snapshot: Arc<CatalogSnapshot>) {
        self.next_generation
            .fetch_max(snapshot.generation.saturating_add(1), Ordering::SeqCst);
        *self.snapshot.write().await = Some(snapshot);
    }

    async fn load_cached_snapshot(&self) -> Option<CatalogSnapshot> {
        let raw = match tokio::fs::read_to_string(&self.snapshot_path).await {
            Ok(raw) => raw,
            // First run on an offline machine: fall back to the built-in
            // seed catalogue so bundled providers stay usable; the next
            // successful refresh replaces it wholesale.
            Err(_) => return Some(self.seed_snapshot()),
        };
        let data = parse_models_data(&raw).unwrap_or_default();
        if data.is_empty() {
            return Some(self.seed_snapshot());
        }

        let metadata = match tokio::fs::read_to_string(&self.metadata_path).await {
            Ok(raw) => serde_json::from_str::<CatalogMetadata>(&raw).unwrap_or_default(),
            Err(_) => CatalogMetadata::default(),
        };
        let metadata = CatalogMetadata {
            source_url: if metadata.source_url.is_empty() {
                format!("{}/api.json", MODELS_DEV_URL)
            } else {
                metadata.source_url
            },
            ..metadata
        };

        Some(self.allocate_snapshot(data, metadata))
    }

    fn seed_snapshot(&self) -> CatalogSnapshot {
        self.allocate_snapshot(
            seed_catalog(),
            CatalogMetadata {
                source_url: SEED_CATALOG_SOURCE_URL.to_string(),
                ..CatalogMetadata::default()
            },
        )
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
    // models.dev 快照统一收在 agendao_home/cache/catalog（~/.agendao,土律·单点权威）。
    agendao_util::agendao_home()
        .join("cache")
        .join("catalog")
        .join("models.snapshot.json")
}

pub fn default_catalog_metadata_path() -> PathBuf {
    agendao_util::agendao_home()
        .join("cache")
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

const SEED_CATALOG_SOURCE_URL: &str = "seed://builtin";

fn seed_model(
    models: &mut HashMap<String, ModelsDataModelInfo>,
    id: &str,
    name: &str,
    context: u64,
    output: u64,
    reasoning: bool,
) {
    models.insert(
        id.to_string(),
        ModelsDataModelInfo {
            id: id.to_string(),
            name: name.to_string(),
            reasoning,
            tool_call: true,
            attachment: true,
            temperature: true,
            limit: ModelLimit {
                context,
                input: None,
                output,
            },
            family: None,
            release_date: None,
            interleaved: None,
            cost: None,
            modalities: None,
            experimental: None,
            status: None,
            options: HashMap::new(),
            headers: None,
            provider: None,
            variants: None,
        },
    );
}

/// Minimal built-in catalogue for offline first runs: bundled providers with
/// their env var names and a couple of evergreen models. Pricing and the
/// full model list arrive with the first successful models.dev refresh.
fn seed_catalog() -> ModelsData {
    let mut openai_models: HashMap<String, ModelsDataModelInfo> = HashMap::new();
    seed_model(&mut openai_models, "gpt-5", "GPT-5", 400_000, 128_000, true);
    seed_model(&mut openai_models, "gpt-5-mini", "GPT-5 mini", 400_000, 128_000, true);

    let mut anthropic_models: HashMap<String, ModelsDataModelInfo> = HashMap::new();
    seed_model(
        &mut anthropic_models,
        "claude-sonnet-4",
        "Claude Sonnet 4",
        200_000,
        64_000,
        true,
    );

    let mut data = HashMap::new();
    data.insert(
        "openai".to_string(),
        ModelsProviderInfo {
            api: Some("https://api.openai.com/v1".to_string()),
            name: "OpenAI".to_string(),
            env: vec!["OPENAI_API_KEY".to_string()],
            id: "openai".to_string(),
            npm: None,
            models: openai_models,
        },
    );
    data.insert(
        "anthropic".to_string(),
        ModelsProviderInfo {
            api: Some("https://api.anthropic.com/v1".to_string()),
            name: "Anthropic".to_string(),
            env: vec!["ANTHROPIC_API_KEY".to_string()],
            id: "anthropic".to_string(),
            npm: None,
            models: anthropic_models,
        },
    );
    data
}

pub(crate) fn load_default_catalog_data_sync() -> ModelsData {
    let Ok(raw) = std::fs::read_to_string(default_catalog_snapshot_path()) else {
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
        match serde_json::from_value::<ModelsProviderInfo>(provider_value.clone()) {
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

fn catalog_refresh_due(metadata: &CatalogMetadata, now_ms: i64, max_age: Duration) -> bool {
    let Some(last_refresh_at) = metadata.last_refresh_at else {
        return true;
    };
    let max_age_ms = i64::try_from(max_age.as_millis()).unwrap_or(i64::MAX);
    now_ms.saturating_sub(last_refresh_at) >= max_age_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_snapshot_path(filename: &str) -> PathBuf {
        let unique = format!(
            "agendao-provider-catalog-test-{}-{}",
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

    #[tokio::test]
    async fn cached_snapshot_falls_back_to_seed_catalog_for_missing_file() {
        let path = temp_snapshot_path("missing.snapshot.json");
        let authority = ModelCatalogAuthority::with_snapshot_path(path);
        // Offline first run: the built-in seed keeps bundled providers
        // usable instead of an empty catalogue.
        let snapshot = authority
            .load_cached_snapshot()
            .await
            .expect("seed snapshot");
        assert_eq!(snapshot.metadata.source_url, SEED_CATALOG_SOURCE_URL);
        assert!(snapshot.data.contains_key("openai"));
        assert!(snapshot.data.contains_key("anthropic"));
        assert!(snapshot.data["openai"]
            .models
            .contains_key("gpt-5"));
        // Seed data is immediately due for a real refresh.
        assert!(catalog_refresh_due(
            &snapshot.metadata,
            1_000,
            DEFAULT_CATALOG_REFRESH_INTERVAL
        ));
    }

    #[tokio::test]
    async fn cached_snapshot_reads_primary_path() {
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

        let authority = ModelCatalogAuthority::with_snapshot_path(path);
        let snapshot = authority
            .load_cached_snapshot()
            .await
            .expect("cached snapshot");
        assert!(snapshot.data.contains_key("openai"));
    }

    #[test]
    fn catalogue_refresh_becomes_due_at_24_hours() {
        let now = 2_000_000_000_i64;
        let mut metadata = CatalogMetadata::default();
        assert!(catalog_refresh_due(
            &metadata,
            now,
            DEFAULT_CATALOG_REFRESH_INTERVAL
        ));

        metadata.last_refresh_at = Some(now - (24 * 60 * 60 * 1_000) + 1);
        assert!(!catalog_refresh_due(
            &metadata,
            now,
            DEFAULT_CATALOG_REFRESH_INTERVAL
        ));

        metadata.last_refresh_at = Some(now - (24 * 60 * 60 * 1_000));
        assert!(catalog_refresh_due(
            &metadata,
            now,
            DEFAULT_CATALOG_REFRESH_INTERVAL
        ));
    }
}
