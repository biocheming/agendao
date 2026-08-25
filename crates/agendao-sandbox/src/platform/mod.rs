//! Platform backends: where kernel-enforced containment actually happens.
//!
//! Each OS gets its own backend family under `cfg` gates (`linux/`,
//! future `macos/`, `windows/`). Backends implement `SandboxBackend`
//! and are registered into the `BackendRegistry` by the server
//! authority — they never decide policy, mint ids, or emit events;
//! those authorities live above them (launcher/lifecycle).
//!
//! `process_tree` is the unix-shared signal/reap helper used by every
//! process-group-based backend, including `NativeBackend`.

#[cfg(unix)]
pub mod process_tree;

#[cfg(unix)]
pub mod pty;
#[cfg(not(unix))]
#[path = "pty_stub.rs"]
pub mod pty;

#[cfg(target_os = "linux")]
pub mod linux;

// Same discipline as `windows`: the SBPL profile construction is pure
// and compiles on every host so its contracts are testable anywhere;
// only the sandbox-exec execution shell inside is `cfg(target_os =
// "macos")`.
pub mod macos;

// The Windows model layer (token/acl/job/wfp + the fail-closed
// backend) compiles everywhere so its contracts are testable on any
// host; only the registry registration is cfg-gated.
pub mod windows;

use std::sync::Arc;

use crate::backend::SandboxBackend;

/// Locate an executable by exact name in `PATH` (no spawn, no shell).
/// Shared by every unix backend's discovery step.
#[cfg(unix)]
pub(crate) fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    fn is_executable_file(path: &std::path::Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable_file(candidate))
}

/// The platform backends this build should register, best first.
///
/// Backends are returned regardless of probe outcome: probing belongs
/// to selection time (`BackendRegistry::select`), so an unavailable
/// backend still contributes its capability reason to the user-facing
/// denial instead of silently shrinking the candidate list.
pub fn default_platform_backends() -> Vec<Arc<dyn SandboxBackend>> {
    #[cfg(target_os = "linux")]
    {
        vec![Arc::new(linux::BwrapBackend::discover())]
    }
    // Seatbelt remains registered only as a directly constructible backend;
    // the default registry stays empty until execution-scoped HOME and a
    // supported sandbox-exec replacement are available. This is fail-closed
    // rather than presenting a pseudo-safe host profile.
    #[cfg(target_os = "macos")]
    {
        Vec::new()
    }
    // Registered so its capability row and fail-closed reason surface
    // in projections, but its probe keeps it unselectable until the
    // kernel path (restricted token + job + WFP) is integrated.
    #[cfg(target_os = "windows")]
    {
        vec![windows::windows_backend()]
    }
    // Other platforms ship no contained backend; contained plans fail
    // closed with "no platform backend registered on this build".
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}
