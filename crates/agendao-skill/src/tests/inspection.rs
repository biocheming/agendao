use super::*;

#[test]
fn load_skill_reads_frontmatter_and_body() {
    let dir = tempdir().unwrap();
    let skill_path = dir.path().join(".agendao/skills/reviewer/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: reviewer
description: "Review code changes"
---

# Reviewer

Do a thorough review.
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(dir.path(), None);
    let parsed = authority
        .load_skill_for_inspection("reviewer", None)
        .unwrap();
    assert_eq!(parsed.meta.name, "reviewer");
    assert_eq!(parsed.meta.description, "Review code changes");
    assert!(parsed.content.contains("Do a thorough review."));
}

#[test]
fn authority_load_skill_detail_returns_formal_detail_view() {
    let dir = tempdir().unwrap();
    let skill_path = dir.path().join(".agendao/skills/reviewer/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: reviewer
description: "Review code changes"
required_commands: [cargo]
metadata:
  hermes:
    tags: [review, rust]
    related_skills: [formatter]
---

# Reviewer
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(dir.path(), None);
    let detail = authority
        .load_skill_detail_for_inspection("reviewer", None)
        .unwrap();
    assert_eq!(detail.tags, vec!["review", "rust"]);
    assert_eq!(detail.related_skills, vec!["formatter"]);
    assert_eq!(detail.required_commands, vec!["cargo"]);
}

#[test]
fn list_skill_categories_aggregates_counts_and_descriptions() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".agendao/skills/chemistry")).unwrap();
    fs::write(
        dir.path().join(".agendao/skills/chemistry/DESCRIPTION.md"),
        r#"---
description: Chemistry workflows and domain-specific guidance
---

# Chemistry
"#,
    )
    .unwrap();
    write_directory_skill(
        &dir.path().join(".agendao/skills"),
        "chemistry/analyze",
        "analyze",
        "Analyze compounds",
        "Analyze",
        &[],
    );
    write_directory_skill(
        &dir.path().join(".agendao/skills"),
        "chemistry/design",
        "design",
        "Design compounds",
        "Design",
        &[],
    );
    write_directory_skill(
        &dir.path().join(".agendao/skills"),
        "utilities/report",
        "report",
        "Write reports",
        "Report",
        &[],
    );

    let authority = SkillAuthority::new(dir.path(), None);
    let categories = authority.list_skill_categories(None).unwrap();

    let chemistry = categories
        .iter()
        .find(|category| category.name == "chemistry")
        .expect("chemistry category should exist");
    assert_eq!(chemistry.skill_count, 2);
    assert_eq!(
        chemistry.description.as_deref(),
        Some("Chemistry workflows and domain-specific guidance")
    );
    let utilities = categories
        .iter()
        .find(|category| category.name == "utilities")
        .expect("utilities category should exist");
    assert_eq!(utilities.skill_count, 1);
    assert!(utilities.description.is_none());
}

