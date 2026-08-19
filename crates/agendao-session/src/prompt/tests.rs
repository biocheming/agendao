use super::*;
use crate::message::MessagePart;
use crate::SessionMessage;
use agendao_config::ConfigStore;
use agendao_skill::SkillGovernanceAuthority;
use agendao_storage::{Database, SkillEvolutionProposalRepository};
use agendao_types::{
    message_source_origin, message_source_surface, MemoryEvidenceRef, MemoryKind, MemoryRecord,
    MemoryRecordId, MemoryScope, MemoryStatus, MemoryValidationStatus, MessageSourceOrigin,
    MessageSourceSurface, ProposalStatus, SkillCapabilityGroupKind, SkillCapabilityMember,
    SkillCapabilityMemberRole,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

async fn prompt_with_memory_and_proposals(
    root: &std::path::Path,
) -> (SessionPrompt, Arc<SkillEvolutionProposalRepository>) {
    isolate_test_config_home();
    let config_store =
        Arc::new(ConfigStore::from_project_dir(root).expect("project config store should load"));
    let db = Database::in_memory().await.expect("db should initialize");
    let proposal_repo = Arc::new(SkillEvolutionProposalRepository::new(db.pool().clone()));
    let prompt = SessionPrompt::default()
        .with_config_store(config_store)
        .with_proposal_repo(proposal_repo.clone());
    (prompt, proposal_repo)
}

fn isolate_test_config_home() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let path =
            std::env::temp_dir().join(format!("agendao-session-test-home-{}", std::process::id()));
        fs::create_dir_all(&path).expect("session test config home should be created");
        std::env::set_var("AGENDAO_HOME", path);
    });
}

fn methodology_candidate_record(
    id: &str,
    session_id: &str,
    workspace_identity: &str,
    linked_skill_name: &str,
) -> MemoryRecord {
    MemoryRecord {
        id: MemoryRecordId(id.to_string()),
        kind: MemoryKind::MethodologyCandidate,
        scope: MemoryScope::WorkspaceShared,
        status: MemoryStatus::Consolidated,
        title: format!("Methodology for {linked_skill_name}"),
        summary: "Refined methodology".to_string(),
        trigger_conditions: vec!["when provider config needs refresh".to_string()],
        normalized_facts: vec!["provider config refresh flow".to_string()],
        boundaries: vec!["only patch existing refresh workflow".to_string()],
        confidence: Some(0.91),
        evidence_refs: vec![MemoryEvidenceRef {
            session_id: Some(session_id.to_string()),
            message_id: Some("msg-1".to_string()),
            tool_call_id: Some("tool-1".to_string()),
            stage_id: Some("stage-review".to_string()),
            note: Some("runtime review nudge".to_string()),
        }],
        source_session_id: Some(session_id.to_string()),
        workspace_identity: Some(workspace_identity.to_string()),
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
        last_validated_at: Some(1_700_000_000),
        expires_at: None,
        derived_skill_name: None,
        linked_skill_name: Some(linked_skill_name.to_string()),
        validation_status: MemoryValidationStatus::Passed,
    }
}

#[test]
fn provider_error_summary_from_anyhow_reads_wrapped_prompt_error() {
    let summary = agendao_provider::ProviderErrorSummary {
        kind: agendao_provider::ProviderErrorKind::InvalidRequest,
        provider_id: "deepseek".to_string(),
        model_id: Some("deepseek-reasoner".to_string()),
        message: "missing replay".to_string(),
        status_code: Some(400),
        standard_code: agendao_provider::error_code::StandardErrorCode::InvalidRequest,
        retryable: false,
        provider_diagnostic: Some(agendao_provider::ProviderDiagnosticSummary {
            severity: agendao_provider::ProviderDiagnosticSeverity::HardFail,
            source: agendao_provider::ProviderDiagnosticSource::RequestValidation,
            code: "thinking_replay_missing".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-reasoner".to_string()),
            message: "missing replay".to_string(),
        }),
    };
    let error = anyhow::Error::new(PromptError::ProviderFailure(summary.clone()))
        .context("session prompt failed");

    let loaded = provider_error_summary_from_anyhow(&error).expect("typed summary should load");

    assert_eq!(loaded, summary);
}

