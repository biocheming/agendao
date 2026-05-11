use super::skill_reflection::{
    augment_system_prompt_with_skill_reflection, extract_tool_call_history,
    prepare_skill_reflection, update_skill_reflection_metadata, SkillReflectionData,
    SkillUsageSummary, ToolCallSummary,
};
use super::*;
use crate::message::MessagePart;
use crate::SessionMessage;
use async_trait::async_trait;
use futures::stream;
use rocode_config::ConfigStore;
use rocode_execution_types::CompiledExecutionRequest;
use rocode_provider::{
    ChatRequest, ChatResponse, ModelInfo, ProviderError, StreamEvent, StreamResult, StreamUsage,
};
use rocode_skill::{RuntimeInstructionSource, SkillGovernanceAuthority};
use rocode_storage::{Database, SkillEvolutionProposalRepository};
use rocode_tool::{Tool, ToolContext, ToolError, ToolResult};
use rocode_types::{
    ContextPressureGovernanceStatus, LightweightTrimSummary, MemoryEvidenceRef, MemoryKind,
    MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus, MemoryValidationStatus,
    ProposalStatus, SkillCapabilityGroupKind, SkillCapabilityMember, SkillCapabilityMemberRole,
    SkillRetirementReason, SkillRetirementReasonKind, SkillVitalityState,
};
use std::fs;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tempfile::tempdir;

struct StaticModelProvider {
    model: Option<ModelInfo>,
}

impl StaticModelProvider {
    fn with_model(model_id: &str, context_window: u64, max_output_tokens: u64) -> Self {
        Self {
            model: Some(ModelInfo {
                id: model_id.to_string(),
                name: "Static Model".to_string(),
                provider: "mock".to_string(),
                context_window,
                max_input_tokens: None,
                max_output_tokens,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
                cost_per_million_cache_read: None,
                cost_per_million_cache_write: None,
            }),
        }
    }
}

#[async_trait]
impl Provider for StaticModelProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.model.clone().into_iter().collect()
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.model.as_ref().filter(|model| model.id == id)
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        Ok(Box::pin(stream::empty()))
    }
}

struct ScriptedStreamProvider {
    model: ModelInfo,
    events: Vec<StreamEvent>,
}

fn write_methodology_skill(
    root: &std::path::Path,
    name: &str,
    template: rocode_skill::SkillMethodologyTemplate,
) {
    let skill_dir = root.join(".rocode/skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let body = rocode_skill::render_methodology_skill_body(name, &template).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: test skill\n---\n\n{body}\n",),
    )
    .unwrap();
}

async fn prompt_with_memory_and_proposals(
    root: &std::path::Path,
) -> (SessionPrompt, Arc<SkillEvolutionProposalRepository>) {
    let config_store =
        Arc::new(ConfigStore::from_project_dir(root).expect("project config store should load"));
    let db = Database::in_memory().await.expect("db should initialize");
    let proposal_repo = Arc::new(SkillEvolutionProposalRepository::new(db.pool().clone()));
    let prompt = SessionPrompt::default()
        .with_config_store(config_store)
        .with_proposal_repo(proposal_repo.clone());
    (prompt, proposal_repo)
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

#[async_trait]
impl Provider for ScriptedStreamProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.model.clone()]
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        if self.model.id == id {
            Some(&self.model)
        } else {
            None
        }
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        Ok(Box::pin(stream::iter(
            self.events
                .clone()
                .into_iter()
                .map(Result::<StreamEvent, ProviderError>::Ok),
        )))
    }
}

#[test]
fn pre_dispatch_governance_is_ready_when_context_is_within_budget() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    session.add_user_message("short request");

    let outcome = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(24),
        Some(96),
        Some("short request"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    let ContextPressureGovernanceOutcome::Proceed(summary) = outcome else {
        panic!("governance should proceed when context is within budget");
    };
    assert_eq!(summary.status, ContextPressureGovernanceStatus::Ready);
    assert!(!summary.compaction_attempted);
    assert!(!summary.blocking);
}

