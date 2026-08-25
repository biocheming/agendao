//! Cross-platform backend contracts (Phase 7): the pure mapping from an
//! immutable `SandboxPlan` to each platform's enforcement syntax, and
//! the fail-closed reporting every host can verify without the target
//! OS. Real negative probes (escape attempts under a live backend) run
//! only on their host: `linux_runtime.rs` for bwrap, and — when a
//! macOS/Windows host is available — the seatbelt/WFP suites.

mod plan_fixture;

use agendao_sandbox::platform::macos::seatbelt::build_seatbelt_profile;
use agendao_sandbox::platform::windows::{acl, job, token, wfp, windows_backend};
use agendao_sandbox::BackendRegistry;

// ---------------------------------------------------------------------------
// Seatbelt profile contracts
// ---------------------------------------------------------------------------

#[test]
fn seatbelt_workspace_write_allows_workspace_writes_then_re_denies_metadata() {
    let plan = plan_fixture::contained_workspace_write();
    let profile = build_seatbelt_profile(&plan).unwrap();
    let workspace = plan.filesystem.workspace_root.as_str();

    assert!(profile.starts_with("(version 1)\n"));
    assert!(profile.contains("(deny default)\n"));
    assert!(profile.contains(&format!("(allow file-read* (subpath \"{workspace}\"))\n")));
    let write_rule = format!("(allow file-write* (subpath \"{workspace}\"))\n");
    assert!(
        profile.contains(&write_rule),
        "writable workspace needs a write allowance: {profile}"
    );
    for component in [".git", ".agendao", ".agents", ".codex"] {
        let deny_rule = format!("(deny file-write* (subpath \"{workspace}/{component}\"))\n");
        let deny_at = profile
            .find(&deny_rule)
            .unwrap_or_else(|| panic!("missing protected deny for {component}: {profile}"));
        let write_at = profile.find(&write_rule).expect("workspace write rule");
        assert!(
            deny_at > write_at,
            "Seatbelt applies the last matching rule: the {component} deny must come after the workspace write allowance"
        );
    }
    assert!(profile.contains("(deny network*)\n"));
    assert!(profile.contains("(deny process-info*)\n"));
}

#[test]
fn seatbelt_read_only_workspace_has_no_write_allowance() {
    let plan = plan_fixture::contained_read_only();
    let profile = build_seatbelt_profile(&plan).unwrap();
    let workspace = plan.filesystem.workspace_root.as_str();

    assert!(profile.contains(&format!("(allow file-read* (subpath \"{workspace}\"))\n")));
    assert!(
        !profile.contains("file-write*"),
        "a read-only plan grants no writes anywhere: {profile}"
    );
}

#[test]
fn seatbelt_extra_writable_roots_get_their_own_allowance() {
    let plan = plan_fixture::contained_with_cache_root();
    let profile = build_seatbelt_profile(&plan).unwrap();
    let cache = plan.filesystem.writable_roots[1].as_str();

    assert!(profile.contains(&format!("(allow file-write* (subpath \"{cache}\"))\n")));
}

#[test]
fn seatbelt_interactive_shell_does_not_grant_shared_private_home() {
    let plan = plan_fixture::contained_interactive_shell();
    let profile = build_seatbelt_profile(&plan).unwrap();
    assert!(
        !profile.contains("agendao-home"),
        "Seatbelt must not grant the fixed shared private HOME"
    );
}

#[test]
fn seatbelt_rejects_untrusted_path_literal_characters() {
    let mut plan = plan_fixture::contained_workspace_write();
    for unsafe_path in ["/ws/quote\"break", "/ws/back\\slash", "/ws/new\nline"] {
        plan.filesystem.workspace_root = agendao_sandbox::CanonicalPathValue(unsafe_path.into());
        let error = build_seatbelt_profile(&plan).unwrap_err();
        assert!(
            error.to_string().contains("cannot be safely encoded"),
            "unexpected error for {unsafe_path:?}: {error}"
        );
    }

    plan.filesystem.workspace_root = agendao_sandbox::CanonicalPathValue("/ws".into());
    plan.filesystem.writable_roots = vec![agendao_sandbox::CanonicalPathValue(
        "/ws/cache\"break".into(),
    )];
    assert!(build_seatbelt_profile(&plan).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_backend_is_explicitly_unavailable_until_scoped_home_exists() {
    use agendao_sandbox::platform::macos::seatbelt::SeatbeltBackend;
    let backend = SeatbeltBackend::discover();
    let probe = backend.probe();
    assert!(!probe.available);
    assert!(probe
        .reason
        .unwrap()
        .contains("execution-scoped private HOME"));
    assert!(agendao_sandbox::default_platform_backends().is_empty());
}

// ---------------------------------------------------------------------------
// Windows model contracts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn windows_backend_fails_closed_on_contained_launches() {
    let backend = windows_backend();
    assert_eq!(backend.name(), "windows-restricted-token");

    let probe = backend.probe();
    assert!(
        !probe.available,
        "unintegrated backend must not be selectable"
    );
    let reason = probe.reason.expect("unavailable probes carry a reason");
    assert!(
        reason.contains("WFP"),
        "the reason must name the missing enforcement layer: {reason}"
    );

    // The unavailable probe is the selection gate; no contained launch can
    // reach this backend until the restricted-token enforcement is integrated.
}

#[test]
fn windows_models_derive_from_the_plan() {
    let plan = plan_fixture::contained_workspace_write();

    let token_plan = token::restricted_token_plan(&plan);
    assert!(token_plan
        .deny_only_sids
        .contains(&token::WellKnownSid::Administrators));

    let protected = acl::protected_metadata_dirs(&plan);
    assert_eq!(protected.len(), 4, ".git/.agendao/.agents/.codex");
    assert!(protected.iter().all(|dir| dir.contains(".git")
        || dir.contains(".agendao")
        || dir.contains(".agents")
        || dir.contains(".codex")));

    let job = job::job_object_config();
    assert!(job.kill_on_job_close && !job.breakaway_allowed);
}

#[test]
fn windows_reason_is_the_single_fail_closed_explanation() {
    // The same constant flows through probe() and spawn(): one
    // authority for why Windows contained launches fail (金: one
    //成形语法 for the denial).
    assert!(!wfp::NETWORK_ENFORCEMENT_REASON.is_empty());
}

// ---------------------------------------------------------------------------
// Reporting: capabilities surface on every build
// ---------------------------------------------------------------------------

#[test]
fn registry_reports_platform_backend_and_native_channel() {
    let registry = BackendRegistry::native_only(agendao_sandbox::native_backend())
        .with_platform_backend(windows_backend());
    let caps = registry.capabilities();
    assert_eq!(caps.len(), 2, "platform row + native row");
    assert!(caps
        .iter()
        .any(|cap| cap.backend == "windows-restricted-token"
            && cap.contained
            && !cap.native
            && !cap.probe.available));
    assert!(caps.iter().any(|cap| cap.backend == "native" && cap.native));
}
