// Message building/conversion/compaction methods for SessionPrompt

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rocode_orchestrator::output_projection::SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY;
use rocode_provider::{get_model_context_limit, Content, ContentPart, Message, Provider, Role};

use crate::compaction::{
    CompactionConfig, CompactionEngine, MessageForPrune, ModelLimits, PruneToolPart, TokenUsage,
    ToolPartStatus,
};
use crate::message_v2::{
    AssistantTime, AssistantTokens, CacheTokens, CompactionPart as V2CompactionPart, MessageInfo,
    MessagePath, MessageWithParts, ModelRef as V2ModelRef, Part as V2Part, StepFinishPart,
    StepStartPart, StepTokens, UserTime,
};
use crate::summary::{summarize_into_session, SummarizeInput};
use crate::{MessageRole, PartType, Session, SessionMessage};

use super::tools_and_output::{compose_session_title_source, generate_session_title_for_session};
use super::SessionPrompt;

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
    fn model_hidden_runtime_hint(message: &SessionMessage) -> Option<&str> {
        match message
            .metadata
            .get("runtime_hint")
            .and_then(|value| value.as_str())
        {
            Some("proposal_notice") => Some("proposal_notice"),
            Some("skill_save_suggestion") => Some("skill_save_suggestion"),
            _ => None,
        }
    }

    fn is_model_visible_message(message: &SessionMessage) -> bool {
        Self::model_hidden_runtime_hint(message).is_none()
    }

    pub(super) fn runtime_compaction_config(
        config_store: Option<&rocode_config::ConfigStore>,
    ) -> CompactionConfig {
        let mut config = CompactionConfig::default();
        let Some(store) = config_store else {
            return config;
        };

        if let Some(compaction) = store.config().compaction.as_ref() {
            if let Some(auto) = compaction.auto {
                config.auto = auto;
            }
            if let Some(prune) = compaction.prune {
                config.prune = prune;
            }
            if let Some(reserved) = compaction.reserved {
                config.reserved = Some(reserved);
            }
        }

        config
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
        if !matches!(msg.role, MessageRole::Assistant) {
            return None;
        }
        if Self::contains_tool_protocol_parts(&msg.parts) {
            return None;
        }
        msg.metadata
            .get(SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(ToOwned::to_owned)
    }

    fn contains_tool_protocol_parts(parts: &[crate::MessagePart]) -> bool {
        parts.iter().any(|part| {
            matches!(
                part.part_type,
                PartType::ToolCall { .. } | PartType::ToolResult { .. }
            )
        })
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
        let start = messages
            .iter()
            .rposition(|m| {
                m.parts
                    .iter()
                    .any(|p| matches!(p.part_type, PartType::Compaction { .. }))
            })
            .unwrap_or(0);
        let tail = messages[start..].to_vec();
        if tail.iter().any(|m| matches!(m.role, MessageRole::User)) {
            return tail;
        }

        // Keep the latest user anchor before the compaction boundary so prompt
        // loop invariants hold (`last_user_idx` must exist).
        if let Some(last_user_idx) = messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::User))
        {
            if last_user_idx < start {
                let mut anchored = Vec::with_capacity(messages.len() - last_user_idx);
                anchored.push(messages[last_user_idx].clone());
                anchored.extend_from_slice(&messages[start..]);
                return anchored;
            }
        }

        tail
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

    pub(super) fn should_compact(
        messages: &[SessionMessage],
        provider: &dyn Provider,
        model_id: &str,
        max_output_tokens: Option<u64>,
        compaction_config: &CompactionConfig,
        live_context_tokens: Option<u64>,
    ) -> bool {
        if Self::should_back_off_auto_compaction(messages) {
            return false;
        }
        if !compaction_config.auto {
            return false;
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
        if engine.is_overflow(&usage, &limits) {
            return true;
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
                return true;
            }
        }
        if usage_count > 0
            && Self::should_trigger_proactive_compaction(usage_count, &limits, compaction_config)
        {
            return true;
        }
        if let Some(live_context_tokens) = live_context_tokens {
            if Self::should_trigger_proactive_compaction(
                live_context_tokens,
                &limits,
                compaction_config,
            ) {
                return true;
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
            return true;
        }
        if estimated_input_tokens > 0
            && Self::should_trigger_proactive_compaction(
                estimated_input_tokens,
                &limits,
                compaction_config,
            )
        {
            return true;
        }

        // Hard cap: 5MB of content to stay under typical 6MB API body limits
        // (leaves ~1MB for JSON overhead, tool definitions, system prompt).
        const MAX_BODY_CHARS: usize = 5_000_000;
        if total_chars > MAX_BODY_CHARS {
            return true;
        }

        // Softer cap based on estimated token count.
        const MAX_CONTEXT_CHARS: usize = 200_000;
        total_chars > MAX_CONTEXT_CHARS
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

    pub(super) fn trigger_compaction(
        session: &mut Session,
        messages: &[SessionMessage],
        focus: Option<&str>,
    ) -> Option<String> {
        let total_messages = messages.len();
        if total_messages < 10 {
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
        compaction_msg.parts.push(crate::MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Compaction {
                summary: summary.clone(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
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

        let summary = SessionPrompt::trigger_compaction(&mut session, &messages, Some("xterm"))
            .expect("focused compaction should produce a summary");
        assert!(summary.contains("Focused on `xterm`."));
        assert!(summary.to_ascii_lowercase().contains("xterm"));
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
