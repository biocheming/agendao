// Message building/conversion/compaction methods for SessionPrompt

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rocode_provider::{get_model_context_limit, Content, ContentPart, Message, Provider, Role};

use crate::compaction::{
    resolved_compaction_config, CompactionConfig, CompactionEngine, MessageForPrune, ModelLimits,
    PruneToolPart, TokenUsage, ToolPartStatus,
};
use crate::message_v2::{
    AssistantTime, AssistantTokens, CacheTokens, CompactionPart as V2CompactionPart, MessageInfo,
    MessagePath, MessageWithParts, ModelRef as V2ModelRef, Part as V2Part, StepFinishPart,
    StepStartPart, StepTokens, UserTime,
};
use crate::session::sanitize_display_text;
use crate::summary::{summarize_into_session, SummarizeInput};
use crate::{MessageRole, PartType, Session, SessionMessage};

use super::surface_contract::{
    parse_hidden_runtime_hint, sanctioned_model_context_projection_for_message,
};
use super::tools_and_output::{compose_session_title_source, generate_session_title_for_session};
use super::{
    SessionPrompt, CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY,
    CONTEXT_COMPACTION_RECORD_METADATA_KEY,
};
use rocode_types::{
    SessionContinuityCompactionSummary, SessionContinuityDependency,
    SessionContinuityDependencyKind, SessionContinuityLedgerEntry, SessionContinuityLedgerKind,
    SessionContinuityLimits, SessionContinuityPacket, SessionContinuityTurn,
};

type LegacyToolResult = (
    String,
    bool,
    Option<String>,
    Option<HashMap<String, serde_json::Value>>,
    Option<Vec<serde_json::Value>>,
);

type LegacyToolResultMap = HashMap<String, LegacyToolResult>;

const CONTEXT_AUTO_COMPACT_THRESHOLD_PERCENT: u64 = 90;
const AUTO_COMPACTION_RECENT_WINDOW_MESSAGES: usize = 12;
const AUTO_COMPACTION_MIN_MESSAGES_AFTER_LAST: usize = 4;
pub(super) const FORCE_COMPACTION_MIN_MESSAGES: usize = 2;
const AUTO_COMPACTION_MIN_MESSAGES: usize = 10;
const MAX_BODY_CHARS: usize = 5_000_000;
const MAX_CONTEXT_CHARS: usize = 200_000;
const COMPACTION_CONTINUITY_RECENT_TAIL_MESSAGES: usize = 6;
const COMPACTION_CONTINUITY_CONTEXT_TEXT_LIMIT: usize = 6_000;
const COMPACTION_CONTINUITY_TURN_TEXT_LIMIT: usize = 1_200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactionAssessment {
    pub reason: &'static str,
    pub limit_tokens: Option<u64>,
    pub body_chars: Option<usize>,
}

struct LegacyToolStateInput<'a> {
    tool_call_id: &'a str,
    tool_name: &'a str,
    input: &'a serde_json::Value,
    status: &'a crate::ToolCallStatus,
    raw: &'a str,
    tool_result: Option<&'a LegacyToolResult>,
    session_id: &'a str,
    message_id: &'a str,
}

impl SessionPrompt {
    pub(crate) fn build_compaction_record(
        trigger: &str,
        phase: Option<&str>,
        reason: Option<&str>,
        forced: bool,
        request_context_tokens: Option<u64>,
        live_context_tokens: Option<u64>,
        limit_tokens: Option<u64>,
        body_chars: Option<usize>,
    ) -> serde_json::Value {
        serde_json::json!({
            "trigger": trigger,
            "phase": phase,
            "reason": reason,
            "forced": forced,
            "request_context_tokens": request_context_tokens,
            "live_context_tokens": live_context_tokens,
            "limit_tokens": limit_tokens,
            "body_chars": body_chars,
        })
    }

    fn model_hidden_runtime_hint(message: &SessionMessage) -> Option<&str> {
        parse_hidden_runtime_hint(
            message
                .metadata
                .get("runtime_hint")
                .and_then(|value| value.as_str())?,
        )
        .map(|hint| hint.as_str())
    }

    pub(super) fn is_model_visible_message(message: &SessionMessage) -> bool {
        Self::model_hidden_runtime_hint(message).is_none()
    }

    pub(super) fn runtime_compaction_config(
        config_store: Option<&rocode_config::ConfigStore>,
    ) -> CompactionConfig {
        resolved_compaction_config(config_store)
    }

    pub(super) fn build_chat_messages(
        session_messages: &[SessionMessage],
        system_prompt: Option<&str>,
    ) -> anyhow::Result<Vec<Message>> {
        let mut messages = Vec::new();

        if let Some(system) = system_prompt {
            messages.push(Message::system(system));
        }

        for msg in session_messages {
            if !Self::is_model_visible_message(msg) {
                continue;
            }

            // Skip messages with no parts — empty Tool/Assistant messages
            // confuse providers, especially the Ethnopic-compatible family
            // which rejects empty content.
            if msg.parts.is_empty() {
                continue;
            }

            if let Some(summary) = Self::projected_model_context_summary(msg) {
                messages.push(Message::assistant(summary));
                continue;
            }

            if matches!(msg.role, MessageRole::Assistant)
                && msg
                    .parts
                    .iter()
                    .any(|p| matches!(p.part_type, PartType::ToolResult { .. }))
            {
                // Backward-compat: old sessions may carry tool_result parts on
                // assistant messages. Split those into a synthetic tool-role
                // message to preserve provider role expectations.
                let mut assistant_parts = Vec::new();
                let mut tool_parts = Vec::new();
                for part in &msg.parts {
                    if matches!(part.part_type, PartType::ToolResult { .. }) {
                        tool_parts.push(part.clone());
                    } else {
                        assistant_parts.push(part.clone());
                    }
                }

                if !assistant_parts.is_empty() {
                    messages.push(Message::assistant_parts(
                        match Self::parts_to_content(&assistant_parts) {
                            Content::Parts(parts) => parts,
                            Content::Text(text) => vec![ContentPart::text(text)],
                        },
                    ));
                }
                if !tool_parts.is_empty() {
                    messages.push(Message::tool_parts(
                        match Self::parts_to_content(&tool_parts) {
                            Content::Parts(parts) => parts,
                            Content::Text(text) => vec![ContentPart::text(text)],
                        },
                    ));
                }
                continue;
            }

            let content = Self::parts_to_content(&msg.parts);
            let role = match msg.role {
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::System => Role::System,
                MessageRole::Tool => Role::Tool,
            };

            messages.push(Message {
                role,
                content,
                cache_control: None,
                provider_options: None,
            });
        }

        Ok(messages)
    }

    fn projected_model_context_summary(msg: &SessionMessage) -> Option<String> {
        sanctioned_model_context_projection_for_message(msg)
            .map(|projection| projection.summary.to_owned())
    }

    pub(super) fn model_context_char_len(message: &SessionMessage) -> usize {
        if !Self::is_model_visible_message(message) {
            return 0;
        }

        if let Some(summary) = Self::projected_model_context_summary(message) {
            return summary.len();
        }

        message
            .parts
            .iter()
            .map(|p| match &p.part_type {
                PartType::Text { text, .. } => text.len(),
                PartType::ToolResult { content, title, .. } => {
                    content.len() + title.as_ref().map_or(0, |t| t.len())
                }
                PartType::ToolCall { input, raw, .. } => {
                    let input_len = serde_json::to_string(input).map_or(0, |s| s.len());
                    input_len + raw.as_ref().map_or(0, |r| r.len())
                }
                PartType::Reasoning { text } => text.len(),
                _ => 0,
            })
            .sum()
    }

    pub(super) fn parts_to_content(parts: &[crate::MessagePart]) -> Content {
        let has_parts = parts
            .iter()
            .any(|p| !matches!(p.part_type, PartType::Text { .. }));

        if !has_parts {
            let text = parts
                .iter()
                .filter_map(|p| match &p.part_type {
                    PartType::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Content::Text(text);
        }

        let content_parts: Vec<ContentPart> = parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(ContentPart::text(text.clone())),
                PartType::Reasoning { text } => Some(ContentPart::reasoning(text.clone())),
                PartType::ToolCall {
                    id, name, input, ..
                } => Some(ContentPart::tool_use(
                    id.clone(),
                    name.clone(),
                    input.clone(),
                )),
                PartType::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                    ..
                } => Some(ContentPart::tool_result(
                    tool_call_id.clone(),
                    content.clone(),
                    Some(*is_error),
                )),
                PartType::File {
                    url,
                    filename,
                    mime,
                } => {
                    if mime.starts_with("image/") {
                        Some(ContentPart::image_url(
                            url.clone(),
                            Some(filename.clone()),
                            Some(mime.clone()),
                        ))
                    } else if mime.starts_with("audio/") {
                        Some(ContentPart::file(
                            url.clone(),
                            Some(filename.clone()),
                            Some(mime.clone()),
                        ))
                    } else {
                        Some(ContentPart {
                            filename: Some(filename.clone()),
                            media_type: Some(mime.clone()),
                            ..ContentPart::text(format!("[File: {} ({})]", filename, mime))
                        })
                    }
                }
                _ => None,
            })
            .collect();

