use agendao_command_render::output_blocks::SchedulerStageBlock;
use agendao_command_render::terminal_tool_block_display::{
    build_file_items, build_image_items, summarize_block_items_inline,
};
#[cfg(feature = "multimodal")]
use agendao_multimodal::PersistedMultimodalExplain;
use agendao_stage_protocol::{StageStatus, StageSummary};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(not(feature = "multimodal"))]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersistedMultimodalExplain {
    #[serde(default)]
    pub attachment_count: usize,
    #[serde(default)]
    pub unsupported_parts: Vec<String>,
    #[serde(default)]
    pub recommended_downgrade: Option<String>,
}

#[cfg(not(feature = "multimodal"))]
impl PersistedMultimodalExplain {
    pub fn summary_line(&self) -> String {
        format!("{} attachment(s)", self.attachment_count)
    }

    pub fn combined_warnings(&self) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub parent_id: Option<String>,
    pub share: Option<ShareInfo>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareInfo {
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub finish: Option<String>,
    pub error: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cost: f64,
    pub tokens: TokenUsage,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub multimodal: Option<PersistedMultimodalExplain>,
    pub parts: Vec<MessagePart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    #[serde(default)]
    pub cache_miss: u64,
    pub cache_write: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MessagePart {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    File {
        path: String,
        mime: String,
    },
    Image {
        url: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        result: String,
        is_error: bool,
        title: Option<String>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct SessionContext {
    pub sessions: HashMap<String, Session>,
    pub messages: HashMap<String, Vec<Message>>,
    pub message_index: HashMap<String, HashMap<String, usize>>,
    // P0-4: Legacy fallback routing cache. Only populated when blocks
    // arrive without live_identity. The cache maps (session_id, prefix) →
    // generated message ID so the same heuristic ID is reused across
    // related blocks. Must not be used for identity-bearing blocks.
    pub legacy_streaming_ids: HashMap<String, HashMap<String, String>>,
    pub current_session_id: Option<String>,
    pub session_status: HashMap<String, SessionStatus>,
    pub session_diff: HashMap<String, Vec<DiffEntry>>,
    pub todos: HashMap<String, Vec<TodoItem>>,
    pub revert: HashMap<String, RevertInfo>,
}

#[derive(Clone, Debug, Default)]
pub enum SessionStatus {
    #[default]
    Idle,
    Running,
    Compacting,
    Reconnecting,
    Retrying {
        message: String,
        attempt: u32,
        next: i64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffEntry {
    pub file: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevertInfo {
    pub message_id: String,
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub diff: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct AttachedSessionInfo {
    pub session_id: String,
    pub stage_name: String,
    pub stage_title: String,
    pub stage_id: Option<String>,
    pub stage_index: Option<u64>,
    pub stage_total: Option<u64>,
    pub status: String,
}

pub fn collect_attached_sessions_from_stage_summaries(
    stage_summaries: &[StageSummary],
    sessions: &HashMap<String, Session>,
) -> Vec<AttachedSessionInfo> {
    let mut result = stage_summaries
        .iter()
        .filter_map(|stage| {
            let attached_id = stage.primary_attached_session_id.as_ref()?;
            let session = sessions.get(attached_id);
            Some(AttachedSessionInfo {
                session_id: attached_id.clone(),
                stage_name: stage.stage_name.clone(),
                stage_title: session
                    .map(|session| session.title.clone())
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| stage.stage_name.clone()),
                stage_id: Some(stage.stage_id.clone()),
                stage_index: stage.index,
                stage_total: stage.total,
                status: scheduler_stage_status_label(stage.status),
            })
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| {
        a.stage_index
            .unwrap_or(u64::MAX)
            .cmp(&b.stage_index.unwrap_or(u64::MAX))
            .then_with(|| a.stage_id.cmp(&b.stage_id))
    });
    result
}

pub fn collect_attached_sessions(messages: &[Message]) -> Vec<AttachedSessionInfo> {
    let mut seen = HashMap::new();
    for msg in messages {
        let meta = match msg.metadata.as_ref() {
            Some(m) => m,
            None => continue,
        };
        let attached_id = match meta
            .get("scheduler_stage_attached_session_id")
            .and_then(|v| v.as_str())
        {
            Some(id) => id.to_string(),
            None => continue,
        };
        let stage_name = meta
            .get("scheduler_stage")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let stage_title = meta
            .get("scheduler_stage_title")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| stage_name.clone());
        let stage_index = meta.get("scheduler_stage_index").and_then(|v| v.as_u64());
        let stage_total = meta.get("scheduler_stage_total").and_then(|v| v.as_u64());
        let status = meta
            .get("scheduler_stage_status")
            .and_then(|v| v.as_str())
            .unwrap_or("running")
            .to_string();
        let stage_id = meta
            .get("stage_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let info = AttachedSessionInfo {
            session_id: attached_id.clone(),
            stage_name,
            stage_title,
            stage_id,
            stage_index,
            stage_total,
            status,
        };
        seen.insert(attached_id, info);
    }

    let mut result: Vec<AttachedSessionInfo> = seen.into_values().collect();
    result.sort_by(|a, b| {
        a.stage_index
            .unwrap_or(u64::MAX)
            .cmp(&b.stage_index.unwrap_or(u64::MAX))
    });
    result
}

fn scheduler_stage_status_label(status: StageStatus) -> String {
    match status {
        StageStatus::Running => "running",
        StageStatus::Waiting => "waiting",
        StageStatus::Done => "done",
        StageStatus::Cancelled => "cancelled",
        StageStatus::Cancelling => "cancelling",
        StageStatus::Blocked => "blocked",
        StageStatus::Retrying => "retrying",
    }
    .to_string()
}

impl SessionContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_session(&self) -> Option<&Session> {
        self.current_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    pub fn current_messages(&self) -> Vec<&Message> {
        self.current_session_id
            .as_ref()
            .and_then(|id| self.messages.get(id))
            .map(|m| m.iter().collect())
            .unwrap_or_default()
    }

    pub fn create_session(&mut self, title: Option<String>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let session = Session {
            id: id.clone(),
            title: title.unwrap_or_else(|| "New Session".to_string()),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        };
        self.sessions.insert(id.clone(), session);
        self.messages.insert(id.clone(), Vec::new());
        self.message_index.insert(id.clone(), HashMap::new());
        self.session_status.insert(id.clone(), SessionStatus::Idle);
        self.current_session_id = Some(id.clone());
        id
    }

    pub fn upsert_session(&mut self, session: Session) {
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.messages.entry(id.clone()).or_default();
        self.message_index.entry(id.clone()).or_default();
        self.session_status
            .entry(id.clone())
            .or_insert(SessionStatus::Idle);
        // Only set current_session_id if no session is active yet.
        // Callers that want to switch sessions should use set_current_session_id().
        if self.current_session_id.is_none() {
            self.current_session_id = Some(id);
        }
    }

    /// Explicitly switch the current session to the given id.
    pub fn set_current_session_id(&mut self, id: String) {
        self.current_session_id = Some(id);
    }

    pub fn clear_current_session_id(&mut self) {
        self.current_session_id = None;
    }

    pub fn set_messages(&mut self, session_id: &str, messages: Vec<Message>) {
        let mut index = HashMap::with_capacity(messages.len());
        for (pos, message) in messages.iter().enumerate() {
            index.insert(message.id.clone(), pos);
        }
        self.messages.insert(session_id.to_string(), messages);
        self.message_index.insert(session_id.to_string(), index);
    }

    pub fn add_message(&mut self, session_id: &str, message: Message) {
        self.upsert_message(session_id, message);
    }

    pub fn upsert_messages_incremental(&mut self, session_id: &str, incoming: Vec<Message>) {
        for message in incoming {
            self.upsert_message_for_incremental_sync(session_id, message);
        }
    }

    pub fn upsert_message(&mut self, session_id: &str, message: Message) {
        let messages = self.messages.entry(session_id.to_string()).or_default();
        let index = self
            .message_index
            .entry(session_id.to_string())
            .or_default();
        if let Some(existing_pos) = index.get(&message.id).copied() {
            if existing_pos < messages.len() {
                messages[existing_pos] = message;
                return;
            }
            // Index drift should be rare; rebuild once to recover.
            index.clear();
            for (pos, msg) in messages.iter().enumerate() {
                index.insert(msg.id.clone(), pos);
            }
        }
        let message_id = message.id.clone();
        messages.push(message);
        index.insert(message_id, messages.len().saturating_sub(1));
    }

    fn upsert_message_for_incremental_sync(&mut self, session_id: &str, message: Message) {
        let messages = self.messages.entry(session_id.to_string()).or_default();
        let index = self
            .message_index
            .entry(session_id.to_string())
            .or_default();
        if let Some(existing_pos) = index.get(&message.id).copied() {
            if let Some(existing) = messages.get_mut(existing_pos) {
                *existing = Self::merge_incremental_sync_message(existing, message);
                return;
            }
            index.clear();
            for (pos, msg) in messages.iter().enumerate() {
                index.insert(msg.id.clone(), pos);
            }
        }
        let message_id = message.id.clone();
        messages.push(message);
        index.insert(message_id, messages.len().saturating_sub(1));
    }

    fn merge_incremental_sync_message(existing: &Message, incoming: Message) -> Message {
        if !Self::should_preserve_local_streaming_assistant(existing, &incoming) {
            return incoming;
        }

        let existing_text = Self::message_part_text_content(&existing.parts);
        let incoming_text = Self::message_part_text_content(&incoming.parts);
        let preserve_text = Self::should_preserve_streaming_text(
            existing_text.as_deref(),
            incoming_text.as_deref(),
        );

        let existing_reasoning = Self::message_part_reasoning_content(&existing.parts);
        let incoming_reasoning = Self::message_part_reasoning_content(&incoming.parts);
        let preserve_reasoning = Self::should_preserve_streaming_text(
            existing_reasoning.as_deref(),
            incoming_reasoning.as_deref(),
        );

        if !preserve_text && !preserve_reasoning {
            return incoming;
        }

        let mut merged = incoming;
        merged.parts = Self::merge_incremental_sync_parts(
            &existing.parts,
            &merged.parts,
            existing_text.as_deref(),
            existing_reasoning.as_deref(),
            preserve_text,
            preserve_reasoning,
        );
        if merged.agent.is_none() {
            merged.agent = existing.agent.clone();
        }
        if merged.model.is_none() {
            merged.model = existing.model.clone();
        }
        if merged.mode.is_none() {
            merged.mode = existing.mode.clone();
        }
        Self::refresh_message_content(&mut merged);
        merged
    }

    fn should_preserve_local_streaming_assistant(existing: &Message, incoming: &Message) -> bool {
        existing.id == incoming.id
            && existing.completed_at.is_none()
            && incoming.completed_at.is_none()
            && (Self::message_part_text_content(&existing.parts).is_some()
                || Self::message_part_reasoning_content(&existing.parts).is_some())
    }

    fn should_preserve_streaming_text(existing: Option<&str>, incoming: Option<&str>) -> bool {
        let Some(existing) = existing.filter(|value| !value.is_empty()) else {
            return false;
        };
        match incoming {
            Some(incoming) if !incoming.is_empty() => {
                existing.len() > incoming.len() && existing.starts_with(incoming)
            }
            _ => true,
        }
    }

    // P0-2: MessagePart text extraction authority (helper-level, not
    // transcript authority). All consumers (mappers, render, merge) must
    // use these for part-to-text conversion — no ad-hoc conversions
    // elsewhere. The real visible transcript authority is
    // SessionContext.messages with its per-part ordering and update rules.
    fn message_part_text_content(parts: &[MessagePart]) -> Option<String> {
        let mut text = String::new();
        for part in parts {
            if let MessagePart::Text { text: value } = part {
                text.push_str(value);
            }
        }
        (!text.is_empty()).then_some(text)
    }

    fn message_part_reasoning_content(parts: &[MessagePart]) -> Option<String> {
        let mut text = String::new();
        for part in parts {
            if let MessagePart::Reasoning { text: value } = part {
                text.push_str(value);
            }
        }
        (!text.is_empty()).then_some(text)
    }

    fn merge_incremental_sync_parts(
        existing_parts: &[MessagePart],
        incoming_parts: &[MessagePart],
        existing_text: Option<&str>,
        existing_reasoning: Option<&str>,
        preserve_text: bool,
        preserve_reasoning: bool,
    ) -> Vec<MessagePart> {
        let mut merged = Vec::with_capacity(incoming_parts.len().max(existing_parts.len()));
        let mut inserted_text = false;
        let mut inserted_reasoning = false;

        for part in incoming_parts {
            match part {
                MessagePart::Text { .. } if preserve_text => {
                    if !inserted_text {
                        merged.push(MessagePart::Text {
                            text: existing_text.unwrap_or_default().to_string(),
                        });
                        inserted_text = true;
                    }
                }
                MessagePart::Reasoning { .. } if preserve_reasoning => {
                    if !inserted_reasoning {
                        merged.push(MessagePart::Reasoning {
                            text: existing_reasoning.unwrap_or_default().to_string(),
                        });
                        inserted_reasoning = true;
                    }
                }
                other => merged.push(other.clone()),
            }
        }

        if preserve_reasoning && !inserted_reasoning {
            if let Some(reasoning) = existing_reasoning.filter(|value| !value.is_empty()) {
                merged.insert(
                    0,
                    MessagePart::Reasoning {
                        text: reasoning.to_string(),
                    },
                );
            }
        }

        if preserve_text && !inserted_text {
            if let Some(text) = existing_text.filter(|value| !value.is_empty()) {
                merged.push(MessagePart::Text {
                    text: text.to_string(),
                });
            }
        }

        if merged.is_empty() {
            return existing_parts.to_vec();
        }

        merged
    }

    pub fn set_status(&mut self, session_id: &str, status: SessionStatus) {
        self.session_status.insert(session_id.to_string(), status);
    }

    pub fn status(&self, session_id: &str) -> &SessionStatus {
        self.session_status
            .get(session_id)
            .unwrap_or(&SessionStatus::Idle)
    }

    /// Incrementally update reasoning content for a message during streaming.
    /// This allows real-time display of thinking content before the message is complete.
    pub fn update_reasoning_incremental(
        &mut self,
        session_id: &str,
        message_id: &str,
        phase: &str,
        text: &str,
    ) {
        if message_id.is_empty() {
            tracing::warn!("update_reasoning_incremental called with empty message_id for session {session_id}");
            return;
        }
        let messages = self.messages.entry(session_id.to_string()).or_default();
        let index = self
            .message_index
            .entry(session_id.to_string())
            .or_default();

        // If the message doesn't exist yet (streaming hasn't synced it),
        // create a placeholder assistant message so reasoning can accumulate.
        if !index.contains_key(message_id) {
            let pos = messages.len();
            messages.push(Message {
                id: message_id.to_string(),
                role: MessageRole::Assistant,
                content: String::new(),
                created_at: chrono::Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: Vec::new(),
            });
            index.insert(message_id.to_string(), pos);
        }

        let Some(&pos) = index.get(message_id) else {
            return;
        };
        let Some(message) = messages.get_mut(pos) else {
            return;
        };

        // Find or create a Reasoning part
        match phase {
            "start" => {
                // Initialize or reset reasoning content
                // Check if there's already a Reasoning part
                let has_reasoning = message
                    .parts
                    .iter()
                    .any(|p| matches!(p, MessagePart::Reasoning { .. }));
                if !has_reasoning {
                    message.parts.push(MessagePart::Reasoning {
                        text: String::new(),
                    });
                }
            }
            "full" => {
                message
                    .parts
                    .retain(|part| !matches!(part, MessagePart::Reasoning { .. }));
                message.parts.push(MessagePart::Reasoning {
                    text: text.to_string(),
                });
            }
            "delta" => {
                // Append reasoning text
                for part in &mut message.parts {
                    if let MessagePart::Reasoning {
                        text: ref mut existing,
                    } = part
                    {
                        existing.push_str(text);
                        break;
                    }
                }
            }
            "end" => {
                // Reasoning complete - nothing special to do, the text is already there
            }
            _ => {}
        }
    }

    pub fn apply_output_block_incremental(
        &mut self,
        session_id: &str,
        block_id: Option<&str>,
        payload: &serde_json::Value,
        live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    ) {
        self.ensure_streaming_session(session_id, None, None);

        let Some(kind) = payload.get("kind").and_then(|value| value.as_str()) else {
            return;
        };

        match kind {
            "message" => self.apply_message_block(session_id, block_id, payload, live_identity),
            "reasoning" => self.apply_reasoning_block(session_id, block_id, payload, live_identity),
            "tool" => self.apply_tool_block(session_id, block_id, payload, live_identity),
            "scheduler_stage" => {
                self.apply_scheduler_stage_block(session_id, block_id, payload, live_identity)
            }
            _ => return,
        }

        self.order_current_turn_for_presentation(session_id);
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.updated_at = Utc::now();
        }
    }

    fn apply_message_block(
        &mut self,
        session_id: &str,
        block_id: Option<&str>,
        payload: &serde_json::Value,
        live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    ) {
        let role = match payload
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("assistant")
        {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            _ => MessageRole::Assistant,
        };
        let phase = payload
            .get("phase")
            .and_then(|value| value.as_str())
            .unwrap_or("delta");
        let text = payload
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        // P3-E: Use live_identity.message_id as the authoritative routing key.
        let routing_id = live_identity
            .map(|id| id.message_id.to_string())
            .or_else(|| {
                block_id
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| self.generated_streaming_id(session_id, "message"));
        let pos = self.ensure_message_for_block(session_id, Some(&routing_id), role.clone());
        let Some(message) = self
            .messages
            .get_mut(session_id)
            .and_then(|messages| messages.get_mut(pos))
        else {
            return;
        };

        match phase {
            "start" => {
                message.role = role;
                message.content.clear();
                message
                    .parts
                    .retain(|part| !matches!(part, MessagePart::Text { .. }));
            }
            "delta" => {
                if let Some(MessagePart::Text { text: existing }) = message
                    .parts
                    .iter_mut()
                    .rev()
                    .find(|part| matches!(part, MessagePart::Text { .. }))
                {
                    existing.push_str(text);
                } else {
                    message.parts.push(MessagePart::Text {
                        text: text.to_string(),
                    });
                }
            }
            "full" => {
                message.role = role;
                message
                    .parts
                    .retain(|part| !matches!(part, MessagePart::Text { .. }));
                message.parts.push(MessagePart::Text {
                    text: text.to_string(),
                });
            }
            "end" => {}
            _ => {}
        }

        Self::refresh_message_content(message);
    }

    fn apply_reasoning_block(
        &mut self,
        session_id: &str,
        block_id: Option<&str>,
        payload: &serde_json::Value,
        live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    ) {
        // P3-E: Use live_identity.message_id as the authoritative routing key.
        // Without identity or block_id, create a fresh streaming placeholder
        // instead of guessing "last assistant".
        let message_id = live_identity
            .map(|id| id.message_id.to_string())
            .or_else(|| {
                block_id
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| self.legacy_streaming_id(session_id, "reasoning"));
        let phase = payload
            .get("phase")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let text = payload
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        self.update_reasoning_incremental(session_id, &message_id, phase, text);
    }

    fn apply_tool_block(
        &mut self,
        session_id: &str,
        block_id: Option<&str>,
        payload: &serde_json::Value,
        live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    ) {
        let tool_call_id = block_id
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.legacy_streaming_id(session_id, "tool"));
        let tool_name = payload
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("tool");
        let phase = payload
            .get("phase")
            .and_then(|value| value.as_str())
            .unwrap_or("running");
        let detail = payload
            .get("detail")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();

        match phase {
            "start" | "running" => {
                // LTS-B2: tool start/running detail is progress-state, not
                // authoritative transcript content. Tool lifecycle remains
                // observable via dedicated tool_call events/progress surfaces.
            }
            "done" | "error" | "result" => {
                // Tool completion remains transcript-bearing: final result is
                // attached to the parent assistant message when identity exists.
                let routing_id = live_identity
                    .map(|id| id.message_id.to_string())
                    .or_else(|| {
                        block_id
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| self.legacy_streaming_id(session_id, "assistant"));
                let pos = self.ensure_message_for_block(
                    session_id,
                    Some(&routing_id),
                    MessageRole::Assistant,
                );
                let Some(message) = self
                    .messages
                    .get_mut(session_id)
                    .and_then(|messages| messages.get_mut(pos))
                else {
                    return;
                };
                let is_error = matches!(phase, "error");
                if let Some(part) = message.parts.iter_mut().find(|part| {
                    matches!(
                        part,
                        MessagePart::ToolResult { id, .. } if *id == tool_call_id
                    )
                }) {
                    if let MessagePart::ToolResult {
                        result,
                        is_error: part_is_error,
                        title,
                        ..
                    } = part
                    {
                        *result = detail.clone();
                        *part_is_error = is_error;
                        *title = Some(tool_name.to_string());
                    }
                } else {
                    message.parts.push(MessagePart::ToolResult {
                        id: tool_call_id.to_string(),
                        result: detail.clone(),
                        is_error,
                        title: Some(tool_name.to_string()),
                        metadata: None,
                    });
                }
                Self::refresh_message_content(message);
            }
            _ => {}
        }
    }

    fn apply_scheduler_stage_block(
        &mut self,
        session_id: &str,
        _block_id: Option<&str>,
        payload: &serde_json::Value,
        _live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    ) {
        let Ok(block) = serde_json::from_value::<SchedulerStageBlock>(payload.clone()) else {
            return;
        };

        // LTS-B3: scheduler stage is progress/topology state, not transcript.
        // The TUI keeps stage summaries and attached-session topology elsewhere;
        // do not materialize a synthetic assistant message for live stage blocks.
        if let Some(attached_session_id) = block.attached_session_id.as_deref() {
            let child_title = format!("Stage: {}", block.title);
            self.ensure_streaming_session(
                attached_session_id,
                Some(session_id.to_string()),
                Some(child_title),
            );
        }
    }

    fn ensure_message_for_block(
        &mut self,
        session_id: &str,
        block_id: Option<&str>,
        role: MessageRole,
    ) -> usize {
        let messages = self.messages.entry(session_id.to_string()).or_default();
        let index = self
            .message_index
            .entry(session_id.to_string())
            .or_default();

        if let Some(message_id) = block_id.filter(|value| !value.is_empty()) {
            if let Some(existing) = index.get(message_id).copied() {
                return existing;
            }

            let pos = messages.len();
            messages.push(Self::streaming_placeholder_message(message_id, role));
            index.insert(message_id.to_string(), pos);
            return pos;
        }

        let generated_id = format!("streaming_{}", Utc::now().timestamp_millis());
        let pos = messages.len();
        messages.push(Self::streaming_placeholder_message(&generated_id, role));
        index.insert(generated_id, pos);
        pos
    }

    fn generated_streaming_id(&self, session_id: &str, prefix: &str) -> String {
        let sequence = self
            .messages
            .get(session_id)
            .map(|messages| messages.len())
            .unwrap_or(0);
        format!("{prefix}_{session_id}_{sequence}")
    }

    fn legacy_streaming_id(&mut self, session_id: &str, prefix: &str) -> String {
        if let Some(existing) = self
            .legacy_streaming_ids
            .get(session_id)
            .and_then(|ids| ids.get(prefix))
        {
            return existing.clone();
        }

        let generated = self.generated_streaming_id(session_id, prefix);
        self.legacy_streaming_ids
            .entry(session_id.to_string())
            .or_default()
            .insert(prefix.to_string(), generated.clone());
        generated
    }

    fn order_current_turn_for_presentation(&mut self, session_id: &str) {
        let Some(messages) = self.messages.get_mut(session_id) else {
            return;
        };
        let start = messages
            .iter()
            .rposition(|message| message.role == MessageRole::User)
            .unwrap_or(0);
        messages[start..].sort_by_key(|message| {
            (
                Self::message_presentation_rank(message),
                Self::message_presentation_sequence(message),
            )
        });

        let mut index = HashMap::with_capacity(messages.len());
        for (pos, message) in messages.iter().enumerate() {
            index.insert(message.id.clone(), pos);
        }
        self.message_index.insert(session_id.to_string(), index);
    }

    fn message_presentation_rank(message: &Message) -> u8 {
        if message.role == MessageRole::User {
            return 0;
        }
        if message
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.contains_key("scheduler_stage"))
        {
            return 30;
        }
        match message.role {
            MessageRole::System => 0,
            MessageRole::Tool => 20,
            MessageRole::Assistant
                if message
                    .parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Reasoning { .. }))
                    && !message
                        .parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::Text { .. })) =>
            {
                10
            }
            MessageRole::Assistant => 90,
            MessageRole::User => 0,
        }
    }

    fn message_presentation_sequence(message: &Message) -> u64 {
        message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("scheduler_stage_index"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    }

    fn ensure_streaming_session(
        &mut self,
        session_id: &str,
        parent_id: Option<String>,
        title: Option<String>,
    ) {
        self.messages.entry(session_id.to_string()).or_default();
        self.message_index
            .entry(session_id.to_string())
            .or_default();
        self.session_status
            .entry(session_id.to_string())
            .or_insert(SessionStatus::Idle);

        let entry = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| Session {
                id: session_id.to_string(),
                title: title.clone().unwrap_or_else(|| "Live Session".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_id: parent_id.clone(),
                share: None,
                metadata: None,
            });

        if let Some(parent_id) = parent_id {
            entry.parent_id = Some(parent_id);
        }
        if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
            entry.title = title;
        }
    }

    fn streaming_placeholder_message(message_id: &str, role: MessageRole) -> Message {
        Message {
            id: message_id.to_string(),
            role,
            content: String::new(),
            created_at: Utc::now(),
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            metadata: None,
            multimodal: None,
            parts: Vec::new(),
        }
    }

    fn refresh_message_content(message: &mut Message) {
        message.content = message
            .parts
            .iter()
            .map(|part| match part {
                MessagePart::Text { text } => text.clone(),
                MessagePart::Reasoning { text } => format!("[reasoning] {}", text),
                MessagePart::ToolCall {
                    name, arguments, ..
                } => {
                    if arguments.trim().is_empty() {
                        format!("[tool:{}]", name)
                    } else {
                        format!("[tool:{}] {}", name, arguments)
                    }
                }
                MessagePart::ToolResult {
                    result, is_error, ..
                } => {
                    if *is_error {
                        format!("[tool-error] {}", result)
                    } else {
                        format!("[tool-result] {}", result)
                    }
                }
                MessagePart::File { path, mime } => {
                    summarize_block_items_inline(&build_file_items(path, mime))
                }
                MessagePart::Image { url } => summarize_block_items_inline(&build_image_items(url)),
            })
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_command_render::governance_fixtures::live_transcript_state_fixture;
    use serde_json::json;

    fn live_identity(
        message_id: &str,
        part_key: &str,
        part_kind: agendao_types::LiveMessagePartKind,
        phase: agendao_types::LivePartPhase,
        legacy_block_id: Option<&str>,
    ) -> agendao_types::LiveMessagePartIdentity {
        agendao_types::LiveMessagePartIdentity {
            message_id: message_id.to_string(),
            part_key: part_key.to_string(),
            part_kind,
            phase,
            legacy_block_id: legacy_block_id.map(str::to_string),
        }
    }

    fn apply_live_block(
        ctx: &mut SessionContext,
        session_id: &str,
        block_id: Option<&str>,
        payload: serde_json::Value,
        live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    ) {
        ctx.apply_output_block_incremental(session_id, block_id, &payload, live_identity);
    }

    #[test]
    fn scheduler_stage_output_block_stays_out_of_transcript_and_tracks_attached_session() {
        let mut ctx = SessionContext::new();
        ctx.upsert_session(Session {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });

        ctx.apply_output_block_incremental(
            "parent",
            Some("stage-message-1"),
            &json!({
                "kind": "scheduler_stage",
                "stage_id": "stage-1",
                "profile": "atlas",
                "stage": "execution-orchestration",
                "title": "Execution Orchestration",
                "text": "child stage running",
                "stage_index": 2,
                "stage_total": 5,
                "status": "running",
                "attached_session_id": "child-1",
                "active_agents": [],
                "active_skills": [],
                "active_categories": [],
                "done_agent_count": 0,
                "total_agent_count": 0
            }),
            None,
        );

        assert!(
            ctx.messages
                .get("parent")
                .is_none_or(|messages| messages.is_empty()),
            "scheduler stage is progress-state and must not materialize a transcript message"
        );

        let child = ctx
            .sessions
            .get("child-1")
            .expect("attached session created");
        assert_eq!(child.parent_id.as_deref(), Some("parent"));
        assert_eq!(child.title, "Stage: Execution Orchestration");
    }

    #[test]
    fn live_scheduler_stage_does_not_insert_message_before_final_answer() {
        let mut ctx = SessionContext::new();
        let fixture = live_transcript_state_fixture();
        ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });
        ctx.add_message(
            "session-1",
            Message {
                id: "user-1".to_string(),
                role: MessageRole::User,
                content: "go".to_string(),
                created_at: Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![MessagePart::Text {
                    text: "go".to_string(),
                }],
            },
        );

        ctx.apply_output_block_incremental(
            "session-1",
            Some("final-answer"),
            &json!({
                "kind": "message",
                "phase": "full",
                "role": "assistant",
                "text": "final"
            }),
            None,
        );
        ctx.apply_output_block_incremental(
            "session-1",
            Some("stage-message"),
            &fixture.scheduler_stage_exclusion.payload(),
            None,
        );

        let ids = ctx
            .messages
            .get("session-1")
            .expect("messages")
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["user-1", "final-answer"]);
    }

    #[test]
    fn live_identity_scheduler_stage_does_not_rewrite_assistant_message() {
        let mut ctx = SessionContext::new();
        let fixture = live_transcript_state_fixture();
        ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });
        ctx.apply_output_block_incremental(
            "session-1",
            Some("assistant-final"),
            &json!({
                "kind": "message",
                "phase": "full",
                "role": "assistant",
                "text": "final"
            }),
            None,
        );

