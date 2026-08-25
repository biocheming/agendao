//! Path boundary contract: the negative probes from plan §8.3 that must
//! hold before any backend exists — lexical escapes, sibling-prefix
//! confusion, symlink escapes, create-target resolution, and protected
//! metadata detection.

mod support;

#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::Path;

use agendao_sandbox::{
    assert_no_symlink_escape, assert_within_root, protected_metadata, resolve_create_target,
    resolve_user_path, workspace_scope, PathViolation, RelativePath,
};
use support::{cleanup, test_root};

fn inside(root: &Path, raw: &str) -> Result<(), PathViolationAssert> {
    match resolve_user_path(raw, root) {
        Ok(_) => Ok(()),
        Err(err) => Err(PathViolationAssert(err)),
    }
}

#[derive(Debug)]
struct PathViolationAssert(PathViolation);

impl PathViolationAssert {
    fn is_lexical_escape(&self) -> bool {
        matches!(self.0, PathViolation::LexicalEscape { .. })
    }
}

#[test]
fn workspace_scheme_cannot_escape_lexically() {
    let root = test_root("path_boundary");
    assert!(inside(&root, "workspace:../outside")
        .unwrap_err()
        .is_lexical_escape());
    assert!(inside(&root, "workspace:/../outside")
        .unwrap_err()
        .is_lexical_escape());
    assert!(inside(&root, "workspace:a/../../b")
        .unwrap_err()
        .is_lexical_escape());
    // Legal forms resolve inside.
    assert!(inside(&root, "workspace:src/main.rs").is_ok());
    assert!(inside(&root, "workspace:./a/../b").is_ok());
    cleanup(&root);
}

#[test]
fn relative_input_escaping_base_is_rejected() {
    let root = test_root("path_boundary");
    assert!(inside(&root, "../outside").unwrap_err().is_lexical_escape());
    assert!(inside(&root, "a/../../escape")
        .unwrap_err()
        .is_lexical_escape());
    assert!(inside(&root, "inside").is_ok());
    cleanup(&root);
}

#[test]
fn absolute_paths_pass_resolution_but_face_canonical_containment() {
    let root = test_root("path_boundary");
    let outside = root.parent().unwrap().join("elsewhere");
    let absolute = outside.to_string_lossy().into_owned();
    // Resolution accepts absolutes…
    assert!(inside(&root, &absolute).is_ok());
    // …containment on canonical paths rejects them.
    let canonical_root = std::fs::canonicalize(&root).unwrap();
    let canonical_outside = std::fs::canonicalize(root.parent().unwrap()).unwrap();
    assert!(matches!(
        assert_within_root(&canonical_outside, &canonical_root).unwrap_err(),
        PathViolation::CanonicalEscape { .. }
    ));
    let inside_canonical = std::fs::canonicalize(&root).unwrap();
    assert_eq!(
        assert_within_root(&inside_canonical, &canonical_root).unwrap(),
        RelativePath::new()
    );
    cleanup(&root);
}

#[test]
fn sibling_prefix_is_not_a_subtree() {
    // /workspace2 must not be contained by /workspace (component-wise
    // comparison, never string prefixes).
    let base = test_root("path_boundary");
    let root = base.join("workspace");
    let sibling = base.join("workspace2");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let canonical_root = std::fs::canonicalize(&root).unwrap();
    let canonical_sibling = std::fs::canonicalize(&sibling).unwrap();
    assert!(matches!(
        assert_within_root(&canonical_sibling, &canonical_root).unwrap_err(),
        PathViolation::CanonicalEscape { .. }
    ));
    let child = std::fs::canonicalize(root.join("nested/deep")).unwrap_err();
    assert!(child.kind() == std::io::ErrorKind::NotFound);
    std::fs::create_dir_all(root.join("nested/deep")).unwrap();
    let child = std::fs::canonicalize(root.join("nested/deep")).unwrap();
    assert!(assert_within_root(&child, &canonical_root).is_ok());
    cleanup(&base);
}

#[cfg(unix)]
#[test]
fn symlink_pointing_outside_is_an_escape() {
    let base = test_root("path_boundary");
    let root = base.join("workspace");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join("link")).unwrap();

    let canonical_root = std::fs::canonicalize(&root).unwrap();
    assert!(matches!(
        assert_no_symlink_escape(&root.join("link"), &canonical_root).unwrap_err(),
        PathViolation::SymlinkEscape { .. }
    ));
    // A link staying inside the workspace is fine.
    std::fs::create_dir_all(root.join("real")).unwrap();
    symlink(root.join("real"), root.join("inner")).unwrap();
    assert!(assert_no_symlink_escape(&root.join("inner"), &canonical_root).is_ok());
    cleanup(&base);
}

#[cfg(unix)]
#[test]
fn create_target_resolves_parent_and_follows_existing_symlink() {
    let base = test_root("path_boundary");
    let root = base.join("workspace");
    let outside = base.join("outside");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join("sub/escape-link")).unwrap();

    // New file under a real directory: parent canonicalized, no existing entry.
    let target = resolve_create_target(&root.join("sub/new.txt")).unwrap();
    assert!(target.existing.is_none());
    assert!(target.parent.as_path().ends_with("sub"));

    // Existing symlink: `existing` is the resolved destination, so the
    // policy layer sees the real write point outside the root.
    let target = resolve_create_target(&root.join("sub/escape-link")).unwrap();
    let existing = target
        .existing
        .expect("symlink resolves to an existing path");
    assert_eq!(existing.as_path(), std::fs::canonicalize(&outside).unwrap());

    // Pathological final components are rejected up front.
    assert!(matches!(
        resolve_create_target(&root.join("..")).unwrap_err(),
        PathViolation::InvalidInput(_)
    ));
    cleanup(&base);
}

#[test]
fn protected_metadata_is_detected_on_any_component() {
    assert!(protected_metadata(Path::new("/w/.git/config")).is_some());
    assert!(protected_metadata(Path::new("/w/.agendao")).is_some());
    assert!(protected_metadata(Path::new("/w/src/.agents/tools.md")).is_some());
    assert!(protected_metadata(Path::new("/w/.codex/manifest")).is_some());
    assert!(protected_metadata(Path::new("/w/src/main.rs")).is_none());
    assert!(protected_metadata(Path::new("/w/git/config")).is_none());
}

#[test]
fn workspace_scope_uses_first_segment() {
    assert_eq!(workspace_scope(&RelativePath::new()).as_str(), ".");
    assert_eq!(
        workspace_scope(&RelativePath::from("src/main.rs")).as_str(),
        "src"
    );
    assert_eq!(
        workspace_scope(&RelativePath::from("a.txt")).as_str(),
        "a.txt"
    );
}

#[test]
fn tilde_and_empty_inputs_are_rejected() {
    let root = test_root("path_boundary");
    assert!(matches!(
        resolve_user_path("~/secrets", &root).unwrap_err(),
        PathViolation::InvalidInput(_)
    ));
    assert!(matches!(
        resolve_user_path("   ", &root).unwrap_err(),
        PathViolation::InvalidInput(_)
    ));
    cleanup(&root);
}
