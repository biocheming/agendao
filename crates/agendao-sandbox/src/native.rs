//! The explicit unsandboxed channel.
//!
//! `NativeBackend` runs a plan with `ProcessMode::Native` directly on the
//! host — no namespaces, no syscall filter. It refuses contained plans:
//! native is an *earned* mode (policy granted it), never a fallback for
//! a missing sandbox backend. Fail-closed selection happens upstream in
//! `BackendRegistry::select`.
//!
//! Even here the environment contract holds: the child gets exactly the
//! authority-resolved environment (`native_inherit` is filtered, not
//! raw). Children run in their own process group with `kill_on_drop` so
//! cancellation can reach the whole tree (plan §5.7).

use std::sync::Arc;

use async_trait::async_trait;

use crate::backend::{
    BackendChild, BackendExit, BackendProbe, BackendViolationToken, ChildEnvironment,
    SandboxBackend, StdioPlan,
};
use crate::model::ProcessMode;
use crate::plan::SandboxPlan;
use crate::request::SpawnSpec;
use crate::violation::SandboxExecutionError;

#[derive(Debug, thiserror::Error)]
#[error("native backend refuses contained plan `{kind}` (native is never a sandbox fallback)")]
pub struct ContainedPlanRefused {
    pub kind: String,
}

pub struct NativeBackend;

impl NativeBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SandboxBackend for NativeBackend {
    fn name(&self) -> &'static str {
        "native"
    }

    fn probe(&self) -> BackendProbe {
        BackendProbe::available()
    }

    fn supports(&self, plan: &SandboxPlan) -> bool {
        plan.process.mode == ProcessMode::Native
    }

    async fn spawn(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        stdio: &StdioPlan,
        _violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        if plan.process.mode != ProcessMode::Native {
            return Err(SandboxExecutionError::Lifecycle(format!(
                "native backend refuses contained plan `{:?}`: native is never a sandbox fallback",
                plan.requested_kind
            )));
        }

        let mut command = tokio::process::Command::new(&spec.program);
        command.args(&spec.args).env_clear().kill_on_drop(true);
        for (key, value) in env {
            command.env(key, value);
        }
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        command
            .stdin(std::process::Stdio::from(stdio.stdin))
            .stdout(std::process::Stdio::from(stdio.stdout))
            .stderr(std::process::Stdio::from(stdio.stderr));
        #[cfg(unix)]
        command.process_group(0);

        let child = command
            .spawn()
            .map_err(|err| SandboxExecutionError::SpawnFailed {
                backend: self.name().to_string(),
                reason: err,
            })?;
        Ok(Box::new(NativeChild { child }))
    }

    /// Interactive native launch: the pty slave becomes stdio and the
    /// controlling terminal. The pre_exec setsid detaches the child from
    /// the host's terminal; TIOCSCTTY then attaches the fresh slave, so
    /// job control works exactly like a terminal expects.
    #[cfg(unix)]
    async fn spawn_pty(
        &self,
        plan: &SandboxPlan,
        spec: &SpawnSpec,
        env: &ChildEnvironment,
        slave: &crate::platform::pty::PtySlave,
        _violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        if plan.process.mode != ProcessMode::Native {
            return Err(SandboxExecutionError::Lifecycle(format!(
                "native backend refuses contained plan `{:?}`: native is never a sandbox fallback",
                plan.requested_kind
            )));
        }

        let mut command = tokio::process::Command::new(&spec.program);
        command.args(&spec.args).env_clear().kill_on_drop(true);
        for (key, value) in env {
            command.env(key, value);
        }
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        attach_slave_stdio(&mut command, slave)?;
        let child = command
            .spawn()
            .map_err(|err| SandboxExecutionError::SpawnFailed {
                backend: self.name().to_string(),
                reason: err,
            })?;
        Ok(Box::new(NativeChild { child }))
    }
}

