use agendao_multimodal::{
    MultimodalAuthority, MultimodalCapabilitiesResponse, MultimodalPolicyResponse,
    MultimodalPreflightRequest, MultimodalPreflightResponse, SessionPartAdapter,
};
use agendao_provider::bootstrap::{ProviderBootstrapState, ProviderModel};
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::server::bootstrap_config_from_config;
use crate::{ApiError, Result, ServerState};

pub(crate) fn multimodal_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/policy", get(get_multimodal_policy))
        .route("/capabilities", get(get_multimodal_capabilities))
        .route("/preflight", post(post_multimodal_preflight))
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct MultimodalCapabilitiesQuery {
    #[serde(default)]
    pub(crate) model: Option<String>,
}

pub(crate) async fn get_multimodal_policy(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<MultimodalPolicyResponse>> {
    let config = state.config_store.config();
    let authority = MultimodalAuthority::from_config(&config);
    Ok(Json(MultimodalPolicyResponse {
        policy: authority.config_view(),
    }))
}

pub(crate) async fn get_multimodal_capabilities(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MultimodalCapabilitiesQuery>,
) -> Result<Json<MultimodalCapabilitiesResponse>> {
    let config = state.config_store.config();
    let authority = MultimodalAuthority::from_config(&config);
    let Some((provider_id, model)) = resolve_selected_model(&state, query.model.as_deref()).await?
    else {
        return Ok(Json(MultimodalCapabilitiesResponse {
            resolved_model: None,
            capability: None,
            warnings: vec!["No active model selected for multimodal capability lookup.".to_string()],
        }));
    };

    let capability = authority
        .capability_authority()
        .capability_view(provider_id.clone(), &model);
    Ok(Json(MultimodalCapabilitiesResponse {
        resolved_model: Some(format!("{}/{}", provider_id, model.id)),
        capability: Some(capability),
        warnings: Vec::new(),
    }))
}

pub(crate) async fn post_multimodal_preflight(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<MultimodalPreflightRequest>,
) -> Result<Json<MultimodalPreflightResponse>> {
    let config = state.config_store.config();
    let authority = MultimodalAuthority::from_config(&config);
    let policy = authority.config_view();
    let Some((provider_id, model)) =
        resolve_selected_model(&state, payload.model.as_deref()).await?
    else {
        return Ok(Json(MultimodalPreflightResponse {
            policy,
            resolved_model: None,
            capability: None,
            result: Default::default(),
            warnings: vec!["No active model selected for multimodal preflight.".to_string()],
        }));
    };

    let capability_authority = authority.capability_authority();
    let capability = capability_authority.capability_view(provider_id.clone(), &model);
    let effective_parts = if payload.session_parts.is_empty() {
        payload.parts
    } else {
        SessionPartAdapter::preflight_parts_from_session_parts(&payload.session_parts)
    };
    let result = capability_authority.preflight(&capability, &effective_parts);
    Ok(Json(MultimodalPreflightResponse {
        policy,
        resolved_model: Some(format!("{}/{}", provider_id, model.id)),
        capability: Some(capability),
        result,
        warnings: Vec::new(),
    }))
}

pub(crate) async fn resolve_selected_model(
    state: &Arc<ServerState>,
    explicit_model: Option<&str>,
) -> Result<Option<(String, ProviderModel)>> {
    let config = state.config_store.config();
    let model_ref = explicit_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| config.model.as_deref().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty());

    let Some(model_ref) = model_ref else {
        return Ok(None);
    };

    let (provider_id, model_id) = if let Some((provider_id, model_id)) = model_ref.split_once('/') {
        (provider_id.to_string(), model_id.to_string())
    } else {
        let providers = state.providers.read().await;
        providers.parse_model_string(&model_ref).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "unknown model `{}` for multimodal lookup",
                model_ref
            ))
        })?
    };

    let model = resolve_provider_model(state, &provider_id, &model_id).await?;
    Ok(Some((provider_id, model)))
}

