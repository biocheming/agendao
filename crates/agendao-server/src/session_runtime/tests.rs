use super::*;
use agendao_provider::{
    ChatRequest, ChatResponse, Choice, Content, Message, ModelInfo, Provider, ProviderError, Role,
    StreamResult,
};
use futures::stream;
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

#[derive(Debug)]
struct MockProvider {
    title: String,
}

#[derive(Debug)]
struct StaticModelProvider {
    model: ModelInfo,
}

#[test]
fn scheduler_context_hydration_event_persists_stage_metadata() {
    let mut message = SessionMessage::assistant("session");
    persist_scheduler_context_hydration_event(
        &mut message,
        "tool_call_1",
        Some(&serde_json::json!({
            "hydrated_count": 2,
            "rejected_count": 1,
            "missing_count": 1,
            "hydrated_message_ids": ["msg_a", "msg_b"],
            "rejected_message_ids": ["msg_x"],
            "missing_message_ids": ["msg_y"],
            "max_chars_per_message": 2000
        })),
    );
    persist_scheduler_context_hydration_event(
        &mut message,
        "tool_call_2",
        Some(&serde_json::json!({
            "hydrated_count": 1,
            "hydrated_message_ids": ["msg_b"]
        })),
    );

    assert_eq!(
        message
            .metadata
            .get(SCHEDULER_STAGE_CONTEXT_HYDRATION_IDS_METADATA_KEY),
        Some(&serde_json::json!(["msg_a", "msg_b"]))
    );
    let events = message
        .metadata
        .get(SCHEDULER_STAGE_CONTEXT_HYDRATION_EVENTS_METADATA_KEY)
        .and_then(|value| value.as_array())
        .expect("hydration events should be recorded");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["tool_call_id"], "tool_call_1");
    assert_eq!(
        events[0]["rejected_message_ids"],
        serde_json::json!(["msg_x"])
    );
    assert_eq!(
        events[0]["missing_message_ids"],
        serde_json::json!(["msg_y"])
    );
}

#[test]
fn scheduler_memory_hydration_event_persists_stage_metadata() {
    let mut message = SessionMessage::assistant("session");
    persist_scheduler_memory_hydration_event(
        &mut message,
        "tool_call_1",
        Some(&serde_json::json!({
            "hydrated_count": 2,
            "rejected_count": 1,
            "missing_count": 1,
            "hydrated_memory_record_ids": ["mem_a", "mem_b"],
            "rejected_memory_record_ids": ["mem_x"],
            "missing_memory_record_ids": ["mem_y"],
            "max_chars_per_record": 4000,
            "include_evidence": true
        })),
    );
    persist_scheduler_memory_hydration_event(
        &mut message,
        "tool_call_2",
        Some(&serde_json::json!({
            "hydrated_count": 1,
            "hydrated_memory_record_ids": ["mem_b"]
        })),
    );

    assert_eq!(
        message
            .metadata
            .get(SCHEDULER_STAGE_MEMORY_HYDRATION_IDS_METADATA_KEY),
        Some(&serde_json::json!(["mem_a", "mem_b"]))
    );
    let events = message
        .metadata
        .get(SCHEDULER_STAGE_MEMORY_HYDRATION_EVENTS_METADATA_KEY)
        .and_then(|value| value.as_array())
        .expect("memory hydration events should be recorded");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["tool_call_id"], "tool_call_1");
    assert_eq!(
        events[0]["rejected_memory_record_ids"],
        serde_json::json!(["mem_x"])
    );
    assert_eq!(
        events[0]["missing_memory_record_ids"],
        serde_json::json!(["mem_y"])
    );
    assert_eq!(events[0]["include_evidence"], true);
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn get_model(&self, _id: &str) -> Option<&ModelInfo> {
        None
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            id: "mock-response".to_string(),
            model: "mock-model".to_string(),
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    content: Content::Text(self.title.clone()),
                    cache_control: None,
                    provider_options: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        })
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        Ok(Box::pin(stream::iter(Vec::<
            Result<agendao_provider::StreamEvent, ProviderError>,
        >::new())))
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
        vec![self.model.clone()]
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        (self.model.id == id).then_some(&self.model)
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        Ok(Box::pin(stream::iter(Vec::<
            Result<agendao_provider::StreamEvent, ProviderError>,
        >::new())))
    }
}

#[test]
fn scheduler_stage_title_prettifies_profile_and_hyphenated_stage_names() {
    assert_eq!(
        scheduler_stage_title("prometheus", "execution-orchestration"),
        "Prometheus · Execution Orchestration"
    );
}

#[test]
fn first_user_message_text_uses_first_real_user_message() {
    let mut session = Session::new("project", ".");
    session.add_assistant_message().add_text("hello");
    session.add_user_message("  First prompt  ");
    session.add_user_message("Second prompt");

    assert_eq!(
        first_user_message_text(&session).as_deref(),
        Some("First prompt")
    );
}

#[test]
fn first_user_message_text_strips_system_reminder_content() {
    let mut session = Session::new("project", ".");
    session.add_user_message(
            "Refactor the session renderer\n<system-reminder>\nInstructions from: /tmp/project/AGENTS.md\nUse reratui.\n</system-reminder>",
        );

    assert_eq!(
        first_user_message_text(&session).as_deref(),
        Some("Refactor the session renderer")
    );
}

#[test]
fn visible_assistant_text_from_orchestrator_output_uses_direct_response() {
    let output = r###"{"mode":"direct","direct_kind":"reply","direct_response":"## Answer\n\n- item","rationale_summary":"concept reply"}"###;

    assert_eq!(
        visible_assistant_text_from_orchestrator_output(output),
        "## Answer\n\n- item"
    );
}

