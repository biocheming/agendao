//! Seatbelt backend: containment via a macOS sandbox profile.
//!
//! The profile text (`build_seatbelt_profile`) is the macOS counterpart
//! of `build_bwrap_args`: a pure mapping from the immutable
//! `SandboxPlan` to the platform's enforcement syntax. It compiles and
//! is contract-tested on every host; the execution shell (spawn through
//! `/usr/bin/sandbox-exec`) only exists on macOS.
//!
//! Platform honesty (documented in `docs/sandbox.md`): Seatbelt filters
//! syscalls but has no mount namespaces. The whole host filesystem
//! stays *visible* (deny-by-default makes it unreadable, not absent),
//! and there is no private tmpfs `/tmp` — writable carve-outs are real
//! host directories. The execution-scoped HOME path required for a safe
//! interactive shell is not yet available in this backend, so Seatbelt is
//! fail-closed and disabled from the default registry until that authority
//! path is integrated.

use crate::model::FilesystemMode;
use crate::path::PROTECTED_METADATA_COMPONENTS;
use crate::plan::SandboxPlan;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SeatbeltProfileError {
    #[error("path cannot be safely encoded as an SBPL literal: {0}")]
    UnsafePath(String),
}

/// Host directories every contained plan may read (never write). The
/// macOS additions over the Linux list are the dynamic linker's
/// territory: `/System` (dyld shared cache lives there) and the dyld
/// store under `/private/var/db`.
const READ_ONLY_HOST_DIRS: &[&str] = &[
    "/System",
    "/Library",
    "/usr",
    "/etc",
    "/private/etc",
    "/private/var/db/dyld",
    "/opt",
];

/// Build the Seatbelt (SBPL) profile for one plan. Pure: no I/O, no
/// environment peeking — everything comes from the plan. Rule order is
/// load-bearing: Seatbelt applies the *last* matching rule, so the
/// protected-metadata denials must come after the workspace write
/// allowance (the same stacking discipline bwrap gets from mount
/// ordering).
pub fn build_seatbelt_profile(plan: &SandboxPlan) -> Result<String, SeatbeltProfileError> {
    let mut profile = String::new();
    profile.push_str("(version 1)\n");
    profile.push_str("(deny default)\n");

    // Process bootstrap prerequisites — no containment meaning, but
    // every macOS process needs them to reach main(): mach ports, basic
    // sysctls, and the standard character devices.
    profile.push_str("(allow mach-lookup)\n");
    profile.push_str("(allow sysctl-read)\n");
    profile.push_str("(allow file-read-metadata (literal \"/\"))\n");
    profile.push_str("(allow file-read* (literal \"/dev/null\"))\n");
    profile.push_str("(allow file-read* (literal \"/dev/urandom\"))\n");
    profile.push_str("(allow file-read* (literal \"/dev/random\"))\n");

    for dir in READ_ONLY_HOST_DIRS {
        profile.push_str(&format!(
            "(allow file-read* (subpath \"{}\"))\n",
            sbpl_literal(dir)?
        ));
    }

    // Workspace readability is unconditional; writability follows the
    // filesystem mode exactly like the bwrap bind/ro-bind choice.
    let workspace = plan.filesystem.workspace_root.as_str();
    let workspace = sbpl_literal(workspace)?;
    profile.push_str(&format!("(allow file-read* (subpath \"{workspace}\"))\n"));

    let workspace_writable = plan.filesystem.mode == FilesystemMode::WorkspaceWrite
        && plan
            .filesystem
            .writable_roots
            .iter()
            .any(|root| root.as_str() == workspace);

    if workspace_writable {
        profile.push_str(&format!("(allow file-write* (subpath \"{workspace}\"))\n"));
    }

    // Explicit writable carve-outs (build cache, private home) — the
    // `Restricted` mode's only write surface.
    for root in &plan.filesystem.writable_roots {
        if root.as_str() == workspace {
            continue; // covered above
        }
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            sbpl_literal(root.as_str())?
        ));
    }

    // Protected metadata: re-denied *after* the workspace allowance so
    // the later rule wins — `.git` stays read-only even when the
    // workspace is writable (bwrap gets the same effect by stacking
    // read-only binds on top of the writable bind).
    if workspace_writable {
        for component in PROTECTED_METADATA_COMPONENTS {
            let protected = format!("{workspace}/{component}");
            profile.push_str(&format!(
                "(deny file-write* (subpath \"{}\"))\n",
                sbpl_literal(&protected)?
            ));
        }
    }

    // Network: `deny default` already covers it; the explicit rule is
    // self-documenting and guards against a future allow widening it.
    profile.push_str("(deny network*)\n");
    // Cross-process introspection: the Seatbelt counterpart of the
    // seccomp ptrace deny on Linux. Self-inspection is not filtered by
    // this operation class.
    profile.push_str("(deny process-info*)\n");

    Ok(profile)
}

