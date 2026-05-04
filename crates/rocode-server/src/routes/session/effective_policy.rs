use std::collections::HashMap;
use std::sync::Arc;

use rocode_orchestrator::SkillTreeRequestPlan;
use rocode_provider::{
    cache::ProviderProfileFingerprint,
    provider_connection_descriptor_candidate_from_config_provider, Provider,
    ProviderProfileDescriptorView,
};
use rocode_session::{resolved_compaction_config, Session};
use rocode_types::{
    SessionEffectiveCompactionPolicy, SessionEffectiveExternalAdapterPolicy,
    SessionEffectiveMemoryPolicy, SessionEffectivePolicyView, SessionEffectiveProviderPolicy,
    SessionEffectiveProviderRuntimeProfile, SessionEffectiveSchedulerPolicy,
    SessionEffectiveSkillTreePolicy, SessionMemoryInsight,
};

use crate::server::bootstrap_config_from_config;
use crate::ServerState;

use super::scheduler::{
    resolve_prompt_request_config, resolve_scheduler_request_defaults, scheduler_mode_kind,
    PromptRequestConfigInput, ResolvedPromptRequestConfig,
};
use super::session_crud::{
    session_agent_override, session_model_override, session_scheduler_profile_override,
    session_variant_override,
};

const SCHEDULER_SOURCE_NONE: &str = "none";
const SCHEDULER_SOURCE_CONFIG_DEFAULT: &str = "config_default";
const SCHEDULER_SOURCE_SESSION_METADATA: &str = "session_metadata";
const SCHEDULER_SOURCE_LEGACY_SESSION_METADATA: &str = "legacy_session_metadata";
const SKILL_TREE_SOURCE_CONFIG: &str = "config_composition";
const SKILL_TREE_SOURCE_SCHEDULER: &str = "scheduler_profile";