#[test]
fn provider_failure_from_anyhow_reads_wrapped_untyped_provider_message() {
    let error = anyhow::Error::new(PromptError::Provider(
            "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request"
                .to_string(),
    ))
    .context("session prompt failed");

    let failure = provider_failure_from_anyhow(&error).expect("provider failure should load");

    assert_eq!(
        failure,
        PromptProviderFailure::UntypedMessage(
            "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request"
                .to_string()
        )
    );
    assert_eq!(
        untyped_provider_error_text_from_anyhow(&error).as_deref(),
        Some(
            "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request"
        )
    );
}

#[tokio::test]
async fn create_user_message_uses_parts_as_authority_not_ingress_shadow_text() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let mut ingress = IngressTurnEnvelope::new_text(
        session.id.clone(),
        IngressSource::Web,
        "turn_shadow",
        100,
        "shadow text should not reach the model",
    );
    ingress.context_key = Some("session_prompt".to_string());
    ingress.idempotency_key = Some("idem_shadow".to_string());
    ingress.stabilization.policy = INGRESS_POLICY_ENTRY_METADATA_ONLY.to_string();

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "authoritative text from parts".to_string(),
        }],
        tools: None,
        ingress: Some(ingress),
    };

    prompt
        .create_user_message(&input, &mut session)
        .await
        .expect("user message should be created");

    let user_message = session
        .messages
        .iter()
        .find(|message| matches!(message.role, MessageRole::User))
        .expect("user message should exist");
    assert_eq!(user_message.get_text(), "authoritative text from parts");
    assert_eq!(
        user_message
            .metadata
            .get("ingress_source")
            .cloned()
            .expect("ingress source metadata should be recorded"),
        serde_json::json!(IngressSource::Web)
    );

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None, &[])
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            agendao_provider::Content::Text(text) => text.clone(),
            agendao_provider::Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered.contains("authoritative text from parts"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("shadow text should not reach the model"),
        "{rendered}"
    );
}

#[tokio::test]
async fn annotate_ingress_metadata_writes_canonical_source_when_present() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let mut ingress = IngressTurnEnvelope::new_text(
        session.id.clone(),
        IngressSource::Tui,
        "turn_src",
        100,
        "message with canonical source",
    );
    ingress.source_origin = Some(MessageSourceOrigin::Operator);
    ingress.source_surface = Some(MessageSourceSurface::Tui);

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "test".to_string(),
        }],
        tools: None,
        ingress: Some(ingress),
    };

    prompt
        .create_user_message(&input, &mut session)
        .await
        .expect("user message should be created");

    let user_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, MessageRole::User))
        .expect("user message should exist");

    let origin = message_source_origin(&user_msg.metadata);
    let surface = message_source_surface(&user_msg.metadata);
    assert_eq!(origin, Some(MessageSourceOrigin::Operator));
    assert_eq!(surface, Some(MessageSourceSurface::Tui));
}

#[tokio::test]
async fn annotate_ingress_metadata_skips_canonical_source_when_absent() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    // IngressTurnEnvelope::new_text leaves source_origin/source_surface as None.
    let ingress = IngressTurnEnvelope::new_text(
        session.id.clone(),
        IngressSource::Api,
        "turn_no_src",
        100,
        "message without canonical source",
    );

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "test".to_string(),
        }],
        tools: None,
        ingress: Some(ingress),
    };

    prompt
        .create_user_message(&input, &mut session)
        .await
        .expect("user message should be created");

    let user_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, MessageRole::User))
        .expect("user message should exist");

    assert!(
        !user_msg.metadata.contains_key("message_source.origin"),
        "source origin key should be absent when ingress has no source"
    );
    assert!(
        !user_msg.metadata.contains_key("message_source.surface"),
        "source surface key should be absent when ingress has no source"
    );
}

