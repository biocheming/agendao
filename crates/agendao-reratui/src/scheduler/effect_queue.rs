//! Effect queue for post-commit effect execution.

use std::any::Any;
use std::cell::RefCell;

use crate::fiber::{AsyncCleanupFn, AsyncPendingEffect, CleanupFn, FiberId, PendingEffect};
use crate::fiber_tree::FiberTree;

thread_local! {
    /// Thread-local effect queue for the current render context
    static EFFECT_QUEUE: RefCell<EffectQueue> = RefCell::new(EffectQueue::new());
}

/// State stored for each effect hook instance
pub struct EffectHookState {
    /// Dependencies from the last render (boxed for type erasure)
    pub deps: Option<Box<dyn Any + Send>>,
    /// Cleanup function from the last effect execution
    pub cleanup: Option<CleanupFn>,
}

impl EffectHookState {
    /// Create a new effect hook state
    pub fn new() -> Self {
        Self {
            deps: None,
            cleanup: None,
        }
    }

    /// Create with initial deps
    pub fn with_deps<D: Any + Send + 'static>(deps: D) -> Self {
        Self {
            deps: Some(Box::new(deps)),
            cleanup: None,
        }
    }

    /// Check if deps have changed
    pub fn deps_changed<D: PartialEq + 'static>(&self, new_deps: &D) -> bool {
        match &self.deps {
            None => true, // No previous deps means first render
            Some(boxed) => {
                match boxed.downcast_ref::<D>() {
                    Some(old_deps) => old_deps != new_deps,
                    None => true, // Type mismatch, treat as changed
                }
            }
        }
    }

    /// Update deps
    pub fn set_deps<D: Any + Send + 'static>(&mut self, deps: D) {
        self.deps = Some(Box::new(deps));
    }

    /// Take the cleanup function (removes it from state)
    pub fn take_cleanup(&mut self) -> Option<CleanupFn> {
        self.cleanup.take()
    }

    /// Set the cleanup function
    pub fn set_cleanup(&mut self, cleanup: CleanupFn) {
        self.cleanup = Some(cleanup);
    }
}

impl Default for EffectHookState {
    fn default() -> Self {
        Self::new()
    }
}

/// Queue of effects to run after commit phase
pub struct EffectQueue {
    /// Effects queued during current render, grouped by fiber
    pending: Vec<(FiberId, PendingEffect)>,
    /// Async effects queued during current render, grouped by fiber
    pending_async: Vec<(FiberId, AsyncPendingEffect)>,
    /// Cleanups to run before new effects
    cleanups_to_run: Vec<CleanupFn>,
    /// Async cleanups to run before new effects
    async_cleanups_to_run: Vec<AsyncCleanupFn>,
}

