use std::collections::HashMap;

use rocode_orchestrator::runtime::events::{
    FinishReason as RuntimeFinishReason, LoopError as RuntimeLoopError, LoopEvent, StepBoundary,
    ToolCallReady as RuntimeToolCallReady, ToolResult as RuntimeToolResult,
};
use rocode_orchestrator::runtime::traits::LoopSink;
use rocode_orchestrator::{classify_tool_error_message, ERROR_CAUSE_KEY};

use super::{AgentError, AgentRenderEvent, AgentRenderOutcome, AgentToolOutput};

pub(super) fn map_runtime_loop_error(error: RuntimeLoopError) -> AgentError {
    match error {
        RuntimeLoopError::ModelError(failure) => {
            if failure.is_provider_not_found() {
                AgentError::NoProvider
            } else {
                AgentError::ProviderError(failure.to_string())
            }
        }
        RuntimeLoopError::ModelErrorWithTermination { failure, .. } => {
            if failure.is_provider_not_found() {
                AgentError::NoProvider
            } else {
                AgentError::ProviderError(failure.to_string())
            }
        }
        RuntimeLoopError::ToolDispatchError { error, .. } => AgentError::ToolError(error),
        RuntimeLoopError::Cancelled => AgentError::Cancelled,
        RuntimeLoopError::SinkError(message) => {
            if let Some(inner) = message.strip_prefix("model call failed: ") {
                return AgentError::ProviderError(inner.to_string());
            }
            AgentError::ProviderError(message)
        }
        RuntimeLoopError::Other(message) => AgentError::ProviderError(message),
    }
}

pub(super) fn finish_reason_to_string(reason: &RuntimeFinishReason) -> String {
    match reason {
        RuntimeFinishReason::EndTurn => "end_turn".to_string(),
        RuntimeFinishReason::ToolUse => "tool_use".to_string(),
        RuntimeFinishReason::MaxSteps => "max_steps".to_string(),
        RuntimeFinishReason::Cancelled => "cancelled".to_string(),
        RuntimeFinishReason::Error(message) => format!("error:{message}"),
        RuntimeFinishReason::Provider(message) => format!("provider:{message}"),
    }
}

#[derive(Default)]
pub(super) struct AgentLoopSink {
    messages: Vec<rocode_provider::Message>,
    step_text: String,
    step_reasoning: String,
    step_tool_calls: Vec<RuntimeToolCallReady>,
    assistant_flushed: bool,
}

