use super::*;
use agendao_command::{CommandArgumentOption, CommandRegistry, CommandSource};
use agendao_multimodal::{
    ModalityPreflightResult, ModalitySupportView, ModalityTransportResult,
    MultimodalDisplaySummary, PreflightCapabilityView, RuntimeMultimodalExplain,
};
use agendao_orchestrator::output_projection::{
    SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY, SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY,
    SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY,
};
use agendao_session::{IngressSource, PartType, Session, SessionStateManager};
use std::sync::Arc;
use tokio::sync::RwLock;

fn test_prompt_runner() -> agendao_session::SessionPrompt {
    agendao_session::SessionPrompt::new(Arc::new(RwLock::new(SessionStateManager::new())))
}

fn text_parts(message: &agendao_session::SessionMessage) -> Vec<&str> {
    message
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn run_started_refresh_preserves_the_new_user_message_for_recovery() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/project").id.clone()
    };
    crate::session_runtime::task_ledger::apply_task_ledger_op(
        &state,
        &session_id,
        0,
        agendao_types::task_ledger::TaskLedgerOp::Create {
            goal: agendao_types::task_ledger::TaskGoal {
                statement: "finish after recovery".to_string(),
                acceptance_criteria: vec![],
                criterion_checks: vec![],
                set_by: agendao_types::task_ledger::TaskLedgerActor::User,
                set_at: 1,
            },
            next_statement: "run the task".to_string(),
        },
    )
    .await
    .expect("create ledger");
    crate::session_runtime::task_ledger_reducer::dispatch_seam(
        &state,
        &session_id,
        agendao_types::task_ledger::TaskLedgerSeamFact::RecoveryInterrupted,
    )
    .await;

    let mut local = state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .expect("session")
        .clone();
    local.add_user_message("original request that must survive abort");

    commit_scheduler_input_and_start_ledger_run(&state, &session_id, &mut local).await;

    let stored = state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .expect("session")
        .clone();
    assert_eq!(
        crate::recovery::latest_user_prompt(&stored).as_deref(),
        Some("original request that must survive abort")
    );
    let ledger = crate::session_runtime::task_ledger::ledger_snapshot_from_record(
        &session_id,
        stored
            .record()
            .metadata
            .get(crate::session_runtime::task_ledger::TASK_LEDGER_METADATA_KEY),
    );
    assert_eq!(
        ledger.status,
        agendao_types::task_ledger::TaskLedgerStatus::Active
    );
    assert!(local
        .messages
        .iter()
        .any(|message| message.get_text() == "original request that must survive abort"));
}

#[test]
fn scheduler_choice_defaults_to_auto_without_an_explicit_agent() {
    assert_eq!(
        super::super::scheduler::resolve_effective_scheduler_choice(None, None, false),
        agendao_orchestrator::selector::SchedulerChoice::Auto
    );
}

#[test]
fn explicit_agent_uses_direct_scheduler_and_explicit_scheduler_is_preserved() {
    assert_eq!(
        super::super::scheduler::resolve_effective_scheduler_choice(None, None, true),
        agendao_orchestrator::selector::SchedulerChoice::Template {
            template: agendao_orchestrator::templates::TemplateId::Direct,
        }
    );

    let explicit = agendao_orchestrator::selector::SchedulerChoice::Template {
        template: agendao_orchestrator::templates::TemplateId::Verify,
    };
    assert_eq!(
        super::super::scheduler::resolve_effective_scheduler_choice(
            None,
            Some(explicit.clone()),
            false,
        ),
        explicit
    );
}

fn test_auto_ledger(
    status: agendao_types::task_ledger::TaskLedgerStatus,
) -> agendao_types::task_ledger::SessionTaskLedger {
    let mut ledger = agendao_types::task_ledger::SessionTaskLedger::empty("auto");
    ledger.goal_generation = 1;
    ledger.revision = 1;
    ledger.goal = Some(agendao_types::task_ledger::TaskGoal {
        statement: "finish the task".to_string(),
        acceptance_criteria: Vec::new(),
        criterion_checks: Vec::new(),
        set_by: agendao_types::task_ledger::TaskLedgerActor::User,
        set_at: 1,
    });
    ledger.next = Some(agendao_types::task_ledger::NextAction {
        statement: "do the next thing".to_string(),
        provenance: agendao_types::task_ledger::NextActionProvenance {
            actor: agendao_types::task_ledger::TaskLedgerActor::Model,
            pre_interrupt: false,
            set_at: 1,
        },
    });
    ledger.status = status;
    ledger
}

#[test]
fn task_ledger_auto_continuation_stops_for_terminal_or_user_states() {
    assert!(matches!(
        plan_task_ledger_auto_continuation(
            None,
            &test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Completed),
            false
        ),
        AutoContinuationPlan::Stop(AutoContinuationStop::Completed)
    ));
    assert!(matches!(
        plan_task_ledger_auto_continuation(
            None,
            &test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::AwaitingUser),
            false
        ),
        AutoContinuationPlan::Stop(AutoContinuationStop::AwaitingUser)
    ));
    assert!(matches!(
        plan_task_ledger_auto_continuation(
            None,
            &test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Blocked),
            false
        ),
        AutoContinuationPlan::Stop(AutoContinuationStop::Blocked)
    ));
    assert!(matches!(
        plan_task_ledger_auto_continuation(
            None,
            &test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Active),
            true
        ),
        AutoContinuationPlan::Stop(AutoContinuationStop::Cancelled)
    ));
}

