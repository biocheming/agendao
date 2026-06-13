//! History tracking hook with undo/redo support.
//!
//! This module provides a hook for tracking value history with undo/redo functionality,
//! similar to browser history or text editor undo stacks.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::use_history;
//!
//! #[component]
//! fn TextEditor() -> Element {
//!     let history = use_history(|| String::new());
//!     
//!     // Update text
//!     history.set("Hello".to_string());
//!     history.set("Hello World".to_string());
//!     
//!     // Undo to "Hello"
//!     history.undo();
//!     
//!     // Redo to "Hello World"
//!     history.redo();
//!     
//!     rsx! {
//!         <Text text={history.current()} />
//!         <Button disabled={!history.can_undo()} on_click={|| history.undo()}>Undo</Button>
//!         <Button disabled={!history.can_redo()} on_click={|| history.redo()}>Redo</Button>
//!     }
//! }
//! ```

use std::sync::{Arc, RwLock};

use crate::fiber::FiberId;
use crate::fiber_tree::with_current_fiber;
use crate::scheduler::batch::{StateUpdate, StateUpdateKind, queue_update};

/// Internal storage for history state.
///
/// Uses past/present/future stacks for undo/redo functionality.
#[derive(Clone)]
struct HistoryStorage<T> {
    /// Stack of past values (most recent at the end)
    past: Vec<T>,
    /// Current value
    present: T,
    /// Stack of future values for redo (most recent at the end)
    future: Vec<T>,
}

impl<T: Clone> HistoryStorage<T> {
    fn new(initial: T) -> Self {
        Self {
            past: Vec::new(),
            present: initial,
            future: Vec::new(),
        }
    }

    fn set(&mut self, value: T) {
        // Push current to past
        self.past.push(self.present.clone());
        // Set new present
        self.present = value;
        // Clear future (new branch)
        self.future.clear();
    }

    fn undo(&mut self) -> bool {
        if let Some(prev) = self.past.pop() {
            // Push current to future
            self.future.push(self.present.clone());
            // Restore previous
            self.present = prev;
            true
        } else {
            false
        }
    }

    fn redo(&mut self) -> bool {
        if let Some(next) = self.future.pop() {
            // Push current to past
            self.past.push(self.present.clone());
            // Restore next
            self.present = next;
            true
        } else {
            false
        }
    }
}

/// Shared storage wrapper for thread-safe access.
#[derive(Clone)]
struct SharedHistoryStorage<T> {
    inner: Arc<RwLock<HistoryStorage<T>>>,
}

impl<T: Clone> SharedHistoryStorage<T> {
    fn new(initial: T) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HistoryStorage::new(initial))),
        }
    }
}

/// Handle for interacting with history state.
///
/// Provides methods to get the current value, set new values,
/// and navigate through history with undo/redo.
#[derive(Clone)]
pub struct HistoryHandle<T> {
    fiber_id: FiberId,
    hook_index: usize,
    storage: SharedHistoryStorage<T>,
}

