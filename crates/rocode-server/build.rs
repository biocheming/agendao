use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    if let Err(error) = ensure_embedded_web_dist() {
        panic!("failed to prepare embedded web frontend assets: {error}");
    }
}

fn ensure_embedded_web_dist() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|err| err.to_string())?);
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "failed to resolve repository root".to_string())?
        .to_path_buf();
    let web_root = repo_root.join("apps").join("rocode-web");
    let dist_root = web_root.join("dist");

    let watched_roots = [
        web_root.join("src"),
        web_root.join("public"),
        web_root.join("scripts"),
    ];
    let watched_files = [
        web_root.join("index.html"),
        web_root.join("package.json"),
        web_root.join("package-lock.json"),
        web_root.join("vite.config.ts"),
        web_root.join("tsconfig.json"),
    ];

    for path in &watched_files {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for root in &watched_roots {
        println!("cargo:rerun-if-changed={}", root.display());
        emit_rerun_if_changed_recursive(root)?;
    }
    println!("cargo:rerun-if-env-changed=ROCODE_WEB_DIST");
    println!("cargo:rerun-if-env-changed=ROCODE_WEB_SKIP_BUILD");

    if env::var_os("ROCODE_WEB_SKIP_BUILD").is_some() {
        println!("cargo:warning=ROCODE_WEB_SKIP_BUILD set; skipping automatic rocode-web build");
        return Ok(());
    }

    if !should_rebuild_web_dist(&watched_roots, &watched_files, &dist_root)? {
        return Ok(());
    }

    let npm = find_npm().ok_or_else(|| {
        format!(
            "rocode-web assets are stale or missing at `{}` and `npm` was not found in PATH",
            dist_root.display()
        )
    })?;

    println!(
        "cargo:warning=building embedded rocode-web assets with `{}` in {}",
        npm.display(),
        web_root.display()
    );

    let status = Command::new(&npm)
        .arg("run")
        .arg("build")
        .current_dir(&web_root)
        .status()
        .map_err(|err| format!("failed to start `{}`: {err}", npm.display()))?;

    if !status.success() {
        return Err(format!(
            "`{}` failed with status {} while building apps/rocode-web",
            npm.display(),
            status
        ));
    }

    if !has_web_dist(&dist_root) {
        return Err(format!(
            "web build completed but `{}` is still missing required dist assets",
            dist_root.display()
        ));
    }

    Ok(())
}

fn should_rebuild_web_dist(
    watched_roots: &[PathBuf],
    watched_files: &[PathBuf],
    dist_root: &Path,
) -> Result<bool, String> {
    if !has_web_dist(dist_root) {
        return Ok(true);
    }

    let mut newest_source_mtime = SystemTime::UNIX_EPOCH;
    for path in watched_files {
        newest_source_mtime = newest_source_mtime.max(file_mtime(path)?);
    }
    for root in watched_roots {
        newest_source_mtime = newest_source_mtime.max(newest_mtime_recursive(root)?);
    }

    let oldest_dist_mtime = oldest_mtime_recursive(dist_root)?;
    Ok(newest_source_mtime > oldest_dist_mtime)
}

fn emit_rerun_if_changed_recursive(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }

    visit_files_recursive(root, &mut |path| {
        println!("cargo:rerun-if-changed={}", path.display());
        Ok(())
    })
}

fn newest_mtime_recursive(root: &Path) -> Result<SystemTime, String> {
    let mut newest = file_mtime(root)?;
    visit_files_recursive(root, &mut |path| {
        newest = newest.max(file_mtime(path)?);
        Ok(())
    })?;
    Ok(newest)
}

fn oldest_mtime_recursive(root: &Path) -> Result<SystemTime, String> {
    let mut oldest = file_mtime(root)?;
    visit_files_recursive(root, &mut |path| {
        oldest = oldest.min(file_mtime(path)?);
        Ok(())
    })?;
    Ok(oldest)
}

fn visit_files_recursive(
    root: &Path,
    visitor: &mut dyn FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }

    if root.is_file() {
        return visitor(root);
    }

    let mut entries = fs::read_dir(root)
        .map_err(|err| format!("failed to read directory `{}`: {err}", root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to enumerate directory `{}`: {err}", root.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.file_name() == Some(OsStr::new("node_modules")) {
            continue;
        }
        if path.is_dir() {
            visit_files_recursive(&path, visitor)?;
        } else if path.is_file() {
            visitor(&path)?;
        }
    }

    Ok(())
}

fn file_mtime(path: &Path) -> Result<SystemTime, String> {
    fs::metadata(path)
        .map_err(|err| format!("failed to stat `{}`: {err}", path.display()))?
        .modified()
        .map_err(|err| format!("failed to read mtime for `{}`: {err}", path.display()))
}

fn has_web_dist(path: &Path) -> bool {
    path.join("index.html").is_file()
        && path.join("app.js").is_file()
        && path.join("app.css").is_file()
}

fn find_npm() -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let candidates = if cfg!(windows) {
        ["npm.cmd", "npm.exe", "npm"]
    } else {
        ["npm", "npm.cmd", "npm.exe"]
    };

    for dir in env::split_paths(&path_var) {
        for candidate in candidates {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}
