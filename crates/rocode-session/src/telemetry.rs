use crate::{MessageRole, Session};
use rocode_types::SessionTelemetrySnapshot;

pub const SESSION_TELEMETRY_METADATA_KEY: &str = "telemetry";

pub fn persist_session_telemetry_snapshot(
    session: &mut Session,
    snapshot: &SessionTelemetrySnapshot,
) -> anyhow::Result<()> {
    let value = serde_json::to_value(snapshot)?;
    session.insert_metadata(SESSION_TELEMETRY_METADATA_KEY.to_string(), value);
    Ok(())
}

pub fn load_session_telemetry_snapshot(session: &Session) -> Option<SessionTelemetrySnapshot> {
    session
        .metadata
        .get(SESSION_TELEMETRY_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

pub fn session_last_run_status_label(session: &Session) -> String {
    if let Some(label) = session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, MessageRole::Assistant))
        .and_then(|message| {
            message
                .finish
                .clone()
                .or_else(|| {
                    message
                        .metadata
                        .get("finish_reason")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned)
                })
                .map(|value| normalize_finish_reason(&value))
        })
    {
        return label;
    }

    match session.status {
        crate::SessionStatus::Completed => "completed".to_string(),
        crate::SessionStatus::Archived => "archived".to_string(),
        crate::SessionStatus::Compacting => "compacting".to_string(),
        crate::SessionStatus::Active => "active".to_string(),
    }
}

fn normalize_finish_reason(reason: &str) -> String {
    let trimmed = reason.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "stop" | "completed" => "completed".to_string(),
        "cancelled" | "canceled" | "abort" | "aborted" => "cancelled".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionMessage;
    use rocode_types::{
        PersistedStageTelemetrySummary, SessionTelemetrySnapshotVersion, SessionUsage,
    };

    fn sample_snapshot() -> SessionTelemetrySnapshot {
        SessionTelemetrySnapshot {
            version: SessionTelemetrySnapshotVersion::V2,
            usage: SessionUsage {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 3,
                cache_write_tokens: 4,
                cache_read_tokens: 5,
                context_tokens: 10,
                total_cost: 0.25,
            },
            stage_summaries: vec![PersistedStageTelemetrySummary {
                stage_id: "stage-1".to_string(),
                stage_name: "Plan".to_string(),
                index: Some(1),
                total: Some(2),
                step: Some(1),
                step_total: Some(3),
                status: rocode_content::stage_protocol::StageStatus::Running,
                prompt_tokens: Some(11),
                completion_tokens: Some(7),
                reasoning_tokens: Some(5),
                cache_read_tokens: Some(2),
                cache_write_tokens: Some(1),
                focus: Some("inspect".to_string()),
                last_event: Some("scheduler.stage.started".to_string()),
                waiting_on: None,
                estimated_context_tokens: Some(99),
                skill_tree_budget: Some(512),
                skill_tree_truncation_strategy: Some("head".to_string()),
                skill_tree_truncated: Some(false),
                retry_attempt: None,
                active_agent_count: 1,
                active_tool_count: 2,
                child_session_count: 0,
                primary_child_session_id: None,
            }],
            memory: None,
            last_run_status: "completed".to_string(),
            updated_at: 123,
        }
    }

    #[test]
    fn telemetry_snapshot_roundtrips_via_session_metadata() {
        let mut session = Session::new("proj", ".");
        let snapshot = sample_snapshot();
        persist_session_telemetry_snapshot(&mut session, &snapshot).expect("persist should work");

        let loaded = load_session_telemetry_snapshot(&session).expect("snapshot should load");
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn telemetry_snapshot_load_tolerates_corrupted_metadata() {
        let mut session = Session::new("proj", ".");
        session.insert_metadata(
            SESSION_TELEMETRY_METADATA_KEY.to_string(),
            serde_json::json!({"usage": "bad"}),
        );

        assert!(load_session_telemetry_snapshot(&session).is_none());
    }

    #[test]
    fn session_last_run_status_prefers_latest_assistant_finish_reason() {
        let mut session = Session::new("proj", ".");
        let mut assistant = SessionMessage::assistant(session.id.clone());
        assistant.finish = Some("stop".to_string());
        session.messages_mut().push(assistant);

        assert_eq!(session_last_run_status_label(&session), "completed");
    }
}
