//! Plan fixtures for platform contract tests: hand-built immutable
//! plans with the exact field combinations each platform's pure
//! mapping must distinguish.

use agendao_sandbox::model::{FilesystemMode, ProcessMode, TrustClass};
use agendao_sandbox::plan::{CanonicalPathValue, FilesystemPlan, ProcessPlan, SandboxPlan};
use agendao_sandbox::request::ProfileKind;

fn plan(
    kind: ProfileKind,
    mode: FilesystemMode,
    workspace: &str,
    writable_roots: &[&str],
) -> SandboxPlan {
    SandboxPlan {
        execution_id: "exec-contract".into(),
        trust_class: TrustClass::ModelReachable,
        requested_kind: kind,
        filesystem: FilesystemPlan {
            mode,
            workspace_root: CanonicalPathValue(workspace.into()),
            writable_roots: writable_roots
                .iter()
                .map(|root| CanonicalPathValue((*root).into()))
                .collect(),
            read_only_roots: Vec::new(),
        },
        network: agendao_sandbox::NetworkPolicy::disabled(),
        environment: agendao_sandbox::EnvironmentPolicy::default(),
        process: ProcessPlan {
            mode: ProcessMode::Contained,
            term_grace_secs: 5,
        },
        fingerprint: "contract-fixture".into(),
        session_origin: None,
    }
}

pub fn contained_workspace_write() -> SandboxPlan {
    plan(
        ProfileKind::WorkspaceWrite,
        FilesystemMode::WorkspaceWrite,
        "/ws",
        &["/ws"],
    )
}

pub fn contained_read_only() -> SandboxPlan {
    plan(ProfileKind::Check, FilesystemMode::ReadOnly, "/ws", &[])
}

pub fn contained_with_cache_root() -> SandboxPlan {
    plan(
        ProfileKind::Check,
        FilesystemMode::Restricted,
        "/ws",
        &["/ws", "/ws/target-cache"],
    )
}

pub fn contained_interactive_shell() -> SandboxPlan {
    plan(
        ProfileKind::InteractiveShell,
        FilesystemMode::WorkspaceWrite,
        "/ws",
        &["/ws"],
    )
}
