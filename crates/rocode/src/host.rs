use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;

use async_trait::async_trait;
use rocode_cli::{
    AcpCommandRequest, DesktopWebCommandRequest, ProductHost, ServerCommandRequest,
    TuiCommandRequest, WebCommandRequest,
};
use rocode_launcher as launcher;
use rocode_server::ServerRuntimeOptions;
use rocode_tui::AppLaunchConfig;

pub struct Host;

#[async_trait]
impl ProductHost for Host {
    async fn run_tui(&self, request: TuiCommandRequest) -> anyhow::Result<()> {
        run_tui(request).await
    }

    async fn run_server(&self, request: ServerCommandRequest) -> anyhow::Result<()> {
        run_server_command(request).await
    }

    async fn run_web(&self, request: WebCommandRequest) -> anyhow::Result<()> {
        run_web_command(request).await
    }

    async fn run_desktop_web(&self, request: DesktopWebCommandRequest) -> anyhow::Result<()> {
        run_desktop_web_command(request).await
    }

    async fn run_acp(&self, request: AcpCommandRequest) -> anyhow::Result<()> {
        run_acp_command(request).await
    }
}

async fn run_server_command(request: ServerCommandRequest) -> anyhow::Result<()> {
    rocode_server::run_server_runtime(ServerRuntimeOptions {
        port: request.port,
        hostname: request.hostname,
        cwd: request.dir,
        mdns: request.mdns,
        mdns_domain: request.mdns_domain,
        cors: request.cors,
    })
    .await
}

async fn run_web_command(request: WebCommandRequest) -> anyhow::Result<()> {
    let WebCommandRequest {
        port,
        hostname,
        dir,
        mdns,
        mdns_domain,
        cors,
    } = request;

    let bind_port = if port == 0 { 3000 } else { port };
    let display_host = if hostname == "0.0.0.0" {
        "localhost".to_string()
    } else {
        hostname.clone()
    };
    let backend_url = format!("http://{}:{}", display_host, bind_port);
    let web_dev_url = launcher::resolve_web_dev_url()?;
    let mut effective_cors = cors;
    let web_dist = if let Some(dev_url) = web_dev_url.as_ref() {
        launcher::push_origin_if_missing(&mut effective_cors, dev_url);
        println!("Web dev server: {}", dev_url);
        None
    } else {
        let web_dist = launcher::resolve_web_dist_dir()?;
        println!("Web assets: {}", web_dist.display());
        Some(web_dist)
    };
    let launch_url = if let Some(dev_url) = web_dev_url {
        launcher::append_browser_api_base(dev_url, &backend_url)
    } else {
        backend_url.clone()
    };
    println!("Backend API: {}", backend_url);
    println!("Web interface: {}", launch_url);

    if let Some(web_dist) = web_dist.as_ref() {
        std::env::set_var("ROCODE_WEB_DIST", web_dist);
    }

    let server_task = tokio::spawn(rocode_server::run_server_runtime(ServerRuntimeOptions {
        port: bind_port,
        hostname,
        cwd: dir,
        mdns,
        mdns_domain,
        cors: effective_cors,
    }));

    launcher::wait_for_server_ready(&backend_url, Duration::from_secs(90), None).await?;
    launcher::try_open_browser(&launch_url);
    server_task.await?
}

async fn run_desktop_web_command(request: DesktopWebCommandRequest) -> anyhow::Result<()> {
    let workspace_dir = launcher::resolve_desktop_workspace("rocode")?;
    launcher::remember_desktop_workspace("rocode", &workspace_dir);
    run_web_command(WebCommandRequest {
        port: request.port,
        hostname: request.hostname,
        dir: Some(workspace_dir),
        mdns: request.mdns,
        mdns_domain: request.mdns_domain,
        cors: request.cors,
    })
    .await
}

async fn run_tui(request: TuiCommandRequest) -> anyhow::Result<()> {
    let TuiCommandRequest {
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
        attach_url,
        password: _password,
    } = request;

    if let Some(project) = project {
        std::env::set_current_dir(&project).map_err(|e| {
            anyhow::anyhow!("Failed to change directory to {}: {}", project.display(), e)
        })?;
    }

    if fork && !continue_last && session.is_none() {
        anyhow::bail!("--fork requires --continue or --session");
    }

    let mut server_task = None;
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
        server_task = Some(tokio::spawn(rocode_server::run_server_runtime(
            ServerRuntimeOptions {
                port: bind_port,
                hostname,
                cwd: None,
                mdns,
                mdns_domain,
                cors,
            },
        )));
        launcher::wait_for_server_ready(&server_url, Duration::from_secs(90), None).await?;
        server_url
    };

    let selected_session =
        resolve_requested_session(&base_url, continue_last, session, fork).await?;
    // rocode-tui creates and drives its own Tokio runtime internally.
    // Run it on a blocking thread so we do not try to nest runtimes inside
    // the product shell's async runtime.
    let run_result = tokio::task::spawn_blocking(move || {
        rocode_tui::run_tui_with_config(AppLaunchConfig {
            base_url: Some(base_url),
            model,
            initial_prompt: prompt,
            agent_name: agent,
            session_id: selected_session,
        })
    })
    .await
    .map_err(|error| anyhow::anyhow!("rocode-tui task failed to join: {}", error))?;

    if let Some(server_task) = server_task {
        server_task.abort();
        let _ = server_task.await;
    }

    run_result
}

