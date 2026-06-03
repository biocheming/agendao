use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agendao_config::ConfigStore;
use agendao_memory::{export_memory_artifact_bundle, import_memory_artifact_bundle};
use agendao_skill::{
    export_workspace_skill_artifact_bundle, import_workspace_skill_artifact_bundle, SkillAuthority,
};
use agendao_storage::{Database, MemoryRepository, MessageRepository, SessionRepository};
use agendao_types::{
    MemoryArtifactImportEnvelope, SessionArtifactBundle, SessionArtifactEntry,
    SessionArtifactImportEnvelope, WorkspaceSkillArtifactImportEnvelope,
};

pub(crate) async fn export_session_data(
    session_id: Option<String>,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let session = if let Some(session_id) = session_id {
        session_repo
            .get(&session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?
    } else {
        session_repo
            .list(None, 1)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No sessions found to export"))?
    };

    let messages = message_repo.list_for_session(&session.id).await?;
    let export = SessionArtifactBundle::new_now(vec![SessionArtifactEntry::new(session, messages)]);

    let json = serde_json::to_string_pretty(&export)?;
    match output {
        Some(path) => {
            fs::write(&path, json)?;
            println!("Exported session data to {}", path.display());
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

pub(crate) async fn export_memory_data(output: Option<PathBuf>) -> anyhow::Result<()> {
    let db = Database::new().await?;
    let memory_repo = MemoryRepository::new(db.pool().clone());
    let export = export_memory_artifact_bundle(&memory_repo).await?;

    let json = serde_json::to_string_pretty(&export)?;
    match output {
        Some(path) => {
            fs::write(&path, json)?;
            println!("Exported memory data to {}", path.display());
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

pub(crate) fn export_skill_data(output: Option<PathBuf>) -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;
    export_workspace_skill_data_from_dir(&current_dir, output)
}

fn parse_share_slug(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    if let Some(idx) = trimmed.rfind("/share/") {
        return Some(trimmed[idx + 7..].to_string());
    }
    if let Some(idx) = trimmed.rfind("/s/") {
        return Some(trimmed[idx + 3..].to_string());
    }
    None
}

pub(crate) async fn import_session_data(file_or_url: String) -> anyhow::Result<()> {
    let raw = if file_or_url.starts_with("http://") || file_or_url.starts_with("https://") {
        let client = reqwest::Client::new();
        let mut text = client.get(&file_or_url).send().await?.text().await?;

        if let Some(slug) = parse_share_slug(&file_or_url) {
            if serde_json::from_str::<serde_json::Value>(&text).is_err() {
                let share_api = format!("https://opencode.ai/api/share/{}/data", slug);
                text = client.get(share_api).send().await?.text().await?;
            }
        }
        text
    } else {
        fs::read_to_string(&file_or_url)?
    };
    let payload: SessionArtifactImportEnvelope = serde_json::from_str(&raw)?;
    let entries = payload.into_entries();

    if entries.is_empty() {
        anyhow::bail!("No session entries found in {}", file_or_url);
    }

    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let mut imported = 0usize;
    for mut entry in entries {
        let session_id = entry.session.id.clone();
        entry.session.messages.clear();

        if session_repo.get(&entry.session.id).await?.is_some() {
            session_repo.update(&entry.session).await?;
        } else {
            session_repo.create(&entry.session).await?;
        }

        for mut message in entry.messages {
            if message.session_id.is_empty() {
                message.session_id = session_id.clone();
            }
            message_repo.upsert(&message).await?;
        }
        imported += 1;
    }

    println!("Imported {} session(s) from {}", imported, file_or_url);
    Ok(())
}

pub(crate) async fn import_memory_data(file: String) -> anyhow::Result<()> {
    let raw = fs::read_to_string(&file)?;
    let payload: MemoryArtifactImportEnvelope = serde_json::from_str(&raw)?;

    let db = Database::new().await?;
    let memory_repo = MemoryRepository::new(db.pool().clone());
    let imported = import_memory_artifact_bundle(&memory_repo, payload).await?;

    println!("Imported {} memory record(s) from {}", imported, file);
    Ok(())
}

pub(crate) fn import_skill_data(file: String) -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;
    import_workspace_skill_data_into_dir(&current_dir, Path::new(&file))?;
    Ok(())
}

fn export_workspace_skill_data_from_dir(
    base_dir: &Path,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let authority = workspace_skill_authority(base_dir);
    let export = export_workspace_skill_artifact_bundle(&authority)?;

    let json = serde_json::to_string_pretty(&export)?;
    match output {
        Some(path) => {
            fs::write(&path, json)?;
            println!("Exported workspace skill data to {}", path.display());
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

fn import_workspace_skill_data_into_dir(base_dir: &Path, file: &Path) -> anyhow::Result<usize> {
    let raw = fs::read_to_string(file)?;
    let payload: WorkspaceSkillArtifactImportEnvelope = serde_json::from_str(&raw)?;

    let authority = workspace_skill_authority(base_dir);
    let imported = import_workspace_skill_artifact_bundle(&authority, payload)?;

    println!(
        "Imported {} workspace skill(s) from {}",
        imported,
        file.display()
    );
    Ok(imported)
}

fn workspace_skill_authority(base_dir: &Path) -> SkillAuthority {
    let config_store = ConfigStore::from_project_dir(base_dir).ok().map(Arc::new);
    SkillAuthority::new(base_dir.to_path_buf(), config_store)
}

#[cfg(test)]
mod tests {
    use super::{export_workspace_skill_data_from_dir, import_workspace_skill_data_into_dir};
    use agendao_skill::SkillAuthority;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = format!(
            "agendao-cli-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("workspace should be created");
        path
    }

    fn write_workspace_skill(
        root: &Path,
        relative_dir: &str,
        name: &str,
        description: &str,
        body: &str,
        supporting_files: &[(&str, &str)],
    ) {
        let skill_dir = root.join(".agendao/skills").join(relative_dir);
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

    fn authority_for(base_dir: &Path) -> SkillAuthority {
        let config_store = agendao_config::ConfigStore::from_project_dir(base_dir)
            .ok()
            .map(Arc::new);
        SkillAuthority::new(base_dir.to_path_buf(), config_store)
    }

    #[test]
    fn workspace_skill_export_and_import_roundtrip_through_cli_helpers() {
        let source = temp_workspace("skill-export-source");
        let target = temp_workspace("skill-export-target");
        let export_path = temp_workspace("skill-export-file").join("skills.json");

        write_workspace_skill(
            &source,
            "reviewer",
            "reviewer",
            "Review code changes",
            "# Reviewer\n\nInspect patches carefully.",
            &[("templates/checklist.md", "- scope\n- tests\n")],
        );

        export_workspace_skill_data_from_dir(&source, Some(export_path.clone())).expect("export");
        assert!(export_path.exists());

        let exported = fs::read_to_string(&export_path).expect("exported artifact");
        assert!(exported.contains("\"version\": \"agendao-rust/workspace-skill/v1\""));
        assert!(exported.contains("\"reviewer\""));
        assert!(!exported.contains(&source.display().to_string()));

        let imported = import_workspace_skill_data_into_dir(&target, &export_path).expect("import");
        assert_eq!(imported, 1);

        let authority = authority_for(&target);
        let skill = authority
            .load_skill_for_inspection("reviewer", None)
            .expect("skill should load");
        assert_eq!(skill.meta.name, "reviewer");
        assert!(skill.content.contains("Inspect patches carefully."));

        let supporting = authority
            .load_skill_file_for_inspection("reviewer", "templates/checklist.md")
            .expect("supporting file should load");
        assert_eq!(supporting.content, "- scope\n- tests\n");

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
        let _ = fs::remove_dir_all(export_path.parent().expect("export parent"));
    }
}
