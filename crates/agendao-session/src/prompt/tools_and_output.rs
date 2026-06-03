use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agendao_orchestrator::session_title_request;
use agendao_provider::cache::{ToolSurfaceSourceDigest, ToolSurfaceSourceKind};
use agendao_provider::{Content, Message, Provider, Role, ToolDefinition};

use crate::{sanitize_display_text, MessageRole, PartType, Session, SessionMessage};

use super::MAX_STEPS;

// --- Structured Output ---

const STRUCTURED_OUTPUT_DESCRIPTION: &str = r#"Use this tool to return your final response in the requested structured format.

IMPORTANT:
- You MUST call this tool exactly once at the end of your response
- The input must be valid JSON matching the required schema
- Complete all necessary research and tool calls BEFORE calling this tool
- This tool provides your final answer - no further actions are taken after calling it"#;

const STRUCTURED_OUTPUT_SYSTEM_PROMPT: &str = r#"IMPORTANT: The user has requested structured output. You MUST use the StructuredOutput tool to provide your final response. Do NOT respond with plain text - you MUST call the StructuredOutput tool with your answer formatted according to the schema."#;
const LEGACY_SYSTEM_REMINDER_PREFIX: &str = "System Reminder Sent:";
const LOADED_INSTRUCTION_FILES_PREFIX: &str = "Loaded instruction files:";

pub struct StructuredOutputConfig {
    pub schema: serde_json::Value,
}

pub fn create_structured_output_tool(schema: serde_json::Value) -> ToolDefinition {
    let mut tool_schema = schema;
    if let Some(obj) = tool_schema.as_object_mut() {
        obj.remove("$schema");
    }

    ToolDefinition {
        name: "StructuredOutput".to_string(),
        description: Some(STRUCTURED_OUTPUT_DESCRIPTION.to_string()),
        parameters: tool_schema,
    }
}

pub fn structured_output_system_prompt() -> String {
    STRUCTURED_OUTPUT_SYSTEM_PROMPT.to_string()
}

pub fn extract_structured_output(parts: &[crate::MessagePart]) -> Option<serde_json::Value> {
    for part in parts {
        if let PartType::ToolCall { name, input, .. } = &part.part_type {
            if name == "StructuredOutput" {
                return Some(input.clone());
            }
        }
    }
    None
}

// --- Plan Mode ---

const PROMPT_PLAN: &str = r#"You are in PLAN mode. The user wants you to create a plan before executing.

## Your task:
1. Understand the user's request thoroughly
2. Explore the codebase to understand the current state
3. Create a detailed plan in the plan file
4. Use the plan_exit tool when done planning

## Important:
- Do NOT make any edits or run commands (except read operations)
- Only create/modify the plan file
- Ask clarifying questions if needed
- Use explore subagent to understand the codebase"#;

const BUILD_SWITCH: &str = r#"The user has approved your plan and wants you to execute it.

## Your task:
1. Execute the plan step by step
2. Make the necessary changes to the codebase
3. Test your changes
4. Verify the implementation matches the plan

## Important:
- You may now use all tools including edit, write, bash
- Follow the plan closely but adapt as needed
- Report progress to the user"#;

pub fn insert_reminders(
    messages: &[SessionMessage],
    agent_name: &str,
    was_plan: bool,
) -> Vec<SessionMessage> {
    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User));

    if let Some(idx) = last_user_idx {
        let mut messages = messages.to_vec();

        if agent_name == "plan" {
            let reminder_text = PROMPT_PLAN.to_string();
            messages[idx].parts.push(crate::MessagePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                part_type: PartType::Text {
                    text: reminder_text,
                    synthetic: None,
                    ignored: None,
                },
                created_at: chrono::Utc::now(),
                message_id: None,
            });
        }

        if was_plan && agent_name == "build" {
            let reminder_text = BUILD_SWITCH.to_string();
            messages[idx].parts.push(crate::MessagePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                part_type: PartType::Text {
                    text: reminder_text,
                    synthetic: None,
                    ignored: None,
                },
                created_at: chrono::Utc::now(),
                message_id: None,
            });
        }

        messages
    } else {
        messages.to_vec()
    }
}

