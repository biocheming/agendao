//! The launcher: the only path from request to running child.
//!
//! Fixed order (plan §5.4):
//!
//! ```text
//! validate request
//!   -> derive minimum profile
//!   -> build immutable plan
//!   -> probe backend (fail closed for contained plans)
//!   -> resolve child environment
//!   -> emit SandboxPrepared
//!   -> [start] spawn wrapper/child
//!   -> emit SandboxStarted
//!   -> lifecycle handle (wait / cancel / deadline)
//! ```
//!
//! Events are minted *here and in `lifecycle.rs` only*, from the
//! plan's identity. Backends receive an immutable plan and return a
//! `BackendChild`; they have no sink, no id minter, and no way to
//! fabricate `SandboxStarted` or a `plan_fingerprint`. That is the
//! Phase 2 completion gate, enforced by construction rather than by
//! runtime checks.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::{
    BackendCapability, BackendRegistry, BackendViolationToken, ChildEnvironment, StdioPlan,
};
use crate::environment::build_child_environment;
use crate::lifecycle::SandboxExecutionHandle;
use crate::model::{FilesystemMode, NetworkMode, ProcessMode};
use crate::path::CanonicalPath;
use crate::plan::{build_plan, PlanContext, SandboxPlan};
use crate::policy::{derive_profile, PolicyInputs};
use crate::request::{ProfileKind, SandboxExecutionRequest, SpawnSpec, INTERACTIVE_PRIVATE_HOME};
use crate::violation::{SandboxExecutionError, SandboxViolation};

mod contract;
pub use contract::{
    AuthorityReadOnlyRoots, DenialReason, EventLog, ExecutionIdMinter, PrepareOptions,
    ProfileSummary, SandboxEvent, SandboxEventSink, UuidIdMinter,
};
/// The boundary object: registry + event sink + id minter.
pub struct SandboxLauncher {
    registry: BackendRegistry,
    sink: Arc<dyn SandboxEventSink>,
    id_minter: Arc<dyn ExecutionIdMinter>,
}

impl SandboxLauncher {
    pub fn new(registry: BackendRegistry, sink: Arc<dyn SandboxEventSink>) -> Self {
        Self {
            registry,
            sink,
            id_minter: Arc::new(UuidIdMinter),
        }
    }

    pub fn with_id_minter(mut self, minter: Arc<dyn ExecutionIdMinter>) -> Self {
        self.id_minter = minter;
        self
    }

    pub fn capabilities(&self) -> Vec<BackendCapability> {
        self.registry.capabilities()
    }

