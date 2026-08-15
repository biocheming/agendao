use super::{
    AgentLoopObserver, AgentObservationContext, AssistantTurn, ConversationItem, ModelBackend,
    ModelBackendError, ModelRequest, ToolCall,
};
use crate::blueprint::{AgentId, ToolId};
use crate::context::Usage;
use agendao_provider::responses::types::ResponsesProviderOptions;
use agendao_provider::{
    assemble_tool_calls, ContentPart, Message, Provider, ProviderApiShape, StreamEvent,
    ToolDefinition,
};
use async_trait::async_trait;
use futures::StreamExt;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct ProviderModelBackend {
    default_route: ModelRoute,
    routes: BTreeMap<AgentId, ModelRoute>,
    tools: BTreeMap<ToolId, ToolDefinition>,
}

#[derive(Clone)]
pub struct ModelRoute {
    pub provider: Arc<dyn Provider>,
    pub request: agendao_execution_types::CompiledExecutionRequest,
}

impl ProviderModelBackend {
    pub fn new(
        provider: Arc<dyn Provider>,
        request_defaults: agendao_execution_types::CompiledExecutionRequest,
        tools: impl IntoIterator<Item = (ToolId, ToolDefinition)>,
    ) -> Self {
        Self {
            default_route: ModelRoute {
                provider,
                request: request_defaults,
            },
            routes: BTreeMap::new(),
            tools: tools.into_iter().collect(),
        }
    }

    pub fn with_routes(mut self, routes: BTreeMap<AgentId, ModelRoute>) -> Self {
        self.routes = routes;
        self
    }
}

#[async_trait]
impl ModelBackend for ProviderModelBackend {
    async fn invoke(
        &self,
        request: ModelRequest,
        context: &AgentObservationContext<'_>,
        observer: &dyn AgentLoopObserver,
    ) -> Result<AssistantTurn, ModelBackendError> {
        let system = String::from_utf8(request.prompt.stable.to_vec()).map_err(|error| {
            ModelBackendError::message(format!("stable prompt is not UTF-8: {error}"))
        })?;
        let tools: Vec<ToolDefinition> = request
            .tools
            .iter()
            .filter_map(|id| self.tools.get(id).cloned())
            .collect();
        if tools.len() != request.tools.len() {
            return Err(ModelBackendError::message(
                "model request references a tool without a schema",
            ));
        }
        let route = self
            .routes
            .get(&request.agent)
            .unwrap_or(&self.default_route);
        let initial_input = initial_input_message(&request)?;
        let (messages, uses_responses_continuation) = project_transport_messages(
            request.conversation_seed.as_ref(),
            request.prompt.dynamic.history_tail.as_slice(),
            request.reasoning_continuation.as_deref(),
            route.provider.api_shape(),
            initial_input,
        )?;

        let mut compiled = route.request.clone();
        if uses_responses_continuation {
            let previous_response_id = request
                .reasoning_continuation
                .expect("Responses continuation checked above");
            let mut provider_options = compiled.provider_options.unwrap_or_default();
            provider_options.insert(
                "openai".to_string(),
                serde_json::to_value(ResponsesProviderOptions {
                    previous_response_id: Some(previous_response_id),
                    ..ResponsesProviderOptions::default()
                })
                .expect("responses options serialize"),
            );
            compiled.provider_options = Some(provider_options);
        }
        let stream = route
            .provider
            .chat_stream(compiled.to_chat_request_with_system(
                messages,
                tools,
                Some(true),
                Some(system),
            ))
            .await
            .map_err(|error| {
                ModelBackendError::Provider(Box::new(agendao_provider::summarize_provider_error(
                    route.provider.id(),
                    Some(&compiled.model_id),
                    &error,
                )))
            })?;
        collect_stream(
            stream,
            context,
            observer,
            route.provider.id(),
            compiled.model_id.as_str(),
        )
        .await
    }
}

fn project_transport_messages(
    conversation_seed: &[Message],
    history: &[ConversationItem],
    continuation: Option<&str>,
    api_shape: Option<ProviderApiShape>,
    initial_input: Message,
) -> Result<(Vec<Message>, bool), ModelBackendError> {
    let uses_responses_continuation =
        continuation.is_some() && api_shape == Some(ProviderApiShape::Responses);
    let mut messages = if uses_responses_continuation {
        Vec::new()
    } else {
        conversation_seed.to_vec()
    };
    if messages.is_empty() && !uses_responses_continuation {
        messages.push(initial_input);
    }
    let history = if uses_responses_continuation {
        history_after_response(history, continuation.expect("checked above"))?
    } else {
        history
    };
    append_conversation_items(&mut messages, history).map_err(ModelBackendError::message)?;
    Ok((messages, uses_responses_continuation))
}

