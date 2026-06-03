//! Session Core - Minimal session types without orchestrator dependencies
//!
//! This crate contains only the core Session and SessionManager types,
//! extracted to break the cyclic dependency:
//! agendao-agent → agendao-orchestrator → agendao-session → agendao-memory → agendao-command → agendao-agent
//!
//! By extracting SessionManager here, agendao-orchestrator can depend on agendao-session-core
//! while agendao-session can still depend on agendao-orchestrator for compaction/summary features.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use agendao_types::Session as SessionRecord;
pub use agendao_types::{
    FileDiff, PermissionRuleset, SessionContextKind, SessionForkExplain, SessionForkHistoryMode,
    SessionForkLifecycleExplain, SessionForkLifecycleScope, SessionOwnershipSummary, SessionRevert,
    SessionShare, SessionStatus, SessionSummary, SessionTime, SessionUsage, SessionUsageBooks,
    SessionMessage, MessageRole, MessagePart, PartType,
};

// ============================================================================
// Core Types (minimal subset from agendao-session)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    record: SessionRecord,
}

impl Session {
    pub fn new(id: String) -> Self {
        let now = Utc::now();
        Self {
            id: id.clone(),
            title: String::new(),
            created_at: now,
            updated_at: now,
            record: SessionRecord {
                id,
                slug: String::new(),
                project_id: String::new(),
                directory: String::new(),
                parent_id: None,
                title: String::new(),
                version: String::from("1"),
                time: SessionTime {
                    created: now.timestamp_millis(),
                    updated: now.timestamp_millis(),
                    compacting: None,
                    archived: None,
                },
                messages: Vec::new(),
                summary: None,
                share: None,
                revert: None,
                permission: None,
                usage: Some(SessionUsage::default()),
                status: SessionStatus::Active,
                metadata: HashMap::new(),
                created_at: now,
                updated_at: now,
            },
        }
    }

    pub fn record(&self) -> &SessionRecord {
        &self.record
    }

    pub fn record_mut(&mut self) -> &mut SessionRecord {
        &mut self.record
    }

    pub fn to_row(&self) -> SessionRow {
        SessionRow {
            id: self.id.clone(),
            title: self.title.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            status: self.record.status.clone(),
        }
    }

    /// Add user message to session (Phase 6.1)
    pub fn add_user_message(&mut self, text: &str) -> String {
        self.add_user_message_inner(text, HashMap::new())
    }

    /// Add a user message with canonical source metadata.
    pub fn add_user_message_with_source(
        &mut self,
        text: &str,
        origin: agendao_types::MessageSourceOrigin,
        surface: agendao_types::MessageSourceSurface,
    ) -> String {
        let mut metadata = HashMap::new();
        agendao_types::apply_message_source_metadata(&mut metadata, origin, surface);
        let (admission, authority_class) = agendao_types::origin_to_admission_authority(origin);
        agendao_types::apply_message_admission_metadata(&mut metadata, admission, authority_class);
        self.add_user_message_inner(text, metadata)
    }

    fn add_user_message_inner(&mut self, text: &str, metadata: HashMap<String, serde_json::Value>) -> String {
        use agendao_types::{SessionMessage, MessageRole, MessagePart, PartType};

        let msg_id = format!("msg_{}", Uuid::new_v4());
        let now = Utc::now();

        let message = SessionMessage {
            id: msg_id.clone(),
            session_id: self.id.clone(),
            role: MessageRole::User,
            parts: vec![MessagePart {
                id: format!("prt_{}", Uuid::new_v4()),
                part_type: PartType::Text {
                    text: text.to_string(),
                    synthetic: None,
                    ignored: None,
                },
                created_at: now,
                message_id: Some(msg_id.clone()),
            }],
            created_at: now,
            metadata,
            usage: None,
            finish: None,
        };

        self.record.messages.push(message);
        self.record.time.updated = now.timestamp_millis();
        self.updated_at = now;

        msg_id
    }

    /// Add assistant message to session (Phase 6.1)
    pub fn add_assistant_message(&mut self, text: &str) -> String {
        use agendao_types::{SessionMessage, MessageRole, MessagePart, PartType};

        let msg_id = format!("msg_{}", Uuid::new_v4());
        let now = Utc::now();

        let message = SessionMessage {
            id: msg_id.clone(),
            session_id: self.id.clone(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart {
                id: format!("prt_{}", Uuid::new_v4()),
                part_type: PartType::Text {
                    text: text.to_string(),
                    synthetic: None,
                    ignored: None,
                },
                created_at: now,
                message_id: Some(msg_id.clone()),
            }],
            created_at: now,
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        };

        self.record.messages.push(message);
        self.record.time.updated = now.timestamp_millis();
        self.updated_at = now;

        msg_id
    }