#[tokio::test]
async fn emit_scheduler_stage_message_appends_assistant_stage_message() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };

    emit_scheduler_stage_message(SchedulerStageMessageInput {
        state: &state,
        session_id: &session_id,
        scheduler_profile: "prometheus",
        stage_name: "plan",
        stage_index: 3,
        stage_total: 4,
        content: "## Plan\n- step",
        exec_ctx: &exec_ctx,
        output_hook: None,
    })
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(message.get_text(), "## Plan\n- step");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage")
            .and_then(|value| value.as_str()),
        Some("plan")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_projection")
            .and_then(|value| value.as_str()),
        Some("transcript")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_loop_budget")
            .and_then(|value| value.as_str()),
        Some("unbounded")
    );

    let summaries = state
        .runtime_telemetry
        .list_stage_summaries(&session_id)
        .await;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].stage_name, "plan");
    assert_eq!(
        summaries[0].status,
        agendao_stage_protocol::StageStatus::Done
    );
}

#[tokio::test]
async fn emit_scheduler_stage_message_consumes_pending_compaction_notice() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        let session_id = sessions.create("project", ".").id.clone();
        let mut session = sessions
            .get(&session_id)
            .cloned()
            .expect("session should exist");
        session.insert_metadata(
            SCHEDULER_STAGE_PENDING_LAST_EVENT_KEY,
            serde_json::json!("Session context compacted before stage execution"),
        );
        session.insert_metadata(
            SCHEDULER_STAGE_PENDING_COMPACTION_PHASE_KEY,
            serde_json::json!("scheduler.pre_run"),
        );
        sessions.update(session);
        session_id
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };

    emit_scheduler_stage_message(SchedulerStageMessageInput {
        state: &state,
        session_id: &session_id,
        scheduler_profile: "prometheus",
        stage_name: "plan",
        stage_index: 1,
        stage_total: 2,
        content: "## Plan\n- compacted before start",
        exec_ctx: &exec_ctx,
        output_hook: None,
    })
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_last_event")
            .and_then(|value| value.as_str()),
        Some("Session context compacted before stage execution")
    );
    assert_eq!(
        message
            .metadata
            .get("context_compaction_phase")
            .and_then(|value| value.as_str()),
        Some("scheduler.pre_run")
    );
    assert!(!session
        .record()
        .metadata
        .contains_key(SCHEDULER_STAGE_PENDING_LAST_EVENT_KEY));
}

#[test]
fn gate_decision_block_does_not_duplicate_final_response_section() {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "scheduler_stage".to_string(),
        serde_json::json!("coordination-gate"),
    );
    metadata.insert(
        "scheduler_decision_kind".to_string(),
        serde_json::json!("gate"),
    );
    metadata.insert(
        "scheduler_decision_title".to_string(),
        serde_json::json!("Decision"),
    );
    metadata.insert(
        "scheduler_decision_fields".to_string(),
        serde_json::json!([{"label":"Outcome","value":"Done","tone":"status"}]),
    );
    metadata.insert(
        "scheduler_decision_sections".to_string(),
        serde_json::json!([]),
    );
    metadata.insert(
        "scheduler_gate_final_response".to_string(),
        serde_json::json!("## Delivery Summary\nDone."),
    );

    let decision = scheduler_decision_block(&metadata, "coordination-gate", "{}").unwrap();
    assert!(decision.sections.is_empty());
}

#[tokio::test]
async fn emit_internal_scheduler_stage_message_is_still_renderable() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };

    emit_scheduler_stage_message(SchedulerStageMessageInput {
        state: &state,
        session_id: &session_id,
        scheduler_profile: "atlas",
        stage_name: "coordination-verification",
        stage_index: 1,
        stage_total: 3,
        content: "## Coordination Verification\n\nMissing proof for task B.",
        exec_ctx: &exec_ctx,
        output_hook: None,
    })
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message.get_text(),
        "## Coordination Verification\n\nMissing proof for task B."
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage")
            .and_then(|value| value.as_str()),
        Some("coordination-verification")
    );
    assert!(!message.metadata.contains_key("scheduler_stage_projection"));
}

#[tokio::test]
async fn lifecycle_hook_updates_stage_runtime_metadata() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );

    hook.on_scheduler_stage_start("prometheus", "plan", 3, None, &exec_ctx)
        .await;
    hook.on_step_start("prometheus", "model", 1, &exec_ctx)
        .await;
    hook.on_tool_start(
        "prometheus",
        "tc_question_1",
        "question",
        &serde_json::json!({
            "questions": [{
                "header": "Scope",
                "question": "Proceed with schema migration?",
                "options": [{"label": "Yes"}]
            }]
        }),
        &exec_ctx,
    )
    .await;
    hook.on_tool_end(
        "prometheus",
        "tc_question_1",
        "question",
        &OrchestratorToolOutput {
            output: "{}".to_string(),
            is_error: false,
            title: Some("User response received".to_string()),
            metadata: Some(serde_json::json!({
                "display.fields": [{
                    "key": "Proceed with schema migration?",
                    "value": "Yes"
                }]
            })),
        },
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_end("prometheus", "plan", 3, 5, "## Plan\n\n- step", &exec_ctx)
        .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_step")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_status")
            .and_then(|value| value.as_str()),
        Some("done")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_focus")
            .and_then(|value| value.as_str()),
        Some("Draft the executable plan and its guardrails.")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_last_event")
            .and_then(|value| value.as_str()),
        Some("Stage completed")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_waiting_on")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_activity")
            .and_then(|value| value.as_str()),
        Some("Answered (1)\n- Proceed with schema migration?: Yes")
    );

    let summaries = state
        .runtime_telemetry
        .list_stage_summaries(&session_id)
        .await;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].stage_name, "plan");
    assert_eq!(summaries[0].step, Some(1));
    assert_eq!(
        summaries[0].status,
        agendao_stage_protocol::StageStatus::Done
    );
    assert_eq!(summaries[0].active_tool_count, 0);
}

