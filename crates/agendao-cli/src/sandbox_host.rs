//! The CLI host's sandbox launch authority.
//!
//! The CLI process is its own authority host (it does not embed the
//! server): production backends — the platform contained backend plus
//! the explicit native channel — under the default session mode. Every
//! user-configured integration this process launches (LSP servers,
//! plugin hosts) goes through this boundary contained; there is no
//! direct-spawn path (sandbox plan Phase 6).

use std::path::PathBuf;
use std::sync::Arc;

use agendao_sandbox::{
    IntegrationSandboxContext, PrepareOptions, PreparedSandboxExecution, SandboxExecutionError,
    SandboxExecutionRequest, SandboxLauncher,
};
use async_trait::async_trait;

struct CliSandboxAuthority(SandboxLauncher);

#[async_trait]
impl agendao_sandbox::SandboxExecutionBoundary for CliSandboxAuthority {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        self.0.prepare(
            request,
            &agendao_sandbox::PolicyInputs::baseline(agendao_types::SessionPermissionMode::Default),
            &options,
        )
    }
}

/// Build the integration launch context for this process: the boundary
/// plus the workspace integrations are scoped to.
pub fn cli_integration_sandbox_context(workspace: PathBuf) -> IntegrationSandboxContext {
    let mut registry =
        agendao_sandbox::BackendRegistry::native_only(agendao_sandbox::native_backend());
    for backend in agendao_sandbox::default_platform_backends() {
        registry = registry.with_platform_backend(backend);
    }
    let launcher = agendao_sandbox::SandboxLauncher::new(
        registry,
        Arc::new(agendao_sandbox::EventLog::default()),
    );
    IntegrationSandboxContext::without_runtime_roots(
        Arc::new(CliSandboxAuthority(launcher)),
        workspace,
    )
}
