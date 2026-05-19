use super::*;
use rocode_command::terminal_tool_block_display::{
    build_file_items, build_image_items, summarize_block_items_inline,
};
use rocode_types::tool_call_observable_arguments;

const LEGACY_SYSTEM_REMINDER_PREFIX: &str = "System Reminder Sent:";
const LOADED_INSTRUCTION_FILES_PREFIX: &str = "Loaded instruction files:";

pub(super) fn apply_incremental_session_sync(
    session_ctx: &mut crate::context::SessionContext,
    session_id: &str,
    session: &SessionInfo,
    mapped_messages: Vec<Message>,
) {
    session_ctx.upsert_session(map_api_session(session));
    session_ctx.upsert_messages_incremental(session_id, mapped_messages);

    if let Some(revert_info) = session.revert.as_ref().map(map_api_revert) {
        session_ctx
            .revert
            .insert(session_id.to_string(), revert_info);
    } else {
        session_ctx.revert.remove(session_id);
    }
}

pub(super) fn map_api_session(session: &SessionInfo) -> Session {
    Session {
        id: session.id.clone(),
        title: session.title.clone(),
        created_at: Utc
            .timestamp_millis_opt(session.time.created)
            .single()
            .unwrap_or_else(Utc::now),
        updated_at: Utc
            .timestamp_millis_opt(session.time.updated)
            .single()
            .unwrap_or_else(Utc::now),
        parent_id: session.parent_id.clone(),
        share: None,
        metadata: session.metadata.clone(),
    }
}

pub(super) fn map_api_message(message: &MessageInfo) -> Message {
    let keep_synthetic_text = message.mode.as_deref() == Some("compaction");
    let parts = merge_adjacent_textual_parts(
        message
            .parts
            .iter()
            .filter_map(|part| map_api_message_part(part, keep_synthetic_text))
            .collect(),
    );

    Message {
        id: message.id.clone(),
        role: match message.role.as_str() {
            "assistant" => MessageRole::Assistant,
            "system" => MessageRole::System,
            "tool" => MessageRole::Tool,
            _ => MessageRole::User,
        },
        content: parts
            .iter()
            .map(message_part_text)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        created_at: Utc
            .timestamp_millis_opt(message.created_at)
            .single()
            .unwrap_or_else(Utc::now),
        completed_at: message
            .completed_at
            .and_then(|ts| Utc.timestamp_millis_opt(ts).single()),
        agent: message.agent.clone(),
        model: message.model.clone(),
        mode: message.mode.clone(),
        finish: message.finish.clone(),
        error: message.error.clone(),
        cost: message.cost,
        tokens: TokenUsage {
            input: message.tokens.input,
            output: message.tokens.output,
            reasoning: message.tokens.reasoning,
            cache_read: message.tokens.cache_read,
            cache_miss: message.tokens.cache_miss,
            cache_write: message.tokens.cache_write,
        },
        metadata: message.metadata.clone(),
        multimodal: message.multimodal.clone(),
        parts,
    }
}

fn merge_adjacent_textual_parts(parts: Vec<ContextMessagePart>) -> Vec<ContextMessagePart> {
    let mut merged: Vec<ContextMessagePart> = Vec::with_capacity(parts.len());

    for part in parts {
        match part {
            ContextMessagePart::Text { text } => {
                if let Some(ContextMessagePart::Text { text: existing }) = merged.last_mut() {
                    append_text_part(existing, &text);
                } else {
                    merged.push(ContextMessagePart::Text { text });
                }
            }
            ContextMessagePart::Reasoning { text } => {
                if let Some(ContextMessagePart::Reasoning { text: existing }) = merged.last_mut() {
                    existing.push_str(&text);
                } else {
                    merged.push(ContextMessagePart::Reasoning { text });
                }
            }
            other => merged.push(other),
        }
    }

    merged
}

