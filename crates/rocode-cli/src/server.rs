use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::time::Duration;

use crate::util::server_url;

pub(crate) async fn wait_for_server_ready(
    base_url: &str,
    timeout: Duration,
    server_handle: Option<&mut tokio::task::JoinHandle<anyhow::Result<()>>>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let start = tokio::time::Instant::now();
    let health = server_url(base_url, "/health");
    let mut server_handle = server_handle;

    loop {
        if let Some(handle) = server_handle.as_mut() {
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok(())) => {
                        anyhow::bail!("Local server exited before becoming ready at {}", base_url)
                    }
                    Ok(Err(error)) => anyhow::bail!("Local server failed to start: {}", error),
                    Err(join_error) => anyhow::bail!("Local server task failed: {}", join_error),
                }
            }
        }

        if start.elapsed() > timeout {
            anyhow::bail!(
                "Timed out waiting for local server to start at {}",
                base_url
            );
        }
        if let Ok(resp) = client.get(&health).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn run_server_command(
    mode: &str,
    port: u16,
    hostname: String,
    dir: Option<PathBuf>,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
) -> anyhow::Result<()> {
    if let Some(workspace_dir) = dir.as_ref() {
        std::env::set_current_dir(workspace_dir).map_err(|error| {
            anyhow::anyhow!(
                "Failed to change workspace directory to {}: {}",
                workspace_dir.display(),
                error
            )
        })?;
    }

    if std::env::var("ROCODE_SERVER_PASSWORD")
        .or_else(|_| std::env::var("OPENCODE_SERVER_PASSWORD"))
        .is_err()
    {
        eprintln!(
            "Warning: ROCODE_SERVER_PASSWORD is not set; server is unsecured (legacy fallback: OPENCODE_SERVER_PASSWORD)."
        );
    }

    let bind_host = if mdns && hostname == "127.0.0.1" {
        "0.0.0.0".to_string()
    } else {
        hostname
    };
    let bind_port = if port == 0 { 3000 } else { port };
    rocode_server::set_cors_whitelist(cors);
    let _mdns_publisher = start_mdns_publisher_if_needed(mdns, &bind_host, bind_port, &mdns_domain);
    let addr: SocketAddr = format!("{}:{}", bind_host, bind_port).parse()?;
    if let Ok(cwd) = std::env::current_dir() {
        println!(
            "Starting ROCode {} server on {} (workspace: {})",
            mode,
            addr,
            cwd.display()
        );
    } else {
        println!("Starting ROCode {} server on {}", mode, addr);
    }
    rocode_server::run_server(addr).await?;
    Ok(())
}

pub(crate) fn try_open_browser(url: &str) {
    let mut candidates: Vec<Vec<String>> = Vec::new();
    if cfg!(target_os = "macos") {
        candidates.push(vec!["open".to_string(), url.to_string()]);
    } else if cfg!(target_os = "windows") {
        candidates.push(vec![
            "cmd".to_string(),
            "/C".to_string(),
            "start".to_string(),
            "".to_string(),
            url.to_string(),
        ]);
    } else {
        candidates.push(vec!["xdg-open".to_string(), url.to_string()]);
    }

    for cmd in candidates {
        if cmd.is_empty() {
            continue;
        }
        let launch_result = ProcessCommand::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if launch_result.is_ok() {
            return;
        }
    }
    eprintln!(
        "Could not auto-open browser. Open this URL manually: {}",
        url
    );
}

pub(crate) async fn run_web_command(
    port: u16,
    hostname: String,
    dir: Option<PathBuf>,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
) -> anyhow::Result<()> {
    if let Some(workspace_dir) = dir.as_ref() {
        std::env::set_current_dir(workspace_dir).map_err(|error| {
            anyhow::anyhow!(
                "Failed to change workspace directory to {}: {}",
                workspace_dir.display(),
                error
            )
        })?;
    }
    let bind_port = if port == 0 { 3000 } else { port };
    let display_host = if hostname == "0.0.0.0" {
        "localhost".to_string()
    } else {
        hostname.clone()
    };
    let url = format!("http://{}:{}", display_host, bind_port);
    println!("Web interface: {}", url);
    try_open_browser(&url);
    run_server_command("web", bind_port, hostname, None, mdns, mdns_domain, cors).await
}

fn desktop_state_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("rocode")
        .join("desktop")
}

fn desktop_workspace_hint_path() -> PathBuf {
    desktop_state_dir().join("last-workspace.txt")
}

fn remember_desktop_workspace(path: &Path) {
    if std::fs::create_dir_all(desktop_state_dir()).is_err() {
        return;
    }
    let _ = std::fs::write(desktop_workspace_hint_path(), path.display().to_string());
}

fn load_desktop_workspace_hint() -> Option<PathBuf> {
    let raw = std::fs::read_to_string(desktop_workspace_hint_path()).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_dir().then_some(path)
}

fn looks_like_workspace_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    [
        ".git",
        ".rocode",
        "rocode.json",
        "rocode.jsonc",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        ".workspace",
    ]
    .iter()
    .any(|entry| path.join(entry).exists())
}

