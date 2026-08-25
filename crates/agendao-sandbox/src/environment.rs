//! Environment policy: env-clear-then-reinject contract.
//!
//! Every sandboxed child starts from an empty environment. The policy
//! then reinjects a minimal core set, applies an exact-name hard-deny
//! list that nothing can override, and screens the remaining names with
//! secret heuristics (`*_API_KEY`, `*_TOKEN`, ...) that an admin
//! allowlist can exempt for confirmed non-sensitive names. Authority
//! keys (`AGENDAO_SANDBOX_*`) can only be injected by the sandbox
//! authority itself.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Prefix reserved for values only the sandbox authority may inject.
pub const AGENDAO_SANDBOX_ENV_PREFIX: &str = "AGENDAO_SANDBOX_";

/// Minimal environment variables reinjected after `env_clear()`.
pub const CORE_ENV_NAMES: [&str; 7] = ["PATH", "HOME", "LANG", "LC_ALL", "TZ", "TERM", "TMPDIR"];

/// Secret-screening pattern over environment variable names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EnvNamePattern {
    Exact { name: String },
    Suffix { suffix: String },
    Contains { fragment: String },
}

impl EnvNamePattern {
    pub fn matches(&self, candidate: &str) -> bool {
        match self {
            EnvNamePattern::Exact { name } => candidate == name,
            EnvNamePattern::Suffix { suffix } => candidate.ends_with(suffix.as_str()),
            EnvNamePattern::Contains { fragment } => candidate.contains(fragment.as_str()),
        }
    }
}

/// Environment policy for one sandbox plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentPolicy {
    /// When true (contained profiles): clear the child environment and
    /// reinject from policy. When false (explicit native): inherit the
    /// host environment, still filtered by the deny rules.
    pub clear_and_reinject: bool,
    pub inherit_core: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hard_deny_exact: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_patterns: Vec<EnvNamePattern>,
    /// Admin-confirmed names exempt from pattern screening.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allow_exact: BTreeSet<String>,
}

impl Default for EnvironmentPolicy {
    fn default() -> Self {
        Self {
            clear_and_reinject: true,
            inherit_core: true,
            hard_deny_exact: default_hard_deny_exact(),
            deny_patterns: default_deny_patterns(),
            allow_exact: BTreeSet::new(),
        }
    }
}

impl EnvironmentPolicy {
    /// Native-profile policy: inherit the host environment, still
    /// filtered by the deny rules (AgenDao-internal credentials never
    /// leak, even to explicitly native children).
    pub fn native_inherit() -> Self {
        Self {
            clear_and_reinject: false,
            ..Self::default()
        }
    }
}

/// AgenDao-internal credentials that must never reach a child process.
pub fn default_hard_deny_exact() -> BTreeSet<String> {
    BTreeSet::from([
        "AGENDAO_SERVER_PASSWORD".to_string(),
        "AGENDAO_INTERNAL_TOKEN".to_string(),
        "AGENDAO_SESSION_TOKEN".to_string(),
    ])
}

/// Default secret heuristics. Exact-name allowlists can exempt
/// confirmed false positives (`MONKEY_PATCH`, `KEYBOARD_LAYOUT`, ...).
pub fn default_deny_patterns() -> Vec<EnvNamePattern> {
    vec![
        EnvNamePattern::Suffix {
            suffix: "_API_KEY".to_string(),
        },
        EnvNamePattern::Suffix {
            suffix: "_SECRET".to_string(),
        },
        EnvNamePattern::Suffix {
            suffix: "_TOKEN".to_string(),
        },
        EnvNamePattern::Suffix {
            suffix: "_PASSWORD".to_string(),
        },
        EnvNamePattern::Contains {
            fragment: "CREDENTIAL".to_string(),
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvironmentError {
    #[error("environment key {key} is hard-denied and can never be injected")]
    HardDeniedKey { key: String },
    #[error("environment key {key} matches a deny pattern and is not allow-listed")]
    DeniedByPattern { key: String },
    #[error("environment key {key} is reserved for sandbox authority injection")]
    AuthorityReserved { key: String },
}

/// True when `name` is denied: exact hard-deny always wins; pattern
/// matches are denied unless the admin allowlist exempts the exact name.
pub fn is_denied(policy: &EnvironmentPolicy, name: &str) -> bool {
    if policy.hard_deny_exact.contains(name) {
        return true;
    }
    if policy.allow_exact.contains(name) {
        return false;
    }
    policy.deny_patterns.iter().any(|p| p.matches(name))
}

/// Validate one proposed override against the policy.
pub fn check_override(policy: &EnvironmentPolicy, name: &str) -> Result<(), EnvironmentError> {
    if name.starts_with(AGENDAO_SANDBOX_ENV_PREFIX) {
        return Err(EnvironmentError::AuthorityReserved {
            key: name.to_string(),
        });
    }
    if policy.hard_deny_exact.contains(name) {
        return Err(EnvironmentError::HardDeniedKey {
            key: name.to_string(),
        });
    }
    if is_denied(policy, name) {
        return Err(EnvironmentError::DeniedByPattern {
            key: name.to_string(),
        });
    }
    Ok(())
}

/// Build the final child environment.
///
/// * contained (`clear_and_reinject`): start empty, reinject the core
///   names that exist in the host, apply overrides, then apply authority
///   injections (authority keys always win).
/// * native: inherit the host environment filtered by the deny rules,
///   then apply overrides and authority injections the same way.
///
/// Overrides are validated strictly — attempting to restore a denied or
/// authority-reserved key is an error, never a silent drop.
pub fn build_child_environment(
    policy: &EnvironmentPolicy,
    host: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
    authority_injected: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, EnvironmentError> {
    for name in overrides.keys() {
        check_override(policy, name)?;
    }
    for name in authority_injected.keys() {
        // Authority keys must use the reserved prefix; anything else is a
        // programming error in the authority itself.
        if !name.starts_with(AGENDAO_SANDBOX_ENV_PREFIX) {
            return Err(EnvironmentError::AuthorityReserved {
                key: name.to_string(),
            });
        }
    }

    let mut child = BTreeMap::new();
    if policy.clear_and_reinject {
        if policy.inherit_core {
            for name in CORE_ENV_NAMES {
                if let Some(value) = host.get(name) {
                    if !is_denied(policy, name) {
                        child.insert(name.to_string(), value.clone());
                    }
                }
            }
        }
    } else {
        for (name, value) in host {
            if !is_denied(policy, name) {
                child.insert(name.clone(), value.clone());
            }
        }
    }
    for (name, value) in overrides {
        child.insert(name.clone(), value.clone());
    }
    for (name, value) in authority_injected {
        child.insert(name.clone(), value.clone());
    }
    Ok(child)
}
