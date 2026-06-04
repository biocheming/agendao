use std::path::PathBuf;

use agendao_cli_core::cli::{
    AgentCommands, AuthCommands, Commands, ConfigCommands, DbCommands, DbOutputFormat,
    DebugCommands, GithubCommands, McpCommands, MemoryCommands, ProviderCommands, SessionCommands,
    SkillCommands,
};
use agendao_cli_core::FrontendRuntimeContext;
use async_trait::async_trait;

#[async_trait(?Send)]
pub trait AdminCommandHandler {
    async fn handle_models(
        &self,
        provider: Option<String>,
        refresh: bool,
        verbose: bool,
    ) -> anyhow::Result<()>;

    async fn handle_session(
        &self,
        action: SessionCommands,
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()>;

    async fn export_memory(&self, output: Option<PathBuf>) -> anyhow::Result<()>;

    async fn import_memory(&self, file: String) -> anyhow::Result<()>;

    async fn handle_skill(
        &self,
        action: SkillCommands,
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()>;

    async fn handle_provider(&self, action: ProviderCommands) -> anyhow::Result<()>;

    async fn handle_stats(
        &self,
        days: Option<i64>,
        tools: Option<usize>,
        models: Option<usize>,
        project: Option<String>,
    ) -> anyhow::Result<()>;

    async fn handle_db(
        &self,
        action: Option<DbCommands>,
        query: Option<String>,
        format: DbOutputFormat,
    ) -> anyhow::Result<()>;

    async fn handle_config(
        &self,
        action: Option<ConfigCommands>,
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()>;

    async fn handle_auth(&self, action: AuthCommands) -> anyhow::Result<()>;

    async fn handle_agent(&self, action: AgentCommands) -> anyhow::Result<()>;

    async fn handle_debug(
        &self,
        action: DebugCommands,
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()>;

    async fn handle_mcp(&self, server: String, action: McpCommands) -> anyhow::Result<()>;

    async fn export_session(
        &self,
        session_id: Option<String>,
        output: Option<PathBuf>,
    ) -> anyhow::Result<()>;

    async fn import_session(&self, file: String) -> anyhow::Result<()>;

    async fn handle_github(&self, action: GithubCommands) -> anyhow::Result<()>;

    async fn handle_pr(&self, number: u32) -> anyhow::Result<()>;

    async fn handle_steer(
        &self,
        session: String,
        message: Vec<String>,
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()>;
}

pub async fn dispatch_admin_command<H>(
    handler: &H,
    command: Commands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()>
where
    H: AdminCommandHandler + Sync,
{
    match command {
        Commands::Models {
            provider,
            refresh,
            verbose,
        } => {
            handler.handle_models(provider, refresh, verbose).await?;
        }
        Commands::Session { action } => {
            handler.handle_session(action, runtime_context).await?;
        }
        Commands::Memory { action } => match action {
            MemoryCommands::Export { output } => {
                handler.export_memory(output).await?;
            }
            MemoryCommands::Import { file } => {
                handler.import_memory(file).await?;
            }
        },
        Commands::Skill { action } => {
            handler.handle_skill(action, runtime_context).await?;
        }
        Commands::Provider { action } => {
            handler.handle_provider(action).await?;
        }
        Commands::Stats {
            days,
            tools,
            models,
            project,
        } => {
            handler.handle_stats(days, tools, models, project).await?;
        }
        Commands::Db {
            action,
            query,
            format,
        } => {
            handler.handle_db(action, query, format).await?;
        }
        Commands::Config { action } => {
            handler.handle_config(action, runtime_context).await?;
        }
        Commands::Auth { action } => {
            handler.handle_auth(action).await?;
        }
        Commands::Agent { action } => {
            handler.handle_agent(action).await?;
        }
        Commands::Debug { action } => {
            handler.handle_debug(action, runtime_context).await?;
        }
        Commands::Mcp { server, action } => {
            handler.handle_mcp(server, action).await?;
        }
        Commands::Export { session_id, output } => {
            handler.export_session(session_id, output).await?;
        }
        Commands::Import { file } => {
            handler.import_session(file).await?;
        }
        Commands::Github { action } => {
            handler.handle_github(action).await?;
        }
        Commands::Pr { number } => {
            handler.handle_pr(number).await?;
        }
        Commands::Steer { session, message } => {
            handler
                .handle_steer(session, message, runtime_context)
                .await?;
        }
        Commands::Run { .. } | Commands::Cli { .. } => {
            anyhow::bail!("interactive CLI commands must be dispatched via frontend entry")
        }
    }

    Ok(())
}