    /// Add usage statistics (Phase 6.1)
    pub fn add_usage(&mut self, input_tokens: u64, output_tokens: u64) {
        if let Some(usage) = &mut self.record.usage {
            usage.input_tokens += input_tokens;
            usage.output_tokens += output_tokens;
            // total_tokens is not a field in SessionUsage, it's computed
        } else {
            self.record.usage = Some(SessionUsage {
                input_tokens,
                output_tokens,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                context_tokens: 0,
                total_cost: 0.0,
            });
        }
    }

    /// Add tool result to session (Phase 6.2)
    pub fn add_tool_result(&mut self, tool_call_id: &str, content: &str, is_error: bool) -> String {
        let msg_id = format!("msg_{}", Uuid::new_v4());
        let now = Utc::now();

        let message = SessionMessage {
            id: msg_id.clone(),
            session_id: self.id.clone(),
            role: MessageRole::Tool,
            parts: vec![MessagePart {
                id: format!("prt_{}", Uuid::new_v4()),
                part_type: PartType::ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    content: content.to_string(),
                    is_error,
                    title: None,
                    metadata: None,
                    attachments: None,
                },
                created_at: now,
                message_id: Some(msg_id.clone()),
            }],
            created_at: now,
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        };

        self.record.messages.push(message);
        self.record.time.updated = now.timestamp_millis();
        self.updated_at = now;

        msg_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub session_id: String,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Session Manager (minimal implementation)
// ============================================================================

/// Required per-session operations for the orchestrator LLM loop.
pub trait SessionAccess {
    fn add_user_message(&mut self, text: &str) -> String;
    fn add_user_message_with_source(
        &mut self,
        text: &str,
        origin: agendao_types::MessageSourceOrigin,
        surface: agendao_types::MessageSourceSurface,
    ) -> String;
    fn add_assistant_message(&mut self, text: &str) -> String;
    fn add_tool_result(&mut self, tool_call_id: &str, content: &str, is_error: bool) -> String;
    fn add_usage(&mut self, input_tokens: u64, output_tokens: u64);
    fn record(&self) -> &SessionRecord;
    fn record_mut(&mut self) -> &mut SessionRecord;
}

/// Session storage abstraction.
/// The default `SessionManager` implements this; `agendao_session::SessionManager`
/// also implements it so `agendao-server` can inject the unified authority.
pub trait SessionStore {
    type Session: SessionAccess;
    fn ensure_session(&mut self, id: &str) -> &mut Self::Session;
    fn get(&self, id: &str) -> Option<&Self::Session>;
    fn get_mut(&mut self, id: &str) -> Option<&mut Self::Session>;
    fn list(&self) -> Vec<&Self::Session>;
}

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    events: Vec<SessionEvent>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            events: Vec::new(),
        }
    }

    pub fn create(&mut self, id: Option<String>) -> &Session {
        let id = id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let session = Session::new(id.clone());

        self.sessions.insert(id.clone(), session);
        self.sessions.get(&id).unwrap()
    }

    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// Get or create a session (Phase 6.1)
    pub fn get_or_create(&mut self, id: &str) -> &mut Session {
        if !self.sessions.contains_key(id) {
            let session = Session::new(id.to_string());
            self.sessions.insert(id.to_string(), session);
        }
        self.sessions.get_mut(id).unwrap()
    }

    pub fn list(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn delete(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }

    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

// Trait impl — SessionAccess for the minimal Session type.
impl SessionAccess for Session {
    fn add_user_message(&mut self, text: &str) -> String {
        self.add_user_message(text)
    }
    fn add_user_message_with_source(
        &mut self,
        text: &str,
        origin: agendao_types::MessageSourceOrigin,
        surface: agendao_types::MessageSourceSurface,
    ) -> String {
        self.add_user_message_with_source(text, origin, surface)
    }
    fn add_assistant_message(&mut self, text: &str) -> String {
        self.add_assistant_message(text)
    }
    fn add_tool_result(&mut self, tool_call_id: &str, content: &str, is_error: bool) -> String {
        self.add_tool_result(tool_call_id, content, is_error)
    }
    fn add_usage(&mut self, input_tokens: u64, output_tokens: u64) {
        self.add_usage(input_tokens, output_tokens)
    }
    fn record(&self) -> &SessionRecord {
        self.record()
    }
    fn record_mut(&mut self) -> &mut SessionRecord {
        self.record_mut()
    }
}

// Trait impl — SessionStore for the minimal SessionManager.
impl SessionStore for SessionManager {
    type Session = Session;

    fn ensure_session(&mut self, id: &str) -> &mut Session {
        self.get_or_create(id)
    }
    fn get(&self, id: &str) -> Option<&Session> {
        self.get(id)
    }
    fn get_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.get_mut(id)
    }
    fn list(&self) -> Vec<&Session> {
        self.list()
    }
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, thiserror::Error)]
pub enum SessionError {
    #[error("Session not found: {0}")]
    NotFound(String),
    #[error("Session error: {0}")]
    Other(String),
}

pub type SessionResult<T> = Result<T, SessionError>;