fn append_text_part(existing: &mut String, incoming: &str) {
    if incoming.is_empty() {
        return;
    }

    if needs_system_reminder_separator(existing, incoming) && !existing.ends_with("\n\n") {
        if existing.ends_with('\n') {
            existing.push('\n');
        } else {
            existing.push_str("\n\n");
        }
    }

    existing.push_str(incoming);
}

fn needs_system_reminder_separator(existing: &str, incoming: &str) -> bool {
    !existing.trim().is_empty() && {
        let incoming = incoming.trim_start();
        incoming.starts_with(LEGACY_SYSTEM_REMINDER_PREFIX)
            || incoming.starts_with(LOADED_INSTRUCTION_FILES_PREFIX)
    }
}

pub(super) fn map_api_revert(revert: &SessionRevertInfo) -> RevertInfo {
    RevertInfo {
        message_id: revert.message_id.clone(),
        part_id: revert.part_id.clone(),
        snapshot: revert.snapshot.clone(),
        diff: revert.diff.clone(),
    }
}

fn map_api_message_part(
    part: &crate::api::MessagePart,
    keep_synthetic_text: bool,
) -> Option<ContextMessagePart> {
    if let Some(text) = &part.text {
        if part.ignored == Some(true) {
            return None;
        }
        if part.part_type == "reasoning" {
            return Some(ContextMessagePart::Reasoning { text: text.clone() });
        }
        // Skip synthetic text parts (auto-continue prompts, etc.)
        if part.synthetic == Some(true) && !keep_synthetic_text {
            return None;
        }
        return Some(ContextMessagePart::Text { text: text.clone() });
    }

    if let Some(file) = &part.file {
        return Some(ContextMessagePart::File {
            path: file.filename.clone(),
            mime: file.mime.clone(),
        });
    }

    if let Some(tool_call) = &part.tool_call {
        return Some(ContextMessagePart::ToolCall {
            id: tool_call.id.clone(),
            name: tool_call.name.clone(),
            arguments: tool_call_observable_arguments(&tool_call.input).unwrap_or_default(),
        });
    }

    if let Some(tool_result) = &part.tool_result {
        return Some(ContextMessagePart::ToolResult {
            id: tool_result.tool_call_id.clone(),
            result: tool_result.content.clone(),
            is_error: tool_result.is_error,
            title: tool_result.title.clone(),
            metadata: tool_result.metadata.clone(),
        });
    }

    None
}

fn message_part_text(part: &ContextMessagePart) -> String {
    match part {
        ContextMessagePart::Text { text } => text.clone(),
        ContextMessagePart::Reasoning { text } => format!("[reasoning] {}", text),
        ContextMessagePart::ToolCall {
            name, arguments, ..
        } => format!("[tool:{}] {}", name, arguments),
        ContextMessagePart::ToolResult {
            result, is_error, ..
        } => {
            if *is_error {
                return format!("[tool-error] {}", result);
            }
            format!("[tool-result] {}", result)
        }
        ContextMessagePart::File { path, mime } => {
            summarize_block_items_inline(&build_file_items(path, mime))
        }
        ContextMessagePart::Image { url } => summarize_block_items_inline(&build_image_items(url)),
    }
}

pub(super) fn infer_task_kind_from_message(message: &Message) -> TaskKind {
    let Some(last_part) = message.parts.last() else {
        return TaskKind::LlmResponse;
    };

    match last_part {
        ContextMessagePart::Text { .. } | ContextMessagePart::Reasoning { .. } => {
            TaskKind::LlmResponse
        }
        ContextMessagePart::ToolCall { name, .. } => task_kind_from_tool_name(name),
        ContextMessagePart::ToolResult { id, .. } => message
            .parts
            .iter()
            .rev()
            .find_map(|part| match part {
                ContextMessagePart::ToolCall {
                    id: call_id, name, ..
                } if call_id == id => Some(task_kind_from_tool_name(name)),
                _ => None,
            })
            .unwrap_or(TaskKind::ToolCall),
        ContextMessagePart::File { .. } => TaskKind::FileRead,
        ContextMessagePart::Image { .. } => TaskKind::LlmResponse,
    }
}

