use async_trait::async_trait;
use clap::Parser;

mod agent_cmd;
mod agent_stream_adapter;
mod api_client;
mod auth;
mod branding;
mod cli;
mod clipboard;
mod db;
mod debug;
mod event_stream;
mod generate;
mod github;
mod import_export;
mod mcp_cmd;
mod providers;
mod remote;
mod run;
mod server_lifecycle;
mod session_cmd;
mod skill_cmd;
mod upgrade;
mod util;

use agent_cmd::handle_agent_command;
use auth::handle_auth_command;
use cli::*;
use db::{handle_db_command, handle_stats_command};
use debug::handle_debug_command;
use generate::{handle_generate_command, list_models};
use github::{handle_github_command, handle_pr_command};
use import_export::{export_session_data, import_session_data};
use mcp_cmd::handle_mcp_command;
use run::{run_non_interactive, RunNonInteractiveOptions};
use session_cmd::{handle_session_command, show_config};
use skill_cmd::handle_skill_command;
use upgrade::{handle_uninstall_command, handle_upgrade_command};

#[derive(Clone, Debug)]
pub struct TuiCommandRequest {
    pub project: Option<std::path::PathBuf>,
    pub model: Option<String>,
    pub continue_last: bool,
    pub session: Option<String>,
    pub fork: bool,
    pub prompt: Option<String>,
    pub agent: Option<String>,
    pub port: u16,
    pub hostname: String,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    pub attach_url: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ServerCommandRequest {
    pub mode: String,
    pub port: u16,
    pub hostname: String,
    pub dir: Option<std::path::PathBuf>,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct WebCommandRequest {
    pub port: u16,
    pub hostname: String,
    pub dir: Option<std::path::PathBuf>,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DesktopWebCommandRequest {
    pub port: u16,
    pub hostname: String,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AcpCommandRequest {
    pub port: u16,
    pub hostname: String,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    pub cwd: std::path::PathBuf,
}

#[async_trait]
pub trait ProductHost {
    async fn run_tui(&self, request: TuiCommandRequest) -> anyhow::Result<()>;
    async fn run_server(&self, request: ServerCommandRequest) -> anyhow::Result<()>;
    async fn run_web(&self, request: WebCommandRequest) -> anyhow::Result<()>;
    async fn run_desktop_web(&self, request: DesktopWebCommandRequest) -> anyhow::Result<()>;
    async fn run_acp(&self, request: AcpCommandRequest) -> anyhow::Result<()>;
}

pub async fn run_with_host<H: ProductHost + Sync>(host: &H) -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Tui {
            project,
            model,
            continue_last,
            session,
            fork,
            prompt,
            agent,
            port,
            hostname,
            mdns,
            mdns_domain,
            cors,
        }) => {
            host.run_tui(TuiCommandRequest {
                project,
                model,
                continue_last,
                session,
                fork,
                prompt,
                agent,
                port,
                hostname,
                mdns,
                mdns_domain,
                cors,
                attach_url: None,
                password: None,
            })
            .await?;
        }
        Some(Commands::Attach {
            url,
            dir,
            session,
            password,
        }) => {
            host.run_tui(TuiCommandRequest {
                project: dir,
                model: None,
                continue_last: false,
                session,
                fork: false,
                prompt: None,
                agent: None,
                port: 0,
                hostname: "127.0.0.1".to_string(),
                mdns: false,
                mdns_domain: "rocode.local".to_string(),
                cors: vec![],
                attach_url: Some(url),
                password,
            })
            .await?;
        }
        Some(Commands::Run {
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
        }) => {
            run_non_interactive(RunNonInteractiveOptions {
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
            })
            .await?;
        }
        Some(Commands::Serve {
            port,
            hostname,
            dir,
            mdns,
            mdns_domain,
            cors,
        }) => {
            host.run_server(ServerCommandRequest {
                mode: "serve".to_string(),
                port,
                hostname,
                dir,
                mdns,
                mdns_domain,
                cors,
            })
            .await?;
        }
        Some(Commands::Web {
            port,
            hostname,
            dir,
            mdns,
            mdns_domain,
            cors,
        }) => {
            host.run_web(WebCommandRequest {
                port,
                hostname,
                dir,
                mdns,
                mdns_domain,
                cors,
            })
            .await?;
        }
        Some(Commands::Acp {
            port,
            hostname,
            mdns,
            mdns_domain,
            cors,
            cwd,
        }) => {
            host.run_acp(AcpCommandRequest {
                port,
                hostname,
                mdns,
                mdns_domain,
                cors,
                cwd,
            })
            .await?;
        }
        Some(Commands::Models {
            provider,
            refresh,
            verbose,
        }) => {
            list_models(provider, refresh, verbose).await?;
        }
        Some(Commands::Session { action }) => {
            handle_session_command(action).await?;
        }
        Some(Commands::Skill { action }) => {
            handle_skill_command(action).await?;
        }
        Some(Commands::Stats {
            days,
            tools,
            models,
            project,
        }) => {
            handle_stats_command(days, tools, models, project).await?;
        }
        Some(Commands::Db {
            action,
            query,
            format,
        }) => {
            handle_db_command(action, query, format).await?;
        }
        Some(Commands::Config) => {
            show_config().await?;
        }
        Some(Commands::Auth { action }) => {
            handle_auth_command(action).await?;
        }
        Some(Commands::Agent { action }) => {
            handle_agent_command(action).await?;
        }
        Some(Commands::Debug { action }) => {
            handle_debug_command(action).await?;
        }
        Some(Commands::Mcp { server, action }) => {
            handle_mcp_command(server, action).await?;
        }
        Some(Commands::Export { session_id, output }) => {
            export_session_data(session_id, output).await?;
        }
        Some(Commands::Import { file }) => {
            import_session_data(file).await?;
        }
        Some(Commands::Github { action }) => {
            handle_github_command(action).await?;
        }
        Some(Commands::Pr { number }) => {
            handle_pr_command(number).await?;
        }
        Some(Commands::Upgrade { target, method }) => {
            handle_upgrade_command(target, method).await?;
        }
        Some(Commands::Uninstall {
            keep_config,
            keep_data,
            dry_run,
            force,
        }) => {
            handle_uninstall_command(keep_config, keep_data, dry_run, force).await?;
        }
        Some(Commands::Generate) => {
            handle_generate_command().await?;
        }
        Some(Commands::Version) => {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Info) => {
            print_build_info();
        }
        None => {
            host.run_tui(TuiCommandRequest {
                project: None,
                model: None,
                continue_last: false,
                session: None,
                fork: false,
                prompt: None,
                agent: None,
                port: 0,
                hostname: "127.0.0.1".to_string(),
                mdns: false,
                mdns_domain: "rocode.local".to_string(),
                cors: vec![],
                attach_url: None,
                password: None,
            })
            .await?;
        }
    }

    Ok(())
}

pub fn spawn_process_reaper() {
    rocode_core::process_registry::global_registry()
        .spawn_reaper(std::time::Duration::from_secs(30));
}

pub fn print_build_info() {
    let built_at = option_env!("ROCODE_BUILD_TIMESTAMP").unwrap_or("unknown");
    let rustc = option_env!("ROCODE_RUSTC_VERSION").unwrap_or("unknown");
    let profile = option_env!("ROCODE_BUILD_PROFILE").unwrap_or("unknown");
    let target = option_env!("TARGET").unwrap_or("unknown");
    let host = option_env!("HOST").unwrap_or("unknown");

    println!("ROCode {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Build Info:");
    println!("  Compiler:   {}", rustc);
    println!("  Profile:    {}", profile);
    println!("  Target:     {}", target);
    println!("  Host:       {}", host);
    println!("  Built at:   {}", built_at);
    println!();

    let data = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("rocode");
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("rocode");
    let config = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("rocode");

    println!("Paths:");
    println!("  Data:       {}", data.display());
    println!("  Config:     {}", config.display());
    println!("  Cache:      {}", cache.display());
}
