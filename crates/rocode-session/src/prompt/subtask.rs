use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::anyhow;
use rocode_execution_types::{
    inline_subtask_request_defaults, session_runtime_request_defaults, CompiledExecutionRequest,
    ExecutionRequestContext,
};
use rocode_provider::{
    Content, ContentPart, Message, Provider, ProviderProfileResolver, ProviderQuirk, Role,
    ToolDefinition, ToolResult as ProviderToolResult,
};
use rocode_tool::{ToolContext, ToolError};
use rocode_types::SubsessionHandoffPacket;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{AgentParams, AskPermissionHook, AskQuestionHook, ModelRef};

const TASK_STATUS_COMPLETED: &str = "completed";
const TASK_NO_TEXT_OUTPUT_MESSAGE: &str =
    "Task completed successfully. No textual output was returned by subagent.";
const MAX_STEPS_SUMMARY_PROMPT: &str = "You have reached the maximum allowed steps for this subtask. Do NOT make any more tool calls. Return a concise final summary of work completed and any remaining work.";
const SUBSESSION_REASONING_OPTION_KEYS: &[&str] = &[
    "reasoning",
    "reasoning_effort",
    "reasoningEffort",
    "thinking",
    "include_reasoning",
    "includeReasoning",
    "enable_thinking",
    "thinkingConfig",
];

