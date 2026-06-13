//! Effect hook with post-commit execution.
//!
//! This module provides `use_effect`, a React-style effect hook that:
//! - Queues effects to run after the commit phase (not during render)
//! - Properly handles cleanup functions
//! - Supports dependency tracking for conditional execution

use std::future::Future;
use std::pin::Pin;

use crate::fiber::{AsyncCleanupFn, AsyncPendingEffect, PendingEffect};
use crate::fiber_tree::with_current_fiber;
use crate::scheduler::effect_queue::{
    queue_async_cleanup, queue_async_effect, queue_cleanup, queue_effect,
};

/// React-style useEffect with proper post-commit execution
///
/// # Differences from use_effect (deprecated)
/// - Effects are queued, not executed immediately
/// - Cleanup runs before new effect, not during render
/// - Proper fiber-scoped state
///
/// # Arguments
/// - `effect`: A function that performs the side effect and optionally returns a cleanup function
/// - `deps`: Dependencies that determine when the effect should re-run
///   - `Some(())` or use `use_effect_once`: Run only once on mount
///   - `None`: Run after every render
///   - `Some((a, b, ...))`: Run when any dependency changes
///
/// # Example
/// ```ignore
/// // Run once on mount
/// use_effect_once(|| {
///     println!("Mounted!");
///     Some(|| println!("Unmounting!"))
/// });
///
/// // Run when count changes
/// use_effect(|| {
///     println!("Count changed to: {}", count);
///     Option::<fn()>::None
/// }, Some((count,)));
///
/// // Run after every render
/// use_effect(|| {
///     println!("Rendered!");
///     Option::<fn()>::None
/// }, None::<()>);
/// ```
pub fn use_effect<Deps, F, C>(effect: F, deps: Option<Deps>)
where
    Deps: PartialEq + Clone + Send + 'static,
    F: FnOnce() -> Option<C> + 'static,
    C: FnOnce() + Send + 'static,
{
    with_current_fiber(|fiber| {
        fiber.track_hook_call("use_effect");
        let hook_index = fiber.next_hook_index();

        // Get previous deps from fiber's hook state
        let prev_deps: Option<Option<Deps>> = fiber.get_hook(hook_index);

        // Determine if effect should run
        let should_run = match (&deps, &prev_deps) {
            // No deps (None) = run every render
            (None, _) => true,
            // First render (no previous state)
            (Some(_), None) => true,
            // Check if deps changed
            (Some(current_deps), Some(Some(prev_deps))) => current_deps != prev_deps,
            // Previous was None, now has deps - run
            (Some(_), Some(None)) => true,
        };

        if should_run {
            // Queue cleanup from previous effect if it exists
            if let Some(cleanup) = fiber.cleanup_by_hook.remove(&hook_index) {
                queue_cleanup(cleanup);
            }

            // Wrap the effect to store cleanup in fiber after execution
            let fiber_id = fiber.id;
            let wrapped_effect = Box::new(move || {
                let cleanup_opt = effect();
                cleanup_opt.map(|c| Box::new(c) as crate::fiber::CleanupFn)
            });

            // Queue new effect for post-commit execution
            queue_effect(
                fiber_id,
                PendingEffect {
                    effect: wrapped_effect,
                    hook_index,
                },
            );
        }

        // Store deps for next render comparison
        fiber.set_hook(hook_index, deps);
    });
}

/// Convenience function for effects that run only once on mount
///
/// Equivalent to `use_effect(effect, Some(()))`
pub fn use_effect_once<F, C>(effect: F)
where
    F: FnOnce() -> Option<C> + 'static,
    C: FnOnce() + Send + 'static,
{
    use_effect(effect, Some(()));
}

