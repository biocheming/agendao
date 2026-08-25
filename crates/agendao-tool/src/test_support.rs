//! Shared test authority for the `agendao-tool` unit-test modules.
//!
//! Several tools (repo history, external catalog runners) drive real
//! subprocesses in their `#[cfg(test)]` suites. Since Phase 8 the
//! boundary is the only launch path for model-reachable execution, so
//! those tests need a live launch authority: the native channel under
//! an unsandboxed-yolo policy, exactly the shape `tests/bash_sandbox.rs`
//! uses. One definition, reused by every in-crate test module, keeps the
//! launch-authority duplication (治理 KPI: 语义重复点数) at zero.

use std::path::PathBuf;
use std::sync::Arc;

use agendao_sandbox::{
    BackendRegistry, EventLog, NativeBackend, PolicyInputs, PrepareOptions, SandboxLauncher,
};
use agendao_tool_core::SandboxExecutionBoundary;
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;

/// A boundary over the native channel with an unsandboxed-yolo policy.
/// The *plumbing* (request shape, streams, lifecycle ladder) is under
/// test; containment itself is the `agendao-sandbox` runtime suite's job.
pub(crate) struct NativeTestAuthority {
    launcher: SandboxLauncher,
}

impl NativeTestAuthority {
    pub(crate) fn new() -> Self {
        // Mirrors the real server authority: native channel plus every
        // platform backend this build registers, so `Check` (contained,
        // read-only) and `WorkspaceWrite` (contained, writable) requests
        // actually run rather than fail closed for want of a backend.
        let mut registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
        for backend in agendao_sandbox::default_platform_backends() {
            registry = registry.with_platform_backend(backend);
        }
        Self {
            launcher: SandboxLauncher::new(registry, Arc::new(EventLog::default())),
        }
    }
}

/// Stable test fixture root outside the repository. Tests must not use
/// `tempfile`/`/tmp`: all artifacts belong below the configured sibling
/// Cargo target directory.
pub(crate) fn target_fixture(suite: &str, test: &str) -> PathBuf {
    let configured = PathBuf::from(
        std::env::var("CARGO_TARGET_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .expect("CARGO_TARGET_DIR must be set to ../target"),
    );
    let target = if configured.is_absolute() {
        configured
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .join(configured)
    };
    let root = target.join("agendao-tool-tests").join(suite).join(test);
    std::fs::create_dir_all(&root).expect("create test fixture directory");
    root.canonicalize()
        .expect("canonical test fixture directory")
}

#[async_trait]
impl SandboxExecutionBoundary for NativeTestAuthority {
    async fn prepare(
        &self,
        request: agendao_sandbox::SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<agendao_sandbox::PreparedSandboxExecution, agendao_sandbox::SandboxExecutionError>
    {
        // The `Check` profile mechanically requires an authority-
        // resolved build-cache root (policy.rs). The real server
        // authority supplies the session's cache dir; the test
        // authority materializes one under the request's workspace.
        let mut inputs = PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo);
        if request.profile_kind == agendao_sandbox::ProfileKind::Check {
            let cache = request.workspace_root.join(".agendao-test-cache");
            let _ = std::fs::create_dir_all(&cache);
            inputs.check_build_cache_root = Some(cache);
        }
        self.launcher.prepare(request, &inputs, &options)
    }
}
