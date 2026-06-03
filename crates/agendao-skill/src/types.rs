use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_SKILL_LIST_DESCRIPTION_CHARS: usize = 1024;

pub(crate) fn truncate_catalog_description(description: &str) -> String {
    let trimmed = description.trim();
    if trimmed.chars().count() <= MAX_SKILL_LIST_DESCRIPTION_CHARS {
        return trimmed.to_string();
    }

    let mut truncated = trimmed
        .chars()
        .take(MAX_SKILL_LIST_DESCRIPTION_CHARS.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillConditions {
    #[serde(default)]
    pub requires_tools: Vec<String>,
    #[serde(default)]
    pub fallback_for_tools: Vec<String>,
    #[serde(default)]
    pub requires_toolsets: Vec<String>,
    #[serde(default)]
    pub fallback_for_toolsets: Vec<String>,
    #[serde(default)]
    pub stage_filter: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFileRef {
    pub relative_path: String,
    pub location: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub location: PathBuf,
    #[serde(default)]
    pub supporting_files: Vec<SkillFileRef>,
    #[serde(default)]
    pub conditions: SkillConditions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetaView {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
}

impl From<&SkillMeta> for SkillMetaView {
    fn from(value: &SkillMeta) -> Self {
        Self {
            name: value.name.clone(),
            description: truncate_catalog_description(&value.description),
            category: value.category.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCategoryView {
    pub name: String,
    pub skill_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillHermesMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_skills: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillAgendaoMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_for_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_toolsets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_for_toolsets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_filter: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillMetadataBlocks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hermes: Option<SkillHermesMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agendao: Option<SkillAgendaoMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillPrerequisites {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillReadinessStatus {
    Available,
    SetupNeeded,
    Unsupported,
}

impl Default for SkillReadinessStatus {
    fn default() -> Self {
        Self::Available
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRequiredEnvironmentVariable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_for: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<SkillPrerequisites>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_environment_variables: Vec<SkillRequiredEnvironmentVariable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SkillMetadataBlocks>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillFrontmatterPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<SkillPrerequisites>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_environment_variables: Option<Vec<SkillRequiredEnvironmentVariable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_commands: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SkillMetadataBlocks>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillDetailView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub related_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<SkillPrerequisites>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SkillMetadataBlocks>,
    #[serde(default)]
    pub required_environment_variables: Vec<SkillRequiredEnvironmentVariable>,
    #[serde(default)]
    pub required_commands: Vec<String>,
    #[serde(default)]
    pub missing_required_environment_variables: Vec<String>,
    #[serde(default)]
    pub missing_required_commands: Vec<String>,
    #[serde(default)]
    pub setup_needed: bool,
    #[serde(default)]
    pub setup_skipped: bool,
    #[serde(default)]
    pub readiness_status: SkillReadinessStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub location: PathBuf,
}

impl From<&SkillMeta> for SkillSummary {
    fn from(value: &SkillMeta) -> Self {
        Self {
            name: value.name.clone(),
            description: value.description.clone(),
            location: value.location.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedSkill {
    pub meta: SkillMeta,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedSkillFile {
    pub skill_name: String,
    pub file_path: String,
    pub location: PathBuf,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_meta_view_truncates_long_descriptions_for_list_views() {
        let meta = SkillMeta {
            name: "example".to_string(),
            description: "a".repeat(MAX_SKILL_LIST_DESCRIPTION_CHARS + 32),
            category: Some("demo".to_string()),
            location: PathBuf::from("/tmp/example/SKILL.md"),
            supporting_files: Vec::new(),
            conditions: SkillConditions::default(),
        };

        let view = SkillMetaView::from(&meta);
        assert_eq!(
            view.description.chars().count(),
            MAX_SKILL_LIST_DESCRIPTION_CHARS
        );
        assert!(view.description.ends_with("..."));
        assert_eq!(
            meta.description.chars().count(),
            MAX_SKILL_LIST_DESCRIPTION_CHARS + 32
        );
    }

    #[test]
    fn skill_meta_view_keeps_short_descriptions_unchanged() {
        let meta = SkillMeta {
            name: "example".to_string(),
            description: "short description".to_string(),
            category: None,
            location: PathBuf::from("/tmp/example/SKILL.md"),
            supporting_files: Vec::new(),
            conditions: SkillConditions::default(),
        };

        let view = SkillMetaView::from(&meta);
        assert_eq!(view.description, "short description");
    }

    #[test]
    fn truncate_catalog_description_keeps_output_within_limit() {
        let description = "a".repeat(MAX_SKILL_LIST_DESCRIPTION_CHARS + 12);
        let truncated = truncate_catalog_description(&description);
        assert_eq!(truncated.chars().count(), MAX_SKILL_LIST_DESCRIPTION_CHARS);
        assert!(truncated.ends_with("..."));
    }
}