impl<T: Clone + Send + Sync + 'static> HistoryHandle<T> {
    /// Returns the current value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = use_history(|| 0);
    /// let value = history.current(); // 0
    /// ```
    pub fn current(&self) -> T {
        self.storage
            .inner
            .read()
            .expect("Failed to read history storage")
            .present
            .clone()
    }

    /// Sets a new value, pushing the current value to the past stack.
    ///
    /// This clears the future stack (redo history).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = use_history(|| 0);
    /// history.set(1);
    /// history.set(2);
    /// assert_eq!(history.current(), 2);
    /// assert!(history.can_undo());
    /// ```
    pub fn set(&self, value: T) {
        {
            let mut storage = self
                .storage
                .inner
                .write()
                .expect("Failed to write history storage");
            storage.set(value);
        }

        // Mark fiber as dirty to trigger re-render
        self.mark_dirty();
    }

    /// Undoes the last change, restoring the previous value.
    ///
    /// Returns `true` if undo was successful, `false` if there's nothing to undo.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = use_history(|| 0);
    /// history.set(1);
    /// history.undo(); // Returns true, current is now 0
    /// history.undo(); // Returns false, nothing to undo
    /// ```
    pub fn undo(&self) -> bool {
        let success = {
            let mut storage = self
                .storage
                .inner
                .write()
                .expect("Failed to write history storage");
            storage.undo()
        };

        if success {
            self.mark_dirty();
        }

        success
    }

    /// Redoes the last undone change.
    ///
    /// Returns `true` if redo was successful, `false` if there's nothing to redo.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = use_history(|| 0);
    /// history.set(1);
    /// history.undo();
    /// history.redo(); // Returns true, current is now 1
    /// history.redo(); // Returns false, nothing to redo
    /// ```
    pub fn redo(&self) -> bool {
        let success = {
            let mut storage = self
                .storage
                .inner
                .write()
                .expect("Failed to write history storage");
            storage.redo()
        };

        if success {
            self.mark_dirty();
        }

        success
    }

    /// Returns `true` if there are past values to undo to.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = use_history(|| 0);
    /// assert!(!history.can_undo()); // Nothing to undo yet
    /// history.set(1);
    /// assert!(history.can_undo()); // Can undo to 0
    /// ```
    pub fn can_undo(&self) -> bool {
        !self
            .storage
            .inner
            .read()
            .expect("Failed to read history storage")
            .past
            .is_empty()
    }

    /// Returns `true` if there are future values to redo to.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = use_history(|| 0);
    /// history.set(1);
    /// assert!(!history.can_redo()); // Nothing to redo
    /// history.undo();
    /// assert!(history.can_redo()); // Can redo to 1
    /// ```
    pub fn can_redo(&self) -> bool {
        !self
            .storage
            .inner
            .read()
            .expect("Failed to read history storage")
            .future
            .is_empty()
    }

    /// Returns the number of past values (undo stack size).
    pub fn past_count(&self) -> usize {
        self.storage
            .inner
            .read()
            .expect("Failed to read history storage")
            .past
            .len()
    }

    /// Returns the number of future values (redo stack size).
    pub fn future_count(&self) -> usize {
        self.storage
            .inner
            .read()
            .expect("Failed to read history storage")
            .future
            .len()
    }

    /// Clears all history, keeping only the current value.
    pub fn clear_history(&self) {
        let mut storage = self
            .storage
            .inner
            .write()
            .expect("Failed to write history storage");
        storage.past.clear();
        storage.future.clear();
    }

    /// Goes back multiple steps in history.
    ///
    /// Returns the number of steps actually taken.
    pub fn go_back(&self, steps: usize) -> usize {
        let mut taken = 0;
        for _ in 0..steps {
            if self.undo() {
                taken += 1;
            } else {
                break;
            }
        }
        taken
    }

    /// Goes forward multiple steps in history.
    ///
    /// Returns the number of steps actually taken.
    pub fn go_forward(&self, steps: usize) -> usize {
        let mut taken = 0;
        for _ in 0..steps {
            if self.redo() {
                taken += 1;
            } else {
                break;
            }
        }
        taken
    }

    fn mark_dirty(&self) {
        // Use the scheduler's queue_update to mark the fiber dirty
        // We use a no-op update that just returns the current storage
        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::Updater(Box::new(|any| {
                    // Just return the same value - we already updated the storage
                    let storage = any
                        .downcast_ref::<SharedHistoryStorage<T>>()
                        .expect("History storage type mismatch");
                    Box::new(storage.clone())
                })),
            },
        );
    }
}