#[test]
fn pre_dispatch_governance_forces_compaction_for_small_overflowing_request_view() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    session.add_user_message("first");
    session.add_user_message("second");

    let outcome = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(120),
        Some(480),
        Some("second"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    let ContextPressureGovernanceOutcome::Proceed(summary) = outcome else {
        panic!("governance should compact and proceed for small overflowing request views");
    };
    assert_eq!(summary.status, ContextPressureGovernanceStatus::Compacted);
    assert_eq!(summary.reason.as_deref(), Some("request_view_overflow"));
    assert!(summary.compaction_attempted);
    assert!(summary.compaction_succeeded);
    assert!(!summary.blocking);

    let persisted = session
        .record()
        .metadata
        .get(CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("summary should persist into session metadata");
    let persisted: rocode_types::ContextPressureGovernanceSummary =
        serde_json::from_value(persisted).expect("persisted summary should parse");
    assert_eq!(persisted.status, ContextPressureGovernanceStatus::Compacted);
    let lifecycle = session
        .record()
        .metadata
        .get(CONTEXT_COMPACTION_LIFECYCLE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("compaction lifecycle should persist into session metadata");
    let lifecycle: rocode_types::ContextCompactionLifecycleSummary =
        serde_json::from_value(lifecycle).expect("persisted lifecycle should parse");
    assert_eq!(
        lifecycle.status,
        rocode_types::ContextCompactionLifecycleStatus::Installed
    );
    let installed = lifecycle
        .installed
        .expect("installed diagnostics should be recorded");
    assert!(installed
        .request_context_tokens
        .is_some_and(|value| value < 120));
    assert!(installed
        .live_context_tokens
        .is_some_and(|value| value < 120));
    assert!(installed.body_chars.is_some_and(|value| value < 480));
    assert!(installed
        .cache_explanation
        .as_deref()
        .is_some_and(|value| value.starts_with("boundary recorded")));
    let compaction_message = session
        .record()
        .messages
        .last()
        .expect("compaction message should be appended");
    let diagnostics = compaction_message
        .metadata
        .get(CONTEXT_COMPACTION_RECORD_METADATA_KEY)
        .expect("compaction diagnostics should be present");
    assert_eq!(diagnostics["forced"], serde_json::json!(true));
}

#[test]
fn pre_dispatch_governance_persists_lightweight_trim_summary_when_trim_succeeds() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    let session_id = session.id.clone();

    session
        .messages_mut()
        .push(SessionMessage::user(session_id.clone(), "earlier user"));
    let mut assistant = SessionMessage::assistant(session_id.clone());
    assistant.add_tool_call(
        "call_round",
        "write_file",
        serde_json::json!({"path": "src/main.rs", "content": "Q".repeat(30_000)}),
    );
    session.messages_mut().push(assistant);
    let mut tool = SessionMessage::tool(session_id.clone());
    tool.add_tool_result("call_round", &"R".repeat(20_000), false);
    session.messages_mut().push(tool);
    session
        .messages_mut()
        .push(SessionMessage::user(session_id.clone(), "latest user"));

    let outcome = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(120),
        Some(480),
        Some("latest user"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    let ContextPressureGovernanceOutcome::Proceed(summary) = outcome else {
        panic!("governance should proceed after lightweight trim");
    };
    assert_eq!(summary.status, ContextPressureGovernanceStatus::Compacted);
    assert_eq!(
        summary.reason.as_deref(),
        Some("lightweight_tool_result_trim")
    );
    let trim = summary
        .lightweight_trim
        .as_ref()
        .expect("lightweight trim summary should be attached");
    assert_eq!(trim.trimmed_rounds, 1);
    assert!(trim.used_round_grouping);
    let trace = summary
        .decision_trace
        .as_ref()
        .expect("decision trace should be attached");
    assert_eq!(trace.mode, "lightweight_trim");
    assert!(trace.lightweight_trim.is_some());

    let persisted = session
        .record()
        .metadata
        .get(CONTEXT_LIGHTWEIGHT_TRIM_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("trim summary should persist into session metadata");
    let persisted: LightweightTrimSummary =
        serde_json::from_value(persisted).expect("persisted trim summary should parse");
    assert_eq!(persisted.trimmed_rounds, 1);
    assert!(persisted.used_round_grouping);
}

#[test]
fn pre_dispatch_governance_records_auto_compaction_backoff_reason() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    let session_id = session.id.clone();

    session
        .messages_mut()
        .push(SessionMessage::user(session_id.clone(), "before compact"));
    let mut compacted = SessionMessage::assistant(session_id.clone());
    compacted.parts.push(MessagePart {
        id: "prt_compact".to_string(),
        part_type: PartType::Compaction {
            summary: "older context compacted".to_string(),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    session.messages_mut().push(compacted);
    session
        .messages_mut()
        .push(SessionMessage::user(session_id.clone(), "after compact"));

    let outcome = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(120),
        Some(480),
        Some("after compact"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    let ContextPressureGovernanceOutcome::Proceed(summary) = outcome else {
        panic!("governance should defer when auto compaction backs off");
    };
    assert_eq!(summary.status, ContextPressureGovernanceStatus::Deferred);
    assert_eq!(summary.reason.as_deref(), Some("auto_compaction_backoff"));
    let trace = summary
        .decision_trace
        .as_ref()
        .expect("decision trace should be recorded");
    assert_eq!(trace.mode, "auto_compaction_backoff");
    let backoff = trace
        .backoff
        .as_ref()
        .expect("backoff details should be recorded");
    assert_eq!(backoff.messages_since_last, 1);
    assert_eq!(backoff.user_turns_since_last, 1);
    assert_eq!(backoff.min_messages_after_last, 4);
}

#[test]
fn pre_dispatch_governance_clears_stale_lightweight_trim_summary_when_no_trim_occurs() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    session.insert_metadata(
        CONTEXT_LIGHTWEIGHT_TRIM_SUMMARY_METADATA_KEY.to_string(),
        serde_json::to_value(LightweightTrimSummary {
            trimmed_rounds: 9,
            trimmed_tool_calls: 9,
            trimmed_tool_results: 9,
            trimmed_call_tokens: 999,
            trimmed_result_tokens: 999,
            used_round_grouping: true,
        })
        .expect("summary serializes"),
    );
    session.add_user_message("only message");

    let _ = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(40),
        Some(120),
        Some("only message"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    assert!(
        session
            .record()
            .metadata
            .get(CONTEXT_LIGHTWEIGHT_TRIM_SUMMARY_METADATA_KEY)
            .is_none(),
        "stale trim summary should be removed when no trim occurs"
    );
}

#[test]
fn pre_dispatch_governance_blocks_when_single_message_remains_over_limit() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    session.add_user_message("only message");

    let outcome = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(120),
        Some(480),
        Some("only message"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    let ContextPressureGovernanceOutcome::Blocked(summary) = outcome else {
        panic!("governance should block when there is no compactable history");
    };
    assert_eq!(summary.status, ContextPressureGovernanceStatus::Blocked);
    assert_eq!(summary.reason.as_deref(), Some("request_view_overflow"));
    assert!(summary.compaction_attempted);
    assert!(!summary.compaction_succeeded);
    assert!(summary.blocking);
    let lifecycle = session
        .record()
        .metadata
        .get(CONTEXT_COMPACTION_LIFECYCLE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("compaction lifecycle should persist into session metadata");
    let lifecycle: rocode_types::ContextCompactionLifecycleSummary =
        serde_json::from_value(lifecycle).expect("persisted lifecycle should parse");
    assert_eq!(
        lifecycle.status,
        rocode_types::ContextCompactionLifecycleStatus::Failed
    );
}

#[test]
fn pre_dispatch_governance_records_compaction_when_reduction_succeeds() {
    let provider = StaticModelProvider::with_model("ctx-model", 100, 20);
    let mut session = Session::new("proj", ".");
    session.insert_metadata("model_provider", serde_json::json!("mock"));
    session.insert_metadata("model_id", serde_json::json!("ctx-model"));
    for index in 0..10 {
        session.add_user_message(format!("message {index}"));
    }

    let outcome = govern_pre_dispatch_session_context(
        &mut session,
        &provider,
        "ctx-model",
        Some(20),
        None,
        None,
        Some(95),
        Some(380),
        Some("message 9"),
        "pre_dispatch_hard_gate",
        "scheduler.pre_dispatch",
        None,
        None,
    );

    let ContextPressureGovernanceOutcome::Proceed(summary) = outcome else {
        panic!("compaction should allow the dispatch to proceed");
    };
    assert_eq!(summary.status, ContextPressureGovernanceStatus::Compacted);
    assert!(summary.compaction_attempted);
    assert!(summary.compaction_succeeded);
    assert!(!summary.blocking);
    let lifecycle = session
        .record()
        .metadata
        .get(CONTEXT_COMPACTION_LIFECYCLE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("compaction lifecycle should persist into session metadata");
    let lifecycle: rocode_types::ContextCompactionLifecycleSummary =
        serde_json::from_value(lifecycle).expect("persisted lifecycle should parse");
    assert_eq!(
        lifecycle.status,
        rocode_types::ContextCompactionLifecycleStatus::Installed
    );
    let installed = lifecycle
        .installed
        .expect("installed diagnostics should be recorded");
    assert!(installed.request_context_tokens.is_some());
    assert!(installed.live_context_tokens.is_some());
    assert!(installed.body_chars.is_some());
    assert!(installed
        .cache_explanation
        .as_deref()
        .is_some_and(|value| value.contains("boundary recorded")));
}

struct MultiTurnScriptedProvider {
    model: ModelInfo,
    turns: Arc<StdMutex<std::collections::VecDeque<Vec<StreamEvent>>>>,
    request_count: Arc<StdMutex<usize>>,
}

impl MultiTurnScriptedProvider {
    fn new(model: ModelInfo, turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            model,
            turns: Arc::new(StdMutex::new(turns.into())),
            request_count: Arc::new(StdMutex::new(0)),
        }
    }
}

#[async_trait]
impl Provider for MultiTurnScriptedProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.model.clone()]
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        if self.model.id == id {
            Some(&self.model)
        } else {
            None
        }
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        {
            let mut count = self
                .request_count
                .lock()
                .expect("request_count lock should not poison");
            *count += 1;
        }

        let events = self
            .turns
            .lock()
            .expect("turns lock should not poison")
            .pop_front()
            .ok_or_else(|| {
                ProviderError::InvalidRequest(
                    "no scripted response left for chat_stream".to_string(),
                )
            })?;

        Ok(Box::pin(stream::iter(
            events
                .into_iter()
                .map(Result::<StreamEvent, ProviderError>::Ok),
        )))
    }
}

struct NoArgEchoTool;

#[async_trait]
impl Tool for NoArgEchoTool {
    fn id(&self) -> &str {
        "noarg_echo"
    }

    fn description(&self) -> &str {
        "Echoes input for tests"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::simple("NoArg Echo", args.to_string()))
    }
}

struct AlwaysInvalidArgsTool;

#[async_trait]
impl Tool for AlwaysInvalidArgsTool {
    fn id(&self) -> &str {
        "needs_path"
    }

    fn description(&self) -> &str {
        "Fails validation for tests"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filePath": { "type": "string" }
            },
            "required": ["filePath"]
        })
    }

    fn validate(&self, _args: &serde_json::Value) -> Result<(), ToolError> {
        Err(ToolError::InvalidArguments(
            "filePath is required".to_string(),
        ))
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionError(
            "validate should prevent execute".to_string(),
        ))
    }
}
#[test]
fn insert_reminders_adds_plan_prompt_for_plan_agent() {
    let messages = vec![SessionMessage::user("ses_test", "plan this")];
    let output = insert_reminders(&messages, "plan", false);
    let last = output.last().unwrap();
    let injected = last
        .parts
        .iter()
        .filter_map(|p| match &p.part_type {
            PartType::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(injected.contains("You are in PLAN mode"));
}

#[test]
fn insert_reminders_adds_build_switch_after_plan() {
    let mut user = SessionMessage::user("ses_test", "execute this");
    user.metadata
        .insert("agent".to_string(), serde_json::json!("plan"));
    let output = insert_reminders(&[user], "build", true);
    let last = output.last().unwrap();
    let injected = last
        .parts
        .iter()
        .filter_map(|p| match &p.part_type {
            PartType::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(injected.contains("The user has approved your plan"));
}

#[test]
fn provider_error_summary_from_anyhow_reads_wrapped_prompt_error() {
    let summary = rocode_provider::ProviderErrorSummary {
        kind: rocode_provider::ProviderErrorKind::InvalidRequest,
        provider_id: "deepseek".to_string(),
        model_id: Some("deepseek-reasoner".to_string()),
        message: "missing replay".to_string(),
        status_code: Some(400),
        standard_code: rocode_provider::error_code::StandardErrorCode::InvalidRequest,
        retryable: false,
        provider_diagnostic: Some(rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::RequestValidation,
            code: "thinking_replay_missing".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-reasoner".to_string()),
            message: "missing replay".to_string(),
        }),
    };
    let error = anyhow::Error::new(PromptError::ProviderFailure(
        rocode_orchestrator::runtime::events::ModelFailure::Provider(summary.clone()),
    ))
    .context("session prompt failed");

    let loaded = provider_error_summary_from_anyhow(&error).expect("typed summary should load");

    assert_eq!(loaded, summary);
}

#[test]
fn provider_failure_from_anyhow_reads_wrapped_untyped_provider_message() {
    let error = anyhow::Error::new(PromptError::ProviderFailure(
        rocode_orchestrator::runtime::events::ModelFailure::Message(
            "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request"
                .to_string(),
        ),
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
async fn prompt_with_update_hook_emits_incremental_snapshots() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let provider = Arc::new(ScriptedStreamProvider {
        model: ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            provider: "mock".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 1024,
            supports_vision: false,
            supports_tools: false,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        },
        events: vec![
            StreamEvent::Start,
            StreamEvent::TextDelta("Hel".to_string()),
            StreamEvent::TextDelta("lo".to_string()),
            StreamEvent::FinishStep {
                finish_reason: Some("stop".to_string()),
                usage: StreamUsage {
                    prompt_tokens: 3,
                    completion_tokens: 2,
                    ..Default::default()
                },
                provider_metadata: None,
            },
            StreamEvent::Done,
        ],
    });

    let snapshots = Arc::new(StdMutex::new(Vec::<Session>::new()));
    let snapshot_sink = snapshots.clone();
    let hook: SessionUpdateHook = Arc::new(move |snapshot| {
        snapshot_sink
            .lock()
            .expect("snapshot lock should not poison")
            .push(snapshot.clone());
    });

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: Some(ModelRef {
            provider_id: "mock".to_string(),
            model_id: "test-model".to_string(),
        }),
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "Say hello".to_string(),
        }],
        tools: None,
        ingress: None,
    };

    prompt
        .prompt_with_update_hook(
            input,
            &mut session,
            PromptRequestContext {
                provider,
                system_prompt: None,
                memory_prefetch: None,
                tools: Vec::new(),
                tool_source_digests: Vec::new(),
                compiled_request: CompiledExecutionRequest::default(),
                hooks: PromptHooks {
                    update_hook: Some(hook),
                    ..Default::default()
                },
            },
        )
        .await
        .expect("prompt_with_update_hook should succeed");

    let snapshots_guard = snapshots.lock().expect("snapshot lock should not poison");
    assert!(snapshots_guard.len() >= 3);
    let saw_partial = snapshots_guard.iter().any(|snap| {
        snap.messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .map(|m| m.get_text() == "Hel")
            .unwrap_or(false)
    });
    assert!(
        saw_partial,
        "expected at least one streamed partial assistant snapshot"
    );
    drop(snapshots_guard);

    let final_text = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(SessionMessage::get_text)
        .unwrap_or_default();
    assert_eq!(final_text, "Hello");
}

#[tokio::test]
async fn prompt_ignores_duplicate_ingress_idempotency_key() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let scripted = MultiTurnScriptedProvider::new(
        ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            provider: "mock".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 1024,
            supports_vision: false,
            supports_tools: false,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        },
        vec![
            vec![
                StreamEvent::Start,
                StreamEvent::TextDelta("ok".to_string()),
                StreamEvent::FinishStep {
                    finish_reason: Some("stop".to_string()),
                    usage: StreamUsage::default(),
                    provider_metadata: None,
                },
                StreamEvent::Done,
            ],
            vec![StreamEvent::Start, StreamEvent::Done],
        ],
    );
    let request_count = scripted.request_count.clone();
    let provider: Arc<dyn Provider> = Arc::new(scripted);
    let mut ingress = IngressTurnEnvelope::new_text(
        session.id.clone(),
        IngressSource::Web,
        "turn_1",
        100,
        "Say hello",
    );
    ingress.idempotency_key = Some("idem_1".to_string());

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: Some(ModelRef {
            provider_id: "mock".to_string(),
            model_id: "test-model".to_string(),
        }),
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "Say hello".to_string(),
        }],
        tools: None,
        ingress: Some(ingress),
    };

    for _ in 0..2 {
        prompt
            .prompt_with_update_hook(
                input.clone(),
                &mut session,
                PromptRequestContext {
                    provider: provider.clone(),
                    system_prompt: None,
                    memory_prefetch: None,
                    tools: Vec::new(),
                    tool_source_digests: Vec::new(),
                    compiled_request: CompiledExecutionRequest::default(),
                    hooks: PromptHooks::default(),
                },
            )
            .await
            .expect("duplicate ingress should be accepted as a no-op");
    }

    let request_count = *request_count
        .lock()
        .expect("request_count lock should not poison");
    assert_eq!(request_count, 1);
    assert_eq!(
        session
            .messages
            .iter()
            .filter(|message| matches!(message.role, MessageRole::User))
            .count(),
        1
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

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None)
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            rocode_provider::Content::Text(text) => text.clone(),
            rocode_provider::Content::Parts(parts) => parts
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
async fn prompt_continues_after_tool_calls_without_finish_step_reason() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let temp_dir = tempfile::tempdir().expect("tempdir should create");
    let file_path = temp_dir.path().join("sample.txt");
    tokio::fs::write(&file_path, "alpha\nbeta")
        .await
        .expect("file should write");
    let file_path = file_path.to_string_lossy().to_string();

    let scripted = MultiTurnScriptedProvider::new(
        ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            provider: "mock".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 1024,
            supports_vision: false,
            supports_tools: true,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        },
        vec![
            vec![
                StreamEvent::Start,
                StreamEvent::ToolCallStart {
                    id: "tool-call-0".to_string(),
                    name: "read".to_string(),
                },
                StreamEvent::ToolCallEnd {
                    id: "tool-call-0".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({ "file_path": file_path }),
                },
                StreamEvent::Done,
            ],
            vec![
                StreamEvent::Start,
                StreamEvent::TextDelta("Read complete".to_string()),
                StreamEvent::FinishStep {
                    finish_reason: Some("stop".to_string()),
                    usage: StreamUsage::default(),
                    provider_metadata: None,
                },
                StreamEvent::Done,
            ],
        ],
    );
    let request_count = scripted.request_count.clone();
    let provider: Arc<dyn Provider> = Arc::new(scripted);

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: Some(ModelRef {
            provider_id: "mock".to_string(),
            model_id: "test-model".to_string(),
        }),
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "Read the file and summarize".to_string(),
        }],
        tools: None,
        ingress: None,
    };

    prompt
        .prompt_with_update_hook(
            input,
            &mut session,
            PromptRequestContext {
                provider,
                system_prompt: None,
                memory_prefetch: None,
                tools: Vec::new(),
                tool_source_digests: Vec::new(),
                compiled_request: CompiledExecutionRequest::default(),
                hooks: PromptHooks::default(),
            },
        )
        .await
        .expect("prompt_with_update_hook should succeed");

    let request_count = *request_count
        .lock()
        .expect("request_count lock should not poison");
    assert_eq!(request_count, 2, "expected a second model round");

    let final_text = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(SessionMessage::get_text)
        .unwrap_or_default();
    assert_eq!(final_text, "Read complete");
}

