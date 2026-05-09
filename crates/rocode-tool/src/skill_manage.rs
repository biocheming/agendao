use async_trait::async_trait;
use rocode_skill::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, SkillGovernedWriteResult, SkillWriteAction, WriteSkillFileRequest,
};
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
    methodology: Option<rocode_skill::SkillMethodologyTemplate>,
    #[serde(default)]
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

#[async_trait]
impl Tool for SkillManageTool {
    fn id(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Create, patch, edit, delete, or manage supporting files for workspace-local skills under .rocode/skills. Create when a complex task succeeded (5+ tool calls), you overcame errors, a user-corrected approach worked, you discovered a non-trivial workflow, or the user asks you to remember a procedure. Prefer the structured `methodology` shape when creating or patching a skill so the result includes trigger conditions, core steps, success criteria, validation, and boundaries. When creating or patching a methodology skill with core steps, review the current session's tool call history and fill each step's optional `experienced_tools` field with the tool names actually used in that step. For commands invoked through bash, use the command name you actually ran, such as `docker` or `cargo`, instead of `bash`; leave `experienced_tools` empty if you are unsure. Patch when instructions are stale or wrong, a skill fails on a specific OS or environment, steps or pitfalls are missing, or you used a skill and found gaps not covered by it. After difficult or iterative tasks, offer to save the approach as a skill. Skip simple one-offs. Confirm with the user before creating or deleting skills."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "patch", "edit", "write_file", "remove_file", "delete"],
                    "description": "Mutation to perform."
                },
                "name": {
                    "type": "string",
                    "description": "Existing skill name for patch/edit/write_file/remove_file/delete, or new skill name for create."
                },
                "new_name": {
                    "type": "string",
                    "description": "Optional renamed skill name for patch."
                },
                "description": {
                    "type": "string",
                    "description": "Skill description for create or patch."
                },
                "body": {
                    "type": "string",
                    "description": "Skill markdown body for create or patch."
                },
                "methodology": {
                    "type": "object",
                    "description": "Structured methodology template for create or patch. Use this when you want skill_manage to generate a rubric-shaped SKILL body with trigger conditions, core steps, success criteria, validation, and boundaries. Each core step may also include an optional experienced_tools array of tool names actually used in that step based on the current session history."
                },
                "frontmatter": {
                    "type": "object",
                    "description": "Optional structured YAML frontmatter patch for rich skill metadata such as version, author, license, tags, prerequisites, required_commands, and metadata blocks."
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
                    "description": "Optional leaf directory name to use under .rocode/skills for create."
                },
                "file_path": {
                    "type": "string",
                    "description": "Supporting file path relative to the skill directory."
                }
            },
            "required": ["action"]
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
        Ok(ToolResult {
            title: format!("Skill {}", write_action_label(&result.result.action)),
            output,
            metadata: format_metadata(&result),
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
        "{action} requires either `body` or `methodology`"
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

    #[test]
    fn description_includes_self_improvement_guidance() {
        let description = SkillManageTool.description();
        assert!(description.contains("complex task succeeded (5+ tool calls)"));
        assert!(description.contains("structured `methodology` shape"));
        assert!(description.contains("current session's tool call history"));
        assert!(description.contains("optional `experienced_tools` field"));
        assert!(description.contains("After difficult or iterative tasks"));
        assert!(description.contains("Patch when instructions are stale or wrong"));
        assert!(description.contains("Confirm with the user before creating or deleting"));
    }
}
