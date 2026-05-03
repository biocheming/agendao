use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use rocode_content::output_blocks::{OutputBlock, StatusBlock};
use rocode_execution_types::{session_runtime_request_defaults, CompiledExecutionRequest};
use rocode_orchestrator::runtime::events::{
    CancelToken as RuntimeCancelToken, LoopError as RuntimeLoopError,
};
use rocode_orchestrator::runtime::policy::{LoopPolicy, ToolDedupScope};
use rocode_orchestrator::runtime::run_loop;
use rocode_orchestrator::runtime::{SimpleModelCaller, SimpleModelCallerConfig};
use rocode_plugin::{HookContext, HookEvent};
use rocode_provider::cache::{
    inspect_cache_fingerprint_change, CacheBustSummary, CacheProtocolFamily,
    CacheRequestFingerprint, CloseAiCacheFingerprint, EthnopicCacheFingerprint,
    EthnopicCachePolicy, PromptSurfaceFingerprint, ProviderProfileFingerprint,
    CACHE_BUST_INSPECTION_METADATA_KEY, CACHE_BUST_SUMMARY_METADATA_KEY,
    CACHE_REQUEST_FINGERPRINT_METADATA_KEY,
};
use rocode_provider::transform::{apply_caching, ProviderType};
use rocode_provider::{Provider, ToolDefinition};

use crate::compaction::{run_compaction, CompactionResult};
use crate::message_v2::ModelRef as V2ModelRef;
use crate::{MessageRole, Session, SessionMessage};

use super::runtime_step::{SessionStepRuntimeOutput, SessionStepSink, SessionStepToolDispatcher};
use super::{
    apply_chat_message_hook_outputs, apply_chat_messages_hook_outputs, is_terminal_finish,
    merge_tool_definitions, session_message_hook_payload, skill_reflection, tools_and_output,
    PromptHooks, PromptInput, PromptRequestContext, SessionPrompt, SessionStepShared, MAX_STEPS,
    PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY,
    PROMPT_SURFACE_SNAPSHOT_INVALIDATION_METADATA_KEY, STREAM_UPDATE_INTERVAL_MS,
};

#[derive(Clone)]
struct SessionStepCancelToken {
    user_cancel: CancellationToken,
    step_complete: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PromptSurfaceRuntimeSnapshot {
    session_id: String,
    generation: u64,
    created_at_ms: i64,
    updated_at_ms: i64,
    protocol_family: CacheProtocolFamily,
    provider_id: String,
    model_id: String,
    api_shape: Option<rocode_provider::cache::CloseAiCompatibleApiShape>,
    system_hash: String,
    stable_system_surface_hash: String,
    tool_surface_hash: String,
    tool_source_surface_hash: String,
    provider_params_hash: String,
    tool_policy_hash: Option<String>,
    reasoning_mode_hash: Option<String>,
    output_projection_policy_hash: String,
    scc_stable_refs_hash: Option<String>,
    closeai_prompt_cache_key: Option<String>,
    ethnopic_policy_hash: Option<String>,
    ethnopic_breakpoint_plan_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ingress_policy_hash: Option<String>,
    invalidation: Option<PromptSurfaceInvalidation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PromptSurfaceInvalidation {
    severity: rocode_provider::cache::CacheBustSeverity,
    reason: String,
    changed_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptSurfaceStableFields {
    protocol_family: CacheProtocolFamily,
    provider_id: String,
    model_id: String,
    api_shape: Option<rocode_provider::cache::CloseAiCompatibleApiShape>,
    system_hash: String,
    stable_system_surface_hash: String,
    tool_surface_hash: String,
    tool_source_surface_hash: String,
    provider_params_hash: String,
    tool_policy_hash: Option<String>,
    reasoning_mode_hash: Option<String>,
    output_projection_policy_hash: String,
    scc_stable_refs_hash: Option<String>,
    closeai_prompt_cache_key: Option<String>,
    ethnopic_policy_hash: Option<String>,
    ethnopic_breakpoint_plan_hash: Option<String>,
    ingress_policy_hash: Option<String>,
}

impl RuntimeCancelToken for SessionStepCancelToken {
    fn is_cancelled(&self) -> bool {
        self.user_cancel.is_cancelled() || self.step_complete.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod cache_fingerprint_tests {
    use super::*;
    use rocode_provider::Message;

    #[test]
    fn cache_request_fingerprint_records_closeai_family_without_wire_changes() {
        let messages = vec![Message::system("system"), Message::user("hello")];
        let compiled = CompiledExecutionRequest {
            model_id: "gpt-test".to_string(),
            provider_options: Some(HashMap::from([(
                "promptCacheKey".to_string(),
                serde_json::json!("rocode:key"),
            )])),
            ..Default::default()
        };

        let fingerprint = SessionPrompt::cache_request_fingerprint(
            "ses_test",
            "openai",
            "gpt-test",
            Some("system"),
            &messages,
            &[],
            &compiled,
            ProviderType::OpenAI,
            Some(openai_provider_profile_fingerprint()),
        );

        assert_eq!(fingerprint.family, CacheProtocolFamily::CloseAiCompatible);
        let provider_profile = fingerprint
            .provider_profile
            .as_ref()
            .expect("provider profile fingerprint should be recorded");
        assert_eq!(provider_profile.provider_id, "openai");
        assert_eq!(
            provider_profile.api_family,
            rocode_provider::ProviderApiFamily::CloseAiCompatible
        );
        assert_eq!(
            provider_profile.api_shape,
            rocode_provider::ProviderApiShape::ChatCompletions
        );
        assert_eq!(
            fingerprint
                .closeai
                .as_ref()
                .and_then(|value| value.prompt_cache_key.as_deref()),
            Some("rocode:key")
        );
        assert!(fingerprint.ethnopic.is_none());
    }

    #[test]
    fn cache_request_fingerprint_ignores_profile_like_request_options() {
        let messages = vec![Message::system("system"), Message::user("hello")];
        let compiled = CompiledExecutionRequest {
            model_id: "gpt-test".to_string(),
            provider_options: Some(HashMap::from([(
                "transport".to_string(),
                serde_json::json!("bearer"),
            )])),
            ..Default::default()
        };

        let fingerprint = SessionPrompt::cache_request_fingerprint(
            "ses_test",
            "custom-provider",
            "gpt-test",
            Some("system"),
            &messages,
            &[],
            &compiled,
            ProviderType::OpenAI,
            None,
        );

        assert_eq!(fingerprint.family, CacheProtocolFamily::CloseAiCompatible);
        assert!(
            fingerprint.provider_profile.is_none(),
            "request options are not provider profile authority"
        );
        assert!(fingerprint.closeai.is_some());
    }

    #[test]
    fn cache_request_fingerprint_records_generated_closeai_cache_key() {
        let messages = vec![Message::system("system"), Message::user("hello")];
        let compiled = CompiledExecutionRequest {
            model_id: "gpt-test".to_string(),
            provider_options: Some(HashMap::from([(
                "cacheStage".to_string(),
                serde_json::json!("chat"),
            )])),
            ..Default::default()
        };

        let fingerprint = SessionPrompt::cache_request_fingerprint(
            "ses_generated_key",
            "openai",
            "gpt-test",
            Some("system"),
            &messages,
            &[],
            &compiled,
            ProviderType::OpenAI,
            None,
        );

        let prompt_cache_key = fingerprint
            .closeai
            .as_ref()
            .and_then(|value| value.prompt_cache_key.as_deref())
            .expect("generated prompt cache key should be recorded");
        assert!(prompt_cache_key.starts_with("rocode:"));
        assert!(!prompt_cache_key.contains("ses_generated_key"));
    }

    fn openai_provider_profile_fingerprint() -> ProviderProfileFingerprint {
        provider_profile_fingerprint("openai", HashMap::new())
    }

    fn provider_profile_fingerprint(
        provider_id: &str,
        options: HashMap<String, serde_json::Value>,
    ) -> ProviderProfileFingerprint {
        let profile = rocode_provider::ProviderProfileResolver::try_resolve_with_options(
            provider_id,
            &options,
        )
        .expect("provider profile should resolve");
        ProviderProfileFingerprint::from_profile(&profile)
    }

    #[test]
    fn latest_cache_request_fingerprint_reads_previous_assistant_metadata() {
        let mut session = Session::new("project", "/tmp");
        let fingerprint = CacheRequestFingerprint {
            family: CacheProtocolFamily::CloseAiCompatible,
            surface: PromptSurfaceFingerprint {
                model: "model-a".to_string(),
                system_hash: "system-a".to_string(),
                tools_hash: "tools-a".to_string(),
                message_prefix_hash: "messages-a".to_string(),
                api_params_hash: "params-a".to_string(),
            },
            provider_profile: None,
            closeai: None,
            ethnopic: None,
        };
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            CACHE_REQUEST_FINGERPRINT_METADATA_KEY.to_string(),
            serde_json::to_value(&fingerprint).expect("fingerprint serializes"),
        );

        let loaded = SessionPrompt::latest_cache_request_fingerprint(&session)
            .expect("fingerprint should load");

        assert_eq!(loaded, fingerprint);
    }

    #[test]
    fn latest_prompt_surface_snapshot_reads_previous_assistant_metadata() {
        let mut session = Session::new("project", "/tmp");
        let snapshot = PromptSurfaceRuntimeSnapshot {
            session_id: session.id.clone(),
            generation: 4,
            created_at_ms: 100,
            updated_at_ms: 200,
            protocol_family: CacheProtocolFamily::CloseAiCompatible,
            provider_id: "openai".to_string(),
            model_id: "gpt-test".to_string(),
            api_shape: Some(rocode_provider::cache::CloseAiCompatibleApiShape::ChatCompletions),
            system_hash: "system-a".to_string(),
            stable_system_surface_hash: "stable-system-a".to_string(),
            tool_surface_hash: "tools-a".to_string(),
            tool_source_surface_hash: "tool-source-a".to_string(),
            provider_params_hash: "params-a".to_string(),
            tool_policy_hash: None,
            reasoning_mode_hash: None,
            output_projection_policy_hash: "projection-a".to_string(),
            scc_stable_refs_hash: None,
            closeai_prompt_cache_key: Some("rocode:key".to_string()),
            ethnopic_policy_hash: None,
            ethnopic_breakpoint_plan_hash: None,
            ingress_policy_hash: None,
            invalidation: None,
        };
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::to_value(&snapshot).expect("snapshot serializes"),
        );

        let loaded = SessionPrompt::latest_prompt_surface_runtime_snapshot(&session)
            .expect("snapshot should load");

        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn prompt_surface_snapshot_keeps_generation_when_stable_fields_match() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let first_fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &first_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            None,
            first_stable,
            100,
        );

        let second_fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-changed", "params-a");
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &second_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        assert_eq!(second.generation, first.generation);
        assert!(second.invalidation.is_none());
        assert_eq!(
            second.created_at_ms, first.created_at_ms,
            "message-prefix changes are request fingerprint diagnostics, not stable snapshot invalidations"
        );
    }

    #[test]
    fn prompt_surface_snapshot_invalidates_on_tool_surface_change() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let first_fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &first_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            None,
            first_stable,
            100,
        );

        let second_fingerprint =
            test_cache_fingerprint("system-a", "tools-b", "messages-a", "params-a");
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &second_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        let invalidation = second
            .invalidation
            .as_ref()
            .expect("tool surface changes should invalidate stable snapshot");
        assert_eq!(second.generation, first.generation + 1);
        assert_eq!(
            invalidation.severity,
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        assert!(invalidation
            .changed_fields
            .contains(&"toolSurfaceHash".to_string()));
    }

    #[test]
    fn prompt_surface_snapshot_reason_can_drive_cache_bust_summary() {
        let summary = CacheBustSummary {
            status: "stable".to_string(),
            severity: rocode_provider::cache::CacheBustSeverity::Stable,
            primary_cause: None,
            change_count: 0,
        };
        let invalidation = PromptSurfaceInvalidation {
            severity: rocode_provider::cache::CacheBustSeverity::HardBust,
            reason: "prompt surface runtime changed: toolSurfaceHash".to_string(),
            changed_fields: vec!["toolSurfaceHash".to_string()],
        };

        let merged =
            SessionPrompt::merge_snapshot_invalidation_into_summary(summary, Some(&invalidation));

        assert_eq!(merged.status, "degraded");
        assert_eq!(
            merged.severity,
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        assert_eq!(
            merged.primary_cause.as_deref(),
            Some(invalidation.reason.as_str())
        );
    }

    #[test]
    fn stable_system_surface_projection_ignores_dynamic_tail_and_date() {
        let first = "You are ROCode.\n  Today's date: Thu Apr 30 2026\n\n## Exact Recent Tail\n- user `m1`:\nold";
        let second =
            "You are ROCode.\n  Today's date: Fri May 01 2026\n\n## Exact Recent Tail\n- user `m2`:\nnew";

        assert_ne!(
            rocode_provider::cache::text_fingerprint(first),
            rocode_provider::cache::text_fingerprint(second)
        );
        assert_eq!(
            SessionPrompt::stable_system_surface_hash(Some(first)),
            SessionPrompt::stable_system_surface_hash(Some(second))
        );
    }

    #[test]
    fn prompt_surface_snapshot_ignores_dynamic_system_tail_for_generation() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let first_system = "You are ROCode.\n\n## Exact Recent Tail\n- user `m1`:\nprevious output";
        let second_system = "You are ROCode.\n\n## Exact Recent Tail\n- user `m2`:\nlatest output";
        let first_fingerprint =
            test_cache_fingerprint("system-full-a", "tools-a", "messages-a", "params-a");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &first_fingerprint,
            Some(first_system),
            "tool-source-a".to_string(),
        );
        let first = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            None,
            first_stable,
            100,
        );

