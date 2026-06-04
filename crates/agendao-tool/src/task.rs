use agendao_types::{
    SessionContextKind, SubsessionHandoffFieldKind, SubsessionHandoffPacket,
    SubsessionResultAbsorbMode,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::Arc;

use agendao_core::agent_task_registry::{global_task_registry, AgentTaskStatus};

use crate::skill_support::{load_skills_prompt_context, LoadedSkillsPromptContext};
use crate::{
    append_subsession_handoff_recent_tail_from_extra, append_tool_repair_event_map,
    merge_tool_repair_telemetry, structured_dangerous_exec_lifetimes, tool_repair_event, Metadata,
    PermissionRequest, TaskAgentInfo, TaskAgentModel, Tool, ToolContext, ToolError, ToolResult,
};

pub struct TaskTool;

struct NormalizedTaskArgs {
    args: Value,
    repair_metadata: Metadata,
}

impl TaskTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TaskTool {
    fn default() -> Self {
        Self::new()
    }
}

const TASK_STATUS_COMPLETED: &str = "completed";
const TASK_NO_TEXT_OUTPUT_MESSAGE: &str =
    "Task completed successfully. No textual output was returned by subagent.";

#[derive(Debug, Serialize, Deserialize)]
struct TaskInput {
    #[serde(default)]
    description: Option<String>,
    #[serde(
        default,
        alias = "request",
        alias = "instructions",
        alias = "goal",
        alias = "message",
        alias = "input"
    )]
    prompt: Option<String>,
    #[serde(alias = "subagentType", alias = "agent", default)]
    subagent_type: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(alias = "taskId", alias = "session_id", alias = "sessionId")]
    task_id: Option<String>,
    command: Option<String>,
    #[serde(alias = "loadSkills", alias = "load_skills", alias = "skills")]
    load_skills: Option<Vec<String>>,
    #[serde(default, alias = "runInBackground", alias = "background")]
    run_in_background: bool,
    /// Inline agent spec: custom system prompt for a dynamically constructed agent.
    #[serde(
        default,
        alias = "agentPrompt",
        alias = "agent_prompt",
        alias = "system_prompt"
    )]
    agent_prompt: Option<String>,
    /// Inline agent spec: allowed tools for a dynamically constructed agent.
    #[serde(
        default,
        alias = "agentTools",
        alias = "agent_tools",
        alias = "allowed_tools"
    )]
    agent_tools: Option<Vec<String>>,
}

#[derive(Debug)]
enum TaskDispatchKind {
    /// Named agent dispatch (subagent_type)
    Agent(String),
    /// Category dispatch → sisyphus-junior + model/prompt override
    Category(String),
}

impl TaskDispatchKind {
    fn label(&self) -> &str {
        match self {
            Self::Agent(name) => name,
            Self::Category(cat) => cat,
        }
    }

    fn scope_key(&self) -> String {
        match self {
            Self::Agent(name) => format!("task:agent:{}", name.trim().to_ascii_lowercase()),
            Self::Category(cat) => format!("task:category:{}", cat.trim().to_ascii_lowercase()),
        }
    }
}

#[derive(Debug)]
struct NormalizedTaskInput {
    description: String,
    prompt: String,
    dispatch: TaskDispatchKind,
    task_id: Option<String>,
    load_skills: Option<Vec<String>>,
    /// Inline agent system prompt for runtime agent construction.
    agent_prompt: Option<String>,
    /// Inline agent allowed tools for runtime agent construction.
    agent_tools: Option<Vec<String>>,
}

impl TaskInput {
    fn normalize(self) -> Result<NormalizedTaskInput, ToolError> {
        let prompt = self
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| ToolError::InvalidArguments("prompt is required. Use `description` only as a short label; put the real delegated instruction in `prompt`. Canonical shape: {\"subagent_type\":\"build\",\"prompt\":\"...\"}".to_string()))?;

        let description = self
            .description
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| derive_description_from_prompt(&prompt));

        let subagent_type = self
            .subagent_type
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);
        let category = self
            .category
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);

        let dispatch = match (subagent_type, category) {
            // Category takes precedence when both are provided
            (Some(primary), Some(category)) if primary != category => {
                tracing::warn!(
                    primary_subagent_type = %primary,
                    category = %category,
                    "task arguments had conflicting subagent_type/category; preferring category"
                );
                TaskDispatchKind::Category(category)
            }
            (Some(primary), Some(_)) => {
                // Same value in both fields — treat as agent dispatch
                TaskDispatchKind::Agent(primary)
            }
            (Some(primary), None) => TaskDispatchKind::Agent(primary),
            (None, Some(category)) => TaskDispatchKind::Category(category),
            (None, None) => {
                return Err(ToolError::InvalidArguments(
                    "subagent_type is required. Canonical shape: {\"subagent_type\":\"build\",\"prompt\":\"...\"}"
                        .to_string(),
                ));
            }
        };

        Ok(NormalizedTaskInput {
            description,
            prompt,
            dispatch,
            task_id: self.task_id,
            load_skills: self.load_skills,
            agent_prompt: self.agent_prompt,
            agent_tools: self.agent_tools,
        })
    }
}

fn derive_description_from_prompt(prompt: &str) -> String {
    let chosen = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("- ["))
        .or_else(|| prompt.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("Delegated task");

    let truncated = chosen.chars().take(40).collect::<String>();
    if truncated.is_empty() {
        "Delegated task".to_string()
    } else {
        truncated
    }
}

