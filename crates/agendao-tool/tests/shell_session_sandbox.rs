//! Shell-session tool ↔ sandbox boundary integration (Phase 4): the
//! interactive tool launches only through the boundary — pty master
//! streams back, the private interactive HOME hides the host's — and
//! fails loudly without an installed authority. Containment specifics
//! live in `agendao-sandbox`'s bwrap pty suite; these tests pin the
//! tool plumbing.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agendao_sandbox::{
    BackendRegistry, BwrapBackend, EventLog, NativeBackend, PolicyInputs, PrepareOptions,
    SandboxBackend, SandboxExecutionError, SandboxExecutionRequest, SandboxLauncher,
};
use agendao_tool::shell_session::ShellSessionTool;
use agendao_tool_core::{SandboxExecutionBoundary, Tool, ToolContext};
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;

/// Default-session authority with the production registry: native plus
/// the real bwrap backend when the host has one, so InteractiveShell
/// requests run contained exactly as production would.
struct SessionAuthority {
    launcher: SandboxLauncher,
}

impl SessionAuthority {
    fn new() -> Self {
        let registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
        #[cfg(target_os = "linux")]
        let registry = registry.with_platform_backend(Arc::new(BwrapBackend::discover()));
        Self {
            launcher: SandboxLauncher::new(registry, Arc::new(EventLog::default())),
        }
    }
}

#[async_trait]
impl SandboxExecutionBoundary for SessionAuthority {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<agendao_sandbox::PreparedSandboxExecution, SandboxExecutionError> {
        self.launcher.prepare(
            request,
            &PolicyInputs::baseline(SessionPermissionMode::Default),
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
        .join("shell_session_sandbox")
        .join(test);
    std::fs::create_dir_all(&root).unwrap();
    // bwrap assembles a fresh rootfs before --chdir, so a relative cwd
    // resolves inside the sandbox tree and the launch dies instantly —
    // the pty then echoes writes forever with no reader behind them.
    root.canonicalize().unwrap()
}

fn ctx_with(dir: &Path, boundary: Option<Arc<dyn SandboxExecutionBoundary>>) -> ToolContext {
    let ctx = ToolContext::new("s".into(), "m".into(), dir.to_string_lossy().into_owned());
    match boundary {
        Some(boundary) => ctx.with_sandbox_execution_boundary(boundary),
        None => ctx,
    }
}

#[cfg(target_os = "linux")]
fn bwrap_available() -> bool {
    BwrapBackend::discover().probe().available
}

async fn start_session(ctx: ToolContext, command: &str) -> agendao_tool::ToolResult {
    ShellSessionTool::new()
        .execute(
            serde_json::json!({
                "operation": "start",
                "command": command,
                "description": "integration test session",
            }),
            ctx,
        )
        .await
        .unwrap()
}

async fn read_until(
    ctx: ToolContext,
    session_id: &str,
    needle: &str,
    deadline: Duration,
) -> String {
    let start = std::time::Instant::now();
    let mut acc = String::new();
    while start.elapsed() < deadline {
        let read = ShellSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "read",
                    "session_id": session_id,
                    "cursor": 0,
                    "wait_ms": 500,
                }),
                ctx.clone(),
            )
            .await
            .unwrap();
        // cursor 0 re-reads the whole retained buffer each poll, so the
        // latest output supersedes the accumulated snapshot.
        acc = read.output;
        if acc.contains(needle) {
            return acc;
        }
    }
    acc
}

#[tokio::test]
async fn fails_loudly_without_an_installed_authority() {
    let root = fixture("no_authority");
    let result = ShellSessionTool::new()
        .execute(
            serde_json::json!({
                "operation": "start",
                "description": "must fail without a boundary",
            }),
            ctx_with(&root, None),
        )
        .await;
    let err = result.expect_err("start without boundary must fail");
    assert!(
        err.to_string()
            .contains("no SandboxExecutionBoundary is installed"),
        "error: {err}"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn contained_session_reports_the_private_home() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = fixture("private_home");
    let ctx = ctx_with(&root, Some(Arc::new(SessionAuthority::new())));

    let start = start_session(ctx.clone(), "/bin/sh").await;
    let session_id = start.metadata["session"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Ask the session for its HOME; the answer must be the private
    // interactive path, never the host's.
    tokio::time::sleep(Duration::from_millis(500)).await;
    ShellSessionTool::new()
        .execute(
            serde_json::json!({
                "operation": "write",
                "session_id": session_id,
                "input": "printf 'home=%s\\n' \"$HOME\"",
                "append_newline": true,
                "description": "print HOME",
            }),
            ctx.clone(),
        )
        .await
        .unwrap();
    let output = read_until(
        ctx.clone(),
        &session_id,
        "home=/tmp/agendao-home",
        Duration::from_secs(10),
    )
    .await;
    assert!(
        output.contains("home=/tmp/agendao-home"),
        "private HOME reported: {output}"
    );
    assert!(
        !output.contains(&format!(
            "home={}",
            std::env::var("HOME").unwrap_or_default()
        )),
        "host HOME must not leak: {output}"
    );
    terminate_session(ctx, &session_id).await;
}

/// The pty read pump parks on the master until the session ends; without
/// termination the #[tokio::test] runtime drop would wait on that blocking
/// read forever. Registering a session promises to end it.
async fn terminate_session(ctx: ToolContext, session_id: &str) {
    let _ = ShellSessionTool::new()
        .execute(
            serde_json::json!({
                "operation": "terminate",
                "session_id": session_id,
                "description": "test teardown",
            }),
            ctx,
        )
        .await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn contained_session_runs_the_workspace_not_the_host() {
    if !bwrap_available() {
        eprintln!("skipping: bwrap not usable on this host");
        return;
    }
    let root = fixture("workspace");
    let ctx = ctx_with(&root, Some(Arc::new(SessionAuthority::new())));

    let start = start_session(ctx.clone(), "/bin/sh").await;
    let session_id = start.metadata["session"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The session's cwd is the workspace; writing there must land in
    // the host-visible fixture directory (bind, not copy).
    tokio::time::sleep(Duration::from_millis(500)).await;
    ShellSessionTool::new()
        .execute(
            serde_json::json!({
                "operation": "write",
                "session_id": session_id,
                "input": "pwd > pwd.txt; echo done-pwd",
                "append_newline": true,
                "description": "record cwd",
            }),
            ctx.clone(),
        )
        .await
        .unwrap();
    let _ = read_until(
        ctx.clone(),
        &session_id,
        "done-pwd",
        Duration::from_secs(10),
    )
    .await;
    terminate_session(ctx, &session_id).await;
    let recorded = std::fs::read_to_string(root.join("pwd.txt")).expect("workspace write landed");
    assert_eq!(
        std::path::Path::new(recorded.trim())
            .canonicalize()
            .unwrap(),
        root.canonicalize().unwrap(),
        "session cwd is the tool context directory"
    );
}
