//! 土 — Per-session state authority.
//!
//! Each active session has exactly one SessionStore which holds
//! the canonical truth for messages, run status, and metadata.

use revue::prelude::*;

/// A message in the transcript.
#[derive(Clone)]
pub struct TranscriptMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Run status of a session.
#[derive(Clone, Debug, PartialEq)]
pub enum RunStatus {
    Idle,
    Sending,
    Running,
    Error(String),
}

/// Per-session state.
#[derive(Clone)]
pub struct SessionStore {
    /// The session ID (None until created on server).
    pub session_id: Signal<Option<String>>,
    /// Messages in the transcript.
    pub messages: Signal<Vec<TranscriptMessage>>,
    /// Current run status.
    pub run_status: Signal<RunStatus>,
    /// Error message, if any.
    pub error: Signal<Option<String>>,
    /// Pending user input (for optimistic UI).
    pub pending_input: Signal<Option<String>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            session_id: signal(None),
            messages: signal(Vec::new()),
            run_status: signal(RunStatus::Idle),
            error: signal(None),
            pending_input: signal(None),
        }
    }

    /// Add a user message to the transcript (optimistic).
    pub fn add_user_message(&self, content: &str, id: &str) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptMessage {
                id: id.to_string(),
                role: MessageRole::User,
                content: content.to_string(),
            });
        });
    }

    /// Add an assistant message (from API response).
    pub fn add_assistant_message(&self, content: &str, id: &str) {
        self.messages.update(|msgs| {
            let replace = msgs.last().map_or(false, |last| {
                last.role == MessageRole::Assistant && last.content.is_empty()
            });
            if replace {
                if let Some(last) = msgs.last_mut() {
                    last.id = id.to_string();
                    last.content = content.to_string();
                }
            } else {
                msgs.push(TranscriptMessage {
                    id: id.to_string(),
                    role: MessageRole::Assistant,
                    content: content.to_string(),
                });
            }
        });
    }

    /// Set the session ID after server creates the session.
    pub fn set_session_id(&self, id: &str) {
        self.session_id.set(Some(id.to_string()));
    }

    /// Get current session ID.
    pub fn get_session_id(&self) -> Option<String> {
        self.session_id.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_store_is_idle() {
        let store = SessionStore::new();
        assert_eq!(store.run_status.get(), RunStatus::Idle);
        assert!(store.messages.get().is_empty());
        assert!(store.session_id.get().is_none());
    }

    #[test]
    fn add_user_message_appends_to_transcript() {
        let store = SessionStore::new();
        store.add_user_message("hello", "msg-1");
        let msgs = store.messages.get();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[0].role, MessageRole::User);
    }

    #[test]
    fn add_assistant_message_appends() {
        let store = SessionStore::new();
        store.add_assistant_message("response", "msg-2");
        let msgs = store.messages.get();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, MessageRole::Assistant);
    }

    #[test]
    fn set_session_id_updates() {
        let store = SessionStore::new();
        store.set_session_id("ses_abc");
        assert_eq!(store.get_session_id(), Some("ses_abc".into()));
    }
}
