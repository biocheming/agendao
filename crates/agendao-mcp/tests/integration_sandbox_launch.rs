//! Stdio MCP servers launch through the sandbox execution boundary
//! (Phase 6): `TrustClass::UserConfiguredIntegration` under the
//! `Integration` profile — contained, workspace-scoped, network denied.
//! These tests pin the request shape the boundary receives and the
//! fail-loudly contract when no authority is installed; the deny-all
//! boundary doubles as the assertion surface, so no real backend is
//! needed to keep the contract tested on every host.

use std::sync::Arc;

use agendao_mcp::client::{McpClientRegistry, McpServerConfig};
use agendao_mcp::{McpClientError, StdioTransport};
use agendao_sandbox::model::TrustClass;
use agendao_sandbox::{
    IntegrationSandboxContext, PrepareOptions, PreparedSandboxExecution, ProfileKind,
    SandboxExecutionBoundary, SandboxExecutionError, SandboxExecutionRequest,
};
use async_trait::async_trait;

/// Records what the boundary was asked and answers with a canned
/// denial: the transport must surface authority failures, never route
/// around them (same shape as the tool-core boundary tests).
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

fn recording_context(
    workspace: &std::path::Path,
) -> (IntegrationSandboxContext, Arc<RecordingBoundary>) {
    let boundary = Arc::new(RecordingBoundary {
        requests: Default::default(),
    });
    (
        IntegrationSandboxContext::without_runtime_roots(boundary.clone(), workspace.to_path_buf()),
        boundary,
    )
}

#[tokio::test]
async fn stdio_launches_reach_the_boundary_with_integration_shape() {
    let workspace = std::env::temp_dir();
    let (sandbox, boundary) = recording_context(&workspace);

    let launch = StdioTransport::new(
        "some-mcp-server",
        &["--flag".to_string()],
        Some(vec![("KEY".to_string(), "value".to_string())]),
        sandbox,
    )
    .await;
    let err = match launch {
        Err(err) => err,
        Ok(_) => panic!("denial must propagate, not fall back to a direct spawn"),
    };

    assert!(
        err.to_string()
            .contains("sandbox denied the MCP server launch"),
        "transport must explain the denial, got: {}",
        err
    );

    let requests = boundary.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "exactly one launch attempt");
    let request = &requests[0];
    assert_eq!(request.trust_class, TrustClass::UserConfiguredIntegration);
    assert_eq!(request.profile_kind, ProfileKind::Integration);
    assert_eq!(request.workspace_root, workspace);
    assert_eq!(request.spec.program, "some-mcp-server");
    assert_eq!(request.spec.args, vec!["--flag".to_string()]);
    assert_eq!(
        request.spec.cwd.as_deref(),
        Some(workspace.as_path()),
        "stdio servers run with the workspace as cwd"
    );
    assert_eq!(
        request.spec.env_overrides.get("KEY").map(String::as_str),
        Some("value")
    );
}

#[tokio::test]
async fn registry_without_authority_fails_loudly_on_stdio() {
    let registry = McpClientRegistry::new();
    let connect = registry
        .add_stdio(McpServerConfig {
            name: "local".into(),
            command: "some-mcp-server".into(),
            args: vec![],
            env: None,
            timeout_ms: None,
        })
        .await;
    let err = match connect {
        Err(err) => err,
        Ok(_) => panic!("no authority installed: stdio must fail, not spawn directly"),
    };

    assert!(
        matches!(err, McpClientError::TransportError(ref message) if
            message.contains("sandbox execution authority")),
        "the error must name the missing authority, got: {}",
        err
    );
}
