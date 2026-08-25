//! The typed boundary every process-launching surface uses.
//!
//! Callers describe *what* to run (`SandboxExecutionRequest`); the
//! authority behind this trait decides *under which plan* (policy merge,
//! backend probing, fingerprinting) and hands back a prepared execution.
//! There is deliberately no escape hatch: no method accepts backend
//! names, sandbox flags, or "already sandboxed" assertions — a caller
//! cannot bypass the plan (sandbox plan §4.4, Phase 2).
//!
//! The trait lives in this crate (not a tooling crate) on purpose:
//! tools, MCP, LSP, plugin hosts, and the PTY surface all consume the
//! same authority without depending on each other's layers. Hosts that
//! install no authority pass `None` and every process-launching surface
//! must fail loudly rather than falling back to a direct spawn.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::model::TrustClass;
use crate::request::{ProfileKind, SandboxExecutionRequest, SpawnSpec};
use crate::{
    canonicalize_existing, AuthorityReadOnlyRoots, CanonicalPath, PrepareOptions,
    PreparedSandboxExecution, SandboxExecutionError,
};

/// Capability handle implemented by the server-side `SandboxAuthority`.
#[async_trait]
pub trait SandboxExecutionBoundary: Send + Sync {
    /// Validate, plan, and probe — without spawning. `start()` on the
    /// returned prepared execution is the actual launch.
    ///
    /// The options carry io shaping (`StdioPlan`) and launch-context
    /// extras (writable roots, term grace) — never policy: the authority
    /// merges policy itself, from governance config plus the completed
    /// permission grant.
    async fn prepare(
        &self,
        request: SandboxExecutionRequest,
        options: PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError>;
}

/// Convenience alias for injected authority handles.
pub type SharedSandboxExecutionBoundary = Arc<dyn SandboxExecutionBoundary>;

/// Launch context for user-configured integrations (MCP servers, LSP
/// servers, plugin hosts): the authority that owns execution plus the
/// workspace the integration is scoped to.
///
/// Integrations run as `TrustClass::UserConfiguredIntegration` under the
/// `Integration` profile kind: contained, workspace-scoped, network
/// denied — never the model tools' workspace-write ceiling by default,
/// and never unrestricted (sandbox plan Phase 6).
#[derive(Clone)]
pub struct IntegrationSandboxContext {
    pub boundary: SharedSandboxExecutionBoundary,
    pub workspace: PathBuf,
    read_only_runtime_roots: AuthorityReadOnlyRoots,
}

impl IntegrationSandboxContext {
    /// Construct an integration launch context from host-authority-selected
    /// runtime roots. Missing roots are rejected rather than silently
    /// substituted with a parent (especially never a whole HOME directory).
    pub fn new(
        boundary: SharedSandboxExecutionBoundary,
        workspace: PathBuf,
        runtime_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, SandboxExecutionError> {
        let roots = runtime_roots
            .into_iter()
            .map(|root| {
                canonicalize_existing(&root).map_err(|error| {
                    SandboxExecutionError::InvalidRequest(format!(
                        "integration runtime root {} is unavailable: {error}",
                        root.display()
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            boundary,
            workspace,
            read_only_runtime_roots: AuthorityReadOnlyRoots::from_canonical(roots),
        })
    }

    pub fn without_runtime_roots(
        boundary: SharedSandboxExecutionBoundary,
        workspace: PathBuf,
    ) -> Self {
        Self {
            boundary,
            workspace,
            read_only_runtime_roots: AuthorityReadOnlyRoots::default(),
        }
    }
    /// Build the request an integration launch always uses. Command,
    /// args, cwd and env overrides come from the user's configuration;
    /// the trust class and profile kind are fixed here so no integration
    /// can widen them.
    pub fn integration_request(&self, spec: SpawnSpec) -> SandboxExecutionRequest {
        SandboxExecutionRequest::new(
            TrustClass::UserConfiguredIntegration,
            ProfileKind::Integration,
            spec,
            &self.workspace,
        )
    }

    /// Prepare through the integration boundary. Runtime visibility comes
    /// solely from the authority-owned context token; ordinary request data
    /// cannot add host-readable roots.
    pub async fn prepare(
        &self,
        mut spec: SpawnSpec,
        mut options: PrepareOptions,
    ) -> Result<PreparedSandboxExecution, SandboxExecutionError> {
        let mut roots = self.read_only_runtime_roots.0.clone();
        roots.extend(resolve_integration_runtime_roots(&mut spec)?);
        options.authority_read_only_roots = AuthorityReadOnlyRoots::from_canonical(roots);
        self.boundary
            .prepare(self.integration_request(spec), options)
            .await
    }
}

/// Narrow plugin-only cache roots. Generic MCP/LSP integrations must not
/// inherit these mounts; callers opt in only for the plugin host.
pub fn plugin_runtime_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    [
        home.join(".cache/opencode/plugins"),
        home.join(".cache/opencode/node_modules"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

/// Resolve only the narrow runtime locations that integration execution
/// needs in practice. This intentionally does not search PATH or bind HOME:
/// a configured bare command remains the backend's normal executable lookup,
/// while an absolute user-home executable is admitted only from conventional
/// tool bins. Plugin package cache is likewise the one exact opencode cache,
/// never `~/.cache` as a whole.
fn resolve_integration_runtime_roots(
    spec: &mut SpawnSpec,
) -> Result<Vec<CanonicalPath>, SandboxExecutionError> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Ok(Vec::new());
    };
    resolve_integration_runtime_roots_from(&home, spec)
}

fn resolve_integration_runtime_roots_from(
    home: &std::path::Path,
    spec: &mut SpawnSpec,
) -> Result<Vec<CanonicalPath>, SandboxExecutionError> {
    let approved_bins = [home.join(".cargo/bin"), home.join(".local/bin")];
    let mut candidates = Vec::new();
    let program = PathBuf::from(&spec.program);
    if program.is_absolute() {
        if let Some(parent) = program.parent() {
            let opencode = home.join(".cache/opencode");
            if approved_bins.iter().any(|bin| parent == bin) || parent.starts_with(&opencode) {
                let target = canonicalize_existing(&program).map_err(|error| {
                    SandboxExecutionError::InvalidRequest(format!(
                        "configured integration executable {} cannot be resolved: {error}",
                        program.display()
                    ))
                })?;
                let target_path = target.as_path();
                let target_allowed = approved_bins.iter().any(|bin| target_path.starts_with(bin))
                    || target_path.starts_with(&opencode);
                if !target_allowed {
                    return Err(SandboxExecutionError::InvalidRequest(format!(
                        "configured integration executable {} resolves outside its approved runtime roots: {}",
                        program.display(),
                        target_path.display()
                    )));
                }
                let target_parent = target_path.parent().ok_or_else(|| {
                    SandboxExecutionError::InvalidRequest(format!(
                        "configured integration executable {} has no parent",
                        target_path.display()
                    ))
                })?;
                candidates.push(target_parent.to_path_buf());
                // The contained process must execute the same canonical object
                // the authority verified, rather than re-following a host
                // symlink after the plan was created.
                spec.program = target_path.to_string_lossy().into_owned();
            }
        }
    }
    for arg in &spec.args {
        let path = PathBuf::from(arg);
        if path.is_absolute() && path.starts_with(home.join(".cache/opencode")) {
            if let Some(parent) = path.parent() {
                candidates.push(parent.to_path_buf());
            }
        }
    }
    Ok(candidates
        .into_iter()
        .filter_map(|path| canonicalize_existing(&path).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::resolve_integration_runtime_roots_from;
    use crate::request::SpawnSpec;

    #[test]
    fn runtime_discovery_is_narrow_and_excludes_credentials() {
        let home = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut spec = SpawnSpec::new(home.join(".ssh/agent").to_string_lossy());
        let roots = resolve_integration_runtime_roots_from(&home, &mut spec).unwrap();
        assert!(roots.iter().all(|root| {
            let path = root.as_path();
            path.ends_with(".cargo/bin") || path.ends_with(".local/bin")
        }));
        assert!(!roots.iter().any(|root| root.as_path().ends_with(".ssh")));
        let mut bare = SpawnSpec::new("rust-analyzer");
        assert!(
            resolve_integration_runtime_roots_from(&home, &mut bare)
                .unwrap()
                .is_empty(),
            "bare PATH commands must not mount a host PATH directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn approved_runtime_symlink_escaping_its_root_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .expect("crate is nested beneath repository root")
            .join("target/agendao-boundary-symlink-test")
            .join(std::process::id().to_string());
        let home = root.join("home");
        let cargo_bin = home.join(".cargo/bin");
        let outside = root.join("outside-runtime");
        std::fs::create_dir_all(&cargo_bin).unwrap();
        std::fs::write(&outside, "#!/bin/sh\nexit 0\n").unwrap();
        symlink(&outside, cargo_bin.join("escaped-runtime")).unwrap();
        let mut spec = SpawnSpec::new(cargo_bin.join("escaped-runtime").to_string_lossy());
        let error = resolve_integration_runtime_roots_from(&home, &mut spec).unwrap_err();
        assert!(error
            .to_string()
            .contains("outside its approved runtime roots"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
