//! State batching system for grouping multiple state updates.
//!
//! This module provides React-like state batching, where multiple state updates
//! within the same event handler are batched into a single re-render.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::scheduler::batch::{begin_batch, end_batch, queue_update};
//!
//! // Start batching
//! begin_batch();
//!
//! // Multiple updates are queued, not applied immediately
//! queue_update(fiber_id, StateUpdate { hook_index: 0, update: StateUpdateKind::Value(Box::new(1)) });
//! queue_update(fiber_id, StateUpdate { hook_index: 0, update: StateUpdateKind::Value(Box::new(2)) });
//!
//! // End batching - applies all updates and returns dirty fibers
//! let dirty = end_batch(&mut tree);
//! ```

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use crate::fiber::FiberId;
use crate::fiber_tree::FiberTree;

thread_local! {
    /// Thread-local state batch for the current render context
    static STATE_BATCH: RefCell<StateBatch> = RefCell::new(StateBatch::new());
}

// ============================================================================
// Cross-thread update infrastructure
// ============================================================================

/// Global queue for cross-thread state updates.
/// Background tasks (from use_interval, use_timeout) write to this queue,
/// and the main render loop drains it into the thread-local batch.
static CROSS_THREAD_UPDATES: Mutex<Vec<CrossThreadUpdate>> = Mutex::new(Vec::new());

/// Thread ID of the main render thread (set during render initialization).
/// Uses RwLock instead of OnceLock to allow resetting in tests.
static MAIN_THREAD_ID: std::sync::RwLock<Option<ThreadId>> = std::sync::RwLock::new(None);

/// A state update that can be sent across threads.
/// This is the cross-thread equivalent of StateUpdate.
pub struct CrossThreadUpdate {
    /// The fiber to update
    pub fiber_id: FiberId,
    /// Index of the hook being updated
    pub hook_index: usize,
    /// The update to apply
    pub update: CrossThreadUpdateKind,
}

/// Thread-safe update kinds for cross-thread state updates.
/// Uses Arc for functional updaters to allow sharing across threads.
pub enum CrossThreadUpdateKind {
    /// Direct value replacement (thread-safe via Box<dyn Any + Send>)
    Value(Box<dyn Any + Send>),
    /// Functional update (thread-safe via Arc wrapper around Mutex<Option<FnOnce>>)
    /// The Mutex<Option<>> pattern allows FnOnce to be called from an Fn context
    Updater(Arc<Mutex<Option<StateUpdaterFn>>>),
    /// Value replacement with equality check - skips update if values are equal
    /// Includes TypeId for runtime type checking when reconstructing equality check
    ValueIfChanged {
        /// The new value to set
        value: Box<dyn Any + Send>,
        /// TypeId of the value for runtime type checking
        type_id: std::any::TypeId,
    },
    /// Functional update with equality check - skips marking dirty if result equals current
    /// The TypeId is extracted from the updater's result when it's first called
    UpdaterIfChanged {
        /// The updater function wrapped in Arc<Mutex<Option<>>> for thread-safety
        updater: Arc<Mutex<Option<StateUpdaterFn>>>,
    },
}

// ============================================================================
// Thread-local batch types
// ============================================================================

/// Type alias for functional updater to reduce complexity
pub type StateUpdaterFn = Box<dyn FnOnce(&dyn Any) -> Box<dyn Any + Send> + Send>;

/// Type alias for equality check function
pub type EqualityCheckFn = Box<dyn FnOnce(&dyn Any, &dyn Any) -> bool + Send>;

/// A pending state update
pub struct StateUpdate {
    /// Index of the hook being updated
    pub hook_index: usize,
    /// The update to apply
    pub update: StateUpdateKind,
}

/// Kind of state update
pub enum StateUpdateKind {
    /// Direct value replacement
    Value(Box<dyn Any + Send>),
    /// Functional update that receives current state and returns new state
    Updater(StateUpdaterFn),
    /// Value replacement with equality check - skips update if values are equal
    ValueIfChanged {
        /// The new value to set
        value: Box<dyn Any + Send>,
        /// Function to check if old and new values are equal
        eq_check: EqualityCheckFn,
    },
    /// Functional update with equality check - skips marking dirty if result equals current
    UpdaterIfChanged {
        /// The updater function
        updater: StateUpdaterFn,
        /// Function to check if old and new values are equal
        eq_check: EqualityCheckFn,
    },
}

/// Tracks pending state updates for batching
///
/// State batching allows multiple state updates within the same event handler
/// to be combined into a single re-render, improving performance and ensuring
/// consistent state transitions.
pub struct StateBatch {
    /// Pending updates grouped by fiber
    updates: HashMap<FiberId, Vec<StateUpdate>>,
    /// Whether we're currently in a batch context
    batching: bool,
    /// Fibers that need re-render
    dirty_fibers: HashSet<FiberId>,
}

impl StateBatch {
    /// Create a new empty batch
    pub fn new() -> Self {
        Self {
            updates: HashMap::new(),
            batching: false,
            dirty_fibers: HashSet::new(),
        }
    }

    /// Begin a batch context
    ///
    /// While batching is active, state updates are queued rather than
    /// applied immediately. Call `end_batch` to apply all queued updates.
    pub fn begin_batch(&mut self) {
        self.batching = true;
    }

    /// End the batch context, apply all pending updates, and return dirty fibers
    ///
    /// This method:
    /// 1. Sets batching to false
    /// 2. Applies all pending state updates to the fiber tree
    /// 3. Returns the set of fibers that need re-rendering
    ///
    /// # Arguments
    ///
    /// * `tree` - The fiber tree to apply updates to
    ///
    /// # Returns
    ///
    /// A set of fiber IDs that have been modified and need re-rendering
    pub fn end_batch(&mut self, tree: &mut FiberTree) -> HashSet<FiberId> {
        self.batching = false;

        // Track which fibers actually had state changes
        let mut actually_dirty: HashSet<FiberId> = HashSet::new();

        // Apply all pending updates to fibers
        for (fiber_id, updates) in self.updates.drain() {
            if let Some(fiber) = tree.get_mut(fiber_id) {
                let mut fiber_changed = false;

                for update in updates {
                    // Ensure hooks vector is large enough
                    if update.hook_index >= fiber.hooks.len() {
                        fiber
                            .hooks
                            .resize_with(update.hook_index + 1, || Box::new(()));
                    }

                    match update.update {
                        StateUpdateKind::Value(value) => {
                            // Directly assign the boxed value
                            fiber.hooks[update.hook_index] = value;
                            fiber_changed = true;
                        }
                        StateUpdateKind::Updater(updater) => {
                            // Get current value, apply updater, set new value
                            let current = &*fiber.hooks[update.hook_index];
                            let new_value = updater(current);
                            fiber.hooks[update.hook_index] = new_value;
                            fiber_changed = true;
                        }
                        StateUpdateKind::ValueIfChanged { value, eq_check } => {
                            // Check if values are equal before updating
                            let current = &*fiber.hooks[update.hook_index];
                            if !eq_check(current, &*value) {
                                fiber.hooks[update.hook_index] = value;
                                fiber_changed = true;
                            }
                        }
                        StateUpdateKind::UpdaterIfChanged { updater, eq_check } => {
                            // Apply updater and check if result differs
                            let current = &*fiber.hooks[update.hook_index];
                            let new_value = updater(current);
                            if !eq_check(&*fiber.hooks[update.hook_index], &*new_value) {
                                fiber.hooks[update.hook_index] = new_value;
                                fiber_changed = true;
                            }
                        }
                    }
                }

                // Only mark fiber as dirty if state actually changed
                if fiber_changed {
                    fiber.dirty = true;
                    actually_dirty.insert(fiber_id);
                }
            }
        }

        // Clear the dirty_fibers set and return only actually dirty fibers
        self.dirty_fibers.clear();
        actually_dirty
    }

