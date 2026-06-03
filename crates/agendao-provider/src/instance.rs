use async_trait::async_trait;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    cache::ProviderProfileFingerprint, runtime::ProviderRuntime, ChatRequest, ChatResponse,
    ModelInfo, Provider, ProviderAdapter, ProviderConfig, ProviderError, StreamResult,
};

/// Runtime provider instance combining provider adapter + config + models.
pub struct ProviderInstance {
    id: String,
    name: String,
    config: ProviderConfig,
    adapter: Arc<dyn ProviderAdapter>,
    client: Client,
    models: HashMap<String, ModelInfo>,
    runtime: Option<ProviderRuntime>,
    provider_profile_fingerprint: Option<ProviderProfileFingerprint>,
}

impl ProviderInstance {
    pub fn new(
        id: String,
        name: String,
        config: ProviderConfig,
        adapter: Arc<dyn ProviderAdapter>,
        models: HashMap<String, ModelInfo>,
    ) -> Self {
        Self {
            id,
            name,
            config,
            adapter,
            client: Client::new(),
            models,
            runtime: None,
            provider_profile_fingerprint: None,
        }
    }

    pub fn with_provider_profile_fingerprint(
        mut self,
        fingerprint: ProviderProfileFingerprint,
    ) -> Self {
        self.provider_profile_fingerprint = Some(fingerprint);
        self
    }

    pub fn with_runtime(mut self, runtime: ProviderRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn runtime(&self) -> Option<&ProviderRuntime> {
        self.runtime.as_ref()
    }

    pub fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.get(id)
    }

    pub fn models(&self) -> Vec<ModelInfo> {
        self.models.values().cloned().collect()
    }
}

#[async_trait]
impl Provider for ProviderInstance {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn provider_profile_fingerprint(&self) -> Option<ProviderProfileFingerprint> {
        self.provider_profile_fingerprint.clone()
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.models.values().cloned().collect()
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.get(id)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let _permit = if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    preflight.check().await?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let result = self.adapter.chat(&self.client, &self.config, request).await;

        if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    match &result {
                        Ok(_) => preflight.on_success(),
                        Err(_) => preflight.on_failure(),
                    }
                }
            }
        }

        result
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let _permit = if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    preflight.check().await?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let result = self
            .adapter
            .chat_stream(&self.client, &self.config, request)
            .await;

        if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    match &result {
                        Ok(_) => preflight.on_success(),
                        Err(_) => preflight.on_failure(),
                    }
                }
            }
        }

        result
    }
}