pub(super) async fn build_session_effective_policy(
    state: &Arc<ServerState>,
    session: &Session,
    memory_insight: Option<&SessionMemoryInsight>,
) -> SessionEffectivePolicyView {
    let config = state.config_store.config();
    let metadata = &session.record().metadata;
    let raw_scheduler_profile = metadata_string(metadata, "scheduler_profile");
    let raw_resolved_scheduler_profile = metadata_string(metadata, "resolved_scheduler_profile");
    let requested_scheduler_profile = session_scheduler_profile_override(session);
    let requested_agent = session_agent_override(session);
    let requested_model = session_model_override(session);
    let requested_variant = session_variant_override(session);
    let mut warnings = Vec::new();

    if raw_scheduler_profile.is_none() && raw_resolved_scheduler_profile.is_some() {
        warnings.push(
            "session is still relying on legacy `resolved_scheduler_profile` fallback metadata"
                .to_string(),
        );
    }

    let memory = build_memory_policy(state, session, memory_insight, &mut warnings).await;
    let external_adapter = build_external_adapter_policy(metadata);
    let compaction = build_compaction_policy(state);

    let resolution = resolve_prompt_request_config(PromptRequestConfigInput {
        state,
        config: &config,
        session_id: &session.record().id,
        requested_agent: requested_agent.as_deref(),
        requested_scheduler_profile: requested_scheduler_profile.as_deref(),
        scheduler_profile_override: None,
        request_model: requested_model.as_deref(),
        request_variant: requested_variant.as_deref(),
        route: "session_effective_policy",
    })
    .await;

    let (scheduler, provider, skill_tree) = match resolution {
        Ok(resolved) => (
            Some(build_scheduler_policy(
                raw_scheduler_profile.as_deref(),
                raw_resolved_scheduler_profile.as_deref(),
                requested_scheduler_profile.as_deref(),
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
            build_skill_tree_policy(
                &config,
                resolved.request_skill_tree_plan.as_ref(),
                resolved.scheduler_skill_tree_applied,
                &mut warnings,
            ),
        ),
        Err(error) => {
            warnings.push(format!(
                "effective policy could not fully resolve current request inputs: {}",
                error
            ));
            (
                Some(build_scheduler_fallback_policy(
                    &config,
                    raw_scheduler_profile.as_deref(),
                    raw_resolved_scheduler_profile.as_deref(),
                    requested_scheduler_profile.as_deref(),
                )),
                None,
                build_skill_tree_policy(&config, None, false, &mut warnings),
            )
        }
    };

    SessionEffectivePolicyView {
        session_id: session.record().id.clone(),
        scheduler,
        provider,
        skill_tree,
        memory,
        compaction,
        external_adapter,
        warnings,
    }
}

fn build_scheduler_policy(
    raw_scheduler_profile: Option<&str>,
    raw_resolved_scheduler_profile: Option<&str>,
    requested_scheduler_profile: Option<&str>,
    resolved: &ResolvedPromptRequestConfig,
) -> SessionEffectiveSchedulerPolicy {
    let effective_profile = resolved.scheduler_profile_name.clone();
    let mode_kind = effective_profile
        .as_deref()
        .map(scheduler_mode_kind)
        .map(str::to_string);

    SessionEffectiveSchedulerPolicy {
        requested_profile: requested_scheduler_profile.map(str::to_string),
        effective_profile,
        source: scheduler_source_label(
            raw_scheduler_profile,
            raw_resolved_scheduler_profile,
            requested_scheduler_profile,
            resolved.scheduler_applied,
        )
        .to_string(),
        applied: resolved.scheduler_applied,
        mode_kind,
        root_agent: resolved.scheduler_root_agent.clone(),
        resolved_agent: resolved
            .resolved_agent
            .as_ref()
            .map(|agent| agent.name.clone()),
    }
}

fn build_scheduler_fallback_policy(
    config: &rocode_config::Config,
    raw_scheduler_profile: Option<&str>,
    raw_resolved_scheduler_profile: Option<&str>,
    requested_scheduler_profile: Option<&str>,
) -> SessionEffectiveSchedulerPolicy {
    let defaults = resolve_scheduler_request_defaults(config, requested_scheduler_profile);
    let effective_profile = defaults
        .as_ref()
        .and_then(|defaults| defaults.profile_name.clone())
        .or_else(|| requested_scheduler_profile.map(str::to_string));
    let applied = defaults.is_some();
    let mode_kind = effective_profile
        .as_deref()
        .map(scheduler_mode_kind)
        .map(str::to_string);

    SessionEffectiveSchedulerPolicy {
        requested_profile: requested_scheduler_profile.map(str::to_string),
        effective_profile,
        source: scheduler_source_label(
            raw_scheduler_profile,
            raw_resolved_scheduler_profile,
            requested_scheduler_profile,
            applied,
        )
        .to_string(),
        applied,
        mode_kind,
        root_agent: defaults.and_then(|defaults| defaults.root_agent_name),
        resolved_agent: None,
    }
}

fn scheduler_source_label(
    raw_scheduler_profile: Option<&str>,
    raw_resolved_scheduler_profile: Option<&str>,
    requested_scheduler_profile: Option<&str>,
    scheduler_applied: bool,
) -> &'static str {
    if raw_scheduler_profile.is_some() && requested_scheduler_profile.is_some() {
        return SCHEDULER_SOURCE_SESSION_METADATA;
    }
    if raw_scheduler_profile.is_none()
        && raw_resolved_scheduler_profile.is_some()
        && requested_scheduler_profile.is_some()
    {
        return SCHEDULER_SOURCE_LEGACY_SESSION_METADATA;
    }
    if scheduler_applied {
        return SCHEDULER_SOURCE_CONFIG_DEFAULT;
    }
    SCHEDULER_SOURCE_NONE
}

fn build_provider_policy(
    config: &rocode_config::Config,
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
            api_family: fingerprint.api_family.as_str().to_string(),
            api_shape: fingerprint.api_shape.as_str().to_string(),
            transport: fingerprint.transport.as_str().to_string(),
            usage_shape: fingerprint.usage_shape.as_str().to_string(),
            cache_family: fingerprint.cache_family.as_str().to_string(),
            quirks: Vec::new(),
        },
        profile_hash: fingerprint.profile_hash.clone(),
    }
}

