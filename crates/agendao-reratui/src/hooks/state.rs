//! State hook with batching support.
//!
//! This module provides React-like `use_state` hook with proper batching semantics.
//! Multiple state updates within the same event handler are batched into a single re-render.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::use_state;
//!
//! #[component]
//! fn Counter() -> Element {
//!     let (count, set_count) = use_state(|| 0);
//!
//!     // Multiple updates are batched - only ONE re-render
//!     let increment_by_5 = move |_| {
//!         set_count.update(|n| n + 1);
//!         set_count.update(|n| n + 1);
//!         set_count.update(|n| n + 1);
//!         set_count.update(|n| n + 1);
//!         set_count.update(|n| n + 1);
//!     };
//!
//!     rsx! { <Text text={count.to_string()} /> }
//! }
//! ```

use std::marker::PhantomData;

use crate::fiber::FiberId;
use crate::fiber_tree::with_current_fiber;
use crate::scheduler::batch::{StateUpdate, StateUpdateKind, queue_update};

/// State setter that queues updates for batching.
///
/// This struct is returned by `use_state` and provides methods to update state.
/// Updates are queued and batched, not applied immediately.
#[derive(Debug)]
pub struct StateSetter<T> {
    pub(crate) fiber_id: FiberId,
    pub(crate) hook_index: usize,
    pub(crate) _marker: PhantomData<T>,
}

impl<T> Clone for StateSetter<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StateSetter<T> {}

impl<T: Clone + Send + 'static> StateSetter<T> {
    /// Set the state to a new value (queued for batching).
    ///
    /// The update is queued and will be applied when the batch ends.
    /// Multiple calls to `set` within the same event handler are batched.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (count, set_count) = use_state(|| 0);
    /// set_count.set(42); // Queued, not applied immediately
    /// ```
    pub fn set(&self, new_value: T) {
        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::Value(Box::new(new_value)),
            },
        );
    }

    /// Update the state using a function (queued for batching).
    ///
    /// The updater function receives the current state value and returns the new value.
    /// This is useful when the new state depends on the previous state.
    ///
    /// When multiple functional updates are queued, they are applied in order,
    /// each receiving the result of the previous update.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (count, set_count) = use_state(|| 0);
    ///
    /// // These are batched and applied in order
    /// set_count.update(|n| n + 1); // 0 -> 1
    /// set_count.update(|n| n + 1); // 1 -> 2
    /// set_count.update(|n| n * 2); // 2 -> 4
    /// ```
    pub fn update<F>(&self, updater: F)
    where
        F: FnOnce(&T) -> T + Send + 'static,
    {
        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::Updater(Box::new(move |any| {
                    let current = any.downcast_ref::<T>().expect("State type mismatch");
                    Box::new(updater(current))
                })),
            },
        );
    }
}

impl<T: Clone + Send + PartialEq + 'static> StateSetter<T> {
    /// Set the state to a new value only if it differs from the current value.
    ///
    /// This is an optimization that skips marking the fiber as dirty if the
    /// new value equals the current value, preventing unnecessary re-renders.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (count, set_count) = use_state(|| 0);
    ///
    /// // This will NOT trigger a re-render if count is already 0
    /// set_count.set_if_changed(0);
    ///
    /// // This WILL trigger a re-render
    /// set_count.set_if_changed(42);
    /// ```
    pub fn set_if_changed(&self, new_value: T) {
        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::ValueIfChanged {
                    value: Box::new(new_value),
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<T>().expect("State type mismatch");
                        let new = new.downcast_ref::<T>().expect("State type mismatch");
                        old == new
                    }),
                },
            },
        );
    }

    /// Update the state using a function, only if the result differs from the current value.
    ///
    /// This is an optimization that skips marking the fiber as dirty if the
    /// updater returns a value equal to the current value, preventing unnecessary re-renders.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (count, set_count) = use_state(|| 5);
    ///
    /// // This will NOT trigger a re-render because 5.max(3) == 5
    /// set_count.update_if_changed(|n| n.max(3));
    ///
    /// // This WILL trigger a re-render because 5.max(10) == 10
    /// set_count.update_if_changed(|n| n.max(10));
    /// ```
    pub fn update_if_changed<F>(&self, updater: F)
    where
        F: FnOnce(&T) -> T + Send + 'static,
    {
        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::UpdaterIfChanged {
                    updater: Box::new(move |any| {
                        let current = any.downcast_ref::<T>().expect("State type mismatch");
                        Box::new(updater(current))
                    }),
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<T>().expect("State type mismatch");
                        let new = new.downcast_ref::<T>().expect("State type mismatch");
                        old == new
                    }),
                },
            },
        );
    }
}

