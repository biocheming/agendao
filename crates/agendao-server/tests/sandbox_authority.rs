//! SandboxAuthority contract (Phase 2): permission grants become minimal
//! profiles, native is denied under default sessions, contained launches
//! fail closed without a platform backend, and events/capabilities flow
//! through the authority's single sink.

mod support;

use std::sync::{Arc, Mutex};

use agendao_sandbox::{
    BackendChild, BackendExit, BackendProbe, BackendViolationToken, ChildEnvironment, EventLog,
    FilesystemMode, HardPolicy, NativeBackend, NetworkMode, PermissionGrantScope, PrepareOptions,
    ProfileKind, ProfileSummary, SandboxBackend, SandboxEvent, SandboxPlan, SpawnSpec, StdioPlan,
    TrustClass,
};
use agendao_server::sandbox_authority::{SandboxAuthority, SandboxAuthorityConfig};
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;
use support::test_root;

/// Minimal fake platform backend: records spawns, children exit at once.
struct FakeBackend {
    spawns: Mutex<Vec<String>>,
    plans: Mutex<Vec<SandboxPlan>>,
}

#[async_trait]
impl SandboxBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn probe(&self) -> BackendProbe {
        BackendProbe::available()
    }

    fn supports(&self, _plan: &SandboxPlan) -> bool {
        true
    }

    async fn spawn(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        _env: &ChildEnvironment,
        _stdio: &StdioPlan,
        _violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, agendao_sandbox::SandboxExecutionError> {
        self.plans.lock().unwrap().push(plan.clone());
        self.spawns
            .lock()
            .unwrap()
            .push(format!("{}:{}", spec.program, plan.fingerprint));
        Ok(Box::new(ExitedChild))
    }
}

struct ExitedChild;

#[async_trait]
impl BackendChild for ExitedChild {
    fn pid(&self) -> Option<u32> {
        Some(7)
    }

    async fn wait(&mut self) -> Result<BackendExit, agendao_sandbox::SandboxExecutionError> {
        Ok(BackendExit {
            success: true,
            code: Some(0),
            signal: None,
        })
    }

    async fn signal_term(&mut self) -> Result<(), agendao_sandbox::SandboxExecutionError> {
        Ok(())
    }

    async fn signal_kill(&mut self) -> Result<(), agendao_sandbox::SandboxExecutionError> {
        Ok(())
    }
}

fn workspace() -> std::path::PathBuf {
    test_root("sandbox_authority")
}

fn request(
    kind: ProfileKind,
    workspace: &std::path::Path,
) -> agendao_sandbox::SandboxExecutionRequest {
    agendao_sandbox::SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        kind,
        SpawnSpec::new("/bin/true"),
        workspace,
    )
}

#[test]
fn deployment_environment_allowlist_reaches_the_immutable_plan() {
    let dir = workspace();
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default)
            .with_environment_allow_exact(["MONKEY_TOKEN"]),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new()))
            .with_platform_backend(Arc::new(FakeBackend {
                spawns: Mutex::new(Vec::new()),
                plans: Mutex::new(Vec::new()),
            })),
        Arc::new(EventLog::default()),
    );
    let plan = authority
        .derive_plan(
            &request(ProfileKind::WorkspaceWrite, &dir),
            None,
            &PrepareOptions::default(),
        )
        .unwrap();
    assert!(plan.environment.allow_exact.contains("MONKEY_TOKEN"));
    assert!(plan
        .environment
        .hard_deny_exact
        .contains("AGENDAO_INTERNAL_TOKEN"));
}

