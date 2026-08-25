use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static FIXTURE_SEQUENCE: AtomicU32 = AtomicU32::new(0);

pub(crate) fn target_fixture_root(name: &str) -> PathBuf {
    let configured = std::env::var("CARGO_TARGET_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            panic!("CARGO_TARGET_DIR is not set; run tests with CARGO_TARGET_DIR=../target")
        });
    let configured = PathBuf::from(configured);
    let target_root = if configured.is_absolute() {
        configured
    } else {
        workspace_root().join(configured)
    };
    let fixture = target_root
        .join("agendao-server-unit-tests")
        .join(name)
        .join(format!(
            "{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&fixture)
        .unwrap_or_else(|error| panic!("create fixture {}: {error}", fixture.display()));
    std::fs::canonicalize(&fixture)
        .unwrap_or_else(|error| panic!("canonicalize fixture {}: {error}", fixture.display()))
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|candidate| candidate.join(".cargo/config.toml").is_file())
        .unwrap_or_else(|| panic!("cannot find workspace root above {}", manifest.display()))
        .to_path_buf()
}
