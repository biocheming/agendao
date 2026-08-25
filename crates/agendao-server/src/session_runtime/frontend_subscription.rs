use agendao_server_core::frontend_events::FrontendEvent;

pub(crate) fn frontend_event_session_id(event: &FrontendEvent) -> Option<&str> {
    match event {
        FrontendEvent::SessionRuntimeReplaced { session_id, .. }
        | FrontendEvent::SessionProjectionReplaced { session_id, .. }
        | FrontendEvent::QuestionUpsert { session_id, .. }
        | FrontendEvent::QuestionRemoved { session_id, .. }
        | FrontendEvent::PermissionUpsert { session_id, .. }
        | FrontendEvent::PermissionRemoved { session_id, .. }
        | FrontendEvent::ToolCallUpsert { session_id, .. }
        | FrontendEvent::DiffReplaced { session_id, .. }
        | FrontendEvent::TodoReplaced { session_id, .. }
        | FrontendEvent::SessionError { session_id, .. }
        | FrontendEvent::TaskLedgerReplaced { session_id, .. }
        | FrontendEvent::SandboxExecutionUpsert { session_id, .. }
        | FrontendEvent::SandboxExecutionRemoved { session_id, .. }
        | FrontendEvent::OutputBlockAppended { session_id, .. } => Some(session_id.as_str()),
        FrontendEvent::ConfigUpdated => None,
    }
}

pub(crate) fn frontend_event_passes_subscription_caps(
    event: &FrontendEvent,
    caps: &agendao_api::FrontendSubscriptionCapabilities,
) -> bool {
    if !caps.final_only
        && caps.reasoning_delta
        && caps.message_text_delta
        && caps.tool_progress
        && caps.runtime_live_view
    {
        return true;
    }

    match event {
        FrontendEvent::OutputBlockAppended { block, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "reasoning" => !caps.final_only && (phase != "delta" || caps.reasoning_delta),
                "message" => phase == "full" || (!caps.final_only && caps.message_text_delta),
                "tool" => {
                    matches!(phase, "done" | "error") || (!caps.final_only && caps.tool_progress)
                }
                _ => !caps.final_only,
            }
        }
        FrontendEvent::SessionRuntimeReplaced { .. }
        | FrontendEvent::SessionProjectionReplaced { .. }
        | FrontendEvent::QuestionUpsert { .. }
        | FrontendEvent::QuestionRemoved { .. }
        | FrontendEvent::PermissionUpsert { .. }
        | FrontendEvent::PermissionRemoved { .. }
        | FrontendEvent::ToolCallUpsert { .. }
        | FrontendEvent::DiffReplaced { .. }
        | FrontendEvent::TodoReplaced { .. }
        | FrontendEvent::SessionError { .. }
        | FrontendEvent::SandboxExecutionUpsert { .. }
        | FrontendEvent::SandboxExecutionRemoved { .. } => true,
        FrontendEvent::TaskLedgerReplaced { ledger, .. } => {
            !caps.final_only
                || matches!(
                    ledger.status,
                    agendao_types::task_ledger::TaskLedgerStatus::AwaitingUser
                        | agendao_types::task_ledger::TaskLedgerStatus::Blocked
                        | agendao_types::task_ledger::TaskLedgerStatus::Interrupted
                        | agendao_types::task_ledger::TaskLedgerStatus::Completed
                )
        }
        FrontendEvent::ConfigUpdated => true,
    }
}

#[cfg(test)]
mod tests {
    use super::frontend_event_passes_subscription_caps;
    use agendao_server_core::frontend_events::FrontendEvent;

    #[test]
    fn cli_tier_keeps_full_message_but_rejects_delta_and_end() {
        let caps = agendao_api::FrontendSubscriptionTier::CliLowFrequency.default_capabilities();
        let event = |phase: &str, text: &str| FrontendEvent::OutputBlockAppended {
            session_id: "ses_1".to_string(),
            block: serde_json::json!({
                "kind": "message",
                "phase": phase,
                "role": "assistant",
                "text": text
            }),
            id: Some("msg_1".to_string()),
            live_identity: None,
        };

        assert!(!frontend_event_passes_subscription_caps(
            &event("delta", "hel"),
            &caps
        ));
        assert!(frontend_event_passes_subscription_caps(
            &event("full", "hello"),
            &caps
        ));
        assert!(!frontend_event_passes_subscription_caps(
            &event("end", ""),
            &caps
        ));
    }
}