impl EffectQueue {
    /// Create a new empty effect queue
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            pending_async: Vec::new(),
            cleanups_to_run: Vec::new(),
            async_cleanups_to_run: Vec::new(),
        }
    }

    /// Queue an effect for post-commit execution
    pub fn queue_effect(&mut self, fiber_id: FiberId, effect: PendingEffect) {
        self.pending.push((fiber_id, effect));
    }

    /// Queue an async effect for post-commit execution
    pub fn queue_async_effect(&mut self, fiber_id: FiberId, effect: AsyncPendingEffect) {
        self.pending_async.push((fiber_id, effect));
    }

    /// Queue a cleanup function to run before new effects
    pub fn queue_cleanup(&mut self, cleanup: CleanupFn) {
        self.cleanups_to_run.push(cleanup);
    }

    /// Queue an async cleanup function to run before new effects
    pub fn queue_async_cleanup(&mut self, cleanup: AsyncCleanupFn) {
        self.async_cleanups_to_run.push(cleanup);
    }

    /// Execute all queued effects (called after commit)
    /// Note: This only runs synchronous effects. Use flush_async for async effects.
    pub fn flush(&mut self, tree: &mut FiberTree) {
        // 1. Run cleanups in reverse order
        while let Some(cleanup) = self.cleanups_to_run.pop() {
            cleanup();
        }

        // 2. Run effects in declaration order
        for (fiber_id, pending) in self.pending.drain(..) {
            if let Some(fiber) = tree.get_mut(fiber_id)
                && let Some(cleanup) = (pending.effect)()
            {
                // Store cleanup indexed by hook_index for proper cleanup ordering
                fiber.cleanup_by_hook.insert(pending.hook_index, cleanup);
            }
        }
    }

    /// Execute all queued async effects (called after commit)
    /// This is an async function that handles async cleanups and effects.
    pub async fn flush_async(&mut self, tree: &mut FiberTree) {
        // 1. Run sync cleanups in reverse order first
        while let Some(cleanup) = self.cleanups_to_run.pop() {
            cleanup();
        }

        // 2. Run async cleanups in reverse order
        while let Some(async_cleanup) = self.async_cleanups_to_run.pop() {
            async_cleanup().await;
        }

        // 3. Run sync effects in declaration order
        for (fiber_id, pending) in self.pending.drain(..) {
            if let Some(fiber) = tree.get_mut(fiber_id)
                && let Some(cleanup) = (pending.effect)()
            {
                // Store cleanup indexed by hook_index for proper cleanup ordering
                fiber.cleanup_by_hook.insert(pending.hook_index, cleanup);
            }
        }

        // 4. Run async effects in declaration order
        for (fiber_id, pending) in self.pending_async.drain(..) {
            if let Some(fiber) = tree.get_mut(fiber_id)
                && let Some(async_cleanup) = (pending.effect)().await
            {
                // Store async cleanup in fiber indexed by hook_index for proper cleanup ordering
                // This ensures async cleanups are associated with their fiber and hook
                fiber
                    .async_cleanup_by_hook
                    .insert(pending.hook_index, async_cleanup);
            }
        }
    }

    /// Check if there are pending effects
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
            || !self.pending_async.is_empty()
            || !self.cleanups_to_run.is_empty()
            || !self.async_cleanups_to_run.is_empty()
    }

    /// Check if there are pending async effects
    pub fn has_pending_async(&self) -> bool {
        !self.pending_async.is_empty() || !self.async_cleanups_to_run.is_empty()
    }

    /// Clear all pending effects and cleanups
    pub fn clear(&mut self) {
        self.pending.clear();
        self.pending_async.clear();
        self.cleanups_to_run.clear();
        self.async_cleanups_to_run.clear();
    }

    /// Get the number of pending effects
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Get the number of pending async effects
    pub fn pending_async_count(&self) -> usize {
        self.pending_async.len()
    }

    /// Drain all pending async effects for testing or manual execution
    /// Returns the pending async effects as a vector
    pub fn drain_async_effects(&mut self) -> Vec<(FiberId, AsyncPendingEffect)> {
        self.pending_async.drain(..).collect()
    }

    /// Get the number of pending cleanups
    pub fn cleanup_count(&self) -> usize {
        self.cleanups_to_run.len()
    }

    /// Get the number of pending async cleanups
    pub fn async_cleanup_count(&self) -> usize {
        self.async_cleanups_to_run.len()
    }
}