#[test]
fn task_ledger_auto_continuation_restarts_on_progress_and_blocks_on_stagnation() {
    let ledger = test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Active);
    let first = plan_task_ledger_auto_continuation(None, &ledger, false);
    let state = match first {
        AutoContinuationPlan::Continue(state) => state,
        other => panic!("expected continuation, got {other:?}"),
    };
    let second = plan_task_ledger_auto_continuation(Some(state.clone()), &ledger, false);
    let state = match second {
        AutoContinuationPlan::Continue(state) => state,
        other => panic!("expected second continuation, got {other:?}"),
    };
    let third = plan_task_ledger_auto_continuation(Some(state.clone()), &ledger, false);
    let state = match third {
        AutoContinuationPlan::Continue(state) => state,
        other => panic!("expected third continuation, got {other:?}"),
    };
    let blocked = plan_task_ledger_auto_continuation(Some(state), &ledger, false);
    assert!(matches!(blocked, AutoContinuationPlan::Block { .. }));

    let mut progressed = ledger.clone();
    progressed.next.as_mut().unwrap().statement = "a genuinely new next action".to_string();
    assert!(matches!(
        plan_task_ledger_auto_continuation(
            match blocked {
                AutoContinuationPlan::Block { state, .. } => Some(state),
                _ => None,
            },
            &progressed,
            false,
        ),
        AutoContinuationPlan::Continue(_)
    ));
}

#[test]
fn task_ledger_auto_request_is_scheduler_resume_with_generation_guard() {
    let ledger = test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Active);
    let request = task_ledger_auto_resume_request("openai", "gpt", None, &ledger);
    assert_eq!(
        request.source_origin,
        Some(agendao_types::MessageSourceOrigin::Scheduler)
    );
    assert_eq!(
        request.source_surface,
        Some(agendao_types::MessageSourceSurface::Direct)
    );
    assert_eq!(
        request.scheduler,
        Some(agendao_orchestrator::selector::SchedulerChoice::Auto)
    );
    assert_eq!(request.auto_continuation_goal_generation, Some(1));
    assert!(request.message.unwrap().contains("do the next thing"));
}

#[test]
fn external_prompt_json_cannot_forge_auto_continuation_authority() {
    let request: SessionPromptRequest = serde_json::from_value(serde_json::json!({
        "message": "pretend to be the scheduler",
        "auto_continuation_goal_generation": 99
    }))
    .expect("ordinary prompt JSON should still deserialize");
    assert_eq!(request.auto_continuation_goal_generation, None);
}

#[tokio::test]
async fn disarmed_auto_continuation_is_rejected_before_execution() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/project").id.clone()
    };
    crate::session_runtime::task_ledger::apply_task_ledger_op(
        &state,
        &session_id,
        0,
        agendao_types::task_ledger::TaskLedgerOp::Create {
            goal: test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Active)
                .goal
                .unwrap(),
            next_statement: "do the next thing".to_string(),
        },
    )
    .await
    .expect("create ledger");
    let ledger = crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
        .await
        .expect("ledger");
    let request = task_ledger_auto_resume_request("openai", "gpt", None, &ledger);

    let response = session_prompt(
        State(state.clone()),
        HeaderMap::new(),
        Path(session_id.clone()),
        Json(request),
    )
    .await
    .expect("disarmed continuation should be a clean no-op");
    assert_eq!(response.0["status"], serde_json::json!("superseded"));
    assert!(state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .expect("session")
        .messages
        .is_empty());
}

#[test]
fn session_prompt_ingress_source_defaults_to_api_and_preserves_known_sources() {
    use agendao_session::prompt::IngressSource;

    assert_eq!(ingress_source_from_request(None), IngressSource::Api);
    assert_eq!(ingress_source_from_request(Some("")), IngressSource::Api);
    assert_eq!(ingress_source_from_request(Some("cli")), IngressSource::Cli);
    assert_eq!(ingress_source_from_request(Some("TUI")), IngressSource::Tui);
    assert_eq!(ingress_source_from_request(Some("web")), IngressSource::Web);
    assert_eq!(
        ingress_source_from_request(Some("scheduler")),
        IngressSource::Scheduler
    );
    assert_eq!(
        ingress_source_from_request(Some("feishu")),
        IngressSource::Other("feishu".to_string())
    );
}

#[test]
fn build_ingress_envelope_uses_entry_metadata_contract() {
    let ingress = build_ingress_envelope(
        "ses_1",
        ingress_source_from_request(None),
        "hello",
        Some("idem_1".to_string()),
        Some("session_prompt".to_string()),
    );

    assert_eq!(ingress.source, agendao_session::prompt::IngressSource::Api);
    assert_eq!(ingress.context_key.as_deref(), Some("session_prompt"));
    assert_eq!(ingress.idempotency_key.as_deref(), Some("idem_1"));
    assert_eq!(
        ingress.stabilization.policy,
        agendao_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY
    );
}

fn unresolved_prompt_payload(text: &str) -> ResolvedPromptPayload {
    ResolvedPromptPayload {
        display_text: text.to_string(),
        execution_text: text.to_string(),
        agent: None,
        model: None,
        scheduler: None,
        command: None,
        pending_raw_arguments: None,
    }
}

fn prompt_request_message(text: &str) -> SessionPromptRequest {
    SessionPromptRequest {
        message: Some(text.to_string()),
        parts: None,
        idempotency_key: None,
        ingress_source: None,
        model: None,
        variant: None,
        agent: None,
        scheduler: None,
        command: None,
        arguments: None,
        recovery: None,
        source_origin: None,
        source_surface: None,
        auto_continuation_goal_generation: None,
    }
}

