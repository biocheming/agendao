use std::collections::HashMap;
use std::sync::Arc;

use agendao_orchestrator::selector::SchedulerChoice;
use agendao_provider::{
    cache::ProviderProfileFingerprint,
    provider_connection_descriptor_candidate_from_config_provider, Provider,
    ProviderProfileDescriptorView,
};
use agendao_session::{resolved_compaction_config, Session};
use agendao_types::{
    SessionEffectiveCompactionPolicy, SessionEffectiveExternalAdapterPolicy,
    SessionEffectiveMemoryPolicy, SessionEffectivePolicyView, SessionEffectiveProviderPolicy,
    SessionEffectiveProviderRuntimeProfile, SessionEffectiveSchedulerPolicy, SessionMemoryInsight,
};

use crate::server::bootstrap_config_from_config;
use crate::ServerState;

use super::scheduler::{
    resolve_prompt_request_config, PromptRequestConfigInput, ResolvedPromptRequestConfig,
};
use super::session_crud::{
    session_agent_override, session_model_override, session_scheduler_override,
    session_variant_override,
};

pub(super) async fn build_session_effective_policy(
    state: &Arc<ServerState>,
    session: &Session,
    memory_insight: Option<&SessionMemoryInsight>,
) -> SessionEffectivePolicyView {
    let config = state.config_store.config();
    let metadata = &session.record().metadata;
    let requested_agent = session_agent_override(session);
    let requested_scheduler = super::scheduler::resolve_effective_scheduler_choice(
        None,
        session_scheduler_override(session),
        requested_agent.is_some(),
    );
    let requested_model = session_model_override(session);
    let requested_variant = session_variant_override(session);
    let mut warnings = Vec::new();

    let memory = build_memory_policy(state, session, memory_insight, &mut warnings).await;
    let external_adapter = build_external_adapter_policy(metadata);
    let compaction = build_compaction_policy(state);

    let resolution = resolve_prompt_request_config(PromptRequestConfigInput {
        state,
        config: &config,
        session_id: &session.record().id,
        requested_agent: requested_agent.as_deref(),
        requested_scheduler: requested_scheduler.as_ref(),
        request_model: requested_model.as_deref(),
        request_variant: requested_variant.as_deref(),
        route: "session_effective_policy",
    })
    .await;

    let (scheduler, provider) = match resolution {
        Ok(resolved) => (
            Some(build_scheduler_policy(
                metadata,
                requested_scheduler.as_ref(),
                &resolved,
            )),
            Some(build_provider_policy(
                &config,
                resolved.provider.as_ref(),
                &resolved.provider_id,
                &resolved.model_id,
                resolved.compiled_request.variant.as_deref(),
                &mut warnings,
            )),
        ),
        Err(error) => {
            warnings.push(format!(
                "effective policy could not fully resolve current request inputs: {}",
                error
            ));
            (
                Some(build_scheduler_policy_from_metadata(
                    metadata,
                    requested_scheduler.as_ref(),
                    None,
                )),
                None,
            )
        }
    };

    SessionEffectivePolicyView {
        session_id: session.record().id.clone(),
        scheduler,
        provider,
        memory,
        compaction,
        external_adapter,
        warnings,
    }
}

fn build_scheduler_policy(
    metadata: &HashMap<String, serde_json::Value>,
    requested_scheduler: Option<&SchedulerChoice>,
    resolved: &ResolvedPromptRequestConfig,
) -> SessionEffectiveSchedulerPolicy {
    build_scheduler_policy_from_metadata(
        metadata,
        requested_scheduler,
        resolved
            .resolved_agent
            .as_ref()
            .map(|agent| agent.name.clone()),
    )
}

