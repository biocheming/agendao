use crate::write::{
    atomic_write_string, build_skill_document, delete_file, ensure_workspace_skill_markdown,
    load_skill_document, parse_skill_frontmatter, prune_empty_skill_parent_dirs,
    resolve_create_skill_markdown_path, supporting_file_path, validate_skill_body,
    validate_skill_description, validate_skill_markdown_size, validate_skill_name,
    validate_supporting_file_size, CreateSkillRequest,
};
use crate::{
    SkillAuthority, SkillError, SkillFileRef, SkillFrontmatter, SkillHermesMetadata, SkillMeta,
    SkillMetadataBlocks, SkillPrerequisites, SkillRequiredEnvironmentVariable, SkillAgendaoMetadata,
};
use agendao_types::{
    WorkspaceSkillArtifactBundle, WorkspaceSkillArtifactEntry, WorkspaceSkillArtifactFile,
    WorkspaceSkillArtifactFrontmatter, WorkspaceSkillArtifactHermesMetadata,
    WorkspaceSkillArtifactImportEnvelope, WorkspaceSkillArtifactLegacyPayload,
    WorkspaceSkillArtifactMetadataBlocks, WorkspaceSkillArtifactPrerequisites,
    WorkspaceSkillArtifactRequiredEnvironmentVariable, WorkspaceSkillArtifactAgendaoMetadata,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub trait WorkspaceSkillArtifactLegacyAdapter {
    fn legacy_format(&self) -> &'static str;

    fn import_entries(
        &self,
        payload: &WorkspaceSkillArtifactLegacyPayload,
    ) -> Result<Vec<WorkspaceSkillArtifactEntry>, SkillError>;
}

pub fn export_workspace_skill_artifact_bundle(
    authority: &SkillAuthority,
) -> Result<WorkspaceSkillArtifactBundle, SkillError> {
    let mut skills = authority
        .discover_skills()
        .into_iter()
        .filter(|meta| authority.is_skill_meta_writable(meta))
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });

    let entries = skills
        .iter()
        .map(|meta| export_workspace_skill_entry(meta))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkspaceSkillArtifactBundle::new_now(entries))
}

pub fn import_workspace_skill_artifact_bundle(
    authority: &SkillAuthority,
    payload: WorkspaceSkillArtifactImportEnvelope,
) -> Result<usize, SkillError> {
    import_workspace_skill_artifact_bundle_with_legacy_adapter(authority, payload, None)
}

pub fn import_workspace_skill_artifact_bundle_with_legacy_adapter(
    authority: &SkillAuthority,
    payload: WorkspaceSkillArtifactImportEnvelope,
    legacy_adapter: Option<&dyn WorkspaceSkillArtifactLegacyAdapter>,
) -> Result<usize, SkillError> {
    let entries = resolve_entries_from_artifact(payload, legacy_adapter)?;
    validate_workspace_skill_entries(&entries)?;

    for entry in &entries {
        import_workspace_skill_entry(authority, entry)?;
    }
    authority.refresh_after_mutation()?;
    Ok(entries.len())
}

fn export_workspace_skill_entry(
    meta: &SkillMeta,
) -> Result<WorkspaceSkillArtifactEntry, SkillError> {
    let document = load_skill_document(&meta.location)?;
    let frontmatter = artifact_frontmatter_from_skill(parse_skill_frontmatter(&document)?);

    let mut supporting_files = meta
        .supporting_files
        .iter()
        .map(export_workspace_supporting_file)
        .collect::<Result<Vec<_>, _>>()?;
    supporting_files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(WorkspaceSkillArtifactEntry {
        frontmatter,
        body: document.body,
        supporting_files,
    })
}

fn export_workspace_supporting_file(
    file: &SkillFileRef,
) -> Result<WorkspaceSkillArtifactFile, SkillError> {
    let content = fs::read_to_string(&file.location).map_err(|error| SkillError::ReadFailed {
        path: file.location.clone(),
        message: error.to_string(),
    })?;

    Ok(WorkspaceSkillArtifactFile {
        relative_path: file.relative_path.clone(),
        content,
    })
}

