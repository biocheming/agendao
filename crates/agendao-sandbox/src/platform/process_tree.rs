//! Unix process-tree signaling shared by every process-group backend.
//!
//! `NativeBackend` and the platform backends spawn children with
//! `process_group(0)` — the child pid *is* the process-group id — so a
//! signal to the negative pgid reaches descendants the direct handle
//! cannot see (grandchildren that escaped a `wait`, daemons that
//! detached from our pipe but not the group). This replaces the old
//! `pkill -P`/`pgrep` helper-process pattern in `agendao-tool`: no
//! helper spawns are needed to clean a tree we can address directly.

/// Signal the child's whole process group. `label` names the backend in
/// error messages; `signal` is "TERM" or "KILL".
pub fn kill_process_group(
    child: &tokio::process::Child,
    label: &str,
    signal: &'static str,
) -> Result<(), crate::violation::SandboxExecutionError> {
    let Some(pid) = child.id() else {
        return Ok(()); // already reaped
    };
    let signum = signal_signum(signal);
    // SAFETY: plain kill(2) on the child's own process group.
    let result = unsafe { libc::kill(-(pid as i32), signum) };
    if result != 0 {
        // ESRCH: the group is already gone, which is what we wanted.
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::NotFound {
            return Err(crate::violation::SandboxExecutionError::Lifecycle(format!(
                "{label} {signal} of process group {pid} failed: {err}"
            )));
        }
    }
    Ok(())
}

fn signal_signum(signal: &str) -> i32 {
    if signal == "TERM" {
        libc::SIGTERM
    } else {
        libc::SIGKILL
    }
}
