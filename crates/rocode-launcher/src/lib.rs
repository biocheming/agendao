use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;

use anyhow::Context;
use reqwest::Client as HttpClient;
use tokio::process::{Child, Command};
use url::Url;

#[derive(Debug, Clone)]
pub struct ServerLaunchOptions {
    pub port: u16,
    pub hostname: String,
    pub cwd: Option<PathBuf>,
    pub web_dist: Option<PathBuf>,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
}

pub fn server_command(options: &ServerLaunchOptions) -> anyhow::Result<Command> {
    let mut command = Command::new(resolve_component_binary("server")?);
    command.arg("--port").arg(options.port.to_string());
    command.arg("--hostname").arg(&options.hostname);
    if let Some(cwd) = options.cwd.as_ref() {
        command.arg("--cwd").arg(cwd);
    }
    if let Some(web_dist) = options.web_dist.as_ref() {
        command.env("ROCODE_WEB_DIST", web_dist);
    }
    if options.mdns {
        command.arg("--mdns");
        command.arg("--mdns-domain").arg(&options.mdns_domain);
    }
    for origin in &options.cors {
        command.arg("--cors").arg(origin);
    }
    Ok(command)
}

pub async fn run_server_foreground(options: &ServerLaunchOptions) -> anyhow::Result<()> {
    let status = server_command(options)?
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("rocode-server exited with status {}", status);
    }
    Ok(())
}

pub fn spawn_server_background(
    options: &ServerLaunchOptions,
    stdout: Stdio,
    stderr: Stdio,
) -> anyhow::Result<Child> {
    let mut command = server_command(options)?;
    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    Ok(command.spawn()?)
}

pub async fn run_tui_foreground<I, K, V>(envs: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new(resolve_component_binary("tui")?);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .envs(envs);
    let status = command.status().await?;
    if !status.success() {
        anyhow::bail!("rocode-tui exited with status {}", status);
    }
    Ok(())
}

pub fn try_resolve_component_binary(component: &str) -> Option<PathBuf> {
    resolve_component_binary(component).ok()
}

pub fn resolve_web_dist_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = try_resolve_web_dist_dir() {
        return Ok(path);
    }

    anyhow::bail!(
        "ROCode Web frontend assets were not found. Build `apps/rocode-web` and set ROCODE_WEB_DIST, or install a package that includes `share/rocode/web`."
    )
}

pub fn try_resolve_web_dist_dir() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("ROCODE_WEB_DIST") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if has_web_dist(&path) {
                return Some(path);
            }
        }
    }

    let mut candidates = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            if dir.file_name().map(|name| name == "bin").unwrap_or(false) {
                if let Some(prefix) = dir.parent() {
                    candidates.push(prefix.join("share").join("rocode").join("web"));
                }
            }
            if dir.file_name().map(|name| name == "MacOS").unwrap_or(false) {
                if let Some(contents) = dir.parent() {
                    candidates.push(contents.join("Resources").join("web"));
                }
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("../../apps/rocode-web/dist"));

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("apps/rocode-web/dist"));
        candidates.push(current_dir.join("rocode/apps/rocode-web/dist"));
    }

    candidates.into_iter().find(|path| has_web_dist(path))
}

pub fn build_tui_env(
    base_url: &str,
    model: Option<String>,
    prompt: Option<String>,
    agent_name: Option<String>,
    session_id: Option<String>,
) -> Vec<(String, String)> {
    let mut envs = vec![("ROCODE_TUI_BASE_URL".to_string(), base_url.to_string())];
    if let Some(model) = model.filter(|value| !value.is_empty()) {
        envs.push(("ROCODE_TUI_MODEL".to_string(), model));
    }
    if let Some(prompt) = prompt.filter(|value| !value.is_empty()) {
        envs.push(("ROCODE_TUI_PROMPT".to_string(), prompt));
    }
    if let Some(agent_name) = agent_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        envs.push(("ROCODE_TUI_AGENT".to_string(), agent_name));
    }
    if let Some(session_id) = session_id.filter(|value| !value.is_empty()) {
        envs.push(("ROCODE_TUI_SESSION".to_string(), session_id));
    }
    envs
}

pub fn resolve_binary_display(path: &Path) -> String {
    path.display().to_string()
}

