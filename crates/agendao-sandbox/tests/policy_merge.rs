//! Policy merge contract: merging only tightens, native needs explicit
//! grant at every layer, and the `Check` profile requires an
//! authority-resolved cache root.

use agendao_sandbox::{
    derive_profile, filesystem_rank, network_rank, FilesystemMode, HardPolicy, NetworkMode,
    PermissionGrantScope, PolicyError, PolicyInputs, ProcessMode, ProfileKind, TrustClass,
};
use agendao_types::SessionPermissionMode;

fn inputs(session: SessionPermissionMode) -> PolicyInputs {
    PolicyInputs::baseline(session)
}

#[test]
fn intersection_never_widens_either_side() {
    let widths = [
        (FilesystemMode::ReadOnly, NetworkMode::Disabled, true),
        (FilesystemMode::ReadOnly, NetworkMode::Disabled, false),
        (FilesystemMode::Restricted, NetworkMode::ProxyOnly, false),
        (FilesystemMode::WorkspaceWrite, NetworkMode::Disabled, false),
        (FilesystemMode::WorkspaceWrite, NetworkMode::ProxyOnly, true),
        (FilesystemMode::Unrestricted, NetworkMode::Enabled, true),
        (FilesystemMode::Unrestricted, NetworkMode::Disabled, false),
    ];
    for a in widths {
        for b in widths {
            let left = HardPolicy {
                max_filesystem: a.0,
                max_network: a.1,
                allow_native: a.2,
            };
            let right = HardPolicy {
                max_filesystem: b.0,
                max_network: b.1,
                allow_native: b.2,
            };
            let merged = left.intersect(&right);
            assert!(filesystem_rank(merged.max_filesystem) <= filesystem_rank(left.max_filesystem));
            assert!(
                filesystem_rank(merged.max_filesystem) <= filesystem_rank(right.max_filesystem)
            );
            assert!(network_rank(merged.max_network) <= network_rank(left.max_network));
            assert!(network_rank(merged.max_network) <= network_rank(right.max_network));
            assert_eq!(merged.allow_native, left.allow_native && right.allow_native);
        }
    }
}

#[test]
fn model_reachable_never_gets_native_under_default_sessions() {
    for session in [
        SessionPermissionMode::Default,
        SessionPermissionMode::TrustedWorkspace,
    ] {
        let err = derive_profile(
            TrustClass::ModelReachable,
            ProfileKind::Native,
            &inputs(session),
        )
        .unwrap_err();
        assert_eq!(err, PolicyError::NativeNotAllowed);
    }
}

#[test]
fn unsandboxed_yolo_is_the_only_default_free_channel_for_native() {
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::Native,
        &inputs(SessionPermissionMode::UnsandboxedYolo),
    )
    .expect("yolo session grants native");
    assert_eq!(profile.process.mode, ProcessMode::Native);
    assert_eq!(profile.filesystem.mode, FilesystemMode::Unrestricted);
    assert_eq!(profile.network.mode, NetworkMode::Enabled);
    assert!(
        !profile.environment.clear_and_reinject,
        "native inherits host env"
    );
}

#[test]
fn admin_hard_policy_overrides_even_yolo() {
    let mut tightened = inputs(SessionPermissionMode::UnsandboxedYolo);
    tightened.admin = Some(HardPolicy::contained_baseline());
    assert_eq!(
        derive_profile(TrustClass::ModelReachable, ProfileKind::Native, &tightened).unwrap_err(),
        PolicyError::NativeNotAllowed
    );
    // Contained execution still works under yolo + admin tightening.
    let contained = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &tightened,
    )
    .expect("contained stays available");
    assert_eq!(contained.process.mode, ProcessMode::Contained);
    assert_eq!(contained.filesystem.mode, FilesystemMode::WorkspaceWrite);
    assert_eq!(contained.network.mode, NetworkMode::Disabled);
}

#[test]
fn trusted_workspace_is_not_wider_than_default() {
    let default = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &inputs(SessionPermissionMode::Default),
    )
    .unwrap();
    let trusted = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &inputs(SessionPermissionMode::TrustedWorkspace),
    )
    .unwrap();
    assert_eq!(default, trusted);
}

#[test]
fn absent_grant_does_not_degrade_process_tools_to_read_only() {
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &inputs(SessionPermissionMode::Default),
    )
    .unwrap();
    assert_eq!(profile.filesystem.mode, FilesystemMode::WorkspaceWrite);
}

#[test]
fn file_grant_without_write_paths_is_read_only() {
    let mut with_grant = inputs(SessionPermissionMode::Default);
    with_grant.grant = Some(PermissionGrantScope {
        write_paths: Vec::new(),
        max_network: None,
    });
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &with_grant,
    )
    .unwrap();
    assert_eq!(profile.filesystem.mode, FilesystemMode::ReadOnly);
}

#[test]
fn check_profile_requires_authority_cache_root_and_stays_read_only() {
    let err = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::Check,
        &inputs(SessionPermissionMode::Default),
    )
    .unwrap_err();
    assert_eq!(err, PolicyError::CheckRequiresCacheRoot);

    let mut check = inputs(SessionPermissionMode::Default);
    check.check_build_cache_root = Some(std::path::PathBuf::from("/noncanonical/target"));
    let profile = derive_profile(TrustClass::ModelReachable, ProfileKind::Check, &check).unwrap();
    assert_eq!(profile.filesystem.mode, FilesystemMode::ReadOnly);
    assert_eq!(profile.network.mode, NetworkMode::Disabled);
    assert_eq!(profile.filesystem.writable_roots.len(), 1);
}

#[test]
fn workspace_write_profiles_default_to_denied_network() {
    // Even under yolo with a proxy-only admin ceiling, the contained
    // request bound denies the network: proxy-only egress is reserved
    // for a dedicated egress profile kind (later phase). Merge-only-
    // tightens means the strictest layer (the request) wins.
    let mut proxy_only = inputs(SessionPermissionMode::UnsandboxedYolo);
    proxy_only.admin = Some(HardPolicy {
        max_filesystem: FilesystemMode::WorkspaceWrite,
        max_network: NetworkMode::ProxyOnly,
        allow_native: false,
    });
    let profile = derive_profile(
        TrustClass::ModelReachable,
        ProfileKind::WorkspaceWrite,
        &proxy_only,
    )
    .unwrap();
    assert_eq!(profile.network.mode, NetworkMode::Disabled);
    assert_eq!(profile.filesystem.mode, FilesystemMode::WorkspaceWrite);
}

#[test]
fn integration_is_read_only_contained_even_under_yolo_without_explicit_grant() {
    let profile = derive_profile(
        TrustClass::UserConfiguredIntegration,
        ProfileKind::Integration,
        &inputs(SessionPermissionMode::UnsandboxedYolo),
    )
    .unwrap();
    assert_eq!(profile.filesystem.mode, FilesystemMode::ReadOnly);
    assert!(profile.filesystem.writable_roots.is_empty());
    assert_eq!(profile.process.mode, ProcessMode::Contained);
    assert_eq!(profile.network.mode, NetworkMode::Disabled);
}
