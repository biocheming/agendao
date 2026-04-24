use anyhow::Result;
use rocode_config::{ConfigStore, UiRecentModelConfig, WorkspaceIdentity, WorkspaceMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentModelEntry {
    pub provider: String,
    pub model: String,
}

impl From<UiRecentModelConfig> for RecentModelEntry {
    fn from(value: UiRecentModelConfig) -> Self {
        Self {
            provider: value.provider,
            model: value.model,
        }
    }
}

impl From<&UiRecentModelConfig> for RecentModelEntry {
    fn from(value: &UiRecentModelConfig) -> Self {
        Self {
            provider: value.provider.clone(),
            model: value.model.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceUserState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_models: Vec<RecentModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalUserState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_models: Vec<RecentModelEntry>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub workspaces: HashMap<String, WorkspaceUserState>,
}

pub struct UserStateAuthority {
    identity: WorkspaceIdentity,
    mode: WorkspaceMode,
    global_path: PathBuf,
    workspace_path: Option<PathBuf>,
    write_lock: Mutex<()>,
}

fn state_file_locks() -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static STATE_FILE_LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
        OnceLock::new();
    STATE_FILE_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn state_file_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut locks = state_file_locks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

impl UserStateAuthority {
    pub fn new(identity: WorkspaceIdentity, mode: WorkspaceMode) -> Self {
        let workspace_path = identity
            .config_dir
            .as_ref()
            .map(|config_dir| config_dir.join("state.json"));
        Self {
            identity,
            mode,
            global_path: default_global_state_path(),
            workspace_path,
            write_lock: Mutex::new(()),
        }
    }

    pub fn from_config_store(store: &ConfigStore) -> Self {
        let identity = store
            .workspace_identity()
            .unwrap_or_else(|| WorkspaceIdentity::fallback(Path::new(".")));
        Self::new(identity, store.workspace_mode())
    }

    pub fn ephemeral() -> Self {
        Self::new(
            WorkspaceIdentity::fallback(Path::new(".")),
            WorkspaceMode::Shared,
        )
    }

    pub async fn resolved_recent_models(
        &self,
        legacy_recent: &[RecentModelEntry],
    ) -> Result<Vec<RecentModelEntry>> {
        match self.mode {
            WorkspaceMode::Isolated => {
                let state = self.read_workspace_state().await?;
                if !state.recent_models.is_empty() {
                    Ok(state.recent_models)
                } else {
                    Ok(legacy_recent.to_vec())
                }
            }
            WorkspaceMode::Shared => {
                let state = self.read_global_state().await?;
                if let Some(workspace) = state.workspaces.get(&self.identity.workspace_key) {
                    if !workspace.recent_models.is_empty() {
                        return Ok(workspace.recent_models.clone());
                    }
                }
                if !state.recent_models.is_empty() {
                    return Ok(state.recent_models);
                }
                Ok(legacy_recent.to_vec())
            }
        }
    }

    pub async fn save_recent_models(&self, recent: &[RecentModelEntry]) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        match self.mode {
            WorkspaceMode::Isolated => {
                let Some(workspace_path) = self.workspace_path.as_ref() else {
                    return Ok(());
                };
                let file_lock = state_file_lock(workspace_path);
                let _file_guard = file_lock.lock().await;
                let mut state = self.read_workspace_state().await?;
                state.recent_models = recent.to_vec();
                self.write_workspace_state(&state).await?;
            }
            WorkspaceMode::Shared => {
                let file_lock = state_file_lock(&self.global_path);
                let _file_guard = file_lock.lock().await;
                let mut state = self.read_global_state().await?;
                state.recent_models = recent.to_vec();
                state.workspaces.insert(
                    self.identity.workspace_key.clone(),
                    WorkspaceUserState {
                        recent_models: recent.to_vec(),
                    },
                );
                self.write_global_state(&state).await?;
            }
        }
        Ok(())
    }

    async fn read_global_state(&self) -> Result<GlobalUserState> {
        read_json_file(&self.global_path).await
    }

    async fn write_global_state(&self, state: &GlobalUserState) -> Result<()> {
        write_json_file(&self.global_path, state).await
    }

    async fn read_workspace_state(&self) -> Result<WorkspaceUserState> {
        let Some(path) = self.workspace_path.as_ref() else {
            return Ok(WorkspaceUserState::default());
        };
        read_json_file(path).await
    }

    async fn write_workspace_state(&self, state: &WorkspaceUserState) -> Result<()> {
        let Some(path) = self.workspace_path.as_ref() else {
            return Ok(());
        };
        write_json_file(path, state).await
    }
}

fn default_global_state_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("rocode")
        .join("global-state.json")
}

async fn read_json_file<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => match serde_json::from_str::<T>(&raw) {
            Ok(value) => Ok(value),
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "failed to parse persisted state file, using default value"
                );
                Ok(T::default())
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

async fn write_json_file<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let raw = serde_json::to_vec_pretty(value)?;
    tokio::fs::write(path, raw).await?;
    Ok(())
}

