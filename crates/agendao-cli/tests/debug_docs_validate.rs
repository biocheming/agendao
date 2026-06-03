use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_agendao_json(current_dir: &Path, args: &[&str]) -> serde_json::Value {
    static CARGO_RUN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = CARGO_RUN_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("cargo run lock should not be poisoned");

    let manifest_path = repo_root().join("crates/agendao/Cargo.toml");
    let output = Command::new("cargo")
        .arg("run")
        .arg("-q")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("-p")
        .arg("agendao")
        .arg("--")
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("agendao should execute");

    if !output.status.success() {
        panic!(
            "command failed: status={}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn make_temp_project_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
    fs::create_dir_all(&dir).expect("temp project dir should create");
    dir
}

#[test]
fn debug_docs_validate_registry_outputs_stable_json_shape() {
    let root = repo_root();
    let value = run_agendao_json(
        &root,
        &[
            "debug",
            "docs",
            "validate",
            "--registry",
            "./docs/examples/context_docs/context-docs-registry.example.json",
        ],
    );

    assert_eq!(value["valid"], serde_json::json!(true));
    assert_eq!(value["libraryCount"], serde_json::json!(2));
    assert!(value["registryPath"].as_str().is_some());

    let libraries = value["libraries"]
        .as_array()
        .expect("libraries should be an array");
    assert_eq!(libraries.len(), 2);

    let react_router = libraries
        .iter()
        .find(|entry| entry["libraryId"] == serde_json::json!("react-router"))
        .expect("react-router summary should exist");
    assert_eq!(
        react_router["displayName"],
        serde_json::json!("React Router")
    );
    assert_eq!(
        react_router["sourceFamily"],
        serde_json::json!("official_docs")
    );
    assert_eq!(react_router["pageCount"], serde_json::json!(2));
    assert!(react_router["indexPath"].as_str().is_some());
    assert!(react_router["resolvedIndexPath"].as_str().is_some());
    assert_eq!(
        react_router["indexLibraryId"],
        serde_json::json!("react-router")
    );
    assert_eq!(react_router["version"], serde_json::json!("7"));
}

#[test]
fn debug_docs_validate_index_outputs_stable_json_shape() {
    let root = repo_root();
    let value = run_agendao_json(
        &root,
        &[
            "debug",
            "docs",
            "validate",
            "--index",
            "./docs/examples/context_docs/react-router.docs-index.example.json",
        ],
    );

    assert_eq!(value["valid"], serde_json::json!(true));
    assert!(value["indexPath"].as_str().is_some());
    assert_eq!(value["libraryId"], serde_json::json!("react-router"));
    assert_eq!(value["version"], serde_json::json!("7"));
    assert_eq!(value["pageCount"], serde_json::json!(2));

    let page_ids = value["pageIds"]
        .as_array()
        .expect("pageIds should be an array");
    assert_eq!(page_ids.len(), 2);
    assert!(page_ids.contains(&serde_json::json!("guides/data-loading")));
    assert!(page_ids.contains(&serde_json::json!("api/components/router-provider")));
}

#[test]
fn debug_docs_validate_without_flags_uses_configured_registry_path() {
    let temp_project = make_temp_project_dir("agendao-debug-docs-validate");
    let registry_path = repo_root()
        .join("docs/examples/context_docs/context-docs-registry.example.json")
        .display()
        .to_string();

    fs::write(
        temp_project.join("agendao.json"),
        serde_json::json!({
            "docs": {
                "contextDocsRegistryPath": registry_path
            }
        })
        .to_string(),
    )
    .expect("agendao.json should write");

    let value = run_agendao_json(&temp_project, &["debug", "docs", "validate"]);

    assert_eq!(value["valid"], serde_json::json!(true));
    assert_eq!(value["libraryCount"], serde_json::json!(2));
    assert!(value["registryPath"].as_str().is_some());

    fs::remove_dir_all(&temp_project).expect("temp project dir should clean up");
}