fn initial_input_message(request: &ModelRequest) -> Result<Message, ModelBackendError> {
    Ok(Message::user(
        serde_json::to_string(&serde_json::json!({
            "workspace": request.prompt.semi_stable.workspace_summary,
            "handoff": request.prompt.semi_stable.handoff,
            "progress": request.prompt.semi_stable.progress_summary,
        }))
        .map_err(|error| ModelBackendError::message(error.to_string()))?,
    ))
}

fn history_after_response<'a>(
    history: &'a [ConversationItem],
    response_id: &str,
) -> Result<&'a [ConversationItem], ModelBackendError> {
    let boundary = history.iter().rposition(|item| {
        matches!(
            item,
            ConversationItem::Assistant { turn }
                if turn.reasoning_continuation.as_deref() == Some(response_id)
        )
    });
    let Some(boundary) = boundary else {
        return Err(ModelBackendError::message(format!(
            "Responses continuation '{response_id}' has no canonical conversation boundary"
        )));
    };
    Ok(&history[boundary + 1..])
}

fn append_conversation_items(
    messages: &mut Vec<Message>,
    history: &[ConversationItem],
) -> Result<(), String> {
    for item in history {
        match item {
            ConversationItem::Assistant { turn } => {
                let mut parts = Vec::new();
                if let Some(reasoning) = turn.reasoning.as_deref().filter(|text| !text.is_empty()) {
                    parts.push(ContentPart::reasoning(reasoning.to_string()));
                }
                if let Some(content) = turn.content.as_deref().filter(|text| !text.is_empty()) {
                    parts.push(ContentPart::text(content.to_string()));
                }
                parts.extend(turn.tool_calls.iter().map(|call| {
                    ContentPart::tool_use(
                        call.id.clone(),
                        call.tool.0.clone(),
                        call.arguments.clone(),
                    )
                }));
                if let Some(message) = Message::assistant_from_parts(parts) {
                    messages.push(message);
                }
            }
            ConversationItem::ToolResult {
                call_id,
                output,
                is_error,
            } => {
                messages.push(Message::tool_parts(vec![ContentPart::tool_result(
                    call_id.clone(),
                    output.clone(),
                    Some(*is_error),
                )]));
            }
        }
    }
    Ok(())
}