pub fn was_plan_agent(messages: &[SessionMessage]) -> bool {
    messages.iter().any(|m| {
        if let Some(agent) = m.metadata.get("agent") {
            agent.as_str() == Some("plan")
        } else {
            false
        }
    })
}

// --- Tool Resolution ---

pub struct ResolvedTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub struct ResolvedToolSurface {
    pub tools: Vec<ToolDefinition>,
    pub source_digests: Vec<ToolSurfaceSourceDigest>,
}

pub fn prioritize_tool_definitions(tools: &mut [ToolDefinition]) {
    tools.sort_by(|a, b| agendao_provider::cache::stable_tool_name_cmp(&a.name, &b.name));
}

pub fn merge_tool_definitions(
    base: Vec<ToolDefinition>,
    extra: Vec<ToolDefinition>,
) -> Vec<ToolDefinition> {
    let mut merged_base: HashMap<String, ToolDefinition> = HashMap::new();
    for tool in base {
        merged_base.insert(tool.name.clone(), tool);
    }
    let base_names = merged_base.keys().cloned().collect::<HashSet<_>>();

    let mut merged_extra: HashMap<String, ToolDefinition> = HashMap::new();
    for tool in extra {
        if !base_names.contains(&tool.name) {
            merged_extra.insert(tool.name.clone(), tool);
        }
    }

    let mut base_tools: Vec<ToolDefinition> = merged_base.into_values().collect();
    let mut extra_tools: Vec<ToolDefinition> = merged_extra.into_values().collect();
    prioritize_tool_definitions(&mut base_tools);
    prioritize_tool_definitions(&mut extra_tools);

    let mut tools = base_tools;
    tools.extend(extra_tools);
    tools
}

pub async fn resolve_tools_with_mcp(
    tool_registry: &agendao_tool::ToolRegistry,
    mcp_tools: Vec<ToolDefinition>,
) -> Vec<ToolDefinition> {
    resolve_tool_surface_with_mcp(tool_registry, mcp_tools)
        .await
        .tools
}

pub async fn resolve_tool_surface(
    tool_registry: &agendao_tool::ToolRegistry,
) -> ResolvedToolSurface {
    resolve_tool_surface_with_mcp(tool_registry, Vec::new()).await
}

pub async fn resolve_tool_surface_with_mcp(
    tool_registry: &agendao_tool::ToolRegistry,
    mcp_tools: Vec<ToolDefinition>,
) -> ResolvedToolSurface {
    let schemas = tool_registry
        .list_schemas()
        .await
        .into_iter()
        .filter(|schema| schema.name != "invalid")
        .collect::<Vec<_>>();
    let mut built_in = Vec::new();
    let mut mcp = Vec::new();
    let mut plugin = Vec::new();
    let mut dynamic = Vec::new();

    for schema in schemas {
        let tool = ToolDefinition {
            name: schema.name,
            description: Some(schema.description),
            parameters: schema.parameters,
        };
        match schema.source_kind {
            agendao_tool::ToolSchemaSourceKind::BuiltIn => built_in.push(tool),
            agendao_tool::ToolSchemaSourceKind::Mcp => mcp.push(tool),
            agendao_tool::ToolSchemaSourceKind::Plugin => plugin.push(tool),
            agendao_tool::ToolSchemaSourceKind::Dynamic => dynamic.push(tool),
        }
    }
    mcp.extend(mcp_tools);

    let mut source_digests = Vec::new();
    push_tool_source_digest(
        &mut source_digests,
        ToolSurfaceSourceKind::BuiltIn,
        &built_in,
    );
    push_tool_source_digest(&mut source_digests, ToolSurfaceSourceKind::Mcp, &mcp);
    push_tool_source_digest(&mut source_digests, ToolSurfaceSourceKind::Plugin, &plugin);
    push_tool_source_digest(
        &mut source_digests,
        ToolSurfaceSourceKind::Dynamic,
        &dynamic,
    );

    let tools = merge_tool_groups(vec![built_in, mcp, plugin, dynamic]);
    ResolvedToolSurface {
        tools,
        source_digests,
    }
}

