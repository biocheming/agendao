use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use rocode_execution_types::ExecutionRequestContext;
use rocode_orchestrator::ToolExecError as OrchestratorToolExecError;
use rocode_provider::ProviderRegistry;
use rocode_tool::{ToolContext, ToolError, ToolRegistry};
use rocode_types::{
    SessionContextKind, SubsessionHandoffFieldKind, SubsessionHandoffPacket,
    SubsessionResultEnvelope,
};

use super::{AgentExecutor, Conversation, SubsessionState};
use crate::ToolCall;

const MAX_SUBSESSION_HANDOFF_TAIL_FIELDS: usize = 3;
const MAX_SUBSESSION_FIELD_CHARS: usize = 4_000;
const MAX_SUBSESSION_TAIL_FIELD_CHARS: usize = 1_200;

pub(super) fn agent_execution_context(info: &crate::AgentInfo) -> ExecutionRequestContext {
    ExecutionRequestContext {
        provider_id: info.model.as_ref().map(|m| m.provider_id.clone()),
        model_id: info.model.as_ref().map(|m| m.model_id.clone()),
        max_tokens: info.max_tokens,
        temperature: info.temperature,
        top_p: info.top_p,
        variant: info.variant.clone(),
        provider_options: (!info.options.is_empty()).then_some(info.options.clone()),
    }
}

pub(super) fn append_provider_message(
    conversation: &mut Conversation,
    message: &rocode_provider::Message,
    tool_name_by_id: &mut HashMap<String, String>,
) {
    match message.role {
        rocode_provider::Role::System => {
            let text = extract_text_from_provider_content(&message.content);
            conversation
                .messages
                .push(crate::AgentMessage::system(text));
        }
        rocode_provider::Role::User => {
            let text = extract_text_from_provider_content(&message.content);
            let attachments = extract_attachments_from_provider_content(&message.content);
            if attachments.is_empty() {
                conversation.add_user_message(text);
            } else {
                conversation.add_user_message_with_attachments(text, attachments);
            }
        }
        rocode_provider::Role::Assistant => match &message.content {
            rocode_provider::Content::Text(text) => {
                conversation.add_assistant_message(text.clone());
            }
            rocode_provider::Content::Parts(parts) => {
                let mut text = String::new();
                let mut reasoning = String::new();
                let mut tool_calls: Vec<ToolCall> = Vec::new();
                for part in parts {
                    if part.content_type == "reasoning" {
                        if let Some(part_text) = &part.text {
                            reasoning.push_str(part_text);
                        }
                        continue;
                    }
                    if let Some(part_text) = &part.text {
                        text.push_str(part_text);
                    }
                    if let Some(tool_use) = &part.tool_use {
                        tool_name_by_id.insert(tool_use.id.clone(), tool_use.name.clone());
                        tool_calls.push(ToolCall {
                            id: tool_use.id.clone(),
                            name: tool_use.name.clone(),
                            arguments: tool_use.input.clone(),
                        });
                    }
                }
                if tool_calls.is_empty() {
                    if reasoning.is_empty() {
                        conversation.add_assistant_message(text);
                    } else {
                        conversation.add_assistant_message_with_reasoning(reasoning, text);
                    }
                } else {
                    if reasoning.is_empty() {
                        conversation.add_assistant_message_with_tools(text, tool_calls);
                    } else {
                        conversation.add_assistant_message_with_reasoning_and_tools(
                            reasoning, text, tool_calls,
                        );
                    }
                }
            }
        },
        rocode_provider::Role::Tool => {
            if let rocode_provider::Content::Parts(parts) = &message.content {
                let mut appended = false;
                for part in parts {
                    if let Some(result) = &part.tool_result {
                        let name = tool_name_by_id
                            .get(&result.tool_use_id)
                            .cloned()
                            .unwrap_or_else(|| "tool".to_string());
                        conversation.add_tool_result(
                            result.tool_use_id.clone(),
                            name,
                            result.content.clone(),
                            result.is_error.unwrap_or(false),
                        );
                        appended = true;
                    }
                }
                if appended {
                    return;
                }
            }

            conversation.add_tool_result(
                "".to_string(),
                "tool".to_string(),
                extract_text_from_provider_content(&message.content),
                false,
            );
        }
    }
}

