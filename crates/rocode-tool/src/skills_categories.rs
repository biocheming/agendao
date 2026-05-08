use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

use crate::skill_support::{
    authority_for, collect_skill_category_views, format_skill_categories_output,
    list_runtime_visible_skill_meta, resolve_skill_filter,
};
use crate::{Tool, ToolContext, ToolError, ToolResult};
use rocode_config::ConfigStore;

pub struct SkillsCategoriesTool;

#[derive(Debug, Default, Deserialize)]
struct SkillsCategoriesInput {
    #[serde(default)]
    verbose: Option<bool>,
}

#[async_trait]
impl Tool for SkillsCategoriesTool {
    fn id(&self) -> &str {
        "skills_categories"
    }

    fn description(&self) -> &str {
        "First-step skill discovery by category. List available skill categories with counts and optional descriptions before calling skills_list(category)."
    }

    fn parameters(&self) -> serde_json::Value {
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config_store = ConfigStore::from_project_dir(&base).ok().map(Arc::new);
        let authority = authority_for(&base, config_store);
        let category_names = authority
            .list_skill_categories(None)
            .unwrap_or_default()
            .into_iter()
            .map(|category| category.name)
            .collect::<Vec<_>>();

        serde_json::json!({
            "type": "object",
            "properties": {
                "verbose": {
                    "type": "boolean",
                    "description": "Reserved for compatibility. Category counts are always included.",
                    "default": false
                }
            },
            "x-known-categories": category_names
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: SkillsCategoriesInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let _ = input.verbose;
        let resolved_filter = resolve_skill_filter(&ctx, None).await;
        let filter = resolved_filter.as_filter();
        let skills = list_runtime_visible_skill_meta(
            std::path::Path::new(&ctx.directory),
            ctx.config_store.clone(),
            Some(&filter),
        )?;
        let categories = collect_skill_category_views(&skills);
        let output = format_skill_categories_output(&categories);

        let mut result = ToolResult::simple("Available skill categories", output)
            .with_metadata("count", serde_json::json!(categories.len()))
            .with_metadata(
                "categories",
                serde_json::json!(categories
                    .iter()
                    .map(|category| serde_json::json!({
                        "name": category.name,
                        "skill_count": category.skill_count,
                        "description": category.description,
                    }))
                    .collect::<Vec<_>>()),
            )
            .with_metadata(
                "hint",
                serde_json::json!(
                    "Use skills_list(category) to inspect the skills inside a relevant category."
                ),
            );

        if categories.is_empty() {
            result =
                result.with_metadata("message", serde_json::json!("No skills directory found."));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_categories_description_points_to_skills_list() {
        let tool = SkillsCategoriesTool;
        assert!(tool.description().contains("skills_list(category)"));
    }
}