#[test]
fn load_skill_parses_agendao_conditions_and_applies_filters() {
    let dir = tempdir().unwrap();
    let skill_path = dir.path().join(".agendao/skills/reviewer/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: reviewer
description: "Review code changes"
metadata:
  agendao:
    requires_tools: [grep]
    requires_toolsets:
      - search
    stage_filter:
      - review
---

Only review here.
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(dir.path(), None);
    let loaded = authority
        .load_skill_for_inspection("reviewer", None)
        .unwrap();
    assert_eq!(
        loaded.meta.conditions.requires_tools,
        vec!["grep".to_string()]
    );
    assert_eq!(
        loaded.meta.conditions.requires_toolsets,
        vec!["search".to_string()]
    );
    assert_eq!(
        loaded.meta.conditions.stage_filter,
        vec!["review".to_string()]
    );

    let available_tools = std::collections::HashSet::from(["grep".to_string()]);
    let available_toolsets = std::collections::HashSet::from(["search".to_string()]);
    let filtered = authority
        .list_skill_meta(Some(&SkillFilter {
            available_tools: Some(&available_tools),
            available_toolsets: Some(&available_toolsets),
            current_stage: Some("review"),
            category: None,
        }))
        .unwrap();
    assert!(filtered.iter().any(|skill| skill.name == "reviewer"));

    let hidden = authority
        .list_skill_meta(Some(&SkillFilter {
            available_tools: Some(&available_tools),
            available_toolsets: Some(&available_toolsets),
            current_stage: Some("execution"),
            category: None,
        }))
        .unwrap();
    assert!(!hidden.iter().any(|skill| skill.name == "reviewer"));
}

#[test]
fn discover_skills_loads_default_and_configured_skill_paths() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let agendao_skill = root.join(".agendao/skills/local/SKILL.md");
    fs::create_dir_all(agendao_skill.parent().unwrap()).unwrap();
    fs::write(
        &agendao_skill,
        r#"---
name: local-skill
description: local
---
project content
"#,
    )
    .unwrap();

    let ignored_foreign_skill = root.join(".foreign/skills/foreign/SKILL.md");
    fs::create_dir_all(ignored_foreign_skill.parent().unwrap()).unwrap();
    fs::write(
        &ignored_foreign_skill,
        r#"---
name: foreign-skill
description: foreign
---
foreign content
"#,
    )
    .unwrap();

    let extra_root = root.join("custom-skills");
    let extra_skill = extra_root.join("remote/SKILL.md");
    fs::create_dir_all(extra_skill.parent().unwrap()).unwrap();
    fs::write(
        &extra_skill,
        r#"---
name: custom-skill
description: custom
---
custom content
"#,
    )
    .unwrap();

    let mut config = Config::default();
    config
        .skill_paths
        .insert("custom".to_string(), "custom-skills".to_string());
    let authority = SkillAuthority::new(root, Some(Arc::new(ConfigStore::new(config))));
    let discovered = authority.list_skill_meta(None).unwrap();
    let names: Vec<String> = discovered.into_iter().map(|s| s.name).collect();

    assert!(names.contains(&"local-skill".to_string()));
    assert!(names.contains(&"custom-skill".to_string()));
    assert!(!names.contains(&"foreign-skill".to_string()));
}

#[test]
fn render_loaded_skills_context_for_inspection_resolves_and_renders_requested_skills() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();

    fs::write(
        &skill_path,
        r#"---
name: agendao-test-review-skill
description: review
---
Check correctness first.
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(root, None);
    let (context, loaded) = authority
        .render_loaded_skills_context_for_inspection(&[
            "agendao-test-review-skill".to_string(),
            "AGENDAO-TEST-REVIEW-SKILL".to_string(),
        ])
        .unwrap();
    assert_eq!(loaded, vec!["agendao-test-review-skill".to_string()]);
    assert!(context.contains("<loaded_skills>"));
    assert!(context.contains("Check correctness first."));
}

#[test]
fn load_skill_file_rejects_missing_or_escaping_paths() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();

    fs::write(
        &skill_path,
        r#"---
name: review-skill
description: review
---
Check correctness first.
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(root, None);
    let err = authority
        .load_skill_file_for_inspection("review-skill", "../outside.md")
        .unwrap_err();
    assert!(err.to_string().contains("invalid skill file path"));
}

#[test]
fn refresh_persists_skill_catalog_snapshot_to_disk() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: review-snapshot
description: review
---
Snapshot me.
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(root, None);
    let snapshot = authority.refresh().unwrap();
    let cache_path = snapshot_path(root);

    assert!(cache_path.exists());
    let persisted: StoredSkillCatalogSnapshot =
        serde_json::from_str(&fs::read_to_string(cache_path).unwrap()).unwrap();
    assert_eq!(persisted.schema, SKILL_CATALOG_SNAPSHOT_SCHEMA);
    assert_eq!(persisted.version, SKILL_CATALOG_SNAPSHOT_VERSION);
    assert!(persisted
        .snapshot
        .skills
        .iter()
        .any(|skill| skill.name == "review-snapshot"));
    assert!(snapshot
        .skills
        .iter()
        .any(|skill| skill.name == "review-snapshot"));
}

#[test]
fn load_skill_cache_reloads_when_file_changes() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: review-cache
description: review
---
First body.
"#,
    )
    .unwrap();

    let authority = SkillAuthority::new(root, None);
    let first = authority
        .load_skill_for_inspection("review-cache", None)
        .unwrap();
    assert!(first.content.contains("First body."));

    fs::write(
        &skill_path,
        r#"---
name: review-cache
description: review
---
Second body.
"#,
    )
    .unwrap();

    let second = authority
        .load_skill_for_inspection("review-cache", None)
        .unwrap();
    assert!(second.content.contains("Second body."));
}