/// React-style useHistory for tracking value history with undo/redo.
///
/// Returns a `HistoryHandle` that provides:
/// - `current()` - Get the current value
/// - `set(value)` - Set a new value (pushes current to history)
/// - `undo()` - Restore the previous value
/// - `redo()` - Restore the next value (after undo)
/// - `can_undo()` / `can_redo()` - Check if undo/redo is available
///
/// # How It Works
///
/// 1. Maintains three stacks: past, present, and future
/// 2. `set()` pushes current to past, sets new present, clears future
/// 3. `undo()` pushes current to future, pops past to present
/// 4. `redo()` pushes current to past, pops future to present
///
/// # Arguments
///
/// * `initializer` - Function that returns the initial value
///
/// # Returns
///
/// A `HistoryHandle<T>` for interacting with the history state.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_history;
///
/// #[component]
/// fn Counter() -> Element {
///     let history = use_history(|| 0);
///     
///     rsx! {
///         <Text text={format!("Count: {}", history.current())} />
///         <Button on_click={|| history.set(history.current() + 1)}>+1</Button>
///         <Button disabled={!history.can_undo()} on_click={|| history.undo()}>Undo</Button>
///         <Button disabled={!history.can_redo()} on_click={|| history.redo()}>Redo</Button>
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_history<T, F>(initializer: F) -> HistoryHandle<T>
where
    T: Clone + Send + Sync + 'static,
    F: FnOnce() -> T,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();
        let fiber_id = fiber.id;

        // Check if storage already exists
        let existing_storage: Option<SharedHistoryStorage<T>> = fiber.get_hook(hook_index);

        let storage = if let Some(storage) = existing_storage {
            storage
        } else {
            // First render - create new storage
            let storage = SharedHistoryStorage::new(initializer());
            fiber.set_hook(hook_index, storage.clone());
            storage
        };

        HistoryHandle {
            fiber_id,
            hook_index,
            storage,
        }
    })
    .expect("use_history must be called within a component render context")
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
        crate::scheduler::batch::clear_state_batch();
    }

    #[test]
    fn test_use_history_basic() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        assert_eq!(history.current(), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_history_set() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        history.set(1);
        assert_eq!(history.current(), 1);
        history.set(2);
        assert_eq!(history.current(), 2);

        cleanup_test();
    }

    #[test]
    fn test_use_history_undo() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        history.set(1);
        history.set(2);

        assert!(history.undo());
        assert_eq!(history.current(), 1);

        assert!(history.undo());
        assert_eq!(history.current(), 0);

        // Can't undo past initial
        assert!(!history.undo());
        assert_eq!(history.current(), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_history_redo() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        history.set(1);
        history.set(2);

        history.undo();
        history.undo();

        assert!(history.redo());
        assert_eq!(history.current(), 1);

        assert!(history.redo());
        assert_eq!(history.current(), 2);

        // Can't redo past latest
        assert!(!history.redo());
        assert_eq!(history.current(), 2);

        cleanup_test();
    }

    #[test]
    fn test_use_history_set_clears_future() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        history.set(1);
        history.set(2);

        history.undo(); // Back to 1
        assert!(history.can_redo());

        history.set(3); // New branch
        assert!(!history.can_redo()); // Future cleared
        assert_eq!(history.current(), 3);

        cleanup_test();
    }

    #[test]
    fn test_use_history_can_undo_redo() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);

        assert!(!history.can_undo());
        assert!(!history.can_redo());

        history.set(1);
        assert!(history.can_undo());
        assert!(!history.can_redo());

        history.undo();
        assert!(!history.can_undo());
        assert!(history.can_redo());

        cleanup_test();
    }

    #[test]
    fn test_use_history_past_future_count() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);

        assert_eq!(history.past_count(), 0);
        assert_eq!(history.future_count(), 0);

        history.set(1);
        history.set(2);
        history.set(3);

        assert_eq!(history.past_count(), 3);
        assert_eq!(history.future_count(), 0);

        history.undo();
        history.undo();

        assert_eq!(history.past_count(), 1);
        assert_eq!(history.future_count(), 2);

        cleanup_test();
    }

    #[test]
    fn test_use_history_clear_history() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        history.set(1);
        history.set(2);
        history.undo();

        history.clear_history();

        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.current(), 1); // Current value preserved

        cleanup_test();
    }

    #[test]
    fn test_use_history_go_back_forward() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(|| 0);
        history.set(1);
        history.set(2);
        history.set(3);
        history.set(4);

        assert_eq!(history.go_back(2), 2);
        assert_eq!(history.current(), 2);

        assert_eq!(history.go_forward(3), 2); // Only 2 steps available
        assert_eq!(history.current(), 4);

        cleanup_test();
    }

    #[test]
    fn test_use_history_stable_across_renders() {
        let fiber_id = setup_test_fiber();

        // First render
        let history1 = use_history(|| 0);
        history1.set(1);
        history1.set(2);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render
        let history2 = use_history(|| 999); // Different initializer ignored

        // Should have same state
        assert_eq!(history2.current(), 2);
        assert!(history2.can_undo());

        cleanup_test();
    }

    #[test]
    fn test_use_history_with_string() {
        let _fiber_id = setup_test_fiber();

        let history = use_history(String::new);
        history.set("Hello".to_string());
        history.set("Hello World".to_string());

        assert_eq!(history.current(), "Hello World");

        history.undo();
        assert_eq!(history.current(), "Hello");

        history.undo();
        assert_eq!(history.current(), "");

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_history must be called within a component render context")]
    fn test_use_history_panics_outside_render() {
        clear_fiber_tree();
        crate::scheduler::batch::clear_state_batch();
        let _ = use_history(|| 0);
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use proptest::prelude::*;

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

    // Property 15: History undo/redo round-trip
    //
    // For any history state with value V1, after set(V2) then undo(),
    // the current value SHALL equal V1, and after redo(), the current
    // value SHALL equal V2.
    //
    // Validates: Requirements 8.2, 8.3
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_history_undo_redo_round_trip(v1 in any::<i32>(), v2 in any::<i32>()) {
            let _fiber_id = setup_test_fiber();

            let history = use_history(|| v1);

            // Set to V2
            history.set(v2);
            prop_assert_eq!(history.current(), v2);

            // Undo should restore V1
            prop_assert!(history.undo());
            prop_assert_eq!(history.current(), v1);

            // Redo should restore V2
            prop_assert!(history.redo());
            prop_assert_eq!(history.current(), v2);

            cleanup_test();
        }

        #[test]
        fn prop_history_multiple_undo_redo_round_trip(values in prop::collection::vec(any::<i32>(), 1..20)) {
            let _fiber_id = setup_test_fiber();

            let initial = values[0];
            let history = use_history(|| initial);

            // Set all values
            for &v in &values[1..] {
                history.set(v);
            }

            // Undo all the way back
            let mut undo_count = 0;
            while history.undo() {
                undo_count += 1;
            }
            prop_assert_eq!(undo_count, values.len() - 1);
            prop_assert_eq!(history.current(), initial);

            // Redo all the way forward
            let mut redo_count = 0;
            while history.redo() {
                redo_count += 1;
            }
            prop_assert_eq!(redo_count, values.len() - 1);
            prop_assert_eq!(history.current(), *values.last().unwrap());

            cleanup_test();
        }
    }

    // Property 16: History tracks all values
    //
    // For any sequence of set() calls on a history hook, can_undo() SHALL
    // return true after each set (except the first), and the undo stack
    // SHALL contain all previous values.
    //
    // Validates: Requirements 8.1, 8.4
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_history_tracks_all_values(values in prop::collection::vec(any::<i32>(), 2..20)) {
            let _fiber_id = setup_test_fiber();

            let initial = values[0];
            let history = use_history(|| initial);

            // Initially can't undo
            prop_assert!(!history.can_undo());

            // Set all values and verify can_undo after each
            for (i, &v) in values[1..].iter().enumerate() {
                history.set(v);
                prop_assert!(history.can_undo(), "Should be able to undo after set #{}", i + 1);
                prop_assert_eq!(history.past_count(), i + 1, "Past count should be {} after set #{}", i + 1, i + 1);
            }

            // Verify we can undo through all values in reverse order
            for expected in values.iter().rev().skip(1) {
                prop_assert!(history.undo());
                prop_assert_eq!(history.current(), *expected);
            }

            cleanup_test();
        }

        #[test]
        fn prop_history_set_clears_future(
            initial in any::<i32>(),
            v1 in any::<i32>(),
            v2 in any::<i32>(),
            v3 in any::<i32>()
        ) {
            let _fiber_id = setup_test_fiber();

            let history = use_history(|| initial);

            // Build up history
            history.set(v1);
            history.set(v2);

            // Undo to create future
            history.undo();
            prop_assert!(history.can_redo());
            prop_assert_eq!(history.future_count(), 1);

            // Set new value should clear future
            history.set(v3);
            prop_assert!(!history.can_redo());
            prop_assert_eq!(history.future_count(), 0);

            cleanup_test();
        }

        #[test]
        fn prop_history_can_undo_redo_consistency(ops in prop::collection::vec(prop_oneof![Just(true), Just(false)], 1..30)) {
            let _fiber_id = setup_test_fiber();

            let history = use_history(|| 0i32);

            // Build some history first
            for i in 1..=5 {
                history.set(i);
            }

            // Apply random undo/redo operations
            for op in ops {
                if op {
                    // Try undo
                    let could_undo = history.can_undo();
                    let did_undo = history.undo();
                    prop_assert_eq!(could_undo, did_undo, "can_undo should predict undo success");
                } else {
                    // Try redo
                    let could_redo = history.can_redo();
                    let did_redo = history.redo();
                    prop_assert_eq!(could_redo, did_redo, "can_redo should predict redo success");
                }
            }

            cleanup_test();
        }

        #[test]
        fn prop_history_past_future_count_invariant(values in prop::collection::vec(any::<i32>(), 1..15)) {
            let _fiber_id = setup_test_fiber();

            let initial = values[0];
            let history = use_history(|| initial);

            // Set all values
            for &v in &values[1..] {
                history.set(v);
            }

            let total_history = values.len() - 1; // Excluding initial

            // At any point, past_count + future_count should equal total history items
            // when we're somewhere in the middle of the history
            for _ in 0..total_history {
                let past = history.past_count();
                let future = history.future_count();
                prop_assert_eq!(
                    past + future, total_history,
                    "past + future should equal total history"
                );
                history.undo();
            }

            cleanup_test();
        }

        #[test]
        fn prop_history_stable_across_renders(initial in any::<i32>(), values in prop::collection::vec(any::<i32>(), 1..10)) {
            let fiber_id = setup_test_fiber();

            // First render
            let history1 = use_history(|| initial);
            for &v in &values {
                history1.set(v);
            }

            let expected_current = *values.last().unwrap();
            let expected_past_count = values.len();

            // Simulate re-render
            crate::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.end_render();
                tree.begin_render(fiber_id);
            });

            // Second render - should have same state
            let history2 = use_history(|| 999); // Different initializer ignored

            prop_assert_eq!(history2.current(), expected_current);
            prop_assert_eq!(history2.past_count(), expected_past_count);

            cleanup_test();
        }
    }
}