fn build_skill_tree_policy(
    config: &rocode_config::Config,
    plan: Option<&SkillTreeRequestPlan>,
    scheduler_skill_tree_applied: bool,
    warnings: &mut Vec<String>,
) -> Option<SessionEffectiveSkillTreePolicy> {
    let configured_skill_tree = config
        .composition
        .as_ref()
        .and_then(|composition| composition.skill_tree.as_ref());
    let enabled = configured_skill_tree
        .map(|skill_tree| !matches!(skill_tree.enabled, Some(false)))
        .unwrap_or(true);

    if let Some(plan) = plan {
        let source = if scheduler_skill_tree_applied {
            SKILL_TREE_SOURCE_SCHEDULER
        } else {
            SKILL_TREE_SOURCE_CONFIG
        };
        return Some(SessionEffectiveSkillTreePolicy {
            configured: true,
            enabled,
            applied: true,
            source: source.to_string(),
            estimated_tokens: Some(plan.estimated_tokens() as u64),
            token_budget: plan.token_budget.map(|value| value as u64),
            truncation_strategy: Some(plan.truncation_strategy.as_label().to_string()),
            truncated: Some(plan.is_truncated()),
        });
    }

    let Some(skill_tree) = configured_skill_tree else {
        return None;
    };

    if enabled {
        warnings.push(
            "skill tree config is present but no request skill tree plan resolved".to_string(),
        );
    }

    Some(SessionEffectiveSkillTreePolicy {
        configured: true,
        enabled,
        applied: false,
        source: SKILL_TREE_SOURCE_CONFIG.to_string(),
        estimated_tokens: None,
        token_budget: skill_tree.token_budget.map(|value| value as u64),
        truncation_strategy: skill_tree.truncation_strategy.clone(),
        truncated: None,
    })
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
            == Some(rocode_session::prompt::INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY);
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
    use async_trait::async_trait;
    use rocode_config::{
        CompactionConfig as AppCompactionConfig, CompositionConfig, Config, ConfigStore,
        ProviderConfig, SkillTreeConfig, SkillTreeNodeConfig,
    };
    use rocode_provider::{
        cache::ProviderProfileFingerprint, ModelInfo, ProviderError, ProviderProfileResolver,
        StreamResult,
    };
    use rocode_session::Session;

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
            _request: rocode_provider::ChatRequest,
        ) -> Result<rocode_provider::ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "mock provider does not handle chat".to_string(),
            ))
        }

        async fn chat_stream(
            &self,
            _request: rocode_provider::ChatRequest,
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
    async fn effective_policy_resolves_scheduler_provider_skill_tree_and_external_adapter() {
        let mut state = ServerState::new();
        state.config_store = Arc::new(ConfigStore::new(Config {
            model: Some("openai/gpt-4o".to_string()),
            skill_paths: HashMap::from([(
                "skill://root".to_string(),
                "# Root Rule\nUse the shared coding policy.".to_string(),
            )]),
            composition: Some(CompositionConfig {
                skill_tree: Some(SkillTreeConfig {
                    enabled: Some(true),
                    root: Some(SkillTreeNodeConfig {
                        node_id: "root".to_string(),
                        markdown_path: "skill://root".to_string(),
                        children: Vec::new(),
                    }),
                    separator: None,
                    token_budget: Some(64),
                    truncation_strategy: Some("tail".to_string()),
                }),
            }),
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    name: Some("OpenAI".to_string()),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    api_style: Some("closeai-compatible".to_string()),
                    api_shape: Some("chat-completions".to_string()),
                    transport: Some("bearer".to_string()),
                    usage_shape: Some("closeai-cached-tokens".to_string()),
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

        let runtime_profile =
            ProviderProfileResolver::resolve_with_npm("openai", "@ai-sdk/openai", &HashMap::new());
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
        session.insert_metadata("scheduler_profile", serde_json::json!("prometheus"));
        session.insert_metadata(
            "last_ingress_source".to_string(),
            serde_json::json!("external:generic-webhook:generic"),
        );
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(
                rocode_session::prompt::INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY
            ),
        );
        session.insert_metadata("last_ingress_batch_count".to_string(), serde_json::json!(1));

        let policy = build_session_effective_policy(&state, &session, None).await;

        assert_eq!(policy.session_id, session_id);
        assert!(policy.warnings.is_empty(), "{:?}", policy.warnings);

        let scheduler = policy.scheduler.expect("scheduler policy");
        assert_eq!(scheduler.requested_profile.as_deref(), Some("prometheus"));
        assert_eq!(scheduler.effective_profile.as_deref(), Some("prometheus"));
        assert_eq!(scheduler.source, "session_metadata");
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
                .map(|profile| profile.api_family.as_str()),
            Some("closeai-compatible")
        );
        assert_eq!(
            provider
                .runtime_profile
                .as_ref()
                .map(|profile| profile.profile.api_shape.as_str()),
            Some("chat-completions")
        );

        let skill_tree = policy.skill_tree.expect("skill tree policy");
        assert!(skill_tree.configured);
        assert!(skill_tree.enabled);
        assert!(skill_tree.applied);
        assert_eq!(skill_tree.source, "config_composition");
        assert_eq!(skill_tree.token_budget, Some(64));
        assert_eq!(skill_tree.truncation_strategy.as_deref(), Some("tail"));

        assert_eq!(policy.compaction.auto, false);
        assert_eq!(policy.compaction.prune, true);
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

    #[tokio::test]
    async fn effective_policy_warns_on_legacy_scheduler_profile_fallback() {
        let state = Arc::new(ServerState::new());
        let mut session = Session::new("session-legacy".to_string(), ".".to_string());
        session.insert_metadata(
            "resolved_scheduler_profile".to_string(),
            serde_json::json!("prometheus"),
        );

        let policy = build_session_effective_policy(&state, &session, None).await;

        assert!(policy
            .warnings
            .iter()
            .any(|warning| warning.contains("legacy `resolved_scheduler_profile`")));
        assert_eq!(
            policy
                .scheduler
                .as_ref()
                .map(|scheduler| scheduler.source.as_str()),
            Some("legacy_session_metadata")
        );
    }
}
