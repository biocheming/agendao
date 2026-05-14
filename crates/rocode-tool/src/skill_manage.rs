use async_trait::async_trait;
use rocode_skill::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, SkillGovernedWriteResult, SkillWriteAction, WriteSkillFileRequest,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::path::Path;

use crate::skill_support::{governance_authority_for, map_skill_error};
use crate::{
    append_tool_repair_event_map, merge_tool_repair_telemetry, tool_repair_event, Metadata,
    PermissionRequest, Tool, ToolContext, ToolError, ToolResult,
};

pub struct SkillManageTool;

struct NormalizedSkillManageArgs {
    args: Value,
    repair_metadata: Metadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SkillManageAction {
    Create,
    Patch,
    Edit,
    WriteFile,
    RemoveFile,
    Delete,
}

#[derive(Debug, Deserialize)]
struct SkillManageInput {
    action: SkillManageAction,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    new_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_json_object_or_string"
    )]
    methodology: Option<rocode_skill::SkillMethodologyTemplate>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_json_object_or_string"
    )]
    frontmatter: Option<rocode_skill::SkillFrontmatterPatch>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    directory_name: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
}

fn deserialize_optional_json_object_or_string<'de, D, T>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }

            let parsed = rocode_util::json::try_parse_json_object_robust(trimmed)
                .ok_or_else(|| serde::de::Error::custom("expected JSON object string"))?;
            serde_json::from_value(parsed)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
        other => serde_json::from_value(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[async_trait]
impl Tool for SkillManageTool {
    fn id(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Create, patch, edit, delete, or manage supporting files for workspace-local skills under .rocode/skills. Create when a complex task succeeded (5+ tool calls), you overcame errors, a user-corrected approach worked, you discovered a non-trivial workflow, or the user asks you to remember a procedure. For create, the most reliable minimal shape is {\"action\":\"create\",\"name\":\"skill-name\",\"description\":\"what it does\",\"methodology\":{...}} or {\"action\":\"create\",\"name\":\"skill-name\",\"description\":\"what it does\",\"body\":\"# Skill...\"}. Prefer the structured `methodology` shape when creating or patching a skill so the result includes trigger conditions, core steps, success criteria, validation, and boundaries. `methodology` and `frontmatter` may be provided either as nested objects or as JSON strings containing objects. Common methodology aliases are normalized automatically: `trigger_conditions` -> `when_to_use`, `boundaries` -> `pitfalls`, `steps` -> `core_steps`, and per-step `name`/`description` -> `title`/`action`. For `frontmatter`, keep human-readable prerequisites in `methodology.prerequisites`; use structured metadata such as `required_commands` or `prerequisites: { env_vars, commands }` only when you specifically need setup metadata. When creating or patching a methodology skill with core steps, review the current session's tool call history and fill each step's optional `experienced_tools` field with the tool names actually used in that step. For commands invoked through bash, use the command name you actually ran, such as `docker` or `cargo`, instead of `bash`; leave `experienced_tools` empty if you are unsure. Patch when instructions are stale or wrong, a skill fails on a specific OS or environment, steps or pitfalls are missing, or you used a skill and found gaps not covered by it. After difficult or iterative tasks, offer to save the approach as a skill. Skip simple one-offs. Confirm with the user before creating or deleting skills."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "patch", "edit", "write_file", "remove_file", "delete"],
                    "description": "Mutation to perform. Pick exactly one of: create, patch, edit, write_file, remove_file, delete."
                },
                "name": {
                    "type": "string",
                    "description": "For create: new skill name. For patch/edit/write_file/remove_file/delete: existing skill name."
                },
                "new_name": {
                    "type": "string",
                    "description": "Optional renamed skill name for patch."
                },
                "description": {
                    "type": "string",
                    "description": "Short one-line skill description for create or patch."
                },
                "body": {
                    "type": "string",
                    "description": "Full SKILL.md markdown body for create or patch. Use this OR `methodology`, not both."
                },
                "methodology": {
                    "description": "Structured methodology template for create or patch. Use this OR `body`, not both. May be either a nested object or a JSON string containing that object. Recommended minimal shape: {\"when_to_use\":[...],\"core_steps\":[{\"title\":\"...\",\"action\":\"...\",\"outcome\":\"...\"}],\"success_criteria\":[...],\"validation\":[...],\"pitfalls\":[...]}. Common aliases are accepted and normalized automatically: `trigger_conditions`, `boundaries`, `steps`, and per-step `name` / `description`.",
                    "oneOf": [
                        { "type": "object" },
                        { "type": "string" }
                    ]
                },
                "frontmatter": {
                    "description": "Optional structured YAML frontmatter patch for rich metadata such as version, author, license, tags, required_commands, metadata blocks, or structured setup prerequisites. May be either a nested object or a JSON string containing that object. Put human-readable prerequisite bullet lists in `methodology.prerequisites`; reserve `frontmatter.prerequisites` for the structured shape `{ \"env_vars\": [...], \"commands\": [...] }`.",
                    "oneOf": [
                        { "type": "object" },
                        { "type": "string" }
                    ]
                },
                "content": {
                    "type": "string",
                    "description": "Full SKILL.md content for edit, or file content for write_file."
                },
                "category": {
                    "type": "string",
                    "description": "Optional workspace-local category path like analysis/review for create."
                },
                "directory_name": {
                    "type": "string",
                    "description": "Optional leaf directory name to use under .rocode/skills for create. If omitted, ROCode derives it from the name."
                },
                "file_path": {
                    "type": "string",
                    "description": "Supporting file path relative to the skill directory."
                }
            },
            "required": ["action"],
            "allOf": [
                {
                    "if": { "properties": { "action": { "const": "create" } } },
                    "then": { "required": ["action", "name", "description"] }
                },
                {
                    "if": { "properties": { "action": { "const": "patch" } } },
                    "then": { "required": ["action", "name"] }
                },
                {
                    "if": { "properties": { "action": { "const": "edit" } } },
                    "then": { "required": ["action", "name", "content"] }
                },
                {
                    "if": { "properties": { "action": { "const": "write_file" } } },
                    "then": { "required": ["action", "name", "file_path", "content"] }
                },
                {
                    "if": { "properties": { "action": { "const": "remove_file" } } },
                    "then": { "required": ["action", "name", "file_path"] }
                },
                {
                    "if": { "properties": { "action": { "const": "delete" } } },
                    "then": { "required": ["action", "name"] }
                }
            ],
            "examples": [
                {
                    "action": "create",
                    "name": "code-audit-methodology",
                    "description": "Reusable code audit workflow",
                    "methodology": {
                        "when_to_use": ["Use when a project needs a repeatable audit workflow."],
                        "core_steps": [
                            {
                                "title": "Survey",
                                "action": "Read the project structure and identify risk surfaces.",
                                "outcome": "The audit scope is clear."
                            }
                        ],
                        "success_criteria": ["The workflow is reusable across projects."],
                        "validation": ["Apply it to a second repo and confirm the steps still fit."]
                    }
                },
                {
                    "action": "patch",
                    "name": "code-audit-methodology",
                    "description": "Update the workflow with missing validation",
                    "methodology": "{\"when_to_use\":[\"Use when the old skill is incomplete.\"],\"core_steps\":[{\"title\":\"Update\",\"action\":\"Add the missing validation steps.\",\"outcome\":\"The skill is more reliable.\"}],\"success_criteria\":[\"The new steps are present.\"],\"validation\":[\"Reload the skill and inspect the rendered sections.\"]}"
                }
            ]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let normalized = normalize_skill_manage_args(args)?;
        let input: SkillManageInput = serde_json::from_value(normalized.args)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let authority =
            governance_authority_for(Path::new(&ctx.directory), ctx.config_store.clone());

        let permission = build_permission_request(&input)?;
        ctx.ask_permission(permission).await?;

        let result = match input.action {
            SkillManageAction::Create => authority
                .create_skill(
                    CreateSkillRequest {
                        name: required_string(input.name.clone(), "name")?,
                        description: required_string(input.description, "description")?,
                        body: resolve_skill_body(
                            required_string(input.name, "name")?.as_str(),
                            input.body,
                            input.methodology,
                            "create",
                        )?,
                        frontmatter: input.frontmatter.clone(),
                        category: optional_trimmed(input.category),
                        directory_name: optional_trimmed(input.directory_name),
                    },
                    "tool:skill_manage",
                )
                .map_err(map_skill_error)?,
            SkillManageAction::Patch => authority
                .patch_skill(
                    PatchSkillRequest {
                        name: required_string(input.name.clone(), "name")?,
                        new_name: optional_trimmed(input.new_name.clone()),
                        description: optional_trimmed(input.description),
                        body: resolve_optional_skill_body(
                            optional_trimmed(input.new_name)
                                .or_else(|| optional_trimmed(input.name))
                                .unwrap_or_else(|| "patched-skill".to_string())
                                .as_str(),
                            input.body,
                            input.methodology,
                            "patch",
                        )?,
                        frontmatter: input.frontmatter.clone(),
                    },
                    "tool:skill_manage",
                )
                .map_err(map_skill_error)?,
            SkillManageAction::Edit => authority
                .edit_skill(
                    EditSkillRequest {
                        name: required_string(input.name, "name")?,
                        content: required_string(input.content, "content")?,
                    },
                    "tool:skill_manage",
                )
                .map_err(map_skill_error)?,
            SkillManageAction::WriteFile => authority
                .write_supporting_file(
                    WriteSkillFileRequest {
                        name: required_string(input.name, "name")?,
                        file_path: required_string(input.file_path, "file_path")?,
                        content: required_string(input.content, "content")?,
                    },
                    "tool:skill_manage",
                )
                .map_err(map_skill_error)?,
            SkillManageAction::RemoveFile => authority
                .remove_supporting_file(
                    RemoveSkillFileRequest {
                        name: required_string(input.name, "name")?,
                        file_path: required_string(input.file_path, "file_path")?,
                    },
                    "tool:skill_manage",
                )
                .map_err(map_skill_error)?,
            SkillManageAction::Delete => authority
                .delete_skill(
                    DeleteSkillRequest {
                        name: required_string(input.name, "name")?,
                    },
                    "tool:skill_manage",
                )
                .map_err(map_skill_error)?,
        };

        let changed_path = result.result.location.to_string_lossy().to_string();
        ctx.do_publish_bus(
            "skill.updated",
            serde_json::json!({
                "action": write_action_label(&result.result.action),
                "skill": result.result.skill_name,
                "path": changed_path,
                "supportingFile": result.result.supporting_file,
                "guardReport": result.guard_report,
            }),
        )
        .await;

        let output = format_output(&result);
        let mut metadata = format_metadata(&result);
        merge_tool_repair_telemetry(&mut metadata, &normalized.repair_metadata);
        Ok(ToolResult {
            title: format!("Skill {}", write_action_label(&result.result.action)),
            output,
            metadata,
            truncated: false,
        })
    }
}

