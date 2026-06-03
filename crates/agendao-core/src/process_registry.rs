//! Global process registry for tracking child processes spawned by the application.
//!
//! Provides a singleton [`ProcessRegistry`] that plugins, tools, and agents
//! use to register/unregister their OS processes. The TUI reads this registry
//! to display a live process panel in the sidebar.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::RwLock;

/// Global singleton process registry.
static REGISTRY: Lazy<ProcessRegistry> = Lazy::new(ProcessRegistry::new);

/// Returns a reference to the global [`ProcessRegistry`].
pub fn global_registry() -> &'static ProcessRegistry {
    &REGISTRY
}

/// RAII guard — drop 时自动从 ProcessRegistry 注销。
///
/// 由 `register()` / `register_with_shutdown()` 返回。
/// 调用者持有 guard（存为字段或 move 进 spawn task），
/// 当 task 退出或 struct 被 drop 时自动清理 — 保证第七条（生命周期对称性）。
pub struct ProcessGuard {
    pid: u32,
    /// When true, `Drop` will NOT call `unregister`.
    /// Used in reconnect scenarios where PID ownership transfers.
    defused: AtomicBool,
}

impl ProcessGuard {
    fn new(pid: u32) -> Self {
        Self {
            pid,
            defused: AtomicBool::new(false),
        }
    }

    /// Disarm the guard so that dropping it will NOT unregister the PID.
    ///
    /// Use this when transferring ownership (e.g. reconnect: old guard is
    /// defused, new guard takes over the new PID).
    pub fn defuse(&self) {
        self.defused.store(true, Ordering::Relaxed);
    }

    /// Returns the PID this guard protects.
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if !self.defused.load(Ordering::Relaxed) {
            global_registry().unregister(self.pid);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    Plugin,
    Bash,
    Agent,
    Mcp,
    Lsp,
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub kind: ProcessKind,
    pub started_at: i64,
    pub cpu_percent: f32,
    pub memory_kb: u64,
}

pub struct ProcessRegistry {
    processes: RwLock<HashMap<u32, ProcessInfo>>,
    /// Graceful shutdown callbacks, keyed by PID.
    shutdown_callbacks: RwLock<HashMap<u32, Arc<dyn Fn() + Send + Sync>>>,
    /// Previous CPU jiffies snapshot for delta-based CPU% calculation.
    prev_cpu: RwLock<HashMap<u32, (u64, u64)>>,
}

impl ProcessRegistry {
    fn new() -> Self {
        Self {
            processes: RwLock::new(HashMap::new()),
            shutdown_callbacks: RwLock::new(HashMap::new()),
            prev_cpu: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, pid: u32, name: String, kind: ProcessKind) -> ProcessGuard {
        let info = ProcessInfo {
            pid,
            name,
            kind,
            started_at: chrono::Utc::now().timestamp(),
            cpu_percent: 0.0,
            memory_kb: 0,
        };
        self.processes.write().insert(pid, info);
        ProcessGuard::new(pid)
    }

    /// Register a process with a graceful shutdown callback.
    ///
    /// When `kill()` is called, the callback fires first (giving the process
    /// a chance to shut down cleanly), then SIGTERM after 500ms, then SIGKILL.
    pub fn register_with_shutdown(
        &self,
        pid: u32,
        name: String,
        kind: ProcessKind,
        on_shutdown: Arc<dyn Fn() + Send + Sync>,
    ) -> ProcessGuard {
        let guard = self.register(pid, name, kind);
        self.shutdown_callbacks.write().insert(pid, on_shutdown);
        guard
    }

    pub fn unregister(&self, pid: u32) {
        self.processes.write().remove(&pid);
        self.shutdown_callbacks.write().remove(&pid);
        self.prev_cpu.write().remove(&pid);
    }

    pub fn list(&self) -> Vec<ProcessInfo> {
        self.processes.read().values().cloned().collect()
    }

    /// Layered kill:
    ///   1. If `on_shutdown` callback exists → call it (graceful)
    ///   2. Wait 500ms
    ///   3. SIGTERM
    ///   4. Wait 500ms
    ///   5. SIGKILL
    ///
    /// Processes without a callback skip straight to SIGTERM → SIGKILL.
    pub fn kill(&self, pid: u32) -> Result<(), std::io::Error> {
        // Fire graceful shutdown callback if registered.
        let shutdown_callback = self.shutdown_callbacks.read().get(&pid).cloned();
        if let Some(cb) = shutdown_callback {
            cb();
            std::thread::sleep(std::time::Duration::from_millis(500));
            if !is_process_alive(pid) {
                self.unregister(pid);
                return Ok(());
            }
        }

        #[cfg(unix)]
        {
            use std::io::{Error, ErrorKind};
            // SIGTERM
            let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if ret != 0 {
                let err = Error::last_os_error();
                if err.kind() != ErrorKind::PermissionDenied {
                    self.unregister(pid);
                }
                return Err(err);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            if is_process_alive(pid) {
                let kill_ret = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                if kill_ret != 0 {
                    let err = Error::last_os_error();
                    if err.kind() != ErrorKind::PermissionDenied {
                        self.unregister(pid);
                    }
                    return Err(err);
                }
            }
            self.unregister(pid);
            Ok(())
        }
        #[cfg(not(unix))]
        {
            self.unregister(pid);
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "kill not supported on this platform",
            ))
        }
    }

