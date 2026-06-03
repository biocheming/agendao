/// Prompt execution logic
///
/// Phase 4.4: Extract complex execution logic from core.rs
/// Phase 6.1: Add multi-turn conversation support
/// Phase 6.2: Add tool calling support

use std::sync::Arc;
use agendao_provider::{ChatRequest, Content, ContentPart, Message, ProviderRegistry, ToolDefinition};
use agendao_session_core::{SessionAccess, SessionStore, MessageRole, PartType};
use agendao_tool::{ToolRegistry, ToolContext};
use crate::{OrchestratorError, PromptExecutionOptions, PromptExecutionResult, UsageInfo};

fn annotate_latest_user_message_metadata(
    session: &mut impl SessionAccess,
    options: &PromptExecutionOptions,
    text: &str,
) {
    let record = session.record_mut();
    let Some(user_msg) = record
        .messages
        .iter_mut()
        .rfind(|message| message.role == MessageRole::User)
    else {
        return;
    };

    if let Some(agent) = options.agent_id.as_deref() {
        user_msg
            .metadata
            .insert("resolved_agent".to_string(), serde_json::json!(agent));
    }
    if let Some(ingress_source) = options.ingress_source.as_deref() {
        user_msg.metadata.insert(
            "ingress_source".to_string(),
            serde_json::json!(ingress_source),
        );
    }
    if let Some(idempotency_key) = options.idempotency_key.as_deref() {
        user_msg.metadata.insert(
            "ingress_idempotency_key".to_string(),
            serde_json::json!(idempotency_key),
        );
    }
    if !text.is_empty() {
        user_msg.metadata.insert(
            "resolved_user_prompt".to_string(),
            serde_json::json!(text),
        );
    }
}

fn annotate_session_prompt_metadata(
    session: &mut impl SessionAccess,
    options: &PromptExecutionOptions,
) {
    let record = session.record_mut();
    if let Some(variant) = options.variant.as_deref() {
        record
            .metadata
            .insert("model_variant".to_string(), serde_json::json!(variant));
    } else {
        record.metadata.remove("model_variant");
    }

    if let Some(profile) = options.scheduler_profile.as_deref() {
        record
            .metadata
            .insert("scheduler_profile".to_string(), serde_json::json!(profile));
    } else {
        record.metadata.remove("scheduler_profile");
    }

    if let Some(ingress_source) = options.ingress_source.as_deref() {
        record.metadata.insert(
            "last_ingress_source".to_string(),
            serde_json::json!(ingress_source),
        );
    }
}

