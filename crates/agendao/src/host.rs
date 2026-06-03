use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agendao_launcher as launcher;
use agendao_server::ServerRuntimeOptions;
use agendao_tui::AppLaunchConfig;

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
    pub unix_socket_path: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ServerCommandRequest {
    pub port: u16,
    pub hostname: String,
    pub dir: Option<std::path::PathBuf>,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    pub unix_socket_path: Option<String>,
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
pub struct AcpCommandRequest {
    pub port: u16,
    pub hostname: String,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    pub cwd: std::path::PathBuf,
}

const DEFAULT_SERVER_PORT: u16 = 3000;
const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

pub fn frontend_runtime_context() -> agendao_cli::FrontendRuntimeContext {
    let auto_started_server = Arc::new(AutoStartedServerOwner::default());
    agendao_cli::FrontendRuntimeContext::new(move |request| {
        let auto_started_server = auto_started_server.clone();
        Box::pin(async move { discover_or_start_local_server(request, auto_started_server).await })
    })
}

#[derive(Default)]
struct AutoStartedServerOwner {
    child: Mutex<Option<tokio::process::Child>>,
    base_url: Mutex<Option<String>>,
}

impl AutoStartedServerOwner {
    fn take_if_matches(&self, base_url: &str) -> Option<tokio::process::Child> {
        let matches = self
            .base_url
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
            .is_some_and(|existing| existing == base_url);
        if !matches {
            return None;
        }
        if let Ok(mut base_url_guard) = self.base_url.lock() {
            *base_url_guard = None;
        }
        self.child.lock().ok().and_then(|mut guard| guard.take())
    }

    fn replace(&self, base_url: String, child: tokio::process::Child) {
        if let Ok(mut guard) = self.child.lock() {
            *guard = Some(child);
        }
        if let Ok(mut guard) = self.base_url.lock() {
            *guard = Some(base_url);
        }
    }
}

impl Drop for AutoStartedServerOwner {
    fn drop(&mut self) {
        let Some(mut child) = self.child.lock().ok().and_then(|mut guard| guard.take()) else {
            return;
        };
        if let Err(error) = child.start_kill() {
            tracing::debug!(%error, "failed to kill auto-started local server on frontend exit");
        }
    }
}

pub async fn run_server_command(request: ServerCommandRequest) -> anyhow::Result<()> {
    agendao_server::run_server_runtime(ServerRuntimeOptions {
        port: request.port,
        hostname: request.hostname,
        cwd: request.dir,
        web_dist: None,
        embedded_web_assets: Some(launcher::embedded_web_asset),
        mdns: request.mdns,
        mdns_domain: request.mdns_domain,
        cors: request.cors,
        unix_socket_path: request.unix_socket_path,
    })
    .await
}

pub async fn run_web_command(request: WebCommandRequest) -> anyhow::Result<()> {
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
    } else if let Some(web_dist) = launcher::try_resolve_web_dist_override() {
        println!("Web assets override: {}", web_dist.display());
        Some(web_dist)
    } else {
        println!("Web assets: embedded");
        None
    };
    let launch_url = if let Some(dev_url) = web_dev_url {
        launcher::append_browser_api_base(dev_url, &backend_url)
    } else {
        backend_url.clone()
    };
    let launch_url = append_server_password_query(launch_url, current_server_password().as_deref());
    let display_launch_url = redact_server_password_query(&launch_url);
    println!("Backend API: {}", backend_url);
    println!("Web interface: {}", display_launch_url);

    let server_task = tokio::spawn(agendao_server::run_server_runtime(ServerRuntimeOptions {
        port: bind_port,
        hostname,
        cwd: dir,
        web_dist,
        embedded_web_assets: Some(launcher::embedded_web_asset),
        mdns,
        mdns_domain,
        cors: effective_cors,
        unix_socket_path: None,
    }));

    launcher::wait_for_server_ready(&backend_url, Duration::from_secs(90), None).await?;
    launcher::try_open_browser(&launch_url);
    server_task.await?
}