    /// Refresh CPU and memory stats by reading `/proc/<pid>/stat` and `/proc/<pid>/status`.
    /// Sums stats across the entire child process tree so that e.g. bun's worker
    /// threads are included in the parent plugin's numbers.
    pub fn refresh_stats(&self) {
        let pids: Vec<u32> = self.processes.read().keys().copied().collect();
        let mut stale = Vec::new();

        for pid in pids {
            match read_proc_tree_stats(pid) {
                Some((cpu_ticks, mem_kb)) => {
                    let cpu_percent = self.compute_cpu_percent(pid, cpu_ticks);
                    if let Some(info) = self.processes.write().get_mut(&pid) {
                        info.cpu_percent = cpu_percent;
                        info.memory_kb = mem_kb;
                    }
                }
                None => {
                    // Process no longer exists
                    stale.push(pid);
                }
            }
        }

        for pid in stale {
            self.unregister(pid);
        }
    }

    fn compute_cpu_percent(&self, pid: u32, current_ticks: u64) -> f32 {
        let mut prev = self.prev_cpu.write();
        let Some(total_now) = read_total_cpu_ticks() else {
            prev.insert(pid, (current_ticks, 0));
            return 0.0;
        };
        let (prev_ticks, prev_total) = prev.get(&pid).copied().unwrap_or((0, total_now));
        prev.insert(pid, (current_ticks, total_now));

        let dticks = current_ticks.saturating_sub(prev_ticks) as f64;
        let dtotal = total_now.saturating_sub(prev_total).max(1) as f64;
        ((dticks / dtotal) * 100.0) as f32
    }

    /// Spawn a background reaper that periodically scans for dead processes.
    ///
    /// This is a defense layer (Strategy A) — the primary cleanup is via
    /// ProcessGuard RAII (Strategy B). The reaper catches any orphaned entries
    /// that somehow escaped the guard (e.g. external kills, mem::forget).
    ///
    /// Must be called after a tokio runtime is available.
    pub fn spawn_reaper(&self, interval: std::time::Duration) {
        let registry = &REGISTRY;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let pids: Vec<u32> = registry.processes.read().keys().copied().collect();
                for pid in pids {
                    if !is_process_alive(pid) {
                        tracing::debug!(pid, "reaper: cleaning up dead process");
                        registry.unregister(pid);
                    }
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Process liveness check
// ---------------------------------------------------------------------------

/// Check if a process is still alive by probing `/proc/<pid>/stat`.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::metadata(format!("/proc/{}", pid)).is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // On non-Linux, use kill(pid, 0) to probe without sending a signal.
        #[cfg(unix)]
        {
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            true // Can't check — assume alive, guard drop is the primary mechanism
        }
    }
}

// ---------------------------------------------------------------------------
// /proc helpers (Linux only)
// ---------------------------------------------------------------------------

/// Read utime+stime from `/proc/<pid>/stat` and VmRSS from `/proc/<pid>/status`.
/// Returns `(cpu_ticks, memory_kb)` or `None` if the process is gone.
fn read_proc_stats(pid: u32) -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
        // Fields after the comm (which may contain spaces/parens):
        // find closing ')' then split the rest.
        let after_comm = stat.rfind(')')? + 2;
        let fields: Vec<&str> = stat[after_comm..].split_whitespace().collect();
        // field index 11 = utime, 12 = stime (0-indexed after comm)
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;

        let status = std::fs::read_to_string(format!("/proc/{}/status", pid)).ok()?;
        let mem_kb = status
            .lines()
            .find(|l| l.starts_with("VmRSS:"))
            .and_then(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0);

        Some((utime + stime, mem_kb))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Some((0, 0))
    }
}

/// Total CPU ticks from `/proc/stat` (sum of all fields on the first `cpu` line).
fn read_total_cpu_ticks() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let cpu_line = stat.lines().next()?;
        let total: u64 = cpu_line
            .split_whitespace()
            .skip(1) // skip "cpu"
            .filter_map(|v| v.parse::<u64>().ok())
            .sum();
        Some(total)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Sum CPU ticks and memory across the entire process tree rooted at `pid`.
/// This captures child workers (e.g. bun spawning threads/subprocesses).
fn read_proc_tree_stats(pid: u32) -> Option<(u64, u64)> {
    let root_stats = read_proc_stats(pid)?;
    let children = collect_descendant_pids(pid);
    let mut total_ticks = root_stats.0;
    let mut total_mem = root_stats.1;
    for child_pid in children {
        if let Some((ticks, mem)) = read_proc_stats(child_pid) {
            total_ticks += ticks;
            total_mem += mem;
        }
    }
    Some((total_ticks, total_mem))
}

