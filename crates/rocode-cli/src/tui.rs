use std::path::PathBuf;
use std::time::Duration;

use rocode_launcher::{
    build_tui_env, run_tui_foreground, spawn_server_background, wait_for_server_ready,
    ServerLaunchOptions,
};

use crate::api_client::CliApiClient;

pub(crate) struct TuiLaunchOptions {
    pub project: Option<PathBuf>,
    pub model: Option<String>,
    pub continue_last: bool,
    pub session: Option<String>,
    pub fork: bool,
    pub agent_name: Option<String>,
    pub initial_prompt: Option<String>,
    pub port: u16,
    pub hostname: String,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    pub attach_url: Option<String>,
    pub password: Option<String>,
}

pub(crate) async fn run_tui(options: TuiLaunchOptions) -> anyhow::Result<()> {
    let TuiLaunchOptions {
        project,
        model,
        continue_last,
        session,
        fork,
        agent_name,
        initial_prompt,
        port,
        hostname,
        mdns,
        mdns_domain,
        cors,
        attach_url,
        password: _password,
    } = options;

    if let Some(project) = project {
        std::env::set_current_dir(&project).map_err(|e| {
            anyhow::anyhow!("Failed to change directory to {}: {}", project.display(), e)
        })?;
    }

    if fork && !continue_last && session.is_none() {
        anyhow::bail!("--fork requires --continue or --session");
    }

    let mut server_child = None;
    let base_url = if let Some(url) = attach_url {
        url
    } else {
        let client_host = if mdns && hostname == "127.0.0.1" {
            "127.0.0.1".to_string()
        } else {
            hostname.clone()
        };
        let bind_port = if port == 0 { 3000 } else { port };
        let server_url = format!("http://{}:{}", client_host, bind_port);
        eprintln!("Starting local server for TUI at {}", server_url);
        let options = ServerLaunchOptions {
            port: bind_port,
            hostname,
            cwd: None,
            web_dist: None,
            mdns,
            mdns_domain,
            cors,
        };
        let mut child = spawn_server_background(
            &options,
            std::process::Stdio::inherit(),
            std::process::Stdio::inherit(),
        )?;
        wait_for_server_ready(&server_url, Duration::from_secs(90), Some(&mut child)).await?;
        server_child = Some(child);
        server_url
    };

    let selected_session =
        resolve_requested_session(&base_url, continue_last, session, fork).await?;
    let run_result = run_tui_foreground(build_tui_env(
        &base_url,
        model,
        initial_prompt,
        agent_name,
        selected_session,
    ))
    .await;

    if let Some(mut child) = server_child {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }

    run_result
}

async fn resolve_requested_session(
    base_url: &str,
    continue_last: bool,
    session: Option<String>,
    fork: bool,
) -> anyhow::Result<Option<String>> {
    let api_client = CliApiClient::new(base_url.to_string());
    let selected = if let Some(session_id) = session {
        Some(session_id)
    } else if continue_last {
        api_client
            .list_sessions(None, Some(100))
            .await?
            .into_iter()
            .find(|s| s.parent_id.is_none())
            .map(|s| s.id)
    } else {
        None
    };

    if !fork {
        return Ok(selected);
    }

    let Some(session_id) = selected else {
        anyhow::bail!("No session is available to fork. Use --session <id> or --continue with an existing session.");
    };

    let forked = api_client.fork_session(&session_id, None).await?;
    eprintln!("Forked session {} -> {}", session_id, forked.id);
    Ok(Some(forked.id))
}