fn format_task_output(session_id: &str, result_text: &str) -> (String, bool) {
    let has_text_output = !result_text.trim().is_empty();
    let task_body = if has_text_output {
        result_text.to_string()
    } else {
        TASK_NO_TEXT_OUTPUT_MESSAGE.to_string()
    };

    (
        format!(
            "task_id: {} (for resuming to continue this task if needed)\ntask_status: {}\n\n<task_result>\n{}\n</task_result>",
            session_id, TASK_STATUS_COMPLETED, task_body
        ),
        has_text_output,
    )
}

fn build_task_handoff_packet(
    input: &NormalizedTaskInput,
    loaded_skills_context: &LoadedSkillsPromptContext,
    prompt_suffix: Option<&str>,
    ctx: &ToolContext,
) -> SubsessionHandoffPacket {
    let mut packet = SubsessionHandoffPacket::bounded_goal(input.prompt.clone());

    if !loaded_skills_context.is_empty() {
        packet.push_titled_text(
            SubsessionHandoffFieldKind::SupportingContext,
            "loaded skills context",
            loaded_skills_context.prompt_context.clone(),
        );
    }

    if let Some(suffix) = prompt_suffix
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        packet.push_titled_text(
            SubsessionHandoffFieldKind::Constraint,
            "dispatch guidance",
            suffix.to_string(),
        );
    }

    append_subsession_handoff_recent_tail_from_extra(&mut packet, &ctx.extra);
    packet
}

