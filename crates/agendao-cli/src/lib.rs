mod admin_import_export;
mod agent_cmd;
mod api_client;
mod auth;
#[cfg(feature = "session-db")]
mod cli_session_store;
mod config_cmd;
#[cfg(feature = "session-db")]
mod db;
#[cfg(not(feature = "session-db"))]
mod db {
    use crate::cli::{DbCommands, DbOutputFormat};

    pub(super) async fn handle_db_command(
        _action: Option<DbCommands>,
        _query: Option<String>,
        _format: DbOutputFormat,
    ) -> anyhow::Result<()> {
        anyhow::bail!("database commands require the `session-db` CLI feature")
    }

    pub(super) async fn handle_stats_command(
        _days: Option<i64>,
        _tools_limit: Option<usize>,
        _models_limit: Option<usize>,
        _project: Option<String>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("stats commands require the `session-db` CLI feature")
    }
}
#[cfg(feature = "lsp")]
mod debug;
#[cfg(not(feature = "lsp"))]
mod debug {
    use crate::cli::DebugCommands;
    use crate::CliRuntimeContext;

    pub(super) async fn handle_debug_command(
        _action: DebugCommands,
        _runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        anyhow::bail!("debug commands require both the `db` and `lsp` CLI features")
    }
}
mod generate;
mod github;
mod github_scheduler;
mod mcp_cmd;
mod provider_cmd;
mod providers;
#[cfg(feature = "run-remote-stream")]
mod remote;
mod sandbox_host;
mod scheduler_choice;
#[cfg(all(feature = "run-core", not(feature = "run-remote-stream")))]
mod remote {
    // Same constructor shape as the real remote module so `run.rs` can
    // build options without a feature-gated field list. The stub only
    // rejects attach after consuming the options so fields stay live.
    #[derive(Clone, Debug)]
    pub(super) struct RemoteAttachOptions {
        pub base_url: String,
        pub input: String,
        pub command: Option<String>,
        pub continue_last: bool,
        pub session: Option<String>,
        pub fork: bool,
        pub share: bool,
        pub model: Option<String>,
        pub agent: Option<String>,
        pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
        pub variant: Option<String>,
        pub format: crate::cli::RunOutputFormat,
        pub title: Option<String>,
        pub directory: Option<String>,
        pub show_thinking: bool,
    }

    pub(super) async fn run_non_interactive_attach(
        options: RemoteAttachOptions,
    ) -> anyhow::Result<()> {
        let RemoteAttachOptions {
            base_url,
            input,
            command,
            continue_last,
            session,
            fork,
            share,
            model,
            agent,
            scheduler,
            variant,
            format,
            title,
            directory,
            show_thinking,
        } = options;
        let _ = (
            base_url,
            input,
            command,
            continue_last,
            session,
            fork,
            share,
            model,
            agent,
            scheduler,
            variant,
            format,
            title,
            directory,
            show_thinking,
        );
        anyhow::bail!("remote streaming support requires the `run-remote-stream` CLI feature")
    }
}
#[cfg(feature = "run-core")]
mod run;
#[cfg(not(feature = "run-core"))]
mod run {
    use crate::cli::RunCommandArgs;
    use crate::CliRuntimeContext;

    pub(super) async fn run_non_interactive(
        _options: serde_json::Value,
        _runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        anyhow::bail!("`agendao run` requires the `run-core` CLI feature to be enabled")
    }

    pub(super) fn run_options_from_args(
        _args: RunCommandArgs,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}
mod session_cmd;
mod skill_cmd;
mod util;

use admin_import_export::{
    export_memory_data, export_session_data, import_memory_data, import_session_data,
};
use agendao_cli_core::cli;
use agendao_cli_core::parse_cli_from;
pub use agendao_cli_core::{CliRuntimeContext, ServerDiscoveryRequest};

async fn dispatch_cli_command(
    cli: cli::Cli,
    runtime_context: &CliRuntimeContext,
) -> anyhow::Result<()> {
    match cli.command {
        cli::Commands::Run { args } => {
            run::run_non_interactive(run::run_options_from_args(args)?, runtime_context).await?
        }
        cli::Commands::Models {
            provider,
            refresh,
            verbose,
        } => generate::list_models(provider, refresh, verbose).await?,
        cli::Commands::Session { action } => {
            session_cmd::handle_session_command(action, runtime_context).await?
        }
        cli::Commands::Memory { action } => match action {
            cli::MemoryCommands::Export { output } => export_memory_data(output).await?,
            cli::MemoryCommands::Import { file } => import_memory_data(file).await?,
        },
        cli::Commands::Skill { action } => {
            skill_cmd::handle_skill_command(action, runtime_context).await?
        }
        cli::Commands::Provider { action } => provider_cmd::handle_provider_command(action).await?,
        cli::Commands::Stats {
            days,
            tools,
            models,
            project,
        } => db::handle_stats_command(days, tools, models, project).await?,
        cli::Commands::Db {
            action,
            query,
            format,
        } => db::handle_db_command(action, query, format).await?,
        cli::Commands::Config { action } => {
            config_cmd::handle_config_command(action, runtime_context).await?
        }
        cli::Commands::Auth { action } => auth::handle_auth_command(action).await?,
        cli::Commands::Agent { action } => agent_cmd::handle_agent_command(action).await?,
        cli::Commands::Debug { action } => {
            debug::handle_debug_command(action, runtime_context).await?
        }
        cli::Commands::Mcp { server, action } => {
            mcp_cmd::handle_mcp_command(server, action).await?
        }
        cli::Commands::Export { session_id, output } => {
            export_session_data(session_id, output).await?
        }
        cli::Commands::Import { file } => import_session_data(file).await?,
        cli::Commands::Github { action } => github::handle_github_command(action).await?,
        cli::Commands::Pr { number } => github::handle_pr_command(number).await?,
        cli::Commands::Steer { session, message } => {
            session_cmd::handle_steer_command(session, message, runtime_context).await?
        }
    }

    Ok(())
}

pub async fn run_cli() -> anyhow::Result<()> {
    run_cli_with_context(std::env::args_os(), CliRuntimeContext::uninitialized()).await
}

pub async fn run_cli_with_context<I, T>(
    args: I,
    runtime_context: CliRuntimeContext,
) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = parse_cli_from(args);
    let result = dispatch_cli_command(cli, &runtime_context).await;
    crate::providers::shutdown_native_plugins().await;
    result
}

pub fn spawn_process_reaper() {
    agendao_core::process_registry::global_registry()
        .spawn_reaper(std::time::Duration::from_secs(30));
}