/// Execute a prompt with the given provider and options
///
/// This is a simplified implementation that:
/// - Supports single-turn conversation (no session history)
/// - No tool calling
/// - No context management
/// - No streaming
pub async fn execute_prompt_simple(
    providers: &Arc<tokio::sync::RwLock<ProviderRegistry>>,
    session_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> Result<PromptExecutionResult, OrchestratorError> {
    // 1. Parse provider and model from options
    let (provider_id, model_id) = parse_model_spec(options.model.as_deref());

    // 2. Get provider from registry
    let providers_guard = providers.read().await;
    let provider = providers_guard
        .get(&provider_id)
        .ok_or_else(|| OrchestratorError::Other(format!("Provider not found: {}", provider_id)))?;

    // 3. Build chat request
    let request = build_simple_request(&model_id, text, options);

    // 4. Call provider
    let response = provider
        .chat(request)
        .await
        .map_err(|e| OrchestratorError::Other(format!("Provider error: {}", e)))?;

    // 5. Extract response text
    let text = extract_response_text(&response);

    // 6. Build result
    let usage = response.usage.map(|u| UsageInfo {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    });

    Ok(PromptExecutionResult {
        session_id: session_id.to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
        text,
        usage,
    })
}

/// Parse model specification into (provider_id, model_id)
///
/// Format: "provider:model" or just "model"
/// Default provider: "anthropic"
/// Default model: "claude-opus-4-7"
fn parse_model_spec(model_spec: Option<&str>) -> (String, String) {
    match model_spec {
        Some(spec) => {
            if let Some((p, m)) = spec.split_once(':') {
                (p.to_string(), m.to_string())
            } else {
                ("anthropic".to_string(), spec.to_string())
            }
        }
        None => ("anthropic".to_string(), "claude-opus-4-7".to_string()),
    }
}

/// Build a simple chat request (single user message, no tools)
fn build_simple_request(
    model_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> ChatRequest {
    ChatRequest {
        model: model_id.to_string(),
        messages: vec![Message::user(text)],
        max_tokens: Some(4096),
        temperature: None,
        top_p: None,
        system: None,
        tools: None,
        stream: Some(false),
        provider_options: None,
        variant: options.variant.clone(),
    }
}

/// Extract text from chat response
fn extract_response_text(response: &agendao_provider::ChatResponse) -> String {
    if let Some(choice) = response.choices.first() {
        match &choice.message.content {
            Content::Text(t) => t.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| p.text.as_ref())
                .cloned()
                .collect::<Vec<_>>()
                .join(""),
        }
    } else {
        String::new()
    }
}

// ============================================================================
// Future enhancements (Phase 5+)
// ============================================================================

/// Execute prompt with full session context (Phase 6.1)
///
/// This supports:
/// - Multi-turn conversation with session history
/// - Session state persistence
/// - Usage statistics accumulation
/// - Automatic session creation
/// - Tool calling loop (Phase 6.2)
///
/// Not yet implemented:
/// - Streaming responses (Phase 6.3)
/// - Context compaction (Phase 6.4)
pub async fn execute_prompt_with_session<S: SessionStore>(
    sessions: &Arc<tokio::sync::Mutex<S>>,
    providers: &Arc<tokio::sync::RwLock<ProviderRegistry>>,
    tools: &Arc<tokio::sync::RwLock<agendao_tool::ToolRegistry>>,
    session_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> Result<PromptExecutionResult, OrchestratorError> {
    // 1. Load or create session; add user message unless pure continue (continue_last + empty text)
    if !options.continue_last || !text.is_empty() {
        let mut sessions_guard = sessions.lock().await;
        let session = sessions_guard.ensure_session(session_id);
        if let (Some(origin), Some(surface)) =
            (options.source_origin, options.source_surface)
        {
            session.add_user_message_with_source(text, origin, surface);
        } else {
            session.add_user_message(text);
        }
        annotate_latest_user_message_metadata(session, options, text);
        annotate_session_prompt_metadata(session, options);
    } else {
        // Ensure session exists even for pure continue (no user message added)
        let mut sessions_guard = sessions.lock().await;
        let session = sessions_guard.ensure_session(session_id);
        annotate_session_prompt_metadata(session, options);
    }

    // 2. Parse provider and model
    let (provider_id, model_id) = parse_model_spec(options.model.as_deref());

    // 3. Get provider
    let providers_guard = providers.read().await;
    let provider = providers_guard
        .get(&provider_id)
        .ok_or_else(|| OrchestratorError::Other(format!("Provider not found: {}", provider_id)))?
        .clone();
    drop(providers_guard);

    // 4. Get available tools
    let tool_definitions = {
        let tools_guard = tools.read().await;
        build_tool_definitions(&*tools_guard).await
    };

    // 5. Enter LLM loop (may iterate multiple times for tool calls)
    let mut final_assistant_msg_id;
    let mut accumulated_usage = UsageInfo {
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
    };

    loop {
        // 5.1 Build messages from session history
        let messages = {
            let sessions_guard = sessions.lock().await;
            let session = sessions_guard
                .get(session_id)
                .ok_or_else(|| OrchestratorError::Other("Session not found".to_string()))?;
            build_messages_from_session(session)
        };

        // 5.2 Build chat request
        let request = ChatRequest {
            model: model_id.clone(),
            messages,
            max_tokens: Some(4096),
            temperature: None,
            top_p: None,
            system: None,
            tools: if tool_definitions.is_empty() {
                None
            } else {
                Some(tool_definitions.clone())
            },
            stream: Some(false),
            provider_options: None,
            variant: options.variant.clone(),
        };

        // 5.3 Call provider
        let response = provider
            .chat(request)
            .await
            .map_err(|e| OrchestratorError::Other(format!("Provider error: {}", e)))?;

        // 5.4 Accumulate usage
        if let Some(usage) = &response.usage {
            accumulated_usage.input_tokens += usage.prompt_tokens;
            accumulated_usage.output_tokens += usage.completion_tokens;
            accumulated_usage.total_tokens += usage.total_tokens;
        }

        // 5.5 Extract response content
        let (response_text, tool_calls) = extract_response_content(&response);

        // 5.6 Save assistant response
        final_assistant_msg_id = {
            let mut sessions_guard = sessions.lock().await;
            let session = sessions_guard
                .get_mut(session_id)
                .ok_or_else(|| OrchestratorError::Other("Session not found".to_string()))?;

            let msg_id = session.add_assistant_message(&response_text);

            // Update usage statistics
            session.add_usage(accumulated_usage.input_tokens, accumulated_usage.output_tokens);

            msg_id
        };

        // 5.7 Check if there are tool calls
        if tool_calls.is_empty() {
            // No tool calls, exit loop
            break;
        }

        // 5.8 Execute each tool call
        for tool_call in tool_calls {
            // Execute tool
            let tool_result = execute_tool(tools, session_id, &tool_call).await?;

            // Save tool result to session
            {
                let mut sessions_guard = sessions.lock().await;
                let session = sessions_guard
                    .get_mut(session_id)
                    .ok_or_else(|| OrchestratorError::Other("Session not found".to_string()))?;

                session.add_tool_result(&tool_call.id, &tool_result, tool_result.starts_with("Error:"));
            }
        }

        // 5.9 Continue loop (send tool results back to LLM)
    }

    // 6. Build final result
    let response_text = {
        let sessions_guard = sessions.lock().await;
        let session = sessions_guard
            .get(session_id)
            .ok_or_else(|| OrchestratorError::Other("Session not found".to_string()))?;

        // Get the last assistant message text
        session
            .record()
            .messages
            .iter()
            .rev()
            .find(|msg| msg.role == MessageRole::Assistant)
            .and_then(|msg| {
                msg.parts
                    .iter()
                    .filter_map(|part| {
                        if let PartType::Text { text, .. } = &part.part_type {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
                    .into()
            })
            .unwrap_or_default()
    };

    Ok(PromptExecutionResult {
        session_id: session_id.to_string(),
        message_id: final_assistant_msg_id,
        text: response_text,
        usage: Some(accumulated_usage),
    })
}

/// Build tool definitions from ToolRegistry
async fn build_tool_definitions(registry: &ToolRegistry) -> Vec<ToolDefinition> {
    let tools = registry.list().await;
    let mut definitions = Vec::new();

    for tool in tools {
        definitions.push(ToolDefinition {
            name: tool.id().to_string(),
            description: Some(tool.description().to_string()),
            parameters: tool.parameters(),
        });
    }

    definitions
}

/// Extract response content and tool calls from provider response
fn extract_response_content(response: &agendao_provider::ChatResponse) -> (String, Vec<ToolCall>) {
    if let Some(choice) = response.choices.first() {
        match &choice.message.content {
            Content::Text(t) => (t.clone(), Vec::new()),
            Content::Parts(parts) => {
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();

                for part in parts {
                    if let Some(text) = &part.text {
                        text_parts.push(text.as_str());
                    }
                    if part.content_type == "tool_use" {
                        if let Some(tool_use) = &part.tool_use {
                            tool_calls.push(ToolCall {
                                id: tool_use.id.clone(),
                                name: tool_use.name.clone(),
                                input: tool_use.input.clone(),
                            });
                        }
                    }
                }

                (text_parts.join(""), tool_calls)
            }
        }
    } else {
        (String::new(), Vec::new())
    }
}

/// Tool call structure
#[derive(Debug, Clone)]
struct ToolCall {
    id: String,
    name: String,
    input: serde_json::Value,
}

/// Execute a tool call
async fn execute_tool(
    tools: &Arc<tokio::sync::RwLock<ToolRegistry>>,
    session_id: &str,
    tool_call: &ToolCall,
) -> Result<String, OrchestratorError> {
    let tools_guard = tools.read().await;

    let tool = tools_guard
        .get(&tool_call.name)
        .await
        .ok_or_else(|| OrchestratorError::Other(format!("Tool not found: {}", tool_call.name)))?;

    // Create a minimal ToolContext
    let ctx = ToolContext::new(
        session_id.to_string(),
        format!("msg_{}", uuid::Uuid::new_v4()),
        std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| ".".to_string()),
    );

    // Execute tool
    let result = tool
        .execute(tool_call.input.clone(), ctx)
        .await
        .map_err(|e| OrchestratorError::Other(format!("Tool execution error: {}", e)))?;

    Ok(result.output)
}

/// Build messages from session history (generic over any SessionAccess impl).
///
/// Converts `agendao_types` session parts into `agendao_provider` messages:
/// - `User` / `System` → text content
/// - `Assistant` → text + optional tool_call parts
/// - `Tool` → tool_result parts (from PartType::ToolResult, stored by the
///   unified authority's add_tool_result)
fn build_messages_from_session(session: &impl SessionAccess) -> Vec<Message> {
    session
        .record()
        .messages
        .iter()
        .filter_map(|msg| {
            let message = match msg.role {
                MessageRole::User | MessageRole::System => {
                    let content = extract_text_content(&msg.parts);
                    Message::user(&content)
                }
                MessageRole::Assistant => {
                    let mut content_parts: Vec<ContentPart> = Vec::new();
                    let mut text_buf = String::new();
                    for part in &msg.parts {
                        match &part.part_type {
                            PartType::Text { text, .. } => text_buf.push_str(text),
                            PartType::ToolCall { id, name, input, .. } => {
                                if !text_buf.is_empty() {
                                    content_parts.push(ContentPart::text(std::mem::take(&mut text_buf)));
                                }
                                content_parts.push(ContentPart::tool_use(
                                    id.clone(),
                                    name.clone(),
                                    input.clone(),
                                ));
                            }
                            PartType::Reasoning { .. } => {
                                // Reasoning is display/diagnostic content —
                                // skip it so it isn't re-injected into the model.
                            }
                            _ => {}
                        }
                    }
                    if !text_buf.is_empty() {
                        content_parts.push(ContentPart::text(std::mem::take(&mut text_buf)));
                    }
                    if content_parts.is_empty() {
                        Message::assistant(&text_buf)
                    } else {
                        Message::assistant_parts(content_parts)
                    }
                }
                MessageRole::Tool => {
                    let mut content_parts: Vec<ContentPart> = Vec::new();
                    for part in &msg.parts {
                        if let PartType::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                            ..
                        } = &part.part_type
                        {
                            content_parts.push(ContentPart::tool_result(
                                tool_call_id.clone(),
                                content.clone(),
                                Some(*is_error),
                            ));
                        }
                    }
                    if content_parts.is_empty() {
                        return None;
                    }
                    Message::tool_parts(content_parts)
                }
            };
            Some(message)
        })
        .collect()
}

fn extract_text_content(parts: &[agendao_types::MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|part| {
            if let PartType::Text { text, .. } = &part.part_type {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Execute prompt with tool calling support (Phase 5)
#[allow(dead_code)]
pub async fn execute_prompt_with_tools(
    _providers: &Arc<tokio::sync::RwLock<ProviderRegistry>>,
    _session_id: &str,
    _text: &str,
    _options: &PromptExecutionOptions,
) -> Result<PromptExecutionResult, OrchestratorError> {
    Err(OrchestratorError::Other(
        "execute_prompt_with_tools not yet implemented (Phase 5)".to_string(),
    ))
}

/// Execute prompt with streaming output (Phase 6.3)
///
/// Returns a stream of events that includes:
/// - Text deltas as they arrive
/// - Tool calls and their execution
/// - Usage statistics
/// - Multi-turn conversation support
///
/// The stream handles the full tool calling loop:
/// 1. Stream LLM response
/// 2. Detect tool calls
/// 3. Execute tools (non-streaming)
/// 4. Continue streaming next LLM round
pub async fn execute_prompt_streaming_with_session<S: SessionStore + Send + 'static>(
    sessions: &Arc<tokio::sync::Mutex<S>>,
    providers: &Arc<tokio::sync::RwLock<ProviderRegistry>>,
    tools: &Arc<tokio::sync::RwLock<agendao_tool::ToolRegistry>>,
    session_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> Result<agendao_provider::StreamResult, OrchestratorError> {
    use futures::stream::{self, StreamExt};
    use agendao_provider::StreamEvent;

    // 1. Add user message to session
    {
        let mut sessions_guard = sessions.lock().await;
        let session = sessions_guard.ensure_session(session_id);
        if let (Some(origin), Some(surface)) =
            (options.source_origin, options.source_surface)
        {
            session.add_user_message_with_source(text, origin, surface);
        } else {
            session.add_user_message(text);
        }
    }

    // 2. Parse provider and model
    let (provider_id, model_id) = parse_model_spec(options.model.as_deref());

    // 3. Get provider
    let providers_guard = providers.read().await;
    let provider = providers_guard
        .get(&provider_id)
        .ok_or_else(|| OrchestratorError::Other(format!("Provider not found: {}", provider_id)))?
        .clone();
    drop(providers_guard);

    // 4. Get tool definitions
    let tool_definitions = {
        let tools_guard = tools.read().await;
        build_tool_definitions(&*tools_guard).await
    };

    // 5. Clone Arc references for the stream
    let sessions = Arc::clone(sessions);
    let tools = Arc::clone(tools);
    let session_id = session_id.to_string();
    let options = options.clone();

    // 6. Create streaming state machine
    struct StreamState<Store: SessionStore> {
        sessions: Arc<tokio::sync::Mutex<Store>>,
        tools: Arc<tokio::sync::RwLock<agendao_tool::ToolRegistry>>,
        session_id: String,
        provider: Arc<dyn agendao_provider::Provider>,
        model_id: String,
        tool_definitions: Vec<agendao_provider::ToolDefinition>,
        options: PromptExecutionOptions,
        current_stream: Option<agendao_provider::StreamResult>,
        accumulated_text: String,
        accumulated_tool_calls: Vec<ToolCall>,
        accumulated_usage: UsageInfo,
        round: u32,
        finished: bool,
    }

    let initial_state = StreamState {
        sessions: sessions.clone(),
        tools: tools.clone(),
        session_id: session_id.clone(),
        provider: provider.clone(),
        model_id: model_id.clone(),
        tool_definitions: tool_definitions.clone(),
        options: options.clone(),
        current_stream: None,
        accumulated_text: String::new(),
        accumulated_tool_calls: Vec::new(),
        accumulated_usage: UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        },
        round: 0,
        finished: false,
    };

    // 7. Start first stream
    let messages = {
        let sessions_guard = sessions.lock().await;
        let session = sessions_guard
            .get(&session_id)
            .ok_or_else(|| OrchestratorError::Other("Session not found".to_string()))?;
        build_messages_from_session(session)
    };

    let request = ChatRequest {
        model: model_id.clone(),
        messages,
        max_tokens: Some(4096),
        temperature: None,
        top_p: None,
        system: None,
        tools: if tool_definitions.is_empty() {
            None
        } else {
            Some(tool_definitions.clone())
        },
        stream: Some(true),
        provider_options: None,
        variant: options.variant.clone(),
    };

    let first_stream = provider
        .chat_stream(request)
        .await
        .map_err(|e| OrchestratorError::Other(format!("Provider error: {}", e)))?;

    let initial_state = StreamState {
        current_stream: Some(first_stream),
        ..initial_state
    };

    // 8. Create unfold stream that handles tool calling loop
    let output_stream = stream::unfold(initial_state, |mut state| async move {
        loop {
            if state.finished {
                return None;
            }

            // If we have a current stream, pull next event
            if let Some(ref mut stream) = state.current_stream {
                match stream.next().await {
                    Some(Ok(event)) => {
                        match &event {
                            StreamEvent::TextDelta(text) => {
                                state.accumulated_text.push_str(text);
                            }
                            StreamEvent::ToolCallEnd { id, name, input } => {
                                state.accumulated_tool_calls.push(ToolCall {
                                    id: id.clone(),
                                    name: name.clone(),
                                    input: input.clone(),
                                });
                            }
                            StreamEvent::FinishStep { usage, .. } => {
                                state.accumulated_usage.input_tokens += usage.prompt_tokens;
                                state.accumulated_usage.output_tokens += usage.completion_tokens;
                                state.accumulated_usage.total_tokens +=
                                    usage.prompt_tokens + usage.completion_tokens;
                            }
                            StreamEvent::Done => {
                                // Stream finished, check if we have tool calls
                                state.current_stream = None;

                                // Save assistant message
                                {
                                    let mut sessions_guard = state.sessions.lock().await;
                                    let session = sessions_guard.get_mut(&state.session_id).unwrap();
                                    session.add_assistant_message(&state.accumulated_text);
                                    session.add_usage(
                                        state.accumulated_usage.input_tokens,
                                        state.accumulated_usage.output_tokens,
                                    );
                                }

                                if state.accumulated_tool_calls.is_empty() {
                                    // No tool calls, we're done
                                    state.finished = true;
                                    return Some((Ok(event), state));
                                } else {
                                    // Execute tools and start next round
                                    let tool_calls = std::mem::take(&mut state.accumulated_tool_calls);
                                    state.accumulated_text.clear();
                                    state.round += 1;

                                    // Execute each tool
                                    for tool_call in tool_calls {
                                        let tool_result = match execute_tool(
                                            &state.tools,
                                            &state.session_id,
                                            &tool_call,
                                        )
                                        .await
                                        {
                                            Ok(result) => result,
                                            Err(e) => format!("Error: {}", e),
                                        };

                                        // Save tool result to session
                                        {
                                            let mut sessions_guard = state.sessions.lock().await;
                                            let session =
                                                sessions_guard.get_mut(&state.session_id).unwrap();
                                            session.add_tool_result(
                                                &tool_call.id,
                                                &tool_result,
                                                tool_result.starts_with("Error:"),
                                            );
                                        }
                                    }

                                    // Build messages for next round
                                    let messages = {
                                        let sessions_guard = state.sessions.lock().await;
                                        let session = sessions_guard.get(&state.session_id).unwrap();
                                        build_messages_from_session(session)
                                    };

                                    // Start next stream
                                    let request = ChatRequest {
                                        model: state.model_id.clone(),
                                        messages,
                                        max_tokens: Some(4096),
                                        temperature: None,
                                        top_p: None,
                                        system: None,
                                        tools: if state.tool_definitions.is_empty() {
                                            None
                                        } else {
                                            Some(state.tool_definitions.clone())
                                        },
                                        stream: Some(true),
                                        provider_options: None,
                                        variant: state.options.variant.clone(),
                                    };

                                    match state.provider.chat_stream(request).await {
                                        Ok(next_stream) => {
                                            state.current_stream = Some(next_stream);
                                            // Forward the Done event, then continue with next stream
                                            return Some((Ok(event), state));
                                        }
                                        Err(e) => {
                                            state.finished = true;
                                            return Some((
                                                Err(agendao_provider::ProviderError::StreamError(
                                                    format!("Failed to start next round: {}", e),
                                                )),
                                                state,
                                            ));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }

                        // Forward event to client
                        return Some((Ok(event), state));
                    }
                    Some(Err(e)) => {
                        state.finished = true;
                        return Some((Err(e), state));
                    }
                    None => {
                        // Stream ended without Done event
                        return None;
                    }
                }
            } else {
                // No current stream and not finished - shouldn't happen
                return None;
            }
        }
    });

    Ok(Box::pin(output_stream))
}