#[derive(Debug, Clone)]
struct InlineToolCall {
    id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub fn tool_definitions_from_schemas(schemas: Vec<ToolSchema>) -> Vec<ToolDefinition> {
    schemas
        .into_iter()
        .map(|s| ToolDefinition {
            name: s.name,
            description: Some(s.description),
            parameters: s.parameters,
        })
        .collect()
}

fn build_inline_tool_definitions(
    schemas: Vec<rocode_tool::ToolSchema>,
    disabled: &HashSet<&str>,
) -> Vec<ToolDefinition> {
    let tool_defs: Vec<ToolDefinition> = schemas
        .into_iter()
        .filter(|schema| !disabled.contains(schema.name.as_str()))
        .map(|schema| ToolDefinition {
            name: schema.name,
            description: Some(schema.description),
            parameters: schema.parameters,
        })
        .collect();
    tool_defs
}

fn provider_requires_thinking_replay_for_options(
    provider_id: &str,
    provider_options: Option<&HashMap<String, Value>>,
) -> bool {
    let options = provider_options.cloned().unwrap_or_default();
    ProviderProfileResolver::try_resolve_with_options(provider_id, &options)
        .map(|profile| {
            profile
                .quirks
                .contains(ProviderQuirk::RequiresThinkingReplay)
        })
        .unwrap_or(false)
}

fn strip_reasoning_provider_options_for_new_continuation(
    mut provider_options: HashMap<String, Value>,
) -> (Option<HashMap<String, Value>>, Vec<String>) {
    let mut removed = Vec::new();

    for key in SUBSESSION_REASONING_OPTION_KEYS {
        if provider_options.remove(*key).is_some() {
            removed.push((*key).to_string());
        }
    }

    if provider_options.contains_key("chat_template_args") {
        provider_options.remove("chat_template_args");
        removed.push("chat_template_args".to_string());
    }

    let provider_options = if provider_options.is_empty() {
        None
    } else {
        Some(provider_options)
    };

    (provider_options, removed)
}

fn sanitize_compiled_request_for_new_subsession_boundary(
    provider_id: &str,
    compiled_request: CompiledExecutionRequest,
) -> CompiledExecutionRequest {
    if !provider_requires_thinking_replay_for_options(
        provider_id,
        compiled_request.provider_options.as_ref(),
    ) {
        return compiled_request;
    }

    let Some(provider_options) = compiled_request.provider_options.clone() else {
        return compiled_request;
    };

    let (provider_options, removed) =
        strip_reasoning_provider_options_for_new_continuation(provider_options);
    if removed.is_empty() {
        return compiled_request;
    }

    tracing::info!(
        provider_id = provider_id,
        removed_keys = ?removed,
        "starting delegated subtask at a fresh continuation boundary; stripping replay-sensitive reasoning options"
    );

    CompiledExecutionRequest {
        provider_options,
        ..compiled_request
    }
}

pub struct SubtaskExecutor {
    pub agent_name: String,
    pub prompt: String,
    pub description: Option<String>,
    pub model: Option<ModelRef>,
    pub execution: Option<ExecutionRequestContext>,
    pub variant: Option<String>,
    pub working_directory: Option<String>,
    pub agent_params: AgentParams,
    pub max_steps: Option<u32>,
    pub ask_question_hook: Option<AskQuestionHook>,
    pub ask_permission_hook: Option<AskPermissionHook>,
    pub question_session_id: Option<String>,
    pub tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    pub config_store: Option<Arc<rocode_config::ConfigStore>>,
    /// Cancellation token — checked at the top of each loop iteration.
    pub abort: Option<CancellationToken>,
}

impl SubtaskExecutor {
    pub fn new(agent_name: &str, prompt: &str) -> Self {
        Self {
            agent_name: agent_name.to_string(),
            prompt: prompt.to_string(),
            description: None,
            model: None,
            execution: None,
            variant: None,
            working_directory: None,
            agent_params: AgentParams::default(),
            max_steps: None,
            ask_question_hook: None,
            ask_permission_hook: None,
            question_session_id: None,
            tool_runtime_config: rocode_tool::ToolRuntimeConfig::default(),
            config_store: None,
            abort: None,
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn with_model(mut self, model: ModelRef) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_execution_context(mut self, execution: Option<ExecutionRequestContext>) -> Self {
        self.execution = execution;
        self
    }

    pub fn with_variant(mut self, variant: Option<String>) -> Self {
        self.variant = variant;
        self
    }

    pub fn with_working_directory(mut self, directory: impl Into<String>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    pub fn with_ask_question_hook(
        mut self,
        ask_question_hook: AskQuestionHook,
        session_id: String,
    ) -> Self {
        self.ask_question_hook = Some(ask_question_hook);
        self.question_session_id = Some(session_id);
        self
    }

    pub fn with_ask_permission_hook(mut self, ask_permission_hook: AskPermissionHook) -> Self {
        self.ask_permission_hook = Some(ask_permission_hook);
        self
    }

    pub fn with_max_steps(mut self, max_steps: Option<u32>) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub fn with_tool_runtime_config(
        mut self,
        tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    ) -> Self {
        self.tool_runtime_config = tool_runtime_config;
        self
    }

    pub fn with_config_store(mut self, config_store: Arc<rocode_config::ConfigStore>) -> Self {
        self.config_store = Some(config_store);
        self
    }

    pub fn with_abort(mut self, token: CancellationToken) -> Self {
        self.abort = Some(token);
        self
    }

    fn format_task_output(subsession_id: &str, result_text: &str) -> String {
        let task_body = if result_text.trim().is_empty() {
            TASK_NO_TEXT_OUTPUT_MESSAGE.to_string()
        } else {
            result_text.to_string()
        };
        format!(
            "task_id: {} (for resuming to continue this task if needed)\ntask_status: {}\n\n<task_result>\n{}\n</task_result>",
            subsession_id, TASK_STATUS_COMPLETED, task_body
        )
    }

    pub async fn execute(
        &self,
        provider: Arc<dyn Provider>,
        tool_registry: &rocode_tool::ToolRegistry,
        ctx: &rocode_tool::ToolContext,
    ) -> anyhow::Result<String> {
        let model = self.resolved_model();
        let model_ref = format!("{}:{}", model.provider_id, model.model_id);
        let title = self
            .description
            .clone()
            .unwrap_or_else(|| "Subtask".to_string());

        let subsession_id = ctx
            .do_create_subsession(
                self.agent_name.clone(),
                Some(title.clone()),
                Some(model_ref),
                vec!["todowrite".to_string(), "todoread".to_string()],
            )
            .await
            .unwrap_or_else(|_| format!("task_{}_{}", self.agent_name, uuid::Uuid::new_v4()));

        if let Ok(output) = ctx
            .do_prompt_subsession(
                subsession_id.clone(),
                SubsessionHandoffPacket::bounded_goal(self.prompt.clone()),
            )
            .await
        {
            return Ok(Self::format_task_output(&subsession_id, &output.text));
        }

        let output = self.execute_inline(provider, tool_registry, &[]).await?;
        Ok(Self::format_task_output(&subsession_id, &output))
    }

    pub async fn execute_inline(
        &self,
        provider: Arc<dyn Provider>,
        tool_registry: &rocode_tool::ToolRegistry,
        disabled_tools: &[String],
    ) -> anyhow::Result<String> {
        let model = self.resolved_model();
        let disabled: HashSet<&str> = disabled_tools.iter().map(|s| s.as_str()).collect();
        let tools = tool_registry.list_schemas().await;
        let tool_defs = build_inline_tool_definitions(tools, &disabled);

        let directory = self
            .working_directory
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
        let mut messages = vec![Message::user(&self.prompt)];
        let mut executed_tool_calls: u32 = 0;
        let compiled_request = sanitize_compiled_request_for_new_subsession_boundary(
            provider.id(),
            self.compiled_request_for_model(&model),
        );

        let mut step: u32 = 0;
        loop {
            step = step.saturating_add(1);

            // Cancellation checkpoint: abort early if the parent requested cancellation.
            if let Some(ref token) = self.abort {
                if token.is_cancelled() {
                    tracing::info!(step, "subtask cancelled by parent");
                    return Err(anyhow!("Task cancelled"));
                }
            }

            let is_last_step = self.max_steps.is_some_and(|limit| step >= limit);
            let mut request_messages = messages.clone();
            if is_last_step {
                request_messages.push(Message {
                    role: Role::Assistant,
                    content: Content::Text(MAX_STEPS_SUMMARY_PROMPT.to_string()),
                    cache_control: None,
                    provider_options: None,
                });
            }
            let request = compiled_request
                .inherit_missing(&session_runtime_request_defaults(None))
                .to_chat_request(request_messages, tool_defs.clone(), false);

            let response = provider.chat(request).await?;
            let choice = response
                .choices
                .first()
                .ok_or_else(|| anyhow!("subtask provider returned no choices"))?;
            let (text_output, tool_calls) = extract_text_and_tool_calls(&choice.message.content);

            if tool_calls.is_empty() {
                if text_output.trim().is_empty() && executed_tool_calls == 0 {
                    return Err(anyhow!(
                        "subtask returned no text and executed no tool calls"
                    ));
                }
                return Ok(text_output);
            }

            if is_last_step {
                if !text_output.trim().is_empty() {
                    return Ok(text_output);
                }
                return Ok(
                    "Subtask reached its configured step limit; returning without further tool execution."
                        .to_string(),
                );
            }

            tracing::debug!(
                step,
                tool_call_count = tool_calls.len(),
                "subtask executing tool calls"
            );

            messages.push(Message {
                role: Role::Assistant,
                content: choice.message.content.clone(),
                cache_control: None,
                provider_options: None,
            });

            for tool_call in tool_calls {
                let mut ctx = ToolContext::new(
                    "subtask".to_string(),
                    "subtask".to_string(),
                    directory.clone(),
                )
                .with_agent(self.agent_name.clone())
                .with_tool_runtime_config(self.tool_runtime_config.clone());
                if let Some(config_store) = self.config_store.clone() {
                    ctx = ctx.with_config_store(config_store);
                }
                if let Some(question_hook) = self.ask_question_hook.clone() {
                    let question_session_id = self
                        .question_session_id
                        .clone()
                        .unwrap_or_else(|| "subtask".to_string());
                    ctx = ctx.with_ask_question(move |questions| {
                        let question_hook = question_hook.clone();
                        let question_session_id = question_session_id.clone();
                        async move { question_hook(question_session_id, questions).await }
                    });
                }
                if let Some(permission_hook) = self.ask_permission_hook.clone() {
                    let permission_session_id = self
                        .question_session_id
                        .clone()
                        .unwrap_or_else(|| "subtask".to_string());
                    ctx = ctx.with_ask(move |request| {
                        let permission_hook = permission_hook.clone();
                        let permission_session_id = permission_session_id.clone();
                        async move { permission_hook(permission_session_id, request).await }
                    });
                }
                ctx.call_id = Some(tool_call.id.clone());

                let execution = if disabled.contains(tool_call.name.as_str()) {
                    Err(ToolError::PermissionDenied(format!(
                        "Tool '{}' is disabled for this subagent session",
                        tool_call.name
                    )))
                } else {
                    tool_registry
                        .execute(&tool_call.name, tool_call.input.clone(), ctx)
                        .await
                        .map(|result| result.output)
                };

                let (tool_output, is_error) = match execution {
                    Ok(output) => (output, false),
                    Err(err) => (err.to_string(), true),
                };

                messages.push(build_tool_result_message(
                    tool_call.id.as_str(),
                    tool_output,
                    is_error,
                ));
                executed_tool_calls += 1;
            }
        }
    }
}

impl SubtaskExecutor {
    fn resolved_model(&self) -> ModelRef {
        self.model
            .as_ref()
            .cloned()
            .or_else(|| {
                self.execution
                    .as_ref()
                    .and_then(|execution| execution.model_ref())
                    .map(|model| ModelRef {
                        provider_id: model.provider_id,
                        model_id: model.model_id,
                    })
            })
            .unwrap_or(ModelRef {
                provider_id: "default".to_string(),
                model_id: "default".to_string(),
            })
    }

    fn compiled_request_for_model(&self, model: &ModelRef) -> CompiledExecutionRequest {
        let defaults = inline_subtask_request_defaults(self.variant.clone())
            .with_max_tokens(self.agent_params.max_tokens)
            .with_temperature(self.agent_params.temperature)
            .with_top_p(self.agent_params.top_p);
        self.execution
            .as_ref()
            .map(|execution| {
                execution.compile_with_model_and_defaults(model.model_id.clone(), &defaults)
            })
            .unwrap_or_else(|| {
                CompiledExecutionRequest {
                    model_id: model.model_id.clone(),
                    ..Default::default()
                }
                .inherit_missing(&defaults)
            })
    }
}

fn extract_text_and_tool_calls(content: &Content) -> (String, Vec<InlineToolCall>) {
    match content {
        Content::Text(text) => (text.clone(), Vec::new()),
        Content::Parts(parts) => {
            let mut text = String::new();
            let mut tool_calls = Vec::new();

            for part in parts {
                if part.content_type != "reasoning" {
                    if let Some(part_text) = &part.text {
                        text.push_str(part_text);
                    }
                }

                if part.content_type == "tool_use" {
                    if let Some(tool_use) = &part.tool_use {
                        tool_calls.push(InlineToolCall {
                            id: tool_use.id.clone(),
                            name: tool_use.name.clone(),
                            input: tool_use.input.clone(),
                        });
                    }
                }
            }

            (text, tool_calls)
        }
    }
}

fn build_tool_result_message(tool_call_id: &str, output: String, is_error: bool) -> Message {
    Message {
        role: Role::Tool,
        content: Content::Parts(vec![ContentPart {
            content_type: "tool_result".to_string(),
            text: None,
            image_url: None,
            tool_use: None,
            tool_result: Some(ProviderToolResult {
                tool_use_id: tool_call_id.to_string(),
                content: output,
                is_error: Some(is_error),
            }),
            cache_control: None,
            filename: None,
            media_type: None,
            provider_options: None,
        }]),
        cache_control: None,
        provider_options: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_inline_tool_definitions_preserves_runtime_input_order() {
        let disabled = HashSet::new();
        let tools = vec![
            rocode_tool::ToolSchema {
                name: "websearch".to_string(),
                description: "web".to_string(),
                parameters: serde_json::json!({}),
                source_kind: rocode_tool::ToolSchemaSourceKind::BuiltIn,
            },
            rocode_tool::ToolSchema {
                name: "task".to_string(),
                description: "task".to_string(),
                parameters: serde_json::json!({}),
                source_kind: rocode_tool::ToolSchemaSourceKind::BuiltIn,
            },
            rocode_tool::ToolSchema {
                name: "task_flow".to_string(),
                description: "task flow".to_string(),
                parameters: serde_json::json!({}),
                source_kind: rocode_tool::ToolSchemaSourceKind::BuiltIn,
            },
        ];

        let tool_defs = build_inline_tool_definitions(tools, &disabled);
        let names: Vec<&str> = tool_defs.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, vec!["websearch", "task", "task_flow"]);
    }

    #[test]
    fn sanitize_compiled_request_strips_reasoning_options_for_replay_sensitive_provider() {
        let compiled = CompiledExecutionRequest {
            model_id: "deepseek-reasoner".to_string(),
            provider_options: Some(HashMap::from([
                (
                    "thinking".to_string(),
                    serde_json::json!({"type": "enabled"}),
                ),
                ("reasoningEffort".to_string(), serde_json::json!("high")),
                (
                    "chat_template_args".to_string(),
                    serde_json::json!({"foo": "bar"}),
                ),
                (
                    "tool_choice".to_string(),
                    serde_json::json!({"type":"function","function":{"name":"read"}}),
                ),
            ])),
            ..Default::default()
        };

        let sanitized = sanitize_compiled_request_for_new_subsession_boundary("deepseek", compiled);
        let options = sanitized
            .provider_options
            .expect("tool_choice should remain after sanitization");

        assert!(!options.contains_key("thinking"));
        assert!(!options.contains_key("reasoningEffort"));
        assert!(!options.contains_key("chat_template_args"));
        assert!(options.contains_key("tool_choice"));
    }

    #[test]
    fn sanitize_compiled_request_preserves_reasoning_options_for_regular_provider() {
        let compiled = CompiledExecutionRequest {
            model_id: "gpt-4.1".to_string(),
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                serde_json::json!({"type": "enabled"}),
            )])),
            ..Default::default()
        };

        let sanitized = sanitize_compiled_request_for_new_subsession_boundary("openai", compiled);
        let options = sanitized
            .provider_options
            .expect("provider options should be preserved");

        assert!(options.contains_key("thinking"));
    }

    #[test]
    fn extract_text_and_tool_calls_keeps_reasoning_out_of_visible_text() {
        let content = Content::Parts(vec![
            ContentPart::reasoning("internal trace".to_string()),
            ContentPart::text("visible answer".to_string()),
            ContentPart::tool_use(
                "tool-call-0".to_string(),
                "read".to_string(),
                serde_json::json!({"path":"README.md"}),
            ),
        ]);

        let (text, tool_calls) = extract_text_and_tool_calls(&content);

        assert_eq!(text, "visible answer");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tool-call-0");
    }
}