#[tokio::test]
async fn annotate_ingress_writes_origin_even_without_surface() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let mut ingress = IngressTurnEnvelope::new_text(
        session.id.clone(),
        IngressSource::Scheduler,
        "turn_sched",
        100,
        "scheduler message",
    );
    ingress.source_origin = Some(MessageSourceOrigin::Scheduler);
    // surface intentionally left None — scheduler has no transport surface

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "test".to_string(),
        }],
        tools: None,
        ingress: Some(ingress),
    };

    prompt
        .create_user_message(&input, &mut session)
        .await
        .expect("user message should be created");

    let user_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, MessageRole::User))
        .expect("user message should exist");

    let origin = message_source_origin(&user_msg.metadata);
    let surface = message_source_surface(&user_msg.metadata);
    assert_eq!(origin, Some(MessageSourceOrigin::Scheduler));
    assert_eq!(
        surface, None,
        "surface should be absent when ingress has no surface"
    );
}

// ── PartInput serde round-trip tests ──

#[test]
fn part_input_text_round_trip() {
    let part = PartInput::Text {
        text: "hello".to_string(),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "text");
    assert_eq!(json["text"], "hello");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::Text { text } if text == "hello"));
}

#[test]
fn part_input_file_round_trip() {
    let part = PartInput::File {
        url: "file:///tmp/test.rs".to_string(),
        filename: Some("test.rs".to_string()),
        mime: Some("text/plain".to_string()),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "file");
    assert_eq!(json["url"], "file:///tmp/test.rs");
    assert_eq!(json["filename"], "test.rs");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::File { url, .. } if url == "file:///tmp/test.rs"));
}

#[test]
fn part_input_try_from_value() {
    let val = serde_json::json!({"type": "text", "text": "hi"});
    let part = PartInput::try_from(val).unwrap();
    assert!(matches!(part, PartInput::Text { text } if text == "hi"));
}

#[test]
fn part_input_try_from_invalid_value() {
    let val = serde_json::json!({"type": "unknown", "data": 42});
    assert!(PartInput::try_from(val).is_err());
}

#[test]
fn part_input_parse_array_mixed() {
    let arr = serde_json::json!([
        {"type": "text", "text": "hello"},
        {"type": "bogus"},
        {"type": "file", "url": "file:///x", "filename": "x", "mime": "text/plain"}
    ]);
    let parts = PartInput::parse_array(&arr);
    assert_eq!(parts.len(), 2);
    assert!(matches!(&parts[0], PartInput::Text { text } if text == "hello"));
    assert!(matches!(&parts[1], PartInput::File { url, .. } if url == "file:///x"));
}

#[test]
fn part_input_parse_array_non_array() {
    let val = serde_json::json!("not an array");
    assert!(PartInput::parse_array(&val).is_empty());
}

#[test]
fn part_input_file_skips_none_fields_in_json() {
    let part = PartInput::File {
        url: "file:///tmp/x".to_string(),
        filename: None,
        mime: None,
    };
    let json = serde_json::to_value(&part).unwrap();
    assert!(json.get("filename").is_none());
    assert!(json.get("mime").is_none());
}

// ── resolve_prompt_parts tests ──

#[tokio::test]
async fn resolve_prompt_parts_plain_text() {
    let parts = resolve_prompt_parts("just plain text", std::path::Path::new("/tmp")).await;
    assert_eq!(parts.len(), 1);
    assert!(matches!(&parts[0], PartInput::Text { text } if text == "just plain text"));
}

#[tokio::test]
async fn resolve_prompt_parts_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.rs");
    tokio::fs::write(&file, "fn main() {}").await.unwrap();

    let parts = resolve_prompt_parts("look at @test.rs", dir.path()).await;
    assert_eq!(parts.len(), 2);
    assert!(
        matches!(&parts[1], PartInput::File { mime, .. } if mime.as_deref() == Some("text/plain"))
    );
}

#[tokio::test]
async fn resolve_prompt_parts_directory() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("src");
    tokio::fs::create_dir(&sub).await.unwrap();

    let parts = resolve_prompt_parts("look at @src", dir.path()).await;
    assert_eq!(parts.len(), 2);
    assert!(
        matches!(&parts[1], PartInput::File { mime, .. } if mime.as_deref() == Some("application/x-directory"))
    );
}

