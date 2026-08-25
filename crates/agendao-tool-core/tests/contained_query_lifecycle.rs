use std::sync::Arc;
use std::time::Duration;

use agendao_sandbox::{
    BackendRegistry, CleanupStatus, EventLog, NativeBackend, PolicyInputs, PrepareOptions,
    SandboxEvent, SandboxExecutionError, SandboxExecutionRequest, SandboxLauncher,
};
use agendao_tool_core::{
    run_contained_query, ContainedQuerySpec, ProfileKind, SandboxExecutionBoundary, ToolContext,
    ToolError,
};
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;

struct NativeAuthority {
    launcher: SandboxLauncher,
    events: Arc<EventLog>,
}

impl NativeAuthority {
    fn new() -> Self {
        let events = Arc::new(EventLog::default());
        Self {
            launcher: SandboxLauncher::new(
                BackendRegistry::native_only(Arc::new(NativeBackend::new())),
                events.clone(),
            ),
            events,
        }
    }
}

#[async_trait]
impl SandboxExecutionBoundary for NativeAuthority {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<agendao_sandbox::PreparedSandboxExecution, SandboxExecutionError> {
        self.launcher.prepare(
            request,
            &PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo),
            &options,
        )
    }
}

fn fixture() -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").expect("CARGO_TARGET_DIR must be ../target");
    let root = std::path::PathBuf::from(target)
        .join("agendao-sandbox-tests")
        .join("contained_query_lifecycle");
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

#[tokio::test]
async fn closed_pipes_then_sleep_is_killed_and_reaped_at_the_deadline() {
    let root = fixture();
    let authority = Arc::new(NativeAuthority::new());
    let ctx = ToolContext::new("s".into(), "m".into(), root.to_string_lossy().into_owned())
        .with_sandbox_execution_boundary(authority.clone() as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);
    let result = run_contained_query(
        ContainedQuerySpec {
            program: "bash".into(),
            args: vec!["-c".into(), "exec 1>&- 2>&-; sleep 30".into()],
            cwd: Some(root),
            env_overrides: Default::default(),
            label: "closed-pipe probe".into(),
        },
        &ctx,
        Duration::from_millis(300),
        ProfileKind::Native,
    )
    .await;
    if !matches!(result, Err(ToolError::Timeout(_))) {
        panic!("closed-pipe query must return ToolError::Timeout");
    }
    let events = authority.events.snapshot();
    let Some(SandboxEvent::Exited { status, .. }) = events.last() else {
        panic!("expected a terminal sandbox event");
    };
    assert_eq!(status.cleanup, CleanupStatus::TimedOut);
}
