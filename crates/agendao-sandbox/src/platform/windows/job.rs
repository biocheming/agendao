//! Job-object model for the Windows backend.
//!
//! Pure configuration only — no Win32 calls in this phase. On Windows
//! the job object is the process-tree containment primitive (the
//! counterpart of bwrap's process group + `PR_SET_PDEATHSIG`): assign
//! the child to a job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, hold
//! the job handle in the `BackendChild`, and closing it during the
//! TERM ladder takes the whole tree down.

/// Job-object limits the backend assigns for every contained launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobObjectConfig {
    /// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: the tree dies when the
    /// authority drops the last job handle — Windows' kill-on-drop.
    pub kill_on_job_close: bool,
    /// `JOB_OBJECT_LIMIT_BREAKAWAY_OK` stays OFF: children cannot
    /// escape the job by breaking away.
    pub breakaway_allowed: bool,
    /// `TerminateJobObject` during the kill escalation phase.
    pub terminate_on_kill: bool,
}

/// The configuration every contained Windows launch gets.
pub fn job_object_config() -> JobObjectConfig {
    JobObjectConfig {
        kill_on_job_close: true,
        breakaway_allowed: false,
        terminate_on_kill: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contained_jobs_never_allow_breakaway() {
        let config = job_object_config();
        assert!(config.kill_on_job_close);
        assert!(
            !config.breakaway_allowed,
            "breakaway would be an escape hatch"
        );
        assert!(config.terminate_on_kill);
    }
}