/// Recursively collect all descendant PIDs by scanning /proc/*/stat for matching ppid.
fn collect_descendant_pids(root_pid: u32) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let mut result = Vec::new();
        let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return result;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid_str) = name.to_str() else {
                continue;
            };
            let Ok(pid) = pid_str.parse::<u32>() else {
                continue;
            };
            if pid == root_pid {
                continue;
            }
            if let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
                if let Some(ppid) = parse_parent_pid_from_stat(&stat) {
                    children_by_parent.entry(ppid).or_default().push(pid);
                }
            }
        }

        let mut queue = vec![root_pid];
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        while let Some(parent) = queue.pop() {
            let Some(children) = children_by_parent.get(&parent) else {
                continue;
            };
            for &child in children {
                if child == root_pid || !seen.insert(child) {
                    continue;
                }
                result.push(child);
                queue.push(child);
            }
        }
        result
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root_pid;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn parse_parent_pid_from_stat(stat: &str) -> Option<u32> {
    let after_comm = stat.rfind(')')? + 2;
    let fields: Vec<&str> = stat[after_comm..].split_whitespace().collect();
    fields.get(1)?.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn registry_test_guard() -> MutexGuard<'static, ()> {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("process registry test lock should not be poisoned")
    }

    #[test]
    fn test_register_with_shutdown_stores_callback() {
        let registry = ProcessRegistry::new();
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        let guard = registry.register_with_shutdown(
            99999,
            "test-proc".to_string(),
            ProcessKind::Plugin,
            Arc::new(move || {
                called_clone.store(true, Ordering::SeqCst);
            }),
        );
        let procs = registry.list();
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].name, "test-proc");
        // Callback is stored but not yet called
        assert!(!called.load(Ordering::SeqCst));
        // Defuse so guard drop doesn't call global_registry().unregister()
        guard.defuse();
    }

    #[test]
    fn test_register_without_shutdown_has_no_callback() {
        let registry = ProcessRegistry::new();
        let guard = registry.register(12345, "basic".to_string(), ProcessKind::Bash);
        let procs = registry.list();
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].name, "basic");
        guard.defuse();
    }

    #[test]
    fn test_kill_calls_shutdown_callback_before_os_kill() {
        let registry = ProcessRegistry::new();
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        // Use a fake PID that doesn't exist to avoid actually killing anything
        let fake_pid = 999_999_999;
        let guard = registry.register_with_shutdown(
            fake_pid,
            "test-layered".to_string(),
            ProcessKind::Plugin,
            Arc::new(move || {
                called_clone.store(true, Ordering::SeqCst);
            }),
        );
        // Defuse — kill() will unregister internally
        guard.defuse();

        // kill() will call the callback first, then try SIGTERM (which will fail for fake PID)
        let _ = registry.kill(fake_pid);

        // Callback should have been called
        assert!(called.load(Ordering::SeqCst));
        // Process should be unregistered regardless
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_kill_without_callback_uses_sigterm() {
        let registry = ProcessRegistry::new();
        let fake_pid = 999_999_998;
        let guard = registry.register(fake_pid, "test-basic".to_string(), ProcessKind::Bash);
        // Defuse — kill() will unregister internally
        guard.defuse();

        // kill() should try SIGTERM directly (will fail for fake PID, that's OK)
        let _ = registry.kill(fake_pid);
        assert!(registry.list().is_empty());
    }

    // ── ProcessGuard tests ──────────────────────────────────────────────

    #[test]
    fn test_guard_drop_unregisters() {
        let _test_guard = registry_test_guard();
        // Use global_registry so guard drop actually unregisters the right registry
        let pid = 888_888_001;
        let guard = global_registry().register(pid, "guard-test".to_string(), ProcessKind::Bash);
        assert!(global_registry().list().iter().any(|p| p.pid == pid));

        // Drop the guard → should auto-unregister
        drop(guard);
        assert!(!global_registry().list().iter().any(|p| p.pid == pid));
    }

    #[test]
    fn test_guard_defuse_skips_unregister() {
        let _test_guard = registry_test_guard();
        let pid = 888_888_002;
        let guard = global_registry().register(pid, "defuse-test".to_string(), ProcessKind::Bash);
        assert!(global_registry().list().iter().any(|p| p.pid == pid));

        guard.defuse();
        drop(guard);

        // Should still be registered because guard was defused
        assert!(global_registry().list().iter().any(|p| p.pid == pid));

        // Manual cleanup
        global_registry().unregister(pid);
    }

    // ── Reaper tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_reaper_cleans_dead_pids() {
        let _test_guard = registry_test_guard();
        // Register a PID that definitely doesn't exist
        let dead_pid = 999_777_001;
        let guard =
            global_registry().register(dead_pid, "reaper-test".to_string(), ProcessKind::Bash);
        // Defuse so guard drop doesn't interfere with reaper cleanup
        guard.defuse();

        assert!(global_registry().list().iter().any(|p| p.pid == dead_pid));

        // Spawn reaper with 100ms interval (fast for testing)
        global_registry().spawn_reaper(std::time::Duration::from_millis(100));

        // Wait for reaper to run at least once
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Dead PID should be cleaned up
        assert!(
            !global_registry().list().iter().any(|p| p.pid == dead_pid),
            "Reaper should have cleaned up the dead PID"
        );
    }
}