        ctx.apply_output_block_incremental(
            "session-1",
            Some("stage-block-1"),
            &fixture.scheduler_stage_exclusion.payload(),
            Some(&fixture.scheduler_stage_exclusion.scheduler_identity()),
        );

        let messages = ctx.messages.get("session-1").expect("messages");
        assert_eq!(
            messages.len(),
            1,
            "scheduler stage with identity must not create or rewrite transcript messages"
        );
        assert_eq!(messages[0].id, "assistant-final");
        assert_eq!(messages[0].content, "final");
    }

    #[test]
    fn child_output_block_creates_background_session_cache() {
        let mut ctx = SessionContext::new();

        ctx.apply_output_block_incremental(
            "child-1",
            Some("assistant-1"),
            &json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "hello child"
            }),
            None,
        );

        let child = ctx
            .sessions
            .get("child-1")
            .expect("attached session placeholder");
        assert_eq!(child.id, "child-1");

        let message = ctx
            .messages
            .get("child-1")
            .and_then(|messages| messages.iter().find(|message| message.id == "assistant-1"))
            .expect("child assistant message");
        assert!(message.content.contains("hello child"));
    }

    #[test]
    fn message_start_preserves_existing_reasoning_parts() {
        let mut ctx = SessionContext::new();

        ctx.apply_output_block_incremental(
            "session-1",
            Some("assistant-1"),
            &json!({
                "kind": "reasoning",
                "phase": "start",
                "text": ""
            }),
            None,
        );
        ctx.apply_output_block_incremental(
            "session-1",
            Some("assistant-1"),
            &json!({
                "kind": "reasoning",
                "phase": "delta",
                "text": "thinking..."
            }),
            None,
        );
        ctx.apply_output_block_incremental(
            "session-1",
            Some("assistant-1"),
            &json!({
                "kind": "message",
                "phase": "start",
                "role": "assistant",
                "text": ""
            }),
            None,
        );

        let message = ctx
            .messages
            .get("session-1")
            .and_then(|messages| messages.iter().find(|message| message.id == "assistant-1"))
            .expect("assistant message");

        assert!(message.parts.iter().any(|part| {
            matches!(
                part,
                MessagePart::Reasoning { text } if text == "thinking..."
            )
        }));
    }

    #[test]
    fn tool_running_detail_stays_out_of_transcript_until_final_result() {
        let mut ctx = SessionContext::new();
        let fixture = live_transcript_state_fixture();

        let identity = fixture.tool_progress_exclusion.message_identity();
        ctx.apply_output_block_incremental(
            "session-1",
            Some(&fixture.tool_progress_exclusion.message.message_id),
            &json!({
                "kind": "message",
                "phase": "full",
                "role": "assistant",
                "text": fixture.tool_progress_exclusion.message.text
            }),
            Some(&identity),
        );

        ctx.apply_output_block_incremental(
            "session-1",
            Some(&fixture.tool_progress_exclusion.tool_running.tool_id),
            &json!({
                "kind": "tool",
                "phase": "running",
                "name": fixture.tool_progress_exclusion.tool_running.tool_name,
                "detail": fixture.tool_progress_exclusion.tool_running.tool_detail
            }),
            Some(&fixture.tool_progress_exclusion.tool_running_identity()),
        );
        ctx.apply_output_block_incremental(
            "session-1",
            Some(&fixture.tool_progress_exclusion.tool_result.tool_id),
            &json!({
                "kind": "tool",
                "phase": "done",
                "name": fixture.tool_progress_exclusion.tool_result.tool_name,
                "detail": fixture.tool_progress_exclusion.tool_result.tool_detail
            }),
            Some(&fixture.tool_progress_exclusion.tool_result_identity()),
        );

        let message = ctx
            .messages
            .get("session-1")
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.id == fixture.tool_progress_exclusion.message.message_id
                })
            })
            .expect("assistant message");

        let tool_calls = message
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::ToolCall { .. } => Some("tool_call"),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(tool_calls.is_empty(), "{tool_calls:?}");

        let tool_results = message
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::ToolResult { id, result, .. } => Some((id.as_str(), result.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tool_results,
            vec![(
                fixture.tool_progress_exclusion.tool_result.tool_id.as_str(),
                fixture
                    .tool_progress_exclusion
                    .tool_result
                    .tool_detail
                    .as_str()
            )],
            "{tool_results:?}"
        );
    }

    #[test]
    fn legacy_reasoning_without_identity_creates_fresh_placeholder_instead_of_reusing_last_assistant(
    ) {
        let mut ctx = SessionContext::new();

        ctx.apply_output_block_incremental(
            "session-1",
            Some("assistant-final"),
            &json!({
                "kind": "message",
                "phase": "full",
                "role": "assistant",
                "text": "final answer"
            }),
            None,
        );
        ctx.apply_output_block_incremental(
            "session-1",
            None,
            &json!({
                "kind": "reasoning",
                "phase": "start",
                "text": ""
            }),
            None,
        );
        ctx.apply_output_block_incremental(
            "session-1",
            None,
            &json!({
                "kind": "reasoning",
                "phase": "delta",
                "text": "late hidden reasoning"
            }),
            None,
        );

        let messages = ctx.messages.get("session-1").expect("messages");
        assert_eq!(messages.len(), 2, "{messages:?}");
        let final_message = messages
            .iter()
            .find(|message| message.id == "assistant-final")
            .expect("final message");
        assert!(
            final_message
                .parts
                .iter()
                .all(|part| !matches!(part, MessagePart::Reasoning { .. })),
            "legacy reasoning must not backfill into last assistant"
        );
        let legacy_message = messages
            .iter()
            .find(|message| message.id != "assistant-final")
            .expect("legacy placeholder");
        assert!(
            legacy_message
                .parts
                .iter()
                .any(|part| matches!(part, MessagePart::Reasoning { text } if text == "late hidden reasoning")),
            "{:?}",
            legacy_message.parts
        );
    }

    #[test]
    fn reasoning_end_then_trailing_assistant_text_stays_in_same_identity_message() {
        let mut ctx = SessionContext::new();

        let reasoning_identity = live_identity(
            "assistant-1",
            agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
            agendao_types::LiveMessagePartKind::AssistantReasoning,
            agendao_types::LivePartPhase::Snapshot,
            Some("assistant-1"),
        );
        apply_live_block(
            &mut ctx,
            "session-1",
            Some("assistant-1"),
            json!({
                "kind": "reasoning",
                "phase": "start",
                "text": ""
            }),
            Some(&reasoning_identity),
        );
        apply_live_block(
            &mut ctx,
            "session-1",
            Some("assistant-1"),
            json!({
                "kind": "reasoning",
                "phase": "delta",
                "text": "thinking"
            }),
            Some(&reasoning_identity),
        );
        apply_live_block(
            &mut ctx,
            "session-1",
            Some("assistant-1"),
            json!({
                "kind": "reasoning",
                "phase": "end",
                "text": ""
            }),
            Some(&live_identity(
                "assistant-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                agendao_types::LiveMessagePartKind::AssistantReasoning,
                agendao_types::LivePartPhase::End,
                Some("assistant-1"),
            )),
        );
        apply_live_block(
            &mut ctx,
            "session-1",
            Some("assistant-1"),
            json!({
                "kind": "message",
                "phase": "delta",
                "role": "assistant",
                "text": "final answer"
            }),
            Some(&live_identity(
                "assistant-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                agendao_types::LiveMessagePartKind::AssistantText,
                agendao_types::LivePartPhase::Snapshot,
                Some("assistant-1"),
            )),
        );

        let messages = ctx.messages.get("session-1").expect("messages");
        assert_eq!(messages.len(), 1, "{messages:?}");
        let message = &messages[0];
        assert_eq!(message.id, "assistant-1");
        assert!(
            message
                .parts
                .iter()
                .any(|part| matches!(part, MessagePart::Reasoning { text } if text == "thinking")),
            "{:?}",
            message.parts
        );
        assert!(
            message
                .parts
                .iter()
                .any(|part| matches!(part, MessagePart::Text { text } if text == "final answer")),
            "{:?}",
            message.parts
        );
    }

    #[test]
    fn assistant_full_snapshot_replaces_non_prefix_partial_in_same_identity_message() {
        let mut ctx = SessionContext::new();
        let text_identity = live_identity(
            "assistant-1",
            agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
            agendao_types::LiveMessagePartKind::AssistantText,
            agendao_types::LivePartPhase::Snapshot,
            Some("assistant-1"),
        );

        apply_live_block(
            &mut ctx,
            "session-1",
            Some("assistant-1"),
            json!({
                "kind": "message",
                "phase": "full",
                "role": "assistant",
                "text": "现在我已掌握"
            }),
            Some(&text_identity),
        );
        apply_live_block(
            &mut ctx,
            "session-1",
            Some("assistant-1"),
            json!({
                "kind": "message",
                "phase": "full",
                "role": "assistant",
                "text": "现在我已掌握充分信息，以下是完整调研报告。"
            }),
            Some(&text_identity),
        );

        let messages = ctx.messages.get("session-1").expect("messages");
        assert_eq!(messages.len(), 1, "{messages:?}");
        let message = &messages[0];
        assert_eq!(message.id, "assistant-1");
        assert_eq!(
            message.content,
            "现在我已掌握充分信息，以下是完整调研报告。"
        );
        let text_parts = message
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            text_parts,
            vec!["现在我已掌握充分信息，以下是完整调研报告。"]
        );
    }

    #[test]
    fn shared_sample_preserves_five_assistant_messages_and_four_tool_cycles() {
        let mut ctx = SessionContext::new();
        let fixture = live_transcript_state_fixture();

        for entry in &fixture.shared_turn_cycles.entries {
            let identity = entry.assistant_identity();
            ctx.apply_output_block_incremental(
                "session-1",
                Some(&entry.message_id),
                &json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": entry.message_text
                }),
                Some(&identity),
            );

            if let Some(tool) = &entry.tool {
                ctx.apply_output_block_incremental(
                    "session-1",
                    Some(&tool.tool_id),
                    &json!({
                        "kind": "tool",
                        "phase": "done",
                        "name": tool.tool_name,
                        "detail": tool.tool_detail
                    }),
                    Some(&tool.tool_result_identity(&entry.message_id)),
                );
            }
        }

        let messages = ctx.messages.get("session-1").expect("messages");
        assert_eq!(
            messages.len(),
            fixture.shared_turn_cycles.expected.assistant_message_count,
            "{messages:?}"
        );

        for entry in &fixture.shared_turn_cycles.entries {
            let message = messages
                .iter()
                .find(|message| message.id == entry.message_id)
                .expect("assistant message");
            assert!(
                message.parts.iter().any(
                    |part| matches!(part, MessagePart::Text { text } if text == &entry.message_text)
                ),
                "{:?}",
                message.parts
            );

            let tool_results = message
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::ToolResult { id, result, .. } => {
                        Some((id.as_str(), result.as_str()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();

            if let Some(tool) = &entry.tool {
                assert_eq!(tool_results.len(), 1, "{tool_results:?}");
                assert_eq!(tool_results[0].0, tool.tool_id);
                assert_eq!(tool_results[0].1, tool.tool_detail);
            } else {
                assert!(tool_results.is_empty(), "{tool_results:?}");
            }
        }
    }

    #[test]
    fn shared_sample_run_tail_contract_matches_tui_status_surface_expectations() {
        let fixture = live_transcript_state_fixture();
        let run_tail = &fixture.run_tail_contract;

        assert_eq!(run_tail.completed_status, "complete");
        assert_eq!(run_tail.error_status, "error");
        assert_eq!(run_tail.awaiting_user_status, "awaiting_user");
        assert!(run_tail.completed_usage.input_tokens > 0);
        assert!(run_tail.completed_usage.output_tokens > 0);
        assert!(run_tail.completed_usage.reasoning_tokens > 0);
        assert!(run_tail.completed_usage.total_cost > 0.0);
        assert!(!run_tail.awaiting_user_detail.is_empty());
        assert!(!run_tail.error_message.is_empty());
    }

    #[test]
    fn canonical_live_stream_matches_tui_visible_transcript_contract() {
        let mut ctx = SessionContext::new();
        let fixture = live_transcript_state_fixture();
        let canonical = &fixture.canonical_live_stream;

        ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });

        for event in &canonical.events {
            ctx.apply_output_block_incremental(
                "session-1",
                event.id.as_deref(),
                &event.payload(),
                event.live_identity.as_ref(),
            );
        }

        let messages = ctx.messages.get("session-1").expect("messages");
        let assistant_messages = messages
            .iter()
            .filter(|message| message.role == MessageRole::Assistant)
            .collect::<Vec<_>>();

        assert_eq!(
            assistant_messages.len(),
            canonical.expected.transcript_blocks.assistant_count,
            "{assistant_messages:?}"
        );

        let final_answer = *assistant_messages.first().expect("final assistant message");

        let reasoning_parts = final_answer
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Reasoning { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reasoning_parts,
            vec!["I need to search for this information."],
            "{:?}",
            final_answer.parts
        );

        let final_tool_results = final_answer
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::ToolResult { id, result, .. } => Some((id.as_str(), result.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            final_tool_results.len(),
            canonical.expected.transcript_blocks.tool_count,
            "{:?}",
            final_answer.parts
        );
        assert!(
            final_tool_results
                .iter()
                .any(|(_, result)| *result == "Found 5 results"),
            "{final_tool_results:?}"
        );
        assert!(
            final_tool_results
                .iter()
                .any(|(_, result)| *result == "file content"),
            "{final_tool_results:?}"
        );
        assert_eq!(final_tool_results.len(), 2, "{final_tool_results:?}");
        assert!(
            final_answer
                .parts
                .iter()
                .all(|part| !matches!(part, MessagePart::ToolCall { .. })),
            "tool running/start progress must stay out of TUI authoritative transcript: {:?}",
            final_answer.parts
        );
        let part_kinds = final_answer
            .parts
            .iter()
            .map(|part| match part {
                MessagePart::Reasoning { .. } => "reasoning",
                MessagePart::ToolResult { .. } => "tool_result",
                MessagePart::Text { .. } => "text",
                MessagePart::ToolCall { .. } => "tool_call",
                MessagePart::File { .. } => "file",
                MessagePart::Image { .. } => "image",
            })
            .collect::<Vec<_>>();
        assert_eq!(
            part_kinds,
            vec!["reasoning", "tool_result", "tool_result", "text"],
            "{part_kinds:?}"
        );
    }
}
