use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use clap::{Args, Parser, Subcommand};
use rocode_client::transport::TransportSelector;

use crate::host::{
    run_acp_command, run_server_command, run_tui, run_web_command, AcpCommandRequest,
    ServerCommandRequest, TuiCommandRequest, WebCommandRequest,
};

const MANAGED_BINARIES: [&str; 1] = ["rocode"];

#[derive(Parser)]
#[command(name = "rocode")]
struct ProductCli {
    #[command(subcommand)]
    command: Option<ProductCommand>,
}

#[derive(Subcommand)]
enum ProductCommand {
    #[command(about = "Start interactive TUI session")]
    Tui(TuiArgs),
    #[command(about = "Attach TUI to a running ROCode server")]
    Attach(AttachArgs),
    #[command(about = "Start HTTP server")]
    Serve(ServerArgs),
    #[command(about = "Start headless server and open web interface")]
    Web(WebArgs),
    #[command(about = "Start ACP (Agent Client Protocol) server")]
    Acp(AcpArgs),
    #[command(about = "Generate OpenAPI specification JSON")]
    Generate,
    #[command(about = "Upgrade rocode to latest or specific version")]
    Upgrade {
        #[arg(value_name = "TARGET")]
        target: Option<String>,
        #[arg(short = 'm', long)]
        method: Option<String>,
    },
    #[command(about = "Uninstall rocode and remove related files")]
    Uninstall {
        #[arg(short = 'c', long = "keep-config", default_value_t = false)]
        keep_config: bool,
        #[arg(short = 'd', long = "keep-data", default_value_t = false)]
        keep_data: bool,
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
        #[arg(short = 'f', long, default_value_t = false)]
        force: bool,
    },
    #[command(about = "Show version")]
    Version,
    #[command(about = "Show build and environment info (compiler, target, profile)")]
    Info,
}

#[derive(Args)]
struct TuiArgs {
    #[arg(value_name = "PROJECT")]
    project: Option<PathBuf>,
    #[arg(short = 'm', long)]
    model: Option<String>,
    #[arg(short = 'c', long = "continue", default_value_t = false)]
    continue_last: bool,
    #[arg(short = 's', long)]
    session: Option<String>,
    #[arg(long, default_value_t = false)]
    fork: bool,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    agent: Option<String>,
    /// Explicitly attach over HTTP instead of the default Direct mode.
    #[arg(long = "attach-url")]
    attach_url: Option<String>,
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    hostname: String,
    #[arg(long, default_value_t = false)]
    mdns: bool,
    #[arg(long = "mdns-domain", default_value = "rocode.local")]
    mdns_domain: String,
    #[arg(long)]
    cors: Vec<String>,
    /// Force Direct (in-process) mode. This is already the default unless
    /// `--socket` or `--attach-url` is provided.
    #[arg(long, default_value_t = false)]
    local: bool,
    /// Use the standard local Unix socket path instead of Direct mode.
    #[arg(long = "socket", alias = "unix-socket", default_value_t = false)]
    socket: bool,
}

#[derive(Args)]
struct AttachArgs {
    #[arg(value_name = "URL")]
    url: String,
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(short = 's', long)]
    session: Option<String>,
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// Prefer Unix socket IPC using the standard local socket path.
    #[arg(long = "socket", alias = "unix-socket", default_value_t = false)]
    socket: bool,
}

#[derive(Args)]
struct ServerArgs {
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    hostname: String,
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    mdns: bool,
    #[arg(long = "mdns-domain", default_value = "rocode.local")]
    mdns_domain: String,
    #[arg(long)]
    cors: Vec<String>,
    /// Also listen on the standard local Unix socket path.
    #[arg(long = "socket", alias = "unix-socket", default_value_t = false)]
    socket: bool,
}

#[derive(Args)]
struct WebArgs {
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    hostname: String,
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    mdns: bool,
    #[arg(long = "mdns-domain", default_value = "rocode.local")]
    mdns_domain: String,
    #[arg(long)]
    cors: Vec<String>,
}