/// Regression test for the prompt loop early-exit bug:
/// When the assistant message has text + tool calls and finish="tool-calls",
/// the loop must NOT break at the top-of-loop check.
/// Previously, the check used `has_finish = !text.is_empty()` which caused
/// premature exit when models emit text before tool calls.
#[test]
fn early_exit_does_not_break_on_tool_calls_finish() {
    // Simulate: user message at index 0, assistant at index 1
    let user = SessionMessage::user("s1", "hello");
    let mut assistant = SessionMessage::assistant("s1");
    // Assistant has text content (model explained before calling tools)
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "Let me read those files for you.".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    // finish_reason is "tool-calls" — loop should continue, not break
    assistant.finish = Some("tool-calls".to_string());

    let messages = [user, assistant];

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User))
        .unwrap();
    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    // The early-exit check from the prompt loop
    let should_break = if let Some(assistant_idx) = last_assistant_idx {
        let assistant = &messages[assistant_idx];
        let is_terminal = assistant
            .finish
            .as_deref()
            .is_some_and(|f| !matches!(f, "tool-calls" | "tool_calls" | "unknown"));
        is_terminal && last_user_idx < assistant_idx
    } else {
        false
    };

    assert!(
        !should_break,
        "early-exit must NOT trigger when finish='tool-calls'"
    );
}

/// Verify that the early-exit check DOES break when finish is terminal
/// (e.g. "stop") and assistant is after the last user message.
#[test]
fn early_exit_breaks_on_terminal_finish() {
    let user = SessionMessage::user("s1", "hello");
    let mut assistant = SessionMessage::assistant("s1");
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "Here is my response.".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.finish = Some("stop".to_string());

    let messages = [user, assistant];

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User))
        .unwrap();
    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    let should_break = if let Some(assistant_idx) = last_assistant_idx {
        let assistant = &messages[assistant_idx];
        let is_terminal = assistant
            .finish
            .as_deref()
            .is_some_and(|f| !matches!(f, "tool-calls" | "tool_calls" | "unknown"));
        is_terminal && last_user_idx < assistant_idx
    } else {
        false
    };

    assert!(should_break, "early-exit MUST trigger when finish='stop'");
}

/// Verify that the early-exit check does NOT break when finish is None
/// (assistant message still streaming / no FinishStep received yet).
#[test]
fn early_exit_does_not_break_when_finish_is_none() {
    let user = SessionMessage::user("s1", "hello");
    let mut assistant = SessionMessage::assistant("s1");
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "partial response...".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    // finish is None — still streaming
    assistant.finish = None;

    let messages = [user, assistant];

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User))
        .unwrap();
    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    let should_break = if let Some(assistant_idx) = last_assistant_idx {
        let assistant = &messages[assistant_idx];
        let is_terminal = assistant
            .finish
            .as_deref()
            .is_some_and(|f| !matches!(f, "tool-calls" | "tool_calls" | "unknown"));
        is_terminal && last_user_idx < assistant_idx
    } else {
        false
    };

    assert!(
        !should_break,
        "early-exit must NOT trigger when finish is None"
    );
}

#[test]
fn chat_message_hook_not_triggered_on_user_message_creation() {
    let source = include_str!("mod.rs");
    let create_user_fn = source
        .find("async fn create_user_message")
        .expect("create_user_message should exist");
    let rest = &source[create_user_fn..];
    let next_method = rest[1..]
        .find("\n    async fn ")
        .or_else(|| rest[1..].find("\n    pub async fn "))
        .map(|offset| offset + 1)
        .unwrap_or(rest.len());
    let create_user_section = &rest[..next_method];
    assert!(
        !create_user_section.contains("HookEvent::ChatMessage"),
        "ChatMessage hook should not be in create_user_message"
    );
}

#[test]
fn proposal_notice_is_hidden_from_model_prompt_surface() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("analyze the workspace and extract lessons");

    maybe_append_proposal_notice(
        &mut session,
        &NudgeDecision::Triggered {
            promoted: 0,
            merged: 0,
            archived: 0,
            promoted_records: 0,
            proposals_created: 2,
            proposals_skipped: 0,
        },
    );

    let notice = session
        .messages
        .last()
        .cloned()
        .expect("proposal notice should be appended");
    assert_eq!(
        notice
            .metadata
            .get("runtime_hint")
            .and_then(|value| value.as_str()),
        Some("proposal_notice")
    );

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None, &[])
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            agendao_provider::Content::Text(text) => text.clone(),
            agendao_provider::Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !rendered.contains("skill evolution proposal(s) generated"),
        "{rendered}"
    );
}

