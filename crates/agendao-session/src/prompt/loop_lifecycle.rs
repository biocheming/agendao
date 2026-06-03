use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use agendao_content::output_blocks::{OutputBlock, StatusBlock};
use agendao_execution_types::{session_runtime_request_defaults, CompiledExecutionRequest};
use agendao_orchestrator::runtime::events::{
    CancelToken as RuntimeCancelToken, LoopError as RuntimeLoopError,
};
use agendao_orchestrator::runtime::policy::{LoopPolicy, ToolDedupScope};
use agendao_orchestrator::runtime::run_loop;
use agendao_orchestrator::runtime::{SimpleModelCaller, SimpleModelCallerConfig};
use agendao_plugin::{HookContext, HookEvent};
use agendao_provider::cache::{
    inspect_cache_fingerprint_change, CacheEvidenceSummary, CacheProtocolFamily,
    CacheRequestFingerprint, CloseAiCacheFingerprint, EthnopicCacheFingerprint,
    EthnopicCachePolicy, PromptSurfaceFingerprint, ProviderProfileFingerprint,
    CACHE_EVIDENCE_INSPECTION_METADATA_KEY, CACHE_EVIDENCE_METADATA_KEY,
    CACHE_REQUEST_FINGERPRINT_METADATA_KEY,
};
use agendao_provider::error_code::StandardErrorCode;
use agendao_provider::transform::{apply_caching, ProviderType};
use agendao_provider::{Provider, ToolDefinition};
use agendao_types::SessionContinuityPacket;

use crate::tool_result_governance::{
    default_tool_result_artifacts_root, govern_tool_result_batch, tool_result_budget,
};
use crate::{MessageRole, Session, SessionMessage};

use super::runtime_step::{SessionStepRuntimeOutput, SessionStepSink, SessionStepToolDispatcher};
use super::{
    apply_chat_message_hook_outputs, apply_chat_messages_hook_outputs, is_terminal_finish,
    merge_tool_definitions, session_message_hook_payload, skill_reflection,
    surface_contract::{
        collect_prompt_surface_provider_options, is_volatile_system_section,
        normalize_stable_system_line, sanctioned_model_context_projection_for_message,
        PromptSurfaceProviderOptionGroup,
    },
    tools_and_output, PromptHooks, PromptInput, PromptRequestContext, SessionPrompt,
    SessionStepShared, MAX_STEPS, PENDING_SANITIZER_STAGE_METADATA_KEY,
    PROMPT_SURFACE_EVIDENCE_METADATA_KEY, PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY,
    STREAM_UPDATE_INTERVAL_MS,
};

const MAX_LENGTH_CONTINUATION_RETRIES: u8 = 2;
const LENGTH_CONTINUATION_PROMPT: &str = "[System: Your previous response was truncated by the output length limit. Continue exactly where you left off. Do not restart or repeat prior text. Finish the answer directly.]";

#[derive(Clone)]
struct SessionStepCancelToken {
    user_cancel: CancellationToken,
    step_complete: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PromptSurfaceStateSnapshot {
    session_id: String,
    generation: u64,
    created_at_ms: i64,
    updated_at_ms: i64,
    protocol_family: CacheProtocolFamily,
    provider_id: String,
    model_id: String,
    api_shape: Option<agendao_provider::cache::CloseAiCompatibleApiShape>,
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
    evidence: Option<PromptSurfaceEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PromptSurfaceEvidence {
    severity: agendao_provider::cache::CacheEvidenceSeverity,
    reason: String,
    changed_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptSurfaceStableFields {
    protocol_family: CacheProtocolFamily,
    provider_id: String,
    model_id: String,
    api_shape: Option<agendao_provider::cache::CloseAiCompatibleApiShape>,
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

fn take_pending_sanitizer_stage(session: &mut Session) -> agendao_types::SanitizerStage {
    let Some(value) = session.remove_metadata(PENDING_SANITIZER_STAGE_METADATA_KEY) else {
        return agendao_types::SanitizerStage::PreRequest;
    };

    value
        .as_str()
        .and_then(|raw| {
            serde_json::from_str::<agendao_types::SanitizerStage>(&format!("\"{raw}\"")).ok()
        })
        .unwrap_or(agendao_types::SanitizerStage::PreRequest)
}

#[cfg(test)]
mod sanitizer_stage_tests {
    use super::*;

    #[test]
    // P2.3 route guard: all bad shapes on resume path must go through shared sanitizer.
    fn resume_path_uses_shared_sanitizer_contract() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            PENDING_SANITIZER_STAGE_METADATA_KEY.to_string(),
            serde_json::json!(agendao_types::SanitizerStage::SessionResume.label()),
        );

        let stage = take_pending_sanitizer_stage(&mut session);
        assert_eq!(stage, agendao_types::SanitizerStage::SessionResume);
        // Stage is consumed on read; second call defaults to PreRequest.
        let stage2 = take_pending_sanitizer_stage(&mut session);
        assert_eq!(stage2, agendao_types::SanitizerStage::PreRequest);
    }

    #[test]
    // P2.3 route guard: post-compaction path uses the same shared sanitizer entry.
    fn post_compaction_continue_uses_shared_sanitizer_contract() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            PENDING_SANITIZER_STAGE_METADATA_KEY.to_string(),
            serde_json::json!(agendao_types::SanitizerStage::PostCompaction.label()),
        );

        let stage = take_pending_sanitizer_stage(&mut session);
        assert_eq!(stage, agendao_types::SanitizerStage::PostCompaction);
    }

    #[test]
    fn fallback_retry_uses_shared_sanitizer_contract() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            PENDING_SANITIZER_STAGE_METADATA_KEY.to_string(),
            serde_json::json!(agendao_types::SanitizerStage::FallbackRetry.label()),
        );

        let stage = take_pending_sanitizer_stage(&mut session);
        assert_eq!(stage, agendao_types::SanitizerStage::FallbackRetry);
    }

    #[test]
    fn default_sanitizer_stage_is_pre_request() {
        let mut session = Session::new("proj", ".");
        let stage = take_pending_sanitizer_stage(&mut session);
        assert_eq!(stage, agendao_types::SanitizerStage::PreRequest);
    }
}

#[cfg(test)]
mod steering_transcript_tests {
    use super::*;

    fn append_steering_messages_to_transcript_and_request(
        session: &mut Session,
        chat_messages: &mut Vec<agendao_provider::Message>,
        steering_msgs: Vec<crate::prompt::SteeringMessage>,
        injected_at_ms: i64,
    ) {
        let owner_id = session.id.clone();
        for (i, sm) in steering_msgs.into_iter().enumerate() {
            let mut steering_msg = crate::SessionMessage::user(session.id.clone(), sm.text.clone());
            steering_msg.metadata.insert(
                "steering_mode".to_string(),
                serde_json::json!("next_tool_boundary"),
            );
            steering_msg
                .metadata
                .insert("steering_index".to_string(), serde_json::json!(i));
            steering_msg.metadata.insert(
                "steering_injected_at".to_string(),
                serde_json::json!(injected_at_ms),
            );
            steering_msg.metadata.insert(
                "steering_owner_session_id".to_string(),
                serde_json::json!(owner_id),
            );
            steering_msg.metadata.insert(
                "steering_injected_during_active_run".to_string(),
                serde_json::json!(true),
            );
            if let Some(ref source) = sm.source_session_id {
                steering_msg.metadata.insert(
                    "steering_source_session_id".to_string(),
                    serde_json::json!(source),
                );
            }
            session.push_message(steering_msg);
            chat_messages.push(agendao_provider::Message::user(sm.text));
        }
    }

