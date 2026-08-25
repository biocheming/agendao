//! Sandbox execution requests.
//!
//! Tools describe *what kind* of execution they need and the spawn
//! payload; they can never attach sandbox parameters ("I am already
//! sandboxed"), never choose backends, and never name unrestricted
//! profiles. The authority turns a `ProfileKind` plus the policy inputs
//! into the final `SandboxProfile`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::TrustClass;

/// Named execution profile kinds. `Native` is a *request* that only the
/// authority may grant (explicit session mode / host management).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    /// Default model-reachable execution: workspace writable, network
    /// denied, environment cleared.
    WorkspaceWrite,
    /// Scheduler criterion check: workspace read-only plus an
    /// authority-resolved build cache root, network denied.
    Check,
    /// Interactive PTY shell: contained, private HOME (never the host's).
    InteractiveShell,
    /// User-configured integration (MCP / LSP / plugin host): contained,
    /// workspace-scoped, network denied. The binary is chosen by user
    /// configuration, so the profile never inherits the model tools'
    /// workspace-write ceiling by default (sandbox plan Phase 6).
    Integration,
    /// Unsandboxed host execution; requires explicit authority grant.
    Native,
}

/// Where a contained interactive shell's HOME points: a directory inside
/// the sandbox's private `/tmp` tmpfs. The launcher rewrites HOME to this
/// path (post-screening, kind-mandated) and the Linux backend `--dir`s it
/// into existence; the host's dotfiles, ssh agents, and credentials stay
/// invisible to the session by construction.
pub const INTERACTIVE_PRIVATE_HOME: &str = "/tmp/agendao-home";

/// The process payload a tool asks the boundary to run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Environment overrides. Validated against the environment policy;
    /// denied or authority-reserved keys are a hard error.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_overrides: BTreeMap<String, String>,
}

impl SpawnSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env_overrides: BTreeMap::new(),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

/// A tool's request to the sandbox execution boundary. No execution id
/// here: ids are minted by the authority so tools cannot forge or reuse
/// audit identities.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SandboxExecutionRequest {
    pub trust_class: TrustClass,
    pub profile_kind: ProfileKind,
    pub spec: SpawnSpec,
    /// Workspace root as known to the orchestrator; canonicalized by the
    /// plan builder, never trusted raw for containment.
    pub workspace_root: PathBuf,
    /// Which session triggered this execution, when one did. Carried on
    /// every lifecycle event so projections route sandbox facts to the
    /// right frontend stream instead of guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_origin: Option<String>,
}

impl SandboxExecutionRequest {
    pub fn new(
        trust_class: TrustClass,
        profile_kind: ProfileKind,
        spec: SpawnSpec,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            trust_class,
            profile_kind,
            spec,
            workspace_root: workspace_root.into(),
            session_origin: None,
        }
    }

    /// Tag the execution with the session that requested it.
    pub fn with_session_origin(mut self, session_id: impl Into<String>) -> Self {
        self.session_origin = Some(session_id.into());
        self
    }
}
