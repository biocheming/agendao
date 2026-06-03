use jsonc_parser::{parse_to_serde_value, ParseOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::builtin_categories::builtin_categories;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCategoryFile {
    pub categories: HashMap<String, TaskCategoryDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCategoryDef {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<CategoryModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryModel {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone)]
pub struct CategoryRegistry {
    categories: HashMap<String, TaskCategoryDef>,
}

impl CategoryRegistry {
    pub fn empty() -> Self {
        Self {
            categories: HashMap::new(),
        }
    }

    /// Create a registry with built-in default categories.
    pub fn with_builtins() -> Self {
        Self {
            categories: builtin_categories(),
        }
    }

    /// Load from file and merge with builtins. User definitions override
    /// builtins on key collision.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read task category file {:?}: {}", path, e))?;

        let parse_options = ParseOptions {
            allow_trailing_commas: true,
            ..Default::default()
        };
        let file: TaskCategoryFile = parse_to_serde_value(&content, &parse_options)
            .map_err(|e| anyhow::anyhow!("Failed to parse task category file: {}", e))?
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to deserialize task category file: {}", e))?
            .unwrap_or_else(|| TaskCategoryFile {
                categories: HashMap::new(),
            });

        // Start with builtins, then overlay user-defined categories.
        let mut merged = builtin_categories();
        merged.extend(file.categories);

        Ok(Self { categories: merged })
    }

    pub fn resolve(&self, category: &str) -> Option<&TaskCategoryDef> {
        self.categories.get(category)
    }

    pub fn category_descriptions(&self) -> Vec<(String, String)> {
        self.categories
            .iter()
            .map(|(name, def)| (name.clone(), def.description.clone()))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }
}
