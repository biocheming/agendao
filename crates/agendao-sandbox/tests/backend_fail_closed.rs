//! Backend fail-closed contract (Phase 2): contained plans never fall
//! back to native; the native backend refuses contained plans at two
//! independent layers (selection and spawn); native plans run even with
//! zero platform backends registered.

mod support;

use std::sync::Arc;

use agendao_sandbox::{
    BackendRegistry, NativeBackend, PrepareOptions, ProfileKind, SandboxBackend,
    SandboxExecutionRequest, SandboxLauncher, SpawnSpec, TrustClass,
};
use agendao_types::SessionPermissionMode;
use support::{cleanup, test_root};

#[tokio::test]
async fn registry_with_no_platform_backend_denies_contained_plans() {
    let root = test_root("backend_fail_closed");
    let log = Arc::new(agendao_sandbox::EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log);

    let err = launcher
        .prepare(
            SandboxExecutionRequest::new(
                TrustClass::ModelReachable,
                ProfileKind::WorkspaceWrite,
                SpawnSpec::new("/bin/true"),
                &root,
            ),
            &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap_err();

    match err {
        agendao_sandbox::SandboxExecutionError::SandboxUnavailable { backend, reason } => {
            // The error must say what was missing, not just "no".
            assert!(
                reason.contains("no platform backend registered"),
                "reason: {reason}"
            );
            assert_eq!(backend, "none");
        }
        other => panic!("expected SandboxUnavailable, got {other:?}"),
    }
    cleanup(&root);
}

#[tokio::test]
async fn native_backend_supports_only_native_plans() {
    let native = NativeBackend::new();
    let root = test_root("backend_fail_closed");

    // A contained plan built directly from the Phase 1 APIs (Default
    // session, WorkspaceWrite kind) — no launcher needed since we are
    // testing the backend's own refusal, not the registry's.
    let profile = agendao_sandbox::derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::Default),
    )
    .unwrap();
    let contained_plan = agendao_sandbox::build_plan(
        &profile,
        ProfileKind::WorkspaceWrite,
        &root,
        &agendao_sandbox::PlanContext::new("test-contained"),
    )
    .unwrap();
    assert!(
        !native.supports(&contained_plan),
        "supports() must refuse contained plans"
    );

    cleanup(&root);
}

#[tokio::test]
async fn native_plan_runs_with_zero_platform_backends() {
    let root = test_root("backend_fail_closed");
    let log = Arc::new(agendao_sandbox::EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log.clone());

    let mut handle = launcher
        .prepare(
            SandboxExecutionRequest::new(
                TrustClass::ModelReachable,
                ProfileKind::Native,
                SpawnSpec::new("/bin/true"),
                &root,
            ),
            &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &PrepareOptions::default(),
        )
        .expect("yolo grants native")
        .start()
        .await
        .expect("native spawn needs no platform backend");
    let exit = handle.wait().await.expect("reap native child");
    assert!(exit.success);
    assert_eq!(
        handle.plan().process.mode,
        agendao_sandbox::ProcessMode::Native
    );
    cleanup(&root);
}

#[tokio::test]
async fn capabilities_projection_names_every_backend_and_probe() {
    let root = test_root("backend_fail_closed");
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, Arc::new(agendao_sandbox::EventLog::default()));
    let caps = launcher.capabilities();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].backend, "native");
    assert!(caps[0].native);
    assert!(!caps[0].contained);
    assert!(caps[0].probe.available);
    cleanup(&root);
}
