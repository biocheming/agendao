//! The Bubblewrap backend: Linux contained execution.
//!
//! `build_bwrap_args` is a *pure function* of (plan, spec, env) — the
//! argv must be reproducible from the plan fingerprint, which is what
//! makes "what policy actually ran" auditable. The backend itself only
//! bridges to `bwrap(1)` and stays blind to policy/events/ids.
//!
//! Isolation layers (plan §6.1):
//! * namespaces via `--unshare-all` (user, pid, ipc, uts, net, cgroup)
//! * mounts: read-only OS dirs, workspace bind per filesystem mode,
//!   protected metadata re-bound read-only on top of a writable
//!   workspace, fresh `/proc`, minimal `/dev`, private `/tmp` tmpfs
//! * `--cap-drop ALL`, `--die-with-parent`, `--new-session`
//! * seccomp cBPF (network family + ptrace → EPERM) as defense in
//!   depth behind the network namespace
//!
//! `ProxyOnly` network is refused in `supports()`: the proxy egress
//! path is a Phase 5 authority capability, and pretending a plain
//! namespace could honor it would be silent under-enforcement.

use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::backend::{
    BackendChild, BackendExit, BackendProbe, BackendViolationToken, ChildEnvironment,
    SandboxBackend, StdioPlan,
};
use crate::model::{FilesystemMode, NetworkMode, ProcessMode};
use crate::path::PROTECTED_METADATA_COMPONENTS;
use crate::plan::SandboxPlan;
use crate::request::{ProfileKind, SpawnSpec};
use crate::violation::SandboxExecutionError;

use super::probe;
use super::seccomp::SeccompFilter;

/// Host directories every contained plan may read (never write). The
/// `-try` variants tolerate merged-/usr layouts where some are symlinks.
const READ_ONLY_HOST_DIRS: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/bin", "/sbin", "/opt"];

pub struct BwrapBackend {
    bwrap_path: PathBuf,
}

impl BwrapBackend {
    /// A backend pointed at a known bwrap path (tests inject a missing
    /// path to exercise the fail-closed probe).
    pub fn new(bwrap_path: PathBuf) -> Self {
        Self { bwrap_path }
    }

    /// Resolve bwrap from PATH, falling back to the packaging-standard
    /// location; probe() decides whether it is actually usable.
    pub fn discover() -> Self {
        Self::new(
            crate::platform::find_in_path("bwrap")
                .unwrap_or_else(|| PathBuf::from("/usr/bin/bwrap")),
        )
    }
}

/// Build the bwrap argv for one plan. Pure: no I/O, no environment
/// peeking — everything comes from the plan, spec, and the
/// authority-resolved child environment.
pub fn build_bwrap_args(
    plan: &SandboxPlan,
    spec: &SpawnSpec,
    env: &ChildEnvironment,
) -> Vec<String> {
    let interactive = plan.requested_kind == ProfileKind::InteractiveShell;

    // bwrap may fork the payload after the host-side pre_exec has claimed
    // the pty. Make the payload's session explicit inside the final
    // namespace rather than relying on that wrapper-session inheritance.
    let mut args: Vec<String> = vec![
        "--unshare-all".into(),
        "--die-with-parent".into(),
        "--new-session".into(),
    ];
    args.extend([
        "--cap-drop".into(),
        "ALL".into(),
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
    ]);
    // The private interactive HOME lives on the /tmp tmpfs above; --dir
    // materializes it so shells find an existing home directory.
    if interactive {
        args.push("--dir".into());
        args.push(crate::request::INTERACTIVE_PRIVATE_HOME.into());
    }
    // NetworkMode::Enabled never reaches a contained backend (network
    // validation ties it to Native); Disabled needs nothing beyond
    // --unshare-all. Neither branch adds --share-net.

    for dir in READ_ONLY_HOST_DIRS {
        args.push("--ro-bind-try".into());
        args.push((*dir).into());
        args.push((*dir).into());
    }

    let workspace = plan.filesystem.workspace_root.as_str();
    // A malformed or legacy plan must never turn an explicit writable
    // root into a writable workspace under a read-only profile. The
    // filesystem mode is the authoritative workspace grant; writable
    // roots are only carve-outs unless that mode explicitly permits it.
    let workspace_writable = plan.filesystem.mode == crate::model::FilesystemMode::WorkspaceWrite
        && plan
            .filesystem
            .writable_roots
            .iter()
            .any(|root| root.as_str() == workspace);
    if workspace_writable {
        args.push("--bind".into());
    } else {
        args.push("--ro-bind".into());
    }
    args.push(workspace.into());
    args.push(workspace.into());

    for root in &plan.filesystem.writable_roots {
        if root.as_str() == workspace {
            continue; // bound above
        }
        args.push("--bind".into());
        args.push(root.as_str().into());
        args.push(root.as_str().into());
    }

    // Authority-resolved integration/runtime roots are exposed read-only;
    // they never become writable carve-outs.
    for root in &plan.filesystem.read_only_roots {
        if root.as_str() == workspace
            || plan
                .filesystem
                .writable_roots
                .iter()
                .any(|w| w.as_str() == root.as_str())
        {
            continue;
        }
        args.push("--ro-bind".into());
        args.push(root.as_str().into());
        args.push(root.as_str().into());
    }

    // Protected metadata: re-bind read-only *on top of* the writable
    // workspace bind. Later mounts stack, so `.git` stays read-only
    // even when the workspace is writable. `-try`: absence is normal.
    if workspace_writable {
        for component in PROTECTED_METADATA_COMPONENTS {
            let protected = format!("{workspace}/{component}");
            args.push("--ro-bind-try".into());
            args.push(protected.clone());
            args.push(protected);
        }
    }

    let cwd = spec
        .cwd
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| workspace.to_string());
    args.push("--chdir".into());
    args.push(cwd);

    args.push("--clearenv".into());
    for (key, value) in env {
        args.push("--setenv".into());
        args.push(key.clone());
        args.push(value.clone());
    }

    args.push(spec.program.clone());
    args.extend(spec.args.iter().cloned());
    args
}