    /// Queue a state update
    ///
    /// The update will be applied when `end_batch` is called.
    /// The fiber is immediately marked as dirty.
    pub fn queue_update(&mut self, fiber_id: FiberId, update: StateUpdate) {
        self.updates.entry(fiber_id).or_default().push(update);
        self.dirty_fibers.insert(fiber_id);
    }

    /// Check if currently batching
    pub fn is_batching(&self) -> bool {
        self.batching
    }

    /// Take all pending updates for a fiber
    pub fn take_updates(&mut self, fiber_id: FiberId) -> Vec<StateUpdate> {
        self.updates.remove(&fiber_id).unwrap_or_default()
    }

    /// Check if there are pending updates
    pub fn has_pending_updates(&self) -> bool {
        !self.updates.is_empty()
    }

    /// Get the number of dirty fibers
    pub fn dirty_fiber_count(&self) -> usize {
        self.dirty_fibers.len()
    }

    /// Check if a specific fiber is dirty
    pub fn is_fiber_dirty(&self, fiber_id: FiberId) -> bool {
        self.dirty_fibers.contains(&fiber_id)
    }

    /// Clear all pending updates without applying them
    pub fn clear(&mut self) {
        self.updates.clear();
        self.dirty_fibers.clear();
        self.batching = false;
    }
}

impl Default for StateBatch {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Cross-thread API functions
// ============================================================================

/// Initialize the main thread ID.
/// This should be called once at the start of the render loop.
/// After this is called, `is_main_thread()` will return true only on this thread.
pub fn init_main_thread() {
    if let Ok(mut guard) = MAIN_THREAD_ID.write() {
        *guard = Some(std::thread::current().id());
    }
}

/// Check if the current thread is the main render thread.
///
/// Returns `true` if:
/// - `init_main_thread()` has been called and we're on that thread, OR
/// - `init_main_thread()` has NOT been called (backward compatibility - assume main thread)
///
/// Returns `false` only if `init_main_thread()` was called on a different thread.
pub fn is_main_thread() -> bool {
    if let Ok(guard) = MAIN_THREAD_ID.read() {
        match *guard {
            Some(id) => id == std::thread::current().id(),
            None => true, // If not initialized, assume we're on main thread (backward compat)
        }
    } else {
        true // If lock is poisoned, assume main thread to avoid breaking things
    }
}

/// Reset the main thread ID (for testing purposes only).
/// This allows tests to re-initialize the main thread tracking.
#[cfg(test)]
pub fn reset_main_thread() {
    if let Ok(mut guard) = MAIN_THREAD_ID.write() {
        *guard = None;
    }
}

/// Queue a cross-thread update to the global queue.
/// This is called from background threads when they need to update state.
pub fn queue_cross_thread_update(update: CrossThreadUpdate) {
    if let Ok(mut queue) = CROSS_THREAD_UPDATES.lock() {
        queue.push(update);
    } else {
        // Mutex is poisoned - log error but don't panic
        tracing::error!("Cross-thread update queue mutex is poisoned, update dropped");
    }
}

/// Drain all cross-thread updates into the thread-local batch.
/// This should be called from the main render thread before `end_batch()`.
pub fn drain_cross_thread_updates() {
    if let Ok(mut queue) = CROSS_THREAD_UPDATES.lock() {
        for update in queue.drain(..) {
            // Convert CrossThreadUpdateKind to StateUpdateKind
            let state_update = StateUpdate {
                hook_index: update.hook_index,
                update: match update.update {
                    CrossThreadUpdateKind::Value(v) => StateUpdateKind::Value(v),
                    CrossThreadUpdateKind::Updater(arc_mutex) => {
                        // Extract the FnOnce from the Arc<Mutex<Option<>>>
                        StateUpdateKind::Updater(Box::new(move |any| {
                            if let Ok(mut guard) = arc_mutex.lock() {
                                if let Some(f) = guard.take() {
                                    f(any)
                                } else {
                                    // Updater already called - this shouldn't happen
                                    panic!("Cross-thread updater called more than once");
                                }
                            } else {
                                panic!("Cross-thread updater mutex poisoned");
                            }
                        }))
                    }
                    CrossThreadUpdateKind::ValueIfChanged { value, type_id } => {
                        // Reconstruct equality check using TypeId for runtime type checking
                        StateUpdateKind::ValueIfChanged {
                            value,
                            eq_check: Box::new(move |old, new| {
                                // Use TypeId to ensure we're comparing the right types
                                // This is a generic equality check that works for any PartialEq type
                                reconstruct_equality_check(old, new, type_id)
                            }),
                        }
                    }
                    CrossThreadUpdateKind::UpdaterIfChanged { updater } => {
                        // For UpdaterIfChanged, we need to extract the TypeId from the result
                        // We do this by wrapping the updater and extracting the type on first call
                        StateUpdateKind::UpdaterIfChanged {
                            updater: Box::new(move |any| {
                                if let Ok(mut guard) = updater.lock() {
                                    if let Some(f) = guard.take() {
                                        f(any)
                                    } else {
                                        panic!("Cross-thread updater called more than once");
                                    }
                                } else {
                                    panic!("Cross-thread updater mutex poisoned");
                                }
                            }),
                            eq_check: Box::new(move |old, new| {
                                // Extract TypeId from the values and use it for comparison
                                // Both old and new should have the same type
                                let type_id = old.type_id();
                                reconstruct_equality_check(old, new, type_id)
                            }),
                        }
                    }
                },
            };
            // Queue to thread-local batch (we're on main thread now)
            STATE_BATCH.with(|batch| {
                batch
                    .borrow_mut()
                    .queue_update(update.fiber_id, state_update);
            });
        }
    } else {
        tracing::error!("Cross-thread update queue mutex is poisoned, updates not drained");
    }
}

/// Helper function to reconstruct equality checks for cross-thread updates.
///
/// This function attempts to downcast the values to common types and compare them.
/// If the type is not recognized, it conservatively returns false (values are different).
///
/// # Arguments
///
/// * `old` - The old value
/// * `new` - The new value
/// * `type_id` - The TypeId of the values for runtime type checking
///
/// # Returns
///
/// `true` if the values are equal, `false` otherwise
fn reconstruct_equality_check(old: &dyn Any, new: &dyn Any, type_id: std::any::TypeId) -> bool {
    // Try common types that implement PartialEq
    // This is a best-effort approach - we can't reconstruct the exact equality check
    // but we can handle common cases

    // Check TypeId matches
    if old.type_id() != type_id || new.type_id() != type_id {
        return false;
    }

    // Try common integer types
    if type_id == std::any::TypeId::of::<i32>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<i32>(), new.downcast_ref::<i32>())
        {
            return old_val == new_val;
        }
    }
    if type_id == std::any::TypeId::of::<i64>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<i64>(), new.downcast_ref::<i64>())
        {
            return old_val == new_val;
        }
    }
    if type_id == std::any::TypeId::of::<u32>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<u32>(), new.downcast_ref::<u32>())
        {
            return old_val == new_val;
        }
    }
    if type_id == std::any::TypeId::of::<u64>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<u64>(), new.downcast_ref::<u64>())
        {
            return old_val == new_val;
        }
    }
    if type_id == std::any::TypeId::of::<usize>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<usize>(), new.downcast_ref::<usize>())
        {
            return old_val == new_val;
        }
    }

    // Try floating point types
    if type_id == std::any::TypeId::of::<f32>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<f32>(), new.downcast_ref::<f32>())
        {
            return old_val == new_val;
        }
    }
    if type_id == std::any::TypeId::of::<f64>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<f64>(), new.downcast_ref::<f64>())
        {
            return old_val == new_val;
        }
    }

    // Try boolean
    if type_id == std::any::TypeId::of::<bool>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<bool>(), new.downcast_ref::<bool>())
        {
            return old_val == new_val;
        }
    }

    // Try String
    if type_id == std::any::TypeId::of::<String>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<String>(), new.downcast_ref::<String>())
        {
            return old_val == new_val;
        }
    }

    // Try &str (though this is less common in state)
    if type_id == std::any::TypeId::of::<&str>() {
        if let (Some(old_val), Some(new_val)) =
            (old.downcast_ref::<&str>(), new.downcast_ref::<&str>())
        {
            return old_val == new_val;
        }
    }

    // For unknown types, conservatively return false (assume values are different)
    // This means the optimization is lost for custom types, but correctness is preserved
    false
}