#[test]
fn steering_preview_is_hidden_from_model_consumed_is_visible() {
    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();

    // Normal user message (model-visible baseline).
    session.add_user_message("run the deploy");

    // Simulate enqueue-time preview: both notice and text with runtime_hint=steering_preview.
    let mut notice = SessionMessage::user(
        sid.as_str(),
        "Steering: will be applied at next tool boundary",
    );
    notice.metadata.insert(
        "runtime_hint".to_string(),
        serde_json::json!("steering_preview"),
    );
    notice
        .metadata
        .insert("steering_status".to_string(), serde_json::json!("pending"));
    session.push_message(notice);

    let mut preview = SessionMessage::user(sid.as_str(), "switch to mode: cautious");
    preview.metadata.insert(
        "runtime_hint".to_string(),
        serde_json::json!("steering_preview"),
    );
    preview
        .metadata
        .insert("steering_status".to_string(), serde_json::json!("pending"));
    session.push_message(preview);

    // Normal assistant reply.
    let mut assistant = SessionMessage::assistant(sid.as_str());
    assistant.add_text("deploy started");
    session.push_message(assistant);

    // Simulate tool-boundary consumed record: same text, model-visible (no runtime_hint).
    let mut consumed = SessionMessage::user(sid.as_str(), "switch to mode: cautious");
    consumed
        .metadata
        .insert("steering_status".to_string(), serde_json::json!("consumed"));
    consumed.metadata.insert(
        "steering_injected_at".to_string(),
        serde_json::json!(chrono::Utc::now().timestamp_millis()),
    );
    session.push_message(consumed);

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None, &[])
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            agendao_provider::Content::Text(text) => text.clone(),
            agendao_provider::Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Preview meta-notice must NOT appear in model context.
    assert!(
        !rendered.contains("will be applied at next tool boundary"),
        "preview notice leaked into model context:\n{rendered}"
    );
    // Consumed steering record IS model-visible — must appear exactly once.
    let steering_occurrences = rendered.matches("switch to mode: cautious").count();
    assert_eq!(
        steering_occurrences, 1,
        "consumed steering should appear exactly once, got {steering_occurrences}:\n{rendered}"
    );
}

#[test]
fn message_with_parts_filters_hidden_runtime_hints() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("record the runtime hint but keep prompt clean");

    maybe_append_proposal_notice(
        &mut session,
        &NudgeDecision::Triggered {
            promoted: 0,
            merged: 0,
            archived: 0,
            promoted_records: 0,
            proposals_created: 1,
            proposals_skipped: 0,
        },
    );

    let converted = SessionPrompt::to_message_with_parts(&session.messages, "mock", "m", ".");
    assert_eq!(
        converted.len(),
        1,
        "runtime hint notice should stay out of model context"
    );
}

#[test]
fn ingress_metadata_is_hidden_from_model_prompt_surface() {
    let mut session = Session::new("proj", ".");
    let message = session.add_user_message("only parts text is visible");
    message.metadata.insert(
        "ingress_source".to_string(),
        serde_json::json!(IngressSource::Web),
    );
    message.metadata.insert(
        "ingress_stabilization".to_string(),
        serde_json::json!({
            "batch_count": 1,
            "dedupe_keys": [],
            "ordering_key": "turn_1",
            "policy": INGRESS_POLICY_ENTRY_METADATA_ONLY,
        }),
    );
    message.metadata.insert(
        "ingress_context_key".to_string(),
        serde_json::json!("session_prompt"),
    );

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None, &[])
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            agendao_provider::Content::Text(text) => text.clone(),
            agendao_provider::Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(rendered, "only parts text is visible");
    assert!(!rendered.contains("ingress_source"), "{rendered}");
    assert!(
        !rendered.contains(INGRESS_POLICY_ENTRY_METADATA_ONLY),
        "{rendered}"
    );
    assert!(!rendered.contains("session_prompt"), "{rendered}");
}

