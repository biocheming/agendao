use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::loader::{
    resolve_configured_path, update_config, write_config, ConfigAuthority, WorkspaceIdentity,
    WorkspaceMode,
};
use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};

use crate::schema::Config;

/// Single source of truth for application configuration.
///
/// Read path is lock-free (`ArcSwap`). Write path (`patch`) swaps the Arc
/// and invalidates derived caches. Consumers hold `Arc<Config>` snapshots.
pub struct ConfigStore {
    base: ArcSwap<Config>,
    plugin_applied: ArcSwapOption<Config>,
    project_dir: RwLock<Option<PathBuf>>,
    workspace_identity: RwLock<Option<WorkspaceIdentity>>,
    workspace_mode: RwLock<WorkspaceMode>,
    revision: AtomicU64,
}

impl ConfigStore {
    /// Create a ConfigStore with an initial config value.
    pub fn new(config: Config) -> Self {
        Self {
            base: ArcSwap::from_pointee(config),
            plugin_applied: ArcSwapOption::empty(),
            project_dir: RwLock::new(None),
            workspace_identity: RwLock::new(None),
            workspace_mode: RwLock::new(WorkspaceMode::Shared),
            revision: AtomicU64::new(0),
        }
    }

    /// Create a ConfigStore by loading config from disk.
    pub fn from_project_dir(project_dir: &Path) -> anyhow::Result<Self> {
        let resolved = ConfigAuthority::resolve(project_dir)?;
        let store = Self::new(resolved.config);
        *store.write_project_dir()? = Some(resolved.inputs.identity.workspace_root.clone());
        *store.write_workspace_identity()? = Some(resolved.inputs.identity);
        *store.write_workspace_mode()? = resolved.inputs.mode;
        Ok(store)
    }

    /// Read current base config. Lock-free, returns Arc snapshot.
    pub fn config(&self) -> Arc<Config> {
        self.base.load_full()
    }

    /// Merge a JSON patch into the base config, invalidate derived caches.
    pub fn patch(&self, patch: serde_json::Value) -> anyhow::Result<Arc<Config>> {
        let current = self.config();
        let mut updated = (*current).clone();

        let patch_config: Config = serde_json::from_value(patch)?;
        if let Some(project_dir) = self.read_project_dir()?.as_deref() {
            update_config(project_dir, &patch_config)?;
        } else {
            tracing::warn!(
                "config patch applied in memory only because project_dir is unset; change will not persist to disk"
            );
        }
        updated.merge(patch_config);

        let new_arc = Arc::new(updated);
        self.base.store(new_arc.clone());
        self.revision.fetch_add(1, Ordering::Relaxed);

        self.invalidate_plugin_cache_blocking();

        Ok(new_arc)
    }

    pub fn replace_with<F>(&self, mutator: F) -> anyhow::Result<Arc<Config>>
    where
        F: FnOnce(&mut Config) -> anyhow::Result<()>,
    {
        let current = self.config();
        let mut updated = (*current).clone();
        mutator(&mut updated)?;

        if let Some(project_dir) = self.read_project_dir()?.as_deref() {
            write_config(project_dir, &updated)?;
        } else {
            tracing::warn!(
                "config replacement applied in memory only because project_dir is unset; change will not persist to disk"
            );
        }

        let new_arc = Arc::new(updated);
        self.base.store(new_arc.clone());
        self.revision.fetch_add(1, Ordering::Relaxed);

        self.invalidate_plugin_cache_blocking();

        Ok(new_arc)
    }

    /// Get the cached plugin-applied config (if any).
    pub async fn plugin_applied(&self) -> Option<Arc<Config>> {
        self.plugin_applied.load_full()
    }

    /// Store plugin-applied config after hooks have been executed.
    pub async fn set_plugin_applied(&self, config: Config) {
        self.plugin_applied.store(Some(Arc::new(config)));
    }

    /// Invalidate the plugin-applied cache. Next consumer must re-run hooks.
    pub async fn invalidate_plugin_cache(&self) {
        self.plugin_applied.store(None);
    }

    /// Reload base config from disk (if project_dir is known).
    pub async fn resolved_scheduler_path(&self) -> Option<PathBuf> {
        let config = self.config();
        let raw = config
            .scheduler_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        let path = PathBuf::from(raw);
        if path.is_absolute() {
            return Some(path);
        }

        let project_dir = self.read_project_dir().ok()?.clone();
        project_dir.map(|dir| resolve_configured_path(&dir, raw))
    }

