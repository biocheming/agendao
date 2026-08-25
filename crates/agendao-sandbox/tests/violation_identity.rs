//! Violation evidence is bound to the one launch that received its opaque token.

mod support;

use std::sync::{Arc, Mutex};

use agendao_sandbox::{
    Attribution, BackendChild, BackendExit, BackendProbe, BackendRegistry, BackendViolationReport,
    BackendViolationToken, ChildEnvironment, EventLog, NativeBackend, PrepareOptions, ProfileKind,
    SandboxBackend, SandboxEvent, SandboxExecutionError, SandboxExecutionRequest, SandboxLauncher,
    SandboxPlan, SandboxViolationKind, SpawnSpec, StdioPlan, TrustClass,
};
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;
use support::{cleanup, test_root};

struct CrossExecutionBackend {
    first_token: Mutex<Option<BackendViolationToken>>,
}

#[async_trait]
impl SandboxBackend for CrossExecutionBackend {
    fn name(&self) -> &'static str {
        "cross-token-fake"
    }
    fn probe(&self) -> BackendProbe {
        BackendProbe::available()
    }
    fn supports(&self, _plan: &SandboxPlan) -> bool {
        true
    }
    async fn spawn(
        &self,
        _plan: &SandboxPlan,
        _spec: &SpawnSpec,
        _env: &ChildEnvironment,
        _stdio: &StdioPlan,
        token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        let mut stored = self.first_token.lock().unwrap();
        let report = match stored.as_ref() {
            Some(stale) => Some(stale.report(
                SandboxViolationKind::PathEscape,
                Some("/outside".into()),
                Attribution::BestEffort,
            )),
            None => {
                *stored = Some(token);
                None
            }
        };
        Ok(Box::new(EvidenceChild { report }))
    }
}

struct EvidenceChild {
    report: Option<BackendViolationReport>,
}

#[async_trait]
impl BackendChild for EvidenceChild {
    fn pid(&self) -> Option<u32> {
        Some(7)
    }
    async fn wait(&mut self) -> Result<BackendExit, SandboxExecutionError> {
        Ok(BackendExit {
            success: true,
            code: Some(0),
            signal: None,
        })
    }
    async fn signal_term(&mut self) -> Result<(), SandboxExecutionError> {
        Ok(())
    }
    async fn signal_kill(&mut self) -> Result<(), SandboxExecutionError> {
        Ok(())
    }
    fn take_violation_report(&mut self) -> Option<BackendViolationReport> {
        self.report.take()
    }
}

fn request(root: &std::path::Path) -> SandboxExecutionRequest {
    SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        SpawnSpec::new("/bin/true"),
        root,
    )
    .with_session_origin("session-a")
}

#[tokio::test]
async fn stale_backend_token_cannot_forge_a_second_execution_violation() {
    let root = test_root("violation_identity");
    let log = Arc::new(EventLog::default());
    let backend = Arc::new(CrossExecutionBackend {
        first_token: Mutex::new(None),
    });
    let launcher = SandboxLauncher::new(
        BackendRegistry::native_only(Arc::new(NativeBackend::new())).with_platform_backend(backend),
        log.clone(),
    );
    let policy = agendao_sandbox::PolicyInputs::baseline(SessionPermissionMode::Default);
    launcher
        .prepare(request(&root), &policy, &PrepareOptions::default())
        .unwrap()
        .start()
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    launcher
        .prepare(request(&root), &policy, &PrepareOptions::default())
        .unwrap()
        .start()
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(
        !log.snapshot()
            .iter()
            .any(|event| matches!(event, SandboxEvent::Violation { .. })),
        "a token from execution one must not authenticate evidence for execution two",
    );
    cleanup(&root);
}
