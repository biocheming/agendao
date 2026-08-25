//! Server-side sandbox authority: the single owner of sandbox launches.
//!
//! It wraps the domain `SandboxLauncher` with this session's governance
//! context (session mode, admin/agent hard policy) and turns completed
//! permission decisions into minimal profiles. Permission intent stays
//! with the existing permission authority — this type *consumes* grants,
//! never re-authorizes them (plan §4.4).
//!
//! It also implements the tool-facing `SandboxExecutionBoundary`, so
//! `ToolContext` gets its authority view here. Events flow out through
//! the injected sink into the runtime event path (Phase 5 wires the
//! `agendao-server-core` projections).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use agendao_sandbox::{
    native_backend, BackendCapability, BackendRegistry, HardPolicy, PermissionGrantScope,
    PolicyInputs, PrepareOptions, PreparedSandboxExecution, SandboxEvent, SandboxEventSink,
    SandboxExecutionError, SandboxExecutionHandle, SandboxExecutionRequest, SandboxLauncher,
    SandboxPlan,
};
use agendao_tool_core::SandboxExecutionBoundary;
use agendao_types::SessionPermissionMode;

/// The production backend registry: the explicit native channel plus
/// every platform backend this build knows (bwrap on Linux). Selection
/// stays fail-closed in the registry — a host without a contained
/// backend denies contained launches rather than falling back.
pub fn production_backend_registry() -> BackendRegistry {
    let mut registry = BackendRegistry::native_only(native_backend());
    for backend in agendao_sandbox::default_platform_backends() {
        registry = registry.with_platform_backend(backend);
    }
    registry
}

/// Phase 5 sink: every sandbox lifecycle event is converted into a
/// `ServerEvent` and broadcast on the runtime event bus, so all
/// frontends see the same sandbox stream through the single projector.
/// Structured tracing stays on as the audit floor — projection is
/// additive, never a replacement for the log record.
pub struct ProjectingSandboxEventSink {
    bus: tokio::sync::broadcast::Sender<std::sync::Arc<agendao_server_core::ServerBusEvent>>,
}

impl ProjectingSandboxEventSink {
    pub fn new(
        bus: tokio::sync::broadcast::Sender<std::sync::Arc<agendao_server_core::ServerBusEvent>>,
    ) -> Self {
        Self { bus }
    }
}

impl SandboxEventSink for ProjectingSandboxEventSink {
    fn record(&self, event: SandboxEvent) {
        match &event {
            SandboxEvent::Denied { .. } | SandboxEvent::Violation { .. } => {
                tracing::warn!(event = ?event, "sandbox event")
            }
            _ => tracing::debug!(event = ?event, "sandbox event"),
        }
        let server_event = match event {
            SandboxEvent::Prepared {
                execution_id,
                session_origin,
                profile,
                plan_fingerprint,
                backend,
            } => agendao_server_core::ServerEvent::SandboxPrepared {
                session_id: session_origin,
                execution_id,
                profile_kind: profile_kind_wire(profile.requested_kind),
                plan_fingerprint,
                backend,
            },
            SandboxEvent::Started {
                execution_id,
                session_origin,
                pid,
                backend,
            } => agendao_server_core::ServerEvent::SandboxStarted {
                session_id: session_origin,
                execution_id,
                pid,
                backend,
            },
            SandboxEvent::Denied {
                execution_id,
                session_origin,
                reason,
                detail,
            } => {
                let (reason, detail) = denial_wire(reason, detail);
                agendao_server_core::ServerEvent::SandboxDenied {
                    session_id: session_origin,
                    execution_id,
                    reason,
                    detail,
                }
            }
            SandboxEvent::Violation { violation } => {
                agendao_server_core::ServerEvent::SandboxViolationReported {
                    session_id: violation.session_origin.clone(),
                    execution_id: violation.execution_id.clone(),
                    violation: serde_json::to_value(&violation).unwrap_or(serde_json::Value::Null),
                }
            }
            SandboxEvent::Exited {
                execution_id,
                session_origin,
                status,
                backend,
            } => agendao_server_core::ServerEvent::SandboxExited {
                session_id: session_origin,
                execution_id,
                backend,
                exit_code: status.code,
                success: status.success,
                cleanup: cleanup_wire(status.cleanup),
            },
        };
        // No subscribers yet is a normal startup state, not an error.
        let _ = self.bus.send(std::sync::Arc::new(
            agendao_server_core::ServerBusEvent::event(server_event),
        ));
    }
}

