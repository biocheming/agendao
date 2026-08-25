//! Backend abstraction: capability probing, spawning, and selection.
//!
//! A backend turns an immutable `SandboxPlan` into a running child. It
//! never sees policy inputs, never mints execution ids, and never emits
//! lifecycle events — those belong to the launcher (`launcher.rs`) so a
//! backend (real or fake) cannot forge `SandboxStarted` or bypass the
//! plan (Phase 2 completion gate).
//!
//! Selection is the fail-closed point: when a contained plan has no
//! available backend, `select` returns `SandboxUnavailable` instead of
//! falling back to native execution. Only an explicit
//! `ProcessMode::Native` plan routes to the native backend.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::model::ProcessMode;
use crate::plan::SandboxPlan;
use crate::request::SpawnSpec;
use crate::violation::{Attribution, SandboxExecutionError, SandboxViolationKind};

/// Opaque per-launch capability issued by the launcher to its selected
/// backend. Backend code can attach this token to observed evidence, but it
/// cannot mint a token or choose the identity eventually projected to users.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendViolationToken(String);

impl BackendViolationToken {
    pub(crate) fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn report(
        &self,
        kind: SandboxViolationKind,
        path_or_endpoint: Option<String>,
        attribution: Attribution,
    ) -> BackendViolationReport {
        BackendViolationReport {
            token: self.clone(),
            kind,
            path_or_endpoint,
            attribution,
        }
    }
}

/// Backend evidence deliberately carries no execution id, fingerprint,
/// session, or backend name. The lifecycle layer verifies its opaque token
/// and derives all identity fields from the immutable launched plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendViolationReport {
    token: BackendViolationToken,
    pub kind: SandboxViolationKind,
    pub path_or_endpoint: Option<String>,
    pub attribution: Attribution,
}

impl BackendViolationReport {
    pub(crate) fn token(&self) -> &BackendViolationToken {
        &self.token
    }
}

/// Result of probing one backend on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendProbe {
    pub available: bool,
    /// Human/actionable reason when unavailable (missing wrapper, no user
    /// namespaces, WFP setup incomplete, …).
    pub reason: Option<String>,
}

impl BackendProbe {
    pub fn available() -> Self {
        Self {
            available: true,
            reason: None,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
        }
    }
}

/// Normalized child exit, portable across backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendExit {
    pub success: bool,
    pub code: Option<i32>,
    /// Termination signal on unix, when observable.
    pub signal: Option<i32>,
}

impl BackendExit {
    pub fn from_status(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            Self {
                success: status.success(),
                code: status.code(),
                signal: status.signal(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                success: status.success(),
                code: status.code(),
                signal: None,
            }
        }
    }
}

/// How one child stdio stream is wired. `Piped` hands the tool a read
/// (or write) handle through `BackendChild::take_*`; io shaping is not
/// policy — it never enters the plan fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StdioSpec {
    #[default]
    Inherit,
    Piped,
    Null,
}

/// Per-stream stdio intent for one launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StdioPlan {
    pub stdin: StdioSpec,
    pub stdout: StdioSpec,
    pub stderr: StdioSpec,
}

impl StdioPlan {
    /// Capture stdout and stderr while leaving stdin alone — the bash
    /// tool's streaming-output shape.
    pub fn piped_output() -> Self {
        Self {
            stdin: StdioSpec::Inherit,
            stdout: StdioSpec::Piped,
            stderr: StdioSpec::Piped,
        }
    }
}

impl From<StdioSpec> for std::process::Stdio {
    fn from(spec: StdioSpec) -> Self {
        match spec {
            StdioSpec::Inherit => Self::inherit(),
            StdioSpec::Piped => Self::piped(),
            StdioSpec::Null => Self::null(),
        }
    }
}

/// Running child as seen by the lifecycle layer. Only process primitives
/// — no events, no policy, no plan mutation.
#[async_trait]
pub trait BackendChild: Send {
    fn pid(&self) -> Option<u32>;
    async fn wait(&mut self) -> Result<BackendExit, SandboxExecutionError>;
    /// Best-effort graceful termination (SIGTERM / process group TERM).
    async fn signal_term(&mut self) -> Result<(), SandboxExecutionError>;
    /// Escalation after the grace period (SIGKILL / job terminate).
    async fn signal_kill(&mut self) -> Result<(), SandboxExecutionError>;

    /// Return one kernel/backend-observed violation, if any. Evidence is
    /// unusable without the launcher-issued token; lifecycle owns identity
    /// validation and event projection.
    fn take_violation_report(&mut self) -> Option<BackendViolationReport> {
        None
    }

    /// Piped stream handles, when the launch asked for `StdioSpec::Piped`.
    /// Each may be taken once; backends without pipes return `None`.
    fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        None
    }
    fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        None
    }
    fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        None
    }
}

