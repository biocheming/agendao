//! Real-bwrap runtime contract (Phase 3): actual contained launches
//! through the launcher, with malicious-shell negative probes — writes
//! outside the workspace, protected-metadata writes, network egress,
//! and host-home reads must all fail inside the sandbox. Skips (with a
//! loud message) on hosts without a usable bwrap so the suite stays
//! green where the backend simply doesn't exist.
#![cfg(target_os = "linux")]

mod support;

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use agendao_sandbox::{
    BackendRegistry, BwrapBackend, EventLog, HardPolicy, IntegrationSandboxContext, NativeBackend,
    PermissionGrantScope, PolicyInputs, PrepareOptions, PreparedSandboxExecution, ProfileKind,
    SandboxBackend, SandboxEvent, SandboxExecutionBoundary, SandboxExecutionError,
    SandboxExecutionRequest, SandboxLauncher, SpawnSpec, TrustClass,
};
use agendao_types::SessionPermissionMode;
use support::{cleanup, test_root};

fn bwrap_available() -> bool {
    BwrapBackend::discover().probe().available
}

fn launcher() -> (SandboxLauncher, Arc<EventLog>) {
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()))
        .with_platform_backend(Arc::new(BwrapBackend::discover()));
    (SandboxLauncher::new(registry, log.clone()), log)
}

struct IntegrationBoundary(SandboxLauncher);

#[async_trait]
impl SandboxExecutionBoundary for IntegrationBoundary {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        self.0.prepare(
            request,
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &options,
        )
    }
}

fn request(
    kind: ProfileKind,
    root: &std::path::Path,
    program: &str,
    script: &str,
) -> SandboxExecutionRequest {
    SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        kind,
        SpawnSpec::new(program).with_args(vec!["-c".into(), script.into()]),
        root,
    )
}

/// Launch `/bin/sh -c <script>` contained and wait for its exit.
async fn run_sh(kind: ProfileKind, root: &std::path::Path, script: &str) -> (i32, Arc<EventLog>) {
    let (launcher, log) = launcher();
    let mut handle = launcher
        .prepare(
            request(kind, root, "/bin/sh", script),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    let exit = handle.wait().await.unwrap();
    (exit.code.unwrap_or(-1), log)
}

#[tokio::test]
async fn seccomp_signal_is_projected_as_best_effort_violation() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_violation_signal");
    let (launcher, log) = launcher();
    let mut handle = launcher
        .prepare(
            request(
                ProfileKind::WorkspaceWrite,
                &root,
                "/bin/sh",
                "kill -SYS $$",
            ),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    let exit = handle.wait().await.unwrap();
    assert!(exit.signal == Some(libc::SIGSYS) || exit.code == Some(128 + libc::SIGSYS));
    assert!(log.snapshot().iter().any(|event| matches!(
        event,
        SandboxEvent::Violation { violation }
            if violation.kind == agendao_sandbox::SandboxViolationKind::SyscallDenied
                && violation.attribution == agendao_sandbox::Attribution::BestEffort
                && violation.backend == "bwrap"
    )));
    cleanup(&root);
}

/// Launch a `Check` profile with the authority-selected cache root supplied
/// by the caller. Server-level tests cover cache-root materialization; this
/// real-bwrap helper verifies that the resolved plan mounts the workspace
/// read-only while preserving the sibling cache carve-out.
async fn run_check_sh(
    workspace: &std::path::Path,
    cache_root: &std::path::Path,
    script: &str,
) -> (i32, Arc<EventLog>) {
    let (launcher, log) = launcher();
    let mut inputs = PolicyInputs::baseline(SessionPermissionMode::Default);
    inputs.check_build_cache_root = Some(cache_root.to_path_buf());
    let mut handle = launcher
        .prepare(
            request(ProfileKind::Check, workspace, "/bin/sh", script),
            &inputs,
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    let exit = handle.wait().await.unwrap();
    (exit.code.unwrap_or(-1), log)
}

#[tokio::test]
async fn contained_launch_runs_and_propagates_child_exit_status() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_exit");
    let (code, log) = run_sh(ProfileKind::WorkspaceWrite, &root, "exit 7").await;
    assert_eq!(code, 7, "the sandboxed child's status is the launch result");
    let events = log.snapshot();
    assert_eq!(events.len(), 3, "prepared -> started -> exited");
    assert!(matches!(&events[2], SandboxEvent::Exited { .. }));
    cleanup(&root);
}

#[tokio::test]
async fn integration_runtime_root_is_readable_but_workspace_stays_read_only() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let workspace = test_root("bwrap_runtime_integration_workspace");
    let runtime = test_root("bwrap_runtime_integration_runtime");
    let marker = runtime.join("marker");
    std::fs::write(&marker, "runtime-ok").unwrap();
    let (launcher, _) = launcher();
    let context = IntegrationSandboxContext::new(
        Arc::new(IntegrationBoundary(launcher)),
        workspace.clone(),
        [runtime.clone()],
    )
    .unwrap();
    let script = format!(
        "test \"$(cat {marker})\" = runtime-ok && echo denied > workspace-write 2>/dev/null",
        marker = marker.display()
    );
    let spec = SpawnSpec::new("/bin/sh").with_args(vec!["-c".into(), script]);
    let result = context
        .prepare(spec, PrepareOptions::default())
        .await
        .unwrap();
    let mut handle = result.start().await.unwrap();
    let exit = handle.wait().await.unwrap();
    assert_ne!(
        exit.code,
        Some(0),
        "integration workspace must remain read-only"
    );
    assert!(!workspace.join("workspace-write").exists());
    cleanup(&workspace);
    cleanup(&runtime);
}

