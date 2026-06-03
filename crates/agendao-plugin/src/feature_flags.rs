//! Runtime feature flags for plugin optimizations.
//!
//! Each flag defaults to `true` (enabled). Use [`set`] to toggle at runtime
//! for rollback without redeployment.

use std::sync::atomic::{AtomicBool, Ordering};

static SEQ_HOOKS: AtomicBool = AtomicBool::new(true);
static TIMEOUT_SELF_HEAL: AtomicBool = AtomicBool::new(true);
static CIRCUIT_BREAKER: AtomicBool = AtomicBool::new(true);
static LARGE_PAYLOAD_FILE: AtomicBool = AtomicBool::new(true);

/// Check whether a named feature flag is enabled.
///
/// Known flags:
/// - `"plugin_seq_hooks"` — sequential hook execution with per-hook timing
/// - `"plugin_timeout_self_heal"` — auto-reconnect on RPC timeout
/// - `"plugin_circuit_breaker"` — circuit breaker around subprocess hooks
/// - `"plugin_large_payload_file_ipc"` — file-based IPC for large payloads
pub fn is_enabled(flag: &str) -> bool {
    match flag {
        "plugin_seq_hooks" => SEQ_HOOKS.load(Ordering::Relaxed),
        "plugin_timeout_self_heal" => TIMEOUT_SELF_HEAL.load(Ordering::Relaxed),
        "plugin_circuit_breaker" => CIRCUIT_BREAKER.load(Ordering::Relaxed),
        "plugin_large_payload_file_ipc" => LARGE_PAYLOAD_FILE.load(Ordering::Relaxed),
        _ => false,
    }
}

/// Set a feature flag at runtime. Unknown flag names are silently ignored.
pub fn set(flag: &str, enabled: bool) {
    match flag {
        "plugin_seq_hooks" => SEQ_HOOKS.store(enabled, Ordering::Relaxed),
        "plugin_timeout_self_heal" => TIMEOUT_SELF_HEAL.store(enabled, Ordering::Relaxed),
        "plugin_circuit_breaker" => CIRCUIT_BREAKER.store(enabled, Ordering::Relaxed),
        "plugin_large_payload_file_ipc" => LARGE_PAYLOAD_FILE.store(enabled, Ordering::Relaxed),
        _ => {}
    }
}

/// All known flag names.
const ALL_FLAGS: &[&str] = &[
    "plugin_seq_hooks",
    "plugin_timeout_self_heal",
    "plugin_circuit_breaker",
    "plugin_large_payload_file_ipc",
];

/// Initialize flags from environment variables.
///
/// For each flag, checks `AGENDAO_<FLAG_NAME_UPPERCASED>` (e.g.
/// `AGENDAO_PLUGIN_SEQ_HOOKS=0` disables sequential hooks).
/// Values `"0"`, `"false"`, `"off"` disable; anything else enables.
/// Logs effective values at info level.
pub fn init_from_env() {
    for &flag in ALL_FLAGS {
        let env_key = format!("AGENDAO_{}", flag.to_uppercase());
        if let Ok(val) = std::env::var(&env_key) {
            let normalized = val.trim().to_ascii_lowercase();
            let enabled = !matches!(normalized.as_str(), "0" | "false" | "off");
            set(flag, enabled);
            tracing::info!(flag = flag, enabled = enabled, env = %env_key, "[plugin-flags] override from env");
        }
    }
    // Log effective state
    for &flag in ALL_FLAGS {
        tracing::info!(
            flag = flag,
            enabled = is_enabled(flag),
            "[plugin-flags] effective"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_flags_enabled_by_default() {
        assert!(is_enabled("plugin_seq_hooks"));
        assert!(is_enabled("plugin_timeout_self_heal"));
        assert!(is_enabled("plugin_circuit_breaker"));
        assert!(is_enabled("plugin_large_payload_file_ipc"));
    }

    #[test]
    fn unknown_flag_returns_false() {
        assert!(!is_enabled("nonexistent_flag"));
    }

    #[test]
    fn set_toggles_flag() {
        // Disable and re-enable to avoid polluting other tests
        set("plugin_seq_hooks", false);
        assert!(!is_enabled("plugin_seq_hooks"));
        set("plugin_seq_hooks", true);
        assert!(is_enabled("plugin_seq_hooks"));
    }

    #[test]
    fn env_parsing_is_case_and_whitespace_insensitive() {
        // Simulate what init_from_env does internally
        for val in ["0", "false", "off", "FALSE", "False", " false ", " OFF "] {
            let normalized = val.trim().to_ascii_lowercase();
            let enabled = !matches!(normalized.as_str(), "0" | "false" | "off");
            assert!(!enabled, "expected disabled for {:?}", val);
        }
        for val in ["1", "true", "yes", "anything"] {
            let normalized = val.trim().to_ascii_lowercase();
            let enabled = !matches!(normalized.as_str(), "0" | "false" | "off");
            assert!(enabled, "expected enabled for {:?}", val);
        }
    }
}
