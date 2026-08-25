//! Canonical file-path authority regression probes.

use std::path::PathBuf;

use agendao_tool_core::{resolve_create_file_path, resolve_existing_file_path};

fn fixture_root(name: &str) -> PathBuf {
    let configured = PathBuf::from(
        std::env::var("CARGO_TARGET_DIR")
            .expect("CARGO_TARGET_DIR=../target is required for fixtures"),
    );
    let target = if configured.is_absolute() {
        configured
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .join(configured)
    };
    let root = target.join("agendao-tool-core-tests").join(name);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn component_boundary_does_not_confuse_workspace_and_workspace2() {
    let base = fixture_root("path-prefix");
    let workspace = base.join("workspace");
    let sibling = base.join("workspace2");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let sibling_file = sibling.join("secret.txt");
    std::fs::write(&sibling_file, "unchanged").unwrap();

    let resolved = resolve_existing_file_path(&sibling_file, &workspace).unwrap();
    assert!(resolved.is_external());
    assert_eq!(
        resolved.display_path(),
        sibling_file.canonicalize().unwrap().display().to_string()
    );
}

#[cfg(unix)]
#[test]
fn workspace_symlink_to_external_is_external_and_create_suffix_is_preserved() {
    use std::os::unix::fs::symlink;

    let base = fixture_root("symlink-create");
    let workspace = base.join("workspace");
    let outside = base.join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let external_file = outside.join("secret.txt");
    std::fs::write(&external_file, "unchanged").unwrap();
    symlink(&outside, workspace.join("link")).unwrap();

    let resolved =
        resolve_existing_file_path(&workspace.join("link/secret.txt"), &workspace).unwrap();
    assert!(resolved.is_external());
    assert_eq!(
        std::fs::read_to_string(&external_file).unwrap(),
        "unchanged"
    );

    let nested =
        resolve_create_file_path(&workspace.join("new/deep/file.txt"), &workspace).unwrap();
    assert!(!nested.is_external());
    assert!(nested
        .display_path()
        .ends_with("workspace/new/deep/file.txt"));

    let escaped = resolve_create_file_path(&workspace.join("link/new.txt"), &workspace).unwrap();
    assert!(escaped.is_external());
}

#[test]
fn relative_existing_and_new_workspace_paths_are_internal() {
    let base = fixture_root("relative");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    let existing = workspace.join("src/main.rs");
    std::fs::write(&existing, "fn main() {}").unwrap();

    let existing = resolve_existing_file_path(&existing, &workspace).unwrap();
    assert!(!existing.is_external());
    let new_file = resolve_create_file_path(&workspace.join("src/new.rs"), &workspace).unwrap();
    assert!(!new_file.is_external());
}