fn choose_workspace_via_system_dialog() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let output = ProcessCommand::new("osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"Select a workspace folder for ROCode\")",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let path = PathBuf::from(selected);
        return path.is_dir().then_some(path);
    }

    if cfg!(target_os = "windows") {
        let script = "$app=New-Object -ComObject Shell.Application; $folder=$app.BrowseForFolder(0,'Select a workspace folder for ROCode',0,0); if($folder){$folder.Self.Path}";
        let output = ProcessCommand::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let path = PathBuf::from(selected);
        return path.is_dir().then_some(path);
    }

    let linux_candidates: [(&str, &[&str]); 2] = [
        (
            "zenity",
            &[
                "--file-selection",
                "--directory",
                "--title=Select a workspace folder for ROCode",
            ],
        ),
        (
            "kdialog",
            &[
                "--getexistingdirectory",
                ".",
                "--title",
                "Select a workspace folder for ROCode",
            ],
        ),
    ];

    for (program, args) in linux_candidates {
        let output = match ProcessCommand::new(program).args(args).output() {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if selected.is_empty() {
            continue;
        }
        let path = PathBuf::from(selected);
        if path.is_dir() {
            return Some(path);
        }
    }

    None
}

fn resolve_desktop_workspace() -> anyhow::Result<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        if looks_like_workspace_dir(&cwd) {
            return Ok(cwd);
        }
    }

    if let Some(path) = load_desktop_workspace_hint() {
        return Ok(path);
    }

    if let Some(path) = choose_workspace_via_system_dialog() {
        remember_desktop_workspace(&path);
        return Ok(path);
    }

    anyhow::bail!(
        "Could not determine a workspace for desktop launch. Start with `rocode web --dir <path>` or launch from inside a project directory."
    );
}

pub(crate) async fn run_desktop_web_command(
    port: u16,
    hostname: String,
    mdns: bool,
    mdns_domain: String,
    cors: Vec<String>,
) -> anyhow::Result<()> {
    let workspace_dir = resolve_desktop_workspace()?;
    remember_desktop_workspace(&workspace_dir);
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

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn service_name_from_mdns_domain(domain: &str, port: u16) -> String {
    let trimmed = domain
        .trim()
        .trim_end_matches('.')
        .trim_end_matches(".local");
    if trimmed.is_empty() {
        format!("rocode-{}", port)
    } else {
        trimmed.to_string()
    }
}

pub(crate) struct MdnsPublisher {
    child: Child,
}

impl Drop for MdnsPublisher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_mdns_command(command: &str, args: &[String]) -> io::Result<MdnsPublisher> {
    let mut child = ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Ok(Some(status)) = child.try_wait() {
        return Err(io::Error::other(format!(
            "mDNS publisher exited immediately with status {}",
            status
        )));
    }

    Ok(MdnsPublisher { child })
}

pub(crate) fn start_mdns_publisher_if_needed(
    enabled: bool,
    bind_host: &str,
    port: u16,
    mdns_domain: &str,
) -> Option<MdnsPublisher> {
    if !enabled {
        return None;
    }
    if is_loopback_host(bind_host) {
        eprintln!("Warning: mDNS enabled but hostname is loopback; skipping mDNS publish.");
        return None;
    }

    let service_name = service_name_from_mdns_domain(mdns_domain, port);
    let attempts: Vec<(String, Vec<String>)> = if cfg!(target_os = "macos") {
        vec![(
            "dns-sd".to_string(),
            vec![
                "-R".to_string(),
                service_name.clone(),
                "_http._tcp".to_string(),
                "local.".to_string(),
                port.to_string(),
                "path=/".to_string(),
            ],
        )]
    } else if cfg!(target_os = "linux") {
        vec![
            (
                "avahi-publish-service".to_string(),
                vec![
                    service_name.clone(),
                    "_http._tcp".to_string(),
                    port.to_string(),
                    "path=/".to_string(),
                ],
            ),
            (
                "avahi-publish".to_string(),
                vec![
                    "-s".to_string(),
                    service_name.clone(),
                    "_http._tcp".to_string(),
                    port.to_string(),
                    "path=/".to_string(),
                ],
            ),
        ]
    } else {
        eprintln!("Warning: mDNS requested but this platform has no configured publisher command.");
        return None;
    };

    let mut last_error: Option<String> = None;
    for (command, args) in attempts {
        match spawn_mdns_command(&command, &args) {
            Ok(publisher) => {
                eprintln!(
                    "mDNS publish enabled via `{}` as service `{}` on port {}.",
                    command, service_name, port
                );
                return Some(publisher);
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    last_error = Some(format!("{}: {}", command, err));
                }
            }
        }
    }

    if let Some(err) = last_error {
        eprintln!("Warning: failed to start mDNS publisher ({})", err);
    } else {
        eprintln!("Warning: mDNS requested but no supported publisher command was found on PATH.");
    }
    None
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
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
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
