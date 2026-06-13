//! Memoization hooks for callbacks and computed values.
//!
//! This module provides React-like memoization hooks with proper fiber-based semantics.
//! These hooks help optimize performance by avoiding unnecessary recalculations and
//! callback recreations.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_callback, use_memo};
//!
//! #[component]
//! fn ExpensiveComponent(items: Vec<String>) -> Element {
//!     // Memoize an expensive computation
//!     let sorted_items = use_memo(|| {
//!         let mut sorted = items.clone();
//!         sorted.sort();
//!         sorted
//!     }, Some(items.clone()));
//!
//!     // Memoize a callback to prevent child re-renders
//!     let on_click = use_callback(|id: usize| {
//!         println!("Clicked item {}", id);
//!     }, None::<()>); // Empty deps = stable callback
//!
//!     rsx! { /* render */ }
//! }
//! ```

use std::sync::Arc;

use crate::fiber_tree::with_current_fiber;

/// Internal storage for memoized callbacks.
///
/// Stores the callback wrapped in Arc for pointer equality checks,
/// along with the previous dependencies for comparison.
struct CallbackStorage<F, Deps> {
    /// The memoized callback wrapped in Arc for stable reference
    callback: Arc<F>,
    /// Previous dependencies for comparison (None means no deps / always stable)
    prev_deps: Option<Deps>,
}

impl<F: Clone, Deps: Clone> Clone for CallbackStorage<F, Deps> {
    fn clone(&self) -> Self {
        Self {
            callback: self.callback.clone(),
            prev_deps: self.prev_deps.clone(),
        }
    }
}

/// Internal storage for memoized values.
struct MemoStorage<T, Deps> {
    /// The memoized value
    value: T,
    /// Previous dependencies for comparison
    prev_deps: Option<Deps>,
}

impl<T: Clone, Deps: Clone> Clone for MemoStorage<T, Deps> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            prev_deps: self.prev_deps.clone(),
        }
    }
}

/// React-style useCallback with dependency tracking.
///
/// Returns a memoized callback that only changes when dependencies change.
/// This is useful for optimizing child components that rely on reference equality
/// to prevent unnecessary re-renders.
///
/// # How It Works
///
/// 1. On first render, wraps the callback in an Arc and stores it
/// 2. On subsequent renders, compares dependencies:
///    - If deps changed: creates new Arc with new callback
///    - If deps unchanged: returns same Arc (pointer-equal)
/// 3. The returned `Arc<F>` can be compared by pointer for equality
///
/// # Arguments
///
/// * `callback` - The callback function to memoize
/// * `deps` - Dependencies that trigger callback recreation when changed.
///   - `Some(deps)` - Recreate when deps change
///   - `None` - Never recreate (stable callback)
///
/// # Returns
///
/// An `Arc<F>` wrapping the callback. The Arc reference is stable when deps
/// are unchanged, allowing pointer equality checks.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_callback;
///
/// #[component]
/// fn Parent() -> Element {
///     let (count, set_count) = use_state(|| 0);
///
///     // Callback that depends on count - recreated when count changes
///     let log_count = use_callback(move |_: ()| {
///         println!("Count is: {}", count);
///     }, Some(count));
///
///     // Stable callback - never recreated
///     let on_reset = use_callback(move |_: ()| {
///         set_count.set(0);
///     }, None::<()>);
///
///     rsx! {
///         <Child on_click={log_count} />
///         <Button on_click={on_reset} label="Reset" />
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_callback<F, Deps>(callback: F, deps: Option<Deps>) -> Arc<F>
where
    F: Clone + Send + Sync + 'static,
    Deps: Clone + PartialEq + Send + 'static,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Check if storage already exists
        let existing_storage: Option<CallbackStorage<F, Deps>> = fiber.get_hook(hook_index);

        let storage = if let Some(mut storage) = existing_storage {
            // Check if dependencies changed
            let deps_changed = match (&storage.prev_deps, &deps) {
                (None, None) => false,                     // Both None = stable, no change
                (Some(_), None) | (None, Some(_)) => true, // One is None, other isn't = changed
                (Some(prev), Some(curr)) => prev != curr,  // Compare values
            };

            if deps_changed {
                // Dependencies changed - create new callback
                storage.callback = Arc::new(callback);
                storage.prev_deps = deps;
                fiber.set_hook(hook_index, storage.clone());
            }

            storage
        } else {
            // First render - create new storage
            let new_storage = CallbackStorage {
                callback: Arc::new(callback),
                prev_deps: deps,
            };
            fiber.set_hook(hook_index, new_storage.clone());
            new_storage
        };

        storage.callback
    })
    .expect("use_callback must be called within a component render context")
}