#[tokio::test]
async fn lifecycle_hook_accumulates_stage_usage_metadata() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );

    hook.on_scheduler_stage_start("prometheus", "plan", 2, None, &exec_ctx)
        .await;
    hook.on_scheduler_stage_usage(
        "plan",
        2,
        &agendao_orchestrator::runtime::events::StepUsage {
            prompt_tokens: 1200,
            completion_tokens: 320,
            context_tokens: 0,
            reasoning_tokens: 40,
            cache_read_tokens: 2,
            cache_miss_tokens: 0,
            cache_write_tokens: 1,
        },
        false,
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_usage(
        "plan",
        2,
        &agendao_orchestrator::runtime::events::StepUsage {
            prompt_tokens: 1300,
            completion_tokens: 340,
            context_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 2,
            cache_miss_tokens: 0,
            cache_write_tokens: 1,
        },
        true,
        &exec_ctx,
    )
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_prompt_tokens")
            .and_then(|value| value.as_u64()),
        Some(1300)
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_completion_tokens")
            .and_then(|value| value.as_u64()),
        Some(340)
    );
    let usage = message.usage.as_ref().expect("usage should exist");
    assert_eq!(usage.input_tokens, 1300);
    assert_eq!(usage.output_tokens, 340);
    assert_eq!(usage.reasoning_tokens, 40);
    assert_eq!(usage.cache_read_tokens, 2);
    assert_eq!(usage.cache_write_tokens, 1);
    // No model pricing attached → total_cost defaults to 0.
    assert_eq!(usage.total_cost, 0.0);
}

#[tokio::test]
async fn lifecycle_hook_computes_total_cost_with_pricing() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    // Sonnet-like pricing: input $3/M, output $15/M,
    // cache_read $0.30/M, cache_write $3.75/M.
    let pricing = ModelPricing::new(3.0, 15.0, Some(0.30), Some(3.75));
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    )
    .with_model_pricing(Some(pricing));

    hook.on_scheduler_stage_start("prometheus", "plan", 2, None, &exec_ctx)
        .await;
    hook.on_scheduler_stage_usage(
        "plan",
        2,
        &agendao_orchestrator::runtime::events::StepUsage {
            prompt_tokens: 1_000_000,
            completion_tokens: 100_000,
            context_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 500_000,
            cache_miss_tokens: 0,
            cache_write_tokens: 200_000,
        },
        true,
        &exec_ctx,
    )
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    let usage = message.usage.as_ref().expect("usage should exist");
    // Expected: 3.0 * 1M/1M + 15.0 * 100K/1M + 0.30 * 500K/1M + 3.75 * 200K/1M
    //         = 3.0     + 1.5      + 0.15       + 0.75
    //         = 5.40
    let expected = 3.0 + 1.5 + 0.15 + 0.75;
    assert!(
        (usage.total_cost - expected).abs() < 1e-10,
        "expected total_cost ≈ {}, got {}",
        expected,
        usage.total_cost
    );
}

#[tokio::test]
async fn lifecycle_hook_merges_split_stage_usage_snapshots() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };
    let hook =
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), "atlas".to_string());

    hook.on_scheduler_stage_start("atlas", "coordination-gate", 2, None, &exec_ctx)
        .await;
    hook.on_scheduler_stage_usage(
        "coordination-gate",
        2,
        &agendao_orchestrator::runtime::events::StepUsage {
            prompt_tokens: 1200,
            completion_tokens: 0,
            context_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_miss_tokens: 0,
            cache_write_tokens: 0,
        },
        false,
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_usage(
        "coordination-gate",
        2,
        &agendao_orchestrator::runtime::events::StepUsage {
            prompt_tokens: 0,
            completion_tokens: 320,
            context_tokens: 0,
            reasoning_tokens: 40,
            cache_read_tokens: 2,
            cache_miss_tokens: 0,
            cache_write_tokens: 1,
        },
        true,
        &exec_ctx,
    )
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_prompt_tokens")
            .and_then(|value| value.as_u64()),
        Some(1200)
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_completion_tokens")
            .and_then(|value| value.as_u64()),
        Some(320)
    );
    let usage = message.usage.as_ref().expect("usage should exist");
    assert_eq!(usage.input_tokens, 1200);
    assert_eq!(usage.output_tokens, 320);
    assert_eq!(usage.reasoning_tokens, 40);
    assert_eq!(usage.cache_read_tokens, 2);
    assert_eq!(usage.cache_write_tokens, 1);
}