pub(super) fn extract_text_from_provider_content(content: &rocode_provider::Content) -> String {
    match content {
        rocode_provider::Content::Text(text) => text.clone(),
        rocode_provider::Content::Parts(parts) => parts
            .iter()
            .filter(|part| part.content_type != "reasoning")
            .filter_map(|part| part.text.clone())
            .collect::<Vec<_>>()
            .join(""),
    }
}

pub(super) fn extract_attachments_from_provider_content(
    content: &rocode_provider::Content,
) -> Vec<crate::Attachment> {
    match content {
        rocode_provider::Content::Text(_) => Vec::new(),
        rocode_provider::Content::Parts(parts) => parts
            .iter()
            .filter_map(|part| {
                if part.content_type == "image_url" || part.content_type == "image" {
                    part.image_url.as_ref().map(|img| crate::Attachment {
                        url: img.url.clone(),
                        mime: part
                            .media_type
                            .clone()
                            .unwrap_or_else(|| "image/png".to_string()),
                        filename: part.filename.clone(),
                    })
                } else {
                    None
                }
            })
            .collect(),
    }
}

pub(super) fn map_tool_error(error: ToolError) -> OrchestratorToolExecError {
    match error {
        ToolError::InvalidArguments(message) => {
            OrchestratorToolExecError::InvalidArguments(message)
        }
        ToolError::PermissionDenied(message) => {
            OrchestratorToolExecError::PermissionDenied(message)
        }
        other => OrchestratorToolExecError::ExecutionError(other.to_string()),
    }
}