fn task_kind_from_tool_name(name: &str) -> TaskKind {
    let normalized = name.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.is_empty() {
        return TaskKind::ToolCall;
    }

    if normalized.contains("read")
        || normalized.contains("grep")
        || normalized.contains("glob")
        || normalized.contains("list")
        || normalized == "ls"
    {
        return TaskKind::FileRead;
    }
    if normalized.contains("write")
        || normalized.contains("edit")
        || normalized.contains("patch")
        || normalized.contains("todo")
    {
        return TaskKind::FileWrite;
    }
    if normalized.contains("bash")
        || normalized.contains("shell")
        || normalized.contains("exec")
        || normalized.contains("command")
    {
        return TaskKind::CommandExec;
    }

    TaskKind::ToolCall
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{MessageTokensInfo, SessionTimeInfo};

    #[test]
    fn merge_adjacent_textual_parts_coalesces_reasoning_and_text() {
        let parts = vec![
            ContextMessagePart::Reasoning {
                text: "think".to_string(),
            },
            ContextMessagePart::Reasoning {
                text: " more".to_string(),
            },
            ContextMessagePart::Text {
                text: "answer".to_string(),
            },
            ContextMessagePart::Text {
                text: " done".to_string(),
            },
        ];

        let merged = merge_adjacent_textual_parts(parts);

        assert_eq!(merged.len(), 2);
        assert!(matches!(
            &merged[0],
            ContextMessagePart::Reasoning { text } if text == "think more"
        ));
        assert!(matches!(
            &merged[1],
            ContextMessagePart::Text { text } if text == "answer done"
        ));
    }

    #[test]
    fn merge_adjacent_textual_parts_puts_system_reminder_on_its_own_line() {
        let parts = vec![
            ContextMessagePart::Text {
                text: "User-facing summary.".to_string(),
            },
            ContextMessagePart::Text {
                text: "Loaded instruction files: /tmp/project/AGENTS.md".to_string(),
            },
        ];

        let merged = merge_adjacent_textual_parts(parts);

        assert_eq!(merged.len(), 1);
        assert!(matches!(
            &merged[0],
            ContextMessagePart::Text { text }
                if text == "User-facing summary.\n\nLoaded instruction files: /tmp/project/AGENTS.md"
        ));
    }

    #[test]
    fn map_api_message_part_uses_observable_arguments_not_raw_shape() {
        let part = crate::api::MessagePart {
            id: "prt_tool".to_string(),
            part_type: "tool_call".to_string(),
            text: None,
            file: None,
            tool_call: Some(crate::api::ToolCall {
                id: "call_1".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path":"/tmp/normalized.txt"}),
                status: Some("running".to_string()),
                raw: Some("{\"file_path\":\"/tmp/raw.txt\"}".to_string()),
                state: None,
            }),
            tool_result: None,
            synthetic: None,
            ignored: None,
        };

        let mapped = map_api_message_part(&part, false).expect("tool call should map");
        assert!(matches!(
            mapped,
            ContextMessagePart::ToolCall { arguments, .. }
                if arguments == "{\"file_path\":\"/tmp/normalized.txt\"}"
        ));
    }

    #[test]
    fn incremental_sync_preserves_local_streaming_assistant_message_with_same_id() {
        let session_id = "session-1";
        let now = Utc::now().timestamp_millis();
        let mut session_ctx = crate::context::SessionContext::new();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });

        session_ctx.apply_output_block_incremental(
            session_id,
            Some("assistant-1"),
            &serde_json::json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "hello world"
            }),
        );

        let before_sync = session_ctx
            .messages
            .get(session_id)
            .and_then(|messages| messages.iter().find(|message| message.id == "assistant-1"))
            .map(|message| message.content.clone())
            .expect("streaming assistant message should exist");
        assert_eq!(before_sync, "hello world");

        let session = SessionInfo {
            id: session_id.to_string(),
            slug: "session-1".to_string(),
            project_id: "project".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            title: "Session".to_string(),
            version: "1".to_string(),
            time: SessionTimeInfo {
                created: now,
                updated: now + 1000,
                compacting: None,
                archived: None,
            },
            summary: None,
            share: None,
            permission: None,
            revert: None,
            fork: None,
            telemetry: None,
            metadata: None,
        };
        let mapped_messages = vec![map_api_message(&MessageInfo {
            id: "assistant-1".to_string(),
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            created_at: now + 500,
            completed_at: None,
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            cost: 0.0,
            tokens: MessageTokensInfo::default(),
            parts: vec![crate::api::MessagePart {
                id: "p1".to_string(),
                part_type: "text".to_string(),
                text: Some("hello".to_string()),
                file: None,
                tool_call: None,
                tool_result: None,
                synthetic: None,
                ignored: None,
            }],
            metadata: None,
            multimodal: None,
        })];

        apply_incremental_session_sync(&mut session_ctx, session_id, &session, mapped_messages);

        let after_sync = session_ctx
            .messages
            .get(session_id)
            .and_then(|messages| messages.iter().find(|message| message.id == "assistant-1"))
            .map(|message| message.content.clone())
            .expect("assistant message should still exist after sync");
        assert_eq!(after_sync, "hello world");
    }

    #[test]
    fn incremental_sync_replaces_completed_assistant_message_with_server_version() {
        let session_id = "session-1";
        let now = Utc::now().timestamp_millis();
        let mut session_ctx = crate::context::SessionContext::new();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });

        session_ctx.add_message(
            session_id,
            Message {
                id: "assistant-1".to_string(),
                role: MessageRole::Assistant,
                content: "hello world".to_string(),
                created_at: Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(Utc::now()),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "hello world".to_string(),
                }],
            },
        );

        let session = SessionInfo {
            id: session_id.to_string(),
            slug: "session-1".to_string(),
            project_id: "project".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            title: "Session".to_string(),
            version: "1".to_string(),
            time: SessionTimeInfo {
                created: now,
                updated: now + 1000,
                compacting: None,
                archived: None,
            },
            summary: None,
            share: None,
            permission: None,
            revert: None,
            fork: None,
            telemetry: None,
            metadata: None,
        };
        let mapped_messages = vec![map_api_message(&MessageInfo {
            id: "assistant-1".to_string(),
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            created_at: now + 500,
            completed_at: Some(now + 700),
            agent: None,
            model: None,
            mode: None,
            finish: Some("stop".to_string()),
            error: None,
            cost: 0.0,
            tokens: MessageTokensInfo::default(),
            parts: vec![crate::api::MessagePart {
                id: "p1".to_string(),
                part_type: "text".to_string(),
                text: Some("hello".to_string()),
                file: None,
                tool_call: None,
                tool_result: None,
                synthetic: None,
                ignored: None,
            }],
            metadata: None,
            multimodal: None,
        })];

        apply_incremental_session_sync(&mut session_ctx, session_id, &session, mapped_messages);

        let after_sync = session_ctx
            .messages
            .get(session_id)
            .and_then(|messages| messages.iter().find(|message| message.id == "assistant-1"))
            .map(|message| message.content.clone())
            .expect("assistant message should still exist after sync");
        assert_eq!(after_sync, "hello");
    }

    #[test]
    fn incremental_sync_preserves_local_streaming_reasoning_when_server_lags() {
        let session_id = "session-1";
        let now = Utc::now().timestamp_millis();
        let mut session_ctx = crate::context::SessionContext::new();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });

        session_ctx.apply_output_block_incremental(
            session_id,
            Some("assistant-1"),
            &serde_json::json!({
                "kind": "reasoning",
                "phase": "start",
                "text": ""
            }),
        );
        session_ctx.apply_output_block_incremental(
            session_id,
            Some("assistant-1"),
            &serde_json::json!({
                "kind": "reasoning",
                "phase": "delta",
                "text": "thinking more"
            }),
        );

        let session = SessionInfo {
            id: session_id.to_string(),
            slug: "session-1".to_string(),
            project_id: "project".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            title: "Session".to_string(),
            version: "1".to_string(),
            time: SessionTimeInfo {
                created: now,
                updated: now + 1000,
                compacting: None,
                archived: None,
            },
            summary: None,
            share: None,
            permission: None,
            revert: None,
            fork: None,
            telemetry: None,
            metadata: None,
        };
        let mapped_messages = vec![map_api_message(&MessageInfo {
            id: "assistant-1".to_string(),
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            created_at: now + 500,
            completed_at: None,
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            cost: 0.0,
            tokens: MessageTokensInfo::default(),
            parts: vec![crate::api::MessagePart {
                id: "p1".to_string(),
                part_type: "reasoning".to_string(),
                text: Some("thinking".to_string()),
                file: None,
                tool_call: None,
                tool_result: None,
                synthetic: None,
                ignored: None,
            }],
            metadata: None,
            multimodal: None,
        })];

        apply_incremental_session_sync(&mut session_ctx, session_id, &session, mapped_messages);

        let after_sync = session_ctx
            .messages
            .get(session_id)
            .and_then(|messages| messages.iter().find(|message| message.id == "assistant-1"))
            .and_then(|message| {
                message.parts.iter().find_map(|part| match part {
                    ContextMessagePart::Reasoning { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .expect("reasoning part should still exist after sync");
        assert_eq!(after_sync, "thinking more");
    }
}

pub(super) fn map_mcp_status(server: &McpStatusInfo) -> McpConnectionStatus {
    match server.status.as_str() {
        "connected" => McpConnectionStatus::Connected,
        "failed" => McpConnectionStatus::Failed,
        "needs_auth" => McpConnectionStatus::NeedsAuth,
        "needs_client_registration" => McpConnectionStatus::NeedsClientRegistration,
        "disabled" => McpConnectionStatus::Disabled,
        _ => McpConnectionStatus::Disconnected,
    }
}

pub(super) fn map_api_run_status(status: &crate::api::SessionStatusInfo) -> SessionStatus {
    if status.busy {
        if status.status.eq_ignore_ascii_case("compacting") {
            return SessionStatus::Compacting;
        }
        if status.status.eq_ignore_ascii_case("retry") {
            return SessionStatus::Retrying {
                message: status.message.clone().unwrap_or_default(),
                attempt: status.attempt.unwrap_or(0),
                next: status.next.unwrap_or_default(),
            };
        }
        return SessionStatus::Running;
    }
    SessionStatus::Idle
}

pub(super) fn agent_color_from_name(
    theme: &crate::theme::Theme,
    agent_name: &str,
) -> ratatui::style::Color {
    if theme.agent_colors.is_empty() {
        return theme.primary;
    }
    let mut hasher = DefaultHasher::new();
    agent_name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % theme.agent_colors.len();
    theme.agent_colors[idx]
}

pub(super) fn provider_from_model(model: &str) -> Option<String> {
    let model = model.trim();
    let (provider, _) = model
        .split_once('/')
        .or_else(|| model.split_once(':'))
        .unwrap_or((model, ""));
    if provider.is_empty() || provider == model {
        return None;
    }
    Some(provider.to_string())
}

pub(super) fn map_api_todo(item: &crate::api::ApiTodoItem) -> crate::context::TodoItem {
    use crate::context::{TodoItem, TodoStatus};
    let status = match item.status.as_str() {
        "in_progress" => TodoStatus::InProgress,
        "completed" | "done" => TodoStatus::Completed,
        "cancelled" | "canceled" => TodoStatus::Cancelled,
        _ => TodoStatus::Pending,
    };
    TodoItem {
        content: item.content.clone(),
        status,
    }
}

pub(super) fn map_api_diff(item: &crate::api::ApiDiffEntry) -> crate::context::DiffEntry {
    use crate::context::DiffEntry;
    DiffEntry {
        file: item.path.clone(),
        additions: item.additions as u32,
        deletions: item.deletions as u32,
    }
}
