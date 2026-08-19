//! P3-C: Shared Live Semantic Consumer.
//!
//! Identity-driven state machine that consumes coalesced live output blocks
//! (P3-B snapshots) and produces explicit semantic actions for frontends.
//!
//! This replaces heuristic message/part guessing and
//! `render_terminal_stream_block_semantic` — no more "last same role" routing,
//! no more `semantic_delta_suffix` prefix comparison, no more implicit
//! `assistant_visible`/`assistant_open` boundary resets.
//!
//! Every live content must carry a `LiveMessagePartIdentity` (P3-A).

use agendao_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveContentMode {
    Snapshot,
    Delta,
}

// ── Semantic Actions ────────────────────────────────────────────────────

/// Discrete semantic action the frontend should take for this live block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticAction {
    /// Start a new assistant message — frontend should emit a header/bullet.
    OpenAssistant { text: String },
    /// Append text to the current assistant message stream.
    AppendTextDelta { text: String },
    /// The assistant text has been rewritten — frontend should replace the
    /// current text with this full snapshot.
    ReplaceTextFull { text: String },
    /// Open a reasoning (thinking) block.
    OpenReasoning { text: String },
    /// Append to the current reasoning stream.
    AppendReasoningDelta { text: String },
    /// The reasoning text has been rewritten — replace the visible snapshot.
    ReplaceReasoningFull { text: String },
    /// Close the reasoning block and return to assistant output.
    CloseReasoning,
    /// A tool call started.
    ToolCallStarted { call_id: String, name: String },
    /// A tool call completed.
    ToolCallCompleted { call_id: String },
    /// Assistant boundary: tool call or other non-text event occurred.
    /// Frontend should prepare for a potential new assistant segment.
    ToolBoundary,
    /// No action — block was fully consumed (e.g., Start/End identity phases).
    NoOp,
}

// ── Consumer State ──────────────────────────────────────────────────────

/// Internal state of the live semantic consumer.
/// Keyed by `{message_id}:{part_key}` so different parts within the same
/// message (text, reasoning, second text block after tool) are tracked
/// independently.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConsumerState {
    /// Last emitted full text per `{message_id}:{part_key}`.
    last_texts: std::collections::HashMap<String, String>,
    /// Currently open reasoning part key, if any.
    reasoning_key: Option<String>,
}

/// Core state machine for live output semantics.
///
/// Input: a coalesced `OutputBlock` with its `LiveMessagePartIdentity`.
/// Output: a `SemanticAction` telling the frontend what to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSemanticConsumer {
    state: ConsumerState,
}

impl LiveSemanticConsumer {
    pub fn new() -> Self {
        Self {
            state: ConsumerState::default(),
        }
    }

    pub fn is_transcript_bearing_kind(kind: &LiveMessagePartKind) -> bool {
        matches!(
            kind,
            LiveMessagePartKind::AssistantText
                | LiveMessagePartKind::AssistantReasoning
                | LiveMessagePartKind::ToolResult
        )
    }

    fn part_slot(&self, identity: &LiveMessagePartIdentity) -> String {
        format!("{}:{}", identity.message_id, identity.part_key)
    }

    /// Consume a live output block and return the semantic action.
    pub fn consume(
        &mut self,
        block_text: Option<&str>,
        identity: &LiveMessagePartIdentity,
        mode: LiveContentMode,
    ) -> SemanticAction {
        match identity.part_kind {
            LiveMessagePartKind::AssistantText => match mode {
                LiveContentMode::Delta => {
                    self.consume_assistant_text_delta(identity, block_text.unwrap_or(""))
                }
                LiveContentMode::Snapshot => {
                    self.consume_assistant_text_snapshot(identity, block_text.unwrap_or(""))
                }
            },
            LiveMessagePartKind::AssistantReasoning => match mode {
                LiveContentMode::Delta => {
                    self.consume_reasoning_delta(identity, block_text.unwrap_or(""))
                }
                LiveContentMode::Snapshot => {
                    self.consume_reasoning_snapshot(identity, block_text.unwrap_or(""))
                }
            },
            LiveMessagePartKind::ToolCall => SemanticAction::ToolBoundary,
            LiveMessagePartKind::ToolResult => SemanticAction::ToolCallCompleted {
                call_id: agendao_types::tool_id_from_part_key(&identity.part_key)
                    .unwrap_or(&identity.part_key)
                    .to_string(),
            },
        }
    }

