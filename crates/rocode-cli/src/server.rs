use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;

use rocode_launcher::{self as launcher, ServerLaunchOptions};

pub(crate) async fn run_server_command(
    mode: &str,
    port: u16,
    hostname: String,
    dir: Option<PathBuf>,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
) -> anyhow::Result<()> {
    let bind_port = if port == 0 { 3000 } else { port };
    let options = ServerLaunchOptions {
        port: bind_port,
        hostname,
        cwd: dir,
        web_dist: None,
        mdns,
        mdns_domain,
        cors,
    };
    if let Some(path) = launcher::try_resolve_component_binary("server") {
        println!(
            "Starting ROCode {} server via {}",
            mode,
            launcher::resolve_binary_display(&path)
        );
    }
    launcher::run_server_foreground(&options).await
}

pub(crate) async fn run_web_command(
    port: u16,
    hostname: String,
    dir: Option<PathBuf>,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
) -> anyhow::Result<()> {
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
    let options = ServerLaunchOptions {
        port: bind_port,
        hostname,
        cwd: dir,
        web_dist,
        mdns,
        mdns_domain,
        cors: effective_cors,
    };
    let mut child =
        launcher::spawn_server_background(&options, Stdio::inherit(), Stdio::inherit())?;
    launcher::wait_for_server_ready(&backend_url, Duration::from_secs(90), Some(&mut child))
        .await?;
    launcher::try_open_browser(&launch_url);
    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("rocode-server exited with status {}", status);
    }
    Ok(())
}

pub(crate) async fn run_desktop_web_command(
    port: u16,
    hostname: String,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
) -> anyhow::Result<()> {
    let workspace_dir = launcher::resolve_desktop_workspace("rocode")?;
    launcher::remember_desktop_workspace("rocode", &workspace_dir);
    run_web_command(port, hostname, Some(workspace_dir), mdns, mdns_domain, cors).await
}

pub(crate) async fn run_acp_command(
    port: u16,
    hostname: String,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
    cwd: PathBuf,
) -> anyhow::Result<()> {
    std::env::set_current_dir(&cwd)
        .map_err(|e| anyhow::anyhow!("Failed to change directory to {}: {}", cwd.display(), e))?;

    if try_run_external_acp_bridge(port, &hostname, mdns, &mdns_domain, &cors, &cwd)? {
        return Ok(());
    }

    eprintln!(
        "Warning: no external ACP stdio bridge runtime found; falling back to HTTP server mode."
    );
    run_server_command("acp", port, hostname, Some(cwd), mdns, mdns_domain, cors).await
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