fn sample_external_ingress(session_id: &str) -> agendao_session::prompt::IngressTurnEnvelope {
    let event = agendao_types::ExternalAdapterEvent {
        adapter_id: "generic".to_string(),
        source: agendao_types::ExternalAdapterSource::GenericWebhook,
        external_event_id: "evt_1".to_string(),
        external_user_id: "user_1".to_string(),
        external_conversation_id: "chat_1".to_string(),
        external_thread_id: None,
        received_at_ms: 1_714_000_000_000,
        text: "hello from webhook".to_string(),
        attachments: Vec::new(),
        idempotency_key: None,
        reply_target: None,
        raw_event_ref: None,
    };
    agendao_session::prompt::external_adapter_event_to_ingress_turn(session_id, &event)
        .expect("external adapter event should map to ingress")
}

#[test]
fn task_ingress_for_prompt_preserves_verified_external_adapter_ingress() {
    let verified_ingress = sample_external_ingress("ses_1");
    let request = prompt_request_message("hello from webhook");
    let resolved = unresolved_prompt_payload("hello from webhook");

    let ingress = task_ingress_for_prompt(
        "ses_1",
        "hello from webhook",
        &request,
        &resolved,
        Some(verified_ingress),
    )
    .unwrap();

    assert_eq!(
        ingress.source,
        agendao_session::prompt::IngressSource::Other(
            "external:generic-webhook:generic".to_string()
        )
    );
    assert_eq!(
        ingress.stabilization.policy,
        agendao_session::prompt::INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY
    );
    assert!(ingress.external_adapter.is_some());
    assert_ne!(ingress.context_key.as_deref(), Some("session_prompt"));
}

#[test]
fn task_ingress_for_prompt_rejects_verified_ingress_for_other_session() {
    let verified_ingress = sample_external_ingress("ses_other");
    let request = prompt_request_message("hello from webhook");
    let resolved = unresolved_prompt_payload("hello from webhook");

    let error = task_ingress_for_prompt(
        "ses_1",
        "hello from webhook",
        &request,
        &resolved,
        Some(verified_ingress),
    )
    .unwrap_err();

    assert!(matches!(error, ApiError::BadRequest(_)));
}

#[test]
fn task_ingress_for_prompt_still_builds_http_entry_ingress_when_unset() {
    let mut request = prompt_request_message("hello");
    request.idempotency_key = Some("idem_1".to_string());
    request.ingress_source = Some("api".to_string());
    let resolved = unresolved_prompt_payload("hello");

    let ingress = task_ingress_for_prompt("ses_1", "hello", &request, &resolved, None).unwrap();

    assert_eq!(ingress.source, agendao_session::prompt::IngressSource::Api);
    assert_eq!(ingress.context_key.as_deref(), Some("session_prompt"));
    assert_eq!(ingress.idempotency_key.as_deref(), Some("idem_1"));
    assert!(ingress.external_adapter.is_none());
}

#[tokio::test]
async fn followup_queue_preserves_fifo_order_and_tracks_runtime_count() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/project").id.clone()
    };

    for (index, message) in ["first", "second", "third"].iter().enumerate() {
        let queued_count = enqueue_followup_prompt(
            &state,
            &session_id,
            QueuedFollowupPrompt {
                request: prompt_request_message(message),
                apply_plugin_config_hooks: true,
                auto_continuation_goal_generation: None,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{message} should queue: {error}"));
        assert_eq!(queued_count, index as u64 + 1);
    }
    assert_eq!(
        state
            .runtime_telemetry
            .runtime_state()
            .get(&session_id)
            .await
            .expect("runtime state should exist")
            .pending_followup_count,
        3
    );

    let adopted = take_followup_prompt(&state, &session_id)
        .await
        .expect("first queued follow-up should be adoptable");
    assert_eq!(adopted.request.message.as_deref(), Some("first"));
    assert_eq!(
        state
            .runtime_telemetry
            .runtime_state()
            .get(&session_id)
            .await
            .expect("runtime state should exist")
            .pending_followup_count,
        2
    );

    let second = take_followup_prompt(&state, &session_id)
        .await
        .expect("second queued follow-up should be adoptable");
    assert_eq!(second.request.message.as_deref(), Some("second"));

    let dropped = drain_followup_prompts(&state, &session_id).await;
    assert_eq!(dropped, 1);
    assert_eq!(
        state
            .runtime_telemetry
            .runtime_state()
            .get(&session_id)
            .await
            .expect("runtime state should exist")
            .pending_followup_count,
        0
    );
    assert!(take_followup_prompt(&state, &session_id).await.is_none());
}

#[tokio::test]
async fn human_followup_is_adopted_before_queued_auto_continuation() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/project").id.clone()
    };
    let ledger = test_auto_ledger(agendao_types::task_ledger::TaskLedgerStatus::Active);
    enqueue_followup_prompt(
        &state,
        &session_id,
        QueuedFollowupPrompt {
            request: task_ledger_auto_resume_request("openai", "gpt", None, &ledger),
            apply_plugin_config_hooks: false,
            auto_continuation_goal_generation: Some(ledger.goal_generation),
        },
    )
    .await
    .expect("auto continuation should queue");
    enqueue_followup_prompt(
        &state,
        &session_id,
        QueuedFollowupPrompt {
            request: prompt_request_message("human correction"),
            apply_plugin_config_hooks: true,
            auto_continuation_goal_generation: None,
        },
    )
    .await
    .expect("human follow-up should queue");

    let adopted = take_followup_prompt(&state, &session_id)
        .await
        .expect("one prompt should be adopted");
    assert_eq!(adopted.request.message.as_deref(), Some("human correction"));
    assert!(!is_task_ledger_auto_continuation(&adopted.request));

    let remaining = take_followup_prompt(&state, &session_id)
        .await
        .expect("auto continuation should remain queued");
    assert!(is_task_ledger_auto_continuation(&remaining.request));
}