pub type SharedUserStateAuthority = Arc<UserStateAuthority>;

#[cfg(test)]
mod tests {
    use super::*;
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
            if let Err(error) = fs::remove_dir_all(&self.path) {
                tracing::warn!(path = %self.path.display(), %error, "failed to remove test temp dir");
            }
        }
    }

    fn make_identity(root: &Path, isolated: bool) -> WorkspaceIdentity {
        let config_dir = isolated.then(|| root.join(".rocode"));
        if let Some(dir) = &config_dir {
            fs::create_dir_all(dir).expect("failed to create workspace config dir");
        }
        WorkspaceIdentity {
            requested_dir: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            config_dir,
            workspace_key: root.to_string_lossy().to_string(),
        }
    }

    fn make_authority(
        identity: WorkspaceIdentity,
        mode: WorkspaceMode,
        global_path: PathBuf,
    ) -> UserStateAuthority {
        let workspace_path = identity
            .config_dir
            .as_ref()
            .map(|config_dir| config_dir.join("state.json"));
        UserStateAuthority {
            identity,
            mode,
            global_path,
            workspace_path,
            write_lock: Mutex::new(()),
        }
    }

    fn recent(provider: &str, model: &str) -> Vec<RecentModelEntry> {
        vec![RecentModelEntry {
            provider: provider.to_string(),
            model: model.to_string(),
        }]
    }

    #[tokio::test]
    async fn isolated_recent_models_are_written_to_workspace_state() {
        let temp = TestDir::new("rocode_state_isolated");
        let workspace_root = temp.path.join("workspace");
        fs::create_dir_all(&workspace_root).expect("failed to create workspace root");
        let global_path = temp.path.join("global/global-state.json");
        let authority = make_authority(
            make_identity(&workspace_root, true),
            WorkspaceMode::Isolated,
            global_path.clone(),
        );

        let expected = recent("openai", "gpt-5.4");
        authority
            .save_recent_models(&expected)
            .await
            .expect("save recent models");

        let workspace_state_path = workspace_root.join(".rocode/state.json");
        let raw = tokio::fs::read_to_string(&workspace_state_path)
            .await
            .expect("read workspace state");
        let state: WorkspaceUserState =
            serde_json::from_str(&raw).expect("deserialize workspace state");
        assert_eq!(state.recent_models, expected);
        assert!(
            !global_path.exists(),
            "isolated save should not write global state"
        );
    }

    #[tokio::test]
    async fn shared_recent_models_are_workspace_keyed_with_global_fallback() {
        let temp = TestDir::new("rocode_state_shared");
        let global_path = temp.path.join("global/global-state.json");
        let workspace_a = temp.path.join("workspace-a");
        let workspace_b = temp.path.join("workspace-b");
        fs::create_dir_all(&workspace_a).expect("failed to create workspace a");
        fs::create_dir_all(&workspace_b).expect("failed to create workspace b");

        let authority_a = make_authority(
            make_identity(&workspace_a, false),
            WorkspaceMode::Shared,
            global_path.clone(),
        );
        let authority_b = make_authority(
            make_identity(&workspace_b, false),
            WorkspaceMode::Shared,
            global_path.clone(),
        );

        let recent_a = recent("anthropic", "claude-sonnet-4");
        authority_a
            .save_recent_models(&recent_a)
            .await
            .expect("save workspace a");
        assert_eq!(
            authority_a
                .resolved_recent_models(&[])
                .await
                .expect("resolve workspace a"),
            recent_a
        );
        assert_eq!(
            authority_b
                .resolved_recent_models(&[])
                .await
                .expect("resolve workspace b fallback"),
            recent_a
        );

        let recent_b = recent("openai", "gpt-5.4");
        authority_b
            .save_recent_models(&recent_b)
            .await
            .expect("save workspace b");
        assert_eq!(
            authority_a
                .resolved_recent_models(&[])
                .await
                .expect("resolve workspace a after workspace b write"),
            recent_a
        );
        assert_eq!(
            authority_b
                .resolved_recent_models(&[])
                .await
                .expect("resolve workspace b"),
            recent_b
        );

        let raw = tokio::fs::read_to_string(&global_path)
            .await
            .expect("read global state");
        let state: GlobalUserState = serde_json::from_str(&raw).expect("deserialize global state");
        assert_eq!(state.recent_models, recent_b);
        assert_eq!(
            state
                .workspaces
                .get(&workspace_a.to_string_lossy().to_string())
                .expect("workspace a entry")
                .recent_models,
            recent_a
        );
        assert_eq!(
            state
                .workspaces
                .get(&workspace_b.to_string_lossy().to_string())
                .expect("workspace b entry")
                .recent_models,
            recent_b
        );
    }

    #[test]
    fn state_file_lock_reuses_lock_for_same_path() {
        let path = PathBuf::from("/tmp/rocode-state-test.json");
        let first = state_file_lock(&path);
        let second = state_file_lock(&path);
        assert!(Arc::ptr_eq(&first, &second));
    }
}