impl Default for EffectQueue {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local access functions

/// Queue an effect to the thread-local effect queue
pub fn queue_effect(fiber_id: FiberId, effect: PendingEffect) {
    EFFECT_QUEUE.with(|q| {
        q.borrow_mut().queue_effect(fiber_id, effect);
    });
}

/// Queue an async effect to the thread-local effect queue
pub fn queue_async_effect(fiber_id: FiberId, effect: AsyncPendingEffect) {
    EFFECT_QUEUE.with(|q| {
        q.borrow_mut().queue_async_effect(fiber_id, effect);
    });
}

/// Queue a cleanup to the thread-local effect queue
pub fn queue_cleanup(cleanup: CleanupFn) {
    EFFECT_QUEUE.with(|q| {
        q.borrow_mut().queue_cleanup(cleanup);
    });
}

/// Queue an async cleanup to the thread-local effect queue
pub fn queue_async_cleanup(cleanup: AsyncCleanupFn) {
    EFFECT_QUEUE.with(|q| {
        q.borrow_mut().queue_async_cleanup(cleanup);
    });
}

/// Flush the thread-local effect queue with a provided tree
pub fn flush_effects_with_tree(tree: &mut FiberTree) {
    EFFECT_QUEUE.with(|q| {
        q.borrow_mut().flush(tree);
    });
}

/// Flush the thread-local effect queue using the thread-local fiber tree
pub fn flush_effects() {
    crate::fiber_tree::with_fiber_tree_mut(|tree| {
        EFFECT_QUEUE.with(|q| {
            q.borrow_mut().flush(tree);
        });
    });
}

/// Flush async effects from the thread-local effect queue using the thread-local fiber tree
/// This handles async cleanups and async effects with tokio.
pub async fn flush_async_effects() {
    // We need to handle the async flush carefully due to thread-local borrowing
    // First, check if there are any async effects to process
    let has_async = has_pending_async_effects();

    if has_async {
        // Drain the async effects and cleanups to process them outside the borrow
        let (async_effects, async_cleanups) = EFFECT_QUEUE.with(|q| {
            let mut queue = q.borrow_mut();
            let effects = queue.pending_async.drain(..).collect::<Vec<_>>();
            let cleanups = queue.async_cleanups_to_run.drain(..).collect::<Vec<_>>();
            (effects, cleanups)
        });

        // Run async cleanups in reverse order
        for async_cleanup in async_cleanups.into_iter().rev() {
            async_cleanup().await;
        }

        // Run async effects in declaration order
        for (fiber_id, pending) in async_effects {
            // Get the fiber to store the cleanup
            let cleanup_opt = (pending.effect)().await;

            if let Some(async_cleanup) = cleanup_opt {
                // Store async cleanup in fiber indexed by hook_index
                crate::fiber_tree::with_fiber_tree_mut(|tree| {
                    if let Some(fiber) = tree.get_mut(fiber_id) {
                        fiber
                            .async_cleanup_by_hook
                            .insert(pending.hook_index, async_cleanup);
                    }
                });
            }
        }
    }
}

/// Check if the thread-local effect queue has pending work
pub fn has_pending_effects() -> bool {
    EFFECT_QUEUE.with(|q| q.borrow().has_pending())
}

/// Check if the thread-local effect queue has pending async work
pub fn has_pending_async_effects() -> bool {
    EFFECT_QUEUE.with(|q| q.borrow().has_pending_async())
}

/// Clear the thread-local effect queue
pub fn clear_effect_queue() {
    EFFECT_QUEUE.with(|q| {
        q.borrow_mut().clear();
    });
}

/// Execute a closure with the thread-local effect queue
pub fn with_effect_queue<R, F: FnOnce(&mut EffectQueue) -> R>(f: F) -> R {
    EFFECT_QUEUE.with(|q| f(&mut q.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_effect_queue_creation() {
        let queue = EffectQueue::new();
        assert!(!queue.has_pending());
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.cleanup_count(), 0);
    }

    #[test]
    fn test_queue_effect() {
        let mut queue = EffectQueue::new();
        let fiber_id = FiberId(1);

        let effect = PendingEffect {
            effect: Box::new(|| None),
            hook_index: 0,
        };

        queue.queue_effect(fiber_id, effect);
        assert!(queue.has_pending());
        assert_eq!(queue.pending_count(), 1);
    }

    #[test]
    fn test_queue_cleanup() {
        let mut queue = EffectQueue::new();

        let cleanup: CleanupFn = Box::new(|| {});
        queue.queue_cleanup(cleanup);

        assert!(queue.has_pending());
        assert_eq!(queue.cleanup_count(), 1);
    }

    #[test]
    fn test_flush_runs_cleanups_before_effects() {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        let execution_order = Arc::new(Mutex::new(Vec::new()));

        let mut queue = EffectQueue::new();

        // Queue a cleanup
        let order_clone = execution_order.clone();
        let cleanup: CleanupFn = Box::new(move || {
            order_clone.lock().unwrap().push("cleanup");
        });
        queue.queue_cleanup(cleanup);

        // Queue an effect
        let order_clone = execution_order.clone();
        let effect = PendingEffect {
            effect: Box::new(move || {
                order_clone.lock().unwrap().push("effect");
                None
            }),
            hook_index: 0,
        };
        queue.queue_effect(fiber_id, effect);

        // Flush
        queue.flush(&mut tree);

        // Verify order: cleanups run before effects
        let order = execution_order.lock().unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], "cleanup");
        assert_eq!(order[1], "effect");
    }

