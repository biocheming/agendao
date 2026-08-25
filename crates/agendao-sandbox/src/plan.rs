//! Immutable sandbox plan and its stable fingerprint.
//!
//! `build_plan` turns a derived profile into a fully resolved,
//! platform-independent execution plan: canonical roots, protected
//! metadata carve-outs, and a fingerprint over the canonical serialized
//! form. The fingerprint is the audit identity of "what policy actually
//! ran" — backend arguments must be reproducible from it.

use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::environment::EnvironmentPolicy;
use crate::model::{FilesystemMode, ProcessMode, SandboxProfile, TrustClass};
use crate::network::NetworkPolicy;
use crate::path::{canonicalize_existing, CanonicalPath};
use crate::request::ProfileKind;

/// Default grace period between SIGTERM and SIGKILL during cleanup.
pub const DEFAULT_TERM_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilesystemPlan {
    pub mode: FilesystemMode,
    pub workspace_root: CanonicalPathValue,
    /// Canonical writable roots (workspace, build cache, private home).
    pub writable_roots: Vec<CanonicalPathValue>,
    /// Canonical authority-resolved read-only runtime roots.
    pub read_only_roots: Vec<CanonicalPathValue>,
}

/// Serializable mirror of `CanonicalPath` for plans crossing process
/// boundaries (events, projections) without losing type discipline.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct CanonicalPathValue(pub String);