#[tokio::test]
async fn lifecycle_hook_step_checkpoint_records_compaction_and_continues() {
    let state = Arc::new(ServerState::new());
    {
        let mut providers = state.providers.write().await;
        providers.register(StaticModelProvider {
            model: ModelInfo {
                id: "ctx-model".to_string(),
                name: "Context Model".to_string(),
                provider: "mock".to_string(),
                context_window: 100,
                max_input_tokens: None,
                max_output_tokens: 20,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
                cost_per_million_cache_read: None,
                cost_per_million_cache_write: None,
            },
        });
    }
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        let mut session = sessions.create("project", ".");
        session.insert_metadata("model_provider", serde_json::json!("mock"));
        session.insert_metadata("model_id", serde_json::json!("ctx-model"));
        for index in 0..10 {
            session.add_user_message(format!("message {index}"));
        }
        let id = session.id.clone();
        sessions.update(session);
        id
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );
    let request_view = vec![agendao_provider::Message::user("message 9".to_string()); 10];
    let initial_checkpoint = agendao_orchestrator::runtime::events::StepCheckpointSnapshot {
        assessment_index: 1,
        max_assessments: 2,
        current_view: agendao_orchestrator::runtime::events::RequestViewMetrics {
            message_count: 10,
            system_prefix_messages: 0,
            compactable_messages: 10,
            user_messages: 10,
            assistant_messages: 0,
            tool_messages: 0,
            checkpoint_summary_messages: 0,
            estimated_context_tokens: Some(95),
            estimated_body_chars: Some(380),
        },
        previous_view: None,
        prior_mutations: Vec::new(),
    };

    hook.on_scheduler_stage_start("prometheus", "plan", 1, None, &exec_ctx)
        .await;
    let default_compact = StepCheckpointDirective::CompactRequestView {
        focus: Some("plan".to_string()),
        reason: Some("request_view_overflow".to_string()),
    };
    let directive = hook
        .on_step_checkpoint(
            "prometheus",
            "ctx-model",
            1,
            Some("plan"),
            Some(1),
            &agendao_orchestrator::runtime::events::StepUsage {
                prompt_tokens: 95,
                completion_tokens: 12,
                context_tokens: 95,
                reasoning_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                cache_write_tokens: 0,
            },
            &request_view,
            &initial_checkpoint,
            &default_compact,
            &exec_ctx,
        )
        .await
        .expect("checkpoint governance should observe matching runtime compaction");

    assert!(directive.is_none());

    let compacted_checkpoint = agendao_orchestrator::runtime::events::StepCheckpointSnapshot {
        assessment_index: 2,
        max_assessments: 2,
        current_view: agendao_orchestrator::runtime::events::RequestViewMetrics {
            message_count: 6,
            system_prefix_messages: 0,
            compactable_messages: 6,
            user_messages: 5,
            assistant_messages: 1,
            tool_messages: 0,
            checkpoint_summary_messages: 1,
            estimated_context_tokens: Some(60),
            estimated_body_chars: Some(240),
        },
        previous_view: Some(initial_checkpoint.current_view.clone()),
        prior_mutations: vec![agendao_orchestrator::runtime::events::RequestViewMutation {
            kind: agendao_orchestrator::runtime::events::RequestViewMutationKind::Compacted,
            reason: Some("request_view_overflow".to_string()),
            focus: Some("plan".to_string()),
            before: initial_checkpoint.current_view.clone(),
            after: agendao_orchestrator::runtime::events::RequestViewMetrics {
                message_count: 6,
                system_prefix_messages: 0,
                compactable_messages: 6,
                user_messages: 5,
                assistant_messages: 1,
                tool_messages: 0,
                checkpoint_summary_messages: 1,
                estimated_context_tokens: Some(60),
                estimated_body_chars: Some(240),
            },
            compacted_message_count: Some(5),
            summary_chars: Some(96),
        }],
    };

    let directive = hook
        .on_step_checkpoint(
            "prometheus",
            "ctx-model",
            1,
            Some("plan"),
            Some(1),
            &agendao_orchestrator::runtime::events::StepUsage {
                prompt_tokens: 60,
                completion_tokens: 12,
                context_tokens: 60,
                reasoning_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                cache_write_tokens: 0,
            },
            &request_view,
            &compacted_checkpoint,
            &StepCheckpointDirective::Continue,
            &exec_ctx,
        )
        .await
        .expect("checkpoint governance should continue after request-view compaction");

    assert!(directive.is_none());
    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let summary = session
        .record()
        .metadata
        .get(agendao_session::prompt::CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("summary should persist");
    let summary: agendao_types::ContextPressureGovernanceSummary =
        serde_json::from_value(summary).expect("summary should parse");
    assert_eq!(
        summary.status,
        agendao_types::ContextPressureGovernanceStatus::Compacted
    );
    assert_eq!(summary.phase, "scheduler.step_checkpoint");
    assert!(summary.compaction_attempted);
    assert!(summary.compaction_succeeded);
    assert!(!summary.blocking);
    let stage_message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        stage_message
            .metadata
            .get("scheduler_stage_last_event")
            .and_then(|value| value.as_str()),
        Some("Context pressure checkpoint compacted request view")
    );
}