        Content::Parts(content_parts)
    }

    pub(super) fn filter_compacted_messages(messages: &[SessionMessage]) -> Vec<SessionMessage> {
        let Some(compaction_index) = messages.iter().rposition(|m| {
            m.parts
                .iter()
                .any(|p| matches!(p.part_type, PartType::Compaction { .. }))
        }) else {
            return messages.to_vec();
        };

        let tail = messages[compaction_index..].to_vec();
        let Some(compaction_message) = messages.get(compaction_index) else {
            return tail;
        };

        if let Some(filtered) = Self::filter_compacted_messages_from_continuity_packet(
            messages,
            compaction_index,
            compaction_message,
        ) {
            return filtered;
        }

        Self::filter_compacted_messages_legacy(messages, compaction_index)
    }

    fn filter_compacted_messages_from_continuity_packet(
        messages: &[SessionMessage],
        compaction_index: usize,
        compaction_message: &SessionMessage,
    ) -> Option<Vec<SessionMessage>> {
        let packet = compaction_message
            .metadata
            .get(CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY)
            .and_then(SessionContinuityPacket::from_value)?;
        let allowed_ids = packet.allowed_message_ids();
        if allowed_ids.is_empty() {
            return None;
        }
        let allowed_set = allowed_ids.into_iter().collect::<HashSet<_>>();
        let filtered = messages
            .iter()
            .enumerate()
            .filter(|(index, message)| {
                *index >= compaction_index || allowed_set.contains(&message.id)
            })
            .map(|(_, message)| message)
            .cloned()
            .collect::<Vec<_>>();
        Self::filter_compacted_messages_packet_result_valid(messages, &packet, &filtered)
            .then_some(filtered)
    }

    fn filter_compacted_messages_packet_result_valid(
        all_messages: &[SessionMessage],
        packet: &SessionContinuityPacket,
        filtered: &[SessionMessage],
    ) -> bool {
        if filtered.is_empty() {
            return false;
        }
        if !filtered
            .iter()
            .any(|message| matches!(message.role, MessageRole::User))
        {
            return false;
        }
        let Some(last_filtered_idx) = filtered
            .iter()
            .enumerate()
            .rfind(|(_, message)| matches!(message.role, MessageRole::User))
            .map(|(index, _)| index)
        else {
            return false;
        };
        let Some(last_user_id) = filtered
            .get(last_filtered_idx)
            .map(|message| message.id.as_str())
        else {
            return false;
        };
        let Some(start_idx) = all_messages
            .iter()
            .position(|message| message.id == last_user_id)
        else {
            return false;
        };
        let expected_current_turn = &all_messages[start_idx..];
        let has_current_turn = expected_current_turn
            .iter()
            .all(|message| filtered.iter().any(|candidate| candidate.id == message.id));
        if !has_current_turn {
            return false;
        }
        packet.continuation_dependencies.iter().all(|dependency| {
            dependency.message_ids.iter().all(|message_id| {
                filtered
                    .iter()
                    .any(|candidate| candidate.id.as_str() == message_id.as_str())
            })
        })
    }

    fn filter_compacted_messages_legacy(
        messages: &[SessionMessage],
        compaction_index: usize,
    ) -> Vec<SessionMessage> {
        let tail = messages[compaction_index..].to_vec();
        if tail.iter().any(|m| matches!(m.role, MessageRole::User)) {
            return tail;
        }

        // Keep the latest user anchor before the compaction boundary so prompt
        // loop invariants hold (`last_user_idx` must exist), and preserve the
        // entire current turn chain between that user and the compaction
        // boundary. Provider continuations may depend on assistant/tool rounds
        // that occurred earlier in the same turn.
        if let Some(last_user_idx) = messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::User))
        {
            if last_user_idx < compaction_index {
                let mut anchored = Vec::with_capacity(messages.len() - last_user_idx);
                anchored.extend_from_slice(&messages[last_user_idx..compaction_index]);
                anchored.extend_from_slice(&messages[compaction_index..]);
                return anchored;
            }
        }

        tail
    }

    fn build_compaction_continuity_packet(
        session: &Session,
        messages: &[SessionMessage],
        summary: &str,
        compaction_message_id: &str,
    ) -> Option<SessionContinuityPacket> {
        let exact_recent_tail = Self::collect_compaction_recent_tail(messages);
        let eligible_message_count = Self::count_compaction_context_messages(messages);
        let working_ledger = Self::build_compaction_working_ledger(session, &exact_recent_tail);
        let continuation_dependencies =
            Self::collect_compaction_continuation_dependencies(messages);

        if exact_recent_tail.is_empty()
            && working_ledger.is_empty()
            && continuation_dependencies.is_empty()
            && summary.trim().is_empty()
        {
            return None;
        }

        let exact_recent_tail_count = exact_recent_tail.len();
        Some(SessionContinuityPacket {
            eligible_message_count,
            exact_recent_tail_count,
            omitted_older_turns: eligible_message_count.saturating_sub(exact_recent_tail_count),
            exact_recent_tail,
            memory_anchors: Vec::new(),
            working_ledger,
            continuation_dependencies,
            latest_compaction_summary: (!summary.trim().is_empty()).then(|| {
                SessionContinuityCompactionSummary {
                    message_id: compaction_message_id.to_string(),
                    summary: summary.trim().to_string(),
                }
            }),
            limits: Some(SessionContinuityLimits {
                recent_tail_messages: COMPACTION_CONTINUITY_RECENT_TAIL_MESSAGES,
                context_text_chars: COMPACTION_CONTINUITY_CONTEXT_TEXT_LIMIT,
                turn_text_chars: COMPACTION_CONTINUITY_TURN_TEXT_LIMIT,
            }),
            recall_policy: Some(
                "exact_tail_for_recent_followups; working_ledger_and_compaction_summary_are_lossy; use live session history or tools when exact prior text, current files, diagnostics, or verification evidence matters."
                    .to_string(),
            ),
            ..SessionContinuityPacket::default()
        })
    }

    fn collect_compaction_recent_tail(messages: &[SessionMessage]) -> Vec<SessionContinuityTurn> {
        let mut turns = messages
            .iter()
            .rev()
            .filter(|message| Self::is_compaction_context_message(message))
            .filter_map(|message| {
                let text = sanitize_display_text(&message.get_text());
                let text = text.trim();
                (!text.is_empty()).then(|| SessionContinuityTurn {
                    message_id: message.id.clone(),
                    role: Self::compaction_role_label(&message.role).to_string(),
                    text: Self::truncate_chars(text, COMPACTION_CONTINUITY_TURN_TEXT_LIMIT),
                    projected: false,
                })
            })
            .take(COMPACTION_CONTINUITY_RECENT_TAIL_MESSAGES)
            .collect::<Vec<_>>();
        turns.reverse();
        turns
    }

    fn collect_compaction_continuation_dependencies(
        messages: &[SessionMessage],
    ) -> Vec<SessionContinuityDependency> {
        let Some(last_user_idx) = messages
            .iter()
            .rposition(|message| matches!(message.role, MessageRole::User))
        else {
            return Vec::new();
        };

        let turn_chain = &messages[last_user_idx..];
        if turn_chain.len() <= 1 {
            return Vec::new();
        }

        let requires_exact_continuation = turn_chain.iter().skip(1).any(|message| {
            matches!(message.role, MessageRole::Tool)
                || (matches!(message.role, MessageRole::Assistant)
                    && message.parts.iter().any(|part| {
                        matches!(
                            part.part_type,
                            PartType::ToolCall { .. } | PartType::Reasoning { .. }
                        )
                    }))
        });
        if !requires_exact_continuation {
            return Vec::new();
        }

        vec![SessionContinuityDependency {
            kind: SessionContinuityDependencyKind::AssistantToolCallContinuation,
            anchor_message_id: Some(messages[last_user_idx].id.clone()),
            message_ids: turn_chain
                .iter()
                .map(|message| message.id.clone())
                .collect(),
        }]
    }

    fn count_compaction_context_messages(messages: &[SessionMessage]) -> usize {
        messages
            .iter()
            .filter(|message| Self::is_compaction_context_message(message))
            .filter(|message| !sanitize_display_text(&message.get_text()).trim().is_empty())
            .count()
    }

    fn is_compaction_context_message(message: &SessionMessage) -> bool {
        matches!(message.role, MessageRole::User | MessageRole::Assistant)
    }

    fn build_compaction_working_ledger(
        session: &Session,
        recent_tail: &[SessionContinuityTurn],
    ) -> Vec<SessionContinuityLedgerEntry> {
        let mut ledger = Vec::new();
        let title = session.title.trim();
        if !title.is_empty() && !session.is_default_title() {
            ledger.push(SessionContinuityLedgerEntry::new(
                SessionContinuityLedgerKind::SessionTitle,
                format!("session_title: {}", Self::truncate_chars(title, 160)),
            ));
        }
        if let Some(summary) = session.summary.as_ref() {
            ledger.push(SessionContinuityLedgerEntry::new(
                SessionContinuityLedgerKind::SessionDiff,
                format!(
                    "session_diff: files={} additions={} deletions={}",
                    summary.files, summary.additions, summary.deletions
                ),
            ));
        }
        if let Some(turn) = recent_tail.iter().rev().find(|turn| turn.role == "user") {
            ledger.push(SessionContinuityLedgerEntry::with_source_id(
                SessionContinuityLedgerKind::LatestUserTurn,
                turn.message_id.clone(),
                format!(
                    "latest_user_turn `{}`: {}",
                    turn.message_id,
                    Self::single_line(&Self::truncate_chars(&turn.text, 240))
                ),
            ));
        }
        if let Some(turn) = recent_tail
            .iter()
            .rev()
            .find(|turn| turn.role == "assistant")
        {
            ledger.push(SessionContinuityLedgerEntry::with_source_id(
                SessionContinuityLedgerKind::LatestAssistantOutcome,
                turn.message_id.clone(),
                format!(
                    "latest_assistant_outcome `{}`: {}",
                    turn.message_id,
                    Self::single_line(&Self::truncate_chars(&turn.text, 360))
                ),
            ));
        }
        if !ledger.is_empty() {
            ledger.push(SessionContinuityLedgerEntry::new(
                SessionContinuityLedgerKind::SourcePolicy,
                "source_policy: use Exact Recent Tail for prior same-session outputs; compaction summary and ledger are lossy continuity aids, not exact replay; use live files, diagnostics, or tools when exact current state matters."
                    .to_string(),
            ));
        }
        ledger
    }

    fn compaction_role_label(role: &MessageRole) -> &'static str {
        match role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        }
    }

    fn single_line(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn truncate_chars(value: &str, limit: usize) -> String {
        if value.chars().count() <= limit {
            return value.to_string();
        }
        let mut truncated = value
            .chars()
            .take(limit.saturating_sub(24))
            .collect::<String>();
        truncated.push_str("\n...[truncated]...");
        truncated
    }

    pub(super) fn token_usage_from_messages(messages: &[SessionMessage]) -> TokenUsage {
        let mut usage = TokenUsage {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_miss: 0,
            cache_write: 0,
            total: 0,
        };

        for msg in messages {
            // Prefer the strongly-typed usage field populated by provider stream/final responses.
            if let Some(msg_usage) = msg.usage.as_ref() {
                usage.input += msg_usage.input_tokens;
                usage.output += msg_usage.output_tokens;
                usage.cache_read += msg_usage.cache_read_tokens;
                usage.cache_miss += msg_usage.cache_miss_tokens;
                usage.cache_write += msg_usage.cache_write_tokens;
                continue;
            }

            // Fallback to metadata for backward compatibility with legacy snapshots.
            let read_metadata_u64 = |key: &str, usage_key: &str| -> u64 {
                msg.metadata
                    .get(key)
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        msg.metadata
                            .get("usage")
                            .and_then(|v| v.get(usage_key))
                            .and_then(|v| v.as_u64())
                    })
                    .unwrap_or(0)
            };

            usage.input += read_metadata_u64("tokens_input", "prompt_tokens");
            usage.output += read_metadata_u64("tokens_output", "completion_tokens");
            usage.cache_read += read_metadata_u64("tokens_cache_read", "cache_read_tokens");
            usage.cache_miss += read_metadata_u64("tokens_cache_miss", "cache_miss_tokens");
            usage.cache_write += read_metadata_u64("tokens_cache_write", "cache_write_tokens");
        }
        usage.total =
            usage.input + usage.output + usage.cache_read + usage.cache_miss + usage.cache_write;
        usage
    }

    fn context_usage_percent(used: u64, limit: u64) -> Option<u64> {
        if limit == 0 {
            return None;
        }
        Some(((used as f64 / limit as f64) * 100.0).round() as u64)
    }

    fn effective_compaction_limit(
        limits: &ModelLimits,
        compaction_config: &CompactionConfig,
    ) -> u64 {
        let reserved = compaction_config
            .reserved
            .unwrap_or_else(|| 20_000_u64.min(limits.max_output));
        limits
            .max_input
            .map(|input| input.saturating_sub(reserved))
            .unwrap_or_else(|| limits.context.saturating_sub(limits.max_output))
    }

    fn should_trigger_proactive_compaction(
        used_tokens: u64,
        limits: &ModelLimits,
        compaction_config: &CompactionConfig,
    ) -> bool {
        let limit = Self::effective_compaction_limit(limits, compaction_config);
        Self::context_usage_percent(used_tokens, limit)
            .map(|percent| percent >= CONTEXT_AUTO_COMPACT_THRESHOLD_PERCENT)
            .unwrap_or(false)
    }

    fn should_back_off_auto_compaction(messages: &[SessionMessage]) -> bool {
        let recent_compaction_offsets: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message
                    .parts
                    .iter()
                    .any(|part| matches!(part.part_type, PartType::Compaction { .. }))
            })
            .map(|(index, _)| index)
            .collect();

        let Some(last_compaction_index) = recent_compaction_offsets.last().copied() else {
            return false;
        };

        let messages_since_last = messages.len().saturating_sub(last_compaction_index + 1);
        if messages_since_last < AUTO_COMPACTION_MIN_MESSAGES_AFTER_LAST {
            return true;
        }

        messages
            .iter()
            .rev()
            .take(AUTO_COMPACTION_RECENT_WINDOW_MESSAGES)
            .filter(|message| {
                message
                    .parts
                    .iter()
                    .any(|part| matches!(part.part_type, PartType::Compaction { .. }))
            })
            .count()
            >= 2
    }

    fn provider_content_part_char_len(part: &ContentPart) -> usize {
        let text_len = part.text.as_ref().map_or(0, |text| text.len());
        let image_len = part.image_url.as_ref().map_or(0, |image| image.url.len());
        let tool_use_len = part.tool_use.as_ref().map_or(0, |tool_use| {
            tool_use.id.len()
                + tool_use.name.len()
                + serde_json::to_string(&tool_use.input).map_or(0, |value| value.len())
        });
        let tool_result_len = part.tool_result.as_ref().map_or(0, |tool_result| {
            tool_result.tool_use_id.len() + tool_result.content.len()
        });
        let filename_len = part.filename.as_ref().map_or(0, |value| value.len());
        let media_type_len = part.media_type.as_ref().map_or(0, |value| value.len());
        let provider_options_len = part.provider_options.as_ref().map_or(0, |value| {
            serde_json::to_string(value).map_or(0, |encoded| encoded.len())
        });

        text_len
            + image_len
            + tool_use_len
            + tool_result_len
            + filename_len
            + media_type_len
            + provider_options_len
    }

    fn provider_message_char_len(message: &Message) -> usize {
        let content_len = match &message.content {
            Content::Text(text) => text.len(),
            Content::Parts(parts) => parts.iter().map(Self::provider_content_part_char_len).sum(),
        };
        let provider_options_len = message.provider_options.as_ref().map_or(0, |value| {
            serde_json::to_string(value).map_or(0, |encoded| encoded.len())
        });

        content_len + provider_options_len
    }

    pub(crate) fn estimate_request_context_tokens_from_provider_messages(
        messages: &[Message],
    ) -> (Option<u64>, usize) {
        let total_chars: usize = messages.iter().map(Self::provider_message_char_len).sum();
        let estimated_tokens = (total_chars > 0).then_some((total_chars as u64) / 4);
        (estimated_tokens, total_chars)
    }

    pub(crate) fn assess_compaction(
        messages: &[SessionMessage],
        provider: &dyn Provider,
        model_id: &str,
        max_output_tokens: Option<u64>,
        compaction_config: &CompactionConfig,
        live_context_tokens: Option<u64>,
        request_context_tokens: Option<u64>,
        request_body_chars: Option<usize>,
    ) -> Option<CompactionAssessment> {
        if Self::should_back_off_auto_compaction(messages) {
            return None;
        }
        if !compaction_config.auto {
            return None;
        }

        let usage = Self::token_usage_from_messages(messages);
        let model = provider.get_model(model_id);
        let limits = ModelLimits {
            context: model
                .map(|info| info.context_window)
                .unwrap_or_else(|| get_model_context_limit(model_id)),
            max_input: model.and_then(|info| info.max_input_tokens),
            max_output: max_output_tokens
                .or_else(|| model.map(|info| info.max_output_tokens))
                .unwrap_or(8192),
        };
        let engine = CompactionEngine::new(compaction_config.clone());
        let compaction_limit = Self::effective_compaction_limit(&limits, compaction_config);
        if engine.is_overflow(&usage, &limits) {
            return Some(CompactionAssessment {
                reason: "usage_overflow",
                limit_tokens: Some(compaction_limit),
                body_chars: None,
            });
        }

        let usage_count = if usage.total > 0 {
            usage.total
        } else {
            usage.input + usage.output + usage.cache_read + usage.cache_miss + usage.cache_write
        };
        let live_context_tokens = live_context_tokens.filter(|tokens| *tokens > 0);
        if let Some(live_context_tokens) = live_context_tokens {
            let live_usage = TokenUsage {
                input: live_context_tokens,
                output: 0,
                cache_read: 0,
                cache_miss: 0,
                cache_write: 0,
                total: live_context_tokens,
            };
            if engine.is_overflow(&live_usage, &limits) {
                return Some(CompactionAssessment {
                    reason: "live_context_overflow",
                    limit_tokens: Some(compaction_limit),
                    body_chars: None,
                });
            }
        }
        if let Some(request_context_tokens) = request_context_tokens.filter(|tokens| *tokens > 0) {
            let request_usage = TokenUsage {
                input: request_context_tokens,
                output: 0,
                cache_read: 0,
                cache_miss: 0,
                cache_write: 0,
                total: request_context_tokens,
            };
            if engine.is_overflow(&request_usage, &limits) {
                return Some(CompactionAssessment {
                    reason: "request_view_overflow",
                    limit_tokens: Some(compaction_limit),
                    body_chars: request_body_chars,
                });
            }
            if Self::should_trigger_proactive_compaction(
                request_context_tokens,
                &limits,
                compaction_config,
            ) {
                return Some(CompactionAssessment {
                    reason: "request_view_threshold",
                    limit_tokens: Some(compaction_limit),
                    body_chars: request_body_chars,
                });
            }
        }
        if usage_count > 0
            && Self::should_trigger_proactive_compaction(usage_count, &limits, compaction_config)
        {
            return Some(CompactionAssessment {
                reason: "usage_threshold",
                limit_tokens: Some(compaction_limit),
                body_chars: None,
            });
        }
        if let Some(live_context_tokens) = live_context_tokens {
            if Self::should_trigger_proactive_compaction(
                live_context_tokens,
                &limits,
                compaction_config,
            ) {
                return Some(CompactionAssessment {
                    reason: "live_context_threshold",
                    limit_tokens: Some(compaction_limit),
                    body_chars: None,
                });
            }
        }

        // Estimate total content size across ALL part types (not just text).
        // This catches large tool results and tool call inputs that the
        // token-based check misses (it relies on cached API response counts).
        let total_chars: usize = messages.iter().map(Self::model_context_char_len).sum();

        let estimated_input_tokens = (total_chars as u64) / 4;
        let estimated_usage = TokenUsage {
            input: estimated_input_tokens,
            output: 0,
            cache_read: 0,
            cache_miss: 0,
            cache_write: 0,
            total: estimated_input_tokens,
        };
        if estimated_input_tokens > 0 && engine.is_overflow(&estimated_usage, &limits) {
            return Some(CompactionAssessment {
                reason: "session_content_overflow",
                limit_tokens: Some(compaction_limit),
                body_chars: Some(total_chars),
            });
        }
        if estimated_input_tokens > 0
            && Self::should_trigger_proactive_compaction(
                estimated_input_tokens,
                &limits,
                compaction_config,
            )
        {
            return Some(CompactionAssessment {
                reason: "session_content_threshold",
                limit_tokens: Some(compaction_limit),
                body_chars: Some(total_chars),
            });
        }

        if total_chars > MAX_BODY_CHARS {
            return Some(CompactionAssessment {
                reason: "request_body_too_large",
                limit_tokens: Some(compaction_limit),
                body_chars: Some(total_chars),
            });
        }

        // Softer cap based on estimated token count.
        if total_chars > MAX_CONTEXT_CHARS {
            return Some(CompactionAssessment {
                reason: "session_content_chars_threshold",
                limit_tokens: Some(compaction_limit),
                body_chars: Some(total_chars),
            });
        }

        None
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn should_compact(
        messages: &[SessionMessage],
        provider: &dyn Provider,
        model_id: &str,
        max_output_tokens: Option<u64>,
        compaction_config: &CompactionConfig,
        live_context_tokens: Option<u64>,
    ) -> bool {
        Self::assess_compaction(
            messages,
            provider,
            model_id,
            max_output_tokens,
            compaction_config,
            live_context_tokens,
            None,
            None,
        )
        .is_some()
    }

    pub(crate) fn should_force_compaction_for_reason(reason: &str) -> bool {
        matches!(
            reason,
            "usage_overflow"
                | "live_context_overflow"
                | "request_view_overflow"
                | "session_content_overflow"
                | "request_body_too_large"
        )
    }

    pub(super) async fn ensure_title(
        session: &mut Session,
        provider: Arc<dyn Provider>,
        model_id: &str,
    ) {
        let Some((_, fallback)) = compose_session_title_source(session) else {
            return;
        };

        if !session.allows_auto_title_regeneration() && session.title.trim() != fallback.trim() {
            return;
        }

        let title = generate_session_title_for_session(session, provider, model_id).await;
        if !title.trim().is_empty() {
            session.set_title(title);
        }
    }

    pub(super) fn to_message_with_parts(
        messages: &[SessionMessage],
        provider_id: &str,
        model_id: &str,
        session_directory: &str,
    ) -> Vec<MessageWithParts> {
        let legacy_tool_results: LegacyToolResultMap = messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|part| match &part.part_type {
                PartType::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                    title,
                    metadata,
                    attachments,
                } => Some((
                    tool_call_id.clone(),
                    (
                        content.clone(),
                        *is_error,
                        title.clone(),
                        metadata.clone(),
                        attachments.clone(),
                    ),
                )),
                _ => None,
            })
            .collect();

        let mut out = Vec::with_capacity(messages.len());
        let mut last_user_id = String::new();

        for msg in messages {
            if !Self::is_model_visible_message(msg) {
                continue;
            }

            let created = msg.created_at.timestamp_millis();
            let mut parts: Vec<V2Part> = msg
                .parts
                .iter()
                .filter_map(|part| match &part.part_type {
                    PartType::Text { text, .. } => Some(V2Part::Text {
                        id: part.id.clone(),
                        session_id: msg.session_id.clone(),
                        message_id: msg.id.clone(),
                        text: text.clone(),
                        synthetic: None,
                        ignored: None,
                        time: None,
                        metadata: None,
                    }),
                    PartType::File {
                        url,
                        filename,
                        mime,
                    } => Some(V2Part::File(crate::message_v2::FilePart {
                        id: part.id.clone(),
                        session_id: msg.session_id.clone(),
                        message_id: msg.id.clone(),
                        mime: mime.clone(),
                        url: url.clone(),
                        filename: Some(filename.clone()),
                        source: None,
                    })),
                    PartType::Compaction { .. } => Some(V2Part::Compaction(V2CompactionPart {
                        id: part.id.clone(),
                        session_id: msg.session_id.clone(),
                        message_id: msg.id.clone(),
                        auto: true,
                    })),
                    PartType::ToolCall {
                        id,
                        name,
                        input,
                        status,
                        raw,
                        state,
                    } => {
                        let state = state.clone().unwrap_or_else(|| {
                            Self::legacy_tool_state_to_v2(LegacyToolStateInput {
                                tool_call_id: id,
                                tool_name: name,
                                input,
                                status,
                                raw: raw.as_deref().unwrap_or_default(),
                                tool_result: legacy_tool_results.get(id),
                                session_id: &msg.session_id,
                                message_id: &msg.id,
                            })
                        });
                        Some(V2Part::Tool(crate::message_v2::ToolPart {
                            id: part.id.clone(),
                            session_id: msg.session_id.clone(),
                            message_id: msg.id.clone(),
                            call_id: id.clone(),
                            tool: name.clone(),
                            state,
                            metadata: None,
                        }))
                    }
                    _ => None,
                })
                .collect();

            if let Some(snapshot) = msg
                .metadata
                .get("step_start_snapshot")
                .or_else(|| msg.metadata.get("snapshot"))
                .and_then(|v| v.as_str())
            {
                parts.push(V2Part::StepStart(StepStartPart {
                    id: format!("prt_{}", uuid::Uuid::new_v4()),
                    session_id: msg.session_id.clone(),
                    message_id: msg.id.clone(),
                    snapshot: Some(snapshot.to_string()),
                }));
            }
            if let Some(snapshot) = msg
                .metadata
                .get("step_finish_snapshot")
                .and_then(|v| v.as_str())
            {
                let input = msg
                    .metadata
                    .get("tokens_input")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, i32::MAX as i64) as i32;
                let output = msg
                    .metadata
                    .get("tokens_output")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, i32::MAX as i64) as i32;
                parts.push(V2Part::StepFinish(StepFinishPart {
                    id: format!("prt_{}", uuid::Uuid::new_v4()),
                    session_id: msg.session_id.clone(),
                    message_id: msg.id.clone(),
                    reason: msg
                        .finish
                        .as_deref()
                        .or_else(|| msg.metadata.get("finish_reason").and_then(|v| v.as_str()))
                        .unwrap_or("stop")
                        .to_string(),
                    snapshot: Some(snapshot.to_string()),
                    cost: msg
                        .metadata
                        .get("cost")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    tokens: StepTokens {
                        total: Some(input.saturating_add(output)),
                        input,
                        output,
                        reasoning: 0,
                        cache: CacheTokens { read: 0, write: 0 },
                    },
                }));
            }

            let info = match msg.role {
                MessageRole::User => {
                    last_user_id = msg.id.clone();
                    MessageInfo::User {
                        id: msg.id.clone(),
                        session_id: msg.session_id.clone(),
                        time: UserTime { created },
                        agent: msg
                            .metadata
                            .get("agent")
                            .and_then(|v| v.as_str())
                            .unwrap_or("general")
                            .to_string(),
                        model: V2ModelRef {
                            provider_id: msg
                                .metadata
                                .get("model_provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or(provider_id)
                                .to_string(),
                            model_id: msg
                                .metadata
                                .get("model_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(model_id)
                                .to_string(),
                        },
                        format: None,
                        summary: None,
                        system: None,
                        tools: None,
                        variant: msg
                            .metadata
                            .get("variant")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    }
                }
                _ => {
                    let input = msg
                        .metadata
                        .get("tokens_input")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .clamp(0, i32::MAX as i64) as i32;
                    let output = msg
                        .metadata
                        .get("tokens_output")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .clamp(0, i32::MAX as i64) as i32;
                    MessageInfo::Assistant {
                        id: msg.id.clone(),
                        session_id: msg.session_id.clone(),
                        time: AssistantTime {
                            created,
                            completed: Some(created),
                        },
                        parent_id: if last_user_id.is_empty() {
                            msg.id.clone()
                        } else {
                            last_user_id.clone()
                        },
                        model_id: msg
                            .metadata
                            .get("model_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(model_id)
                            .to_string(),
                        provider_id: msg
                            .metadata
                            .get("model_provider")
                            .and_then(|v| v.as_str())
                            .unwrap_or(provider_id)
                            .to_string(),
                        mode: msg
                            .metadata
                            .get("mode")
                            .and_then(|v| v.as_str())
                            .unwrap_or("default")
                            .to_string(),
                        agent: msg
                            .metadata
                            .get("agent")
                            .and_then(|v| v.as_str())
                            .unwrap_or("general")
                            .to_string(),
                        path: MessagePath {
                            cwd: session_directory.to_string(),
                            root: session_directory.to_string(),
                        },
                        summary: None,
                        cost: msg
                            .metadata
                            .get("cost")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        tokens: AssistantTokens {
                            total: Some(input.saturating_add(output)),
                            input,
                            output,
                            reasoning: 0,
                            cache: CacheTokens { read: 0, write: 0 },
                        },
                        error: None,
                        structured: None,
                        variant: msg
                            .metadata
                            .get("variant")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        finish: msg.finish.clone().or_else(|| {
                            msg.metadata
                                .get("finish_reason")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        }),
                    }
                }
            };

            out.push(MessageWithParts { info, parts });
        }

        out
    }

    fn legacy_tool_state_to_v2(input_data: LegacyToolStateInput<'_>) -> crate::ToolState {
        let now = chrono::Utc::now().timestamp_millis();
        match input_data.status {
            crate::ToolCallStatus::Pending => crate::ToolState::Pending {
                input: input_data.input.clone(),
                raw: input_data.raw.to_string(),
            },
            crate::ToolCallStatus::Running => crate::ToolState::Running {
                input: input_data.input.clone(),
                title: None,
                metadata: None,
                time: crate::RunningTime { start: now },
            },
            crate::ToolCallStatus::Completed => {
                let (output, title, mut metadata, part_attachments) = input_data
                    .tool_result
                    .map(|(content, _, title, metadata, attachments)| {
                        (
                            content.clone(),
                            title
                                .clone()
                                .unwrap_or_else(|| input_data.tool_name.to_string()),
                            metadata.clone().unwrap_or_default(),
                            attachments.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            String::new(),
                            input_data.tool_name.to_string(),
                            HashMap::new(),
                            None,
                        )
                    });

                let mut merged_attachments = Vec::new();
                if let Some(values) = part_attachments {
                    merged_attachments.extend(values);
                }
                if let Some(values) = Self::take_attachment_values(&mut metadata) {
                    merged_attachments.extend(values);
                }
                let (_, normalized_attachments) = Self::normalize_tool_attachments(
                    (!merged_attachments.is_empty()).then_some(merged_attachments),
                    input_data.session_id,
                    input_data.message_id,
                );

                crate::ToolState::Completed {
                    input: input_data.input.clone(),
                    output,
                    title,
                    metadata,
                    time: crate::CompletedTime {
                        start: now,
                        end: now,
                        compacted: None,
                    },
                    attachments: normalized_attachments,
                }
            }
            crate::ToolCallStatus::Error => {
                let error = input_data
                    .tool_result
                    .map(|(content, _, _, _, _)| content.clone())
                    .unwrap_or_else(|| {
                        format!("Tool execution failed: {}", input_data.tool_call_id)
                    });
                crate::ToolState::Error {
                    input: input_data.input.clone(),
                    error,
                    metadata: None,
                    time: crate::ErrorTime {
                        start: now,
                        end: now,
                    },
                }
            }
        }
    }

    pub(super) async fn summarize_session(
        session: &mut Session,
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        provider: &dyn Provider,
    ) -> anyhow::Result<()> {
        let directory = session.directory.clone();
        let worktree = std::path::Path::new(&directory);
        let last_user = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let messages =
            Self::to_message_with_parts(&session.messages, provider_id, model_id, &directory);
        summarize_into_session(
            &SummarizeInput {
                session_id: session_id.to_string(),
                message_id: last_user,
            },
            session,
            &messages,
            worktree,
            Some(provider),
            Some(model_id),
            None,
        )
        .await?;

        Ok(())
    }

    pub(super) fn prune_after_loop(session: &mut Session, compaction_config: &CompactionConfig) {
        let mut tool_name_by_call: HashMap<String, String> = HashMap::new();
        for msg in &session.messages {
            for part in &msg.parts {
                if let PartType::ToolCall { id, name, .. } = &part.part_type {
                    tool_name_by_call.insert(id.clone(), name.clone());
                }
            }
        }

        let mut prune_messages: Vec<MessageForPrune> = session
            .messages
            .iter()
            .map(|m| {
                let parts: Vec<PruneToolPart> = m
                    .parts
                    .iter()
                    .filter_map(|p| match &p.part_type {
                        PartType::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                            ..
                        } => Some(PruneToolPart {
                            id: p.id.clone(),
                            tool: tool_name_by_call
                                .get(tool_call_id)
                                .cloned()
                                .unwrap_or_default(),
                            output: content.clone(),
                            status: if *is_error {
                                ToolPartStatus::Error
                            } else {
                                ToolPartStatus::Completed
                            },
                            compacted: None,
                        }),
                        _ => None,
                    })
                    .collect();
                MessageForPrune {
                    role: match m.role {
                        MessageRole::User => "user".to_string(),
                        _ => "assistant".to_string(),
                    },
                    parts,
                    summary: false,
                }
            })
            .collect();

        let engine = CompactionEngine::new(compaction_config.clone());
        let pruned_ids = engine.prune(&mut prune_messages);
        if pruned_ids.is_empty() {
            return;
        }
        let pruned: HashSet<String> = pruned_ids.into_iter().collect();
        for msg in session.messages_mut() {
            for part in &mut msg.parts {
                if !pruned.contains(&part.id) {
                    continue;
                }
                if let PartType::ToolResult { content, .. } = &mut part.part_type {
                    let compacted = content.chars().take(200).collect::<String>();
                    *content = format!("[tool result compacted]\n{}", compacted);
                }
            }
        }

        // Record the compacting timestamp so the session DB row reflects that pruning occurred.
        session.record_mut().time.compacting = Some(chrono::Utc::now().timestamp_millis());
        session.touch();
    }

    pub(crate) fn trigger_compaction_with_record(
        session: &mut Session,
        messages: &[SessionMessage],
        focus: Option<&str>,
        record: Option<serde_json::Value>,
        force: bool,
    ) -> Option<String> {
        let total_messages = messages.len();
        let min_messages = if force {
            FORCE_COMPACTION_MIN_MESSAGES
        } else {
            AUTO_COMPACTION_MIN_MESSAGES
        };
        if total_messages < min_messages {
            return None;
        }

        let keep_count = total_messages / 2;
        let default_summary_parts: Vec<String> = messages
            .iter()
            .take(keep_count)
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        let focus = focus.map(str::trim).filter(|value| !value.is_empty());
        let focus_terms: Vec<String> = focus
            .map(|value| {
                value
                    .split_whitespace()
                    .map(|term| term.trim().to_ascii_lowercase())
                    .filter(|term| !term.is_empty())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
        let summary_parts = if focus_terms.is_empty() {
            default_summary_parts
        } else {
            let mut focused_parts: Vec<String> = messages
                .iter()
                .take(keep_count)
                .flat_map(|message| message.parts.iter())
                .filter_map(|part| match &part.part_type {
                    PartType::Text { text, .. } => Some(text),
                    _ => None,
                })
                .filter(|text| {
                    let lowercase = text.to_ascii_lowercase();
                    focus_terms.iter().any(|term| lowercase.contains(term))
                })
                .cloned()
                .collect();
            if focused_parts.is_empty() {
                default_summary_parts
            } else {
                let existing_parts = focused_parts.clone();
                focused_parts.extend(
                    default_summary_parts
                        .into_iter()
                        .filter(|text| !existing_parts.iter().any(|existing| existing == text))
                        .take(12),
                );
                focused_parts
            }
        };

        let summary = format!(
            "Compacted {} messages.{} Summary: {}...",
            total_messages - keep_count,
            focus
                .map(|value| format!(" Focused on `{value}`."))
                .unwrap_or_default(),
            summary_parts
                .join(" ")
                .chars()
                .take(500)
                .collect::<String>()
        );

        // Persist the compaction summary as a Compaction part on a new assistant message.
        // This mirrors the TS behavior where compaction creates an assistant message with
        // summary=true and a compaction part, so that filter_compacted_messages can find it.
        let mut compaction_msg = SessionMessage::assistant(session.id.clone());
        if let Some(record) = record {
            let mut record = record;
            if let Some(object) = record.as_object_mut() {
                object
                    .entry("message_count_before".to_string())
                    .or_insert_with(|| serde_json::json!(total_messages));
                object
                    .entry("compacted_message_count".to_string())
                    .or_insert_with(|| serde_json::json!(total_messages - keep_count));
                object
                    .entry("kept_message_count".to_string())
                    .or_insert_with(|| serde_json::json!(keep_count));
                object
                    .entry("summary".to_string())
                    .or_insert_with(|| serde_json::json!(summary.clone()));
            }
            compaction_msg
                .metadata
                .insert(CONTEXT_COMPACTION_RECORD_METADATA_KEY.to_string(), record);
        }
        compaction_msg.parts.push(crate::MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Compaction {
                summary: summary.clone(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        if let Some(packet) = Self::build_compaction_continuity_packet(
            session,
            messages,
            &summary,
            &compaction_msg.id,
        ) {
            compaction_msg.metadata.insert(
                CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY.to_string(),
                packet.metadata_value(),
            );
        }
        session.messages_mut().push(compaction_msg);

        // Set the compacting timestamp on the session.
        session.record_mut().time.compacting = Some(chrono::Utc::now().timestamp_millis());
        session.touch();

        Some(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use rocode_config::{CompactionConfig as AppCompactionConfig, Config, ConfigStore};
    use rocode_orchestrator::output_projection::{
        ContextProjectionPolicy, SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY,
        SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY,
    };
    use rocode_provider::{ChatRequest, ChatResponse, ModelInfo, ProviderError, StreamResult};

    struct StaticModelProvider {
        model: Option<ModelInfo>,
    }

    impl StaticModelProvider {
        fn with_model(model_id: &str, context_window: u64, max_output_tokens: u64) -> Self {
            Self {
                model: Some(ModelInfo {
                    id: model_id.to_string(),
                    name: "Static Model".to_string(),
                    provider: "mock".to_string(),
                    context_window,
                    max_input_tokens: None,
                    max_output_tokens,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.0,
                    cost_per_million_output: 0.0,
                    cost_per_million_cache_read: None,
                    cost_per_million_cache_write: None,
                }),
            }
        }
    }

    #[async_trait]
    impl Provider for StaticModelProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            self.model.clone().into_iter().collect()
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            self.model.as_ref().filter(|model| model.id == id)
        }
        // PLACEHOLDER_TESTS_CONTINUE_1

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "chat() not used in this test".to_string(),
            ))
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[test]
    fn filter_compacted_messages_keeps_tail_after_last_compaction() {
        let session_id = "ses_test".to_string();
        let before = SessionMessage::user(session_id.clone(), "before");
        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        let after = SessionMessage::user(session_id, "after");

        let filtered = SessionPrompt::filter_compacted_messages(&[before, compact, after]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered[0]
            .parts
            .iter()
            .any(|p| matches!(p.part_type, PartType::Compaction { .. })));
    }

    #[test]
    fn filter_compacted_messages_prefers_continuity_packet_allowed_ids() {
        let session_id = "ses_test_packet_owner".to_string();
        let before = SessionMessage::user(session_id.clone(), "before");
        let user_after = SessionMessage::user(session_id.clone(), "after");
        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact_packet_owner".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        compact.metadata.insert(
            CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 2,
                "exact_recent_tail_count": 1,
                "omitted_older_turns": 1,
                "exact_recent_tail": [
                    {
                        "message_id": user_after.id,
                        "role": "user",
                        "text": "after",
                        "projected": false
                    }
                ],
                "latest_compaction_summary": {
                    "message_id": compact.id,
                    "summary": "summary"
                }
            }),
        );

        let filtered = SessionPrompt::filter_compacted_messages(&[
            before.clone(),
            compact.clone(),
            user_after.clone(),
        ]);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].id, compact.id);
        assert_eq!(filtered[1].id, user_after.id);
        assert!(!filtered.iter().any(|message| message.id == before.id));
    }

    #[test]
    fn parts_to_content_preserves_audio_file_parts() {
        let now = chrono::Utc::now();
        let content = SessionPrompt::parts_to_content(&[crate::MessagePart {
            id: "prt_audio".to_string(),
            part_type: PartType::File {
                url: "data:audio/wav;base64,UklGRg==".to_string(),
                filename: "voice.wav".to_string(),
                mime: "audio/wav".to_string(),
            },
            created_at: now,
            message_id: None,
        }]);

        let Content::Parts(parts) = content else {
            panic!("expected structured content");
        };
        assert!(matches!(
            parts.first(),
            Some(part)
                if part.content_type == "file"
                    && part.media_type.as_deref() == Some("audio/wav")
                    && part.image_url.as_ref().map(|value| value.url.as_str())
                        == Some("data:audio/wav;base64,UklGRg==")
        ));
    }

    #[test]
    fn filter_compacted_messages_preserves_latest_user_anchor_when_tail_has_no_user() {
        let session_id = "ses_test_anchor".to_string();
        let user = SessionMessage::user(session_id.clone(), "user anchor");

        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact_anchor".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });

        let assistant_after = SessionMessage::assistant(session_id);
        let filtered =
            SessionPrompt::filter_compacted_messages(&[user.clone(), compact, assistant_after]);

        assert_eq!(filtered.len(), 3);
        assert!(matches!(filtered[0].role, MessageRole::User));
        assert_eq!(filtered[0].id, user.id);
    }

    #[test]
    fn filter_compacted_messages_preserves_current_turn_assistant_tool_chain_after_user_anchor() {
        let session_id = "ses_test_turn_chain".to_string();
        let user = SessionMessage::user(session_id.clone(), "continue the same turn");

        let mut assistant_before = SessionMessage::assistant(session_id.clone());
        assistant_before.add_reasoning("need to inspect build output");
        assistant_before.add_tool_call(
            "call_1",
            "bash",
            serde_json::json!({ "command": "npm install" }),
        );

        let mut tool_after = SessionMessage::tool(session_id.clone());
        tool_after.add_tool_result("call_1", "installed", false);

        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact_turn_chain".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });

        let mut assistant_after = SessionMessage::assistant(session_id.clone());
        assistant_after.add_reasoning("now run typecheck");
        assistant_after.add_tool_call(
            "call_2",
            "bash",
            serde_json::json!({ "command": "npx tsc --noEmit" }),
        );

        let mut tool_after_compaction = SessionMessage::tool(session_id);
        tool_after_compaction.add_tool_result("call_2", "build failed", false);

        let filtered = SessionPrompt::filter_compacted_messages(&[
            user.clone(),
            assistant_before,
            tool_after,
            compact.clone(),
            assistant_after.clone(),
            tool_after_compaction.clone(),
        ]);

        assert_eq!(filtered.len(), 6);
        assert_eq!(filtered[0].id, user.id);
        assert_eq!(filtered[1].parts.len(), 2);
        assert_eq!(filtered[2].parts.len(), 1);
        assert_eq!(filtered[3].id, compact.id);
        assert_eq!(filtered[4].id, assistant_after.id);
        assert_eq!(filtered[5].id, tool_after_compaction.id);
    }

    #[test]
    fn filter_compacted_messages_preserves_current_turn_chain_when_compaction_is_latest_message() {
        let session_id = "ses_test_turn_chain_latest_compact".to_string();
        let user = SessionMessage::user(session_id.clone(), "continue the same turn");

        let mut assistant = SessionMessage::assistant(session_id.clone());
        assistant.add_reasoning("need to inspect build output");
        assistant.add_tool_call(
            "call_1",
            "bash",
            serde_json::json!({ "command": "npx tsc --noEmit" }),
        );

        let mut tool = SessionMessage::tool(session_id.clone());
        tool.add_tool_result("call_1", "build failed", false);

        let mut compact = SessionMessage::assistant(session_id);
        compact.parts.push(crate::MessagePart {
            id: "prt_compact_turn_chain_latest".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });

        let filtered = SessionPrompt::filter_compacted_messages(&[
            user.clone(),
            assistant.clone(),
            tool.clone(),
            compact.clone(),
        ]);

        assert_eq!(filtered.len(), 4);
        assert_eq!(filtered[0].id, user.id);
        assert_eq!(filtered[1].id, assistant.id);
        assert_eq!(filtered[2].id, tool.id);
        assert_eq!(filtered[3].id, compact.id);
    }

    #[test]
    fn filter_compacted_messages_falls_back_when_packet_omits_current_turn_chain() {
        let session_id = "ses_test_packet_fallback".to_string();
        let user = SessionMessage::user(session_id.clone(), "continue the same turn");

        let mut assistant_before = SessionMessage::assistant(session_id.clone());
        assistant_before.add_reasoning("need to inspect build output");
        assistant_before.add_tool_call(
            "call_1",
            "bash",
            serde_json::json!({ "command": "npm install" }),
        );

        let mut tool_after = SessionMessage::tool(session_id.clone());
        tool_after.add_tool_result("call_1", "installed", false);

        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact_packet_fallback".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        compact.metadata.insert(
            CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 2,
                "exact_recent_tail_count": 1,
                "omitted_older_turns": 1,
                "exact_recent_tail": [
                    {
                        "message_id": user.id,
                        "role": "user",
                        "text": "continue the same turn",
                        "projected": false
                    }
                ],
                "latest_compaction_summary": {
                    "message_id": compact.id,
                    "summary": "summary"
                }
            }),
        );

        let mut assistant_after = SessionMessage::assistant(session_id.clone());
        assistant_after.add_reasoning("now run typecheck");
        assistant_after.add_tool_call(
            "call_2",
            "bash",
            serde_json::json!({ "command": "npx tsc --noEmit" }),
        );

        let mut tool_after_compaction = SessionMessage::tool(session_id);
        tool_after_compaction.add_tool_result("call_2", "build failed", false);

        let filtered = SessionPrompt::filter_compacted_messages(&[
            user.clone(),
            assistant_before,
            tool_after,
            compact.clone(),
            assistant_after.clone(),
            tool_after_compaction.clone(),
        ]);

        assert_eq!(filtered.len(), 6);
        assert_eq!(filtered[0].id, user.id);
        assert_eq!(filtered[3].id, compact.id);
        assert_eq!(filtered[4].id, assistant_after.id);
        assert_eq!(filtered[5].id, tool_after_compaction.id);
    }

    #[test]
    fn filter_compacted_messages_packet_preserves_declared_continuation_dependencies() {
        let session_id = "ses_test_packet_continuation_dependency".to_string();
        let user = SessionMessage::user(session_id.clone(), "continue the same turn");

        let mut assistant_before = SessionMessage::assistant(session_id.clone());
        assistant_before.add_reasoning("inspect previous tool output");
        assistant_before.add_tool_call(
            "call_1",
            "bash",
            serde_json::json!({ "command": "npm install" }),
        );

        let mut tool_after = SessionMessage::tool(session_id.clone());
        tool_after.add_tool_result("call_1", "installed", false);

        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact_packet_continuation_dependency".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        compact.metadata.insert(
            CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY.to_string(),
            SessionContinuityPacket {
                eligible_message_count: 2,
                exact_recent_tail_count: 1,
                omitted_older_turns: 1,
                exact_recent_tail: vec![SessionContinuityTurn {
                    message_id: user.id.clone(),
                    role: "user".to_string(),
                    text: "continue the same turn".to_string(),
                    projected: false,
                }],
                continuation_dependencies: vec![SessionContinuityDependency {
                    kind: SessionContinuityDependencyKind::AssistantToolCallContinuation,
                    anchor_message_id: Some(user.id.clone()),
                    message_ids: vec![
                        user.id.clone(),
                        assistant_before.id.clone(),
                        tool_after.id.clone(),
                    ],
                }],
                latest_compaction_summary: Some(SessionContinuityCompactionSummary {
                    message_id: compact.id.clone(),
                    summary: "summary".to_string(),
                }),
                ..SessionContinuityPacket::default()
            }
            .metadata_value(),
        );

        let user_after = SessionMessage::user(session_id, "follow up after compaction");

        let filtered = SessionPrompt::filter_compacted_messages(&[
            user.clone(),
            assistant_before.clone(),
            tool_after.clone(),
            compact.clone(),
            user_after.clone(),
        ]);

        assert_eq!(filtered.len(), 5);
        assert_eq!(filtered[0].id, user.id);
        assert_eq!(filtered[1].id, assistant_before.id);
        assert_eq!(filtered[2].id, tool_after.id);
        assert_eq!(filtered[3].id, compact.id);
        assert_eq!(filtered[4].id, user_after.id);
    }

    #[test]
    fn runtime_compaction_config_reads_store_values() {
        let config = Config {
            compaction: Some(AppCompactionConfig {
                auto: Some(false),
                prune: Some(false),
                reserved: Some(2048),
            }),
            ..Default::default()
        };
        let store = ConfigStore::new(config);

        let resolved = SessionPrompt::runtime_compaction_config(Some(&store));
        assert!(!resolved.auto);
        assert!(!resolved.prune);
        assert_eq!(resolved.reserved, Some(2048));
    }

    #[test]
    fn prune_after_loop_compacts_large_old_tool_results() {
        let mut session = Session::new("proj", ".");
        let session_id = session.id.clone();

        session
            .messages_mut()
            .push(SessionMessage::user(session_id.clone(), "old user message"));

        let mut old_assistant = SessionMessage::assistant(session_id.clone());
        old_assistant.add_tool_call("call_a", "bash", serde_json::json!({"command": "echo a"}));
        old_assistant.add_tool_result("call_a", "A".repeat(140_000), false);
        old_assistant.add_tool_call("call_b", "bash", serde_json::json!({"command": "echo b"}));
        old_assistant.add_tool_result("call_b", "B".repeat(140_000), false);
        session.messages_mut().push(old_assistant);
        // PLACEHOLDER_TESTS_CONTINUE_2

        session
            .messages_mut()
            .push(SessionMessage::user(session_id.clone(), "new user one"));
        session
            .messages_mut()
            .push(SessionMessage::assistant(session_id.clone()));
        session
            .messages_mut()
            .push(SessionMessage::user(session_id.clone(), "new user two"));
        session
            .messages_mut()
            .push(SessionMessage::assistant(session_id));

        SessionPrompt::prune_after_loop(&mut session, &CompactionConfig::default());

        let compacted_count = session
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::ToolResult { content, .. } => Some(content),
                _ => None,
            })
            .filter(|c| c.starts_with("[tool result compacted]"))
            .count();

        assert!(
            compacted_count >= 1,
            "expected at least one tool result to be compacted"
        );
    }

    #[test]
    fn should_compact_prefers_provider_model_limits() {
        let provider = StaticModelProvider::with_model("tiny-model", 1000, 100);
        let mut msg = SessionMessage::user("ses_test", "hello");
        msg.metadata
            .insert("tokens_input".to_string(), serde_json::json!(950_u64));

        let compact = SessionPrompt::should_compact(
            &[msg],
            &provider,
            "tiny-model",
            None,
            &CompactionConfig::default(),
            None,
        );
        assert!(compact);
    }

    #[test]
    fn should_compact_counts_tool_results() {
        let provider = StaticModelProvider::with_model("big-model", 1_000_000, 65536);
        let mut msg = SessionMessage::assistant("ses_test");
        let large_content = "x".repeat(5_100_000);
        msg.parts.push(crate::MessagePart {
            id: "part_1".to_string(),
            part_type: PartType::ToolResult {
                tool_call_id: "tc_1".to_string(),
                content: large_content,
                is_error: false,
                title: None,
                metadata: None,
                attachments: None,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        // PLACEHOLDER_TESTS_CONTINUE_3

        let compact = SessionPrompt::should_compact(
            &[msg],
            &provider,
            "big-model",
            None,
            &CompactionConfig::default(),
            None,
        );
        assert!(
            compact,
            "should trigger compaction for >5MB tool result content"
        );
    }

    #[test]
    fn should_compact_uses_max_input_tokens() {
        let provider = StaticModelProvider {
            model: Some(ModelInfo {
                id: "limited-model".to_string(),
                name: "Limited Model".to_string(),
                provider: "mock".to_string(),
                context_window: 1_000_000,
                max_input_tokens: Some(50_000),
                max_output_tokens: 8192,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
                cost_per_million_cache_read: None,
                cost_per_million_cache_write: None,
            }),
        };
        let mut msg = SessionMessage::user("ses_test", "hello");
        msg.metadata
            .insert("tokens_input".to_string(), serde_json::json!(48_000_u64));

        let compact = SessionPrompt::should_compact(
            &[msg],
            &provider,
            "limited-model",
            None,
            &CompactionConfig::default(),
            None,
        );
        assert!(
            compact,
            "should trigger proactive compaction when input tokens approach the budget"
        );

        let live_only = SessionMessage::user("ses_test", "small message");
        let compact = SessionPrompt::should_compact(
            &[live_only],
            &provider,
            "limited-model",
            None,
            &CompactionConfig::default(),
            Some(48_000),
        );
        assert!(
            compact,
            "should trigger proactive compaction from live context telemetry"
        );
    }

    #[test]
    fn should_compact_respects_disabled_auto_compaction() {
        let provider = StaticModelProvider::with_model("tiny-model", 1000, 100);
        let mut msg = SessionMessage::user("ses_test", "hello");
        msg.metadata
            .insert("tokens_input".to_string(), serde_json::json!(950_u64));

        let compact = SessionPrompt::should_compact(
            &[msg],
            &provider,
            "tiny-model",
            None,
            &CompactionConfig {
                auto: false,
                reserved: None,
                prune: true,
            },
            None,
        );
        assert!(
            !compact,
            "should not trigger automatic compaction when auto=false"
        );
    }

    #[test]
    fn should_compact_respects_reserved_token_budget() {
        let provider = StaticModelProvider::with_model("reserved-model", 10_000, 2048);
        let mut msg = SessionMessage::user("ses_test", "hello");
        msg.metadata
            .insert("tokens_input".to_string(), serde_json::json!(9_300_u64));

        let compact = SessionPrompt::should_compact(
            &[msg],
            &provider,
            "reserved-model",
            Some(800),
            &CompactionConfig {
                auto: true,
                reserved: Some(1_000),
                prune: true,
            },
            None,
        );
        assert!(
            compact,
            "should trigger compaction when reserved token budget shrinks usable input"
        );
    }

    #[test]
    fn should_compact_estimates_current_input_without_usage_metadata() {
        let provider = StaticModelProvider::with_model("estimated-model", 2_000, 256);
        let mut msg = SessionMessage::user("ses_test", "");
        msg.parts.clear();
        msg.parts.push(crate::MessagePart {
            id: "part_estimated".to_string(),
            part_type: PartType::Text {
                text: "x".repeat(7_500),
                synthetic: None,
                ignored: None,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });

        let compact = SessionPrompt::should_compact(
            &[msg],
            &provider,
            "estimated-model",
            None,
            &CompactionConfig::default(),
            None,
        );
        assert!(
            compact,
            "should trigger compaction from current message content even without usage metadata"
        );
    }

    #[test]
    fn should_compact_backs_off_after_recent_compactions() {
        let provider = StaticModelProvider::with_model("estimated-model", 2_000, 256);
        let session_id = "ses_test".to_string();

        let mut compacted = SessionMessage::assistant(session_id.clone());
        compacted.parts.push(crate::MessagePart {
            id: "part_compacted".to_string(),
            part_type: PartType::Compaction {
                summary: "Earlier turns summarized".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });

        let mut msg = SessionMessage::user(session_id, "");
        msg.parts.clear();
        msg.parts.push(crate::MessagePart {
            id: "part_estimated".to_string(),
            part_type: PartType::Text {
                text: "x".repeat(7_500),
                synthetic: None,
                ignored: None,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });

        let compact = SessionPrompt::should_compact(
            &[compacted, msg],
            &provider,
            "estimated-model",
            None,
            &CompactionConfig::default(),
            None,
        );
        assert!(
            !compact,
            "should back off auto-compaction when the last compaction was too recent"
        );
    }

    #[test]
    fn trigger_compaction_mentions_focus_topic() {
        let mut session = Session::new("proj", ".");
        let session_id = session.id.clone();

        let messages: Vec<SessionMessage> = (0..10)
            .map(|index| {
                let text = if index % 2 == 0 {
                    format!("xterm terminal integration note {index}")
                } else {
                    format!("other note {index}")
                };
                SessionMessage::user(session_id.clone(), text)
            })
            .collect();

        let summary = SessionPrompt::trigger_compaction_with_record(
            &mut session,
            &messages,
            Some("xterm"),
            None,
            false,
        )
        .expect("focused compaction should produce a summary");
        assert!(summary.contains("Focused on `xterm`."));
        assert!(summary.to_ascii_lowercase().contains("xterm"));
    }

    #[test]
    fn forced_compaction_bypasses_auto_minimum_and_records_diagnostics() {
        let mut session = Session::new("proj", ".");
        session.record_mut().title = "Voicecraft build session".to_string();
        let session_id = session.id.clone();
        let mut assistant = SessionMessage::assistant(session_id.clone());
        assistant.add_text("Implemented the initial world bootstrap and wrote src/game/World.ts");
        let messages = vec![
            SessionMessage::user(session_id.clone(), "first"),
            assistant,
            SessionMessage::user(session_id, "second"),
        ];
        let record = SessionPrompt::build_compaction_record(
            "overflow_recovery",
            Some("prompt.provider_overflow"),
            Some("provider_overflow"),
            true,
            Some(120_000),
            None,
            Some(100_000),
            Some(480_000),
        );

        let summary = SessionPrompt::trigger_compaction_with_record(
            &mut session,
            &messages,
            None,
            Some(record),
            true,
        )
        .expect("forced compaction should not require 10 messages");

        let compaction_message = session
            .record()
            .messages
            .last()
            .expect("compaction message should be appended");
        let diagnostics = compaction_message
            .metadata
            .get(CONTEXT_COMPACTION_RECORD_METADATA_KEY)
            .expect("diagnostics metadata should exist");

        assert_eq!(
            diagnostics["trigger"],
            serde_json::json!("overflow_recovery")
        );
        assert_eq!(diagnostics["forced"], serde_json::json!(true));
        assert_eq!(diagnostics["compacted_message_count"], serde_json::json!(2));
        assert_eq!(diagnostics["kept_message_count"], serde_json::json!(1));
        assert_eq!(diagnostics["message_count_before"], serde_json::json!(3));
        assert_eq!(diagnostics["summary"], serde_json::json!(summary));

        let packet = compaction_message
            .metadata
            .get(CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY)
            .and_then(SessionContinuityPacket::from_value)
            .expect("continuity packet metadata should exist");
        assert_eq!(packet.version, 1);
        assert_eq!(packet.exact_recent_tail_count, 3);
        assert_eq!(packet.eligible_message_count, 3);
        assert!(packet
            .working_ledger
            .iter()
            .any(|entry| entry.kind == SessionContinuityLedgerKind::SessionTitle));
        assert!(packet
            .working_ledger
            .iter()
            .any(|entry| entry.kind == SessionContinuityLedgerKind::LatestUserTurn));
        assert_eq!(
            packet
                .latest_compaction_summary
                .as_ref()
                .map(|item| item.message_id.as_str()),
            Some(compaction_message.id.as_str())
        );
        assert_eq!(
            packet
                .latest_compaction_summary
                .as_ref()
                .map(|item| item.summary.as_str()),
            Some(summary.as_str())
        );
        assert!(packet.continuation_dependencies.is_empty());
    }

    #[test]
    fn forced_compaction_records_continuation_dependency_for_assistant_tool_chain() {
        let mut session = Session::new("proj", ".");
        let session_id = session.id.clone();

        let user = SessionMessage::user(session_id.clone(), "continue the same turn");
        let mut assistant = SessionMessage::assistant(session_id.clone());
        assistant.add_reasoning("need to inspect build output");
        assistant.add_tool_call(
            "call_1",
            "bash",
            serde_json::json!({ "command": "npx tsc --noEmit" }),
        );

        let mut tool = SessionMessage::tool(session_id);
        tool.add_tool_result("call_1", "build failed", false);

        let messages = vec![user.clone(), assistant.clone(), tool.clone()];
        let _summary = SessionPrompt::trigger_compaction_with_record(
            &mut session,
            &messages,
            None,
            None,
            true,
        )
        .expect("forced compaction should produce a summary");

        let compaction_message = session
            .record()
            .messages
            .last()
            .expect("compaction message should be appended");
        let packet = compaction_message
            .metadata
            .get(CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY)
            .and_then(SessionContinuityPacket::from_value)
            .expect("continuity packet metadata should exist");

        assert_eq!(packet.continuation_dependencies.len(), 1);
        let dependency = &packet.continuation_dependencies[0];
        assert_eq!(
            dependency.kind,
            SessionContinuityDependencyKind::AssistantToolCallContinuation
        );
        assert_eq!(
            dependency.anchor_message_id.as_deref(),
            Some(user.id.as_str())
        );
        assert_eq!(
            dependency.message_ids,
            vec![user.id.clone(), assistant.id.clone(), tool.id.clone()]
        );
        assert!(packet
            .allowed_message_ids()
            .contains(&compaction_message.id));
    }

    #[test]
    fn continuity_allowed_message_ids_excludes_projected_tail_turns() {
        let packet = SessionContinuityPacket {
            exact_recent_tail: vec![
                SessionContinuityTurn {
                    message_id: "msg_user".to_string(),
                    role: "user".to_string(),
                    text: "latest question".to_string(),
                    projected: false,
                },
                SessionContinuityTurn {
                    message_id: "msg_projected_assistant".to_string(),
                    role: "assistant".to_string(),
                    text: "projected assistant output".to_string(),
                    projected: true,
                },
            ],
            continuation_dependencies: vec![SessionContinuityDependency {
                kind: SessionContinuityDependencyKind::AssistantToolCallContinuation,
                anchor_message_id: Some("msg_user".to_string()),
                message_ids: vec![
                    "msg_user".to_string(),
                    "msg_assistant_tool".to_string(),
                    "msg_tool_result".to_string(),
                ],
            }],
            latest_compaction_summary: Some(SessionContinuityCompactionSummary {
                message_id: "msg_compact".to_string(),
                summary: "summary".to_string(),
            }),
            ..SessionContinuityPacket::default()
        };

        let allowed = packet.allowed_message_ids();

        assert!(allowed.contains(&"msg_user".to_string()));
        assert!(allowed.contains(&"msg_assistant_tool".to_string()));
        assert!(allowed.contains(&"msg_tool_result".to_string()));
        assert!(allowed.contains(&"msg_compact".to_string()));
        assert!(!allowed.contains(&"msg_projected_assistant".to_string()));
    }

    #[test]
    fn assess_compaction_prefers_request_view_thresholds() {
        let provider = StaticModelProvider {
            model: Some(ModelInfo {
                id: "request-view-model".to_string(),
                name: "Request View Model".to_string(),
                provider: "mock".to_string(),
                context_window: 1_000_000,
                max_input_tokens: Some(50_000),
                max_output_tokens: 8_192,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
                cost_per_million_cache_read: None,
                cost_per_million_cache_write: None,
            }),
        };
        let messages = vec![SessionMessage::user("ses_test", "small user message")];

        let assessment = SessionPrompt::assess_compaction(
            &messages,
            &provider,
            "request-view-model",
            None,
            &CompactionConfig::default(),
            None,
            Some(38_000),
            Some(152_000),
        )
        .expect("request view should trigger proactive compaction");

        assert_eq!(assessment.reason, "request_view_threshold");
        assert_eq!(assessment.body_chars, Some(152_000));
    }

    #[test]
    fn prune_after_loop_respects_disabled_prune() {
        let mut session = Session::new("proj", ".");
        let session_id = session.id.clone();

        session
            .messages_mut()
            .push(SessionMessage::user(session_id.clone(), "old user message"));

        let mut old_assistant = SessionMessage::assistant(session_id.clone());
        old_assistant.add_tool_call("call_a", "bash", serde_json::json!({"command": "echo a"}));
        old_assistant.add_tool_result("call_a", "A".repeat(140_000), false);
        old_assistant.add_tool_call("call_b", "bash", serde_json::json!({"command": "echo b"}));
        old_assistant.add_tool_result("call_b", "B".repeat(140_000), false);
        session.messages_mut().push(old_assistant);

        session
            .messages_mut()
            .push(SessionMessage::user(session_id.clone(), "new user one"));
        session
            .messages_mut()
            .push(SessionMessage::assistant(session_id.clone()));
        session
            .messages_mut()
            .push(SessionMessage::user(session_id.clone(), "new user two"));
        session
            .messages_mut()
            .push(SessionMessage::assistant(session_id));

        SessionPrompt::prune_after_loop(
            &mut session,
            &CompactionConfig {
                auto: true,
                reserved: None,
                prune: false,
            },
        );

        let compacted_count = session
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::ToolResult { content, .. } => Some(content),
                _ => None,
            })
            .filter(|c| c.starts_with("[tool result compacted]"))
            .count();

        assert_eq!(
            compacted_count, 0,
            "expected prune=false to preserve tool results"
        );
    }

    #[test]
    fn token_usage_from_messages_prefers_usage_field_over_metadata() {
        let mut msg = SessionMessage::assistant("ses_test");
        msg.metadata
            .insert("tokens_input".to_string(), serde_json::json!(1_u64));
        msg.metadata
            .insert("tokens_output".to_string(), serde_json::json!(2_u64));
        msg.metadata
            .insert("tokens_cache_read".to_string(), serde_json::json!(3_u64));
        msg.metadata
            .insert("tokens_cache_write".to_string(), serde_json::json!(4_u64));
        msg.usage = Some(crate::message::MessageUsage {
            input_tokens: 100,
            output_tokens: 200,
            reasoning_tokens: 50,
            cache_read_tokens: 30,
            cache_miss_tokens: 0,
            cache_write_tokens: 20,
            context_tokens: 100,
            total_cost: 0.0,
        });

        let usage = SessionPrompt::token_usage_from_messages(&[msg]);
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 200);
        assert_eq!(usage.cache_read, 30);
        assert_eq!(usage.cache_write, 20);
        assert_eq!(usage.total, 350);
    }
    // PLACEHOLDER_TESTS_CONTINUE_4

    #[test]
    fn token_usage_from_messages_falls_back_to_usage_metadata_object() {
        let mut msg = SessionMessage::assistant("ses_test");
        msg.metadata.insert(
            "usage".to_string(),
            serde_json::json!({
                "prompt_tokens": 77_u64,
                "completion_tokens": 33_u64,
                "reasoning_tokens": 11_u64,
                "cache_read_tokens": 5_u64,
                "cache_write_tokens": 2_u64
            }),
        );

        let usage = SessionPrompt::token_usage_from_messages(&[msg]);
        assert_eq!(usage.input, 77);
        assert_eq!(usage.output, 33);
        assert_eq!(usage.cache_read, 5);
        assert_eq!(usage.cache_write, 2);
        assert_eq!(usage.total, 117);
    }

    #[test]
    fn build_chat_messages_splits_legacy_assistant_tool_results() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("working");
        assistant.add_tool_result("call_1", "ok", false);

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].role, Role::Assistant));
        assert!(matches!(messages[1].role, Role::Tool));
    }

    #[test]
    fn build_chat_messages_uses_scheduler_model_context_projection_for_assistant_text() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("very long visible scheduler delivery");
        assistant.metadata.insert(
            SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
            serde_json::to_value(ContextProjectionPolicy::OnDemandArtifact)
                .expect("policy should serialize"),
        );
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("compact scheduler summary with artifact reference"),
        );

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, Role::Assistant));
        assert!(matches!(
            &messages[0].content,
            Content::Text(text) if text == "compact scheduler summary with artifact reference"
        ));
    }

    #[test]
    fn build_chat_messages_keeps_legacy_projection_without_policy() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("legacy large assistant text");
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("legacy summary"),
        );

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, Role::Assistant));
        assert!(matches!(
            &messages[0].content,
            Content::Text(text) if text == "legacy summary"
        ));
    }

    #[test]
    fn build_chat_messages_rejects_unsanctioned_full_projection_policy() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("full output should stay visible");
        assistant.metadata.insert(
            SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
            serde_json::to_value(ContextProjectionPolicy::Full).expect("policy should serialize"),
        );
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("must not override visible text"),
        );

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, Role::Assistant));
        assert!(matches!(
            &messages[0].content,
            Content::Text(text) if text == "full output should stay visible"
        ));
    }

    #[test]
    fn build_chat_messages_preserves_user_text_even_when_projection_metadata_exists() {
        let sid = "sid".to_string();
        let mut user = SessionMessage::user(sid, "exact user instruction");
        user.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("should not replace user intent"),
        );

        let messages = SessionPrompt::build_chat_messages(&[user], None).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, Role::User));
        assert!(matches!(
            &messages[0].content,
            Content::Text(text) if text == "exact user instruction"
        ));
    }

    #[test]
    fn build_chat_messages_ignores_provider_diagnostic_metadata() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("visible answer");
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-v4".to_string()),
            message: "thinking replay rejected".to_string(),
        }
        .attach_to_metadata(&mut assistant.metadata);

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, Role::Assistant));
        assert!(matches!(
            &messages[0].content,
            Content::Text(text) if text == "visible answer"
        ));
    }

    #[test]
    fn build_chat_messages_ignores_tool_preflight_metadata() {
        let sid = "sid".to_string();
        let mut tool = SessionMessage::tool(sid);
        tool.add_tool_result("call_1", "read ok", false);
        let mut metadata = HashMap::new();
        metadata.insert(
            rocode_tool::EXECUTION_PREFLIGHT_METADATA_KEY.to_string(),
            serde_json::to_value(rocode_tool::ExecutionPreflightMetadata {
                runner: "read".to_string(),
                subject: "/tmp/report.md".to_string(),
                status: rocode_tool::ExecutionPreflightStatus::SoftWarn,
                issues: vec![rocode_tool::ExecutionPreflightIssue {
                    severity: rocode_tool::ExecutionPreflightSeverity::SoftWarn,
                    code: "missing_context".to_string(),
                    message: "context snapshot was partial".to_string(),
                }],
                output: String::new(),
                metadata: HashMap::new(),
                attachment_count: 0,
            })
            .expect("preflight metadata should serialize"),
        );
        match &mut tool.parts[0].part_type {
            PartType::ToolResult {
                metadata: part_metadata,
                ..
            } => {
                *part_metadata = Some(metadata);
            }
            other => panic!("expected tool result part, got {other:?}"),
        }

        let messages = SessionPrompt::build_chat_messages(&[tool], None).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0].role, Role::Tool));
        match &messages[0].content {
            Content::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].content_type, "tool_result");
                let tool_result = parts[0].tool_result.as_ref().expect("tool result content");
                assert_eq!(tool_result.content, "read ok");
            }
            other => panic!("expected tool content, got {other:?}"),
        }
    }

    #[test]
    fn build_chat_messages_does_not_project_tool_protocol_rounds() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("checking workspace");
        assistant.add_tool_call("tool-call-0", "ls", serde_json::json!({"path": "."}));
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("summary must not replace tool call"),
        );

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();

        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            Content::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0].text.as_deref(), Some("checking workspace"));
                assert_eq!(parts[1].content_type, "tool_use");
            }
            other => panic!("expected parts content, got {other:?}"),
        }
    }

    #[test]
    fn model_context_char_len_uses_projection_summary_for_large_assistant_output() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_text("x".repeat(10_000));
        assistant.metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!("short summary"),
        );

        assert_eq!(
            SessionPrompt::model_context_char_len(&assistant),
            "short summary".len()
        );
    }

    #[test]
    fn build_chat_messages_preserves_reasoning_parts() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_reasoning("internal trace");
        assistant.add_text("visible answer");

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();
        assert_eq!(messages.len(), 1);

        match &messages[0].content {
            Content::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0].content_type, "reasoning");
                assert_eq!(parts[0].text.as_deref(), Some("internal trace"));
                assert_eq!(parts[1].content_type, "text");
                assert_eq!(parts[1].text.as_deref(), Some("visible answer"));
            }
            other => panic!("expected parts content, got {other:?}"),
        }
    }

    #[test]
    fn build_chat_messages_preserves_reasoning_alongside_tool_calls() {
        let sid = "sid".to_string();
        let mut assistant = SessionMessage::assistant(sid);
        assistant.add_reasoning("reasoning before tool");
        assistant.add_text("checking workspace");
        assistant.add_tool_call(
            "tool-call-0",
            "ls",
            serde_json::json!({ "path": "/tmp/workspace" }),
        );

        let messages = SessionPrompt::build_chat_messages(&[assistant], None).unwrap();
        assert_eq!(messages.len(), 1);

        match &messages[0].content {
            Content::Parts(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0].content_type, "reasoning");
                assert_eq!(parts[0].text.as_deref(), Some("reasoning before tool"));
                assert_eq!(parts[1].content_type, "text");
                assert_eq!(parts[1].text.as_deref(), Some("checking workspace"));
                assert_eq!(parts[2].content_type, "tool_use");
                assert_eq!(
                    parts[2]
                        .tool_use
                        .as_ref()
                        .map(|tool_use| tool_use.id.as_str()),
                    Some("tool-call-0")
                );
            }
            other => panic!("expected parts content, got {other:?}"),
        }
    }

    #[test]
    fn legacy_tool_state_to_v2_recovers_attachments_from_tool_result_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "attachment".to_string(),
            serde_json::json!({ "mime": "application/pdf", "url": "data:application/pdf;base64,AA==" }),
        );
        metadata.insert(
            "preview".to_string(),
            serde_json::json!("PDF read successfully"),
        );

        let tool_result = (
            "PDF read successfully".to_string(),
            false,
            Some("Read".to_string()),
            Some(metadata),
            None,
        );

        let input = serde_json::json!({ "file_path": "report.pdf" });
        let state = SessionPrompt::legacy_tool_state_to_v2(LegacyToolStateInput {
            tool_call_id: "tool-call-1",
            tool_name: "read",
            input: &input,
            status: &crate::ToolCallStatus::Completed,
            raw: "",
            tool_result: Some(&tool_result),
            session_id: "ses_1",
            message_id: "msg_1",
        });

        match state {
            crate::ToolState::Completed {
                metadata,
                attachments,
                ..
            } => {
                assert!(!metadata.contains_key("attachment"));
                assert_eq!(attachments.as_ref().map(|v| v.len()), Some(1));
                assert_eq!(
                    attachments
                        .as_ref()
                        .and_then(|v| v.first())
                        .map(|f| f.mime.as_str()),
                    Some("application/pdf")
                );
            }
            _ => panic!("expected completed state"),
        }
    }
}
