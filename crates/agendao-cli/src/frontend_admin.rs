use crate::cli::Commands;
use crate::server_lifecycle::FrontendRuntimeContext;
use agendao_cli_admin::{
    dispatch_admin_command as dispatch_admin_command_with, AdminCommandHandler,
};
use async_trait::async_trait;

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
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::session_cmd::handle_session_command(action, runtime_context).await
    }

    async fn export_memory(&self, output: Option<std::path::PathBuf>) -> anyhow::Result<()> {
        crate::import_export::export_memory_data(output).await
    }

    async fn import_memory(&self, file: String) -> anyhow::Result<()> {
        crate::import_export::import_memory_data(file).await
    }

    async fn handle_skill(
        &self,
        action: crate::cli::SkillCommands,
        runtime_context: &FrontendRuntimeContext,
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
        runtime_context: &FrontendRuntimeContext,
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
        runtime_context: &FrontendRuntimeContext,
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
        crate::import_export::export_session_data(session_id, output).await
    }

    async fn import_session(&self, file: String) -> anyhow::Result<()> {
        crate::import_export::import_session_data(file).await
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
        runtime_context: &FrontendRuntimeContext,
    ) -> anyhow::Result<()> {
        crate::session_cmd::handle_steer_command(session, message, runtime_context).await
    }
}

pub(crate) async fn dispatch_admin_command(
    command: Commands,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    dispatch_admin_command_with(&LegacyAdminHandler, command, runtime_context).await
}