#[async_trait]
impl Tool for TaskTool {
    fn id(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Direct subagent dispatch. Prefer task_flow for lifecycle operations (create/resume/get/list/cancel). Use task only when you need raw subagent dispatch without lifecycle semantics.

Canonical shape:
{\"subagent_type\":\"build\",\"prompt\":\"...\"}

Legacy aliases accepted for recovery only (prefer the canonical names above):
`agent`→subagent_type, `request`/`instructions`/`goal`/`message`→prompt,
`title`/`summary`/`task`→description, `session_id`→task_id, `system_prompt`→agent_prompt,
`allowed_tools`→agent_tools."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "description": "The type of specialized agent to use for this task (e.g., 'explore', 'librarian', 'oracle'). Use any name for a runtime-constructed agent when paired with agent_prompt."
                },
                "description": {
                    "type": "string",
                    "description": "A short task label. If omitted, AgenDao derives it from the delegated prompt."
                },
                "prompt": {
                    "type": "string",
                    "description": "The task for the agent to perform."
                },
                "task_id": {
                    "type": "string",
                    "description": "Resume a previous task by passing its task_id."
                },
                "command": {
                    "type": "string",
                    "description": "The command that triggered this task (optional)"
                },
                "load_skills": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Skills to load for the sub-agent (optional)"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run the task in background (default: false)"
                },
                "agent_prompt": {
                    "type": "string",
                    "description": "Inline system prompt for a dynamically constructed agent. When subagent_type is not a known agent, this prompt defines the agent's role and behavior at runtime."
                },
                "agent_tools": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Allowed tools for a dynamically constructed agent. Only tools available to the parent agent can be granted."
                }
            },
            "required": ["subagent_type", "prompt"],
            "examples": [
                {
                    "subagent_type": "build",
                    "prompt": "Investigate the failing integration test and return a concrete fix."
                },
                {
                    "agent": "security-auditor",
                    "description": "Audit auth middleware",
                    "request": "Audit the authentication middleware for bypasses and missing checks."
                }
            ]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let normalized = normalize_task_args(args);
        let raw_input: TaskInput = serde_json::from_value(normalized.args)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let input = raw_input.normalize()?;

        let dispatch_label = input.dispatch.label().to_string();

        let bypass_check = ctx
            .extra
            .get("bypassAgentCheck")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !bypass_check {
            ctx.ask_permission(
                PermissionRequest::new("task")
                    .with_pattern(&dispatch_label)
                    .with_scope_key(input.dispatch.scope_key())
                    .with_supported_lifetimes(structured_dangerous_exec_lifetimes())
                    .with_risk_tag("dangerous_exec")
                    .with_metadata("description", serde_json::json!(&input.description))
                    .with_metadata("subagent_type", serde_json::json!(&dispatch_label))
                    .always_allow(),
            )
            .await?;
        }

        // Resolve agent info, model, and prompt suffix based on dispatch kind
        let (agent, preferred_model, prompt_suffix) = match &input.dispatch {
            TaskDispatchKind::Category(category) => {
                let category_info = ctx.do_resolve_category(category).await;
                // For category dispatch, use sisyphus-junior as the agent
                let agent = ctx.do_get_agent_info("sisyphus-junior").await;
                let preferred_model = if let Some(ref info) = category_info {
                    info.model
                        .as_ref()
                        .map(|m| format!("{}:{}", m.provider_id, m.model_id))
                } else {
                    None
                };
                let preferred_model = match preferred_model {
                    Some(m) => Some(m),
                    None => {
                        // Fall back to agent model, then last model
                        if let Some(model) = agent.as_ref().and_then(|a| {
                            a.model
                                .as_ref()
                                .map(|m| format!("{}:{}", m.provider_id, m.model_id))
                        }) {
                            Some(model)
                        } else {
                            ctx.do_get_last_model().await
                        }
                    }
                };
                let prompt_suffix = category_info.and_then(|info| info.prompt_suffix);
                (agent, preferred_model, prompt_suffix)
            }
            TaskDispatchKind::Agent(name) => {
                let agent = ctx.do_get_agent_info(name).await;

                // If the agent name is not found in the registry and the caller
                // provides an inline agent spec, dynamically build one.
                let agent = match (agent, &input.agent_prompt) {
                    (Some(info), _) => Some(info),
                    (None, Some(_agent_prompt)) => {
                        // Inline spec provided — attempt runtime construction
                        ctx.do_build_agent(
                            name.clone(),
                            input.agent_prompt.clone(),
                            None,
                            None,
                            input.agent_tools.clone().unwrap_or_default(),
                        )
                        .await
                        .ok()
                    }
                    (None, None) => None,
                };

                let preferred_model = if let Some(model) = agent.as_ref().and_then(|a| {
                    a.model
                        .as_ref()
                        .map(|m| format!("{}:{}", m.provider_id, m.model_id))
                }) {
                    Some(model)
                } else {
                    ctx.do_get_last_model().await
                };
                (agent, preferred_model, None)
            }
        };

        let disabled_tools = get_disabled_tools(agent.as_ref(), input.load_skills.as_ref());

        let session_id = if let Some(task_id) = &input.task_id {
            task_id.clone()
        } else {
            ctx.do_create_subsession(
                dispatch_label.clone(),
                Some(input.description.clone()),
                preferred_model.clone(),
                disabled_tools.clone(),
            )
            .await?
        };

        let title = input.description.clone();
        let loaded_skills_context = load_skills_prompt_context(
            Path::new(&ctx.directory),
            ctx.config_store.clone(),
            input.load_skills.as_deref(),
            Some(&ctx.extra),
        )?;
        let loaded_skill_names = loaded_skills_context.loaded_skill_names();
        let handoff = build_task_handoff_packet(
            &input,
            &loaded_skills_context,
            prompt_suffix.as_deref(),
            &ctx,
        );

        // Clone the abort token so the cancel callback can trigger it.
        let cancel_token = ctx.abort.clone();

        // Register task in AgentTaskRegistry for /tasks visibility.
        let agent_task_id = global_task_registry().register(
            Some(ctx.session_id.clone()),
            dispatch_label.clone(),
            input.description.clone(),
            agent.as_ref().and_then(|a| a.steps),
            Arc::new(move || cancel_token.cancel()),
        );

        // Notify RuntimeControlRegistry (if wired) so the agent task appears
        // in the execution topology with a parent link to the enclosing tool call.
        ctx.do_publish_bus(
            "agent_task.registered",
            serde_json::json!({
                "task_id": agent_task_id,
                "session_id": ctx.session_id,
                "agent_name": dispatch_label,
                "parent_tool_call_id": ctx.call_id,
            }),
        )
        .await;

        let result_text = match ctx
            .do_prompt_subsession(session_id.clone(), handoff.clone())
            .await
        {
            Ok(result) => {
                global_task_registry()
                    .complete(&agent_task_id, AgentTaskStatus::Completed { steps: 0 });
                ctx.do_publish_bus(
                    "agent_task.completed",
                    serde_json::json!({ "task_id": agent_task_id }),
                )
                .await;
                result.text
            }
            Err(e) => {
                let status = if ctx.abort.is_cancelled() {
                    AgentTaskStatus::Cancelled
                } else {
                    AgentTaskStatus::Failed {
                        error: e.to_string(),
                    }
                };
                global_task_registry().complete(&agent_task_id, status);
                ctx.do_publish_bus(
                    "agent_task.completed",
                    serde_json::json!({ "task_id": agent_task_id }),
                )
                .await;
                return Err(e);
            }
        };
        let model = parse_model_ref(preferred_model.as_deref());

        let (output, has_text_output) = format_task_output(&session_id, &result_text);

        let mut metadata = Metadata::new();
        metadata.insert("agentTaskId".into(), serde_json::json!(agent_task_id));
        metadata.insert("sessionId".into(), serde_json::json!(session_id));
        metadata.insert(
            "sessionContextKind".into(),
            serde_json::to_value(SessionContextKind::DelegatedSubsession)
                .unwrap_or_else(|_| serde_json::json!("delegated_subsession")),
        );
        metadata.insert(
            "taskStatus".into(),
            serde_json::json!(TASK_STATUS_COMPLETED),
        );
        metadata.insert(
            "sessionHandoffRichness".into(),
            serde_json::to_value(handoff.effective_richness())
                .unwrap_or_else(|_| serde_json::json!("bounded")),
        );
        metadata.insert(
            "resultAbsorbMode".into(),
            serde_json::to_value(SubsessionResultAbsorbMode::SummaryOnly)
                .unwrap_or_else(|_| serde_json::json!("summary_only")),
        );
        metadata.insert("hasTextOutput".into(), serde_json::json!(has_text_output));
        metadata.insert(
            "model".into(),
            serde_json::json!({
                "modelID": model.model_id,
                "providerID": model.provider_id,
            }),
        );
        if !loaded_skill_names.is_empty() {
            metadata.insert("loadedSkills".into(), serde_json::json!(loaded_skill_names));
            metadata.insert(
                "loadedSkillCount".into(),
                serde_json::json!(loaded_skill_names.len()),
            );
            metadata.insert(
                "loadedSkillViews".into(),
                serde_json::json!(loaded_skills_context.loaded_skills),
            );
        }

        let mut metadata = metadata;
        merge_tool_repair_telemetry(&mut metadata, &normalized.repair_metadata);
        Ok(ToolResult {
            title,
            output,
            metadata,
            truncated: false,
        })
    }
}