#[tokio::test]
async fn lifecycle_hook_step_checkpoint_blocks_next_step_when_context_remains_unsafe() {
    let state = Arc::new(ServerState::new());
    {
        let mut providers = state.providers.write().await;
        providers.register(StaticModelProvider {
            model: ModelInfo {
                id: "ctx-model".to_string(),
                name: "Context Model".to_string(),
                provider: "mock".to_string(),
                context_window: 100,
                max_input_tokens: None,
                max_output_tokens: 20,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
                cost_per_million_cache_read: None,
                cost_per_million_cache_write: None,
            },
        });
    }
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        let mut session = sessions.create("project", ".");
        session.insert_metadata("model_provider", serde_json::json!("mock"));
        session.insert_metadata("model_id", serde_json::json!("ctx-model"));
        session.add_user_message("first");
        session.add_user_message("second");
        let id = session.id.clone();
        sessions.update(session);
        id
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );
    let request_view = vec![agendao_provider::Message::user("second".to_string()); 2];
    let blocked_checkpoint = agendao_orchestrator::runtime::events::StepCheckpointSnapshot {
        assessment_index: 2,
        max_assessments: 2,
        current_view: agendao_orchestrator::runtime::events::RequestViewMetrics {
            message_count: 2,
            system_prefix_messages: 0,
            compactable_messages: 2,
            user_messages: 2,
            assistant_messages: 0,
            tool_messages: 0,
            checkpoint_summary_messages: 0,
            estimated_context_tokens: Some(120),
            estimated_body_chars: Some(480),
        },
        previous_view: None,
        prior_mutations: vec![agendao_orchestrator::runtime::events::RequestViewMutation {
            kind: agendao_orchestrator::runtime::events::RequestViewMutationKind::Compacted,
            reason: Some("request_view_overflow".to_string()),
            focus: Some("plan".to_string()),
            before: agendao_orchestrator::runtime::events::RequestViewMetrics {
                message_count: 10,
                system_prefix_messages: 0,
                compactable_messages: 10,
                user_messages: 10,
                assistant_messages: 0,
                tool_messages: 0,
                checkpoint_summary_messages: 0,
                estimated_context_tokens: Some(160),
                estimated_body_chars: Some(640),
            },
            after: agendao_orchestrator::runtime::events::RequestViewMetrics {
                message_count: 2,
                system_prefix_messages: 0,
                compactable_messages: 2,
                user_messages: 2,
                assistant_messages: 0,
                tool_messages: 0,
                checkpoint_summary_messages: 0,
                estimated_context_tokens: Some(120),
                estimated_body_chars: Some(480),
            },
            compacted_message_count: Some(8),
            summary_chars: Some(80),
        }],
    };

    hook.on_scheduler_stage_start("prometheus", "plan", 1, None, &exec_ctx)
        .await;
    let directive = hook
        .on_step_checkpoint(
            "prometheus",
            "ctx-model",
            1,
            Some("plan"),
            Some(1),
            &agendao_orchestrator::runtime::events::StepUsage {
                prompt_tokens: 120,
                completion_tokens: 12,
                context_tokens: 120,
                reasoning_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                cache_write_tokens: 0,
            },
            &request_view,
            &blocked_checkpoint,
            &StepCheckpointDirective::Continue,
            &exec_ctx,
        )
        .await
        .expect("checkpoint governance should return a block directive");

    let Some(StepCheckpointDirective::Block { reason }) = directive else {
        panic!("checkpoint governance should block the next step");
    };
    assert!(reason.contains("Context pressure gate blocked the next scheduler step"));

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let summary = session
        .record()
        .metadata
        .get(agendao_session::prompt::CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("summary should persist");
    let summary: agendao_types::ContextPressureGovernanceSummary =
        serde_json::from_value(summary).expect("summary should parse");
    assert_eq!(
        summary.status,
        agendao_types::ContextPressureGovernanceStatus::Blocked
    );
    assert!(summary.blocking);

    let stage_message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        stage_message
            .metadata
            .get("scheduler_stage_status")
            .and_then(|value| value.as_str()),
        Some("blocked")
    );
    assert_eq!(
        stage_message
            .metadata
            .get("scheduler_stage_last_event")
            .and_then(|value| value.as_str()),
        Some("Context pressure gate blocked next step")
    );
}

#[tokio::test]
async fn lifecycle_hook_step_checkpoint_prefers_observed_usage_over_low_estimate() {
    let state = Arc::new(ServerState::new());
    {
        let mut providers = state.providers.write().await;
        providers.register(StaticModelProvider {
            model: ModelInfo {
                id: "ctx-model".to_string(),
                name: "Context Model".to_string(),
                provider: "mock".to_string(),
                context_window: 100,
                max_input_tokens: None,
                max_output_tokens: 20,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
                cost_per_million_cache_read: None,
                cost_per_million_cache_write: None,
            },
        });
    }
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        let mut session = sessions.create("project", ".");
        session.insert_metadata("model_provider", serde_json::json!("mock"));
        session.insert_metadata("model_id", serde_json::json!("ctx-model"));
        for index in 0..4 {
            session.add_user_message(format!("message {index}"));
        }
        let id = session.id.clone();
        sessions.update(session);
        id
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );
    let checkpoint = agendao_orchestrator::runtime::events::StepCheckpointSnapshot {
        assessment_index: 1,
        max_assessments: 2,
        current_view: agendao_orchestrator::runtime::events::RequestViewMetrics {
            message_count: 4,
            system_prefix_messages: 0,
            compactable_messages: 4,
            user_messages: 4,
            assistant_messages: 0,
            tool_messages: 0,
            checkpoint_summary_messages: 0,
            estimated_context_tokens: Some(60),
            estimated_body_chars: Some(240),
        },
        previous_view: None,
        prior_mutations: Vec::new(),
    };

    hook.on_scheduler_stage_start("prometheus", "plan", 1, None, &exec_ctx)
        .await;
    let directive = hook
        .on_step_checkpoint(
            "prometheus",
            "ctx-model",
            1,
            Some("plan"),
            Some(1),
            &agendao_orchestrator::runtime::events::StepUsage {
                prompt_tokens: 120,
                completion_tokens: 12,
                context_tokens: 120,
                reasoning_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                cache_write_tokens: 0,
            },
            &vec![agendao_provider::Message::user("message 3".to_string()); 4],
            &checkpoint,
            &StepCheckpointDirective::Continue,
            &exec_ctx,
        )
        .await
        .expect("checkpoint governance should use observed usage to request compaction");

    let Some(StepCheckpointDirective::CompactRequestView { focus, .. }) = directive else {
        panic!("checkpoint governance should request request-view compaction");
    };
    assert_eq!(focus.as_deref(), Some("plan"));

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let summary = session
        .record()
        .metadata
        .get(agendao_session::prompt::CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY)
        .cloned()
        .expect("summary should persist");
    let summary: agendao_types::ContextPressureGovernanceSummary =
        serde_json::from_value(summary).expect("summary should parse");
    assert_eq!(summary.request_context_tokens, Some(120));
    assert!(summary.limit_tokens.is_some_and(|limit| limit < 120));
    assert_eq!(
        summary.status,
        agendao_types::ContextPressureGovernanceStatus::Deferred
    );
    assert!(summary
        .request_pressure_percent
        .is_some_and(|percent| percent >= 100));
}

