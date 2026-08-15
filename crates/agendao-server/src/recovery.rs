//! Run-level recovery protocol.

use serde::{Deserialize, Serialize};

use agendao_server_core::runtime_control::SessionExecutionTopology;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryProtocolStatus {
    Running,
    AwaitingUser,
    Recoverable,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryActionKind {
    AbortRun,
    Retry,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryActionInfo {
    pub kind: RecoveryActionKind,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecoveryProtocol {
    pub session_id: String,
    pub status: RecoveryProtocolStatus,
    pub active_execution_count: usize,
    pub pending_question_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_prompt: Option<String>,
    #[serde(default)]
    pub actions: Vec<RecoveryActionInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteRecoveryRequest {
    pub action: RecoveryActionKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RecoveryExecutionContext {
    pub action: Option<RecoveryActionKind>,
}

pub(crate) fn latest_user_prompt(session: &agendao_session::Session) -> Option<String> {
    session.last_owner_local_user_message().and_then(|message| {
        message
            .metadata
            .get("resolved_user_prompt")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let text = message.get_text().trim().to_string();
                (!text.is_empty()).then_some(text)
            })
    })
}

fn latest_assistant_finish(session: &agendao_session::Session) -> Option<&str> {
    session
        .last_owner_local_assistant_message()
        .and_then(|message| message.finish.as_deref())
}

pub(crate) fn protocol_allows_recovery_action(
    protocol: &SessionRecoveryProtocol,
    action: &RecoveryActionKind,
) -> bool {
    protocol
        .actions
        .iter()
        .any(|candidate| &candidate.kind == action)
}

pub(crate) fn build_session_recovery_protocol(
    session_id: &str,
    session: &agendao_session::Session,
    topology: &SessionExecutionTopology,
    pending_question_count: usize,
) -> SessionRecoveryProtocol {
    let last_user_prompt = latest_user_prompt(session);
    let busy = topology.active_count > 0;
    let mut actions = Vec::new();

    if busy || pending_question_count > 0 {
        actions.push(RecoveryActionInfo {
            kind: RecoveryActionKind::AbortRun,
            label: "Abort run".to_string(),
            description: "Stop the current run and return control to recovery mode.".to_string(),
        });
    } else {
        if last_user_prompt.is_some()
            && matches!(
                latest_assistant_finish(session),
                Some("error" | "cancelled")
            )
        {
            actions.push(RecoveryActionInfo {
                kind: RecoveryActionKind::Retry,
                label: "Retry last run".to_string(),
                description: "Re-run the last request with the same scheduler selection."
                    .to_string(),
            });
        }
        if last_user_prompt.is_some() {
            actions.push(RecoveryActionInfo {
                kind: RecoveryActionKind::Resume,
                label: "Resume".to_string(),
                description: "Continue the request while preserving verified prior work."
                    .to_string(),
            });
        }
    }

    let (status, summary) = if busy {
        (
            RecoveryProtocolStatus::Running,
            Some(format!(
                "{} active execution(s) are still running.",
                topology.active_count
            )),
        )
    } else if pending_question_count > 0 {
        (
            RecoveryProtocolStatus::AwaitingUser,
            Some(format!(
                "{} pending question(s) need answers before recovery can continue.",
                pending_question_count
            )),
        )
    } else if actions.is_empty() {
        (
            RecoveryProtocolStatus::Idle,
            Some("No recovery action is available for this session.".to_string()),
        )
    } else {
        (
            RecoveryProtocolStatus::Recoverable,
            Some("Run-level recovery actions are available.".to_string()),
        )
    };

    SessionRecoveryProtocol {
        session_id: session_id.to_string(),
        status,
        active_execution_count: topology.active_count,
        pending_question_count,
        summary,
        last_user_prompt,
        actions,
    }
}

pub(crate) fn compose_retry_prompt(base_prompt: &str) -> String {
    format!(
        "Recovery protocol: retry the previous request with the same scheduler selection and constraints. Preserve valid prior work, but re-run failed or incomplete work.\n\nOriginal request:\n{base_prompt}"
    )
}

pub(crate) fn compose_resume_prompt(base_prompt: &str) -> String {
    format!(
        "Recovery protocol: resume the previous request without restarting discovery. Preserve verified work, artifacts, decisions, and constraints.\n\nOriginal request:\n{base_prompt}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_prompt_composition_preserves_original_request() {
        assert!(compose_retry_prompt("fix it").contains("fix it"));
        assert!(compose_resume_prompt("fix it").contains("fix it"));
    }
}
