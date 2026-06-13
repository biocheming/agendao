//! 水 — Event handler: FrontendEvent → SessionStore mutation.
//!
//! Maps incoming server events to the correct Signals in SessionStore.
//! This is the single authority for telemetry-to-state translation.

use agendao_server_core::frontend_events::FrontendEvent;
use agendao_client::SessionRunStatusKind;

use crate::store::session_store::{RunStatus, SessionStore};

/// Apply a FrontendEvent to the appropriate SessionStore.
///
/// Returns the session_id that was affected, if any.
pub fn apply_frontend_event(
    event: &FrontendEvent,
    session: &SessionStore,
) -> Option<String> {
    match event {
        // ── Runtime status ──
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

        // ── Output blocks (streaming) ──
        FrontendEvent::OutputBlockAppended { session_id, block, id, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str());
            let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");

            let block_id = id.as_deref().unwrap_or("");

            match (kind, phase) {
                ("message", Some("delta")) => {
                    // Streaming message text — append to current message
                    session.append_message_text(block_id, text);
                }
                ("message", Some("complete")) => {
                    // Message finished
                    session.finalize_message(block_id);
                    session.run_status.set(RunStatus::Idle);
                }
                ("reasoning", Some("delta")) => {
                    session.append_thinking_text(block_id, text);
                }
                ("reasoning", Some("complete")) => {
                    session.finalize_thinking(block_id);
                }
                ("scheduler_stage", Some("complete")) => {
                    session.finalize_stage(block_id);
                }
                ("scheduler_stage", _) => {
                    session.append_stage_block(block_id, text);
                }
                _ => {
                    // Unknown/unhandled block type — ignore for now
                }
            }
            Some(session_id.clone())
        }

        // ── Tool calls ──
        FrontendEvent::ToolCallUpsert { session_id, tool_call_id, tool_name, .. } => {
            session.set_active_tool(tool_call_id.clone(), tool_name.clone());
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }

        // ── Questions ──
        FrontendEvent::QuestionUpsert { session_id, .. } => {
            session.run_status.set(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::QuestionRemoved { session_id, .. } => {
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }

        // ── Permissions ──
        FrontendEvent::PermissionUpsert { session_id, .. } => {
            session.run_status.set(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionRemoved { session_id, .. } => {
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }

        // ── Projection / telemetry — captured but not displayed in Phase 3 ──
        FrontendEvent::SessionProjectionReplaced { session_id, .. } => {
            Some(session_id.clone())
        }

        // ── Diff — not rendered in Phase 3 ──
        FrontendEvent::DiffReplaced { session_id, .. } => {
            Some(session_id.clone())
        }
    }
}
