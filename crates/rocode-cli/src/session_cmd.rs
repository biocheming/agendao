use rocode_storage::{Database, MessageRepository, SessionRepository};

use crate::cli::{SessionCommands, SessionListFormat};
use crate::util::truncate_text;

pub(crate) async fn handle_session_command(action: SessionCommands) -> anyhow::Result<()> {
    let db = Database::new()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to open session database: {}", e))?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    match action {
        SessionCommands::List {
            max_count,
            format,
            project,
        } => {
            let limit = max_count.unwrap_or(50).max(1);
            let sessions = session_repo
                .list(project.as_deref(), limit)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to list sessions: {}", e))?;

            if sessions.is_empty() {
                return Ok(());
            }

            match format {
                SessionListFormat::Json => {
                    let rows: Vec<_> = sessions
                        .into_iter()
                        .filter(|s| s.parent_id.is_none())
                        .map(|s| {
                            serde_json::json!({
                                "id": s.id,
                                "title": s.title,
                                "updated": s.time.updated,
                                "created": s.time.created,
                                "projectId": s.project_id,
                                "directory": s.directory
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                }
                SessionListFormat::Table => {
                    println!("Session ID                      Title                      Updated");
                    println!(
                        "-----------------------------------------------------------------------"
                    );
                    for session in sessions.into_iter().filter(|s| s.parent_id.is_none()) {
                        println!(
                            "{:<30} {:<25} {}",
                            session.id,
                            truncate_text(&session.title, 25),
                            session.time.updated
                        );
                    }
                }
            }
        }
        SessionCommands::Show { session_id } => {
            let Some(session) = session_repo
                .get(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to load session: {}", e))?
            else {
                println!("Session not found: {}", session_id);
                return Ok(());
            };

            let messages = message_repo
                .list_for_session(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to load session messages: {}", e))?;

            println!("\nSession: {}", session.id);
            println!("  Title: {}", session.title);
            println!("  Project: {}", session.project_id);
            println!("  Directory: {}", session.directory);
            println!("  Status: {:?}", session.status);
            println!("  Created: {}", session.time.created);
            println!("  Updated: {}", session.time.updated);
            println!("  Messages: {}", messages.len());
        }
        SessionCommands::Delete { session_id } => {
            message_repo
                .delete_for_session(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to delete session messages: {}", e))?;
            session_repo
                .delete(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to delete session: {}", e))?;
            println!("Session {} deleted.", session_id);
        }
    }
    Ok(())
}
