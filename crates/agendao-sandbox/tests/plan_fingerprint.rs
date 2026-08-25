//! Plan fingerprint contract: stability for identical inputs, sensitivity
//! to every policy-relevant change, and order-independence of writable
//! roots.

mod support;

use agendao_sandbox::{
    build_plan, canonicalize_existing, derive_profile, FilesystemMode, PlanContext, PolicyInputs,
    ProfileKind, TrustClass,
};
use agendao_types::SessionPermissionMode;
use support::{cleanup, test_root};

fn workspace_fixture() -> std::path::PathBuf {
    let base = test_root("plan_fingerprint");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    workspace
}

#[test]
fn identical_inputs_yield_identical_fingerprints() {
    let workspace = workspace_fixture();
    let base = workspace.parent().unwrap().to_path_buf();
    let build = |id: &str| {
        let profile = derive_profile(
            TrustClass::ModelReachable,
            ProfileKind::WorkspaceWrite,
            &PolicyInputs::baseline(SessionPermissionMode::Default),
        )
        .unwrap();
        build_plan(
            &profile,
            ProfileKind::WorkspaceWrite,
            &workspace,
            &PlanContext::new(id),
        )
        .unwrap()
    };
    let first = build("exec-1");
    let second = build("exec-1");
    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(first, second);
    // Different execution ids are different auditable runs.
    let other = build("exec-2");
    assert_ne!(first.fingerprint, other.fingerprint);
    cleanup(&base);
}

#[test]
fn every_policy_field_changes_the_fingerprint() {
    let workspace = workspace_fixture();
    let base = workspace.parent().unwrap().to_path_buf();
    let cache_root = base.join("target-cache");
    std::fs::create_dir_all(&cache_root).unwrap();

    let baseline = {
        let profile = derive_profile(
            TrustClass::ModelReachable,
            ProfileKind::WorkspaceWrite,
            &PolicyInputs::baseline(SessionPermissionMode::Default),
        )
        .unwrap();
        build_plan(
            &profile,
            ProfileKind::WorkspaceWrite,
            &workspace,
            &PlanContext::new("x"),
        )
        .unwrap()
    };

    // A different profile kind with different filesystem mode and roots.
    let mut check = PolicyInputs::baseline(SessionPermissionMode::Default);
    check.check_build_cache_root = Some(cache_root.clone());
    let check_plan = {
        let profile =
            derive_profile(TrustClass::ModelReachable, ProfileKind::Check, &check).unwrap();
        build_plan(
            &profile,
            ProfileKind::Check,
            &workspace,
            &PlanContext::new("x"),
        )
        .unwrap()
    };
    assert_eq!(check_plan.filesystem.mode, FilesystemMode::ReadOnly);
    assert_ne!(
        baseline.fingerprint, check_plan.fingerprint,
        "filesystem mode and writable roots must be visible in the fingerprint"
    );

    // Native via the explicit yolo channel.
    let native = {
        let profile = derive_profile(
            TrustClass::ModelReachable,
            ProfileKind::Native,
            &PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
        )
        .unwrap();
        build_plan(
            &profile,
            ProfileKind::Native,
            &workspace,
            &PlanContext::new("x"),
        )
        .unwrap()
    };
    assert_ne!(baseline.fingerprint, native.fingerprint);

    // Lifecycle knobs are part of the auditable plan identity.
    let slower_grace = {
        let profile = derive_profile(
            TrustClass::ModelReachable,
            ProfileKind::WorkspaceWrite,
            &PolicyInputs::baseline(SessionPermissionMode::Default),
        )
        .unwrap();
        build_plan(
            &profile,
            ProfileKind::WorkspaceWrite,
            &workspace,
            &PlanContext {
                execution_id: "x".into(),
                extra_writable_roots: Vec::new(),
                extra_read_only_roots: Vec::new(),
                term_grace: Some(std::time::Duration::from_secs(30)),
                session_origin: None,
            },
        )
        .unwrap()
    };
    assert_ne!(baseline.fingerprint, slower_grace.fingerprint);
    cleanup(&base);
}

#[test]
fn writable_root_order_does_not_change_identity() {
    let workspace = workspace_fixture();
    let base = workspace.parent().unwrap().to_path_buf();
    let extra_a = base.join("cache-a");
    let extra_b = base.join("cache-b");
    std::fs::create_dir_all(&extra_a).unwrap();
    std::fs::create_dir_all(&extra_b).unwrap();

    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &PolicyInputs::baseline(SessionPermissionMode::Default),
    )
    .unwrap();

    let plan_one = build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &workspace,
        &PlanContext {
            execution_id: "x".into(),
            extra_writable_roots: vec![
                canonicalize_existing(&extra_a).unwrap(),
                canonicalize_existing(&extra_b).unwrap(),
            ],
            extra_read_only_roots: Vec::new(),
            term_grace: None,
            session_origin: None,
        },
    )
    .unwrap();

    let plan_two = build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &workspace,
        &PlanContext {
            execution_id: "x".into(),
            extra_writable_roots: vec![
                canonicalize_existing(&extra_b).unwrap(),
                canonicalize_existing(&extra_a).unwrap(),
            ],
            extra_read_only_roots: Vec::new(),
            term_grace: None,
            session_origin: None,
        },
    )
    .unwrap();

    assert_eq!(plan_one.fingerprint, plan_two.fingerprint);
    assert_eq!(plan_one.filesystem.writable_roots.len(), 3);
    cleanup(&base);
}

#[test]
fn nonexisting_workspace_root_fails_the_plan() {
    let base = test_root("plan_fingerprint");
    let ghost = base.join("does-not-exist");
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &PolicyInputs::baseline(SessionPermissionMode::Default),
    )
    .unwrap();
    assert!(build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &ghost,
        &PlanContext::new("x")
    )
    .is_err());
    cleanup(&base);
}

#[test]
fn nonexisting_explicit_writable_root_fails_instead_of_binding_its_ancestor() {
    let workspace = workspace_fixture();
    let base = workspace.parent().unwrap().to_path_buf();
    let missing_cache = workspace.join("target");
    let mut inputs = PolicyInputs::baseline(SessionPermissionMode::Default);
    inputs.check_build_cache_root = Some(missing_cache.clone());
    let profile = derive_profile(TrustClass::ModelReachable, ProfileKind::Check, &inputs).unwrap();

    let err = build_plan(
        &profile,
        ProfileKind::Check,
        &workspace,
        &PlanContext::new("missing-cache"),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::PlanError::WritableRootInvalid(path) if path == missing_cache
    ));
    cleanup(&base);
}