    /// Validate, derive, plan, probe, and resolve — without spawning.
    /// Every failure emits `SandboxDenied` (under the freshly minted id,
    /// so even a request that never built a plan is auditable) and
    /// returns the typed error.
    pub fn prepare(
        &self,
        request: SandboxExecutionRequest,
        inputs: &PolicyInputs,
        options: &PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        let execution_id = self.id_minter.mint();
        match self.prepare_inner(&request, inputs, options, execution_id.clone()) {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                self.sink.record(SandboxEvent::Denied {
                    execution_id,
                    session_origin: request.session_origin.clone(),
                    reason: denial_from_error(&error),
                    detail: Some(error.to_string()),
                });
                Err(error)
            }
        }
    }

    /// Pure derivation for explanation surfaces (auditing, UI, negative
    /// probes): the plan a request *would* run under. Deliberately
    /// backend-independent — probing belongs to the launch path — and
    /// emits no events, because nothing executed.
    pub fn derive_plan(
        &self,
        request: &SandboxExecutionRequest,
        inputs: &PolicyInputs,
        options: &PrepareOptions,
    ) -> Result<SandboxPlan, SandboxExecutionError> {
        let execution_id = self.id_minter.mint();
        let (plan, _env) = self.plan_and_env(request, inputs, options, execution_id)?;
        Ok(Arc::unwrap_or_clone(plan))
    }

    fn prepare_inner(
        &self,
        request: &SandboxExecutionRequest,
        inputs: &PolicyInputs,
        options: &PrepareOptions,
        execution_id: String,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        // Steps 1-3 + 5: validate, derive, plan, resolve environment.
        let (plan, env) = self.plan_and_env(request, inputs, options, execution_id)?;

        // 4. Probe/select the backend — the fail-closed point for
        //    contained plans (no native fallback, ever).
        let backend = self.registry.select(&plan)?;

        // 6. Emit Prepared — the plan identity is now auditable.
        self.sink.record(SandboxEvent::Prepared {
            execution_id: plan.execution_id.clone(),
            session_origin: plan.session_origin.clone(),
            profile: ProfileSummary {
                requested_kind: plan.requested_kind,
                process_mode: plan.process.mode,
                filesystem_mode: plan.filesystem.mode,
                network_mode: plan.network.mode,
            },
            plan_fingerprint: plan.fingerprint.clone(),
            backend: backend.name().to_string(),
        });

        Ok(PreparedSandboxExecution {
            plan,
            backend,
            spec: request.spec.clone(),
            env,
            stdio: options.stdio,
            sink: self.sink.clone(),
            violation_token: BackendViolationToken::new(),
            started: false,
        })
    }

    /// Validation, policy merge, plan build, and environment resolution
    /// — everything that does not depend on backend availability.
    fn plan_and_env(
        &self,
        request: &SandboxExecutionRequest,
        inputs: &PolicyInputs,
        options: &PrepareOptions,
        execution_id: String,
    ) -> Result<(Arc<SandboxPlan>, ChildEnvironment), SandboxExecutionError> {
        // 1. Validate: the payload must name a real program.
        if request.spec.program.trim().is_empty() {
            return Err(SandboxExecutionError::InvalidRequest(
                "spec.program must name a program".into(),
            ));
        }

        // 2. Derive the minimum profile from the merged policy.
        let profile = derive_profile(request.trust_class, request.profile_kind, inputs)?;

        // 3. Build the immutable, fingerprinted plan.
        let context = PlanContext {
            extra_writable_roots: options.extra_writable_roots.clone(),
            extra_read_only_roots: options.authority_read_only_roots.0.clone(),
            term_grace: options.term_grace,
            execution_id,
            session_origin: request.session_origin.clone(),
        };
        let plan = Arc::new(build_plan(
            &profile,
            request.profile_kind,
            &request.workspace_root,
            &context,
        )?);

        // 5. Resolve the child environment (env-clear, core reinject,
        //    overrides screened, authority keys injected).
        let host: BTreeMap<String, String> = std::env::vars().collect();
        let authority = BTreeMap::from([
            (
                format!(
                    "{}EXECUTION_ID",
                    crate::environment::AGENDAO_SANDBOX_ENV_PREFIX
                ),
                plan.execution_id.clone(),
            ),
            (
                format!(
                    "{}PLAN_FINGERPRINT",
                    crate::environment::AGENDAO_SANDBOX_ENV_PREFIX
                ),
                plan.fingerprint.clone(),
            ),
        ]);
        let env = build_child_environment(
            &plan.environment,
            &host,
            &request.spec.env_overrides,
            &authority,
        )?;

        // Interactive contained shells get a private HOME, last write
        // wins: the core reinject above deliberately re-entered the
        // host HOME (shells need one), and this authority-side rewrite
        // — after screening, keyed on the plan's kind — replaces it
        // with the sandbox-private path. Host dotfiles, ssh agents, and
        // credentials are invisible to the session by construction; the
        // Linux backend `--dir`s the directory into the private tmpfs.
        let mut env = env;
        if plan.requested_kind == ProfileKind::InteractiveShell
            && plan.process.mode == ProcessMode::Contained
        {
            env.insert("HOME".into(), INTERACTIVE_PRIVATE_HOME.into());
        }

        Ok((plan, env))
    }
}

/// Map a typed error onto the denial payload.
fn denial_from_error(error: &SandboxExecutionError) -> DenialReason {
    match error {
        SandboxExecutionError::InvalidRequest(_) => DenialReason::InvalidRequest,
        SandboxExecutionError::Policy(_) => DenialReason::PolicyDenied,
        SandboxExecutionError::Plan(_) => DenialReason::PlanFailed,
        SandboxExecutionError::Environment(_) => DenialReason::EnvironmentRejected,
        SandboxExecutionError::SandboxUnavailable { reason, .. } => {
            DenialReason::BackendUnavailable {
                capability: reason.clone(),
            }
        }
        SandboxExecutionError::SpawnFailed { .. } => DenialReason::SpawnFailed,
        SandboxExecutionError::AlreadyFinished | SandboxExecutionError::Lifecycle(_) => {
            DenialReason::SpawnFailed
        }
    }
}

