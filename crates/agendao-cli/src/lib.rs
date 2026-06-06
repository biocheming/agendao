mod admin_import_export;
mod agent_cmd;
mod agent_stream_adapter;
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
    use crate::server_lifecycle::CliRuntimeContext;

    pub(super) async fn handle_debug_command(
        _action: DebugCommands,
        _runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        anyhow::bail!("debug commands require both the `db` and `lsp` CLI features")
    }
}
mod generate;
mod github;
mod mcp_cmd;
mod provider_cmd;
mod providers;
#[cfg(feature = "run-remote-stream")]
mod remote;
#[cfg(all(feature = "run-core", not(feature = "run-remote-stream")))]
mod remote {
    #[allow(dead_code)]
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
        pub scheduler_profile: Option<String>,
        pub variant: Option<String>,
        pub format: crate::cli::RunOutputFormat,
        pub title: Option<String>,
        pub directory: Option<String>,
        pub show_thinking: bool,
    }

    pub(super) async fn run_non_interactive_attach(
        _options: RemoteAttachOptions,
    ) -> anyhow::Result<()> {
        anyhow::bail!("remote streaming support requires the `run-remote-stream` CLI feature")
    }
}
#[cfg(feature = "run-core")]
mod run;
#[cfg(not(feature = "run-core"))]
mod run {
    use crate::cli::RunCommandArgs;
    use crate::server_lifecycle::CliRuntimeContext;

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

pub mod cli {
    pub use agendao_cli_core::cli::*;
}

pub mod server_lifecycle {
    pub use agendao_cli_core::{CliRuntimeContext, ServerDiscoveryRequest};
}

use admin_import_export::{
    export_memory_data, export_session_data, import_memory_data, import_session_data,
};
use agendao_cli_admin::dispatch_admin_command as dispatch_admin_command_with;
use agendao_cli_admin::AdminCommandHandler;
use agendao_cli_core::parse_cli_from;
use async_trait::async_trait;
pub use server_lifecycle::{CliRuntimeContext, ServerDiscoveryRequest};

struct LegacyAdminHandler;

#[async_trait(?Send)]
impl AdminCommandHandler for LegacyAdminHandler {
    async fn handle_models(
        &self,
        provider: Option<String>,
        refresh: bool,
        verbose: bool,
    ) -> anyhow::Result<()> {
        crate::generate::list_models(provider, refresh, verbose).await
    }

    async fn handle_session(
        &self,
        action: crate::cli::SessionCommands,
        runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::session_cmd::handle_session_command(action, runtime_context).await
    }

    async fn export_memory(&self, output: Option<std::path::PathBuf>) -> anyhow::Result<()> {
        export_memory_data(output).await
    }

    async fn import_memory(&self, file: String) -> anyhow::Result<()> {
        import_memory_data(file).await
    }

    async fn handle_skill(
        &self,
        action: crate::cli::SkillCommands,
        runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::skill_cmd::handle_skill_command(action, runtime_context).await
    }

    async fn handle_provider(&self, action: crate::cli::ProviderCommands) -> anyhow::Result<()> {
        crate::provider_cmd::handle_provider_command(action).await
    }

    async fn handle_stats(
        &self,
        days: Option<i64>,
        tools: Option<usize>,
        models: Option<usize>,
        project: Option<String>,
    ) -> anyhow::Result<()> {
        crate::db::handle_stats_command(days, tools, models, project).await
    }

    async fn handle_db(
        &self,
        action: Option<crate::cli::DbCommands>,
        query: Option<String>,
        format: crate::cli::DbOutputFormat,
    ) -> anyhow::Result<()> {
        crate::db::handle_db_command(action, query, format).await
    }

    async fn handle_config(
        &self,
        action: Option<crate::cli::ConfigCommands>,
        runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::config_cmd::handle_config_command(action, runtime_context).await
    }

    async fn handle_auth(&self, action: crate::cli::AuthCommands) -> anyhow::Result<()> {
        crate::auth::handle_auth_command(action).await
    }

    async fn handle_agent(&self, action: crate::cli::AgentCommands) -> anyhow::Result<()> {
        crate::agent_cmd::handle_agent_command(action).await
    }

    async fn handle_debug(
        &self,
        action: crate::cli::DebugCommands,
        runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::debug::handle_debug_command(action, runtime_context).await
    }

    async fn handle_mcp(
        &self,
        server: String,
        action: crate::cli::McpCommands,
    ) -> anyhow::Result<()> {
        crate::mcp_cmd::handle_mcp_command(server, action).await
    }

    async fn export_session(
        &self,
        session_id: Option<String>,
        output: Option<std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        export_session_data(session_id, output).await
    }

    async fn import_session(&self, file: String) -> anyhow::Result<()> {
        import_session_data(file).await
    }

    async fn handle_github(&self, action: crate::cli::GithubCommands) -> anyhow::Result<()> {
        crate::github::handle_github_command(action).await
    }

    async fn handle_pr(&self, number: u32) -> anyhow::Result<()> {
        crate::github::handle_pr_command(number).await
    }

    async fn handle_steer(
        &self,
        session: String,
        message: Vec<String>,
        runtime_context: &CliRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::session_cmd::handle_steer_command(session, message, runtime_context).await
    }
}

async fn dispatch_cli_command(
    cli: cli::Cli,
    runtime_context: &CliRuntimeContext,
) -> anyhow::Result<()> {
    match cli.command {
        cli::Commands::Run { args } => {
            run::run_non_interactive(run::run_options_from_args(args)?, runtime_context).await?
        }
        command => {
            dispatch_admin_command_with(&LegacyAdminHandler, command, runtime_context).await?;
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
    dispatch_cli_command(cli, &runtime_context).await
}

pub fn spawn_process_reaper() {
    agendao_core::process_registry::global_registry()
        .spawn_reaper(std::time::Duration::from_secs(30));
}
