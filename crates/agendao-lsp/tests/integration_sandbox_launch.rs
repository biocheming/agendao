//! LSP servers launch through the sandbox execution boundary (Phase 6):
//! `TrustClass::UserConfiguredIntegration` under the `Integration`
//! profile — contained, workspace-scoped, network denied. The deny-all
//! boundary doubles as the assertion surface, so no real backend is
//! needed to keep the contract tested on every host.

use std::sync::Arc;

use agendao_lsp::{LspClient, LspServerConfig};
use agendao_sandbox::model::TrustClass;
use agendao_sandbox::{
    IntegrationSandboxContext, PrepareOptions, PreparedSandboxExecution, ProfileKind,
    SandboxExecutionBoundary, SandboxExecutionError, SandboxExecutionRequest,
};
use async_trait::async_trait;

/// Records what the boundary was asked and answers with a canned
/// denial: the client must surface authority failures, never route
/// around them.
struct RecordingBoundary {
    requests: std::sync::Mutex<Vec<SandboxExecutionRequest>>,
}

#[async_trait]
impl SandboxExecutionBoundary for RecordingBoundary {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        _options: PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        self.requests.lock().unwrap().push(request);
        Err(SandboxExecutionError::SandboxUnavailable {
            backend: "none".into(),
            reason: "test boundary refuses everything".into(),
        })
    }
}

#[tokio::test]
async fn lsp_launches_reach_the_boundary_with_integration_shape() {
    let workspace = std::env::temp_dir();
    let boundary = Arc::new(RecordingBoundary {
        requests: Default::default(),
    });
    let sandbox =
        IntegrationSandboxContext::without_runtime_roots(boundary.clone(), workspace.clone());

    let start = LspClient::start(
        LspServerConfig {
            id: "rust".into(),
            command: "rust-analyzer".into(),
            args: vec!["--flag".into()],
            initialization_options: None,
        },
        workspace.clone(),
        sandbox,
    )
    .await;
    let err = match start {
        Err(err) => err,
        Ok(_) => panic!("denial must propagate, not fall back to a direct spawn"),
    };

    let message = err.to_string();
    assert!(
        message.contains("sandbox denied the LSP server launch"),
        "client must explain the denial, got: {}",
        message
    );

    let requests = boundary.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "exactly one launch attempt");
    let request = &requests[0];
    assert_eq!(request.trust_class, TrustClass::UserConfiguredIntegration);
    assert_eq!(request.profile_kind, ProfileKind::Integration);
    assert_eq!(request.workspace_root, workspace);
    assert_eq!(request.spec.program, "rust-analyzer");
    assert_eq!(request.spec.args, vec!["--flag".to_string()]);
    assert_eq!(request.spec.cwd.as_deref(), Some(workspace.as_path()));
}