/// Check if there are pending cross-thread updates.
pub fn has_cross_thread_updates() -> bool {
    CROSS_THREAD_UPDATES
        .lock()
        .map(|queue| !queue.is_empty())
        .unwrap_or(false)
}

/// Clear all cross-thread updates (for testing purposes).
#[cfg(test)]
pub fn clear_cross_thread_updates() {
    if let Ok(mut queue) = CROSS_THREAD_UPDATES.lock() {
        queue.clear();
    }
}

/// Test helper: Simulate a re-entrant update by holding STATE_BATCH borrow
/// while calling queue_update. This forces the update to go to the cross-thread queue.
#[cfg(test)]
pub fn test_simulate_reentrant_update(
    fiber_id: FiberId,
    hook_index: usize,
    value: Box<dyn Any + Send>,
) {
    STATE_BATCH.with(|batch| {
        let _guard = batch.borrow_mut(); // Hold the borrow

        // This should fall back to cross-thread queue
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index,
                update: StateUpdateKind::Value(value),
            },
        );
    });
}

// ============================================================================
// Thread-local API functions
// ============================================================================

/// Begin a batch context on the thread-local state batch
///
/// While batching is active, state updates are queued rather than
/// applied immediately.
pub fn begin_batch() {
    STATE_BATCH.with(|batch| {
        batch.borrow_mut().begin_batch();
    });
}

/// End the batch context and apply all pending updates using a provided tree
///
/// # Arguments
///
/// * `tree` - The fiber tree to apply updates to
///
/// # Returns
///
/// A set of fiber IDs that have been modified and need re-rendering
pub fn end_batch_with_tree(tree: &mut FiberTree) -> HashSet<FiberId> {
    STATE_BATCH.with(|batch| batch.borrow_mut().end_batch(tree))
}

/// End the batch context and apply all pending updates using the thread-local fiber tree
///
/// # Returns
///
/// A set of fiber IDs that have been modified and need re-rendering
pub fn end_batch() -> HashSet<FiberId> {
    crate::fiber_tree::with_fiber_tree_mut(|tree| {
        STATE_BATCH.with(|batch| batch.borrow_mut().end_batch(tree))
    })
    .unwrap_or_default()
}

/// Queue a state update - automatically routes to correct queue based on thread.
///
/// If called from the main render thread (after `init_main_thread()`), the update
/// is queued to the thread-local batch for immediate batching.
///
/// If called from a background thread (e.g., from a tokio::spawn task in use_interval),
/// the update is queued to the global cross-thread queue, which will be drained
/// into the thread-local batch on the next render frame.
///
/// # Arguments
///
/// * `fiber_id` - The fiber to update
/// * `update` - The state update to queue
pub fn queue_update(fiber_id: FiberId, update: StateUpdate) {
    // Warn if state update is queued during render phase (development mode only)
    #[cfg(debug_assertions)]
    {
        if crate::runtime::is_in_render_phase() {
            tracing::warn!(
                fiber_id = ?fiber_id,
                hook_index = update.hook_index,
                "State update queued during render phase! State updates should be triggered by events or effects, not during render. \
                This can lead to infinite render loops and performance issues."
            );
        }
    }

    // First check if we're on the main thread
    if !is_main_thread() {
        // Definitely a background thread - use cross-thread queue
        queue_update_to_cross_thread(fiber_id, update);
        return;
    }

    // We're on the main thread, but we might be in a spawned task that's
    // running on the main thread (tokio work-stealing). Try to borrow
    // STATE_BATCH - if it fails, we're in a re-entrant situation and
    // should use the cross-thread queue instead.
    //
    // We use a Cell to pass the update out if try_borrow_mut fails.
    let fallback_update: std::cell::Cell<Option<StateUpdate>> = std::cell::Cell::new(None);

    let queued = STATE_BATCH.with(|batch| {
        match batch.try_borrow_mut() {
            Ok(mut b) => {
                b.queue_update(fiber_id, update);
                true
            }
            Err(_) => {
                // STATE_BATCH is already borrowed - we're in a re-entrant call
                // This can happen when a tokio task runs on the main thread
                // during drain_cross_thread_updates or end_batch
                fallback_update.set(Some(update));
                false
            }
        }
    });

    if !queued {
        // Couldn't borrow STATE_BATCH, use cross-thread queue
        if let Some(update) = fallback_update.take() {
            queue_update_to_cross_thread(fiber_id, update);
        }
    }
}

/// Internal function to queue an update to the cross-thread queue
fn queue_update_to_cross_thread(fiber_id: FiberId, update: StateUpdate) {
    let cross_update = CrossThreadUpdate {
        fiber_id,
        hook_index: update.hook_index,
        update: match update.update {
            StateUpdateKind::Value(v) => CrossThreadUpdateKind::Value(v),
            StateUpdateKind::Updater(f) => {
                // Wrap FnOnce in Arc<Mutex<Option<>>> for thread-safety
                CrossThreadUpdateKind::Updater(Arc::new(Mutex::new(Some(f))))
            }
            StateUpdateKind::ValueIfChanged { value, eq_check: _ } => {
                // Preserve type information for equality check reconstruction
                let type_id = (*value).type_id();
                CrossThreadUpdateKind::ValueIfChanged { value, type_id }
            }
            StateUpdateKind::UpdaterIfChanged {
                updater,
                eq_check: _,
            } => {
                // For UpdaterIfChanged, we can't extract the TypeId until the updater runs
                // The equality check will extract it from the result values at runtime
                CrossThreadUpdateKind::UpdaterIfChanged {
                    updater: Arc::new(Mutex::new(Some(updater))),
                }
            }
        },
    };
    queue_cross_thread_update(cross_update);
}

/// Check if currently batching on the thread-local state batch
pub fn is_batching() -> bool {
    STATE_BATCH.with(|batch| batch.borrow().is_batching())
}

/// Execute a closure with the thread-local state batch
pub fn with_state_batch<R, F: FnOnce(&StateBatch) -> R>(f: F) -> R {
    STATE_BATCH.with(|batch| f(&batch.borrow()))
}

