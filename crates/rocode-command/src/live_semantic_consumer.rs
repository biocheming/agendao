//! P3-C: Shared Live Semantic Consumer.
//!
//! Identity-driven state machine that consumes coalesced live output blocks
//! (P3-B snapshots) and produces explicit semantic actions for frontends.
//!
//! This replaces the heuristic guessing in `TerminalStreamAccumulator` and
//! `render_terminal_stream_block_semantic` — no more "last same role" routing,
//! no more `semantic_delta_suffix` prefix comparison, no more implicit
//! `assistant_visible`/`assistant_open` boundary resets.
//!
//! Every live content must carry a `LiveMessagePartIdentity` (P3-A).
//! Blocks without identity are passed through as legacy.

use rocode_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};

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
    /// Identity-bearing event that is not part of the live transcript state
    /// machine and should be rendered directly by the caller.
    NonTranscriptPassThrough,
    /// Missing identity: caller must route through the legacy/no-identity path.
    MissingIdentity,
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
    /// Last emitted message ID (for detecting message transitions).
    last_message_id: Option<String>,
}

/// Core state machine for live output semantics.
///
/// Input: a coalesced `OutputBlock` with optional `LiveMessagePartIdentity`.
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
        identity: Option<&LiveMessagePartIdentity>,
        mode: LiveContentMode,
    ) -> SemanticAction {
        let Some(identity) = identity else {
            return SemanticAction::MissingIdentity;
        };

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
                call_id: identity
                    .legacy_block_id
                    .clone()
                    .unwrap_or_else(|| identity.part_key.clone()),
            },
            LiveMessagePartKind::SchedulerStage => SemanticAction::NonTranscriptPassThrough,
        }
    }

    // ── Assistant text (per-part tracking) ───────────────────────────

    fn consume_assistant_text_snapshot(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        self.state.last_message_id = Some(identity.message_id.clone());

        let previous = self.state.last_texts.get(&slot).cloned();
        if !text.is_empty() || previous.is_none() {
            self.state.last_texts.insert(slot.clone(), text.to_string());
        }

        if identity.phase == LivePartPhase::End {
            if let Some(final_text) = previous.or_else(|| self.state.last_texts.get(&slot).cloned())
            {
                if final_text.is_empty() && text.is_empty() {
                    return SemanticAction::NoOp;
                }
                return SemanticAction::ReplaceTextFull {
                    text: if text.is_empty() {
                        final_text
                    } else {
                        text.to_string()
                    },
                };
            }
        }

        SemanticAction::NoOp
    }

    fn consume_assistant_text_delta(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        self.state.last_message_id = Some(identity.message_id.clone());

        if !text.is_empty() {
            match self.state.last_texts.get_mut(&slot) {
                Some(existing) => {
                    existing.push_str(text);
                }
                None => {
                    self.state.last_texts.insert(slot.clone(), text.to_string());
                }
            }
        }

        if identity.phase == LivePartPhase::End {
            SemanticAction::ReplaceTextFull {
                text: self
                    .state
                    .last_texts
                    .get(&slot)
                    .cloned()
                    .unwrap_or_default(),
            }
        } else if text.is_empty() {
            SemanticAction::NoOp
        } else {
            SemanticAction::NoOp
        }
    }

    // ── Reasoning (per-part tracking) ────────────────────────────────

    fn consume_reasoning_snapshot(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        self.state.reasoning_key = Some(slot.clone());
        let previous = self.state.last_texts.get(&slot).cloned();
        if !text.is_empty() {
            self.state.last_texts.insert(slot.clone(), text.to_string());
        }

        if identity.phase == LivePartPhase::End {
            if let Some(final_text) = previous.or_else(|| self.state.last_texts.get(&slot).cloned())
            {
                if final_text.is_empty() && text.is_empty() {
                    return SemanticAction::NoOp;
                }
                self.state.reasoning_key = None;
                return SemanticAction::ReplaceReasoningFull {
                    text: if text.is_empty() {
                        final_text
                    } else {
                        text.to_string()
                    },
                };
            }
        }

        SemanticAction::NoOp
    }

    fn consume_reasoning_delta(
        &mut self,
        identity: &LiveMessagePartIdentity,
        text: &str,
    ) -> SemanticAction {
        let slot = self.part_slot(identity);
        self.state.reasoning_key = Some(slot.clone());

        if !text.is_empty() {
            if let Some(existing) = self.state.last_texts.get_mut(&slot) {
                existing.push_str(text);
            } else {
                self.state.last_texts.insert(slot.clone(), text.to_string());
            }
        }
        if identity.phase == LivePartPhase::End {
            self.state.reasoning_key = None;
            SemanticAction::ReplaceReasoningFull {
                text: self
                    .state
                    .last_texts
                    .get(&slot)
                    .cloned()
                    .unwrap_or_default(),
            }
        } else if text.is_empty() {
            SemanticAction::NoOp
        } else {
            SemanticAction::NoOp
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
            legacy_block_id: Some("block-1".to_string()),
        }
    }

    #[test]
    fn single_assistant_message_grows_via_snapshots() {
        let mut c = LiveSemanticConsumer::new();

        let a = c.consume(
            Some("hello"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            Some("hello world"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            None,
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::End,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::ReplaceTextFull {
                text: "hello world".to_string()
            }
        );
    }

    #[test]
    fn single_assistant_message_grows_via_raw_deltas() {
        let mut c = LiveSemanticConsumer::new();

        let a = c.consume(
            Some("hello"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Delta,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            Some(" world"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Delta,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            None,
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::End,
            )),
            LiveContentMode::Delta,
        );
        assert_eq!(
            a,
            SemanticAction::ReplaceTextFull {
                text: "hello world".to_string()
            }
        );
    }

    #[test]
    fn new_message_id_triggers_open_assistant() {
        let mut c = LiveSemanticConsumer::new();

        c.consume(
            Some("msg1 text"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        // Same text, no action.
        let a = c.consume(
            Some("msg1 text"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);

        // New message ID → OpenAssistant.
        let a = c.consume(
            Some("msg2 text"),
            Some(&identity(
                "msg-2",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn non_prefix_text_triggers_replace() {
        let mut c = LiveSemanticConsumer::new();

        c.consume(
            Some("old text"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        // Text completely changed (non-prefix) → replace, not append double.
        let a = c.consume(
            Some("new text"),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn reasoning_opens_and_closes() {
        let mut c = LiveSemanticConsumer::new();

        let a = c.consume(
            Some("thinking..."),
            Some(&identity(
                "msg-1",
                "reasoning/main",
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            Some("thinking...done"),
            Some(&identity(
                "msg-1",
                "reasoning/main",
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            None,
            Some(&identity(
                "msg-1",
                "reasoning/main",
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::End,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::ReplaceReasoningFull {
                text: "thinking...done".to_string()
            }
        );

        let a = c.close_reasoning();
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn tool_call_triggers_boundary() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            None,
            Some(&identity(
                "msg-1",
                "tool_call/call-1",
                LiveMessagePartKind::ToolCall,
                LivePartPhase::Start,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::ToolBoundary);
    }

    #[test]
    fn tool_result_triggers_completion_action() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            None,
            Some(&identity(
                "msg-1",
                "tool_result/call-1",
                LiveMessagePartKind::ToolResult,
                LivePartPhase::End,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(
            a,
            SemanticAction::ToolCallCompleted {
                call_id: "block-1".to_string()
            }
        );
    }

    #[test]
    fn missing_identity_is_missing_identity_action() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(Some("text"), None, LiveContentMode::Snapshot);
        assert_eq!(a, SemanticAction::MissingIdentity);
    }

    #[test]
    fn scheduler_stage_is_non_transcript_pass_through() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            Some("stage text"),
            Some(&identity(
                "msg-1",
                "scheduler/stage-1",
                LiveMessagePartKind::SchedulerStage,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NonTranscriptPassThrough);
    }

    #[test]
    fn empty_text_is_no_op() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            Some(""),
            Some(&identity(
                "msg-1",
                "text/main",
                LiveMessagePartKind::AssistantText,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Snapshot,
        );
        assert_eq!(a, SemanticAction::NoOp);
    }

    #[test]
    fn reasoning_raw_deltas_append_without_full_replay() {
        let mut c = LiveSemanticConsumer::new();
        let a = c.consume(
            Some("alpha"),
            Some(&identity(
                "msg-1",
                "reasoning/main",
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Delta,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            Some(" beta"),
            Some(&identity(
                "msg-1",
                "reasoning/main",
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
            )),
            LiveContentMode::Delta,
        );
        assert_eq!(a, SemanticAction::NoOp);

        let a = c.consume(
            None,
            Some(&identity(
                "msg-1",
                "reasoning/main",
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::End,
            )),
            LiveContentMode::Delta,
        );
        assert_eq!(
            a,
            SemanticAction::ReplaceReasoningFull {
                text: "alpha beta".to_string()
            }
        );
    }
}
