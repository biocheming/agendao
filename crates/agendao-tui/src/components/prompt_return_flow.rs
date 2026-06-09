use crate::context::session_context::TokenUsage;

/// A single prioritized item for the prompt's secondary info strip.
///
/// The strip shows at most one item — the highest-priority piece
/// of information that either flows back from the last turn (compaction,
/// replay, usage) or reflects the current draft state (attachments).
#[derive(Clone, Debug)]
pub enum ReturnFlowItem {
    /// Compaction is currently in progress (phase != `Installed` / `Failed` / `Skipped`).
    CompactionInProgress { percent: Option<u8> },
    /// Compaction just finished (status is `Installed`).
    CompactionJustFinished,
    /// The current prompt is a resend or replay of a previous turn.
    ReplaySource { label: String },
    /// Attachments currently queued on the draft before submission.
    /// This is the current input-side carry state (木), not a last-turn
    /// return flow (水); it shares the secondary strip layer with return-flow
    /// items for visual consistency, not semantic identity.
    CurrentDraftAttachment { count: usize, image_count: usize },
    /// Token usage from the last assistant turn.
    LastTurnUsage { input: u64, output: u64, reasoning: u64 },
}

/// The resolved secondary strip — at most one item, determined by priority.
#[derive(Clone, Debug, Default)]
pub struct ReturnFlowStrip {
    pub primary: Option<ReturnFlowItem>,
}

/// Serialize a [`ReturnFlowItem`] into the compact human-readable line
/// rendered in the prompt area.
pub fn format_return_flow_item(item: &ReturnFlowItem) -> String {
    match item {
        ReturnFlowItem::CompactionInProgress { percent } => {
            if let Some(pct) = percent {
                format!("Compacting conversation {}%", pct)
            } else {
                "Compacting conversation…".to_string()
            }
        }
        ReturnFlowItem::CompactionJustFinished => {
            "Compaction complete".to_string()
        }
        ReturnFlowItem::ReplaySource { label } => {
            label.clone()
        }
        ReturnFlowItem::CurrentDraftAttachment { count, image_count } => {
            if *count == 1 {
                "1 attachment queued for this prompt".to_string()
            } else if *image_count == *count {
                if *image_count == 1 {
                    "1 image queued for this prompt".to_string()
                } else {
                    format!("{} images queued for this prompt", image_count)
                }
            } else {
                format!("{} attachments queued for this prompt", count)
            }
        }
        ReturnFlowItem::LastTurnUsage { input, output, reasoning } => {
            let mut parts = vec![
                format!("↑{}", format_tokens(*input)),
                format!("↓{}", format_tokens(*output)),
            ];
            if *reasoning > 0 {
                parts.push(format!("reason {}", format_tokens(*reasoning)));
            }
            format!("Last turn: {}", parts.join(" "))
        }
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1_000)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Resolve the highest-priority item for the prompt's secondary strip.
///
/// Priority order:
/// 1. Compaction in-progress / just-finished                          (water → wood)
/// 2. Replay / resend source trail                                    (water → wood)
/// 3. Current draft attachment state                                  (wood-side carry)
/// 4. Last-turn usage summary                                         (water → wood)
///
/// Returns `None` when there is nothing to show.
pub fn resolve_return_flow_strip(
    compaction_in_progress: bool,
    compaction_just_finished: bool,
    compaction_percent: Option<u8>,
    replay_source: Option<&str>,
    attachment_count: usize,
    attachment_image_count: usize,
    last_turn_tokens: Option<&TokenUsage>,
) -> Option<ReturnFlowItem> {
    if compaction_in_progress {
        return Some(ReturnFlowItem::CompactionInProgress {
            percent: compaction_percent,
        });
    }

    if compaction_just_finished {
        return Some(ReturnFlowItem::CompactionJustFinished);
    }

    if let Some(label) = replay_source {
        if !label.is_empty() {
            return Some(ReturnFlowItem::ReplaySource {
                label: label.to_string(),
            });
        }
    }

    // Current draft state — not a return flow, but shares the secondary
    // strip layer so the user sees "what is queued" before "what happened".
    if attachment_count > 0 {
        return Some(ReturnFlowItem::CurrentDraftAttachment {
            count: attachment_count,
            image_count: attachment_image_count,
        });
    }

    if let Some(tokens) = last_turn_tokens {
        if tokens.input > 0 || tokens.output > 0 {
            return Some(ReturnFlowItem::LastTurnUsage {
                input: tokens.input,
                output: tokens.output,
                reasoning: tokens.reasoning,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64, reasoning: u64) -> TokenUsage {
        TokenUsage {
            input,
            output,
            reasoning,
            cache_read: 0,
            cache_miss: 0,
            cache_write: 0,
        }
    }

    #[test]
    fn compaction_progress_takes_priority_over_everything() {
        let usage = tokens(1000, 500, 200);
        let item = resolve_return_flow_strip(
            true, false, Some(29),
            None,
            3, 2,
            Some(&usage),
        );
        assert!(matches!(item, Some(ReturnFlowItem::CompactionInProgress { percent: Some(29) })));
    }

    #[test]
    fn compation_just_finished_takes_priority_over_attachment_and_usage() {
        let usage = tokens(1000, 500, 200);
        let item = resolve_return_flow_strip(
            false, true, None,
            None,
            3, 2,
            Some(&usage),
        );
        assert!(matches!(item, Some(ReturnFlowItem::CompactionJustFinished)));
    }

    #[test]
    fn current_draft_attachment_without_higher_priority() {
        let usage = tokens(1000, 500, 200);
        let item = resolve_return_flow_strip(
            false, false, None,
            None,
            2, 1,
            Some(&usage),
        );
        let pending = item.unwrap();
        match pending {
            ReturnFlowItem::CurrentDraftAttachment { count, image_count } => {
                assert_eq!(count, 2);
                assert_eq!(image_count, 1);
            }
            other => panic!("expected CurrentDraftAttachment, got {:?}", other),
        }
    }

    #[test]
    fn last_turn_usage_when_nothing_else() {
        let usage = tokens(12000, 3400, 900);
        let item = resolve_return_flow_strip(
            false, false, None,
            None,
            0, 0,
            Some(&usage),
        );
        let turn = item.unwrap();
        match turn {
            ReturnFlowItem::LastTurnUsage { input, output, reasoning } => {
                assert_eq!(input, 12000);
                assert_eq!(output, 3400);
                assert_eq!(reasoning, 900);
            }
            other => panic!("expected LastTurnUsage, got {:?}", other),
        }
    }

    #[test]
    fn no_item_when_nothing_to_show() {
        let item = resolve_return_flow_strip(
            false, false, None,
            None,
            0, 0,
            None,
        );
        assert!(item.is_none());
    }

    #[test]
    fn attachment_count_zero_falls_to_last_turn_usage() {
        let usage = tokens(500, 200, 0);
        let item = resolve_return_flow_strip(
            false, false, None,
            None,
            0, 0,
            Some(&usage),
        );
        assert!(matches!(item, Some(ReturnFlowItem::LastTurnUsage { .. })));
    }
}