fn push_tool_source_digest(
    target: &mut Vec<ToolSurfaceSourceDigest>,
    source: ToolSurfaceSourceKind,
    tools: &[ToolDefinition],
) {
    if tools.is_empty() {
        return;
    }
    target.push(ToolSurfaceSourceDigest {
        source,
        tool_count: tools.len(),
        tools_hash: agendao_provider::cache::tool_surface_fingerprint(tools),
    });
}

fn merge_tool_groups(groups: Vec<Vec<ToolDefinition>>) -> Vec<ToolDefinition> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for mut group in groups {
        prioritize_tool_definitions(&mut group);
        for tool in group {
            if seen.insert(tool.name.clone()) {
                merged.push(tool);
            }
        }
    }
    merged
}

pub async fn resolve_tools_with_mcp_registry(
    tool_registry: &agendao_tool::ToolRegistry,
    mcp_registry: Option<&agendao_mcp::McpToolRegistry>,
) -> Vec<ToolDefinition> {
    let dynamic_mcp_tools = if let Some(registry) = mcp_registry {
        registry
            .list()
            .await
            .into_iter()
            .map(|tool| ToolDefinition {
                name: tool.full_name,
                description: tool.description,
                parameters: tool.input_schema,
            })
            .collect()
    } else {
        Vec::new()
    };

    resolve_tool_surface_with_mcp(tool_registry, dynamic_mcp_tools)
        .await
        .tools
}

pub async fn resolve_tools(tool_registry: &agendao_tool::ToolRegistry) -> Vec<ToolDefinition> {
    resolve_tool_surface(tool_registry).await.tools
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use async_trait::async_trait;

    struct SourceKindTool {
        id: &'static str,
        source_kind: agendao_tool::ToolSchemaSourceKind,
    }

    #[async_trait]
    impl agendao_tool::Tool for SourceKindTool {
        fn id(&self) -> &str {
            self.id
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        fn source_kind(&self) -> agendao_tool::ToolSchemaSourceKind {
            self.source_kind
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: agendao_tool::ToolContext,
        ) -> Result<agendao_tool::ToolResult, agendao_tool::ToolError> {
            Ok(agendao_tool::ToolResult::simple("ok", "ok"))
        }
    }

    #[test]
    fn prioritize_tool_definitions_prefers_task_flow_over_task() {
        let mut tools = vec![
            ToolDefinition {
                name: "websearch".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "task".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "task_flow".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];

        prioritize_tool_definitions(&mut tools);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, vec!["task_flow", "task", "websearch"]);
    }

    #[test]
    fn prioritize_tool_definitions_prefers_skill_discovery_before_skill_loading_tools() {
        let mut tools = vec![
            ToolDefinition {
                name: "skill".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "websearch".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "skill_manage".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "skill_view".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "skills_list".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "skills_categories".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];

        prioritize_tool_definitions(&mut tools);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "skills_categories",
                "skills_list",
                "skill_view",
                "skill",
                "skill_manage",
                "websearch"
            ]
        );
    }

    #[test]
    fn prioritize_tool_definitions_pushes_bash_after_structured_tools() {
        let mut tools = vec![
            ToolDefinition {
                name: "bash".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "read".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "skill_manage".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];

        prioritize_tool_definitions(&mut tools);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, vec!["skill_manage", "read", "bash"]);
    }

    #[test]
    fn merge_tool_definitions_keeps_base_tool_on_name_conflict() {
        let base = vec![ToolDefinition {
            name: "read".to_string(),
            description: Some("built-in read".to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"}
                }
            }),
        }];
        let extra = vec![ToolDefinition {
            name: "read".to_string(),
            description: Some("external read".to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        }];

        let merged = merge_tool_definitions(base, extra);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "read");
        assert_eq!(merged[0].description.as_deref(), Some("built-in read"));
        assert!(merged[0].parameters["properties"]
            .get("file_path")
            .is_some());
    }

    #[test]
    fn merge_tool_definitions_is_stable_across_extra_tool_order() {
        let base = vec![ToolDefinition {
            name: "task".to_string(),
            description: None,
            parameters: serde_json::json!({}),
        }];
        let extra_a = vec![
            ToolDefinition {
                name: "github_search".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "repo_scan".to_string(),
                description: None,
                parameters: serde_json::json!({}),
            },
        ];
        let mut extra_b = extra_a.clone();
        extra_b.reverse();

        let names_a = merge_tool_definitions(base.clone(), extra_a)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let names_b = merge_tool_definitions(base, extra_b)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert_eq!(names_a, names_b);
        assert_eq!(names_a, vec!["task", "github_search", "repo_scan"]);
    }

    #[test]
    fn merge_tool_definitions_keeps_base_group_before_extra_group() {
        let base = vec![ToolDefinition {
            name: "z_builtin".to_string(),
            description: None,
            parameters: serde_json::json!({}),
        }];
        let extra = vec![ToolDefinition {
            name: "a_mcp".to_string(),
            description: None,
            parameters: serde_json::json!({}),
        }];

        let names = merge_tool_definitions(base, extra)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["z_builtin", "a_mcp"]);
    }

    #[tokio::test]
    async fn resolve_tool_surface_records_non_wire_source_digests() {
        let registry = agendao_tool::ToolRegistry::new();
        registry
            .register(SourceKindTool {
                id: "read",
                source_kind: agendao_tool::ToolSchemaSourceKind::BuiltIn,
            })
            .await;
        registry
            .register(SourceKindTool {
                id: "plugin_lookup",
                source_kind: agendao_tool::ToolSchemaSourceKind::Plugin,
            })
            .await;
        registry
            .register(SourceKindTool {
                id: "dynamic_plan",
                source_kind: agendao_tool::ToolSchemaSourceKind::Dynamic,
            })
            .await;

        let surface = resolve_tool_surface(&registry).await;
        let sources = surface
            .source_digests
            .iter()
            .map(|digest| digest.source)
            .collect::<Vec<_>>();
        let names = surface
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert!(sources.contains(&ToolSurfaceSourceKind::BuiltIn));
        assert!(sources.contains(&ToolSurfaceSourceKind::Plugin));
        assert!(sources.contains(&ToolSurfaceSourceKind::Dynamic));
        assert_eq!(names, vec!["read", "plugin_lookup", "dynamic_plan"]);
    }

    #[tokio::test]
    async fn resolve_tool_surface_hides_invalid_tool_from_model_surface() {
        let registry = agendao_tool::create_default_registry().await;
        let surface = resolve_tool_surface(&registry).await;
        let names = surface
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(
            !names.contains(&"invalid"),
            "invalid should remain available for fallback execution but not be exposed on the model tool surface"
        );
    }
}

