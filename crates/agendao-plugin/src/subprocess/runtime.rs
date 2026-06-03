//! JS runtime detection — finds bun, deno, or node on `$PATH`.

use std::path::PathBuf;

/// Supported JavaScript runtimes, in order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsRuntime {
    Bun,
    Deno,
    Node,
}

impl JsRuntime {
    /// The executable name.
    pub fn command(&self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Deno => "deno",
            Self::Node => "node",
        }
    }

    /// Arguments to run a TS file.
    pub fn run_args(&self, script: &str) -> Vec<String> {
        match self {
            Self::Bun => vec!["run".into(), script.into()],
            Self::Deno => vec!["run".into(), "--allow-all".into(), script.into()],
            Self::Node => {
                // Node >=22 has native TS support via --experimental-strip-types
                vec!["--experimental-strip-types".into(), script.into()]
            }
        }
    }

    /// The package manager command to use for `npm install`.
    /// For Bun/Deno we use their built-in install; for Node we use npm.
    pub fn install_command(&self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Deno => "deno",
            Self::Node => "npm",
        }
    }

    /// Arguments for the install command.
    pub fn install_args(&self) -> Vec<String> {
        vec!["install".into()]
    }
}

/// Detect the best available JS runtime.
///
/// Override order with `AGENDAO_PLUGIN_RUNTIME`.
/// Default preference is bun > deno > node (node requires >=22.6 for TS).
pub fn detect_runtime() -> Option<JsRuntime> {
    if let Ok(raw) = std::env::var("AGENDAO_PLUGIN_RUNTIME") {
        let forced = raw.trim().to_ascii_lowercase();
        let runtime = match forced.as_str() {
            "bun" => Some(JsRuntime::Bun),
            "deno" => Some(JsRuntime::Deno),
            "node" => Some(JsRuntime::Node),
            _ => None,
        };
        if let Some(rt) = runtime {
            if rt == JsRuntime::Node && !node_supports_strip_types() {
                tracing::warn!(
                    "AGENDAO_PLUGIN_RUNTIME=node but node <22.6 lacks TS support; ignoring"
                );
            } else if which::which(rt.command()).is_ok() {
                return Some(rt);
            }
        }
    }

    // Bun and Deno natively support TS; Node requires >=22.6.
    for rt in [JsRuntime::Bun, JsRuntime::Deno] {
        if which::which(rt.command()).is_ok() {
            return Some(rt);
        }
    }
    if node_supports_strip_types() {
        return Some(JsRuntime::Node);
    }
    None
}

/// Check whether the `node` on PATH is >=22.6 (has --experimental-strip-types).
fn node_supports_strip_types() -> bool {
    let Ok(path) = which::which("node") else {
        return false;
    };
    let Ok(output) = std::process::Command::new(path).arg("--version").output() else {
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout);
    // version looks like "v22.6.0\n"
    let version = version.trim().trim_start_matches('v');
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let major: u32 = parts[0].parse().unwrap_or(0);
    let minor: u32 = parts[1].parse().unwrap_or(0);
    major > 22 || (major == 22 && minor >= 6)
}

/// Return the full path to the runtime binary, if found.
pub fn runtime_path(rt: JsRuntime) -> Option<PathBuf> {
    which::which(rt.command()).ok()
}