    pub async fn resolved_task_category_path(&self) -> Option<PathBuf> {
        let config = self.config();
        let raw = config
            .task_category_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        let path = PathBuf::from(raw);
        if path.is_absolute() {
            return Some(path);
        }

        let project_dir = self.read_project_dir().ok()?.clone();
        project_dir.map(|dir| resolve_configured_path(&dir, raw))
    }

    pub async fn reload(&self) -> anyhow::Result<Arc<Config>> {
        let project_dir = self.read_project_dir()?.clone();
        let project_dir = project_dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no project directory set for config reload"))?;
        let resolved = ConfigAuthority::resolve(project_dir)?;
        let config = resolved.config;
        let new_arc = Arc::new(config);
        self.base.store(new_arc.clone());
        self.revision.fetch_add(1, Ordering::Relaxed);
        *self.write_workspace_identity()? = Some(resolved.inputs.identity);
        *self.write_workspace_mode()? = resolved.inputs.mode;
        self.invalidate_plugin_cache().await;
        Ok(new_arc)
    }

    pub fn project_dir(&self) -> Option<PathBuf> {
        self.read_project_dir()
            .map(|guard| guard.clone())
            .unwrap_or_else(|error| {
                tracing::error!(%error, "failed to read project_dir from config store");
                None
            })
    }

    pub fn workspace_identity(&self) -> Option<WorkspaceIdentity> {
        self.read_workspace_identity()
            .map(|guard| guard.clone())
            .unwrap_or_else(|error| {
                tracing::error!(%error, "failed to read workspace_identity from config store");
                None
            })
    }

    pub fn workspace_mode(&self) -> WorkspaceMode {
        self.read_workspace_mode()
            .map(|guard| *guard)
            .unwrap_or_else(|error| {
                tracing::error!(%error, "failed to read workspace_mode from config store");
                WorkspaceMode::Shared
            })
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn invalidate_plugin_cache_blocking(&self) {
        self.plugin_applied.store(None);
    }

    fn read_project_dir(&self) -> anyhow::Result<RwLockReadGuard<'_, Option<PathBuf>>> {
        recover_read_lock(self.project_dir.read(), "project_dir")
    }

    fn write_project_dir(&self) -> anyhow::Result<RwLockWriteGuard<'_, Option<PathBuf>>> {
        recover_write_lock(self.project_dir.write(), "project_dir")
    }

    fn read_workspace_identity(
        &self,
    ) -> anyhow::Result<RwLockReadGuard<'_, Option<WorkspaceIdentity>>> {
        recover_read_lock(self.workspace_identity.read(), "workspace_identity")
    }

    fn write_workspace_identity(
        &self,
    ) -> anyhow::Result<RwLockWriteGuard<'_, Option<WorkspaceIdentity>>> {
        recover_write_lock(self.workspace_identity.write(), "workspace_identity")
    }

    fn read_workspace_mode(&self) -> anyhow::Result<RwLockReadGuard<'_, WorkspaceMode>> {
        recover_read_lock(self.workspace_mode.read(), "workspace_mode")
    }

    fn write_workspace_mode(&self) -> anyhow::Result<RwLockWriteGuard<'_, WorkspaceMode>> {
        recover_write_lock(self.workspace_mode.write(), "workspace_mode")
    }
}

fn recover_read_lock<'a, T>(
    result: LockResult<RwLockReadGuard<'a, T>>,
    lock_name: &'static str,
) -> anyhow::Result<RwLockReadGuard<'a, T>> {
    match result {
        Ok(guard) => Ok(guard),
        Err(poisoned) => {
            tracing::error!(
                lock = lock_name,
                "recovering from poisoned config store read lock"
            );
            Ok(poisoned.into_inner())
        }
    }
}