/// Encode a path for an SBPL string literal. We deliberately reject rather
/// than guess at undocumented escape semantics: quotes, backslashes, NUL,
/// and controls can terminate or inject policy forms on different Seatbelt
/// versions. The backend therefore fails closed for such paths.
fn sbpl_literal(path: &str) -> Result<String, SeatbeltProfileError> {
    if path.is_empty()
        || path
            .chars()
            .any(|character| character == '"' || character == '\\' || character.is_control())
    {
        return Err(SeatbeltProfileError::UnsafePath(path.to_string()));
    }
    Ok(path.to_string())
}

#[cfg(target_os = "macos")]
pub mod backend {
    use std::path::PathBuf;
    use std::sync::Arc;

    use async_trait::async_trait;

    use crate::backend::{
        BackendChild, BackendProbe, BackendViolationToken, ChildEnvironment, SandboxBackend,
        StdioPlan,
    };
    use crate::model::ProcessMode;
    use crate::plan::SandboxPlan;
    use crate::request::SpawnSpec;
    use crate::violation::SandboxExecutionError;

    pub struct SeatbeltBackend {
        _sandbox_exec: PathBuf,
    }

    impl SeatbeltBackend {
        /// A backend pointed at a known sandbox-exec path (tests inject
        /// a missing path to exercise the fail-closed probe).
        pub fn new(sandbox_exec: PathBuf) -> Self {
            Self {
                _sandbox_exec: sandbox_exec,
            }
        }

        /// Resolve sandbox-exec from PATH, falling back to the
        /// system location; probe() decides whether it is usable.
        pub fn discover() -> Self {
            Self::new(
                crate::platform::find_in_path("sandbox-exec")
                    .unwrap_or_else(|| PathBuf::from("/usr/bin/sandbox-exec")),
            )
        }
    }

    #[async_trait]
    impl SandboxBackend for SeatbeltBackend {
        fn name(&self) -> &'static str {
            "seatbelt"
        }

        fn probe(&self) -> BackendProbe {
            BackendProbe::unavailable(
                "Seatbelt backend disabled: execution-scoped private HOME and a supported \
                 replacement for deprecated sandbox-exec are not available",
            )
        }

        fn supports(&self, plan: &SandboxPlan) -> bool {
            let _ = plan;
            false
        }

        async fn spawn(
            &self,
            plan: &SandboxPlan,
            _spec: &SpawnSpec,
            _env: &ChildEnvironment,
            _stdio: &StdioPlan,
            _violation_token: BackendViolationToken,
        ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
            if plan.process.mode != ProcessMode::Contained {
                return Err(SandboxExecutionError::Lifecycle(format!(
                    "seatbelt backend refuses non-contained plan `{:?}`",
                    plan.requested_kind
                )));
            }
            Err(SandboxExecutionError::SandboxUnavailable {
                backend: self.name().to_string(),
                reason: self
                    .probe()
                    .reason
                    .unwrap_or_else(|| "Seatbelt disabled".into()),
            })
        }

        /// Interactive contained launch: identical profile and
        /// sandbox-exec argv as the piped path — only the stdio shape
        /// differs (pty slave + controlling terminal).
        async fn spawn_pty(
            &self,
            plan: &SandboxPlan,
            _spec: &SpawnSpec,
            _env: &ChildEnvironment,
            _slave: &crate::platform::pty::PtySlave,
            _violation_token: BackendViolationToken,
        ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
            if plan.process.mode != ProcessMode::Contained {
                return Err(SandboxExecutionError::Lifecycle(format!(
                    "seatbelt backend refuses non-contained plan `{:?}`",
                    plan.requested_kind
                )));
            }
            Err(SandboxExecutionError::SandboxUnavailable {
                backend: self.name().to_string(),
                reason: self
                    .probe()
                    .reason
                    .unwrap_or_else(|| "Seatbelt disabled".into()),
            })
        }
    }

    /// Convenience constructor matching the registry's expected type.
    pub fn seatbelt_backend() -> Arc<dyn SandboxBackend> {
        Arc::new(SeatbeltBackend::discover())
    }
}

#[cfg(target_os = "macos")]
pub use backend::{seatbelt_backend, SeatbeltBackend};