fn normalize_task_args(args: Value) -> NormalizedTaskArgs {
    let Value::Object(mut root) = args else {
        return NormalizedTaskArgs {
            args,
            repair_metadata: Metadata::new(),
        };
    };
    let mut repair_metadata = Metadata::new();
    let mut aliases = Vec::new();

    move_alias(&mut root, "agent", "subagent_type", &mut aliases);
    move_alias(&mut root, "session_id", "task_id", &mut aliases);
    move_alias(&mut root, "sessionId", "task_id", &mut aliases);
    move_alias(&mut root, "request", "prompt", &mut aliases);
    move_alias(&mut root, "instructions", "prompt", &mut aliases);
    move_alias(&mut root, "goal", "prompt", &mut aliases);
    move_alias(&mut root, "message", "prompt", &mut aliases);
    move_alias(&mut root, "input", "prompt", &mut aliases);
    move_alias(&mut root, "load_skills", "loadSkills", &mut aliases);
    move_alias(&mut root, "skills", "loadSkills", &mut aliases);
    move_alias(&mut root, "background", "runInBackground", &mut aliases);
    move_alias(&mut root, "system_prompt", "agentPrompt", &mut aliases);
    move_alias(&mut root, "agent_prompt", "agentPrompt", &mut aliases);
    move_alias(&mut root, "allowed_tools", "agentTools", &mut aliases);
    move_alias(&mut root, "agent_tools", "agentTools", &mut aliases);

    if root.get("description").is_none() {
        move_alias(&mut root, "title", "description", &mut aliases);
        move_alias(&mut root, "summary", "description", &mut aliases);
        move_alias(&mut root, "task", "description", &mut aliases);
        move_alias(&mut root, "label", "description", &mut aliases);
    }

    if root.get("prompt").is_none()
        && root.get("description").is_some()
        && root.get("subagent_type").is_some()
    {
        if let Some(description) = root.get("description").cloned() {
            root.insert("prompt".to_string(), description);
            let mut event = tool_repair_event("fallback_normalization", "tool", "task");
            event.insert("source".to_string(), serde_json::json!("description"));
            event.insert("target".to_string(), serde_json::json!("prompt"));
            append_tool_repair_event_map(&mut repair_metadata, event);
        }
    }

    if !aliases.is_empty() {
        let mut event = tool_repair_event("alias_normalization", "tool", "task");
        event.insert("aliases".to_string(), serde_json::json!(aliases));
        append_tool_repair_event_map(&mut repair_metadata, event);
    }

    NormalizedTaskArgs {
        args: Value::Object(root),
        repair_metadata,
    }
}

fn move_alias(root: &mut Map<String, Value>, from: &str, to: &str, aliases: &mut Vec<String>) {
    if root.get(to).is_some() {
        return;
    }
    if let Some(value) = root.remove(from) {
        root.insert(to.to_string(), value);
        aliases.push(format!("{from}->{to}"));
    }
}

fn get_disabled_tools(
    agent: Option<&TaskAgentInfo>,
    _load_skills: Option<&Vec<String>>,
) -> Vec<String> {
    let mut disabled = vec!["todowrite".to_string(), "todoread".to_string()];

    let has_task_permission = agent.map(|a| a.can_use_task).unwrap_or(false);
    if !has_task_permission {
        disabled.push("task".to_string());
    }

    disabled
}