#[tokio::test]
async fn create_user_message_persists_pending_subtask_payload() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        tools: None,
        ingress: None,
        parts: vec![PartInput::Subtask {
            prompt: "Inspect codegen path".to_string(),
            description: Some("Inspect codegen".to_string()),
            agent: "explore".to_string(),
        }],
    };

    prompt
        .create_user_message(&input, &mut session)
        .await
        .expect("create_user_message should succeed");

    let msg = session.messages.last().expect("user message should exist");
    let pending = msg
        .metadata
        .get("pending_subtasks")
        .and_then(|v| v.as_array())
        .expect("pending_subtasks metadata should exist");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].get("agent").and_then(|v| v.as_str()),
        Some("explore")
    );
    assert_eq!(
        pending[0].get("prompt").and_then(|v| v.as_str()),
        Some("Inspect codegen path")
    );
    assert!(msg.parts.iter().any(|p| match &p.part_type {
        PartType::Subtask { status, .. } => status == "pending",
        _ => false,
    }));
}
#[test]
fn shell_exec_uses_zsh_login_invocation() {
    let invocation = resolve_shell_invocation(Some("/bin/zsh"), "echo hello");
    assert_eq!(invocation.program, "/bin/zsh");
    assert_eq!(invocation.args[0], "-c");
    assert_eq!(invocation.args[1], "-l");
    assert!(invocation.args[2].contains(".zshenv"));
    assert!(invocation.args[2].contains("eval"));
}