    #[test]
    fn test_flush_runs_cleanups_in_reverse_order() {
        let mut tree = FiberTree::new();
        let _ = tree.mount(None, None);

        let execution_order = Arc::new(Mutex::new(Vec::new()));

        let mut queue = EffectQueue::new();

        // Queue multiple cleanups
        for i in 1..=3 {
            let order_clone = execution_order.clone();
            let cleanup: CleanupFn = Box::new(move || {
                order_clone.lock().unwrap().push(i);
            });
            queue.queue_cleanup(cleanup);
        }

        queue.flush(&mut tree);

        // Verify reverse order: 3, 2, 1
        let order = execution_order.lock().unwrap();
        assert_eq!(*order, vec![3, 2, 1]);
    }

    #[test]
    fn test_flush_runs_effects_in_declaration_order() {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        let execution_order = Arc::new(Mutex::new(Vec::new()));

        let mut queue = EffectQueue::new();

        // Queue multiple effects
        for i in 1..=3 {
            let order_clone = execution_order.clone();
            let effect = PendingEffect {
                effect: Box::new(move || {
                    order_clone.lock().unwrap().push(i);
                    None
                }),
                hook_index: i,
            };
            queue.queue_effect(fiber_id, effect);
        }

        queue.flush(&mut tree);

        // Verify declaration order: 1, 2, 3
        let order = execution_order.lock().unwrap();
        assert_eq!(*order, vec![1, 2, 3]);
    }

    #[test]
    fn test_effect_returns_cleanup() {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        let mut queue = EffectQueue::new();

        let effect = PendingEffect {
            effect: Box::new(|| Some(Box::new(|| {}) as CleanupFn)),
            hook_index: 0,
        };
        queue.queue_effect(fiber_id, effect);

        queue.flush(&mut tree);

        // Verify cleanup was stored in fiber by hook_index
        let fiber = tree.get(fiber_id).unwrap();
        assert_eq!(fiber.cleanup_by_hook.len(), 1);
        assert!(fiber.cleanup_by_hook.contains_key(&0));
    }