#[tokio::test]
async fn aborting_a_session_drops_queued_followups() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/project").id.clone()
    };

    for message in ["queued-one", "queued-two"] {
        enqueue_followup_prompt(
            &state,
            &session_id,
            QueuedFollowupPrompt {
                request: prompt_request_message(message),
                apply_plugin_config_hooks: true,
                auto_continuation_goal_generation: None,
            },
        )
        .await
        .expect("follow-up should queue");
    }

    let response = super::super::cancel::abort_session_execution(&state, &session_id).await;
    assert_eq!(response["dropped_queued_prompts"], serde_json::json!(2));
    assert!(take_followup_prompt(&state, &session_id).await.is_none());
    assert_eq!(
        state
            .runtime_telemetry
            .runtime_state()
            .get(&session_id)
            .await
            .expect("runtime state should exist")
            .pending_followup_count,
        0
    );
}

#[tokio::test]
async fn abort_clears_pending_question_and_awaiting_ledger_after_run_token_retires() {
    let state = Arc::new(ServerState::new());
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        sessions.create("project", "/tmp/project").id.clone()
    };
    crate::session_runtime::task_ledger::apply_task_ledger_op(
        &state,
        &session_id,
        0,
        agendao_types::task_ledger::TaskLedgerOp::Create {
            goal: agendao_types::task_ledger::TaskGoal {
                statement: "answer then resume".to_string(),
                acceptance_criteria: vec![],
                criterion_checks: vec![],
                set_by: agendao_types::task_ledger::TaskLedgerActor::User,
                set_at: 1,
            },
            next_statement: "wait for answer".to_string(),
        },
    )
    .await
    .expect("create ledger");
    let (question, _waiter) = state
        .runtime_telemetry
        .register_question(
            session_id.clone(),
            vec![agendao_tool::QuestionDef {
                question: "continue?".to_string(),
                header: None,
                options: vec![],
                multiple: false,
            }],
        )
        .await;
    crate::session_runtime::task_ledger_reducer::dispatch_seam(
        &state,
        &session_id,
        agendao_types::task_ledger::TaskLedgerSeamFact::InteractionAwaiting {
            kind: agendao_types::task_ledger::AwaitingInteractionKind::Question,
            interaction_id: question.id,
        },
    )
    .await;

    let response = super::super::cancel::abort_session_execution(&state, &session_id).await;
    assert_eq!(response["aborted"], serde_json::json!(true));
    assert_eq!(
        response["cancelled_pending_questions"],
        serde_json::json!(1)
    );
    assert!(state
        .runtime_telemetry
        .list_questions_for_session(&session_id)
        .await
        .is_empty());
    let ledger = crate::session_runtime::task_ledger::task_ledger_snapshot(&state, &session_id)
        .await
        .expect("ledger");
    assert_eq!(
        ledger.status,
        agendao_types::task_ledger::TaskLedgerStatus::Interrupted
    );
    assert!(ledger.awaiting_interactions.is_empty());
    assert!(
        ledger
            .next
            .as_ref()
            .expect("pre-interrupt next")
            .provenance
            .pre_interrupt
    );
}