/// React-style useAsyncEffect with proper post-commit execution for async operations
///
/// This hook is similar to `use_effect` but supports async effect functions
/// and async cleanup functions. It's useful for effects that need to perform
/// async operations like data fetching, subscriptions, or other I/O.
///
/// # Differences from use_effect
/// - Effect function returns a Future instead of executing synchronously
/// - Cleanup function can also be async
/// - Integrates with tokio for async execution
///
/// # Arguments
/// - `effect`: An async function that performs the side effect and optionally returns an async cleanup function
/// - `deps`: Dependencies that determine when the effect should re-run
///   - `Some(())` or use `use_async_effect_once`: Run only once on mount
///   - `None`: Run after every render
///   - `Some((a, b, ...))`: Run when any dependency changes
///
/// # Example
/// ```ignore
/// // Async effect that fetches data
/// use_async_effect(|| {
///     let set_data = set_data.clone();
///     async move {
///         let data = fetch_data().await;
///         set_data.set(data);
///         
///         // Optional async cleanup
///         Some(|| async move {
///             println!("Cleaning up...");
///         })
///     }
/// }, Some((user_id,)));
///
/// // Run once on mount
/// use_async_effect_once(|| async {
///     println!("Mounted!");
///     Some(|| async { println!("Unmounting!") })
/// });
/// ```
pub fn use_async_effect<Deps, F, Fut, C, CFut>(effect: F, deps: Option<Deps>)
where
    Deps: PartialEq + Clone + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Option<C>> + Send + 'static,
    C: FnOnce() -> CFut + Send + 'static,
    CFut: Future<Output = ()> + Send + 'static,
{
    with_current_fiber(|fiber| {
        fiber.track_hook_call("use_async_effect");
        let hook_index = fiber.next_hook_index();

        // Get previous deps from fiber's hook state
        let prev_deps: Option<Option<Deps>> = fiber.get_hook(hook_index);

        // Determine if effect should run
        let should_run = match (&deps, &prev_deps) {
            // No deps (None) = run every render
            (None, _) => true,
            // First render (no previous state)
            (Some(_), None) => true,
            // Check if deps changed
            (Some(current_deps), Some(Some(prev_deps))) => current_deps != prev_deps,
            // Previous was None, now has deps - run
            (Some(_), Some(None)) => true,
        };

        if should_run {
            // Queue cleanup from previous async effect if it exists
            if let Some(async_cleanup) = fiber.async_cleanup_by_hook.remove(&hook_index) {
                queue_async_cleanup(async_cleanup);
            }

            // Wrap the async effect to return the proper type
            let fiber_id = fiber.id;
            let wrapped_effect: crate::fiber::AsyncEffectFn = Box::new(move || {
                Box::pin(async move {
                    let cleanup_opt = effect().await;
                    cleanup_opt.map(|c| {
                        Box::new(move || Box::pin(c()) as Pin<Box<dyn Future<Output = ()> + Send>>)
                            as AsyncCleanupFn
                    })
                })
            });

            // Queue new async effect for post-commit execution
            queue_async_effect(
                fiber_id,
                AsyncPendingEffect {
                    effect: wrapped_effect,
                    hook_index,
                },
            );
        }

        // Store deps for next render comparison
        fiber.set_hook(hook_index, deps);
    });
}