#[tokio::test]
async fn lifecycle_hook_tracks_active_stage_capabilities_from_tool_args() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };
    let hook =
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), "atlas".to_string());

    hook.on_scheduler_stage_start(
        "atlas",
        "execution-orchestration",
        2,
        Some(&SchedulerStageCapabilities {
            skill_list: vec![
                "debug".to_string().into(),
                "frontend-ui-ux".to_string().into(),
            ],
            agents: vec!["build".to_string(), "explore".to_string()],
            categories: vec!["frontend".to_string()],
            attached_session: false,
        }),
        &exec_ctx,
    )
    .await;
    hook.on_tool_start(
        "atlas",
        "tc_task_flow_1",
        "task_flow",
        &serde_json::json!({
            "operation": "create",
            "agent": "build",
            "load_skills": ["frontend-ui-ux"],
            "category": "frontend",
            "description": "Implement UI polish"
        }),
        &exec_ctx,
    )
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_available_skill_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_available_agent_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_available_category_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_active_agents")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("build")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_active_skills")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("frontend-ui-ux")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_active_categories")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("frontend")
    );
}

#[tokio::test]
async fn lifecycle_hook_routes_attached_session_content_to_attached_session() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let emitted = Arc::new(StdMutex::new(Vec::<OutputBlockEvent>::new()));
    let emitted_hook = emitted.clone();
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };
    let hook =
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), "atlas".to_string())
            .with_output_hook(Some(Arc::new(move |event| {
                let emitted_hook = emitted_hook.clone();
                Box::pin(async move {
                    emitted_hook
                        .lock()
                        .expect("output block lock should not poison")
                        .push(event);
                })
            })));

    hook.on_scheduler_stage_start(
        "atlas",
        "execution-orchestration",
        2,
        Some(&SchedulerStageCapabilities {
            skill_list: vec![],
            agents: vec![],
            categories: vec![],
            attached_session: true,
        }),
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_content(
        "execution-orchestration",
        2,
        "attached session streamed content",
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_reasoning(
        "execution-orchestration",
        2,
        "attached session streamed reasoning",
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_end(
        "atlas",
        "execution-orchestration",
        2,
        2,
        "## Execution Orchestration\n\nFinal stage body",
        &exec_ctx,
    )
    .await;

    let sessions = state.sessions.lock().await;
    let parent = sessions
        .get(&session_id)
        .expect("parent session should exist");
    let parent_stage_message = parent
        .record()
        .messages
        .last()
        .expect("parent stage message");
    let attached_session_id = parent_stage_message
        .metadata
        .get("scheduler_stage_attached_session_id")
        .and_then(|value| value.as_str())
        .expect("attached session id")
        .to_string();

    let child = sessions
        .get(&attached_session_id)
        .expect("attached session should exist");
    let child_message = child
        .record()
        .messages
        .last()
        .expect("child assistant message");
    assert_eq!(
        child_message.get_text(),
        "attached session streamed content"
    );
    assert_eq!(child_message.finish.as_deref(), Some("end_turn"));
    assert_eq!(child.parent_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(
        child.context_kind(),
        SessionContextKind::SchedulerStageOutputSession
    );
    assert_eq!(
        parent_stage_message
            .metadata
            .get("scheduler_stage_attached_session_kind"),
        Some(&serde_json::json!("scheduler_stage_output_session"))
    );
    drop(sessions);

    let emitted = emitted
        .lock()
        .expect("output block lock should not poison")
        .clone();
    let child_identities = emitted
        .iter()
        .filter(|event| event.session_id == attached_session_id)
        .map(|event| event.live_identity.clone())
        .collect::<Vec<_>>();
    let child_blocks = emitted
        .into_iter()
        .filter(|event| event.session_id == attached_session_id)
        .map(|event| event.block)
        .collect::<Vec<_>>();
    assert!(matches!(
        child_blocks.as_slice(),
        [
            OutputBlock::Message(message_start),
            OutputBlock::Message(message_delta),
            OutputBlock::Reasoning(reasoning_start),
            OutputBlock::Reasoning(reasoning_delta),
            OutputBlock::Reasoning(reasoning_end),
            OutputBlock::Message(message_end),
        ] if message_start == &MessageBlock::start(OutputMessageRole::Assistant)
            && message_delta
                == &MessageBlock::delta(
                    OutputMessageRole::Assistant,
                    "attached session streamed content",
                )
            && reasoning_start == &ReasoningBlock::start()
            && reasoning_delta == &ReasoningBlock::delta("attached session streamed reasoning")
            && reasoning_end == &ReasoningBlock::end()
            && message_end == &MessageBlock::end(OutputMessageRole::Assistant)
    ));
    assert_eq!(
        child_identities
            .iter()
            .map(|identity| identity.as_ref().map(|id| (&id.part_key, id.phase)))
            .collect::<Vec<_>>(),
        vec![
            Some((
                &agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
                agendao_types::LivePartPhase::Start
            )),
            Some((
                &agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
                agendao_types::LivePartPhase::Append
            )),
            Some((
                &agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
                agendao_types::LivePartPhase::Start
            )),
            Some((
                &agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
                agendao_types::LivePartPhase::Append
            )),
            Some((
                &agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
                agendao_types::LivePartPhase::End
            )),
            Some((
                &agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
                agendao_types::LivePartPhase::End
            )),
        ]
    );
}

#[tokio::test]
async fn lifecycle_hook_emits_scheduler_stage_and_reasoning_blocks_for_non_attached_session() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let emitted = Arc::new(StdMutex::new(Vec::<OutputBlockEvent>::new()));
    let emitted_hook = emitted.clone();
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };
    let hook =
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), "atlas".to_string())
            .with_output_hook(Some(Arc::new(move |event| {
                let emitted_hook = emitted_hook.clone();
                Box::pin(async move {
                    emitted_hook
                        .lock()
                        .expect("output block lock should not poison")
                        .push(event);
                })
            })));

    // Start stage without attached session (attached_session: false)
    hook.on_scheduler_stage_start(
        "atlas",
        "execution-orchestration",
        1,
        Some(&SchedulerStageCapabilities {
            skill_list: vec![],
            agents: vec![],
            categories: vec![],
            attached_session: false,
        }),
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_reasoning(
        "execution-orchestration",
        1,
        "main session reasoning",
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_content(
        "execution-orchestration",
        1,
        "main session streamed content",
        &exec_ctx,
    )
    .await;
    hook.on_scheduler_stage_end(
        "atlas",
        "execution-orchestration",
        1,
        1,
        "Final content",
        &exec_ctx,
    )
    .await;

    let emitted_blocks = emitted.lock().expect("emitted blocks").clone();
    let emitted_identities = emitted_blocks
        .iter()
        .filter(|event| event.session_id == session_id)
        .map(|event| event.live_identity.clone())
        .collect::<Vec<_>>();

    let session_blocks = emitted_blocks
        .iter()
        .filter(|event| event.session_id == session_id)
        .map(|event| &event.block)
        .collect::<Vec<_>>();
    let sequence = session_blocks
        .iter()
        .map(|block| match block {
            OutputBlock::Message(message) => match message.phase {
                agendao_output_blocks::MessagePhase::Start => "message:start",
                agendao_output_blocks::MessagePhase::Delta => "message:delta",
                agendao_output_blocks::MessagePhase::End => "message:end",
                agendao_output_blocks::MessagePhase::Full => "message:full",
            },
            OutputBlock::Reasoning(reasoning) => match reasoning.phase {
                agendao_output_blocks::MessagePhase::Start => "reasoning:start",
                agendao_output_blocks::MessagePhase::Delta => "reasoning:delta",
                agendao_output_blocks::MessagePhase::End => "reasoning:end",
                agendao_output_blocks::MessagePhase::Full => "reasoning:full",
            },
            OutputBlock::SchedulerStage(_) => "scheduler_stage",
            _ => "other",
        })
        .collect::<Vec<_>>();

    assert_eq!(
        sequence,
        vec![
            "scheduler_stage",
            "reasoning:start",
            "reasoning:delta",
            "message:delta",
            "scheduler_stage",
            "reasoning:end",
        ]
    );
    assert!(session_blocks.iter().any(|block| matches!(
        block,
        OutputBlock::Message(message)
            if *message
                == MessageBlock::delta(
                    OutputMessageRole::Assistant,
                    "main session streamed content",
                )
    )));
    assert!(emitted_identities.iter().any(|identity| matches!(
        identity,
        Some(live_identity)
            if live_identity.part_key == agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY
                && live_identity.phase == agendao_types::LivePartPhase::Append
    )));
    assert!(emitted_identities.iter().any(|identity| matches!(
        identity,
        Some(live_identity)
            if live_identity.part_key == agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY
                && live_identity.phase == agendao_types::LivePartPhase::Append
    )));
}

#[tokio::test]
async fn lifecycle_hook_tracks_active_stage_capabilities_from_tool_result_metadata() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };
    let hook =
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), "atlas".to_string());

    hook.on_scheduler_stage_start(
        "atlas",
        "execution-orchestration",
        2,
        Some(&SchedulerStageCapabilities {
            skill_list: vec![
                "debug".to_string().into(),
                "frontend-ui-ux".to_string().into(),
            ],
            agents: vec!["build".to_string(), "explore".to_string()],
            categories: vec!["frontend".to_string()],
            attached_session: false,
        }),
        &exec_ctx,
    )
    .await;
    hook.on_tool_end(
        "atlas",
        "tc_task_flow_2",
        "task_flow",
        &OrchestratorToolOutput {
            output: "delegated".to_string(),
            is_error: false,
            title: None,
            metadata: Some(serde_json::json!({
                "delegated": true,
                "loadedSkills": ["frontend-ui-ux"],
                "task": {
                    "agent": "build"
                }
            })),
        },
        &exec_ctx,
    )
    .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_active_agents")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("build")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_active_skills")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("frontend-ui-ux")
    );
}