impl Default for SkillManageTool {
    fn default() -> Self {
        Self
    }
}

fn build_permission_request(input: &SkillManageInput) -> Result<PermissionRequest, ToolError> {
    let action = match input.action {
        SkillManageAction::Create => "create",
        SkillManageAction::Patch => "patch",
        SkillManageAction::Edit => "edit",
        SkillManageAction::WriteFile => "write_file",
        SkillManageAction::RemoveFile => "remove_file",
        SkillManageAction::Delete => "delete",
    };

    match input.action {
        SkillManageAction::Create => {
            required_string(input.name.clone(), "name")?;
            required_string(input.description.clone(), "description")?;
            require_skill_body_or_methodology(&input.body, &input.methodology, "create")?;
        }
        SkillManageAction::Patch => {
            required_string(input.name.clone(), "name")?;
            ensure_body_and_methodology_not_both_set(&input.body, &input.methodology, "patch")?;
        }
        SkillManageAction::Edit => {
            required_string(input.name.clone(), "name")?;
            required_string(input.content.clone(), "content")?;
        }
        SkillManageAction::WriteFile => {
            required_string(input.name.clone(), "name")?;
            required_string(input.file_path.clone(), "file_path")?;
            required_string(input.content.clone(), "content")?;
        }
        SkillManageAction::RemoveFile => {
            required_string(input.name.clone(), "name")?;
            required_string(input.file_path.clone(), "file_path")?;
        }
        SkillManageAction::Delete => {
            required_string(input.name.clone(), "name")?;
        }
    }

    let mut request = PermissionRequest::new("skill_manage")
        .with_pattern(
            optional_trimmed(input.name.clone()).unwrap_or_else(|| "new-skill".to_string()),
        )
        .with_metadata("action", serde_json::json!(action));

    if let Some(name) = optional_trimmed(input.name.clone()) {
        request = request.with_metadata("name", serde_json::json!(name));
    }
    if let Some(new_name) = optional_trimmed(input.new_name.clone()) {
        request = request.with_metadata("new_name", serde_json::json!(new_name));
    }
    if let Some(category) = optional_trimmed(input.category.clone()) {
        request = request.with_metadata("category", serde_json::json!(category));
    }
    if let Some(file_path) = optional_trimmed(input.file_path.clone()) {
        request = request
            .with_pattern(file_path.clone())
            .with_metadata("file_path", serde_json::json!(file_path));
    }
    if let Some(description) = optional_trimmed(input.description.clone()) {
        request = request.with_metadata("description", serde_json::json!(description));
    }

    Ok(request)
}