#[tokio::test]
async fn workspace_write_allowed_inside_the_workspace() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_ws_write");
    let marker = root.join("inside.txt");
    let script = format!("echo ok > {}", marker.display());
    let (code, _) = run_sh(ProfileKind::WorkspaceWrite, &root, &script).await;
    assert_eq!(code, 0, "script: {script}");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        "ok",
        "the write landed in the host-visible workspace"
    );
    cleanup(&root);
}

#[tokio::test]
async fn writes_outside_the_workspace_fail() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_escape");
    // Host /usr is read-only inside; a write attempt must fail.
    let (code, _) = run_sh(
        ProfileKind::WorkspaceWrite,
        &root,
        "echo escape > /usr/local/agendao-escape-probe 2>/dev/null",
    )
    .await;
    assert_ne!(code, 0, "writes outside the workspace must not succeed");
    assert!(
        !std::path::Path::new("/usr/local/agendao-escape-probe").exists(),
        "nothing may leak onto the host"
    );
    cleanup(&root);
}

#[tokio::test]
async fn protected_metadata_stays_read_only_under_workspace_write() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_protected");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let (code, _) = run_sh(
        ProfileKind::WorkspaceWrite,
        &root,
        "echo pwned > .git/config-probe 2>/dev/null",
    )
    .await;
    assert_ne!(
        code, 0,
        ".git must stay read-only even in a writable workspace"
    );
    assert!(!root.join(".git/config-probe").exists());
    cleanup(&root);
}

#[tokio::test]
async fn network_egress_is_denied() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_net");
    // bash's /dev/tcp needs no external tooling: an unshared, down
    // network namespace makes the connect fail.
    let (code, _) = run_sh(
        ProfileKind::WorkspaceWrite,
        &root,
        "exec 3<>/dev/tcp/198.51.100.1/80",
    )
    .await;
    assert_ne!(code, 0, "network egress must fail inside the sandbox");
    cleanup(&root);
}

#[tokio::test]
async fn host_home_directories_are_not_mounted() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_home");
    // The workspace itself is bound at its original deep path, so its
    // prefix directories exist as bwrap mount scaffolding — the honest
    // probe is *content outside the workspace*: the host's dotfiles
    // must not be reachable through the home prefix.
    let home = std::env::var("HOME").unwrap_or_default();
    let mut script = String::from("test ! -d /root");
    if !home.is_empty() {
        script.push_str(&format!(
            " && test ! -e '{home}/.bashrc' && test ! -e '{home}/.ssh'",
        ));
    }
    let (code, _) = run_sh(ProfileKind::WorkspaceWrite, &root, &script).await;
    assert_eq!(
        code, 0,
        "host home content must not be reachable (script: {script})"
    );
    cleanup(&root);
}

#[tokio::test]
async fn host_environment_secrets_do_not_leak() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    // SAFETY(test): scoped env mutation, restored by the end of the test.
    unsafe { std::env::set_var("AGENDAO_TEST_SECRET", "leak-me") };
    let root = test_root("bwrap_runtime_env");
    let (code, _) = run_sh(
        ProfileKind::WorkspaceWrite,
        &root,
        "test -z \"$AGENDAO_TEST_SECRET\"",
    )
    .await;
    assert_eq!(code, 0, "clearenv must drop host-process secrets");
    let (code2, _) = run_sh(
        ProfileKind::WorkspaceWrite,
        &root,
        "test -n \"$AGENDAO_SANDBOX_EXECUTION_ID\"",
    )
    .await;
    assert_eq!(code2, 0, "authority-injected identity env must be present");
    unsafe { std::env::remove_var("AGENDAO_TEST_SECRET") };
    cleanup(&root);
}