impl AgentLoopSink {
    fn flush_assistant_with_tools(&mut self) {
        if self.assistant_flushed {
            return;
        }

        let mut parts = Vec::new();
        if !self.step_reasoning.is_empty() {
            parts.push(rocode_provider::ContentPart {
                content_type: "reasoning".to_string(),
                text: Some(self.step_reasoning.clone()),
                image_url: None,
                tool_use: None,
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }
        if !self.step_text.is_empty() {
            parts.push(rocode_provider::ContentPart {
                content_type: "text".to_string(),
                text: Some(self.step_text.clone()),
                image_url: None,
                tool_use: None,
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }

        for tool_call in &self.step_tool_calls {
            parts.push(rocode_provider::ContentPart {
                content_type: "tool_use".to_string(),
                text: None,
                image_url: None,
                tool_use: Some(rocode_provider::ToolUse {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    input: tool_call.arguments.clone(),
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
        self.assistant_flushed = true;
    }
}

#[async_trait::async_trait]
impl LoopSink for AgentLoopSink {
    async fn on_event(&mut self, event: &LoopEvent) -> Result<(), RuntimeLoopError> {
        match event {
            LoopEvent::TextChunk(text) => self.step_text.push_str(text),
            LoopEvent::ReasoningChunk { text, .. } => self.step_reasoning.push_str(text),
            LoopEvent::ToolCallReady(tool_call) => self.step_tool_calls.push(tool_call.clone()),
            _ => {}
        }
        Ok(())
    }

    async fn on_tool_result(
        &mut self,
        _call: &RuntimeToolCallReady,
        result: &RuntimeToolResult,
    ) -> Result<(), RuntimeLoopError> {
        self.flush_assistant_with_tools();

        self.messages.push(rocode_provider::Message {
            role: rocode_provider::Role::Tool,
            content: rocode_provider::Content::Parts(vec![rocode_provider::ContentPart {
                content_type: "tool_result".to_string(),
                text: None,
                image_url: None,
                tool_use: None,
                tool_result: Some(rocode_provider::ToolResult {
                    tool_use_id: result.tool_call_id.clone(),
                    content: result.output.clone(),
                    is_error: Some(result.is_error),
                }),
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            }]),
            cache_control: None,
            provider_options: None,
        });
        Ok(())
    }

    async fn on_step_boundary(&mut self, ctx: &StepBoundary) -> Result<(), RuntimeLoopError> {
        match ctx {
            StepBoundary::Start { .. } => {
                self.step_text.clear();
                self.step_reasoning.clear();
                self.step_tool_calls.clear();
                self.assistant_flushed = false;
            }
            StepBoundary::End { .. } => {
                if !self.assistant_flushed {
                    if !self.step_reasoning.is_empty() {
                        // Include reasoning as a structured message with parts.
                        let mut parts = vec![rocode_provider::ContentPart {
                            content_type: "reasoning".to_string(),
                            text: Some(self.step_reasoning.clone()),
                            image_url: None,
                            tool_use: None,
                            tool_result: None,
                            cache_control: None,
                            filename: None,
                            media_type: None,
                            provider_options: None,
                        }];
                        if !self.step_text.is_empty() {
                            parts.push(rocode_provider::ContentPart {
                                content_type: "text".to_string(),
                                text: Some(self.step_text.clone()),
                                image_url: None,
                                tool_use: None,
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
                    } else {
                        self.messages
                            .push(rocode_provider::Message::assistant(self.step_text.clone()));
                    }
                    self.assistant_flushed = true;
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct AgentStreamingSinkDiagnostics {
    pub(super) tool_results: u32,
    pub(super) tool_errors: u32,
}

#[derive(Default)]
pub(super) struct AgentStreamingLoopSink {
    messages: Vec<rocode_provider::Message>,
    outcome: AgentRenderOutcome,
    step_text: String,
    step_reasoning: String,
    step_tool_calls: Vec<RuntimeToolCallReady>,
    assistant_flushed: bool,
    message_open: bool,
    reasoning_open: bool,
    tool_name_by_id: HashMap<String, String>,
    diagnostics: AgentStreamingSinkDiagnostics,
}

impl AgentStreamingLoopSink {
    pub(super) fn into_output(
        mut self,
    ) -> (
        Vec<rocode_provider::Message>,
        AgentRenderOutcome,
        AgentStreamingSinkDiagnostics,
    ) {
        self.close_reasoning_if_open();
        self.close_assistant_if_open();
        (self.messages, self.outcome, self.diagnostics)
    }

    fn ensure_assistant_open(&mut self) {
        if !self.message_open {
            self.close_reasoning_if_open();
            self.outcome.events.push(AgentRenderEvent::AssistantStart);
            self.message_open = true;
        }
    }

    fn close_assistant_if_open(&mut self) {
        if self.message_open {
            self.outcome.events.push(AgentRenderEvent::AssistantEnd);
            self.message_open = false;
        }
    }

    fn ensure_reasoning_open(&mut self) {
        if !self.reasoning_open {
            self.outcome.events.push(AgentRenderEvent::ReasoningStart);
            self.reasoning_open = true;
        }
    }

    fn close_reasoning_if_open(&mut self) {
        if self.reasoning_open {
            self.outcome.events.push(AgentRenderEvent::ReasoningEnd);
            self.reasoning_open = false;
        }
    }

    fn flush_assistant_with_tools(&mut self) {
        if self.assistant_flushed {
            return;
        }

        let mut parts = Vec::new();
        if !self.step_reasoning.is_empty() {
            parts.push(rocode_provider::ContentPart {
                content_type: "reasoning".to_string(),
                text: Some(self.step_reasoning.clone()),
                image_url: None,
                tool_use: None,
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }
        if !self.step_text.is_empty() {
            parts.push(rocode_provider::ContentPart {
                content_type: "text".to_string(),
                text: Some(self.step_text.clone()),
                image_url: None,
                tool_use: None,
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            });
        }

        for tool_call in &self.step_tool_calls {
            parts.push(rocode_provider::ContentPart {
                content_type: "tool_use".to_string(),
                text: None,
                image_url: None,
                tool_use: Some(rocode_provider::ToolUse {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    input: tool_call.arguments.clone(),
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
        self.assistant_flushed = true;
    }

    fn output_metadata(metadata: &Option<serde_json::Value>) -> HashMap<String, serde_json::Value> {
        metadata
            .as_ref()
            .and_then(|value| value.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl LoopSink for AgentStreamingLoopSink {
    async fn on_event(&mut self, event: &LoopEvent) -> Result<(), RuntimeLoopError> {
        match event {
            LoopEvent::TextChunk(text) => {
                self.step_text.push_str(text);
                if !text.is_empty() {
                    self.ensure_assistant_open();
                    self.outcome
                        .events
                        .push(AgentRenderEvent::AssistantDelta(text.clone()));
                }
            }
            LoopEvent::ReasoningChunk { text, .. } => {
                self.step_reasoning.push_str(text);
                if !text.is_empty() {
                    self.ensure_reasoning_open();
                    self.outcome
                        .events
                        .push(AgentRenderEvent::ReasoningDelta(text.clone()));
                }
            }
            LoopEvent::ToolCallProgress {
                id,
                name,
                partial_input,
            } => {
                if let Some(name) = name {
                    self.tool_name_by_id.insert(id.clone(), name.clone());
                    self.close_reasoning_if_open();
                    self.close_assistant_if_open();
                    self.outcome.events.push(AgentRenderEvent::ToolStart {
                        id: id.clone(),
                        name: name.clone(),
                    });
                }
                if !partial_input.is_empty() {
                    let tool_name = self
                        .tool_name_by_id
                        .get(id)
                        .cloned()
                        .or_else(|| name.clone())
                        .unwrap_or_else(|| id.clone());
                    self.outcome.events.push(AgentRenderEvent::ToolProgress {
                        id: id.clone(),
                        name: tool_name,
                        input: partial_input.clone(),
                    });
                }
            }
            LoopEvent::ToolCallReady(call) => {
                self.step_tool_calls.push(call.clone());
                self.tool_name_by_id
                    .insert(call.id.clone(), call.name.clone());
                self.outcome.events.push(AgentRenderEvent::ToolEnd {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    input: call.arguments.clone(),
                });
            }
            LoopEvent::Error(message) => {
                self.outcome.stream_error = Some(message.clone());
                return Err(RuntimeLoopError::ModelError(
                    rocode_orchestrator::runtime::events::ModelFailure::Message(message.clone()),
                ));
            }
            LoopEvent::StepDone { usage, .. } => {
                if let Some(usage) = usage {
                    self.outcome.prompt_tokens =
                        self.outcome.prompt_tokens.max(usage.prompt_tokens);
                    self.outcome.completion_tokens =
                        self.outcome.completion_tokens.max(usage.completion_tokens);
                }
            }
        }
        Ok(())
    }

    async fn on_tool_result(
        &mut self,
        _call: &RuntimeToolCallReady,
        result: &RuntimeToolResult,
    ) -> Result<(), RuntimeLoopError> {
        self.flush_assistant_with_tools();
        let resolved_name = self
            .tool_name_by_id
            .get(&result.tool_call_id)
            .cloned()
            .unwrap_or_else(|| result.tool_name.clone());

        if result.is_error {
            self.diagnostics.tool_errors += 1;
            let mut metadata = Self::output_metadata(&result.metadata);
            // Defensive fallback: some tool execution paths can still produce an
            // error result without orchestrator-injected metadata.
            if !metadata.contains_key(ERROR_CAUSE_KEY) {
                metadata.insert(
                    ERROR_CAUSE_KEY.to_string(),
                    serde_json::Value::String(
                        classify_tool_error_message(&result.output).to_string(),
                    ),
                );
            }
            self.outcome.events.push(AgentRenderEvent::ToolError {
                tool_call_id: result.tool_call_id.clone(),
                tool_name: resolved_name,
                error: result.output.clone(),
                metadata,
            });
        } else {
            self.outcome.events.push(AgentRenderEvent::ToolResult {
                tool_call_id: result.tool_call_id.clone(),
                tool_name: resolved_name,
                output: AgentToolOutput {
                    output: result.output.clone(),
                    title: result.title.clone().unwrap_or_default(),
                    metadata: Self::output_metadata(&result.metadata),
                },
            });
        }
        self.diagnostics.tool_results += 1;

        self.messages.push(rocode_provider::Message {
            role: rocode_provider::Role::Tool,
            content: rocode_provider::Content::Parts(vec![rocode_provider::ContentPart {
                content_type: "tool_result".to_string(),
                text: None,
                image_url: None,
                tool_use: None,
                tool_result: Some(rocode_provider::ToolResult {
                    tool_use_id: result.tool_call_id.clone(),
                    content: result.output.clone(),
                    is_error: Some(result.is_error),
                }),
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            }]),
            cache_control: None,
            provider_options: None,
        });

        Ok(())
    }

    async fn on_step_boundary(&mut self, ctx: &StepBoundary) -> Result<(), RuntimeLoopError> {
        match ctx {
            StepBoundary::Start { .. } => {
                self.step_text.clear();
                self.step_reasoning.clear();
                self.step_tool_calls.clear();
                self.assistant_flushed = false;
            }
            StepBoundary::End { .. } => {
                if !self.assistant_flushed {
                    if !self.step_reasoning.is_empty() {
                        let mut parts = vec![rocode_provider::ContentPart {
                            content_type: "reasoning".to_string(),
                            text: Some(self.step_reasoning.clone()),
                            image_url: None,
                            tool_use: None,
                            tool_result: None,
                            cache_control: None,
                            filename: None,
                            media_type: None,
                            provider_options: None,
                        }];
                        if !self.step_text.is_empty() {
                            parts.push(rocode_provider::ContentPart {
                                content_type: "text".to_string(),
                                text: Some(self.step_text.clone()),
                                image_url: None,
                                tool_use: None,
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
                    } else {
                        self.messages
                            .push(rocode_provider::Message::assistant(self.step_text.clone()));
                    }
                    self.assistant_flushed = true;
                }
                self.close_reasoning_if_open();
                self.close_assistant_if_open();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::map_runtime_loop_error;
    use crate::executor::AgentError;
    use rocode_orchestrator::runtime::events::LoopError as RuntimeLoopError;

    #[test]
    fn sink_error_no_longer_maps_no_provider_by_substring() {
        let error = map_runtime_loop_error(RuntimeLoopError::SinkError(
            "no provider available".to_string(),
        ));

        assert!(
            matches!(error, AgentError::ProviderError(message) if message == "no provider available")
        );
    }

    #[test]
    fn sink_error_still_strips_model_call_failed_prefix() {
        let error = map_runtime_loop_error(RuntimeLoopError::SinkError(
            "model call failed: upstream exploded".to_string(),
        ));

        assert!(
            matches!(error, AgentError::ProviderError(message) if message == "upstream exploded")
        );
    }

    #[test]
    fn other_error_no_longer_maps_no_provider_by_substring() {
        let error =
            map_runtime_loop_error(RuntimeLoopError::Other("no provider available".to_string()));

        assert!(
            matches!(error, AgentError::ProviderError(message) if message == "no provider available")
        );
    }
}
