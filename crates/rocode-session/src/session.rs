use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use uuid::Uuid;

use rocode_core::bus::{Bus, BusEventDef};
use rocode_plugin::{HookContext, HookEvent};
use rocode_types::Session as SessionRecord;
pub use rocode_types::{
    FileDiff, PermissionRuleset, SessionContextKind, SessionForkExplain, SessionForkHistoryMode,
    SessionForkLifecycleExplain, SessionForkLifecycleScope, SessionOwnershipSummary, SessionRevert,
    SessionShare, SessionStatus, SessionSummary, SessionTime, SessionUsage, SessionUsageBooks,
};

#[cfg(test)]
use crate::MessageUsage;
use crate::{MessagePart, MessageRole, SessionMessage};

// ============================================================================
// Bus Event Definitions (matches TS Session.Event)
// ============================================================================

pub static SESSION_CREATED_EVENT: BusEventDef = BusEventDef::new("session.created");
pub static SESSION_UPDATED_EVENT: BusEventDef = BusEventDef::new("session.updated");
pub static SESSION_DELETED_EVENT: BusEventDef = BusEventDef::new("session.deleted");
pub static SESSION_DIFF_EVENT: BusEventDef = BusEventDef::new("session.diff");
pub static SESSION_ERROR_EVENT: BusEventDef = BusEventDef::new("session.error");

// Message-level events (matches TS MessageV2.Event)
pub static MESSAGE_UPDATED_EVENT: BusEventDef = BusEventDef::new("message.updated");
pub static MESSAGE_REMOVED_EVENT: BusEventDef = BusEventDef::new("message.removed");
pub static PART_UPDATED_EVENT: BusEventDef = BusEventDef::new("message.part.updated");
pub static PART_REMOVED_EVENT: BusEventDef = BusEventDef::new("message.part.removed");
pub static PART_DELTA_EVENT: BusEventDef = BusEventDef::new("message.part.delta");
pub static COMMAND_EXECUTED_EVENT: BusEventDef = BusEventDef::new("command.executed");
pub const SESSION_CONTEXT_KIND_METADATA_KEY: &str = "session_context_kind";
pub const FORK_ORIGIN_SESSION_ID_METADATA_KEY: &str = "fork_origin_session_id";
pub const FORK_ORIGIN_MESSAGE_ID_METADATA_KEY: &str = "fork_origin_message_id";
pub const FORK_POLICY_FROZEN_METADATA_KEY: &str = "fork_policy_frozen";
pub const FORK_IMPORTED_HISTORY_METADATA_KEY: &str = "fork_imported_history";
pub const FORK_HISTORY_MODE_METADATA_KEY: &str = "fork_history_mode";
pub const FORK_HISTORY_MESSAGE_LIMIT_METADATA_KEY: &str = "fork_history_message_limit";
pub const FORK_SOURCE_HISTORY_MESSAGE_COUNT_METADATA_KEY: &str =
    "fork_source_history_message_count";
const LEGACY_STAGE_ATTACHED_SESSION_TITLE_PREFIX: &str = "Stage: ";

const FORK_POLICY_METADATA_ALLOWLIST: &[&str] = &[
    "agent",
    "model_id",
    "model_provider",
    "model_variant",
    "scheduler_applied",
    "scheduler_profile",
    "scheduler_root_agent",
    "scheduler_selection_source",
    "scheduler_selection_trace",
    "scheduler_selection_warning",
    "scheduler_skill_tree_applied",
];