#[test]
fn live_web_ingress_batch_merges_parts_and_uses_stabilized_ingress() {
    let mut session = Session::new("project", "/tmp");
    let now_ms = 1_000;
    let mut first = build_ingress_envelope(
        &session.id,
        IngressSource::Web,
        "first",
        Some("web_1".to_string()),
        Some("session_prompt".to_string()),
    );
    first.received_at_ms = now_ms;
    first.stabilized_at_ms = now_ms;

    let mut second = build_ingress_envelope(
        &session.id,
        IngressSource::Web,
        "second",
        Some("web_2".to_string()),
        Some("session_prompt".to_string()),
    );
    second.received_at_ms = now_ms + 10;
    second.stabilized_at_ms = now_ms + 10;

    let owner = live_web_ingress::open(
        &mut session,
        first,
        vec![agendao_session::prompt::PartInput::Text {
            text: "first".to_string(),
        }],
        now_ms,
    )
    .expect("leader batch should open");
    assert!(live_web_ingress::append_if_present(
        &mut session,
        second,
        vec![agendao_session::prompt::PartInput::Text {
            text: "second".to_string(),
        }],
        now_ms + 10,
    ));

    let batch = drain_live_web_ingress_batch(&mut session, &owner).expect("batch should drain");
    let (ingress, parts) =
        resolve_live_web_ingress_batch(batch).expect("batch should resolve to one turn");

    assert_eq!(
        ingress.stabilization.policy,
        agendao_session::prompt::INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH
    );
    let rendered = parts
        .iter()
        .filter_map(|part| match part {
            agendao_session::prompt::PartInput::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rendered, vec!["first", "second"]);
}

#[test]
fn live_web_ingress_batch_does_not_accept_command_turns() {
    let mut session = Session::new("project", "/tmp");
    let now_ms = 1_000;
    let mut ingress = build_ingress_envelope(
        &session.id,
        IngressSource::Web,
        "/new",
        Some("web_cmd".to_string()),
        Some("session_prompt".to_string()),
    );
    ingress.command = Some("new".to_string());

    assert!(!live_web_ingress::append_if_present(
        &mut session,
        ingress.clone(),
        vec![agendao_session::prompt::PartInput::Text {
            text: "/new".to_string(),
        }],
        now_ms,
    ));
    assert!(live_web_ingress::open(
        &mut session,
        ingress,
        vec![agendao_session::prompt::PartInput::Text {
            text: "/new".to_string(),
        }],
        now_ms,
    )
    .is_none());
}

#[test]
fn scheduler_session_context_carries_recent_turns() {
    let mut session = Session::new("project", "/tmp");
    session.set_title("Martini3 antibody formulation research");
    session.add_user_message("检索近年来 martini3 在抗体制剂开发中的研究");
    {
        let assistant = session.add_assistant_message();
        assistant.add_text("Found papers A, B, and C with notes about antibody formulation.");
    }
    let block = build_scheduler_session_context_block(&session)
        .expect("same-session scheduler context should render");

    assert!(block.contains("## Session Continuity Context"));
    assert!(block.contains("## Context Coverage"));
    assert!(block.contains("## Hydration Guidance"));
    assert!(block.contains("scheduler_context_hydrate"));
    assert!(block.contains("Martini3 antibody formulation research"));
    assert!(block.contains("Found papers A, B, and C"));
    assert!(block.contains("exact_tail_message_ids"));
}

#[test]
fn scheduler_session_context_uses_projection_summary_for_projected_assistant_output() {
    let mut session = Session::new("project", "/tmp");
    session.add_user_message("检索 AlphaFold3 方法学研究");
    let assistant_id = {
        let assistant = session.add_assistant_message();
        assistant.add_text("full report body that should not be placed in scheduler context");
        assistant.metadata.insert(
            SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
            serde_json::json!("OnDemandArtifact"),
        );
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!(
                "Large assistant output stored as artifact `art_assistant_test`. Summary:\nAlphaFold3 methodology survey summary"
            ),
        );
        assistant.metadata.insert(
            SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY.to_string(),
            serde_json::json!([{"id": "art_assistant_test"}]),
        );
        assistant.id.clone()
    };

    let block = build_scheduler_session_context_block(&session)
        .expect("same-session scheduler context should render");
    let packet = build_scheduler_session_context_packet(&session)
        .expect("same-session scheduler context packet should render");
    let metadata = packet.metadata_value();

    assert!(block.contains("Projected assistant output for model context"));
    assert!(block.contains("AlphaFold3 methodology survey summary"));
    assert!(block.contains(&assistant_id));
    assert!(!block.contains("full report body that should not be placed"));
    assert_eq!(
        metadata["exact_recent_tail"][1]["projected"],
        serde_json::json!(true)
    );
}

#[test]
fn scheduler_session_context_rejects_unsanctioned_full_projection_policy() {
    let mut session = Session::new("project", "/tmp");
    session.add_user_message("总结这次调查");
    let assistant_id = {
        let assistant = session.add_assistant_message();
        assistant.add_text("full report body should remain the scheduler context source");
        assistant.metadata.insert(
            SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
            serde_json::json!("Full"),
        );
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("summary must not override full projection"),
        );
        assistant.id.clone()
    };

    let block = build_scheduler_session_context_block(&session)
        .expect("same-session scheduler context should render");
    let packet = build_scheduler_session_context_packet(&session)
        .expect("same-session scheduler context packet should render");
    let metadata = packet.metadata_value();

    assert!(!block.contains("Projected assistant output for model context"));
    assert!(block.contains("full report body should remain the scheduler context source"));
    assert!(!block.contains("summary must not override full projection"));
    assert_eq!(
        metadata["exact_recent_tail"][1]["projected"],
        serde_json::json!(false)
    );
    assert_eq!(
        metadata["exact_recent_tail"][1]["message_id"].as_str(),
        Some(assistant_id.as_str())
    );
}

#[test]
fn scheduler_session_context_reports_recent_tail_coverage() {
    let mut session = Session::new("project", "/tmp");
    for index in 0..8 {
        session.add_user_message(format!("turn {index}"));
    }

    let block = build_scheduler_session_context_block(&session)
        .expect("same-session scheduler context should render");

    assert!(block.contains("exact_recent_tail: last 6 of 8 eligible"));
    assert!(block.contains("omitted_older_turns: 2"));
    assert!(!block.contains("turn 0"));
    assert!(!block.contains("turn 1"));
    assert!(block.contains("turn 7"));
}

#[test]
fn scheduler_session_context_anchors_compaction_summary() {
    let mut session = Session::new("project", "/tmp");
    session.add_user_message("earlier research request");
    let compaction_id = {
        let summary = session.add_assistant_message();
        summary
            .metadata
            .insert("summary".to_string(), serde_json::json!(true));
        summary.add_text("Compacted research findings about Martini3 antibodies.");
        summary.id.clone()
    };

    let block = build_scheduler_session_context_block(&session)
        .expect("same-session scheduler context should render");

    assert!(block.contains("## Latest Compaction Summary"));
    assert!(block.contains(&format!("source: assistant `{compaction_id}`")));
    assert!(block.contains(&format!("compaction_summary_message_id: `{compaction_id}`")));
}

#[test]
fn scheduler_session_context_packet_metadata_names_hydration_policy() {
    let mut session = Session::new("project", "/tmp");
    session.add_user_message("first request");

    let packet = build_scheduler_session_context_packet(&session)
        .expect("same-session scheduler context packet should render");
    let metadata = packet.metadata_value();

    assert_eq!(metadata["version"], serde_json::json!(1));
    assert!(metadata["recall_policy"]
        .as_str()
        .expect("recall policy should be present")
        .contains("use_scheduler_context_hydrate"));
}

