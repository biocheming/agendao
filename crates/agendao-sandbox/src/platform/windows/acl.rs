//! Protected-metadata DACL generation for the Windows backend.
//!
//! Pure SDDL text construction — the same protected-metadata boundary
//! bwrap enforces with stacked read-only binds and Seatbelt with
//! trailing deny rules. On Windows the boundary is a per-directory
//! DACL: deny-write ACEs on each protected component under the
//! workspace, applied by the integration layer via
//! `SetNamedSecurityInfoW` when the kernel path lands.

use crate::path::PROTECTED_METADATA_COMPONENTS;
use crate::plan::SandboxPlan;

/// Deny-write ACE string for one protected directory: denies every
/// write-flavored right to World (`WD`) while leaving reads intact so
/// git and the agents can still traverse their metadata. Mask
/// `0x50156` = `DELETE | FILE_WRITE_DATA | FILE_APPEND_DATA |
/// FILE_WRITE_EA | FILE_DELETE_CHILD | FILE_WRITE_ATTRIBUTES |
/// WRITE_DAC` (the last one blocks DACL-editing around the deny).
pub fn deny_write_sddl() -> &'static str {
    "D:AI(D;;0x50156;;;WD)"
}

/// The protected metadata directories under the workspace that must
/// stay read-only even when the workspace itself is writable.
pub fn protected_metadata_dirs(plan: &SandboxPlan) -> Vec<String> {
    let workspace = plan.filesystem.workspace_root.as_str();
    let workspace_writable = plan
        .filesystem
        .writable_roots
        .iter()
        .any(|root| root.as_str() == workspace);
    if !workspace_writable {
        return Vec::new();
    }
    PROTECTED_METADATA_COMPONENTS
        .iter()
        .map(|component| format!("{workspace}\\{component}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(workspace: &str, writable: &[&str]) -> SandboxPlan {
        SandboxPlan {
            execution_id: "exec-test".into(),
            trust_class: crate::model::TrustClass::ModelReachable,
            requested_kind: crate::request::ProfileKind::WorkspaceWrite,
            filesystem: crate::plan::FilesystemPlan {
                mode: crate::model::FilesystemMode::WorkspaceWrite,
                workspace_root: crate::plan::CanonicalPathValue(workspace.into()),
                writable_roots: writable
                    .iter()
                    .map(|root| crate::plan::CanonicalPathValue((*root).into()))
                    .collect(),
                read_only_roots: Vec::new(),
            },
            network: crate::network::NetworkPolicy::disabled(),
            environment: crate::environment::EnvironmentPolicy::default(),
            process: crate::plan::ProcessPlan {
                mode: crate::model::ProcessMode::Contained,
                term_grace_secs: 5,
            },
            fingerprint: "fp".into(),
            session_origin: None,
        }
    }

    #[test]
    fn protected_dirs_cover_every_component_when_workspace_writable() {
        let plan = plan_with("C:\\ws", &["C:\\ws"]);
        let dirs = protected_metadata_dirs(&plan);
        let expected: Vec<String> = [".git", ".agendao", ".agents", ".codex"]
            .iter()
            .map(|component| format!("C:\\ws\\{component}"))
            .collect();
        assert_eq!(dirs, expected);
    }

    #[test]
    fn read_only_workspace_has_no_protected_dirs() {
        let plan = plan_with("C:\\ws", &[]);
        assert!(protected_metadata_dirs(&plan).is_empty());
    }

    #[test]
    fn deny_sddl_is_a_protective_dacl() {
        assert!(deny_write_sddl().starts_with("D:"));
    }
}
