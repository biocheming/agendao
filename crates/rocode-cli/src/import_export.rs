use std::fs;
use std::path::PathBuf;

use rocode_memory::{export_memory_artifact_bundle, import_memory_artifact_bundle};
use rocode_storage::{Database, MemoryRepository, MessageRepository, SessionRepository};
use rocode_types::{
    MemoryArtifactImportEnvelope, SessionArtifactBundle, SessionArtifactEntry,
    SessionArtifactImportEnvelope,
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
