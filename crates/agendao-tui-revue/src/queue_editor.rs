//! Local queue-body editor state.
//!
//! The editor is deliberately detached from `QueueSummary`: it owns only a
//! draft and the authority coordinates needed to submit it.  A server receipt
//! never rewrites the projected queue; the next authoritative snapshot does.

use agendao_types::submission::{QueueEditRequest, QueueMutationDisposition, QueuedInputSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEditDraft {
    pub session_id: String,
    pub item_id: String,
    pub base_revision: u64,
    pub original: String,
    pub content: String,
    pub pending_revision: Option<u64>,
}

impl QueueEditDraft {
    pub fn begin(session_id: impl Into<String>, item: &QueuedInputSnapshot, revision: u64) -> Self {
        Self {
            session_id: session_id.into(),
            item_id: item.item_id.clone(),
            base_revision: revision,
            original: item.content.clone(),
            content: item.content.clone(),
            pending_revision: None,
        }
    }

    pub fn set_content(&mut self, content: impl Into<String>) {
        self.content = content.into();
    }
    pub fn is_dirty(&self) -> bool {
        self.content != self.original
    }
    pub fn cancel(self) {}

    pub fn request(&mut self, draft_revision: u64) -> Option<QueueEditRequest> {
        if !self.is_dirty() || self.pending_revision.is_some() {
            return None;
        }
        self.pending_revision = Some(draft_revision);
        let (request, _ctx) = crate::command_gateway::CommandGateway::prepare_queue_edit(
            self.session_id.clone(),
            self.item_id.clone(),
            self.base_revision,
            draft_revision,
            self.content.clone(),
        );
        Some(request)
    }

    /// Returns true only for a receipt belonging to this draft.  Rejected or
    /// transport failures intentionally leave the draft intact.
    pub fn settle(
        &mut self,
        response: &Result<QueueMutationDisposition, String>,
        current_session: &str,
        current_draft_revision: u64,
    ) -> bool {
        if current_session != self.session_id {
            return false;
        }
        let Some(sent_draft_revision) = self.pending_revision.take() else {
            return false;
        };
        match response {
            Ok(QueueMutationDisposition::Applied {
                session_id,
                item_id,
                queue_revision,
                ..
            }) if session_id == &self.session_id
                && item_id == &self.item_id
                && *queue_revision > self.base_revision =>
            {
                self.base_revision = *queue_revision;
                if current_draft_revision == sent_draft_revision {
                    self.original = self.content.clone();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item() -> QueuedInputSnapshot {
        QueuedInputSnapshot {
            item_id: "q1".into(),
            client_request_id: "r".into(),
            content: "old".into(),
            position: 0,
            created_at_ms: 0,
        }
    }

    #[test]
    fn stale_or_wrong_session_receipt_keeps_draft() {
        let mut d = QueueEditDraft::begin("s1", &item(), 4);
        d.set_content("new");
        let _ = d.request(9).unwrap();
        let bad = Ok(QueueMutationDisposition::Applied {
            session_id: "s2".into(),
            item_id: "q1".into(),
            position: 0,
            queue_revision: 5,
        });
        assert!(!d.settle(&bad, "s1", 9));
        assert_eq!(d.content, "new");
    }

    #[test]
    fn failure_preserves_draft_and_duplicate_receipt_is_ignored() {
        let mut d = QueueEditDraft::begin("s1", &item(), 4);
        d.set_content("new");
        let _ = d.request(9).unwrap();
        assert!(!d.settle(&Err("503".into()), "s1", 9));
        let ok = Ok(QueueMutationDisposition::Applied {
            session_id: "s1".into(),
            item_id: "q1".into(),
            position: 0,
            queue_revision: 5,
        });
        assert!(!d.settle(&ok, "s1", 9));
        assert_eq!(d.content, "new");
    }

    #[test]
    fn modified_during_flight_is_retained_and_can_retry() {
        let mut d = QueueEditDraft::begin("s1", &item(), 4);
        d.set_content("first");
        let _ = d.request(9).unwrap();
        d.set_content("second");
        let ok = Ok(QueueMutationDisposition::Applied {
            session_id: "s1".into(),
            item_id: "q1".into(),
            position: 0,
            queue_revision: 5,
        });
        assert!(!d.settle(&ok, "s1", 10));
        assert_eq!(d.content, "second");
        assert!(d.request(11).is_some());
    }
}