/// Wire form of a profile kind. A hard match (not serde serialization)
/// on purpose: adding a `ProfileKind` variant must break this arm so the
/// wire vocabulary is decided consciously, never inherited silently.
fn profile_kind_wire(kind: agendao_sandbox::ProfileKind) -> String {
    match kind {
        agendao_sandbox::ProfileKind::WorkspaceWrite => "workspace_write",
        agendao_sandbox::ProfileKind::Check => "check",
        agendao_sandbox::ProfileKind::InteractiveShell => "interactive_shell",
        agendao_sandbox::ProfileKind::Integration => "integration",
        agendao_sandbox::ProfileKind::Native => "native",
    }
    .to_string()
}

/// Wire form of a denial. `BackendUnavailable` folds its capability into
/// `detail` so the UI can say exactly what is missing on this host.
fn denial_wire(
    reason: agendao_sandbox::DenialReason,
    detail: Option<String>,
) -> (String, Option<String>) {
    match reason {
        agendao_sandbox::DenialReason::InvalidRequest => ("invalid_request".into(), detail),
        agendao_sandbox::DenialReason::PolicyDenied => ("policy_denied".into(), detail),
        agendao_sandbox::DenialReason::PlanFailed => ("plan_failed".into(), detail),
        agendao_sandbox::DenialReason::EnvironmentRejected => {
            ("environment_rejected".into(), detail)
        }
        agendao_sandbox::DenialReason::BackendUnavailable { capability } => (
            "backend_unavailable".into(),
            Some(detail.unwrap_or(capability)),
        ),
        agendao_sandbox::DenialReason::SpawnFailed => ("spawn_failed".into(), detail),
    }
}

/// Wire form of the exit cleanup ladder.
fn cleanup_wire(status: agendao_sandbox::CleanupStatus) -> String {
    match status {
        agendao_sandbox::CleanupStatus::NaturalExit => "natural_exit",
        agendao_sandbox::CleanupStatus::TerminatedByRequest => "terminated_by_request",
        agendao_sandbox::CleanupStatus::KilledAfterGrace => "killed_after_grace",
        agendao_sandbox::CleanupStatus::TimedOut => "timed_out",
    }
    .to_string()
}

/// Governance defaults fixed at authority construction.
#[derive(Debug, Clone)]
pub struct SandboxAuthorityConfig {
    pub session_mode: SessionPermissionMode,
    /// Admin hard policy from deployment configuration, when present.
    pub admin: Option<HardPolicy>,
    /// Agent-level hard policy, when present.
    pub agent: Option<HardPolicy>,
    /// Exact names exempt from heuristic screening. Hard-denied names win.
    pub environment_allow_exact: BTreeSet<String>,
}

impl SandboxAuthorityConfig {
    pub fn for_session(session_mode: SessionPermissionMode) -> Self {
        Self {
            session_mode,
            admin: None,
            agent: None,
            environment_allow_exact: std::env::var("AGENDAO_SANDBOX_ENV_ALLOW_EXACT")
                .ok()
                .into_iter()
                .flat_map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect(),
        }
    }

    pub fn with_admin(mut self, admin: HardPolicy) -> Self {
        self.admin = Some(admin);
        self
    }

    pub fn with_agent(mut self, agent: HardPolicy) -> Self {
        self.agent = Some(agent);
        self
    }

    pub fn with_environment_allow_exact<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.environment_allow_exact = names.into_iter().map(Into::into).collect();
        self
    }
}

/// The launch authority for one session's sandboxed executions.
pub struct SandboxAuthority {
    config: SandboxAuthorityConfig,
    launcher: Arc<SandboxLauncher>,
}

impl SandboxAuthority {
    pub fn new(
        config: SandboxAuthorityConfig,
        registry: BackendRegistry,
        event_sink: Arc<dyn SandboxEventSink>,
    ) -> Self {
        Self {
            config,
            launcher: Arc::new(SandboxLauncher::new(registry, event_sink)),
        }
    }

    /// Rebind this authority to one session-mode snapshot while preserving
    /// the deployment backend registry, hard policies, and event sink.  This
    /// creates no mutable cross-session state: each returned authority owns
    /// its fixed policy configuration for one launch.
    pub fn for_session_mode(&self, session_mode: SessionPermissionMode) -> Self {
        Self {
            config: SandboxAuthorityConfig {
                session_mode,
                admin: self.config.admin.clone(),
                agent: self.config.agent.clone(),
                environment_allow_exact: self.config.environment_allow_exact.clone(),
            },
            launcher: self.launcher.clone(),
        }
    }