#[derive(Args)]
struct AcpArgs {
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    hostname: String,
    #[arg(long, default_value_t = false)]
    mdns: bool,
    #[arg(long = "mdns-domain", default_value = "rocode.local")]
    mdns_domain: String,
    #[arg(long)]
    cors: Vec<String>,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallMethod {
    Local,
    Curl,
    Npm,
    Pnpm,
    Bun,
    Brew,
    Choco,
    Scoop,
    Unknown,
}

fn normalize_tui_shorthand_args(args: Vec<OsString>) -> Vec<OsString> {
    let Some(first_arg) = args.get(1).and_then(|value| value.to_str()) else {
        return args;
    };
    if !matches!(first_arg, "-s" | "--session" | "-c" | "--continue") {
        return args;
    }

    let mut normalized = Vec::with_capacity(args.len() + 1);
    let mut iter = args.into_iter();
    if let Some(bin) = iter.next() {
        normalized.push(bin);
    }
    normalized.push(OsString::from("tui"));
    normalized.extend(iter);
    normalized
}

fn resolve_socket_path(enabled: bool) -> anyhow::Result<Option<String>> {
    if !enabled {
        return Ok(None);
    }

    TransportSelector::default_unix_socket_path().map(Some).ok_or_else(|| {
        anyhow::anyhow!("--socket is not supported on this platform")
    })
}

impl InstallMethod {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Self::Local,
            "curl" => Self::Curl,
            "npm" => Self::Npm,
            "pnpm" => Self::Pnpm,
            "bun" => Self::Bun,
            "brew" => Self::Brew,
            "choco" => Self::Choco,
            "scoop" => Self::Scoop,
            _ => Self::Unknown,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Curl => "curl",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Bun => "bun",
            Self::Brew => "brew",
            Self::Choco => "choco",
            Self::Scoop => "scoop",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
struct RemovalTarget {
    label: String,
    path: PathBuf,
}

pub async fn dispatch_if_product_command(args: Vec<OsString>) -> anyhow::Result<bool> {
    let args = normalize_tui_shorthand_args(args);
    let Some(command) = args.get(1).and_then(|value| value.to_str()) else {
        run_tui(default_tui_request()).await?;
        return Ok(true);
    };

    if !matches!(
        command,
        "tui"
            | "attach"
            | "serve"
            | "web"
            | "acp"
            | "generate"
            | "upgrade"
            | "uninstall"
            | "version"
            | "info"
    ) {
        return Ok(false);
    }

    let cli = ProductCli::parse_from(args);
    match cli.command {
        None => {
            run_tui(default_tui_request()).await?;
        }
        Some(ProductCommand::Tui(args)) => {
            let unix_socket_path = resolve_socket_path(args.socket)?;
            run_tui(TuiCommandRequest {
                project: args.project,
                model: args.model,
                continue_last: args.continue_last,
                session: args.session,
                fork: args.fork,
                prompt: args.prompt,
                agent: args.agent,
                port: args.port,
                hostname: args.hostname,
                mdns: args.mdns,
                mdns_domain: args.mdns_domain,
                cors: args.cors,
                attach_url: args.attach_url,
                password: None,
                unix_socket_path,
            })
            .await?;
        }
        Some(ProductCommand::Attach(args)) => {
            run_tui(TuiCommandRequest {
                project: args.dir,
                model: None,
                continue_last: false,
                session: args.session,
                fork: false,
                prompt: None,
                agent: None,
                port: 0,
                hostname: "127.0.0.1".to_string(),
                mdns: false,
                mdns_domain: "rocode.local".to_string(),
                cors: vec![],
                attach_url: Some(args.url),
                password: args.password,
                unix_socket_path: resolve_socket_path(args.socket)?,
            })
            .await?;
        }
        Some(ProductCommand::Serve(args)) => {
            run_server_command(ServerCommandRequest {
                port: args.port,
                hostname: args.hostname,
                dir: args.dir,
                mdns: args.mdns,
                mdns_domain: args.mdns_domain,
                cors: args.cors,
                unix_socket_path: resolve_socket_path(args.socket)?,
            })
            .await?;
        }
        Some(ProductCommand::Web(args)) => {
            run_web_command(WebCommandRequest {
                port: args.port,
                hostname: args.hostname,
                dir: args.dir,
                mdns: args.mdns,
                mdns_domain: args.mdns_domain,
                cors: args.cors,
            })
            .await?;
        }
        Some(ProductCommand::Acp(args)) => {
            run_acp_command(AcpCommandRequest {
                port: args.port,
                hostname: args.hostname,
                mdns: args.mdns,
                mdns_domain: args.mdns_domain,
                cors: args.cors,
                cwd: args.cwd,
            })
            .await?;
        }
        Some(ProductCommand::Generate) => {
            rocode_server::print_openapi_spec().await?;
        }
        Some(ProductCommand::Upgrade { target, method }) => {
            handle_upgrade_command(target, method).await?;
        }
        Some(ProductCommand::Uninstall {
            keep_config,
            keep_data,
            dry_run,
            force,
        }) => {
            handle_uninstall_command(keep_config, keep_data, dry_run, force).await?;
        }
        Some(ProductCommand::Version) => {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
        Some(ProductCommand::Info) => {
            print_build_info();
        }
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: Vec<&str>) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }

    fn display_args(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn normalize_tui_shorthand_routes_session_to_tui() {
        let normalized = normalize_tui_shorthand_args(strings(vec!["rocode", "-s", "ses_123"]));

        assert_eq!(
            display_args(normalized),
            vec!["rocode", "tui", "-s", "ses_123"]
        );
    }

    #[test]
    fn normalize_tui_shorthand_leaves_cli_run_untouched() {
        let normalized =
            normalize_tui_shorthand_args(strings(vec!["rocode", "run", "-s", "ses_123"]));

        assert_eq!(
            display_args(normalized),
            vec!["rocode", "run", "-s", "ses_123"]
        );
    }

    #[test]
    fn socket_flag_enables_default_socket_for_tui() {
        let cli = ProductCli::parse_from(["rocode", "tui", "--socket"]);
        let ProductCommand::Tui(args) = cli.command.expect("tui command") else {
            panic!("expected tui command");
        };

        assert!(args.socket);
        let socket_path = resolve_socket_path(args.socket);
        #[cfg(unix)]
        assert_eq!(socket_path.unwrap().as_deref(), Some("/tmp/rocode.sock"));
        #[cfg(not(unix))]
        assert!(socket_path.is_err());
    }

    #[test]
    fn tui_accepts_local_with_socket_override() {
        let cli = ProductCli::parse_from(["rocode", "tui", "--local", "--socket"]);
        let ProductCommand::Tui(args) = cli.command.expect("tui command") else {
            panic!("expected tui command");
        };
        assert!(args.local);
        assert!(args.socket);
    }

    #[test]
    fn socket_flag_enables_default_socket_for_attach() {
        let cli =
            ProductCli::parse_from(["rocode", "attach", "http://127.0.0.1:3000", "--socket"]);
        let ProductCommand::Attach(args) = cli.command.expect("attach command") else {
            panic!("expected attach command");
        };

        assert!(args.socket);
        let socket_path = resolve_socket_path(args.socket);
        #[cfg(unix)]
        assert_eq!(socket_path.unwrap().as_deref(), Some("/tmp/rocode.sock"));
        #[cfg(not(unix))]
        assert!(socket_path.is_err());
    }

    #[test]
    fn socket_flag_enables_default_socket_for_serve() {
        let cli = ProductCli::parse_from(["rocode", "serve", "--socket"]);
        let ProductCommand::Serve(args) = cli.command.expect("serve command") else {
            panic!("expected serve command");
        };

        assert!(args.socket);
        let socket_path = resolve_socket_path(args.socket);
        #[cfg(unix)]
        assert_eq!(socket_path.unwrap().as_deref(), Some("/tmp/rocode.sock"));
        #[cfg(not(unix))]
        assert!(socket_path.is_err());
    }
}

fn default_tui_request() -> TuiCommandRequest {
    TuiCommandRequest {
        project: None, model: None, continue_last: false, session: None, fork: false,
        prompt: None, agent: None, port: 0, hostname: "127.0.0.1".to_string(),
        mdns: false, mdns_domain: "rocode.local".to_string(), cors: vec![],
        attach_url: None, password: None, unix_socket_path: None,
    }
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = ProcessCommand::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn binary_filename(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn has_managed_binary_set(dir: &Path) -> bool {
    MANAGED_BINARIES
        .iter()
        .all(|name| dir.join(binary_filename(name)).is_file())
}

fn managed_binary_dir() -> Option<PathBuf> {
    let dir = current_exe_dir()?;
    let dir_name = dir.file_name()?.to_string_lossy();
    if dir_name != "bin" || !has_managed_binary_set(&dir) {
        return None;
    }
    Some(dir)
}

fn managed_bundle_root() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let macos_dir = current_exe.parent()?;
    if macos_dir.file_name()?.to_string_lossy() != "MacOS" {
        return None;
    }
    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()?.to_string_lossy() != "Contents" {
        return None;
    }
    let app_dir = contents_dir.parent()?;
    if !app_dir
        .extension()
        .map(|value| value == "app")
        .unwrap_or(false)
    {
        return None;
    }
    Some(app_dir.to_path_buf())
}

fn managed_web_asset_dir() -> Option<PathBuf> {
    if let Some(bin_dir) = managed_binary_dir() {
        if let Some(prefix) = bin_dir.parent() {
            return Some(prefix.join("share").join("rocode").join("web"));
        }
    }
    None
}

fn detect_install_method() -> InstallMethod {
    if managed_binary_dir().is_some() || managed_bundle_root().is_some() {
        return InstallMethod::Local;
    }

    let checks: &[(InstallMethod, &str, &[&str], &str)] = &[
        (
            InstallMethod::Npm,
            "npm",
            &["list", "-g", "--depth=0"],
            "rocode-ai",
        ),
        (
            InstallMethod::Pnpm,
            "pnpm",
            &["list", "-g", "--depth=0"],
            "rocode-ai",
        ),
        (InstallMethod::Bun, "bun", &["pm", "ls", "-g"], "rocode-ai"),
        (
            InstallMethod::Brew,
            "brew",
            &["list", "--formula", "rocode"],
            "rocode",
        ),
        (
            InstallMethod::Choco,
            "choco",
            &["list", "--limit-output", "rocode"],
            "rocode",
        ),
        (InstallMethod::Scoop, "scoop", &["list", "rocode"], "rocode"),
    ];

    for (method, program, args, marker) in checks {
        if let Some(text) = command_text(program, args) {
            if text.to_ascii_lowercase().contains(marker) {
                return *method;
            }
        }
    }

    InstallMethod::Unknown
}

async fn latest_version(method: InstallMethod) -> anyhow::Result<String> {
    let client = reqwest::Client::new();

    match method {
        InstallMethod::Local
        | InstallMethod::Curl
        | InstallMethod::Choco
        | InstallMethod::Unknown => {
            let json: serde_json::Value = client
                .get("https://api.github.com/repos/anomalyco/rocode/releases/latest")
                .header("User-Agent", "rocode")
                .send()
                .await?
                .json()
                .await?;
            let tag = json
                .get("tag_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse latest GitHub release"))?;
            Ok(tag.trim_start_matches('v').to_string())
        }
        InstallMethod::Brew => {
            let json: serde_json::Value = client
                .get("https://formulae.brew.sh/api/formula/rocode.json")
                .send()
                .await?
                .json()
                .await?;
            let version = json
                .get("versions")
                .and_then(|v| v.get("stable"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse brew stable version"))?;
            Ok(version.to_string())
        }
        InstallMethod::Npm | InstallMethod::Pnpm | InstallMethod::Bun => {
            let json: serde_json::Value = client
                .get("https://registry.npmjs.org/rocode-ai/latest")
                .send()
                .await?
                .json()
                .await?;
            let version = json
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse npm latest version"))?;
            Ok(version.to_string())
        }
        InstallMethod::Scoop => {
            let json: serde_json::Value = client
                .get("https://raw.githubusercontent.com/ScoopInstaller/Main/master/bucket/rocode.json")
                .send()
                .await?
                .json()
                .await?;
            let version = json
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse scoop version"))?;
            Ok(version.to_string())
        }
    }
}

fn run_upgrade_process(method: InstallMethod, target: &str) -> anyhow::Result<()> {
    let status = match method {
        InstallMethod::Local => {
            anyhow::bail!(
                "Local installs package ROCode as a single `rocode` binary. Reinstall from the matching release bundle or rerun `./scripts/install-local.sh release` from a source checkout."
            );
        }
        InstallMethod::Curl => ProcessCommand::new("sh")
            .arg("-c")
            .arg("curl -fsSL https://rocode.dev/install | bash")
            .env("VERSION", target)
            .status(),
        InstallMethod::Npm => ProcessCommand::new("npm")
            .args(["install", "-g", &format!("rocode-ai@{}", target)])
            .status(),
        InstallMethod::Pnpm => ProcessCommand::new("pnpm")
            .args(["install", "-g", &format!("rocode-ai@{}", target)])
            .status(),
        InstallMethod::Bun => ProcessCommand::new("bun")
            .args(["install", "-g", &format!("rocode-ai@{}", target)])
            .status(),
        InstallMethod::Brew => ProcessCommand::new("brew")
            .args(["upgrade", "rocode"])
            .status(),
        InstallMethod::Choco => ProcessCommand::new("choco")
            .args(["upgrade", "rocode", "--version", target, "-y"])
            .status(),
        InstallMethod::Scoop => ProcessCommand::new("scoop")
            .args(["install", &format!("rocode@{}", target)])
            .status(),
        InstallMethod::Unknown => {
            anyhow::bail!("Unknown install method; pass --method to specify one explicitly.")
        }
    }
    .map_err(|e| anyhow::anyhow!("Failed to execute upgrade command: {}", e))?;

    if !status.success() {
        anyhow::bail!("Upgrade command exited with status {}", status);
    }
    Ok(())
}

fn prompt_yes_no(question: &str) -> anyhow::Result<bool> {
    print!("{} [y/N]: ", question);
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

async fn handle_upgrade_command(
    target: Option<String>,
    method: Option<String>,
) -> anyhow::Result<()> {
    let detected = detect_install_method();
    let method = method
        .as_deref()
        .map(InstallMethod::parse)
        .unwrap_or(detected);

    println!("Using method: {}", method.as_str());

    if method == InstallMethod::Unknown
        && !prompt_yes_no("Installation method is unknown. Continue anyway?")?
    {
        println!("Cancelled.");
        return Ok(());
    }

    let target = if let Some(target) = target {
        target.trim_start_matches('v').to_string()
    } else {
        latest_version(method).await?
    };

    let current = env!("CARGO_PKG_VERSION").trim_start_matches('v');
    if current == target {
        println!("rocode upgrade skipped: {} is already installed", target);
        return Ok(());
    }

    println!("From {} -> {}", current, target);
    run_upgrade_process(method, &target)?;
    println!("Upgrade complete.");
    Ok(())
}

async fn handle_uninstall_command(
    keep_config: bool,
    keep_data: bool,
    dry_run: bool,
    force: bool,
) -> anyhow::Result<()> {
    let mut targets: Vec<RemovalTarget> = vec![];

    if let Some(app_dir) = managed_bundle_root() {
        targets.push(RemovalTarget {
            label: "app-bundle".to_string(),
            path: app_dir,
        });
    } else if let Some(bin_dir) = managed_binary_dir() {
        for name in MANAGED_BINARIES {
            let path = bin_dir.join(binary_filename(name));
            if path.exists() {
                targets.push(RemovalTarget {
                    label: "binary".to_string(),
                    path,
                });
            }
        }
        if let Some(web_dir) = managed_web_asset_dir() {
            if web_dir.exists() {
                targets.push(RemovalTarget {
                    label: "web-assets".to_string(),
                    path: web_dir,
                });
            }
        }
    }

    let data_dir = dirs::data_local_dir().map(|p| p.join("rocode"));
    let cache_dir = dirs::cache_dir().map(|p| p.join("rocode"));
    let config_dir = dirs::config_dir().map(|p| p.join("rocode"));
    let state_dir = dirs::state_dir().map(|p| p.join("rocode"));

    for (label, path) in [
        ("data", data_dir),
        ("cache", cache_dir),
        ("config", config_dir),
        ("state", state_dir),
    ] {
        if let Some(path) = path {
            targets.push(RemovalTarget {
                label: label.to_string(),
                path,
            });
        }
    }

    println!("Uninstall targets:");
    for target in &targets {
        println!("  {:<10} {}", target.label, target.path.display());
    }

    if dry_run {
        println!("Dry run mode, no files removed.");
        return Ok(());
    }

    if !force {
        println!("Use --force to perform removal.");
        return Ok(());
    }

    for target in targets.drain(..) {
        if (target.label == "config" && keep_config) || (target.label == "data" && keep_data) {
            println!("Skipping {} ({})", target.label, target.path.display());
            continue;
        }
        if !target.path.exists() {
            continue;
        }

        let metadata = std::fs::symlink_metadata(&target.path)
            .map_err(|e| anyhow::anyhow!("Failed stating {}: {}", target.path.display(), e))?;
        if metadata.is_dir() {
            std::fs::remove_dir_all(&target.path)
                .map_err(|e| anyhow::anyhow!("Failed removing {}: {}", target.path.display(), e))?;
        } else {
            std::fs::remove_file(&target.path)
                .map_err(|e| anyhow::anyhow!("Failed removing {}: {}", target.path.display(), e))?;
        }
        println!("Removed {}", target.path.display());
    }
    Ok(())
}

fn print_build_info() {
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
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rocode");
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rocode");
    let config = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rocode");

    println!("Paths:");
    println!("  Data:       {}", data.display());
    println!("  Config:     {}", config.display());
    println!("  Cache:      {}", cache.display());
}
