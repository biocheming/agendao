use std::collections::HashMap;

use agendao_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};

#[derive(Default)]
pub(crate) struct LiveSnapshotAccumulator {
    values: HashMap<String, String>,
}

impl LiveSnapshotAccumulator {
    pub(crate) fn update(
        &mut self,
        session_id: &str,
        identity: &LiveMessagePartIdentity,
        incoming: &str,
    ) -> Option<String> {
        let key = format!(
            "{}:{}:{}",
            session_id, identity.message_id, identity.part_key
        );
        if identity.phase == LivePartPhase::End {
            self.values.remove(&key);
            return None;
        }
        if !matches!(
            identity.phase,
            LivePartPhase::Append | LivePartPhase::Snapshot
        ) {
            return None;
        }

        let entry = self.values.entry(key).or_default();
        if identity.phase == LivePartPhase::Append {
            entry.reserve(incoming.len());
            entry.push_str(incoming);
        } else {
            merge_snapshot_text_in_place(entry, incoming);
        }
        Some(entry.clone())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
}

pub(crate) fn coalesced_text_field(identity: &LiveMessagePartIdentity) -> Option<&'static str> {
    match identity.part_kind {
        LiveMessagePartKind::AssistantText | LiveMessagePartKind::AssistantReasoning => {
            Some("text")
        }
        LiveMessagePartKind::ToolCall => Some("detail"),
        _ => None,
    }
}

pub(crate) fn merge_snapshot_text_in_place(existing: &mut String, incoming: &str) {
    if incoming.is_empty() {
        return;
    }
    if existing.is_empty() {
        existing.reserve(incoming.len());
        existing.push_str(incoming);
        return;
    }
    if incoming.starts_with(existing.as_str()) {
        existing.push_str(&incoming[existing.len()..]);
        return;
    }
    if existing.starts_with(incoming) {
        return;
    }

    let overlap = suffix_prefix_overlap(existing, incoming);
    existing.reserve(incoming.len() - overlap);
    existing.push_str(&incoming[overlap..]);
}

fn suffix_prefix_overlap(existing: &str, incoming: &str) -> usize {
    let max = existing.len().min(incoming.len());
    for size in (1..=max).rev() {
        if existing.is_char_boundary(existing.len() - size)
            && incoming.is_char_boundary(size)
            && existing[existing.len() - size..] == incoming[..size]
        {
            return size;
        }
    }
    0
}
