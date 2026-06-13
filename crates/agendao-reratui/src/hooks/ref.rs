//! Ref hook for mutable values that persist across renders without triggering re-renders.
//!
//! This module provides React-like `use_ref` hook with proper fiber-based semantics.
//! Unlike `use_state`, mutations to refs do NOT trigger re-renders.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::use_ref;
//!
//! #[component]
//! fn Counter() -> Element {
//!     let render_count = use_ref(|| 0);
//!
//!     // This mutation does NOT trigger a re-render
//!     render_count.update(|n| *n += 1);
//!
//!     rsx! { <Text text={format!("Rendered {} times", render_count.get())} /> }
//! }
//! ```

use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use crate::fiber::FiberId;
use crate::fiber_tree::with_current_fiber;

/// Internal storage for ref values using Arc<RwLock<T>> for thread-safe access.
#[derive(Debug)]
pub(crate) struct RefStorage<T> {
    pub(crate) value: Arc<RwLock<T>>,
}

impl<T> Clone for RefStorage<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

/// A mutable reference that persists across renders without triggering re-renders.
///
/// This struct is returned by `use_ref` and provides methods to access and mutate
/// the stored value. Unlike `StateSetter`, mutations do NOT mark the fiber as dirty
/// and do NOT trigger re-renders.
///
/// # Thread Safety
///
/// `Ref` is thread-safe and can be safely shared across async tasks.
/// It uses `Arc<RwLock<T>>` internally for concurrent access.
#[derive(Debug)]
pub struct Ref<T> {
    pub(crate) fiber_id: FiberId,
    pub(crate) hook_index: usize,
    pub(crate) storage: Arc<RwLock<T>>,
    pub(crate) _marker: PhantomData<T>,
}

impl<T> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Self {
            fiber_id: self.fiber_id,
            hook_index: self.hook_index,
            storage: self.storage.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: Clone + Send + 'static> Ref<T> {
    /// Get a clone of the current value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let count_ref = use_ref(|| 0);
    /// let current_count = count_ref.get(); // Returns 0
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn get(&self) -> T {
        self.storage.read().expect("Ref lock poisoned").clone()
    }

    /// Set a new value (does NOT trigger re-render).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let count_ref = use_ref(|| 0);
    /// count_ref.set(42); // Does NOT trigger re-render
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn set(&self, value: T) {
        let mut guard = self.storage.write().expect("Ref lock poisoned");
        *guard = value;
    }

    /// Update the value using a mutable closure (does NOT trigger re-render).
    ///
    /// This is useful for complex mutations without needing to clone the value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let count_ref = use_ref(|| 0);
    /// count_ref.update(|n| *n += 1); // Does NOT trigger re-render
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        let mut guard = self.storage.write().expect("Ref lock poisoned");
        f(&mut *guard);
    }

    /// Access the value immutably with a closure.
    ///
    /// This is useful for reading nested fields without cloning the entire value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let data_ref = use_ref(|| vec![1, 2, 3]);
    /// let len = data_ref.with(|v| v.len()); // Returns 3
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let guard = self.storage.read().expect("Ref lock poisoned");
        f(&*guard)
    }

    /// Access the value mutably with a closure (does NOT trigger re-render).
    ///
    /// This is useful for complex mutations that need to return a value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let data_ref = use_ref(|| vec![1, 2, 3]);
    /// let popped = data_ref.with_mut(|v| v.pop()); // Returns Some(3)
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut guard = self.storage.write().expect("Ref lock poisoned");
        f(&mut *guard)
    }

    /// Replace the value and return the old value (does NOT trigger re-render).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let count_ref = use_ref(|| 10);
    /// let old = count_ref.replace(20); // Returns 10
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn replace(&self, new_value: T) -> T {
        let mut guard = self.storage.write().expect("Ref lock poisoned");
        std::mem::replace(&mut *guard, new_value)
    }
}

impl<T: Clone + Send + Default + 'static> Ref<T> {
    /// Take the value, leaving `Default::default()` in its place (does NOT trigger re-render).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let data_ref = use_ref(|| vec![1, 2, 3]);
    /// let data = data_ref.take(); // Returns vec![1, 2, 3], ref now holds vec![]
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn take(&self) -> T {
        let mut guard = self.storage.write().expect("Ref lock poisoned");
        std::mem::take(&mut *guard)
    }
}