fn recover_write_lock<'a, T>(
    result: LockResult<RwLockWriteGuard<'a, T>>,
    lock_name: &'static str,
) -> anyhow::Result<RwLockWriteGuard<'a, T>> {
    match result {
        Ok(guard) => Ok(guard),
        Err(poisoned) => {
            tracing::error!(
                lock = lock_name,
                "recovering from poisoned config store write lock"
            );
            Ok(poisoned.into_inner())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock error")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("failed to create test temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn config_returns_arc_without_clone() {
        let store = ConfigStore::new(Config::default());
        let a = store.config();
        let b = store.config();
        // Same Arc, not cloned data
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn patch_replaces_config() {
        let store = ConfigStore::new(Config::default());
        let before = store.config();

        let patch = serde_json::json!({ "model": "test-model" });
        store.patch(patch).unwrap();

        let after = store.config();
        assert!(!Arc::ptr_eq(&before, &after));
    }

    #[tokio::test]
    async fn patch_invalidates_plugin_cache() {
        let store = ConfigStore::new(Config::default());

        store.set_plugin_applied(Config::default()).await;
        assert!(store.plugin_applied().await.is_some());

        store.patch(serde_json::json!({})).unwrap();
        assert!(store.plugin_applied().await.is_none());
    }

    #[tokio::test]
    async fn resolved_scheduler_path_uses_project_dir_for_relative_paths() {
        let store = ConfigStore::new(Config {
            scheduler_path: Some(".agendao/scheduler/sisyphus.jsonc".to_string()),
            ..Default::default()
        });

        *store.project_dir.write().expect("project_dir poisoned") =
            Some(PathBuf::from("/tmp/agendao-project"));

        assert_eq!(
            store.resolved_scheduler_path().await,
            Some(PathBuf::from(
                "/tmp/agendao-project/.agendao/scheduler/sisyphus.jsonc"
            ))
        );
    }

    #[test]
    fn poisoned_project_dir_lock_is_recovered() {
        let store = Arc::new(ConfigStore::new(Config::default()));
        {
            let mut guard = store
                .project_dir
                .write()
                .expect("project_dir write should succeed before poison");
            *guard = Some(PathBuf::from("/tmp/agendao-project"));
        }

        let poisoned = store.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned
                .project_dir
                .write()
                .expect("project_dir write should succeed before panic");
            panic!("poison project_dir lock");
        });

        assert_eq!(
            store.project_dir(),
            Some(PathBuf::from("/tmp/agendao-project"))
        );
    }

    #[tokio::test]
    async fn patch_persists_to_disk_when_project_dir_is_known() {
        let temp = TestDir::new("agendao_config_store_patch");
        fs::write(temp.path.join("agendao.json"), r#"{ "model": "before" }"#).expect("seed config");

        let store = ConfigStore::from_project_dir(&temp.path).expect("store");
        store
            .patch(serde_json::json!({
                "uiPreferences": { "showThinking": true }
            }))
            .expect("patch");

        let reloaded = store.reload().await.expect("reload");
        let ui = reloaded.ui_preferences.as_ref().expect("ui preferences");
        assert_eq!(reloaded.model.as_deref(), Some("before"));
        assert_eq!(ui.show_thinking, Some(true));
    }

    #[test]
    fn patch_waits_for_project_dir_lock_instead_of_skipping_disk_persist() {
        let temp = TestDir::new("agendao_config_store_patch_lock");
        fs::write(temp.path.join("agendao.json"), r#"{ "model": "before" }"#).expect("seed config");

        let store = Arc::new(ConfigStore::from_project_dir(&temp.path).expect("store"));
        let write_guard = store.project_dir.write().expect("project_dir poisoned");
        let store_clone = store.clone();

        let worker = std::thread::spawn(move || {
            store_clone
                .patch(serde_json::json!({
                    "uiPreferences": { "showThinking": true }
                }))
                .expect("patch");
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(write_guard);
        worker.join().expect("worker join");

        let reloaded = crate::load_config(&temp.path).expect("reload from disk");
        let ui = reloaded.ui_preferences.as_ref().expect("ui preferences");
        assert_eq!(reloaded.model.as_deref(), Some("before"));
        assert_eq!(ui.show_thinking, Some(true));
    }

    #[tokio::test]
    async fn replace_with_persists_full_config_when_project_dir_is_known() {
        let temp = TestDir::new("agendao_config_store_replace");
        fs::write(
            temp.path.join("agendao.json"),
            r#"{ "provider": { "old": { "name": "Old" } } }"#,
        )
        .expect("seed config");

        let store = ConfigStore::from_project_dir(&temp.path).expect("store");
        store
            .replace_with(|config| {
                config.provider = Some(HashMap::from([(
                    "new".to_string(),
                    crate::schema::ProviderConfig {
                        name: Some("New".to_string()),
                        ..Default::default()
                    },
                )]));
                Ok(())
            })
            .expect("replace");

        let reloaded = store.reload().await.expect("reload");
        let providers = reloaded.provider.as_ref().expect("provider map");
        assert!(providers.get("old").is_none());
        assert_eq!(
            providers
                .get("new")
                .and_then(|provider| provider.name.as_deref()),
            Some("New")
        );
    }
}