pub(super) fn attach_subsession_callbacks(
    ctx: ToolContext,
    subsessions: Arc<Mutex<HashMap<String, SubsessionState>>>,
    providers: Arc<ProviderRegistry>,
    tools: Arc<ToolRegistry>,
    agent_registry: Arc<crate::AgentRegistry>,
) -> ToolContext {
    let parent_abort = ctx.abort.clone();
    let tool_runtime_config = ctx.runtime_config.clone();

    let ctx = ctx.with_get_agent_info({
        let registry = agent_registry.clone();
        move |name| {
            let registry = registry.clone();
            async move {
                Ok(registry.get(&name).map(|info| rocode_tool::TaskAgentInfo {
                    name: info.name.clone(),
                    model: info.model.as_ref().map(|m| rocode_tool::TaskAgentModel {
                        provider_id: m.provider_id.clone(),
                        model_id: m.model_id.clone(),
                    }),
                    can_use_task: info.is_tool_allowed("task"),
                    steps: info.max_steps,
                    execution: Some(agent_execution_context(info)),
                    max_tokens: info.max_tokens,
                    temperature: info.temperature,
                    top_p: info.top_p,
                    variant: info.variant.clone(),
                }))
            }
        }
    });

    // Bridge the build_agent callback so that task tools can dynamically
    // construct agents at runtime without polluting the global registry.
    let parent_agent_name = ctx.agent.clone();
    let ctx = ctx.with_build_agent({
        let registry = agent_registry.clone();
        move |name, system_prompt, model, max_steps, allowed_tools| {
            let registry = registry.clone();
            let parent_agent_name = parent_agent_name.clone();
            async move {
                // If already registered, return the existing definition
                if let Some(info) = registry.get(&name) {
                    return Ok(rocode_tool::TaskAgentInfo {
                        name: info.name.clone(),
                        model: info.model.as_ref().map(|m| rocode_tool::TaskAgentModel {
                            provider_id: m.provider_id.clone(),
                            model_id: m.model_id.clone(),
                        }),
                        can_use_task: info.is_tool_allowed("task"),
                        steps: info.max_steps,
                        execution: Some(agent_execution_context(info)),
                        max_tokens: info.max_tokens,
                        temperature: info.temperature,
                        top_p: info.top_p,
                        variant: info.variant.clone(),
                    });
                }

                // Build a temporary agent from the inline spec
                let mut agent = crate::AgentInfo::custom(&name);
                if let Some(prompt) = system_prompt {
                    agent = agent.with_system_prompt(prompt);
                }
                if let Some(steps) = max_steps {
                    agent = agent.with_max_steps(steps);
                }
                if let Some((provider_id, model_id)) = parse_model_string(model.as_deref()) {
                    agent = agent.with_model(model_id, provider_id);
                }

                // Permission inheritance: intersect requested tools with
                // parent agent's allowed tools to prevent privilege escalation.
                if !allowed_tools.is_empty() {
                    let parent_tools: std::collections::HashSet<String> = registry
                        .get(&parent_agent_name)
                        .map(|p| p.allowed_tools.iter().cloned().collect())
                        .unwrap_or_default();
                    let inherited: Vec<String> = allowed_tools
                        .into_iter()
                        .filter(|tool| parent_tools.contains(tool))
                        .collect();
                    // Re-derive permission from the parent's ruleset for safety
                    let ruleset = rocode_permission::build_agent_ruleset(&name, &[]);
                    agent.allowed_tools = inherited;
                    agent = agent.with_permission(ruleset);
                }

                Ok(rocode_tool::TaskAgentInfo {
                    name: agent.name.clone(),
                    model: agent.model.as_ref().map(|m| rocode_tool::TaskAgentModel {
                        provider_id: m.provider_id.clone(),
                        model_id: m.model_id.clone(),
                    }),
                    can_use_task: agent.is_tool_allowed("task"),
                    steps: agent.max_steps,
                    execution: Some(agent_execution_context(&agent)),
                    max_tokens: agent.max_tokens,
                    temperature: agent.temperature,
                    top_p: agent.top_p,
                    variant: agent.variant.clone(),
                })
            }
        }
    });

    ctx.with_create_subsession({
        let subsessions = subsessions.clone();
        let registry = agent_registry.clone();
        move |agent_name, _title, model, disabled_tools| {
            let subsessions = subsessions.clone();
            let registry = registry.clone();
            async move {
                let mut agent = registry.get(&agent_name).cloned().ok_or_else(|| {
                    ToolError::InvalidArguments(format!(
                        "Unknown agent type: {} is not a valid agent type",
                        agent_name
                    ))
                })?;

                if let Some((provider_id, model_id)) = parse_model_string(model.as_deref()) {
                    agent = agent.with_model(model_id, provider_id);
                }

                let conversation = if let Some(system_prompt) = &agent.system_prompt {
                    Conversation::with_system_prompt(system_prompt.clone())
                } else {
                    Conversation::new()
                };

                // A delegated subsession starts fresh and receives context only
                // through explicit handoff on prompt_subsession.
                let session_id = format!("task_{}_{}", agent_name, uuid::Uuid::new_v4().simple());
                let mut store = subsessions.lock().await;
                store.insert(
                    session_id.clone(),
                    SubsessionState {
                        kind: SessionContextKind::DelegatedSubsession,
                        agent,
                        conversation,
                        disabled_tools: disabled_tools.into_iter().collect(),
                    },
                );
                Ok(session_id)
            }
        }
    })
    .with_create_synthetic_message({
        let subsessions = subsessions.clone();
        move |session_id, agent, text, attachments| {
            let subsessions = subsessions.clone();
            async move {
                let mut store = subsessions.lock().await;
                let Some(state) = store.get_mut(&session_id) else {
                    return Ok(());
                };
                let mapped = attachments
                    .into_iter()
                    .map(|attachment| crate::Attachment {
                        url: attachment.url,
                        mime: attachment.mime,
                        filename: attachment.filename,
                    })
                    .collect::<Vec<_>>();
                let mut content = text;
                if content.trim().is_empty() && !mapped.is_empty() {
                    content = " ".to_string();
                }
                let message = if mapped.is_empty() {
                    crate::AgentMessage::user(content)
                } else {
                    crate::AgentMessage::user_with_attachments(content, mapped)
                };
                if let Some(agent_name) = agent {
                    tracing::debug!(session_id = %session_id, synthetic_agent = %agent_name, "attached synthetic message to subsession conversation");
                }
                state.conversation.messages.push(message);
                Ok(())
            }
        }
    })
    .with_prompt_subsession({
        let subsessions = subsessions.clone();
        let providers = providers.clone();
        let tools = tools.clone();
        let registry = agent_registry.clone();
        let parent_abort = parent_abort.clone();
        let tool_runtime_config = tool_runtime_config.clone();
        move |session_id, handoff| {
            let subsessions = subsessions.clone();
            let providers = providers.clone();
            let tools = tools.clone();
            let registry = registry.clone();
            let parent_abort = parent_abort.clone();
            let tool_runtime_config = tool_runtime_config.clone();
            async move {
                let state = {
                    let store = subsessions.lock().await;
                    store.get(&session_id).cloned()
                }
                .ok_or_else(|| {
                    ToolError::ExecutionError(format!(
                        "Unknown subagent session: {}. Start without task_id first.",
                        session_id
                    ))
                })?;

                let mut executor = AgentExecutor::new(
                    state.agent,
                    providers.clone(),
                    tools.clone(),
                    registry.clone(),
                )
                .with_tool_runtime_config(tool_runtime_config.clone())
                .with_disabled_tools(state.disabled_tools.iter().cloned());
                executor.conversation = state.conversation;

                let output = executor
                    .execute_subsession_with_cancel_token(
                        render_subsession_handoff_packet(&handoff),
                        parent_abort.clone(),
                    )
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionError(format!("Subagent execution failed: {}", e))
                    })?;

                let mut store = subsessions.lock().await;
                if let Some(state) = store.get_mut(&session_id) {
                    state.conversation = executor.conversation.clone();
                }

                Ok(SubsessionResultEnvelope::summary(output))
            }
        }
    })
}

