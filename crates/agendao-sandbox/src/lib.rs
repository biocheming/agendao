//! AgenDao sandbox domain crate.
//!
//! Single implementation authority for OS-level execution isolation:
//! policy merging, path resolution, environment/network/process intent,
//! immutable sandbox plans, and platform backends (Bubblewrap on Linux,
//! Seatbelt on macOS, restricted token + Job Object on Windows).
//!
//! Governance invariants (see `docs/plans/sandbox-system-architecture-and-implementation-plan.md`):
//!
//! * This crate never depends on `agendao-server`, never holds sessions,
//!   permission engines, databases, or frontend senders.
//! * `SandboxPlan` instances are only produced by the server-side
//!   `SandboxAuthority`; tool input can never assert "already sandboxed".
//! * Backend unavailability fails closed for contained profiles; there is
//!   no silent fallback to native execution.
//!
//! Public type names are prefixed with `Sandbox` to keep the `execution`
//! namespace distinct from `agendao-execution-types` (scheduler/provider
//! execution state) and `agendao_tool::execution_preflight` (readiness
//! reports).

pub mod backend;
pub mod boundary;
pub mod driver;
pub mod environment;
pub mod launcher;
pub mod lifecycle;
pub mod model;
pub mod native;
pub mod network;
pub mod path;
pub mod plan;
pub mod platform;
pub mod policy;
pub mod request;
pub mod violation;

pub use backend::{
    BackendCapability, BackendChild, BackendExit, BackendProbe, BackendRegistry,
    BackendViolationReport, BackendViolationToken, ChildEnvironment, SandboxBackend, StdioPlan,
    StdioSpec,
};
pub use boundary::{
    plugin_runtime_roots, IntegrationSandboxContext, SandboxExecutionBoundary,
    SharedSandboxExecutionBoundary,
};
pub use driver::{ExitStatus, SandboxHandleDriver};
pub use environment::{
    build_child_environment, default_deny_patterns, default_hard_deny_exact, is_denied,
    EnvNamePattern, EnvironmentError, EnvironmentPolicy, AGENDAO_SANDBOX_ENV_PREFIX,
    CORE_ENV_NAMES,
};
pub use launcher::{
    AuthorityReadOnlyRoots, DenialReason, EventLog, ExecutionIdMinter, PrepareOptions,
    PreparedSandboxExecution, ProfileSummary, SandboxEvent, SandboxEventSink, SandboxLauncher,
    UuidIdMinter,
};
pub use lifecycle::{CleanupStatus, SandboxExecutionHandle, SandboxExit};
pub use model::{
    FilesystemMode, FilesystemPolicy, NetworkMode, ProcessMode, ProcessPolicy, SandboxProfile,
    TrustClass,
};
pub use native::{native_backend, NativeBackend};
pub use network::{validate as validate_network, NetworkPolicy, NetworkPolicyError};
pub use path::{
    assert_no_symlink_escape, assert_within_root, canonicalize_existing, protected_metadata,
    resolve_create_target, resolve_user_path, workspace_scope, CanonicalPath, CreateTarget,
    PathViolation, ProtectedPath, RelativePath, ScopeKey, UserPath, PROTECTED_METADATA_COMPONENTS,
};
pub use plan::{
    build_plan, CanonicalPathValue, FilesystemPlan, PlanContext, PlanError, ProcessPlan,
    SandboxPlan, DEFAULT_TERM_GRACE,
};
#[cfg(not(target_os = "linux"))]
pub use platform::default_platform_backends;
#[cfg(unix)]
pub use platform::process_tree;
#[cfg(unix)]
pub use platform::pty::{openpty, PtyDimensions, PtyMaster, PtySlave};
#[cfg(not(unix))]
pub use platform::pty::{PtyDimensions, PtyMaster, PtySlave};
#[cfg(target_os = "linux")]
pub use platform::{
    default_platform_backends,
    linux::{build_bwrap_args, bwrap_backend, BwrapBackend},
};
pub use policy::{
    derive_profile, filesystem_rank, network_rank, HardPolicy, PermissionGrantScope, PolicyError,
    PolicyInputs,
};
pub use request::{ProfileKind, SandboxExecutionRequest, SpawnSpec, INTERACTIVE_PRIVATE_HOME};
pub use violation::{Attribution, SandboxExecutionError, SandboxViolation, SandboxViolationKind};