fn required_string(value: Option<String>, field: &str) -> Result<String, ToolError> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments(format!("{field} is required")))
}

fn require_skill_body_or_methodology(
    body: &Option<String>,
    methodology: &Option<rocode_skill::SkillMethodologyTemplate>,
    action: &str,
) -> Result<(), ToolError> {
    ensure_body_and_methodology_not_both_set(body, methodology, action)?;
    let has_body = body.as_ref().is_some_and(|value| !value.trim().is_empty());
    if has_body || methodology.is_some() {
        return Ok(());
    }
    Err(ToolError::InvalidArguments(format!(
        "{action} requires either `body` or `methodology`. Minimal create shape: {{\"action\":\"create\",\"name\":\"skill-name\",\"description\":\"what it does\",\"methodology\":{{\"when_to_use\":[\"...\"],\"core_steps\":[{{\"title\":\"...\",\"action\":\"...\"}}],\"success_criteria\":[\"...\"],\"validation\":[\"...\"],\"pitfalls\":[\"...\"]}}}}"
    )))
}

fn ensure_body_and_methodology_not_both_set(
    body: &Option<String>,
    methodology: &Option<rocode_skill::SkillMethodologyTemplate>,
    action: &str,
) -> Result<(), ToolError> {
    if body.as_ref().is_some_and(|value| !value.trim().is_empty()) && methodology.is_some() {
        return Err(ToolError::InvalidArguments(format!(
            "{action} accepts either `body` or `methodology`, not both"
        )));
    }
    Ok(())
}