#[test]
fn scheduler_session_context_carries_memory_anchors_from_last_prefetch() {
    let mut session = Session::new("project", "/tmp");
    session.insert_metadata(
        MEMORY_LAST_PREFETCH_METADATA_KEY.to_string(),
        serde_json::to_value(MemoryRetrievalPacket {
            generated_at: 42,
            snapshot: false,
            query: Some("follow up".to_string()),
            scopes: vec![agendao_types::MemoryScope::SessionEphemeral],
            items: vec![agendao_types::MemoryRecallView {
                card: agendao_types::MemoryCardView {
                    id: agendao_types::MemoryRecordId("mem_123".to_string()),
                    kind: agendao_types::MemoryKind::Lesson,
                    scope: agendao_types::MemoryScope::SessionEphemeral,
                    status: agendao_types::MemoryStatus::Validated,
                    title: "Prior Martini3 bibliography decision".to_string(),
                    summary: "Use the saved paper shortlist.".to_string(),
                    derived_skill_name: None,
                    linked_skill_name: None,
                    confidence: Some(0.9),
                    validation_status: agendao_types::MemoryValidationStatus::Passed,
                    last_validated_at: None,
                },
                why_recalled: "query matched Martini3 follow-up".to_string(),
                evidence_summary: None,
            }],
            note: None,
            budget_limit: Some(6),
        })
        .expect("memory packet should serialize"),
    );

    let packet = build_scheduler_session_context_packet(&session)
        .expect("memory anchors alone should render scheduler context");
    let block = packet.render();
    let metadata = packet.metadata_value();

    assert!(block.contains("## Memory Anchors"));
    assert!(block.contains("mem_123"));
    assert!(block.contains("Prior Martini3 bibliography decision"));
    assert_eq!(metadata["memory_anchors"][0]["record_id"], "mem_123");
    assert_eq!(metadata["memory_anchors"][0]["status"], "Validated");
}

#[test]
fn scheduler_session_context_packet_metadata_is_typed_authority() {
    let mut session = Session::new("project", "/tmp");
    let first_id = session.add_user_message("first request").id.clone();
    let second_id = {
        let message = session.add_assistant_message();
        message.add_text("first answer body that should stay in the continuity packet");
        message.id.clone()
    };

    let packet = build_scheduler_session_context_packet(&session)
        .expect("same-session scheduler context packet should render");
    let metadata = packet.metadata_value();
    let restored = SessionContinuityPacket::from_value(&metadata)
        .expect("typed continuity packet should deserialize");

    assert_eq!(metadata["version"], serde_json::json!(1));
    assert_eq!(metadata["eligible_message_count"], serde_json::json!(2));
    assert_eq!(metadata["omitted_older_turns"], serde_json::json!(0));
    assert_eq!(metadata["exact_recent_tail"][0]["message_id"], first_id);
    assert_eq!(metadata["exact_recent_tail"][0]["role"], "user");
    assert_eq!(metadata["exact_recent_tail"][0]["text"], "first request");
    assert_eq!(metadata["exact_recent_tail"][1]["message_id"], second_id);
    assert_eq!(metadata["exact_recent_tail"][1]["role"], "assistant");
    assert_eq!(
        metadata["exact_recent_tail"][1]["text"],
        "first answer body that should stay in the continuity packet"
    );
    assert!(!metadata["working_ledger"]
        .as_array()
        .expect("working ledger should serialize")
        .is_empty());
    assert_eq!(restored.render(), packet.render());
}

#[test]
fn scheduler_continuity_packet_carries_task_ledger_without_second_prompt_projection() {
    let mut session = Session::new("project", "/tmp");
    let mut ledger = agendao_types::task_ledger::SessionTaskLedger::empty(&session.id);
    ledger
        .apply(
            0,
            agendao_types::task_ledger::TaskLedgerOp::Create {
                goal: agendao_types::task_ledger::TaskGoal {
                    statement: "resume the governed task".to_string(),
                    acceptance_criteria: vec!["check passes".to_string()],
                    criterion_checks: vec![],
                    set_by: agendao_types::task_ledger::TaskLedgerActor::User,
                    set_at: 1,
                },
                next_statement: "run the remaining check".to_string(),
            },
            2,
        )
        .expect("create ledger");
    session.insert_metadata(
        agendao_types::task_ledger::TASK_LEDGER_METADATA_KEY.to_string(),
        serde_json::to_value(&ledger).expect("serialize ledger"),
    );

    let packet = build_scheduler_session_context_packet(&session)
        .expect("task ledger alone should produce a continuity packet");
    let projected = packet
        .task_ledger
        .as_ref()
        .expect("typed task ledger continuity");
    assert_eq!(projected.revision, ledger.revision);
    assert_eq!(
        projected.next.as_ref().map(|next| next.statement.as_str()),
        Some("run the remaining check")
    );
    assert!(
        !packet.render().contains("resume the governed task"),
        "the continuity packet is audit metadata; live task-ledger projection has one separate injection point"
    );
}

#[test]
fn scheduler_session_context_keeps_source_anchors_when_truncated() {
    let mut session = Session::new("project", "/tmp");
    let mut latest_message_id = String::new();
    for index in 0..6 {
        let message = session.add_user_message(format!("turn {index} {}", "x".repeat(2_000)));
        latest_message_id = message.id.clone();
    }

    let block = build_scheduler_session_context_block(&session)
        .expect("same-session scheduler context should render");

    assert!(block.contains("## Source Anchors"));
    assert!(block.contains("## Hydration Guidance"));
    assert!(block.contains(&format!("`{latest_message_id}`")));
    assert!(block.contains("scheduler_context_hydrate"));
    assert!(block.contains("...[truncated]..."));
    assert!(block.chars().count() <= SCHEDULER_CONTEXT_TEXT_LIMIT);
}