/// React-style useMemo for memoizing computed values.
///
/// Returns a memoized value that only recomputes when dependencies change.
/// This is useful for expensive computations that shouldn't run on every render.
///
/// # How It Works
///
/// 1. On first render, calls the compute function and stores the result
/// 2. On subsequent renders, compares dependencies:
///    - If deps changed: recomputes and stores new value
///    - If deps unchanged: returns cached value
///
/// # Arguments
///
/// * `compute` - Function that computes the value to memoize
/// * `deps` - Dependencies that trigger recomputation when changed.
///   - `Some(deps)` - Recompute when deps change
///   - `None` - Never recompute (compute once)
///
/// # Returns
///
/// The memoized value (cloned from storage).
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_memo;
///
/// #[component]
/// fn FilteredList(items: Vec<Item>, filter: String) -> Element {
///     // Only recompute when items or filter change
///     let filtered = use_memo(|| {
///         items.iter()
///             .filter(|item| item.name.contains(&filter))
///             .cloned()
///             .collect::<Vec<_>>()
///     }, Some((items.clone(), filter.clone())));
///
///     rsx! {
///         <List items={filtered} />
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_memo<T, Deps, F>(compute: F, deps: Option<Deps>) -> T
where
    T: Clone + Send + 'static,
    Deps: Clone + PartialEq + Send + 'static,
    F: FnOnce() -> T,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Check if storage already exists
        let existing_storage: Option<MemoStorage<T, Deps>> = fiber.get_hook(hook_index);

        let storage = if let Some(mut storage) = existing_storage {
            // Check if dependencies changed
            let deps_changed = match (&storage.prev_deps, &deps) {
                (None, None) => false,                     // Both None = stable, no change
                (Some(_), None) | (None, Some(_)) => true, // One is None, other isn't = changed
                (Some(prev), Some(curr)) => prev != curr,  // Compare values
            };

            if deps_changed {
                // Dependencies changed - recompute
                storage.value = compute();
                storage.prev_deps = deps;
                fiber.set_hook(hook_index, storage.clone());
            }

            storage
        } else {
            // First render - compute and store
            let new_storage = MemoStorage {
                value: compute(),
                prev_deps: deps,
            };
            fiber.set_hook(hook_index, new_storage.clone());
            new_storage
        };

        storage.value
    })
    .expect("use_memo must be called within a component render context")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};

    fn setup_test_fiber() -> FiberId {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        clear_fiber_tree();
    }

    // ==================== use_callback tests ====================

    #[test]
    fn test_use_callback_basic() {
        let _fiber_id = setup_test_fiber();

        let callback = use_callback(|x: i32| x * 2, None::<()>);
        assert_eq!(callback(5), 10);

        cleanup_test();
    }

    #[test]
    fn test_use_callback_stable_with_none_deps() {
        let fiber_id = setup_test_fiber();

        // Use a named function to ensure same type across renders
        fn add_one(x: i32) -> i32 {
            x + 1
        }

        // First render
        let callback1 = use_callback(add_one, None::<()>);
        let ptr1 = Arc::as_ptr(&callback1);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render with same function type (but None deps = stable)
        let callback2 = use_callback(add_one, None::<()>);
        let ptr2 = Arc::as_ptr(&callback2);

        // Should be pointer-equal (stable)
        assert_eq!(ptr1, ptr2, "Callback with None deps should be stable");

        // Should still use the original callback
        assert_eq!(callback2(5), 6);

        cleanup_test();
    }

    #[test]
    fn test_use_callback_recreated_on_dep_change() {
        let fiber_id = setup_test_fiber();

        fn multiply(x: i32) -> i32 {
            x * 2
        }

        // First render with dep = 1
        let callback1 = use_callback(multiply, Some(1));
        let ptr1 = Arc::as_ptr(&callback1);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render with dep = 2 (changed)
        let callback2 = use_callback(multiply, Some(2));
        let ptr2 = Arc::as_ptr(&callback2);

        // Should NOT be pointer-equal (deps changed)
        assert_ne!(ptr1, ptr2, "Callback should be recreated when deps change");

        cleanup_test();
    }

    #[test]
    fn test_use_callback_stable_with_same_deps() {
        let fiber_id = setup_test_fiber();

        fn double(x: i32) -> i32 {
            x * 2
        }

        // First render with dep = 42
        let callback1 = use_callback(double, Some(42));
        let ptr1 = Arc::as_ptr(&callback1);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render with same dep = 42
        let callback2 = use_callback(double, Some(42));
        let ptr2 = Arc::as_ptr(&callback2);

        // Should be pointer-equal (deps unchanged)
        assert_eq!(ptr1, ptr2, "Callback should be stable when deps unchanged");

        // Should still use the original callback
        assert_eq!(callback2(5), 10);

        cleanup_test();
    }

    #[test]
    fn test_use_callback_with_tuple_deps() {
        let fiber_id = setup_test_fiber();

        fn noop(_: ()) -> i32 {
            1
        }

        // First render
        let callback1 = use_callback(noop, Some((1, "hello")));
        let ptr1 = Arc::as_ptr(&callback1);

        // Re-render with same deps
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        let callback2 = use_callback(noop, Some((1, "hello")));
        let ptr2 = Arc::as_ptr(&callback2);

        assert_eq!(ptr1, ptr2, "Same tuple deps should be stable");

        // Re-render with different deps
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        let callback3 = use_callback(noop, Some((2, "hello")));
        let ptr3 = Arc::as_ptr(&callback3);

        assert_ne!(ptr2, ptr3, "Different tuple deps should recreate");

        cleanup_test();
    }

    #[test]
    fn test_multiple_callbacks() {
        let _fiber_id = setup_test_fiber();

        let cb1 = use_callback(|x: i32| x + 1, None::<()>);
        let cb2 = use_callback(|x: i32| x * 2, None::<()>);
        let cb3 = use_callback(|s: &str| s.len(), None::<()>);

        assert_eq!(cb1(5), 6);
        assert_eq!(cb2(5), 10);
        assert_eq!(cb3("hello"), 5);

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_callback must be called within a component render context")]
    fn test_use_callback_panics_outside_render() {
        clear_fiber_tree();
        let _ = use_callback(|_: ()| {}, None::<()>);
    }

    // ==================== use_memo tests ====================

    #[test]
    fn test_use_memo_basic() {
        let _fiber_id = setup_test_fiber();

        let value = use_memo(|| 42, None::<()>);
        assert_eq!(value, 42);

        cleanup_test();
    }

    #[test]
    fn test_use_memo_stable_with_none_deps() {
        let fiber_id = setup_test_fiber();

        use std::sync::atomic::{AtomicUsize, Ordering};
        let compute_count = Arc::new(AtomicUsize::new(0));

        // First render
        let cc1 = compute_count.clone();
        let value1 = use_memo(
            move || {
                cc1.fetch_add(1, Ordering::SeqCst);
                100
            },
            None::<()>,
        );

        assert_eq!(value1, 100);
        assert_eq!(compute_count.load(Ordering::SeqCst), 1);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render - should NOT recompute
        let cc2 = compute_count.clone();
        let value2 = use_memo(
            move || {
                cc2.fetch_add(1, Ordering::SeqCst);
                200
            },
            None::<()>,
        );

        assert_eq!(value2, 100); // Still 100, not 200
        assert_eq!(compute_count.load(Ordering::SeqCst), 1); // Still 1, not recomputed

        cleanup_test();
    }

    #[test]
    fn test_use_memo_recomputes_on_dep_change() {
        let fiber_id = setup_test_fiber();

        // First render with dep = 1
        let value1 = use_memo(|| "first", Some(1));
        assert_eq!(value1, "first");

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render with dep = 2 (changed)
        let value2 = use_memo(|| "second", Some(2));
        assert_eq!(value2, "second"); // Recomputed

        cleanup_test();
    }

    #[test]
    fn test_use_memo_stable_with_same_deps() {
        let fiber_id = setup_test_fiber();

        // First render
        let _value1 = use_memo(|| vec![1, 2, 3], Some("key"));

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render with same deps
        let value2 = use_memo(|| vec![4, 5, 6], Some("key"));

        // Should return cached value
        assert_eq!(value2, vec![1, 2, 3]);

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_memo must be called within a component render context")]
    fn test_use_memo_panics_outside_render() {
        clear_fiber_tree();
        let _ = use_memo(|| 42, None::<()>);
    }
}