    /// The policy inputs for one execution: session defaults plus the
    /// completed permission grant's scope (never re-derived here).
    pub fn policy_inputs(&self, grant: Option<&PermissionGrantScope>) -> PolicyInputs {
        PolicyInputs {
            platform: HardPolicy::unrestricted(),
            admin: self.config.admin.clone(),
            agent: self.config.agent.clone(),
            session_mode: self.config.session_mode,
            grant: grant.cloned(),
            check_build_cache_root: None,
            environment_allow_exact: self.config.environment_allow_exact.clone(),
        }
    }

    /// Derive the plan for a request without spawning — the
    /// "explain what would run" surface (auditing, UI, negative probes).
    /// Backend-independent by design: probing belongs to the launch path.
    pub fn derive_plan(
        &self,
        request: &SandboxExecutionRequest,
        grant: Option<&PermissionGrantScope>,
        options: &PrepareOptions,
    ) -> Result<SandboxPlan, SandboxExecutionError> {
        let inputs = self.policy_inputs(grant);
        self.launcher.derive_plan(request, &inputs, options)
    }

    /// Full launch: prepare (policy merge, probe, plan, events) + spawn.
    pub async fn launch(
        &self,
        request: SandboxExecutionRequest,
        grant: Option<&PermissionGrantScope>,
        options: &PrepareOptions,
    ) -> Result<SandboxExecutionHandle, SandboxExecutionError> {
        let inputs = self.policy_inputs(grant);
        let prepared = self.launcher.prepare(request, &inputs, options)?;
        prepared.start().await
    }

    /// Launch a scheduler criterion check: the `Check` profile with the
    /// authority-resolved build cache root — the conventional cargo
    /// `../target` sibling of the request's workspace. The root is chosen
    /// and materialized here, never by the calling tool (request.rs
    /// contract). Plan construction only accepts exact existing writable
    /// roots, so a missing cache path can never degrade into a writable
    /// workspace bind.
    pub async fn launch_check(
        &self,
        request: SandboxExecutionRequest,
        options: &PrepareOptions,
    ) -> Result<SandboxExecutionHandle, SandboxExecutionError> {
        // The scheduler asks this authority to run a criterion; it does not
        // get to choose a wider profile through the generic request shape.
        let request = SandboxExecutionRequest {
            profile_kind: agendao_sandbox::ProfileKind::Check,
            ..request
        };
        let cache_root = self.materialize_check_build_cache_root(&request.workspace_root)?;
        let inputs = PolicyInputs {
            check_build_cache_root: Some(cache_root),
            ..self.policy_inputs(None)
        };
        let prepared = self.launcher.prepare(request, &inputs, options)?;
        prepared.start().await
    }

    /// Capability projection: what can run on this host, and why not.
    pub fn capabilities(&self) -> Vec<BackendCapability> {
        self.launcher.capabilities()
    }

    pub fn session_mode(&self) -> SessionPermissionMode {
        self.config.session_mode
    }

    /// Create the sole host-side writable carve-out for a `Check` launch.
    ///
    /// Resolve the workspace first so a symlinked workspace gets a cache
    /// sibling in its physical parent, then create exactly that directory.
    /// This authority-side I/O is deliberate: contained backends can bind
    /// only existing host roots, and the plan builder must remain fail-closed
    /// rather than substituting an ancestor for a missing root.
    fn materialize_check_build_cache_root(
        &self,
        workspace_root: &std::path::Path,
    ) -> Result<PathBuf, SandboxExecutionError> {
        let workspace = std::fs::canonicalize(workspace_root).map_err(|_| {
            agendao_sandbox::PlanError::WorkspaceRootInvalid(workspace_root.to_path_buf())
        })?;
        let parent = workspace.parent().ok_or_else(|| {
            SandboxExecutionError::InvalidRequest(format!(
                "workspace root {} has no parent for the check build cache",
                workspace.display()
            ))
        })?;
        let cache_root = parent.join("target");
        std::fs::create_dir_all(&cache_root).map_err(|err| {
            SandboxExecutionError::Lifecycle(format!(
                "create check build cache root {}: {err}",
                cache_root.display()
            ))
        })?;
        Ok(cache_root)
    }
}

#[async_trait]
impl SandboxExecutionBoundary for SandboxAuthority {
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        // Boundary view carries no file-level grant: process tools
        // (bash, PTY) never had one. File-granted paths go through
        // `launch` with the completed decision's scope. Io shaping and
        // launch extras (writable roots, term grace) arrive via options
        // and never widen policy.
        self.launcher
            .prepare(request, &self.policy_inputs(None), &options)
    }
}