#[test]
fn shell_exec_uses_bash_login_invocation() {
    let invocation = resolve_shell_invocation(Some("/bin/bash"), "echo hello");
    assert_eq!(invocation.program, "/bin/bash");
    assert_eq!(invocation.args[0], "-c");
    assert_eq!(invocation.args[1], "-l");
    assert!(invocation.args[2].contains("shopt -s expand_aliases"));
    assert!(invocation.args[2].contains(".bashrc"));
}

#[tokio::test]
async fn resolve_tools_with_mcp_registry_includes_mcp_tools() {
    let tool_registry = rocode_tool::create_default_registry().await;
    let mcp_registry = rocode_mcp::McpToolRegistry::new();
    mcp_registry
        .register(rocode_mcp::McpTool::new(
            "github",
            "search",
            Some("Search GitHub".to_string()),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        ))
        .await;

    let tools = resolve_tools_with_mcp_registry(&tool_registry, Some(&mcp_registry)).await;
    assert!(tools.iter().any(|t| t.name == "github_search"));
}

#[tokio::test]
async fn execute_tool_calls_ignores_empty_tool_name() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run tools"));

    let mut assistant = SessionMessage::assistant(sid);
    assistant.parts.push(crate::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4()),
        part_type: PartType::ToolCall {
            id: "call_empty".to_string(),
            name: " ".to_string(),
            input: serde_json::json!({}),
            status: crate::ToolCallStatus::Running,
            raw: None,
            state: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.add_tool_call("call_ok", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");
    let result_ids: Vec<&str> = tool_msg
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(result_ids, vec!["call_ok"]);
}

#[tokio::test]
async fn execute_tool_calls_runs_no_arg_tool() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run noarg"));
    let mut assistant = SessionMessage::assistant(sid);
    assistant.add_tool_call("call_noarg", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");

    let (content, is_error) = tool_msg
        .parts
        .iter()
        .find_map(|part| match &part.part_type {
            PartType::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } if tool_call_id == "call_noarg" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("noarg result should exist");

    assert!(!is_error);
    assert_eq!(content, "{}");
}

#[tokio::test]
async fn execute_tool_calls_routes_invalid_arguments_to_invalid_tool() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(AlwaysInvalidArgsTool).await;
    tool_registry
        .register(rocode_tool::invalid::InvalidTool)
        .await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run invalid"));
    let mut assistant = SessionMessage::assistant(sid);
    assistant.add_tool_call("call_invalid", "needs_path", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let assistant_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .expect("assistant message should exist");
    let tool_call = assistant_msg
        .parts
        .iter()
        .find_map(|part| match &part.part_type {
            PartType::ToolCall {
                id,
                name,
                input,
                status,
                ..
            } if id == "call_invalid" => Some((name, input, status)),
            _ => None,
        })
        .expect("tool call should exist");
    assert_eq!(tool_call.0, "invalid");
    assert_eq!(
        tool_call.1.get("tool").and_then(|v| v.as_str()),
        Some("needs_path")
    );
    assert!(tool_call.1.get("receivedArgs").is_none());
    assert!(matches!(tool_call.2, crate::ToolCallStatus::Completed));

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");
    let (content, is_error) = tool_msg
        .parts
        .iter()
        .find_map(|part| match &part.part_type {
            PartType::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } if tool_call_id == "call_invalid" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("invalid fallback result should exist");
    assert!(!is_error);
    assert!(content.contains("The arguments provided to the tool are invalid:"));
}

#[tokio::test]
async fn execute_tool_calls_only_runs_running_tool_calls() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run running only"));
    let mut assistant = SessionMessage::assistant(sid);
    assistant.parts.push(crate::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4()),
        part_type: PartType::ToolCall {
            id: "call_pending".to_string(),
            name: "noarg_echo".to_string(),
            input: serde_json::json!({}),
            status: crate::ToolCallStatus::Pending,
            raw: Some("{".to_string()),
            state: Some(crate::ToolState::Pending {
                input: serde_json::json!({}),
                raw: "{".to_string(),
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.add_tool_call("call_running", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");
    let result_ids: Vec<&str> = tool_msg
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(result_ids, vec!["call_running"]);
}

#[tokio::test]
async fn execute_tool_calls_reused_call_id_in_new_turn_still_executes() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();

    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "turn one"));
    let mut assistant_1 = SessionMessage::assistant(sid.clone());
    assistant_1.add_tool_call("tool-call-0", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant_1);
    let mut tool_msg_1 = SessionMessage::tool(sid.clone());
    tool_msg_1.add_tool_result("tool-call-0", "{}", false);
    session.messages_mut().push(tool_msg_1);

    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "turn two"));
    let mut assistant_2 = SessionMessage::assistant(sid);
    assistant_2.add_tool_call("tool-call-0", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant_2);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msgs: Vec<&SessionMessage> = session
        .messages
        .iter()
        .filter(|m| matches!(m.role, MessageRole::Tool))
        .collect();
    assert!(
        tool_msgs.len() >= 2,
        "expected a second tool message for the new turn"
    );

    let last_tool_msg = tool_msgs.last().expect("latest tool message should exist");
    let second_turn_result_count = last_tool_msg
        .parts
        .iter()
        .filter(|part| {
            matches!(
                &part.part_type,
                PartType::ToolResult { tool_call_id, .. } if tool_call_id == "tool-call-0"
            )
        })
        .count();
    assert_eq!(second_turn_result_count, 1);
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
fn part_input_agent_round_trip() {
    let part = PartInput::Agent {
        name: "explore".to_string(),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "agent");
    assert_eq!(json["name"], "explore");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::Agent { name } if name == "explore"));
}