/// Wire the pty slave as a command's stdio and controlling terminal:
/// three fd duplicates for 0/1/2 plus a pre_exec setsid + TIOCSCTTY on
/// the fourth. Shared by every unix backend's `spawn_pty` so terminal
/// semantics cannot drift between native and contained launches.
#[cfg(unix)]
pub(crate) fn attach_slave_stdio(
    command: &mut tokio::process::Command,
    slave: &crate::platform::pty::PtySlave,
) -> Result<(), SandboxExecutionError> {
    use std::os::unix::io::AsRawFd;

    let ctty_fd = slave
        .try_clone()
        .map_err(|err| SandboxExecutionError::SpawnFailed {
            backend: "pty".to_string(),
            reason: err,
        })?;
    command
        .stdin(std::process::Stdio::from(slave.try_clone().map_err(
            |err| SandboxExecutionError::SpawnFailed {
                backend: "pty".to_string(),
                reason: err,
            },
        )?))
        .stdout(std::process::Stdio::from(slave.try_clone().map_err(
            |err| SandboxExecutionError::SpawnFailed {
                backend: "pty".to_string(),
                reason: err,
            },
        )?))
        .stderr(std::process::Stdio::from(slave.try_clone().map_err(
            |err| SandboxExecutionError::SpawnFailed {
                backend: "pty".to_string(),
                reason: err,
            },
        )?));

    {
        use std::os::unix::process::CommandExt;
        let raw = ctty_fd.as_raw_fd();
        // SAFETY: the closure only calls setsid, ioctl, and getpid —
        // async-signal-safe between fork and exec. It runs in the child,
        // where `raw` still refers to the inherited slave duplicate.
        unsafe {
            command.as_std_mut().pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // Claim the slave as the controlling terminal, then make
                // this (fresh) process group the terminal's foreground
                // group. Without TIOCSPGRP a job-control shell reading
                // the tty gets SIGTTIN'd into a permanent stop — alive,
                // silent, unkillable by anything but a signal.
                // libc types the TIOCSCTTY request differently per platform
                // (c_int on Linux, c_ulong on macOS); `as _` adopts the
                // platform's ioctl signature.
                let tiocsctty = libc::TIOCSCTTY as _;
                if libc::ioctl(raw, tiocsctty, 0u64) == -1 {
                    // best-effort: no ctty, no job control
                    return Ok(());
                }
                let pgid: libc::pid_t = libc::getpid();
                if libc::ioctl(raw, libc::TIOCSPGRP, &pgid) == -1 {
                    // best-effort: some shells degrade gracefully
                }
                Ok(())
            });
        }
    }
    Ok(())
}

struct NativeChild {
    child: tokio::process::Child,
}

#[async_trait]
impl BackendChild for NativeChild {
    fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    async fn wait(&mut self) -> Result<BackendExit, SandboxExecutionError> {
        let status = self.child.wait().await.map_err(|err| {
            SandboxExecutionError::Lifecycle(format!("native wait failed: {err}"))
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
        signal_group(&self.child, "TERM")
    }

    async fn signal_kill(&mut self) -> Result<(), SandboxExecutionError> {
        signal_group(&self.child, "KILL")
    }
}

/// The child was spawned with `process_group(0)`, so its pid *is* the
/// pgid; signaling the group reaches descendants the direct handle
/// cannot. Shared implementation: `platform::process_tree`.
#[cfg(unix)]
fn signal_group(
    child: &tokio::process::Child,
    signal: &'static str,
) -> Result<(), SandboxExecutionError> {
    crate::platform::process_tree::kill_process_group(child, "native", signal)
}

#[cfg(windows)]
fn signal_group(
    _child: &tokio::process::Child,
    _signal: &'static str,
) -> Result<(), SandboxExecutionError> {
    // kill_on_drop covers cleanup on Windows; TERM/KILL distinction does
    // not exist there, so both escalate the same way.
    Ok(())
}

/// Convenience constructor matching the registry's expected type.
pub fn native_backend() -> Arc<dyn SandboxBackend> {
    Arc::new(NativeBackend::new())
}
