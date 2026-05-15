use rocode_types::SanitizerAction;

use crate::{Content, ContentPart, Message, Role};

const INTERRUPTED_TOOL_RESULT_TEXT: &str = "[Tool execution was interrupted]";

#[derive(Debug, Clone, Copy, Default)]
pub struct SanitizerOptions {
    pub drop_thinking_only_assistant: bool,
    /// When true, synthetic repairs (interrupted tool placeholders) are
    /// skipped but their corresponding actions are still recorded.
    pub skip_synthetic_repair: bool,
}

/// Sanitize messages for protocol transport (backward-compatible signature).
pub fn sanitize_messages_for_protocol(
    messages: &[Message],
    options: SanitizerOptions,
) -> Vec<Message> {
    sanitize_messages_for_protocol_with_actions(messages, options, None)
}

/// Sanitize messages and record every cleanup action into the supplied vector.
pub fn sanitize_messages_for_protocol_with_actions(
    messages: &[Message],
    options: SanitizerOptions,
    mut actions_out: Option<&mut Vec<SanitizerAction>>,
) -> Vec<Message> {
    let mut sanitized = Vec::new();
    let mut pending_tool_use_ids = Vec::new();
    let mut seen_tool_use_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut dedup_rewrites: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut dedup_suffix: u64 = 0;

    let skip_synthetic = options.skip_synthetic_repair;

    // Helper to record an action without consuming the outer Option.
    let mut record = |action: SanitizerAction| {
        if let Some(ref mut actions) = actions_out {
            actions.push(action);
        }
    };

    for message in messages {
        match message.role {
            Role::System | Role::User => {
                flush_pending(
                    &mut sanitized,
                    &mut pending_tool_use_ids,
                    &mut record,
                    skip_synthetic,
                );
                sanitized.push(message.clone());
            }
            Role::Assistant => {
                flush_pending(
                    &mut sanitized,
                    &mut pending_tool_use_ids,
                    &mut record,
                    skip_synthetic,
                );
                // Skip whitespace-only or empty-content assistant messages.
                if is_empty_or_whitespace_assistant(message) {
                    record(SanitizerAction::AssistantMalformedPlaceholder);
                    continue;
                }
                if options.drop_thinking_only_assistant && is_thinking_only_assistant(message) {
                    record(SanitizerAction::ThinkingOnlyAssistant);
                    continue;
                }
                // Strip trailing invalid thinking blocks from otherwise valid messages.
                let (cleaned, trailing_stripped) = strip_trailing_invalid_thinking(message);
                if trailing_stripped {
                    record(SanitizerAction::TrailingInvalidThinkingBlock);
                }
                // Detect and resolve duplicate tool_use IDs.
                let ids = assistant_tool_use_ids(&cleaned);
                // Step 1: intra-message duplicates — remove the duplicate part entirely.
                let has_intra_dupes = has_intra_message_duplicate_ids(&ids);
                if has_intra_dupes {
                    let mut recorded = std::collections::HashSet::new();
                    for id in &ids {
                        if ids.iter().filter(|i| *i == id).count() > 1
                            && recorded.insert(id.clone())
                        {
                            record(SanitizerAction::DuplicateToolId {
                                tool_call_id: id.clone(),
                            });
                        }
                    }
                }
                let mut deduped = if has_intra_dupes {
                    deduplicate_tool_use_ids_in_message(&cleaned)
                } else {
                    cleaned
                };
                // Step 2: cross-message duplicates — rewrite the duplicate ID to a
                // unique suffix so the provider never sees the same id twice.
                deduped = rewrite_cross_message_duplicate_ids(
                    &deduped, &mut seen_tool_use_ids, &mut dedup_rewrites, &mut dedup_suffix, &mut record,
                );
                let deduped_ids = assistant_tool_use_ids(&deduped);
                pending_tool_use_ids = deduped_ids;
                sanitized.push(deduped);
            }
            Role::Tool => {
                // Rewrite tool_result IDs that reference deduplicated tool_use IDs.
                let message = if !dedup_rewrites.is_empty() {
                    rewrite_tool_result_ids(message, &dedup_rewrites)
                } else {
                    message.clone()
                };
                let (tool_result_parts, text_parts) =
                    sanitize_tool_message_content(&message.content, &mut pending_tool_use_ids);
                // Detect orphaned tool results.
                let matched_ids: std::collections::HashSet<&str> = tool_result_parts
                    .iter()
                    .filter_map(|p| p.tool_result.as_ref().map(|tr| tr.tool_use_id.as_str()))
                    .collect();
                if let Content::Parts(parts) = &message.content {
                    for part in parts {
                        if let Some(tool_result) = &part.tool_result {
                            // Check against the rewritten ID if applicable.
                            let effective_id = dedup_rewrites
                                .get(&tool_result.tool_use_id)
                                .unwrap_or(&tool_result.tool_use_id);
                            if !matched_ids.contains(effective_id.as_str()) {
                                record(SanitizerAction::OrphanedToolResult {
                                    tool_call_id: tool_result.tool_use_id.clone(),
                                });
                            }
                        }
                    }
                }
                if !tool_result_parts.is_empty() {
                    sanitized.push(Message::tool_parts(tool_result_parts));
                }
                if !text_parts.is_empty() {
                    flush_pending(
                        &mut sanitized,
                        &mut pending_tool_use_ids,
                        &mut record,
                        skip_synthetic,
                    );
                    sanitized.push(Message {
                        role: Role::User,
                        content: Content::Parts(text_parts),
                        cache_control: None,
                        provider_options: None,
                    });
                }
            }
        }
    }

    flush_pending(
        &mut sanitized,
        &mut pending_tool_use_ids,
        &mut record,
        skip_synthetic,
    );
    sanitized
}