// --- Misc ---

pub fn max_steps_for_agent(agent_steps: Option<u32>) -> u32 {
    agent_steps.unwrap_or(MAX_STEPS)
}

fn is_system_reminder_open_tag(line: &str) -> bool {
    line.starts_with("<system-reminder") || line.starts_with("<system_reminder")
}

fn is_system_reminder_close_tag(line: &str) -> bool {
    line.starts_with("</system-reminder") || line.starts_with("</system_reminder")
}

pub fn sanitize_session_title_source(text: &str) -> String {
    let mut lines = Vec::new();
    let mut in_system_reminder = false;
    let mut previous_blank = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        if is_system_reminder_open_tag(trimmed) {
            in_system_reminder = true;
            if trimmed.contains("</system-reminder>") || trimmed.contains("</system_reminder>") {
                in_system_reminder = false;
            }
            continue;
        }

        if in_system_reminder {
            if is_system_reminder_close_tag(trimmed) {
                in_system_reminder = false;
            }
            continue;
        }

        if is_system_reminder_close_tag(trimmed)
            || trimmed.starts_with(LEGACY_SYSTEM_REMINDER_PREFIX)
            || trimmed.starts_with(LOADED_INSTRUCTION_FILES_PREFIX)
            || trimmed.starts_with("Instructions from:")
        {
            continue;
        }

        if trimmed.is_empty() {
            if previous_blank {
                continue;
            }
            previous_blank = true;
            lines.push(String::new());
            continue;
        }

        previous_blank = false;
        lines.push(raw_line.to_string());
    }

    sanitize_display_text(&lines.join("\n")).trim().to_string()
}

