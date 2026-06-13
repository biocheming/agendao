//! 土 — Per-session state authority.
//!
//! Each active session has exactly one SessionStore holding
//! the canonical Signal truth for messages, run status, and active tools.

use revue::prelude::*;

/// A message in the transcript.
#[derive(Clone)]
pub struct TranscriptMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    /// True while the message is still receiving deltas.
    pub streaming: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    Thinking,
    Stage,
    System,
}

/// Run status of a session.
#[derive(Clone, Debug, PartialEq)]
pub enum RunStatus {
    Idle,
    Sending,
    Running,
    WaitingUser,
    Error(String),
}

/// Per-session state — all fields are Signals for reactive rendering.
#[derive(Clone)]
pub struct SessionStore {
    pub session_id: Signal<Option<String>>,
    pub messages: Signal<Vec<TranscriptMessage>>,
    pub run_status: Signal<RunStatus>,
    pub error: Signal<Option<String>>,
    /// Map of active tool_call_id → tool_name.
    pub active_tools: Signal<Vec<(String, String)>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            session_id: signal(None),
            messages: signal(Vec::new()),
            run_status: signal(RunStatus::Idle),
            error: signal(None),
            active_tools: signal(Vec::new()),
        }
    }

    // ── User messages ──

    pub fn add_user_message(&self, content: &str, id: &str) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptMessage {
                id: id.to_string(),
                role: MessageRole::User,
                content: content.to_string(),
                streaming: false,
            });
        });
    }

    // ── Streaming output blocks ──

    /// Append streaming text to the current assistant message.
    /// Creates a new placeholder if no streaming message exists.
    pub fn append_message_text(&self, block_id: &str, text: &str) {
        self.messages.update(|msgs| {
            let streaming = msgs.last_mut().filter(|m| m.streaming);
            if let Some(msg) = streaming {
                msg.content.push_str(text);
            } else {
                msgs.push(TranscriptMessage {
                    id: block_id.to_string(),
                    role: MessageRole::Assistant,
                    content: text.to_string(),
                    streaming: true,
                });
            }
        });
    }

    /// Mark the streaming assistant message as complete.
    pub fn finalize_message(&self, _block_id: &str) {
        self.messages.update(|msgs| {
            if let Some(msg) = msgs.last_mut().filter(|m| m.streaming) {
                msg.streaming = false;
            }
        });
    }

    /// Append thinking/reasoning text.
    pub fn append_thinking_text(&self, _block_id: &str, text: &str) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptMessage {
                id: String::new(),
                role: MessageRole::Thinking,
                content: text.to_string(),
                streaming: true,
            });
        });
    }

    /// Finalize the thinking block.
    pub fn finalize_thinking(&self, _block_id: &str) {
        self.messages.update(|msgs| {
            if let Some(msg) = msgs.last_mut().filter(|m| m.streaming && m.role == MessageRole::Thinking) {
                msg.streaming = false;
            }
        });
    }

    /// Append a scheduler stage update.
    pub fn append_stage_block(&self, _block_id: &str, text: &str) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptMessage {
                id: String::new(),
                role: MessageRole::Stage,
                content: text.to_string(),
                streaming: true,
            });
        });
    }

    /// Finalize a stage block.
    pub fn finalize_stage(&self, _block_id: &str) {
        self.messages.update(|msgs| {
            if let Some(msg) = msgs.last_mut().filter(|m| m.streaming && m.role == MessageRole::Stage) {
                msg.streaming = false;
            }
        });
    }

    // ── Tools ──

    pub fn set_active_tool(&self, id: String, name: String) {
        self.active_tools.update(|tools| {
            tools.retain(|(tid, _)| tid != &id);
            tools.push((id, name));
        });
    }

    // ── Session ID ──

    pub fn set_session_id(&self, id: &str) {
        self.session_id.set(Some(id.to_string()));
    }

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
    }

    #[test]
    fn add_user_message() {
        let store = SessionStore::new();
        store.add_user_message("hi", "u1");
        let msgs = store.messages.get();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi");
        assert!(!msgs[0].streaming);
    }

    #[test]
    fn streaming_message_deltas_accumulate() {
        let store = SessionStore::new();
        store.append_message_text("b1", "Hello");
        store.append_message_text("b1", " World");
        let msgs = store.messages.get();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello World");
        assert!(msgs[0].streaming);
    }

    #[test]
    fn finalize_stops_streaming() {
        let store = SessionStore::new();
        store.append_message_text("b1", "Hi");
        store.finalize_message("b1");
        assert!(!store.messages.get()[0].streaming);
    }

    #[test]
    fn finalize_before_streaming_is_noop() {
        let store = SessionStore::new();
        store.finalize_message("x");
        assert!(store.messages.get().is_empty());
    }

    #[test]
    fn set_active_tool_tracks_multiple() {
        let store = SessionStore::new();
        store.set_active_tool("t1".into(), "bash".into());
        store.set_active_tool("t2".into(), "read".into());
        assert_eq!(store.active_tools.get().len(), 2);
    }
}
