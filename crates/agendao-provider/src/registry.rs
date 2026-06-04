use crate::bootstrap::{apply_custom_loaders, filter_models_by_status};
use crate::catalog::{
    default_model_catalog_authority, metadata_path_for_snapshot, ModelCatalogAuthority,
};
use crate::models::{
    ModelInfo as CatalogModelInfo, ModelsData, ProviderInfo as ModelsProviderInfo,
};
use std::path::PathBuf;
use std::sync::Arc;

pub struct ModelsRegistry {
    authority: Arc<ModelCatalogAuthority>,
}

impl ModelsRegistry {
    pub fn new(cache_path: PathBuf) -> Self {
        Self {
            authority: Arc::new(ModelCatalogAuthority::new(
                cache_path.clone(),
                metadata_path_for_snapshot(&cache_path),
            )),
        }
    }

    pub async fn get(&self) -> ModelsData {
        self.authority.data().await
    }

    pub async fn refresh(&self) {
        let _ = self.authority.refresh(true).await;
    }

    pub async fn get_provider(&self, provider_id: &str) -> Option<ModelsProviderInfo> {
        let data = self.get().await;
        data.get(provider_id).cloned()
    }

    pub async fn get_model(&self, provider_id: &str, model_id: &str) -> Option<CatalogModelInfo> {
        let data = self.get().await;
        data.get(provider_id)
            .and_then(|provider| provider.models.get(model_id).cloned())
    }

    pub async fn list_models_for_provider(&self, provider_id: &str) -> Vec<CatalogModelInfo> {
        let data = self.get().await;
        data.get(provider_id)
            .map(|provider| provider.models.values().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn get_with_customization(&self, enable_experimental: bool) -> ModelsData {
        let mut data = self.get().await;
        apply_custom_loaders(&mut data);
        filter_models_by_status(&mut data, enable_experimental);
        data
    }
}

impl Default for ModelsRegistry {
    fn default() -> Self {
        Self {
            authority: default_model_catalog_authority(),
        }
    }
}