pub fn generate_session_title(first_user_message: &str) -> String {
    let normalized = sanitize_session_title_source(first_user_message);
    let first_line = normalized.lines().next().unwrap_or("").trim();

    if first_line.chars().count() > 100 {
        format!("{}...", first_line.chars().take(97).collect::<String>())
    } else if first_line.is_empty() {
        "New Session".to_string()
    } else {
        first_line.to_string()
    }
}

fn trim_title_source(text: &str, max_chars: usize) -> String {
    let normalized = sanitize_session_title_source(text);
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        normalized.chars().take(max_chars).collect::<String>()
    }
}

pub fn compose_session_title_source(session: &Session) -> Option<(String, String)> {
    let first_user = session
        .messages
        .iter()
        .find(|message| matches!(message.role, MessageRole::User))
        .map(SessionMessage::get_text)
        .map(|text| sanitize_session_title_source(&text))
        .filter(|text| !text.is_empty())?;

    let fallback = generate_session_title(&first_user);
    let mut sections = vec![format!(
        "User request:\n{}",
        trim_title_source(&first_user, 400)
    )];

    if let Some(assistant_text) = session
        .messages
        .iter()
        .rev()
        .filter(|message| matches!(message.role, MessageRole::Assistant))
        .map(SessionMessage::get_text)
        .map(|text| trim_title_source(&text, 600))
        .find(|text| !text.trim().is_empty())
    {
        sections.push(format!("Assistant outcome:\n{}", assistant_text));
    }

    Some((sections.join("\n\n"), fallback))
}

/// Generate a refined session title from the session's first-turn context.
/// Uses the first user request and, when available, the latest assistant
/// outcome already persisted in the session.
pub async fn generate_session_title_for_session(
    session: &Session,
    provider: Arc<dyn Provider>,
    model_id: &str,
) -> String {
    let Some((title_source, fallback)) = compose_session_title_source(session) else {
        return "New Session".to_string();
    };

    let request = session_title_request(model_id).to_chat_request_with_system(
        vec![Message {
            role: Role::User,
            content: Content::Text(format!(
                "Generate a short session title (under 80 chars) for this conversation.\n\
                 Base it on the actual task and outcome, not the user's raw wording.\n\
                 Do not mention system reminders, instruction files, or metadata wrappers.\n\
                 Reply with ONLY the title, no quotes or explanation.\n\n{}",
                title_source
            )),
            cache_control: None,
            provider_options: None,
        }],
        vec![],
        None,
        Some(
            "You generate concise conversation titles. Prefer compact task-focused summaries. Never mention system reminders or instruction-file wrappers. Reply with only the title."
                .to_string(),
        ),
    );

    match provider.chat(request).await {
        Ok(response) => {
            let text = response
                .choices
                .first()
                .map(|c| match &c.message.content {
                    Content::Text(t) => t.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                })
                .unwrap_or_default();

            let cleaned = text
                .replace(['"', '\''], "")
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("<think>"))
                .unwrap_or("")
                .to_string();

            if cleaned.is_empty() {
                fallback
            } else if cleaned.chars().count() > 100 {
                format!("{}...", cleaned.chars().take(97).collect::<String>())
            } else {
                cleaned
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate title via LLM, using fallback");
            fallback
        }
    }
}

