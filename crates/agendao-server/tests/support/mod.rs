//! Shared test fixtures for sandbox-related server tests. Per the
//! sandbox plan (§8.2), host-side fixture roots live under
//! `CARGO_TARGET_DIR/agendao-sandbox-tests/<test>/<unique>` — never the
//! system temp directory.

use std::path::PathBuf;
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

/// Canonical absolute fixture root for one test (relative target dirs
/// would leak `..` segments into path comparisons).
pub fn test_root(test_name: &str) -> PathBuf {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let root = require_target_dir()
        .join("agendao-sandbox-tests")
        .join(test_name)
        .join(unique);
    std::fs::create_dir_all(&root)
        .unwrap_or_else(|err| panic!("create fixture root {}: {err}", root.display()));
    std::fs::canonicalize(&root)
        .unwrap_or_else(|err| panic!("canonicalize fixture root {}: {err}", root.display()))
}
