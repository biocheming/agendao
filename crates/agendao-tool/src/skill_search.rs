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

pub struct SkillSearchTool;

#[derive(Debug, Deserialize)]
struct SkillSearchInput {
    query: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    12
}

const MAX_LIMIT: usize = 50;

#[async_trait]
impl Tool for SkillSearchTool {
    fn id(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search skills by keyword before using skill_view(name). Use this when a category is large or when you only know a topic such as pubmed, docking, citation, or review."
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
                "query": {
                    "type": "string",
                    "description": "Keyword to match against skill name, description, or category. Examples: pubmed, semantic scholar, docking, citation."
                },
                "category": {
                    "type": "string",
                    "description": "Optional category to narrow the search. Use skills_categories first if you do not know category names.",
                    "enum": categories
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 12
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: SkillSearchInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let query = input.query.trim();
        if query.is_empty() {
            return Err(ToolError::InvalidArguments(
                "query must not be empty".to_string(),
            ));
        }

        let resolved_filter = resolve_skill_filter(&ctx, input.category.as_deref()).await;
        let filter = resolved_filter.as_filter();
        let mut skills = list_runtime_visible_skill_meta(
            std::path::Path::new(&ctx.directory),
            ctx.config_store.clone(),
            Some(&filter),
        )?;
        let needle = query.to_ascii_lowercase();
        skills.retain(|skill| {
            skill.name.to_ascii_lowercase().contains(&needle)
                || skill.description.to_ascii_lowercase().contains(&needle)
                || skill
                    .category
                    .as_deref()
                    .map(|category| category.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
        });
        skills.sort_by(|left, right| {
            search_skill_rank(left, &needle)
                .cmp(&search_skill_rank(right, &needle))
                .then_with(|| left.name.cmp(&right.name))
        });

        let total_matches = skills.len();
        let limit = input.limit.clamp(1, MAX_LIMIT);
        if skills.len() > limit {
            skills.truncate(limit);
        }

        let categories = collect_skill_categories(&skills);
        let output = format_skill_list_output(&skills);
        let mut result = ToolResult::simple("Matching skills", output)
            .with_metadata("query", serde_json::json!(query))
            .with_metadata("count", serde_json::json!(skills.len()))
            .with_metadata("total_matches", serde_json::json!(total_matches))
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
                    "Use skill_view(name) with one exact short skill name from these results. Category labels help discovery only; do not turn them into skill_view.file_path values."
                ),
            );

        if total_matches == 0 {
            result = result.with_metadata("message", serde_json::json!("No matching skills found."));
        }

        Ok(result)
    }
}

fn search_skill_rank(skill: &agendao_skill::SkillMetaView, needle: &str) -> (u8, u8, u8) {
    let exact_name = (skill.name.eq_ignore_ascii_case(needle)) as u8;
    let prefix_name = skill.name.to_ascii_lowercase().starts_with(needle) as u8;
    let category_match = skill
        .category
        .as_deref()
        .map(|category| category.eq_ignore_ascii_case(needle))
        .unwrap_or(false) as u8;
    (
        1_u8.saturating_sub(exact_name),
        1_u8.saturating_sub(prefix_name),
        1_u8.saturating_sub(category_match),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_search_description_points_to_skill_view() {
        let tool = SkillSearchTool;
        assert!(tool.description().contains("skill_view(name)"));
    }

    #[test]
    fn skill_search_hint_warns_against_turning_categories_into_paths() {
        let hint = "Use skill_view(name) with one exact short skill name from these results. Category labels help discovery only; do not turn them into skill_view.file_path values.";
        assert!(hint.contains("skill_view.file_path"));
        assert!(hint.contains("Category labels help discovery only"));
    }
}