/// React-style useRef with fiber-based storage.
///
/// Returns a mutable reference that persists across component re-renders.
/// Unlike `use_state`, mutations to refs do NOT trigger re-renders, making them ideal for:
///
/// - **Storing mutable values**: Track values that change but don't affect rendering
/// - **Previous values**: Keep track of previous state or props
/// - **DOM-like references**: Store references to terminal widgets or areas
/// - **Timers and intervals**: Manage async handles without re-rendering
/// - **Caching**: Store expensive computation results
/// - **Instance variables**: Component-scoped mutable storage
///
/// # Thread Safety
///
/// The returned `Ref` is thread-safe and can be safely shared across async tasks.
///
/// ## Comparison with `use_state`
///
/// | Feature | `use_ref` | `use_state` |
/// |---------|--------------|----------------|
/// | Triggers re-render | ❌ No | ✅ Yes |
/// | Mutable access | ✅ Direct | ❌ Via setter |
/// | Use case | Side effects, caching | UI state |
/// | Performance | Faster (no re-render) | Slower (re-renders) |
///
/// # Arguments
///
/// * `initializer` - A function that returns the initial value.
///   Only called on the first render.
///
/// # Returns
///
/// A `Ref<T>` that provides methods to access and mutate the value.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_ref;
///
/// #[component]
/// fn Timer() -> Element {
///     let interval_id = use_ref(|| None::<IntervalHandle>);
///
///     use_effect(|| {
///         let handle = set_interval(|| { /* ... */ }, 1000);
///         interval_id.set(Some(handle));
///
///         // Cleanup
///         Some(Box::new(move || {
///             if let Some(handle) = interval_id.take() {
///                 handle.cancel();
///             }
///         }))
///     }, None::<()>);
///
///     rsx! { <Text text="Timer running..." /> }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_ref<T, F>(initializer: F) -> Ref<T>
where
    T: Clone + Send + Sync + 'static,
    F: FnOnce() -> T,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Get or initialize the ref storage
        let storage = fiber.get_or_init_hook(hook_index, || RefStorage {
            value: Arc::new(RwLock::new(initializer())),
        });

        Ref {
            fiber_id: fiber.id,
            hook_index,
            storage: storage.value,
            _marker: PhantomData,
        }
    })
    .expect("use_ref must be called within a component render context")
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_use_ref_initial_value() {
        let _fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| 42);
        assert_eq!(ref_handle.get(), 42);

        cleanup_test();
    }

    #[test]
    fn test_use_ref_returns_same_value_on_rerender() {
        let fiber_id = setup_test_fiber();

        // First render
        let ref_handle1 = use_ref(|| 100);
        assert_eq!(ref_handle1.get(), 100);

        // Simulate re-render by resetting hook index
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render - should return same value, not call initializer again
        let ref_handle2 = use_ref(|| 999);
        assert_eq!(ref_handle2.get(), 100); // Still 100, not 999

        cleanup_test();
    }

    #[test]
    fn test_ref_set_does_not_trigger_rerender() {
        let fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| 0);

        // End render and mark clean
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.mark_clean(fiber_id);
        });

        // Set should NOT mark fiber dirty
        ref_handle.set(42);

        // Verify fiber is NOT dirty
        let is_dirty = crate::fiber_tree::with_fiber_tree(|tree| {
            tree.get(fiber_id).map(|f| f.dirty).unwrap_or(false)
        })
        .unwrap_or(false);

        assert!(!is_dirty, "Ref mutation should NOT mark fiber as dirty");
        assert_eq!(ref_handle.get(), 42);

        cleanup_test();
    }

    #[test]
    fn test_ref_update_does_not_trigger_rerender() {
        let fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| 10);

        // End render and mark clean
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.mark_clean(fiber_id);
        });

        // Update should NOT mark fiber dirty
        ref_handle.update(|n| *n += 5);

        // Verify fiber is NOT dirty
        let is_dirty = crate::fiber_tree::with_fiber_tree(|tree| {
            tree.get(fiber_id).map(|f| f.dirty).unwrap_or(false)
        })
        .unwrap_or(false);

        assert!(!is_dirty, "Ref mutation should NOT mark fiber as dirty");
        assert_eq!(ref_handle.get(), 15);

        cleanup_test();
    }

    #[test]
    fn test_ref_with_accessor() {
        let _fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| vec![1, 2, 3]);
        let len = ref_handle.with(|v| v.len());
        assert_eq!(len, 3);

        cleanup_test();
    }

    #[test]
    fn test_ref_with_mut_accessor() {
        let _fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| vec![1, 2, 3]);
        let popped = ref_handle.with_mut(|v| v.pop());
        assert_eq!(popped, Some(3));
        assert_eq!(ref_handle.get(), vec![1, 2]);

        cleanup_test();
    }

    #[test]
    fn test_ref_replace() {
        let _fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| 10);
        let old = ref_handle.replace(20);
        assert_eq!(old, 10);
        assert_eq!(ref_handle.get(), 20);

        cleanup_test();
    }

    #[test]
    fn test_ref_take() {
        let _fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| vec![1, 2, 3]);
        let taken = ref_handle.take();
        assert_eq!(taken, vec![1, 2, 3]);
        assert_eq!(ref_handle.get(), Vec::<i32>::new());

        cleanup_test();
    }

    #[test]
    fn test_multiple_refs() {
        let _fiber_id = setup_test_fiber();

        let ref1 = use_ref(|| 1);
        let ref2 = use_ref(|| "hello".to_string());
        let ref3 = use_ref(|| vec![true, false]);

        assert_eq!(ref1.get(), 1);
        assert_eq!(ref2.get(), "hello");
        assert_eq!(ref3.get(), vec![true, false]);

        cleanup_test();
    }

    #[test]
    fn test_ref_is_clone() {
        let _fiber_id = setup_test_fiber();

        let ref_handle = use_ref(|| 42);
        let ref_clone = ref_handle.clone();

        // Both should point to the same storage
        ref_handle.set(100);
        assert_eq!(ref_clone.get(), 100);

        cleanup_test();
    }

    #[test]
    fn test_ref_persists_across_multiple_renders() {
        let fiber_id = setup_test_fiber();

        // First render
        let ref_handle = use_ref(|| 0);
        ref_handle.set(42);

        // Simulate multiple re-renders
        for i in 1..=5 {
            crate::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.end_render();
                tree.begin_render(fiber_id);
            });

            let ref_handle = use_ref(|| 999); // Initializer should be ignored
            assert_eq!(ref_handle.get(), 42, "Render {}: value should persist", i);
        }

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_ref must be called within a component render context")]
    fn test_use_ref_panics_outside_render() {
        clear_fiber_tree();

        // This should panic because there's no current fiber
        let _ = use_ref(|| 0);
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use proptest::prelude::*;

    /// Test mutex to ensure tests run sequentially since they share global state
    static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // ============================================================
        // **Property 1: Ref value persistence across renders**
        // **Validates: Requirements 1.1, 1.3**
        //
        // For any ref created with `use_ref` and any sequence of mutations,
        // the ref value SHALL persist across simulated re-renders without loss.
        // ============================================================

        #[test]
        fn prop_ref_value_persistence_across_renders(
            initial_value in any::<i32>(),
            mutations in prop::collection::vec(any::<i32>(), 0..10),
            num_renders in 1usize..10
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render - create ref with initial value
            let ref_handle = use_ref(|| initial_value);
            prop_assert_eq!(ref_handle.get(), initial_value, "Initial value should match");

            // Apply mutations
            let mut expected_value = initial_value;
            for mutation in &mutations {
                ref_handle.set(*mutation);
                expected_value = *mutation;
            }

            // Simulate multiple re-renders
            for render_num in 1..=num_renders {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                // Get ref again (initializer should be ignored)
                let ref_handle = use_ref(|| 999999);

                // Property: Value should persist across renders
                prop_assert_eq!(
                    ref_handle.get(),
                    expected_value,
                    "Render {}: value should persist (expected {}, got {})",
                    render_num,
                    expected_value,
                    ref_handle.get()
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_ref_update_persists_across_renders(
            initial_value in any::<i32>(),
            increments in prop::collection::vec(1i32..100, 1..10),
            num_renders in 1usize..5
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render - create ref
            let ref_handle = use_ref(|| initial_value);

            // Apply incremental updates
            let mut expected_value = initial_value;
            for inc in &increments {
                ref_handle.update(|n| *n += inc);
                expected_value += inc;
            }

            prop_assert_eq!(ref_handle.get(), expected_value, "Value after updates should match");

            // Simulate re-renders and verify persistence
            for _ in 1..=num_renders {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                let ref_handle = use_ref(|| 0);
                prop_assert_eq!(
                    ref_handle.get(),
                    expected_value,
                    "Value should persist after re-render"
                );
            }

            cleanup_test();
        }

        // ============================================================
        // **Property 2: Ref mutations do not trigger re-renders**
        // **Validates: Requirements 1.2**
        //
        // For any component using `use_ref`, when the ref value is mutated
        // via `set()` or `update()`, the fiber SHALL NOT be marked as dirty.
        // ============================================================

        #[test]
        fn prop_ref_set_does_not_mark_dirty(
            initial_value in any::<i32>(),
            new_values in prop::collection::vec(any::<i32>(), 1..20)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            let ref_handle = use_ref(|| initial_value);

            // End render and mark fiber as clean
            with_fiber_tree_mut(|tree| {
                tree.end_render();
                tree.mark_clean(fiber_id);
            });

            // Apply multiple set mutations
            for new_value in &new_values {
                ref_handle.set(*new_value);

                // Property: Fiber should NOT be dirty after set
                let is_dirty = crate::fiber_tree::with_fiber_tree(|tree| {
                    tree.get(fiber_id).map(|f| f.dirty).unwrap_or(true)
                })
                .unwrap_or(true);

                prop_assert!(
                    !is_dirty,
                    "Fiber should NOT be dirty after ref.set({})",
                    new_value
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_ref_update_does_not_mark_dirty(
            initial_value in any::<i32>(),
            num_updates in 1usize..20
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            let ref_handle = use_ref(|| initial_value);

            // End render and mark fiber as clean
            with_fiber_tree_mut(|tree| {
                tree.end_render();
                tree.mark_clean(fiber_id);
            });

            // Apply multiple update mutations
            for i in 0..num_updates {
                ref_handle.update(|n| *n += 1);

                // Property: Fiber should NOT be dirty after update
                let is_dirty = crate::fiber_tree::with_fiber_tree(|tree| {
                    tree.get(fiber_id).map(|f| f.dirty).unwrap_or(true)
                })
                .unwrap_or(true);

                prop_assert!(
                    !is_dirty,
                    "Fiber should NOT be dirty after update #{} (ref.update)",
                    i + 1
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_ref_with_mut_does_not_mark_dirty(
            initial_vec in prop::collection::vec(any::<i32>(), 0..10),
            num_mutations in 1usize..10
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            let ref_handle = use_ref(|| initial_vec);

            // End render and mark fiber as clean
            with_fiber_tree_mut(|tree| {
                tree.end_render();
                tree.mark_clean(fiber_id);
            });

            // Apply mutations using with_mut
            for i in 0..num_mutations {
                ref_handle.with_mut(|v| v.push(i as i32));

                // Property: Fiber should NOT be dirty after with_mut
                let is_dirty = crate::fiber_tree::with_fiber_tree(|tree| {
                    tree.get(fiber_id).map(|f| f.dirty).unwrap_or(true)
                })
                .unwrap_or(true);

                prop_assert!(
                    !is_dirty,
                    "Fiber should NOT be dirty after with_mut mutation #{}",
                    i + 1
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_ref_replace_does_not_mark_dirty(
            initial_value in any::<i32>(),
            replacements in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            let ref_handle = use_ref(|| initial_value);

            // End render and mark fiber as clean
            with_fiber_tree_mut(|tree| {
                tree.end_render();
                tree.mark_clean(fiber_id);
            });

            // Apply replace mutations
            for (i, new_value) in replacements.iter().enumerate() {
                let _old = ref_handle.replace(*new_value);

                // Property: Fiber should NOT be dirty after replace
                let is_dirty = crate::fiber_tree::with_fiber_tree(|tree| {
                    tree.get(fiber_id).map(|f| f.dirty).unwrap_or(true)
                })
                .unwrap_or(true);

                prop_assert!(
                    !is_dirty,
                    "Fiber should NOT be dirty after replace #{}",
                    i + 1
                );
            }

            cleanup_test();
        }

        // ============================================================
        // Additional property: Ref clone shares storage
        // ============================================================

        #[test]
        fn prop_ref_clone_shares_storage(
            initial_value in any::<i32>(),
            mutations in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let _fiber_id = setup_test_fiber();

            let ref_handle = use_ref(|| initial_value);
            let ref_clone = ref_handle.clone();

            // Property: Both handles should see the same value
            prop_assert_eq!(ref_handle.get(), ref_clone.get(), "Clone should have same initial value");

            // Apply mutations through original
            for mutation in &mutations {
                ref_handle.set(*mutation);

                // Property: Clone should see the mutation
                prop_assert_eq!(
                    ref_clone.get(),
                    *mutation,
                    "Clone should see mutation from original"
                );
            }

            // Apply mutation through clone
            ref_clone.set(12345);
            prop_assert_eq!(
                ref_handle.get(),
                12345,
                "Original should see mutation from clone"
            );

            cleanup_test();
        }
    }
}
