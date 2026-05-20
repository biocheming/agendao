use crate::runtime::events::{
    CancelToken, FinishReason, LoopError, LoopEvent, LoopOutcome, LoopRequest, ModelFailure,
    RequestViewMetrics, RequestViewMutation, RequestViewMutationKind, StepBoundary,
    StepCheckpointDirective, StepCheckpointSnapshot, StepUsage, ToolCallReady, ToolResult,
};
use crate::runtime::normalizer;
use crate::runtime::policy::{LoopPolicy, ModelContextLimits, ToolDedupScope, ToolErrorStrategy};
use crate::runtime::traits::{LoopSink, ModelCaller, ToolDispatcher};
use futures::StreamExt;
use rocode_provider::is_retryable_stream_error_message;
use std::collections::HashSet;
use tracing::Instrument;

// ---------------------------------------------------------------------------
// Internal conversation state – uses only rocode_provider types.
// ---------------------------------------------------------------------------

const CHECKPOINT_COMPACTION_MAX_SUMMARY_CHARS: usize = 500;
const CHECKPOINT_COMPACTION_MAX_PART_CHARS: usize = 160;
const MAX_INITIAL_STREAM_RETRIES: u32 = 1;

fn can_retry_initial_stream_fault(
    error: &rocode_provider::ProviderError,
    saw_visible_stream_output: bool,
) -> bool {
    if saw_visible_stream_output {
        return false;
    }
    match error {
        rocode_provider::ProviderError::Timeout
        | rocode_provider::ProviderError::NetworkError(_) => true,
        rocode_provider::ProviderError::StreamError(message) => {
            is_retryable_stream_error_message(message)
        }
        _ => false,
    }
}

fn stream_termination_from_provider_error(
    error: &rocode_provider::ProviderError,
) -> rocode_provider::StreamTermination {
    match error {
        rocode_provider::ProviderError::NetworkError(_) | rocode_provider::ProviderError::Timeout => {
            rocode_provider::StreamTermination::TransportClosed
        }
        rocode_provider::ProviderError::StreamError(message) => {
            rocode_provider::StreamTermination::StreamCorrupt {
                message: message.clone(),
            }
        }
        other => rocode_provider::StreamTermination::ProviderError {
            message: other.to_string(),
        },
    }
}

struct LoopConversation<'a> {
    messages: &'a mut Vec<rocode_provider::Message>,
}

struct CheckpointCompactionResult {
    summary: String,
    compacted_message_count: usize,
}

impl<'a> LoopConversation<'a> {
    fn from_messages(messages: &'a mut Vec<rocode_provider::Message>) -> Self {
        Self { messages }
    }

    fn messages(&self) -> &[rocode_provider::Message] {
        self.messages.as_slice()
    }

    fn add_assistant_turn(&mut self, reasoning: &str, text: &str, tool_calls: &[ToolCallReady]) {
        let provider_tool_calls: Vec<rocode_provider::ToolUse> = tool_calls
            .iter()
            .map(|tc| rocode_provider::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.arguments.clone(),
            })
            .collect();

        if let Some(message) = rocode_provider::Message::assistant_turn(
            Some(reasoning),
            Some(text),
            &provider_tool_calls,
        ) {
            self.messages.push(message);
        }
    }

    fn add_tool_result(&mut self, call_id: &str, output: &str, is_error: bool) {
        self.messages
            .push(rocode_provider::Message::tool_parts(vec![
                rocode_provider::ContentPart::tool_result(
                    call_id.to_string(),
                    output.to_string(),
                    Some(is_error),
                ),
            ]));
    }

    fn replace_messages(&mut self, messages: Vec<rocode_provider::Message>) {
        *self.messages = messages;
    }

    fn compact_for_checkpoint(
        &mut self,
        focus: Option<&str>,
        min_compactable_messages: usize,
    ) -> Option<CheckpointCompactionResult> {
        let system_prefix_len = self
            .messages()
            .iter()
            .take_while(|message| matches!(message.role, rocode_provider::Role::System))
            .count();
        let compactable = &self.messages()[system_prefix_len..];
        if compactable.len() < min_compactable_messages {
            return None;
        }

        let keep_count = compactable.len() / 2;
        let compacted_count = compactable.len().saturating_sub(keep_count);
        let summary = build_checkpoint_compaction_summary(&compactable[..compacted_count], focus);

        let mut rewritten = self.messages()[..system_prefix_len].to_vec();
        rewritten.push(rocode_provider::Message::assistant(summary.clone()));
        rewritten.extend_from_slice(&compactable[compacted_count..]);
        self.replace_messages(rewritten);
        Some(CheckpointCompactionResult {
            summary,
            compacted_message_count: compacted_count,
        })
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut truncated = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        truncated.push_str("...");
    }
    truncated
}

