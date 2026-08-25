//! Launcher contract (Phase 2): the fixed lifecycle order, event
//! identity, fail-closed probing, and forgery resistance. The fake
//! backend has no path to mint events or rewrite plans — these tests
//! pin that by construction and by assertion.

#[path = "support/launcher.rs"]
mod launcher_support;
mod support;

use std::sync::Arc;

use agendao_sandbox::{
    BackendRegistry, DenialReason, EventLog, NativeBackend, PrepareOptions, ProfileKind,
    SandboxEvent, SandboxExecutionRequest, SandboxLauncher, SpawnSpec, TrustClass,
};
use agendao_types::SessionPermissionMode;
use launcher_support::{sequential_minter, FakeBackend};
use support::{cleanup, test_root};

fn launcher_with(fake: Arc<FakeBackend>) -> (SandboxLauncher, Arc<EventLog>) {
    let log = Arc::new(EventLog::default());
    let registry =
        BackendRegistry::native_only(Arc::new(NativeBackend::new())).with_platform_backend(fake);
    let launcher = SandboxLauncher::new(registry, log.clone()).with_id_minter(sequential_minter());
    (launcher, log)
}

fn contained_request(workspace: &std::path::Path) -> SandboxExecutionRequest {
    SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        SpawnSpec::new("/bin/true"),
        workspace,
    )
}

#[tokio::test]
async fn prepared_started_exited_order_and_identity() {
    let root = test_root("launcher_contract");
    let (launcher, log) = launcher_with(Arc::new(FakeBackend::available()));
    let request = contained_request(&root);

    let prepared = launcher
        .prepare(request, &policy_default(), &PrepareOptions::default())
        .expect("contained prepare succeeds with an available backend");
    let fingerprint = prepared.plan().fingerprint.clone();
    let execution_id = prepared.plan().execution_id.clone();
    let mut handle = prepared.start().await.expect("spawn via fake backend");
    let exit = handle.wait().await.expect("reap");

    let events = log.snapshot();
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            SandboxEvent::Prepared { .. } => "prepared",
            SandboxEvent::Started { .. } => "started",
            SandboxEvent::Exited { .. } => "exited",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["prepared", "started", "exited"]);

    // Every event names the same authority-minted identity and the same
    // plan fingerprint the backend actually received.
    for event in &events {
        assert_eq!(event.execution_id(), execution_id);
    }
    match &events[0] {
        SandboxEvent::Prepared {
            plan_fingerprint,
            backend,
            ..
        } => {
            assert_eq!(plan_fingerprint, &fingerprint);
            assert_eq!(backend, "fake");
        }
        _ => panic!("first event must be Prepared"),
    }
    match &events[2] {
        SandboxEvent::Exited {
            status, backend, ..
        } => {
            assert!(status.success);
            assert_eq!(backend, "fake");
        }
        _ => panic!("last event must be Exited"),
    }
    assert!(exit.success);
    cleanup(&root);
}

#[tokio::test]
async fn backend_receives_the_plan_fingerprint_and_authority_env() {
    let root = test_root("launcher_contract");
    let fake = Arc::new(FakeBackend::available());
    let (launcher, _log) = launcher_with(fake.clone());

    let prepared = launcher
        .prepare(
            contained_request(&root),
            &policy_default(),
            &PrepareOptions::default(),
        )
        .unwrap();
    let fingerprint = prepared.plan().fingerprint.clone();
    prepared.start().await.unwrap().wait().await.unwrap();

    let recorded = fake.recorded();
    assert_eq!(recorded.len(), 1, "exactly one spawn");
    // The backend runs exactly the fingerprinted plan it was probed for.
    assert_eq!(recorded[0].fingerprint, fingerprint);
    // Authority keys are injected with the reserved prefix.
    let env_keys: Vec<&str> = recorded[0].env.iter().map(|(k, _)| k.as_str()).collect();
    assert!(env_keys.contains(&"AGENDAO_SANDBOX_EXECUTION_ID"));
    assert!(env_keys.contains(&"AGENDAO_SANDBOX_PLAN_FINGERPRINT"));
    cleanup(&root);
}