    // ── Assistant text (per-part tracking) ───────────────────────────

    fn consume_assistant_text_snapshot(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        if identity.phase == LivePartPhase::End {
            self.state.last_texts.remove(&slot);
            return SemanticAction::NoOp;
        }
        if text.is_empty() {
            return SemanticAction::NoOp;
        }

        let previous = self.state.last_texts.insert(slot, text.to_string());
        match previous {
            None => SemanticAction::OpenAssistant {
                text: text.to_string(),
            },
            Some(previous) if text == previous => SemanticAction::NoOp,
            Some(previous) if text.starts_with(&previous) => SemanticAction::AppendTextDelta {
                text: text[previous.len()..].to_string(),
            },
            Some(_) => SemanticAction::ReplaceTextFull {
                text: text.to_string(),
            },
        }
    }

    fn consume_assistant_text_delta(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        if identity.phase == LivePartPhase::End {
            self.state.last_texts.remove(&slot);
            return SemanticAction::NoOp;
        }
        if text.is_empty() {
            return SemanticAction::NoOp;
        }

        match self.state.last_texts.get_mut(&slot) {
            Some(existing) => {
                existing.push_str(text);
                SemanticAction::AppendTextDelta {
                    text: text.to_string(),
                }
            }
            None => {
                self.state.last_texts.insert(slot, text.to_string());
                SemanticAction::OpenAssistant {
                    text: text.to_string(),
                }
            }
        }
    }

    // ── Reasoning (per-part tracking) ────────────────────────────────

    fn consume_reasoning_snapshot(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        if identity.phase == LivePartPhase::End {
            self.state.last_texts.remove(&slot);
            self.state.reasoning_key = None;
            return SemanticAction::CloseReasoning;
        }
        if text.is_empty() {
            return SemanticAction::NoOp;
        }

        self.state.reasoning_key = Some(slot.clone());
        let previous = self.state.last_texts.insert(slot, text.to_string());
        match previous {
            None => SemanticAction::OpenReasoning {
                text: text.to_string(),
            },
            Some(previous) if text == previous => SemanticAction::NoOp,
            Some(previous) if text.starts_with(&previous) => SemanticAction::AppendReasoningDelta {
                text: text[previous.len()..].to_string(),
            },
            Some(_) => SemanticAction::ReplaceReasoningFull {
                text: text.to_string(),
            },
        }
    }

    fn consume_reasoning_delta(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        if identity.phase == LivePartPhase::End {
            self.state.reasoning_key = None;
            self.state.last_texts.remove(&slot);
            return SemanticAction::CloseReasoning;
        }
        if text.is_empty() {
            return SemanticAction::NoOp;
        }

        self.state.reasoning_key = Some(slot.clone());
        match self.state.last_texts.get_mut(&slot) {
            Some(existing) => {
                existing.push_str(text);
                SemanticAction::AppendReasoningDelta {
                    text: text.to_string(),
                }
            }
            None => {
                self.state.last_texts.insert(slot, text.to_string());
                SemanticAction::OpenReasoning {
                    text: text.to_string(),
                }
            }
        }
    }