const FORK_CACHE_STABILITY_METADATA_KEYS: &[&str] = &[
    "agent",
    "model_id",
    "model_provider",
    "model_variant",
    "scheduler_applied",
    "scheduler_profile",
    "scheduler_root_agent",
    "scheduler_skill_tree_applied",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionForkSpec<'a> {
    pub message_id: Option<&'a str>,
    pub history_mode: SessionForkHistoryMode,
    pub history_message_limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionForkError {
    SessionNotFound,
    OriginMessageNotFound(String),
    InvalidRequest(String),
}

/// Canonical allowlist for server-side session list search.
///
/// This is owned by `rocode-session` so all adapters expose the same list-search
/// semantics and cannot silently broaden them with detail-only fields.
pub const SESSION_LIST_SEARCH_FIELDS: &[&str] = &["title"];

fn looks_like_provider_response_json(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let has = |key: &str| object.contains_key(key);

    (has("choices") && (has("model") || has("usage") || has("id") || has("object")))
        || (has("candidates") && (has("usage") || has("model_version") || has("model")))
        || (has("output") && (has("model") || has("usage") || has("id")))
        || (has("message") && has("type") && (has("usage") || has("delta")))
}

fn strip_trailing_provider_response_json(text: &str) -> String {
    let trimmed = text.trim_end();
    let candidate_starts = [
        trimmed.rfind("\n\n{"),
        trimmed.rfind("\n{"),
        trimmed.rfind("\n\n["),
        trimmed.rfind("\n["),
    ];

    for start_index in candidate_starts.into_iter().flatten() {
        let candidate = trimmed[start_index..].trim_start();
        if !(candidate.starts_with('{') || candidate.starts_with('[')) {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        if !looks_like_provider_response_json(&parsed) {
            continue;
        }
        let prefix = trimmed[..start_index].trim_end();
        if prefix.is_empty() {
            continue;
        }
        return prefix.to_string();
    }

    trimmed.to_string()
}

pub fn sanitize_display_text(text: &str) -> String {
    let mut lines = Vec::new();
    let mut in_pseudo_invoke = false;
    let mut previous_blank = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("minimax:tool_call") {
            continue;
        }
        if trimmed.starts_with("<invoke ") {
            in_pseudo_invoke = true;
            continue;
        }
        if in_pseudo_invoke {
            if trimmed.starts_with("</invoke>") {
                in_pseudo_invoke = false;
            }
            continue;
        }
        if trimmed.starts_with("<parameter ") || trimmed.starts_with("</invoke>") {
            continue;
        }

        if trimmed.is_empty() {
            if previous_blank {
                continue;
            }
            previous_blank = true;
            lines.push(String::new());
            continue;
        }

        previous_blank = false;
        lines.push(raw_line.to_string());
    }

    strip_trailing_provider_response_json(&lines.join("\n"))
        .trim()
        .to_string()
}

// ============================================================================
// Session Info Schema
// ============================================================================

// ============================================================================
// Session Event Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionEvent {
    Created {
        info: Session,
    },
    Updated {
        info: Session,
    },
    Deleted {
        info: Session,
    },
    Diff {
        session_id: String,
        diff: Vec<FileDiff>,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        error: SessionError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

// ============================================================================
// Session Status
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum RunStatus {
    #[default]
    Idle,
    Busy,
    Retrying {
        attempt: u32,
        #[serde(default)]
        message: String,
        /// Timestamp (millis) of the next retry attempt.
        #[serde(default)]
        next: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionStateEvent {
    StatusChanged {
        session_id: String,
        status: RunStatus,
    },
    Error {
        session_id: String,
        error: String,
    },
}

pub struct SessionStateManager {
    states: HashMap<String, RunStatus>,
}

impl SessionStateManager {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    pub fn set(&mut self, session_id: &str, status: RunStatus) {
        self.states.insert(session_id.to_string(), status);
    }

    pub fn get(&self, session_id: &str) -> RunStatus {
        self.states.get(session_id).cloned().unwrap_or_default()
    }

    pub fn is_busy(&self, session_id: &str) -> bool {
        matches!(
            self.get(session_id),
            RunStatus::Busy | RunStatus::Retrying { .. }
        )
    }

    pub fn assert_not_busy(&self, session_id: &str) -> Result<(), BusyError> {
        if self.is_busy(session_id) {
            return Err(BusyError {
                session_id: session_id.to_string(),
            });
        }
        Ok(())
    }

    pub fn set_busy(&mut self, session_id: &str) {
        self.set(session_id, RunStatus::Busy);
    }

    pub fn set_retrying(&mut self, session_id: &str, attempt: u32, message: String, next: i64) {
        self.set(
            session_id,
            RunStatus::Retrying {
                attempt,
                message,
                next,
            },
        );
    }

    pub fn set_idle(&mut self, session_id: &str) {
        self.set(session_id, RunStatus::Idle);
    }

    pub fn remove(&mut self, session_id: &str) {
        self.states.remove(session_id);
    }

    pub fn busy_sessions(&self) -> Vec<&str> {
        self.states
            .iter()
            .filter(|(_, s)| matches!(s, RunStatus::Busy | RunStatus::Retrying { .. }))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// List all session statuses.
    /// Matches TS `SessionStatus.list()` returning all tracked states.
    pub fn list(&self) -> &HashMap<String, RunStatus> {
        &self.states
    }
}

impl Default for SessionStateManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Session
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Session {
    inner: SessionRecord,
}

impl Session {
    pub fn record(&self) -> &SessionRecord {
        &self.inner
    }

    pub fn into_record(self) -> SessionRecord {
        self.inner
    }

    pub(crate) fn record_mut(&mut self) -> &mut SessionRecord {
        &mut self.inner
    }

    pub fn messages_mut(&mut self) -> &mut Vec<SessionMessage> {
        &mut self.inner.messages
    }

    pub fn insert_metadata(
        &mut self,
        key: impl Into<String>,
        value: serde_json::Value,
    ) -> Option<serde_json::Value> {
        let previous = self.inner.metadata.insert(key.into(), value);
        self.touch();
        previous
    }

    pub fn remove_metadata(&mut self, key: &str) -> Option<serde_json::Value> {
        let previous = self.inner.metadata.remove(key);
        self.touch();
        previous
    }

    pub fn set_directory(&mut self, directory: impl Into<String>) {
        let directory = directory.into();
        if self.inner.directory != directory {
            self.inner.directory = directory;
            self.touch();
        }
    }

    pub fn push_message(&mut self, message: SessionMessage) {
        self.inner.messages.push(message);
        self.touch();
    }
}

impl AsRef<SessionRecord> for Session {
    fn as_ref(&self) -> &SessionRecord {
        self.record()
    }
}

impl From<SessionRecord> for Session {
    fn from(inner: SessionRecord) -> Self {
        Self { inner }
    }
}

impl From<Session> for SessionRecord {
    fn from(session: Session) -> Self {
        session.into_record()
    }
}

impl Deref for Session {
    type Target = SessionRecord;

    fn deref(&self) -> &Self::Target {
        self.record()
    }
}

impl Session {
    const VERSION: &'static str = "1.0.0";
    const AUTO_TITLE_PENDING_REFINE_KEY: &'static str = "auto_title_pending_refine";

    fn serialize_context_kind(kind: SessionContextKind) -> serde_json::Value {
        serde_json::to_value(kind).expect("session context kind should serialize")
    }

    fn infer_legacy_context_kind(&self) -> SessionContextKind {
        if self.fork_origin_session_id().is_some()
            || self.fork_policy_frozen()
            || self
                .inner
                .messages
                .iter()
                .any(Self::is_imported_fork_history_message)
        {
            return SessionContextKind::ExplicitFullHistoryFork;
        }

        if self.inner.parent_id.is_some() {
            if self
                .inner
                .title
                .starts_with(LEGACY_STAGE_ATTACHED_SESSION_TITLE_PREFIX)
            {
                return SessionContextKind::SchedulerStageOutputSession;
            }
            return SessionContextKind::DelegatedSubsession;
        }

        SessionContextKind::RootSessionContinuity
    }

    /// Create a new session
    pub fn new(project_id: impl Into<String>, directory: impl Into<String>) -> Self {
        let now = Utc::now();
        let slug = Self::generate_slug();
        let mut metadata = HashMap::new();
        metadata.insert(
            SESSION_CONTEXT_KIND_METADATA_KEY.to_string(),
            Self::serialize_context_kind(SessionContextKind::RootSessionContinuity),
        );

        Self {
            inner: SessionRecord {
                id: format!("ses_{}", Uuid::new_v4().simple()),
                slug,
                project_id: project_id.into(),
                directory: directory.into(),
                parent_id: None,
                title: format!("New session - {}", now.to_rfc3339()),
                version: Self::VERSION.to_string(),
                time: SessionTime::default(),
                messages: Vec::new(),
                summary: None,
                share: None,
                revert: None,
                permission: None,
                usage: None,
                status: SessionStatus::Active,
                metadata,
                created_at: now,
                updated_at: now,
            },
        }
    }

    /// Create an attached session with an explicit context kind so attached-session
    /// ownership is never inferred from `parent_id` alone.
    pub fn attached_with_context_kind(parent: &Session, kind: SessionContextKind) -> Self {
        let now = Utc::now();
        let slug = Self::generate_slug();
        let parent_record = parent.record();
        let mut metadata = HashMap::new();
        metadata.insert(
            SESSION_CONTEXT_KIND_METADATA_KEY.to_string(),
            Self::serialize_context_kind(kind),
        );

        Self {
            inner: SessionRecord {
                id: format!("ses_{}", Uuid::new_v4().simple()),
                slug,
                project_id: parent_record.project_id.clone(),
                directory: parent_record.directory.clone(),
                parent_id: Some(parent_record.id.clone()),
                title: format!("Attached session - {}", now.to_rfc3339()),
                version: Self::VERSION.to_string(),
                time: SessionTime::default(),
                messages: Vec::new(),
                summary: None,
                share: None,
                revert: None,
                permission: parent_record.permission.clone(),
                usage: None,
                status: SessionStatus::Active,
                metadata,
                created_at: now,
                updated_at: now,
            },
        }
    }

    pub fn context_kind(&self) -> SessionContextKind {
        self.inner
            .metadata
            .get(SESSION_CONTEXT_KIND_METADATA_KEY)
            .and_then(|value| serde_json::from_value::<SessionContextKind>(value.clone()).ok())
            .unwrap_or_else(|| self.infer_legacy_context_kind())
    }

    pub fn ownership_summary(&self) -> SessionOwnershipSummary {
        self.context_kind().ownership_summary()
    }

    pub fn set_context_kind(&mut self, kind: SessionContextKind) {
        self.inner.metadata.insert(
            SESSION_CONTEXT_KIND_METADATA_KEY.to_string(),
            Self::serialize_context_kind(kind),
        );
        self.touch();
    }

    pub fn fork_origin_session_id(&self) -> Option<&str> {
        self.inner
            .metadata
            .get(FORK_ORIGIN_SESSION_ID_METADATA_KEY)
            .and_then(|value| value.as_str())
    }

    pub fn fork_origin_message_id(&self) -> Option<&str> {
        self.inner
            .metadata
            .get(FORK_ORIGIN_MESSAGE_ID_METADATA_KEY)
            .and_then(|value| value.as_str())
    }

    pub fn fork_policy_frozen(&self) -> bool {
        self.inner
            .metadata
            .get(FORK_POLICY_FROZEN_METADATA_KEY)
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    pub fn fork_history_mode(&self) -> SessionForkHistoryMode {
        self.inner
            .metadata
            .get(FORK_HISTORY_MODE_METADATA_KEY)
            .and_then(|value| serde_json::from_value::<SessionForkHistoryMode>(value.clone()).ok())
            .unwrap_or_default()
    }

    pub fn fork_history_message_limit(&self) -> Option<usize> {
        self.inner
            .metadata
            .get(FORK_HISTORY_MESSAGE_LIMIT_METADATA_KEY)
            .and_then(|value| serde_json::from_value::<usize>(value.clone()).ok())
    }

    pub fn fork_source_history_message_count(&self) -> Option<usize> {
        self.inner
            .metadata
            .get(FORK_SOURCE_HISTORY_MESSAGE_COUNT_METADATA_KEY)
            .and_then(|value| serde_json::from_value::<usize>(value.clone()).ok())
    }

    pub fn imported_history_message_count(&self) -> usize {
        self.inner
            .messages
            .iter()
            .filter(|message| Self::is_imported_fork_history_message(message))
            .count()
    }

    pub fn is_imported_fork_history_message_id(&self, message_id: &str) -> bool {
        self.get_message(message_id)
            .map(Self::is_imported_fork_history_message)
            .unwrap_or(false)
    }

    pub fn fork_explain(&self) -> Option<SessionForkExplain> {
        if self.context_kind() != SessionContextKind::ExplicitFullHistoryFork {
            return None;
        }

        let origin_session_id = self.fork_origin_session_id()?.to_string();
        Some(SessionForkExplain {
            origin_session_id,
            origin_message_id: self.fork_origin_message_id().map(str::to_string),
            history_mode: self.fork_history_mode(),
            history_message_limit: self.fork_history_message_limit(),
            source_history_messages: self.fork_source_history_message_count(),
            imported_history_messages: self.imported_history_message_count(),
            policy_frozen: self.fork_policy_frozen(),
            frozen_policy_keys: self.fork_frozen_policy_keys(),
            cache_stability_keys: self.fork_cache_stability_keys(),
            lifecycle: SessionForkLifecycleExplain {
                usage_replay_scope: SessionForkLifecycleScope::LocalOnly,
                revert_scope: SessionForkLifecycleScope::LocalOnly,
                recovery_scope: SessionForkLifecycleScope::LocalOnly,
                compaction_scope: SessionForkLifecycleScope::ForkPromptSurface,
            },
        })
    }

    pub fn is_imported_fork_history_message(message: &SessionMessage) -> bool {
        message
            .metadata
            .get(FORK_IMPORTED_HISTORY_METADATA_KEY)
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    fn copy_fork_policy_metadata_from(source: &Session, target: &mut Session) {
        for key in FORK_POLICY_METADATA_ALLOWLIST {
            if let Some(value) = source.record().metadata.get(*key).cloned() {
                target.insert_metadata((*key).to_string(), value);
            }
        }
        target.insert_metadata(FORK_POLICY_FROZEN_METADATA_KEY, serde_json::json!(true));
    }

    fn fork_frozen_policy_keys(&self) -> Vec<String> {
        FORK_POLICY_METADATA_ALLOWLIST
            .iter()
            .filter(|key| self.inner.metadata.contains_key(**key))
            .map(|key| (*key).to_string())
            .collect()
    }

    fn fork_cache_stability_keys(&self) -> Vec<String> {
        FORK_CACHE_STABILITY_METADATA_KEYS
            .iter()
            .filter(|key| self.inner.metadata.contains_key(**key))
            .map(|key| (*key).to_string())
            .collect()
    }

    fn imported_fork_history_message(
        source_session_id: &str,
        target_session_id: &str,
        source_message: &SessionMessage,
    ) -> SessionMessage {
        let mut message = source_message.clone();
        message.session_id = target_session_id.to_string();
        message.metadata.insert(
            FORK_IMPORTED_HISTORY_METADATA_KEY.to_string(),
            serde_json::json!(true),
        );
        message
            .metadata
            .entry(FORK_ORIGIN_SESSION_ID_METADATA_KEY.to_string())
            .or_insert_with(|| serde_json::json!(source_session_id));
        message
            .metadata
            .entry(FORK_ORIGIN_MESSAGE_ID_METADATA_KEY.to_string())
            .or_insert_with(|| serde_json::json!(&source_message.id));
        message
    }

    fn generate_slug() -> String {
        let uuid_part = &Uuid::new_v4().simple().to_string()[..8];
        format!("session-{}", uuid_part)
    }

    /// Check if title is a default generated title
    pub fn is_default_title(&self) -> bool {
        let prefix = if self.inner.parent_id.is_some() {
            if self.inner.title.starts_with("Attached session - ") {
                "Attached session - "
            } else {
                // Read old generated titles without reintroducing the legacy label.
                "Child session - "
            }
        } else {
            "New session - "
        };

        if !self.inner.title.starts_with(prefix) {
            return false;
        }

        let timestamp_part = &self.inner.title[prefix.len()..];
        chrono::DateTime::parse_from_rfc3339(timestamp_part).is_ok()
    }

    /// Whether the current title is an auto-generated placeholder that may be
    /// replaced by the refined LLM-generated title after the first assistant turn.
    pub fn allows_auto_title_regeneration(&self) -> bool {
        self.is_default_title()
            || self
                .inner
                .metadata
                .get(Self::AUTO_TITLE_PENDING_REFINE_KEY)
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
    }

    /// Get a forked title
    pub fn get_forked_title(&self) -> String {
        // Simple implementation without regex dependency
        if self.inner.title.ends_with(")") && self.inner.title.contains(" (fork #") {
            if let Some(pos) = self.inner.title.rfind(" (fork #") {
                let base = &self.inner.title[..pos];
                let num_part = &self.inner.title[pos + 8..self.inner.title.len() - 1];
                if let Ok(num) = num_part.parse::<u32>() {
                    return format!("{} (fork #{})", base, num + 1);
                }
            }
        }
        format!("{} (fork #1)", self.inner.title)
    }

    /// Touch the session (update timestamp)
    pub fn touch(&mut self) {
        let now = Utc::now();
        self.inner.time.updated = now.timestamp_millis();
        self.inner.updated_at = now;
    }

    // ========================================================================
    // Message Operations
    // ========================================================================

    /// Add a user message
    pub fn add_user_message(&mut self, text: impl Into<String>) -> &mut SessionMessage {
        let msg = SessionMessage::user(&self.inner.id, text);
        self.inner.messages.push(msg);
        self.touch();
        self.inner.messages.last_mut().unwrap()
    }

    /// Add a synthetic user message with optional attachments.
    pub fn add_synthetic_user_message(
        &mut self,
        text: impl Into<String>,
        attachments: &[crate::FilePart],
    ) -> &mut SessionMessage {
        let mut msg = SessionMessage::user(&self.inner.id, text);
        msg.mark_text_parts_synthetic();
        for attachment in attachments {
            msg.add_file(
                attachment.url.clone(),
                attachment
                    .filename
                    .clone()
                    .unwrap_or_else(|| "attachment".to_string()),
                attachment.mime.clone(),
            );
        }
        self.inner.messages.push(msg);
        self.touch();
        self.inner.messages.last_mut().unwrap()
    }

    /// Add an assistant message
    pub fn add_assistant_message(&mut self) -> &mut SessionMessage {
        let msg = SessionMessage::assistant(&self.inner.id);
        self.inner.messages.push(msg);
        self.touch();
        self.inner.messages.last_mut().unwrap()
    }

    /// Get the last user message
    pub fn last_user_message(&self) -> Option<&SessionMessage> {
        self.inner
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
    }

    pub fn last_owner_local_user_message(&self) -> Option<&SessionMessage> {
        self.inner.messages.iter().rev().find(|message| {
            matches!(message.role, MessageRole::User)
                && !Self::is_imported_fork_history_message(message)
        })
    }

    /// Get the last assistant message
    pub fn last_assistant_message(&self) -> Option<&SessionMessage> {
        self.inner
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
    }

    pub fn last_owner_local_assistant_message(&self) -> Option<&SessionMessage> {
        self.inner.messages.iter().rev().find(|message| {
            matches!(message.role, MessageRole::Assistant)
                && !Self::is_imported_fork_history_message(message)
        })
    }

    /// Get message count
    pub fn message_count(&self) -> usize {
        self.inner.messages.len()
    }

    /// Get a message by ID
    pub fn get_message(&self, id: &str) -> Option<&SessionMessage> {
        self.inner.messages.iter().find(|m| m.id == id)
    }

    /// Get a mutable message by ID
    pub fn get_message_mut(&mut self, id: &str) -> Option<&mut SessionMessage> {
        self.inner.messages.iter_mut().find(|m| m.id == id)
    }

    /// Remove a message by ID
    pub fn remove_message(&mut self, id: &str) -> Option<SessionMessage> {
        if let Some(pos) = self.inner.messages.iter().position(|m| m.id == id) {
            let msg = self.inner.messages.remove(pos);
            self.touch();
            Some(msg)
        } else {
            None
        }
    }

    // ========================================================================
    // Part-Level Operations
    // ========================================================================

    /// Update a message by replacing it entirely
    pub fn update_message(&mut self, msg: SessionMessage) -> Option<&SessionMessage> {
        if let Some(pos) = self.inner.messages.iter().position(|m| m.id == msg.id) {
            self.inner.messages[pos] = msg;
            self.touch();
            Some(&self.inner.messages[pos])
        } else {
            // New message - append
            self.inner.messages.push(msg);
            self.touch();
            self.inner.messages.last()
        }
    }

    /// Update a specific part within a message
    pub fn update_part(&mut self, msg_id: &str, part: MessagePart) -> Option<&MessagePart> {
        let part_id = part.id.clone();
        let msg = self.get_message_mut(msg_id)?;
        if let Some(pos) = msg.parts.iter().position(|p| p.id == part_id) {
            msg.parts[pos] = part;
        } else {
            msg.parts.push(part);
        }
        self.touch();
        // Return reference to the part
        let msg = self.get_message(msg_id)?;
        msg.parts.iter().find(|p| p.id == part_id)
    }

    /// Remove a specific part from a message
    pub fn remove_part(&mut self, msg_id: &str, part_id: &str) -> Option<MessagePart> {
        let msg = self.get_message_mut(msg_id)?;
        if let Some(pos) = msg.parts.iter().position(|p| p.id == part_id) {
            let removed = msg.parts.remove(pos);
            self.touch();
            Some(removed)
        } else {
            None
        }
    }

    // ========================================================================
    // Usage Aggregation
    // ========================================================================

    /// Aggregate usage across all assistant messages in the session
    pub fn get_usage(&self) -> SessionUsage {
        let mut usage = SessionUsage::default();
        for msg in &self.inner.messages {
            if matches!(msg.role, MessageRole::Assistant) {
                if let Some(ref msg_usage) = msg.usage {
                    if !Self::is_imported_fork_history_message(msg) {
                        usage.input_tokens += msg_usage.input_tokens;
                        usage.output_tokens += msg_usage.output_tokens;
                        usage.reasoning_tokens += msg_usage.reasoning_tokens;
                        usage.cache_write_tokens += msg_usage.cache_write_tokens;
                        usage.cache_read_tokens += msg_usage.cache_read_tokens;
                        usage.cache_miss_tokens += msg_usage.cache_miss_tokens;
                        usage.total_cost += msg_usage.total_cost;
                    }
                    usage.context_tokens = msg_usage.live_context_tokens().unwrap_or_default();
                }
            }
        }
        usage
    }

    pub fn latest_request_context_tokens(&self) -> Option<u64> {
        self.inner.messages.iter().rev().find_map(|message| {
            if !matches!(message.role, MessageRole::Assistant) {
                return None;
            }
            if Self::is_imported_fork_history_message(message) {
                return None;
            }
            message
                .usage
                .as_ref()
                .and_then(crate::message::MessageUsage::request_context_tokens)
        })
    }

    pub fn usage_books(&self) -> SessionUsageBooks {
        let usage = self.get_usage();
        SessionUsageBooks {
            request_context_tokens: self.latest_request_context_tokens(),
            live_context_tokens: usage.live_context_tokens(),
            workflow_cumulative: usage.workflow_usage_summary(),
        }
    }

    /// Share the session (set share URL)
    pub fn share_session(&mut self, url: impl Into<String>) {
        self.inner.share = Some(SessionShare { url: url.into() });
        self.touch();
    }

    /// Unshare the session
    pub fn unshare_session(&mut self) {
        self.inner.share = None;
        self.touch();
    }

    /// Compute diff summary from messages
    pub fn diff(&self) -> Vec<FileDiff> {
        self.inner
            .summary
            .as_ref()
            .and_then(|s| s.diffs.clone())
            .unwrap_or_default()
    }

    // ========================================================================
    // Setters
    // ========================================================================

    /// Set the title
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.inner.title = title.into();
        self.inner
            .metadata
            .remove(Self::AUTO_TITLE_PENDING_REFINE_KEY);
        self.touch();
    }

    /// Set an immediate auto-generated title that should still be replaced by
    /// the refined LLM title after the first completed turn.
    pub fn set_auto_title(&mut self, title: impl Into<String>) {
        self.inner.title = title.into();
        self.inner.metadata.insert(
            Self::AUTO_TITLE_PENDING_REFINE_KEY.to_string(),
            serde_json::json!(true),
        );
        self.touch();
    }

    /// Set the archived status
    pub fn set_archived(&mut self, time: Option<i64>) {
        self.inner.time.archived = time.or_else(|| Some(Utc::now().timestamp_millis()));
        self.inner.status = SessionStatus::Archived;
        self.touch();
    }

    /// Set the permission ruleset
    pub fn set_permission(&mut self, permission: PermissionRuleset) {
        self.inner.permission = Some(permission);
        self.touch();
    }

    /// Set the revert information
    pub fn set_revert(&mut self, revert: SessionRevert) {
        self.inner.revert = Some(revert);
        self.touch();
    }

    /// Clear the revert information
    pub fn clear_revert(&mut self) {
        self.inner.revert = None;
        self.touch();
    }

    /// Set the summary
    pub fn set_summary(&mut self, summary: SessionSummary) {
        self.inner.summary = Some(summary);
        self.touch();
    }

    /// Set the share information
    pub fn set_share(&mut self, share: SessionShare) {
        self.inner.share = Some(share);
        self.touch();
    }

    /// Clear the share information
    pub fn clear_share(&mut self) {
        self.inner.share = None;
        self.touch();
    }

    /// Update usage statistics
    pub fn update_usage(&mut self, usage: SessionUsage) {
        self.inner.usage = Some(usage);
        self.touch();
    }

    /// Start compacting
    pub fn start_compacting(&mut self) {
        self.inner.time.compacting = Some(Utc::now().timestamp_millis());
        self.inner.status = SessionStatus::Compacting;
    }

    /// Finish compacting
    pub fn finish_compacting(&mut self) {
        self.inner.time.compacting = None;
        self.inner.status = SessionStatus::Active;
        self.touch();
    }

    /// Mark as completed
    pub fn complete(&mut self) {
        self.inner.status = SessionStatus::Completed;
        self.touch();
    }

    // ========================================================================
    // Serialization Helpers
    // ========================================================================

    /// Convert to a database row representation
    pub fn to_row(&self) -> SessionRow {
        SessionRow {
            id: self.inner.id.clone(),
            slug: self.inner.slug.clone(),
            project_id: self.inner.project_id.clone(),
            directory: self.inner.directory.clone(),
            parent_id: self.inner.parent_id.clone(),
            title: self.inner.title.clone(),
            version: self.inner.version.clone(),
            time_created: self.inner.time.created,
            time_updated: self.inner.time.updated,
            time_compacting: self.inner.time.compacting,
            time_archived: self.inner.time.archived,
            share_url: self.inner.share.as_ref().map(|s| s.url.clone()),
            summary_additions: self.inner.summary.as_ref().map(|s| s.additions),
            summary_deletions: self.inner.summary.as_ref().map(|s| s.deletions),
            summary_files: self.inner.summary.as_ref().map(|s| s.files),
            revert: self.inner.revert.clone(),
            permission: self.inner.permission.clone(),
        }
    }

    /// Create from a database row representation
    pub fn from_row(row: SessionRow) -> Self {
        let summary = if row.summary_additions.is_some()
            || row.summary_deletions.is_some()
            || row.summary_files.is_some()
        {
            Some(SessionSummary {
                additions: row.summary_additions.unwrap_or(0),
                deletions: row.summary_deletions.unwrap_or(0),
                files: row.summary_files.unwrap_or(0),
                diffs: None,
            })
        } else {
            None
        };

        let share = row.share_url.map(|url| SessionShare { url });

        let status = if row.time_archived.is_some() {
            SessionStatus::Archived
        } else if row.time_compacting.is_some() {
            SessionStatus::Compacting
        } else {
            SessionStatus::Active
        };

        let created_at = DateTime::from_timestamp_millis(row.time_created).unwrap_or_else(Utc::now);
        let updated_at = DateTime::from_timestamp_millis(row.time_updated).unwrap_or_else(Utc::now);

        Self {
            inner: SessionRecord {
                id: row.id,
                slug: row.slug,
                project_id: row.project_id,
                directory: row.directory,
                parent_id: row.parent_id,
                title: row.title,
                version: row.version,
                time: SessionTime {
                    created: row.time_created,
                    updated: row.time_updated,
                    compacting: row.time_compacting,
                    archived: row.time_archived,
                },
                messages: Vec::new(),
                summary,
                share,
                revert: row.revert,
                permission: row.permission,
                usage: None,
                status,
                metadata: HashMap::new(),
                created_at,
                updated_at,
            },
        }
    }
}

/// Database row representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time_created: i64,
    pub time_updated: i64,
    pub time_compacting: Option<i64>,
    pub time_archived: Option<i64>,
    pub share_url: Option<String>,
    pub summary_additions: Option<u64>,
    pub summary_deletions: Option<u64>,
    pub summary_files: Option<u64>,
    pub revert: Option<SessionRevert>,
    pub permission: Option<PermissionRuleset>,
}

// ============================================================================
// Session Manager
// ============================================================================

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    events: Vec<SessionEvent>,
    bus: Option<Arc<Bus>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            events: Vec::new(),
            bus: None,
        }
    }

    /// Create a new SessionManager with a Bus for event publishing
    pub fn with_bus(bus: Arc<Bus>) -> Self {
        Self {
            sessions: HashMap::new(),
            events: Vec::new(),
            bus: Some(bus),
        }
    }

    /// Publish an event to the Bus (fire-and-forget from sync context)
    fn publish_event(&self, def: &'static BusEventDef, properties: serde_json::Value) {
        if let Some(ref bus) = self.bus {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let bus = bus.clone();
                handle.spawn(async move {
                    bus.publish(def, properties).await;
                });
            }
        }
    }

    /// Publish a session info event (Created/Updated/Deleted)
    fn publish_session_event(&self, def: &'static BusEventDef, session: &Session) {
        if let Ok(json) = serde_json::to_value(session) {
            self.publish_event(def, serde_json::json!({ "info": json }));
        }
    }

    /// Publish a message event
    fn publish_message_event(&self, def: &'static BusEventDef, msg: &SessionMessage) {
        if let Ok(json) = serde_json::to_value(msg) {
            self.publish_event(def, serde_json::json!({ "info": json }));
        }
    }

    /// Publish a part event
    fn publish_part_event(&self, def: &'static BusEventDef, part: &MessagePart) {
        if let Ok(json) = serde_json::to_value(part) {
            self.publish_event(def, serde_json::json!({ "part": json }));
        }
    }

    /// Publish a part delta event (streaming text updates)
    pub fn publish_part_delta(
        &self,
        session_id: &str,
        message_id: &str,
        part_id: &str,
        field: &str,
        delta: &str,
    ) {
        self.publish_event(
            &PART_DELTA_EVENT,
            serde_json::json!({
                "sessionID": session_id,
                "messageID": message_id,
                "partID": part_id,
                "field": field,
                "delta": delta,
            }),
        );
    }

    /// Create a new session
    pub fn create(
        &mut self,
        project_id: impl Into<String>,
        directory: impl Into<String>,
    ) -> Session {
        let session = Session::new(project_id, directory);
        self.sessions
            .insert(session.record().id.clone(), session.clone());
        self.events.push(SessionEvent::Created {
            info: session.clone(),
        });

        // Publish to Bus
        self.publish_session_event(&SESSION_CREATED_EVENT, &session);

        // Plugin hook: session.start — notify plugins of new session
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let session_id = session.record().id.clone();
            handle.spawn(async move {
                rocode_plugin::trigger(
                    HookContext::new(HookEvent::SessionStart).with_session(&session_id),
                )
                .await;
            });
        }

        session
    }

    /// Fork a session from a source snapshot with an explicit history import mode.
    pub fn fork(
        &mut self,
        session_id: &str,
        spec: SessionForkSpec<'_>,
    ) -> Result<Session, SessionForkError> {
        let original = self
            .sessions
            .get(session_id)
            .ok_or(SessionForkError::SessionNotFound)?;
        let forked_title = original.get_forked_title();

        let mut forked = Session::attached_with_context_kind(
            original,
            SessionContextKind::ExplicitFullHistoryFork,
        );
        Session::copy_fork_policy_metadata_from(original, &mut forked);

        let forked_id = forked.record().id.clone();
        let source_messages = if let Some(msg_id) = spec.message_id {
            let cutoff = original
                .record()
                .messages
                .iter()
                .position(|message| message.id == msg_id)
                .ok_or_else(|| SessionForkError::OriginMessageNotFound(msg_id.to_string()))?;
            &original.record().messages[..=cutoff]
        } else {
            original.record().messages.as_slice()
        };
        let imported_messages = match spec.history_mode {
            SessionForkHistoryMode::None => {
                if spec.history_message_limit.is_some() {
                    return Err(SessionForkError::InvalidRequest(
                        "`history_message_limit` is only valid when `history_mode=last_n`"
                            .to_string(),
                    ));
                }
                Vec::new()
            }
            SessionForkHistoryMode::All => {
                if spec.history_message_limit.is_some() {
                    return Err(SessionForkError::InvalidRequest(
                        "`history_message_limit` is only valid when `history_mode=last_n`"
                            .to_string(),
                    ));
                }
                source_messages
                    .iter()
                    .map(|message| {
                        Session::imported_fork_history_message(session_id, &forked_id, message)
                    })
                    .collect::<Vec<_>>()
            }
            SessionForkHistoryMode::LastN => {
                let limit = spec.history_message_limit.ok_or_else(|| {
                    SessionForkError::InvalidRequest(
                        "`history_mode=last_n` requires a positive `history_message_limit`"
                            .to_string(),
                    )
                })?;
                if limit == 0 {
                    return Err(SessionForkError::InvalidRequest(
                        "`history_message_limit` must be greater than 0".to_string(),
                    ));
                }
                let start = source_messages.len().saturating_sub(limit);
                source_messages[start..]
                    .iter()
                    .map(|message| {
                        Session::imported_fork_history_message(session_id, &forked_id, message)
                    })
                    .collect::<Vec<_>>()
            }
        };

        {
            let forked_record = forked.record_mut();
            forked_record.parent_id = None;
            forked_record.title = forked_title;
            forked_record.messages = imported_messages;
            forked_record.metadata.insert(
                FORK_ORIGIN_SESSION_ID_METADATA_KEY.to_string(),
                serde_json::json!(session_id),
            );
            forked_record.metadata.insert(
                FORK_HISTORY_MODE_METADATA_KEY.to_string(),
                serde_json::to_value(spec.history_mode)
                    .expect("fork history mode should serialize"),
            );
            forked_record.metadata.insert(
                FORK_SOURCE_HISTORY_MESSAGE_COUNT_METADATA_KEY.to_string(),
                serde_json::json!(source_messages.len()),
            );
            if let Some(limit) = spec.history_message_limit {
                forked_record.metadata.insert(
                    FORK_HISTORY_MESSAGE_LIMIT_METADATA_KEY.to_string(),
                    serde_json::json!(limit),
                );
            }
            if let Some(msg_id) = spec.message_id {
                forked_record.metadata.insert(
                    FORK_ORIGIN_MESSAGE_ID_METADATA_KEY.to_string(),
                    serde_json::json!(msg_id),
                );
            }
        }

        self.sessions.insert(forked_id, forked.clone());
        self.events.push(SessionEvent::Created {
            info: forked.clone(),
        });
        self.publish_session_event(&SESSION_CREATED_EVENT, &forked);
        Ok(forked)
    }

    /// Set share info and publish session.updated.
    pub fn share(&mut self, session_id: &str, url: impl Into<String>) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_share(SessionShare { url: url.into() });
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Clear share info and publish session.updated.
    pub fn unshare(&mut self, session_id: &str) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.clear_share();
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set archived time and publish session.updated.
    pub fn set_archived(&mut self, session_id: &str, time: Option<i64>) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_archived(time);
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set permission rules and publish session.updated.
    pub fn set_permission(
        &mut self,
        session_id: &str,
        permission: PermissionRuleset,
    ) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_permission(permission);
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set revert info and publish session.updated.
    pub fn set_revert(&mut self, session_id: &str, revert: SessionRevert) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_revert(revert);
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Clear revert info and publish session.updated.
    pub fn clear_revert(&mut self, session_id: &str) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.clear_revert();
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set summary and publish session.updated.
    pub fn set_summary(&mut self, session_id: &str, summary: SessionSummary) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_summary(summary);
            session.clone()
        };
        self.events.push(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Publish command.executed event.
    pub fn publish_command_executed(
        &self,
        command_name: &str,
        session_id: &str,
        arguments: Vec<String>,
        message_id: &str,
    ) {
        self.publish_event(
            &COMMAND_EXECUTED_EVENT,
            serde_json::json!({
                "name": command_name,
                "sessionID": session_id,
                "arguments": arguments,
                "messageID": message_id,
            }),
        );
    }

    /// Get a session by ID
    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    /// Get a mutable session by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// List all sessions
    pub fn list(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    /// List sessions with filters
    pub fn list_filtered(&self, filter: SessionFilter) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| {
                if let Some(ref dir) = filter.directory {
                    if s.record().directory != *dir {
                        return false;
                    }
                }
                if filter.roots && s.record().parent_id.is_some() {
                    return false;
                }
                if let Some(start) = filter.start {
                    if s.record().time.updated < start {
                        return false;
                    }
                }
                if let Some(ref search) = filter.search {
                    if !Self::matches_list_search(s, search) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Get attached sessions of a session.
    pub fn attached_sessions(&self, parent_id: &str) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.record().parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    /// Delete a session
    pub fn delete(&mut self, id: &str) -> Option<Session> {
        let attached_sessions: Vec<String> = self
            .attached_sessions(id)
            .iter()
            .map(|s| s.record().id.clone())
            .collect();
        for attached_id in attached_sessions {
            self.delete(&attached_id);
        }

        let session = self.sessions.remove(id)?;
        self.events.push(SessionEvent::Deleted {
            info: session.clone(),
        });

        // Publish to Bus
        self.publish_session_event(&SESSION_DELETED_EVENT, &session);

        // Plugin hook: session.end — notify plugins of session deletion
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let session_id = session.record().id.clone();
            handle.spawn(async move {
                rocode_plugin::trigger(
                    HookContext::new(HookEvent::SessionEnd).with_session(&session_id),
                )
                .await;
            });
        }

        Some(session)
    }

    /// Update a session
    pub fn update(&mut self, session: Session) {
        let id = session.record().id.clone();
        self.sessions.insert(id, session.clone());
        self.events.push(SessionEvent::Updated {
            info: session.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &session);
    }

    /// Get events (and clear them)
    pub fn drain_events(&mut self) -> Vec<SessionEvent> {
        self.events.drain(..).collect()
    }

    /// Get session count
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    // ========================================================================
    // Message/Part Operations with Bus Publishing
    // ========================================================================

    /// Update a message in a session and publish Bus event
    pub fn update_message(&mut self, session_id: &str, msg: SessionMessage) -> Option<()> {
        let session = self.sessions.get_mut(session_id)?;
        session.update_message(msg.clone());
        self.publish_message_event(&MESSAGE_UPDATED_EVENT, &msg);
        Some(())
    }

    /// Remove a message from a session and publish Bus event
    pub fn remove_message(&mut self, session_id: &str, message_id: &str) -> Option<SessionMessage> {
        let session = self.sessions.get_mut(session_id)?;
        let msg = session.remove_message(message_id)?;
        self.publish_event(
            &MESSAGE_REMOVED_EVENT,
            serde_json::json!({
                "sessionID": session_id,
                "messageID": message_id,
            }),
        );
        Some(msg)
    }

    /// Update a part in a message and publish Bus event
    pub fn update_part(
        &mut self,
        session_id: &str,
        message_id: &str,
        part: MessagePart,
    ) -> Option<()> {
        let session = self.sessions.get_mut(session_id)?;
        session.update_part(message_id, part.clone());
        self.publish_part_event(&PART_UPDATED_EVENT, &part);
        Some(())
    }

    /// Remove a part from a message and publish Bus event
    pub fn remove_part(
        &mut self,
        session_id: &str,
        message_id: &str,
        part_id: &str,
    ) -> Option<MessagePart> {
        let session = self.sessions.get_mut(session_id)?;
        let part = session.remove_part(message_id, part_id)?;
        self.publish_event(
            &PART_REMOVED_EVENT,
            serde_json::json!({
                "sessionID": session_id,
                "messageID": message_id,
                "partID": part_id,
            }),
        );
        Some(part)
    }

    /// Publish a session error event
    pub fn publish_error(&self, session_id: Option<&str>, error: serde_json::Value) {
        let mut props = serde_json::json!({ "error": error });
        if let Some(sid) = session_id {
            props["sessionID"] = serde_json::Value::String(sid.to_string());
        }
        self.publish_event(&SESSION_ERROR_EVENT, props);
    }

    /// Publish a session diff event
    pub fn publish_diff(&self, session_id: &str, diffs: &[FileDiff]) {
        if let Ok(diff_json) = serde_json::to_value(diffs) {
            self.publish_event(
                &SESSION_DIFF_EVENT,
                serde_json::json!({
                    "sessionID": session_id,
                    "diff": diff_json,
                }),
            );
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Set the Bus for event publishing (can be called after construction)
    pub fn set_bus(&mut self, bus: Arc<Bus>) {
        self.bus = Some(bus);
    }

    fn matches_list_search(session: &Session, search: &str) -> bool {
        let needle = search.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return true;
        }

        debug_assert_eq!(SESSION_LIST_SEARCH_FIELDS, ["title"]);

        session
            .record()
            .title
            .to_ascii_lowercase()
            .contains(&needle)
    }
}

/// Filter options for listing sessions
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    pub directory: Option<String>,
    pub roots: bool,
    pub start: Option<i64>,
    /// Free-text search over lightweight list fields only.
    ///
    /// This must never inspect detail-only sources such as metadata,
    /// persisted telemetry snapshots, messages, or derived runtime state.
    pub search: Option<String>,
    pub limit: Option<usize>,
}

// ============================================================================
// Busy Error
// ============================================================================

#[derive(Debug, Clone)]
pub struct BusyError {
    pub session_id: String,
}

impl std::fmt::Display for BusyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session {} is busy", self.session_id)
    }
}

impl std::error::Error for BusyError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::{timeout, Duration};

    fn fork_spec<'a>(
        message_id: Option<&'a str>,
        history_mode: SessionForkHistoryMode,
        history_message_limit: Option<usize>,
    ) -> SessionForkSpec<'a> {
        SessionForkSpec {
            message_id,
            history_mode,
            history_message_limit,
        }
    }

    #[test]
    fn test_session_creation() {
        let session = Session::new("project-1", "/path/to/project");
        assert!(session.id.starts_with("ses_"));
        assert!(session.title.starts_with("New session"));
        assert!(session.parent_id.is_none());
        assert_eq!(session.status, SessionStatus::Active);
        assert_eq!(
            session.context_kind(),
            SessionContextKind::RootSessionContinuity
        );
    }

    #[test]
    fn test_typed_attached_session() {
        let parent = Session::new("project-1", "/path/to/project");
        let child =
            Session::attached_with_context_kind(&parent, SessionContextKind::DelegatedSubsession);

        assert!(child.parent_id.is_some());
        assert_eq!(child.parent_id.clone().unwrap(), parent.id);
        assert!(child.title.starts_with("Attached session"));
        assert_eq!(
            child.context_kind(),
            SessionContextKind::DelegatedSubsession
        );
    }

    #[test]
    fn test_explicit_attached_session_kind() {
        let parent = Session::new("project-1", "/path/to/project");
        let child = Session::attached_with_context_kind(
            &parent,
            SessionContextKind::SchedulerStageOutputSession,
        );

        assert_eq!(
            child.context_kind(),
            SessionContextKind::SchedulerStageOutputSession
        );
        assert!(!child.context_kind().owns_prompt_continuity());
    }

    #[test]
    fn test_legacy_attached_session_without_context_kind_infers_delegated_subsession() {
        let parent = Session::new("project-1", "/path/to/project");
        let mut child =
            Session::attached_with_context_kind(&parent, SessionContextKind::DelegatedSubsession);
        child.remove_metadata(SESSION_CONTEXT_KIND_METADATA_KEY);

        assert_eq!(
            child.context_kind(),
            SessionContextKind::DelegatedSubsession
        );
    }

    #[test]
    fn test_legacy_stage_attached_session_without_context_kind_infers_stage_output_sink() {
        let parent = Session::new("project-1", "/path/to/project");
        let mut child = Session::attached_with_context_kind(
            &parent,
            SessionContextKind::SchedulerStageOutputSession,
        );
        child.remove_metadata(SESSION_CONTEXT_KIND_METADATA_KEY);
        child.set_title("Stage: Execution Orchestration - atlas");

        assert_eq!(
            child.context_kind(),
            SessionContextKind::SchedulerStageOutputSession
        );
    }

    #[test]
    fn test_ownership_summary_marks_provider_model_as_request_shape_only() {
        let session = Session::new("project-1", "/path/to/project");

        let ownership = session.ownership_summary();

        assert_eq!(
            ownership.context_kind,
            SessionContextKind::RootSessionContinuity
        );
        assert!(ownership.owns_prompt_continuity);
        assert!(ownership.compact_owner);
        assert_eq!(
            ownership.provider_model_role,
            rocode_types::SessionProviderModelRole::RequestShapeOnly
        );
        assert_eq!(
            ownership.workflow_usage_role,
            rocode_types::SessionWorkflowUsageRole::ObservationOnly
        );
    }

    #[test]
    fn test_stage_output_sink_is_not_compact_owner() {
        let parent = Session::new("project-1", "/path/to/project");
        let child = Session::attached_with_context_kind(
            &parent,
            SessionContextKind::SchedulerStageOutputSession,
        );

        let ownership = child.ownership_summary();

        assert_eq!(
            ownership.handoff_mode,
            rocode_types::SessionHandoffMode::StageOutputSink
        );
        assert!(!ownership.owns_prompt_continuity);
        assert!(!ownership.compact_owner);
    }

    #[test]
    fn test_add_messages() {
        let mut session = Session::new("project-1", "/path/to/project");

        session.add_user_message("Hello");
        assert_eq!(session.message_count(), 1);

        session.add_assistant_message();
        assert_eq!(session.message_count(), 2);
    }

    #[test]
    fn test_session_manager() {
        let mut manager = SessionManager::new();

        let session = manager.create("project-1", "/path/to/project");
        assert!(manager.get(&session.id).is_some());
        assert_eq!(manager.count(), 1);

        let child =
            Session::attached_with_context_kind(&session, SessionContextKind::DelegatedSubsession);
        manager.update(child.clone());
        assert!(child.parent_id.is_some());

        manager.delete(&session.id);
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_fork_title() {
        let session = Session::new("project-1", "/path/to/project");
        let title1 = session.get_forked_title();
        assert!(title1.ends_with("(fork #1)"));

        let mut temp = session.clone();
        temp.set_title(title1);
        let title2 = temp.get_forked_title();
        assert!(title2.ends_with("(fork #2)"));
    }

    #[test]
    fn test_session_manager_fork_marks_explicit_full_history_fork() {
        let mut manager = SessionManager::new();
        let session = manager.create("project-1", "/path/to/project");
        let forked = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::All, None),
            )
            .expect("expected forked session");

        assert_eq!(
            forked.context_kind(),
            SessionContextKind::ExplicitFullHistoryFork
        );
        assert!(forked.parent_id.is_none());
    }

    #[test]
    fn test_legacy_fork_without_context_kind_infers_full_history_fork() {
        let mut manager = SessionManager::new();
        let session = manager.create("project-1", "/path/to/project");
        let mut forked = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::All, None),
            )
            .expect("expected forked session");
        forked.remove_metadata(SESSION_CONTEXT_KIND_METADATA_KEY);

        assert_eq!(
            forked.context_kind(),
            SessionContextKind::ExplicitFullHistoryFork
        );
    }

    #[test]
    fn test_session_manager_fork_freezes_policy_and_marks_imported_history() {
        let mut manager = SessionManager::new();
        let mut session = manager.create("project-1", "/path/to/project");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("openai"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("gpt-4.1"));
        session.insert_metadata("scheduler_profile".to_string(), serde_json::json!("atlas"));
        session.insert_metadata("last_ingress_source".to_string(), serde_json::json!("web"));
        session.insert_metadata("custom_session_key".to_string(), serde_json::json!(true));
        session.add_user_message("hello");
        let selected_message_id = {
            let assistant = session.add_assistant_message();
            assistant.add_text("answer");
            assistant.usage = Some(MessageUsage {
                input_tokens: 120,
                output_tokens: 30,
                reasoning_tokens: 4,
                cache_write_tokens: 2,
                cache_read_tokens: 3,
                cache_miss_tokens: 1,
                context_tokens: 120,
                total_cost: 0.42,
            });
            assistant.id.clone()
        };
        manager.update(session.clone());

        let forked = manager
            .fork(
                &session.id,
                fork_spec(
                    Some(&selected_message_id),
                    SessionForkHistoryMode::All,
                    None,
                ),
            )
            .expect("expected forked session");

        assert_eq!(
            forked.context_kind(),
            SessionContextKind::ExplicitFullHistoryFork
        );
        assert!(forked.parent_id.is_none());
        assert_eq!(forked.fork_origin_session_id(), Some(session.id.as_str()));
        assert_eq!(
            forked.fork_origin_message_id(),
            Some(selected_message_id.as_str())
        );
        assert!(forked.fork_policy_frozen());
        assert_eq!(
            forked.metadata.get("model_provider"),
            Some(&serde_json::json!("openai"))
        );
        assert_eq!(
            forked.metadata.get("model_id"),
            Some(&serde_json::json!("gpt-4.1"))
        );
        assert_eq!(
            forked.metadata.get("scheduler_profile"),
            Some(&serde_json::json!("atlas"))
        );
        assert!(!forked.metadata.contains_key("last_ingress_source"));
        assert!(!forked.metadata.contains_key("custom_session_key"));
        assert_eq!(forked.fork_history_mode(), SessionForkHistoryMode::All);
        assert_eq!(forked.fork_history_message_limit(), None);
        assert_eq!(forked.fork_source_history_message_count(), Some(2));
        assert_eq!(forked.record().messages.len(), 2);
        assert_eq!(forked.imported_history_message_count(), 2);
        assert!(forked.record().messages.iter().all(|message| {
            message.session_id == forked.id
                && message
                    .metadata
                    .get(FORK_IMPORTED_HISTORY_METADATA_KEY)
                    .and_then(|value| value.as_bool())
                    == Some(true)
                && message
                    .metadata
                    .get(FORK_ORIGIN_SESSION_ID_METADATA_KEY)
                    .and_then(|value| value.as_str())
                    == Some(session.id.as_str())
        }));
    }

    #[test]
    fn test_fork_usage_excludes_imported_history_but_preserves_live_context() {
        let mut manager = SessionManager::new();
        let mut session = manager.create("project-1", "/path/to/project");
        session.add_user_message("hello");
        {
            let assistant = session.add_assistant_message();
            assistant.usage = Some(MessageUsage {
                input_tokens: 100,
                output_tokens: 20,
                reasoning_tokens: 5,
                cache_write_tokens: 1,
                cache_read_tokens: 2,
                cache_miss_tokens: 3,
                context_tokens: 150,
                total_cost: 0.15,
            });
        }
        manager.update(session.clone());

        let mut forked = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::All, None),
            )
            .expect("expected forked session");

        let inherited_usage = forked.get_usage();
        assert_eq!(inherited_usage.input_tokens, 0);
        assert_eq!(inherited_usage.output_tokens, 0);
        assert_eq!(inherited_usage.reasoning_tokens, 0);
        assert_eq!(inherited_usage.cache_write_tokens, 0);
        assert_eq!(inherited_usage.cache_read_tokens, 0);
        assert_eq!(inherited_usage.cache_miss_tokens, 0);
        assert_eq!(inherited_usage.context_tokens, 150);
        assert_eq!(inherited_usage.total_cost, 0.0);
        assert_eq!(forked.latest_request_context_tokens(), None);

        let local = forked.add_assistant_message();
        local.usage = Some(MessageUsage {
            input_tokens: 40,
            output_tokens: 10,
            reasoning_tokens: 1,
            cache_write_tokens: 0,
            cache_read_tokens: 4,
            cache_miss_tokens: 0,
            context_tokens: 70,
            total_cost: 0.07,
        });

        let local_usage = forked.get_usage();
        assert_eq!(local_usage.input_tokens, 40);
        assert_eq!(local_usage.output_tokens, 10);
        assert_eq!(local_usage.reasoning_tokens, 1);
        assert_eq!(local_usage.cache_read_tokens, 4);
        assert_eq!(local_usage.context_tokens, 70);
        assert!((local_usage.total_cost - 0.07).abs() < f64::EPSILON);
        assert_eq!(forked.latest_request_context_tokens(), Some(70));

        let fork_explain = forked.fork_explain().expect("fork explain should exist");
        assert_eq!(fork_explain.origin_session_id, session.id);
        assert_eq!(fork_explain.history_mode, SessionForkHistoryMode::All);
        assert_eq!(fork_explain.history_message_limit, None);
        assert_eq!(fork_explain.source_history_messages, Some(2));
        assert_eq!(fork_explain.imported_history_messages, 2);
        assert!(fork_explain.policy_frozen);
        assert!(fork_explain.frozen_policy_keys.is_empty());
        assert!(fork_explain.cache_stability_keys.is_empty());
        assert_eq!(
            fork_explain.lifecycle.usage_replay_scope,
            SessionForkLifecycleScope::LocalOnly
        );
        assert_eq!(
            fork_explain.lifecycle.revert_scope,
            SessionForkLifecycleScope::LocalOnly
        );
        assert_eq!(
            fork_explain.lifecycle.recovery_scope,
            SessionForkLifecycleScope::LocalOnly
        );
        assert_eq!(
            fork_explain.lifecycle.compaction_scope,
            SessionForkLifecycleScope::ForkPromptSurface
        );
        assert!(forked.is_imported_fork_history_message_id(&forked.record().messages[0].id));
        assert!(forked.last_owner_local_user_message().is_none());
        assert_eq!(
            forked
                .last_owner_local_assistant_message()
                .and_then(|message| message.usage.as_ref())
                .and_then(MessageUsage::request_context_tokens),
            Some(70)
        );
    }

    #[test]
    fn test_session_manager_fork_last_n_imports_tail_and_records_mode() {
        let mut manager = SessionManager::new();
        let mut session = manager.create("project-1", "/path/to/project");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("openai"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("gpt-4.1"));
        session.add_user_message("one");
        session.add_user_message("two");
        session.add_user_message("three");
        manager.update(session.clone());

        let forked = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::LastN, Some(2)),
            )
            .expect("expected forked session");

        assert_eq!(forked.record().messages.len(), 2);
        assert_eq!(forked.fork_history_mode(), SessionForkHistoryMode::LastN);
        assert_eq!(forked.fork_history_message_limit(), Some(2));
        assert_eq!(forked.fork_source_history_message_count(), Some(3));

        let fork_explain = forked.fork_explain().expect("fork explain should exist");
        assert_eq!(fork_explain.history_mode, SessionForkHistoryMode::LastN);
        assert_eq!(fork_explain.history_message_limit, Some(2));
        assert_eq!(fork_explain.source_history_messages, Some(3));
        assert_eq!(fork_explain.imported_history_messages, 2);
        assert_eq!(
            fork_explain.lifecycle.compaction_scope,
            SessionForkLifecycleScope::ForkPromptSurface
        );
    }

    #[test]
    fn test_session_manager_fork_none_records_lineage_without_imported_history() {
        let mut manager = SessionManager::new();
        let mut session = manager.create("project-1", "/path/to/project");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("openai"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("gpt-4.1"));
        session.add_user_message("hello");
        manager.update(session.clone());

        let forked = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::None, None),
            )
            .expect("expected forked session");

        assert_eq!(forked.record().messages.len(), 0);
        assert_eq!(forked.fork_history_mode(), SessionForkHistoryMode::None);
        assert_eq!(forked.fork_source_history_message_count(), Some(1));

        let fork_explain = forked.fork_explain().expect("fork explain should exist");
        assert_eq!(fork_explain.history_mode, SessionForkHistoryMode::None);
        assert_eq!(fork_explain.imported_history_messages, 0);
        assert_eq!(fork_explain.source_history_messages, Some(1));
        assert_eq!(
            fork_explain.lifecycle.recovery_scope,
            SessionForkLifecycleScope::LocalOnly
        );
    }

    #[test]
    fn test_session_manager_fork_rejects_invalid_history_mode_contract() {
        let mut manager = SessionManager::new();
        let session = manager.create("project-1", "/path/to/project");

        let error = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::LastN, None),
            )
            .expect_err("expected invalid fork contract");
        assert!(matches!(error, SessionForkError::InvalidRequest(_)));

        let error = manager
            .fork(
                &session.id,
                fork_spec(None, SessionForkHistoryMode::All, Some(2)),
            )
            .expect_err("expected invalid fork contract");
        assert!(matches!(error, SessionForkError::InvalidRequest(_)));
    }

    #[test]
    fn test_auto_title_can_be_refined_but_manual_title_cannot() {
        let mut session = Session::new("project-1", "/path");
        assert!(session.allows_auto_title_regeneration());

        session.set_auto_title("Immediate Title");
        assert!(session.allows_auto_title_regeneration());

        session.set_title("Manual Title");
        assert!(!session.allows_auto_title_regeneration());
    }

    #[test]
    fn test_sanitize_display_text_strips_pseudo_tool_markup() {
        let cleaned = sanitize_display_text(
            "before\nminimax:tool_call (minimax:tool_call)\n<invoke name=\"Bash\">\n<parameter name=\"command\">pwd</parameter>\n</invoke>\nafter",
        );
        assert_eq!(cleaned, "before\nafter");
    }

    #[test]
    fn test_sanitize_display_text_strips_trailing_provider_response_json() {
        let cleaned = sanitize_display_text(
            "Answer body\n\n{\"id\":\"chatcmpl_1\",\"object\":\"chat.completion\",\"model\":\"glm\",\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"Answer body\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}",
        );
        assert_eq!(cleaned, "Answer body");
    }

    #[test]
    fn test_update_message() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();

        let updated = session.get_message(&msg_id).unwrap().clone();
        session.update_message(updated);
        assert!(session.get_message(&msg_id).is_some());
    }

    #[test]
    fn test_update_message_new() {
        let mut session = Session::new("project-1", "/path");
        let new_msg = SessionMessage::user(&session.id, "Brand new");
        let new_id = new_msg.id.clone();
        session.update_message(new_msg);
        assert!(session.get_message(&new_id).is_some());
        assert_eq!(session.message_count(), 1);
    }

    #[test]
    fn list_search_only_matches_lightweight_title_field() {
        let mut manager = SessionManager::new();
        let mut session = manager.create("project-1", "/path/to/project");
        session.set_title("Atlas Planning Session");
        session.insert_metadata("model_provider".to_string(), serde_json::json!("zhipuai"));
        session.insert_metadata("model_id".to_string(), serde_json::json!("glm-5.1"));
        session.insert_metadata(
            "telemetry".to_string(),
            serde_json::json!({
                "last_run_status": "completed",
                "stage_summaries": [{"stage_name": "Route"}]
            }),
        );
        manager.update(session.clone());

        let by_title = manager.list_filtered(SessionFilter {
            search: Some("planning".to_string()),
            ..Default::default()
        });
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].id, session.id);

        let by_metadata = manager.list_filtered(SessionFilter {
            search: Some("zhipuai".to_string()),
            ..Default::default()
        });
        assert!(by_metadata.is_empty());

        let by_telemetry = manager.list_filtered(SessionFilter {
            search: Some("route".to_string()),
            ..Default::default()
        });
        assert!(by_telemetry.is_empty());
    }

    #[test]
    fn test_update_part() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();
        let part_id = msg.parts[0].id.clone();

        // Update existing part
        let replacement = MessagePart {
            id: part_id.clone(),
            part_type: crate::PartType::Text {
                text: "Updated".into(),
                synthetic: None,
                ignored: None,
            },
            created_at: Utc::now(),
            message_id: None,
        };
        let result = session.update_part(&msg_id, replacement);
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, part_id);
    }

    #[test]
    fn test_remove_part() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();
        let part_id = msg.parts[0].id.clone();

        let removed = session.remove_part(&msg_id, &part_id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, part_id);
        assert_eq!(session.get_message(&msg_id).unwrap().parts.len(), 0);
    }

    #[test]
    fn test_remove_part_not_found() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();

        let removed = session.remove_part(&msg_id, "nonexistent");
        assert!(removed.is_none());
    }

    #[test]
    fn test_share_unshare() {
        let mut session = Session::new("project-1", "/path");

        session.share_session("https://example.com/share/123");
        assert!(session.share.is_some());
        assert_eq!(
            session.share.as_ref().unwrap().url,
            "https://example.com/share/123"
        );

        session.unshare_session();
        assert!(session.share.is_none());
    }

    #[test]
    fn test_get_usage_empty() {
        let session = Session::new("project-1", "/path");
        let usage = session.get_usage();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.total_cost, 0.0);
    }

    #[test]
    fn test_get_usage_aggregation() {
        let mut session = Session::new("project-1", "/path");

        // Add an assistant message with usage
        let msg = session.add_assistant_message();
        msg.usage = Some(MessageUsage {
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: 10,
            cache_write_tokens: 20,
            cache_read_tokens: 30,
            cache_miss_tokens: 0,
            context_tokens: 100,
            total_cost: 0.005,
        });

        // Add another assistant message with usage
        let msg2 = session.add_assistant_message();
        msg2.usage = Some(MessageUsage {
            input_tokens: 200,
            output_tokens: 100,
            reasoning_tokens: 20,
            cache_write_tokens: 40,
            cache_read_tokens: 60,
            cache_miss_tokens: 0,
            context_tokens: 200,
            total_cost: 0.010,
        });

        // Add a user message (should not be counted)
        let user_msg = session.add_user_message("test");
        user_msg.usage = Some(MessageUsage {
            input_tokens: 999,
            output_tokens: 999,
            reasoning_tokens: 999,
            cache_write_tokens: 999,
            cache_read_tokens: 999,
            cache_miss_tokens: 999,
            context_tokens: 999,
            total_cost: 999.0,
        });

        let usage = session.get_usage();
        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.output_tokens, 150);
        assert_eq!(usage.reasoning_tokens, 30);
        assert_eq!(usage.cache_write_tokens, 60);
        assert_eq!(usage.cache_read_tokens, 90);
        assert!((usage.total_cost - 0.015).abs() < f64::EPSILON);
    }

    #[test]
    fn test_diff_empty() {
        let session = Session::new("project-1", "/path");
        let diffs = session.diff();
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_with_summary() {
        let mut session = Session::new("project-1", "/path");
        session.set_summary(SessionSummary {
            additions: 10,
            deletions: 5,
            files: 2,
            diffs: Some(vec![
                FileDiff {
                    path: "src/main.rs".into(),
                    additions: 7,
                    deletions: 3,
                },
                FileDiff {
                    path: "src/lib.rs".into(),
                    additions: 3,
                    deletions: 2,
                },
            ]),
        });

        let diffs = session.diff();
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].path, "src/main.rs");
    }

    #[tokio::test]
    async fn session_share_publishes_updated_event() {
        let bus = Arc::new(Bus::new());
        let mut manager = SessionManager::with_bus(bus.clone());
        let session = manager.create("project-1", "/path");
        let mut rx = bus.subscribe_channel();

        let updated = manager
            .share(&session.id, "https://share.opencode.ai/test")
            .expect("session should exist");
        assert_eq!(
            updated.share.as_ref().map(|share| share.url.as_str()),
            Some("https://share.opencode.ai/test")
        );

        let event = timeout(Duration::from_secs(1), async {
            loop {
                let event = rx.recv().await.expect("event channel closed");
                if event.event_type == SESSION_UPDATED_EVENT.event_type {
                    break event;
                }
            }
        })
        .await
        .expect("event timeout");
        assert_eq!(event.event_type, SESSION_UPDATED_EVENT.event_type);
        assert_eq!(event.properties["info"]["id"], session.id);
        assert_eq!(
            event.properties["info"]["share"]["url"],
            "https://share.opencode.ai/test"
        );
    }

    #[tokio::test]
    async fn command_executed_event_is_published() {
        let bus = Arc::new(Bus::new());
        let manager = SessionManager::with_bus(bus.clone());
        let mut rx = bus.subscribe_channel();

        manager.publish_command_executed(
            "review",
            "session-1",
            vec!["--fast".to_string()],
            "message-1",
        );

        let event = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed");
        assert_eq!(event.event_type, COMMAND_EXECUTED_EVENT.event_type);
        assert_eq!(event.properties["name"], "review");
        assert_eq!(event.properties["sessionID"], "session-1");
        assert_eq!(event.properties["arguments"][0], "--fast");
        assert_eq!(event.properties["messageID"], "message-1");
    }
}