#[async_trait]
impl SandboxBackend for BwrapBackend {
    fn name(&self) -> &'static str {
        "bwrap"
    }

    fn probe(&self) -> BackendProbe {
        let path_probe = probe::probe_bwrap();
        if !path_probe.available {
            return path_probe;
        }
        // PATH lookup passed; the configured path itself must be the
        // executable we will run (a test-injected path may not exist).
        match std::fs::metadata(&self.bwrap_path) {
            Ok(meta) if meta.is_file() => BackendProbe::available(),
            _ => BackendProbe::unavailable(format!(
                "bwrap missing from PATH (install the bubblewrap package): {} not executable",
                self.bwrap_path.display()
            )),
        }
    }

    fn supports(&self, plan: &SandboxPlan) -> bool {
        plan.process.mode == ProcessMode::Contained
            && plan.filesystem.mode != FilesystemMode::Unrestricted
            // ProxyOnly needs the authority's proxy endpoint, not a
            // plain namespace; Enabled is native-only by validation.
            && plan.network.mode == NetworkMode::Disabled
    }

    async fn spawn(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        stdio: &StdioPlan,
        _violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        if plan.process.mode != ProcessMode::Contained {
            return Err(SandboxExecutionError::Lifecycle(format!(
                "bwrap backend refuses non-contained plan `{:?}`",
                plan.requested_kind
            )));
        }

        let mut args = build_bwrap_args(plan, spec, env);
        let filter = SeccompFilter::deny_network_and_ptrace();
        let seccomp_fd =
            seccomp_memfd(&filter).map_err(|err| SandboxExecutionError::SpawnFailed {
                backend: self.name().to_string(),
                reason: err,
            })?;

        // Seccomp options must precede the program; the program is the
        // tail of `args`, so inserting at the front is always safe.
        // The fd is pinned to 3 in `pre_exec` below.
        args.insert(0, "3".into());
        args.insert(0, "--seccomp".into());

        let mut command = tokio::process::Command::new(&self.bwrap_path);
        command
            .args(&args)
            .env_clear()
            .kill_on_drop(true)
            // Pipes apply to the bwrap process; the sandboxed payload
            // inherits them through the exec chain.
            .stdin(std::process::Stdio::from(stdio.stdin))
            .stdout(std::process::Stdio::from(stdio.stdout))
            .stderr(std::process::Stdio::from(stdio.stderr));
        // The bwrap process leads its own group; sandbox children die
        // with it via PR_SET_PDEATHSIG (--die-with-parent).
        command.process_group(0);

        {
            use std::os::unix::process::CommandExt;
            let raw = seccomp_fd.as_raw_fd();
            // SAFETY: the closure only calls dup2(2) — async-signal-
            // safe between fork and exec. It runs in the bwrap child,
            // where `raw` still refers to the inherited memfd copy.
            unsafe {
                command.as_std_mut().pre_exec(move || {
                    if libc::dup2(raw, 3) == -1 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }

        let child = command
            .spawn()
            .map_err(|err| SandboxExecutionError::SpawnFailed {
                backend: self.name().to_string(),
                reason: err,
            })?;
        Ok(Box::new(BwrapChild { child }))
    }

    /// Interactive contained launch: same argv construction, same seccomp
    /// filter, same namespaces as the piped path — only the stdio shape
    /// differs (pty slave + controlling terminal instead of pipes). The
    /// slave-attachment dance (setsid + TIOCSCTTY + fd 0/1/2) is shared
    /// with the native backend so terminal semantics stay identical.
    async fn spawn_pty(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        slave: &crate::platform::pty::PtySlave,
        _violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        if plan.process.mode != ProcessMode::Contained {
            return Err(SandboxExecutionError::Lifecycle(format!(
                "bwrap backend refuses non-contained plan `{:?}`",
                plan.requested_kind
            )));
        }

        let mut args = build_bwrap_args(plan, spec, env);
        let filter = SeccompFilter::deny_network_and_ptrace();
        let seccomp_fd =
            seccomp_memfd(&filter).map_err(|err| SandboxExecutionError::SpawnFailed {
                backend: self.name().to_string(),
                reason: err,
            })?;

        // Seccomp options must precede the program; the program is the
        // tail of `args`, so inserting at the front is always safe.
        // The fd is pinned to 3 in `pre_exec` below.
        args.insert(0, "3".into());
        args.insert(0, "--seccomp".into());

        let mut command = tokio::process::Command::new(&self.bwrap_path);
        command.args(&args).env_clear().kill_on_drop(true);
        // The pty slave replaces the stdio plan entirely — the terminal
        // IS the io shape; everything else (seccomp, namespaces, caps)
        // matches the piped launch.
        crate::native::attach_slave_stdio(&mut command, slave)?;

        {
            use std::os::unix::process::CommandExt;
            let raw = seccomp_fd.as_raw_fd();
            // SAFETY: the closure only calls dup2(2) — async-signal-safe
            // between fork and exec. It runs in the bwrap child, where
            // `raw` still refers to the inherited memfd copy. Ordering
            // with attach_slave_stdio's pre_exec does not matter: fd 3
            // never collides with the slave's 0/1/2.
            unsafe {
                command.as_std_mut().pre_exec(move || {
                    if libc::dup2(raw, 3) == -1 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }

        let child = command
            .spawn()
            .map_err(|err| SandboxExecutionError::SpawnFailed {
                backend: self.name().to_string(),
                reason: err,
            })?;
        Ok(Box::new(BwrapChild { child }))
    }
}

/// Materialize the filter as a CLOEXEC memfd bwrap can read. A memfd
/// failure is a spawn failure: seccomp is defense in depth, but a host
/// that cannot hand bwrap the filter is a host we fail loudly on
/// rather than silently degrading the containment stack.
fn seccomp_memfd(filter: &SeccompFilter) -> std::io::Result<SeccompMemfd> {
    SeccompMemfd::with_bytes(&filter.to_bpf_bytes())
}

struct SeccompMemfd(std::fs::File);

impl SeccompMemfd {
    fn with_bytes(bytes: &[u8]) -> std::io::Result<Self> {
        use std::io::Write;
        use std::os::unix::io::FromRawFd;
        // SAFETY: fresh memfd_create descriptor, immediately owned.
        let raw = unsafe { libc::memfd_create(c"agendao-seccomp".as_ptr(), libc::MFD_CLOEXEC) };
        if raw == -1 {
            return Err(std::io::Error::last_os_error());
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(raw) };
        file.write_all(bytes)?;
        file.flush()?;
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(0))?;
        Ok(Self(file))
    }
}

impl AsRawFd for SeccompMemfd {
    fn as_raw_fd(&self) -> i32 {
        self.0.as_raw_fd()
    }
}

struct BwrapChild {
    child: tokio::process::Child,
}

#[async_trait]
impl BackendChild for BwrapChild {
    fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    async fn wait(&mut self) -> Result<BackendExit, SandboxExecutionError> {
        let status =
            self.child.wait().await.map_err(|err| {
                SandboxExecutionError::Lifecycle(format!("bwrap wait failed: {err}"))
            })?;
        Ok(BackendExit::from_status(status))
    }

    fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    async fn signal_term(&mut self) -> Result<(), SandboxExecutionError> {
        crate::platform::process_tree::kill_process_group(&self.child, "bwrap", "TERM")
    }

    async fn signal_kill(&mut self) -> Result<(), SandboxExecutionError> {
        crate::platform::process_tree::kill_process_group(&self.child, "bwrap", "KILL")
    }
}

/// Convenience constructor matching the registry's expected type.
pub fn bwrap_backend() -> Arc<dyn SandboxBackend> {
    Arc::new(BwrapBackend::discover())
}