fn compact_json_preview(value: &serde_json::Value) -> String {
    truncate_chars(&value.to_string(), CHECKPOINT_COMPACTION_MAX_PART_CHARS)
}

fn provider_message_summary_fragments(message: &rocode_provider::Message) -> Vec<String> {
    match &message.content {
        rocode_provider::Content::Text(text) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text.trim().to_string()]
            }
        }
        rocode_provider::Content::Parts(parts) => parts
            .iter()
            .filter_map(|part| match part.content_type.as_str() {
                "text" | "reasoning" => part
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string),
                "tool_use" => part.tool_use.as_ref().map(|tool_use| {
                    format!(
                        "tool_use {} {}",
                        tool_use.name,
                        compact_json_preview(&tool_use.input)
                    )
                }),
                "tool_result" => part.tool_result.as_ref().map(|tool_result| {
                    format!(
                        "tool_result {} {}",
                        tool_result.tool_use_id,
                        truncate_chars(&tool_result.content, CHECKPOINT_COMPACTION_MAX_PART_CHARS)
                    )
                }),
                "image_url" | "file" => part
                    .filename
                    .as_deref()
                    .map(str::to_string)
                    .or_else(|| part.image_url.as_ref().map(|image| image.url.clone()))
                    .map(|value| format!("attachment {value}")),
                _ => part
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string),
            })
            .collect(),
    }
}

fn build_checkpoint_compaction_summary(
    messages: &[rocode_provider::Message],
    focus: Option<&str>,
) -> String {
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
    let fragments: Vec<String> = messages
        .iter()
        .flat_map(provider_message_summary_fragments)
        .collect();
    let selected = if focus_terms.is_empty() {
        fragments
    } else {
        let focused: Vec<String> = fragments
            .iter()
            .filter(|fragment| {
                let lowercase = fragment.to_ascii_lowercase();
                focus_terms.iter().any(|term| lowercase.contains(term))
            })
            .cloned()
            .collect();
        if focused.is_empty() {
            fragments
        } else {
            focused
        }
    };
    let summary_body = truncate_chars(&selected.join(" "), CHECKPOINT_COMPACTION_MAX_SUMMARY_CHARS);
    if let Some(focus) = focus {
        format!(
            "Checkpoint context summary of {} earlier messages. Focused on `{focus}`. Summary: {}",
            messages.len(),
            summary_body
        )
    } else {
        format!(
            "Checkpoint context summary of {} earlier messages. Summary: {}",
            messages.len(),
            summary_body
        )
    }
}

fn provider_message_context_char_len(message: &rocode_provider::Message) -> usize {
    match &message.content {
        rocode_provider::Content::Text(text) => text.chars().count(),
        rocode_provider::Content::Parts(parts) => parts
            .iter()
            .map(|part| {
                part.text
                    .as_ref()
                    .map(|text| text.chars().count())
                    .unwrap_or(0)
                    + part
                        .tool_use
                        .as_ref()
                        .map(|tool_use| {
                            tool_use.name.chars().count()
                                + tool_use.input.to_string().chars().count()
                        })
                        .unwrap_or(0)
                    + part
                        .tool_result
                        .as_ref()
                        .map(|tool_result| {
                            tool_result.tool_use_id.chars().count()
                                + tool_result.content.chars().count()
                        })
                        .unwrap_or(0)
                    + part
                        .image_url
                        .as_ref()
                        .map(|image| image.url.chars().count())
                        .unwrap_or(0)
                    + part
                        .filename
                        .as_ref()
                        .map(|filename| filename.chars().count())
                        .unwrap_or(0)
                    + part
                        .media_type
                        .as_ref()
                        .map(|media_type| media_type.chars().count())
                        .unwrap_or(0)
            })
            .sum(),
    }
}

fn is_checkpoint_summary_message(message: &rocode_provider::Message) -> bool {
    matches!(
        (&message.role, &message.content),
        (rocode_provider::Role::Assistant, rocode_provider::Content::Text(text))
            if text.starts_with("Checkpoint context summary of")
    )
}

