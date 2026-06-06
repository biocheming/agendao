#[cfg(feature = "session-db")]
use crate::cli_session_store;
use std::path::PathBuf;

#[cfg(feature = "session-db")]
pub(super) async fn export_session_data(
    session_id: Option<String>,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let export = cli_session_store::export_session_bundle(session_id.as_deref()).await?;

    let json = serde_json::to_string_pretty(&export)?;
    match output {
        Some(path) => {
            std::fs::write(&path, json)?;
            println!("Exported session data to {}", path.display());
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

#[cfg(not(feature = "session-db"))]
pub(super) async fn export_session_data(
    _session_id: Option<String>,
    _output: Option<PathBuf>,
) -> anyhow::Result<()> {
    anyhow::bail!("session export requires the `session-db` CLI feature")
}

#[cfg(feature = "session-db")]
pub(super) async fn import_session_data(file_or_url: String) -> anyhow::Result<()> {
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
        std::fs::read_to_string(&file_or_url)?
    };
    let payload: agendao_types::SessionArtifactImportEnvelope = serde_json::from_str(&raw)?;
    let imported = cli_session_store::import_session_bundle(payload).await?;

    println!("Imported {} session(s) from {}", imported, file_or_url);
    Ok(())
}

#[cfg(not(feature = "session-db"))]
pub(super) async fn import_session_data(_file_or_url: String) -> anyhow::Result<()> {
    anyhow::bail!("session import requires the `session-db` CLI feature")
}

#[cfg(all(feature = "memory-db", feature = "memory"))]
pub(super) async fn export_memory_data(output: Option<PathBuf>) -> anyhow::Result<()> {
    let db = agendao_storage::Database::new().await?;
    let memory_repo = agendao_storage::MemoryRepository::new(db.pool().clone());
    let export = agendao_memory::export_memory_artifact_bundle(&memory_repo).await?;

    let json = serde_json::to_string_pretty(&export)?;
    match output {
        Some(path) => {
            std::fs::write(&path, json)?;
            println!("Exported memory data to {}", path.display());
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

#[cfg(not(all(feature = "memory-db", feature = "memory")))]
pub(super) async fn export_memory_data(_output: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("memory export requires both the `memory-db` and `memory` CLI features")
}

#[cfg(all(feature = "memory-db", feature = "memory"))]
pub(super) async fn import_memory_data(file: String) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(&file)?;
    let payload: agendao_types::MemoryArtifactImportEnvelope = serde_json::from_str(&raw)?;
    let db = agendao_storage::Database::new().await?;
    let memory_repo = agendao_storage::MemoryRepository::new(db.pool().clone());
    let imported = agendao_memory::import_memory_artifact_bundle(&memory_repo, payload).await?;

    println!("Imported {} memory record(s) from {}", imported, file);
    Ok(())
}

#[cfg(not(all(feature = "memory-db", feature = "memory")))]
pub(super) async fn import_memory_data(_file: String) -> anyhow::Result<()> {
    anyhow::bail!("memory import requires both the `memory-db` and `memory` CLI features")
}