fn resolve_skill_body(
    skill_name: &str,
    body: Option<String>,
    methodology: Option<rocode_skill::SkillMethodologyTemplate>,
    action: &str,
) -> Result<String, ToolError> {
    ensure_body_and_methodology_not_both_set(&body, &methodology, action)?;
    if let Some(methodology) = methodology {
        return rocode_skill::render_methodology_skill_body(skill_name, &methodology)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()));
    }
    required_string(body, "body")
}

fn resolve_optional_skill_body(
    skill_name: &str,
    body: Option<String>,
    methodology: Option<rocode_skill::SkillMethodologyTemplate>,
    action: &str,
) -> Result<Option<String>, ToolError> {
    ensure_body_and_methodology_not_both_set(&body, &methodology, action)?;
    if let Some(methodology) = methodology {
        return rocode_skill::render_methodology_skill_body(skill_name, &methodology)
            .map(Some)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()));
    }
    Ok(optional_trimmed_multiline(body))
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn optional_trimmed_multiline(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.replace("\r\n", "\n"))
        .filter(|value| !value.trim().is_empty())
}

fn normalize_skill_manage_args(args: Value) -> Result<NormalizedSkillManageArgs, ToolError> {
    let mut repair_metadata = Metadata::new();
    let args = match args {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Value::String(raw)
            } else if let Some(parsed) = rocode_util::json::try_parse_json_object_robust(trimmed) {
                let mut event = tool_repair_event(
                    rocode_types::RepairKind::JsonStringObjectParse.as_str(),
                    "tool",
                    "skill_manage",
                );
                event.insert("field".to_string(), serde_json::json!("$root"));
                // P1.1: capture the raw string and the parsed object.
                event.insert("raw_shape".to_string(), Value::String(raw.clone()));
                event.insert("normalized_shape".to_string(), parsed.clone());
                append_tool_repair_event_map(&mut repair_metadata, event);
                parsed
            } else {
                Value::String(raw)
            }
        }
        other => other,
    };
    let mut root = match args {
        Value::Object(map) => map,
        other => {
            return Ok(NormalizedSkillManageArgs {
                args: other,
                repair_metadata,
            });
        }
    };

    for wrapper_key in ["input", "payload", "arguments"] {
        if root.get("action").is_some() {
            break;
        }
        let Some(wrapper_value) = root.remove(wrapper_key) else {
            continue;
        };
        let Some(wrapper_map) =
            take_nested_root_object(wrapper_key, wrapper_value, &mut repair_metadata)?
        else {
            continue;
        };
        if wrapper_map.get("action").is_none() {
            root.insert(wrapper_key.to_string(), Value::Object(wrapper_map));
            continue;
        }
        for (key, value) in wrapper_map {
            root.entry(key).or_insert(value);
        }
        let mut event = tool_repair_event("fallback_normalization", "tool", "skill_manage");
        event.insert("source".to_string(), serde_json::json!(wrapper_key));
        event.insert("target".to_string(), serde_json::json!("$root"));
        append_tool_repair_event_map(&mut repair_metadata, event);
        break;
    }

    if matches!(root.get("action").and_then(Value::as_str), Some("create"))
        && root.get("body").is_none()
        && root.get("methodology").is_none()
        && root.get("content").is_some()
    {
        if let Some(content) = root.remove("content") {
            root.insert("body".to_string(), content);
            let mut event = tool_repair_event("field_alias_normalization", "tool", "skill_manage");
            event.insert("from".to_string(), serde_json::json!("content"));
            event.insert("to".to_string(), serde_json::json!("body"));
            append_tool_repair_event_map(&mut repair_metadata, event);
        }
    }

    let mut methodology = take_object_like(&mut root, "methodology", &mut repair_metadata)?;
    let mut frontmatter = take_object_like(&mut root, "frontmatter", &mut repair_metadata)?;

    if let Some(methodology_map) = methodology.as_mut() {
        let aliases = normalize_methodology_aliases(methodology_map);
        if !aliases.is_empty() {
            let mut event = tool_repair_event("alias_normalization", "tool", "skill_manage");
            event.insert("aliases".to_string(), serde_json::json!(aliases));
            event.insert("scope".to_string(), serde_json::json!("methodology"));
            append_tool_repair_event_map(&mut repair_metadata, event);
        }
    }

    if let Some(frontmatter_map) = frontmatter.as_mut() {
        if let Some(target) =
            normalize_frontmatter_shorthands(frontmatter_map, methodology.as_mut())
        {
            let mut event = tool_repair_event("fallback_normalization", "tool", "skill_manage");
            event.insert(
                "source".to_string(),
                serde_json::json!("frontmatter.prerequisites"),
            );
            event.insert("target".to_string(), serde_json::json!(target));
            append_tool_repair_event_map(&mut repair_metadata, event);
        }
    }

    if let Some(methodology_map) = methodology {
        root.insert("methodology".to_string(), Value::Object(methodology_map));
    }
    if let Some(frontmatter_map) = frontmatter {
        root.insert("frontmatter".to_string(), Value::Object(frontmatter_map));
    }

    Ok(NormalizedSkillManageArgs {
        args: Value::Object(root),
        repair_metadata,
    })
}

