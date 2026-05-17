use crate::api_client::{
    CliApiClient, ProvisionExternalAdapterSessionRequest, ProvisionExternalAdapterSessionResponse,
};
use rocode_storage::{Database, MessageRepository, SessionRepository};

use crate::cli::{SessionCommands, SessionListFormat, SessionProvisionFormat};
use crate::server_lifecycle::FrontendRuntimeContext;
use crate::util::truncate_text;

pub(crate) async fn handle_session_command(
    action: SessionCommands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match action {
        SessionCommands::ProvisionExternalAdapter {
            adapter_id,
            actor_id,
            workspace_id,
            route_policy_id,
            scheduler_profile,
            directory,
            project_id,
            title,
            format,
        } => {
            let client = session_client(runtime_context).await?;
            let response = client
                .provision_external_adapter_session(&ProvisionExternalAdapterSessionRequest {
                    adapter_id,
                    actor_id,
                    workspace_id,
                    route_policy_id,
                    scheduler_profile,
                    directory: directory.map(|path| path.display().to_string()),
                    project_id,
                    title,
                })
                .await?;
            print_provisioned_external_adapter_session(&response, format)?;
            return Ok(());
        }
        SessionCommands::List {
            max_count,
            format,
            project,
        } => {
            let db = Database::new()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to open session database: {}", e))?;
            let session_repo = SessionRepository::new(db.pool().clone());
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
            let db = Database::new()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to open session database: {}", e))?;
            let session_repo = SessionRepository::new(db.pool().clone());
            let message_repo = MessageRepository::new(db.pool().clone());
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
            let db = Database::new()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to open session database: {}", e))?;
            let session_repo = SessionRepository::new(db.pool().clone());
            let message_repo = MessageRepository::new(db.pool().clone());
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

async fn session_client(runtime_context: &FrontendRuntimeContext) -> anyhow::Result<CliApiClient> {
    let base_url = runtime_context.discover_or_start_server(None).await?;
    Ok(CliApiClient::new(base_url))
}

fn print_provisioned_external_adapter_session(
    response: &ProvisionExternalAdapterSessionResponse,
    format: SessionProvisionFormat,
) -> anyhow::Result<()> {
    match format {
        SessionProvisionFormat::Json => {
            println!("{}", serde_json::to_string_pretty(response)?);
        }
        SessionProvisionFormat::Text => {
            println!("Provisioned external adapter session");
            println!("  Session: {}", response.session.id);
            println!("  Adapter: {}", response.adapter);
            println!("  Source: {}", response.source.as_str());
            println!("  Actor: {}", response.binding.actor_id);
            println!("  Workspace: {}", response.binding.workspace_id);
            println!(
                "  Route policy: {}",
                response.binding.route_policy_id.as_deref().unwrap_or("--")
            );
            println!("  Title: {}", response.session.title);
            println!("  Directory: {}", response.session.directory);
        }
    }

    Ok(())
}

/// Submit a mid-run steering message to a session.
/// Constitution §9: CLI submits; runtime consumes at next tool boundary.
pub(crate) async fn handle_steer_command(
    session: String,
    message: Vec<String>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    let text = message.join(" ");
    if text.trim().is_empty() {
        anyhow::bail!("steering message cannot be empty");
    }

    let client = session_client(runtime_context).await?;
    let response = client.submit_steering(&session, text.trim()).await?;

    let owner_session_id = response
        .get("owner_session_id")
        .and_then(|v| v.as_str())
        .unwrap_or(&session);
    let pending_count = response
        .get("pending_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    println!("Steering message enqueued");
    println!("  Owner session: {owner_session_id}");
    println!("  Pending count: {pending_count}");
    if let Some(id) = response.get("id").and_then(|v| v.as_str()) {
        println!("  Steer ID: {id}");
    }

    Ok(())
}
