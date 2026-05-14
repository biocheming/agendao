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
                if options.drop_thinking_only_assistant && is_thinking_only_assistant(message) {
                    record(SanitizerAction::ThinkingOnlyAssistant);
                    continue;
                }
                pending_tool_use_ids = assistant_tool_use_ids(message);
                sanitized.push(message.clone());
            }
            Role::Tool => {
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
                            if !matched_ids.contains(tool_result.tool_use_id.as_str()) {
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
}