pub async fn run_tui(request: TuiCommandRequest) -> anyhow::Result<()> {
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
        password,
        unix_socket_path,
    } = request;

    if fork && !continue_last && session.is_none() {
        anyhow::bail!("--fork requires --continue or --session");
    }

    let working_dir = project.clone();
    let server_password = password.or_else(current_server_password);
    let use_http = attach_url.is_some();
    let use_socket = !use_http && unix_socket_path.is_some();
    let use_direct = !use_http && !use_socket;

    if use_direct {
        eprintln!("Starting TUI in Direct (in-process) mode");
        let local_server = create_local_server_state(working_dir.clone()).await?;
        let selected_session = resolve_requested_session_local(
            &local_server,
            continue_last,
            session,
            fork,
        )
        .await?;
        let run_result = tokio::task::spawn_blocking(move || {
            agendao_tui::run_tui_with_config(AppLaunchConfig {
                base_url: None,
                server_password: None,
                model,
                initial_prompt: prompt,
                agent_name: agent,
                session_id: selected_session,
                working_dir,
                unix_socket_path: None,
                local_direct: true,
                local_server: Some(local_server),
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("agendao-tui task failed to join: {}", error))?;
        return run_result;
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
        if let Some(socket_path) = unix_socket_path.as_deref() {
            eprintln!(
                "Starting local server for TUI at {} with Unix socket {}",
                server_url, socket_path
            );
        } else {
            eprintln!("Starting local server for TUI at {}", server_url);
        }
        server_task = Some(tokio::spawn(agendao_server::run_server_runtime(
            ServerRuntimeOptions {
                port: bind_port,
                hostname,
                cwd: working_dir.clone(),
                web_dist: None,
                embedded_web_assets: Some(launcher::embedded_web_asset),
                mdns,
                mdns_domain,
                cors,
                unix_socket_path: unix_socket_path.clone(),
            },
        )));
        launcher::wait_for_server_ready(&server_url, Duration::from_secs(90), None).await?;
        server_url
    };

    let selected_session = resolve_requested_session(
        &base_url,
        server_password.clone(),
        continue_last,
        session,
        fork,
        unix_socket_path.as_deref(),
    )
    .await?;
    // agendao-tui creates and drives its own Tokio runtime internally.
    // Run it on a blocking thread so we do not try to nest runtimes inside
    // the product shell's async runtime.
    let run_result = tokio::task::spawn_blocking(move || {
            agendao_tui::run_tui_with_config(AppLaunchConfig {
                base_url: Some(base_url),
                server_password,
                model,
                initial_prompt: prompt,
                agent_name: agent,
                session_id: selected_session,
                working_dir,
                unix_socket_path,
                local_direct: false,
                local_server: None,
            })
        })
    .await
    .map_err(|error| anyhow::anyhow!("agendao-tui task failed to join: {}", error))?;

    if let Some(server_task) = server_task {
        server_task.abort();
        let _ = server_task.await;
    }

    run_result
}

async fn create_local_server_state(
    working_dir: Option<std::path::PathBuf>,
) -> anyhow::Result<Arc<agendao_server::ServerState>> {
    let workspace_root = match working_dir {
        Some(dir) => dir.canonicalize().unwrap_or(dir),
        None => std::env::current_dir()?,
    };
    Ok(Arc::new(
        agendao_server::ServerState::new_with_storage_for_url_in_workspace(
            "http://127.0.0.1:0".to_string(),
            workspace_root,
        )
        .await?,
    ))
}

async fn resolve_requested_session_local(
    state: &Arc<agendao_server::ServerState>,
    continue_last: bool,
    session: Option<String>,
    fork: bool,
) -> anyhow::Result<Option<String>> {
    let selected = if let Some(session_id) = session {
        Some(session_id)
    } else if continue_last {
        agendao_server::local_list_sessions(Arc::clone(state), None, Some(100))
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

    let forked =
        agendao_server::local_fork_session(Arc::clone(state), &session_id, None).await?;
    eprintln!("Forked session {} -> {}", session_id, forked.id);
    Ok(Some(forked.id))
}

async fn resolve_requested_session(
    base_url: &str,
    server_password: Option<String>,
    continue_last: bool,
    session: Option<String>,
    fork: bool,
    unix_socket_path: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let transport = match unix_socket_path {
        Some(socket_path) => {
            let selector = agendao_client::transport::TransportSelector::new(
                Some(socket_path.to_string()),
                base_url.to_string(),
                server_password.clone(),
            );
            Some(selector.select_unix_required().await?)
        }
        None => None,
    };

    let api_client =
        agendao_client::AsyncApiClient::new_with_password(base_url.to_string(), server_password);
    let selected = if let Some(session_id) = session {
        Some(session_id)
    } else if continue_last {
        let sessions = if let Some(ref t) = transport {
            t.list_sessions().await?
        } else {
            vec![]
        };
        let sessions = if sessions.is_empty() {
            api_client.list_sessions(None, Some(100)).await?
        } else {
            sessions
        };
        sessions
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

fn current_server_password() -> Option<String> {
    std::env::var("AGENDAO_SERVER_PASSWORD")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn append_server_password_query(launch_url: String, server_password: Option<&str>) -> String {
    let Some(server_password) = server_password
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return launch_url;
    };
    let Ok(mut url) = reqwest::Url::parse(&launch_url) else {
        return launch_url;
    };
    if !url.query_pairs().any(|(key, _)| key == "server_password") {
        url.query_pairs_mut()
            .append_pair("server_password", server_password);
    }
    url.to_string()
}

fn redact_server_password_query(launch_url: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(launch_url) else {
        return launch_url.to_string();
    };
    let retained_pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| {
            if key == "server_password" {
                (key.into_owned(), "<redacted>".to_string())
            } else {
                (key.into_owned(), value.into_owned())
            }
        })
        .collect();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.clear();
        for (key, value) in retained_pairs {
            pairs.append_pair(&key, &value);
        }
    }
    url.to_string()
}

pub async fn run_acp_command(request: AcpCommandRequest) -> anyhow::Result<()> {
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
        port,
        hostname,
        dir: Some(cwd),
        mdns,
        mdns_domain,
        cors,
        unix_socket_path: None,
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

#[cfg(test)]
mod tests {
    use super::{
        append_server_password_query, redact_server_password_query, AutoStartedServerOwner,
    };
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    #[test]
    fn append_server_password_query_adds_password_without_overwriting_existing_value() {
        assert_eq!(
            append_server_password_query("http://localhost:3000".to_string(), Some("secret")),
            "http://localhost:3000/?server_password=secret"
        );
        assert_eq!(
            append_server_password_query(
                "http://localhost:3000/?server_password=existing".to_string(),
                Some("secret"),
            ),
            "http://localhost:3000/?server_password=existing"
        );
    }

    #[test]
    fn redact_server_password_query_hides_password_for_display() {
        assert_eq!(
            redact_server_password_query("http://localhost:3000/?server_password=secret&x=1"),
            "http://localhost:3000/?server_password=%3Credacted%3E&x=1"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auto_started_server_owner_drop_kills_owned_child() {
        use std::os::raw::c_int;

        unsafe extern "C" {
            fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
        }

        const WNOHANG: c_int = 1;

        let owner = AutoStartedServerOwner::default();
        let child = tokio::process::Command::new("sh")
            .kill_on_drop(true)
            .arg("-c")
            .arg("sleep 30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id().expect("child pid") as c_int;
        owner.replace("http://127.0.0.1:3000".to_string(), child);

        drop(owner);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut status = 0;
            let result = unsafe { waitpid(pid, &mut status, WNOHANG) };
            if result == pid {
                return;
            }
            if result == -1 {
                let errno = std::io::Error::last_os_error().raw_os_error();
                if errno == Some(10) {
                    return;
                }
                panic!("waitpid failed for auto-started child {pid}: errno={errno:?}");
            }
            assert!(
                Instant::now() < deadline,
                "auto-started child {pid} was not killed when owner dropped"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
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
        .env("AGENDAO_ACP_BRIDGE_ACTIVE", "1");

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

async fn discover_or_start_local_server(
    request: agendao_cli::ServerDiscoveryRequest,
    auto_started_server: Arc<AutoStartedServerOwner>,
) -> anyhow::Result<String> {
    let base_url = resolve_server_url(request.port_override);

    if health_check(&base_url).await.is_ok() {
        tracing::info!("Connected to existing server at {}", base_url);
        return Ok(base_url);
    }

    let port = request.port_override.unwrap_or(DEFAULT_SERVER_PORT);
    tracing::info!(
        "No server found — starting local server on 127.0.0.1:{}",
        port
    );
    let current_exe = std::env::current_exe()?;
    let mut child = tokio::process::Command::new(current_exe)
        .kill_on_drop(true)
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--hostname")
        .arg("127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .args(
            request
                .cwd
                .as_ref()
                .map(|cwd| vec!["--dir".to_string(), cwd.display().to_string()])
                .unwrap_or_default(),
        )
        .spawn()?;

    launcher::wait_for_server_ready(&base_url, SERVER_STARTUP_TIMEOUT, Some(&mut child)).await?;
    if let Some(mut previous_child) = auto_started_server.take_if_matches(&base_url) {
        if let Err(error) = previous_child.start_kill() {
            tracing::debug!(%error, "failed to replace stale auto-started local server");
        }
    }
    auto_started_server.replace(base_url.clone(), child);

    tracing::info!("Local server ready at {}", base_url);
    Ok(base_url)
}

fn resolve_server_url(port_override: Option<u16>) -> String {
    if let Ok(url) = std::env::var("AGENDAO_SERVER_URL") {
        let url = url.trim().to_string();
        if !url.is_empty() {
            return url;
        }
    }

    if let Ok(url) = std::env::var("AGENDAO_TUI_BASE_URL") {
        let url = url.trim().to_string();
        if !url.is_empty() {
            return url;
        }
    }

    let port = port_override.unwrap_or(DEFAULT_SERVER_PORT);
    format!("http://127.0.0.1:{}", port)
}

async fn health_check(base_url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let resp = client
        .get(format!("{}/health", base_url.trim_end_matches('/')))
        .send()
        .await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("Health check failed: {}", resp.status());
    }
}

fn try_run_external_acp_bridge(
    port: u16,
    hostname: &str,
    mdns: bool,
    mdns_domain: &str,
    cors: &[String],
    cwd: &Path,
) -> anyhow::Result<bool> {
    if std::env::var("AGENDAO_ACP_BRIDGE_ACTIVE").ok().as_deref() == Some("1") {
        return Ok(false);
    }

    let acp_args = build_acp_network_args(port, hostname, mdns, mdns_domain, cors, cwd);

    if let Ok(bin) = std::env::var("AGENDAO_ACP_BRIDGE_BIN") {
        let bin = bin.trim();
        if bin.is_empty() {
            anyhow::bail!("AGENDAO_ACP_BRIDGE_BIN is set but empty.");
        }
        return run_acp_bridge_candidate(bin, &acp_args, None);
    }

    if let Ok(agendao_path) = which::which("agendao") {
        let is_self = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                let lhs = std::fs::canonicalize(exe).ok()?;
                let rhs = std::fs::canonicalize(agendao_path).ok()?;
                Some(lhs == rhs)
            })
            .unwrap_or(false);
        if !is_self && run_acp_bridge_candidate("agendao", &acp_args, None)? {
            return Ok(true);
        }
    }

    Ok(false)
}
