use std::sync::Arc;

use agendao_server_core::runtime_events::{QuestionResolutionKind, ServerEvent};
use tokio_util::sync::CancellationToken;

use super::request_question_answers;
use crate::ServerState;

fn sample_question() -> agendao_tool::QuestionDef {
    agendao_tool::QuestionDef {
        header: Some("Scope".to_string()),
        question: "Proceed with migration?".to_string(),
        options: vec![agendao_tool::QuestionOption {
            label: "Yes".to_string(),
            description: Some("Continue".to_string()),
        }],
        multiple: false,
    }
}

#[tokio::test]
async fn request_question_answers_emits_each_lifecycle_event_once() {
    let state = Arc::new(ServerState::new());
    let mut events = state.event_bus.subscribe();

    let responder = tokio::spawn({
        let state = state.clone();
        async move {
            loop {
                if let Some(question) = state.runtime_telemetry.list_questions().await.first() {
                    state
                        .runtime_telemetry
                        .answer_question(&question.id, vec![vec!["Yes".to_string()]])
                        .await;
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
    });

    let answers = request_question_answers(
        state,
        "session-1".to_string(),
        vec![sample_question()],
        CancellationToken::new(),
    )
    .await
    .expect("question should be answered");
    responder.await.expect("responder join");
    assert_eq!(answers, vec![vec!["Yes".to_string()]]);

    let mut created = 0;
    let mut answered = 0;
    while let Ok(event) = events.try_recv() {
        match event.event_ref() {
            ServerEvent::QuestionCreated { session_id, .. } if session_id == "session-1" => {
                created += 1;
            }
            ServerEvent::QuestionResolved {
                session_id,
                resolution: Some(QuestionResolutionKind::Answered),
                ..
            } if session_id == "session-1" => {
                answered += 1;
            }
            _ => {}
        }
    }
    assert_eq!(
        created, 1,
        "question creation must have one event authority"
    );
    assert_eq!(
        answered, 1,
        "question resolution must have one event authority"
    );
}

#[tokio::test]
async fn cancellation_cleans_up_and_emits_one_resolution() {
    let state = Arc::new(ServerState::new());
    let abort = CancellationToken::new();
    let mut events = state.event_bus.subscribe();

    let request = tokio::spawn({
        let state = state.clone();
        let abort = abort.clone();
        async move {
            request_question_answers(
                state,
                "session-cancel".to_string(),
                vec![sample_question()],
                abort,
            )
            .await
        }
    });

    loop {
        if !state.runtime_telemetry.list_questions().await.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    abort.cancel();

    assert!(matches!(
        request.await.expect("request join"),
        Err(agendao_tool::ToolError::Cancelled)
    ));
    assert!(state.runtime_telemetry.list_questions().await.is_empty());

    let mut cancelled = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.event_ref(),
            ServerEvent::QuestionResolved {
                session_id,
                resolution: Some(QuestionResolutionKind::Cancelled),
                ..
            } if session_id == "session-cancel"
        ) {
            cancelled += 1;
        }
    }
    assert_eq!(cancelled, 1, "cancellation must have one event authority");
}

#[tokio::test]
async fn execution_cancellation_uses_question_cancellation_semantics() {
    let state = Arc::new(ServerState::new());
    let mut events = state.event_bus.subscribe();
    let request = tokio::spawn({
        let state = state.clone();
        async move {
            request_question_answers(
                state,
                "session-execution-cancel".to_string(),
                vec![sample_question()],
                CancellationToken::new(),
            )
            .await
        }
    });

    let question_id = loop {
        if let Some(question) = state.runtime_telemetry.list_questions().await.first() {
            break question.id.clone();
        }
        tokio::task::yield_now().await;
    };

    assert!(matches!(
        state.runtime_telemetry.cancel_execution(&question_id).await,
        Some(agendao_server_core::runtime_control::ExecutionKind::Question)
    ));
    assert!(matches!(
        request.await.expect("request join"),
        Err(agendao_tool::ToolError::Cancelled)
    ));
    assert!(state.runtime_telemetry.list_questions().await.is_empty());

    let mut cancelled = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.event_ref(),
            ServerEvent::QuestionResolved {
                session_id,
                resolution: Some(QuestionResolutionKind::Cancelled),
                ..
            } if session_id == "session-execution-cancel"
        ) {
            cancelled += 1;
        }
    }
    assert_eq!(
        cancelled, 1,
        "execution cancel must use the event authority"
    );
}