    #[test]
    fn steering_injection_writes_same_messages_to_transcript_and_request() {
        let mut session = Session::new("proj", ".");
        let mut chat_messages = Vec::new();

        append_steering_messages_to_transcript_and_request(
            &mut session,
            &mut chat_messages,
            vec![
                crate::prompt::SteeringMessage {
                    text: "stop and explain".to_string(),
                    created_at: 1,
                    source_session_id: None,
                },
                crate::prompt::SteeringMessage {
                    text: "do not write code".to_string(),
                    created_at: 2,
                    source_session_id: Some("attached_ses".to_string()),
                },
            ],
            1,
        );

        assert_eq!(chat_messages.len(), 2);
        let last = session.messages.last().expect("message should be appended");
        assert_eq!(
            last.metadata.get("steering_mode").and_then(|v| v.as_str()),
            Some("next_tool_boundary")
        );
        assert_eq!(last.get_text(), "do not write code");
        assert_eq!(session.messages.len(), 2);
    }
}

#[cfg(test)]
mod cache_fingerprint_tests {
    use super::*;
    use agendao_provider::Message;

    #[test]
    fn cache_request_fingerprint_records_closeai_family_without_wire_changes() {
        let messages = vec![Message::system("system"), Message::user("hello")];
        let compiled = CompiledExecutionRequest {
            model_id: "gpt-test".to_string(),
            provider_options: Some(HashMap::from([(
                "promptCacheKey".to_string(),
                serde_json::json!("agendao:key"),
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
            agendao_provider::ProviderApiFamily::CloseAiCompatible
        );
        assert_eq!(
            provider_profile.api_shape,
            agendao_provider::ProviderApiShape::ChatCompletions
        );
        assert_eq!(
            fingerprint
                .closeai
                .as_ref()
                .and_then(|value| value.prompt_cache_key.as_deref()),
            Some("agendao:key")
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
        assert!(prompt_cache_key.starts_with("agendao:"));
        assert!(!prompt_cache_key.contains("ses_generated_key"));
    }

    fn openai_provider_profile_fingerprint() -> ProviderProfileFingerprint {
        provider_profile_fingerprint("openai", HashMap::new())
    }

    fn provider_profile_fingerprint(
        provider_id: &str,
        options: HashMap<String, serde_json::Value>,
    ) -> ProviderProfileFingerprint {
        let profile = agendao_provider::ProviderProfileResolver::try_resolve_with_options(
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
    fn latest_prompt_surface_state_snapshot_reads_previous_assistant_metadata() {
        let mut session = Session::new("project", "/tmp");
        let snapshot = PromptSurfaceStateSnapshot {
            session_id: session.id.clone(),
            generation: 4,
            created_at_ms: 100,
            updated_at_ms: 200,
            protocol_family: CacheProtocolFamily::CloseAiCompatible,
            provider_id: "openai".to_string(),
            model_id: "gpt-test".to_string(),
            api_shape: Some(agendao_provider::cache::CloseAiCompatibleApiShape::ChatCompletions),
            system_hash: "system-a".to_string(),
            stable_system_surface_hash: "stable-system-a".to_string(),
            tool_surface_hash: "tools-a".to_string(),
            tool_source_surface_hash: "tool-source-a".to_string(),
            provider_params_hash: "params-a".to_string(),
            tool_policy_hash: None,
            reasoning_mode_hash: None,
            output_projection_policy_hash: "projection-a".to_string(),
            scc_stable_refs_hash: None,
            closeai_prompt_cache_key: Some("agendao:key".to_string()),
            ethnopic_policy_hash: None,
            ethnopic_breakpoint_plan_hash: None,
            ingress_policy_hash: None,
            evidence: None,
        };
        let assistant = session.add_assistant_message();
        assistant.metadata.insert(
            PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY.to_string(),
            serde_json::to_value(&snapshot).expect("snapshot serializes"),
        );

        let loaded = SessionPrompt::latest_prompt_surface_state_snapshot(&session)
            .expect("snapshot should load");

        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn prompt_surface_state_snapshot_keeps_generation_when_stable_fields_match() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let first_fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &first_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, first_stable, 100);

        let second_fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-changed", "params-a");
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &second_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        assert_eq!(second.generation, first.generation);
        assert!(second.evidence.is_none());
        assert_eq!(
            second.created_at_ms, first.created_at_ms,
            "message-prefix changes are request fingerprint diagnostics, not stable snapshot evidence changes"
        );
    }

    #[test]
    fn prompt_surface_state_snapshot_invalidates_on_tool_surface_change() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let first_fingerprint =
            test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &first_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, first_stable, 100);

        let second_fingerprint =
            test_cache_fingerprint("system-a", "tools-b", "messages-a", "params-a");
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &second_fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        let evidence = second
            .evidence
            .as_ref()
            .expect("tool surface changes should invalidate stable snapshot");
        assert_eq!(second.generation, first.generation + 1);
        assert_eq!(
            evidence.severity,
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        assert!(evidence
            .changed_fields
            .contains(&"toolSurfaceHash".to_string()));
    }

    #[test]
    fn prompt_surface_state_snapshot_reason_can_drive_cache_evidence() {
        let summary = CacheEvidenceSummary {
            status: "stable".to_string(),
            severity: agendao_provider::cache::CacheEvidenceSeverity::Stable,
            primary_cause: None,
            change_count: 0,
        };
        let evidence = PromptSurfaceEvidence {
            severity: agendao_provider::cache::CacheEvidenceSeverity::HighChange,
            reason: "surface changed: toolSurfaceHash".to_string(),
            changed_fields: vec!["toolSurfaceHash".to_string()],
        };

        let merged = SessionPrompt::merge_snapshot_evidence_into_summary(summary, Some(&evidence));

        assert_eq!(merged.status, "degraded");
        assert_eq!(
            merged.severity,
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        assert_eq!(
            merged.primary_cause.as_deref(),
            Some(evidence.reason.as_str())
        );
    }

    #[test]
    fn stable_system_surface_projection_ignores_dynamic_tail_and_date() {
        let first = "You are AgenDao.\n  Today's date: Thu Apr 30 2026\n  Current local time: 2026-04-30 08:00:00 +08:00\n  Local timezone: CST\n\n## Exact Recent Tail\n- user `m1`:\nold";
        let second =
            "You are AgenDao.\n  Today's date: Fri May 01 2026\n  Current local time: 2026-05-01 09:15:42 +08:00\n  Local timezone: CST\n\n## Exact Recent Tail\n- user `m2`:\nnew";

        assert_ne!(
            agendao_provider::cache::text_fingerprint(first),
            agendao_provider::cache::text_fingerprint(second)
        );
        assert_eq!(
            SessionPrompt::stable_system_surface_hash(Some(first)),
            SessionPrompt::stable_system_surface_hash(Some(second))
        );
    }

    #[test]
    fn prompt_surface_state_snapshot_ignores_dynamic_system_tail_for_generation() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let first_system = "You are AgenDao.\n\n## Exact Recent Tail\n- user `m1`:\nprevious output";
        let second_system = "You are AgenDao.\n\n## Exact Recent Tail\n- user `m2`:\nlatest output";
        let first_fingerprint =
            test_cache_fingerprint("system-full-a", "tools-a", "messages-a", "params-a");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &first_fingerprint,
            Some(first_system),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, first_stable, 100);

        let second_fingerprint =
            test_cache_fingerprint("system-full-b", "tools-a", "messages-a", "params-a");
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &second_fingerprint,
            Some(second_system),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        assert_eq!(second.generation, first.generation);
        assert!(second.evidence.is_none());
        assert_ne!(second.system_hash, first.system_hash);
        assert_eq!(
            second.stable_system_surface_hash,
            first.stable_system_surface_hash
        );
    }

    #[test]
    fn prompt_surface_state_snapshot_records_ingress_policy_without_user_text() {
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
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, stable, 100);

        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("web"));
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(crate::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY),
        );
        let stable_again = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            stable_again,
            200,
        );

        assert!(first.ingress_policy_hash.is_some());
        assert_eq!(second.generation, first.generation);
        assert!(second.evidence.is_none());
    }

