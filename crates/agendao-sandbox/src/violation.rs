//! Sandbox violations and the typed execution error taxonomy.
//!
//! A *violation* is something the sandbox observed at runtime (a blocked
//! write, a denied syscall, an egress attempt). The *error* type is the
//! single failure currency of the execution boundary: every refusal —
//! policy denial, backend unavailability, spawn failure — is one of these
//! variants and flows into the same runtime event path (plan §5.4, §7).
//!
//! Governance: backend unavailability is `SandboxUnavailable` and fails
//! closed for contained profiles; it is never a silent fallback to native
//! execution.

use std::io;

use crate::environment::EnvironmentError;
use crate::plan::PlanError;
use crate::policy::PolicyError;

/// What kind of containment boundary was hit at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxViolationKind {
    /// A path outside the allowed roots was touched.
    PathEscape,
    /// Protected metadata (`.git`, `.agendao`, `.agents`, `.codex`) was
    /// targeted for writing.
    ProtectedMetadata,
    /// A network egress attempt under a denying policy.
    NetworkEgress,
    /// A denied environment key leaked into the child.
    EnvironmentLeak,
    /// A syscall was rejected by the syscall filter.
    SyscallDenied,
    /// A child escaped or outlived its process containment.
    ProcessEscape,
}

impl SandboxViolationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxViolationKind::PathEscape => "path_escape",
            SandboxViolationKind::ProtectedMetadata => "protected_metadata",
            SandboxViolationKind::NetworkEgress => "network_egress",
            SandboxViolationKind::EnvironmentLeak => "environment_leak",
            SandboxViolationKind::SyscallDenied => "syscall_denied",
            SandboxViolationKind::ProcessEscape => "process_escape",
        }
    }
}

/// How precisely the violation is attributed (plan §7): seccomp and exit
/// codes often prove only "something was denied", not which URL or path.
/// Consumers must render according to this, never upgrade best-effort
/// inference into precise kernel evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    /// Kernel/backend reported the exact target.
    Exact,
    /// The backend reported the violation with its own classification.
    BackendReported,
    /// Inferred from signals/exit codes; treat as diagnostic, not proof.
    BestEffort,
}

/// One observed containment event.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SandboxViolation {
    pub execution_id: String,
    /// Immutable plan identity for correlation. This is minted by the
    /// launcher/lifecycle path, never accepted from backend evidence.
    pub plan_fingerprint: String,
    /// The session that requested the execution, when known — so the
    /// violation routes to the same stream as its lifecycle events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_origin: Option<String>,
    pub kind: SandboxViolationKind,
    /// Path or endpoint the violation touched, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_or_endpoint: Option<String>,
    pub attribution: Attribution,
    /// Backend that observed the violation (`native` for the unsandboxed
    /// channel, which by construction reports nothing).
    pub backend: String,
}

impl std::fmt::Display for SandboxViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sandbox violation [{}] in execution {}: {}",
            self.backend,
            self.execution_id,
            self.kind.as_str()
        )?;
        if let Some(target) = &self.path_or_endpoint {
            write!(f, " target {target}")?;
        }
        match self.attribution {
            Attribution::Exact => write!(f, " (exact)"),
            Attribution::BackendReported => write!(f, " (backend-reported)"),
            Attribution::BestEffort => write!(f, " (best-effort inference)"),
        }
    }
}

/// The single failure currency of the sandbox execution boundary. Not
/// `Clone`/`PartialEq` because spawn failures carry live `io::Error`s.
#[derive(Debug, thiserror::Error)]
pub enum SandboxExecutionError {
    #[error("invalid execution request: {0}")]
    InvalidRequest(String),
    #[error("policy denied the request: {0}")]
    Policy(#[from] PolicyError),
    #[error("plan construction failed: {0}")]
    Plan(#[from] PlanError),
    #[error("environment rejected: {0}")]
    Environment(#[from] EnvironmentError),
    /// Fail-closed backend unavailability. For contained profiles this is
    /// terminal: there is no native fallback.
    #[error("sandbox backend `{backend}` unavailable: {reason}")]
    SandboxUnavailable { backend: String, reason: String },
    #[error("spawn via backend `{backend}` failed: {reason}")]
    SpawnFailed { backend: String, reason: io::Error },
    #[error("execution already finished")]
    AlreadyFinished,
    #[error("sandbox lifecycle error: {0}")]
    Lifecycle(String),
}

impl SandboxExecutionError {
    /// Backend named in the error, when the failure is backend-scoped.
    pub fn backend(&self) -> Option<&str> {
        match self {
            SandboxExecutionError::SandboxUnavailable { backend, .. }
            | SandboxExecutionError::SpawnFailed { backend, .. } => Some(backend.as_str()),
            _ => None,
        }
    }

    /// A short machine-readable denial reason for the `SandboxDenied`
    /// event payload.
    pub fn denial_reason(&self) -> &'static str {
        match self {
            SandboxExecutionError::InvalidRequest(_) => "invalid_request",
            SandboxExecutionError::Policy(_) => "policy_denied",
            SandboxExecutionError::Plan(_) => "plan_failed",
            SandboxExecutionError::Environment(_) => "environment_rejected",
            SandboxExecutionError::SandboxUnavailable { .. } => "backend_unavailable",
            SandboxExecutionError::SpawnFailed { .. } => "spawn_failed",
            SandboxExecutionError::AlreadyFinished => "already_finished",
            SandboxExecutionError::Lifecycle(_) => "lifecycle_error",
        }
    }
}