#[tokio::test]
async fn request_active_scheduler_stage_abort_marks_stage_cancelling() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );

    hook.on_scheduler_stage_start("prometheus", "plan", 2, None, &exec_ctx)
        .await;

    let info = request_active_scheduler_stage_abort(&state, &session_id)
        .await
        .expect("abort info should exist");
    assert_eq!(info.stage_name.as_deref(), Some("plan"));

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_status")
            .and_then(|value| value.as_str()),
        Some("cancelling")
    );
}

#[tokio::test]
async fn finalize_active_scheduler_stage_cancelled_marks_terminal_status() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "prometheus".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );

    hook.on_scheduler_stage_start("prometheus", "interview", 1, None, &exec_ctx)
        .await;
    request_active_scheduler_stage_abort(&state, &session_id).await;
    let info = finalize_active_scheduler_stage_cancelled(&state, &session_id)
        .await
        .expect("cancel info should exist");
    assert_eq!(info.stage_name.as_deref(), Some("interview"));

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_stage_status")
            .and_then(|value| value.as_str()),
        Some("cancelled")
    );
    assert!(!message.metadata.contains_key("scheduler_stage_streaming"));
}

#[tokio::test]
async fn route_stage_decision_is_normalized_into_metadata() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "router".to_string(),
        metadata: HashMap::new(),
    };
    let hook = SessionSchedulerLifecycleHook::new(
        state.clone(),
        session_id.clone(),
        "prometheus".to_string(),
    );

    hook.on_scheduler_stage_start("prometheus", "route", 1, None, &exec_ctx)
        .await;
    hook.on_scheduler_stage_end(
            "prometheus",
            "route",
            1,
            4,
            r#"{"mode":"orchestrate","preset":"prometheus","insert_plan_stage":false,"review_mode":"normal","rationale_summary":"planner workflow required"}"#,
            &exec_ctx,
        )
        .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_decision_kind")
            .and_then(|value| value.as_str()),
        Some("route")
    );
    let fields = message
        .metadata
        .get("scheduler_decision_fields")
        .and_then(|value| value.as_array())
        .expect("decision fields should exist");
    assert!(fields.iter().any(|field| {
        field.get("label").and_then(|value| value.as_str()) == Some("Outcome")
            && field.get("value").and_then(|value| value.as_str()) == Some("Orchestrate")
    }));
}