/// Convenience function for async effects that run only once on mount
///
/// Equivalent to `use_async_effect(effect, Some(()))`
pub fn use_async_effect_once<F, Fut, C, CFut>(effect: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Option<C>> + Send + 'static,
    C: FnOnce() -> CFut + Send + 'static,
    CFut: Future<Output = ()> + Send + 'static,
{
    use_async_effect(effect, Some(()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use crate::scheduler::effect_queue::{
        clear_effect_queue, flush_effects_with_tree, has_pending_effects,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn setup_test_fiber() -> crate::fiber::FiberId {
        clear_effect_queue();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        clear_fiber_tree();
        clear_effect_queue();
    }

    #[test]
    fn test_effect_queued_not_executed_immediately() {
        let _fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));
        let executed_clone = executed.clone();

        use_effect(
            move || {
                executed_clone.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some(()),
        );

        // Effect should be queued but not executed
        assert_eq!(executed.load(Ordering::SeqCst), 0);
        assert!(has_pending_effects());

        cleanup_test();
    }

    #[test]
    fn test_effect_runs_on_flush() {
        let _fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));
        let executed_clone = executed.clone();

        use_effect(
            move || {
                executed_clone.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some(()),
        );

        // Flush effects
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        // Effect should have executed
        assert_eq!(executed.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_effect_with_empty_deps_runs_once() {
        let fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));

        // First render
        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                Some(()), // Empty deps - run once
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 1);

        // Second render - effect should NOT run again
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                Some(()), // Same empty deps
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        // Should still be 1 - effect didn't run again
        assert_eq!(executed.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_effect_with_none_deps_runs_every_render() {
        let fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));

        // First render
        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                None::<()>, // None deps - run every render
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 1);

        // Second render - effect SHOULD run again
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                None::<()>,
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        // Should be 2 - effect ran again
        assert_eq!(executed.load(Ordering::SeqCst), 2);

        cleanup_test();
    }

    #[test]
    fn test_effect_with_changing_deps() {
        let fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));

        // First render with deps = (1,)
        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                Some((1i32,)),
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 1);

        // Second render with same deps = (1,) - should NOT run
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                Some((1i32,)), // Same deps
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 1); // Still 1

        // Third render with different deps = (2,) - SHOULD run
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        {
            let executed_clone = executed.clone();
            use_effect(
                move || {
                    executed_clone.fetch_add(1, Ordering::SeqCst);
                    Option::<fn()>::None
                },
                Some((2i32,)), // Different deps
            );
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 2); // Now 2

        cleanup_test();
    }

    #[test]
    fn test_use_effect_once() {
        let fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));

        // First render
        {
            let executed_clone = executed.clone();
            use_effect_once(move || {
                executed_clone.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            });
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 1);

        // Second render - should NOT run again
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        {
            let executed_clone = executed.clone();
            use_effect_once(move || {
                executed_clone.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            });
        }

        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            flush_effects_with_tree(tree);
        });

        assert_eq!(executed.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[tokio::test]
    async fn test_async_effect_queued_not_executed_immediately() {
        let _fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));
        let executed_clone = executed.clone();

        use_async_effect(
            move || {
                let executed = executed_clone.clone();
                async move {
                    executed.fetch_add(1, Ordering::SeqCst);
                    Option::<fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>::None
                }
            },
            Some(()),
        );

        // Effect should be queued but not executed
        assert_eq!(executed.load(Ordering::SeqCst), 0);
        assert!(crate::scheduler::effect_queue::has_pending_async_effects());

        cleanup_test();
    }

    #[tokio::test]
    async fn test_async_effect_with_empty_deps_runs_once() {
        let fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));

        // First render
        {
            let executed_clone = executed.clone();
            use_async_effect(
                move || {
                    let executed = executed_clone.clone();
                    async move {
                        executed.fetch_add(1, Ordering::SeqCst);
                        Option::<
                            fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
                        >::None
                    }
                },
                Some(()), // Empty deps - run once
            );
        }

        // Verify effect was queued
        assert!(crate::scheduler::effect_queue::has_pending_async_effects());

        // Flush async effects
        let pending =
            crate::scheduler::effect_queue::with_effect_queue(|queue| queue.drain_async_effects());

        // Execute the async effects
        for (_fiber_id, pending_effect) in pending {
            let future = (pending_effect.effect)();
            future.await;
        }

        assert_eq!(executed.load(Ordering::SeqCst), 1);

        // Second render - effect should NOT run again
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        {
            let executed_clone = executed.clone();
            use_async_effect(
                move || {
                    let executed = executed_clone.clone();
                    async move {
                        executed.fetch_add(1, Ordering::SeqCst);
                        Option::<
                            fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
                        >::None
                    }
                },
                Some(()), // Same empty deps
            );
        }

        // Should not have queued a new async effect since deps didn't change
        assert!(!crate::scheduler::effect_queue::has_pending_async_effects());

        // Should still be 1 - effect didn't run again
        assert_eq!(executed.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_async_effect_once() {
        let _fiber_id = setup_test_fiber();

        let executed = Arc::new(AtomicUsize::new(0));
        let executed_clone = executed.clone();

        use_async_effect_once(move || {
            let executed = executed_clone.clone();
            async move {
                executed.fetch_add(1, Ordering::SeqCst);
                Option::<fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>::None
            }
        });

        // Effect should be queued
        assert!(crate::scheduler::effect_queue::has_pending_async_effects());

        cleanup_test();
    }
}