fn take_nested_root_object(
    field: &str,
    value: Value,
    repair_metadata: &mut Metadata,
) -> Result<Option<Map<String, Value>>, ToolError> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) => Ok(Some(map)),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let mut event = tool_repair_event(
                rocode_types::RepairKind::JsonStringObjectParse.as_str(),
                "tool",
                "skill_manage",
            );
            event.insert("field".to_string(), serde_json::json!(field));
            // P1.1: raw_shape is the JSON string, normalized_shape is the parsed object.
            event.insert("raw_shape".to_string(), Value::String(raw.clone()));
            let parsed =
                rocode_util::json::try_parse_json_object_robust(trimmed).ok_or_else(|| {
                    ToolError::InvalidArguments(format!(
                        "{field} must be a JSON object or object string"
                    ))
                })?;
            event.insert("normalized_shape".to_string(), parsed.clone());
            append_tool_repair_event_map(repair_metadata, event);
            match parsed {
                Value::Object(map) => Ok(Some(map)),
                _ => Err(ToolError::InvalidArguments(format!(
                    "{field} must be a JSON object or object string"
                ))),
            }
        }
        _ => Err(ToolError::InvalidArguments(format!(
            "{field} must be a JSON object or object string"
        ))),
    }
}

fn take_object_like(
    root: &mut Map<String, Value>,
    field: &str,
    repair_metadata: &mut Metadata,
) -> Result<Option<Map<String, Value>>, ToolError> {
    let Some(value) = root.remove(field) else {
        return Ok(None);
    };

    let value = match value {
        Value::Null => return Ok(None),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let mut event = tool_repair_event(
                rocode_types::RepairKind::JsonStringObjectParse.as_str(),
                "tool",
                "skill_manage",
            );
            event.insert("field".to_string(), serde_json::json!(field));
            event.insert("raw_shape".to_string(), Value::String(raw.clone()));
            let parsed =
                rocode_util::json::try_parse_json_object_robust(trimmed).ok_or_else(|| {
                    ToolError::InvalidArguments(format!(
                        "{field} must be a JSON object or object string"
                    ))
                })?;
            event.insert("normalized_shape".to_string(), parsed.clone());
            append_tool_repair_event_map(repair_metadata, event);
            parsed
        }
        other => other,
    };

    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => Err(ToolError::InvalidArguments(format!(
            "{field} must be a JSON object or object string"
        ))),
    }
}

fn normalize_methodology_aliases(methodology: &mut Map<String, Value>) -> Vec<String> {
    let mut aliases = Vec::new();
    move_array_alias(
        methodology,
        "trigger_conditions",
        "when_to_use",
        &mut aliases,
        "methodology",
    );
    move_array_alias(
        methodology,
        "steps",
        "core_steps",
        &mut aliases,
        "methodology",
    );

    if let Some(boundaries) = methodology.remove("boundaries") {
        if methodology.get("pitfalls").is_none() {
            methodology.insert("pitfalls".to_string(), boundaries);
        } else {
            append_value_array(methodology, "pitfalls", boundaries);
        }
        aliases.push("methodology.boundaries->pitfalls".to_string());
    }

    if let Some(Value::Array(steps)) = methodology.get_mut("core_steps") {
        for (index, step) in steps.iter_mut().enumerate() {
            let Some(step_map) = step.as_object_mut() else {
                continue;
            };
            if step_map.get("title").is_none() {
                if let Some(name) = step_map.remove("name") {
                    step_map.insert("title".to_string(), name);
                    aliases.push(format!("methodology.core_steps[{index}].name->title"));
                }
            }
            if step_map.get("action").is_none() {
                if let Some(description) = step_map.remove("description") {
                    step_map.insert("action".to_string(), description);
                    aliases.push(format!(
                        "methodology.core_steps[{index}].description->action"
                    ));
                }
            }
        }
    }
    aliases
}

fn normalize_frontmatter_shorthands(
    frontmatter: &mut Map<String, Value>,
    methodology: Option<&mut Map<String, Value>>,
) -> Option<&'static str> {
    let Some(prerequisites_value) = frontmatter.get("prerequisites").cloned() else {
        return None;
    };
    let Some(prerequisites) = string_array_from_value(&prerequisites_value) else {
        return None;
    };

    if let Some(methodology) = methodology {
        frontmatter.remove("prerequisites");
        append_string_array(methodology, "prerequisites", prerequisites);
        return Some("methodology.prerequisites");
    }

    if frontmatter.get("required_commands").is_none() && strings_look_like_commands(&prerequisites)
    {
        frontmatter.remove("prerequisites");
        frontmatter.insert(
            "required_commands".to_string(),
            Value::Array(prerequisites.into_iter().map(Value::String).collect()),
        );
        return Some("frontmatter.required_commands");
    }
    None
}

