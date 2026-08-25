//! Shared test fixtures. Per the sandbox plan (§8.2), every fixture root
//! lives under `CARGO_TARGET_DIR/agendao-sandbox-tests/<test>/<unique>`
//! — never the system temp directory. When `CARGO_TARGET_DIR` is unset
//! the helper fails loudly instead of silently falling back.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

fn require_target_dir() -> PathBuf {
    let configured = std::env::var("CARGO_TARGET_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            panic!(
                "CARGO_TARGET_DIR is not set; run tests with CARGO_TARGET_DIR=../target \
                 (host-side fixtures must never fall back to the system temp dir)"
            )
        });
    let configured = PathBuf::from(configured);
    if configured.is_absolute() {
        configured
    } else {
        workspace_root().join(configured)
    }
}

/// Cargo keeps a relative `CARGO_TARGET_DIR` verbatim in the test process,
/// while the test process itself starts from its crate directory. Resolve it
/// from the workspace root, the same base Cargo used for the command, so the
/// project-wide `../target` convention cannot turn into `crates/target`.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|candidate| candidate.join(".cargo/config.toml").is_file())
        .unwrap_or_else(|| panic!("cannot find workspace root above {}", manifest.display()))
        .to_path_buf()
}

static SEQUENCE: AtomicU32 = AtomicU32::new(0);

fn unique_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Create an isolated fixture root for one test and return it as a
/// canonical absolute path (relative `CARGO_TARGET_DIR` values would
/// otherwise leak `..` segments into path comparisons).
pub fn test_root(test_name: &str) -> PathBuf {
    let root = require_target_dir()
        .join("agendao-sandbox-tests")
        .join(test_name)
        .join(unique_id());
    std::fs::create_dir_all(&root)
        .unwrap_or_else(|err| panic!("create fixture root {}: {err}", root.display()));
    std::fs::canonicalize(&root)
        .unwrap_or_else(|err| panic!("canonicalize fixture root {}: {err}", root.display()))
}

/// Remove a fixture root. Kept on failure for diagnostics by callers.
pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}