#[test]
fn scheduler_prompt_merge_keeps_memory_before_current_prompt() {
    let merged = merge_scheduler_prompt_with_memory(
        "把你前面检索的结果写到 markdown 文档中",
        Some("Frozen Memory Snapshot:\n- preference"),
        Some("Turn Memory Recall:\n- related method"),
    );

    assert!(merged.contains("Frozen Memory Snapshot"));
    assert!(merged.contains("Turn Memory Recall"));
    assert!(merged.ends_with("把你前面检索的结果写到 markdown 文档中"));
}

#[tokio::test]
async fn scheduler_user_message_preserves_attachment_only_parts() {
    let prompt_runner = test_prompt_runner();
    let mut session = Session::new("project", "/tmp");
    let input = agendao_session::PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![agendao_session::PartInput::File {
            url: "data:text/plain;base64,SGVsbG8=".to_string(),
            filename: Some("note.txt".to_string()),
            mime: Some("text/plain".to_string()),
        }],
        tools: None,
        ingress: None,
    };

    let message_id = create_scheduler_user_message(
        &prompt_runner,
        &mut session,
        &input,
        SchedulerUserMessageContext {
            display_prompt_text: "[1 attachment]",
            resolved_user_prompt: "",
            choice: &agendao_orchestrator::selector::SchedulerChoice::Auto,
            recovery: None,
        },
    )
    .await
    .expect("scheduler attachment-only user message should be created");

    let message = session
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .expect("user message should exist");
    assert!(
        text_parts(message).contains(&"[1 attachment]"),
        "attachment-only scheduler prompt should retain a visible summary text part"
    );
    assert!(message.parts.iter().any(|part| matches!(
        &part.part_type,
        PartType::File { filename, mime, .. }
        if filename == "note.txt" && mime == "text/plain"
    )));
    assert_eq!(
        message.metadata.get("scheduler"),
        Some(&serde_json::json!({ "kind": "auto" }))
    );
}

#[tokio::test]
async fn scheduler_user_message_keeps_text_and_file_parts_together() {
    let prompt_runner = test_prompt_runner();
    let mut session = Session::new("project", "/tmp");
    let input = agendao_session::PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![
            agendao_session::PartInput::Text {
                text: "Inspect @note.txt".to_string(),
            },
            agendao_session::PartInput::File {
                url: "data:text/plain;base64,SGVsbG8=".to_string(),
                filename: Some("note.txt".to_string()),
                mime: Some("text/plain".to_string()),
            },
        ],
        tools: None,
        ingress: None,
    };

    let message_id = create_scheduler_user_message(
        &prompt_runner,
        &mut session,
        &input,
        SchedulerUserMessageContext {
            display_prompt_text: "Inspect @note.txt",
            resolved_user_prompt: "Inspect @note.txt",
            choice: &agendao_orchestrator::selector::SchedulerChoice::Auto,
            recovery: None,
        },
    )
    .await
    .expect("scheduler text+attachment user message should be created");

    let message = session
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .expect("user message should exist");
    assert!(
        text_parts(message).contains(&"Inspect @note.txt"),
        "scheduler prompt text should remain visible alongside attachment parts"
    );
    assert!(message.parts.iter().any(|part| matches!(
        &part.part_type,
        PartType::File { filename, .. } if filename == "note.txt"
    )));
    assert_eq!(
        message.metadata.get("resolved_user_prompt"),
        Some(&serde_json::json!("Inspect @note.txt"))
    );
}