fn move_array_alias(
    target: &mut Map<String, Value>,
    from: &str,
    to: &str,
    aliases: &mut Vec<String>,
    scope: &str,
) {
    if target.get(to).is_some() {
        return;
    }
    if let Some(value) = target.remove(from) {
        target.insert(to.to_string(), value);
        aliases.push(format!("{scope}.{from}->{to}"));
    }
}

fn append_value_array(target: &mut Map<String, Value>, key: &str, incoming: Value) {
    let mut current = target
        .remove(key)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    if let Value::Array(mut incoming_items) = incoming {
        current.append(&mut incoming_items);
    }
    target.insert(key.to_string(), Value::Array(current));
}

fn append_string_array(target: &mut Map<String, Value>, key: &str, incoming: Vec<String>) {
    let mut current = target
        .remove(key)
        .and_then(|value| string_array_from_value(&value))
        .unwrap_or_default();
    for item in incoming {
        if !current.iter().any(|existing| existing == &item) {
            current.push(item);
        }
    }
    target.insert(
        key.to_string(),
        Value::Array(current.into_iter().map(Value::String).collect()),
    );
}

fn string_array_from_value(value: &Value) -> Option<Vec<String>> {
    let Value::Array(items) = value else {
        return None;
    };
    let values = items
        .iter()
        .map(|item| item.as_str().map(str::trim).map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    Some(values.into_iter().filter(|item| !item.is_empty()).collect())
}

fn strings_look_like_commands(values: &[String]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| {
            !value.chars().any(char::is_whitespace)
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
        })
}

fn write_action_label(action: &SkillWriteAction) -> &'static str {
    match action {
        SkillWriteAction::Created => "created",
        SkillWriteAction::Patched => "patched",
        SkillWriteAction::Edited => "edited",
        SkillWriteAction::SupportingFileWritten => "supporting_file_written",
        SkillWriteAction::SupportingFileRemoved => "supporting_file_removed",
        SkillWriteAction::Deleted => "deleted",
    }
}

fn format_output(result: &SkillGovernedWriteResult) -> String {
    let mut output = format!(
        "<skill_manage_result action=\"{}\" skill=\"{}\" path=\"{}\">",
        write_action_label(&result.result.action),
        result.result.skill_name,
        result.result.location.display()
    );
    if let Some(skill) = &result.result.skill {
        output.push_str(&format!(
            "\nname: {}\ndescription: {}\nlocation: {}",
            skill.name,
            skill.description,
            skill.location.display()
        ));
        if let Some(category) = skill.category.as_deref() {
            output.push_str(&format!("\ncategory: {}", category));
        }
        output.push_str(&format!(
            "\nsupporting_files: {}",
            skill.supporting_files.len()
        ));
    }
    if let Some(file_path) = result.result.supporting_file.as_deref() {
        output.push_str(&format!("\nfile_path: {}", file_path));
    }
    if let Some(report) = &result.guard_report {
        output.push_str(&format!(
            "\nguard_status: {:?}\nguard_violations: {}",
            report.status,
            report.violations.len()
        ));
    }
    output.push_str("\n</skill_manage_result>");
    output
}

