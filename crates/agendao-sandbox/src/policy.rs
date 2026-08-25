//! Policy merge: the only place permission/session/workspace inputs
//! become a `SandboxProfile`.
//!
//! Fixed order (see plan §5.3):
//!
//! ```text
//! platform hard deny ∩ admin hard ∩ agent hard ∩ session mode
//!   ∩ permission grant scope ∩ tool request
//! ```
//!
//! Merging only tightens. Each dimension contributes an upper bound and
//! the result is the intersection (minimum width). A `Native` request
//! that no dimension explicitly grants fails loudly — it is never
//! silently downgraded to contained, and contained is never silently
//! upgraded to native.

use std::collections::BTreeSet;
use std::path::PathBuf;

use agendao_types::SessionPermissionMode;

use crate::environment::EnvironmentPolicy;
use crate::model::{FilesystemMode, FilesystemPolicy, NetworkMode, ProcessMode, ProcessPolicy};
use crate::network::NetworkPolicy;
use crate::request::ProfileKind;

/// Width ranking for filesystem modes; merge takes the minimum.
pub fn filesystem_rank(mode: FilesystemMode) -> u8 {
    match mode {
        FilesystemMode::ReadOnly => 0,
        FilesystemMode::Restricted => 1,
        FilesystemMode::WorkspaceWrite => 2,
        FilesystemMode::Unrestricted => 3,
    }
}

/// Width ranking for network modes; merge takes the minimum.
pub fn network_rank(mode: NetworkMode) -> u8 {
    match mode {
        NetworkMode::Disabled => 0,
        NetworkMode::ProxyOnly => 1,
        NetworkMode::Enabled => 2,
    }
}

/// A hard upper bound contributed by one governance layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardPolicy {
    pub max_filesystem: FilesystemMode,
    pub max_network: NetworkMode,
    pub allow_native: bool,
}

impl HardPolicy {
    /// Contained upper bound used by the session layer (`Default` and
    /// `TrustedWorkspace`) and by contained profile kinds: workspace
    /// writable at most, network denied, native never granted.
    pub fn contained_baseline() -> Self {
        Self {
            max_filesystem: FilesystemMode::WorkspaceWrite,
            max_network: NetworkMode::Disabled,
            allow_native: false,
        }
    }

    /// No restriction (used by the unsandboxed session mode only).
    pub fn unrestricted() -> Self {
        Self {
            max_filesystem: FilesystemMode::Unrestricted,
            max_network: NetworkMode::Enabled,
            allow_native: true,
        }
    }

    /// The intersection of two bounds: each field takes the narrower;
    /// native only survives when both bounds allow it.
    pub fn intersect(&self, other: &HardPolicy) -> HardPolicy {
        let max_filesystem =
            if filesystem_rank(self.max_filesystem) <= filesystem_rank(other.max_filesystem) {
                self.max_filesystem
            } else {
                other.max_filesystem
            };
        let max_network = if network_rank(self.max_network) <= network_rank(other.max_network) {
            self.max_network
        } else {
            other.max_network
        };
        HardPolicy {
            max_filesystem,
            max_network,
            allow_native: self.allow_native && other.allow_native,
        }
    }
}

/// Scope of the completed permission grant that feeds the sandbox
/// authority (permission is decided elsewhere; this is only its shape).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermissionGrantScope {
    /// Explicitly granted writable paths (empty = no write grant).
    pub write_paths: Vec<PathBuf>,
    /// Upper bound on network width carried by the grant.
    pub max_network: Option<NetworkMode>,
}

/// All inputs to profile derivation.
#[derive(Debug, Clone)]
pub struct PolicyInputs {
    /// Platform hard ceiling. The default is *unrestricted*: the platform
    /// layer exists for deployments that must forbid widths outright;
    /// tightening is the session layer's job, so an explicit
    /// `UnsandboxedYolo` session is not silently blocked here.
    pub platform: HardPolicy,
    pub admin: Option<HardPolicy>,
    pub agent: Option<HardPolicy>,
    pub session_mode: SessionPermissionMode,
    /// File-level permission grant scope, when the execution went through
    /// one. Process-level tools (bash, PTY) pass `None` — an absent grant
    /// must not read as "read-only".
    pub grant: Option<PermissionGrantScope>,
    /// Canonical build cache root required by the `Check` profile
    /// (resolved by the authority, never chosen by tools).
    pub check_build_cache_root: Option<PathBuf>,
    pub environment_allow_exact: BTreeSet<String>,
}