    #[test]
    fn prompt_surface_state_snapshot_soft_degrades_on_ingress_policy_change() {
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
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, first_stable, 100);

        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("api"));
        session.insert_metadata(
            "last_ingress_policy".to_string(),
            serde_json::json!(crate::prompt::INGRESS_POLICY_SCHEDULER_METADATA_ONLY),
        );
        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        let evidence = second
            .evidence
            .as_ref()
            .expect("ingress policy changes should be tracked");
        assert_eq!(second.generation, first.generation + 1);
        assert_eq!(
            evidence.severity,
            agendao_provider::cache::CacheEvidenceSeverity::LowChange
        );
        assert!(evidence
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
            &session.messages,
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );

        assert_eq!(
            stable.api_shape,
            Some(agendao_provider::cache::CloseAiCompatibleApiShape::Responses)
        );
    }

    #[test]
    fn prompt_surface_state_snapshot_invalidates_on_output_projection_policy_change() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let fingerprint = test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");

        let mut projected = SessionMessage::assistant(session.id.clone());
        projected.add_text("large assistant delivery");
        projected.metadata.insert(
            agendao_orchestrator::output_projection::SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY
                .to_string(),
            serde_json::to_value(
                agendao_orchestrator::output_projection::ContextProjectionPolicy::OnDemandArtifact,
            )
            .expect("policy should serialize"),
        );
        projected.metadata.insert(
            agendao_orchestrator::output_projection::SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY
                .to_string(),
            serde_json::json!("artifact-backed summary"),
        );

        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &[projected],
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, first_stable, 100);

        let mut full = SessionMessage::assistant(session.id.clone());
        full.add_text("large assistant delivery");

        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &[full],
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        let evidence = second
            .evidence
            .as_ref()
            .expect("projection policy changes should invalidate stable snapshot");
        assert_eq!(second.generation, first.generation + 1);
        assert_eq!(
            evidence.severity,
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        assert!(evidence
            .changed_fields
            .contains(&"outputProjectionPolicyHash".to_string()));
    }

    #[test]
    fn prompt_surface_state_snapshot_ignores_provider_diagnostic_metadata() {
        let compiled = CompiledExecutionRequest::default();
        let session = Session::new("project", "/tmp");
        let fingerprint = test_cache_fingerprint("system-a", "tools-a", "messages-a", "params-a");

        let mut baseline = SessionMessage::assistant(session.id.clone());
        baseline.add_text("visible answer");
        let first_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &[baseline.clone()],
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let first =
            SessionPrompt::build_prompt_surface_state_snapshot("ses_test", None, first_stable, 100);

        let summary = agendao_provider::ProviderDiagnosticSummary {
            severity: agendao_provider::ProviderDiagnosticSeverity::HardFail,
            source: agendao_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-v4".to_string()),
            message: "thinking replay rejected".to_string(),
        };
        summary.attach_to_metadata(&mut baseline.metadata);

        let second_stable = SessionPrompt::prompt_surface_stable_fields(
            &session,
            &[baseline],
            "openai",
            "gpt-test",
            &compiled,
            &fingerprint,
            Some("system"),
            "tool-source-a".to_string(),
        );
        let second = SessionPrompt::build_prompt_surface_state_snapshot(
            "ses_test",
            Some(&first),
            second_stable,
            200,
        );

        assert_eq!(
            first.output_projection_policy_hash,
            second.output_projection_policy_hash
        );
        assert_eq!(second.generation, first.generation);
        assert!(second.evidence.is_none());
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
                prompt_cache_key: Some("agendao:key".to_string()),
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
    tool_source_digests: Vec<agendao_provider::cache::ToolSurfaceSourceDigest>,
    compiled_request: CompiledExecutionRequest,
    hooks: PromptHooks,
    config_store: Option<Arc<agendao_config::ConfigStore>>,
}

#[derive(Clone)]
struct RuntimeStepContext {
    provider: Arc<dyn Provider>,
    model_id: String,
    provider_id: String,
    agent_name: Option<String>,
    compiled_request: CompiledExecutionRequest,
    hooks: PromptHooks,
    config_store: Option<Arc<agendao_config::ConfigStore>>,
}

struct RuntimeStepInput {
    session_id: String,
    assistant_index: usize,
    chat_messages: Vec<agendao_provider::Message>,
    tool_registry: Arc<agendao_tool::ToolRegistry>,
    step_ctx: RuntimeStepContext,
}

struct PreparedChatMessages {
    prompt_messages: Vec<SessionMessage>,
    chat_messages: Vec<agendao_provider::Message>,
}

impl SessionPrompt {
    fn append_length_continuation_prompt(session: &mut Session) {
        session.add_synthetic_user_message(LENGTH_CONTINUATION_PROMPT, &[]);
    }

