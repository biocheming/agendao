//! 水 — EventBus: protocol-agnostic event channel.
//!
//! All three agendao transports (local direct / unix socket / HTTP SSE)
//! deliver `FrontendEvent`. EventBus provides a single `UnboundedReceiver`
//! that the main loop drains on each Tick, regardless of transport.

use agendao_server_core::frontend_events::FrontendEvent;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Protocol-agnostic event bus.
///
/// The sender can be handed to any transport task (direct bridge,
/// socket subscriber, SSE listener). The main event loop drains the
/// receiver on each Tick and applies events to SessionStore.
pub struct EventBus {
    rx: UnboundedReceiver<FrontendEvent>,
    tx: UnboundedSender<FrontendEvent>,
}

impl EventBus {
    /// Create a new event bus with an unbounded channel.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { rx, tx }
    }

    /// Return a sender that transports can push events into.
    pub fn sender(&self) -> UnboundedSender<FrontendEvent> {
        self.tx.clone()
    }

    /// Drain all pending events from the channel (non-blocking).
    pub fn drain(&mut self) -> Vec<FrontendEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_bus_drain_empty() {
        let mut bus = EventBus::new();
        assert!(bus.drain().is_empty());
    }

    #[test]
    fn event_bus_send_and_drain() {
        let mut bus = EventBus::new();
        let tx = bus.sender();
        tx.send(FrontendEvent::QuestionUpsert {
            session_id: "s1".into(),
            question: agendao_client::QuestionInfo {
                id: "q1".into(),
                session_id: "s1".into(),
                questions: vec!["ok?".into()],
                options: None,
                items: vec![],
            },
        })
        .unwrap();

        let events = bus.drain();
        assert_eq!(events.len(), 1);
        if let FrontendEvent::QuestionUpsert { session_id, .. } = &events[0] {
            assert_eq!(session_id, "s1");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn event_bus_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EventBus>();
    }

    /// Integration test: EventBus → SessionStore full pipeline.
    #[test]
    fn event_bus_to_session_pipeline() {
        let mut bus = EventBus::new();
        let session = crate::store::session_store::SessionStore::new();
        let tx = bus.sender();

        // Simulate SSE: OutputBlockAppended (assistant message delta)
        tx.send(FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({"kind": "message", "phase": "delta", "text": "Hello"}),
            id: Some("msg-1".into()),
            live_identity: None,
        }).unwrap();

        tx.send(FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({"kind": "message", "phase": "delta", "text": " World"}),
            id: Some("msg-1".into()),
            live_identity: None,
        }).unwrap();

        // Drain and apply
        let events = bus.drain();
        assert_eq!(events.len(), 2);
        for e in &events {
            crate::telemetry::event_handler::apply_frontend_event(e, &session);
        }

        // Verify: session has 1 assistant message with "Hello World"
        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            crate::store::types::TranscriptBlock::AssistantMsg { content, .. } => {
                assert_eq!(content, "Hello World");
            }
            _ => panic!("expected AssistantMsg"),
        }
    }

    /// Integration test: ToolCall lifecycle.
    /// Uses the web-aligned schema: `kind: "tool"` + `phase: "start" | "running" |
    /// "done" | "error"` + `name`. The previous test used a legacy
    /// `tool_call` kind that the server never emits, masking the
    /// "transcript stays empty" bug we hit in real runs.
    #[test]
    fn event_bus_tool_call_lifecycle() {
        let mut bus = EventBus::new();
        let session = crate::store::session_store::SessionStore::new();
        let tx = bus.sender();

        tx.send(FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({"kind": "tool", "phase": "start", "name": "bash", "detail": "ls -la"}),
            id: Some("t1".into()),
            live_identity: None,
        }).unwrap();

        let events = bus.drain();
        for e in &events {
            crate::telemetry::event_handler::apply_frontend_event(e, &session);
        }

        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            crate::store::types::TranscriptBlock::ToolCall { name, params, .. } => {
                assert_eq!(name, "bash");
                assert_eq!(params, "ls -la");
            }
            _ => panic!("expected ToolCall block"),
        }
    }
}
