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

use crate::runtime_events::{DiffEntry, ToolCallPhase};
use agendao_api::{
    ContextCompactionLifecycleSummary, ContextCompactionSummary, PermissionRequestInfo,
    QuestionInfo, SessionCacheSemanticsSummary, SessionContextClosureContract,
    SessionExecutionTopology, SessionRuntimeState, SessionUsage, SessionUsageBooks,
};
use agendao_types::LiveMessagePartIdentity;
use serde::{Deserialize, Serialize};

/// Canonical frontend authority event.
///
/// Each variant is a delta instruction: the frontend should apply this change
/// to its local state without issuing follow-up queries.
// 线缆协议类型：variant 负载即协议字段，Box 大 variant 会波及 serde 线格式
// 与 server/tui 两侧全部构造/match 点，故保留现状。
#[allow(clippy::large_enum_variant)]
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

    // ── Projection ──────────────────────────────────────────────────
    /// Replace the projection snapshot (topology, usage,
    /// usage_books, compaction, cache, closure).
    /// Emitted on: topology change, usage update,
    /// telemetry projection change.
    ///
    /// This is the single authority for the "projection" layer of session
    /// telemetry — the fields below cover everything the TUI sidebar / status /
    /// insights panels need without a follow-up get_session_telemetry() query.
    ///
    #[serde(rename = "session.projection.replaced")]
    SessionProjectionReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        /// Topology may not be established yet when usage changes;
        /// Optional so the projector never fabricates a fake authority.
        #[serde(default)]
        topology: Option<SessionExecutionTopology>,
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

    // ── Sandbox ──────────────────────────────────────────────────────
    /// A sandboxed execution appeared or changed — upsert into the
    /// session's active sandbox set. The payload is the authority's own
    /// fact (backend, fingerprint); frontends must never present an
    /// execution as sandboxed without one of these.
    #[serde(rename = "sandbox.execution.upsert")]
    SandboxExecutionUpsert {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "execution")]
        execution: crate::runtime_state::SandboxExecutionSummary,
    },
    /// A sandboxed execution left the active set — exited, denied, or
    /// violated before it ever started. `outcome` tells the frontend
    /// which, so "denied" never renders as a failed run.
    #[serde(rename = "sandbox.execution.removed")]
    SandboxExecutionRemoved {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "executionID")]
        execution_id: String,
        outcome: crate::runtime_state::SandboxOutcomeSummary,
    },

    // ── Diff ─────────────────────────────────────────────────────────
    /// Diff list has changed — replace the entire diff view.
    #[serde(rename = "diff.replaced")]
    DiffReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        diffs: Vec<DiffEntry>,
    },

    // ── Todo ─────────────────────────────────────────────────────────
    /// Todo list has changed — replace the entire todo view.
    /// Emitted on: todowrite tool invocation (TodoManager update).
    #[serde(rename = "todo.replaced")]
    TodoReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        todos: Vec<agendao_types::TodoInfo>,
    },

    // ── Task ledger ──────────────────────────────────────────────────
    /// The session task ledger was committed at a new revision. `ledger`
    /// carries the raw authority plus the server-derived rendering projection.
    #[serde(rename = "task-ledger.replaced")]
    TaskLedgerReplaced {
        #[serde(rename = "sessionID")]
        session_id: String,
        ledger: agendao_types::task_ledger::SessionTaskLedgerView,
        cause: agendao_types::task_ledger::TaskLedgerCause,
    },

    // ── Config ───────────────────────────────────────────────────────────
    /// Global configuration has changed — frontends should reload
    /// config-derived state (providers, modes, settings). Global event with
    /// no session scope.
    #[serde(rename = "config.updated")]
    ConfigUpdated,

    // ── Errors ───────────────────────────────────────────────────────────
    /// A run-time error occurred in a session (e.g. mid-turn provider
    /// failure). Projected from `ServerEvent::Error` so frontends can surface
    /// it immediately instead of waiting for the next runtime snapshot.
    #[serde(rename = "session.error")]
    SessionError {
        #[serde(rename = "sessionID")]
        session_id: String,
        error: String,
        #[serde(rename = "messageID", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
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

/// In-process payload of the canonical FrontendEvent bus.
///
/// The frontend bus is an in-process typed channel: the single projector
/// publishes one typed `FrontendEvent` and every subscriber shares it through
/// `Arc` — in-process subscribers (direct bridge for TUI / Unix socket) see
/// the typed event directly with no JSON round trip.
///
/// The canonical JSON wire text is materialized lazily on first demand and
/// then shared by every network-boundary (SSE) subscriber of the same event,
/// so each event is serialized at most once per process instead of once per
/// subscriber — and zero times when only in-process subscribers exist.
#[derive(Debug)]
pub struct FrontendBusEvent {
    event: FrontendEvent,
    json: std::sync::OnceLock<String>,
}

impl FrontendBusEvent {
    pub fn new(event: FrontendEvent) -> Self {
        Self {
            event,
            json: std::sync::OnceLock::new(),
        }
    }

    pub fn event(&self) -> &FrontendEvent {
        &self.event
    }

    pub fn into_event(self) -> FrontendEvent {
        self.event
    }

    /// Canonical JSON wire text — byte-identical to
    /// `serde_json::to_string` of the typed event, serialized at most once
    /// and shared by all consumers of this envelope.
    pub fn json(&self) -> &str {
        self.json.get_or_init(|| {
            serde_json::to_string(&self.event).unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to serialize FrontendEvent for wire broadcast");
                String::new()
            })
        })
    }

    /// Test probe: `true` once the JSON wire text has been materialized.
    /// In-process-only pipelines must leave this `false`.
    pub fn is_json_materialized(&self) -> bool {
        self.json.get().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FrontendBusEvent: lazy shared JSON wire text ───────────────────

    fn sample_bus_event() -> FrontendBusEvent {
        FrontendBusEvent::new(FrontendEvent::OutputBlockAppended {
            session_id: "ses_1".into(),
            block: serde_json::json!({"kind": "message", "text": "hello"}),
            id: Some("msg_1".into()),
            live_identity: None,
        })
    }

    #[test]
    fn bus_event_starts_unmaterialized() {
        let bus = sample_bus_event();
        assert!(!bus.is_json_materialized());
        assert!(matches!(
            bus.event(),
            FrontendEvent::OutputBlockAppended { .. }
        ));
    }

    #[test]
    fn bus_event_json_matches_direct_serialization_byte_for_byte() {
        let bus = sample_bus_event();
        let expected = serde_json::to_string(bus.event()).expect("direct json");
        assert_eq!(bus.json(), expected);
    }

    #[test]
    fn bus_event_json_is_serialized_once_and_shared() {
        let bus = sample_bus_event();
        let first = bus.json() as *const str;
        let second = bus.json() as *const str;
        assert!(
            bus.is_json_materialized(),
            "json() must materialize the wire text"
        );
        assert_eq!(
            first, second,
            "repeated json() must return the same allocation (serialize once, share)"
        );
    }

    #[test]
    fn bus_event_into_event_recovers_typed_event() {
        let bus = sample_bus_event();
        match bus.into_event() {
            FrontendEvent::OutputBlockAppended {
                session_id, block, ..
            } => {
                assert_eq!(session_id, "ses_1");
                assert_eq!(block["text"], "hello");
            }
            other => panic!("expected OutputBlockAppended, got {:?}", other),
        }
    }

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
                active_sandbox: vec![],
            },
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "session.runtime.replaced");
        assert_eq!(json["sessionID"], "ses_1");
        assert_eq!(json["runtime"]["run_status"], "idle");

        let roundtrip: FrontendEvent = serde_json::from_value(json).expect("deserialize");
        match roundtrip {
            FrontendEvent::SessionRuntimeReplaced {
                session_id,
                runtime,
            } => {
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
                items: vec![agendao_api::QuestionItemInfo {
                    question: "Proceed?".to_string(),
                    header: None,
                    options: vec![],
                    multiple: false,
                }],
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
            FrontendEvent::ToolCallUpsert {
                tool_call_id,
                tool_name,
                phase,
                ..
            } => {
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
            FrontendEvent::OutputBlockAppended {
                session_id,
                block,
                id,
                ..
            } => {
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
                    active_sandbox: vec![],
                },
            })
            .unwrap(),
            serde_json::to_value(FrontendEvent::QuestionRemoved {
                session_id: "s".into(),
                question_id: "q".into(),
            })
            .unwrap(),
            serde_json::to_value(FrontendEvent::PermissionRemoved {
                session_id: "s".into(),
                permission_id: "p".into(),
                reply: "once".into(),
            })
            .unwrap(),
            serde_json::to_value(FrontendEvent::DiffReplaced {
                session_id: "s".into(),
                diffs: vec![],
            })
            .unwrap(),
        ];
        for json in &events {
            assert!(
                json.get("type").and_then(|v| v.as_str()).is_some(),
                "missing 'type' field in: {}",
                json
            );
        }
    }
}