    pub(super) fn append_stream_tool_results_as_message(
        session: &mut Session,
        session_id: &str,
        stream_tool_results: Vec<super::StreamToolResultEntry>,
        config_store: Option<&agendao_config::ConfigStore>,
    ) {
        if stream_tool_results.is_empty() {
            return;
        }

        let artifacts_root = default_tool_result_artifacts_root(&session.record().directory);
        let config = config_store.map(|store| store.config());
        let stream_tool_results = govern_tool_result_batch(
            session_id,
            stream_tool_results,
            &artifacts_root,
            tool_result_budget(
                config
                    .as_deref()
                    .and_then(|cfg| cfg.runtime_budget.as_ref()),
            ),
        );

        let mut tool_msg = SessionMessage::tool(session_id.to_string());
        for (tool_call_id, content, is_error, title, metadata, attachments) in stream_tool_results {
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
            live_identity: None,
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

        session.insert_metadata(
            PENDING_SANITIZER_STAGE_METADATA_KEY,
            serde_json::json!(agendao_types::SanitizerStage::SessionResume.label()),
        );

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
            tool_result_budget(
                input
                    .step_ctx
                    .config_store
                    .as_ref()
                    .map(|store| store.config())
                    .as_deref()
                    .and_then(|cfg| cfg.runtime_budget.as_ref()),
            ),
        );
        let policy = LoopPolicy {
            max_steps: Some(MAX_STEPS),
            tool_dedup: ToolDedupScope::None,
            ..Default::default()
        };
        let mut chat_messages = input.chat_messages;
        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &policy,
            &cancel,
            &mut chat_messages,
        )
        .await;
        let mut output = sink.into_output();

        let persisted = subsessions.lock().await.clone();
        Self::save_persisted_subsessions(session, &persisted);

