mod agent_cmd;
mod agent_stream_adapter;
mod api_client;
mod auth;
mod branding;
mod cli_local_data;
mod clipboard;
mod config_cmd;
#[cfg(feature = "session-db")]
mod db;
#[cfg(not(feature = "session-db"))]
mod db {
    use crate::cli::{DbCommands, DbOutputFormat};

    pub(crate) async fn handle_db_command(
        _action: Option<DbCommands>,
        _query: Option<String>,
        _format: DbOutputFormat,
    ) -> anyhow::Result<()> {
        anyhow::bail!("database commands require the `session-db` CLI feature")
    }

    pub(crate) async fn handle_stats_command(
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
    use crate::server_lifecycle::FrontendRuntimeContext;

    pub(crate) async fn handle_debug_command(
        _action: DebugCommands,
        _runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()> {
        anyhow::bail!("debug commands require both the `db` and `lsp` CLI features")
    }
}
mod event_stream;
mod frontend_admin;
mod frontend_entry;
mod frontend_interactive;
mod generate;
mod github;
mod import_export;
mod local_dispatch;
mod local_server_bridge;
mod mcp_cmd;
mod provider_cmd;
mod providers;
#[cfg(feature = "interactive")]
mod remote;
#[cfg(feature = "interactive")]
mod run;
#[cfg(not(feature = "interactive"))]
mod run {
    use std::path::PathBuf;

    use crate::cli::{InteractiveCliMode, RunOutputFormat};
    use crate::server_lifecycle::FrontendRuntimeContext;

    #[derive(Clone, Debug)]
    pub(crate) struct RunNonInteractiveOptions {
        pub(crate) message: Vec<String>,
        pub(crate) command: Option<String>,
        pub(crate) continue_last: bool,
        pub(crate) session: Option<String>,
        pub(crate) fork: bool,
        pub(crate) share: bool,
        pub(crate) model: Option<String>,
        pub(crate) requested_agent: Option<String>,
        pub(crate) requested_scheduler_profile: Option<String>,
        pub(crate) files: Vec<PathBuf>,
        pub(crate) format: RunOutputFormat,
        pub(crate) title: Option<String>,
        pub(crate) attach: Option<String>,
        pub(crate) dir: Option<PathBuf>,
        pub(crate) port: Option<u16>,
        pub(crate) variant: Option<String>,
        pub(crate) thinking: bool,
        pub(crate) interactive_mode: InteractiveCliMode,
        pub(crate) unix_socket: Option<String>,
    }

    pub(crate) async fn run_non_interactive(
        _options: RunNonInteractiveOptions,
        _runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()> {
        anyhow::bail!(
            "`agendao run` requires the `interactive` and `local-server` CLI features to be enabled"
        )
    }
}
mod session_cmd;
mod skill_cmd;
mod util;

pub mod cli {
    pub use agendao_cli_core::cli::*;
}

pub mod server_lifecycle {
    pub use agendao_cli_core::{FrontendRuntimeContext, ServerDiscoveryRequest};
}

use agendao_cli_core::parse_cli_from;
use frontend_entry::dispatch_cli_command;
pub use server_lifecycle::{FrontendRuntimeContext, ServerDiscoveryRequest};

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
    let cli = parse_cli_from(args);
    dispatch_cli_command(cli, &runtime_context).await
}

pub fn spawn_process_reaper() {
    agendao_core::process_registry::global_registry()
        .spawn_reaper(std::time::Duration::from_secs(30));
}