fn parse_model_ref(raw: Option<&str>) -> TaskAgentModel {
    let Some(raw) = raw else {
        return TaskAgentModel {
            model_id: "default".to_string(),
            provider_id: "default".to_string(),
        };
    };

    let pair = raw.split_once(':').or_else(|| raw.split_once('/'));
    if let Some((provider, model)) = pair {
        return TaskAgentModel {
            model_id: model.to_string(),
            provider_id: provider.to_string(),
        };
    }

    TaskAgentModel {
        model_id: raw.to_string(),
        provider_id: "default".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::{SubsessionHandoffPacket, SubsessionResultEnvelope};
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    fn summary(text: &str) -> SubsessionResultEnvelope {
        SubsessionResultEnvelope::summary(text.to_string())
    }

    fn goal_text(packet: &SubsessionHandoffPacket) -> Option<&str> {
        packet
            .fields
            .iter()
            .find(|field| field.kind == SubsessionHandoffFieldKind::Goal)
            .map(|field| field.text.as_str())
    }

    fn has_field_containing(
        packet: &SubsessionHandoffPacket,
        kind: SubsessionHandoffFieldKind,
        needle: &str,
    ) -> bool {
        packet
            .fields
            .iter()
            .any(|field| field.kind == kind && field.text.contains(needle))
    }

    #[test]
    fn task_description_directs_lifecycle_semantics_to_task_flow() {
        let description = TaskTool::new().description().to_string();
        assert!(description.contains("Prefer task_flow"));
        assert!(description.contains("create/resume/get/list/cancel"));
        assert!(description.contains("Canonical shape"));
        assert!(description.contains("agent`→subagent_type"));
    }

    #[test]
    fn task_parameters_no_longer_require_description() {
        let schema = TaskTool::new().parameters();
        let required = schema["required"]
            .as_array()
            .expect("task schema should expose required");
        assert!(required.iter().any(|value| value == "subagent_type"));
        assert!(required.iter().any(|value| value == "prompt"));
        assert!(!required.iter().any(|value| value == "description"));
    }

    #[test]
    fn normalize_task_args_accepts_task_flow_style_aliases() {
        let raw = serde_json::json!({
            "agent": "security-auditor",
            "request": "Audit the auth middleware.",
            "title": "Auth audit",
            "session_id": "task_build_42",
            "allowed_tools": ["read", "grep"],
            "system_prompt": "You are a focused security auditor."
        });

        let normalized = normalize_task_args(raw);
        let input: TaskInput = serde_json::from_value(normalized.args)
            .expect("normalized task args should deserialize");
        assert_eq!(input.subagent_type.as_deref(), Some("security-auditor"));
        assert_eq!(input.prompt.as_deref(), Some("Audit the auth middleware."));
        assert_eq!(input.description.as_deref(), Some("Auth audit"));
        assert_eq!(input.task_id.as_deref(), Some("task_build_42"));
        assert_eq!(
            input.agent_tools.as_deref(),
            Some(&["read".to_string(), "grep".to_string()][..])
        );
        assert_eq!(
            input.agent_prompt.as_deref(),
            Some("You are a focused security auditor.")
        );
        let repair_events = crate::tool_repair_events(&normalized.repair_metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("alias_normalization")
        }));
    }

    #[test]
    fn normalize_task_args_can_promote_description_into_prompt() {
        let raw = serde_json::json!({
            "subagent_type": "build",
            "description": "Investigate the failing integration test."
        });

        let normalized = normalize_task_args(raw);
        let input: TaskInput = serde_json::from_value(normalized.args)
            .expect("normalized task args should deserialize");
        assert_eq!(
            input.prompt.as_deref(),
            Some("Investigate the failing integration test.")
        );
        let repair_events = crate::tool_repair_events(&normalized.repair_metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("fallback_normalization")
                && event.get("target").and_then(|value| value.as_str()) == Some("prompt")
        }));
    }

    #[tokio::test]
    async fn task_creates_subsession_and_prompts_it() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));
        let prompt_calls = Arc::new(Mutex::new(Vec::<(String, SubsessionHandoffPacket)>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|name| async move {
                if name == "build" {
                    Ok(Some(TaskAgentInfo {
                        name: "build".to_string(),
                        model: None,
                        can_use_task: true,
                        steps: None,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    }))
                } else {
                    Ok(None)
                }
            })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_build_123".to_string())
                    }
                }
            })
            .with_prompt_subsession({
                let prompt_calls = prompt_calls.clone();
                move |session_id, prompt| {
                    let prompt_calls = prompt_calls.clone();
                    async move {
                        prompt_calls.lock().await.push((session_id, prompt));
                        Ok(summary("subagent output"))
                    }
                }
            });

        let args = serde_json::json!({
            "description": "Investigate issue",
            "prompt": "Please inspect runtime behavior",
            "subagent_type": "build"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();

        assert_eq!(result.title, "Investigate issue");
        assert!(result
            .output
            .contains("task_id: task_build_123 (for resuming to continue this task if needed)"));
        assert!(result
            .output
            .contains("<task_result>\nsubagent output\n</task_result>"));
        assert_eq!(
            result.metadata.get("sessionId"),
            Some(&serde_json::json!("task_build_123"))
        );
        assert_eq!(
            result.metadata.get("sessionContextKind"),
            Some(&serde_json::json!("delegated_subsession"))
        );
        assert_eq!(
            result.metadata.get("sessionHandoffRichness"),
            Some(&serde_json::json!("bounded"))
        );
        assert_eq!(
            result.metadata.get("resultAbsorbMode"),
            Some(&serde_json::json!("summary_only"))
        );
        assert!(result
            .metadata
            .get("agentTaskId")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.starts_with('a')));
        assert_eq!(
            result.metadata.get("model"),
            Some(&serde_json::json!({
                "modelID": "model-y",
                "providerID": "provider-x"
            }))
        );

        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].0, "build");
        assert_eq!(create_calls[0].1, Some("Investigate issue".to_string()));
        assert_eq!(create_calls[0].2, Some("provider-x:model-y".to_string()));
        assert_eq!(
            create_calls[0].3,
            vec!["todowrite".to_string(), "todoread".to_string()]
        );

        let prompt_calls = prompt_calls.lock().await.clone();
        assert_eq!(prompt_calls.len(), 1);
        assert_eq!(prompt_calls[0].0, "task_build_123");
        assert_eq!(
            goal_text(&prompt_calls[0].1),
            Some("Please inspect runtime behavior")
        );
    }

    #[tokio::test]
    async fn task_permission_request_uses_scope_only_matcher_and_session_lifetimes() {
        let requests = Arc::new(Mutex::new(Vec::<crate::PermissionRequest>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_ask({
                let requests = requests.clone();
                move |req| {
                    let requests = requests.clone();
                    async move {
                        requests.lock().await.push(req);
                        Ok(())
                    }
                }
            })
            .with_get_agent_info(|name| async move {
                if name == "build" {
                    Ok(Some(TaskAgentInfo {
                        name: "build".to_string(),
                        model: None,
                        can_use_task: true,
                        steps: None,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    }))
                } else {
                    Ok(None)
                }
            })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession(|_agent, _title, _model, _disabled_tools| async move {
                Ok("task_build_123".to_string())
            })
            .with_prompt_subsession(|_session_id, _prompt| async move {
                Ok(summary("subagent output"))
            });

        TaskTool::new()
            .execute(
                serde_json::json!({
                    "description": "Investigate issue",
                    "prompt": "Please inspect runtime behavior",
                    "subagent_type": "build"
                }),
                ctx,
            )
            .await
            .expect("task should succeed");

        let requests = requests.lock().await.clone();
        let task_request = requests
            .iter()
            .find(|req| req.permission == "task")
            .expect("task permission request should exist");
        assert_eq!(task_request.scope_key.as_deref(), Some("task:agent:build"));
        assert_eq!(
            task_request.matcher_kind,
            Some(agendao_permission::PermissionMatcherKind::ScopeOnly)
        );
        assert_eq!(
            task_request.matcher_key.as_deref(),
            Some("task:agent:build")
        );
        assert_eq!(
            task_request.supported_lifetimes,
            structured_dangerous_exec_lifetimes()
        );
        assert!(task_request
            .risk_tags
            .iter()
            .any(|tag| tag == "dangerous_exec"));
    }

    #[tokio::test]
    async fn task_reuses_existing_task_id_without_creating_subsession() {
        let created = Arc::new(Mutex::new(false));
        let prompted = Arc::new(Mutex::new(Vec::<(String, SubsessionHandoffPacket)>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|name| async move {
                if name == "build" {
                    Ok(Some(TaskAgentInfo {
                        name: "build".to_string(),
                        model: None,
                        can_use_task: true,
                        steps: None,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    }))
                } else {
                    Ok(None)
                }
            })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession({
                let created = created.clone();
                move |_agent, _title, _model, _disabled_tools| {
                    let created = created.clone();
                    async move {
                        *created.lock().await = true;
                        Ok("should_not_be_used".to_string())
                    }
                }
            })
            .with_prompt_subsession({
                let prompted = prompted.clone();
                move |session_id, prompt| {
                    let prompted = prompted.clone();
                    async move {
                        prompted.lock().await.push((session_id, prompt));
                        Ok(summary("continued output"))
                    }
                }
            });

        let args = serde_json::json!({
            "description": "Continue task",
            "prompt": "Continue where you left off",
            "subagent_type": "build",
            "task_id": "task_existing_42"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();

        assert!(!(*created.lock().await));
        assert!(result
            .metadata
            .get("agentTaskId")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.starts_with('a')));
        let prompted = prompted.lock().await.clone();
        assert_eq!(prompted.len(), 1);
        assert_eq!(prompted[0].0, "task_existing_42");
        assert_eq!(
            goal_text(&prompted[0].1),
            Some("Continue where you left off")
        );
        assert!(result
            .output
            .contains("task_id: task_existing_42 (for resuming to continue this task if needed)"));
    }

    #[tokio::test]
    async fn task_recognizes_dynamic_agent_with_model_and_can_use_task() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|name| async move {
                if name == "librarian" {
                    Ok(Some(TaskAgentInfo {
                        name: "librarian".to_string(),
                        model: Some(TaskAgentModel {
                            provider_id: "openai".to_string(),
                            model_id: "gpt-4o".to_string(),
                        }),
                        can_use_task: true,
                        steps: None,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    }))
                } else {
                    Ok(None)
                }
            })
            .with_get_last_model(
                |_session_id| async move { Ok(Some("ethnopic:test-model".into())) },
            )
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_librarian_abc".to_string())
                    }
                }
            })
            .with_prompt_subsession(|_session_id, _prompt| async move {
                Ok(summary("librarian result"))
            });

        let args = serde_json::json!({
            "description": "Search docs",
            "prompt": "Find relevant documentation",
            "subagent_type": "librarian"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();

        // Agent's own model should be preferred over get_last_model fallback
        assert_eq!(
            result.metadata.get("model"),
            Some(&serde_json::json!({
                "modelID": "gpt-4o",
                "providerID": "openai"
            }))
        );

        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        // Model passed to create_subsession should be the agent's model
        assert_eq!(create_calls[0].2, Some("openai:gpt-4o".to_string()));
        // can_use_task=true means "task" should NOT be in disabled_tools
        assert_eq!(
            create_calls[0].3,
            vec!["todowrite".to_string(), "todoread".to_string()]
        );
    }

    #[tokio::test]
    async fn task_unknown_agent_falls_back_to_last_model_and_disables_task() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|_name| async move { Ok(None) })
            .with_get_last_model(
                |_session_id| async move { Ok(Some("ethnopic:test-model".into())) },
            )
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_unknown_xyz".to_string())
                    }
                }
            })
            .with_prompt_subsession(|_session_id, _prompt| async move {
                Ok(summary("fallback result"))
            });

        let args = serde_json::json!({
            "description": "Do something",
            "prompt": "Handle this",
            "subagent_type": "nonexistent_agent"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();

        // Should fall back to get_last_model
        assert_eq!(
            result.metadata.get("model"),
            Some(&serde_json::json!({
                "modelID": "test-model",
                "providerID": "ethnopic"
            }))
        );

        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].2, Some("ethnopic:test-model".to_string()));
        // Unknown agent → can_use_task defaults to false → "task" should be disabled
        assert!(create_calls[0].3.contains(&"task".to_string()));
    }

    #[tokio::test]
    async fn task_no_callback_disables_task_tool() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));

        // No with_get_agent_info — simulates paths where callback isn't injected
        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_last_model(
                |_session_id| async move { Ok(Some("ethnopic:test-model".into())) },
            )
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_nocb_xyz".to_string())
                    }
                }
            })
            .with_prompt_subsession(|_session_id, _prompt| async move {
                Ok(summary("no callback result"))
            });

        let args = serde_json::json!({
            "description": "Do something",
            "prompt": "Handle this",
            "subagent_type": "build"
        });

        let _result = TaskTool::new().execute(args, ctx).await.unwrap();

        let create_calls = create_calls.lock().await.clone();
        // Without callback, agent=None → task disabled (backward compat)
        assert!(create_calls[0].3.contains(&"task".to_string()));
    }

    #[tokio::test]
    async fn task_accepts_category_alias_and_derives_description_from_prompt() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|_name| async move { Ok(None) })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_alias_1".to_string())
                    }
                }
            })
            .with_prompt_subsession(|_session_id, _prompt| async move { Ok(summary("ok")) });

        let args = serde_json::json!({
            "prompt": "Inspect HTML structure and report key sections",
            "category": "explore"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();
        assert_eq!(result.title, "Inspect HTML structure and report key se");

        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].0, "explore");
        assert_eq!(
            create_calls[0].1,
            Some("Inspect HTML structure and report key se".to_string())
        );
    }

    #[tokio::test]
    async fn task_accepts_both_category_and_subagent_type_when_equal() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|_name| async move { Ok(None) })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_both_1".to_string())
                    }
                }
            })
            .with_prompt_subsession(|_session_id, _prompt| async move { Ok(summary("ok")) });

        let args = serde_json::json!({
            "prompt": "Inspect HTML structure and report key sections",
            "category": "explore",
            "subagent_type": "explore"
        });

        let _ = TaskTool::new().execute(args, ctx).await.unwrap();
        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].0, "explore");
    }

    #[tokio::test]
    async fn task_conflicting_category_and_subagent_type_prefers_category() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|_name| async move { Ok(None) })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_conflict_pref_1".to_string())
                    }
                }
            })
            .with_prompt_subsession(|_session_id, _prompt| async move { Ok(summary("ok")) });

        let args = serde_json::json!({
            "prompt": "Inspect HTML structure and report key sections",
            "category": "explore",
            "subagent_type": "build"
        });

        let _ = TaskTool::new().execute(args, ctx).await.unwrap();
        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].0, "explore");
    }

    #[tokio::test]
    async fn task_description_only_can_drive_low_level_dispatch() {
        let prompt_calls = Arc::new(Mutex::new(Vec::<(String, SubsessionHandoffPacket)>::new()));
        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|name| async move {
                if name == "explore" {
                    Ok(Some(TaskAgentInfo {
                        name: "explore".to_string(),
                        model: None,
                        can_use_task: true,
                        steps: None,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    }))
                } else {
                    Ok(None)
                }
            })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession(|_agent, _title, _model, _disabled_tools| async move {
                Ok("task_explore_123".to_string())
            })
            .with_prompt_subsession({
                let prompt_calls = prompt_calls.clone();
                move |session_id, prompt| {
                    let prompt_calls = prompt_calls.clone();
                    async move {
                        prompt_calls.lock().await.push((session_id, prompt));
                        Ok(summary("ok"))
                    }
                }
            });
        let args = serde_json::json!({
            "description": "Inspect the codebase and summarize the top risks.",
            "subagent_type": "explore"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();
        assert_eq!(
            result.metadata.get("sessionId"),
            Some(&serde_json::json!("task_explore_123"))
        );
        let prompt_calls = prompt_calls.lock().await.clone();
        assert_eq!(prompt_calls.len(), 1);
        assert_eq!(
            goal_text(&prompt_calls[0].1),
            Some("Inspect the codebase and summarize the top risks.")
        );
    }

    #[tokio::test]
    async fn task_empty_subagent_output_is_reported_as_completed_without_polling_hint() {
        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_get_agent_info(|name| async move {
                if name == "build" {
                    Ok(Some(TaskAgentInfo {
                        name: "build".to_string(),
                        model: None,
                        can_use_task: true,
                        steps: None,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    }))
                } else {
                    Ok(None)
                }
            })
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_create_subsession(|_agent, _title, _model, _disabled_tools| async move {
                Ok("task_build_empty".to_string())
            })
            .with_prompt_subsession(|_session_id, _prompt| async move { Ok(summary("   \n")) });

        let args = serde_json::json!({
            "description": "Investigate issue",
            "prompt": "Please inspect runtime behavior",
            "subagent_type": "build"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();

        assert!(result.output.contains("task_status: completed"));
        assert!(result.output.contains(TASK_NO_TEXT_OUTPUT_MESSAGE));
        assert_eq!(
            result.metadata.get("taskStatus"),
            Some(&serde_json::json!(TASK_STATUS_COMPLETED))
        );
        assert_eq!(
            result.metadata.get("hasTextOutput"),
            Some(&serde_json::json!(false))
        );
    }

    #[tokio::test]
    async fn task_load_skills_injects_skill_context_into_subtask_prompt() {
        let dir = tempdir().unwrap();
        let skill_path = dir.path().join(".opencode/skills/frontend-ui-ux/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(
            dir.path().join("agendao.json"),
            r#"{
  "skill_paths": {
    "legacy-opencode": ".opencode/skills"
  }
}"#,
        )
        .unwrap();
        fs::write(
            &skill_path,
            r#"---
name: frontend-ui-ux
description: frontend
---
Use clear visual hierarchy.
"#,
        )
        .unwrap();

        let prompted = Arc::new(Mutex::new(Vec::<(String, SubsessionHandoffPacket)>::new()));
        let ctx = ToolContext::new(
            "session-1".into(),
            "message-1".into(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_get_agent_info(|name| async move {
            if name == "build" {
                Ok(Some(TaskAgentInfo {
                    name: "build".to_string(),
                    model: None,
                    can_use_task: true,
                    steps: None,
                    execution: None,
                    max_tokens: None,
                    temperature: None,
                    top_p: None,
                    variant: None,
                }))
            } else {
                Ok(None)
            }
        })
        .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
        .with_create_subsession(|_agent, _title, _model, _disabled_tools| async move {
            Ok("task_build_skill".to_string())
        })
        .with_prompt_subsession({
            let prompted = prompted.clone();
            move |session_id, prompt| {
                let prompted = prompted.clone();
                async move {
                    prompted.lock().await.push((session_id, prompt));
                    Ok(summary("skill result"))
                }
            }
        });

        let args = serde_json::json!({
            "description": "Design page",
            "prompt": "Redesign dashboard layout",
            "subagent_type": "build",
            "load_skills": ["frontend-ui-ux"]
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();
        let prompted = prompted.lock().await.clone();
        assert_eq!(prompted.len(), 1);
        assert_eq!(goal_text(&prompted[0].1), Some("Redesign dashboard layout"));
        assert_eq!(
            prompted[0].1.effective_richness(),
            agendao_types::SubsessionHandoffRichness::Enriched
        );
        assert!(has_field_containing(
            &prompted[0].1,
            SubsessionHandoffFieldKind::SupportingContext,
            "<loaded_skills>"
        ));
        assert!(has_field_containing(
            &prompted[0].1,
            SubsessionHandoffFieldKind::SupportingContext,
            "frontend-ui-ux"
        ));
        assert!(has_field_containing(
            &prompted[0].1,
            SubsessionHandoffFieldKind::SupportingContext,
            "Use clear visual hierarchy."
        ));
        assert_eq!(
            result.metadata.get("loadedSkillCount"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            result.metadata.get("loadedSkillViews"),
            Some(&serde_json::json!([{
                "name": "frontend-ui-ux",
                "description": "frontend",
                "category": serde_json::Value::Null,
            }]))
        );
    }

    #[tokio::test]
    async fn task_builds_dynamic_agent_from_inline_spec() {
        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));
        let prompt_calls = Arc::new(Mutex::new(Vec::<(String, SubsessionHandoffPacket)>::new()));

        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            // No with_get_agent_info — "custom-reviewer" is not a known agent
            .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
            .with_build_agent(
                |name, _system_prompt, _model, max_steps, _allowed_tools| async move {
                    Ok(TaskAgentInfo {
                        name,
                        model: None,
                        can_use_task: true,
                        steps: max_steps,
                        execution: None,
                        max_tokens: None,
                        temperature: None,
                        top_p: None,
                        variant: None,
                    })
                },
            )
            .with_create_subsession({
                let create_calls = create_calls.clone();
                move |agent, title, model, disabled_tools| {
                    let create_calls = create_calls.clone();
                    async move {
                        create_calls
                            .lock()
                            .await
                            .push((agent, title, model, disabled_tools));
                        Ok("task_custom_reviewer_1".to_string())
                    }
                }
            })
            .with_prompt_subsession({
                let prompt_calls = prompt_calls.clone();
                move |session_id, prompt| {
                    let prompt_calls = prompt_calls.clone();
                    async move {
                        prompt_calls.lock().await.push((session_id, prompt));
                        Ok(summary("custom reviewer output"))
                    }
                }
            });

        let args = serde_json::json!({
            "description": "Custom review",
            "prompt": "Review the code for security issues",
            "subagent_type": "custom-reviewer",
            "agent_prompt": "You are a security-focused code reviewer. Identify vulnerabilities.",
            "agent_tools": ["read", "grep", "glob"]
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();

        assert_eq!(result.title, "Custom review");
        assert!(result.output.contains("custom reviewer output"));

        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].0, "custom-reviewer");

        let prompt_calls = prompt_calls.lock().await.clone();
        assert_eq!(prompt_calls.len(), 1);
        assert_eq!(
            goal_text(&prompt_calls[0].1),
            Some("Review the code for security issues")
        );
    }

    #[tokio::test]
    async fn task_without_build_agent_callback_falls_back_for_unknown_name() {
        let ctx = ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            // No with_get_agent_info, no with_build_agent
            .with_get_last_model(|_session_id| async move { Ok(Some("p:m".into())) })
            .with_create_subsession(|_agent, _title, _model, _disabled_tools| async move {
                Ok("task_fallback_1".to_string())
            })
            .with_prompt_subsession(|_session_id, _prompt| async move {
                Ok(summary("fallback output"))
            });

        let args = serde_json::json!({
            "description": "Fallback test",
            "prompt": "Do something",
            "subagent_type": "nonexistent_agent"
        });

        let result = TaskTool::new().execute(args, ctx).await.unwrap();
        // Without build_agent callback, the unknown agent name still works
        // (existing fallback behavior is preserved)
        assert!(result.output.contains("fallback output"));
    }

    #[test]
    fn task_dispatch_scope_key_is_stable() {
        assert_eq!(
            TaskDispatchKind::Agent("Build".to_string()).scope_key(),
            "task:agent:build"
        );
        assert_eq!(
            TaskDispatchKind::Category("Research".to_string()).scope_key(),
            "task:category:research"
        );
    }
}