fn request_view_metrics(messages: &[rocode_provider::Message]) -> RequestViewMetrics {
    let body_chars: usize = messages.iter().map(provider_message_context_char_len).sum();
    let system_prefix_messages = messages
        .iter()
        .take_while(|message| matches!(message.role, rocode_provider::Role::System))
        .count();
    let compactable_messages = messages.len().saturating_sub(system_prefix_messages);
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut tool_messages = 0usize;
    let mut checkpoint_summary_messages = 0usize;
    for message in messages {
        match message.role {
            rocode_provider::Role::System => {}
            rocode_provider::Role::User => user_messages += 1,
            rocode_provider::Role::Assistant => {
                assistant_messages += 1;
                if is_checkpoint_summary_message(message) {
                    checkpoint_summary_messages += 1;
                }
            }
            rocode_provider::Role::Tool => tool_messages += 1,
        }
    }

    RequestViewMetrics {
        message_count: messages.len(),
        system_prefix_messages,
        compactable_messages,
        user_messages,
        assistant_messages,
        tool_messages,
        checkpoint_summary_messages,
        estimated_context_tokens: (body_chars > 0).then_some((body_chars as u64) / 4),
        estimated_body_chars: (body_chars > 0).then_some(body_chars),
    }
}

#[derive(Default)]
struct StepCheckpointCollector {
    max_assessments: usize,
    assessments: Vec<RequestViewMetrics>,
    prior_mutations: Vec<RequestViewMutation>,
}

impl StepCheckpointCollector {
    fn new(max_assessments: usize) -> Self {
        Self {
            max_assessments,
            assessments: Vec::new(),
            prior_mutations: Vec::new(),
        }
    }

    fn snapshot(&mut self, messages: &[rocode_provider::Message]) -> StepCheckpointSnapshot {
        let current_view = request_view_metrics(messages);
        let previous_view = self.assessments.last().cloned();
        self.assessments.push(current_view.clone());
        StepCheckpointSnapshot {
            assessment_index: self.assessments.len(),
            max_assessments: self.max_assessments,
            current_view,
            previous_view,
            prior_mutations: self.prior_mutations.clone(),
        }
    }

    fn can_mutate_after_current_assessment(&self) -> bool {
        self.assessments.len() < self.max_assessments
    }

    fn record_compaction(
        &mut self,
        before: RequestViewMetrics,
        after: RequestViewMetrics,
        focus: Option<String>,
        reason: Option<String>,
        result: &CheckpointCompactionResult,
    ) {
        self.prior_mutations.push(RequestViewMutation {
            kind: RequestViewMutationKind::Compacted,
            reason,
            focus,
            before,
            after,
            compacted_message_count: Some(result.compacted_message_count),
            summary_chars: Some(result.summary.chars().count()),
        });
    }

    fn record_replacement(
        &mut self,
        before: RequestViewMetrics,
        after: RequestViewMetrics,
        reason: Option<String>,
    ) {
        self.prior_mutations.push(RequestViewMutation {
            kind: RequestViewMutationKind::Replaced,
            reason,
            focus: None,
            before,
            after,
            compacted_message_count: None,
            summary_chars: None,
        });
    }
}