fn format_metadata(result: &SkillGovernedWriteResult) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert(
        "action".to_string(),
        serde_json::json!(write_action_label(&result.result.action)),
    );
    metadata.insert(
        "name".to_string(),
        serde_json::json!(&result.result.skill_name),
    );
    metadata.insert(
        "location".to_string(),
        serde_json::json!(result.result.location.to_string_lossy().to_string()),
    );
    if let Some(skill) = &result.result.skill {
        metadata.insert(
            "skill".to_string(),
            serde_json::json!({
                "name": skill.name,
                "description": skill.description,
                "category": skill.category,
                "location": skill.location.to_string_lossy().to_string(),
                "supporting_files": skill.supporting_files.iter().map(|file| file.relative_path.clone()).collect::<Vec<_>>(),
            }),
        );
        metadata.insert(
            "display.summary".to_string(),
            serde_json::json!(format!(
                "{} {}",
                write_action_label(&result.result.action),
                skill.name
            )),
        );
    }
    if let Some(file_path) = result.result.supporting_file.as_deref() {
        metadata.insert("file_path".to_string(), serde_json::json!(file_path));
    }
    if let Some(report) = &result.guard_report {
        metadata.insert(
            "guard_report".to_string(),
            serde_json::to_value(report).unwrap_or_default(),
        );
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[tokio::test]
    async fn permission_denial_has_no_filesystem_side_effect() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Err(ToolError::PermissionDenied("denied".to_string())) });

        let err = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "blocked-skill",
                    "description": "blocked",
                    "body": "Blocked body."
                }),
                ctx,
            )
            .await
            .unwrap_err();

        assert!(matches!(err, ToolError::PermissionDenied(_)));
        assert!(!dir
            .path()
            .join(".rocode/skills/blocked-skill/SKILL.md")
            .exists());
    }

    #[tokio::test]
    async fn successful_create_is_visible_to_authority_immediately() {
        let dir = tempdir().unwrap();
        let requests = Arc::new(Mutex::new(Vec::<PermissionRequest>::new()));
        let seen = requests.clone();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(move |req| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(req);
                Ok(())
            }
        });

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "local-skill",
                    "description": "local",
                    "body": "Created from tool."
                }),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("local-skill"));
        let authority = crate::skill_support::authority_for(dir.path(), None);
        let names = authority
            .list_skill_meta(None)
            .unwrap()
            .into_iter()
            .map(|skill| skill.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"local-skill".to_string()));

        let permissions = requests.lock().unwrap();
        assert_eq!(permissions.len(), 1);
        assert_eq!(permissions[0].permission, "skill_manage");
    }

    #[tokio::test]
    async fn create_accepts_methodology_template_without_raw_body() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "structured-skill",
                    "description": "structured",
                    "methodology": {
                        "when_to_use": ["Use when a provider refresh workflow must be repeated."],
                        "when_not_to_use": ["Do not use for one-off local experiments."],
                        "core_steps": [
                            {
                                "title": "Refresh",
                                "action": "Run the refresh flow and capture the diff.",
                                "outcome": "Provider inventory is updated."
                            }
                        ],
                        "success_criteria": ["The expected provider ids are visible after refresh."],
                        "validation": ["Re-open the provider list and confirm the new ids appear."],
                        "pitfalls": ["Do not overwrite workspace-local sandbox overrides."]
                    }
                }),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("structured-skill"));
        let authority = crate::skill_support::authority_for(dir.path(), None);
        let loaded = authority
            .load_skill_for_inspection("structured-skill", None)
            .unwrap();
        assert!(loaded.content.contains("## When To Use"));
        assert!(loaded.content.contains("## Core Steps"));
        assert!(loaded.content.contains("## Validation"));
    }

    #[tokio::test]
    async fn create_accepts_stringified_methodology_and_frontmatter() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "stringified-skill",
                    "description": "structured from strings",
                    "methodology": "{\"when_to_use\":[\"Use when the model stringifies nested JSON.\"],\"core_steps\":[{\"title\":\"Parse\",\"action\":\"Accept stringified methodology objects.\",\"outcome\":\"Create succeeds.\"}],\"success_criteria\":[\"The skill is created.\"],\"validation\":[\"Load the generated skill.\"],\"pitfalls\":[\"Do not require the model to emit a raw nested object every time.\"]}",
                    "frontmatter": "{\"author\":\"rocode\",\"license\":\"MIT\",\"tags\":[\"skills\",\"ergonomics\"]}"
                }),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("stringified-skill"));
        let authority = crate::skill_support::authority_for(dir.path(), None);
        let loaded = authority
            .load_skill_for_inspection("stringified-skill", None)
            .unwrap();
        let source = authority
            .load_skill_source_for_inspection("stringified-skill", None)
            .unwrap();
        assert!(loaded.content.contains("## Core Steps"));
        assert!(loaded
            .content
            .contains("Use when the model stringifies nested JSON."));
        assert!(source.contains("author: rocode"));
        assert!(source.contains("license: MIT"));
        assert!(source.contains("tags:"));
        assert!(source.contains("- skills"));
        let repair_events = crate::tool_repair_events(&result.metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("json_string_object_parse")
                && event.get("field").and_then(|value| value.as_str()) == Some("methodology")
                && event.get("raw_shape").is_some()
                && event.get("normalized_shape").is_some()
        }));
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("json_string_object_parse")
                && event.get("field").and_then(|value| value.as_str()) == Some("frontmatter")
                && event.get("raw_shape").is_some()
                && event.get("normalized_shape").is_some()
        }));
    }

    #[tokio::test]
    async fn create_without_body_or_methodology_returns_helpful_shape() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let err = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "missing-shape-skill",
                    "description": "missing methodology"
                }),
                ctx,
            )
            .await
            .expect_err("create without body or methodology should fail");

        let message = err.to_string();
        assert!(message.contains("requires either `body` or `methodology`"));
        assert!(message.contains("\"action\":\"create\""));
        assert!(message.contains("\"methodology\""));
    }

    #[tokio::test]
    async fn create_normalizes_common_methodology_aliases_and_frontmatter_prereq_lists() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "alias-friendly-skill",
                    "description": "alias normalization",
                    "methodology": {
                        "trigger_conditions": ["Use when repeated audit work needs to be saved."],
                        "boundaries": ["Do not use for one-off experiments."],
                        "steps": [
                            {
                                "name": "Survey",
                                "description": "Read the project and capture risk surfaces.",
                                "outcome": "The scope is clear."
                            }
                        ],
                        "success_criteria": ["The workflow is reusable."],
                        "validation": ["Apply it to a second repository."]
                    },
                    "frontmatter": {
                        "author": "rocode",
                        "prerequisites": ["Ability to read the target codebase", "Basic security review literacy"]
                    }
                }),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("alias-friendly-skill"));
        let authority = crate::skill_support::authority_for(dir.path(), None);
        let loaded = authority
            .load_skill_for_inspection("alias-friendly-skill", None)
            .unwrap();
        assert!(loaded.content.contains("## When To Use"));
        assert!(loaded
            .content
            .contains("Use when repeated audit work needs to be saved."));
        assert!(loaded.content.contains("## Prerequisites"));
        assert!(loaded
            .content
            .contains("Ability to read the target codebase"));
        assert!(loaded.content.contains("## Core Steps"));
        assert!(loaded.content.contains("**Survey**"));
        assert!(loaded.content.contains("## Boundaries"));
        assert!(loaded
            .content
            .contains("Do not use for one-off experiments."));
        let repair_events = crate::tool_repair_events(&result.metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("alias_normalization")
                && event
                    .get("aliases")
                    .and_then(|value| value.as_array())
                    .is_some_and(|aliases| {
                        aliases.iter().any(|value| {
                            value.as_str() == Some("methodology.trigger_conditions->when_to_use")
                        })
                    })
        }));
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("fallback_normalization")
                && event.get("target").and_then(|value| value.as_str())
                    == Some("methodology.prerequisites")
        }));
    }

    #[tokio::test]
    async fn create_treats_content_as_body_alias() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "content-alias-skill",
                    "description": "content alias",
                    "content": "# Content Alias\n\nBody"
                }),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("content-alias-skill"));
        let repair_events = crate::tool_repair_events(&result.metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("field_alias_normalization")
                && event.get("from").and_then(|value| value.as_str()) == Some("content")
                && event.get("to").and_then(|value| value.as_str()) == Some("body")
        }));
    }

    #[tokio::test]
    async fn create_accepts_root_json_string_payload() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let result = tool
            .execute(
                serde_json::json!(
                    "{\"action\":\"create\",\"name\":\"root-json-skill\",\"description\":\"root string\",\"methodology\":{\"when_to_use\":[\"Use when the model stringifies the whole tool payload.\"],\"core_steps\":[{\"title\":\"Parse\",\"action\":\"Accept the root JSON string.\",\"outcome\":\"Create succeeds.\"}],\"success_criteria\":[\"The skill is created.\"],\"validation\":[\"Load the created skill.\"],\"pitfalls\":[\"Do not reject a valid object string at the root.\"]}}"
                ),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("root-json-skill"));
        let repair_events = crate::tool_repair_events(&result.metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("json_string_object_parse")
                && event.get("field").and_then(|value| value.as_str()) == Some("$root")
                && event.get("raw_shape").is_some()
                && event.get("normalized_shape").is_some()
        }));
    }

    #[tokio::test]
    async fn create_unwraps_nested_payload_object() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) });

        let result = tool
            .execute(
                serde_json::json!({
                    "payload": {
                        "action": "create",
                        "name": "payload-skill",
                        "description": "payload wrapper",
                        "methodology": {
                            "when_to_use": ["Use when arguments are wrapped under payload."],
                            "core_steps": [
                                {
                                    "title": "Unwrap",
                                    "action": "Promote payload fields to the root.",
                                    "outcome": "The request uses the normal create shape."
                                }
                            ],
                            "success_criteria": ["The skill is created."],
                            "validation": ["Load the generated skill."],
                            "pitfalls": ["Do not duplicate fields when root fields already exist."]
                        }
                    }
                }),
                ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("payload-skill"));
        let repair_events = crate::tool_repair_events(&result.metadata);
        assert!(repair_events.iter().any(|event| {
            event.get("kind").and_then(|value| value.as_str()) == Some("fallback_normalization")
                && event.get("source").and_then(|value| value.as_str()) == Some("payload")
                && event.get("target").and_then(|value| value.as_str()) == Some("$root")
        }));
    }

    #[test]
    fn description_includes_self_improvement_guidance() {
        let description = SkillManageTool.description();
        assert!(description.contains("complex task succeeded (5+ tool calls)"));
        assert!(description.contains("most reliable minimal shape"));
        assert!(description.contains("structured `methodology` shape"));
        assert!(description.contains("may be provided either as nested objects or as JSON strings"));
        assert!(description.contains("Common methodology aliases are normalized automatically"));
        assert!(description
            .contains("keep human-readable prerequisites in `methodology.prerequisites`"));
        assert!(description.contains("current session's tool call history"));
        assert!(description.contains("optional `experienced_tools` field"));
        assert!(description.contains("After difficult or iterative tasks"));
        assert!(description.contains("Patch when instructions are stale or wrong"));
        assert!(description.contains("Confirm with the user before creating or deleting"));
    }

    #[test]
    fn parameters_include_action_aware_requirements_and_examples() {
        let schema = SkillManageTool.parameters();
        let all_of = schema
            .get("allOf")
            .and_then(|value| value.as_array())
            .expect("skill_manage schema should expose action-aware requirements");
        assert!(!all_of.is_empty());

        let examples = schema
            .get("examples")
            .and_then(|value| value.as_array())
            .expect("skill_manage schema should expose examples");
        assert!(examples.len() >= 2);

        let methodology = schema
            .get("properties")
            .and_then(|value| value.get("methodology"))
            .expect("methodology property should exist");
        let methodology_one_of = methodology
            .get("oneOf")
            .and_then(|value| value.as_array())
            .expect("methodology should accept object or string");
        assert_eq!(methodology_one_of.len(), 2);
    }
}