#[test]
fn part_input_subtask_round_trip() {
    let part = PartInput::Subtask {
        prompt: "do stuff".to_string(),
        description: Some("stuff".to_string()),
        agent: "build".to_string(),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "subtask");
    assert_eq!(json["agent"], "build");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::Subtask { agent, .. } if agent == "build"));
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
        {"type": "agent", "name": "explore"},
        {"type": "bogus"},
        {"type": "file", "url": "file:///x", "filename": "x", "mime": "text/plain"}
    ]);
    let parts = PartInput::parse_array(&arr);
    assert_eq!(parts.len(), 3); // bogus entry skipped
    assert!(matches!(&parts[0], PartInput::Text { text } if text == "hello"));
    assert!(matches!(&parts[1], PartInput::Agent { name } if name == "explore"));
    assert!(matches!(&parts[2], PartInput::File { url, .. } if url == "file:///x"));
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
    let parts = resolve_prompt_parts("just plain text", std::path::Path::new("/tmp"), &[]).await;
    assert_eq!(parts.len(), 1);
    assert!(matches!(&parts[0], PartInput::Text { text } if text == "just plain text"));
}

#[tokio::test]
async fn resolve_prompt_parts_agent_fallback() {
    // @explore doesn't exist as a file, but is a known agent
    let agents = vec!["explore".to_string(), "build".to_string()];
    let parts = resolve_prompt_parts(
        "check @explore for details",
        std::path::Path::new("/tmp"),
        &agents,
    )
    .await;
    assert_eq!(parts.len(), 2);
    assert!(matches!(&parts[0], PartInput::Text { .. }));
    assert!(matches!(&parts[1], PartInput::Agent { name } if name == "explore"));
}