#[tokio::test]
async fn file_grant_without_write_paths_derives_read_only() {
    let dir = workspace();
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        Arc::new(EventLog::default()),
    );

    let empty_grant = PermissionGrantScope {
        write_paths: Vec::new(),
        max_network: None,
    };
    let plan = authority
        .derive_plan(
            &request(ProfileKind::WorkspaceWrite, &dir),
            Some(&empty_grant),
            &PrepareOptions::default(),
        )
        .unwrap();
    assert_eq!(plan.filesystem.mode, FilesystemMode::ReadOnly);

    // The same request with no grant at all (process tools) keeps the
    // workspace-write default.
    let plan = authority
        .derive_plan(
            &request(ProfileKind::WorkspaceWrite, &dir),
            None,
            &PrepareOptions::default(),
        )
        .unwrap();
    assert_eq!(plan.filesystem.mode, FilesystemMode::WorkspaceWrite);
}

#[tokio::test]
async fn default_sessions_never_derive_native() {
    let dir = workspace();
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        Arc::new(EventLog::default()),
    );
    let err = authority
        .derive_plan(
            &request(ProfileKind::Native, &dir),
            None,
            &PrepareOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::Policy(
            agendao_sandbox::PolicyError::NativeNotAllowed
        )
    ));
}

#[tokio::test]
async fn trusted_workspace_sessions_never_derive_native() {
    let dir = workspace();
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::TrustedWorkspace),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        Arc::new(EventLog::default()),
    );
    let err = authority
        .derive_plan(
            &request(ProfileKind::Native, &dir),
            None,
            &PrepareOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::Policy(
            agendao_sandbox::PolicyError::NativeNotAllowed
        )
    ));
}

#[tokio::test]
async fn yolo_sessions_allow_native_profile() {
    let dir = workspace();
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::UnsandboxedYolo),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        Arc::new(EventLog::default()),
    );
    let plan = authority
        .derive_plan(
            &request(ProfileKind::Native, &dir),
            None,
            &PrepareOptions::default(),
        )
        .expect("explicit yolo session may request native");
    assert_eq!(plan.filesystem.mode, FilesystemMode::Unrestricted);
}

#[tokio::test]
async fn admin_hard_policy_overrides_even_yolo_sessions() {
    let dir = workspace();
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::UnsandboxedYolo)
            .with_admin(HardPolicy::contained_baseline()),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        Arc::new(EventLog::default()),
    );
    let err = authority
        .derive_plan(
            &request(ProfileKind::Native, &dir),
            None,
            &PrepareOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::Policy(
            agendao_sandbox::PolicyError::NativeNotAllowed
        )
    ));
}

#[tokio::test]
async fn contained_launch_fails_closed_without_platform_backend() {
    let dir = workspace();
    let log = Arc::new(EventLog::default());
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        log.clone(),
    );
    let err = authority
        .launch(
            request(ProfileKind::WorkspaceWrite, &dir),
            None,
            &PrepareOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        agendao_sandbox::SandboxExecutionError::SandboxUnavailable { .. }
    ));
    assert_eq!(log.snapshot().len(), 1, "denial is auditable");
}

#[tokio::test]
async fn launch_via_fake_backend_runs_the_full_event_ladder() {
    let dir = workspace();
    let log = Arc::new(EventLog::default());
    let fake = Arc::new(FakeBackend {
        spawns: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
    });
    let registry = agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new()))
        .with_platform_backend(fake.clone() as Arc<dyn SandboxBackend>);
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default),
        registry,
        log.clone(),
    );

    let mut handle = authority
        .launch(
            request(ProfileKind::WorkspaceWrite, &dir),
            None,
            &PrepareOptions::default(),
        )
        .await
        .unwrap();
    let exit = handle.wait().await.unwrap();
    assert!(exit.success);

    let events = log.snapshot();
    assert_eq!(events.len(), 3, "prepared -> started -> exited");
    match &events[0] {
        SandboxEvent::Prepared {
            profile:
                ProfileSummary {
                    filesystem_mode,
                    network_mode,
                    ..
                },
            backend,
            ..
        } => {
            assert_eq!(filesystem_mode, &FilesystemMode::WorkspaceWrite);
            assert_eq!(network_mode, &NetworkMode::Disabled);
            assert_eq!(backend, "fake");
        }
        _ => panic!("expected Prepared first"),
    }
    // The fake backend ran exactly one spawn under the plan fingerprint.
    assert_eq!(fake.spawns.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn capabilities_projection_lists_native_and_platform() {
    let fake = Arc::new(FakeBackend {
        spawns: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
    });
    let registry = agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new()))
        .with_platform_backend(fake as Arc<dyn SandboxBackend>);
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default),
        registry,
        Arc::new(EventLog::default()),
    );
    let caps = authority.capabilities();
    assert_eq!(caps.len(), 2);
    assert!(caps.iter().any(|c| c.backend == "fake" && c.contained));
    assert!(caps.iter().any(|c| c.backend == "native" && c.native));
}

