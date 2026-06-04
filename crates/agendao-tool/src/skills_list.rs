use agendao_config::ConfigStore;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

use crate::skill_support::{
    authority_for, collect_skill_categories, format_skill_list_output,
    list_runtime_visible_skill_meta, resolve_skill_filter,
};
use crate::{Tool, ToolContext, ToolError, ToolResult};

pub struct SkillsListTool;

#[derive(Debug, Deserialize)]
struct SkillsListInput {
    #[serde(default)]
    category: Option<String>,
}

#[async_trait]
impl Tool for SkillsListTool {
    fn id(&self) -> &str {
        "skills_list"
    }

    fn description(&self) -> &str {
        "Second-step skill discovery. After skills_categories, list available skills with name, description, and category before using skill_view(name) or skill(name)."
    }

    fn parameters(&self) -> serde_json::Value {
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config_store = ConfigStore::from_project_dir(&base).ok().map(Arc::new);
        let authority = authority_for(&base, config_store);
        let categories = authority
            .list_skill_categories(None)
            .unwrap_or_default()
            .into_iter()
            .map(|category| category.name)
            .collect::<Vec<_>>();

        serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "Optional category filter. Use skills_categories first if you do not know the category names.",
                    "enum": categories
                }
            }
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: SkillsListInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let resolved_filter = resolve_skill_filter(&ctx, input.category.as_deref()).await;
        let filter = resolved_filter.as_filter();
        let skills = list_runtime_visible_skill_meta(
            std::path::Path::new(&ctx.directory),
            ctx.config_store.clone(),
            Some(&filter),
        )?;
        let categories = collect_skill_categories(&skills);
        let output = format_skill_list_output(&skills);

        let mut result = ToolResult::simple("Available skills", output)
            .with_metadata("count", serde_json::json!(skills.len()))
            .with_metadata(
                "skills",
                serde_json::json!(skills
                    .iter()
                    .map(|skill| serde_json::json!({
                        "name": skill.name,
                        "description": skill.description,
                        "category": skill.category,
                    }))
                    .collect::<Vec<_>>()),
            )
            .with_metadata("categories", serde_json::json!(categories))
            .with_metadata(
                "hint",
                serde_json::json!(
                    "Use skill_view(name) to see full content, tags, and linked files"
                ),
            );

        if skills.is_empty() {
            result = result.with_metadata("message", serde_json::json!("No skills found."));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_list_description_points_to_skills_categories() {
        let tool = SkillsListTool;
        assert!(tool.description().contains("skills_categories"));
    }
}