#[tokio::test]
async fn read_only_workspace_denies_writes() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_ro");
    // An empty permission grant downgrades WorkspaceWrite to ReadOnly
    // (Phase 2 rule); the same inputs drive launch and derivation.
    let inputs = PolicyInputs {
        platform: HardPolicy::unrestricted(),
        admin: None,
        agent: None,
        session_mode: SessionPermissionMode::Default,
        grant: Some(PermissionGrantScope {
            write_paths: Vec::new(),
            max_network: None,
        }),
        check_build_cache_root: None,
        environment_allow_exact: Default::default(),
    };
    let (launcher, _log) = launcher();
    let plan = launcher
        .derive_plan(
            &request(ProfileKind::WorkspaceWrite, &root, "/bin/sh", "true"),
            &inputs,
            &PrepareOptions::default(),
        )
        .unwrap();
    assert_eq!(
        plan.filesystem.mode,
        agendao_sandbox::FilesystemMode::ReadOnly
    );

    let mut handle = launcher
        .prepare(
            request(
                ProfileKind::WorkspaceWrite,
                &root,
                "/bin/sh",
                "echo x > ro-probe.txt 2>/dev/null",
            ),
            &inputs,
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    let exit = handle.wait().await.unwrap();
    assert!(!exit.success, "write must fail on a read-only bind");
    assert!(!root.join("ro-probe.txt").exists());
    cleanup(&root);
}

#[tokio::test]
async fn check_profile_denies_workspace_writes_but_allows_the_sibling_target_cache() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let fixture = test_root("bwrap_runtime_check_cache");
    let workspace = fixture.join("workspace");
    let cache = fixture.join("target");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let workspace_probe = workspace.join("must-stay-read-only");
    let cache_probe = cache.join("must-be-writable");
    let script = format!(
        "if echo blocked > '{}' 2>/dev/null; then exit 10; fi; echo allowed > '{}'",
        workspace_probe.display(),
        cache_probe.display()
    );

    let (code, _) = run_check_sh(&workspace, &cache, &script).await;
    assert_eq!(code, 0, "Check profile script failed: {script}");
    assert!(
        !workspace_probe.exists(),
        "Check must not write to its workspace"
    );
    assert_eq!(
        std::fs::read_to_string(&cache_probe).unwrap().trim(),
        "allowed",
        "Check must write to the explicit sibling target cache"
    );
    cleanup(&fixture);
}

#[tokio::test]
async fn cancel_terminates_the_whole_sandbox_tree() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = test_root("bwrap_runtime_cancel");
    let (launcher, _log) = launcher();
    let mut handle = launcher
        .prepare(
            request(
                ProfileKind::WorkspaceWrite,
                &root,
                "/usr/bin/bash",
                "sleep 300 & while :; do :; done",
            ),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap()
        .start()
        .await
        .unwrap();
    // Let bwrap set up mounts and the busy loop start.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let exit = handle.cancel().await.unwrap();
    assert_eq!(
        exit.cleanup,
        agendao_sandbox::CleanupStatus::TerminatedByRequest
    );
    cleanup(&root);
}

#[tokio::test]
async fn missing_bwrap_fails_closed_with_capability_reason() {
    let root = test_root("bwrap_runtime_missing");
    let log = Arc::new(EventLog::default());
    let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()))
        .with_platform_backend(Arc::new(BwrapBackend::new(
            "/nonexistent/agendao-bwrap".into(),
        )));
    let launcher = SandboxLauncher::new(registry, log.clone());

    let err = launcher
        .prepare(
            request(ProfileKind::WorkspaceWrite, &root, "/bin/sh", "exit 0"),
            &PolicyInputs::baseline(SessionPermissionMode::Default),
            &PrepareOptions::default(),
        )
        .unwrap_err();
    match err {
        agendao_sandbox::SandboxExecutionError::SandboxUnavailable { backend, reason } => {
            assert_eq!(backend, "bwrap");
            assert!(
                reason.contains("bwrap missing"),
                "capability reason: {reason}"
            );
        }
        other => panic!("expected SandboxUnavailable, got {other:?}"),
    }
    let events = log.snapshot();
    assert_eq!(events.len(), 1, "only the denial is auditable");
    assert!(matches!(&events[0], SandboxEvent::Denied { .. }));
    // No shell ever ran: the denial is the whole story.
    assert!(
        !root.join("anything").exists(),
        "no filesystem effect from a denied launch"
    );
    cleanup(&root);
}