#[test]
fn annotate_last_user_message_multimodal_metadata_persists_explain_fields() {
    let mut session = Session::new("project", "/tmp");
    session.add_user_message("[audio input]");

    annotate_last_user_message_multimodal_metadata(
        &mut session,
        &RuntimeMultimodalExplain {
            summary: MultimodalDisplaySummary {
                primary_text: String::new(),
                attachment_count: 1,
                badges: vec!["audio".to_string()],
                compact_label: "[audio input]".to_string(),
                kinds: vec!["audio".to_string()],
            },
            capability: PreflightCapabilityView {
                provider_id: "openai".to_string(),
                model_id: "gpt-audio".to_string(),
                attachment: true,
                tool_call: false,
                reasoning: false,
                temperature: true,
                input: ModalitySupportView {
                    text: true,
                    audio: true,
                    image: false,
                    video: false,
                    pdf: false,
                },
                output: ModalitySupportView {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
            },
            result: ModalityPreflightResult {
                warnings: vec!["Audio accepted.".to_string()],
                unsupported_parts: Vec::new(),
                recommended_downgrade: None,
                hard_block: false,
            },
            transport: ModalityTransportResult {
                replaced_parts: vec!["voice.wav".to_string()],
                warnings: vec![
                    "ERROR: Cannot read \"voice.wav\" (this model does not support audio input). Inform the user.".to_string(),
                ],
            },
            resolved_model: "openai/gpt-audio".to_string(),
        },
    );

    let message = session
        .messages
        .iter()
        .rfind(|message| matches!(message.role, agendao_session::MessageRole::User))
        .expect("user message should exist");

    assert_eq!(
        message
            .metadata
            .get("multimodal_resolved_model")
            .and_then(|value| value.as_str()),
        Some("openai/gpt-audio")
    );
    assert_eq!(
        message
            .metadata
            .get("multimodal_compact_label")
            .and_then(|value| value.as_str()),
        Some("[audio input]")
    );
    assert_eq!(
        message
            .metadata
            .get("multimodal_attachment_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert!(message.metadata.contains_key("multimodal_preflight"));
    assert_eq!(
        message
            .metadata
            .get("multimodal_transport")
            .and_then(|value| value.get("replaced_parts"))
            .and_then(|value| value.as_array())
            .map(|value| value.len()),
        Some(1)
    );
}

#[test]
fn parse_command_argument_map_preserves_quoted_values() {
    let fields = vec![
        CommandArgumentField {
            key: "goal".to_string(),
            label: "Goal".to_string(),
            required: true,
            kind: CommandArgumentKind::LongText,
            repeatable: false,
            options: Vec::new(),
        },
        CommandArgumentField {
            key: "scope".to_string(),
            label: "Scope".to_string(),
            required: true,
            kind: CommandArgumentKind::GlobList,
            repeatable: true,
            options: Vec::new(),
        },
        CommandArgumentField {
            key: "ship".to_string(),
            label: "Ship".to_string(),
            required: false,
            kind: CommandArgumentKind::Boolean,
            repeatable: false,
            options: vec![CommandArgumentOption {
                label: "true".to_string(),
                description: None,
            }],
        },
    ];

    let parsed = parse_command_argument_map(
        Some("--goal \"reduce test flakes\" --scope src/** tests/** --ship"),
        &fields,
    );

    assert_eq!(
        parsed.get("goal"),
        Some(&vec!["reduce test flakes".to_string()])
    );
    assert_eq!(
        parsed.get("scope"),
        Some(&vec!["src/**".to_string(), "tests/**".to_string()])
    );
    assert_eq!(parsed.get("ship"), Some(&vec!["true".to_string()]));
}

#[test]
fn missing_required_command_fields_only_returns_unset_fields() {
    let fields = vec![
        CommandArgumentField {
            key: "goal".to_string(),
            label: "Goal".to_string(),
            required: true,
            kind: CommandArgumentKind::LongText,
            repeatable: false,
            options: Vec::new(),
        },
        CommandArgumentField {
            key: "verify".to_string(),
            label: "Verify".to_string(),
            required: true,
            kind: CommandArgumentKind::CommandLine,
            repeatable: false,
            options: Vec::new(),
        },
    ];

    let parsed = parse_command_argument_map(Some("--goal improve-docs"), &fields);
    let missing = missing_required_command_fields(&fields, &parsed);

    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].key, "verify");
}

#[test]
fn hydrate_scheduler_command_arguments_does_not_inject_hidden_defaults() {
    let registry = CommandRegistry::new();
    let command = registry.get("autoresearch").expect("autoresearch command");
    let invocation = command
        .invocation
        .as_ref()
        .expect("autoresearch invocation");

    let (arguments, raw_arguments) =
        hydrate_scheduler_command_arguments("", &invocation.argument_schema)
            .expect("empty arguments should parse");

    assert!(arguments.is_empty());
    assert!(raw_arguments.is_empty());
}

#[test]
fn hydrate_scheduler_command_arguments_preserves_explicit_user_values() {
    let registry = CommandRegistry::new();
    let command = registry.get("autoresearch").expect("autoresearch command");
    let invocation = command
        .invocation
        .as_ref()
        .expect("autoresearch invocation");

    let (arguments, raw_arguments) = hydrate_scheduler_command_arguments(
        "--goal \"teacher demo goal\" --verify ./custom-verify.sh",
        &invocation.argument_schema,
    )
    .expect("explicit arguments should parse");

    assert_eq!(
        arguments.get("goal"),
        Some(&vec!["teacher demo goal".to_string()])
    );
    assert_eq!(
        arguments.get("verify"),
        Some(&vec!["./custom-verify.sh".to_string()])
    );
    assert!(raw_arguments.contains("--goal \"teacher demo goal\""));
    assert!(raw_arguments.contains("--verify ./custom-verify.sh"));
    assert!(!raw_arguments.contains("--guard"));
    assert!(!raw_arguments.contains("--iterations"));
}

#[tokio::test]
async fn configured_command_uses_merged_template_agent_and_model() {
    let config = AppConfig {
        command: Some(std::collections::HashMap::from([(
            "inherited".to_string(),
            agendao_config::CommandConfig {
                description: Some("Inherited command".to_string()),
                template: Some("Inspect $ARGUMENTS".to_string()),
                agent: Some("global-agent".to_string()),
                model: Some("deepseek-v4-flash".to_string()),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };

    let resolved = resolve_prompt_payload(
        "/inherited exact marker",
        "session-command",
        "/workspace",
        &config,
    )
    .await
    .expect("configured command should resolve");

    assert_eq!(resolved.execution_text, "Inspect exact marker");
    assert_eq!(resolved.agent.as_deref(), Some("global-agent"));
    assert_eq!(resolved.model.as_deref(), Some("deepseek-v4-flash"));
    assert_eq!(
        resolved.command.as_ref().map(|command| &command.source),
        Some(&CommandSource::Config)
    );
}

#[tokio::test]
async fn goal_command_resolves_plain_text_without_pinning_scheduler() {
    let resolved = resolve_prompt_payload(
        "/goal finish the parser and run all tests",
        "session-goal",
        "/workspace",
        &AppConfig::default(),
    )
    .await
    .expect("goal command should resolve");

    assert_eq!(
        command_goal_statement(&resolved).as_deref(),
        Some("finish the parser and run all tests")
    );
    assert!(resolved.scheduler.is_none());
    assert!(resolved
        .execution_text
        .contains("finish the parser and run all tests"));
}
