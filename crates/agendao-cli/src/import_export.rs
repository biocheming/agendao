use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(any(feature = "session-db", all(feature = "memory-db", feature = "memory")))]
use crate::cli_local_data;
use agendao_config::ConfigStore;
use agendao_skill::{
    export_workspace_skill_artifact_bundle, import_workspace_skill_artifact_bundle, SkillAuthority,
};
#[cfg(all(feature = "memory-db", feature = "memory"))]
use agendao_types::MemoryArtifactImportEnvelope;
#[cfg(feature = "session-db")]
use agendao_types::SessionArtifactImportEnvelope;
use agendao_types::WorkspaceSkillArtifactImportEnvelope;

#[cfg(feature = "session-db")]
pub(crate) async fn export_session_data(
    session_id: Option<String>,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let export = cli_local_data::export_session_bundle(session_id.as_deref()).await?;

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

#[cfg(not(feature = "session-db"))]
pub(crate) async fn export_session_data(
    _session_id: Option<String>,
    _output: Option<PathBuf>,
) -> anyhow::Result<()> {
    anyhow::bail!("session export requires the `session-db` CLI feature")
}

#[cfg(all(feature = "memory-db", feature = "memory"))]
pub(crate) async fn export_memory_data(output: Option<PathBuf>) -> anyhow::Result<()> {
    let export = cli_local_data::export_memory_bundle().await?;

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

#[cfg(not(all(feature = "memory-db", feature = "memory")))]
pub(crate) async fn export_memory_data(_output: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("memory export requires both the `memory-db` and `memory` CLI features")
}

pub(crate) fn export_skill_data(output: Option<PathBuf>) -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;
    export_workspace_skill_data_from_dir(&current_dir, output)
}

#[cfg(feature = "session-db")]
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

#[cfg(feature = "session-db")]
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
    let imported = cli_local_data::import_session_bundle(payload).await?;

    println!("Imported {} session(s) from {}", imported, file_or_url);
    Ok(())
}

#[cfg(not(feature = "session-db"))]
pub(crate) async fn import_session_data(_file_or_url: String) -> anyhow::Result<()> {
    anyhow::bail!("session import requires the `session-db` CLI feature")
}

#[cfg(all(feature = "memory-db", feature = "memory"))]
pub(crate) async fn import_memory_data(file: String) -> anyhow::Result<()> {
    let raw = fs::read_to_string(&file)?;
    let payload: MemoryArtifactImportEnvelope = serde_json::from_str(&raw)?;
    let imported = cli_local_data::import_memory_bundle(payload).await?;

    println!("Imported {} memory record(s) from {}", imported, file);
    Ok(())
}

#[cfg(not(all(feature = "memory-db", feature = "memory")))]
pub(crate) async fn import_memory_data(_file: String) -> anyhow::Result<()> {
    anyhow::bail!("memory import requires both the `memory-db` and `memory` CLI features")
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
