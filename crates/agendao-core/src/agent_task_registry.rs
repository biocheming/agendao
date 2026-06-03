//! Global registry for tracking agent task lifecycle.
//!
//! Each agent task dispatched via TaskTool is registered here with a cancel
//! callback, step progress, and ring-buffered output. TUI and HTTP API
//! consume this registry to list, inspect, and cancel tasks.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::RwLock;

/// Global singleton agent task registry.
static TASK_REGISTRY: Lazy<AgentTaskRegistry> = Lazy::new(AgentTaskRegistry::new);

/// Returns a reference to the global [`AgentTaskRegistry`].
pub fn global_task_registry() -> &'static AgentTaskRegistry {
    &TASK_REGISTRY
}

const OUTPUT_TAIL_CAPACITY: usize = 50;
/// Cleanup fires when finished tasks exceed this count, keeping only CLEANUP_KEEP.
/// Set equal to CLEANUP_KEEP so the finished count never exceeds CLEANUP_KEEP + 1.
const CLEANUP_THRESHOLD: usize = 50;
const CLEANUP_KEEP: usize = 50;

#[derive(Debug, Clone)]
pub enum AgentTaskStatus {
    Pending,
    Running { step: u32 },
    Completed { steps: u32 },
    Cancelled,
    Failed { error: String },
}

impl AgentTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Cancelled | Self::Failed { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub struct AgentTask {
    pub id: String,
    pub session_id: Option<String>,
    pub agent_name: String,
    pub prompt: String,
    pub status: AgentTaskStatus,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub max_steps: Option<u32>,
    pub output_tail: VecDeque<String>,
}

pub struct AgentTaskRegistry {
    tasks: RwLock<HashMap<String, AgentTask>>,
    next_id: AtomicU32,
    cancel_callbacks: RwLock<HashMap<String, Arc<dyn Fn() + Send + Sync>>>,
}

impl AgentTaskRegistry {
    fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            next_id: AtomicU32::new(1),
            cancel_callbacks: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new agent task. Returns a short ID like "a1", "a2", etc.
    pub fn register(
        &self,
        session_id: Option<String>,
        agent_name: String,
        prompt: String,
        max_steps: Option<u32>,
        on_cancel: Arc<dyn Fn() + Send + Sync>,
    ) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("a{}", seq);
        let task = AgentTask {
            id: id.clone(),
            session_id,
            agent_name,
            prompt,
            status: AgentTaskStatus::Running { step: 0 },
            started_at: chrono::Utc::now().timestamp(),
            finished_at: None,
            max_steps,
            output_tail: VecDeque::with_capacity(OUTPUT_TAIL_CAPACITY),
        };
        self.tasks.write().insert(id.clone(), task);
        self.cancel_callbacks.write().insert(id.clone(), on_cancel);
        id
    }

    /// Update the current step number for a running task.
    pub fn update_step(&self, id: &str, step: u32) {
        if let Some(task) = self.tasks.write().get_mut(id) {
            if !task.status.is_terminal() {
                task.status = AgentTaskStatus::Running { step };
            }
        }
    }

    /// Append a line to the task's output ring buffer.
    pub fn append_output(&self, id: &str, line: String) {
        if let Some(task) = self.tasks.write().get_mut(id) {
            if task.output_tail.len() >= OUTPUT_TAIL_CAPACITY {
                task.output_tail.pop_front();
            }
            task.output_tail.push_back(line);
        }
    }

    /// Mark a task as completed, cancelled, or failed. Idempotent — first call wins.
    pub fn complete(&self, id: &str, status: AgentTaskStatus) {
        let mut tasks = self.tasks.write();
        if let Some(task) = tasks.get_mut(id) {
            if !task.status.is_terminal() {
                task.status = status;
                task.finished_at = Some(chrono::Utc::now().timestamp());
            }
        }
        self.cancel_callbacks.write().remove(id);
        // Auto-cleanup old finished tasks.
        self.cleanup_finished(&mut tasks);
    }