/// Execute a closure with mutable access to the thread-local state batch
pub fn with_state_batch_mut<R, F: FnOnce(&mut StateBatch) -> R>(f: F) -> R {
    STATE_BATCH.with(|batch| f(&mut batch.borrow_mut()))
}

/// Clear the thread-local state batch
pub fn clear_state_batch() {
    STATE_BATCH.with(|batch| {
        batch.borrow_mut().clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_state_batch_creation() {
        let batch = StateBatch::new();
        assert!(!batch.is_batching());
        assert!(!batch.has_pending_updates());
        assert_eq!(batch.dirty_fiber_count(), 0);
    }

    #[test]
    fn test_begin_and_end_batch() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();

        assert!(!batch.is_batching());

        batch.begin_batch();
        assert!(batch.is_batching());

        let dirty = batch.end_batch(&mut tree);
        assert!(!batch.is_batching());
        assert!(dirty.is_empty());
    }

    #[test]
    fn test_queue_update_marks_fiber_dirty() {
        let mut batch = StateBatch::new();
        let fiber_id = FiberId(1);

        assert!(!batch.is_fiber_dirty(fiber_id));

        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        assert!(batch.is_fiber_dirty(fiber_id));
        assert!(batch.has_pending_updates());
        assert_eq!(batch.dirty_fiber_count(), 1);
    }

    #[test]
    fn test_multiple_updates_same_fiber() {
        let mut batch = StateBatch::new();
        let fiber_id = FiberId(1);

        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(1i32)),
            },
        );
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(2i32)),
            },
        );
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 1,
                update: StateUpdateKind::Value(Box::new("hello".to_string())),
            },
        );

        // Still only one dirty fiber
        assert_eq!(batch.dirty_fiber_count(), 1);
        assert!(batch.has_pending_updates());
    }

    #[test]
    fn test_multiple_fibers_dirty() {
        let mut batch = StateBatch::new();
        let fiber1 = FiberId(1);
        let fiber2 = FiberId(2);
        let fiber3 = FiberId(3);

        batch.queue_update(
            fiber1,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(1i32)),
            },
        );
        batch.queue_update(
            fiber2,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(2i32)),
            },
        );
        batch.queue_update(
            fiber3,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(3i32)),
            },
        );

        assert_eq!(batch.dirty_fiber_count(), 3);
        assert!(batch.is_fiber_dirty(fiber1));
        assert!(batch.is_fiber_dirty(fiber2));
        assert!(batch.is_fiber_dirty(fiber3));
    }

    #[test]
    fn test_end_batch_applies_value_updates() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        let dirty = batch.end_batch(&mut tree);

        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(42));
    }

    #[test]
    fn test_end_batch_applies_functional_updates() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 10
        tree.get_mut(fiber_id).unwrap().set_hook(0, 10i32);

        batch.begin_batch();

        // Queue functional update that adds 5
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Updater(Box::new(|current| {
                    let val = current.downcast_ref::<i32>().unwrap();
                    Box::new(val + 5)
                })),
            },
        );

        let dirty = batch.end_batch(&mut tree);

        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(15));
    }

    #[test]
    fn test_chained_functional_updates() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 0
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        batch.begin_batch();

        // Queue multiple functional updates
        for _ in 0..5 {
            batch.queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Updater(Box::new(|current| {
                        let val = current.downcast_ref::<i32>().unwrap();
                        Box::new(val + 1)
                    })),
                },
            );
        }

        batch.end_batch(&mut tree);

        // All 5 updates should have been applied
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(5));
    }

    #[test]
    fn test_take_updates() {
        let mut batch = StateBatch::new();
        let fiber_id = FiberId(1);

        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(1i32)),
            },
        );
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 1,
                update: StateUpdateKind::Value(Box::new(2i32)),
            },
        );

        let updates = batch.take_updates(fiber_id);
        assert_eq!(updates.len(), 2);

        // Updates should be removed
        let updates_again = batch.take_updates(fiber_id);
        assert!(updates_again.is_empty());
    }

    #[test]
    fn test_clear_batch() {
        let mut batch = StateBatch::new();
        let fiber_id = FiberId(1);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        assert!(batch.is_batching());
        assert!(batch.has_pending_updates());

        batch.clear();

        assert!(!batch.is_batching());
        assert!(!batch.has_pending_updates());
        assert_eq!(batch.dirty_fiber_count(), 0);
    }

    #[test]
    fn test_thread_local_begin_batch() {
        // Clear any existing state
        clear_state_batch();

        assert!(!is_batching());

        begin_batch();
        assert!(is_batching());

        // Clean up
        clear_state_batch();
    }

    #[test]
    #[serial]
    fn test_thread_local_queue_and_end_batch() {
        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        begin_batch();

        // Use with_state_batch_mut to directly queue to local batch
        // This avoids the routing logic in queue_update
        with_state_batch_mut(|batch| {
            batch.queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Value(Box::new(100i32)),
                },
            );
        });

        let dirty = end_batch_with_tree(&mut tree);

        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(100));

        clear_state_batch();
    }

    #[test]
    #[serial]
    fn test_with_state_batch() {
        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();

        let fiber_id = FiberId(1);

        // Use with_state_batch_mut to directly queue to local batch
        with_state_batch_mut(|batch| {
            batch.queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Value(Box::new(42i32)),
                },
            );
        });

        let has_updates = with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_updates);

        let is_dirty = with_state_batch(|batch| batch.is_fiber_dirty(fiber_id));
        assert!(is_dirty);

        clear_state_batch();
    }

    #[test]
    #[serial]
    fn test_with_state_batch_mut() {
        clear_state_batch();

        with_state_batch_mut(|batch| {
            batch.begin_batch();
        });

        assert!(is_batching());

        clear_state_batch();
    }

    #[test]
    fn test_end_batch_marks_fiber_dirty_in_tree() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Mark fiber as clean initially
        tree.mark_clean(fiber_id);
        assert!(!tree.get(fiber_id).unwrap().dirty);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        batch.end_batch(&mut tree);

        // Fiber should be marked dirty after state update
        assert!(tree.get(fiber_id).unwrap().dirty);
    }

    #[test]
    fn test_update_nonexistent_fiber() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let nonexistent_fiber = FiberId(999);

        batch.begin_batch();
        batch.queue_update(
            nonexistent_fiber,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        // Should not panic, just skip the update
        let dirty = batch.end_batch(&mut tree);

        // The fiber is NOT in dirty set because it doesn't exist in the tree
        assert!(!dirty.contains(&nonexistent_fiber));
    }

    #[test]
    fn test_default_impl() {
        let batch: StateBatch = Default::default();
        assert!(!batch.is_batching());
        assert!(!batch.has_pending_updates());
    }

    #[test]
    fn test_value_if_changed_skips_equal_values() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 42
        tree.get_mut(fiber_id).unwrap().set_hook(0, 42i32);
        tree.mark_clean(fiber_id);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::ValueIfChanged {
                    value: Box::new(42i32), // Same value
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<i32>().unwrap();
                        let new = new.downcast_ref::<i32>().unwrap();
                        old == new
                    }),
                },
            },
        );

        let dirty = batch.end_batch(&mut tree);

        // Fiber should NOT be dirty because value didn't change
        assert!(!dirty.contains(&fiber_id));
        assert!(!tree.get(fiber_id).unwrap().dirty);
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(42));
    }

    #[test]
    fn test_value_if_changed_updates_different_values() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 42
        tree.get_mut(fiber_id).unwrap().set_hook(0, 42i32);
        tree.mark_clean(fiber_id);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::ValueIfChanged {
                    value: Box::new(100i32), // Different value
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<i32>().unwrap();
                        let new = new.downcast_ref::<i32>().unwrap();
                        old == new
                    }),
                },
            },
        );

        let dirty = batch.end_batch(&mut tree);

        // Fiber SHOULD be dirty because value changed
        assert!(dirty.contains(&fiber_id));
        assert!(tree.get(fiber_id).unwrap().dirty);
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(100));
    }

    #[test]
    fn test_updater_if_changed_skips_equal_results() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 5
        tree.get_mut(fiber_id).unwrap().set_hook(0, 5i32);
        tree.mark_clean(fiber_id);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::UpdaterIfChanged {
                    updater: Box::new(|any| {
                        let n = any.downcast_ref::<i32>().unwrap();
                        Box::new((*n).max(3)) // 5.max(3) = 5, no change
                    }),
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<i32>().unwrap();
                        let new = new.downcast_ref::<i32>().unwrap();
                        old == new
                    }),
                },
            },
        );

        let dirty = batch.end_batch(&mut tree);

        // Fiber should NOT be dirty because result equals current
        assert!(!dirty.contains(&fiber_id));
        assert!(!tree.get(fiber_id).unwrap().dirty);
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(5));
    }

    #[test]
    fn test_updater_if_changed_updates_different_results() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 5
        tree.get_mut(fiber_id).unwrap().set_hook(0, 5i32);
        tree.mark_clean(fiber_id);

        batch.begin_batch();
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::UpdaterIfChanged {
                    updater: Box::new(|any| {
                        let n = any.downcast_ref::<i32>().unwrap();
                        Box::new((*n).max(10)) // 5.max(10) = 10, changed!
                    }),
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<i32>().unwrap();
                        let new = new.downcast_ref::<i32>().unwrap();
                        old == new
                    }),
                },
            },
        );

        let dirty = batch.end_batch(&mut tree);

        // Fiber SHOULD be dirty because result differs
        assert!(dirty.contains(&fiber_id));
        assert!(tree.get(fiber_id).unwrap().dirty);
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(10));
    }

    #[test]
    fn test_mixed_updates_with_equality_check() {
        let mut batch = StateBatch::new();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Initialize hook with value 10
        tree.get_mut(fiber_id).unwrap().set_hook(0, 10i32);
        tree.mark_clean(fiber_id);

        batch.begin_batch();

        // First: regular update (always marks dirty)
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(20i32)),
            },
        );

        // Second: equality-checked update with same value (should not add to dirty)
        batch.queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::ValueIfChanged {
                    value: Box::new(20i32), // Same as previous update
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<i32>().unwrap();
                        let new = new.downcast_ref::<i32>().unwrap();
                        old == new
                    }),
                },
            },
        );

        let dirty = batch.end_batch(&mut tree);

        // Fiber should be dirty because first update changed it
        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(20));
    }

    // ========================================================================
    // Cross-thread update tests
    // ========================================================================

    #[test]
    #[serial]
    fn test_is_main_thread_without_init() {
        // Reset to ensure clean state
        reset_main_thread();

        // Without init_main_thread(), is_main_thread() should return true
        // (backward compatibility - assume main thread if not explicitly set)
        assert!(is_main_thread());
    }

    #[test]
    #[serial]
    fn test_init_main_thread_and_is_main_thread() {
        // Reset to ensure clean state
        reset_main_thread();

        // Initialize main thread
        init_main_thread();

        // Should return true on the same thread
        assert!(is_main_thread());

        // Clean up
        reset_main_thread();
    }

    #[test]
    #[serial]
    fn test_queue_cross_thread_update_adds_to_queue() {
        reset_main_thread();
        clear_cross_thread_updates();

        let fiber_id = FiberId(42);
        let update = CrossThreadUpdate {
            fiber_id,
            hook_index: 0,
            update: CrossThreadUpdateKind::Value(Box::new(123i32)),
        };

        queue_cross_thread_update(update);

        assert!(has_cross_thread_updates());

        clear_cross_thread_updates();
    }

    #[test]
    #[serial]
    fn test_drain_cross_thread_updates_moves_to_local_batch() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let fiber_id = FiberId(42);

        // Queue a cross-thread update directly
        let update = CrossThreadUpdate {
            fiber_id,
            hook_index: 0,
            update: CrossThreadUpdateKind::Value(Box::new(999i32)),
        };
        queue_cross_thread_update(update);

        assert!(has_cross_thread_updates());

        // Drain to local batch
        drain_cross_thread_updates();

        // Cross-thread queue should be empty now
        assert!(!has_cross_thread_updates());

        // Local batch should have the update
        let has_updates = with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_updates);

        let is_dirty = with_state_batch(|batch| batch.is_fiber_dirty(fiber_id));
        assert!(is_dirty);

        clear_state_batch();
    }

    #[test]
    fn test_drain_cross_thread_updates_applies_value() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        // Queue a cross-thread value update
        let update = CrossThreadUpdate {
            fiber_id,
            hook_index: 0,
            update: CrossThreadUpdateKind::Value(Box::new(42i32)),
        };
        queue_cross_thread_update(update);

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty = end_batch_with_tree(&mut tree);

        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(42));

        clear_state_batch();
    }

    #[test]
    fn test_drain_cross_thread_updates_applies_updater() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 10i32);

        // Queue a cross-thread functional update
        let updater: StateUpdaterFn = Box::new(|any| {
            let n = any.downcast_ref::<i32>().unwrap();
            Box::new(n + 5)
        });
        let update = CrossThreadUpdate {
            fiber_id,
            hook_index: 0,
            update: CrossThreadUpdateKind::Updater(Arc::new(Mutex::new(Some(updater)))),
        };
        queue_cross_thread_update(update);

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty = end_batch_with_tree(&mut tree);

        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(15));

        clear_state_batch();
    }

    #[test]
    fn test_cross_thread_updates_preserve_order() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        // Queue multiple updates in order
        for i in 1..=5 {
            let updater: StateUpdaterFn = Box::new(move |any| {
                let n = any.downcast_ref::<i32>().unwrap();
                Box::new(n + i)
            });
            let update = CrossThreadUpdate {
                fiber_id,
                hook_index: 0,
                update: CrossThreadUpdateKind::Updater(Arc::new(Mutex::new(Some(updater)))),
            };
            queue_cross_thread_update(update);
        }

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        end_batch_with_tree(&mut tree);

        // 0 + 1 + 2 + 3 + 4 + 5 = 15
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(15));

        clear_state_batch();
    }

    #[test]
    fn test_queue_update_routes_to_local_on_main_thread() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Since init_main_thread() may or may not have been called,
        // and is_main_thread() returns true by default, this should
        // route to the local batch
        let fiber_id = FiberId(1);
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        // Should be in local batch, not cross-thread queue
        let has_local = with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_local);

        // Cross-thread queue should be empty
        assert!(!has_cross_thread_updates());

        clear_state_batch();
    }

    // ========================================================================
    // Re-entrancy and edge case tests
    // ========================================================================

    #[test]
    fn test_queue_update_fallback_to_cross_thread_on_reentrant_call() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let fiber_id = FiberId(1);

        // Simulate a re-entrant call by holding a borrow of STATE_BATCH
        // while calling queue_update
        STATE_BATCH.with(|batch| {
            let _guard = batch.borrow_mut(); // Hold the borrow

            // Now call queue_update - it should fall back to cross-thread queue
            // because try_borrow_mut will fail
            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Value(Box::new(42i32)),
                },
            );

            // The update should be in the cross-thread queue, not local batch
            assert!(has_cross_thread_updates());
        });

        // After releasing the borrow, local batch should still be empty
        // (the update went to cross-thread queue)
        let has_local = with_state_batch(|batch| batch.has_pending_updates());
        assert!(!has_local);

        // Cross-thread queue should have the update
        assert!(has_cross_thread_updates());

        clear_state_batch();
        clear_cross_thread_updates();
    }

    #[test]
    fn test_queue_update_fallback_applies_correctly_after_drain() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        // Simulate a re-entrant call that falls back to cross-thread queue
        STATE_BATCH.with(|batch| {
            let _guard = batch.borrow_mut();

            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Value(Box::new(100i32)),
                },
            );
        });

        // Now drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty = end_batch_with_tree(&mut tree);

        // The update should have been applied
        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(100));

        clear_state_batch();
        clear_cross_thread_updates();
    }

    #[test]
    fn test_queue_update_fallback_with_functional_updater() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 10i32);

        // Simulate a re-entrant call with a functional updater
        STATE_BATCH.with(|batch| {
            let _guard = batch.borrow_mut();

            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Updater(Box::new(|any| {
                        let n = any.downcast_ref::<i32>().unwrap();
                        Box::new(n * 2)
                    })),
                },
            );
        });

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty = end_batch_with_tree(&mut tree);

        // The functional update should have been applied: 10 * 2 = 20
        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(20));

        clear_state_batch();
        clear_cross_thread_updates();
    }

    #[test]
    fn test_queue_update_fallback_preserves_order_with_multiple_updates() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        // Simulate multiple re-entrant calls
        STATE_BATCH.with(|batch| {
            let _guard = batch.borrow_mut();

            // Queue multiple updates that should be applied in order
            for i in 1..=3 {
                queue_update(
                    fiber_id,
                    StateUpdate {
                        hook_index: 0,
                        update: StateUpdateKind::Updater(Box::new(move |any| {
                            let n = any.downcast_ref::<i32>().unwrap();
                            Box::new(n + i)
                        })),
                    },
                );
            }
        });

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        end_batch_with_tree(&mut tree);

        // Updates should be applied in order: 0 + 1 + 2 + 3 = 6
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(6));

        clear_state_batch();
        clear_cross_thread_updates();
    }

    #[test]
    fn test_queue_update_mixed_local_and_fallback() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        // First, queue a normal update (goes to local batch)
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(10i32)),
            },
        );

        // Then simulate a re-entrant call (goes to cross-thread queue)
        STATE_BATCH.with(|batch| {
            let _guard = batch.borrow_mut();

            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Updater(Box::new(|any| {
                        let n = any.downcast_ref::<i32>().unwrap();
                        Box::new(n + 5)
                    })),
                },
            );
        });

        // Local batch should have the first update
        let has_local = with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_local);

        // Cross-thread queue should have the second update
        assert!(has_cross_thread_updates());

        // Drain cross-thread updates into local batch
        begin_batch();
        drain_cross_thread_updates();
        let dirty = end_batch_with_tree(&mut tree);

        // Both updates should be applied: first 10, then 10 + 5 = 15
        assert!(dirty.contains(&fiber_id));
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(15));

        clear_state_batch();
        clear_cross_thread_updates();
    }

    #[test]
    fn test_queue_update_fallback_with_value_if_changed() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 42i32);
        tree.mark_clean(fiber_id);

        // Simulate a re-entrant call with ValueIfChanged
        // The equality check is now preserved when falling back to cross-thread queue
        STATE_BATCH.with(|batch| {
            let _guard = batch.borrow_mut();

            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::ValueIfChanged {
                        value: Box::new(42i32), // Same value
                        eq_check: Box::new(|old, new| {
                            let old = old.downcast_ref::<i32>().unwrap();
                            let new = new.downcast_ref::<i32>().unwrap();
                            old == new
                        }),
                    },
                },
            );
        });

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty = end_batch_with_tree(&mut tree);

        // The update should NOT be applied because values are equal
        // The equality check is now preserved in cross-thread updates
        assert!(!dirty.contains(&fiber_id));
        assert!(!tree.get(fiber_id).unwrap().dirty);
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(42));

        clear_state_batch();
        clear_cross_thread_updates();
    }

    #[test]
    fn test_try_borrow_mut_success_path() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        let fiber_id = FiberId(1);

        // Normal case: no existing borrow, should use local batch
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(42i32)),
            },
        );

        // Should be in local batch
        let has_local = with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_local);

        // Cross-thread queue should be empty
        assert!(!has_cross_thread_updates());

        clear_state_batch();
    }

    #[test]
    fn test_background_thread_always_uses_cross_thread_queue() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Initialize main thread to current thread
        init_main_thread();

        // Spawn a background thread that queues an update
        let fiber_id = FiberId(1);
        let handle = std::thread::spawn(move || {
            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Value(Box::new(99i32)),
                },
            );
        });

        handle.join().unwrap();

        // The update should be in the cross-thread queue
        assert!(has_cross_thread_updates());

        // Local batch should be empty (update came from background thread)
        let has_local = with_state_batch(|batch| batch.has_pending_updates());
        assert!(!has_local);

        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();
    }

    #[test]
    fn test_multiple_background_threads_queue_updates() {
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Initialize main thread
        init_main_thread();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

        // Spawn multiple background threads
        let handles: Vec<_> = (1..=5)
            .map(|i| {
                std::thread::spawn(move || {
                    queue_update(
                        fiber_id,
                        StateUpdate {
                            hook_index: 0,
                            update: StateUpdateKind::Updater(Box::new(move |any| {
                                let n = any.downcast_ref::<i32>().unwrap();
                                Box::new(n + i)
                            })),
                        },
                    );
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // All updates should be in cross-thread queue
        assert!(has_cross_thread_updates());

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        end_batch_with_tree(&mut tree);

        // All updates should be applied: 0 + 1 + 2 + 3 + 4 + 5 = 15
        assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(15));

        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // ========================================================================
    // Property tests for cross-thread state updates
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Property 1: Cross-thread updates are applied after drain**
        /// **Validates: Requirements 1.1, 1.2**
        ///
        /// For any value queued from a background thread, after drain_cross_thread_updates()
        /// and end_batch(), the value should be present in the fiber tree.
        #[test]
        fn prop_cross_thread_updates_applied_after_drain(
            values in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

            init_main_thread();

            // Use a barrier to ensure all threads start at the same time
            let barrier = Arc::new(std::sync::Barrier::new(values.len()));

            // Queue updates from background threads directly to cross-thread queue
            let handles: Vec<_> = values.iter().map(|&val| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait(); // Synchronize thread start
                    queue_cross_thread_update(CrossThreadUpdate {
                        fiber_id,
                        hook_index: 0,
                        update: CrossThreadUpdateKind::Value(Box::new(val)),
                    });
                })
            }).collect();

            for handle in handles {
                handle.join().unwrap();
            }

            // Property: Cross-thread queue should have updates
            prop_assert!(has_cross_thread_updates(),
                "Cross-thread queue should have updates after background thread queuing");

            // Drain and apply
            begin_batch();
            drain_cross_thread_updates();
            let dirty = end_batch_with_tree(&mut tree);

            // Property: Fiber should be dirty
            prop_assert!(dirty.contains(&fiber_id),
                "Fiber should be marked dirty after cross-thread updates");

            // Property: Cross-thread queue should be empty after drain
            prop_assert!(!has_cross_thread_updates(),
                "Cross-thread queue should be empty after drain");

            // Property: Some value should be applied (last one wins)
            let final_value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            prop_assert!(final_value.is_some(),
                "Fiber should have a value after updates");

            // Clean up
            clear_state_batch();
            clear_cross_thread_updates();
            reset_main_thread();
        }

        /// **Property 2: Update ordering is preserved**
        /// **Validates: Requirements 1.3**
        ///
        /// For a sequence of functional updates queued in order, the final result
        /// should reflect all updates applied in order.
        #[test]
        fn prop_update_ordering_preserved(
            increments in prop::collection::vec(1i32..10, 1..20)
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

            // Queue updates directly to local batch to test ordering
            begin_batch();
            for inc in &increments {
                let inc_val = *inc;
                with_state_batch_mut(|batch| {
                    batch.queue_update(
                        fiber_id,
                        StateUpdate {
                            hook_index: 0,
                            update: StateUpdateKind::Updater(Box::new(move |any| {
                                let n = any.downcast_ref::<i32>().unwrap();
                                Box::new(n + inc_val)
                            })),
                        },
                    );
                });
            }
            let dirty = end_batch_with_tree(&mut tree);

            // Property: Fiber should be dirty
            prop_assert!(dirty.contains(&fiber_id),
                "Fiber should be marked dirty after updates");

            // Property: Final value should be sum of all increments
            let expected: i32 = increments.iter().sum();
            let actual = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            prop_assert_eq!(actual, Some(expected),
                "Final value should be sum of all increments: expected {}, got {:?}", expected, actual);

            // Clean up
            clear_state_batch();
            clear_cross_thread_updates();
        }

        /// **Property 3: Main thread uses thread-local batching**
        /// **Validates: Requirements 2.1, 2.2**
        ///
        /// Updates queued on the main thread should go to the thread-local batch,
        /// not the cross-thread queue.
        #[test]
        fn prop_main_thread_uses_local_batch(
            values in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            // Don't initialize main thread - is_main_thread() returns true by default
            let fiber_id = FiberId(1);

            for &val in &values {
                queue_update(
                    fiber_id,
                    StateUpdate {
                        hook_index: 0,
                        update: StateUpdateKind::Value(Box::new(val)),
                    },
                );
            }

            // Property: Updates should be in local batch
            let has_local = with_state_batch(|batch| batch.has_pending_updates());
            prop_assert!(has_local,
                "Updates from main thread should be in local batch");

            // Property: Cross-thread queue should be empty
            prop_assert!(!has_cross_thread_updates(),
                "Cross-thread queue should be empty for main thread updates");

            // Property: Only one fiber should be dirty (batched)
            let dirty_count = with_state_batch(|batch| batch.dirty_fiber_count());
            prop_assert_eq!(dirty_count, 1,
                "Only one fiber should be dirty regardless of update count");

            // Clean up
            clear_state_batch();
            clear_cross_thread_updates();
        }

        /// **Property 4: Concurrent cross-thread access is safe**
        /// **Validates: Requirements 5.2, 5.3**
        ///
        /// Multiple threads queuing updates concurrently should not cause panics
        /// or data races, and all updates should be applied.
        #[test]
        fn prop_concurrent_access_safe(
            thread_count in 2usize..10,
            updates_per_thread in 1usize..5
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

            init_main_thread();

            // Use a barrier to ensure all threads start at the same time
            let barrier = Arc::new(std::sync::Barrier::new(thread_count));

            // Spawn multiple threads that each queue multiple updates
            let handles: Vec<_> = (0..thread_count).map(|_| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait(); // Synchronize thread start
                    for _ in 0..updates_per_thread {
                        queue_cross_thread_update(CrossThreadUpdate {
                            fiber_id,
                            hook_index: 0,
                            update: CrossThreadUpdateKind::Updater(Arc::new(Mutex::new(Some(
                                Box::new(move |any| {
                                    let n = any.downcast_ref::<i32>().unwrap();
                                    Box::new(n + 1)
                                })
                            )))),
                        });
                    }
                })
            }).collect();

            // Wait for all threads - should not panic
            for handle in handles {
                handle.join().expect("Thread should not panic");
            }

            // Drain and apply
            begin_batch();
            drain_cross_thread_updates();
            let dirty = end_batch_with_tree(&mut tree);

            // Property: Fiber should be dirty
            prop_assert!(dirty.contains(&fiber_id),
                "Fiber should be marked dirty after concurrent updates");

            // Property: All updates should be applied
            let expected = (thread_count * updates_per_thread) as i32;
            let actual = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            prop_assert_eq!(actual, Some(expected),
                "All {} updates should be applied, got {:?}", expected, actual);

            // Clean up
            clear_state_batch();
            clear_cross_thread_updates();
            reset_main_thread();
        }

        /// **Property 4: State Batch Atomicity**
        /// **Validates: Requirements 3.1, 3.2, 3.3**
        ///
        /// For any sequence of state updates queued during a batch, all updates SHALL
        /// be applied atomically when end_batch is called, and the set of dirty fibers
        /// SHALL be returned.
        #[test]
        fn prop_state_batch_atomicity(
            fiber_count in 1usize..10,
            updates_per_fiber in 1usize..10
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_ids: Vec<_> = (0..fiber_count)
                .map(|_| {
                    let id = tree.mount(None, None);
                    tree.get_mut(id).unwrap().set_hook(0, 0i32);
                    id
                })
                .collect();

            // Begin batch
            begin_batch();

            // Queue multiple updates for each fiber directly to local batch
            for &fiber_id in &fiber_ids {
                for i in 0..updates_per_fiber {
                    with_state_batch_mut(|batch| {
                        batch.queue_update(
                            fiber_id,
                            StateUpdate {
                                hook_index: 0,
                                update: StateUpdateKind::Updater(Box::new(move |any| {
                                    let n = any.downcast_ref::<i32>().unwrap();
                                    Box::new(n + (i as i32 + 1))
                                })),
                            },
                        );
                    });
                }
            }

            // Property: Updates should be queued, not applied yet
            for &fiber_id in &fiber_ids {
                let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
                prop_assert_eq!(value, Some(0),
                    "Updates should not be applied until end_batch");
            }

            // End batch - apply all updates atomically
            let dirty = end_batch_with_tree(&mut tree);

            // Property: All fibers should be in the dirty set
            prop_assert_eq!(dirty.len(), fiber_count,
                "All {} fibers should be marked dirty", fiber_count);
            for &fiber_id in &fiber_ids {
                prop_assert!(dirty.contains(&fiber_id),
                    "Fiber {:?} should be in dirty set", fiber_id);
            }

            // Property: All updates should be applied
            let expected_sum: i32 = (1..=updates_per_fiber as i32).sum();
            for &fiber_id in &fiber_ids {
                let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
                prop_assert_eq!(value, Some(expected_sum),
                    "All updates should be applied atomically, expected {}, got {:?}",
                    expected_sum, value);
            }

            clear_state_batch();
            clear_cross_thread_updates();
        }

        /// **Property 5: Functional Updater Chaining**
        /// **Validates: Requirements 3.5**
        ///
        /// For any sequence of functional updaters queued for the same hook, each updater
        /// SHALL receive the result of the previous updater, producing a correctly chained
        /// final value.
        #[test]
        fn prop_functional_updater_chaining(
            operations in prop::collection::vec((any::<bool>(), 1i32..10), 1..20)
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

            begin_batch();

            // Queue a sequence of functional updates
            // Each operation is (is_add, value): if true, add; if false, multiply
            for &(is_add, value) in &operations {
                queue_update(
                    fiber_id,
                    StateUpdate {
                        hook_index: 0,
                        update: StateUpdateKind::Updater(Box::new(move |any| {
                            let n = any.downcast_ref::<i32>().unwrap();
                            if is_add {
                                Box::new(n.saturating_add(value))
                            } else {
                                Box::new(n.saturating_mul(value))
                            }
                        })),
                    },
                );
            }

            end_batch_with_tree(&mut tree);

            // Property: Final value should reflect all operations applied in order
            let mut expected = 0i32;
            for &(is_add, value) in &operations {
                if is_add {
                    expected = expected.saturating_add(value);
                } else {
                    expected = expected.saturating_mul(value);
                }
            }

            let actual = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            prop_assert_eq!(actual, Some(expected),
                "Functional updaters should chain correctly: expected {}, got {:?}",
                expected, actual);

            clear_state_batch();
            clear_cross_thread_updates();
        }

        /// **Property 6: Equality Check Optimization**
        /// **Validates: Requirements 3.6**
        ///
        /// For any ValueIfChanged or UpdaterIfChanged update where the new value equals
        /// the current value, the fiber SHALL NOT be marked as dirty.
        #[test]
        fn prop_equality_check_optimization(
            initial_value in any::<i32>(),
            same_value_updates in 1usize..10,
            different_value in any::<i32>()
        ) {
            // Ensure different_value is actually different
            prop_assume!(initial_value != different_value);

            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            tree.get_mut(fiber_id).unwrap().set_hook(0, initial_value);
            tree.mark_clean(fiber_id);

            begin_batch();

            // Queue multiple ValueIfChanged updates with the same value directly to local batch
            for _ in 0..same_value_updates {
                with_state_batch_mut(|batch| {
                    batch.queue_update(
                        fiber_id,
                        StateUpdate {
                            hook_index: 0,
                            update: StateUpdateKind::ValueIfChanged {
                                value: Box::new(initial_value),
                                eq_check: Box::new(|old, new| {
                                    let old = old.downcast_ref::<i32>().unwrap();
                                    let new = new.downcast_ref::<i32>().unwrap();
                                    old == new
                                }),
                            },
                        },
                    );
                });
            }

            let dirty = end_batch_with_tree(&mut tree);

            // Property: Fiber should NOT be dirty (all values were equal)
            prop_assert!(!dirty.contains(&fiber_id),
                "Fiber should not be dirty when ValueIfChanged updates have equal values");
            prop_assert!(!tree.get(fiber_id).unwrap().dirty,
                "Fiber dirty flag should be false");

            // Now test with a different value
            tree.mark_clean(fiber_id);
            begin_batch();

            with_state_batch_mut(|batch| {
                batch.queue_update(
                    fiber_id,
                    StateUpdate {
                        hook_index: 0,
                        update: StateUpdateKind::ValueIfChanged {
                            value: Box::new(different_value),
                            eq_check: Box::new(|old, new| {
                                let old = old.downcast_ref::<i32>().unwrap();
                                let new = new.downcast_ref::<i32>().unwrap();
                                old == new
                            }),
                        },
                    },
                );
            });

            let dirty = end_batch_with_tree(&mut tree);

            // Property: Fiber SHOULD be dirty (value changed)
            prop_assert!(dirty.contains(&fiber_id),
                "Fiber should be dirty when ValueIfChanged has different value");
            prop_assert_eq!(
                tree.get(fiber_id).unwrap().get_hook::<i32>(0),
                Some(different_value),
                "Value should be updated to different_value"
            );

            clear_state_batch();
            clear_cross_thread_updates();
        }

        /// **Property 5: Re-entrant calls fall back to cross-thread queue**
        /// **Validates: Requirements 5.2**
        ///
        /// When STATE_BATCH is already borrowed, queue_update should fall back
        /// to the cross-thread queue without panicking.
        #[test]
        fn prop_reentrant_fallback_safe(
            values in prop::collection::vec(any::<i32>(), 1..5)
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let fiber_id = FiberId(1);

            // Simulate re-entrant calls
            STATE_BATCH.with(|batch| {
                let _guard = batch.borrow_mut(); // Hold the borrow

                for &val in &values {
                    // This should fall back to cross-thread queue
                    queue_update(
                        fiber_id,
                        StateUpdate {
                            hook_index: 0,
                            update: StateUpdateKind::Value(Box::new(val)),
                        },
                    );
                }
            });

            // Property: All updates should be in cross-thread queue
            prop_assert!(has_cross_thread_updates(),
                "Re-entrant updates should fall back to cross-thread queue");

            // Property: Local batch should be empty (updates went to cross-thread)
            let has_local = with_state_batch(|batch| batch.has_pending_updates());
            prop_assert!(!has_local,
                "Local batch should be empty after re-entrant fallback");

            // Clean up
            clear_state_batch();
            clear_cross_thread_updates();
        }

        /// **Property 14: Cross-Thread Update Delivery**
        /// **Validates: Requirements 3.7**
        ///
        /// For any state update queued from a background thread, the update SHALL be
        /// delivered to the main thread's batch and applied on the next frame.
        #[test]
        fn prop_cross_thread_update_delivery(
            update_count in 1usize..20,
            thread_count in 1usize..5
        ) {
            reset_main_thread();
            clear_state_batch();
            clear_cross_thread_updates();

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);

            init_main_thread();

            // Use a barrier to ensure all threads start at the same time
            let barrier = Arc::new(std::sync::Barrier::new(thread_count));

            // Spawn background threads that queue updates directly to cross-thread queue
            let handles: Vec<_> = (0..thread_count).map(|_| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait(); // Synchronize thread start
                    for _ in 0..update_count {
                        queue_cross_thread_update(CrossThreadUpdate {
                            fiber_id,
                            hook_index: 0,
                            update: CrossThreadUpdateKind::Updater(Arc::new(Mutex::new(Some(
                                Box::new(move |any| {
                                    let n = any.downcast_ref::<i32>().unwrap();
                                    Box::new(n + 1)
                                })
                            )))),
                        });
                    }
                })
            }).collect();

            // Wait for all background threads to complete
            for handle in handles {
                handle.join().expect("Background thread should not panic");
            }

            // Property: Cross-thread queue should have updates
            prop_assert!(has_cross_thread_updates(),
                "Cross-thread queue should have updates after background threads queue them");

            // Simulate main thread processing (next frame)
            begin_batch();
            drain_cross_thread_updates();
            let dirty = end_batch_with_tree(&mut tree);

            // Property: Updates SHALL be delivered to main thread's batch
            prop_assert!(dirty.contains(&fiber_id),
                "Fiber should be marked dirty after cross-thread updates are drained");

            // Property: Updates SHALL be applied on the next frame
            let expected = (update_count * thread_count) as i32;
            let actual = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            prop_assert_eq!(actual, Some(expected),
                "All {} updates should be applied after drain, got {:?}", expected, actual);

            // Property: Cross-thread queue should be empty after drain
            prop_assert!(!has_cross_thread_updates(),
                "Cross-thread queue should be empty after drain");

            // Clean up
            clear_state_batch();
            clear_cross_thread_updates();
            reset_main_thread();
        }
    }
}