#[tokio::test]
async fn proposal_generation_syncs_positive_evolution_evidence_to_skill_governance() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".agendao/skills/provider-refresh"))
        .expect("skill dir should exist");
    fs::write(
        dir.path().join(".agendao/skills/provider-refresh/SKILL.md"),
        r#"---
name: provider-refresh
description: refresh provider
---
Use for provider refresh tasks.
"#,
    )
    .expect("skill file should exist");

    let (prompt, proposal_repo) = prompt_with_memory_and_proposals(dir.path()).await;
    let record = methodology_candidate_record(
        "mem_provider_refresh",
        "ses_skill_nudge",
        "ws:test",
        "provider-refresh",
    );
    let candidates = vec![record];

    let summary = agendao_storage::generate_skill_evolution_proposals(
        proposal_repo.as_ref(),
        &candidates,
        "ses_skill_nudge",
    )
    .await
    .expect("proposal generation should succeed");
    prompt.sync_skill_memory_promotion_evidence(
        dir.path().to_str(),
        "ses_skill_nudge",
        &candidates,
    );
    prompt
        .sync_skill_proposal_evidence(
            dir.path().to_str(),
            "ses_skill_nudge",
            proposal_repo.as_ref(),
            &linked_methodology_skill_names(&candidates),
        )
        .await;

    assert_eq!(summary.proposals_created, 1);
    assert_eq!(summary.proposals_skipped, 0);
    assert_eq!(
        proposal_repo
            .list_by_status(&ProposalStatus::Draft)
            .await
            .expect("draft proposals should list")
            .len(),
        1
    );

    let governance = SkillGovernanceAuthority::new(dir.path(), None);
    let snapshot = governance
        .skill_operational_snapshots()
        .into_iter()
        .find(|entry| entry.skill_name == "provider-refresh")
        .expect("operational snapshot should exist");
    let evolution = snapshot
        .evolution
        .expect("positive evolution evidence should be recorded");
    assert_eq!(evolution.memory_promotion_count, 1);
    assert_eq!(evolution.proposal_signal_count, 1);
    assert_eq!(evolution.last_observed_draft_proposal_count, 1);
    assert!(evolution.last_memory_promotion_at.is_some());
    assert!(evolution.last_proposal_at.is_some());
    assert!(evolution.last_positive_signal_at.is_some());
}

#[tokio::test]
async fn proposal_generation_retargets_specialization_to_canonical_skill() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".agendao/skills/provider-refresh"))
        .expect("canonical skill dir should exist");
    fs::write(
        dir.path().join(".agendao/skills/provider-refresh/SKILL.md"),
        r#"---
name: provider-refresh
description: refresh provider
---
Use for provider refresh tasks.
"#,
    )
    .expect("canonical skill file should exist");
    fs::create_dir_all(dir.path().join(".agendao/skills/provider-refresh-gitlab"))
        .expect("specialization skill dir should exist");
    fs::write(
        dir.path()
            .join(".agendao/skills/provider-refresh-gitlab/SKILL.md"),
        r#"---
name: provider-refresh-gitlab
description: refresh gitlab provider
---
Use for GitLab-specific provider refresh tasks.
"#,
    )
    .expect("specialization skill file should exist");

    let governance = SkillGovernanceAuthority::new(dir.path(), None);
    governance
        .activate_skill_capability_group(
            Some("provider-refresh-family"),
            SkillCapabilityGroupKind::CanonicalFamily,
            Some("provider-refresh"),
            vec![
                SkillCapabilityMember {
                    skill_name: "provider-refresh".to_string(),
                    role: SkillCapabilityMemberRole::Canonical,
                },
                SkillCapabilityMember {
                    skill_name: "provider-refresh-gitlab".to_string(),
                    role: SkillCapabilityMemberRole::Specialization,
                },
            ],
            vec!["GitLab refresh is governed under the shared provider refresh skill".to_string()],
            "test:activate-group",
        )
        .expect("capability group should activate");

    let (prompt, proposal_repo) = prompt_with_memory_and_proposals(dir.path()).await;
    let candidates = vec![methodology_candidate_record(
        "mem_provider_refresh_gitlab",
        "ses_skill_nudge",
        "ws:test",
        "provider-refresh-gitlab",
    )];
    let proposal_candidates = prompt.retarget_methodology_candidates_for_composition(
        dir.path().to_str(),
        "ses_skill_nudge",
        &candidates,
    );
    assert_eq!(
        proposal_candidates[0].linked_skill_name.as_deref(),
        Some("provider-refresh")
    );

    let summary = agendao_storage::generate_skill_evolution_proposals(
        proposal_repo.as_ref(),
        &proposal_candidates,
        "ses_skill_nudge",
    )
    .await
    .expect("proposal generation should succeed");
    prompt
        .sync_skill_proposal_evidence(
            dir.path().to_str(),
            "ses_skill_nudge",
            proposal_repo.as_ref(),
            &linked_methodology_skill_names(&proposal_candidates),
        )
        .await;

    assert_eq!(summary.proposals_created, 1);
    let drafts = proposal_repo
        .list_by_status(&ProposalStatus::Draft)
        .await
        .expect("draft proposals should list");
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        drafts[0].linked_skill_name.as_deref(),
        Some("provider-refresh")
    );

    let canonical_snapshot = SkillGovernanceAuthority::new(dir.path(), None)
        .skill_operational_snapshots()
        .into_iter()
        .find(|entry| entry.skill_name == "provider-refresh")
        .expect("canonical snapshot should exist");
    assert_eq!(
        canonical_snapshot
            .evolution
            .as_ref()
            .map(|entry| entry.proposal_signal_count),
        Some(1)
    );
}