        let second_fingerprint =
            test_cache_fingerprint("system-full-b", "tools-a", "messages-a", "params-a");
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &second_fingerprint,
            Some(second_system),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        assert_eq!(second.generation, first.generation);
        assert!(second.invalidation.is_none());
        assert_ne!(second.system_hash, first.system_hash);
        assert_eq!(
            second.stable_system_surface_hash,
            first.stable_system_surface_hash
        );
    }

    #[test]
    fn prompt_surface_snapshot_records_ingress_policy_without_user_text() {
        let compiled = CompiledExecutionRequest::default();
        let mut session = Session::new("project", "/tmp");
        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("web"));
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(crate::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY),
        );
        let fingerprint = test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");

        let stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_runtime_snapshot("ses_test", None, stable, 100);

        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("web"));
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(crate::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY),
        );
        let stable_again = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            Some(&first),
            stable_again,
            200,
        );

        assert!(first.ingress_policy_hash.is_some());
        assert_eq!(second.generation, first.generation);
        assert!(second.invalidation.is_none());
    }

    #[test]
    fn prompt_surface_snapshot_soft_degrades_on_ingress_policy_change() {
        let compiled = CompiledExecutionRequest::default();
        let mut session = Session::new("project", "/tmp");
        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("web"));
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(crate::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY),
        );
        let fingerprint = test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");

        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            None,
            first_stable,
            100,
        );

        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("api"));
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(crate::prompt::INGRESS_POLICY_SCHEDULER_METADATA_ONLY),
        );
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_runtime_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        let invalidation = second
            .invalidation
            .as_ref()
            .expect("ingress policy changes should be tracked");
        assert_eq!(second.generation, first.generation + 1);
        assert_eq!(
            invalidation.severity,
            rocode_provider::cache::CacheBustSeverity::SoftDegradation
        );
        assert!(invalidation
            .changed_fields
            .contains(&"ingressPolicyHash".to_string()));
    }

    #[test]
    fn prompt_surface_stable_fields_records_responses_api_shape_from_provider_profile() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let mut fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");
        fingerprint.provider_profile = Some(provider_profile_fingerprint(
            "openai",
            HashMap::from([("useResponsesApi".to_string(), serde_json::json!(true))]),
        ));

        let stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );

        assert_eq!(
            stable.api_shape,
            Some(rocode_provider::cache::CloseAiCompatibleApiShape::Responses)
        );
    }

    #[test]
    fn scc_stable_refs_hash_ignores_memory_anchor_title_text() {
        let mut first = Session::new("project", "/tmp");
        first.insert_metadata(
            "scheduler_session_context_packet".to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 2,
                "exact_recent_tail": [
                    {"message_id": "msg_user", "role": "user", "projected": false}
                ],
                "memory_anchors": [
                    {"record_id": "mem_1", "title": "First title", "kind": "Lesson", "status": "Validated"}
                ],
                "latest_compaction_summary": {"message_id": "msg_compact"}
            }),
        );
        let mut second = Session::new("project", "/tmp");
        second.insert_metadata(
            "scheduler_session_context_packet".to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 2,
                "exact_recent_tail": [
                    {"message_id": "msg_user", "role": "user", "projected": false}
                ],
                "memory_anchors": [
                    {"record_id": "mem_1", "title": "Changed title", "kind": "Lesson", "status": "Validated"}
                ],
                "latest_compaction_summary": {"message_id": "msg_compact"}
            }),
        );

        assert_eq!(
            SessionPrompt::scc_stable_refs_hash(&first),
            SessionPrompt::scc_stable_refs_hash(&second)
        );
    }

    #[test]
    fn tool_policy_hash_tracks_provider_tool_choice_fields() {
        let options = HashMap::from([(
            "tool_choice".to_string(),
            serde_json::json!({"type": "function", "function": {"name": "read"}}),
        )]);

        assert!(SessionPrompt::tool_policy_hash(Some(&options)).is_some());
        assert!(SessionPrompt::tool_policy_hash(None).is_none());
    }

    fn test_cache_fingerprint(
        system_hash: &str,
        tools_hash: &str,
        message_prefix_hash: &str,
        api_params_hash: &str,
    ) -> CacheRequestFingerprint {
        CacheRequestFingerprint {
            family: CacheProtocolFamily::CloseAiCompatible,
            surface: PromptSurfaceFingerprint {
                model: "gpt-test".to_string(),
                system_hash: system_hash.to_string(),
                tools_hash: tools_hash.to_string(),
                message_prefix_hash: message_prefix_hash.to_string(),
                api_params_hash: api_params_hash.to_string(),
            },
            provider_profile: None,
            closeai: Some(CloseAiCacheFingerprint {
                prompt_cache_key: Some("rocode:key".to_string()),
                prompt_cache_retention: None,
                previous_response_id_used: false,
                incremental_input_used: false,
                cached_tokens_observed: 0,
            }),
            ethnopic: None,
        }
    }
}

