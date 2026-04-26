use crate::runtime::events::{FinishReason, LoopEvent, StepUsage, ToolCallReady};
use rocode_provider::StreamEvent;

// ---------------------------------------------------------------------------
// EventNormalizer – single source of truth for StreamEvent → LoopEvent.
//
// This is the ONLY place where raw StreamEvent variants are interpreted.
// All three current loop implementations (orchestrator, agent, session)
// have their own interpretation; this normalizer replaces all of them.
// ---------------------------------------------------------------------------

/// Convert a single `StreamEvent` into zero or more `LoopEvent`s.
///
/// Most events map 1:1. Some events (like `Start`, `TextStart`, `TextEnd`)
/// are purely structural and produce no `LoopEvent`. `FinishStep` maps to
/// `StepDone` with usage information.
pub fn normalize(event: StreamEvent) -> Vec<LoopEvent> {
    match event {
        // -- Text ---------------------------------------------------------
        StreamEvent::TextDelta(text) => vec![LoopEvent::TextChunk(text)],

        // -- Reasoning ----------------------------------------------------
        StreamEvent::ReasoningDelta { id, text } => {
            vec![LoopEvent::ReasoningChunk { id, text }]
        }

        // -- Tool call (assembled) ----------------------------------------
        // ToolCallEnd is the only event that produces ToolCallReady.
        // By the time we see this, assemble_tool_calls has already
        // combined Start+Delta into End for providers that stream fragments.
        StreamEvent::ToolCallEnd { id, name, input } => {
            if name.trim().is_empty() {
                tracing::warn!(tool_call_id = %id, "normalizer: ignoring ToolCallEnd with empty tool name");
                return vec![];
            }
            vec![LoopEvent::ToolCallReady(ToolCallReady {
                id,
                name,
                arguments: input,
            })]
        }

        // -- Tool call progress (streaming fragments) ---------------------
        StreamEvent::ToolCallStart { id, name } => {
            if name.trim().is_empty() {
                return vec![];
            }
            vec![LoopEvent::ToolCallProgress {
                id,
                name: Some(name),
                partial_input: String::new(),
            }]
        }
        StreamEvent::ToolCallDelta { id, input } => {
            vec![LoopEvent::ToolCallProgress {
                id,
                name: None,
                partial_input: input,
            }]
        }

        // -- Tool input (alternative streaming path) ----------------------
        StreamEvent::ToolInputStart { id, tool_name } => {
            if tool_name.trim().is_empty() {
                return vec![];
            }
            vec![LoopEvent::ToolCallProgress {
                id,
                name: Some(tool_name),
                partial_input: String::new(),
            }]
        }
        StreamEvent::ToolInputDelta { id, delta } => {
            vec![LoopEvent::ToolCallProgress {
                id,
                name: None,
                partial_input: delta,
            }]
        }
        StreamEvent::ToolInputEnd { .. } => {
            // The actual assembled tool call will come as ToolCallEnd.
            vec![]
        }

        // -- Step completion ----------------------------------------------
        StreamEvent::FinishStep {
            finish_reason,
            usage,
            ..
        } => {
            let reason = match finish_reason.as_deref() {
                Some("stop") => FinishReason::EndTurn,
                Some("tool-calls") | Some("tool_calls") => FinishReason::ToolUse,
                Some(other) => FinishReason::Provider(other.to_string()),
                None => FinishReason::EndTurn,
            };
            let step_usage = StepUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                context_tokens: usage.context_tokens.max(usage.prompt_tokens),
                reasoning_tokens: usage.reasoning_tokens,
                cache_read_tokens: usage.cache_read_tokens,
                cache_write_tokens: usage.cache_write_tokens,
            };
            vec![LoopEvent::StepDone {
                finish_reason: reason,
                usage: Some(step_usage),
            }]
        }

        StreamEvent::Usage {
            prompt_tokens,
            completion_tokens,
            context_tokens,
        } => {
            // Standalone usage event (e.g. ethnopic message_start).
            // Emit as StepDone-like usage update without changing finish reason.
            // The loop accumulates usage; this is informational for the sink.
            vec![LoopEvent::StepDone {
                finish_reason: FinishReason::EndTurn,
                usage: Some(StepUsage {
                    prompt_tokens,
                    completion_tokens,
                    context_tokens: context_tokens.max(prompt_tokens),
                    ..Default::default()
                }),
            }]
        }

        // -- Errors -------------------------------------------------------
        StreamEvent::Error(msg) => vec![LoopEvent::Error(msg)],

        // -- Terminal events ----------------------------------------------
        StreamEvent::Done | StreamEvent::Finish => {
            // These signal stream end. The loop detects end-of-stream via
            // `stream.next().await` returning None (after assemble_tool_calls
            // flushes). No LoopEvent needed.
            vec![]
        }

        // -- ToolResult / ToolError from provider -------------------------
        // These are provider-side tool results (e.g. server-side execution).
        // Not used in the current agentic loop (tools execute locally).
        // Pass through as informational events.
        StreamEvent::ToolResult {
            tool_call_id,
            tool_name,
            ..
        } => {
            tracing::debug!(
                tool_call_id = %tool_call_id,
                tool_name = %tool_name,
                "normalizer: provider-side ToolResult (passthrough)"
            );
            vec![]
        }
        StreamEvent::ToolError {
            tool_call_id,
            error,
            ..
        } => {
            tracing::warn!(
                tool_call_id = %tool_call_id,
                error = %error,
                "normalizer: provider-side ToolError"
            );
            vec![LoopEvent::Error(format!(
                "provider tool error [{}]: {}",
                tool_call_id, error
            ))]
        }

        // -- Structural events (no semantic content) ----------------------
        StreamEvent::Start
        | StreamEvent::TextStart
        | StreamEvent::TextEnd
        | StreamEvent::ReasoningStart { .. }
        | StreamEvent::ReasoningEnd { .. }
        | StreamEvent::StartStep => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_maps_to_text_chunk() {
        let events = normalize(StreamEvent::TextDelta("hello".into()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LoopEvent::TextChunk(t) if t == "hello"));
    }

    #[test]
    fn tool_call_end_maps_to_tool_call_ready() {
        let events = normalize(StreamEvent::ToolCallEnd {
            id: "tc-1".into(),
            name: "read".into(),
            input: serde_json::json!({"path": "/tmp/a"}),
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LoopEvent::ToolCallReady(tc) if tc.name == "read"));
    }

    #[test]
    fn empty_tool_name_is_filtered() {
        let events = normalize(StreamEvent::ToolCallEnd {
            id: "tc-2".into(),
            name: "  ".into(),
            input: serde_json::json!({}),
        });
        assert!(events.is_empty());
    }

    #[test]
    fn tool_call_start_maps_to_progress() {
        let events = normalize(StreamEvent::ToolCallStart {
            id: "tc-3".into(),
            name: "write".into(),
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            LoopEvent::ToolCallProgress { id, name: Some(n), .. } if id == "tc-3" && n == "write"
        ));
    }

    #[test]
    fn finish_step_stop_maps_to_end_turn() {
        let events = normalize(StreamEvent::FinishStep {
            finish_reason: Some("stop".into()),
            usage: rocode_provider::StreamUsage::default(),
            provider_metadata: None,
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            LoopEvent::StepDone {
                finish_reason: FinishReason::EndTurn,
                ..
            }
        ));
    }

    #[test]
    fn finish_step_tool_calls_maps_to_tool_use() {
        let events = normalize(StreamEvent::FinishStep {
            finish_reason: Some("tool-calls".into()),
            usage: rocode_provider::StreamUsage::default(),
            provider_metadata: None,
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            LoopEvent::StepDone {
                finish_reason: FinishReason::ToolUse,
                ..
            }
        ));
    }

    #[test]
    fn done_produces_no_events() {
        assert!(normalize(StreamEvent::Done).is_empty());
    }

    #[test]
    fn structural_events_produce_nothing() {
        assert!(normalize(StreamEvent::Start).is_empty());
        assert!(normalize(StreamEvent::TextStart).is_empty());
        assert!(normalize(StreamEvent::TextEnd).is_empty());
        assert!(normalize(StreamEvent::ReasoningStart { id: "r-1".into() }).is_empty());
        assert!(normalize(StreamEvent::ReasoningEnd { id: "r-1".into() }).is_empty());
        assert!(normalize(StreamEvent::StartStep).is_empty());
    }

    #[test]
    fn reasoning_delta_maps_correctly() {
        let events = normalize(StreamEvent::ReasoningDelta {
            id: "r-1".into(),
            text: "thinking...".into(),
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            LoopEvent::ReasoningChunk { id, text } if id == "r-1" && text == "thinking..."
        ));
    }

    #[test]
    fn tool_input_start_maps_to_progress() {
        let events = normalize(StreamEvent::ToolInputStart {
            id: "ti-1".into(),
            tool_name: "bash".into(),
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            LoopEvent::ToolCallProgress { id, name: Some(n), .. } if id == "ti-1" && n == "bash"
        ));
    }

    #[test]
    fn tool_input_end_produces_nothing() {
        assert!(normalize(StreamEvent::ToolInputEnd { id: "ti-1".into() }).is_empty());
    }

    #[test]
    fn error_event_maps_to_error() {
        let events = normalize(StreamEvent::Error("boom".into()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LoopEvent::Error(e) if e == "boom"));
    }
}
