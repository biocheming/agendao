//! Lifecycle cleanup contract (Phase 2): TERM→grace→KILL escalation,
//! timeout-driven cancellation, single-exit-event semantics, and process
//! trees surviving a direct-child kill (the reason cancellation signals
//! the *group*). Uses the real native backend with real processes.

mod support;

use std::sync::Arc;
use std::time::Duration;

use agendao_sandbox::{
    BackendRegistry, CleanupStatus, EventLog, NativeBackend, PrepareOptions, ProfileKind,
    SandboxEvent, SandboxExecutionRequest, SandboxLauncher, SpawnSpec, TrustClass,
};
use agendao_types::SessionPermissionMode;
use support::{cleanup, test_root};

fn launcher() -> (SandboxLauncher, Arc<EventLog>) {
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    (SandboxLauncher::new(registry, log.clone()), log)
}

fn native_request(workspace: &std::path::Path, spec: SpawnSpec) -> SandboxExecutionRequest {
    SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::Native,
        spec,
        workspace,
    )
}

fn bash(args: &[&str]) -> SpawnSpec {
    SpawnSpec::new("bash").with_args(args.iter().map(|s| s.to_string()).collect())
}

async fn run_to_exit(
    workspace: &std::path::Path,
    spec: SpawnSpec,
    grace_secs: u64,
    mode: WaitMode,
) -> (agendao_sandbox::SandboxExit, Vec<SandboxEvent>) {
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log.clone());
    let options = PrepareOptions {
        extra_writable_roots: Vec::new(),
        term_grace: Some(Duration::from_secs(grace_secs)),
        ..Default::default()
    };
    let mut handle = launcher
        .prepare(
            native_request(workspace, spec),
            &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &options,
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    let exit = match mode {
        WaitMode::Wait => handle.wait().await.unwrap(),
        WaitMode::Cancel => handle.cancel().await.unwrap(),
        WaitMode::Deadline(limit) => handle.wait_with_timeout(limit).await.unwrap(),
    };
    (exit, log.snapshot())
}

enum WaitMode {
    Wait,
    Cancel,
    Deadline(Duration),
}

#[tokio::test]
async fn natural_exit_emits_single_exited_event() {
    let root = test_root("lifecycle_cleanup");
    let (exit, events) = run_to_exit(&root, bash(&["-c", "exit 0"]), 1, WaitMode::Wait).await;
    assert!(exit.success);
    assert_eq!(exit.cleanup, CleanupStatus::NaturalExit);
    let exited = events
        .iter()
        .filter(|e| matches!(e, SandboxEvent::Exited { .. }))
        .count();
    assert_eq!(exited, 1, "exactly one Exited event");
    cleanup(&root);
}

#[tokio::test]
async fn cancel_terminates_a_sleeping_child_within_grace() {
    let root = test_root("lifecycle_cleanup");
    let (exit, events) = run_to_exit(&root, bash(&["-c", "sleep 30"]), 5, WaitMode::Cancel).await;
    assert!(!exit.success, "a TERMed child is not a success");
    assert_eq!(exit.cleanup, CleanupStatus::TerminatedByRequest);
    assert!(exit.signal.is_some(), "expected signal death, got {exit:?}");
    // Prepared -> Started -> Exited, nothing else.
    assert_eq!(events.len(), 3);
    cleanup(&root);
}

#[cfg(unix)]
#[tokio::test]
async fn cancel_escalates_to_kill_when_term_is_trapped() {
    let root = test_root("lifecycle_cleanup");
    // `trap '' TERM` + a bash-builtin busy loop: the leader ignores TERM
    // (an external `sleep` would die to the group signal and let bash
    // exit on its own), forcing the KILL escalation after the grace.
    // The settle wait matters: cancelling in the same millisecond as the
    // spawn can race bash before its `trap` statement even runs.
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log.clone());
    let mut handle = launcher
        .prepare(
            native_request(&root, bash(&["-c", "trap '' TERM; while :; do :; done"])),
            &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &PrepareOptions {
                extra_writable_roots: Vec::new(),
                term_grace: Some(Duration::from_secs(1)),
                ..Default::default()
            },
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let exit = handle.cancel().await.unwrap();
    assert_eq!(exit.cleanup, CleanupStatus::KilledAfterGrace);
    assert_eq!(exit.signal, Some(9), "SIGKILL is the escalation");
    cleanup(&root);
}

#[cfg(unix)]
#[tokio::test]
async fn cancel_reaches_grandchildren_in_the_process_group() {
    let root = test_root("lifecycle_cleanup");
    // A grandchild sleep shares the leader's process group. After
    // cancellation the whole group must be gone — signal-0 probing the
    // pgid must fail with ESRCH once the kernel reaps the remnants.
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
    let launcher = SandboxLauncher::new(registry, log.clone());
    let mut handle = launcher
        .prepare(
            native_request(&root, bash(&["-c", "sleep 30 & while :; do :; done"])),
            &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    let pgid = handle.pid().expect("running child has a pid");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let exit = handle.cancel().await.unwrap();
    assert!(!exit.success);

    let mut group_gone = false;
    for _ in 0..100 {
        // SAFETY: existence probe (signal 0) on the child's group.
        if unsafe { libc::kill(-(pgid as i32), 0) } == -1 {
            group_gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        group_gone,
        "process group {pgid} must be empty after cancel"
    );
    cleanup(&root);
}

#[tokio::test]
async fn deadline_timeout_runs_the_same_ladder() {
    let root = test_root("lifecycle_cleanup");
    let (exit, events) = run_to_exit(
        &root,
        bash(&["-c", "sleep 30"]),
        5,
        WaitMode::Deadline(Duration::from_millis(300)),
    )
    .await;
    assert_eq!(exit.cleanup, CleanupStatus::TimedOut);
    assert_eq!(events.len(), 3);
    cleanup(&root);
}

#[tokio::test]
async fn double_wait_is_an_error_not_a_duplicate_event() {
    let root = test_root("lifecycle_cleanup");
    let (launcher, log) = launcher();
    let mut handle = launcher
        .prepare(
            native_request(&root, bash(&["-c", "exit 0"])),
            &agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    handle.wait().await.unwrap();
    let err = handle.wait().await.unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::AlreadyFinished
    ));
    assert_eq!(
        log.snapshot()
            .iter()
            .filter(|e| matches!(e, SandboxEvent::Exited { .. }))
            .count(),
        1
    );
    cleanup(&root);
}
