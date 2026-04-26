use crate::runtime::events::{
    CancelToken, FinishReason, LoopError, LoopEvent, LoopOutcome, LoopRequest, StepBoundary,
    StepUsage, ToolCallReady, ToolResult,
};
use crate::runtime::normalizer;
use crate::runtime::policy::{LoopPolicy, ToolDedupScope, ToolErrorStrategy};
use crate::runtime::traits::{LoopSink, ModelCaller, ToolDispatcher};
use futures::StreamExt;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Internal conversation state – uses only rocode_provider types.
// ---------------------------------------------------------------------------

struct LoopConversation {
    messages: Vec<rocode_provider::Message>,
}

impl LoopConversation {
    fn from_messages(messages: Vec<rocode_provider::Message>) -> Self {
        Self { messages }
    }

    fn messages(&self) -> &[rocode_provider::Message] {
        &self.messages
    }

    fn add_assistant_turn(&mut self, reasoning: &str, text: &str, tool_calls: &[ToolCallReady]) {
        if reasoning.is_empty() && tool_calls.is_empty() {
            self.messages
                .push(rocode_provider::Message::assistant(text.to_string()));
            return;
        }

        let mut parts = Vec::new();
        if !reasoning.is_empty() {
            parts.push(rocode_provider::ContentPart {
                content_type: "reasoning".to_string(),
                text: Some(reasoning.to_string()),
                image_url: None,
                tool_use: None,
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }
        if !text.is_empty() {
            parts.push(rocode_provider::ContentPart {
                content_type: "text".to_string(),
                text: Some(text.to_string()),
                image_url: None,
                tool_use: None,
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }
        for tc in tool_calls {
            parts.push(rocode_provider::ContentPart {
                content_type: "tool_use".to_string(),
                text: None,
                image_url: None,
                tool_use: Some(rocode_provider::ToolUse {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    input: tc.arguments.clone(),
                }),
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }
        self.messages.push(rocode_provider::Message {
            role: rocode_provider::Role::Assistant,
            content: rocode_provider::Content::Parts(parts),
            cache_control: None,
            provider_options: None,
        });
    }

    fn add_tool_result(&mut self, call_id: &str, output: &str, is_error: bool) {
        self.messages.push(rocode_provider::Message {
            role: rocode_provider::Role::Tool,
            content: rocode_provider::Content::Parts(vec![rocode_provider::ContentPart {
                content_type: "tool_result".to_string(),
                text: None,
                image_url: None,
                tool_use: None,
                tool_result: Some(rocode_provider::ToolResult {
                    tool_use_id: call_id.to_string(),
                    content: output.to_string(),
                    is_error: Some(is_error),
                }),
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            }]),
            cache_control: None,
            provider_options: None,
        });
    }
}

// ---------------------------------------------------------------------------
// run_loop – the single source of truth for the agentic execution cycle.
//
// Push-based: events are dispatched to LoopSink immediately, never buffered.
//
// Cancellation checkpoints (3 fixed positions):
//   1. Before model call
//   2. After each stream event
//   3. Before each tool dispatch
//
// Observability: tracing spans carry session_id, step, tool_call_id,
// finish_reason consistently.
// ---------------------------------------------------------------------------

pub async fn run_loop<S: LoopSink>(
    model: &dyn ModelCaller,
    tools: &dyn ToolDispatcher,
    sink: &mut S,
    policy: &LoopPolicy,
    cancel: &dyn CancelToken,
    messages: Vec<rocode_provider::Message>,
) -> Result<LoopOutcome, LoopError> {
    let mut conversation = LoopConversation::from_messages(messages);
    let mut step: u32 = 0;
    let mut total_tool_calls: u32 = 0;
    let mut content = String::new();

    // Global dedup set (only used when policy.tool_dedup == Global).
    let mut global_executed_ids: HashSet<String> = HashSet::new();

    while policy
        .max_steps
        .map(|max_steps| step < max_steps)
        .unwrap_or(true)
    {
        step += 1;
        let span = tracing::info_span!("runtime_loop_step", step = step);
        let _enter = span.enter();

        // ── Cancellation checkpoint 1: before model call ──────────────
        if cancel.is_cancelled() {
            tracing::info!(step, "cancelled before model call");
            return Ok(LoopOutcome {
                content,
                total_steps: step,
                total_tool_calls,
                finish_reason: FinishReason::Cancelled,
            });
        }

        // ── Step start ────────────────────────────────────────────────
        sink.on_step_boundary(&StepBoundary::Start { step })
            .await
            .map_err(|e| LoopError::SinkError(e.to_string()))?;

        // ── Build request and call model ──────────────────────────────
        let tool_defs = tools.list_definitions().await;
        let req = LoopRequest {
            messages: conversation.messages().to_vec(),
            tools: tool_defs,
        };

        let raw_stream = model.call_stream(req).await?;
        // Wrap with assemble_tool_calls to normalize Start+Delta→End.
        let mut stream = rocode_provider::assemble_tool_calls(raw_stream);

        // ── Consume stream: normalize → dispatch to sink ─────────────
        let mut step_content = String::new();
        let mut step_reasoning = String::new();
        let mut step_tool_calls: Vec<ToolCallReady> = Vec::new();
        let mut step_usage: Option<StepUsage> = None;
        let mut had_error = false;

        while let Some(event_result) = stream.next().await {
            // ── Cancellation checkpoint 2: after each event ───────────
            if cancel.is_cancelled() {
                tracing::info!(step, "cancelled during stream consumption");
                sink.on_step_boundary(&StepBoundary::End {
                    step,
                    finish_reason: FinishReason::Cancelled,
                    tool_calls_count: 0,
                    had_error,
                    usage: step_usage,
                })
                .await
                .map_err(|e| LoopError::SinkError(e.to_string()))?;
                return Ok(LoopOutcome {
                    content,
                    total_steps: step,
                    total_tool_calls,
                    finish_reason: FinishReason::Cancelled,
                });
            }

            match event_result {
                Ok(stream_event) => {
                    let loop_events = normalizer::normalize(stream_event);
                    for loop_event in loop_events {
                        sink.on_event(&loop_event)
                            .await
                            .map_err(|e| LoopError::SinkError(e.to_string()))?;

                        match loop_event {
                            LoopEvent::TextChunk(text) => step_content.push_str(&text),
                            LoopEvent::ReasoningChunk { text, .. } => {
                                step_reasoning.push_str(&text)
                            }
                            LoopEvent::ToolCallReady(tc) => step_tool_calls.push(tc),
                            LoopEvent::StepDone { usage: Some(u), .. } => {
                                if let Some(existing) = step_usage.as_mut() {
                                    existing.merge_snapshot(&u);
                                } else {
                                    step_usage = Some(u);
                                }
                            }
                            LoopEvent::StepDone { usage: None, .. } => {}
                            LoopEvent::Error(_) => had_error = true,
                            _ => {}
                        }
                    }
                }
                Err(provider_err) => {
                    let err_msg = provider_err.to_string();
                    let err_event = LoopEvent::Error(err_msg.clone());
                    sink.on_event(&err_event)
                        .await
                        .map_err(|e| LoopError::SinkError(e.to_string()))?;
                    sink.on_step_boundary(&StepBoundary::End {
                        step,
                        finish_reason: FinishReason::Error(err_msg.clone()),
                        tool_calls_count: 0,
                        had_error: true,
                        usage: step_usage,
                    })
                    .await
                    .map_err(|e| LoopError::SinkError(e.to_string()))?;
                    return Err(LoopError::ModelError(err_msg));
                }
            }
        }

        // Keep latest content for the outcome.
        content = step_content.clone();

        // ── No tool calls → model finished ───────────────────────────
        if step_tool_calls.is_empty() {
            conversation.add_assistant_turn(&step_reasoning, &step_content, &[]);
            sink.on_step_boundary(&StepBoundary::End {
                step,
                finish_reason: FinishReason::EndTurn,
                tool_calls_count: 0,
                had_error,
                usage: step_usage,
            })
            .await
            .map_err(|e| LoopError::SinkError(e.to_string()))?;

            return Ok(LoopOutcome {
                content,
                total_steps: step,
                total_tool_calls,
                finish_reason: FinishReason::EndTurn,
            });
        }

        // ── Has tool calls → execute them ────────────────────────────
        conversation.add_assistant_turn(&step_reasoning, &step_content, &step_tool_calls);
        let step_tc_count = step_tool_calls.len() as u32;
        total_tool_calls += step_tc_count;

        // Per-step dedup set (only used when policy.tool_dedup == PerStep).
        let mut step_executed_ids: HashSet<String> = HashSet::new();

        for call in &step_tool_calls {
            // ── Cancellation checkpoint 3: before tool dispatch ───────
            if cancel.is_cancelled() {
                tracing::info!(
                    step,
                    tool_call_id = %call.id,
                    "cancelled before tool dispatch"
                );
                sink.on_step_boundary(&StepBoundary::End {
                    step,
                    finish_reason: FinishReason::Cancelled,
                    tool_calls_count: step_tc_count,
                    had_error,
                    usage: step_usage.clone(),
                })
                .await
                .map_err(|e| LoopError::SinkError(e.to_string()))?;
                return Ok(LoopOutcome {
                    content,
                    total_steps: step,
                    total_tool_calls,
                    finish_reason: FinishReason::Cancelled,
                });
            }

            // ── Dedup check ──────────────────────────────────────────
            let should_execute = match policy.tool_dedup {
                ToolDedupScope::Global => global_executed_ids.insert(call.id.clone()),
                ToolDedupScope::PerStep => step_executed_ids.insert(call.id.clone()),
                ToolDedupScope::None => true,
            };

            if !should_execute {
                tracing::warn!(
                    tool_call_id = %call.id,
                    tool_name = %call.name,
                    "skipping duplicate tool call"
                );
                let skip_result = ToolResult {
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    output: "(skipped: duplicate tool_call_id)".to_string(),
                    is_error: false,
                    title: None,
                    metadata: None,
                };
                sink.on_tool_result(call, &skip_result)
                    .await
                    .map_err(|e| LoopError::SinkError(e.to_string()))?;
                conversation.add_tool_result(&call.id, &skip_result.output, false);
                continue;
            }

            let tool_span = tracing::info_span!(
                "tool_dispatch",
                step = step,
                tool_call_id = %call.id,
                tool_name = %call.name,
            );
            let result = {
                let _enter = tool_span.enter();
                tools.execute(call).await
            };

            // ── Handle tool error per policy ─────────────────────────
            if result.is_error {
                match policy.on_tool_error {
                    ToolErrorStrategy::Fail => {
                        sink.on_step_boundary(&StepBoundary::End {
                            step,
                            finish_reason: FinishReason::Error(result.output.clone()),
                            tool_calls_count: step_tc_count,
                            had_error: true,
                            usage: step_usage.clone(),
                        })
                        .await
                        .map_err(|e| LoopError::SinkError(e.to_string()))?;
                        return Err(LoopError::ToolDispatchError {
                            tool: call.name.clone(),
                            error: result.output.clone(),
                        });
                    }
                    ToolErrorStrategy::Skip => {
                        tracing::warn!(
                            tool_call_id = %call.id,
                            error = %result.output,
                            "skipping failed tool call (policy: Skip)"
                        );
                        let skip_output = format!("(skipped: {})", result.output);
                        let skip_result = ToolResult {
                            tool_call_id: call.id.clone(),
                            tool_name: call.name.clone(),
                            output: skip_output.clone(),
                            is_error: true,
                            title: None,
                            metadata: None,
                        };
                        sink.on_tool_result(call, &skip_result)
                            .await
                            .map_err(|e| LoopError::SinkError(e.to_string()))?;
                        conversation.add_tool_result(&call.id, &skip_output, true);
                        continue;
                    }
                    ToolErrorStrategy::ReportAndContinue => {
                        // Fall through to normal result handling.
                    }
                }
            }

            sink.on_tool_result(call, &result)
                .await
                .map_err(|e| LoopError::SinkError(e.to_string()))?;

            conversation.add_tool_result(&call.id, &result.output, result.is_error);
        }

        // ── Step end ─────────────────────────────────────────────────
        sink.on_step_boundary(&StepBoundary::End {
            step,
            finish_reason: FinishReason::ToolUse,
            tool_calls_count: step_tc_count,
            had_error,
            usage: step_usage,
        })
        .await
        .map_err(|e| LoopError::SinkError(e.to_string()))?;
    }

    // ── Max steps exceeded ────────────────────────────────────────────
    tracing::warn!(
        max_steps = policy.max_steps,
        "runtime loop max steps exceeded"
    );
    Ok(LoopOutcome {
        content,
        total_steps: step,
        total_tool_calls,
        finish_reason: FinishReason::MaxSteps,
    })
}