    /// Cancel a running task by invoking its cancel callback.
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        // Check if task exists and is still running.
        {
            let tasks = self.tasks.read();
            match tasks.get(id) {
                None => return Err(format!("Task \"{}\" not found", id)),
                Some(task) if task.status.is_terminal() => {
                    return Err(format!("Task \"{}\" is already finished", id));
                }
                _ => {}
            }
        }
        // Fire the cancel callback.
        if let Some(cb) = self.cancel_callbacks.read().get(id) {
            let cb = cb.clone();
            cb();
        }
        // Mark as cancelled.
        self.complete(id, AgentTaskStatus::Cancelled);
        Ok(())
    }

    /// List all tasks (running first, then finished by recency).
    pub fn list(&self) -> Vec<AgentTask> {
        let tasks = self.tasks.read();
        let mut result: Vec<AgentTask> = tasks.values().cloned().collect();
        result.sort_by(|a, b| {
            let a_terminal = a.status.is_terminal() as u8;
            let b_terminal = b.status.is_terminal() as u8;
            a_terminal
                .cmp(&b_terminal)
                .then(b.started_at.cmp(&a.started_at))
        });
        result
    }

    /// Get a single task by ID.
    pub fn get(&self, id: &str) -> Option<AgentTask> {
        self.tasks.read().get(id).cloned()
    }

    /// Remove oldest finished tasks when count exceeds threshold.
    fn cleanup_finished(&self, tasks: &mut HashMap<String, AgentTask>) {
        let finished: Vec<String> = tasks
            .iter()
            .filter(|(_, t)| t.status.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
        if finished.len() > CLEANUP_THRESHOLD {
            let mut by_time: Vec<(String, i64)> = finished
                .into_iter()
                .map(|id| {
                    let t = tasks[&id].finished_at.unwrap_or(0);
                    (id, t)
                })
                .collect();
            by_time.sort_by_key(|(_, t)| *t);
            let to_remove = by_time.len() - CLEANUP_KEEP;
            for (id, _) in by_time.into_iter().take(to_remove) {
                tasks.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn mock_cancel_token() -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }

    #[test]
    fn test_register_and_list() {
        let registry = AgentTaskRegistry::new();
        let id = registry.register(
            None,
            "explore".to_string(),
            "Analyze auth".to_string(),
            Some(10),
            mock_cancel_token(),
        );
        let tasks = registry.list();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].agent_name, "explore");
        assert!(id.starts_with('a'));
    }

    #[test]
    fn test_update_step() {
        let registry = AgentTaskRegistry::new();
        let id = registry.register(
            None,
            "explore".to_string(),
            "test".to_string(),
            None,
            mock_cancel_token(),
        );
        registry.update_step(&id, 3);
        let task = registry.get(&id).unwrap();
        assert!(matches!(task.status, AgentTaskStatus::Running { step: 3 }));
    }

    #[test]
    fn test_append_output_ring_buffer() {
        let registry = AgentTaskRegistry::new();
        let id = registry.register(
            None,
            "explore".to_string(),
            "test".to_string(),
            None,
            mock_cancel_token(),
        );
        for i in 0..60 {
            registry.append_output(&id, format!("line {}", i));
        }
        let task = registry.get(&id).unwrap();
        assert_eq!(task.output_tail.len(), 50);
        assert_eq!(task.output_tail[0], "line 10");
        assert_eq!(task.output_tail[49], "line 59");
    }

    #[test]
    fn test_complete_marks_finished() {
        let registry = AgentTaskRegistry::new();
        let id = registry.register(
            None,
            "explore".to_string(),
            "test".to_string(),
            None,
            mock_cancel_token(),
        );
        registry.complete(&id, AgentTaskStatus::Completed { steps: 5 });
        let task = registry.get(&id).unwrap();
        assert!(matches!(
            task.status,
            AgentTaskStatus::Completed { steps: 5 }
        ));
        assert!(task.finished_at.is_some());
    }

    #[test]
    fn test_cancel_sets_cancelled() {
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        let registry = AgentTaskRegistry::new();
        let id = registry.register(
            None,
            "explore".to_string(),
            "test".to_string(),
            None,
            Arc::new(move || {
                called_clone.store(true, Ordering::SeqCst);
            }),
        );
        let result = registry.cancel(&id);
        assert!(result.is_ok());
        assert!(called.load(Ordering::SeqCst));
        let task = registry.get(&id).unwrap();
        assert!(matches!(task.status, AgentTaskStatus::Cancelled));
    }

    #[test]
    fn test_cancel_already_done_errors() {
        let registry = AgentTaskRegistry::new();
        let id = registry.register(
            None,
            "explore".to_string(),
            "test".to_string(),
            None,
            mock_cancel_token(),
        );
        registry.complete(&id, AgentTaskStatus::Completed { steps: 5 });
        let result = registry.cancel(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_auto_cleanup_old_tasks() {
        let registry = AgentTaskRegistry::new();
        for _ in 0..110 {
            let id = registry.register(
                None,
                "agent".to_string(),
                "test".to_string(),
                None,
                mock_cancel_token(),
            );
            registry.complete(&id, AgentTaskStatus::Completed { steps: 1 });
        }
        let tasks = registry.list();
        let finished = tasks
            .iter()
            .filter(|t| matches!(t.status, AgentTaskStatus::Completed { .. }))
            .count();
        assert!(finished <= 50);
    }
}