/// A validated, planned execution awaiting spawn. Holding this proves
/// the request passed policy merge, backend probing, and environment
/// screening. Starting it is the only way to obtain a handle.
pub struct PreparedSandboxExecution {
    plan: Arc<SandboxPlan>,
    backend: Arc<dyn crate::backend::SandboxBackend>,
    spec: SpawnSpec,
    env: ChildEnvironment,
    /// Io shaping only — deliberately absent from the plan and its
    /// fingerprint, because pipes are not policy.
    stdio: StdioPlan,
    sink: Arc<dyn SandboxEventSink>,
    violation_token: BackendViolationToken,
    started: bool,
}

impl std::fmt::Debug for PreparedSandboxExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedSandboxExecution")
            .field("plan", &self.plan)
            .field("backend", &self.backend.name())
            .field("program", &self.spec.program)
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl PreparedSandboxExecution {
    pub fn plan(&self) -> &SandboxPlan {
        &self.plan
    }

    /// Spawn and emit `SandboxStarted`. Consumes self so an execution
    /// can never be started twice.
    pub async fn start(mut self) -> Result<SandboxExecutionHandle, SandboxExecutionError> {
        if self.started {
            return Err(SandboxExecutionError::AlreadyFinished);
        }
        self.started = true;
        let child = match self
            .backend
            .spawn(
                &self.plan,
                &self.spec,
                &self.env,
                &self.stdio,
                self.violation_token.clone(),
            )
            .await
        {
            Ok(child) => child,
            Err(error) => {
                self.record_spawn_failure(&error);
                return Err(error);
            }
        };
        Ok(self.finish_start(child))
    }

    /// Spawn on a freshly allocated pty: the slave becomes the child's
    /// stdio and controlling terminal (the `StdioPlan` is ignored — the
    /// terminal IS the io shape); the master side is handed back to the
    /// host for reader/writer/resize. Same event ladder as `start`.
    #[cfg(unix)]
    pub async fn start_pty(
        mut self,
        dims: crate::platform::pty::PtyDimensions,
    ) -> Result<(SandboxExecutionHandle, crate::platform::pty::PtyMaster), SandboxExecutionError>
    {
        if self.started {
            return Err(SandboxExecutionError::AlreadyFinished);
        }
        self.started = true;
        let (master, slave) = crate::platform::pty::openpty(dims).map_err(|err| {
            SandboxExecutionError::SpawnFailed {
                backend: "pty".to_string(),
                reason: err,
            }
        })?;
        let child = match self
            .backend
            .spawn_pty(
                &self.plan,
                &self.spec,
                &self.env,
                &slave,
                self.violation_token.clone(),
            )
            .await
        {
            Ok(child) => child,
            Err(error) => {
                self.record_spawn_failure(&error);
                return Err(error);
            }
        };
        // The slave's last handle drops here; the child keeps its own
        // inherited copies, and the master stays open in the host.
        drop(slave);
        Ok((self.finish_start(child), master))
    }

    /// A terminal is not emulated with an unsandboxed host process on
    /// platforms without a contained PTY backend. Callers compile, then get
    /// an explicit fail-closed error at runtime.
    #[cfg(not(unix))]
    pub async fn start_pty(
        self,
        _dims: crate::platform::pty::PtyDimensions,
    ) -> Result<(SandboxExecutionHandle, crate::platform::pty::PtyMaster), SandboxExecutionError>
    {
        Err(SandboxExecutionError::SandboxUnavailable {
            backend: self.backend.name().to_string(),
            reason: "contained PTY launches are unsupported on this platform".to_string(),
        })
    }

    fn record_spawn_failure(&self, error: &SandboxExecutionError) {
        self.sink.record(SandboxEvent::Denied {
            execution_id: self.plan.execution_id.clone(),
            session_origin: self.plan.session_origin.clone(),
            reason: denial_from_error(error),
            detail: Some(error.to_string()),
        });
    }

    /// Wrap the backend child and emit `SandboxStarted` from the plan's
    /// identity — the single place a started execution comes from.
    fn finish_start(&self, child: Box<dyn crate::backend::BackendChild>) -> SandboxExecutionHandle {
        let backend_name = self.backend.name().to_string();
        let handle = SandboxExecutionHandle::new(
            child,
            backend_name.clone(),
            self.plan.clone(),
            self.sink.clone(),
            self.violation_token.clone(),
        );
        self.sink.record(SandboxEvent::Started {
            execution_id: handle.execution_id().to_string(),
            session_origin: self.plan.session_origin.clone(),
            pid: handle.pid(),
            backend: backend_name,
        });
        handle
    }
}
