use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::util::parse_http_json;

const MANAGED_BINARIES: [&str; 3] = ["rocode", "rocode-server", "rocode-tui"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallMethod {
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

impl InstallMethod {
    pub(crate) fn parse(value: &str) -> Self {
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

    pub(crate) fn as_str(self) -> &'static str {
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
            let response = client
                .get("https://api.github.com/repos/anomalyco/rocode/releases/latest")
                .header("User-Agent", "rocode-cli-rust")
                .send()
                .await?;
            let json: serde_json::Value = parse_http_json(response).await?;
            let tag = json
                .get("tag_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse latest GitHub release"))?;
            Ok(tag.trim_start_matches('v').to_string())
        }
        InstallMethod::Brew => {
            let response = client
                .get("https://formulae.brew.sh/api/formula/rocode.json")
                .send()
                .await?;
            let json: serde_json::Value = parse_http_json(response).await?;
            let version = json
                .get("versions")
                .and_then(|v| v.get("stable"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse brew stable version"))?;
            Ok(version.to_string())
        }
        InstallMethod::Npm | InstallMethod::Pnpm | InstallMethod::Bun => {
            let response = client
                .get("https://registry.npmjs.org/rocode-ai/latest")
                .send()
                .await?;
            let json: serde_json::Value = parse_http_json(response).await?;
            let version = json
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to parse npm latest version"))?;
            Ok(version.to_string())
        }
        InstallMethod::Scoop => {
            let response = client
                .get("https://raw.githubusercontent.com/ScoopInstaller/Main/master/bucket/rocode.json")
                .send()
                .await?;
            let json: serde_json::Value = parse_http_json(response).await?;
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
                "Local side-by-side installs must upgrade `rocode`, `rocode-server`, and `rocode-tui` together. Reinstall from the matching release bundle or rerun `./scripts/install-local.sh release` from a source checkout."
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

pub(crate) async fn handle_upgrade_command(
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

pub(crate) async fn handle_uninstall_command(
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
