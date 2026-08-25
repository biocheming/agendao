//! Network policy intent. The platform backend enforces isolation; this
//! module only defines and validates the intent, including the rule that
//! unfiltered egress exists only on explicit native/trusted profiles.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::model::NetworkMode;
use crate::model::ProcessMode;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NetworkPolicy {
    pub mode: NetworkMode,
    /// Authority-provided proxy endpoint; required for `ProxyOnly`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_endpoint: Option<String>,
    /// Unix sockets the child may connect to (authority-resolved paths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_unix_sockets: Vec<PathBuf>,
    /// Host allowlist for `Enabled`; empty means unfiltered.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_hosts: BTreeSet<String>,
}

impl NetworkPolicy {
    pub fn disabled() -> Self {
        Self {
            mode: NetworkMode::Disabled,
            proxy_endpoint: None,
            allowed_unix_sockets: Vec::new(),
            allowed_hosts: BTreeSet::new(),
        }
    }

    pub fn proxy_only(endpoint: impl Into<String>) -> Self {
        Self {
            mode: NetworkMode::ProxyOnly,
            proxy_endpoint: Some(endpoint.into()),
            allowed_unix_sockets: Vec::new(),
            allowed_hosts: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetworkPolicyError {
    #[error("proxy-only network policy requires an authority-provided proxy endpoint")]
    ProxyOnlyWithoutEndpoint,
    #[error("enabled network egress requires an explicit native/trusted process profile")]
    EnabledRequiresNative,
    #[error("disabled network policy cannot carry allowlists")]
    DisabledWithAllowlist,
}

/// Cross-domain validation: network intent must be consistent with the
/// process containment intent.
pub fn validate(policy: &NetworkPolicy, process: ProcessMode) -> Result<(), NetworkPolicyError> {
    match policy.mode {
        NetworkMode::Disabled => {
            if !policy.allowed_unix_sockets.is_empty() || !policy.allowed_hosts.is_empty() {
                return Err(NetworkPolicyError::DisabledWithAllowlist);
            }
        }
        NetworkMode::ProxyOnly => {
            if policy.proxy_endpoint.is_none() {
                return Err(NetworkPolicyError::ProxyOnlyWithoutEndpoint);
            }
        }
        NetworkMode::Enabled => {
            if process != ProcessMode::Native {
                return Err(NetworkPolicyError::EnabledRequiresNative);
            }
        }
    }
    Ok(())
}
