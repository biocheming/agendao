use crate::api_client::{
    CliApiClient, ProvisionExternalAdapterSessionRequest, ProvisionExternalAdapterSessionResponse,
};

#[cfg(feature = "session-db")]
use crate::cli::SessionListFormat;
use crate::cli::{SessionBlueprintCommands, SessionCommands, SessionProvisionFormat};
#[cfg(feature = "session-db")]
use crate::cli_session_store;
#[cfg(feature = "session-db")]
use crate::util::truncate_text;
use crate::CliRuntimeContext;

pub(super) async fn handle_session_command(
    action: SessionCommands,
    runtime_context: &CliRuntimeContext,
) -> anyhow::Result<()> {
    match action {
        SessionCommands::Blueprint { action } => {
            handle_blueprint_command(action, runtime_context).await
        }
        SessionCommands::ProvisionExternalAdapter {
            adapter_id,
            actor_id,
            workspace_id,
            route_policy_id,
            scheduler,
            directory,
            project_id,
            title,
            format,
        } => {
            let scheduler = crate::scheduler_choice::parse_scheduler_choice(scheduler.as_deref())?;
            let client = session_client(runtime_context).await?;
            let response = client
                .provision_external_adapter_session(&ProvisionExternalAdapterSessionRequest {
                    adapter_id,
                    actor_id,
                    workspace_id,
                    route_policy_id,
                    scheduler,
                    directory: directory.map(|path| path.display().to_string()),
                    project_id,
                    title,
                })
                .await?;
            print_provisioned_external_adapter_session(&response, format)?;
            Ok(())
        }
        SessionCommands::List {
            max_count,
            format,
            project,
        } => {
            #[cfg(feature = "session-db")]
            {
                let limit = max_count.unwrap_or(50).max(1);
                let sessions = cli_session_store::list_sessions(project.as_deref(), limit)
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
                        Ok(())
                    }
                    SessionListFormat::Table => {
                        println!(
                            "Session ID                      Title                      Updated"
                        );
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
                        Ok(())
                    }
                }
            }
            #[cfg(not(feature = "session-db"))]
            {
                let _ = (max_count, format, project);
                anyhow::bail!("session list requires the `session-db` CLI feature");
            }
        }
        SessionCommands::Show { session_id } => {
            #[cfg(feature = "session-db")]
            {
                let Some(detail) = cli_session_store::get_session_detail(&session_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to load session: {}", e))?
                else {
                    println!("Session not found: {}", session_id);
                    return Ok(());
                };
                let session = detail.session;

                println!("\nSession: {}", session.id);
                println!("  Title: {}", session.title);
                println!("  Project: {}", session.project_id);
                println!("  Directory: {}", session.directory);
                println!("  Status: {:?}", session.status);
                println!("  Created: {}", session.time.created);
                println!("  Updated: {}", session.time.updated);
                println!("  Messages: {}", detail.message_count);
                Ok(())
            }
            #[cfg(not(feature = "session-db"))]
            {
                let _ = session_id;
                anyhow::bail!("session show requires the `session-db` CLI feature");
            }
        }
        SessionCommands::Delete { session_id } => {
            #[cfg(feature = "session-db")]
            {
                cli_session_store::delete_session(&session_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to delete session: {}", e))?;
                println!("Session {} deleted.", session_id);
                Ok(())
            }
            #[cfg(not(feature = "session-db"))]
            {
                let _ = session_id;
                anyhow::bail!("session delete requires the `session-db` CLI feature");
            }
        }
    }
}

async fn handle_blueprint_command(
    action: SessionBlueprintCommands,
    runtime_context: &CliRuntimeContext,
) -> anyhow::Result<()> {
    let client = session_client(runtime_context).await?;
    match action {
        SessionBlueprintCommands::Inspect { session_id } => {
            let view = client.get_session_blueprint(&session_id).await?;
            println!("{}", serde_json::to_string_pretty(&view)?);
        }
        SessionBlueprintCommands::Save {
            session_id,
            output,
            force,
        } => {
            let view = client.get_session_blueprint(&session_id).await?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true);
            if force {
                options.create(true).truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options.open(&output).map_err(|error| {
                anyhow::anyhow!("failed to create Blueprint '{}': {error}", output.display())
            })?;
            use std::io::Write;
            serde_json::to_writer_pretty(&mut file, &view.blueprint)?;
            file.write_all(b"\n")?;
            println!("{}", output.display());
        }
        SessionBlueprintCommands::Edit { session_id, file } => {
            let blueprint = crate::scheduler_choice::parse_blueprint_file(&file)?;
            let view = client
                .set_session_blueprint(
                    &session_id,
                    &crate::api_client::SetSessionBlueprintRequest { blueprint },
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&view)?);
        }
        SessionBlueprintCommands::Reject { session_id } => {
            let response = client.reject_session_blueprint(&session_id).await?;
            println!("{}", response.rejected_fingerprint);
        }
    }
    Ok(())
}

async fn session_client(runtime_context: &CliRuntimeContext) -> anyhow::Result<CliApiClient> {
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
pub(super) async fn handle_steer_command(
    session: String,
    message: Vec<String>,
    runtime_context: &CliRuntimeContext,
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