/// Fully resolved child environment produced by the authority
/// (env-clear → core reinject → overrides → authority keys).
pub type ChildEnvironment = BTreeMap<String, String>;

/// A sandbox backend. Implementations live in `platform/` (Bubblewrap,
/// Seatbelt, restricted token) plus the explicit native channel.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Stable backend name for events and capability projections.
    fn name(&self) -> &'static str;

    /// Probe host capabilities. Cheap; called on every selection so a
    /// backend that becomes unavailable fails the next launch closed.
    fn probe(&self) -> BackendProbe;

    /// Whether this backend can execute the given plan's process mode.
    /// Platform backends run `Contained` plans; the native backend runs
    /// `Native` plans and must refuse contained ones (it is never a
    /// fallback).
    fn supports(&self, plan: &SandboxPlan) -> bool;

    /// Spawn the plan. The environment is fully resolved already; the
    /// backend applies it verbatim (env-clear semantics included). The
    /// stdio plan shapes pipes only — never policy.
    async fn spawn(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        stdio: &StdioPlan,
        violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError>;

    /// Spawn the plan on a pty: the slave becomes the child's stdio and
    /// controlling terminal, so the backend keeps every isolation layer
    /// the piped path has (seccomp fd hand-off included). Backends
    /// without terminal support fail closed here — a pty host can never
    /// fall back to spawning the argv itself, which would silently drop
    /// backend-enforced defense in depth.
    #[cfg(unix)]
    async fn spawn_pty(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        slave: &crate::platform::pty::PtySlave,
        violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        let _ = (plan, spec, env, slave, violation_token);
        Err(SandboxExecutionError::SandboxUnavailable {
            backend: self.name().to_string(),
            reason: "backend does not support interactive terminal launches".to_string(),
        })
    }
}

/// Ordered backend registry. Platform backends are tried in registration
/// order; the native backend is separate and only selected for explicit
/// `ProcessMode::Native` plans.
#[derive(Clone)]
pub struct BackendRegistry {
    native: Arc<dyn SandboxBackend>,
    platform: Vec<Arc<dyn SandboxBackend>>,
}

impl BackendRegistry {
    /// A registry with only the explicit native channel (no platform
    /// backends registered yet): every contained launch fails closed.
    pub fn native_only(native: Arc<dyn SandboxBackend>) -> Self {
        Self {
            native,
            platform: Vec::new(),
        }
    }

    /// Register a platform backend (contained execution). Order matters:
    /// the first available backend that supports the plan wins.
    pub fn with_platform_backend(mut self, backend: Arc<dyn SandboxBackend>) -> Self {
        self.platform.push(backend);
        self
    }

    /// Select the backend for a plan. THE fail-closed point:
    ///
    /// * `Contained` → first platform backend whose probe is available;
    ///   none available → `SandboxUnavailable` naming what was missing
    ///   (the first failing probe's capability reason, so the user can
    ///   act on it).
    /// * `Native` → the native backend (policy already guaranteed the
    ///   plan earned that mode).
    pub fn select(
        &self,
        plan: &SandboxPlan,
    ) -> Result<Arc<dyn SandboxBackend>, SandboxExecutionError> {
        match plan.process.mode {
            ProcessMode::Contained => {
                let mut first_failure: Option<String> = None;
                for backend in &self.platform {
                    if !backend.supports(plan) {
                        continue;
                    }
                    let probe = backend.probe();
                    if probe.available {
                        return Ok(backend.clone());
                    }
                    if first_failure.is_none() {
                        first_failure = Some(probe.reason.unwrap_or_else(|| {
                            format!("backend `{}` probed unavailable", backend.name())
                        }));
                    }
                }
                let names: Vec<&str> = self.platform.iter().map(|b| b.name()).collect();
                let reason = match first_failure {
                    Some(failure) => {
                        format!("{failure} (candidates: [{}])", names.join(", "))
                    }
                    None if self.platform.is_empty() => {
                        "no platform backend registered on this build".to_string()
                    }
                    None => "no platform backend supports this plan".to_string(),
                };
                Err(SandboxExecutionError::SandboxUnavailable {
                    backend: names.first().copied().unwrap_or("none").to_string(),
                    reason,
                })
            }
            ProcessMode::Native => Ok(self.native.clone()),
        }
    }

    /// Capability summary for projections: one probe per backend.
    pub fn capabilities(&self) -> Vec<BackendCapability> {
        let mut caps: Vec<BackendCapability> = self
            .platform
            .iter()
            .map(|b| BackendCapability {
                backend: b.name().to_string(),
                contained: true,
                native: false,
                probe: b.probe(),
            })
            .collect();
        caps.push(BackendCapability {
            backend: self.native.name().to_string(),
            contained: false,
            native: true,
            probe: self.native.probe(),
        });
        caps
    }
}

/// One row of the capability projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCapability {
    pub backend: String,
    pub contained: bool,
    pub native: bool,
    pub probe: BackendProbe,
}
