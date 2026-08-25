use std::sync::Arc;

use agendao_server_core::runtime_control::QuestionReply;
use tokio_util::sync::CancellationToken;

use crate::ServerState;

pub(crate) async fn request_question_answers(
    state: Arc<ServerState>,
    session_id: String,
    questions: Vec<agendao_tool::QuestionDef>,
    abort: CancellationToken,
) -> Result<Vec<Vec<String>>, agendao_tool::ToolError> {
    if questions.is_empty() {
        return Ok(Vec::new());
    }

    let (question, rx) = state
        .runtime_telemetry
        .register_question(session_id.clone(), questions)
        .await;

    super::task_ledger_reducer::dispatch_interaction(
        &state,
        &session_id,
        agendao_types::task_ledger::AwaitingInteractionKind::Question,
        &question.id,
        false,
    )
    .await;

    let mut rx = rx;
    let reply = tokio::select! {
        reply = &mut rx => reply,
        _ = abort.cancelled() => {
            state.runtime_telemetry.cancel_question(&question.id).await;
            rx.await
        },
    };

    super::task_ledger_reducer::dispatch_interaction(
        &state,
        &session_id,
        agendao_types::task_ledger::AwaitingInteractionKind::Question,
        &question.id,
        true,
    )
    .await;

    match reply {
        Ok(QuestionReply::Answers(answers)) => Ok(answers),
        Ok(QuestionReply::Rejected) => Err(agendao_tool::ToolError::QuestionRejected(
            "User rejected question request".to_string(),
        )),
        Ok(QuestionReply::Cancelled) => Err(agendao_tool::ToolError::Cancelled),
        Err(_) => Err(agendao_tool::ToolError::ExecutionError(
            "Question response channel closed".to_string(),
        )),
    }
}

#[cfg(test)]
#[path = "question/tests.rs"]
mod tests;
