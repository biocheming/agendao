//! Shared test authority for `agendao-tool-web` test modules.
//!
//! `run_git_command` (this crate) now launches git through the sandbox
//! boundary (Phase 8), so test suites that drive real git against a
//! fixture repo need a live launch authority. Mirrors the real server
//! authority: native channel plus every platform backend this build
//! registers, so the `Check` (contained, read-only) profile actually
//! runs.

use std::sync::Arc;

use agendao_sandbox::{
    BackendRegistry, EventLog, NativeBackend, PolicyInputs, PrepareOptions, SandboxLauncher,
};
use agendao_tool_core::SandboxExecutionBoundary;
use agendao_types::SessionPermissionMode;
use async_trait::async_trait;

pub(crate) struct NativeTestAuthority {
    launcher: SandboxLauncher,
}

impl NativeTestAuthority {
    pub(crate) fn new() -> Self {
        let mut registry = BackendRegistry::native_only(Arc::new(NativeBackend::new()));
        for backend in agendao_sandbox::default_platform_backends() {
            registry = registry.with_platform_backend(backend);
        }
        Self {
            launcher: SandboxLauncher::new(registry, Arc::new(EventLog::default())),
        }
    }
}

#[async_trait]
impl SandboxExecutionBoundary for NativeTestAuthority {
    async fn prepare(
        &self,
        request: agendao_sandbox::SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<agendao_sandbox::PreparedSandboxExecution, agendao_sandbox::SandboxExecutionError>
    {
        let mut inputs = PolicyInputs::baseline(SessionPermissionMode::UnsandboxedYolo);
        if request.profile_kind == agendao_sandbox::ProfileKind::Check {
            let cache = request.workspace_root.join(".agendao-test-cache");
            let _ = std::fs::create_dir_all(&cache);
            inputs.check_build_cache_root = Some(cache);
        }
        self.launcher.prepare(request, &inputs, &options)
    }
}
