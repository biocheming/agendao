use super::*;
use crate::test_support::target_fixture;
use crate::{Tool, ToolContext, ToolError, ToolResult};
use agendao_sandbox::{PrepareOptions, SandboxExecutionRequest};
use async_trait::async_trait;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

const SHELL_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A yolo native authority for the embedded roundtrips: these tests
/// exercise the session plumbing (write/read/terminate), not
/// containment — the sandbox crate's `linux_pty_runtime` covers the
/// contained contract against real bwrap.
async fn native_test_boundary() -> Arc<dyn agendao_tool_core::SandboxExecutionBoundary> {
    struct NativeAuthority(agendao_sandbox::SandboxLauncher);

    #[async_trait]
    impl agendao_tool_core::SandboxExecutionBoundary for NativeAuthority {
        async fn prepare(
            &self,
            request: SandboxExecutionRequest,
            options: PrepareOptions,
        ) -> Result<agendao_sandbox::PreparedSandboxExecution, agendao_sandbox::SandboxExecutionError>
        {
            self.0.prepare(
                request,
                &agendao_sandbox::PolicyInputs::baseline(
                    agendao_types::SessionPermissionMode::UnsandboxedYolo,
                ),
                &options,
            )
        }
    }

    let registry = agendao_sandbox::BackendRegistry::native_only(std::sync::Arc::new(
        agendao_sandbox::NativeBackend::new(),
    ));
    let log = std::sync::Arc::new(agendao_sandbox::EventLog::default());
    Arc::new(NativeAuthority(agendao_sandbox::SandboxLauncher::new(
        registry, log,
    )))
}

fn should_skip_pty_test(err: &ToolError) -> bool {
    matches!(err, ToolError::ExecutionError(message) if message.contains("failed to create PTY")
            || message.contains("failed to openpty")
            || message.contains("sandbox shell launch failed"))
}

async fn run_shell_test<F>(name: &str, future: F)
where
    F: Future<Output = ()>,
{
    match tokio::time::timeout(SHELL_TEST_TIMEOUT, future).await {
        Ok(()) => {}
        Err(_) => panic!(
            "shell session test `{}` exceeded {:?}; PTY tests must fail fast instead of hanging",
            name, SHELL_TEST_TIMEOUT
        ),
    }
}

fn test_exit_script(marker: &str) -> String {
    #[cfg(windows)]
    {
        format!("echo {marker}\r\nexit\r\n")
    }
    #[cfg(not(windows))]
    {
        format!("printf '{marker}\\n'\nexit\n")
    }
}

async fn wait_for_session_state(
    ctx: ToolContext,
    session_id: &serde_json::Value,
    expected: &str,
) -> ToolResult {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let status = ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "status",
                    "session_id": session_id
                }),
                ctx.clone(),
            )
            .await
            .expect("status should succeed");
        if status.metadata["session"]["state"] == serde_json::json!(expected) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "shell session did not reach `{}` before timeout; last state was {}",
            expected,
            status.metadata["session"]["state"]
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn schema_exposes_shell_session_operations() {
    let schema = ShellSessionTool::new().parameters();
    let operations = schema["properties"]["operation"]["enum"]
        .as_array()
        .expect("operation enum");
    assert!(operations.iter().any(|value| value == "start"));
    assert!(operations.iter().any(|value| value == "write"));
    assert!(operations.iter().any(|value| value == "read"));
    assert!(operations.iter().any(|value| value == "status"));
    assert!(operations.iter().any(|value| value == "terminate"));
}

#[test]
fn shell_metadata_includes_scope_key() {
    let session = ShellSessionView {
        id: "shell_1".to_string(),
        command: "bash".to_string(),
        args: vec!["-lc".to_string()],
        cwd: "/repo".to_string(),
        pid: 42,
        created_at: 0,
        state: ShellSessionState::Running,
        exit_code: None,
        error: None,
    };

    let metadata = shell_metadata("start", &session);
    assert_eq!(
        metadata.get("scope_key"),
        Some(&serde_json::json!("cmd:bash"))
    );
}

