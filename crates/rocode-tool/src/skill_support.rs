use rocode_config::ConfigStore;
use rocode_skill::{
    infer_toolsets_from_tools, LoadedSkill, LoadedSkillFile, SkillAuthority, SkillCategoryView,
    SkillDetailView, SkillError, SkillFilter, SkillGovernanceAuthority, SkillMetaView,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::{Metadata, ToolContext, ToolError};

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedSkillsPromptContext {
    pub prompt_context: String,
    pub loaded_skills: Vec<SkillMetaView>,
}

impl LoadedSkillsPromptContext {
    pub fn loaded_skill_names(&self) -> Vec<String> {
        self.loaded_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.prompt_context.trim().is_empty()
    }
}

pub(crate) fn map_skill_error(err: SkillError) -> ToolError {
    match err {
        SkillError::UnknownSkill {
            requested,
            available,
        } => ToolError::InvalidArguments(format!(
            "Unknown skill: {}. Available skills: {}",
            requested, available
        )),
        SkillError::InvalidSkillFilePath { skill, file_path } => ToolError::InvalidArguments(
            format!("Invalid skill file path for {}: {}", skill, file_path),
        ),
        SkillError::SkillFileNotFound { skill, file_path } => ToolError::InvalidArguments(format!(
            "Skill file not found for {}: {}",
            skill, file_path
        )),
        SkillError::InvalidWriteTarget { path } => ToolError::InvalidArguments(format!(
            "Skill writes are limited to workspace .rocode/skills: {}",
            path.display()
        )),
        SkillError::SkillNotWritable { name, path } => ToolError::InvalidArguments(format!(
            "Skill {} is outside the workspace sandbox and cannot be modified here: {}",
            name,
            path.display()
        )),
        SkillError::InvalidSkillName { name } => {
            ToolError::InvalidArguments(format!("Invalid skill name: {}", name))
        }
        SkillError::InvalidSkillDescription { name } => {
            ToolError::InvalidArguments(format!("Invalid skill description for {}", name))
        }
        SkillError::InvalidSkillContent { message } => ToolError::InvalidArguments(message),
        SkillError::InvalidSkillCategory { category } => {
            ToolError::InvalidArguments(format!("Invalid skill category path: {}", category))
        }
        SkillError::InvalidSkillFrontmatter { message } => {
            ToolError::InvalidArguments(format!("Invalid skill frontmatter: {}", message))
        }
        SkillError::SkillAlreadyExists { name } => {
            ToolError::InvalidArguments(format!("Skill already exists: {}", name))
        }
        SkillError::GuardBlocked { report } => ToolError::InvalidArguments(format!(
            "Skill guard blocked {}: {}",
            report.skill_name,
            summarize_guard_report(&report)
        )),
        SkillError::SkillWriteSizeExceeded { path, size, limit } => {
            ToolError::InvalidArguments(format!(
                "Skill write exceeds size limit for {}: {} bytes > {} bytes",
                path, size, limit
            ))
        }
        SkillError::ArtifactFetchTimeout {
            locator,
            timeout_ms,
        } => ToolError::Timeout(format!(
            "Artifact fetch timed out for {} after {}ms",
            locator, timeout_ms
        )),
        SkillError::ArtifactDownloadSizeExceeded {
            locator,
            size,
            limit,
        } => ToolError::InvalidArguments(format!(
            "Artifact download size limit exceeded for {}: {} bytes > {} bytes",
            locator, size, limit
        )),
        SkillError::ArtifactExtractSizeExceeded { path, size, limit } => {
            ToolError::InvalidArguments(format!(
                "Artifact extract size limit exceeded for {}: {} bytes > {} bytes",
                path.display(),
                size,
                limit
            ))
        }
        SkillError::ArtifactChecksumMismatch { expected, actual } => {
            ToolError::InvalidArguments(format!(
                "Artifact checksum mismatch: expected sha256:{}, got sha256:{}",
                expected, actual
            ))
        }
        SkillError::ArtifactLayoutMismatch { path, message } => {
            ToolError::InvalidArguments(format!(
                "Artifact layout mismatch at {}: {}",
                path.display(),
                message
            ))
        }
        SkillError::ReadFailed { path, message } => {
            ToolError::ExecutionError(format!("Failed to read {}: {}", path.display(), message))
        }
        SkillError::WriteFailed { path, message } => {
            ToolError::ExecutionError(format!("Failed to write {}: {}", path.display(), message))
        }
        SkillError::CachePoisoned { resource } => {
            ToolError::ExecutionError(format!("Skill cache state is unavailable: {}", resource))
        }
    }
}

fn summarize_guard_report(report: &rocode_types::SkillGuardReport) -> String {
    report
        .violations
        .iter()
        .map(|violation| violation.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) fn authority_for(base: &Path, config_store: Option<Arc<ConfigStore>>) -> SkillAuthority {
    SkillAuthority::new(base.to_path_buf(), config_store)
}

pub(crate) fn governance_authority_for(
    base: &Path,
    config_store: Option<Arc<ConfigStore>>,
) -> SkillGovernanceAuthority {
    SkillGovernanceAuthority::new(base.to_path_buf(), config_store)
}

pub(crate) fn load_skill_with_runtime_materialization(
    authority: &SkillAuthority,
    _base_dir: &Path,
    _config_store: Option<Arc<ConfigStore>>,
    requested_name: &str,
    filter: Option<&SkillFilter<'_>>,
    _extra: Option<&Metadata>,
) -> Result<LoadedSkill, ToolError> {
    authority
        .load_skill(requested_name, filter)
        .map_err(map_skill_error)
}

pub(crate) fn resolve_skill_with_runtime_materialization(
    authority: &SkillAuthority,
    _base_dir: &Path,
    _config_store: Option<Arc<ConfigStore>>,
    requested_name: &str,
    filter: Option<&SkillFilter<'_>>,
    _extra: Option<&Metadata>,
) -> Result<rocode_skill::SkillMeta, ToolError> {
    authority
        .resolve_skill(requested_name, filter)
        .map_err(map_skill_error)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedSkillFilter {
    pub available_tools: Option<HashSet<String>>,
    pub available_toolsets: Option<HashSet<String>>,
    pub current_stage: Option<String>,
    pub category: Option<String>,
}

impl ResolvedSkillFilter {
    pub(crate) fn as_filter(&self) -> SkillFilter<'_> {
        SkillFilter {
            available_tools: self.available_tools.as_ref(),
            available_toolsets: self.available_toolsets.as_ref(),
            current_stage: self.current_stage.as_deref(),
            category: self.category.as_deref(),
        }
    }
}

pub(crate) async fn resolve_skill_filter(
    ctx: &ToolContext,
    category: Option<&str>,
) -> ResolvedSkillFilter {
    let available_tools = if let Some(tools) = metadata_string_set(&ctx.extra, "available_tool_ids")
    {
        Some(tools)
    } else if let Some(registry) = ctx.registry.as_ref() {
        Some(
            registry
                .list_ids()
                .await
                .into_iter()
                .map(|tool| tool.to_ascii_lowercase())
                .collect::<HashSet<_>>(),
        )
    } else {
        None
    };

    let available_toolsets = metadata_string_set(&ctx.extra, "available_toolsets").or_else(|| {
        available_tools
            .as_ref()
            .map(|tools| infer_toolsets_from_tools(tools.iter().map(String::as_str)))
    });

    ResolvedSkillFilter {
        available_tools,
        available_toolsets,
        current_stage: ctx
            .extra
            .get("scheduler_stage")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        category: category
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

pub(crate) fn load_skills_prompt_context(
    base_dir: &Path,
    config_store: Option<Arc<ConfigStore>>,
    requested_skills: Option<&[String]>,
    extra: Option<&Metadata>,
) -> Result<LoadedSkillsPromptContext, ToolError> {
    let Some(requested_skills) = requested_skills else {
        return Ok(LoadedSkillsPromptContext::default());
    };

    let authority = authority_for(base_dir, config_store.clone());
    let requested_skills = normalize_requested_skill_names(requested_skills);
    if requested_skills.is_empty() {
        return Ok(LoadedSkillsPromptContext::default());
    }

    let mut rendered = Vec::new();
    let mut loaded_skills = Vec::new();
    for requested_skill in requested_skills {
        let skill = load_skill_with_runtime_materialization(
            &authority,
            base_dir,
            config_store.clone(),
            &requested_skill,
            None,
            extra,
        )?;
        let (output, _) = format_loaded_skill_output(&skill, None, base_dir, None, None);
        rendered.push(output);
        loaded_skills.push(SkillMetaView::from(&skill.meta));
    }

    Ok(LoadedSkillsPromptContext {
        prompt_context: format!(
            "<loaded_skills>\n{}\n</loaded_skills>",
            rendered.join("\n\n")
        ),
        loaded_skills,
    })
}

pub(crate) fn format_loaded_skill_output(
    skill: &LoadedSkill,
    detail: Option<&SkillDetailView>,
    base_dir: &Path,
    arguments: Option<&serde_json::Value>,
    prompt: Option<&str>,
) -> (String, Metadata) {
    let detail = detail.cloned().unwrap_or_default();
    let linked_files = build_linked_files(&skill.meta.supporting_files);
    let usage_hint = (!linked_files.is_empty()).then_some(
        "To view linked files, call skill_view(name, file_path) where file_path is e.g. 'references/api.md' or 'assets/config.yaml'",
    );

    let mut output = format!("<skill_content name=\"{}\">\n\n", skill.meta.name);
    output.push_str(&format!("# Skill: {}\n\n", skill.meta.name));
    output.push_str(&skill.content);
    output.push_str("\n\n");
    output.push_str(&format!(
        "Base directory for this skill: {}\n",
        skill.meta.location.parent().unwrap_or(base_dir).display()
    ));
    output.push_str(
        "Relative paths in this skill (e.g., scripts/, references/) are relative to this base directory.\n",
    );
    if !skill.meta.supporting_files.is_empty() {
        output.push_str("Supporting files available via skill_view(name, file_path):\n\n");
        output.push_str("<skill_files>\n");
        for file in &skill.meta.supporting_files {
            output.push_str(&format!("<file>{}</file>\n", file.relative_path));
        }
        output.push_str("</skill_files>\n");
    }

    if let Some(args) = arguments {
        output.push_str(&format!(
            "\n**Arguments:**\n```json\n{}\n```\n",
            serde_json::to_string_pretty(args).unwrap_or_default()
        ));
    }

    if let Some(prompt) = prompt.filter(|value| !value.trim().is_empty()) {
        output.push_str(&format!("\n**Additional Instructions:**\n{}\n", prompt));
    }

    output.push_str("\n</skill_content>");

    let mut metadata = Metadata::new();
    metadata.insert("name".to_string(), serde_json::json!(&skill.meta.name));
    metadata.insert(
        "dir".to_string(),
        serde_json::json!(skill
            .meta
            .location
            .parent()
            .unwrap_or(base_dir)
            .to_string_lossy()
            .to_string()),
    );
    metadata.insert(
        "location".to_string(),
        serde_json::json!(skill.meta.location.to_string_lossy().to_string()),
    );
    metadata.insert(
        "description".to_string(),
        serde_json::json!(&skill.meta.description),
    );
    metadata.insert("version".to_string(), serde_json::json!(detail.version));
    metadata.insert("author".to_string(), serde_json::json!(detail.author));
    metadata.insert("license".to_string(), serde_json::json!(detail.license));
    metadata.insert("platforms".to_string(), serde_json::json!(detail.platforms));
    metadata.insert("tags".to_string(), serde_json::json!(detail.tags));
    metadata.insert(
        "related_skills".to_string(),
        serde_json::json!(detail.related_skills),
    );
    metadata.insert(
        "prerequisites".to_string(),
        serde_json::json!(detail.prerequisites),
    );
    metadata.insert("metadata".to_string(), serde_json::json!(detail.metadata));
    metadata.insert(
        "path".to_string(),
        serde_json::json!(skill.meta.location.to_string_lossy().to_string()),
    );
    metadata.insert(
        "supporting_files".to_string(),
        serde_json::json!(skill
            .meta
            .supporting_files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect::<Vec<_>>()),
    );
    metadata.insert(
        "linked_files".to_string(),
        serde_json::to_value(&linked_files).unwrap_or_else(|_| serde_json::json!({})),
    );
    metadata.insert("usage_hint".to_string(), serde_json::json!(usage_hint));
    metadata.insert(
        "required_environment_variables".to_string(),
        serde_json::json!(detail.required_environment_variables),
    );
    metadata.insert(
        "required_commands".to_string(),
        serde_json::json!(detail.required_commands),
    );
    metadata.insert(
        "missing_required_environment_variables".to_string(),
        serde_json::json!(detail.missing_required_environment_variables),
    );
    metadata.insert(
        "missing_required_commands".to_string(),
        serde_json::json!(detail.missing_required_commands),
    );
    metadata.insert(
        "setup_needed".to_string(),
        serde_json::json!(detail.setup_needed),
    );
    metadata.insert(
        "setup_skipped".to_string(),
        serde_json::json!(detail.setup_skipped),
    );
    metadata.insert(
        "readiness_status".to_string(),
        serde_json::json!(detail.readiness_status),
    );
    metadata.insert(
        "display.summary".to_string(),
        serde_json::json!(skill.meta.description.clone()),
    );

    (output, metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocode_skill::{
        LoadedSkill, SkillConditions, SkillDetailView, SkillFileRef, SkillHermesMetadata,
        SkillMeta, SkillMetadataBlocks, SkillPrerequisites, SkillReadinessStatus,
        SkillRequiredEnvironmentVariable, SkillRocodeMetadata,
    };
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn load_skill_with_runtime_materialization_does_not_create_missing_workspace_skill() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("harness/skills")).unwrap();
        std::fs::write(
            dir.path().join("harness/skills/evaluate_properties.md"),
            "# Evaluate\nUse ./tools/mol evaluate.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            r#"
## Skill References
- Target workspace skill: `drug-discovery-evaluate-properties` -- evaluate with wrapper
- Legacy reference source: `harness/skills/evaluate_properties.md` -> if `drug-discovery-evaluate-properties` does not exist, create it
"#,
        )
        .unwrap();

        let authority = authority_for(dir.path(), None);
        let error = load_skill_with_runtime_materialization(
            &authority,
            dir.path(),
            None,
            "drug-discovery-evaluate-properties",
            None,
            None,
        )
        .unwrap_err();

        assert!(
            matches!(error, ToolError::InvalidArguments(message) if message.contains("Unknown skill"))
        );
        assert!(!dir
            .path()
            .join(".rocode/skills/drug-discovery-evaluate-properties/SKILL.md")
            .exists());
    }

    #[test]
    fn format_loaded_skill_output_exposes_hermes_style_linked_file_metadata() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join("example");
        let skill = LoadedSkill {
            meta: SkillMeta {
                name: "example".to_string(),
                description: "Example skill".to_string(),
                category: Some("demo".to_string()),
                location: skill_dir.join("SKILL.md"),
                supporting_files: vec![
                    SkillFileRef {
                        relative_path: "references/api.md".to_string(),
                        location: skill_dir.join("references/api.md"),
                    },
                    SkillFileRef {
                        relative_path: "scripts/run.sh".to_string(),
                        location: skill_dir.join("scripts/run.sh"),
                    },
                ],
                conditions: SkillConditions::default(),
            },
            content: "# Example".to_string(),
        };
        let detail = SkillDetailView {
            version: Some("1.2.3".to_string()),
            author: Some("Example Author".to_string()),
            license: Some("MIT".to_string()),
            platforms: vec!["linux".to_string(), "macos".to_string()],
            tags: vec!["chemistry".to_string(), "design".to_string()],
            related_skills: vec!["molecule-report".to_string()],
            prerequisites: Some(SkillPrerequisites {
                env_vars: vec!["LEGACY_API_KEY".to_string()],
                commands: vec!["legacy-cli".to_string()],
            }),
            metadata: Some(SkillMetadataBlocks {
                hermes: Some(SkillHermesMetadata {
                    tags: vec!["chemistry".to_string(), "design".to_string()],
                    related_skills: vec!["molecule-report".to_string()],
                }),
                rocode: Some(SkillRocodeMetadata {
                    requires_tools: vec!["skill_manage".to_string()],
                    fallback_for_tools: Vec::new(),
                    requires_toolsets: Vec::new(),
                    fallback_for_toolsets: Vec::new(),
                    stage_filter: vec!["implementation".to_string()],
                }),
            }),
            required_environment_variables: vec![SkillRequiredEnvironmentVariable {
                name: "DEMO_API_KEY".to_string(),
                description: Some("Demo token".to_string()),
                prompt: None,
                help: None,
                required_for: None,
            }],
            required_commands: vec!["demo-cli".to_string()],
            missing_required_environment_variables: vec!["DEMO_API_KEY".to_string()],
            missing_required_commands: vec!["demo-cli".to_string()],
            setup_needed: true,
            setup_skipped: false,
            readiness_status: SkillReadinessStatus::SetupNeeded,
        };

        let (_, metadata) =
            format_loaded_skill_output(&skill, Some(&detail), dir.path(), None, None);
        assert_eq!(metadata.get("name"), Some(&serde_json::json!("example")));
        assert_eq!(metadata.get("version"), Some(&serde_json::json!("1.2.3")));
        assert_eq!(
            metadata.get("author"),
            Some(&serde_json::json!("Example Author"))
        );
        assert_eq!(metadata.get("license"), Some(&serde_json::json!("MIT")));
        assert_eq!(
            metadata.get("platforms"),
            Some(&serde_json::json!(["linux", "macos"]))
        );
        assert_eq!(
            metadata.get("tags"),
            Some(&serde_json::json!(["chemistry", "design"]))
        );
        assert_eq!(
            metadata.get("related_skills"),
            Some(&serde_json::json!(["molecule-report"]))
        );
        assert_eq!(
            metadata.get("prerequisites"),
            Some(&serde_json::json!({
                "env_vars": ["LEGACY_API_KEY"],
                "commands": ["legacy-cli"]
            }))
        );
        assert_eq!(
            metadata.get("metadata"),
            Some(&serde_json::json!({
                "hermes": {
                    "tags": ["chemistry", "design"],
                    "related_skills": ["molecule-report"]
                },
                "rocode": {
                    "requires_tools": ["skill_manage"],
                    "stage_filter": ["implementation"]
                }
            }))
        );
        assert_eq!(
            metadata.get("required_environment_variables"),
            Some(&serde_json::json!([{
                "name": "DEMO_API_KEY",
                "description": "Demo token"
            }]))
        );
        assert_eq!(
            metadata.get("required_commands"),
            Some(&serde_json::json!(["demo-cli"]))
        );
        assert_eq!(
            metadata.get("missing_required_environment_variables"),
            Some(&serde_json::json!(["DEMO_API_KEY"]))
        );
        assert_eq!(
            metadata.get("missing_required_commands"),
            Some(&serde_json::json!(["demo-cli"]))
        );
        assert_eq!(metadata.get("setup_needed"), Some(&serde_json::json!(true)));
        assert_eq!(
            metadata.get("setup_skipped"),
            Some(&serde_json::json!(false))
        );
        assert_eq!(
            metadata.get("readiness_status"),
            Some(&serde_json::json!("setup_needed"))
        );
        assert_eq!(
            metadata.get("path"),
            Some(&serde_json::json!(skill_dir
                .join("SKILL.md")
                .to_string_lossy()
                .to_string()))
        );
        assert_eq!(
            metadata.get("usage_hint"),
            Some(&serde_json::json!(
                "To view linked files, call skill_view(name, file_path) where file_path is e.g. 'references/api.md' or 'assets/config.yaml'"
            ))
        );
        let linked_files = metadata.get("linked_files").cloned().unwrap_or_default();
        assert_eq!(
            linked_files["references"],
            serde_json::json!(["references/api.md"])
        );
        assert_eq!(
            linked_files["scripts"],
            serde_json::json!(["scripts/run.sh"])
        );
    }

    #[test]
    fn format_loaded_skill_file_output_exposes_file_and_file_type_metadata() {
        let file = LoadedSkillFile {
            skill_name: "example".to_string(),
            file_path: "references/api.md".to_string(),
            location: PathBuf::from("/tmp/example/references/api.md"),
            content: "hello".to_string(),
        };

        let (_, metadata) = format_loaded_skill_file_output(&file);
        assert_eq!(metadata.get("name"), Some(&serde_json::json!("example")));
        assert_eq!(
            metadata.get("file"),
            Some(&serde_json::json!("references/api.md"))
        );
        assert_eq!(metadata.get("file_type"), Some(&serde_json::json!(".md")));
        assert_eq!(metadata.get("is_binary"), Some(&serde_json::json!(false)));
    }
}

pub(crate) fn format_loaded_skill_file_output(file: &LoadedSkillFile) -> (String, Metadata) {
    let output = format!(
        "<skill_file skill=\"{}\" path=\"{}\">\n\n{}\n\n</skill_file>",
        file.skill_name, file.file_path, file.content
    );

    let mut metadata = Metadata::new();
    metadata.insert("name".to_string(), serde_json::json!(&file.skill_name));
    metadata.insert("file".to_string(), serde_json::json!(&file.file_path));
    metadata.insert("file_path".to_string(), serde_json::json!(&file.file_path));
    metadata.insert(
        "location".to_string(),
        serde_json::json!(file.location.to_string_lossy().to_string()),
    );
    metadata.insert(
        "path".to_string(),
        serde_json::json!(file.location.to_string_lossy().to_string()),
    );
    metadata.insert(
        "file_type".to_string(),
        serde_json::json!(file_extension(&file.file_path)),
    );
    metadata.insert("is_binary".to_string(), serde_json::json!(false));
    metadata.insert(
        "display.summary".to_string(),
        serde_json::json!(format!("{} :: {}", file.skill_name, file.file_path)),
    );

    (output, metadata)
}

pub(crate) fn format_skill_list_output(skills: &[SkillMetaView]) -> String {
    if skills.is_empty() {
        return "<available_skills />".to_string();
    }

    let mut output = String::from("<available_skills>\n");
    for skill in skills {
        match skill.category.as_deref() {
            Some(category) if !category.is_empty() => {
                output.push_str(&format!(
                    "- [{}] {}: {}\n",
                    category, skill.name, skill.description
                ));
            }
            _ => {
                output.push_str(&format!("- {}: {}\n", skill.name, skill.description));
            }
        }
    }
    output.push_str("</available_skills>");
    output
}

pub(crate) fn format_skill_categories_output(categories: &[SkillCategoryView]) -> String {
    if categories.is_empty() {
        return "<skill_categories />".to_string();
    }

    let mut output = String::from("<skill_categories>\n");
    for category in categories {
        match category.description.as_deref() {
            Some(description) if !description.is_empty() => output.push_str(&format!(
                "- {} ({} skills): {}\n",
                category.name, category.skill_count, description
            )),
            _ => output.push_str(&format!(
                "- {} ({} skills)\n",
                category.name, category.skill_count
            )),
        }
    }
    output.push_str("</skill_categories>");
    output
}

pub(crate) fn collect_skill_categories(skills: &[SkillMetaView]) -> Vec<String> {
    skills
        .iter()
        .filter_map(|skill| skill.category.clone())
        .filter(|category| !category.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn build_linked_files(
    supporting_files: &[rocode_skill::SkillFileRef],
) -> BTreeMap<&'static str, Vec<String>> {
    let mut linked_files = BTreeMap::<&'static str, Vec<String>>::new();
    for file in supporting_files {
        let bucket = if file.relative_path.starts_with("references/") {
            "references"
        } else if file.relative_path.starts_with("templates/") {
            "templates"
        } else if file.relative_path.starts_with("assets/") {
            "assets"
        } else if file.relative_path.starts_with("scripts/") {
            "scripts"
        } else {
            "other"
        };
        linked_files
            .entry(bucket)
            .or_default()
            .push(file.relative_path.clone());
    }

    linked_files
}

fn file_extension(file_path: &str) -> String {
    std::path::Path::new(file_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value))
        .unwrap_or_default()
}

fn normalize_requested_skill_names(raw_names: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for raw_name in raw_names {
        let trimmed = raw_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if normalized
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(trimmed))
        {
            continue;
        }
        normalized.push(trimmed.to_string());
    }
    normalized
}

fn metadata_string_set(metadata: &Metadata, key: &str) -> Option<HashSet<String>> {
    let values = metadata.get(key)?.as_array()?;
    Some(
        values
            .iter()
            .filter_map(|value| value.as_str())
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect(),
    )
}