#[derive(Clone)]
struct PromptLoopContext {
    provider: Arc<dyn Provider>,
    model_id: String,
    provider_id: String,
    agent_name: Option<String>,
    system_prompt: Option<String>,
    tools: Vec<ToolDefinition>,
    tool_source_digests: Vec<rocode_provider::cache::ToolSurfaceSourceDigest>,
    compiled_request: CompiledExecutionRequest,
    hooks: PromptHooks,
    config_store: Option<Arc<rocode_config::ConfigStore>>,
    memory_authority: Option<Arc<rocode_memory::MemoryAuthority>>,
}

#[derive(Clone)]
struct RuntimeStepContext {
    provider: Arc<dyn Provider>,
    model_id: String,
    provider_id: String,
    agent_name: Option<String>,
    compiled_request: CompiledExecutionRequest,
    hooks: PromptHooks,
    config_store: Option<Arc<rocode_config::ConfigStore>>,
}

struct RuntimeStepInput {
    session_id: String,
    assistant_index: usize,
    chat_messages: Vec<rocode_provider::Message>,
    tool_registry: Arc<rocode_tool::ToolRegistry>,
    step_ctx: RuntimeStepContext,
}

impl SessionPrompt {
    async fn emit_context_compaction_status(
        output_block_hook: Option<&super::OutputBlockHook>,
        session_id: &str,
        block: StatusBlock,
    ) {
        let Some(output_block_hook) = output_block_hook else {
            return;
        };
        output_block_hook(super::OutputBlockEvent {
            session_id: session_id.to_string(),
            block: OutputBlock::Status(block),
            id: None,
        })
        .await;
    }

    pub async fn prompt_with_update_hook(
        &self,
        input: PromptInput,
        session: &mut Session,
        request: PromptRequestContext,
    ) -> anyhow::Result<()> {
        self.prompt_with_optional_reservation(input, session, request, None)
            .await
    }

    pub async fn prompt_with_reserved_update_hook(
        &self,
        input: PromptInput,
        session: &mut Session,
        request: PromptRequestContext,
        token: CancellationToken,
    ) -> anyhow::Result<()> {
        self.prompt_with_optional_reservation(input, session, request, Some(token))
            .await
    }

    async fn prompt_with_optional_reservation(
        &self,
        input: PromptInput,
        session: &mut Session,
        request: PromptRequestContext,
        reserved_token: Option<CancellationToken>,
    ) -> anyhow::Result<()> {
        let PromptRequestContext {
            provider,
            system_prompt,
            memory_prefetch,
            tools,
            tool_source_digests,
            compiled_request,
            hooks,
        } = request;
        let system_prompt =
            skill_reflection::augment_system_prompt_with_skill_reflection(session, system_prompt);

        let used_reserved_token = reserved_token.is_some();
        if !used_reserved_token && Self::is_duplicate_ingress_turn(session, &input) {
            tracing::debug!(
                session_id = %input.session_id,
                "ignored duplicate ingress turn before prompt execution"
            );
            return Ok(());
        }

        let token = match reserved_token {
            Some(token) => token,
            None => {
                self.assert_not_busy(&input.session_id).await?;
                let cancel_token = self.start(&input.session_id).await;
                match cancel_token {
                    Some(t) => t,
                    None => return Err(anyhow::anyhow!("Session already running")),
                }
            }
        };
        let session_id = input.session_id.clone();

        if used_reserved_token && Self::is_duplicate_ingress_turn(session, &input) {
            tracing::debug!(
                session_id = %input.session_id,
                "ignored duplicate ingress turn after reserved ingress window"
            );
            self.finish_run(&session_id).await;
            return Ok(());
        }

        let model_id = input
            .model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let provider_id = input
            .model
            .as_ref()
            .map(|m| m.provider_id.clone())
            .unwrap_or_else(|| "ethnopic".to_string());

        let result = async {
            Self::record_ingress_turn(session, &input);

            self.create_user_message(&input, session).await?;
            self.apply_runtime_workspace_context(session).await?;
            Self::apply_runtime_memory_prefetch(session, memory_prefetch.as_ref())?;
            Self::annotate_latest_user_message(session, &input, system_prompt.as_deref());

            if session.is_default_title() {
                if let Some(text) = session
                    .messages
                    .iter()
                    .find(|m| matches!(m.role, MessageRole::User))
                    .map(|m| m.get_text())
                {
                    let immediate = tools_and_output::generate_session_title(
                        &tools_and_output::sanitize_session_title_source(&text),
                    );
                    if !immediate.is_empty() && immediate != "New Session" {
                        session.set_auto_title(immediate);
                    }
                }
            }

            session.touch();
            Self::emit_session_update(hooks.update_hook.as_ref(), session);

            if input.no_reply {
                return Ok(());
            }

            {
                let mut session_state = self.session_state.write().await;
                session_state.set_busy(&input.session_id);
            }

            let result = self
                .loop_inner(
                    session_id.clone(),
                    token,
                    session,
                    PromptLoopContext {
                        provider,
                        model_id,
                        provider_id,
                        agent_name: input.agent.clone(),
                        system_prompt,
                        tools,
                        tool_source_digests,
                        compiled_request,
                        hooks,
                        config_store: self.config_store.clone(),
                        memory_authority: self.memory_authority.clone(),
                    },
                )
                .await;

            if let Err(e) = result {
                tracing::error!("Prompt loop error for session {}: {}", session_id, e);
                return Err(e);
            }

            // Nudge-triggered background consolidation: after a completed turn,
            // scan the execution evidence and run deterministic memory review if
            // enough tool/error/skill signals were produced.
            let nudge = crate::RuntimeReviewNudge::from_session(session, 0);
            let decision = self.maybe_enqueue_background_review(&nudge).await;
            crate::maybe_append_proposal_notice(session, &decision);

            Ok(())
        }
        .await;

        self.finish_run(&session_id).await;
        result
    }

    fn ingress_scoped_idempotency_key(input: &PromptInput) -> Option<String> {
        let ingress = input.ingress.as_ref()?;
        let key = ingress.idempotency_key.as_deref()?.trim();
        if key.is_empty() {
            return None;
        }
        Some(format!(
            "{}:{}:{}",
            ingress.session_id,
            serde_json::to_string(&ingress.source).unwrap_or_else(|_| "\"unknown\"".to_string()),
            key
        ))
    }