async fn collect_stream(
    stream: agendao_provider::StreamResult,
    context: &AgentObservationContext<'_>,
    observer: &dyn AgentLoopObserver,
    provider_id: &str,
    model_id: &str,
) -> Result<AssistantTurn, ModelBackendError> {
    let mut stream = assemble_tool_calls(stream);
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut usage = Usage::default();
    let mut finish_reason = None;
    let mut response_id = None;
    while let Some(event) = stream.next().await {
        match event.map_err(|error| {
            ModelBackendError::Provider(Box::new(agendao_provider::summarize_provider_error(
                provider_id,
                Some(model_id),
                &error,
            )))
        })? {
            StreamEvent::TextDelta(delta) => {
                observer
                    .text_delta(context, &delta)
                    .await
                    .map_err(ModelBackendError::message)?;
                text.push_str(&delta);
            }
            StreamEvent::ReasoningDelta { id, text } => {
                observer
                    .reasoning_delta(context, &id, &text)
                    .await
                    .map_err(ModelBackendError::message)?;
                reasoning.push_str(&text);
            }
            StreamEvent::ToolInputDelta { id, delta } => {
                observer
                    .tool_input_delta(context, &id, None, &delta)
                    .await
                    .map_err(ModelBackendError::message)?;
            }
            StreamEvent::ToolCallDelta { id, input } => {
                observer
                    .tool_input_delta(context, &id, None, &input)
                    .await
                    .map_err(ModelBackendError::message)?;
            }
            StreamEvent::ToolCallEnd { id, name, input } => tool_calls.push(ToolCall {
                id,
                tool: ToolId::new(name),
                arguments: input,
            }),
            StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_miss_tokens,
                cache_write_tokens,
                ..
            } => {
                usage.input_tokens = usage.input_tokens.max(prompt_tokens);
                usage.output_tokens = usage.output_tokens.max(completion_tokens);
                usage.cache_read_tokens = usage.cache_read_tokens.max(cache_read_tokens);
                usage.cache_miss_tokens = usage.cache_miss_tokens.max(cache_miss_tokens);
                usage.cache_write_tokens = usage.cache_write_tokens.max(cache_write_tokens);
            }
            StreamEvent::FinishStep {
                finish_reason: step_finish_reason,
                usage: step_usage,
                provider_metadata,
            } => {
                usage.input_tokens = usage.input_tokens.max(step_usage.prompt_tokens);
                usage.output_tokens = usage.output_tokens.max(step_usage.completion_tokens);
                usage.reasoning_tokens = usage.reasoning_tokens.max(step_usage.reasoning_tokens);
                usage.cache_read_tokens = usage.cache_read_tokens.max(step_usage.cache_read_tokens);
                usage.cache_miss_tokens = usage.cache_miss_tokens.max(step_usage.cache_miss_tokens);
                usage.cache_write_tokens =
                    usage.cache_write_tokens.max(step_usage.cache_write_tokens);
                finish_reason = step_finish_reason.or(finish_reason);
                response_id = provider_metadata
                    .as_ref()
                    .and_then(|value| value.get("response_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or(response_id);
            }
            StreamEvent::Error(error) => return Err(ModelBackendError::message(error)),
            _ => {}
        }
    }
    Ok(AssistantTurn {
        content: (!text.is_empty()).then_some(text),
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        tool_calls,
        usage,
        finish_reason,
        reasoning_continuation: response_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_provider::{Content, Role};

    fn canonical_tool_round() -> Vec<ConversationItem> {
        vec![
            ConversationItem::Assistant {
                turn: AssistantTurn {
                    content: None,
                    reasoning: Some("need evidence".to_string()),
                    tool_calls: vec![ToolCall {
                        id: "call-1".to_string(),
                        tool: ToolId::from("read"),
                        arguments: serde_json::json!({"path": "README.md"}),
                    }],
                    usage: Usage::default(),
                    finish_reason: Some("tool-calls".to_string()),
                    reasoning_continuation: Some("resp-1".to_string()),
                },
            },
            ConversationItem::ToolResult {
                call_id: "call-1".to_string(),
                output: "contents".to_string(),
                is_error: false,
            },
        ]
    }

    fn role_names(messages: &[Message]) -> Vec<&'static str> {
        messages
            .iter()
            .map(|message| match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            })
            .collect()
    }

    #[test]
    fn responses_continuation_sends_only_items_after_response_boundary() {
        let (messages, uses_token) = project_transport_messages(
            &[Message::user("canonical seed")],
            &canonical_tool_round(),
            Some("resp-1"),
            Some(ProviderApiShape::Responses),
            Message::user("synthetic input"),
        )
        .expect("Responses projection");

        assert!(uses_token);
        assert_eq!(role_names(&messages), ["tool"]);
        let Content::Parts(parts) = &messages[0].content else {
            panic!("tool result must use typed parts");
        };
        assert_eq!(parts[0].tool_result.as_ref().unwrap().tool_use_id, "call-1");
    }

    #[test]
    fn replay_protocols_keep_complete_canonical_tool_round() {
        for shape in [
            ProviderApiShape::ChatCompletions,
            ProviderApiShape::AnthropicMessages,
        ] {
            let (messages, uses_token) = project_transport_messages(
                &[Message::user("canonical seed")],
                &canonical_tool_round(),
                Some("resp-1"),
                Some(shape),
                Message::user("synthetic input"),
            )
            .expect("replay projection");

            assert!(!uses_token);
            assert_eq!(role_names(&messages), ["user", "assistant", "tool"]);
            let Content::Parts(assistant) = &messages[1].content else {
                panic!("assistant tool call must use typed parts");
            };
            let Content::Parts(tool) = &messages[2].content else {
                panic!("tool result must use typed parts");
            };
            assert_eq!(
                assistant.last().unwrap().tool_use.as_ref().unwrap().id,
                "call-1"
            );
            assert_eq!(tool[0].tool_result.as_ref().unwrap().tool_use_id, "call-1");
        }
    }

    #[test]
    fn responses_continuation_requires_a_matching_canonical_boundary() {
        let error = project_transport_messages(
            &[Message::user("canonical seed")],
            &canonical_tool_round(),
            Some("expired-local-boundary"),
            Some(ProviderApiShape::Responses),
            Message::user("synthetic input"),
        )
        .expect_err("missing boundary must fail");

        assert!(error
            .to_string()
            .contains("no canonical conversation boundary"));
    }
}