pub fn interrupted_tool_result_text() -> &'static str {
    INTERRUPTED_TOOL_RESULT_TEXT
}

pub fn sanitize_messages_for_text_protocol(messages: &[Message]) -> Vec<Message> {
    let sanitized = sanitize_messages_for_protocol(
        messages,
        SanitizerOptions {
            drop_thinking_only_assistant: true,
            ..Default::default()
        },
    );
    let mut projected = Vec::new();

    for message in sanitized {
        let text = content_visible_text_lossy(&message.content);
        if text.is_empty() || matches!(message.role, Role::Tool) {
            continue;
        }

        push_or_merge_text_message(&mut projected, message.role, text);
    }

    projected
}

pub fn content_visible_text_lossy(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Parts(parts) => parts
            .iter()
            .filter(|part| !matches!(part.content_type.as_str(), "reasoning" | "thinking"))
            .filter_map(|part| part.text.as_ref())
            .filter(|text| !text.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

// ── Internal helpers ────────────────────────────────────────────────────

fn assistant_tool_use_ids(message: &Message) -> Vec<String> {
    match &message.content {
        Content::Text(_) => Vec::new(),
        Content::Parts(parts) => parts
            .iter()
            .filter_map(|part| part.tool_use.as_ref().map(|tool_use| tool_use.id.clone()))
            .collect(),
    }
}

fn interrupted_tool_result_part(tool_use_id: String) -> ContentPart {
    ContentPart::tool_result(tool_use_id, INTERRUPTED_TOOL_RESULT_TEXT, Some(true))
}

fn flush_pending(
    messages: &mut Vec<Message>,
    pending_tool_use_ids: &mut Vec<String>,
    record: &mut dyn FnMut(SanitizerAction),
    skip_synthetic: bool,
) {
    if pending_tool_use_ids.is_empty() {
        return;
    }

    let ids: Vec<String> = pending_tool_use_ids.drain(..).collect();
    for id in &ids {
        record(SanitizerAction::OrphanedToolResult {
            tool_call_id: id.clone(),
        });
    }
    // Strict mode: record the action but don't inject synthetic placeholder.
    if !skip_synthetic {
        let tool_result_parts = ids.into_iter().map(interrupted_tool_result_part).collect();
        messages.push(Message::tool_parts(tool_result_parts));
    }
}

fn sanitize_tool_message_content(
    content: &Content,
    pending_tool_use_ids: &mut Vec<String>,
) -> (Vec<ContentPart>, Vec<ContentPart>) {
    match content {
        Content::Text(text) => {
            let text_parts = if text.is_empty() {
                Vec::new()
            } else {
                vec![ContentPart::text(text.clone())]
            };
            (Vec::new(), text_parts)
        }
        Content::Parts(parts) => {
            let mut tool_result_parts = Vec::new();
            let mut text_parts = Vec::new();

            for part in parts {
                if let Some(tool_result) = &part.tool_result {
                    if let Some(index) = pending_tool_use_ids
                        .iter()
                        .position(|pending_id| pending_id == &tool_result.tool_use_id)
                    {
                        pending_tool_use_ids.remove(index);
                        tool_result_parts.push(part.clone());
                    } else {
                        tracing::warn!(
                            tool_call_id = %tool_result.tool_use_id,
                            "dropping orphan tool_result without pending assistant tool_call"
                        );
                    }
                    continue;
                }

                if let Some(text) = &part.text {
                    if !text.is_empty() {
                        text_parts.push(ContentPart::text(text.clone()));
                    }
                }
            }

            (tool_result_parts, text_parts)
        }
    }
}

/// True when the assistant message has no meaningful visible content.
/// This catches whitespace-only, empty, and compaction placeholder messages.
fn is_empty_or_whitespace_assistant(message: &Message) -> bool {
    match &message.content {
        Content::Text(text) => text.trim().is_empty() || is_compaction_placeholder_text(text),
        Content::Parts(parts) => {
            let has_meaningful = parts.iter().any(|p| {
                p.tool_use.is_some()
                    || p.image_url.is_some()
                    || p.tool_result.is_some()
                    || p.text.as_ref().is_some_and(|t| {
                        let trimmed = t.trim();
                        !trimmed.is_empty() && !is_compaction_placeholder_text(trimmed)
                    })
            });
            !has_meaningful
        }
    }
}

fn is_compaction_placeholder_text(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed == "[compacted]"
        || trimmed == "[trimmed]"
        || trimmed == "[pruned]"
        || trimmed.starts_with("[compacted]")
        || trimmed.starts_with("[trimmed")
}

/// Strip trailing thinking/reasoning blocks from an assistant message.
/// Returns `(cleaned_message, was_stripped)`.
fn strip_trailing_invalid_thinking(message: &Message) -> (Message, bool) {
    let Content::Parts(parts) = &message.content else {
        return (message.clone(), false);
    };
    let last_meaningful = parts.iter().rposition(|p| {
        !matches!(p.content_type.as_str(), "reasoning" | "thinking")
            && (p.tool_use.is_some()
                || p.image_url.is_some()
                || p.text.as_ref().is_some_and(|t| !t.trim().is_empty()))
    });
    let Some(last_idx) = last_meaningful else {
        return (message.clone(), false);
    };
    let has_trailing_thinking = parts[last_idx + 1..]
        .iter()
        .any(|p| matches!(p.content_type.as_str(), "reasoning" | "thinking"));
    if !has_trailing_thinking {
        return (message.clone(), false);
    }
    let mut cleaned = message.clone();
    if let Content::Parts(ref mut cleaned_parts) = cleaned.content {
        cleaned_parts.truncate(last_idx + 1);
    }
    (cleaned, true)
}

/// Rewrite tool_use IDs that have already been seen in previous messages.
/// Each duplicate gets a `--dedup-N` suffix so the provider never encounters
/// the same tool_use id twice in one request.
fn rewrite_cross_message_duplicate_ids(
    message: &Message,
    seen_ids: &mut std::collections::HashSet<String>,
    rewrites: &mut std::collections::HashMap<String, String>,
    suffix: &mut u64,
    record: &mut dyn FnMut(SanitizerAction),
) -> Message {
    let Content::Parts(_parts) = &message.content else {
        return message.clone();
    };
    let mut rewritten = message.clone();
    let Content::Parts(ref mut rewritten_parts) = rewritten.content else {
        return rewritten;
    };
    for part in rewritten_parts.iter_mut() {
        if let Some(ref mut tool_use) = part.tool_use {
            if !seen_ids.insert(tool_use.id.clone()) {
                let original = tool_use.id.clone();
                let new_id = loop {
                    *suffix += 1;
                    let candidate = format!("{original}--dedup-{suffix}");
                    if seen_ids.insert(candidate.clone()) {
                        break candidate;
                    }
                };
                rewrites.insert(original.clone(), new_id.clone());
                tool_use.id = new_id;
                record(SanitizerAction::DuplicateToolId {
                    tool_call_id: original,
                });
            }
        }
    }
    rewritten
}

/// Apply dedup rewrites to tool_result's tool_use_id fields.
fn rewrite_tool_result_ids(
    message: &Message,
    rewrites: &std::collections::HashMap<String, String>,
) -> Message {
    let mut rewritten = message.clone();
    if let Content::Parts(ref mut rewritten_parts) = rewritten.content {
        for part in rewritten_parts.iter_mut() {
            if let Some(ref mut tool_result) = part.tool_result {
                if let Some(new_id) = rewrites.get(&tool_result.tool_use_id) {
                    tool_result.tool_use_id = new_id.clone();
                }
            }
        }
    }
    rewritten
}

/// True when the same tool_use ID appears more than once within a single message.
fn has_intra_message_duplicate_ids(ids: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    ids.iter().any(|id| !seen.insert(id.clone()))
}

/// Remove duplicate tool_use parts within a single message (same ID appearing
/// multiple times in one assistant turn).
fn deduplicate_tool_use_ids_in_message(message: &Message) -> Message {
    let Content::Parts(_parts) = &message.content else {
        return message.clone();
    };
    let mut seen = std::collections::HashSet::new();
    let mut deduped = message.clone();
    if let Content::Parts(ref mut deduped_parts) = deduped.content {
        deduped_parts.retain(|p| {
            p.tool_use
                .as_ref()
                .map_or(true, |tu| seen.insert(tu.id.clone()))
        });
    }
    deduped
}

fn is_thinking_only_assistant(message: &Message) -> bool {
    if !matches!(message.role, Role::Assistant) {
        return false;
    }

    match &message.content {
        Content::Text(_) => false,
        Content::Parts(parts) => {
            let mut has_reasoning = false;
            let mut has_visible_text = false;
            let mut has_tool_use = false;
            let mut has_other_payload = false;

            for part in parts {
                match part.content_type.as_str() {
                    "reasoning" | "thinking" => {
                        if part
                            .text
                            .as_ref()
                            .is_some_and(|text| !text.trim().is_empty())
                        {
                            has_reasoning = true;
                        }
                    }
                    "text" => {
                        if part
                            .text
                            .as_ref()
                            .is_some_and(|text| !text.trim().is_empty())
                        {
                            has_visible_text = true;
                        }
                    }
                    "tool_use" => {
                        if part.tool_use.is_some() {
                            has_tool_use = true;
                        }
                    }
                    _ => {
                        if part
                            .text
                            .as_ref()
                            .is_some_and(|text| !text.trim().is_empty())
                            || part.image_url.is_some()
                            || part.tool_result.is_some()
                        {
                            has_other_payload = true;
                        }
                    }
                }
            }

            has_reasoning && !has_visible_text && !has_tool_use && !has_other_payload
        }
    }
}

fn push_or_merge_text_message(messages: &mut Vec<Message>, role: Role, text: String) {
    if let Some(last) = messages.last_mut() {
        if same_role(&last.role, &role) {
            match &mut last.content {
                Content::Text(existing) => {
                    if !existing.is_empty() && !text.is_empty() {
                        existing.push_str("\n\n");
                    }
                    existing.push_str(&text);
                    return;
                }
                Content::Parts(_) => {}
            }
        }
    }

    messages.push(Message {
        role,
        content: Content::Text(text),
        cache_control: None,
        provider_options: None,
    });
}

fn same_role(left: &Role, right: &Role) -> bool {
    matches!(
        (left, right),
        (Role::System, Role::System)
            | (Role::User, Role::User)
            | (Role::Assistant, Role::Assistant)
            | (Role::Tool, Role::Tool)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn injects_interrupted_tool_results_before_new_assistant_segment() {
        let messages = vec![
            Message::assistant_parts(vec![ContentPart::tool_use(
                "tool-call-0",
                "first",
                json!({}),
            )]),
            Message::assistant_parts(vec![ContentPart::tool_use(
                "tool-call-0",
                "second",
                json!({}),
            )]),
        ];

        let sanitized = sanitize_messages_for_protocol(&messages, SanitizerOptions::default());
        assert_eq!(sanitized.len(), 4);
        assert!(matches!(sanitized[1].role, Role::Tool));
        assert!(matches!(
            &sanitized[1].content,
            Content::Parts(parts)
                if matches!(
                    &parts[0].tool_result,
                    Some(tool_result)
                        if tool_result.tool_use_id == "tool-call-0"
                            && tool_result.content == INTERRUPTED_TOOL_RESULT_TEXT
                )
        ));
    }

    #[test]
    fn records_actions_when_collector_provided() {
        let messages = vec![
            Message::user("first"),
            Message::assistant_parts(vec![ContentPart::reasoning("hidden")]),
            Message::user("second"),
        ];

        let mut actions = Vec::new();
        let _sanitized = sanitize_messages_for_protocol_with_actions(
            &messages,
            SanitizerOptions {
                drop_thinking_only_assistant: true,
                ..Default::default()
            },
            Some(&mut actions),
        );

        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], SanitizerAction::ThinkingOnlyAssistant));
    }

    #[test]
    fn records_interrupted_tool_calls() {
        let messages = vec![
            Message::assistant_parts(vec![ContentPart::tool_use("call-1", "ls", json!({}))]),
            // New assistant without tool_result for call-1 → interrupted
            Message::assistant_parts(vec![ContentPart::tool_use("call-2", "read", json!({}))]),
        ];

        let mut actions = Vec::new();
        let _sanitized = sanitize_messages_for_protocol_with_actions(
            &messages,
            SanitizerOptions::default(),
            Some(&mut actions),
        );

        assert!(actions
            .iter()
            .any(|a| matches!(a, SanitizerAction::OrphanedToolResult { tool_call_id } if tool_call_id == "call-1")));
    }

    #[test]
    fn drops_thinking_only_assistant_when_requested() {
        let messages = vec![
            Message::user("first"),
            Message::assistant_parts(vec![ContentPart::reasoning("hidden")]),
            Message::user("second"),
        ];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions {
                drop_thinking_only_assistant: true,
                ..Default::default()
            },
        );
        assert_eq!(sanitized.len(), 2);
        assert!(matches!(sanitized[0].role, Role::User));
        assert!(matches!(sanitized[1].role, Role::User));
    }

    #[test]
    fn flushes_pending_tool_results_before_tool_text_redirect() {
        let messages = vec![
            Message::assistant_parts(vec![
                ContentPart::tool_use("call-1", "one", json!({})),
                ContentPart::tool_use("call-2", "two", json!({})),
            ]),
            Message::tool_parts(vec![
                ContentPart::tool_result("call-1", "ok", None),
                ContentPart::text("redirect"),
            ]),
        ];

        let sanitized = sanitize_messages_for_protocol(&messages, SanitizerOptions::default());
        assert_eq!(sanitized.len(), 4);
        assert!(matches!(sanitized[1].role, Role::Tool));
        assert!(matches!(sanitized[2].role, Role::Tool));
        assert!(matches!(sanitized[3].role, Role::User));
        assert!(matches!(
            &sanitized[2].content,
            Content::Parts(parts)
                if matches!(
                    &parts[0].tool_result,
                    Some(tool_result)
                        if tool_result.tool_use_id == "call-2"
                            && tool_result.content == INTERRUPTED_TOOL_RESULT_TEXT
                )
        ));
    }

    #[test]
    fn text_protocol_projection_drops_tool_only_turns_and_merges_roles() {
        let messages = vec![
            Message::user("before"),
            Message::assistant_parts(vec![ContentPart::tool_use("call-1", "ls", json!({}))]),
            Message::tool_parts(vec![ContentPart::tool_result("call-1", "ok", None)]),
            Message::user("after"),
        ];

        let sanitized = sanitize_messages_for_text_protocol(&messages);
        assert_eq!(sanitized.len(), 1);
        assert!(matches!(sanitized[0].role, Role::User));
        assert!(matches!(
            &sanitized[0].content,
            Content::Text(text) if text == "before\n\nafter"
        ));
    }

    #[test]
    fn text_protocol_projection_strips_reasoning_text() {
        let messages = vec![Message::assistant_parts(vec![
            ContentPart::reasoning("hidden"),
            ContentPart::text("visible"),
        ])];

        let sanitized = sanitize_messages_for_text_protocol(&messages);
        assert_eq!(sanitized.len(), 1);
        assert!(matches!(
            &sanitized[0].content,
            Content::Text(text) if text == "visible"
        ));
    }

    #[test]
    fn strict_sanitizer_does_not_inject_interrupted_tool_placeholder() {
        // Two consecutive assistant messages with tool_use but no tool_result:
        // the first assistant's tool calls should be synthetic-interrupted.
        // In strict mode (skip_synthetic_repair=true), those placeholders
        // are NOT injected — the messages are just dropped.
        let messages = vec![
            Message::assistant_parts(vec![ContentPart::tool_use("call-1", "ls", json!({}))]),
            Message::assistant_parts(vec![ContentPart::tool_use("call-2", "read", json!({}))]),
        ];

        let mut actions = Vec::new();
        let sanitized = sanitize_messages_for_protocol_with_actions(
            &messages,
            SanitizerOptions {
                drop_thinking_only_assistant: false,
                skip_synthetic_repair: true,
            },
            Some(&mut actions),
        );

        // Actions are still recorded.
        assert!(actions
            .iter()
            .any(|a| matches!(a, SanitizerAction::OrphanedToolResult { tool_call_id } if tool_call_id == "call-1")));

        // But no synthetic tool messages are injected — only the valid assistant messages remain.
        let tool_msg_count = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Tool))
            .count();
        assert_eq!(
            tool_msg_count, 0,
            "strict mode should not inject synthetic tool placeholders"
        );
    }

    // ── Narrow P0.2 acceptance tests ───────────────────────────────────

    #[test]
    fn pre_request_sanitizer_repairs_orphaned_tool_result() {
        let messages = vec![
            Message::assistant_parts(vec![ContentPart::tool_use("call-1", "read", json!({}))]),
            // No tool message → call-1 is orphaned.
            Message::assistant_parts(vec![ContentPart::text("next turn")]),
        ];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions::default(),
        );

        // An interrupted tool result placeholder is injected between the two assistants.
        assert!(sanitized.iter().any(|m| {
            matches!(&m.content, Content::Parts(parts)
                if parts.iter().any(|p| p.tool_result.as_ref().is_some_and(|tr| tr.tool_use_id == "call-1")))
        }));
    }

    // P2.3: trailing invalid thinking blocks from fallback/retry must not
    // carry over into the next provider request.
    #[test]
    fn fallback_retry_sanitizer_strips_invalid_continuation_residue() {
        // Simulate a message with valid tool_use followed by trailing thinking blocks
        // — the kind of residue left after a provider fallback.
        let messages = vec![Message::assistant_parts(vec![
            ContentPart::text("I will read the file"),
            ContentPart::tool_use("call-1", "read", json!({})),
            ContentPart::reasoning("thinking about the next step"),
        ])];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions {
                drop_thinking_only_assistant: true,
                ..Default::default()
            },
        );

        let first = &sanitized[0];
        match &first.content {
            Content::Parts(parts) => {
                // The trailing thinking block should be stripped.
                let has_trailing_thinking = parts
                    .iter()
                    .any(|p| matches!(p.content_type.as_str(), "reasoning" | "thinking"));
                assert!(!has_trailing_thinking, "trailing thinking should be stripped");
                // But the tool_use and text should remain.
                assert!(parts.iter().any(|p| p.tool_use.is_some()));
                assert!(parts
                    .iter()
                    .any(|p| p.content_type == "text" && p.text.as_ref().is_some_and(|t| !t.trim().is_empty())));
            }
            _ => panic!("expected parts content"),
        }
    }

    #[test]
    fn strict_mode_does_not_inject_synthetic_placeholder_on_resume() {
        // Simulate resume: an assistant with a tool_use that has no result.
        let messages = vec![Message::assistant_parts(vec![ContentPart::tool_use(
            "call-resume",
            "read",
            json!({}),
        )])];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions {
                drop_thinking_only_assistant: false,
                skip_synthetic_repair: true,
            },
        );

        // In strict mode, no synthetic tool placeholder is injected.
        let tool_msg_count = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Tool))
            .count();
        assert_eq!(tool_msg_count, 0);
        // Only the assistant message remains.
        assert_eq!(sanitized.len(), 1);
    }

    // P2.3: compaction/trimmed placeholder text must not re-enter model context.
    #[test]
    fn sanitized_messages_do_not_reintroduce_compaction_placeholder_text() {
        // Simulate a post-compaction message: an assistant that is just "[compacted]".
        let messages = vec![
            Message::user("continue"),
            Message::assistant_parts(vec![ContentPart::text("[compacted]")]),
        ];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions::default(),
        );

        // The placeholder-only assistant should be stripped.
        let assistant_count = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Assistant))
            .count();
        assert_eq!(assistant_count, 0, "placeholder-only assistant should be dropped");
    }

    #[test]
    fn filter_whitespace_only_assistant_messages() {
        let messages = vec![
            Message::user("hello"),
            Message::assistant_parts(vec![]),
            Message::user("world"),
        ];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions::default(),
        );

        // Empty assistant should be dropped; only two user messages remain.
        assert_eq!(sanitized.len(), 2);
        assert!(matches!(sanitized[0].role, Role::User));
        assert!(matches!(sanitized[1].role, Role::User));
    }

    // P2.3: compaction-like trimmed markers must be dropped before model context.
    #[test]
    fn placeholder_only_assistant_is_dropped_after_compaction_like_trim() {
        let messages = vec![
            Message::user("continue"),
            Message::assistant_parts(vec![ContentPart::text("[compacted]")]),
            Message::assistant_parts(vec![ContentPart::text("[trimmed]")]),
        ];

        let mut actions = Vec::new();
        let sanitized = sanitize_messages_for_protocol_with_actions(
            &messages,
            SanitizerOptions::default(),
            Some(&mut actions),
        );

        // Both placeholder-only assistants must be dropped.
        let assistant_texts: Vec<String> = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Assistant))
            .flat_map(|m| match &m.content {
                Content::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| p.text.clone())
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        assert!(
            !assistant_texts.iter().any(|t| t.contains("[compacted]")),
            "no [compacted] text should reach the model"
        );
        assert!(
            !assistant_texts.iter().any(|t| t.contains("[trimmed]")),
            "no [trimmed] text should reach the model"
        );

        // Actions recorded.
        assert!(actions
            .iter()
            .any(|a| matches!(a, SanitizerAction::AssistantMalformedPlaceholder)));
    }

    #[test]
    fn detects_duplicate_tool_use_ids_in_same_batch() {
        let messages = vec![Message::assistant_parts(vec![
            ContentPart::tool_use("dup-id", "read", json!({})),
            ContentPart::tool_use("dup-id", "write", json!({})),
        ])];

        let mut actions = Vec::new();
        let _sanitized = sanitize_messages_for_protocol_with_actions(
            &messages,
            SanitizerOptions::default(),
            Some(&mut actions),
        );

        assert!(actions
            .iter()
            .any(|a| matches!(a, SanitizerAction::DuplicateToolId { tool_call_id } if tool_call_id == "dup-id")));
    }

    // P2.3: protects against duplicate tool_use_id in provider request payload.
    #[test]
    fn cross_message_duplicate_tool_use_ids_are_rewritten_with_unique_suffix() {
        // Two separate assistant messages reusing the same tool_use id:
        // the second occurrence should be rewritten to a unique suffix.
        let messages = vec![
            Message::assistant_parts(vec![ContentPart::tool_use("dup-x", "read", json!({}))]),
            Message::user("go on"),
            Message::assistant_parts(vec![ContentPart::tool_use("dup-x", "write", json!({}))]),
            Message::tool_parts(vec![ContentPart::tool_result("dup-x", "done", None)]),
        ];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions::default(),
        );

        // The second assistant's tool_use should have a rewritten ID.
        let second_assistant_ids: Vec<&str> = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Assistant))
            .flat_map(|m| match &m.content {
                Content::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| p.tool_use.as_ref().map(|tu| tu.id.as_str()))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        // First occurrence keeps original, second is rewritten.
        assert_eq!(second_assistant_ids, vec!["dup-x", "dup-x--dedup-1"]);

        // The tool_result that referenced the old ID should be rewritten too.
        let tool_result_ids: Vec<&str> = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Tool))
            .flat_map(|m| match &m.content {
                Content::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| p.tool_result.as_ref().map(|tr| tr.tool_use_id.as_str()))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        // The tool_result referencing "dup-x" should now reference "dup-x--dedup-1"
        // since the second tool_use was rewritten.
        assert!(tool_result_ids.contains(&"dup-x--dedup-1"));
    }

    #[test]
    fn repeated_cross_message_duplicate_ids_remain_globally_unique() {
        let messages = vec![
            Message::assistant_parts(vec![ContentPart::tool_use("dup-x", "read", json!({}))]),
            Message::assistant_parts(vec![ContentPart::tool_use("dup-x", "write", json!({}))]),
            Message::assistant_parts(vec![ContentPart::tool_use("dup-x", "edit", json!({}))]),
            Message::tool_parts(vec![ContentPart::tool_result("dup-x", "done", None)]),
        ];

        let sanitized = sanitize_messages_for_protocol(
            &messages,
            SanitizerOptions::default(),
        );

        let assistant_ids: Vec<&str> = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Assistant))
            .flat_map(|m| match &m.content {
                Content::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| p.tool_use.as_ref().map(|tu| tu.id.as_str()))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(assistant_ids, vec!["dup-x", "dup-x--dedup-1", "dup-x--dedup-2"]);

        let tool_result_ids: Vec<&str> = sanitized
            .iter()
            .filter(|m| matches!(m.role, Role::Tool))
            .flat_map(|m| match &m.content {
                Content::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| p.tool_result.as_ref().map(|tr| tr.tool_use_id.as_str()))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        assert!(tool_result_ids.contains(&"dup-x"));
        assert!(tool_result_ids.contains(&"dup-x--dedup-1"));
        assert!(tool_result_ids.contains(&"dup-x--dedup-2"));
    }
}
