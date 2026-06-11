//! Frontend authority events — the canonical delta contract for all frontends.
//!
//! Unlike `ServerEvent` (execution-domain events), `FrontendEvent` tells the
//! frontend **what to change** in its visible state. Every event carries enough
//! payload for the frontend to apply the change without an extra query.
//!
//! ## Architecture
//!
//! ```text
//! ServerEvent (execution domain)
//!     │
//!     └── projector ──→ FrontendEvent (frontend authority)
//!                             │
//!              ┌──────────────┼──────────────┐
//!             SSE          Unix Socket     Direct
//! ```
//!
//! All transports forward the same `FrontendEvent`. Frontends (TUI / Web / CLI)
//! apply them through a single applier.

use agendao_api::{
    AttachedSessionSummary, ContextCompactionLifecycleSummary, ContextCompactionSummary,
    PermissionRequestInfo, QuestionInfo, SessionCacheSemanticsSummary,
    SessionContextClosureContract, SessionExecutionTopology, SessionRuntimeState,
    SessionUsage, SessionUsageBooks,
};
use agendao_stage_protocol::StageSummary;
use agendao_types::LiveMessagePartIdentity;
use crate::runtime_events::{DiffEntry, ToolCallPhase};
use serde::{Deserialize, Serialize};

/// Canonical frontend authority event.
///
/// Each variant is a delta instruction: the frontend should apply this change
/// to its local state without issuing follow-up queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendEvent {
    // ── Runtime ──────────────────────────────────────────────────────

    /// Replace the entire session runtime state.
    /// Emitted on: run start, run end, status change, tool lifecycle change.
    #[serde(rename = "session.runtime.replaced")]
    SessionRuntimeReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        runtime: SessionRuntimeState,
    },

    // ── Projection (topology / stages / attached sessions) ───────────

    /// Replace the projection snapshot (topology, stages, attached sessions,
    /// usage, usage_books, compaction, cache, closure).
    /// Emitted on: topology change, stage change, attached session
    /// attach/detach, usage update, telemetry projection change.
    ///
    /// This is the single authority for the "projection" layer of session
    /// telemetry — the fields below cover everything the TUI sidebar / status /
    /// insights panels need without a follow-up get_session_telemetry() query.
    #[serde(rename = "session.projection.replaced")]
    SessionProjectionReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        /// Topology may not be established yet when stages/usage change;
        /// Optional so the projector never fabricates a fake authority.
        #[serde(default)]
        topology: Option<SessionExecutionTopology>,
        #[serde(default)]
        stages: Vec<StageSummary>,
        #[serde(default)]
        attached_sessions: Vec<AttachedSessionSummary>,
        #[serde(default)]
        usage: Option<SessionUsage>,
        #[serde(default)]
        usage_books: Option<SessionUsageBooks>,
        #[serde(default)]
        context_compaction_summary: Option<ContextCompactionSummary>,
        #[serde(default)]
        context_compaction_lifecycle_summary: Option<ContextCompactionLifecycleSummary>,
        #[serde(default)]
        cache_semantics: Option<SessionCacheSemanticsSummary>,
        #[serde(default)]
        context_closure_contract: Option<SessionContextClosureContract>,
    },

    // ── Question ─────────────────────────────────────────────────────

    /// A question has been created or updated — upsert into pending queue.
    #[serde(rename = "question.upsert")]
    QuestionUpsert {
        #[serde(rename = "sessionID")]
        session_id: String,
        question: QuestionInfo,
    },

    /// A question has been resolved — remove from pending queue.
    #[serde(rename = "question.removed")]
    QuestionRemoved {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "questionID")]
        question_id: String,
    },

    // ── Permission ───────────────────────────────────────────────────

    /// A permission request has been created — upsert into pending queue.
    #[serde(rename = "permission.upsert")]
    PermissionUpsert {
        #[serde(rename = "sessionID")]
        session_id: String,
        permission: PermissionRequestInfo,
    },

    /// A permission request has been resolved — remove from pending queue.
    #[serde(rename = "permission.removed")]
    PermissionRemoved {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "permissionID")]
        permission_id: String,
        /// The reply that resolved this permission.
        reply: String,
    },

    // ── Tool lifecycle ───────────────────────────────────────────────

    /// A tool call started or completed — upsert into active tool set.
    #[serde(rename = "tool_call.upsert")]
    ToolCallUpsert {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        phase: ToolCallPhase,
    },

    // ── Diff ─────────────────────────────────────────────────────────

    /// Diff list has changed — replace the entire diff view.
    #[serde(rename = "diff.replaced")]
    DiffReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        diffs: Vec<DiffEntry>,
    },

    // ── Output ───────────────────────────────────────────────────────

    /// An output block has been appended to the session transcript.
    #[serde(rename = "output_block")]
    OutputBlockAppended {
        #[serde(rename = "sessionID")]
        session_id: String,
        block: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live_identity: Option<LiveMessagePartIdentity>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_runtime_replaced_roundtrip() {
        let event = FrontendEvent::SessionRuntimeReplaced {
            session_id: "ses_1".to_string(),
            runtime: SessionRuntimeState {
                session_id: "ses_1".to_string(),
                run_status: agendao_api::SessionRunStatusKind::Idle,
                current_message_id: None,
                usage: None,
                active_stage_id: None,
                active_stage_count: 0,
                active_tools: vec![],
                pending_question: None,
                pending_permission: None,
                pending_followup_count: 0,
                attached_sessions: vec![],
            },
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "session.runtime.replaced");
        assert_eq!(json["sessionID"], "ses_1");
        assert_eq!(json["runtime"]["run_status"], "idle");

        let roundtrip: FrontendEvent = serde_json::from_value(json).expect("deserialize");
        match roundtrip {
            FrontendEvent::SessionRuntimeReplaced { session_id, runtime } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(runtime.run_status, agendao_api::SessionRunStatusKind::Idle);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn question_upsert_removed_roundtrip() {
        let upsert = FrontendEvent::QuestionUpsert {
            session_id: "ses_1".to_string(),
            question: QuestionInfo {
                id: "q_1".to_string(),
                session_id: "ses_1".to_string(),
                questions: vec!["Proceed?".to_string()],
                options: None,
                items: vec![],
            },
        };
        let json = serde_json::to_value(&upsert).expect("serialize");
        assert_eq!(json["type"], "question.upsert");
        assert_eq!(json["question"]["id"], "q_1");

        let removed = FrontendEvent::QuestionRemoved {
            session_id: "ses_1".to_string(),
            question_id: "q_1".to_string(),
        };
        let json = serde_json::to_value(&removed).expect("serialize");
        assert_eq!(json["type"], "question.removed");
        assert_eq!(json["questionID"], "q_1");
    }

    #[test]
    fn permission_upsert_removed_roundtrip() {
        let upsert = FrontendEvent::PermissionUpsert {
            session_id: "ses_1".to_string(),
            permission: PermissionRequestInfo {
                id: "p_1".to_string(),
                session_id: "ses_1".to_string(),
                tool: "bash".to_string(),
                permission_class: None,
                scope_key: None,
                scope_label: None,
                origin_tool: None,
                supported_lifetimes: vec![],
                matcher_kind: None,
                matcher_key: None,
                matcher_label: None,
                grant_target_summary: None,
                risk_tags: vec![],
                input: serde_json::json!({"command": "cargo test"}),
                message: "Allow cargo test?".to_string(),
            },
        };
        let json = serde_json::to_value(&upsert).expect("serialize");
        assert_eq!(json["type"], "permission.upsert");
        assert_eq!(json["permission"]["id"], "p_1");
        assert_eq!(json["permission"]["tool"], "bash");

        let removed = FrontendEvent::PermissionRemoved {
            session_id: "ses_1".to_string(),
            permission_id: "p_1".to_string(),
            reply: "once".to_string(),
        };
        let json = serde_json::to_value(&removed).expect("serialize");
        assert_eq!(json["type"], "permission.removed");
        assert_eq!(json["permissionID"], "p_1");
        assert_eq!(json["reply"], "once");
    }

    #[test]
    fn tool_call_upsert_roundtrip() {
        let event = FrontendEvent::ToolCallUpsert {
            session_id: "ses_1".to_string(),
            tool_call_id: "tc_1".to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Start,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "tool_call.upsert");
        assert_eq!(json["toolCallId"], "tc_1");
        assert_eq!(json["toolName"], "bash");
        assert_eq!(json["phase"], "start");

        let roundtrip: FrontendEvent = serde_json::from_value(json).expect("deserialize");
        match roundtrip {
            FrontendEvent::ToolCallUpsert { tool_call_id, tool_name, phase, .. } => {
                assert_eq!(tool_call_id, "tc_1");
                assert_eq!(tool_name, "bash");
                assert_eq!(phase, ToolCallPhase::Start);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn output_block_appended_roundtrip() {
        let event = FrontendEvent::OutputBlockAppended {
            session_id: "ses_1".to_string(),
            block: serde_json::json!({"kind": "message", "text": "hello"}),
            id: Some("msg_1".to_string()),
            live_identity: None,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "output_block");
        assert_eq!(json["sessionID"], "ses_1");
        assert_eq!(json["block"]["text"], "hello");

        let roundtrip: FrontendEvent = serde_json::from_value(json).expect("deserialize");
        match roundtrip {
            FrontendEvent::OutputBlockAppended { session_id, block, id, .. } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(block["text"], "hello");
                assert_eq!(id.unwrap(), "msg_1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn frontend_event_type_field_is_always_present() {
        // Every variant must serialize with a "type" field for SSE event routing.
        let events = vec![
            serde_json::to_value(FrontendEvent::SessionRuntimeReplaced {
                session_id: "s".into(),
                runtime: SessionRuntimeState {
                    session_id: "s".into(),
                    run_status: agendao_api::SessionRunStatusKind::Idle,
                    current_message_id: None,
                    usage: None,
                    active_stage_id: None,
                    active_stage_count: 0,
                    active_tools: vec![],
                    pending_question: None,
                    pending_permission: None,
                    pending_followup_count: 0,
                    attached_sessions: vec![],
                },
            }).unwrap(),
            serde_json::to_value(FrontendEvent::QuestionRemoved {
                session_id: "s".into(),
                question_id: "q".into(),
            }).unwrap(),
            serde_json::to_value(FrontendEvent::PermissionRemoved {
                session_id: "s".into(),
                permission_id: "p".into(),
                reply: "once".into(),
            }).unwrap(),
            serde_json::to_value(FrontendEvent::DiffReplaced {
                session_id: "s".into(),
                diffs: vec![],
            }).unwrap(),
        ];
        for json in &events {
            assert!(json.get("type").and_then(|v| v.as_str()).is_some(),
                "missing 'type' field in: {}", json);
        }
    }
}