fn resolve_entries_from_artifact(
    payload: WorkspaceSkillArtifactImportEnvelope,
    legacy_adapter: Option<&dyn WorkspaceSkillArtifactLegacyAdapter>,
) -> Result<Vec<WorkspaceSkillArtifactEntry>, SkillError> {
    match payload {
        WorkspaceSkillArtifactImportEnvelope::Bundle(bundle) => Ok(bundle.skills),
        WorkspaceSkillArtifactImportEnvelope::Legacy(legacy) => match legacy_adapter {
            Some(adapter) if adapter.legacy_format() == legacy.legacy_format => {
                adapter.import_entries(&legacy)
            }
            _ => Err(SkillError::InvalidSkillContent {
                message: format!(
                    "Unsupported legacy workspace skill artifact format: {} (explicit legacy adapter required)",
                    legacy.legacy_format
                ),
            }),
        },
    }
}

fn validate_workspace_skill_entries(
    entries: &[WorkspaceSkillArtifactEntry],
) -> Result<(), SkillError> {
    let mut names = HashSet::new();
    for entry in entries {
        let name = validate_skill_name(&entry.frontmatter.name)?.to_ascii_lowercase();
        if !names.insert(name.clone()) {
            return Err(SkillError::InvalidSkillContent {
                message: format!("Duplicate skill name in workspace skill artifact: {name}"),
            });
        }

        validate_skill_description(&entry.frontmatter.name, &entry.frontmatter.description)?;
        validate_skill_body(&entry.body)?;

        let mut file_paths = HashSet::new();
        for file in &entry.supporting_files {
            let path = file.relative_path.trim();
            if path.is_empty() {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "Supporting file path must not be empty for skill `{}`",
                        entry.frontmatter.name
                    ),
                });
            }
            if !file_paths.insert(path.to_ascii_lowercase()) {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "Duplicate supporting file path in workspace skill artifact for `{}`: {}",
                        entry.frontmatter.name, file.relative_path
                    ),
                });
            }
        }
    }
    Ok(())
}

fn import_workspace_skill_entry(
    authority: &SkillAuthority,
    entry: &WorkspaceSkillArtifactEntry,
) -> Result<(), SkillError> {
    let name = validate_skill_name(&entry.frontmatter.name)?;
    let description = validate_skill_description(&name, &entry.frontmatter.description)?;
    let body = validate_skill_body(&entry.body)?;
    let mut frontmatter = skill_frontmatter_from_artifact(&entry.frontmatter);
    frontmatter.name = name.clone();
    frontmatter.description = description.clone();

    let existing = match authority.resolve_skill_for_inspection(&name, None) {
        Ok(meta) => Some(meta),
        Err(SkillError::UnknownSkill { .. }) => None,
        Err(error) => return Err(error),
    };

    let target = match existing.as_ref() {
        Some(meta) => {
            ensure_workspace_skill_markdown(authority.base_dir(), &name, &meta.location)?;
            meta.location.clone()
        }
        None => {
            let target = resolve_create_skill_markdown_path(
                authority.base_dir(),
                &CreateSkillRequest {
                    name: name.clone(),
                    description: description.clone(),
                    body: body.clone(),
                    frontmatter: None,
                    category: None,
                    directory_name: None,
                },
            )?;
            if target.exists() {
                return Err(SkillError::InvalidWriteTarget { path: target });
            }
            target
        }
    };

    let content = build_skill_document(&frontmatter, &body)?;
    validate_skill_markdown_size(&content, &target.to_string_lossy())?;
    atomic_write_string(&target, &content)?;

    let stop_at = target
        .parent()
        .ok_or_else(|| SkillError::InvalidWriteTarget {
            path: target.clone(),
        })?;
    synchronize_supporting_files(
        &name,
        &target,
        existing
            .as_ref()
            .map(|meta| meta.supporting_files.as_slice())
            .unwrap_or(&[]),
        &entry.supporting_files,
        stop_at,
    )?;

    Ok(())
}