fn render_subsession_handoff_packet(packet: &SubsessionHandoffPacket) -> String {
    let mut sections = Vec::new();
    let mut sanctioned_tail_count = 0usize;
    for field in &packet.fields {
        let text = field.text.trim();
        if text.is_empty() {
            continue;
        }
        let text = if matches!(field.kind, SubsessionHandoffFieldKind::SanctionedRecentTail) {
            if sanctioned_tail_count >= MAX_SUBSESSION_HANDOFF_TAIL_FIELDS {
                continue;
            }
            sanctioned_tail_count += 1;
            truncate_subsession_field(text, MAX_SUBSESSION_TAIL_FIELD_CHARS)
        } else {
            truncate_subsession_field(text, MAX_SUBSESSION_FIELD_CHARS)
        };
        let label = match field.kind {
            SubsessionHandoffFieldKind::Goal => "Goal",
            SubsessionHandoffFieldKind::Constraint => "Constraints",
            SubsessionHandoffFieldKind::Fact => "Facts",
            SubsessionHandoffFieldKind::RequiredPath => "Required Paths",
            SubsessionHandoffFieldKind::SupportingContext => "Supporting Context",
            SubsessionHandoffFieldKind::PreflightContext => "Preflight Context",
            SubsessionHandoffFieldKind::RecentConclusion => "Recent Conclusions",
            SubsessionHandoffFieldKind::SanctionedRecentTail => "Sanctioned Recent Tail",
        };
        let heading = field
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|title| format!("## {}: {}", label, title))
            .unwrap_or_else(|| format!("## {}", label));
        sections.push(format!("{}\n\n{}", heading, text));
    }

    if sections.is_empty() {
        String::new()
    } else {
        sections.join("\n\n")
    }
}

fn truncate_subsession_field(text: &str, max_chars: usize) -> String {
    let truncated = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

pub(super) fn parse_model_string(raw: Option<&str>) -> Option<(String, String)> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }

    let (provider, model) = raw.split_once(':').or_else(|| raw.split_once('/'))?;

    if provider.is_empty() || model.is_empty() {
        return None;
    }

    Some((provider.to_string(), model.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_provider_message_preserves_assistant_reasoning() {
        let provider_message = rocode_provider::Message::assistant_parts(vec![
            rocode_provider::ContentPart::reasoning("internal trace".to_string()),
            rocode_provider::ContentPart::text("visible answer".to_string()),
            rocode_provider::ContentPart::tool_use(
                "tool-call-0".to_string(),
                "ls".to_string(),
                serde_json::json!({"path":"."}),
            ),
        ]);
        let mut conversation = Conversation::new();
        let mut tool_name_by_id = HashMap::new();

        append_provider_message(&mut conversation, &provider_message, &mut tool_name_by_id);

        assert_eq!(conversation.messages.len(), 1);
        let message = &conversation.messages[0];
        assert_eq!(message.reasoning, "internal trace");
        assert_eq!(message.content, "visible answer");
        assert_eq!(message.tool_calls.len(), 1);
        assert_eq!(message.tool_calls[0].id, "tool-call-0");
    }
}
