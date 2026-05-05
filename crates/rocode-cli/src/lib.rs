use clap::Parser;

mod agent_cmd;
mod agent_stream_adapter;
mod api_client;
mod auth;
mod branding;
mod cli;
mod clipboard;
mod config_cmd;
mod db;
mod debug;
mod event_stream;
mod generate;
mod github;
mod import_export;
mod mcp_cmd;
mod provider_cmd;
mod providers;
mod remote;
mod run;
mod server_lifecycle;
mod session_cmd;
mod skill_cmd;
mod util;

use agent_cmd::handle_agent_command;
use auth::handle_auth_command;
use cli::*;
use config_cmd::handle_config_command;
use db::{handle_db_command, handle_stats_command};
use debug::handle_debug_command;
use generate::list_models;
use github::{handle_github_command, handle_pr_command};
use import_export::{
    export_memory_data, export_session_data, import_memory_data, import_session_data,
};
use mcp_cmd::handle_mcp_command;
use provider_cmd::handle_provider_command;
use run::{run_non_interactive, RunNonInteractiveOptions};
pub use server_lifecycle::{FrontendRuntimeContext, ServerDiscoveryRequest};
use session_cmd::handle_session_command;
use skill_cmd::handle_skill_command;

pub async fn run_frontend() -> anyhow::Result<()> {
    run_frontend_with_context(std::env::args_os(), FrontendRuntimeContext::uninitialized()).await
}

pub async fn run_frontend_with_context<I, T>(
    args: I,
    runtime_context: FrontendRuntimeContext,
) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Run {
            message,
            command,
            continue_last,
            session,
            fork,
            share,
            model,
            agent,
            scheduler_profile,
            file,
            format,
            title,
            attach,
            dir,
            port,
            variant,
            thinking,
            interactive_mode,
        } => {
            run_non_interactive(
                RunNonInteractiveOptions {
                    message,
                    command,
                    continue_last,
                    session,
                    fork,
                    share,
                    model,
                    requested_agent: agent,
                    requested_scheduler_profile: scheduler_profile,
                    files: file,
                    format,
                    title,
                    attach,
                    dir,
                    port,
                    variant,
                    thinking,
                    interactive_mode,
                },
                &runtime_context,
            )
            .await?;
        }
        Commands::Models {
            provider,
            refresh,
            verbose,
        } => {
            list_models(provider, refresh, verbose).await?;
        }
        Commands::Session { action } => {
            handle_session_command(action).await?;
        }
        Commands::Memory { action } => match action {
            MemoryCommands::Export { output } => {
                export_memory_data(output).await?;
            }
            MemoryCommands::Import { file } => {
                import_memory_data(file).await?;
            }
        },
        Commands::Skill { action } => {
            handle_skill_command(action, &runtime_context).await?;
        }
        Commands::Provider { action } => {
            handle_provider_command(action).await?;
        }
        Commands::Stats {
            days,
            tools,
            models,
            project,
        } => {
            handle_stats_command(days, tools, models, project).await?;
        }
        Commands::Db {
            action,
            query,
            format,
        } => {
            handle_db_command(action, query, format).await?;
        }
        Commands::Config { action } => {
            handle_config_command(action, &runtime_context).await?;
        }
        Commands::Auth { action } => {
            handle_auth_command(action).await?;
        }
        Commands::Agent { action } => {
            handle_agent_command(action).await?;
        }
        Commands::Debug { action } => {
            handle_debug_command(action, &runtime_context).await?;
        }
        Commands::Mcp { server, action } => {
            handle_mcp_command(server, action).await?;
        }
        Commands::Export { session_id, output } => {
            export_session_data(session_id, output).await?;
        }
        Commands::Import { file } => {
            import_session_data(file).await?;
        }
        Commands::Github { action } => {
            handle_github_command(action).await?;
        }
        Commands::Pr { number } => {
            handle_pr_command(number).await?;
        }
    }

    Ok(())
}

pub fn spawn_process_reaper() {
    rocode_core::process_registry::global_registry()
        .spawn_reaper(std::time::Duration::from_secs(30));
}
