//! Environment contract: env-clear-then-reinject, hard-deny keys that
//! nothing can restore, pattern screening with exact allowlist escape,
//! and authority-reserved keys (plan §5.5, §8.3-7).

use std::collections::{BTreeMap, BTreeSet};

use agendao_sandbox::{
    build_child_environment, is_denied, EnvNamePattern, EnvironmentError, EnvironmentPolicy,
    AGENDAO_SANDBOX_ENV_PREFIX,
};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn default_policy_denies_secret_heuristics() {
    let policy = EnvironmentPolicy::default();
    for name in [
        "DEEPSEEK_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "DB_PASSWORD",
        "SERVICE_SECRET",
        "GOOGLE_CREDENTIALS_JSON",
        "AGENDAO_SERVER_PASSWORD",
    ] {
        assert!(is_denied(&policy, name), "{name} must be denied");
    }
}

#[test]
fn heuristics_do_not_sweep_innocent_lookalikes() {
    let policy = EnvironmentPolicy::default();
    for name in [
        "MONKEY_PATCH",
        "KEYBOARD_LAYOUT",
        "PATH",
        "HOME",
        "TOKENS_COUNT",
    ] {
        // TOKENS_COUNT is not a *_TOKEN suffix; not denied by default.
        let _ = name;
    }
    assert!(!is_denied(&policy, "MONKEY_PATCH"));
    assert!(!is_denied(&policy, "KEYBOARD_LAYOUT"));
    assert!(!is_denied(&policy, "TOKENS_COUNT"));
}

#[test]
fn admin_allowlist_exempts_exact_names_only() {
    let policy = EnvironmentPolicy {
        allow_exact: ["MONKEY_PATCH".to_string()].into_iter().collect(),
        ..EnvironmentPolicy::default()
    };
    assert!(!is_denied(&policy, "MONKEY_PATCH"));
    assert!(is_denied(&policy, "MONKEY_PATCH_API_KEY"));
}

#[test]
fn policy_inputs_project_admin_allowlist_but_hard_deny_still_wins() {
    let mut inputs =
        agendao_sandbox::PolicyInputs::baseline(agendao_types::SessionPermissionMode::Default);
    inputs.environment_allow_exact = BTreeSet::from([
        "MONKEY_TOKEN".to_string(),
        "AGENDAO_INTERNAL_TOKEN".to_string(),
    ]);
    let profile = agendao_sandbox::derive_profile(
        agendao_sandbox::TrustClass::ModelReachable,
        agendao_sandbox::ProfileKind::WorkspaceWrite,
        &inputs,
    )
    .unwrap();
    assert!(
        agendao_sandbox::environment::check_override(&profile.environment, "MONKEY_TOKEN").is_ok()
    );
    assert!(matches!(
        agendao_sandbox::environment::check_override(
            &profile.environment,
            "AGENDAO_INTERNAL_TOKEN"
        ),
        Err(agendao_sandbox::EnvironmentError::HardDeniedKey { .. })
    ));
}

#[test]
fn contained_child_gets_core_only_and_overrides() {
    let policy = EnvironmentPolicy::default();
    let host = env(&[
        ("PATH", "/usr/bin"),
        ("HOME", "/home/dev"),
        ("LANG", "C.UTF-8"),
        ("EDITOR", "vim"),
        ("DEEPSEEK_API_KEY", "sk-leak"),
    ]);
    let overrides = env(&[("AGENDAO_TOOL_HINT", "1")]);
    let authority = env(&[("AGENDAO_SANDBOX_PLAN", "workspace_write")]);
    let child = build_child_environment(&policy, &host, &overrides, &authority).unwrap();

    assert_eq!(child.get("PATH").map(String::as_str), Some("/usr/bin"));
    assert_eq!(child.get("HOME").map(String::as_str), Some("/home/dev"));
    assert!(!child.contains_key("EDITOR"), "non-core host vars dropped");
    assert!(!child.contains_key("DEEPSEEK_API_KEY"), "secrets dropped");
    assert_eq!(
        child.get("AGENDAO_TOOL_HINT").map(String::as_str),
        Some("1")
    );
    assert!(child.contains_key(&format!("{AGENDAO_SANDBOX_ENV_PREFIX}PLAN")));
}

#[test]
fn overrides_cannot_restore_hard_deny_keys() {
    let policy = EnvironmentPolicy::default();
    let overrides = env(&[("AGENDAO_SERVER_PASSWORD", "guess")]);
    let err = build_child_environment(&policy, &BTreeMap::new(), &overrides, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(
        err,
        EnvironmentError::HardDeniedKey {
            key: "AGENDAO_SERVER_PASSWORD".to_string()
        }
    );
}

#[test]
fn overrides_matching_patterns_are_rejected() {
    let policy = EnvironmentPolicy::default();
    let overrides = env(&[("MY_SERVICE_TOKEN", "x")]);
    let err = build_child_environment(&policy, &BTreeMap::new(), &overrides, &BTreeMap::new())
        .unwrap_err();
    assert!(matches!(err, EnvironmentError::DeniedByPattern { .. }));

    // Allowlisted names pass even when they match a pattern.
    let policy = EnvironmentPolicy {
        allow_exact: ["MY_SERVICE_TOKEN".to_string()].into_iter().collect(),
        ..policy
    };
    assert!(
        build_child_environment(&policy, &BTreeMap::new(), &overrides, &BTreeMap::new()).is_ok()
    );
}

#[test]
fn authority_prefix_is_reserved() {
    let policy = EnvironmentPolicy::default();
    let overrides = env(&[("AGENDAO_SANDBOX_FAKE", "spoof")]);
    let err = build_child_environment(&policy, &BTreeMap::new(), &overrides, &BTreeMap::new())
        .unwrap_err();
    assert!(matches!(err, EnvironmentError::AuthorityReserved { .. }));

    // Authority injections must themselves use the prefix.
    let bad_authority = env(&[("AGENDAO_NOT_SANDBOX", "x")]);
    assert!(matches!(
        build_child_environment(&policy, &BTreeMap::new(), &BTreeMap::new(), &bad_authority)
            .unwrap_err(),
        EnvironmentError::AuthorityReserved { .. }
    ));
}

#[test]
fn native_inheritance_is_still_filtered() {
    let policy = EnvironmentPolicy::native_inherit();
    let host = env(&[
        ("PATH", "/usr/bin"),
        ("EDITOR", "vim"),
        ("AGENDAO_SERVER_PASSWORD", "server-secret"),
        ("GITHUB_TOKEN", "gh-secret"),
    ]);
    let child =
        build_child_environment(&policy, &host, &BTreeMap::new(), &BTreeMap::new()).unwrap();
    assert_eq!(child.get("EDITOR").map(String::as_str), Some("vim"));
    assert!(!child.contains_key("AGENDAO_SERVER_PASSWORD"));
    assert!(!child.contains_key("GITHUB_TOKEN"));
}

#[test]
fn pattern_kinds_match_as_documented() {
    assert!(EnvNamePattern::Exact {
        name: "AWS_SESSION_TOKEN".into()
    }
    .matches("AWS_SESSION_TOKEN"));
    assert!(!EnvNamePattern::Exact {
        name: "AWS_SESSION_TOKEN".into()
    }
    .matches("OTHER"));
    assert!(EnvNamePattern::Suffix {
        suffix: "_KEY".into()
    }
    .matches("SSH_KEY"));
    assert!(EnvNamePattern::Contains {
        fragment: "CREDENTIAL".into()
    }
    .matches("GCLOUD_CREDENTIALS"));
}
