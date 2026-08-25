//! Bash tool ↔ sandbox boundary integration (Phase 4): bash launches
//! only through the boundary — streaming output, exit codes, timeout,
//! cancellation — and refuses to run at all without an installed
//! authority (no direct-spawn fallback, sandbox plan §4.4).
//!
//! The boundary here is the real launcher over the native channel with
//! an unsandboxed-yolo policy: these tests pin the *plumbing* (request
//! shape, streams, lifecycle ladder), while containment itself is
//! covered by the bwrap runtime suite in `agendao-sandbox`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agendao_sandbox::{
    BackendRegistry, CleanupStatus, EventLog, NativeBackend, PolicyInputs, PrepareOptions,
    SandboxEvent, SandboxExecutionError, SandboxExecutionRequest, SandboxLauncher,
};
use agendao_tool::bash::BashTool;
use agendao_tool_core::{SandboxExecutionBoundary, Tool, ToolContext, ToolError};
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;

struct NativeTestAuthority {
    launcher: SandboxLauncher,
    events: Arc<EventLog>,
}

impl NativeTestAuthority {
    fn new() -> Self {
        let events = Arc::new(EventLog::default());
        let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
        Self {
            launcher: SandboxLauncher::new(registry, events.clone()),
            events,
        }
    }
}

#[async_trait]
impl SandboxExecutionBoundary for NativeTestAuthority {
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

fn fixture(test: &str) -> PathBuf {
    let configured = std::path::PathBuf::from(
        std::env::var("CARGO_TARGET_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("CARGO_TARGET_DIR must be set (../target)")),
    );
    let target = if configured.is_absolute() {
        configured
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .join(configured)
    };
    let root = target
        .join("agendao-sandbox-tests")
        .join("bash_sandbox")
        .join(test);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn ctx_in(dir: &Path) -> ToolContext {
    ToolContext::new("s".into(), "m".into(), dir.to_string_lossy().into_owned())
}

fn bash_args(command: &str) -> serde_json::Value {
    serde_json::json!({
        "command": command,
        "description": "test",
    })
}

#[tokio::test]
async fn runs_through_the_boundary_and_streams_output() {
    let root = fixture("streams");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority.clone() as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);

    let result = BashTool::new()
        .execute(bash_args("echo hello-from-boundary"), ctx)
        .await
        .unwrap();

    assert!(
        result.output.contains("hello-from-boundary"),
        "output streamed back through the boundary: {}",
        result.output
    );
    assert_eq!(
        result.metadata.get("exit_code"),
        Some(&serde_json::json!(0))
    );

    let events = authority.events.snapshot();
    assert_eq!(events.len(), 3, "prepared -> started -> exited");
    assert!(matches!(events[0], SandboxEvent::Prepared { .. }));
    assert!(matches!(events[1], SandboxEvent::Started { .. }));
    assert!(matches!(events[2], SandboxEvent::Exited { .. }));
}

#[tokio::test]
async fn child_exit_code_propagates_to_the_tool_result() {
    let root = fixture("exit_code");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);

    let result = BashTool::new()
        .execute(bash_args("exit 7"), ctx)
        .await
        .unwrap();

    assert_eq!(
        result.metadata.get("exit_code"),
        Some(&serde_json::json!(7)),
        "the sandboxed child's status is the tool result"
    );
    assert!(result.output.contains("exited with code: 7"));
}

#[tokio::test]
async fn fails_loudly_without_an_installed_authority() {
    let root = fixture("no_authority");
    let ctx = ctx_in(&root);
    assert!(ctx.sandbox_execution.is_none());

    let err = BashTool::new()
        .execute(bash_args("touch no-authority-proof.txt"), ctx)
        .await
        .unwrap_err();

    match err {
        ToolError::ExecutionError(message) => {
            assert!(
                message.contains("sandbox execution authority"),
                "the error names the missing authority: {message}"
            );
        }
        other => panic!("expected ExecutionError, got {other:?}"),
    }
    assert!(
        !root.join("no-authority-proof.txt").exists(),
        "no process may run without the boundary"
    );
}

#[tokio::test]
async fn cancellation_runs_the_boundary_ladder() {
    let root = fixture("cancel");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority.clone() as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);

    let abort = ctx.abort.clone();
    let task = tokio::spawn(async move {
        BashTool::new()
            .execute(bash_args("sleep 30"), ctx)
            .await
            .map(|_| ())
    });
    // Let the launch reach the running state before aborting.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    abort.cancel();

    let outcome = task.await.unwrap();
    assert!(
        matches!(outcome, Err(ToolError::Cancelled)),
        "abort surfaces as Cancelled: {outcome:?}"
    );

    let events = authority.events.snapshot();
    let Some(SandboxEvent::Exited { status, .. }) = events.last() else {
        panic!("expected a terminal Exited event, got {events:?}");
    };
    assert_eq!(status.cleanup, CleanupStatus::TerminatedByRequest);
}

#[tokio::test]
async fn timeout_cancels_through_the_boundary() {
    let root = fixture("timeout");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority.clone() as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);

    let args = serde_json::json!({
        "command": "sleep 30",
        "description": "test",
        "timeout": 300,
    });
    let err = BashTool::new().execute(args, ctx).await.unwrap_err();
    assert!(
        err.to_string().contains("timed out"),
        "timeout error surfaces: {err}"
    );

    let events = authority.events.snapshot();
    let Some(SandboxEvent::Exited { status, .. }) = events.last() else {
        panic!("expected a terminal Exited event, got {events:?}");
    };
    assert_eq!(status.cleanup, CleanupStatus::TimedOut);
}

#[tokio::test]
async fn stdout_eof_keeps_stderr_draining() {
    let root = fixture("stdout_eof_stderr_continues");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);

    let result = BashTool::new()
        .execute(
            bash_args("exec 1>&-; sleep 0.05; printf stderr-after-stdout-eof >&2"),
            ctx,
        )
        .await
        .unwrap();
    assert!(
        result.output.contains("stderr-after-stdout-eof"),
        "stderr must still be drained after stdout EOF: {}",
        result.output
    );
}

#[tokio::test]
async fn unterminated_output_is_bounded_and_marked_truncated() {
    let root = fixture("unterminated_output");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        BashTool::new().execute(bash_args("yes x | tr -d '\\n' | head -c 200000"), ctx),
    )
    .await
    .expect("unbroken output must not make the tool hang")
    .unwrap();
    assert!(result.truncated);
    assert!(result.output.contains("Output truncated at 51200 bytes"));
    assert!(result.output.len() <= 50 * 1024);
    assert_eq!(
        result.metadata.get("output_limit_bytes"),
        Some(&serde_json::json!(50 * 1024))
    );
}

#[tokio::test]
async fn closed_pipes_then_sleep_still_obey_the_deadline() {
    let root = fixture("closed_pipes_then_sleep");
    let authority = Arc::new(NativeTestAuthority::new());
    let ctx = ctx_in(&root)
        .with_sandbox_execution_boundary(authority.clone() as Arc<dyn SandboxExecutionBoundary>)
        .with_sandbox_native_allowed(true);
    let args = serde_json::json!({
        "command": "exec 1>&- 2>&-; sleep 30",
        "description": "test",
        "timeout": 300,
    });
    let error = BashTool::new().execute(args, ctx).await.unwrap_err();
    assert!(error.to_string().contains("timed out"), "error: {error}");
    let events = authority.events.snapshot();
    let Some(SandboxEvent::Exited { status, .. }) = events.last() else {
        panic!("expected terminal event");
    };
    assert_eq!(status.cleanup, CleanupStatus::TimedOut);
}
