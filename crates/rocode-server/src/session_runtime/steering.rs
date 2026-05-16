//! Owner-session steering queue authority (Constitution §5: single owner).
//!
//! Only the server runtime holds the steering queue. TUI/CLI/Web submit
//! requests; the prompt runtime consumes them at tool boundaries (§9).

use std::collections::{HashMap, VecDeque};

use crate::session_runtime::state::PendingSteeringMessageSummary;

/// Full steering message — the internal type held in the queue.
#[derive(Debug, Clone)]
pub struct PendingSteeringMessage {
    pub id: String,
    pub owner_session_id: String,
    pub text: String,
    pub created_at: i64,
    pub source_session_id: Option<String>,
    pub deliver_at: String,
}

impl PendingSteeringMessage {
    pub fn to_summary(&self) -> PendingSteeringMessageSummary {
        PendingSteeringMessageSummary {
            id: self.id.clone(),
            owner_session_id: self.owner_session_id.clone(),
            text: self.text.clone(),
            created_at: self.created_at,
            source_session_id: self.source_session_id.clone(),
            deliver_at: self.deliver_at.clone(),
        }
    }
}

/// Process-wide steering queue store. Keyed by owner session id.
/// Constitution §5: this is the only steering queue authority.
#[derive(Debug, Default)]
pub struct SessionSteeringQueueStore {
    queues: HashMap<String, VecDeque<PendingSteeringMessage>>,
}

impl SessionSteeringQueueStore {
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
        }
    }

    /// Enqueue a steering message for an owner session.
    pub fn enqueue(&mut self, owner_session_id: &str, message: PendingSteeringMessage) {
        self.queues
            .entry(owner_session_id.to_string())
            .or_default()
            .push_back(message);
    }

    /// Drain all pending steering messages for an owner session.
    /// Returns them in FIFO order.
    pub fn drain(&mut self, owner_session_id: &str) -> Vec<PendingSteeringMessage> {
        let mut queue = self
            .queues
            .remove(owner_session_id)
            .unwrap_or_default();
        let drained: Vec<_> = queue.drain(..).collect();
        if !queue.is_empty() {
            self.queues
                .insert(owner_session_id.to_string(), queue);
        }
        drained
    }

    /// Peek at pending count without consuming.
    pub fn pending_count(&self, owner_session_id: &str) -> usize {
        self.queues
            .get(owner_session_id)
            .map(|q| q.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(id: &str, owner: &str, text: &str) -> PendingSteeringMessage {
        PendingSteeringMessage {
            id: id.to_string(),
            owner_session_id: owner.to_string(),
            text: text.to_string(),
            created_at: 1,
            source_session_id: None,
            deliver_at: "next_tool_boundary".to_string(),
        }
    }

    #[test]
    fn drain_empties_queue_and_returns_all_messages() {
        let mut store = SessionSteeringQueueStore::new();
        store.enqueue("s1", make_msg("a", "s1", "first"));
        store.enqueue("s1", make_msg("b", "s1", "second"));
        store.enqueue("s2", make_msg("c", "s2", "other"));

        assert_eq!(store.pending_count("s1"), 2);
        assert_eq!(store.pending_count("s2"), 1);

        let drained = store.drain("s1");
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].text, "first");
        assert_eq!(drained[1].text, "second");

        // After drain: queue for s1 is empty.
        assert_eq!(store.pending_count("s1"), 0);
        // s2 unaffected.
        assert_eq!(store.pending_count("s2"), 1);
    }
}