impl CanonicalPathValue {
    pub fn from_canonical(path: &CanonicalPath) -> Self {
        Self(path.as_path().to_string_lossy().into_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessPlan {
    pub mode: ProcessMode,
    /// Seconds between SIGTERM and SIGKILL escalation.
    pub term_grace_secs: u64,
}

/// The immutable, auditable execution plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SandboxPlan {
    pub execution_id: String,
    pub trust_class: TrustClass,
    pub requested_kind: ProfileKind,
    pub filesystem: FilesystemPlan,
    pub network: NetworkPolicy,
    pub environment: EnvironmentPolicy,
    pub process: ProcessPlan,
    pub fingerprint: String,
    /// The session that requested this execution, when one did. Not part
    /// of the fingerprint (it explains origin, not behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_origin: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("workspace root {0} does not exist or cannot be canonicalized")]
    WorkspaceRootInvalid(std::path::PathBuf),
    #[error("writable root {0} does not exist or cannot be canonicalized")]
    WritableRootInvalid(std::path::PathBuf),
    #[error("network policy invalid: {0}")]
    Network(#[from] crate::network::NetworkPolicyError),
}

/// Everything the plan builder needs besides the derived profile.
#[derive(Debug, Clone)]
pub struct PlanContext {
    pub execution_id: String,
    /// The requesting session, carried onto the plan so every lifecycle
    /// event routes without guessing.
    pub session_origin: Option<String>,
    /// Extra writable roots already canonicalized by the authority
    /// (e.g. the interactive shell's private HOME).
    pub extra_writable_roots: Vec<CanonicalPath>,
    /// Read-only roots selected by the host authority for integrations.
    pub extra_read_only_roots: Vec<CanonicalPath>,
    pub term_grace: Option<Duration>,
}

impl PlanContext {
    pub fn new(execution_id: impl Into<String>) -> Self {
        Self {
            execution_id: execution_id.into(),
            session_origin: None,
            extra_writable_roots: Vec::new(),
            extra_read_only_roots: Vec::new(),
            term_grace: None,
        }
    }
}

/// Resolve an authority-selected writable root to its exact canonical
/// identity. A bind mount can only grant a root which already exists on
/// the host: silently falling back to an existing ancestor would widen
/// the grant (for example a missing `workspace/target` becomes the whole
/// workspace). Authorities that own a lazily-created root must materialize
/// it before they ask the plan builder to bind it.
fn canonicalize_root(path: &Path) -> Result<CanonicalPath, PlanError> {
    canonicalize_existing(path).map_err(|_| PlanError::WritableRootInvalid(path.to_path_buf()))
}

/// Deterministic, order-stable fingerprint payload.
#[derive(serde::Serialize)]
struct FingerprintPayload<'a> {
    execution_id: &'a str,
    trust_class: &'a TrustClass,
    requested_kind: &'a ProfileKind,
    filesystem_mode: &'a FilesystemMode,
    workspace_root: &'a str,
    writable_roots: Vec<&'a str>,
    read_only_roots: Vec<&'a str>,
    network: &'a NetworkPolicy,
    environment: &'a EnvironmentPolicy,
    process_mode: &'a ProcessMode,
    term_grace_secs: u64,
}

/// Build the immutable plan from a derived profile. All paths are
/// canonicalized here; nothing downstream may re-resolve them.
pub fn build_plan(
    profile: &SandboxProfile,
    kind: ProfileKind,
    workspace_root: &Path,
    context: &PlanContext,
) -> Result<SandboxPlan, PlanError> {
    crate::network::validate(&profile.network, profile.process.mode)?;

    let workspace_canon = canonicalize_existing(workspace_root)
        .map_err(|_| PlanError::WorkspaceRootInvalid(workspace_root.to_path_buf()))?;

    let mut writable: Vec<CanonicalPath> = Vec::new();
    if profile.filesystem.mode == FilesystemMode::WorkspaceWrite {
        writable.push(workspace_canon.clone());
    }
    for root in &profile.filesystem.writable_roots {
        writable.push(canonicalize_root(root)?);
    }
    writable.extend(context.extra_writable_roots.iter().cloned());
    let mut writable_values: Vec<CanonicalPathValue> = writable
        .iter()
        .map(CanonicalPathValue::from_canonical)
        .collect();
    writable_values.sort();
    writable_values.dedup();

    let mut read_only: Vec<CanonicalPath> = profile
        .filesystem
        .read_only_roots
        .iter()
        .map(|root| canonicalize_root(root))
        .collect::<Result<_, _>>()?;
    read_only.extend(context.extra_read_only_roots.iter().cloned());
    let mut read_only_values: Vec<CanonicalPathValue> = read_only
        .iter()
        .map(CanonicalPathValue::from_canonical)
        .collect();
    read_only_values.sort();
    read_only_values.dedup();

    let term_grace = context.term_grace.unwrap_or(DEFAULT_TERM_GRACE);
    let filesystem = FilesystemPlan {
        mode: profile.filesystem.mode,
        workspace_root: CanonicalPathValue::from_canonical(&workspace_canon),
        writable_roots: writable_values.clone(),
        read_only_roots: read_only_values.clone(),
    };

    let fingerprint_source = FingerprintPayload {
        execution_id: &context.execution_id,
        trust_class: &profile.trust_class,
        requested_kind: &kind,
        filesystem_mode: &profile.filesystem.mode,
        workspace_root: filesystem.workspace_root.as_str(),
        writable_roots: writable_values.iter().map(|v| v.as_str()).collect(),
        read_only_roots: read_only_values.iter().map(|v| v.as_str()).collect(),
        network: &profile.network,
        environment: &profile.environment,
        process_mode: &profile.process.mode,
        term_grace_secs: term_grace.as_secs(),
    };

    let fingerprint = fingerprint_of(&fingerprint_source);

    Ok(SandboxPlan {
        execution_id: context.execution_id.clone(),
        trust_class: profile.trust_class,
        requested_kind: kind,
        filesystem,
        network: profile.network.clone(),
        environment: profile.environment.clone(),
        process: ProcessPlan {
            mode: profile.process.mode,
            term_grace_secs: term_grace.as_secs(),
        },
        fingerprint,
        session_origin: context.session_origin.clone(),
    })
}

/// Canonical JSON → SHA-256. Stability comes from `serde_json`'s ordered
/// maps and the explicitly sorted writable roots above.
fn fingerprint_of(payload: &FingerprintPayload<'_>) -> String {
    let serialized = serde_json::to_vec(payload).expect("fingerprint payload serializes");
    let mut hasher = Sha256::new();
    hasher.update(&serialized);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}