    fn is_duplicate_ingress_turn(session: &Session, input: &PromptInput) -> bool {
        let Some(scoped_key) = Self::ingress_scoped_idempotency_key(input) else {
            return false;
        };
        session
            .metadata
            .get("ingress_seen_idempotency_keys")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(scoped_key.as_str()))
            })
            .unwrap_or(false)
    }

    fn record_ingress_turn(session: &mut Session, input: &PromptInput) {
        let Some(ingress) = input.ingress.as_ref() else {
            return;
        };
        session.insert_metadata(
            "last_ingress_source".to_string(),
            serde_json::json!(&ingress.source),
        );
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(&ingress.stabilization.policy),
        );
        session.insert_metadata(
            "last_ingress_batch_count".to_string(),
            serde_json::json!(ingress.stabilization.batch_count),
        );

        let Some(scoped_key) = Self::ingress_scoped_idempotency_key(input) else {
            return;
        };
        let mut keys = session
            .metadata
            .get("ingress_seen_idempotency_keys")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if !keys
            .iter()
            .any(|item| item.as_str() == Some(scoped_key.as_str()))
        {
            keys.push(serde_json::json!(scoped_key));
            const MAX_INGRESS_IDEMPOTENCY_KEYS: usize = 64;
            if keys.len() > MAX_INGRESS_IDEMPOTENCY_KEYS {
                let start = keys.len() - MAX_INGRESS_IDEMPOTENCY_KEYS;
                keys = keys.split_off(start);
            }
            session.insert_metadata(
                "ingress_seen_idempotency_keys".to_string(),
                serde_json::Value::Array(keys),
            );
        }
    }

    pub async fn resume_session(
        &self,
        session_id: &str,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        compiled_request: CompiledExecutionRequest,
    ) -> anyhow::Result<()> {
        let system_prompt =
            skill_reflection::augment_system_prompt_with_skill_reflection(session, system_prompt);
        let token = self.resume(session_id).await;

        let token = match token {
            Some(t) => t,
            None => {
                return Err(anyhow::anyhow!(
                    "Session {} is not running, cannot resume",
                    session_id
                ));
            }
        };

        let model = session.messages.iter().rev().find_map(|m| match m.role {
            MessageRole::User => session
                .metadata
                .get("model_provider")
                .and_then(|p| p.as_str())
                .zip(session.metadata.get("model_id").and_then(|i| i.as_str()))
                .map(|(provider_id, model_id)| super::ModelRef {
                    provider_id: provider_id.to_string(),
                    model_id: model_id.to_string(),
                }),
            _ => None,
        });

        let model_id = model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let provider_id = model
            .as_ref()
            .map(|m| m.provider_id.clone())
            .unwrap_or_else(|| "ethnopic".to_string());

        let session_id = session_id.to_string();
        let resume_agent = session
            .metadata
            .get("agent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let compiled_request = compiled_request.inherit_missing(&session_runtime_request_defaults(
            session
                .metadata
                .get("model_variant")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        ));

        {
            let mut session_state = self.session_state.write().await;
            session_state.set_busy(&session_id);
        }

        let result = self
            .loop_inner(
                session_id.clone(),
                token,
                session,
                PromptLoopContext {
                    provider,
                    model_id,
                    provider_id,
                    agent_name: resume_agent,
                    system_prompt,
                    tools,
                    tool_source_digests: Vec::new(),
                    compiled_request: compiled_request.clone(),
                    hooks: PromptHooks::default(),
                    config_store: self.config_store.clone(),
                    memory_authority: self.memory_authority.clone(),
                },
            )
            .await;

        self.finish_run(&session_id).await;

        if let Err(e) = result {
            tracing::error!("Resume prompt loop error for session {}: {}", session_id, e);
            return Err(e);
        }

        Ok(())
    }

    async fn run_runtime_step(
        &self,
        token: CancellationToken,
        session: &mut Session,
        resolved_tools: Vec<ToolDefinition>,
        input: RuntimeStepInput,
    ) -> anyhow::Result<SessionStepRuntimeOutput> {
        let assistant_message_id = session
            .messages
            .get(input.assistant_index)
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let shared = Arc::new(Mutex::new(SessionStepShared {
            assistant_message_id: Some(assistant_message_id),
        }));
        let step_complete = Arc::new(AtomicBool::new(false));
        let cancel = SessionStepCancelToken {
            user_cancel: token.clone(),
            step_complete: step_complete.clone(),
        };

        let subsessions = Arc::new(Mutex::new(Self::load_persisted_subsessions(session)));

        let model = SimpleModelCaller {
            provider: input.step_ctx.provider.clone(),
            config: SimpleModelCallerConfig {
                request: input
                    .step_ctx
                    .compiled_request
                    .with_model(input.step_ctx.model_id.clone())
                    .inherit_missing(&session_runtime_request_defaults(None)),
            },
        };
        let allowed_tools = Some(Arc::new(
            resolved_tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<HashSet<_>>(),
        ));

        let tools = SessionStepToolDispatcher {
            session_id: input.session_id.clone(),
            directory: session.directory.clone(),
            agent_name: input.step_ctx.agent_name.clone().unwrap_or_default(),
            abort_token: token.clone(),
            tool_registry: input.tool_registry,
            provider: input.step_ctx.provider.clone(),
            provider_id: input.step_ctx.provider_id.clone(),
            model_id: input.step_ctx.model_id.clone(),
            resolved_tools,
            allowed_tools,
            shared,
            subsessions: subsessions.clone(),
            agent_lookup: input.step_ctx.hooks.agent_lookup.clone(),
            ask_question_hook: input.step_ctx.hooks.ask_question_hook.clone(),
            ask_permission_hook: input.step_ctx.hooks.ask_permission_hook.clone(),
            publish_bus_hook: input.step_ctx.hooks.publish_bus_hook.clone(),
            tool_runtime_config: self.tool_runtime_config.clone(),
            config_store: input.step_ctx.config_store.clone(),
            runtime_skill_instructions: session.metadata.get("runtime_skill_instructions").cloned(),
        };

        let mut sink = SessionStepSink::new(
            session,
            input.assistant_index,
            input.step_ctx.hooks.update_hook.as_ref(),
            input.step_ctx.hooks.event_broadcast.as_ref(),
            input.step_ctx.hooks.output_block_hook.as_ref(),
            step_complete,
        );
        let policy = LoopPolicy {
            max_steps: Some(MAX_STEPS),
            tool_dedup: ToolDedupScope::None,
            ..Default::default()
        };
        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &policy,
            &cancel,
            input.chat_messages,
        )
        .await;
        let output = sink.into_output();

        let persisted = subsessions.lock().await.clone();
        Self::save_persisted_subsessions(session, &persisted);

        match outcome {
            Ok(_) => Ok(output),
            Err(RuntimeLoopError::ModelError(message)) => Err(anyhow::anyhow!("{}", message)),
            Err(RuntimeLoopError::ToolDispatchError { tool, error }) => {
                let lower = error.to_ascii_lowercase();
                if token.is_cancelled()
                    || lower.contains("cancelled")
                    || lower.contains("canceled")
                    || lower.contains("aborted")
                {
                    Ok(output)
                } else {
                    Err(anyhow::anyhow!(
                        "Tool dispatch failed ({}): {}",
                        tool,
                        error
                    ))
                }
            }
            Err(RuntimeLoopError::Cancelled) => Ok(output),
            Err(RuntimeLoopError::SinkError(message)) | Err(RuntimeLoopError::Other(message)) => {
                Err(anyhow::anyhow!("{}", message))
            }
        }
    }

    async fn maybe_compact_context(
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        session: &mut Session,
        provider: &Arc<dyn Provider>,
        filtered_messages: &[SessionMessage],
        compiled_request: &CompiledExecutionRequest,
        config_store: Option<&rocode_config::ConfigStore>,
        output_block_hook: Option<&super::OutputBlockHook>,
        memory_authority: Option<Arc<rocode_memory::MemoryAuthority>>,
    ) {
        let compaction_config = Self::runtime_compaction_config(config_store);
        if !Self::should_compact(
            filtered_messages,
            provider.as_ref(),
            model_id,
            compiled_request.max_tokens,
            &compaction_config,
            None,
        ) {
            return;
        }

        tracing::info!(
            "Context overflow detected, triggering compaction for session {}",
            session_id
        );
        Self::emit_context_compaction_status(
            output_block_hook,
            session_id,
            StatusBlock::warning("Auto-compacting context to stay within the model window..."),
        )
        .await;

        let parent_id = filtered_messages
            .last()
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let compaction_messages =
            Self::build_chat_messages(filtered_messages, None).unwrap_or_default();
        let compaction_messages_with_parts = Self::to_message_with_parts(
            filtered_messages,
            provider_id,
            model_id,
            &session.directory,
        );
        let model_ref = V2ModelRef {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        };

        match run_compaction::<crate::compaction::NoopSessionOps>(
            session_id,
            &parent_id,
            compaction_messages,
            compaction_messages_with_parts,
            model_ref,
            provider.clone(),
            crate::compaction::RunCompactionOptions {
                abort: CancellationToken::new(),
                auto: true,
                config: Some(compaction_config.clone()),
                session_ops: None,
                memory_authority: memory_authority.clone(),
            },
        )
        .await
        {
            Ok(CompactionResult::Continue) => {
                tracing::info!(
                    "LLM compaction complete for session {}, continuing",
                    session_id
                );
                Self::emit_context_compaction_status(
                    output_block_hook,
                    session_id,
                    StatusBlock::success("Context compacted. Continuing the session."),
                )
                .await;
            }
            Ok(CompactionResult::Stop) => {
                tracing::warn!(
                    "LLM compaction returned stop for session {}, falling back to simple compaction",
                    session_id
                );
                if let Some(summary) = Self::trigger_compaction(session, filtered_messages, None) {
                    tracing::info!("Fallback compaction (from stop) complete: {}", summary);
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::success("Context compacted. Continuing the session."),
                    )
                    .await;
                } else {
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::warning(
                            "Auto-compaction ran, but the context could not be reduced.",
                        ),
                    )
                    .await;
                }
            }
            Err(e) => {
                tracing::warn!(
                    "LLM compaction failed for session {}: {}, falling back to simple compaction",
                    session_id,
                    e
                );
                if let Some(summary) = Self::trigger_compaction(session, filtered_messages, None) {
                    tracing::info!("Fallback compaction complete: {}", summary);
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::success("Context compacted. Continuing the session."),
                    )
                    .await;
                } else {
                    Self::emit_context_compaction_status(
                        output_block_hook,
                        session_id,
                        StatusBlock::warning(
                            "Auto-compaction failed, and the context could not be reduced.",
                        ),
                    )
                    .await;
                }
            }
        }
    }

    async fn prepare_chat_messages(
        session_id: &str,
        agent_name: Option<&str>,
        system_prompt: Option<&str>,
        mut filtered_messages: Vec<SessionMessage>,
        provider_type: ProviderType,
    ) -> anyhow::Result<Vec<rocode_provider::Message>> {
        if rocode_plugin::should_trigger_script_hooks(HookEvent::ChatMessagesTransform, agent_name)
            .await
        {
            let hook_messages = serde_json::Value::Array(
                filtered_messages
                    .iter()
                    .map(session_message_hook_payload)
                    .collect(),
            );
            let message_hook_outputs = rocode_plugin::trigger_collect(
                HookContext::new(HookEvent::ChatMessagesTransform)
                    .with_session(session_id)
                    .with_data("message_count", serde_json::json!(filtered_messages.len()))
                    .with_data("messages", hook_messages),
            )
            .await;
            apply_chat_messages_hook_outputs(&mut filtered_messages, message_hook_outputs);
        }

        let mut prompt_messages = filtered_messages;
        if let Some(agent) = agent_name {
            let was_plan = super::was_plan_agent(&prompt_messages);
            prompt_messages = super::insert_reminders(&prompt_messages, agent, was_plan);
        }

        let mut chat_messages = Self::build_chat_messages(&prompt_messages, system_prompt)?;
        apply_caching(&mut chat_messages, provider_type);
        Ok(chat_messages)
    }

    fn latest_cache_request_fingerprint(session: &Session) -> Option<CacheRequestFingerprint> {
        session.messages.iter().rev().find_map(|message| {
            message
                .metadata
                .get(CACHE_REQUEST_FINGERPRINT_METADATA_KEY)
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
        })
    }

    fn latest_prompt_surface_runtime_snapshot(
        session: &Session,
    ) -> Option<PromptSurfaceRuntimeSnapshot> {
        session
            .metadata
            .get(PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .or_else(|| {
                session.messages.iter().rev().find_map(|message| {
                    message
                        .metadata
                        .get(PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY)
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                })
            })
    }

    fn prompt_surface_stable_fields(
        session: &Session,
        provider_id: &str,
        model_id: &str,
        compiled_request: &CompiledExecutionRequest,
        fingerprint: &CacheRequestFingerprint,
        system_prompt: Option<&str>,
        tool_source_surface_hash: String,
    ) -> PromptSurfaceStableFields {
        let provider_options = compiled_request.provider_options.as_ref();
        let reasoning_mode_hash = provider_options
            .and_then(|options| {
                let mut relevant = serde_json::Map::new();
                for key in [
                    "reasoning",
                    "reasoning_effort",
                    "reasoningEffort",
                    "thinking",
                    "include_reasoning",
                    "includeReasoning",
                ] {
                    if let Some(value) = options.get(key) {
                        relevant.insert(key.to_string(), value.clone());
                    }
                }
                (!relevant.is_empty()).then_some(serde_json::Value::Object(relevant))
            })
            .map(|value| rocode_provider::cache::json_fingerprint(&value));
        let tool_policy_hash = Self::tool_policy_hash(provider_options);
        let api_shape =
            (fingerprint.family == CacheProtocolFamily::CloseAiCompatible).then(
                || match fingerprint
                    .provider_profile
                    .as_ref()
                    .map(|profile| profile.api_shape)
                {
                    Some(rocode_provider::ProviderApiShape::Responses) => {
                        rocode_provider::cache::CloseAiCompatibleApiShape::Responses
                    }
                    _ => rocode_provider::cache::CloseAiCompatibleApiShape::ChatCompletions,
                },
            );
        let closeai_prompt_cache_key = fingerprint
            .closeai
            .as_ref()
            .and_then(|closeai| closeai.prompt_cache_key.clone());
        let ethnopic_policy_hash = (fingerprint.family == CacheProtocolFamily::EthnopicCompatible)
            .then(|| {
                rocode_provider::cache::ethnopic_cache_policy_hash(&EthnopicCachePolicy::default())
            });
        let ethnopic_breakpoint_plan_hash = fingerprint.ethnopic.as_ref().map(|ethnopic| {
            rocode_provider::cache::json_fingerprint(&serde_json::json!({
                "breakpoint_placement": ethnopic.breakpoint_placement,
                "cache_control_hash": ethnopic.cache_control_hash,
            }))
        });
        let ingress_policy_hash = Self::ingress_policy_hash(session);

        PromptSurfaceStableFields {
            protocol_family: fingerprint.family,
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            api_shape,
            system_hash: fingerprint.surface.system_hash.clone(),
            stable_system_surface_hash: Self::stable_system_surface_hash(system_prompt),
            tool_surface_hash: fingerprint.surface.tools_hash.clone(),
            tool_source_surface_hash,
            provider_params_hash: fingerprint.surface.api_params_hash.clone(),
            tool_policy_hash,
            reasoning_mode_hash,
            output_projection_policy_hash: rocode_provider::cache::json_fingerprint(
                &serde_json::json!({
                    "policy": "default",
                    "owner": "prompt_surface_authority",
                }),
            ),
            scc_stable_refs_hash: Self::scc_stable_refs_hash(session),
            closeai_prompt_cache_key,
            ethnopic_policy_hash,
            ethnopic_breakpoint_plan_hash,
            ingress_policy_hash,
        }
    }

    fn ingress_policy_hash(session: &Session) -> Option<String> {
        let policy = session
            .metadata
            .get("last_ingress_policy")
            .and_then(|value| value.as_str())?;
        let source = session
            .metadata
            .get("last_ingress_source")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        Some(rocode_provider::cache::json_fingerprint(
            &serde_json::json!({
                "source": source,
                "policy": policy,
            }),
        ))
    }

    fn stable_system_surface_hash(system_prompt: Option<&str>) -> String {
        let projection = Self::stable_system_surface_projection(system_prompt.unwrap_or_default());
        rocode_provider::cache::text_fingerprint(&projection)
    }

    fn stable_system_surface_projection(system_prompt: &str) -> String {
        let volatile_sections = [
            "Exact Recent Tail",
            "Working Ledger",
            "Latest Compaction Summary",
        ];
        let mut lines = Vec::new();
        let mut skipping_section = false;

        for line in system_prompt.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                let title = header.trim();
                skipping_section = volatile_sections
                    .iter()
                    .any(|volatile| title.eq_ignore_ascii_case(volatile));
                if skipping_section {
                    continue;
                }
            }

            if skipping_section {
                continue;
            }

            let trimmed = line.trim_start();
            if trimmed.starts_with("Today's date:") || trimmed.starts_with("Today’s date:") {
                let indent_len = line.len().saturating_sub(trimmed.len());
                let indent = &line[..indent_len];
                lines.push(format!("{indent}Today's date: <dynamic>"));
                continue;
            }

            lines.push(line.to_string());
        }

        lines.join("\n")
    }

    fn scc_stable_refs_hash(session: &Session) -> Option<String> {
        let packet = session.metadata.get("scheduler_session_context_packet")?;
        let exact_recent_tail = packet
            .get("exact_recent_tail")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        serde_json::json!({
                            "message_id": item.get("message_id").cloned().unwrap_or(serde_json::Value::Null),
                            "role": item.get("role").cloned().unwrap_or(serde_json::Value::Null),
                            "projected": item.get("projected").cloned().unwrap_or(serde_json::Value::Null),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let memory_anchors = packet
            .get("memory_anchors")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        serde_json::json!({
                            "record_id": item.get("record_id").cloned().unwrap_or(serde_json::Value::Null),
                            "kind": item.get("kind").cloned().unwrap_or(serde_json::Value::Null),
                            "status": item.get("status").cloned().unwrap_or(serde_json::Value::Null),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Some(rocode_provider::cache::json_fingerprint(
            &serde_json::json!({
                "version": packet.get("version").cloned().unwrap_or(serde_json::Value::Null),
                "eligible_message_count": packet
                    .get("eligible_message_count")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                "exact_recent_tail": exact_recent_tail,
                "memory_anchors": memory_anchors,
                "latest_compaction_summary": packet
                    .get("latest_compaction_summary")
                    .and_then(|value| value.get("message_id"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            }),
        ))
    }

    fn tool_source_surface_hash(
        base_source_digests: &[rocode_provider::cache::ToolSurfaceSourceDigest],
        base_tools: &[ToolDefinition],
        extra_tools: &[ToolDefinition],
    ) -> String {
        let base_names = base_tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<HashSet<_>>();
        let effective_extra = extra_tools
            .iter()
            .filter(|tool| !base_names.contains(&tool.name))
            .cloned()
            .collect::<Vec<_>>();

        let mut groups = if base_source_digests.is_empty() {
            vec![rocode_provider::cache::ToolSurfaceSourceDigest {
                source: rocode_provider::cache::ToolSurfaceSourceKind::Base,
                tool_count: base_tools.len(),
                tools_hash: rocode_provider::cache::tool_surface_fingerprint(base_tools),
            }]
        } else {
            base_source_digests.to_vec()
        };
        groups.push(rocode_provider::cache::ToolSurfaceSourceDigest {
            source: rocode_provider::cache::ToolSurfaceSourceKind::Mcp,
            tool_count: effective_extra.len(),
            tools_hash: rocode_provider::cache::tool_surface_fingerprint(&effective_extra),
        });

        rocode_provider::cache::tool_source_surface_fingerprint(&groups)
    }

    fn tool_policy_hash(
        provider_options: Option<&HashMap<String, serde_json::Value>>,
    ) -> Option<String> {
        let provider_options = provider_options?;
        let mut relevant = serde_json::Map::new();
        for key in [
            "allowed_tools",
            "allowedTools",
            "tool_choice",
            "toolChoice",
            "allowed_tool_names",
            "allowedToolNames",
        ] {
            if let Some(value) = provider_options.get(key) {
                relevant.insert(key.to_string(), value.clone());
            }
        }
        (!relevant.is_empty())
            .then(|| rocode_provider::cache::json_fingerprint(&serde_json::Value::Object(relevant)))
    }

    fn build_prompt_surface_runtime_snapshot(
        session_id: &str,
        previous: Option<&PromptSurfaceRuntimeSnapshot>,
        stable: PromptSurfaceStableFields,
        now_ms: i64,
    ) -> PromptSurfaceRuntimeSnapshot {
        let invalidation =
            previous.and_then(|snapshot| Self::prompt_surface_invalidation(snapshot, &stable));
        let generation = match previous {
            Some(snapshot) if invalidation.is_none() => snapshot.generation,
            Some(snapshot) => snapshot.generation.saturating_add(1),
            None => 1,
        };
        let created_at_ms = previous
            .filter(|_| invalidation.is_none())
            .map(|snapshot| snapshot.created_at_ms)
            .unwrap_or(now_ms);

        PromptSurfaceRuntimeSnapshot {
            session_id: session_id.to_string(),
            generation,
            created_at_ms,
            updated_at_ms: now_ms,
            protocol_family: stable.protocol_family,
            provider_id: stable.provider_id,
            model_id: stable.model_id,
            api_shape: stable.api_shape,
            system_hash: stable.system_hash,
            stable_system_surface_hash: stable.stable_system_surface_hash,
            tool_surface_hash: stable.tool_surface_hash,
            tool_source_surface_hash: stable.tool_source_surface_hash,
            provider_params_hash: stable.provider_params_hash,
            tool_policy_hash: stable.tool_policy_hash,
            reasoning_mode_hash: stable.reasoning_mode_hash,
            output_projection_policy_hash: stable.output_projection_policy_hash,
            scc_stable_refs_hash: stable.scc_stable_refs_hash,
            closeai_prompt_cache_key: stable.closeai_prompt_cache_key,
            ethnopic_policy_hash: stable.ethnopic_policy_hash,
            ethnopic_breakpoint_plan_hash: stable.ethnopic_breakpoint_plan_hash,
            ingress_policy_hash: stable.ingress_policy_hash,
            invalidation,
        }
    }

    fn prompt_surface_invalidation(
        previous: &PromptSurfaceRuntimeSnapshot,
        current: &PromptSurfaceStableFields,
    ) -> Option<PromptSurfaceInvalidation> {
        let mut changed_fields = Vec::new();
        let mut severity = rocode_provider::cache::CacheBustSeverity::Stable;

        macro_rules! compare_field {
            ($field:literal, $prev:expr, $current:expr, $field_severity:expr) => {
                if $prev != $current {
                    changed_fields.push($field.to_string());
                    severity = severity.max($field_severity);
                }
            };
        }

        compare_field!(
            "protocolFamily",
            previous.protocol_family,
            current.protocol_family,
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "providerId",
            previous.provider_id.as_str(),
            current.provider_id.as_str(),
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "modelId",
            previous.model_id.as_str(),
            current.model_id.as_str(),
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "apiShape",
            previous.api_shape,
            current.api_shape,
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "stableSystemSurfaceHash",
            previous.stable_system_surface_hash.as_str(),
            current.stable_system_surface_hash.as_str(),
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "toolSurfaceHash",
            previous.tool_surface_hash.as_str(),
            current.tool_surface_hash.as_str(),
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "toolSourceSurfaceHash",
            previous.tool_source_surface_hash.as_str(),
            current.tool_source_surface_hash.as_str(),
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "providerParamsHash",
            previous.provider_params_hash.as_str(),
            current.provider_params_hash.as_str(),
            rocode_provider::cache::CacheBustSeverity::HardBust
        );
        compare_field!(
            "toolPolicyHash",
            previous.tool_policy_hash.as_deref(),
            current.tool_policy_hash.as_deref(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "reasoningModeHash",
            previous.reasoning_mode_hash.as_deref(),
            current.reasoning_mode_hash.as_deref(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "outputProjectionPolicyHash",
            previous.output_projection_policy_hash.as_str(),
            current.output_projection_policy_hash.as_str(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "sccStableRefsHash",
            previous.scc_stable_refs_hash.as_deref(),
            current.scc_stable_refs_hash.as_deref(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "closeaiPromptCacheKey",
            previous.closeai_prompt_cache_key.as_deref(),
            current.closeai_prompt_cache_key.as_deref(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "ethnopicPolicyHash",
            previous.ethnopic_policy_hash.as_deref(),
            current.ethnopic_policy_hash.as_deref(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "ethnopicBreakpointPlanHash",
            previous.ethnopic_breakpoint_plan_hash.as_deref(),
            current.ethnopic_breakpoint_plan_hash.as_deref(),
            rocode_provider::cache::CacheBustSeverity::LikelyBust
        );
        compare_field!(
            "ingressPolicyHash",
            previous.ingress_policy_hash.as_deref(),
            current.ingress_policy_hash.as_deref(),
            rocode_provider::cache::CacheBustSeverity::SoftDegradation
        );

        if changed_fields.is_empty() {
            return None;
        }

        let reason = format!(
            "prompt surface runtime changed: {}",
            changed_fields.join(", ")
        );
        Some(PromptSurfaceInvalidation {
            severity,
            reason,
            changed_fields,
        })
    }

    fn merge_snapshot_invalidation_into_summary(
        mut summary: CacheBustSummary,
        invalidation: Option<&PromptSurfaceInvalidation>,
    ) -> CacheBustSummary {
        let Some(invalidation) = invalidation else {
            return summary;
        };
        if invalidation.severity >= summary.severity {
            summary.status = "degraded".to_string();
            summary.severity = invalidation.severity;
            summary.primary_cause = Some(invalidation.reason.clone());
        }
        summary
    }

    fn cache_request_fingerprint(
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        system_prompt: Option<&str>,
        messages: &[rocode_provider::Message],
        tools: &[ToolDefinition],
        compiled_request: &CompiledExecutionRequest,
        provider_type: ProviderType,
        provider_profile: Option<ProviderProfileFingerprint>,
    ) -> CacheRequestFingerprint {
        let family = Self::cache_protocol_family(provider_type);
        let api_params = serde_json::json!({
            "provider_id": provider_id,
            "max_tokens": compiled_request.max_tokens,
            "temperature": compiled_request.temperature,
            "top_p": compiled_request.top_p,
            "variant": compiled_request.variant,
            "provider_options": compiled_request.provider_options,
        });
        let surface =
            PromptSurfaceFingerprint::new(model_id, system_prompt, tools, messages, &api_params);
        let closeai = (family == CacheProtocolFamily::CloseAiCompatible).then(|| {
            let provider_options = compiled_request.provider_options.as_ref();
            let prompt_cache_key = Self::closeai_prompt_cache_key_for_fingerprint(
                session_id,
                provider_id,
                provider_options,
            );
            CloseAiCacheFingerprint {
                prompt_cache_key,
                prompt_cache_retention: provider_options.and_then(|options| {
                    options
                        .get("prompt_cache_retention")
                        .or_else(|| options.get("promptCacheRetention"))
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned)
                }),
                previous_response_id_used: false,
                incremental_input_used: false,
                cached_tokens_observed: 0,
            }
        });
        let ethnopic = (family == CacheProtocolFamily::EthnopicCompatible).then(|| {
            let breakpoint_plan =
                rocode_provider::cache::plan_ethnopic_message_breakpoints(messages);
            let cache_policy = EthnopicCachePolicy::default();
            EthnopicCacheFingerprint {
                cache_control_hash: rocode_provider::cache::json_fingerprint(&serde_json::json!({
                    "policy": cache_policy,
                    "breakpoints": breakpoint_plan,
                })),
                breakpoint_placement: breakpoint_plan.message_indices().collect(),
                ttl: None,
                scope: None,
                cache_read_observed: 0,
                cache_write_observed: 0,
            }
        });

        CacheRequestFingerprint {
            family,
            surface,
            provider_profile,
            closeai,
            ethnopic,
        }
    }

    fn closeai_prompt_cache_key_for_fingerprint(
        session_id: &str,
        provider_id: &str,
        provider_options: Option<&HashMap<String, serde_json::Value>>,
    ) -> Option<String> {
        let options = provider_options.cloned().unwrap_or_default();
        if let Some(existing) = options
            .get("promptCacheKey")
            .or_else(|| options.get("prompt_cache_key"))
            .and_then(|value| value.as_str())
        {
            return Some(existing.to_string());
        }

        let provider_options_object = options
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        rocode_provider::cache::closeai_prompt_cache_key_field(
            provider_id,
            "",
            &provider_options_object,
        )
        .map(|_| {
            rocode_provider::cache::build_prompt_cache_key(
                rocode_provider::cache::PromptCacheKeyContext {
                    session_id,
                    stage: options
                        .get("cacheStage")
                        .and_then(|value| value.as_str())
                        .unwrap_or("chat"),
                    preset_hash: options
                        .get("cachePresetHash")
                        .and_then(|value| value.as_str()),
                    repo_hash: options
                        .get("cacheRepoHash")
                        .and_then(|value| value.as_str()),
                },
            )
        })
    }

    fn cache_protocol_family(provider_type: ProviderType) -> CacheProtocolFamily {
        match provider_type {
            ProviderType::Ethnopic | ProviderType::Bedrock | ProviderType::Gateway => {
                CacheProtocolFamily::EthnopicCompatible
            }
            ProviderType::OpenAI | ProviderType::OpenRouter => {
                CacheProtocolFamily::CloseAiCompatible
            }
            ProviderType::Other => CacheProtocolFamily::Disabled,
        }
    }

    fn finalize_assistant_message(
        session: &mut Session,
        assistant_index: usize,
        step_output: &SessionStepRuntimeOutput,
    ) {
        if let Some(assistant_msg) = session.messages_mut().get_mut(assistant_index) {
            if let Some(reason) = step_output.finish_reason.clone() {
                assistant_msg
                    .metadata
                    .insert("finish_reason".to_string(), serde_json::json!(reason));
            }
            assistant_msg.metadata.insert(
                "completed_at".to_string(),
                serde_json::json!(chrono::Utc::now().timestamp_millis()),
            );
            assistant_msg.metadata.insert(
                "usage".to_string(),
                serde_json::json!({
                    "prompt_tokens": step_output.prompt_tokens,
                    "completion_tokens": step_output.completion_tokens,
                    "reasoning_tokens": step_output.reasoning_tokens,
                    "cache_read_tokens": step_output.cache_read_tokens,
                    "cache_miss_tokens": step_output.cache_miss_tokens,
                    "cache_write_tokens": step_output.cache_write_tokens,
                }),
            );
            assistant_msg.metadata.insert(
                "tokens_input".to_string(),
                serde_json::json!(step_output.prompt_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_output".to_string(),
                serde_json::json!(step_output.completion_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_reasoning".to_string(),
                serde_json::json!(step_output.reasoning_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_cache_read".to_string(),
                serde_json::json!(step_output.cache_read_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_cache_miss".to_string(),
                serde_json::json!(step_output.cache_miss_tokens),
            );
            assistant_msg.metadata.insert(
                "tokens_cache_write".to_string(),
                serde_json::json!(step_output.cache_write_tokens),
            );
            assistant_msg.usage = Some(crate::message::MessageUsage {
                input_tokens: step_output.prompt_tokens,
                output_tokens: step_output.completion_tokens,
                reasoning_tokens: step_output.reasoning_tokens,
                cache_read_tokens: step_output.cache_read_tokens,
                cache_miss_tokens: step_output.cache_miss_tokens,
                cache_write_tokens: step_output.cache_write_tokens,
                context_tokens: step_output.prompt_tokens,
                ..Default::default()
            });
        }
    }

    async fn run_chat_message_hook(
        session: &mut Session,
        session_id: &str,
        assistant_index: usize,
        agent_name: Option<&str>,
        provider: &Arc<dyn Provider>,
        model_id: &str,
        has_tool_calls: bool,
    ) {
        if !rocode_plugin::should_trigger_script_hooks(HookEvent::ChatMessage, agent_name).await {
            return;
        }
        let Some(assistant_msg) = session.messages.get(assistant_index).cloned() else {
            return;
        };

        let mut hook_ctx = HookContext::new(HookEvent::ChatMessage)
            .with_session(session_id)
            .with_data("message_id", serde_json::json!(&assistant_msg.id))
            .with_data("message", session_message_hook_payload(&assistant_msg))
            .with_data("parts", serde_json::json!(&assistant_msg.parts))
            .with_data("has_tool_calls", serde_json::json!(has_tool_calls));

        if let Some(model) = provider.get_model(model_id) {
            hook_ctx = hook_ctx.with_data(
                "model",
                serde_json::json!({
                    "id": model.id,
                    "name": model.name,
                    "provider": model.provider,
                }),
            );
        } else {
            hook_ctx = hook_ctx.with_data("model_id", serde_json::json!(model_id));
        }
        hook_ctx = hook_ctx.with_data("sessionID", serde_json::json!(session_id));
        if let Some(agent) = agent_name {
            hook_ctx = hook_ctx.with_data("agent", serde_json::json!(agent));
        }

        let hook_outputs = rocode_plugin::trigger_collect(hook_ctx).await;
        if let Some(current_assistant) = session.messages_mut().get_mut(assistant_index) {
            apply_chat_message_hook_outputs(current_assistant, hook_outputs);
        }
    }

    async fn loop_inner(
        &self,
        session_id: String,
        token: CancellationToken,
        session: &mut Session,
        prompt_ctx: PromptLoopContext,
    ) -> anyhow::Result<()> {
        let mut step = 0u32;
        let provider_type = ProviderType::from_provider_id(&prompt_ctx.provider_id);
        let mut post_first_step_ran = false;
        let turn_start_index = session.messages.len().saturating_sub(1);

        loop {
            if token.is_cancelled() {
                tracing::info!("Prompt loop cancelled for session {}", session_id);
                break;
            }

            let filtered_messages = Self::filter_compacted_messages(&session.messages);

            let last_user_idx = filtered_messages
                .iter()
                .rposition(|m| matches!(m.role, MessageRole::User));

            let last_assistant_idx = filtered_messages
                .iter()
                .rposition(|m| matches!(m.role, MessageRole::Assistant));

            let last_user_idx = match last_user_idx {
                Some(idx) => idx,
                None => return Err(anyhow::anyhow!("No user message found")),
            };

            if self
                .process_pending_subtasks(
                    session,
                    prompt_ctx.provider.clone(),
                    &prompt_ctx.provider_id,
                    &prompt_ctx.model_id,
                    &prompt_ctx.hooks,
                )
                .await?
            {
                tracing::info!("Processed pending subtask parts for session {}", session_id);
                continue;
            }

            if let Some(assistant_idx) = last_assistant_idx {
                let assistant = &filtered_messages[assistant_idx];
                if is_terminal_finish(assistant.finish.as_deref()) && last_user_idx < assistant_idx
                {
                    tracing::info!(
                        finish = ?assistant.finish,
                        "Prompt loop complete for session {}", session_id
                    );
                    break;
                }
            }

            step += 1;
            if step > MAX_STEPS {
                tracing::warn!("Max steps reached for session {}", session_id);
                break;
            }

            Self::maybe_compact_context(
                &session_id,
                &prompt_ctx.provider_id,
                &prompt_ctx.model_id,
                session,
                &prompt_ctx.provider,
                &filtered_messages,
                &prompt_ctx.compiled_request,
                prompt_ctx.config_store.as_deref(),
                prompt_ctx.hooks.output_block_hook.as_ref(),
                prompt_ctx.memory_authority.clone(),
            )
            .await;

            tracing::info!(
                step = step,
                session_id = %session_id,
                message_count = filtered_messages.len(),
                "prompt loop step start"
            );

            let chat_messages = Self::prepare_chat_messages(
                &session_id,
                prompt_ctx.agent_name.as_deref(),
                prompt_ctx.system_prompt.as_deref(),
                filtered_messages,
                provider_type,
            )
            .await?;
            let base_tools = prompt_ctx.tools.clone();
            let extra_tools = Self::mcp_tools_from_session(session);
            let tool_source_surface_hash = Self::tool_source_surface_hash(
                prompt_ctx.tool_source_digests.as_slice(),
                &base_tools,
                &extra_tools,
            );
            let resolved_tools = merge_tool_definitions(base_tools, extra_tools);
            let provider_profile = prompt_ctx.provider.provider_profile_fingerprint();
            let cache_fingerprint = Self::cache_request_fingerprint(
                &session_id,
                &prompt_ctx.provider_id,
                &prompt_ctx.model_id,
                prompt_ctx.system_prompt.as_deref(),
                &chat_messages,
                &resolved_tools,
                &prompt_ctx.compiled_request,
                provider_type,
                provider_profile,
            );
            let previous_cache_fingerprint = Self::latest_cache_request_fingerprint(session);
            let cache_bust_inspection = inspect_cache_fingerprint_change(
                previous_cache_fingerprint.as_ref(),
                &cache_fingerprint,
            );
            let previous_prompt_surface_snapshot =
                Self::latest_prompt_surface_runtime_snapshot(session);
            let prompt_surface_stable_fields = Self::prompt_surface_stable_fields(
                session,
                &prompt_ctx.provider_id,
                &prompt_ctx.model_id,
                &prompt_ctx.compiled_request,
                &cache_fingerprint,
                prompt_ctx.system_prompt.as_deref(),
                tool_source_surface_hash,
            );
            let prompt_surface_snapshot = Self::build_prompt_surface_runtime_snapshot(
                &session_id,
                previous_prompt_surface_snapshot.as_ref(),
                prompt_surface_stable_fields,
                chrono::Utc::now().timestamp_millis(),
            );
            if let Ok(value) = serde_json::to_value(&prompt_surface_snapshot) {
                session.insert_metadata(PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY, value);
            }

            let tool_registry = Arc::new(rocode_tool::create_default_registry().await);

            let assistant_index = session.messages.len();
            let assistant_message_id =
                rocode_core::id::create(rocode_core::id::Prefix::Message, true, None);
            let mut assistant_metadata = HashMap::new();
            assistant_metadata.insert(
                "model_provider".to_string(),
                serde_json::json!(&prompt_ctx.provider_id),
            );
            assistant_metadata.insert(
                "model_id".to_string(),
                serde_json::json!(&prompt_ctx.model_id),
            );
            if let Some(agent) = prompt_ctx.agent_name.as_deref() {
                assistant_metadata.insert("agent".to_string(), serde_json::json!(agent));
                assistant_metadata.insert("mode".to_string(), serde_json::json!(agent));
            }
            if let Ok(value) = serde_json::to_value(&cache_fingerprint) {
                assistant_metadata
                    .insert(CACHE_REQUEST_FINGERPRINT_METADATA_KEY.to_string(), value);
            }
            if let Ok(value) = serde_json::to_value(&cache_bust_inspection) {
                assistant_metadata.insert(CACHE_BUST_INSPECTION_METADATA_KEY.to_string(), value);
            }
            if let Ok(value) = serde_json::to_value(&prompt_surface_snapshot) {
                assistant_metadata.insert(
                    PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY.to_string(),
                    value,
                );
            }
            if let Some(invalidation) = prompt_surface_snapshot.invalidation.as_ref() {
                if let Ok(value) = serde_json::to_value(invalidation) {
                    assistant_metadata.insert(
                        PROMPT_SURFACE_SNAPSHOT_INVALIDATION_METADATA_KEY.to_string(),
                        value,
                    );
                }
            }
            let cache_bust_summary = Self::merge_snapshot_invalidation_into_summary(
                CacheBustSummary::from(&cache_bust_inspection),
                prompt_surface_snapshot.invalidation.as_ref(),
            );
            if let Ok(value) = serde_json::to_value(cache_bust_summary) {
                assistant_metadata.insert(CACHE_BUST_SUMMARY_METADATA_KEY.to_string(), value);
            }
            session.messages_mut().push(SessionMessage {
                id: assistant_message_id,
                session_id: session_id.clone(),
                role: MessageRole::Assistant,
                parts: Vec::new(),
                created_at: chrono::Utc::now(),
                metadata: assistant_metadata,
                usage: None,
                finish: None,
            });
            session.touch();
            Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);

            let step_output = self
                .run_runtime_step(
                    token.clone(),
                    session,
                    resolved_tools,
                    RuntimeStepInput {
                        session_id: session_id.clone(),
                        assistant_index,
                        chat_messages,
                        tool_registry: tool_registry.clone(),
                        step_ctx: RuntimeStepContext {
                            provider: prompt_ctx.provider.clone(),
                            model_id: prompt_ctx.model_id.clone(),
                            provider_id: prompt_ctx.provider_id.clone(),
                            agent_name: prompt_ctx.agent_name.clone(),
                            compiled_request: prompt_ctx.compiled_request.clone(),
                            hooks: prompt_ctx.hooks.clone(),
                            config_store: prompt_ctx.config_store.clone(),
                        },
                    },
                )
                .await?;

            let finish_reason = step_output.finish_reason.clone();
            let executed_local_tools_this_step = step_output.executed_local_tools_this_step;

            Self::finalize_assistant_message(session, assistant_index, &step_output);

            if !step_output.stream_tool_results.is_empty() {
                let mut tool_msg = SessionMessage::tool(session_id.clone());
                for (tool_call_id, content, is_error, title, metadata, attachments) in
                    step_output.stream_tool_results
                {
                    Self::push_tool_result_part(
                        &mut tool_msg,
                        tool_call_id,
                        content,
                        is_error,
                        title,
                        metadata,
                        attachments,
                    );
                }
                session.messages_mut().push(tool_msg);
            }

            let has_tool_calls = session
                .messages
                .get(assistant_index)
                .map(Self::has_unresolved_tool_calls)
                .unwrap_or(false);

            session.touch();
            Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);

            Self::run_chat_message_hook(
                session,
                &session_id,
                assistant_index,
                prompt_ctx.agent_name.as_deref(),
                &prompt_ctx.provider,
                &prompt_ctx.model_id,
                has_tool_calls,
            )
            .await;

            if executed_local_tools_this_step {
                continue;
            }

            if !post_first_step_ran {
                Self::ensure_title(session, prompt_ctx.provider.clone(), &prompt_ctx.model_id)
                    .await;
                let _ = Self::summarize_session(
                    session,
                    &session_id,
                    &prompt_ctx.provider_id,
                    &prompt_ctx.model_id,
                    prompt_ctx.provider.as_ref(),
                )
                .await;
                post_first_step_ran = true;
            }

            if is_terminal_finish(finish_reason.as_deref()) {
                Self::maybe_append_runtime_skill_save_suggestion(session, turn_start_index);
                skill_reflection::update_skill_reflection_metadata(
                    self.config_store.clone(),
                    session,
                );
                Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);
                tracing::info!(
                    "Prompt loop complete for session {} with finish: {:?}",
                    session_id,
                    finish_reason
                );
                break;
            }
        }

        if token.is_cancelled() {
            Self::abort_pending_tool_calls(session);
        }

        let compaction_config = Self::runtime_compaction_config(prompt_ctx.config_store.as_deref());
        Self::prune_after_loop(session, &compaction_config);
        session.touch();
        Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);

        Ok(())
    }

    pub(super) fn emit_session_update(
        update_hook: Option<&super::SessionUpdateHook>,
        session: &Session,
    ) {
        if let Some(hook) = update_hook {
            hook(session);
        }
    }

    pub(super) fn maybe_emit_session_update(
        update_hook: Option<&super::SessionUpdateHook>,
        session: &Session,
        last_emit: &mut Instant,
        force: bool,
    ) {
        let elapsed = last_emit.elapsed();
        if force || elapsed >= Duration::from_millis(STREAM_UPDATE_INTERVAL_MS) {
            Self::emit_session_update(update_hook, session);
            *last_emit = Instant::now();
        }
    }
}
