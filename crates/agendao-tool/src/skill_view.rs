use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;

use crate::skill_support::{
    attach_skill_runtime_preflight, authority_for, format_loaded_skill_file_output,
    format_loaded_skill_output, load_skill_file_with_runtime_materialization,
    load_skill_prompt_packet_with_runtime_materialization, map_skill_error, resolve_skill_filter,
    resolve_skill_with_runtime_materialization,
};
use crate::{PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

pub struct SkillViewTool;

#[derive(Debug, Deserialize)]
struct SkillViewInput {
    name: String,
    #[serde(default)]
    file_path: Option<String>,
}

#[async_trait]
impl Tool for SkillViewTool {
    fn id(&self) -> &str {
        "skill_view"
    }

    fn description(&self) -> &str {
        "Load a specific skill's full SKILL.md content or one supporting file. Use skills_categories, then skills_list, to choose the correct skill."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact skill name. Use skills_categories and skills_list first to inspect names, descriptions, and categories."
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional supporting file path relative to the skill root, e.g. references/api.md"
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
            let loaded = load_skill_file_with_runtime_materialization(
                Path::new(&ctx.directory),
                ctx.config_store.clone(),
                &input.name,
                file_path,
                Some(&filter),
                Some(&ctx.extra),
            )?;
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
}