#[test]
fn session_output_buffer_is_strictly_bounded_and_keeps_the_tail() {
    let mut buffer = vec![b'a'; BUFFER_LIMIT - 2];
    append_session_output(&mut buffer, b"WXYZ");
    assert_eq!(buffer.len(), BUFFER_LIMIT);
    assert_eq!(&buffer[buffer.len() - 4..], b"WXYZ");

    append_session_output(&mut buffer, &vec![b'z'; BUFFER_LIMIT + 17]);
    assert_eq!(buffer.len(), BUFFER_LIMIT);
    assert!(buffer.iter().all(|byte| *byte == b'z'));
}

#[tokio::test]
async fn shell_session_roundtrip_start_write_read_status() {
    run_shell_test("shell_session_roundtrip_start_write_read_status", async {
        let dir = target_fixture("shell_session_unit", "roundtrip");
        let permissions = Arc::new(AsyncMutex::new(Vec::<String>::new()));
        let permissions_clone = permissions.clone();
        let ctx = ToolContext::new(
            "session-1".into(),
            "message-1".into(),
            dir.to_string_lossy().to_string(),
        )
        .with_sandbox_execution_boundary(native_test_boundary().await)
        .with_sandbox_native_allowed(true)
        .with_ask(move |req| {
            let permissions_clone = permissions_clone.clone();
            async move {
                permissions_clone.lock().await.push(req.permission);
                Ok(())
            }
        });

        let start = match ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "start",
                    "description": "Start shell for structured tool testing"
                }),
                ctx.clone(),
            )
            .await
        {
            Ok(result) => result,
            Err(err) if should_skip_pty_test(&err) => {
                eprintln!(
                    "skipping PTY integration test in current environment: {}",
                    err
                );
                return;
            }
            Err(err) => panic!("shell session start should succeed: {}", err),
        };
        let session_id = start.metadata["session"]["id"]
            .as_str()
            .expect("session id")
            .to_string();

        let marker = "hello-shell";
        ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "write",
                    "session_id": session_id,
                    "input": test_exit_script(marker),
                    "description": "Emit a marker and exit"
                }),
                ctx.clone(),
            )
            .await
            .expect("write should succeed");

        let read = ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "read",
                    "session_id": start.metadata["session"]["id"],
                    "cursor": 0,
                    "wait_ms": 2_000
                }),
                ctx.clone(),
            )
            .await
            .expect("read should succeed");
        assert!(read.output.contains(marker), "output was: {}", read.output);

        let status = wait_for_session_state(ctx, &start.metadata["session"]["id"], "exited").await;
        assert_eq!(
            status.metadata["session"]["state"],
            serde_json::json!("exited")
        );

        let permissions = permissions.lock().await;
        assert!(
            permissions
                .iter()
                .filter(|item| item.as_str() == "bash")
                .count()
                >= 2
        );
    })
    .await;
}

#[tokio::test]
async fn shell_session_terminate_stops_running_process() {
    run_shell_test("shell_session_terminate_stops_running_process", async {
        let dir = target_fixture("shell_session_unit", "terminate");
        let ctx = ToolContext::new(
            "session-2".into(),
            "message-2".into(),
            dir.to_string_lossy().to_string(),
        )
        .with_sandbox_execution_boundary(native_test_boundary().await)
        .with_sandbox_native_allowed(true);
        let start = match ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "start"
                }),
                ctx.clone(),
            )
            .await
        {
            Ok(result) => result,
            Err(err) if should_skip_pty_test(&err) => {
                eprintln!(
                    "skipping PTY integration test in current environment: {}",
                    err
                );
                return;
            }
            Err(err) => panic!("start should succeed: {}", err),
        };
        let session_id = start.metadata["session"]["id"]
            .as_str()
            .expect("session id")
            .to_string();

        ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "terminate",
                    "session_id": session_id
                }),
                ctx.clone(),
            )
            .await
            .expect("terminate should succeed");

        tokio::time::sleep(Duration::from_millis(250)).await;
        let status = ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "status",
                    "session_id": start.metadata["session"]["id"]
                }),
                ctx,
            )
            .await
            .expect("status should succeed");
        assert_ne!(
            status.metadata["session"]["state"],
            serde_json::json!("running")
        );
    })
    .await;
}
