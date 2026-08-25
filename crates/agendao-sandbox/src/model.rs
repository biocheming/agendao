//! Immutable sandbox domain model.
//!
//! Phase 0 contract freeze: the variants and fields below are the minimal
//! frozen surface. Phase 1 extends the policy structs with detail fields
//! (writable roots, deny patterns, socket allowlists, cleanup deadlines)
//! without renaming these types.

use std::path::PathBuf;

use crate::environment::EnvironmentPolicy;
use crate::network::NetworkPolicy;

/// Who is able to trigger a subprocess launch. Drives which sandbox
/// profiles are reachable; `ModelReachable` can never resolve to
/// `FilesystemMode::Unrestricted` or `ProcessMode::Native` by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    /// The model can trigger this spawn directly (bash, PTY, scheduler
    /// criterion, tool catalog runners).
    ModelReachable,
    /// User-configured integrations (plugin hosts, MCP servers, LSP
    /// servers). The binary is chosen by user configuration, but the
    /// model can reach it through tool calls.
    UserConfiguredIntegration,
    /// Product/instance management (installers, launchers, host git
    /// helpers). Never reachable from model tool input.
    HostManagement,
}

/// Filesystem isolation intent. `Unrestricted` can only be produced by the
/// server authority from an explicit session mode or escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemMode {
    /// Whole visible root is read-only.
    ReadOnly,
    /// Workspace tree writable; everything else read-only.
    WorkspaceWrite,
    /// Explicit read/write carve-outs only.
    Restricted,
    /// No filesystem isolation (explicit native/escalation only).
    Unrestricted,
}

/// Network isolation intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// No network access at all.
    Disabled,
    /// Only the authority-provided proxy endpoint is reachable.
    ProxyOnly,
    /// Unfiltered egress (explicit native/trusted profile only).
    Enabled,
}

/// Process-tree containment intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessMode {
    /// Kernel-enforced containment (namespaces / Job Object / profile).
    Contained,
    /// Plain host process (explicit native/escalation only).
    Native,
}

/// Filesystem policy details. `writable_roots` is authoritative-only:
/// entries are canonical absolute paths resolved by the sandbox path
/// authority (workspace root, `Check` profile `build_cache_root`,
/// private interactive HOME).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilesystemPolicy {
    pub mode: FilesystemMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_roots: Vec<PathBuf>,
    /// Authority-resolved host paths exposed read-only to contained runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_only_roots: Vec<PathBuf>,
}

/// Process policy details.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessPolicy {
    pub mode: ProcessMode,
}

/// A named, immutable execution profile. Instances are built by the
/// server `SandboxAuthority`; tool code only selects a profile kind and
/// can never widen one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SandboxProfile {
    pub trust_class: TrustClass,
    pub filesystem: FilesystemPolicy,
    pub network: NetworkPolicy,
    pub environment: EnvironmentPolicy,
    pub process: ProcessPolicy,
}

impl SandboxProfile {
    /// Built-in profile kinds known to the authority. Construction of
    /// `Unrestricted`/`Native` variants from model input is rejected.
    pub fn contained_workspace_write() -> Self {
        Self {
            trust_class: TrustClass::ModelReachable,
            filesystem: FilesystemPolicy {
                mode: FilesystemMode::WorkspaceWrite,
                writable_roots: Vec::new(),
                read_only_roots: Vec::new(),
            },
            network: NetworkPolicy::disabled(),
            environment: EnvironmentPolicy::default(),
            process: ProcessPolicy {
                mode: ProcessMode::Contained,
            },
        }
    }
}