#[test]
fn config_store_revision_invalidates_skill_roots() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let skill_a = root.join("skills-a/alpha/SKILL.md");
    fs::create_dir_all(skill_a.parent().unwrap()).unwrap();
    fs::write(
        &skill_a,
        r#"---
name: alpha-skill
description: alpha
---
Alpha.
"#,
    )
    .unwrap();

    let skill_b = root.join("skills-b/beta/SKILL.md");
    fs::create_dir_all(skill_b.parent().unwrap()).unwrap();
    fs::write(
        &skill_b,
        r#"---
name: beta-skill
description: beta
---
Beta.
"#,
    )
    .unwrap();

    let mut config = Config::default();
    config
        .skill_paths
        .insert("custom".to_string(), "skills-a".to_string());
    let store = Arc::new(ConfigStore::new(config));
    let authority = SkillAuthority::new(root, Some(store.clone()));

    let first = authority.list_skill_meta(None).unwrap();
    assert!(first.iter().any(|skill| skill.name == "alpha-skill"));
    assert!(!first.iter().any(|skill| skill.name == "beta-skill"));

    store
        .replace_with(|config| {
            config
                .skill_paths
                .insert("custom".to_string(), "skills-b".to_string());
            Ok(())
        })
        .unwrap();

    let second = authority.list_skill_meta(None).unwrap();
    assert!(!second.iter().any(|skill| skill.name == "alpha-skill"));
    assert!(second.iter().any(|skill| skill.name == "beta-skill"));
}

#[test]
fn corrupted_snapshot_falls_back_to_rebuild() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: fallback-skill
description: fallback
---
Fallback.
"#,
    )
    .unwrap();

    let snapshot_file = snapshot_path(root);
    fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();
    fs::write(&snapshot_file, "{ definitely-not-json").unwrap();

    let authority = SkillAuthority::new(root, None);
    let skills = authority.list_skill_meta(None).unwrap();
    assert!(skills.iter().any(|skill| skill.name == "fallback-skill"));

    let repaired: StoredSkillCatalogSnapshot =
        serde_json::from_str(&fs::read_to_string(snapshot_file).unwrap()).unwrap();
    assert_eq!(repaired.version, SKILL_CATALOG_SNAPSHOT_VERSION);
}

#[test]
fn unsupported_snapshot_version_falls_back_to_rebuild() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: versioned-skill
description: fallback
---
Version fallback.
"#,
    )
    .unwrap();

    let snapshot_file = snapshot_path(root);
    fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();
    let stale = serde_json::json!({
        "schema": SKILL_CATALOG_SNAPSHOT_SCHEMA,
        "version": SKILL_CATALOG_SNAPSHOT_VERSION + 1,
        "snapshot": {
            "roots": [],
            "signatures": [],
            "skills": []
        }
    });
    fs::write(&snapshot_file, serde_json::to_vec_pretty(&stale).unwrap()).unwrap();

    let authority = SkillAuthority::new(root, None);
    let skills = authority.list_skill_meta(None).unwrap();
    assert!(skills.iter().any(|skill| skill.name == "versioned-skill"));

    let repaired: StoredSkillCatalogSnapshot =
        serde_json::from_str(&fs::read_to_string(snapshot_file).unwrap()).unwrap();
    assert_eq!(repaired.version, SKILL_CATALOG_SNAPSHOT_VERSION);
    assert!(repaired
        .snapshot
        .skills
        .iter()
        .any(|skill| skill.name == "versioned-skill"));
}

#[test]
fn refresh_after_mutation_reloads_new_skill_immediately() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let authority = SkillAuthority::new(root, None);

    authority.refresh().unwrap();

    let skill_path = root.join(".agendao/skills/review/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(
        &skill_path,
        r#"---
name: write-hook-skill
description: write hook
---
Visible after mutation.
"#,
    )
    .unwrap();

    let snapshot = authority.refresh_after_mutation().unwrap();
    assert!(snapshot
        .skills
        .iter()
        .any(|skill| skill.name == "write-hook-skill"));
}