#[tokio::test]
async fn resolve_prompt_parts_deduplicates() {
    let parts = resolve_prompt_parts(
        "see @explore and @explore again",
        std::path::Path::new("/tmp"),
        &["explore".to_string()],
    )
    .await;
    // text + one agent (deduplicated)
    assert_eq!(parts.len(), 2);
}

#[tokio::test]
async fn resolve_prompt_parts_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.rs");
    tokio::fs::write(&file, "fn main() {}").await.unwrap();

    let parts = resolve_prompt_parts("look at @test.rs", dir.path(), &[]).await;
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

    let parts = resolve_prompt_parts("look at @src", dir.path(), &[]).await;
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

    let messages = vec![user, assistant];

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

#[test]
fn current_context_estimate_ignores_completed_scheduler_stage_metadata() {
    let mut assistant = SessionMessage::assistant("s1");
    assistant.metadata.insert(
        "scheduler_stage_status".to_string(),
        serde_json::json!("done"),
    );
    assistant.metadata.insert(
        "scheduler_stage_context_tokens".to_string(),
        serde_json::json!(1_388_907_u64),
    );
    assistant.metadata.insert(
        "scheduler_stage_prompt_tokens".to_string(),
        serde_json::json!(1_388_907_u64),
    );
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "done".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });

    let estimated = estimate_current_context_tokens(&[assistant]).unwrap();

    assert!(
        estimated < 1_000,
        "completed scheduler stage peak telemetry must not drive live context pressure: {estimated}"
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

    let messages = vec![user, assistant];

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

    let messages = vec![user, assistant];

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
fn runtime_skill_save_suggestion_skips_turns_that_are_only_complex() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("optimize this workflow");
    let assistant = session.add_assistant_message();
    assistant.finish = Some("stop".to_string());
    let tool = SessionMessage::tool(session.id.clone());
    session.messages_mut().push(tool);

    let tool_msg = session.messages_mut().last_mut().unwrap();
    for index in 0..3 {
        tool_msg.parts.push(MessagePart {
            id: format!("prt_tool_{index}"),
            part_type: PartType::ToolResult {
                tool_call_id: format!("call_{index}"),
                content: "ok".to_string(),
                is_error: false,
                title: None,
                metadata: None,
                attachments: None,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
    }

    SessionPrompt::maybe_append_runtime_skill_save_suggestion(&mut session, 0);

    assert!(!session.messages.iter().any(|message| {
        matches!(message.role, MessageRole::Assistant)
            && message
                .metadata
                .get("runtime_hint")
                .and_then(|value| value.as_str())
                == Some("skill_save_suggestion")
    }));
}

#[test]
fn runtime_skill_save_suggestion_triggers_for_methodology_shaped_turns() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("fix the failing parser and verify it");

    let assistant = session.add_assistant_message();
    assistant.finish = Some("tool-calls".to_string());
    assistant.parts.push(MessagePart {
        id: "prt_tool_edit".to_string(),
        part_type: PartType::ToolCall {
            id: "call_edit".to_string(),
            name: "edit".to_string(),
            input: serde_json::json!({
                "file_path": "src/parser.rs",
                "old_string": "broken()",
                "new_string": "fixed()"
            }),
            raw: None,
            status: crate::ToolCallStatus::Completed,
            state: Some(crate::ToolState::Completed {
                input: serde_json::json!({
                    "file_path": "src/parser.rs",
                    "old_string": "broken()",
                    "new_string": "fixed()"
                }),
                output: "patched parser".to_string(),
                title: "Edited parser".to_string(),
                metadata: std::collections::HashMap::new(),
                time: crate::CompletedTime {
                    start: chrono::Utc::now().timestamp_millis(),
                    end: chrono::Utc::now().timestamp_millis(),
                    compacted: None,
                },
                attachments: None,
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.parts.push(MessagePart {
        id: "prt_tool_bash_failed".to_string(),
        part_type: PartType::ToolCall {
            id: "call_bash_failed".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "cargo test parser",
                "description": "Run parser tests"
            }),
            raw: None,
            status: crate::ToolCallStatus::Error,
            state: Some(crate::ToolState::Error {
                input: serde_json::json!({
                    "command": "cargo test parser",
                    "description": "Run parser tests"
                }),
                error: "parser test still failing".to_string(),
                metadata: None,
                time: crate::ErrorTime {
                    start: chrono::Utc::now().timestamp_millis(),
                    end: chrono::Utc::now().timestamp_millis(),
                },
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });

    let followup = session.add_assistant_message();
    followup.finish = Some("stop".to_string());
    followup.add_text("Patched the parser and verified the failure mode; this flow is reusable.");

    SessionPrompt::maybe_append_runtime_skill_save_suggestion(&mut session, 0);

    let note = session
        .messages
        .iter()
        .find(|message| {
            message
                .metadata
                .get("runtime_hint")
                .and_then(|value| value.as_str())
                == Some("skill_save_suggestion")
        })
        .expect("runtime hint note should be appended");

    assert!(note.parts.iter().any(|part| {
        matches!(
            &part.part_type,
            PartType::Text { text, .. }
                if text.contains("reusable triggers, steps, validation, and boundaries")
                    && text.contains("skill_manage({ action: \"create\", name, description, methodology })")
                    && text.contains("Do not call the `invalid` tool directly.")
        )
    }));
}

#[test]
fn extract_tool_call_history_from_session_parts() {
    let mut session = Session::new("proj", ".");
    let assistant = session.add_assistant_message();
    SessionPrompt::upsert_tool_call_part(
        assistant,
        "call_bash",
        Some("bash"),
        Some(serde_json::json!({"command": "cargo test"})),
        None,
        Some(crate::ToolCallStatus::Completed),
        None,
    );

    let mut tool_message = SessionMessage::tool(session.id.clone());
    SessionPrompt::push_tool_result_part(
        &mut tool_message,
        "call_bash".to_string(),
        "ok".to_string(),
        false,
        None,
        None,
        None,
    );
    session.messages_mut().push(tool_message);

    let history = extract_tool_call_history(&session);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].tool_name, "bash");
    assert_eq!(history[0].tool_input_summary, "command=cargo test");
    assert_eq!(history[0].tool_result_summary, "ok");
}

#[test]
fn prepare_skill_reflection_returns_none_without_runtime_skills() {
    let session = Session::new("proj", ".");
    assert!(prepare_skill_reflection(None, &session).is_none());
}

#[test]
fn prepare_skill_reflection_loads_methodology_from_runtime_skill_names() {
    let dir = tempdir().unwrap();
    write_methodology_skill(
        dir.path(),
        "runtime-check",
        rocode_skill::SkillMethodologyTemplate {
            when_to_use: vec!["Use when runtime checks should be repeated.".to_string()],
            when_not_to_use: vec!["Do not use for ad-hoc notes.".to_string()],
            prerequisites: vec![],
            core_steps: vec![rocode_skill::SkillMethodologyStep {
                title: "Check".to_string(),
                action: "Run the runtime check.".to_string(),
                outcome: Some("Runtime state is known.".to_string()),
                experienced_tools: vec!["cargo".to_string()],
            }],
            success_criteria: vec!["The runtime state is visible.".to_string()],
            validation: vec!["Repeat the check after changes.".to_string()],
            pitfalls: vec!["Do not patch production configs while checking.".to_string()],
            references: vec![],
        },
    );

    let mut session = Session::new("proj", dir.path().to_string_lossy().to_string());
    session.insert_metadata(
        "runtime_skill_instructions",
        serde_json::to_value(vec![RuntimeInstructionSource {
            path: dir.path().join("AGENTS.md"),
            content: r#"
1. For the harness protocol itself
   - target workspace skill: `runtime-check`
   - target path: `.rocode/skills/runtime-check/SKILL.md`
   - description: `Reusable runtime verification workflow.`
"#
            .to_string(),
        }])
        .unwrap(),
    );

    let reflection = prepare_skill_reflection(None, &session).expect("reflection should exist");
    assert_eq!(reflection.skills_used.len(), 1);
    assert_eq!(reflection.skills_used[0].name, "runtime-check");
    assert_eq!(
        reflection.skills_used[0]
            .methodology
            .as_ref()
            .expect("methodology should load")
            .core_steps[0]
            .experienced_tools,
        vec!["cargo".to_string()]
    );
}

#[test]
fn prepare_skill_reflection_keeps_retired_skill_history_visible() {
    let dir = tempdir().unwrap();
    let governance = SkillGovernanceAuthority::new(dir.path(), None);
    governance
        .create_skill(
            rocode_skill::CreateSkillRequest {
                name: "runtime-check".to_string(),
                description: "Reusable runtime verification workflow.".to_string(),
                body:
                    rocode_skill::render_methodology_skill_body(
                        "runtime-check",
                        &rocode_skill::SkillMethodologyTemplate {
                            when_to_use: vec![
                                "Use when runtime checks should be repeated.".to_string()
                            ],
                            when_not_to_use: vec!["Do not use for ad-hoc notes.".to_string()],
                            prerequisites: vec![],
                            core_steps: vec![rocode_skill::SkillMethodologyStep {
                                title: "Check".to_string(),
                                action: "Run the runtime check.".to_string(),
                                outcome: Some("Runtime state is known.".to_string()),
                                experienced_tools: vec!["cargo".to_string()],
                            }],
                            success_criteria: vec!["The runtime state is visible.".to_string()],
                            validation: vec!["Repeat the check after changes.".to_string()],
                            pitfalls: vec![
                                "Do not patch production configs while checking.".to_string()
                            ],
                            references: vec![],
                        },
                    )
                    .expect("render methodology body"),
                frontmatter: None,
                category: Some("ops".to_string()),
                directory_name: None,
            },
            "test:create-runtime-check",
        )
        .expect("create skill");
    governance
        .set_skill_vitality_state(
            "runtime-check",
            SkillVitalityState::Retired,
            SkillRetirementReason {
                kind: SkillRetirementReasonKind::ManualOverride,
                summary: "manual retire".to_string(),
                noted_at: 123,
                related_skill_name: None,
            },
            "test:retire-runtime-check",
        )
        .expect("retire skill");

    let mut session = Session::new("proj", dir.path().to_string_lossy().to_string());
    session.insert_metadata(
        "runtime_skill_instructions",
        serde_json::to_value(vec![RuntimeInstructionSource {
            path: dir.path().join("AGENTS.md"),
            content: r#"
1. For the harness protocol itself
   - target workspace skill: `runtime-check`
   - target path: `.rocode/skills/runtime-check/SKILL.md`
   - description: `Reusable runtime verification workflow.`
"#
            .to_string(),
        }])
        .unwrap(),
    );

    let reflection = prepare_skill_reflection(None, &session)
        .expect("retired skill should remain visible to historical reflection");
    assert_eq!(reflection.skills_used.len(), 1);
    assert_eq!(reflection.skills_used[0].name, "runtime-check");
    assert_eq!(
        reflection.skills_used[0]
            .methodology
            .as_ref()
            .expect("methodology should still load")
            .core_steps[0]
            .experienced_tools,
        vec!["cargo".to_string()]
    );
}

#[tokio::test]
async fn apply_runtime_workspace_context_adds_composition_governance_reminder() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("AGENTS.md"),
        r#"
1. For the harness protocol itself
   - target workspace skill: `provider-refresh-gitlab`
   - target path: `.rocode/skills/provider-refresh-gitlab/SKILL.md`
   - description: `GitLab provider refresh workflow.`

2. For the harness protocol itself
   - target workspace skill: `frontend-ui-ux`
   - target path: `.rocode/skills/frontend-ui-ux/SKILL.md`
   - description: `Frontend UX workflow.`

3. For the harness protocol itself
   - target workspace skill: `frontend-ui-a11y`
   - target path: `.rocode/skills/frontend-ui-a11y/SKILL.md`
   - description: `Frontend accessibility workflow.`
"#,
    )
    .unwrap();

    let governance = SkillGovernanceAuthority::new(dir.path(), None);
    for skill_name in [
        "provider-refresh",
        "provider-refresh-gitlab",
        "frontend-ui-ux",
        "frontend-ui-a11y",
    ] {
        governance
            .create_skill(
                rocode_skill::CreateSkillRequest {
                    name: skill_name.to_string(),
                    description: format!("{skill_name} skill"),
                    body: format!("Use {skill_name}."),
                    frontmatter: None,
                    category: Some("test".to_string()),
                    directory_name: None,
                },
                "test:create",
            )
            .unwrap();
    }
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
            vec!["gitlab refresh is governed by shared provider refresh".to_string()],
            "test:activate-group",
        )
        .unwrap();
    governance
        .activate_skill_capability_group(
            Some("frontend-delivery-bundle"),
            SkillCapabilityGroupKind::ComplementaryBundle,
            None,
            vec![
                SkillCapabilityMember {
                    skill_name: "frontend-ui-ux".to_string(),
                    role: SkillCapabilityMemberRole::Complementary,
                },
                SkillCapabilityMember {
                    skill_name: "frontend-ui-a11y".to_string(),
                    role: SkillCapabilityMemberRole::Complementary,
                },
            ],
            vec!["frontend delivery needs both ux and a11y coverage".to_string()],
            "test:activate-group",
        )
        .unwrap();

    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", dir.path().to_string_lossy().to_string());
    session.add_user_message("run the runtime skills");

    prompt
        .apply_runtime_workspace_context(&mut session)
        .await
        .expect("runtime workspace context should apply");

    let user_message = session.messages.last().expect("user message should exist");
    let rendered = user_message.get_text();
    assert!(rendered.contains("Runtime Skill Governance:"));
    assert!(rendered.contains("provider-refresh-gitlab"));
    assert!(rendered.contains("provider-refresh"));
    assert!(rendered.contains("frontend-ui-ux"));
    assert!(rendered.contains("frontend-ui-a11y"));
    assert!(rendered.contains("Keep their responsibilities distinct"));
}

#[test]
fn update_skill_reflection_metadata_removes_stale_value_when_none() {
    let mut session = Session::new("proj", ".");
    session.insert_metadata("skill_reflection", serde_json::json!({"stale": true}));

    update_skill_reflection_metadata(None, &mut session);
    assert!(!session.metadata.contains_key("skill_reflection"));
}

#[test]
fn augment_system_prompt_with_skill_reflection_consumes_metadata_once() {
    let mut session = Session::new("proj", ".");
    session.insert_metadata(
        "skill_reflection",
        serde_json::to_value(SkillReflectionData {
            skills_used: vec![SkillUsageSummary {
                name: "runtime-check".to_string(),
                methodology: None,
            }],
            tool_calls: vec![ToolCallSummary {
                tool_name: "bash".to_string(),
                tool_input_summary: "command=cargo test".to_string(),
                tool_result_summary: "ok".to_string(),
            }],
        })
        .unwrap(),
    );

    let first = augment_system_prompt_with_skill_reflection(&mut session, Some("BASE".to_string()))
        .expect("prompt should exist");
    assert!(first.contains("BASE"));
    assert!(first.contains("## Skill Usage Reflection"));
    assert!(!session.metadata.contains_key("skill_reflection"));

    let second =
        augment_system_prompt_with_skill_reflection(&mut session, Some("BASE".to_string()))
            .expect("base prompt should remain");
    assert_eq!(second, "BASE");
}

#[test]
fn tool_is_validation_accepts_generic_validation_signals() {
    assert!(tool_is_validation(
        "health_check",
        &serde_json::json!({ "operation": "status" })
    ));
    assert!(tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "kubectl apply -f deploy.yaml --dry-run=client"
        })
    ));
    assert!(tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "acme-cli verify dataset"
        })
    ));
}