#[tokio::test]
async fn check_launch_materializes_only_the_workspace_sibling_target_carve_out() {
    let fixture = test_root("sandbox_authority_check_cache");
    let workspace = fixture.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let target = fixture.join("target");
    assert!(
        !target.exists(),
        "the authority, rather than the fixture, must materialize the cache root"
    );

    let fake = Arc::new(FakeBackend {
        spawns: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
    });
    let registry = agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new()))
        .with_platform_backend(fake.clone() as Arc<dyn SandboxBackend>);
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::Default),
        registry,
        Arc::new(EventLog::default()),
    );

    let mut handle = authority
        .launch_check(
            // `launch_check` owns the criterion profile, regardless of a
            // wider generic request that reaches this authority API.
            request(ProfileKind::WorkspaceWrite, &workspace),
            &PrepareOptions::default(),
        )
        .await
        .unwrap();
    assert!(handle.wait().await.unwrap().success);

    let expected_target = std::fs::canonicalize(&target).unwrap();
    let expected_workspace = std::fs::canonicalize(&workspace).unwrap();
    let plans = fake.plans.lock().unwrap();
    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert_eq!(plan.requested_kind, ProfileKind::Check);
    assert_eq!(plan.filesystem.mode, FilesystemMode::ReadOnly);
    assert_eq!(plan.filesystem.writable_roots.len(), 1);
    assert_eq!(
        plan.filesystem.writable_roots[0].as_str(),
        expected_target.to_str().unwrap(),
        "Check may write only its workspace-sibling ../target cache"
    );
    assert_ne!(
        plan.filesystem.writable_roots[0].as_str(),
        expected_workspace.to_str().unwrap(),
        "a missing target must never degrade to a writable workspace"
    );
}

#[tokio::test]
async fn boundary_prepare_passes_stdio_through_to_a_yolo_native_launch() {
    // Phase 4 wiring: the tool-facing boundary forwards launch options
    // (io shaping) verbatim. A yolo authority + native backend + piped
    // output must yield readable child streams.
    let dir = workspace();
    let log = Arc::new(EventLog::default());
    let authority = SandboxAuthority::new(
        SandboxAuthorityConfig::for_session(SessionPermissionMode::UnsandboxedYolo),
        agendao_sandbox::BackendRegistry::native_only(Arc::new(NativeBackend::new())),
        log.clone(),
    );
    let boundary: Arc<dyn agendao_tool_core::SandboxExecutionBoundary> = Arc::new(authority);

    let spec = SpawnSpec::new("/bin/sh").with_args(vec!["-c".into(), "echo piped".into()]);
    let request = agendao_sandbox::SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::Native,
        spec,
        &dir,
    );
    let options = PrepareOptions {
        stdio: StdioPlan::piped_output(),
        ..Default::default()
    };
    let mut handle = boundary
        .prepare(request, options)
        .await
        .unwrap()
        .start()
        .await
        .unwrap();

    let mut stdout = handle.take_stdout().expect("piped stdout was requested");
    let mut line = String::new();
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf).await.unwrap();
    line.push_str(&String::from_utf8_lossy(&buf));
    assert!(line.contains("piped"), "captured: {line:?}");

    let exit = handle.wait().await.unwrap();
    assert!(exit.success);
    assert_eq!(log.snapshot().len(), 3);
    drop(dir);
}