#[test]
fn review_nudge_scope_isolated_by_workspace_and_inflight_state() {
    let prompt = SessionPrompt::default();
    let now = tokio::time::Instant::now();
    let cooldown = core::time::Duration::from_secs(600);

    assert_eq!(
        prompt.try_begin_review_nudge_scope("directory:/repo-a", now, cooldown),
        Ok(())
    );
    assert_eq!(
        prompt.try_begin_review_nudge_scope("directory:/repo-a", now, cooldown),
        Err(SkippedReason::ReviewInFlight)
    );
    assert_eq!(
        prompt.try_begin_review_nudge_scope("directory:/repo-b", now, cooldown),
        Ok(())
    );

    prompt.finish_review_nudge_scope("directory:/repo-a", Some(now));
    prompt.finish_review_nudge_scope("directory:/repo-b", None);
}

#[test]
fn review_nudge_failure_does_not_burn_cooldown_but_success_does() {
    let prompt = SessionPrompt::default();
    let now = tokio::time::Instant::now();
    let cooldown = core::time::Duration::from_secs(600);
    let scope = "directory:/repo-a";

    assert_eq!(
        prompt.try_begin_review_nudge_scope(scope, now, cooldown),
        Ok(())
    );
    prompt.finish_review_nudge_scope(scope, None);
    assert_eq!(
        prompt.try_begin_review_nudge_scope(scope, now, cooldown),
        Ok(())
    );

    prompt.finish_review_nudge_scope(scope, Some(now));
    assert_eq!(
        prompt.try_begin_review_nudge_scope(
            scope,
            now + core::time::Duration::from_secs(1),
            cooldown
        ),
        Err(SkippedReason::CooldownActive)
    );
    assert_eq!(
        prompt.try_begin_review_nudge_scope(
            scope,
            now + cooldown + core::time::Duration::from_secs(1),
            cooldown
        ),
        Ok(())
    );
}

// ── P1.1 Commit 5: old-path / new-view equivalence guards ────────────
//
// IMPORTANT: these tests compare the new view rendering against a
// FROZEN BASELINE (inline reimplementation of the original logic),
// NOT against the now-delegated SessionPrompt method.  This ensures
// the equivalence test itself doesn't become self-proving after
// SessionPrompt::render_memory_prefetch_reminder() delegates to the
// view.