pub async fn wait_for_server_ready(
    base_url: &str,
    timeout: Duration,
    server_child: Option<&mut Child>,
) -> anyhow::Result<()> {
    let client = HttpClient::new();
    let start = tokio::time::Instant::now();
    let health = server_url(base_url, "/health");
    let mut server_child = server_child;

    loop {
        if let Some(child) = server_child.as_mut() {
            if let Some(status) = child.try_wait()? {
                anyhow::bail!(
                    "Local server exited before becoming ready at {} with status {}",
                    base_url,
                    status
                );
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

pub fn try_open_browser(url: &str) {
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

pub fn resolve_web_dev_url() -> anyhow::Result<Option<Url>> {
    let Ok(value) = std::env::var("ROCODE_WEB_DEV_URL") else {
        return Ok(None);
    };

    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("ROCODE_WEB_DEV_URL is set but empty");
    }

    let url = Url::parse(trimmed)
        .with_context(|| format!("Failed to parse ROCODE_WEB_DEV_URL `{trimmed}`"))?;
    match url.scheme() {
        "http" | "https" => Ok(Some(url)),
        other => anyhow::bail!(
            "ROCODE_WEB_DEV_URL must use http or https, got scheme `{}`",
            other
        ),
    }
}

pub fn append_browser_api_base(mut frontend_url: Url, backend_url: &str) -> String {
    let retained_pairs: Vec<(String, String)> = frontend_url
        .query_pairs()
        .filter(|(key, _)| key != "api_base_url")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();

    {
        let mut pairs = frontend_url.query_pairs_mut();
        pairs.clear();
        for (key, value) in retained_pairs {
            pairs.append_pair(&key, &value);
        }
        pairs.append_pair("api_base_url", backend_url);
    }

    frontend_url.to_string()
}

pub fn push_origin_if_missing(cors: &mut Vec<String>, url: &Url) {
    let origin = url.origin().ascii_serialization();
    if origin == "null"
        || cors
            .iter()
            .any(|value| value.trim_end_matches('/') == origin)
    {
        return;
    }

    cors.push(origin);
}

pub fn remember_desktop_workspace(app_name: &str, path: &Path) {
    let Some(state_dir) = desktop_state_dir(app_name) else {
        return;
    };
    if std::fs::create_dir_all(&state_dir).is_err() {
        return;
    }
    let _ = std::fs::write(
        state_dir.join("last-workspace.txt"),
        path.display().to_string(),
    );
}

pub fn resolve_desktop_workspace(app_name: &str) -> anyhow::Result<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        if looks_like_workspace_dir(&cwd) {
            return Ok(cwd);
        }
    }

    if let Some(path) = load_desktop_workspace_hint(app_name) {
        return Ok(path);
    }

    if let Some(path) = choose_workspace_via_system_dialog(app_name) {
        remember_desktop_workspace(app_name, &path);
        return Ok(path);
    }

    anyhow::bail!(
        "Could not determine a workspace for desktop launch. Start with `rocode web --dir <path>` or launch from inside a project directory."
    );
}

fn resolve_component_binary(component: &str) -> anyhow::Result<PathBuf> {
    let env_key = format!("ROCODE_{}_BIN", component.to_ascii_uppercase());
    if let Ok(value) = std::env::var(&env_key) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{env_key} is set but empty");
        }
        let path = PathBuf::from(trimmed);
        if !path.is_file() {
            anyhow::bail!(
                "{env_key} points to `{}`, but that file does not exist.",
                path.display()
            );
        }
        return Ok(path);
    }

    let binary_name = format!("rocode-{}{}", component, std::env::consts::EXE_SUFFIX);
    let sibling_candidate = std::env::current_exe()
        .ok()
        .and_then(|current_exe| current_exe.parent().map(|dir| dir.join(&binary_name)));
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let sibling = dir.join(&binary_name);
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }

    if let Ok(path) = which::which(&binary_name) {
        return Ok(path);
    }

    let sibling_display = sibling_candidate
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    let install_hint = match component {
        "server" | "tui" => {
            "Build or install the side-by-side ROCode binaries, for example `./scripts/install-local.sh release ~/.local`."
        }
        _ => "Build or install the matching ROCode component binary.",
    };
    anyhow::bail!(
        "Required launcher target `{}` was not found. Checked sibling path `{}` and PATH. {} You can also set {} explicitly.",
        binary_name,
        sibling_display,
        install_hint,
        env_key
    )
}

fn has_web_dist(path: &Path) -> bool {
    path.join("index.html").is_file()
        && path.join("app.js").is_file()
        && path.join("app.css").is_file()
}

fn server_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn desktop_state_dir(app_name: &str) -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join(app_name)
            .join("desktop"),
    )
}

fn load_desktop_workspace_hint(app_name: &str) -> Option<PathBuf> {
    let state_dir = desktop_state_dir(app_name)?;
    let raw = std::fs::read_to_string(state_dir.join("last-workspace.txt")).ok()?;
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

fn choose_workspace_via_system_dialog(app_name: &str) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let output = ProcessCommand::new("osascript")
            .args([
                "-e",
                &format!(
                    "POSIX path of (choose folder with prompt \"Select a workspace folder for {}\")",
                    app_name.to_ascii_uppercase()
                ),
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
        let script = format!(
            "$app=New-Object -ComObject Shell.Application; $folder=$app.BrowseForFolder(0,'Select a workspace folder for {}',0,0); if($folder){{$folder.Self.Path}}",
            app_name.to_ascii_uppercase()
        );
        let output = ProcessCommand::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let path = PathBuf::from(selected);
        return path.is_dir().then_some(path);
    }

    let zenity_title = format!(
        "--title=Select a workspace folder for {}",
        app_name.to_ascii_uppercase()
    );
    let kdialog_title = format!(
        "Select a workspace folder for {}",
        app_name.to_ascii_uppercase()
    );
    let linux_candidates: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--file-selection".to_string(),
                "--directory".to_string(),
                zenity_title,
            ],
        ),
        (
            "kdialog",
            vec![
                "--getexistingdirectory".to_string(),
                ".".to_string(),
                "--title".to_string(),
                kdialog_title,
            ],
        ),
    ];

    for (program, args) in linux_candidates {
        let output = match ProcessCommand::new(program).args(&args).output() {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
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