fn build_scheduler_policy_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
    requested_scheduler: Option<&SchedulerChoice>,
    resolved_agent: Option<String>,
) -> SessionEffectiveSchedulerPolicy {
    let blueprint = metadata.get("scheduler_blueprint");
    SessionEffectiveSchedulerPolicy {
        requested_kind: requested_scheduler.map(scheduler_choice_kind),
        blueprint_name: blueprint
            .and_then(|value| value.get("name"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        blueprint_fingerprint: metadata_string(metadata, "scheduler_blueprint_fingerprint"),
        source: metadata_string(metadata, "scheduler_selection_source").unwrap_or_else(|| {
            if requested_scheduler.is_some() {
                "session"
            } else {
                "none"
            }
            .to_string()
        }),
        applied: blueprint.is_some() || requested_scheduler.is_some(),
        resolved_agent,
    }
}

fn scheduler_choice_kind(choice: &SchedulerChoice) -> String {
    match choice {
        SchedulerChoice::Auto => "auto",
        SchedulerChoice::Template { .. } => "template",
        SchedulerChoice::Blueprint { .. } => "blueprint",
    }
    .to_string()
}

fn build_provider_policy(
    config: &agendao_config::Config,
    provider: &dyn Provider,
    provider_id: &str,
    model_id: &str,
    variant: Option<&str>,
    warnings: &mut Vec<String>,
) -> SessionEffectiveProviderPolicy {
    let bootstrap = bootstrap_config_from_config(config);
    let (configured_descriptor, configured_descriptor_error) = match bootstrap
        .providers
        .get(provider_id)
    {
        Some(configured) => match provider_connection_descriptor_candidate_from_config_provider(
            provider_id,
            configured,
        ) {
            Ok(candidate) => (Some(candidate), None),
            Err(error) => (None, Some(error.to_string())),
        },
        None => (None, None),
    };

    if let Some(error) = configured_descriptor_error.as_deref() {
        warnings.push(format!(
            "provider descriptor projection failed for `{}`: {}",
            provider_id, error
        ));
    }

    let runtime_profile = provider
        .provider_profile_fingerprint()
        .map(|fingerprint| runtime_profile_from_fingerprint(&fingerprint));
    if runtime_profile.is_none() {
        warnings.push(format!(
            "provider `{}` did not expose a runtime profile fingerprint",
            provider_id
        ));
    }

    SessionEffectiveProviderPolicy {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        resolved_model: format!("{}/{}", provider_id, model_id),
        variant: variant.map(str::to_string),
        configured_descriptor,
        configured_descriptor_error,
        runtime_profile,
    }
}

fn runtime_profile_from_fingerprint(
    fingerprint: &ProviderProfileFingerprint,
) -> SessionEffectiveProviderRuntimeProfile {
    SessionEffectiveProviderRuntimeProfile {
        profile: ProviderProfileDescriptorView {
            provider_id: fingerprint.provider_id.clone(),
            npm: fingerprint.npm.clone(),
            source: "runtime_fingerprint".to_string(),
            api_family: fingerprint.api_family.as_str().to_string(),
            api_shape: fingerprint.api_shape.as_str().to_string(),
            transport: fingerprint.transport.as_str().to_string(),
            usage_shape: fingerprint.usage_shape.as_str().to_string(),
            cache_family: fingerprint.cache_family.as_str().to_string(),
            quirks: fingerprint.quirks.clone(),
        },
        profile_hash: fingerprint.profile_hash.clone(),
    }
}

async fn build_memory_policy(
    state: &Arc<ServerState>,
    session: &Session,
    memory_insight: Option<&SessionMemoryInsight>,
    warnings: &mut Vec<String>,
) -> Option<SessionEffectiveMemoryPolicy> {
    let owned_insight;
    let insight = if let Some(insight) = memory_insight {
        insight
    } else {
        owned_insight = match state
            .runtime_memory
            .build_session_memory_insight(session)
            .await
        {
            Ok(insight) => insight,
            Err(error) => {
                warnings.push(format!(
                    "memory policy view could not be resolved: {}",
                    error
                ));
                return None;
            }
        };
        owned_insight.as_ref()?
    };

    Some(SessionEffectiveMemoryPolicy {
        workspace_key: insight.summary.workspace_key.clone(),
        workspace_mode: insight.summary.workspace_mode.clone(),
        allowed_scopes: insight.summary.allowed_scopes.clone(),
        frozen_snapshot_items: insight.summary.frozen_snapshot_items,
        last_prefetch_items: insight.summary.last_prefetch_items,
    })
}

fn build_compaction_policy(state: &Arc<ServerState>) -> SessionEffectiveCompactionPolicy {
    let resolved = resolved_compaction_config(Some(state.config_store.as_ref()));
    SessionEffectiveCompactionPolicy {
        auto: resolved.auto,
        prune: resolved.prune,
        reserved: resolved.reserved,
    }
}

fn build_external_adapter_policy(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<SessionEffectiveExternalAdapterPolicy> {
    let source = metadata_string(metadata, "last_ingress_source")?;
    let policy = metadata_string(metadata, "last_ingress_policy");
    let is_external = source.starts_with("external:")
        || policy.as_deref()
            == Some(agendao_session::prompt::INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY);
    if !is_external {
        return None;
    }

    Some(SessionEffectiveExternalAdapterPolicy {
        last_ingress_source: source,
        last_ingress_policy: policy,
        last_ingress_batch_count: metadata
            .get("last_ingress_batch_count")
            .and_then(|value| value.as_u64()),
    })
}

fn metadata_string(metadata: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_config::{
        CompactionConfig as AppCompactionConfig, Config, ConfigStore, ProviderConfig,
    };
    use agendao_provider::{
        cache::ProviderProfileFingerprint, ModelInfo, ProviderError, ProviderProfileResolver,
        StreamResult,
    };
    use agendao_session::Session;
    use async_trait::async_trait;

    struct MockProvider {
        id: String,
        name: String,
        models: Vec<ModelInfo>,
        profile: Option<ProviderProfileFingerprint>,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn provider_profile_fingerprint(&self) -> Option<ProviderProfileFingerprint> {
            self.profile.clone()
        }

        fn models(&self) -> Vec<ModelInfo> {
            self.models.clone()
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            self.models.iter().find(|model| model.id == id)
        }

        async fn chat(
            &self,
            _request: agendao_provider::ChatRequest,
        ) -> Result<agendao_provider::ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "mock provider does not handle chat".to_string(),
            ))
        }

        async fn chat_stream(
            &self,
            _request: agendao_provider::ChatRequest,
        ) -> Result<StreamResult, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "mock provider does not handle chat_stream".to_string(),
            ))
        }
    }

    fn sample_model() -> ModelInfo {
        ModelInfo {
            id: "gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
            provider: "openai".to_string(),
            context_window: 128_000,
            max_input_tokens: None,
            max_output_tokens: 16_384,
            supports_vision: true,
            supports_tools: true,
            cost_per_million_input: 5.0,
            cost_per_million_output: 15.0,
            cost_per_million_cache_read: Some(1.0),
            cost_per_million_cache_write: Some(2.0),
        }
    }

    #[tokio::test]
    async fn effective_policy_resolves_scheduler_provider_and_external_adapter() {
        let mut state = ServerState::new();
        state.config_store = Arc::new(ConfigStore::new(Config {
            model: Some("openai/gpt-4o".to_string()),
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    name: Some("OpenAI".to_string()),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    api_style: Some("openai-compatible".to_string()),
                    api_shape: Some("chat-completions".to_string()),
                    transport: Some("bearer".to_string()),
                    usage_shape: Some("openai-cached-tokens".to_string()),
                    env: Some(vec!["OPENAI_API_KEY".to_string()]),
                    ..Default::default()
                },
            )])),
            compaction: Some(AppCompactionConfig {
                auto: Some(false),
                prune: Some(true),
                reserved: Some(512),
            }),
            ..Default::default()
        }));

        let runtime_profile = ProviderProfileResolver::resolve_with_options(
            "openai",
            &HashMap::from([(
                "provider_profile".to_string(),
                serde_json::json!({
                    "api_style": "openai-compatible",
                    "api_shape": "chat-completions",
                    "transport": "bearer",
                    "usage_shape": "openai-cached-tokens",
                    "quirks": ["requires-thinking-replay"]
                }),
            )]),
        );
        state
            .providers
            .write()
            .await
            .register_arc(Arc::new(MockProvider {
                id: "openai".to_string(),
                name: "OpenAI".to_string(),
                models: vec![sample_model()],
                profile: Some(ProviderProfileFingerprint::from_profile(&runtime_profile)),
            }));
        let state = Arc::new(state);

        let mut session = Session::new("session-1".to_string(), ".".to_string());
        let session_id = session.record().id.clone();
        session.insert_metadata(
            "scheduler",
            serde_json::json!({"kind": "template", "template": "verify"}),
        );
        session.insert_metadata(
            "last_ingress_source".to_string(),
            serde_json::json!("external:generic-webhook:generic"),
        );
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(
                agendao_session::prompt::INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY
            ),
        );
        session.insert_metadata("last_ingress_batch_count".to_string(), serde_json::json!(1));

        let policy = build_session_effective_policy(&state, &session, None).await;

        assert_eq!(policy.session_id, session_id);
        assert!(policy.warnings.is_empty(), "{:?}", policy.warnings);

        let scheduler = policy.scheduler.expect("scheduler policy");
        assert_eq!(scheduler.requested_kind.as_deref(), Some("template"));
        assert_eq!(scheduler.blueprint_name, None);
        assert_eq!(scheduler.blueprint_fingerprint, None);
        assert_eq!(scheduler.source, "session");
        assert!(scheduler.applied);

        let provider = policy.provider.expect("provider policy");
        assert_eq!(provider.provider_id, "openai");
        assert_eq!(provider.model_id, "gpt-4o");
        assert_eq!(provider.resolved_model, "openai/gpt-4o");
        assert_eq!(
            provider
                .configured_descriptor
                .as_ref()
                .and_then(|descriptor| descriptor.profile.as_ref())
                .map(|profile| profile.source.as_str()),
            Some("config_override")
        );
        assert_eq!(
            provider
                .configured_descriptor
                .as_ref()
                .and_then(|descriptor| descriptor.profile.as_ref())
                .map(|profile| profile.api_family.as_str()),
            Some("openai-compatible")
        );
        assert_eq!(
            provider
                .runtime_profile
                .as_ref()
                .map(|profile| profile.profile.source.as_str()),
            Some("runtime_fingerprint")
        );
        assert_eq!(
            provider
                .runtime_profile
                .as_ref()
                .map(|profile| profile.profile.api_shape.as_str()),
            Some("chat-completions")
        );
        assert_eq!(
            provider
                .runtime_profile
                .as_ref()
                .map(|profile| profile.profile.quirks.clone()),
            Some(vec!["requires-thinking-replay".to_string()])
        );

        assert!(!policy.compaction.auto);
        assert!(policy.compaction.prune);
        assert_eq!(policy.compaction.reserved, Some(512));

        let external = policy.external_adapter.expect("external adapter policy");
        assert_eq!(
            external.last_ingress_source,
            "external:generic-webhook:generic"
        );
        assert_eq!(
            external.last_ingress_policy.as_deref(),
            Some("external_adapter_metadata_only")
        );
        assert_eq!(external.last_ingress_batch_count, Some(1));
    }
}
