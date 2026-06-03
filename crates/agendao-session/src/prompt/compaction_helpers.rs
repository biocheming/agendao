use crate::{PartType, Session, SessionMessage};

pub fn should_compact(messages: &[SessionMessage], max_tokens: u64) -> bool {
    let total_chars: usize = messages
        .iter()
        .map(|m| {
            m.parts
                .iter()
                .filter_map(|p| match &p.part_type {
                    PartType::Text { text, .. } => Some(text.len()),
                    _ => None,
                })
                .sum::<usize>()
        })
        .sum();

    let estimated_tokens = total_chars / 4;
    estimated_tokens > max_tokens as usize
}

pub fn trigger_compaction(session: &mut Session, messages: &[SessionMessage]) -> Option<String> {
    if !should_compact(messages, 100000) {
        return None;
    }

    let text_content: String = messages
        .iter()
        .rev()
        .take(10)
        .flat_map(|m| {
            m.parts.iter().filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
        })
        .collect::<Vec<_>>()
        .join("\n");

    let summary = format!(
        "[Context Compaction Triggered]\nRecent messages summarized:\n{}",
        text_content.chars().take(500).collect::<String>()
    );

    // Persist the compaction summary as a Compaction part on a new assistant message.
    let mut compaction_msg = SessionMessage::assistant(session.id.clone());
    compaction_msg.parts.push(crate::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4()),
        part_type: PartType::Compaction {
            summary: summary.clone(),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    session.push_message(compaction_msg);

    // Set the compacting timestamp on the session.
    session.record_mut().time.compacting = Some(chrono::Utc::now().timestamp_millis());
    session.touch();

    Some(summary)
}