pub(crate) async fn resolve_provider_model(
    state: &Arc<ServerState>,
    provider_id: &str,
    model_id: &str,
) -> Result<ProviderModel> {
    let config = state.config_store.config();
    let auth_store = state.auth_manager.list().await;
    let models_dev = state.catalog_authority.data().await;
    let bootstrap_config = bootstrap_config_from_config(&config);
    let bootstrap_state = ProviderBootstrapState::init(&models_dev, &bootstrap_config, &auth_store);

    let Some(provider) = bootstrap_state.providers.get(provider_id) else {
        return Err(ApiError::BadRequest(format!(
            "provider `{}` is not available for multimodal lookup",
            provider_id
        )));
    };
    let Some(model) = provider.models.get(model_id) else {
        return Err(ApiError::BadRequest(format!(
            "model `{}` was not found under provider `{}`",
            model_id, provider_id
        )));
    };

    Ok(model.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerState;
    use agendao_config::{Config, ConfigStore, MultimodalConfig, ProviderConfig, VoiceConfig};
    use agendao_provider::{
        metadata_path_for_snapshot, CatalogMetadata, ModelCatalogAuthority, ModelLimit,
        ModelModalities, ModelsData, ModelsDevInfo, ModelsProviderInfo,
    };
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn test_state(config: Config, models_data: ModelsData) -> Arc<ServerState> {
        let temp = tempdir().expect("tempdir");
        let temp_path = temp.keep();
        let snapshot_path = temp_path.join("models.snapshot.json");
        std::fs::write(
            &snapshot_path,
            serde_json::to_vec(&models_data).expect("serialize snapshot"),
        )
        .expect("write snapshot");
        std::fs::write(
            metadata_path_for_snapshot(&snapshot_path),
            serde_json::to_vec(&CatalogMetadata::default()).expect("serialize metadata"),
        )
        .expect("write metadata");

        let mut state = ServerState::new();
        state.config_store = Arc::new(ConfigStore::new(config));
        state.catalog_authority =
            Arc::new(ModelCatalogAuthority::with_snapshot_path(snapshot_path));
        Arc::new(state)
    }

    fn sample_models_data() -> ModelsData {
        let mut providers = HashMap::new();
        providers.insert(
            "openai".to_string(),
            ModelsProviderInfo {
                api: Some("https://api.openai.com/v1".to_string()),
                name: "OpenAI".to_string(),
                env: vec!["OPENAI_API_KEY".to_string()],
                id: "openai".to_string(),
                npm: Some("@ai-sdk/openai".to_string()),
                models: HashMap::from([(
                    "gpt-audio".to_string(),
                    ModelsDevInfo {
                        id: "gpt-audio".to_string(),
                        name: "GPT Audio".to_string(),
                        family: None,
                        release_date: Some("2026-01-01".to_string()),
                        attachment: true,
                        reasoning: false,
                        temperature: true,
                        tool_call: false,
                        interleaved: None,
                        cost: None,
                        limit: ModelLimit {
                            context: 128000,
                            input: None,
                            output: 4096,
                        },
                        modalities: Some(ModelModalities {
                            input: vec!["text".to_string(), "audio".to_string()],
                            output: vec!["text".to_string()],
                        }),
                        experimental: None,
                        status: Some("stable".to_string()),
                        options: HashMap::new(),
                        headers: None,
                        provider: None,
                        variants: None,
                    },
                )]),
            },
        );
        providers
    }

    #[tokio::test]
    async fn policy_route_returns_resolved_multimodal_policy() {
        let state = test_state(
            Config {
                multimodal: Some(MultimodalConfig {
                    voice: Some(VoiceConfig {
                        duration_seconds: Some(22),
                        attach_audio: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            sample_models_data(),
        );

        let Json(response) = get_multimodal_policy(State(state))
            .await
            .expect("policy route should succeed");
        assert_eq!(response.policy.voice.duration_seconds, 22);
        assert!(!response.policy.voice.attach_audio);
    }

    #[tokio::test]
    async fn preflight_route_uses_provider_ground_truth_audio_support() {
        let state = test_state(
            Config {
                model: Some("openai/gpt-audio".to_string()),
                provider: Some(HashMap::from([(
                    "openai".to_string(),
                    ProviderConfig {
                        name: Some("OpenAI".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
            sample_models_data(),
        );

        let Json(response) = post_multimodal_preflight(
            State(state),
            Json(MultimodalPreflightRequest {
                model: None,
                parts: vec![agendao_multimodal::PreflightInputPart {
                    kind: Some(agendao_multimodal::ModalityKind::Audio),
                    mime: Some("audio/wav".to_string()),
                    byte_len: Some(512),
                    label: Some("voice.wav".to_string()),
                }],
                session_parts: Vec::new(),
            }),
        )
        .await
        .expect("preflight route should succeed");

        assert_eq!(response.resolved_model.as_deref(), Some("openai/gpt-audio"));
        assert!(response.capability.is_some());
        assert!(response.result.unsupported_parts.is_empty());
        assert!(!response.result.hard_block);
    }

    #[tokio::test]
    async fn preflight_route_accepts_session_parts_contract() {
        let state = test_state(
            Config {
                model: Some("openai/gpt-audio".to_string()),
                provider: Some(HashMap::from([(
                    "openai".to_string(),
                    ProviderConfig {
                        name: Some("OpenAI".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
            sample_models_data(),
        );

        let Json(response) = post_multimodal_preflight(
            State(state),
            Json(MultimodalPreflightRequest {
                model: None,
                parts: Vec::new(),
                session_parts: vec![agendao_types::PromptPart::File {
                    url: "data:audio/wav;base64,UklGRg==".to_string(),
                    filename: Some("voice.wav".to_string()),
                    mime: Some("audio/wav".to_string()),
                }],
            }),
        )
        .await
        .expect("preflight route should succeed");

        assert_eq!(response.resolved_model.as_deref(), Some("openai/gpt-audio"));
        assert!(response.result.warnings.is_empty());
        assert!(!response.result.hard_block);
    }
}