#[tokio::test]
async fn gate_stage_decision_is_normalized_into_metadata() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", ".").id.clone()
    };
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: ".".to_string(),
        agent_name: "atlas".to_string(),
        metadata: HashMap::new(),
    };
    let hook =
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), "atlas".to_string());

    hook.on_scheduler_stage_start("atlas", "coordination-gate", 2, None, &exec_ctx)
        .await;
    hook.on_scheduler_stage_end(
            "atlas",
            "coordination-gate",
            2,
            3,
            r#"{"status":"continue","summary":"Task B still lacks evidence.","next_input":"Run one more worker round on task B."}"#,
            &exec_ctx,
        )
        .await;

    let sessions = state.sessions.lock().await;
    let session = sessions.get(&session_id).expect("session should exist");
    let message = session
        .record()
        .messages
        .last()
        .expect("stage message should exist");
    assert_eq!(
        message
            .metadata
            .get("scheduler_gate_status")
            .and_then(|value| value.as_str()),
        Some("continue")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_gate_summary")
            .and_then(|value| value.as_str()),
        Some("Task B still lacks evidence.")
    );
    assert_eq!(
        message
            .metadata
            .get("scheduler_gate_next_input")
            .and_then(|value| value.as_str()),
        Some("Run one more worker round on task B.")
    );
}

#[tokio::test]
async fn ensure_default_session_title_updates_default_title_only() {
    let mut session = Session::new("project", ".");
    session.add_user_message("Fix the scheduler event flow");
    ensure_default_session_title(
        &mut session,
        Arc::new(MockProvider {
            title: "Scheduler Event Flow".to_string(),
        }),
        "mock-model",
    )
    .await;
    assert_eq!(session.record().title, "Scheduler Event Flow");

    let mut auto_named = Session::new("project", ".");
    auto_named.add_user_message("Fix the scheduler event flow");
    auto_named.set_auto_title("Fix the scheduler event flow");
    ensure_default_session_title(
        &mut auto_named,
        Arc::new(MockProvider {
            title: "Refined Scheduler Title".to_string(),
        }),
        "mock-model",
    )
    .await;
    assert_eq!(auto_named.title, "Refined Scheduler Title");

    let mut named = Session::new("project", ".");
    named.set_title("Pinned Title");
    named.add_user_message("Ignored input");
    ensure_default_session_title(
        &mut named,
        Arc::new(MockProvider {
            title: "Should Not Replace".to_string(),
        }),
        "mock-model",
    )
    .await;
    assert_eq!(named.title, "Pinned Title");

    let mut legacy_buggy = Session::new("project", ".");
    legacy_buggy.add_user_message("Fix the scheduler event flow");
    legacy_buggy.set_title("Fix the scheduler event flow");
    legacy_buggy
        .add_assistant_message()
        .add_text("Implemented a proper session title refresh after the first completed turn.");
    ensure_default_session_title(
        &mut legacy_buggy,
        Arc::new(MockProvider {
            title: "Refresh Session Titles After First Turn".to_string(),
        }),
        "mock-model",
    )
    .await;
    assert_eq!(
        legacy_buggy.title,
        "Refresh Session Titles After First Turn"
    );
}
