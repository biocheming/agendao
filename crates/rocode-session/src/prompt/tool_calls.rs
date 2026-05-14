use std::collections::HashMap;

use crate::{PartType, Session, SessionMessage};

use super::SessionPrompt;

impl SessionPrompt {
    pub(super) fn upsert_tool_call_part(
        message: &mut SessionMessage,
        tool_call_id: &str,
        tool_name: Option<&str>,
        input: Option<serde_json::Value>,
        raw_input: Option<String>,
        status: Option<crate::ToolCallStatus>,
        tool_state: Option<crate::ToolState>,
    ) {
        let mut input = input;
        let mut raw_input = raw_input;
        let mut status = status;
        if let Some(state) = tool_state.as_ref() {
            let (state_input, state_raw, state_status) = Self::state_projection(state);
            input = Some(state_input);
            // Only override raw_input if state_projection returns Some.
            // Running/Completed/Error states return None for raw, but the
            // caller may have explicitly provided a raw value that should
            // be preserved.
            if state_raw.is_some() {
                raw_input = state_raw;
            }
            status = Some(state_status);
        }

        let mut found = false;
        for part in &mut message.parts {
            if let PartType::ToolCall {
                id,
                name,
                input: part_input,
                status: part_status,
                raw,
                state,
            } = &mut part.part_type
            {
                if id == tool_call_id {
                    if let Some(next_name) = tool_name {
                        if !next_name.is_empty() {
                            *name = next_name.to_string();
                        }
                    }
                    if let Some(next_input) = input.as_ref() {
                        *part_input = next_input.clone();
                    }
                    if let Some(next_raw) = raw_input.as_ref() {
                        *raw = Some(next_raw.clone());
                    }
                    if let Some(next_status) = status.as_ref() {
                        *part_status = next_status.clone();
                    }
                    if let Some(next_state) = tool_state.as_ref() {
                        *state = Some(next_state.clone());
                    }
                    found = true;
                    break;
                }
            }
        }

        if found {
            return;
        }

        message.parts.push(crate::MessagePart {
            id: rocode_core::id::create(rocode_core::id::Prefix::Part, true, None),
            part_type: PartType::ToolCall {
                id: tool_call_id.to_string(),
                name: tool_name.unwrap_or_default().to_string(),
                input: input.unwrap_or_else(|| serde_json::json!({})),
                status: status.unwrap_or(crate::ToolCallStatus::Pending),
                raw: raw_input,
                state: tool_state,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
    }

    pub(super) fn state_projection(
        state: &crate::ToolState,
    ) -> (serde_json::Value, Option<String>, crate::ToolCallStatus) {
        match state {
            crate::ToolState::Pending { input, raw } => (
                input.clone(),
                Some(raw.clone()),
                crate::ToolCallStatus::Pending,
            ),
            crate::ToolState::Running { input, .. } => {
                (input.clone(), None, crate::ToolCallStatus::Running)
            }
            crate::ToolState::Completed { input, .. } => {
                (input.clone(), None, crate::ToolCallStatus::Completed)
            }
            crate::ToolState::Error { input, .. } => {
                (input.clone(), None, crate::ToolCallStatus::Error)
            }
        }
    }

    pub(super) fn push_tool_result_part(
        message: &mut SessionMessage,
        tool_call_id: String,
        content: String,
        is_error: bool,
        title: Option<String>,
        metadata: Option<HashMap<String, serde_json::Value>>,
        attachments: Option<Vec<serde_json::Value>>,
    ) {
        message.parts.push(crate::MessagePart {
            id: rocode_core::id::create(rocode_core::id::Prefix::Part, true, None),
            part_type: PartType::ToolResult {
                tool_call_id,
                content,
                is_error,
                title,
                metadata,
                attachments,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
    }

    pub(super) fn take_attachment_values(
        metadata: &mut HashMap<String, serde_json::Value>,
    ) -> Option<Vec<serde_json::Value>> {
        let mut attachments = Vec::new();

        if let Some(value) = metadata.remove("attachments") {
            match value {
                serde_json::Value::Array(values) => attachments.extend(values),
                serde_json::Value::Null => {}
                other => attachments.push(other),
            }
        }

        if let Some(value) = metadata.remove("attachment") {
            if !value.is_null() {
                attachments.push(value);
            }
        }

        if attachments.is_empty() {
            None
        } else {
            Some(attachments)
        }
    }

    pub(super) fn normalize_tool_attachments(
        raw_attachments: Option<Vec<serde_json::Value>>,
        session_id: &str,
        message_id: &str,
    ) -> (
        Option<Vec<serde_json::Value>>,
        Option<Vec<crate::message_v2::FilePart>>,
    ) {
        let mut normalized_json = Vec::new();
        let mut normalized_files = Vec::new();

        for value in raw_attachments.unwrap_or_default() {
            let Some(obj) = value.as_object() else {
                continue;
            };

            let Some(mime) = obj.get("mime").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(url) = obj.get("url").and_then(|v| v.as_str()) else {
                continue;
            };

            let filename = obj
                .get("filename")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let id = obj
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    rocode_core::id::create(rocode_core::id::Prefix::Part, true, None)
                });
            let normalized_session_id = obj
                .get("sessionID")
                .or_else(|| obj.get("session_id"))
                .and_then(|v| v.as_str())
                .unwrap_or(session_id)
                .to_string();
            let normalized_message_id = obj
                .get("messageID")
                .or_else(|| obj.get("message_id"))
                .and_then(|v| v.as_str())
                .unwrap_or(message_id)
                .to_string();

            let mut normalized = serde_json::Map::new();
            normalized.insert("type".to_string(), serde_json::json!("file"));
            normalized.insert("id".to_string(), serde_json::json!(id.clone()));
            normalized.insert(
                "sessionID".to_string(),
                serde_json::json!(normalized_session_id.clone()),
            );
            normalized.insert(
                "messageID".to_string(),
                serde_json::json!(normalized_message_id.clone()),
            );
            normalized.insert("mime".to_string(), serde_json::json!(mime));
            normalized.insert("url".to_string(), serde_json::json!(url));
            if let Some(name) = filename.clone() {
                normalized.insert("filename".to_string(), serde_json::json!(name));
            }

            normalized_json.push(serde_json::Value::Object(normalized));
            normalized_files.push(crate::message_v2::FilePart {
                id,
                session_id: normalized_session_id,
                message_id: normalized_message_id,
                mime: mime.to_string(),
                url: url.to_string(),
                filename,
                source: None,
            });
        }

        (
            (!normalized_json.is_empty()).then_some(normalized_json),
            (!normalized_files.is_empty()).then_some(normalized_files),
        )
    }

    pub(super) fn extract_tool_attachments_from_metadata(
        metadata: &mut HashMap<String, serde_json::Value>,
        session_id: &str,
        message_id: &str,
    ) -> (
        Option<Vec<serde_json::Value>>,
        Option<Vec<crate::message_v2::FilePart>>,
    ) {
        let raw_attachments = Self::take_attachment_values(metadata);
        Self::normalize_tool_attachments(raw_attachments, session_id, message_id)
    }

    pub(super) fn has_unresolved_tool_calls(message: &SessionMessage) -> bool {
        message.parts.iter().any(|part| {
            let PartType::ToolCall {
                name,
                input,
                status,
                raw,
                state,
                ..
            } = &part.part_type
            else {
                return false;
            };

            if name.trim().is_empty() {
                return false;
            }

            Self::tool_call_input_for_execution(status, input, raw.as_deref(), state.as_ref())
                .is_some()
        })
    }

    pub(super) fn parse_json_or_string(raw: &str) -> serde_json::Value {
        // First try standard JSON parse.
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
            // If the parsed value is a JSON string, it may itself contain a
            // JSON object (double-encoded). Try robust object recovery first.
            if matches!(val, serde_json::Value::String(_)) {
                if let Some(obj) = rocode_util::json::try_parse_json_object_robust(raw) {
                    return obj;
                }
            }
            return val;
        }

        // Recover object-shaped arguments with robust parsing.
        if let Some(obj) = rocode_util::json::try_parse_json_object_robust(raw) {
            return obj;
        }

        // If the raw string looks like it could be a JSON object with issues,
        // wrap it so tools can still access it via the registry normalizer.
        tracing::warn!(
            raw_len = raw.len(),
            raw_preview = %raw.chars().take(200).collect::<String>(),
            "tool call arguments failed JSON parse, wrapping as string"
        );
        serde_json::Value::String(raw.to_string())
    }

    pub(super) fn invalid_tool_payload(tool_name: &str, error: &str) -> serde_json::Value {
        serde_json::json!({
            "tool": tool_name,
            "error": error,
        })
    }

    pub(super) fn prevalidate_tool_arguments(
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        if tool_name != "write" {
            return None;
        }

        if let Some(obj) = input.as_object() {
            let file_path = obj
                .get("file_path")
                .or_else(|| obj.get("filePath"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let content = obj.get("content").and_then(|v| v.as_str());
            if file_path.is_some() && content.is_some() {
                return None;
            }

            let keys = obj.keys().cloned().collect::<Vec<_>>();
            let mut payload = if file_path.is_none() {
                Self::invalid_tool_payload(
                    "write",
                    "The write tool was called without file_path/filePath. Provide both file_path and content.",
                )
            } else {
                Self::invalid_tool_payload(
                    "write",
                    "The write tool was called without content. Provide both file_path and content.",
                )
            };
            if let Some(map) = payload.as_object_mut() {
                map.insert(
                    "receivedArgs".to_string(),
                    serde_json::json!({
                        "type": "object",
                        "keys": keys,
                    }),
                );
            }
            return Some(payload);
        }

        if let Some(raw) = input.as_str() {
            let mut payload = Self::invalid_tool_payload(
                "write",
                "The write tool arguments could not be parsed into an object. Provide a JSON object with file_path and content.",
            );
            if let Some(map) = payload.as_object_mut() {
                map.insert(
                    "receivedArgs".to_string(),
                    serde_json::json!({
                        "type": "string",
                        "raw_len": raw.len(),
                        "preview": raw.chars().take(240).collect::<String>(),
                    }),
                );
            }
            return Some(payload);
        }

        None
    }

    pub(super) fn sanitize_tool_call_input_for_history(
        tool_name: &str,
        input: &serde_json::Value,
        error: Option<&str>,
    ) -> serde_json::Value {
        if input.is_object() {
            return input.clone();
        }

        if let Some(raw) = input.as_str() {
            if let Some(parsed) = rocode_util::json::try_parse_json_object_robust(raw) {
                return parsed;
            }
            if let Some(recovered) =
                rocode_util::json::recover_tool_arguments_from_jsonish(tool_name, raw)
            {
                return recovered;
            }

            let mut payload = Self::invalid_tool_payload(
                tool_name,
                error.unwrap_or("Tool arguments are malformed or truncated"),
            );
            if let Some(map) = payload.as_object_mut() {
                map.insert(
                    "receivedArgs".to_string(),
                    serde_json::json!({
                        "type": "string",
                        "raw_len": raw.len(),
                        "preview": raw.chars().take(240).collect::<String>(),
                    }),
                );
            }
            return payload;
        }

        Self::invalid_tool_payload(
            tool_name,
            error.unwrap_or("Tool arguments are non-object and cannot be replayed"),
        )
    }

    pub(super) fn tool_call_input_for_execution(
        status: &crate::ToolCallStatus,
        input: &serde_json::Value,
        raw: Option<&str>,
        state: Option<&crate::ToolState>,
    ) -> Option<serde_json::Value> {
        tracing::info!(
            status = %format!("{:?}", status),
            input_type = %if input.is_object() { format!("object(keys={})", input.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(",")).unwrap_or_default()) } else if input.is_string() { format!("string(len={})", input.as_str().map(|s| s.len()).unwrap_or(0)) } else { format!("{:?}", input) },
            raw_len = %raw.map(|r| r.len()).unwrap_or(0),
            raw_preview = %raw.unwrap_or("None").chars().take(200).collect::<String>(),
            state_variant = %match state {
                Some(crate::ToolState::Pending { .. }) => "Pending",
                Some(crate::ToolState::Running { .. }) => "Running",
                Some(crate::ToolState::Completed { .. }) => "Completed",
                Some(crate::ToolState::Error { .. }) => "Error",
                None => "None",
            },
            "[DIAG] tool_call_input_for_execution entry"
        );
        let (state_input, state_raw) = match state {
            Some(crate::ToolState::Pending { input, raw }) => {
                (Some(input.clone()), Some(raw.as_str()))
            }
            Some(crate::ToolState::Running { input, .. })
            | Some(crate::ToolState::Completed { input, .. })
            | Some(crate::ToolState::Error { input, .. }) => (Some(input.clone()), None),
            None => (None, None),
        };

        let raw_input = state_raw.or(raw).map(str::trim).filter(|s| !s.is_empty());

        match status {
            // TS parity: tool execution begins on "tool-call" (running state),
            // not on partial/pending input fragments.
            crate::ToolCallStatus::Pending => None,
            crate::ToolCallStatus::Running => {
                // Try raw input first (most authoritative source).
                if let Some(raw) = raw_input {
                    let parsed = Self::parse_json_or_string(raw);
                    // If parse_json_or_string returned a non-empty object, use it.
                    if parsed.is_object() && parsed.as_object().is_some_and(|o| !o.is_empty()) {
                        return Some(parsed);
                    }
                    // If it returned a Value::String, the registry normalizer
                    // will try to parse it again. Still usable.
                    if parsed.is_string() {
                        return Some(parsed);
                    }
                }

                // Fall back to state_input or PartType input.
                let fallback = state_input.unwrap_or_else(|| input.clone());

                // If the fallback is an empty object but the PartType input is
                // a string (e.g. Value::String wrapping JSON), try to parse it.
                if fallback.is_object() && fallback.as_object().is_some_and(|o| o.is_empty()) {
                    // Try the PartType's input directly — it might be a
                    // Value::String containing valid JSON.
                    if let Some(s) = input.as_str() {
                        if let Some(parsed) = rocode_util::json::try_parse_json_object_robust(s) {
                            tracing::debug!(
                                "tool_call_input_for_execution: recovered args from input string"
                            );
                            return Some(parsed);
                        }
                    }
                    // Also try raw from PartType even if state_raw was empty.
                    if let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) {
                        if let Some(parsed) = rocode_util::json::try_parse_json_object_robust(raw) {
                            tracing::debug!(
                                "tool_call_input_for_execution: recovered args from raw field"
                            );
                            return Some(parsed);
                        }
                    }
                }

                Some(fallback)
            }
            crate::ToolCallStatus::Completed | crate::ToolCallStatus::Error => None,
        }
    }

    pub(super) fn tool_call_raw_shape_for_execution(
        input: &serde_json::Value,
        raw: Option<&str>,
        state: Option<&crate::ToolState>,
    ) -> serde_json::Value {
        match state {
            Some(crate::ToolState::Pending { raw, .. }) if !raw.trim().is_empty() => {
                serde_json::Value::String(raw.clone())
            }
            _ => {
                if let Some(raw) = raw.filter(|value| !value.trim().is_empty()) {
                    serde_json::Value::String(raw.to_string())
                } else {
                    input.clone()
                }
            }
        }
    }

    pub(super) fn append_delta_part(message: &mut SessionMessage, reasoning: bool, delta: &str) {
        if delta.is_empty() {
            return;
        }

        for part in message.parts.iter_mut().rev() {
            match (&mut part.part_type, reasoning) {
                (PartType::Reasoning { text }, true) => {
                    text.push_str(delta);
                    return;
                }
                (PartType::Text { text, .. }, false) => {
                    text.push_str(delta);
                    return;
                }
                _ => {}
            }
        }

        message.parts.push(crate::MessagePart {
            id: rocode_core::id::create(rocode_core::id::Prefix::Part, true, None),
            part_type: if reasoning {
                PartType::Reasoning {
                    text: delta.to_string(),
                }
            } else {
                PartType::Text {
                    text: delta.to_string(),
                    synthetic: None,
                    ignored: None,
                }
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
    }

    /// Mark any tool calls that lack a corresponding tool result as aborted.
    pub(super) fn abort_pending_tool_calls(session: &mut Session) {
        let mut resolved_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for msg in &session.messages {
            for part in &msg.parts {
                if let PartType::ToolResult { tool_call_id, .. } = &part.part_type {
                    resolved_call_ids.insert(tool_call_id.clone());
                }
            }
        }

        let mut pending_calls: Vec<String> = Vec::new();
        for msg in &session.messages {
            for part in &msg.parts {
                if let PartType::ToolCall { id, .. } = &part.part_type {
                    if !resolved_call_ids.contains(id) {
                        pending_calls.push(id.clone());
                    }
                }
            }
        }

        if pending_calls.is_empty() {
            return;
        }

        tracing::info!(
            count = pending_calls.len(),
            "Marking pending tool calls as aborted"
        );
        let pending_set: std::collections::HashSet<String> =
            pending_calls.iter().cloned().collect();
        let now = chrono::Utc::now().timestamp_millis();
        for msg in session.messages_mut() {
            for part in &mut msg.parts {
                if let PartType::ToolCall {
                    id,
                    name,
                    input,
                    status,
                    state,
                    ..
                } = &mut part.part_type
                {
                    if !pending_set.contains(id) {
                        continue;
                    }
                    let sanitized_input = Self::sanitize_tool_call_input_for_history(
                        name,
                        input,
                        Some("Tool execution aborted"),
                    );
                    *input = sanitized_input.clone();
                    *status = crate::ToolCallStatus::Error;
                    *state = Some(crate::ToolState::Error {
                        input: sanitized_input,
                        error: "Tool execution aborted".to_string(),
                        metadata: None,
                        time: crate::ErrorTime {
                            start: now,
                            end: now,
                        },
                    });
                }
            }
        }

        let mut tool_results_msg = SessionMessage::tool(session.id.clone());
        for call_id in &pending_calls {
            tool_results_msg.add_tool_result(call_id, "Tool execution aborted", true);
        }
        session.push_message(tool_results_msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MessageRole, PartType, Session, SessionMessage};
    use std::collections::HashMap;

    #[test]
    fn abort_pending_tool_calls_marks_unresolved_calls_as_error() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();

        // Add a user message
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "do something"));

        // Add an assistant message with two tool calls but only one result
        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({"command": "echo a"}));
        assistant.add_tool_call("call_2", "read_file", serde_json::json!({"path": "foo.rs"}));
        session.messages_mut().push(assistant);
        let mut existing_tool_result = SessionMessage::tool(sid.clone());
        existing_tool_result.add_tool_result("call_1", "output a", false);
        session.messages_mut().push(existing_tool_result);
        // call_2 has no result — simulates abort mid-execution

        SessionPrompt::abort_pending_tool_calls(&mut session);

        // call_2 should now have an error result in the latest tool message
        let last_tool_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Tool))
            .unwrap();

        let error_results: Vec<_> = last_tool_msg
            .parts
            .iter()
            .filter(|p| matches!(
                &p.part_type,
                PartType::ToolResult { tool_call_id, is_error, content, .. }
                    if tool_call_id == "call_2" && *is_error && content == "Tool execution aborted"
            ))
            .collect();

        assert_eq!(error_results.len(), 1, "call_2 should have an error result");
    }

    #[test]
    fn abort_pending_tool_calls_noop_when_all_resolved() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();

        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "do something"));

        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({"command": "echo a"}));
        session.messages_mut().push(assistant);
        let mut tool_result = SessionMessage::tool(sid.clone());
        tool_result.add_tool_result("call_1", "output a", false);
        session.messages_mut().push(tool_result);

        let part_count_before = session.messages.last().map(|m| m.parts.len()).unwrap_or(0);

        SessionPrompt::abort_pending_tool_calls(&mut session);

        let part_count_after = session.messages.last().map(|m| m.parts.len()).unwrap_or(0);

        assert_eq!(
            part_count_before, part_count_after,
            "No new parts should be added"
        );
    }

    #[test]
    fn abort_pending_tool_calls_handles_multiple_pending() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();

        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "do something"));

        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({}));
        assistant.add_tool_call("call_2", "read_file", serde_json::json!({}));
        assistant.add_tool_call("call_3", "write_file", serde_json::json!({}));
        // No results at all — all three are pending
        session.messages_mut().push(assistant);

        SessionPrompt::abort_pending_tool_calls(&mut session);

        let last_tool_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Tool))
            .unwrap();

        let abort_results: Vec<_> = last_tool_msg
            .parts
            .iter()
            .filter(|p| {
                matches!(
                    &p.part_type,
                    PartType::ToolResult { is_error, content, .. }
                        if *is_error && content == "Tool execution aborted"
                )
            })
            .collect();

        assert_eq!(
            abort_results.len(),
            3,
            "All three pending calls should be aborted"
        );
    }

    #[test]
    fn tool_call_input_for_execution_skips_empty_pending() {
        let input = serde_json::json!({});
        let resolved = SessionPrompt::tool_call_input_for_execution(
            &crate::ToolCallStatus::Pending,
            &input,
            None,
            None,
        );
        assert!(resolved.is_none());
    }

    #[test]
    fn tool_call_input_for_execution_pending_ignores_raw_input() {
        let input = serde_json::json!({});
        let resolved = SessionPrompt::tool_call_input_for_execution(
            &crate::ToolCallStatus::Pending,
            &input,
            Some("[filePath=/tmp/a.html]"),
            None,
        );
        assert!(resolved.is_none());
    }

    #[test]
    fn tool_call_input_for_execution_running_keeps_object_input() {
        let input = serde_json::json!({});
        let resolved = SessionPrompt::tool_call_input_for_execution(
            &crate::ToolCallStatus::Running,
            &input,
            None,
            None,
        );
        assert_eq!(resolved, Some(serde_json::json!({})));
    }

    #[test]
    fn tool_call_raw_shape_for_execution_prefers_raw_string() {
        let raw = SessionPrompt::tool_call_raw_shape_for_execution(
            &serde_json::json!({"parsed": true}),
            Some("{\"raw\":true}"),
            None,
        );
        assert_eq!(raw, serde_json::Value::String("{\"raw\":true}".to_string()));
    }

    #[test]
    fn invalid_tool_payload_is_ts_shape() {
        let payload = SessionPrompt::invalid_tool_payload("read", "missing filePath");
        assert_eq!(payload.get("tool").and_then(|v| v.as_str()), Some("read"));
        assert_eq!(
            payload.get("error").and_then(|v| v.as_str()),
            Some("missing filePath")
        );
        assert!(payload.get("receivedArgs").is_none());
    }

    #[test]
    fn sanitize_tool_call_input_for_history_wraps_unrecoverable_string() {
        let input = serde_json::Value::String("not-json".to_string());
        let sanitized = SessionPrompt::sanitize_tool_call_input_for_history(
            "write",
            &input,
            Some("Invalid arguments"),
        );
        assert!(sanitized.is_object());
        assert_eq!(sanitized["tool"], "write");
        assert_eq!(sanitized["error"], "Invalid arguments");
        assert_eq!(sanitized["receivedArgs"]["type"], "string");
    }

    #[test]
    fn abort_pending_tool_calls_sanitizes_pending_tool_input_for_replay() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();
        session
            .messages_mut()
            .push(SessionMessage::user(sid.clone(), "do something"));

        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.parts.push(crate::MessagePart {
            id: "prt_bad".to_string(),
            part_type: PartType::ToolCall {
                id: "call_bad".to_string(),
                name: "write".to_string(),
                input: serde_json::Value::String("not-json".to_string()),
                status: crate::ToolCallStatus::Pending,
                raw: Some("not-json".to_string()),
                state: Some(crate::ToolState::Pending {
                    input: serde_json::json!({}),
                    raw: "not-json".to_string(),
                }),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        session.messages_mut().push(assistant);

        SessionPrompt::abort_pending_tool_calls(&mut session);

        let assistant = session
            .messages
            .iter()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .expect("assistant message missing");
        let tool_call = assistant
            .parts
            .iter()
            .find_map(|p| match &p.part_type {
                PartType::ToolCall { input, status, .. } => Some((input, status)),
                _ => None,
            })
            .expect("tool call missing");
        assert!(
            tool_call.0.is_object(),
            "pending malformed input should be sanitized for replay"
        );
        assert!(matches!(tool_call.1, crate::ToolCallStatus::Error));
    }

    #[test]
    fn parse_json_or_string_recovers_stringified_object() {
        let inner = r#"{"file_path":"/tmp/a","content":"hello"}"#;
        let outer = serde_json::to_string(inner).expect("stringify should succeed");
        let parsed = SessionPrompt::parse_json_or_string(&outer);
        assert_eq!(parsed["file_path"], "/tmp/a");
        assert_eq!(parsed["content"], "hello");
    }

    #[test]
    fn parse_json_or_string_keeps_plain_json_string_when_not_object() {
        let raw = serde_json::to_string("just text").expect("stringify should succeed");
        let parsed = SessionPrompt::parse_json_or_string(&raw);
        assert_eq!(parsed, serde_json::Value::String("just text".to_string()));
    }

    #[test]
    fn prevalidate_write_args_requires_file_path() {
        let input = serde_json::json!({
            "content": "<html>...</html>"
        });
        let invalid = SessionPrompt::prevalidate_tool_arguments("write", &input)
            .expect("should produce invalid payload");
        assert_eq!(invalid["tool"], "write");
        assert!(
            invalid["error"]
                .as_str()
                .unwrap_or_default()
                .contains("without file_path"),
            "error should mention missing file_path"
        );
        assert_eq!(invalid["receivedArgs"]["type"], "object");
    }

    #[test]
    fn prevalidate_write_args_accepts_complete_payload() {
        let input = serde_json::json!({
            "file_path": "t2.html",
            "content": "<html>...</html>"
        });
        assert!(SessionPrompt::prevalidate_tool_arguments("write", &input).is_none());
    }

    #[test]
    fn extract_tool_attachments_from_metadata_moves_attachment_payload_out() {
        let mut metadata = HashMap::new();
        metadata.insert("note".to_string(), serde_json::json!("ok"));
        metadata.insert(
            "attachments".to_string(),
            serde_json::json!([{ "mime": "application/pdf", "url": "data:application/pdf;base64,AA==" }]),
        );
        metadata.insert(
            "attachment".to_string(),
            serde_json::json!({ "mime": "image/png", "url": "data:image/png;base64,BB==" }),
        );

        let (attachments, file_parts) =
            SessionPrompt::extract_tool_attachments_from_metadata(&mut metadata, "ses_1", "msg_1");

        assert_eq!(metadata.get("note").and_then(|v| v.as_str()), Some("ok"));
        assert!(!metadata.contains_key("attachments"));
        assert!(!metadata.contains_key("attachment"));
        assert_eq!(attachments.as_ref().map(|v| v.len()), Some(2));
        assert_eq!(file_parts.as_ref().map(|v| v.len()), Some(2));
    }
}
