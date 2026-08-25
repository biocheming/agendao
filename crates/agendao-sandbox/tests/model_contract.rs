//! Model contract: enum wire forms and profile construction guarantees.

use agendao_sandbox::{
    EnvironmentPolicy, FilesystemMode, FilesystemPolicy, NetworkMode, NetworkPolicy, ProcessMode,
    ProcessPolicy, SandboxProfile, TrustClass,
};

#[test]
fn trust_class_serializes_snake_case() {
    for (value, text) in [
        (TrustClass::ModelReachable, "\"model_reachable\""),
        (
            TrustClass::UserConfiguredIntegration,
            "\"user_configured_integration\"",
        ),
        (TrustClass::HostManagement, "\"host_management\""),
    ] {
        assert_eq!(serde_json::to_string(&value).unwrap(), text);
        let parsed: TrustClass = serde_json::from_str(text).unwrap();
        assert_eq!(parsed, value);
    }
}

#[test]
fn mode_enums_round_trip_snake_case() {
    for (filesystem, network, process) in [
        (
            FilesystemMode::ReadOnly,
            NetworkMode::Disabled,
            ProcessMode::Contained,
        ),
        (
            FilesystemMode::WorkspaceWrite,
            NetworkMode::ProxyOnly,
            ProcessMode::Native,
        ),
        (
            FilesystemMode::Restricted,
            NetworkMode::Enabled,
            ProcessMode::Native,
        ),
        (
            FilesystemMode::Unrestricted,
            NetworkMode::Enabled,
            ProcessMode::Native,
        ),
    ] {
        let json = serde_json::json!({
            "filesystem": filesystem,
            "network": network,
            "process": process,
        });
        let text = serde_json::to_string(&json).unwrap();
        assert!(
            !text.contains("WorkspaceWrite") && !text.contains("Disabled"),
            "enum variants must serialize as snake_case, got {text}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, json);
    }
}

#[test]
fn contained_workspace_write_profile_is_default_strict() {
    let profile = SandboxProfile::contained_workspace_write();
    assert_eq!(profile.trust_class, TrustClass::ModelReachable);
    assert_eq!(profile.filesystem.mode, FilesystemMode::WorkspaceWrite);
    assert!(profile.filesystem.writable_roots.is_empty());
    assert_eq!(profile.network.mode, NetworkMode::Disabled);
    assert!(profile.environment.clear_and_reinject);
    assert_eq!(profile.process.mode, ProcessMode::Contained);
}

#[test]
fn default_environment_policy_is_deny_strict() {
    let policy = EnvironmentPolicy::default();
    assert!(policy.clear_and_reinject);
    assert!(policy.inherit_core);
    // AgenDao-internal credentials are hard-denied by default.
    assert!(policy.hard_deny_exact.contains("AGENDAO_SERVER_PASSWORD"));
    // Secret heuristics ship by default.
    assert!(!policy.deny_patterns.is_empty());
    assert!(policy.allow_exact.is_empty());
}

#[test]
fn sandbox_profile_is_cloneable_and_comparable_for_plan_reuse() {
    let profile = SandboxProfile {
        trust_class: TrustClass::UserConfiguredIntegration,
        filesystem: FilesystemPolicy {
            mode: FilesystemMode::ReadOnly,
            writable_roots: Vec::new(),
            read_only_roots: Vec::new(),
        },
        network: NetworkPolicy::disabled(),
        environment: EnvironmentPolicy::default(),
        process: ProcessPolicy {
            mode: ProcessMode::Contained,
        },
    };
    let clone = profile.clone();
    assert_eq!(clone, profile);
}