#[test]
fn tool_is_validation_rejects_plain_output_commands() {
    assert!(!tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "echo verify deployment"
        })
    ));
    assert!(!tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "cat verify.txt"
        })
    ));
}

#[test]
fn runtime_skill_save_suggestion_skips_turns_that_already_used_skill_manage() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("optimize this workflow");
    let assistant = session.add_assistant_message();
    assistant.finish = Some("stop".to_string());
    assistant.parts.push(MessagePart {
        id: "prt_tool_call".to_string(),
        part_type: PartType::ToolCall {
            id: "call_skill".to_string(),
            name: "skill_manage".to_string(),
            input: serde_json::json!({ "action": "create" }),
            raw: None,
            status: crate::ToolCallStatus::Completed,
            state: Some(crate::ToolState::Completed {
                input: serde_json::json!({ "action": "create" }),
                output: "created".to_string(),
                title: "Skill created".to_string(),
                metadata: std::collections::HashMap::new(),
                time: crate::CompletedTime {
                    start: chrono::Utc::now().timestamp_millis(),
                    end: chrono::Utc::now().timestamp_millis(),
                    compacted: None,
                },
                attachments: None,
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });

    SessionPrompt::maybe_append_runtime_skill_save_suggestion(&mut session, 0);

    assert!(!session.messages.iter().any(|message| {
        message
            .metadata
            .get("runtime_hint")
            .and_then(|value| value.as_str())
            == Some("skill_save_suggestion")
    }));
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

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None)
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            rocode_provider::Content::Text(text) => text.clone(),
            rocode_provider::Content::Parts(parts) => parts
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
    assert_eq!(SessionPrompt::model_context_char_len(&notice), 0);
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

    let provider_messages = SessionPrompt::build_chat_messages(&session.messages, None)
        .expect("chat messages should build");
    let rendered = provider_messages
        .iter()
        .map(|message| match &message.content {
            rocode_provider::Content::Text(text) => text.clone(),
            rocode_provider::Content::Parts(parts) => parts
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
    fs::create_dir_all(dir.path().join(".rocode/skills/provider-refresh"))
        .expect("skill dir should exist");
    fs::write(
        dir.path().join(".rocode/skills/provider-refresh/SKILL.md"),
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

    let summary = rocode_storage::generate_skill_evolution_proposals(
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
    fs::create_dir_all(dir.path().join(".rocode/skills/provider-refresh"))
        .expect("canonical skill dir should exist");
    fs::write(
        dir.path().join(".rocode/skills/provider-refresh/SKILL.md"),
        r#"---
name: provider-refresh
description: refresh provider
---
Use for provider refresh tasks.
"#,
    )
    .expect("canonical skill file should exist");
    fs::create_dir_all(dir.path().join(".rocode/skills/provider-refresh-gitlab"))
        .expect("specialization skill dir should exist");
    fs::write(
        dir.path()
            .join(".rocode/skills/provider-refresh-gitlab/SKILL.md"),
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

    let summary = rocode_storage::generate_skill_evolution_proposals(
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