    #[test]
    fn test_clear_queue() {
        let mut queue = EffectQueue::new();
        let fiber_id = FiberId(1);

        queue.queue_effect(
            fiber_id,
            PendingEffect {
                effect: Box::new(|| None),
                hook_index: 0,
            },
        );
        queue.queue_cleanup(Box::new(|| {}));

        assert!(queue.has_pending());

        queue.clear();

        assert!(!queue.has_pending());
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.cleanup_count(), 0);
    }

    #[test]
    fn test_effect_hook_state_creation() {
        let state = EffectHookState::new();
        assert!(state.deps.is_none());
        assert!(state.cleanup.is_none());
    }

    #[test]
    fn test_effect_hook_state_with_deps() {
        let state = EffectHookState::with_deps((1, 2, 3));
        assert!(state.deps.is_some());
    }

    #[test]
    fn test_effect_hook_state_deps_changed() {
        let mut state = EffectHookState::new();

        // First render - no deps, should be "changed"
        assert!(state.deps_changed(&(1, 2)));

        // Set deps
        state.set_deps((1, 2));

        // Same deps - not changed
        assert!(!state.deps_changed(&(1, 2)));

        // Different deps - changed
        assert!(state.deps_changed(&(1, 3)));
    }

    #[test]
    fn test_effect_hook_state_cleanup() {
        let mut state = EffectHookState::new();

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        state.set_cleanup(Box::new(move || {
            called_clone.store(true, Ordering::SeqCst);
        }));

        assert!(state.cleanup.is_some());

        let cleanup = state.take_cleanup();
        assert!(state.cleanup.is_none());

        cleanup.unwrap()();
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_thread_local_queue_effect() {
        // Clear any existing state
        clear_effect_queue();

        let fiber_id = FiberId(1);
        queue_effect(
            fiber_id,
            PendingEffect {
                effect: Box::new(|| None),
                hook_index: 0,
            },
        );

        assert!(has_pending_effects());

        clear_effect_queue();
        assert!(!has_pending_effects());
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::{Arc, Mutex};

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Property 7: Effect Execution Ordering**
        /// **Validates: Requirements 4.2, 4.3**
        ///
        /// For any set of queued effects and cleanups, cleanups SHALL run in reverse order
        /// before new effects, and new effects SHALL run in declaration order.
        #[test]
        fn prop_effect_execution_ordering(
            cleanup_count in 1usize..10,
            effect_count in 1usize..10
        ) {
            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            let mut queue = EffectQueue::new();

            let execution_order = Arc::new(Mutex::new(Vec::new()));

            // Queue cleanups
            for i in 0..cleanup_count {
                let order_clone = execution_order.clone();
                let cleanup: CleanupFn = Box::new(move || {
                    order_clone.lock().unwrap().push(format!("cleanup_{}", i));
                });
                queue.queue_cleanup(cleanup);
            }

            // Queue effects
            for i in 0..effect_count {
                let order_clone = execution_order.clone();
                let effect = PendingEffect {
                    effect: Box::new(move || {
                        order_clone.lock().unwrap().push(format!("effect_{}", i));
                        None
                    }),
                    hook_index: i,
                };
                queue.queue_effect(fiber_id, effect);
            }

            // Flush the queue
            queue.flush(&mut tree);

            let order = execution_order.lock().unwrap();

            // Property 1: Cleanups run before effects
            let cleanup_end_idx = cleanup_count;
            for i in 0..cleanup_count {
                prop_assert!(
                    order[i].starts_with("cleanup_"),
                    "Cleanups should run before effects, but found {} at index {}",
                    order[i], i
                );
            }

            // Property 2: Cleanups run in reverse order
            for i in 0..cleanup_count {
                let expected_cleanup_idx = cleanup_count - 1 - i;
                prop_assert_eq!(
                    &order[i],
                    &format!("cleanup_{}", expected_cleanup_idx),
                    "Cleanups should run in reverse order"
                );
            }

            // Property 3: Effects run in declaration order
            for i in 0..effect_count {
                let order_idx = cleanup_end_idx + i;
                prop_assert_eq!(
                    &order[order_idx],
                    &format!("effect_{}", i),
                    "Effects should run in declaration order"
                );
            }

            // Property 4: Total execution count matches queued count
            prop_assert_eq!(
                order.len(),
                cleanup_count + effect_count,
                "All cleanups and effects should execute exactly once"
            );
        }

        /// **Property 8: Effect Dependency Tracking**
        /// **Validates: Requirements 4.4, 4.5, 4.6**
        ///
        /// For any effect with dependencies, the effect SHALL run only when dependencies
        /// change (or on first render), and SHALL NOT run when dependencies are equal.
        #[test]
        fn prop_effect_dependency_tracking(
            initial_deps in any::<(i32, String)>(),
            same_deps_renders in 1usize..5,
            changed_deps in any::<(i32, String)>()
        ) {
            // Ensure changed_deps is actually different
            prop_assume!(initial_deps != changed_deps);

            let mut effect_state = EffectHookState::new();
            let mut run_count = 0;

            // First render - should run (no previous deps)
            prop_assert!(
                effect_state.deps_changed(&initial_deps),
                "Effect should run on first render (no previous deps)"
            );
            effect_state.set_deps(initial_deps.clone());
            run_count += 1;

            // Multiple renders with same deps - should NOT run
            for _ in 0..same_deps_renders {
                prop_assert!(
                    !effect_state.deps_changed(&initial_deps),
                    "Effect should NOT run when deps are equal"
                );
                // Don't increment run_count since effect doesn't run
            }

            // Property: Effect ran exactly once during same-deps renders
            prop_assert_eq!(
                run_count, 1,
                "Effect should run exactly once when deps don't change"
            );

            // Render with changed deps - should run
            prop_assert!(
                effect_state.deps_changed(&changed_deps),
                "Effect should run when deps change"
            );
            effect_state.set_deps(changed_deps.clone());
            run_count += 1;

            // Property: Effect ran exactly twice total (first render + deps change)
            prop_assert_eq!(
                run_count, 2,
                "Effect should run exactly twice: first render and when deps change"
            );

            // Another render with same changed deps - should NOT run
            prop_assert!(
                !effect_state.deps_changed(&changed_deps),
                "Effect should NOT run when deps remain equal after change"
            );
        }

        /// **Property: Effect with None deps runs every render**
        /// **Validates: Requirement 4.5**
        ///
        /// When deps are None, the effect SHALL run after every render.
        #[test]
        fn prop_effect_none_deps_runs_every_render(
            render_count in 1usize..20
        ) {
            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            let mut queue = EffectQueue::new();

            let execution_count = Arc::new(Mutex::new(0));

            // Simulate multiple renders with None deps
            for _ in 0..render_count {
                let count_clone = execution_count.clone();
                let effect = PendingEffect {
                    effect: Box::new(move || {
                        *count_clone.lock().unwrap() += 1;
                        None
                    }),
                    hook_index: 0,
                };
                queue.queue_effect(fiber_id, effect);
                queue.flush(&mut tree);
            }

            // Property: Effect should run exactly render_count times
            let final_count = *execution_count.lock().unwrap();
            prop_assert_eq!(
                final_count,
                render_count,
                "Effect with None deps should run after every render"
            );
        }

        /// **Property: Effect with Some(()) deps runs only once**
        /// **Validates: Requirement 4.6**
        ///
        /// When deps are Some(()), the effect SHALL run only once on mount.
        #[test]
        fn prop_effect_empty_deps_runs_once(
            render_count in 2usize..20
        ) {
            let mut effect_state = EffectHookState::new();
            let mut run_count = 0;

            // First render - should run
            if effect_state.deps_changed(&()) {
                run_count += 1;
                effect_state.set_deps(());
            }

            // Multiple subsequent renders with same empty deps - should NOT run
            for _ in 1..render_count {
                if effect_state.deps_changed(&()) {
                    run_count += 1;
                }
            }

            // Property: Effect should run exactly once
            prop_assert_eq!(
                run_count, 1,
                "Effect with Some(()) deps should run only once on mount"
            );
        }

        /// **Property: Cleanup runs before new effect**
        /// **Validates: Requirement 4.7**
        ///
        /// When deps change, the effect SHALL queue cleanup from previous effect
        /// before the new effect runs.
        #[test]
        fn prop_cleanup_runs_before_new_effect(
            initial_deps in any::<i32>(),
            changed_deps in any::<i32>()
        ) {
            prop_assume!(initial_deps != changed_deps);

            let mut tree = FiberTree::new();
            let fiber_id = tree.mount(None, None);
            let mut queue = EffectQueue::new();

            let execution_order = Arc::new(Mutex::new(Vec::new()));

            // First effect with cleanup
            let order_clone = execution_order.clone();
            let effect1 = PendingEffect {
                effect: Box::new(move || {
                    order_clone.lock().unwrap().push("effect1");
                    let order_clone2 = order_clone.clone();
                    Some(Box::new(move || {
                        order_clone2.lock().unwrap().push("cleanup1");
                    }) as CleanupFn)
                }),
                hook_index: 0,
            };
            queue.queue_effect(fiber_id, effect1);
            queue.flush(&mut tree);

            // Queue the cleanup from the first effect
            if let Some(fiber) = tree.get_mut(fiber_id) {
                if let Some(cleanup) = fiber.cleanup_by_hook.remove(&0) {
                    queue.queue_cleanup(cleanup);
                }
            }

            // Second effect (deps changed)
            let order_clone = execution_order.clone();
            let effect2 = PendingEffect {
                effect: Box::new(move || {
                    order_clone.lock().unwrap().push("effect2");
                    None
                }),
                hook_index: 0,
            };
            queue.queue_effect(fiber_id, effect2);
            queue.flush(&mut tree);

            let order = execution_order.lock().unwrap();

            // Property: Cleanup runs before new effect
            prop_assert_eq!(order.len(), 3, "Should have 3 executions");
            prop_assert_eq!(&order[0], &"effect1".to_string(), "First effect runs first");
            prop_assert_eq!(&order[1], &"cleanup1".to_string(), "Cleanup runs before new effect");
            prop_assert_eq!(&order[2], &"effect2".to_string(), "New effect runs after cleanup");
        }
    }
}
