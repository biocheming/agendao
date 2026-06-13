//! 水 — FrontendEvent → SessionStore Signal mapping.

use agendao_server_core::frontend_events::FrontendEvent;
use agendao_client::SessionRunStatusKind;
use crate::store::session_store::SessionStore;
use crate::store::types::*;

pub fn apply_frontend_event(event: &FrontendEvent, session: &SessionStore) -> Option<String> {
    match event {
        FrontendEvent::SessionRuntimeReplaced { session_id, runtime } => {
            let status = match runtime.run_status {
                SessionRunStatusKind::Idle => RunStatus::Idle,
                SessionRunStatusKind::Running => RunStatus::Running,
                SessionRunStatusKind::WaitingOnUser => RunStatus::WaitingUser,
                _ => RunStatus::Idle,
            };
            session.run_status.set(status);
            Some(session_id.clone())
        }

        FrontendEvent::OutputBlockAppended { session_id, block, id, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str());
            let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let tool_name = block.get("tool_name").and_then(|v| v.as_str());
            let params = block.get("params").and_then(|v| v.as_str()).unwrap_or("");
            let bid = id.as_deref().unwrap_or("");

            match (kind, phase) {
                ("message", Some("delta")) => session.push_assistant_delta(bid, text),
                ("message", Some("complete")) => session.run_status.set(RunStatus::Idle),
                ("reasoning", _) => session.push_thinking(bid, text),
                ("tool_call", Some("start")) => {
                    session.upsert_tool_call(bid, tool_name.unwrap_or("?"), params, ToolPhase::Starting);
                }
                ("tool_call", Some("running")) => {
                    session.upsert_tool_call(bid, tool_name.unwrap_or("?"), params, ToolPhase::Running);
                }
                ("tool_call", Some("done")) => {
                    session.upsert_tool_call(bid, tool_name.unwrap_or("?"), params, ToolPhase::Done);
                }
                ("tool_result", _) => {
                    let is_error = block.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                    session.push_tool_result(bid, tool_name.unwrap_or("?"), text, is_error);
                }
                ("skill", _) => session.push_skill(bid, tool_name.unwrap_or(text)),
                ("scheduler_stage", _) => session.push_stage(bid, tool_name.unwrap_or("stage"), text),
                ("compaction", _) => {
                    let before = block.get("before").and_then(|v| v.as_u64()).unwrap_or(0);
                    let after = block.get("after").and_then(|v| v.as_u64()).unwrap_or(0);
                    session.push_compaction(bid, before, after);
                }
                _ => {}
            }
            Some(session_id.clone())
        }

        FrontendEvent::ToolCallUpsert { session_id, tool_call_id, tool_name, phase } => {
            let tp = match phase {
                agendao_server_core::runtime_events::ToolCallPhase::Start => ToolPhase::Starting,
                agendao_server_core::runtime_events::ToolCallPhase::Complete => ToolPhase::Done,
            };
            session.set_active_tool(tool_call_id, tool_name, tp);
            Some(session_id.clone())
        }

        FrontendEvent::QuestionUpsert { session_id, .. } => {
            session.run_status.set(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::QuestionRemoved { session_id, .. } => {
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionUpsert { session_id, .. } => {
            session.run_status.set(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionRemoved { session_id, .. } => {
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }

        FrontendEvent::SessionProjectionReplaced { session_id, usage, .. } => {
            if let Some(ref u) = usage {
                let input = u.input_tokens;
                let output = u.output_tokens;
                let read = u.cache_read_tokens;
                let miss = u.cache_miss_tokens;
                let write = u.cache_write_tokens;
                session.set_token_usage(input, output, read, miss, write);
            }
            Some(session_id.clone())
        }

        FrontendEvent::DiffReplaced { session_id, .. } => Some(session_id.clone()),
    }
}