        match outcome {
            Ok(outcome) => {
                // P3-F: Propagate stream termination for backfill decisions.
                output.stream_termination = outcome.stream_termination;
                Ok(output)
            }
            Err(RuntimeLoopError::ModelError(failure)) => Err(anyhow::Error::new(
                super::PromptError::ProviderFailure(failure),
            )),
            Err(RuntimeLoopError::ModelErrorWithTermination {
                failure,
                stream_termination,
            }) => {
                output.stream_termination = Some(stream_termination);
                Err(anyhow::Error::new(super::PromptError::ProviderFailure(
                    failure,
                )))
            }
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

    async fn compact_context_for_reason(
        session_id: &str,
        session: &mut Session,
        filtered_messages: &[SessionMessage],
        update_hook: Option<&super::SessionUpdateHook>,
        compaction_lifecycle_hook: Option<&super::CompactionLifecycleHook>,
        output_block_hook: Option<&super::OutputBlockHook>,
        status_start: &'static str,
        status_success: &'static str,
        status_failure: &'static str,
        lifecycle: agendao_types::ContextCompactionLifecycleSummary,
        record: serde_json::Value,
        force: bool,
    ) -> bool {
        super::persist_context_compaction_lifecycle_summary(session, &lifecycle);
        session.start_compacting();
        Self::emit_session_update(update_hook, session);
        super::emit_context_compaction_lifecycle(compaction_lifecycle_hook, &lifecycle);
        Self::emit_context_compaction_status(
            output_block_hook,
            session_id,
            StatusBlock::warning(status_start),
        )
        .await;

        if let Some(summary) = Self::trigger_compaction_with_record(
            session,
            filtered_messages,
            None,
            Some(record),
            force,
        ) {
            tracing::info!(session_id = %session_id, summary, "context compacted");
            session.insert_metadata(
                PENDING_SANITIZER_STAGE_METADATA_KEY,
                serde_json::json!(agendao_types::SanitizerStage::PostCompaction.label()),
            );
            let mut lifecycle = lifecycle.clone();
            lifecycle.status = agendao_types::ContextCompactionLifecycleStatus::Installed;
            super::install_compaction_lifecycle_summary(session, &mut lifecycle);
            super::persist_context_compaction_lifecycle_summary(session, &lifecycle);
            session.finish_compacting();
            Self::emit_session_update(update_hook, session);
            super::emit_context_compaction_lifecycle(compaction_lifecycle_hook, &lifecycle);
            Self::emit_context_compaction_status(
                output_block_hook,
                session_id,
                StatusBlock::success(status_success),
            )
            .await;
            return true;
        }

        let mut lifecycle = lifecycle;
        lifecycle.status = agendao_types::ContextCompactionLifecycleStatus::Failed;
        super::persist_context_compaction_lifecycle_summary(session, &lifecycle);
        session.finish_compacting();
        Self::emit_session_update(update_hook, session);
        super::emit_context_compaction_lifecycle(compaction_lifecycle_hook, &lifecycle);
        Self::emit_context_compaction_status(
            output_block_hook,
            session_id,
            StatusBlock::warning(status_failure),
        )
        .await;
        false
    }

    async fn maybe_compact_context_from_request_view(
        session_id: &str,
        session: &mut Session,
        filtered_messages: &[SessionMessage],
        provider: &Arc<dyn Provider>,
        model_id: &str,
        compiled_request: &CompiledExecutionRequest,
        config_store: Option<&agendao_config::ConfigStore>,
        update_hook: Option<&super::SessionUpdateHook>,
        compaction_lifecycle_hook: Option<&super::CompactionLifecycleHook>,
        output_block_hook: Option<&super::OutputBlockHook>,
        request_context_tokens: Option<u64>,
        request_body_chars: Option<usize>,
    ) -> bool {
        let compaction_config = Self::runtime_compaction_config(config_store);
        if let Some(trim_summary) = Self::apply_lightweight_tool_result_trim(session) {
            let live_context_tokens =
                super::estimate_current_context_tokens(&session.record().messages);
            let summary = super::context_pressure_governance_summary(
                "auto_preflight",
                "prompt.pre_request",
                agendao_types::ContextPressureGovernanceStatus::Compacted,
                Some("lightweight_tool_result_trim"),
                request_context_tokens,
                live_context_tokens,
                None,
                request_body_chars,
                true,
                true,
                false,
                Some(trim_summary.clone()),
                Some(super::context_compaction_decision_trace(
                    "prompt.pre_request",
                    "lightweight_trim",
                    Some("lightweight_tool_result_trim"),
                    None,
                    None,
                    Some(trim_summary.clone()),
                )),
            );
            super::record_context_pressure_governance_summary(session, &summary);
            super::persist_lightweight_trim_summary(session, Some(&trim_summary));
            tracing::info!(
                session_id = %session_id,
                trimmed_rounds = trim_summary.trimmed_rounds,
                trimmed_tool_calls = trim_summary.trimmed_tool_calls,
                trimmed_tool_results = trim_summary.trimmed_tool_results,
                trimmed_call_tokens = trim_summary.trimmed_call_tokens,
                trimmed_result_tokens = trim_summary.trimmed_result_tokens,
                "applied lightweight tool-result trim before pre-request compaction"
            );
            return true;
        }
        super::persist_lightweight_trim_summary(session, None);
        let Some(assessment) = Self::assess_compaction(
            filtered_messages,
            provider.as_ref(),
            model_id,
            compiled_request.max_tokens,
            &compaction_config,
            None,
            request_context_tokens,
            request_body_chars,
        ) else {
            if let Some(backoff) = Self::auto_compaction_backoff_summary(filtered_messages) {
                let summary = super::context_pressure_governance_summary(
                    "auto_preflight",
                    "prompt.pre_request",
                    agendao_types::ContextPressureGovernanceStatus::Deferred,
                    Some("auto_compaction_backoff"),
                    request_context_tokens,
                    None,
                    None,
                    request_body_chars,
                    false,
                    false,
                    false,
                    None,
                    Some(super::context_compaction_decision_trace(
                        "prompt.pre_request",
                        "auto_compaction_backoff",
                        Some("auto_compaction_backoff"),
                        None,
                        Some(backoff),
                        None,
                    )),
                );
                super::record_context_pressure_governance_summary(session, &summary);
            }
            return false;
        };

        tracing::info!(
            session_id = %session_id,
            reason = assessment.reason,
            request_context_tokens,
            request_body_chars,
            "pre-request compaction triggered from request view"
        );
        let force_compaction = Self::should_force_compaction_for_reason(assessment.reason);
        let lifecycle = super::context_compaction_lifecycle_summary(
            "auto_preflight",
            Some("prompt.pre_request"),
            Some(assessment.reason),
            agendao_types::ContextCompactionLifecycleStatus::Started,
            force_compaction,
            request_context_tokens,
            None,
            assessment.limit_tokens,
            assessment.body_chars,
        );
        let record = Self::build_compaction_record(
            "auto_preflight",
            Some("prompt.pre_request"),
            Some(assessment.reason),
            force_compaction,
            request_context_tokens,
            None,
            assessment.limit_tokens,
            assessment.body_chars,
        );
        let compacted = Self::compact_context_for_reason(
            session_id,
            session,
            filtered_messages,
            update_hook,
            compaction_lifecycle_hook,
            output_block_hook,
            "Auto-compacting context before the next provider request...",
            "Context compacted before the next provider request.",
            "Auto-compaction ran, but the context could not be reduced.",
            lifecycle,
            record,
            force_compaction,
        )
        .await;
        let live_context_tokens =
            super::estimate_current_context_tokens(&session.record().messages);
        let summary = super::context_pressure_governance_summary(
            "auto_preflight",
            "prompt.pre_request",
            if compacted {
                agendao_types::ContextPressureGovernanceStatus::Compacted
            } else {
                agendao_types::ContextPressureGovernanceStatus::Deferred
            },
            Some(assessment.reason),
            request_context_tokens,
            live_context_tokens,
            assessment.limit_tokens,
            assessment.body_chars.or(request_body_chars),
            true,
            compacted,
            false,
            None,
            Some(super::context_compaction_decision_trace(
                "prompt.pre_request",
                if compacted {
                    "full_compaction"
                } else {
                    "deferred_after_compaction_attempt"
                },
                Some(assessment.reason),
                Some(&assessment),
                None,
                None,
            )),
        );
        super::record_context_pressure_governance_summary(session, &summary);
        compacted
    }

    fn provider_failure_is_overflow(error: &anyhow::Error) -> bool {
        if let Some(summary) = super::provider_error_summary_from_anyhow(error) {
            if summary.standard_code == StandardErrorCode::RequestTooLarge {
                return true;
            }
            if agendao_provider::ProviderError::is_overflow(&summary.message) {
                return true;
            }
        }

        super::untyped_provider_error_text_from_anyhow(error)
            .map(|message| agendao_provider::ProviderError::is_overflow(&message))
            .unwrap_or(false)
    }

    async fn maybe_recover_provider_overflow(
        session_id: &str,
        session: &mut Session,
        assistant_message_id: &str,
        update_hook: Option<&super::SessionUpdateHook>,
        compaction_lifecycle_hook: Option<&super::CompactionLifecycleHook>,
        output_block_hook: Option<&super::OutputBlockHook>,
        request_context_tokens: Option<u64>,
        request_body_chars: Option<usize>,
    ) -> bool {
        let drop_placeholder = session
            .get_message(assistant_message_id)
            .map(|message| {
                message.parts.is_empty() && message.finish.is_none() && message.usage.is_none()
            })
            .unwrap_or(false);
        if drop_placeholder {
            let _ = session.remove_message(assistant_message_id);
        }

        let filtered_messages = Self::filter_compacted_messages(&session.messages);
        let lifecycle = super::context_compaction_lifecycle_summary(
            "overflow_recovery",
            Some("prompt.provider_overflow"),
            Some("provider_overflow"),
            agendao_types::ContextCompactionLifecycleStatus::Started,
            true,
            request_context_tokens,
            None,
            None,
            request_body_chars,
        );
        let record = Self::build_compaction_record(
            "overflow_recovery",
            Some("prompt.provider_overflow"),
            Some("provider_overflow"),
            true,
            request_context_tokens,
            None,
            None,
            request_body_chars,
        );
        Self::compact_context_for_reason(
            session_id,
            session,
            &filtered_messages,
            update_hook,
            compaction_lifecycle_hook,
            output_block_hook,
            "Provider rejected the request as too large. Compacting and retrying...",
            "Context compacted after provider overflow. Retrying the session.",
            "Provider overflow recovery could not reduce the context.",
            lifecycle,
            record,
            true,
        )
        .await
    }

    async fn prepare_chat_messages(
        session_id: &str,
        agent_name: Option<&str>,
        system_prompt: Option<&str>,
        mut filtered_messages: Vec<SessionMessage>,
        provider_type: ProviderType,
    ) -> anyhow::Result<PreparedChatMessages> {
        if agendao_plugin::should_trigger_agent_hooks(HookEvent::ChatMessagesTransform, agent_name)
            .await
        {
            let hook_messages = serde_json::Value::Array(
                filtered_messages
                    .iter()
                    .map(session_message_hook_payload)
                    .collect(),
            );
            let message_hook_outputs = agendao_plugin::trigger_collect(
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
        Ok(PreparedChatMessages {
            prompt_messages,
            chat_messages,
        })
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

    fn latest_prompt_surface_state_snapshot(
        session: &Session,
    ) -> Option<PromptSurfaceStateSnapshot> {
        session
            .metadata
            .get(PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .or_else(|| {
                session.messages.iter().rev().find_map(|message| {
                    message
                        .metadata
                        .get(PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY)
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                })
            })
    }

    fn prompt_surface_stable_fields(
        session: &Session,
        prompt_messages: &[SessionMessage],
        provider_id: &str,
        model_id: &str,
        compiled_request: &CompiledExecutionRequest,
        fingerprint: &CacheRequestFingerprint,
        system_prompt: Option<&str>,
        tool_source_surface_hash: String,
    ) -> PromptSurfaceStableFields {
        let provider_options = compiled_request.provider_options.as_ref();
        let reasoning_mode_hash = provider_options
            .map(|options| {
                collect_prompt_surface_provider_options(
                    options,
                    PromptSurfaceProviderOptionGroup::ReasoningMode,
                )
            })
            .filter(|relevant| !relevant.is_empty())
            .map(serde_json::Value::Object)
            .map(|value| agendao_provider::cache::json_fingerprint(&value));
        let tool_policy_hash = Self::tool_policy_hash(provider_options);
        let api_shape =
            (fingerprint.family == CacheProtocolFamily::CloseAiCompatible).then(
                || match fingerprint
                    .provider_profile
                    .as_ref()
                    .map(|profile| profile.api_shape)
                {
                    Some(agendao_provider::ProviderApiShape::Responses) => {
                        agendao_provider::cache::CloseAiCompatibleApiShape::Responses
                    }
                    _ => agendao_provider::cache::CloseAiCompatibleApiShape::ChatCompletions,
                },
            );
        let closeai_prompt_cache_key = fingerprint
            .closeai
            .as_ref()
            .and_then(|closeai| closeai.prompt_cache_key.clone());
        let ethnopic_policy_hash = (fingerprint.family == CacheProtocolFamily::EthnopicCompatible)
            .then(|| {
                agendao_provider::cache::ethnopic_cache_policy_hash(&EthnopicCachePolicy::default())
            });
        let ethnopic_breakpoint_plan_hash = fingerprint.ethnopic.as_ref().map(|ethnopic| {
            agendao_provider::cache::json_fingerprint(&serde_json::json!({
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
            output_projection_policy_hash: Self::output_projection_policy_hash(prompt_messages),
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
        Some(agendao_provider::cache::json_fingerprint(
            &serde_json::json!({
                "source": source,
                "policy": policy,
            }),
        ))
    }

    fn stable_system_surface_hash(system_prompt: Option<&str>) -> String {
        let projection = Self::stable_system_surface_projection(system_prompt.unwrap_or_default());
        agendao_provider::cache::text_fingerprint(&projection)
    }

    fn stable_system_surface_projection(system_prompt: &str) -> String {
        let mut lines = Vec::new();
        let mut skipping_section = false;

        for line in system_prompt.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                let title = header.trim();
                skipping_section = is_volatile_system_section(title);
                if skipping_section {
                    continue;
                }
            }

            if skipping_section {
                continue;
            }

            lines.push(normalize_stable_system_line(line).into_owned());
        }

        lines.join("\n")
    }

    fn scc_stable_refs_hash(session: &Session) -> Option<String> {
        let packet = session
            .metadata
            .get("scheduler_session_context_packet")
            .and_then(SessionContinuityPacket::from_value)?;

        Some(agendao_provider::cache::json_fingerprint(
            &packet.stable_refs_value(),
        ))
    }

    fn tool_source_surface_hash(
        base_source_digests: &[agendao_provider::cache::ToolSurfaceSourceDigest],
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
            vec![agendao_provider::cache::ToolSurfaceSourceDigest {
                source: agendao_provider::cache::ToolSurfaceSourceKind::Base,
                tool_count: base_tools.len(),
                tools_hash: agendao_provider::cache::tool_surface_fingerprint(base_tools),
            }]
        } else {
            base_source_digests.to_vec()
        };
        groups.push(agendao_provider::cache::ToolSurfaceSourceDigest {
            source: agendao_provider::cache::ToolSurfaceSourceKind::Mcp,
            tool_count: effective_extra.len(),
            tools_hash: agendao_provider::cache::tool_surface_fingerprint(&effective_extra),
        });

        agendao_provider::cache::tool_source_surface_fingerprint(&groups)
    }

    fn tool_policy_hash(
        provider_options: Option<&HashMap<String, serde_json::Value>>,
    ) -> Option<String> {
        let relevant = collect_prompt_surface_provider_options(
            provider_options?,
            PromptSurfaceProviderOptionGroup::ToolPolicy,
        );
        (!relevant.is_empty())
            .then(|| agendao_provider::cache::json_fingerprint(&serde_json::Value::Object(relevant)))
    }

    fn output_projection_policy_hash(prompt_messages: &[SessionMessage]) -> String {
        let projected = prompt_messages
            .iter()
            .filter_map(sanctioned_model_context_projection_for_message)
            .map(|projection| {
                serde_json::json!({
                    "path": projection.path.as_str(),
                    "policy": projection.policy,
                    "legacy_without_policy": projection.legacy_without_policy,
                })
            })
            .collect::<Vec<_>>();

        agendao_provider::cache::json_fingerprint(&serde_json::json!({
            "owner": "sanctioned_model_context_projection",
            "entries": projected,
        }))
    }

    fn build_prompt_surface_state_snapshot(
        session_id: &str,
        previous: Option<&PromptSurfaceStateSnapshot>,
        stable: PromptSurfaceStableFields,
        now_ms: i64,
    ) -> PromptSurfaceStateSnapshot {
        let evidence =
            previous.and_then(|snapshot| Self::prompt_surface_evidence(snapshot, &stable));
        let generation = match previous {
            Some(snapshot) if evidence.is_none() => snapshot.generation,
            Some(snapshot) => snapshot.generation.saturating_add(1),
            None => 1,
        };
        let created_at_ms = previous
            .filter(|_| evidence.is_none())
            .map(|snapshot| snapshot.created_at_ms)
            .unwrap_or(now_ms);

        PromptSurfaceStateSnapshot {
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
            evidence,
        }
    }

    fn prompt_surface_evidence(
        previous: &PromptSurfaceStateSnapshot,
        current: &PromptSurfaceStableFields,
    ) -> Option<PromptSurfaceEvidence> {
        let mut changed_fields = Vec::new();
        let mut severity = agendao_provider::cache::CacheEvidenceSeverity::Stable;

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
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "providerId",
            previous.provider_id.as_str(),
            current.provider_id.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "modelId",
            previous.model_id.as_str(),
            current.model_id.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "apiShape",
            previous.api_shape,
            current.api_shape,
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "stableSystemSurfaceHash",
            previous.stable_system_surface_hash.as_str(),
            current.stable_system_surface_hash.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "toolSurfaceHash",
            previous.tool_surface_hash.as_str(),
            current.tool_surface_hash.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "toolSourceSurfaceHash",
            previous.tool_source_surface_hash.as_str(),
            current.tool_source_surface_hash.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "providerParamsHash",
            previous.provider_params_hash.as_str(),
            current.provider_params_hash.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::HighChange
        );
        compare_field!(
            "toolPolicyHash",
            previous.tool_policy_hash.as_deref(),
            current.tool_policy_hash.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "reasoningModeHash",
            previous.reasoning_mode_hash.as_deref(),
            current.reasoning_mode_hash.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "outputProjectionPolicyHash",
            previous.output_projection_policy_hash.as_str(),
            current.output_projection_policy_hash.as_str(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "sccStableRefsHash",
            previous.scc_stable_refs_hash.as_deref(),
            current.scc_stable_refs_hash.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "closeaiPromptCacheKey",
            previous.closeai_prompt_cache_key.as_deref(),
            current.closeai_prompt_cache_key.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "ethnopicPolicyHash",
            previous.ethnopic_policy_hash.as_deref(),
            current.ethnopic_policy_hash.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "ethnopicBreakpointPlanHash",
            previous.ethnopic_breakpoint_plan_hash.as_deref(),
            current.ethnopic_breakpoint_plan_hash.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::MediumChange
        );
        compare_field!(
            "ingressPolicyHash",
            previous.ingress_policy_hash.as_deref(),
            current.ingress_policy_hash.as_deref(),
            agendao_provider::cache::CacheEvidenceSeverity::LowChange
        );

        if changed_fields.is_empty() {
            return None;
        }

        let reason = format!("surface changed: {}", changed_fields.join(", "));
        Some(PromptSurfaceEvidence {
            severity,
            reason,
            changed_fields,
        })
    }

    fn merge_snapshot_evidence_into_summary(
        mut summary: CacheEvidenceSummary,
        evidence: Option<&PromptSurfaceEvidence>,
    ) -> CacheEvidenceSummary {
        let Some(evidence) = evidence else {
            return summary;
        };
        if evidence.severity >= summary.severity {
            summary.status = "degraded".to_string();
            summary.severity = evidence.severity;
            summary.primary_cause = Some(evidence.reason.clone());
        }
        summary
    }

    fn cache_request_fingerprint(
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        system_prompt: Option<&str>,
        messages: &[agendao_provider::Message],
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
                agendao_provider::cache::plan_ethnopic_message_breakpoints(messages);
            let cache_policy = EthnopicCachePolicy::default();
            EthnopicCacheFingerprint {
                cache_control_hash: agendao_provider::cache::json_fingerprint(&serde_json::json!({
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
        agendao_provider::cache::closeai_prompt_cache_key_field(
            provider_id,
            "",
            &provider_options_object,
        )
        .map(|_| {
            agendao_provider::cache::build_prompt_cache_key(
                agendao_provider::cache::PromptCacheKeyContext {
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
            if let Some(stream_termination) = step_output.stream_termination.as_ref() {
                if let Ok(value) = serde_json::to_value(stream_termination) {
                    assistant_msg
                        .metadata
                        .insert("stream_termination".to_string(), value);
                }
            }
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
        if !agendao_plugin::should_trigger_agent_hooks(HookEvent::ChatMessage, agent_name).await {
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

        let hook_outputs = agendao_plugin::trigger_collect(hook_ctx).await;
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
        let mut overflow_recovery_attempts = 0_u8;
        let mut length_continuation_retries = 0_u8;
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

            let PreparedChatMessages {
                prompt_messages,
                mut chat_messages,
            } = Self::prepare_chat_messages(
                &session_id,
                prompt_ctx.agent_name.as_deref(),
                prompt_ctx.system_prompt.as_deref(),
                filtered_messages,
                provider_type,
            )
            .await?;

            let sanitizer_stage = take_pending_sanitizer_stage(session);

            // P0.2: Run the shared sanitizer contract on every pre-request path.
            // P1.2: Policy gates whether synthetic repairs are injected.
            let repair_policy =
                crate::compaction::effective_repair_policy(prompt_ctx.config_store.as_deref());
            let (sanitized, _telemetry) = super::sanitizer_contract::sanitize_with_contract(
                &chat_messages,
                sanitizer_stage,
                repair_policy,
                &mut session.record_mut().metadata,
            );
            chat_messages = sanitized;

            // P0.4: Inject the latest tool batch summary into the model context
            // so the model can consume structured results from the previous turn.
            Self::inject_latest_tool_batch_summary(session, &mut chat_messages);

            // Tool-boundary steering (Constitution §5, §9): drain pending steering,
            // inject into chat_messages for the current request, and write a clean
            // model-visible transcript record for future replay.
            if let Some(ref hook) = prompt_ctx.hooks.steering_boundary_hook {
                let owner_id = session.record().id.clone();
                let steering_msgs = hook(owner_id.clone()).await;
                let consumed = steering_msgs.len();
                let last_source = steering_msgs
                    .iter()
                    .rev()
                    .find_map(|sm| sm.source_session_id.clone());
                let now = chrono::Utc::now().timestamp_millis();
                let last_latency_ms = steering_msgs.iter().rev().find_map(|sm| {
                    (sm.created_at > 0).then_some(now.saturating_sub(sm.created_at) as u64)
                });
                for (i, sm) in steering_msgs.into_iter().enumerate() {
                    // Write the stable, model-visible transcript record.
                    // Unlike the enqueue-time preview (runtime_hint=steering_preview),
                    // this record IS replayed to the model in future turns so it
                    // retains the steering context across compaction/resume.
                    let mut record =
                        crate::SessionMessage::user(session.id.clone(), sm.text.clone());
                    record.metadata.insert(
                        "steering_mode".to_string(),
                        serde_json::json!("next_tool_boundary"),
                    );
                    record
                        .metadata
                        .insert("steering_status".to_string(), serde_json::json!("consumed"));
                    record
                        .metadata
                        .insert("steering_index".to_string(), serde_json::json!(i));
                    record
                        .metadata
                        .insert("steering_injected_at".to_string(), serde_json::json!(now));
                    record.metadata.insert(
                        "steering_owner_session_id".to_string(),
                        serde_json::json!(owner_id.clone()),
                    );
                    record.metadata.insert(
                        "steering_injected_during_active_run".to_string(),
                        serde_json::json!(true),
                    );
                    if let Some(ref source) = sm.source_session_id {
                        record.metadata.insert(
                            "steering_source_session_id".to_string(),
                            serde_json::json!(source),
                        );
                    }
                    let (admission, authority) = agendao_types::origin_to_admission_authority(
                        agendao_types::MessageSourceOrigin::System,
                    );
                    agendao_types::apply_message_source_metadata(
                        &mut record.metadata,
                        agendao_types::MessageSourceOrigin::System,
                        agendao_types::MessageSourceSurface::HttpApi,
                    );
                    agendao_types::apply_message_admission_metadata(
                        &mut record.metadata,
                        admission,
                        authority,
                    );
                    session.push_message(record);

                    // Inject into the current request's chat messages.
                    chat_messages.push(agendao_provider::Message::user(sm.text));
                }

                // Write session-level steering telemetry (Patch 4).
                let consumed = consumed as u64;
                let previous: u64 = session
                    .record()
                    .metadata
                    .get("consumed_steering_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                session.insert_metadata(
                    "consumed_steering_count".to_string(),
                    serde_json::json!(previous + consumed),
                );
                session.insert_metadata(
                    "last_steering_injected_at".to_string(),
                    serde_json::json!(now),
                );
                // Always write to clear stale values from previous batches.
                session.insert_metadata(
                    "last_steering_source_session_id".to_string(),
                    serde_json::json!(last_source),
                );
                session.insert_metadata(
                    "last_steering_latency_ms".to_string(),
                    serde_json::json!(last_latency_ms),
                );
            }

            let (request_context_tokens, request_body_chars) =
                Self::estimate_request_context_tokens_from_provider_messages(&chat_messages);
            let filtered_messages = Self::filter_compacted_messages(&session.messages);
            if Self::maybe_compact_context_from_request_view(
                &session_id,
                session,
                &filtered_messages,
                &prompt_ctx.provider,
                &prompt_ctx.model_id,
                &prompt_ctx.compiled_request,
                prompt_ctx.config_store.as_deref(),
                prompt_ctx.hooks.update_hook.as_ref(),
                prompt_ctx.hooks.compaction_lifecycle_hook.as_ref(),
                prompt_ctx.hooks.output_block_hook.as_ref(),
                request_context_tokens,
                Some(request_body_chars),
            )
            .await
            {
                continue;
            }

            tracing::info!(
                step = step,
                session_id = %session_id,
                message_count = prompt_messages.len(),
                request_context_tokens,
                "prompt loop step start"
            );
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
            let cache_evidence_inspection = inspect_cache_fingerprint_change(
                previous_cache_fingerprint.as_ref(),
                &cache_fingerprint,
            );
            let previous_prompt_surface_state_snapshot =
                Self::latest_prompt_surface_state_snapshot(session);
            let prompt_surface_stable_fields = Self::prompt_surface_stable_fields(
                session,
                &prompt_messages,
                &prompt_ctx.provider_id,
                &prompt_ctx.model_id,
                &prompt_ctx.compiled_request,
                &cache_fingerprint,
                prompt_ctx.system_prompt.as_deref(),
                tool_source_surface_hash,
            );
            let prompt_surface_state_snapshot = Self::build_prompt_surface_state_snapshot(
                &session_id,
                previous_prompt_surface_state_snapshot.as_ref(),
                prompt_surface_stable_fields,
                chrono::Utc::now().timestamp_millis(),
            );
            if let Ok(value) = serde_json::to_value(&prompt_surface_state_snapshot) {
                session.insert_metadata(PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY, value);
            }

            let tool_registry = Arc::new(agendao_tool::create_default_registry().await);

            let assistant_index = session.messages.len();
            let assistant_message_id =
                agendao_core::id::create(agendao_core::id::Prefix::Message, true, None);
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
            if let Ok(value) = serde_json::to_value(&cache_evidence_inspection) {
                assistant_metadata
                    .insert(CACHE_EVIDENCE_INSPECTION_METADATA_KEY.to_string(), value);
            }
            if let Ok(value) = serde_json::to_value(&prompt_surface_state_snapshot) {
                assistant_metadata.insert(
                    PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY.to_string(),
                    value,
                );
            }
            if let Some(evidence) = prompt_surface_state_snapshot.evidence.as_ref() {
                if let Ok(value) = serde_json::to_value(evidence) {
                    assistant_metadata
                        .insert(PROMPT_SURFACE_EVIDENCE_METADATA_KEY.to_string(), value);
                }
            }
            let cache_evidence = Self::merge_snapshot_evidence_into_summary(
                CacheEvidenceSummary::from(&cache_evidence_inspection),
                prompt_surface_state_snapshot.evidence.as_ref(),
            );
            if let Ok(value) = serde_json::to_value(cache_evidence) {
                assistant_metadata.insert(CACHE_EVIDENCE_METADATA_KEY.to_string(), value);
            }
            session.messages_mut().push(SessionMessage {
                id: assistant_message_id.clone(),
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

            let step_output = match self
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
                .await
            {
                Ok(output) => {
                    overflow_recovery_attempts = 0;
                    output
                }
                Err(error) => {
                    if overflow_recovery_attempts == 0
                        && Self::provider_failure_is_overflow(&error)
                        && Self::maybe_recover_provider_overflow(
                            &session_id,
                            session,
                            &assistant_message_id,
                            prompt_ctx.hooks.update_hook.as_ref(),
                            prompt_ctx.hooks.compaction_lifecycle_hook.as_ref(),
                            prompt_ctx.hooks.output_block_hook.as_ref(),
                            request_context_tokens,
                            Some(request_body_chars),
                        )
                        .await
                    {
                        session.insert_metadata(
                            PENDING_SANITIZER_STAGE_METADATA_KEY,
                            serde_json::json!(agendao_types::SanitizerStage::FallbackRetry.label()),
                        );
                        overflow_recovery_attempts = 1;
                        continue;
                    }
                    return Err(error);
                }
            };

            let finish_reason = step_output.finish_reason.clone();
            let executed_local_tools_this_step = step_output.executed_local_tools_this_step;

            Self::finalize_assistant_message(session, assistant_index, &step_output);

            Self::append_stream_tool_results_as_message(
                session,
                &session_id,
                step_output.stream_tool_results,
                self.config_store.as_deref(),
            );

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

            if finish_reason.as_deref() == Some("length") && !has_tool_calls {
                if length_continuation_retries < MAX_LENGTH_CONTINUATION_RETRIES {
                    length_continuation_retries += 1;
                    tracing::info!(
                        session_id = %session_id,
                        retry = length_continuation_retries,
                        max_retries = MAX_LENGTH_CONTINUATION_RETRIES,
                        "assistant output hit max tokens; scheduling synthetic continuation turn"
                    );
                    Self::append_length_continuation_prompt(session);
                    session.touch();
                    Self::emit_session_update(prompt_ctx.hooks.update_hook.as_ref(), session);
                    continue;
                }
                tracing::warn!(
                    session_id = %session_id,
                    max_retries = MAX_LENGTH_CONTINUATION_RETRIES,
                    "assistant output remained truncated after continuation retries"
                );
            } else {
                length_continuation_retries = 0;
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::PromptError;
    use agendao_orchestrator::runtime::events::ModelFailure;
    use agendao_provider::{
        error_code::StandardErrorCode, ProviderDiagnosticSeverity, ProviderDiagnosticSource,
        ProviderDiagnosticSummary, ProviderErrorKind, ProviderErrorSummary,
    };

    #[test]
    fn provider_failure_is_overflow_accepts_typed_request_too_large() {
        let summary = ProviderErrorSummary {
            kind: ProviderErrorKind::InvalidRequest,
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-reasoner".to_string()),
            message: "maximum context length exceeded".to_string(),
            status_code: Some(400),
            standard_code: StandardErrorCode::RequestTooLarge,
            retryable: false,
            provider_diagnostic: Some(ProviderDiagnosticSummary {
                severity: ProviderDiagnosticSeverity::HardFail,
                source: ProviderDiagnosticSource::RequestValidation,
                code: "context_overflow".to_string(),
                provider_id: "deepseek".to_string(),
                model_id: Some("deepseek-reasoner".to_string()),
                message: "maximum context length exceeded".to_string(),
            }),
        };
        let error = anyhow::Error::new(PromptError::ProviderFailure(ModelFailure::Provider(
            summary,
        )));

        assert!(SessionPrompt::provider_failure_is_overflow(&error));
    }

    #[test]
    fn provider_failure_is_overflow_accepts_untyped_overflow_text() {
        let error = anyhow::Error::new(PromptError::ProviderFailure(ModelFailure::Message(
            "provider rejected request: maximum context length is 128000 tokens".to_string(),
        )));

        assert!(SessionPrompt::provider_failure_is_overflow(&error));
    }
}
