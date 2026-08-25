//! Host capability probe for the Linux backend. Zero subprocesses:
//! everything here is filesystem metadata and sysctl reads, so probing
//! is cheap enough to run on every launch (and stays inside the spawn
//! inventory contract by construction).

use crate::backend::BackendProbe;
use crate::platform::find_in_path;

/// Unprivileged user namespaces: bwrap needs one even for root-less
/// setups. Debian-family exposes an explicit kill switch; everyone else
/// reports a quota where 0 means "no user namespaces at all".
fn user_namespaces_available() -> Result<(), String> {
    if let Ok(value) = std::fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone") {
        if value.trim() == "0" {
            return Err(
                "unprivileged user namespaces are disabled (kernel.unprivileged_userns_clone=0)"
                    .to_string(),
            );
        }
    }
    if let Ok(value) = std::fs::read_to_string("/proc/sys/user/max_user_namespaces") {
        if value.trim() == "0" {
            return Err("user namespaces are disabled (user.max_user_namespaces=0)".to_string());
        }
    }
    Ok(())
}

/// Probe bwrap availability. The reason strings are user-facing
/// capability explanations — they flow into `SandboxUnavailable`
/// denials when a contained launch fails closed.
pub fn probe_bwrap() -> BackendProbe {
    match find_in_path("bwrap") {
        None => {
            BackendProbe::unavailable("bwrap missing from PATH (install the bubblewrap package)")
        }
        Some(_) => match user_namespaces_available() {
            Ok(()) => BackendProbe::available(),
            Err(reason) => {
                BackendProbe::unavailable(format!("bwrap present but unusable: {reason}"))
            }
        },
    }
}
