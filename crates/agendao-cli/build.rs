use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    embed_windows_icon();

    // Capture rustc version at compile time.
    let rustc_version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string());
    println!(
        "cargo:rustc-env=AGENDAO_RUSTC_VERSION={}",
        rustc_version.trim()
    );

    // Capture the target triple.
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=AGENDAO_TARGET={}", target);

    // Capture the build profile (debug or release).
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=AGENDAO_PROFILE={}", profile);

    // Capture the host triple (what we're compiling on).
    let host = std::env::var("HOST").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=AGENDAO_HOST={}", host);

    // Build timestamp (ISO 8601).
    let now = chrono_lite_utc_now();
    println!("cargo:rustc-env=AGENDAO_BUILD_TIME={}", now);
}

fn embed_windows_icon() {
    println!("cargo:rerun-if-changed=../../icons/agendao.ico");

    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows-msvc") {
        return;
    }

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
    let icon_path = manifest_dir.join("../../icons/agendao.ico");
    if !icon_path.is_file() {
        println!(
            "cargo:warning=agendao-cli: windows icon not found at {}",
            icon_path.display()
        );
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR missing"));
    let rc_path = out_dir.join("agendao-icon.rc");
    let res_path = out_dir.join("agendao-icon.res");
    let icon_path = icon_path.to_string_lossy().replace('\\', "/");
    let rc_body = format!("1 ICON \"{}\"\n", icon_path);

    if let Err(error) = fs::write(&rc_path, rc_body) {
        println!(
            "cargo:warning=agendao-cli: failed to write icon resource script: {}",
            error
        );
        return;
    }

    let compiler = find_resource_compiler();
    let Some(compiler) = compiler else {
        println!(
            "cargo:warning=agendao-cli: no rc compiler found; windows executable icon will be omitted"
        );
        return;
    };

    let status = Command::new(&compiler)
        .args([
            "/nologo",
            "/fo",
            res_path.to_string_lossy().as_ref(),
            rc_path.to_string_lossy().as_ref(),
        ])
        .status();

    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg-bin=agendao={}", res_path.display());
        }
        Ok(status) => {
            println!(
                "cargo:warning=agendao-cli: {} failed with status {}; windows executable icon will be omitted",
                compiler,
                status
            );
        }
        Err(error) => {
            println!(
                "cargo:warning=agendao-cli: failed to run {}: {}; windows executable icon will be omitted",
                compiler,
                error
            );
        }
    }
}

fn find_resource_compiler() -> Option<&'static str> {
    ["llvm-rc", "rc"]
        .into_iter()
        .find(|candidate| Command::new(candidate).arg("/?").output().is_ok())
}

/// Minimal UTC timestamp without pulling in chrono at build time.
fn chrono_lite_utc_now() -> String {
    Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
