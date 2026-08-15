use agendao_skill::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, SkillGovernedWriteResult, SkillWriteAction, WriteSkillFileRequest,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;

use crate::skill_support::{governance_authority_for, map_skill_error};
use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

pub struct SkillManageTool;

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
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    methodology: Option<agendao_skill::SkillMethodologyTemplate>,
    #[serde(default)]
    frontmatter: Option<agendao_skill::SkillFrontmatterPatch>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    directory_name: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
}

#[async_trait]
impl Tool for SkillManageTool {
    fn id(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Manage workspace-local skills under .agendao/skills. Create when a complex task succeeded (5+ tool calls), you overcame errors, a corrected approach worked, or the user asks you to remember a procedure. Confirm with the user before creating or deleting.

Minimal create shape (methodology variant, preferred):
{\"action\":\"create\",\"name\":\"skill-name\",\"description\":\"what it does\",\"methodology\":{\"when_to_use\":\"...\",\"core_steps\":[{\"title\":\"...\",\"action\":\"...\"}]}}

Minimal create shape (body variant, for simple free-form skills):
{\"action\":\"create\",\"name\":\"skill-name\",\"description\":\"what it does\",\"body\":\"# Skill...\"}

Other actions: patch, edit, delete. Patch when instructions are stale, steps are missing, or a skill failed in a specific environment. Skip simple one-offs.

Canonical content fields: use `body` for free-form markdown skills or `methodology` for structured skills. Methodology and frontmatter must be nested objects."
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
                    "description": "Structured methodology template for create or patch. Use this OR `body`, not both. Required canonical fields include `when_to_use` and `core_steps`; each step uses `title`, `action`, and optional `outcome`.",
                    "type": "object"
                },
                "frontmatter": {
                    "description": "Optional structured YAML frontmatter patch for rich metadata such as version, author, license, tags, required_commands, metadata blocks, or structured setup prerequisites.",
                    "type": "object"
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
                    "description": "Optional leaf directory name to use under .agendao/skills for create. If omitted, AgenDao derives it from the name."
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
        let input: SkillManageInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
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
        let metadata = format_metadata(&result);
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
    methodology: &Option<agendao_skill::SkillMethodologyTemplate>,
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
    methodology: &Option<agendao_skill::SkillMethodologyTemplate>,
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
    methodology: Option<agendao_skill::SkillMethodologyTemplate>,
    action: &str,
) -> Result<String, ToolError> {
    ensure_body_and_methodology_not_both_set(&body, &methodology, action)?;
    if let Some(methodology) = methodology {
        return agendao_skill::render_methodology_skill_body(skill_name, &methodology)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()));
    }
    required_string(body, "body")
}

fn resolve_optional_skill_body(
    skill_name: &str,
    body: Option<String>,
    methodology: Option<agendao_skill::SkillMethodologyTemplate>,
    action: &str,
) -> Result<Option<String>, ToolError> {
    ensure_body_and_methodology_not_both_set(&body, &methodology, action)?;
    if let Some(methodology) = methodology {
        return agendao_skill::render_methodology_skill_body(skill_name, &methodology)
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
            .join(".agendao/skills/blocked-skill/SKILL.md")
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
    async fn rejects_noncanonical_input_shapes() {
        let dir = tempdir().unwrap();
        let tool = SkillManageTool;
        let inputs = [
            serde_json::json!("{\"action\":\"delete\",\"name\":\"legacy\"}"),
            serde_json::json!({"payload": {"action": "delete", "name": "legacy"}}),
            serde_json::json!({
                "action": "create",
                "name": "legacy",
                "description": "legacy",
                "content": "legacy body"
            }),
            serde_json::json!({
                "action": "create",
                "name": "legacy",
                "description": "legacy",
                "methodology": "{}"
            }),
            serde_json::json!({
                "action": "write_file",
                "name": "legacy",
                "filepath": "references/api.md",
                "content": "legacy"
            }),
        ];

        for input in inputs {
            let ctx = ToolContext::new(
                "session".to_string(),
                "message".to_string(),
                dir.path().to_string_lossy().to_string(),
            )
            .with_ask(|_| async { Ok(()) });
            assert!(matches!(
                tool.execute(input, ctx).await,
                Err(ToolError::InvalidArguments(_))
            ));
        }
    }

    #[test]
    fn description_includes_self_improvement_guidance() {
        let description = SkillManageTool.description();
        assert!(description.contains("complex task succeeded (5+ tool calls)"));
        assert!(description.contains("methodology"));
        assert!(description.contains("create"));
        assert!(description.contains("must be nested objects"));
        assert!(description.contains("Patch when instructions are stale"));
        assert!(description.contains("Confirm with the user before creating or deleting"));
        assert!(description.contains("Skip simple one-offs"));
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
        assert_eq!(methodology.get("type"), Some(&serde_json::json!("object")));
    }
}
