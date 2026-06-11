use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;

use crate::skill_support::{
    attach_skill_runtime_preflight, authority_for, format_loaded_skill_file_output,
    format_loaded_skill_output, format_supporting_files_hint, load_skill_file_with_runtime_materialization,
    load_skill_prompt_packet_with_runtime_materialization, map_skill_error, resolve_skill_filter,
    resolve_skill_with_runtime_materialization,
};
use crate::{PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

pub struct SkillViewTool;

#[derive(Debug, Deserialize)]
struct SkillViewInput {
    name: String,
    #[serde(default, alias = "filepath")]
    file_path: Option<String>,
}

#[async_trait]
impl Tool for SkillViewTool {
    fn id(&self) -> &str {
        "skill_view"
    }

    fn description(&self) -> &str {
        "Load a specific skill's full SKILL.md content or one supporting file for inspection. Use skills_categories, then skills_list, to choose the correct skill. Skill files are not execution resource ids; if you need a runnable tool, return to tool_catalog_search and use tool_catalog_call with an exact search result name."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact short skill name such as `semantic-scholar` or `author-network`. Do not pass category-prefixed display labels or pseudo file paths."
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional supporting file path relative to the skill root, e.g. references/api.md. This is only for linked files inside the skill. Do not pass category paths like `skills/semantic-scholar/skill.md`, and do not pass skill file paths to tool_catalog_call.tool."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: SkillViewInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let authority = authority_for(Path::new(&ctx.directory), ctx.config_store.clone());
        let resolved_filter = resolve_skill_filter(&ctx, None).await;
        let filter = resolved_filter.as_filter();

        if let Some(file_path) = input.file_path.as_deref() {
            let meta = resolve_skill_with_runtime_materialization(
                Path::new(&ctx.directory),
                ctx.config_store.clone(),
                &input.name,
                Some(&filter),
                Some(&ctx.extra),
            )?;
            ctx.ask_permission(
                PermissionRequest::new("skill")
                    .with_pattern(&meta.name)
                    .with_always(&meta.name),
            )
            .await?;
            let loaded = match load_skill_file_with_runtime_materialization(
                Path::new(&ctx.directory),
                ctx.config_store.clone(),
                &input.name,
                file_path,
                Some(&filter),
                Some(&ctx.extra),
            ) {
                Ok(loaded) => loaded,
                Err(ToolError::InvalidArguments(message))
                    if message.starts_with("Skill file not found for ")
                        || message.starts_with("Invalid skill file path for ") =>
                {
                    let packet = load_skill_prompt_packet_with_runtime_materialization(
                        Path::new(&ctx.directory),
                        ctx.config_store.clone(),
                        &input.name,
                        Some(&filter),
                        Some(std::slice::from_ref(&input.name)),
                    )?;
                    let (output, mut metadata) =
                        format_loaded_skill_output(&packet, None, None);
                    metadata.insert(
                        "requested_file_path".to_string(),
                        serde_json::json!(file_path),
                    );
                    metadata.insert(
                        "file_path_error".to_string(),
                        serde_json::json!(message.clone()),
                    );
                    metadata.insert(
                        "hint".to_string(),
                        serde_json::json!(format_supporting_files_hint(&meta.supporting_files)),
                    );
                    attach_skill_runtime_preflight(
                        &mut metadata,
                        &packet.meta.name,
                        &packet.meta.name,
                        &meta.supporting_files,
                        &packet.detail,
                    );
                    return Ok(ToolResult {
                        title: format!("Loaded skill: {}", packet.meta.name),
                        output,
                        metadata,
                        truncated: false,
                    });
                }
                Err(err) => return Err(err),
            };
            let detail = authority
                .load_skill_detail_for_meta_for_inspection(&meta)
                .map_err(map_skill_error)?;
            let (output, mut metadata) = format_loaded_skill_file_output(&loaded);
            attach_skill_runtime_preflight(
                &mut metadata,
                &meta.name,
                &format!("{}::{}", meta.name, loaded.file_path),
                &meta.supporting_files,
                &detail,
            );
            return Ok(ToolResult {
                title: format!(
                    "Loaded skill file: {} :: {}",
                    loaded.skill_name, loaded.file_path
                ),
                output,
                metadata,
                truncated: false,
            });
        }

        ctx.ask_permission(
            PermissionRequest::new("skill")
                .with_pattern(&input.name)
                .with_always(&input.name),
        )
        .await?;

        let packet = load_skill_prompt_packet_with_runtime_materialization(
            Path::new(&ctx.directory),
            ctx.config_store.clone(),
            &input.name,
            Some(&filter),
            Some(std::slice::from_ref(&input.name)),
        )?;
        let (output, mut metadata) = format_loaded_skill_output(&packet, None, None);
        attach_skill_runtime_preflight(
            &mut metadata,
            &packet.meta.name,
            &packet.meta.name,
            &packet.meta.supporting_files,
            &packet.detail,
        );
        Ok(ToolResult {
            title: format!("Loaded skill: {}", packet.meta.name),
            output,
            metadata,
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn skill_view_file_result_attaches_skill_runtime_preflight() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join(".agendao/skills/frontend-ui-ux");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: frontend-ui-ux
description: frontend
required_commands: [definitely-missing-skill-cli]
---
Use clear visual hierarchy.
"#,
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("references/api.md"),
            "Use the design tokens in this file.\n",
        )
        .unwrap();

        let ctx = ToolContext::new(
            "session-1".into(),
            "message-1".into(),
            dir.path().to_string_lossy().to_string(),
        );
        let args = serde_json::json!({
            "name": "frontend-ui-ux",
            "file_path": "references/api.md"
        });

        let result = SkillViewTool.execute(args, ctx).await.unwrap();

        assert_eq!(result.metadata["file"], "references/api.md");
        assert_eq!(
            result.metadata["preflight"]["subject"],
            "frontend-ui-ux::references/api.md"
        );
        assert_eq!(result.metadata["preflight"]["status"], "soft_warn");
        assert_eq!(
            result.metadata["preflight"]["metadata"]["missing_required_commands"],
            serde_json::json!(["definitely-missing-skill-cli"])
        );
    }

    #[test]
    fn skill_view_description_warns_against_using_skill_files_as_tool_ids() {
        let tool = SkillViewTool;
        assert!(tool.description().contains("not execution resource ids"));
        assert!(tool.description().contains("tool_catalog_search"));
        assert!(tool.parameters()["properties"]["file_path"]["description"]
            .as_str()
            .expect("file_path description")
            .contains("do not pass skill file paths to tool_catalog_call.tool"));
        assert!(tool.parameters()["properties"]["name"]["description"]
            .as_str()
            .expect("name description")
            .contains("Exact short skill name"));
        assert!(tool.parameters()["properties"]["file_path"]["description"]
            .as_str()
            .expect("file_path description")
            .contains("Do not pass category paths like `skills/semantic-scholar/skill.md`"));
    }

    #[tokio::test]
    async fn skill_view_missing_supporting_file_falls_back_to_main_skill_with_hint() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join(".agendao/skills/pubmed-database");
        std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: pubmed-database
description: PubMed search
---
Use PubMed APIs.
"#,
        )
        .unwrap();
        std::fs::write(skill_dir.join("scripts/search.py"), "print('ok')\n").unwrap();

        let ctx = ToolContext::new(
            "session-1".into(),
            "message-1".into(),
            dir.path().to_string_lossy().to_string(),
        );
        let result = SkillViewTool
            .execute(
                serde_json::json!({
                    "name": "pubmed-database",
                    "file_path": "references/api.md"
                }),
                ctx,
            )
            .await
            .expect("fallback should succeed");

        assert_eq!(result.title, "Loaded skill: pubmed-database");
        assert!(result.output.contains("# Skill: pubmed-database"));
        assert_eq!(
            result.metadata.get("requested_file_path"),
            Some(&serde_json::json!("references/api.md"))
        );
        assert!(result.metadata["file_path_error"]
            .as_str()
            .unwrap_or_default()
            .contains("Skill file not found"));
        assert!(!result.metadata["hint"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .is_empty());
    }

    #[test]
    fn skill_view_accepts_filepath_alias() {
        let input: SkillViewInput = serde_json::from_value(serde_json::json!({
            "name": "pubmed-database",
            "filepath": "references/api.md"
        }))
        .expect("filepath alias should deserialize");

        assert_eq!(input.file_path.as_deref(), Some("references/api.md"));
    }
}
