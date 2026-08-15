//! Shared lifecycle mediation for side-effect operations.
//!
//! Adapters (CLI, TUI, Server, tools) route process mutations through
//! [`global_lifecycle()`] instead of talking to domain registries directly.

use std::sync::{Arc, OnceLock};

use agendao_core::process_registry::global_registry;

/// Side-effect operations that adapters may request.
pub trait LifecycleCommands: Send + Sync {
    /// Kill a subprocess (layered: on_shutdown callback, then SIGTERM/SIGKILL).
    fn kill_process(&self, pid: u32) -> Result<(), std::io::Error>;
}

struct DefaultLifecycleCommands;

impl LifecycleCommands for DefaultLifecycleCommands {
    fn kill_process(&self, pid: u32) -> Result<(), std::io::Error> {
        tracing::info!(pid, "kill_process requested via execution lifecycle");
        global_registry().kill(pid)
    }
}

static LIFECYCLE: OnceLock<Arc<dyn LifecycleCommands>> = OnceLock::new();

/// Returns the global lifecycle commands mediator.
pub fn global_lifecycle() -> &'static Arc<dyn LifecycleCommands> {
    LIFECYCLE.get_or_init(|| Arc::new(DefaultLifecycleCommands))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_lifecycle_returns_consistent_reference() {
        let a = global_lifecycle();
        let b = global_lifecycle();
        assert!(Arc::ptr_eq(a, b));
    }

    #[test]
    fn kill_nonexistent_process_returns_error() {
        let lc = global_lifecycle();
        let result = lc.kill_process(999_999_999);
        assert!(result.is_err());
    }
}