impl PolicyInputs {
    pub fn baseline(session_mode: SessionPermissionMode) -> Self {
        Self {
            platform: HardPolicy::unrestricted(),
            admin: None,
            agent: None,
            session_mode,
            grant: None,
            check_build_cache_root: None,
            environment_allow_exact: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("native execution requested but no policy layer grants it")]
    NativeNotAllowed,
    #[error("check profile requires an authority-resolved build cache root")]
    CheckRequiresCacheRoot,
}

/// The bound contributed by the session mode. `UnsandboxedYolo` is the
/// only session-level path to native execution; `TrustedWorkspace`
/// deliberately does not widen anything over `Default`.
fn session_bound(mode: SessionPermissionMode) -> HardPolicy {
    match mode {
        SessionPermissionMode::Default | SessionPermissionMode::TrustedWorkspace => {
            HardPolicy::contained_baseline()
        }
        SessionPermissionMode::UnsandboxedYolo => HardPolicy::unrestricted(),
    }
}

/// The bound contributed by the permission grant: no write paths means
/// read-only; network width is capped by the grant.
fn grant_bound(grant: &PermissionGrantScope) -> HardPolicy {
    HardPolicy {
        max_filesystem: if grant.write_paths.is_empty() {
            FilesystemMode::ReadOnly
        } else {
            FilesystemMode::WorkspaceWrite
        },
        max_network: grant.max_network.unwrap_or(NetworkMode::Disabled),
        allow_native: false,
    }
}

/// Derive the final immutable profile for a request.
pub fn derive_profile(
    trust_class: crate::model::TrustClass,
    kind: ProfileKind,
    inputs: &PolicyInputs,
) -> Result<crate::model::SandboxProfile, PolicyError> {
    let mut bound = inputs.platform.clone();
    if let Some(admin) = &inputs.admin {
        bound = bound.intersect(admin);
    }
    if let Some(agent) = &inputs.agent {
        bound = bound.intersect(agent);
    }
    bound = bound.intersect(&session_bound(inputs.session_mode));
    if let Some(grant) = &inputs.grant {
        bound = bound.intersect(&grant_bound(grant));
    }

    // The tool request itself is also a bound: contained profile kinds
    // never ask for more than workspace-writable/no-network/native-off.
    let request_bound = match kind {
        ProfileKind::WorkspaceWrite | ProfileKind::InteractiveShell => {
            HardPolicy::contained_baseline()
        }
        ProfileKind::Integration => HardPolicy {
            max_filesystem: if inputs
                .grant
                .as_ref()
                .is_some_and(|grant| !grant.write_paths.is_empty())
            {
                FilesystemMode::WorkspaceWrite
            } else {
                FilesystemMode::ReadOnly
            },
            max_network: NetworkMode::Disabled,
            allow_native: false,
        },
        ProfileKind::Check => HardPolicy {
            max_filesystem: FilesystemMode::ReadOnly,
            max_network: NetworkMode::Disabled,
            allow_native: false,
        },
        ProfileKind::Native => HardPolicy::unrestricted(),
    };
    bound = bound.intersect(&request_bound);

    let wants_native = kind == ProfileKind::Native;
    if wants_native && !bound.allow_native {
        return Err(PolicyError::NativeNotAllowed);
    }

    // `Unrestricted` is only reachable when the Native request survived
    // every layer (Yolo session + no admin/agent/platform tightening).
    let filesystem = match bound.max_filesystem {
        FilesystemMode::Unrestricted => FilesystemPolicy {
            mode: FilesystemMode::Unrestricted,
            writable_roots: Vec::new(),
            read_only_roots: Vec::new(),
        },
        mode if filesystem_rank(mode) >= filesystem_rank(FilesystemMode::WorkspaceWrite) => {
            FilesystemPolicy {
                mode: FilesystemMode::WorkspaceWrite,
                writable_roots: Vec::new(),
                read_only_roots: Vec::new(),
            }
        }
        _ => FilesystemPolicy {
            mode: FilesystemMode::ReadOnly,
            writable_roots: Vec::new(),
            read_only_roots: Vec::new(),
        },
    };

    let mut writable_roots = Vec::new();
    if kind == ProfileKind::Check {
        let cache_root = inputs
            .check_build_cache_root
            .clone()
            .ok_or(PolicyError::CheckRequiresCacheRoot)?;
        writable_roots.push(cache_root);
    }
    if let Some(grant) = &inputs.grant {
        if filesystem.mode == FilesystemMode::WorkspaceWrite && kind != ProfileKind::Check {
            writable_roots.extend(grant.write_paths.iter().cloned());
        }
    }
    let filesystem = FilesystemPolicy {
        writable_roots,
        ..filesystem
    };

    let process = ProcessPolicy {
        mode: if wants_native && bound.allow_native {
            ProcessMode::Native
        } else {
            ProcessMode::Contained
        },
    };

    Ok(crate::model::SandboxProfile {
        trust_class,
        filesystem,
        network: NetworkPolicy {
            mode: bound.max_network,
            proxy_endpoint: None,
            allowed_unix_sockets: Vec::new(),
            allowed_hosts: Default::default(),
        },
        environment: {
            let mut environment = if process.mode == ProcessMode::Native {
                EnvironmentPolicy::native_inherit()
            } else {
                EnvironmentPolicy::default()
            };
            environment.allow_exact = inputs.environment_allow_exact.clone();
            environment
        },
        process,
    })
}
