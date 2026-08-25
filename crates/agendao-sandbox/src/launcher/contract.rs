use super::*;

/// Profile summary carried by `SandboxPrepared` — enough to audit what
/// ran without shipping the full policy object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProfileSummary {
    pub requested_kind: ProfileKind,
    pub process_mode: ProcessMode,
    pub filesystem_mode: FilesystemMode,
    pub network_mode: NetworkMode,
}

/// Why an execution never started. `BackendUnavailable` carries the
/// capability reason so projections can tell the user exactly what is
/// missing on this host.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DenialReason {
    InvalidRequest,
    PolicyDenied,
    PlanFailed,
    EnvironmentRejected,
    BackendUnavailable { capability: String },
    SpawnFailed,
}

/// Typed runtime events (plan §7). Field-for-field compatible with the
/// `agendao-server-core` projections landed in Phase 5; payloads never
/// contain secrets or full environments.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum SandboxEvent {
    Prepared {
        execution_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_origin: Option<String>,
        profile: ProfileSummary,
        plan_fingerprint: String,
        backend: String,
    },
    Started {
        execution_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_origin: Option<String>,
        pid: Option<u32>,
        backend: String,
    },
    Denied {
        execution_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_origin: Option<String>,
        reason: DenialReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Violation {
        #[serde(flatten)]
        violation: SandboxViolation,
    },
    Exited {
        execution_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_origin: Option<String>,
        status: crate::lifecycle::SandboxExit,
        backend: String,
    },
}

impl SandboxEvent {
    pub fn execution_id(&self) -> &str {
        match self {
            SandboxEvent::Prepared { execution_id, .. }
            | SandboxEvent::Started { execution_id, .. }
            | SandboxEvent::Denied { execution_id, .. }
            | SandboxEvent::Exited { execution_id, .. } => execution_id,
            SandboxEvent::Violation { violation } => &violation.execution_id,
        }
    }
}

/// Where the launcher publishes lifecycle events. Implementations
/// forward into the runtime event path (server-side authority) or a
/// test log.
pub trait SandboxEventSink: Send + Sync {
    fn record(&self, event: SandboxEvent);
}

/// Test/projection-friendly sink keeping the full event log.
#[derive(Default)]
pub struct EventLog {
    events: std::sync::Mutex<Vec<SandboxEvent>>,
}

impl SandboxEventSink for EventLog {
    fn record(&self, event: SandboxEvent) {
        self.events.lock().expect("event log poisoned").push(event);
    }
}

impl EventLog {
    pub fn snapshot(&self) -> Vec<SandboxEvent> {
        self.events.lock().expect("event log poisoned").clone()
    }
}

/// Mints execution ids. The authority owns identity; tools never supply
/// one, so forged/reused audit ids are impossible by construction.
pub trait ExecutionIdMinter: Send + Sync {
    fn mint(&self) -> String;
}

/// Production minter: random UUID v4.
#[derive(Default)]
pub struct UuidIdMinter;

impl ExecutionIdMinter for UuidIdMinter {
    fn mint(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// A non-forgeable authority token for integration runtime mounts.
///
/// Its path list is private to the sandbox crate. Hosts obtain one only
/// through `IntegrationSandboxContext::new`, after canonical resolution.
#[derive(Debug, Clone, Default)]
pub struct AuthorityReadOnlyRoots(pub(crate) Vec<CanonicalPath>);

impl AuthorityReadOnlyRoots {
    pub(crate) fn from_canonical(mut roots: Vec<CanonicalPath>) -> Self {
        roots.sort();
        roots.dedup();
        Self(roots)
    }
}

/// Authority-side extras beyond the derived profile: pre-canonicalized
/// extra writable roots (interactive shell private HOME, check cache
/// root is policy-side), a lifecycle override, and stdio shaping.
#[derive(Debug, Clone, Default)]
pub struct PrepareOptions {
    pub extra_writable_roots: Vec<CanonicalPath>,
    /// Only an authority-created token can add integration runtime mounts.
    pub authority_read_only_roots: AuthorityReadOnlyRoots,
    pub term_grace: Option<Duration>,
    /// Stdio shaping for the launch (pipes vs inherit). Not policy:
    /// it never enters the plan fingerprint.
    pub stdio: crate::backend::StdioPlan,
}