    /// Explicitly close the current reasoning block.
    pub fn close_reasoning(&mut self) -> SemanticAction {
        if self.state.reasoning_key.is_some() {
            self.state.reasoning_key = None;
            SemanticAction::CloseReasoning
        } else {
            SemanticAction::NoOp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::{tool_call_part_key, tool_result_part_key};

    fn identity(
        msg_id: &str,
        part_key: &str,
        kind: LiveMessagePartKind,
        phase: LivePartPhase,
    ) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: msg_id.to_string(),
            part_key: part_key.to_string(),
            part_kind: kind,
            phase,
        }
    }

    #[test]
    fn single_assistant_message_grows_via_snapshots() {
        let mut c = LiveSemanticConsumer::new();

        let a = c.consume(
            Some("hello"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::OpenAssistant {
                text: "hello".to_string()
            }
        );

        let a = c.consume(
            Some("hello world"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::AppendTextDelta {
                text: " world".to_string()
            }
        );

        let a = c.consume(
            None,
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::End,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn single_assistant_message_grows_via_raw_deltas() {
        let mut c = LiveSemanticConsumer::new();

        let a = c.consume(
            Some("hello"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Delta,
        );
        assert_eq!(
            a,
            SemanticAction::OpenAssistant {
                text: "hello".to_string()
            }
        );

        let a = c.consume(
            Some(" world"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Delta,
        );
        assert_eq!(
            a,
            SemanticAction::AppendTextDelta {
                text: " world".to_string()
            }
        );

        let a = c.consume(
            None,
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::End,
            ),
            LiveContentMode::Delta,
        );
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn new_message_id_triggers_open_assistant() {
        let mut c = LiveSemanticConsumer::new();

        c.consume(
            Some("msg1 text"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        // Same text, no action.
        let a = c.consume(
            Some("msg1 text"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);

        // New message ID → OpenAssistant.
        let a = c.consume(
            Some("msg2 text"),
            &identity(
                "msg-2",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::OpenAssistant {
                text: "msg2 text".to_string()
            }
        );
    }

    #[test]
    fn non_prefix_text_triggers_replace() {
        let mut c = LiveSemanticConsumer::new();

        c.consume(
            Some("old text"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        // Text completely changed (non-prefix) → replace, not append double.
        let a = c.consume(
            Some("new text"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::ReplaceTextFull {
                text: "new text".to_string()
            }
        );
    }

    #[test]
    fn reasoning_opens_and_closes() {
        let mut c = LiveSemanticConsumer::new();

        let a = c.consume(
            Some("thinking..."),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::OpenReasoning {
                text: "thinking...".to_string()
            }
        );

        let a = c.consume(
            Some("thinking...done"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::AppendReasoningDelta {
                text: "done".to_string()
            }
        );

        let a = c.consume(
            None,
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::End,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::CloseReasoning);

        let a = c.close_reasoning();
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn tool_call_triggers_boundary() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            None,
            &identity(
                "msg-1",
                &tool_call_part_key("call-1"),
                LiveMessagePartKind::ToolCall,
                LivePartPhase::Start,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::ToolBoundary);
    }

    #[test]
    fn tool_result_triggers_completion_action() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            None,
            &identity(
                "msg-1",
                &tool_result_part_key("call-1"),
                LiveMessagePartKind::ToolResult,
                LivePartPhase::End,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::ToolCallCompleted {
                call_id: "call-1".to_string()
            }
        );
    }

    #[test]
    fn empty_text_is_no_op() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            Some(""),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn reasoning_raw_deltas_append_without_full_replay() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            Some("alpha"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Delta,
        );
        assert_eq!(
            a,
            SemanticAction::OpenReasoning {
                text: "alpha".to_string()
            }
        );

        let a = c.consume(
            Some(" beta"),
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            ),
            LiveContentMode::Delta,
        );
        assert_eq!(
            a,
            SemanticAction::AppendReasoningDelta {
                text: " beta".to_string()
            }
        );

        let a = c.consume(
            None,
            &identity(
                "msg-1",
                agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::End,
            ),
            LiveContentMode::Delta,
        );
        assert_eq!(a, SemanticAction::CloseReasoning);
    }
}