async fn resolve_requested_session(
    base_url: &str,
    continue_last: bool,
    session: Option<String>,
    fork: bool,
) -> anyhow::Result<Option<String>> {
    let api_client = rocode_client::AsyncApiClient::new(base_url.to_string());
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
        anyhow::bail!(
            "No session is available to fork. Use --session <id> or --continue with an existing session."
        );
    };

    let forked = api_client.fork_session(&session_id, None).await?;
    eprintln!("Forked session {} -> {}", session_id, forked.id);
    Ok(Some(forked.id))
}

async fn run_acp_command(request: AcpCommandRequest) -> anyhow::Result<()> {
    let AcpCommandRequest {
        port,
        hostname,
        mdns,
        mdns_domain,
        cors,
        cwd,
    } = request;

    std::env::set_current_dir(&cwd)
        .map_err(|e| anyhow::anyhow!("Failed to change directory to {}: {}", cwd.display(), e))?;

    if try_run_external_acp_bridge(port, &hostname, mdns, &mdns_domain, &cors, &cwd)? {
        return Ok(());
    }

    eprintln!(
        "Warning: no external ACP stdio bridge runtime found; falling back to HTTP server mode."
    );
    run_server_command(ServerCommandRequest {
        mode: "acp".to_string(),
        port,
        hostname,
        dir: Some(cwd),
        mdns,
        mdns_domain,
        cors,
    })
    .await
}

fn build_acp_network_args(
    port: u16,
    hostname: &str,
    mdns: bool,
    mdns_domain: &str,
    cors: &[String],
    cwd: &Path,
) -> Vec<String> {
    let mut args = vec![
        "acp".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--hostname".to_string(),
        hostname.to_string(),
        "--cwd".to_string(),
        cwd.display().to_string(),
    ];

    if mdns {
        args.push("--mdns".to_string());
        args.push("--mdns-domain".to_string());
        args.push(mdns_domain.to_string());
    }

    for origin in cors {
        args.push("--cors".to_string());
        args.push(origin.clone());
    }

    args
}

fn find_local_ts_opencode_package_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("../opencode/packages/opencode"));
        candidates.push(cwd.join("opencode/packages/opencode"));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(mut base) = exe.parent().map(PathBuf::from) {
            for _ in 0..6 {
                candidates.push(base.join("../opencode/packages/opencode"));
                candidates.push(base.join("opencode/packages/opencode"));
                if !base.pop() {
                    break;
                }
            }
        }
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.join("src/index.ts").exists())
}

fn run_acp_bridge_candidate(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> anyhow::Result<bool> {
    let mut cmd = ProcessCommand::new(program);
    cmd.args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("ROCODE_ACP_BRIDGE_ACTIVE", "1")
        .env("OPENCODE_ACP_BRIDGE_ACTIVE", "1");

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let status = match cmd.status() {
        Ok(status) => status,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            anyhow::bail!("Failed to launch ACP bridge command `{}`: {}", program, err);
        }
    };

    if !status.success() {
        anyhow::bail!(
            "ACP bridge command `{}` exited with status {}",
            program,
            status
        );
    }

    Ok(true)
}

fn try_run_external_acp_bridge(
    port: u16,
    hostname: &str,
    mdns: bool,
    mdns_domain: &str,
    cors: &[String],
    cwd: &Path,
) -> anyhow::Result<bool> {
    if std::env::var("ROCODE_ACP_BRIDGE_ACTIVE")
        .or_else(|_| std::env::var("OPENCODE_ACP_BRIDGE_ACTIVE"))
        .ok()
        .as_deref()
        == Some("1")
    {
        return Ok(false);
    }

    let acp_args = build_acp_network_args(port, hostname, mdns, mdns_domain, cors, cwd);

    if let Ok(bin) =
        std::env::var("ROCODE_ACP_BRIDGE_BIN").or_else(|_| std::env::var("OPENCODE_ACP_BRIDGE_BIN"))
    {
        let bin = bin.trim();
        if bin.is_empty() {
            anyhow::bail!(
                "ROCODE_ACP_BRIDGE_BIN is set but empty (legacy fallback: OPENCODE_ACP_BRIDGE_BIN)."
            );
        }
        return run_acp_bridge_candidate(bin, &acp_args, None);
    }

    if let Ok(rocode_path) = which::which("rocode").or_else(|_| which::which("opencode")) {
        let is_self = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                let lhs = std::fs::canonicalize(exe).ok()?;
                let rhs = std::fs::canonicalize(rocode_path).ok()?;
                Some(lhs == rhs)
            })
            .unwrap_or(false);
        if !is_self && run_acp_bridge_candidate("rocode", &acp_args, None)? {
            return Ok(true);
        }
    }

    if which::which("bun").is_ok() {
        if let Some(pkg_dir) = find_local_ts_opencode_package_dir() {
            let mut bun_args = vec![
                "run".to_string(),
                "--cwd".to_string(),
                pkg_dir.display().to_string(),
                "--conditions=browser".to_string(),
                "src/index.ts".to_string(),
            ];
            bun_args.extend(acp_args);
            if run_acp_bridge_candidate("bun", &bun_args, None)? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}
