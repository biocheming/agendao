//! ToolContext ↔ SandboxExecutionBoundary contract (Phase 2): the
//! boundary is optional but, once installed, is the only process
//! execution surface a tool sees; it clones with the context and
//! forwards requests verbatim to the authority.

use std::sync::Arc;

use agendao_sandbox::ProfileKind;
use agendao_sandbox::{
    PrepareOptions, SandboxExecutionError, SandboxExecutionRequest, SpawnSpec, TrustClass,
};
use agendao_tool_core::{SandboxExecutionBoundary, ToolContext};
use async_trait::async_trait;

/// Records what the boundary was asked and answers with a canned error:
/// tools must surface authority failures, never route around them.
struct RecordingBoundary {
    requests: std::sync::Mutex<Vec<SandboxExecutionRequest>>,
}

#[async_trait]
impl SandboxExecutionBoundary for RecordingBoundary {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        _options: PrepareOptions,
    ) -> Result<agendao_sandbox::PreparedSandboxExecution, SandboxExecutionError> {
        self.requests.lock().unwrap().push(request);
        Err(SandboxExecutionError::SandboxUnavailable {
            backend: "none".into(),
            reason: "test boundary refuses everything".into(),
        })
    }
}

#[tokio::test]
async fn boundary_defaults_to_absent_and_is_injectable() {
    let ctx = ToolContext::new("s".into(), "m".into(), "/w".into());
    assert!(
        ctx.sandbox_execution.is_none(),
        "no authority installed by default"
    );

    let boundary: Arc<dyn SandboxExecutionBoundary> = Arc::new(RecordingBoundary {
        requests: Default::default(),
    });
    let ctx = ctx.with_sandbox_execution_boundary(boundary);
    assert!(ctx.sandbox_execution.is_some());
}

#[tokio::test]
async fn cloned_contexts_share_the_same_authority() {
    let boundary: Arc<dyn SandboxExecutionBoundary> = Arc::new(RecordingBoundary {
        requests: Default::default(),
    });
    let ctx = ToolContext::new("s".into(), "m".into(), "/w".into())
        .with_sandbox_execution_boundary(boundary.clone());
    let clone = ctx.clone();
    // Arc identity: both contexts talk to the same authority instance.
    let a = Arc::as_ptr(ctx.sandbox_execution.as_ref().unwrap()) as *const u8;
    let b = Arc::as_ptr(clone.sandbox_execution.as_ref().unwrap()) as *const u8;
    assert_eq!(a, b);
}

#[tokio::test]
async fn prepare_reaches_the_authority_verbatim() {
    let boundary = Arc::new(RecordingBoundary {
        requests: Default::default(),
    });
    let weak = Arc::downgrade(&boundary);
    let ctx = ToolContext::new("s".into(), "m".into(), "/w".into())
        .with_sandbox_execution_boundary(boundary);

    let request = SandboxExecutionRequest::new(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        SpawnSpec::new("/bin/true"),
        "/w",
    );
    let result = ctx
        .sandbox_execution
        .as_ref()
        .expect("boundary installed")
        .prepare(request.clone(), PrepareOptions::default())
        .await;

    // The refusal is the authority's typed error, surfaced unchanged.
    assert!(matches!(
        result,
        Err(SandboxExecutionError::SandboxUnavailable { .. })
    ));
    let recorded = weak.upgrade().unwrap();
    let requests = recorded.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0], request,
        "the authority sees the request verbatim"
    );
}