async fn run_step_checkpoint_cycle<S: LoopSink>(
    conversation: &mut LoopConversation<'_>,
    sink: &mut S,
    policy: &LoopPolicy,
    model_context_limits: Option<ModelContextLimits>,
    end_boundary: &StepBoundary,
    step_usage: Option<&StepUsage>,
    strict: bool,
) -> Result<(), LoopError> {
    let mut checkpoint_collector =
        StepCheckpointCollector::new(policy.checkpoint_governance.max_assessments);
    loop {
        let checkpoint = checkpoint_collector.snapshot(conversation.messages());
        let default_directive = policy.checkpoint_governance.default_directive(
            model_context_limits,
            step_usage,
            &checkpoint,
        );
        let directive = sink
            .on_step_checkpoint(
                end_boundary,
                conversation.messages(),
                &checkpoint,
                &default_directive,
            )
            .await?
            .unwrap_or(default_directive);
        match directive {
            StepCheckpointDirective::Continue => break,
            StepCheckpointDirective::Block { reason } => {
                if strict {
                    return Err(LoopError::Other(reason));
                }
                tracing::warn!(reason, "ignoring final step checkpoint block");
                break;
            }
            StepCheckpointDirective::CompactRequestView { focus, reason } => {
                if !checkpoint_collector.can_mutate_after_current_assessment() {
                    if strict {
                        return Err(LoopError::Other(reason.unwrap_or_else(|| {
                            "step checkpoint exceeded the request-view compaction retry budget"
                                .to_string()
                        })));
                    }
                    tracing::warn!(
                        "ignoring final step checkpoint compaction request after retry budget was exhausted"
                    );
                    break;
                }
                let before = checkpoint.current_view.clone();
                let compacted = conversation.compact_for_checkpoint(
                    focus.as_deref(),
                    policy.checkpoint_governance.min_compactable_messages,
                );
                let Some(compacted) = compacted else {
                    if strict {
                        return Err(LoopError::Other(reason.unwrap_or_else(|| {
                            "step checkpoint requested request-view compaction, but no compactable history was available".to_string()
                        })));
                    }
                    tracing::warn!(
                        "ignoring final step checkpoint compaction request because no compactable history was available"
                    );
                    break;
                };
                let after = request_view_metrics(conversation.messages());
                checkpoint_collector.record_compaction(before, after, focus, reason, &compacted);
            }
            StepCheckpointDirective::ReplaceRequestView { messages, reason } => {
                if !checkpoint_collector.can_mutate_after_current_assessment() {
                    if strict {
                        return Err(LoopError::Other(reason.unwrap_or_else(|| {
                            "step checkpoint exceeded the request-view replacement retry budget"
                                .to_string()
                        })));
                    }
                    tracing::warn!(
                        "ignoring final step checkpoint replacement request after retry budget was exhausted"
                    );
                    break;
                }
                let before = checkpoint.current_view.clone();
                conversation.replace_messages(messages);
                let after = request_view_metrics(conversation.messages());
                checkpoint_collector.record_replacement(before, after, reason);
            }
        }
    }

    Ok(())
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
    messages: &mut Vec<rocode_provider::Message>,
) -> Result<LoopOutcome, LoopError> {
    let mut conversation = LoopConversation::from_messages(messages);
    let mut step: u32 = 0;
    let mut total_tool_calls: u32 = 0;
    let mut content = String::new();
    let model_context_limits = model.context_limits();

    // Global dedup set (only used when policy.tool_dedup == Global).
    let mut global_executed_ids: HashSet<String> = HashSet::new();

    while policy
        .max_steps
        .map(|max_steps| step < max_steps)
        .unwrap_or(true)
    {
        step += 1;
        tracing::debug!(step, "runtime loop step started");

        // ── Cancellation checkpoint 1: before model call ──────────────
        if cancel.is_cancelled() {
            tracing::info!(step, "cancelled before model call");
            return Ok(LoopOutcome {
                content,
                total_steps: step,
                total_tool_calls,
                finish_reason: FinishReason::Cancelled,
                stream_termination: None,
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

        // ── Consume stream: normalize → dispatch to sink ─────────────
        let mut step_content = String::new();
        let mut step_reasoning = String::new();
        let mut step_tool_calls: Vec<ToolCallReady> = Vec::new();
        let mut step_usage: Option<StepUsage>;
        let mut had_error: bool;
        let mut stream_retry_count = 0;

        'stream_attempt: loop {
            step_content.clear();
            step_reasoning.clear();
            step_tool_calls.clear();
            step_usage = None;
            had_error = false;
            let mut saw_visible_stream_output = false;

            let raw_stream = model.call_stream(req.clone()).await?;
            // Wrap with assemble_tool_calls to normalize Start+Delta→End.
            let mut stream = rocode_provider::assemble_tool_calls(raw_stream);

            loop {
                // P3-F: stream idle timeout watchdog. If no event arrives
                // within the configured window, the stream is treated as hung
                // and the step terminates with a TransportTimeout.
                let event_result = if let Some(timeout_ms) = policy.stream_event_timeout_ms {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        stream.next(),
                    )
                    .await
                    {
                        Ok(Some(result)) => result,
                        Ok(None) => break, // stream ended normally
                        Err(_elapsed) => {
                            tracing::warn!(
                                step,
                                timeout_ms,
                                "model stream idle timeout — no event in watchdog window"
                            );
                            let err_msg = format!(
                                "Stream timed out after {}s with no events",
                                timeout_ms / 1000
                            );
                            sink.on_event(&LoopEvent::Error(err_msg.clone()))
                                .await
                                .map_err(|e| LoopError::SinkError(e.to_string()))?;
                            sink.on_step_boundary(&StepBoundary::End {
                                step,
                                finish_reason: FinishReason::Error(err_msg),
                                tool_calls_count: 0,
                                had_error: true,
                                usage: step_usage,
                            })
                            .await
                            .map_err(|e| LoopError::SinkError(e.to_string()))?;
                            return Err(LoopError::ModelErrorWithTermination {
                                failure: ModelFailure::Message(format!(
                                    "stream transport timeout after {}s",
                                    timeout_ms / 1000
                                )),
                                stream_termination: rocode_provider::StreamTermination::TransportTimeout,
                            });
                        }
                    }
                } else {
                    match stream.next().await {
                        Some(result) => result,
                        None => break,
                    }
                };

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
                        stream_termination: Some(rocode_provider::StreamTermination::Interrupted),
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
                                LoopEvent::TextChunk(text) => {
                                    if !text.is_empty() {
                                        saw_visible_stream_output = true;
                                    }
                                    step_content.push_str(&text);
                                }
                                LoopEvent::ReasoningChunk { text, .. } => {
                                    if !text.is_empty() {
                                        saw_visible_stream_output = true;
                                    }
                                    step_reasoning.push_str(&text)
                                }
                                LoopEvent::ToolCallProgress { .. } => {
                                    saw_visible_stream_output = true;
                                }
                                LoopEvent::ToolCallReady(tc) => {
                                    saw_visible_stream_output = true;
                                    step_tool_calls.push(tc);
                                }
                                LoopEvent::StepDone { usage: Some(u), .. } => {
                                    if let Some(existing) = step_usage.as_mut() {
                                        existing.merge_snapshot(&u);
                                    } else {
                                        step_usage = Some(u);
                                    }
                                }
                                LoopEvent::StepDone { usage: None, .. } => {}
                                LoopEvent::Error(_) => had_error = true,
                            }
                        }
                    }
                    Err(provider_err) => {
                        if stream_retry_count < MAX_INITIAL_STREAM_RETRIES
                            && can_retry_initial_stream_fault(
                                &provider_err,
                                saw_visible_stream_output,
                            )
                        {
                            stream_retry_count += 1;
                            tracing::warn!(
                                step,
                                retry = stream_retry_count,
                                error = %provider_err,
                                "retrying model stream after transient stream fault before visible output"
                            );
                            continue 'stream_attempt;
                        }

                        let failure = model.model_failure_from_provider_error(&provider_err);
                        let err_msg = failure.message().to_string();
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
                        return Err(LoopError::ModelErrorWithTermination {
                            failure,
                            stream_termination: stream_termination_from_provider_error(&provider_err),
                        });
                    }
                }
            }

            break 'stream_attempt;
        }

        // Keep latest content for the outcome.
        content = step_content.clone();

        // ── No tool calls → model finished ───────────────────────────
        if step_tool_calls.is_empty() {
            conversation.add_assistant_turn(&step_reasoning, &step_content, &[]);
            let end_step_usage = step_usage.clone();
            let end_boundary = StepBoundary::End {
                step,
                finish_reason: FinishReason::EndTurn,
                tool_calls_count: 0,
                had_error,
                usage: end_step_usage,
            };
            sink.on_step_boundary(&end_boundary)
                .await
                .map_err(|e| LoopError::SinkError(e.to_string()))?;
            run_step_checkpoint_cycle(
                &mut conversation,
                sink,
                policy,
                model_context_limits,
                &end_boundary,
                step_usage.as_ref(),
                false,
            )
            .await?;

            return Ok(LoopOutcome {
                content,
                total_steps: step,
                total_tool_calls,
                finish_reason: FinishReason::EndTurn,
                stream_termination: Some(rocode_provider::StreamTermination::Completed),
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
                    stream_termination: Some(rocode_provider::StreamTermination::Interrupted),
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
            let result = tools.execute(call).instrument(tool_span).await;

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
        let end_boundary = StepBoundary::End {
            step,
            finish_reason: FinishReason::ToolUse,
            tool_calls_count: step_tc_count,
            had_error,
            usage: step_usage.clone(),
        };
        sink.on_step_boundary(&end_boundary)
            .await
            .map_err(|e| LoopError::SinkError(e.to_string()))?;
        run_step_checkpoint_cycle(
            &mut conversation,
            sink,
            policy,
            model_context_limits,
            &end_boundary,
            step_usage.as_ref(),
            true,
        )
        .await?;
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
        stream_termination: Some(rocode_provider::StreamTermination::Completed),
    })
}
