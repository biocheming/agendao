//! Plugin host processes launch through the sandbox execution boundary
//! (Phase 6): `TrustClass::UserConfiguredIntegration` under the
//! `Integration` profile — contained, workspace-scoped, network denied.
//! These tests pin the request shape the boundary receives and the
//! fail-loudly contract when no authority is installed; the deny-all
//! boundary doubles as the assertion surface, so no real backend is
//! needed to keep the contract tested on every host.

use std::sync::Arc;

use agendao_plugin::subprocess::client::PluginSubprocess;
use agendao_plugin::subprocess::loader::{PluginLoader, PluginLoaderError};
use agendao_plugin::subprocess::{JsRuntime, PluginContext};
use agendao_sandbox::model::TrustClass;
use agendao_sandbox::{
    IntegrationSandboxContext, PrepareOptions, PreparedSandboxExecution, ProfileKind,
    SandboxExecutionBoundary, SandboxExecutionError, SandboxExecutionRequest,
};
use async_trait::async_trait;

/// Records what the boundary was asked and answers with a canned
/// denial: the launcher must surface authority failures, never route
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

fn plugin_context() -> PluginContext {
    PluginContext {
        worktree: "/tmp".into(),
        directory: "/tmp".into(),
        server_url: "http://127.0.0.1:3000".into(),
        internal_token: String::new(),
    }
}

#[tokio::test]
async fn plugin_host_launches_reach_the_boundary_with_integration_shape() {
    let workspace = std::env::temp_dir().join("plugin-sandbox-probe");
    let (sandbox, boundary) = recording_context(&workspace);
    let npm_dir = workspace.join("npm");

    // The deny-all boundary rejects at prepare, so the runtime binary is
    // never executed — the enum can name any runtime without the host
    // needing it installed.
    let launch = PluginSubprocess::spawn(
        JsRuntime::Node,
        "plugin-host.ts",
        "file:///tmp/some-plugin.ts",
        plugin_context(),
        Some(npm_dir.as_path()),
        sandbox,
    )
    .await;
    let err = match launch {
        Err(err) => err,
        Ok(_) => panic!("denial must propagate, not fall back to a direct spawn"),
    };

    assert!(
        err.to_string()
            .contains("sandbox denied the plugin host launch"),
        "launcher must explain the denial, got: {}",
        err
    );

    let requests = boundary.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "exactly one launch attempt");
    let request = &requests[0];
    assert_eq!(request.trust_class, TrustClass::UserConfiguredIntegration);
    assert_eq!(request.profile_kind, ProfileKind::Integration);
    assert_eq!(request.workspace_root, workspace);
    assert_eq!(request.spec.program, "node");
    assert_eq!(
        request.spec.args,
        vec![
            "--experimental-strip-types".to_string(),
            "plugin-host.ts".to_string()
        ]
    );
    assert_eq!(
        request.spec.cwd.as_deref(),
        Some(npm_dir.as_path()),
        "npm-resolved plugins run with the npm dir as cwd so bare imports resolve"
    );
}

#[tokio::test]
async fn loader_without_authority_fails_loudly_on_load_all() {
    let loader = match PluginLoader::new() {
        Ok(loader) => loader,
        Err(PluginLoaderError::NoRuntime) => {
            panic!("this host has a JS runtime; NoRuntime here means detection broke")
        }
        Err(error) => panic!("unexpected loader construction failure: {}", error),
    };

    let load = loader
        .load_all(
            &["file:///tmp/some-plugin.ts".to_string()],
            &plugin_context(),
        )
        .await;
    let err = match load {
        Err(err) => err,
        Ok(_) => panic!("no authority installed: load_all must fail, not spawn directly"),
    };

    assert!(
        matches!(err, PluginLoaderError::NoSandboxAuthority),
        "the error must name the missing authority, got: {}",
        err
    );
}
