mod wellknown;

use agendao_config::{Config, ConfigStore, WorkspaceIdentity, WorkspaceMode};
use agendao_state::{RecentModelEntry, UserStateAuthority};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub use wellknown::{clear_wellknown_cache, load_config_with_remote, load_wellknown};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedWorkspaceContext {
    pub identity: WorkspaceIdentity,
    pub mode: WorkspaceMode,
    pub config: Config,
    #[serde(default)]
    pub recent_models: Vec<RecentModelEntry>,
}

impl ResolvedWorkspaceContext {
    pub fn empty() -> Self {
        Self {
            identity: WorkspaceIdentity::fallback(std::path::Path::new(".")),
            mode: WorkspaceMode::Shared,
            config: Config::default(),
            recent_models: Vec::new(),
        }
    }
}

pub struct ResolvedWorkspaceContextAuthority {
    config_store: Arc<ConfigStore>,
    user_state: Arc<UserStateAuthority>,
}

impl ResolvedWorkspaceContextAuthority {
    pub fn new(config_store: Arc<ConfigStore>, user_state: Arc<UserStateAuthority>) -> Self {
        Self {
            config_store,
            user_state,
        }
    }

    pub async fn resolve(&self) -> Result<ResolvedWorkspaceContext> {
        let config = (*self.config_store.config()).clone();
        let identity = self
            .config_store
            .workspace_identity()
            .unwrap_or_else(|| WorkspaceIdentity::fallback(std::path::Path::new(".")));
        let mode = self.config_store.workspace_mode();
        let legacy_recent = config
            .ui_preferences
            .as_ref()
            .map(|prefs| {
                prefs
                    .recent_models
                    .iter()
                    .map(RecentModelEntry::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let recent_models = self
            .user_state
            .resolved_recent_models(&legacy_recent)
            .await?;

        Ok(ResolvedWorkspaceContext {
            identity,
            mode,
            config,
            recent_models,
        })
    }
}