/// Generate a session title using an LLM (matching TS `ensureTitle`).
/// Falls back to `generate_session_title` on any failure.
pub async fn generate_session_title_llm(
    first_user_message: &str,
    provider: Arc<dyn Provider>,
    model_id: &str,
) -> String {
    let normalized_first_user_message = sanitize_session_title_source(first_user_message);
    let fallback = generate_session_title(&normalized_first_user_message);

    let request = session_title_request(model_id).to_chat_request_with_system(
        vec![Message {
            role: Role::User,
            content: Content::Text(format!(
                "Generate a short title (under 80 chars) for this conversation. \
                     Do not mention system reminders, instruction files, or metadata wrappers. \
                     Reply with ONLY the title, no quotes or explanation.\n\n{}",
                normalized_first_user_message
            )),
            cache_control: None,
            provider_options: None,
        }],
        vec![],
        None,
        Some(
            "You generate concise conversation titles. Never mention system reminders or instruction-file wrappers. Reply with only the title."
                .to_string(),
        ),
    );

    match provider.chat(request).await {
        Ok(response) => {
            // Extract text from the first choice
            let text = response
                .choices
                .first()
                .map(|c| match &c.message.content {
                    Content::Text(t) => t.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                })
                .unwrap_or_default();

            // Clean up: remove thinking tags, take first non-empty line
            let cleaned = text
                .replace(['"', '\''], "")
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("<think>"))
                .unwrap_or("")
                .to_string();

            if cleaned.is_empty() {
                fallback
            } else if cleaned.chars().count() > 100 {
                format!("{}...", cleaned.chars().take(97).collect::<String>())
            } else {
                cleaned
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate title via LLM, using fallback");
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use agendao_provider::{
        ChatRequest, ChatResponse, Choice, Message as ProviderMessage, ModelInfo, ProviderError,
        StreamResult,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct CaptureProvider {
        title: String,
        last_prompt: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl Provider for CaptureProvider {
        fn id(&self) -> &str {
            "capture"
        }

        fn name(&self) -> &str {
            "Capture"
        }

        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        fn get_model(&self, _id: &str) -> Option<&ModelInfo> {
            None
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let text = request
                .messages
                .first()
                .map(|message| match &message.content {
                    Content::Text(text) => text.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|part| part.text.clone())
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .unwrap_or_default();
            *self.last_prompt.lock().expect("capture prompt") = Some(text);
            Ok(ChatResponse {
                id: "capture-response".to_string(),
                model: "capture-model".to_string(),
                choices: vec![Choice {
                    index: 0,
                    message: ProviderMessage {
                        role: Role::Assistant,
                        content: Content::Text(self.title.clone()),
                        cache_control: None,
                        provider_options: None,
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: None,
            })
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::iter(Vec::<
                Result<agendao_provider::StreamEvent, ProviderError>,
            >::new())))
        }
    }

    #[test]
    fn compose_session_title_source_includes_assistant_outcome() {
        let mut session = Session::new("project", ".");
        session.add_user_message("根据 ./t.html 文件，设计一个科技感更加浓重的网页");
        session
            .add_assistant_message()
            .add_text("已完成首页重构，强化了深色科技风、发光边框和分层卡片布局。");

        let (source, fallback) =
            compose_session_title_source(&session).expect("title source should exist");
        assert!(source.contains("User request:"));
        assert!(source.contains("Assistant outcome:"));
        assert!(source.contains("已完成首页重构"));
        assert_eq!(fallback, "根据 ./t.html 文件，设计一个科技感更加浓重的网页");
    }

    #[tokio::test]
    async fn generate_session_title_for_session_uses_assistant_context() {
        let mut session = Session::new("project", ".");
        session.add_user_message("Fix the scheduler session title flow after first reply");
        session
            .add_assistant_message()
            .add_text("Implemented refined title regeneration based on the first completed turn.");

        let last_prompt = Arc::new(Mutex::new(None));
        let provider = Arc::new(CaptureProvider {
            title: "Refine Session Titles After First Reply".to_string(),
            last_prompt: last_prompt.clone(),
        });

        let title = generate_session_title_for_session(&session, provider, "mock-model").await;
        assert_eq!(title, "Refine Session Titles After First Reply");

        let captured = last_prompt
            .lock()
            .expect("capture prompt")
            .clone()
            .unwrap_or_default();
        assert!(captured.contains("User request:"));
        assert!(captured.contains("Assistant outcome:"));
        assert!(captured.contains("Implemented refined title regeneration"));
    }

    #[test]
    fn sanitize_session_title_source_strips_system_reminder_wrappers() {
        let cleaned = sanitize_session_title_source(
            "帮我重构 TUI\n\n<system-reminder>\nInstructions from: /tmp/project/AGENTS.md\nBe strict.\n</system-reminder>\n\nLoaded instruction files: /tmp/project/AGENTS.md",
        );

        assert_eq!(cleaned, "帮我重构 TUI");
    }

    #[test]
    fn generate_session_title_ignores_system_reminder_text() {
        let title = generate_session_title(
            "Fix the reratui migration flow\n<system-reminder>\nInstructions from: /tmp/project/AGENTS.md\nUse latest reratui.\n</system-reminder>",
        );

        assert_eq!(title, "Fix the reratui migration flow");
    }
}
