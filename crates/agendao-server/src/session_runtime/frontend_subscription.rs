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
        | FrontendEvent::OutputBlockAppended { session_id, .. } => Some(session_id.as_str()),
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
                "message" => !caps.final_only && caps.message_text_delta,
                "scheduler_stage" => !caps.final_only && caps.tool_progress,
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
        | FrontendEvent::DiffReplaced { .. } => true,
    }
}