mod reflow_equivalence_tests {
    use super::super::reflow_context::PromptReflowContext;
    use super::super::{
        render_session_reflow_diagnostics_summary, REQUEST_BOUNDARY_HYGIENE_SUMMARY_METADATA_KEY,
    };
    use agendao_types::{
        RequestBoundaryHygieneActionKind, RequestBoundaryHygieneActionSummary,
        RequestBoundaryHygieneSummary, Session as SessionRecord, SessionContinuityPacket,
        SessionContinuityTurn, SessionStatus, SessionTime, SessionUsage,
    };
    use chrono::Utc;
    use std::collections::HashMap;

    #[test]
    fn continuity_context_build_preserves_allowed_ids_semantics() {
        let packet = SessionContinuityPacket {
            version: 1,
            eligible_message_count: 3,
            exact_recent_tail_count: 2,
            omitted_older_turns: 1,
            exact_recent_tail: vec![
                SessionContinuityTurn {
                    message_id: "msg-a".to_string(),
                    role: "user".to_string(),
                    text: "question".to_string(),
                    projected: false,
                },
                SessionContinuityTurn {
                    message_id: "msg-b".to_string(),
                    role: "assistant".to_string(),
                    text: "answer".to_string(),
                    projected: false,
                },
            ],
            memory_anchors: vec![],
            working_ledger: vec![],
            task_ledger: None,
            continuation_dependencies: vec![],
            latest_compaction_summary: None,
            limits: None,
            recall_policy: None,
        };

        let view =
            PromptReflowContext::build("ses-ct", None, Some(&packet), false, false, None, None)
                .continuity
                .expect("continuity view should exist");

        // The view's hydrate_message_ids must match the packet's allowed_message_ids.
        assert_eq!(view.hydrate_message_ids, packet.allowed_message_ids());
        // View fields must match packet fields.
        assert_eq!(view.eligible_message_count, 3);
        assert_eq!(view.exact_recent_tail_count, 2);
        assert_eq!(view.omitted_older_turns, 1);
        assert!(!view.has_continuation_dependency);
        assert!(view.compaction_summary.is_none());
    }

    #[test]
    fn session_reflow_diagnostics_summary_surfaces_hygiene_metadata() {
        let now = Utc::now();
        let mut session = crate::Session::from(SessionRecord {
            id: "session-reflow".to_string(),
            slug: "session-reflow".to_string(),
            project_id: "project".to_string(),
            directory: "/tmp".to_string(),
            parent_id: None,
            title: "Reflow".to_string(),
            version: "1".to_string(),
            time: SessionTime {
                created: now.timestamp_millis(),
                updated: now.timestamp_millis(),
                compacting: None,
                archived: None,
            },
            messages: Vec::new(),
            summary: None,
            share: None,
            revert: None,
            permission: None,
            usage: Some(SessionUsage::default()),
            status: SessionStatus::Active,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
        });
        let hygiene = RequestBoundaryHygieneSummary {
            dropped_orphan_tool_results: 1,
            dropped_dangling_tool_calls: 0,
            compressed_tool_results: 1,
            actions: vec![
                RequestBoundaryHygieneActionSummary {
                    kind: RequestBoundaryHygieneActionKind::DroppedOrphanToolResult,
                    tool_call_id: "call-orphan".to_string(),
                    tool_name: None,
                    original_chars: Some(17),
                },
                RequestBoundaryHygieneActionSummary {
                    kind: RequestBoundaryHygieneActionKind::CompressedToolResult,
                    tool_call_id: "call-long".to_string(),
                    tool_name: Some("grep".to_string()),
                    original_chars: Some(24_001),
                },
            ],
        };
        session.insert_metadata(
            REQUEST_BOUNDARY_HYGIENE_SUMMARY_METADATA_KEY,
            serde_json::to_value(&hygiene).expect("hygiene metadata should serialize"),
        );

        let summary = render_session_reflow_diagnostics_summary(&session)
            .expect("reflow diagnostics summary should render");

        assert!(summary.contains("request_boundary_hygiene: dropped_orphan_tool_results=1"));
        assert!(summary.contains("compressed_tool_results=1"));
        assert!(summary.contains("dropped_orphan_tool_result: call_id=call-orphan"));
        assert!(summary
            .contains("compressed_tool_result: call_id=call-long tool=grep original_chars=24001"));
    }
}

// ── P1.1 Commit 2: memory-reflow invariant guards ──────────────────────

mod memory_prefetch_tests {}