#[tokio::test]
async fn contained_fails_closed_without_any_platform_backend() {
    let root = test_root("launcher_contract");
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log.clone());

    let err = launcher
        .prepare(
            contained_request(&root),
            &policy_default(),
            &PrepareOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::SandboxUnavailable { .. }
    ));
    // The denial is auditable with the capability reason.
    let events = log.snapshot();
    assert_eq!(events.len(), 1);
    match &events[0] {
        SandboxEvent::Denied { reason, .. } => match reason {
            DenialReason::BackendUnavailable { capability } => {
                assert!(capability.contains("no platform backend registered"));
            }
            other => panic!("expected BackendUnavailable, got {other:?}"),
        },
        _ => panic!("expected Denied event"),
    }
    cleanup(&root);
}

#[tokio::test]
async fn unavailable_probe_fails_closed_and_names_the_capability() {
    let root = test_root("launcher_contract");
    let fake = Arc::new(FakeBackend::unavailable("bwrap missing"));
    let (launcher, log) = launcher_with(fake.clone());

    let err = launcher
        .prepare(
            contained_request(&root),
            &policy_default(),
            &PrepareOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::SandboxUnavailable { .. }
    ));
    match &log.snapshot()[0] {
        SandboxEvent::Denied { reason, .. } => match reason {
            DenialReason::BackendUnavailable { capability } => {
                // The specific probe reason leads, candidates follow.
                assert!(
                    capability.starts_with("bwrap missing"),
                    "capability: {capability}"
                );
            }
            other => panic!("expected BackendUnavailable, got {other:?}"),
        },
        _ => panic!("expected Denied"),
    }
    assert!(
        fake.recorded().is_empty(),
        "backend must not be spawned into"
    );
    cleanup(&root);
}

#[tokio::test]
async fn policy_denial_emits_denied_and_never_reaches_a_backend() {
    let root = test_root("launcher_contract");
    let fake = Arc::new(FakeBackend::available());
    let (launcher, log) = launcher_with(fake.clone());

    // Default session never grants Native.
    let request = SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::Native,
        SpawnSpec::new("/bin/true"),
        &root,
    );
    let err = launcher
        .prepare(request, &policy_default(), &PrepareOptions::default())
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::Policy(
            agendao_sandbox::PolicyError::NativeNotAllowed
        )
    ));
    match &log.snapshot()[0] {
        SandboxEvent::Denied { reason, .. } => {
            assert_eq!(reason, &DenialReason::PolicyDenied);
        }
        _ => panic!("expected Denied"),
    }
    assert!(fake.recorded().is_empty());
    cleanup(&root);
}

#[tokio::test]
async fn invalid_program_is_rejected_before_any_planning() {
    let root = test_root("launcher_contract");
    let (launcher, log) = launcher_with(Arc::new(FakeBackend::available()));
    let mut request = contained_request(&root);
    request.spec.program = "   ".into();

    let err = launcher
        .prepare(request, &policy_default(), &PrepareOptions::default())
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::InvalidRequest(_)
    ));
    match &log.snapshot()[0] {
        SandboxEvent::Denied { reason, .. } => {
            assert_eq!(reason, &DenialReason::InvalidRequest);
        }
        _ => panic!("expected Denied"),
    }
    cleanup(&root);
}

#[tokio::test]
async fn denied_env_override_is_rejected_with_event() {
    let root = test_root("launcher_contract");
    let (launcher, log) = launcher_with(Arc::new(FakeBackend::available()));
    let mut request = contained_request(&root);
    request
        .spec
        .env_overrides
        .insert("AGENDAO_SERVER_PASSWORD".into(), "guess".into());

    let err = launcher
        .prepare(request, &policy_default(), &PrepareOptions::default())
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::Environment(_)
    ));
    match &log.snapshot()[0] {
        SandboxEvent::Denied { reason, .. } => {
            assert_eq!(reason, &DenialReason::EnvironmentRejected);
        }
        _ => panic!("expected Denied"),
    }
    cleanup(&root);
}

#[tokio::test]
async fn execution_ids_are_authority_minted_not_request_supplied() {
    let root = test_root("launcher_contract");
    let (launcher, _log) = launcher_with(Arc::new(FakeBackend::available()));

    let first = launcher
        .prepare(
            contained_request(&root),
            &policy_default(),
            &PrepareOptions::default(),
        )
        .unwrap();
    let second = launcher
        .prepare(
            contained_request(&root),
            &policy_default(),
            &PrepareOptions::default(),
        )
        .unwrap();
    // The minter is authority-side: identical requests still get fresh
    // audit identities (the request type has no id field to forge with).
    assert_ne!(first.plan().execution_id, second.plan().execution_id);
    drop(first);
    drop(second);
    cleanup(&root);
}

fn policy_default() -> agendao_sandbox::PolicyInputs {
    agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::Default)
}