/// React-style useState with batching support.
///
/// Returns a tuple of the current state value and a setter to update it.
/// State updates are batched within event handlers for optimal performance.
///
/// # Features
///
/// - State updates are batched within event handlers
/// - Functional updates receive the latest state value
/// - Fiber-scoped state (no global index collision)
///
/// # Arguments
///
/// * `initializer` - A function that returns the initial state value.
///   Only called on the first render.
///
/// # Returns
///
/// A tuple of `(current_value, setter)` where:
/// - `current_value` is the current state value (cloned)
/// - `setter` is a `StateSetter` that can be used to update the state
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_state;
///
/// #[component]
/// fn Counter() -> Element {
///     let (count, set_count) = use_state(|| 0);
///
///     let increment = move |_| {
///         set_count.update(|n| n + 1);
///     };
///
///     rsx! {
///         <Block>
///             <Text text={format!("Count: {}", count)} />
///             <Button label="+" on_click={increment} />
///         </Block>
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_state<T, F>(initializer: F) -> (T, StateSetter<T>)
where
    T: Clone + Send + 'static,
    F: FnOnce() -> T,
{
    with_current_fiber(|fiber| {
        fiber.track_hook_call("use_state");
        let hook_index = fiber.next_hook_index();

        // Get or initialize state
        let value = fiber.get_or_init_hook(hook_index, initializer);

        let setter = StateSetter {
            fiber_id: fiber.id,
            hook_index,
            _marker: PhantomData,
        };

        (value, setter)
    })
    .expect("use_state must be called within a component render context")
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
        crate::scheduler::batch::clear_state_batch();
    }

    #[test]
    fn test_use_state_initial_value() {
        let _fiber_id = setup_test_fiber();

        let (value, _setter) = use_state(|| 42);
        assert_eq!(value, 42);

        cleanup_test();
    }

    #[test]
    fn test_use_state_returns_same_value_on_rerender() {
        let fiber_id = setup_test_fiber();

        // First render
        let (value1, _setter1) = use_state(|| 100);
        assert_eq!(value1, 100);

        // Simulate re-render by resetting hook index
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render - should return same value, not call initializer again
        let (value2, _setter2) = use_state(|| 999);
        assert_eq!(value2, 100); // Still 100, not 999

        cleanup_test();
    }

    #[test]
    fn test_state_setter_set_queues_update() {
        let fiber_id = setup_test_fiber();

        let (_value, setter) = use_state(|| 0);

        // Set should queue an update
        setter.set(42);

        // Verify update was queued
        let has_updates =
            crate::scheduler::batch::with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_updates);

        // Verify fiber is marked dirty
        let is_dirty =
            crate::scheduler::batch::with_state_batch(|batch| batch.is_fiber_dirty(fiber_id));
        assert!(is_dirty);

        cleanup_test();
    }

    #[test]
    fn test_state_setter_update_queues_functional_update() {
        let fiber_id = setup_test_fiber();

        let (_value, setter) = use_state(|| 10);

        // Update should queue a functional update
        setter.update(|n| n + 5);

        // Verify update was queued
        let has_updates =
            crate::scheduler::batch::with_state_batch(|batch| batch.has_pending_updates());
        assert!(has_updates);

        // Verify fiber is marked dirty
        let is_dirty =
            crate::scheduler::batch::with_state_batch(|batch| batch.is_fiber_dirty(fiber_id));
        assert!(is_dirty);

        cleanup_test();
    }

    #[test]
    fn test_multiple_state_hooks() {
        let _fiber_id = setup_test_fiber();

        let (count, _set_count) = use_state(|| 0);
        let (name, _set_name) = use_state(|| "Alice".to_string());
        let (active, _set_active) = use_state(|| true);

        assert_eq!(count, 0);
        assert_eq!(name, "Alice");
        assert!(active);

        cleanup_test();
    }

    #[test]
    fn test_state_setter_is_copy() {
        let _fiber_id = setup_test_fiber();

        let (_value, setter) = use_state(|| 0);

        // StateSetter should be Copy
        let setter_copy = setter;
        let _setter_copy2 = setter_copy;
        let _setter_copy3 = setter;

        cleanup_test();
    }

    #[test]
    fn test_batched_updates_applied_correctly() {
        let fiber_id = setup_test_fiber();

        // Initialize state
        let (_value, setter) = use_state(|| 0);

        // Queue multiple updates
        setter.set(10);
        setter.update(|n| n + 5);
        setter.update(|n| n * 2);

        // End render and apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            // Apply the batch
            let dirty =
                crate::scheduler::batch::with_state_batch_mut(|batch| batch.end_batch(tree));

            assert!(dirty.contains(&fiber_id));

            // Check final value: 0 -> 10 -> 15 -> 30
            let fiber = tree.get(fiber_id).unwrap();
            let final_value = fiber.get_hook::<i32>(0);
            assert_eq!(final_value, Some(30));
        });

        cleanup_test();
    }

    #[test]
    fn test_functional_updates_receive_latest_state() {
        let fiber_id = setup_test_fiber();

        // Initialize state
        let (_value, setter) = use_state(|| 0);

        // Queue multiple functional updates
        for _ in 0..5 {
            setter.update(|n| n + 1);
        }

        // End render and apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });

            // Each update should have received the latest state
            let fiber = tree.get(fiber_id).unwrap();
            let final_value = fiber.get_hook::<i32>(0);
            assert_eq!(final_value, Some(5));
        });

        cleanup_test();
    }

    #[test]
    fn test_state_setter_debug() {
        let _fiber_id = setup_test_fiber();

        let (_value, setter) = use_state(|| 0);

        // StateSetter should implement Debug
        let debug_str = format!("{:?}", setter);
        assert!(debug_str.contains("StateSetter"));

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_state must be called within a component render context")]
    fn test_use_state_panics_outside_render() {
        // Clear any existing fiber tree
        clear_fiber_tree();
        crate::scheduler::batch::clear_state_batch();

        // This should panic because there's no current fiber
        let _ = use_state(|| 0);
    }

    #[test]
    fn test_set_if_changed_skips_equal_value() {
        let fiber_id = setup_test_fiber();

        // Initialize state with value 42
        let (_value, setter) = use_state(|| 42);

        // End render to apply initial state
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.mark_clean(fiber_id);
            tree.begin_render(fiber_id);
        });

        // Clear any pending updates
        crate::scheduler::batch::clear_state_batch();

        // Set to same value - should not mark dirty
        setter.set_if_changed(42);

        // Apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            let dirty =
                crate::scheduler::batch::with_state_batch_mut(|batch| batch.end_batch(tree));

            // Fiber should NOT be dirty because value didn't change
            assert!(!dirty.contains(&fiber_id));
            assert!(!tree.get(fiber_id).unwrap().dirty);
        });

        cleanup_test();
    }

    #[test]
    fn test_set_if_changed_updates_different_value() {
        let fiber_id = setup_test_fiber();

        // Initialize state with value 42
        let (_value, setter) = use_state(|| 42);

        // End render to apply initial state
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.mark_clean(fiber_id);
            tree.begin_render(fiber_id);
        });

        // Clear any pending updates
        crate::scheduler::batch::clear_state_batch();

        // Set to different value - should mark dirty
        setter.set_if_changed(100);

        // Apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            let dirty =
                crate::scheduler::batch::with_state_batch_mut(|batch| batch.end_batch(tree));

            // Fiber SHOULD be dirty because value changed
            assert!(dirty.contains(&fiber_id));
            assert!(tree.get(fiber_id).unwrap().dirty);
            assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(100));
        });

        cleanup_test();
    }

    #[test]
    fn test_update_if_changed_skips_equal_result() {
        let fiber_id = setup_test_fiber();

        // Initialize state with value 5
        let (_value, setter) = use_state(|| 5);

        // End render to apply initial state
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.mark_clean(fiber_id);
            tree.begin_render(fiber_id);
        });

        // Clear any pending updates
        crate::scheduler::batch::clear_state_batch();

        // Update with function that returns same value: 5.max(3) = 5
        setter.update_if_changed(|n| (*n).max(3));

        // Apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            let dirty =
                crate::scheduler::batch::with_state_batch_mut(|batch| batch.end_batch(tree));

            // Fiber should NOT be dirty because result equals current
            assert!(!dirty.contains(&fiber_id));
            assert!(!tree.get(fiber_id).unwrap().dirty);
            assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(5));
        });

        cleanup_test();
    }

    #[test]
    fn test_update_if_changed_updates_different_result() {
        let fiber_id = setup_test_fiber();

        // Initialize state with value 5
        let (_value, setter) = use_state(|| 5);

        // End render to apply initial state
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.mark_clean(fiber_id);
            tree.begin_render(fiber_id);
        });

        // Clear any pending updates
        crate::scheduler::batch::clear_state_batch();

        // Update with function that returns different value: 5.max(10) = 10
        setter.update_if_changed(|n| (*n).max(10));

        // Apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            let dirty =
                crate::scheduler::batch::with_state_batch_mut(|batch| batch.end_batch(tree));

            // Fiber SHOULD be dirty because result differs
            assert!(dirty.contains(&fiber_id));
            assert!(tree.get(fiber_id).unwrap().dirty);
            assert_eq!(tree.get(fiber_id).unwrap().get_hook::<i32>(0), Some(10));
        });

        cleanup_test();
    }
}
