//! Run-level recovery protocol.

use serde::{Deserialize, Serialize};

use agendao_server_core::runtime_control::SessionExecutionTopology;
use agendao_types::task_ledger::{current_checkpoints, SessionTaskLedger};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoint_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_statement: Option<String>,
}

impl RecoveryExecutionContext {
    pub(crate) fn from_ledger(action: RecoveryActionKind, ledger: &SessionTaskLedger) -> Self {
        let governed = ledger.revision > 0 && ledger.goal.is_some();
        Self {
            action: Some(action),
            ledger_revision: governed.then_some(ledger.revision),
            checkpoint_ids: if governed {
                current_checkpoints(ledger)
                    .into_iter()
                    .map(|checkpoint| checkpoint.id.clone())
                    .collect()
            } else {
                Vec::new()
            },
            open_ids: if governed {
                ledger
                    .open_questions()
                    .into_iter()
                    .map(|question| question.id.clone())
                    .collect()
            } else {
                Vec::new()
            },
            next_statement: if governed {
                ledger.next.as_ref().map(|next| next.statement.clone())
            } else {
                None
            },
        }
    }
}

pub(crate) fn latest_user_prompt(session: &agendao_session::Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| {
            matches!(message.role, agendao_session::MessageRole::User)
                && !agendao_session::Session::is_imported_fork_history_message(message)
                && !matches!(
                    message
                        .metadata
                        .get("message_source.origin")
                        .and_then(serde_json::Value::as_str),
                    Some("system" | "scheduler" | "runtime")
                )
        })
        .and_then(|message| {
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

pub(crate) fn compose_resume_prompt(
    base_prompt: &str,
    recovery: &RecoveryExecutionContext,
) -> String {
    let anchor = match recovery.ledger_revision {
        Some(revision) => format!(
            "Authoritative recovery anchor: task-ledger revision {revision}; current checkpoint ids: {}; open ids: {}; Next: {}. Continue from this Next, preserve checkpointed evidence, and resolve Open items. The separately injected <task-ledger> block is authoritative; this line only names the recovery boundary.",
            if recovery.checkpoint_ids.is_empty() {
                "none".to_string()
            } else {
                recovery.checkpoint_ids.join(", ")
            },
            if recovery.open_ids.is_empty() {
                "none".to_string()
            } else {
                recovery.open_ids.join(", ")
            },
            recovery.next_statement.as_deref().unwrap_or("not set"),
        ),
        None => "No typed task-ledger recovery anchor is available; use live session history and workspace state.".to_string(),
    };
    format!(
        "Recovery protocol: resume without restarting discovery. {anchor}\n\nOriginal request (background only, not the recovery authority):\n{base_prompt}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_prompt_composition_preserves_original_request() {
        assert!(compose_retry_prompt("fix it").contains("fix it"));
        let recovery = RecoveryExecutionContext {
            action: Some(RecoveryActionKind::Resume),
            ledger_revision: Some(7),
            checkpoint_ids: vec!["chk-03".to_string()],
            open_ids: vec!["open-02".to_string()],
            next_statement: Some("run the remaining test".to_string()),
        };
        let prompt = compose_resume_prompt("fix it", &recovery);
        assert!(prompt.contains("fix it"));
        assert!(prompt.contains("revision 7"));
        assert!(prompt.contains("chk-03"));
        assert!(prompt.contains("open-02"));
        assert!(prompt.contains("run the remaining test"));
        assert!(prompt.contains("background only"));
    }

    #[test]
    fn recovery_prompt_ignores_system_steering() {
        let mut manager = agendao_session::SessionManager::new();
        let mut session = manager.create("project", "/tmp/recovery-owner");
        session.add_user_message("original task");
        session.add_user_message_with_source(
            "system steering",
            agendao_types::MessageSourceOrigin::System,
            agendao_types::MessageSourceSurface::Direct,
        );

        assert_eq!(
            latest_user_prompt(&session).as_deref(),
            Some("original task")
        );
    }
}
