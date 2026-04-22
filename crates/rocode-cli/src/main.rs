use clap::Parser;
use std::io::IsTerminal;

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
mod server;
mod server_lifecycle;
mod session_cmd;
mod skill_cmd;
mod tui;
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
use server::{run_acp_command, run_desktop_web_command, run_server_command, run_web_command};
use session_cmd::{handle_session_command, show_config};
use skill_cmd::handle_skill_command;
use tui::{run_tui, TuiLaunchOptions};
use upgrade::{handle_uninstall_command, handle_upgrade_command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Write logs to a file so they're visible even when TUI captures stderr.
    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("rocode")
        .join("log");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("rocode.log"))
        .ok();
    if let Some(file) = log_file {
        use tracing_subscriber::EnvFilter;
        let default_level = if cfg!(debug_assertions) {
            "debug"
        } else {
            "warn"
        };
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    } else {
        use tracing_subscriber::EnvFilter;
        let default_level = if cfg!(debug_assertions) {
            "debug"
        } else {
            "warn"
        };
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level)),
            )
            .init();
    }

    let cli = Cli::parse();

    // Start the background process reaper (defense layer A — 30s interval).
    // Primary cleanup is via ProcessGuard RAII (strategy B); this is insurance.
    rocode_core::process_registry::global_registry()
        .spawn_reaper(std::time::Duration::from_secs(30));

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
            run_tui(TuiLaunchOptions {
                project,
                model,
                continue_last,
                session,
                fork,
                agent_name: agent,
                initial_prompt: prompt,
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
            run_tui(TuiLaunchOptions {
                project: dir,
                model: None,
                continue_last: false,
                session,
                fork: false,
                agent_name: None,
                initial_prompt: None,
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
            run_server_command("serve", port, hostname, dir, mdns, mdns_domain, cors).await?;
        }
        Some(Commands::Web {
            port,
            hostname,
            dir,
            mdns,
            mdns_domain,
            cors,
        }) => {
            run_web_command(port, hostname, dir, mdns, mdns_domain, cors).await?;
        }
        Some(Commands::Acp {
            port,
            hostname,
            mdns,
            mdns_domain,
            cors,
            cwd,
        }) => {
            run_acp_command(port, hostname, mdns, mdns_domain, cors, cwd).await?;
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
            println!("ROCode {}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Info) => {
            show_build_info();
        }
        None => {
            let launched_from_desktop =
                !std::io::stdin().is_terminal() && !std::io::stdout().is_terminal();
            if launched_from_desktop {
                run_desktop_web_command(
                    0,
                    "127.0.0.1".to_string(),
                    false,
                    "rocode.local".to_string(),
                    vec![],
                )
                .await?;
                return Ok(());
            }
            run_tui(TuiLaunchOptions {
                project: None,
                model: None,
                continue_last: false,
                session: None,
                fork: false,
                agent_name: None,
                initial_prompt: None,
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

fn show_build_info() {
    println!("ROCode {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Build Info:");
    println!("  Compiler:   {}", env!("ROCODE_RUSTC_VERSION"));
    println!("  Profile:    {}", env!("ROCODE_PROFILE"));
    println!("  Target:     {}", env!("ROCODE_TARGET"));
    println!("  Host:       {}", env!("ROCODE_HOST"));
    println!("  Built at:   {}", env!("ROCODE_BUILD_TIME"));
    println!();
    println!("Paths:");
    let data_dir = dirs::data_local_dir().unwrap_or_default().join("rocode");
    let config_dir = dirs::config_dir().unwrap_or_default().join("rocode");
    let cache_dir = dirs::cache_dir().unwrap_or_default().join("rocode");
    println!("  Data:       {}", data_dir.display());
    println!("  Config:     {}", config_dir.display());
    println!("  Cache:      {}", cache_dir.display());
    println!();
    println!("Plugin ABI:");
    println!("  Native plugins (dylib) must be compiled with the same");
    println!("  Rust compiler version listed above.");
}