fn synchronize_supporting_files(
    skill_name: &str,
    skill_markdown: &Path,
    existing_files: &[SkillFileRef],
    desired_files: &[WorkspaceSkillArtifactFile],
    stop_at: &Path,
) -> Result<(), SkillError> {
    let desired_paths = desired_files
        .iter()
        .map(|file| file.relative_path.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    for existing in existing_files {
        if desired_paths.contains(&existing.relative_path.to_ascii_lowercase()) {
            continue;
        }
        delete_file(&existing.location, skill_name, &existing.relative_path)?;
        prune_empty_skill_parent_dirs(&existing.location, stop_at);
    }

    let mut desired = desired_files.to_vec();
    desired.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    for file in &desired {
        let path = supporting_file_path(skill_markdown, &file.relative_path)
            .map_err(|error| map_supporting_file_error(skill_name, error))?;
        validate_supporting_file_size(&file.relative_path, &file.content)?;
        atomic_write_string(&path, &file.content)?;
    }

    Ok(())
}

fn map_supporting_file_error(skill_name: &str, error: SkillError) -> SkillError {
    match error {
        SkillError::InvalidSkillFilePath { file_path, .. } => SkillError::InvalidSkillFilePath {
            skill: skill_name.to_string(),
            file_path,
        },
        other => other,
    }
}

fn artifact_frontmatter_from_skill(
    frontmatter: SkillFrontmatter,
) -> WorkspaceSkillArtifactFrontmatter {
    WorkspaceSkillArtifactFrontmatter {
        name: frontmatter.name,
        description: frontmatter.description,
        version: frontmatter.version,
        author: frontmatter.author,
        license: frontmatter.license,
        platforms: frontmatter.platforms,
        tags: frontmatter.tags,
        related_skills: frontmatter.related_skills,
        prerequisites: frontmatter
            .prerequisites
            .map(|value| WorkspaceSkillArtifactPrerequisites {
                env_vars: value.env_vars,
                commands: value.commands,
            }),
        required_environment_variables: frontmatter
            .required_environment_variables
            .into_iter()
            .map(|value| WorkspaceSkillArtifactRequiredEnvironmentVariable {
                name: value.name,
                description: value.description,
                prompt: value.prompt,
                help: value.help,
                required_for: value.required_for,
            })
            .collect(),
        required_commands: frontmatter.required_commands,
        metadata: frontmatter
            .metadata
            .map(|value| WorkspaceSkillArtifactMetadataBlocks {
                hermes: value
                    .hermes
                    .map(|hermes| WorkspaceSkillArtifactHermesMetadata {
                        tags: hermes.tags,
                        related_skills: hermes.related_skills,
                    }),
                agendao: value
                    .agendao
                    .map(|agendao| WorkspaceSkillArtifactAgendaoMetadata {
                        requires_tools: agendao.requires_tools,
                        fallback_for_tools: agendao.fallback_for_tools,
                        requires_toolsets: agendao.requires_toolsets,
                        fallback_for_toolsets: agendao.fallback_for_toolsets,
                        stage_filter: agendao.stage_filter,
                    }),
            }),
    }
}

fn skill_frontmatter_from_artifact(
    frontmatter: &WorkspaceSkillArtifactFrontmatter,
) -> SkillFrontmatter {
    SkillFrontmatter {
        name: frontmatter.name.clone(),
        description: frontmatter.description.clone(),
        version: frontmatter.version.clone(),
        author: frontmatter.author.clone(),
        license: frontmatter.license.clone(),
        platforms: frontmatter.platforms.clone(),
        tags: frontmatter.tags.clone(),
        related_skills: frontmatter.related_skills.clone(),
        prerequisites: frontmatter
            .prerequisites
            .as_ref()
            .map(|value| SkillPrerequisites {
                env_vars: value.env_vars.clone(),
                commands: value.commands.clone(),
            }),
        required_environment_variables: frontmatter
            .required_environment_variables
            .iter()
            .map(|value| SkillRequiredEnvironmentVariable {
                name: value.name.clone(),
                description: value.description.clone(),
                prompt: value.prompt.clone(),
                help: value.help.clone(),
                required_for: value.required_for.clone(),
            })
            .collect(),
        required_commands: frontmatter.required_commands.clone(),
        metadata: frontmatter
            .metadata
            .as_ref()
            .map(|value| SkillMetadataBlocks {
                hermes: value.hermes.as_ref().map(|hermes| SkillHermesMetadata {
                    tags: hermes.tags.clone(),
                    related_skills: hermes.related_skills.clone(),
                }),
                agendao: value.agendao.as_ref().map(|agendao| SkillAgendaoMetadata {
                    requires_tools: agendao.requires_tools.clone(),
                    fallback_for_tools: agendao.fallback_for_tools.clone(),
                    requires_toolsets: agendao.requires_toolsets.clone(),
                    fallback_for_toolsets: agendao.fallback_for_toolsets.clone(),
                    stage_filter: agendao.stage_filter.clone(),
                }),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        export_workspace_skill_artifact_bundle, import_workspace_skill_artifact_bundle,
        import_workspace_skill_artifact_bundle_with_legacy_adapter,
        WorkspaceSkillArtifactLegacyAdapter,
    };
    use crate::{SkillAuthority, SkillError};
    use agendao_types::{
        WorkspaceSkillArtifactBundle, WorkspaceSkillArtifactEntry, WorkspaceSkillArtifactFile,
        WorkspaceSkillArtifactFrontmatter, WorkspaceSkillArtifactImportEnvelope,
        WorkspaceSkillArtifactLegacyPayload,
    };
    use std::fs;
    use tempfile::tempdir;

    struct AlphaLegacyAdapter;

    impl WorkspaceSkillArtifactLegacyAdapter for AlphaLegacyAdapter {
        fn legacy_format(&self) -> &'static str {
            "workspace-skill-alpha"
        }

        fn import_entries(
            &self,
            payload: &WorkspaceSkillArtifactLegacyPayload,
        ) -> Result<Vec<WorkspaceSkillArtifactEntry>, SkillError> {
            #[derive(serde::Deserialize)]
            struct LegacySkill {
                name: String,
                description: String,
                body: String,
            }

            #[derive(serde::Deserialize)]
            struct LegacyPayload {
                skills: Vec<LegacySkill>,
            }

            let raw = payload
                .payload
                .clone()
                .ok_or_else(|| SkillError::InvalidSkillContent {
                    message: "legacy workspace skill payload body missing".to_string(),
                })?;
            let parsed: LegacyPayload =
                serde_json::from_value(raw).map_err(|error| SkillError::InvalidSkillContent {
                    message: error.to_string(),
                })?;
            Ok(parsed
                .skills
                .into_iter()
                .map(|skill| WorkspaceSkillArtifactEntry {
                    frontmatter: WorkspaceSkillArtifactFrontmatter {
                        name: skill.name,
                        description: skill.description,
                        ..WorkspaceSkillArtifactFrontmatter::default()
                    },
                    body: skill.body,
                    supporting_files: Vec::new(),
                })
                .collect())
        }
    }

    fn write_skill(
        root: &std::path::Path,
        relative_dir: &str,
        name: &str,
        description: &str,
        body: &str,
        supporting_files: &[(&str, &str)],
    ) {
        let skill_dir = root.join(relative_dir);
        fs::create_dir_all(&skill_dir).expect("skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
        )
        .expect("skill markdown");
        for (relative_path, content) in supporting_files {
            let path = skill_dir.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("supporting parent");
            }
            fs::write(path, content).expect("supporting file");
        }
    }

    #[test]
    fn export_workspace_skill_bundle_includes_only_workspace_writable_skills() {
        let dir = tempdir().expect("tempdir");
        write_skill(
            &dir.path().join(".agendao/skills"),
            "reviewer",
            "reviewer",
            "Workspace reviewer",
            "# Reviewer",
            &[("templates/checklist.md", "- scope\n")],
        );
        write_skill(
            &dir.path().join(".agents/skills"),
            "global-reviewer",
            "global-reviewer",
            "Global reviewer",
            "# Global",
            &[],
        );

        let authority = SkillAuthority::new(dir.path(), None);
        let bundle = export_workspace_skill_artifact_bundle(&authority).expect("export");
        let json = serde_json::to_string(&bundle).expect("serialize");

        assert_eq!(bundle.skills.len(), 1);
        assert_eq!(bundle.skills[0].frontmatter.name, "reviewer");
        assert!(json.contains("templates/checklist.md"));
        assert!(!json.contains("global-reviewer"));
        assert!(!json.contains(&dir.path().to_string_lossy().to_string()));
    }

    #[test]
    fn import_workspace_skill_bundle_creates_workspace_skill_and_supporting_files() {
        let dir = tempdir().expect("tempdir");
        let authority = SkillAuthority::new(dir.path(), None);
        let bundle = WorkspaceSkillArtifactBundle::new(
            123,
            vec![WorkspaceSkillArtifactEntry {
                frontmatter: WorkspaceSkillArtifactFrontmatter {
                    name: "reviewer".to_string(),
                    description: "Review code changes".to_string(),
                    version: Some("1.0.0".to_string()),
                    ..WorkspaceSkillArtifactFrontmatter::default()
                },
                body: "# Reviewer\n\nInspect patches carefully.".to_string(),
                supporting_files: vec![WorkspaceSkillArtifactFile {
                    relative_path: "templates/checklist.md".to_string(),
                    content: "- scope\n- tests\n".to_string(),
                }],
            }],
        );

        let imported = import_workspace_skill_artifact_bundle(
            &authority,
            WorkspaceSkillArtifactImportEnvelope::Bundle(bundle),
        )
        .expect("import");

        assert_eq!(imported, 1);
        let loaded = authority
            .load_skill_for_inspection("reviewer", None)
            .expect("skill should load");
        assert_eq!(loaded.meta.name, "reviewer");
        assert!(loaded.content.contains("Inspect patches carefully."));

        let supporting = authority
            .load_skill_file_for_inspection("reviewer", "templates/checklist.md")
            .expect("supporting file should load");
        assert_eq!(supporting.content, "- scope\n- tests\n");
    }

    #[test]
    fn workspace_skill_artifact_roundtrips_through_parse_import_and_re_export() {
        let source = tempdir().expect("tempdir");
        write_skill(
            &source.path().join(".agendao/skills"),
            "reviewer",
            "reviewer",
            "Review code changes",
            "# Reviewer\n\nInspect patches carefully.",
            &[("templates/checklist.md", "- scope\n- tests\n")],
        );
        let source_authority = SkillAuthority::new(source.path(), None);
        let exported = export_workspace_skill_artifact_bundle(&source_authority).expect("export");
        let payload = serde_json::to_string(&exported).expect("serialize");
        let parsed: WorkspaceSkillArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("parse");

        let target = tempdir().expect("tempdir");
        let target_authority = SkillAuthority::new(target.path(), None);
        import_workspace_skill_artifact_bundle(&target_authority, parsed).expect("import");
        let replayed =
            export_workspace_skill_artifact_bundle(&target_authority).expect("re-export");

        assert_eq!(replayed.version, exported.version);
        assert_eq!(replayed.skills, exported.skills);
    }

    #[test]
    fn import_workspace_skill_bundle_rejects_duplicate_skill_names() {
        let dir = tempdir().expect("tempdir");
        let authority = SkillAuthority::new(dir.path(), None);
        let bundle = WorkspaceSkillArtifactBundle::new(
            123,
            vec![
                WorkspaceSkillArtifactEntry {
                    frontmatter: WorkspaceSkillArtifactFrontmatter {
                        name: "reviewer".to_string(),
                        description: "one".to_string(),
                        ..WorkspaceSkillArtifactFrontmatter::default()
                    },
                    body: "# One".to_string(),
                    supporting_files: Vec::new(),
                },
                WorkspaceSkillArtifactEntry {
                    frontmatter: WorkspaceSkillArtifactFrontmatter {
                        name: "Reviewer".to_string(),
                        description: "two".to_string(),
                        ..WorkspaceSkillArtifactFrontmatter::default()
                    },
                    body: "# Two".to_string(),
                    supporting_files: Vec::new(),
                },
            ],
        );

        let error = import_workspace_skill_artifact_bundle(
            &authority,
            WorkspaceSkillArtifactImportEnvelope::Bundle(bundle),
        )
        .expect_err("duplicate names should fail");
        assert!(matches!(error, SkillError::InvalidSkillContent { .. }));
        assert!(error.to_string().contains("Duplicate skill name"));
    }

    #[test]
    fn import_workspace_skill_bundle_rejects_legacy_payload_without_explicit_adapter() {
        let dir = tempdir().expect("tempdir");
        let authority = SkillAuthority::new(dir.path(), None);
        let envelope =
            WorkspaceSkillArtifactImportEnvelope::Legacy(WorkspaceSkillArtifactLegacyPayload {
                legacy_format: "workspace-skill-alpha".to_string(),
                payload: Some(serde_json::json!({"skills": []})),
            });

        let error = import_workspace_skill_artifact_bundle(&authority, envelope)
            .expect_err("legacy payload should fail closed");
        assert!(error
            .to_string()
            .contains("Unsupported legacy workspace skill artifact format"));
    }

    #[test]
    fn import_workspace_skill_bundle_accepts_matching_explicit_legacy_adapter() {
        let dir = tempdir().expect("tempdir");
        let authority = SkillAuthority::new(dir.path(), None);
        let envelope =
            WorkspaceSkillArtifactImportEnvelope::Legacy(WorkspaceSkillArtifactLegacyPayload {
                legacy_format: "workspace-skill-alpha".to_string(),
                payload: Some(serde_json::json!({
                    "skills": [{
                        "name": "legacy-reviewer",
                        "description": "Legacy reviewer",
                        "body": "# Legacy"
                    }]
                })),
            });

        let imported = import_workspace_skill_artifact_bundle_with_legacy_adapter(
            &authority,
            envelope,
            Some(&AlphaLegacyAdapter),
        )
        .expect("legacy adapter should import");
        assert_eq!(imported, 1);

        let loaded = authority
            .load_skill_for_inspection("legacy-reviewer", None)
            .expect("imported skill should load");
        assert_eq!(loaded.meta.name, "legacy-reviewer");
        assert!(loaded.content.contains("# Legacy"));
    }
}
