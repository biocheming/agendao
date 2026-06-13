//! Reducer hook for complex state management with actions.
//!
//! This module provides React-like `use_reducer` hook with proper fiber-based semantics.
//! Multiple dispatches within the same event handler are batched into a single re-render.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::use_reducer;
//!
//! #[derive(Clone)]
//! enum Action {
//!     Increment,
//!     Decrement,
//!     Reset,
//! }
//!
//! fn reducer(state: &i32, action: Action) -> i32 {
//!     match action {
//!         Action::Increment => state + 1,
//!         Action::Decrement => state - 1,
//!         Action::Reset => 0,
//!     }
//! }
//!
//! #[component]
//! fn Counter() -> Element {
//!     let (count, dispatch) = use_reducer(reducer, 0);
//!
//!     // Multiple dispatches are batched - only ONE re-render
//!     let increment_by_3 = move |_| {
//!         dispatch.dispatch(Action::Increment);
//!         dispatch.dispatch(Action::Increment);
//!         dispatch.dispatch(Action::Increment);
//!     };
//!
//!     rsx! { <Text text={count.to_string()} /> }
//! }
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::fiber::FiberId;
use crate::fiber_tree::with_current_fiber;
use crate::scheduler::batch::{StateUpdate, StateUpdateKind, queue_update};

/// Type alias for reducer function
type ReducerFn<S, A> = Arc<dyn Fn(&S, A) -> S + Send + Sync>;

/// Internal storage for reducer function.
///
/// This struct stores the reducer function in an Arc so it can be shared
/// across dispatches without requiring the reducer to be Clone.
pub(crate) struct ReducerStorage<S> {
    /// The reducer function that computes new state from current state and action
    /// We use Any to store the reducer since we can't have generic type parameters
    /// in the storage that need to be Clone.
    pub(crate) reducer: Arc<dyn std::any::Any + Send + Sync>,
    pub(crate) _marker: PhantomData<S>,
}

impl<S> Clone for ReducerStorage<S> {
    fn clone(&self) -> Self {
        Self {
            reducer: self.reducer.clone(),
            _marker: PhantomData,
        }
    }
}

/// Wrapper type for the reducer function to enable downcasting
struct ReducerFnWrapper<S, A>(ReducerFn<S, A>);

/// Dispatch function for sending actions to the reducer.
///
/// This struct is returned by `use_reducer` and provides the `dispatch` method
/// to send actions. Dispatches are queued and batched, not applied immediately.
///
/// # Stability
///
/// The `Dispatch` struct is stable across renders - the same instance is returned
/// on each render, making it safe to use in dependency arrays and callbacks.
pub struct Dispatch<S, A> {
    pub(crate) fiber_id: FiberId,
    pub(crate) hook_index: usize,
    pub(crate) reducer: ReducerFn<S, A>,
    pub(crate) _marker: PhantomData<(S, A)>,
}

impl<S, A> fmt::Debug for Dispatch<S, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatch")
            .field("fiber_id", &self.fiber_id)
            .field("hook_index", &self.hook_index)
            .finish_non_exhaustive()
    }
}

impl<S, A> Clone for Dispatch<S, A> {
    fn clone(&self) -> Self {
        Self {
            fiber_id: self.fiber_id,
            hook_index: self.hook_index,
            reducer: self.reducer.clone(),
            _marker: PhantomData,
        }
    }
}

impl<S: Clone + Send + 'static, A: Send + 'static> Dispatch<S, A> {
    /// Dispatch an action to the reducer (queued for batching).
    ///
    /// The action is processed by the reducer function to compute the new state.
    /// The update is queued and will be applied when the batch ends.
    /// Multiple calls to `dispatch` within the same event handler are batched.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (count, dispatch) = use_reducer(reducer, 0);
    ///
    /// // These are batched into a single re-render
    /// dispatch.dispatch(Action::Increment);
    /// dispatch.dispatch(Action::Increment);
    /// dispatch.dispatch(Action::Increment);
    /// ```
    pub fn dispatch(&self, action: A) {
        let reducer = self.reducer.clone();
        queue_update(
            self.fiber_id,
            StateUpdate {
                hook_index: self.hook_index,
                update: StateUpdateKind::Updater(Box::new(move |any| {
                    let current = any
                        .downcast_ref::<S>()
                        .expect("Reducer state type mismatch");
                    Box::new(reducer(current, action))
                })),
            },
        );
    }
}

/// React-style useReducer with batching support.
///
/// Returns a tuple of the current state value and a dispatch function to send actions.
/// State updates are batched within event handlers for optimal performance.
///
/// # Differences from use_reducer (deprecated)
///
/// - State updates are batched within event handlers
/// - Dispatch function is stable across renders (same reference)
/// - Fiber-scoped state (no global index collision)
/// - Reducer receives `&S` instead of `S` for efficiency
///
/// # Arguments
///
/// * `reducer` - A function that takes the current state reference and an action,
///   returning the new state. The reducer should be a pure function.
/// * `initial_state` - The initial state value. Only used on the first render.
///
/// # Returns
///
/// A tuple of `(current_state, dispatch)` where:
/// - `current_state` is the current state value (cloned)
/// - `dispatch` is a `Dispatch` that can be used to send actions
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_reducer;
///
/// #[derive(Clone)]
/// enum Action {
///     Increment,
///     Decrement,
///     SetValue(i32),
/// }
///
/// fn reducer(state: &i32, action: Action) -> i32 {
///     match action {
///         Action::Increment => state + 1,
///         Action::Decrement => state - 1,
///         Action::SetValue(v) => v,
///     }
/// }
///
/// #[component]
/// fn Counter() -> Element {
///     let (count, dispatch) = use_reducer(reducer, 0);
///
///     let increment = {
///         let dispatch = dispatch.clone();
///         move |_| dispatch.dispatch(Action::Increment)
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
pub fn use_reducer<S, A, R>(reducer: R, initial_state: S) -> (S, Dispatch<S, A>)
where
    S: Clone + Send + 'static,
    A: Send + 'static,
    R: Fn(&S, A) -> S + Send + Sync + 'static,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Get or initialize the reducer storage (stores the reducer function wrapped in Arc)
        let reducer_arc: ReducerFn<S, A> = Arc::new(reducer);
        let storage = fiber.get_or_init_hook(hook_index, || ReducerStorage {
            reducer: Arc::new(ReducerFnWrapper(reducer_arc.clone()))
                as Arc<dyn std::any::Any + Send + Sync>,
            _marker: PhantomData::<S>,
        });

        // Extract the reducer from storage
        let reducer_fn = storage
            .reducer
            .downcast_ref::<ReducerFnWrapper<S, A>>()
            .expect("Reducer type mismatch")
            .0
            .clone();

        // Get or initialize the state at the next hook index
        let state_hook_index = fiber.next_hook_index();
        let state = fiber.get_or_init_hook(state_hook_index, || initial_state);

        let dispatch = Dispatch {
            fiber_id: fiber.id,
            hook_index: state_hook_index,
            reducer: reducer_fn,
            _marker: PhantomData,
        };

        (state, dispatch)
    })
    .expect("use_reducer must be called within a component render context")
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

    #[derive(Clone, Debug, PartialEq)]
    enum TestAction {
        Increment,
        Add(i32),
    }

    fn test_reducer(state: &i32, action: TestAction) -> i32 {
        match action {
            TestAction::Increment => state + 1,
            TestAction::Add(n) => state + n,
        }
    }

    #[test]
    fn test_use_reducer_initial_value() {
        let _fiber_id = setup_test_fiber();

        let (state, _dispatch) = use_reducer(test_reducer, 42);
        assert_eq!(state, 42);

        cleanup_test();
    }

    #[test]
    fn test_use_reducer_returns_same_value_on_rerender() {
        let fiber_id = setup_test_fiber();

        // First render
        let (state1, _dispatch1) = use_reducer(test_reducer, 100);
        assert_eq!(state1, 100);

        // Simulate re-render by resetting hook index
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render - should return same value, not call initializer again
        let (state2, _dispatch2) = use_reducer(test_reducer, 999);
        assert_eq!(state2, 100); // Still 100, not 999

        cleanup_test();
    }

    #[test]
    fn test_dispatch_queues_update() {
        let fiber_id = setup_test_fiber();

        let (_state, dispatch) = use_reducer(test_reducer, 0);

        // Dispatch should queue an update
        dispatch.dispatch(TestAction::Increment);

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
    fn test_dispatch_applies_reducer() {
        let fiber_id = setup_test_fiber();

        // Initialize reducer
        let (_state, dispatch) = use_reducer(test_reducer, 10);

        // Dispatch an action
        dispatch.dispatch(TestAction::Add(5));

        // End render and apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });

            // Check final value: 10 + 5 = 15
            // State is at hook index 1 (reducer storage is at 0)
            let fiber = tree.get(fiber_id).unwrap();
            let final_value = fiber.get_hook::<i32>(1);
            assert_eq!(final_value, Some(15));
        });

        cleanup_test();
    }

    #[test]
    fn test_multiple_dispatches_batched() {
        let fiber_id = setup_test_fiber();

        // Initialize reducer
        let (_state, dispatch) = use_reducer(test_reducer, 0);

        // Queue multiple dispatches
        dispatch.dispatch(TestAction::Increment);
        dispatch.dispatch(TestAction::Increment);
        dispatch.dispatch(TestAction::Increment);
        dispatch.dispatch(TestAction::Add(10));

        // End render and apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            let dirty =
                crate::scheduler::batch::with_state_batch_mut(|batch| batch.end_batch(tree));

            // Only one fiber should be dirty (batched)
            assert_eq!(dirty.len(), 1);
            assert!(dirty.contains(&fiber_id));

            // Check final value: 0 + 1 + 1 + 1 + 10 = 13
            let fiber = tree.get(fiber_id).unwrap();
            let final_value = fiber.get_hook::<i32>(1);
            assert_eq!(final_value, Some(13));
        });

        cleanup_test();
    }

    #[test]
    fn test_dispatch_is_clone() {
        let _fiber_id = setup_test_fiber();

        let (_state, dispatch) = use_reducer(test_reducer, 0);

        // Dispatch should be Clone
        let dispatch_clone = dispatch.clone();
        let _dispatch_clone2 = dispatch_clone.clone();

        cleanup_test();
    }

    #[test]
    fn test_dispatch_stability_across_renders() {
        let fiber_id = setup_test_fiber();

        // First render
        let (_state1, dispatch1) = use_reducer(test_reducer, 0);
        let dispatch1_fiber_id = dispatch1.fiber_id;
        let dispatch1_hook_index = dispatch1.hook_index;

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render
        let (_state2, dispatch2) = use_reducer(test_reducer, 999);

        // Dispatch should have same fiber_id and hook_index (stable reference)
        assert_eq!(dispatch1_fiber_id, dispatch2.fiber_id);
        assert_eq!(dispatch1_hook_index, dispatch2.hook_index);

        cleanup_test();
    }

    #[test]
    fn test_multiple_reducers() {
        let _fiber_id = setup_test_fiber();

        fn string_reducer(state: &String, action: &str) -> String {
            format!("{}{}", state, action)
        }

        let (count, _dispatch_count) = use_reducer(test_reducer, 0);
        let (text, _dispatch_text) = use_reducer(string_reducer, String::new());

        assert_eq!(count, 0);
        assert_eq!(text, "");

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_reducer must be called within a component render context")]
    fn test_use_reducer_panics_outside_render() {
        // Clear any existing fiber tree
        clear_fiber_tree();
        crate::scheduler::batch::clear_state_batch();

        // This should panic because there's no current fiber
        let _ = use_reducer(test_reducer, 0);
    }

    #[test]
    fn test_complex_state_reducer() {
        let fiber_id = setup_test_fiber();

        #[derive(Clone, Debug, PartialEq)]
        struct TodoState {
            todos: Vec<String>,
            count: usize,
        }

        #[derive(Clone)]
        enum TodoAction {
            Add(String),
        }

        fn todo_reducer(state: &TodoState, action: TodoAction) -> TodoState {
            match action {
                TodoAction::Add(text) => {
                    let mut todos = state.todos.clone();
                    todos.push(text);
                    TodoState {
                        todos,
                        count: state.count + 1,
                    }
                }
            }
        }

        let initial = TodoState {
            todos: vec![],
            count: 0,
        };

        let (_state, dispatch) = use_reducer(todo_reducer, initial);

        // Add some todos
        dispatch.dispatch(TodoAction::Add("Task 1".to_string()));
        dispatch.dispatch(TodoAction::Add("Task 2".to_string()));
        dispatch.dispatch(TodoAction::Add("Task 3".to_string()));

        // End render and apply batch
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();

            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });

            let fiber = tree.get(fiber_id).unwrap();
            let final_state = fiber.get_hook::<TodoState>(1).unwrap();
            assert_eq!(final_state.todos.len(), 3);
            assert_eq!(final_state.count, 3);
            assert_eq!(final_state.todos[0], "Task 1");
            assert_eq!(final_state.todos[1], "Task 2");
            assert_eq!(final_state.todos[2], "Task 3");
        });

        cleanup_test();
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
        crate::scheduler::batch::clear_state_batch();
    }

    /// Simple counter reducer for testing
    fn counter_reducer(state: &i32, action: i32) -> i32 {
        state + action
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // ============================================================
        // **Property 3: Reducer applies actions correctly**
        // **Validates: Requirements 2.2**
        //
        // For any reducer function, initial state, and sequence of actions,
        // dispatching each action SHALL produce the same result as applying
        // the reducer function sequentially.
        // ============================================================

        #[test]
        fn prop_reducer_applies_actions_correctly(
            initial_state in any::<i32>(),
            actions in prop::collection::vec(-100i32..100, 1..20)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // Initialize reducer
            let (_state, dispatch) = use_reducer(counter_reducer, initial_state);

            // Dispatch all actions
            for action in &actions {
                dispatch.dispatch(*action);
            }

            // Calculate expected value by applying reducer sequentially
            let expected = actions.iter().fold(initial_state, |acc, action| {
                counter_reducer(&acc, *action)
            });

            // End render and apply batch
            let final_value = with_fiber_tree_mut(|tree| {
                tree.end_render();

                crate::scheduler::batch::with_state_batch_mut(|batch| {
                    batch.end_batch(tree);
                });

                // State is at hook index 1 (reducer storage is at 0)
                let fiber = tree.get(fiber_id).unwrap();
                fiber.get_hook::<i32>(1)
            }).flatten();

            prop_assert_eq!(
                final_value,
                Some(expected),
                "Reducer should apply actions correctly: expected {}, got {:?}",
                expected,
                final_value
            );

            cleanup_test();
        }

        #[test]
        fn prop_reducer_with_complex_actions(
            initial_state in 0i32..1000,
            operations in prop::collection::vec(
                prop_oneof![
                    Just(1i32),   // Increment
                    Just(-1i32),  // Decrement
                    (1i32..50).prop_map(|n| n),  // Add positive
                    (-50i32..-1).prop_map(|n| n), // Add negative
                ],
                1..30
            )
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            let (_state, dispatch) = use_reducer(counter_reducer, initial_state);

            // Dispatch all operations
            for op in &operations {
                dispatch.dispatch(*op);
            }

            let expected = operations.iter().fold(initial_state, |acc, op| acc + op);

            // Apply batch and verify
            let final_value = with_fiber_tree_mut(|tree| {
                tree.end_render();

                crate::scheduler::batch::with_state_batch_mut(|batch| {
                    batch.end_batch(tree);
                });

                let fiber = tree.get(fiber_id).unwrap();
                fiber.get_hook::<i32>(1)
            }).flatten();

            prop_assert_eq!(
                final_value,
                Some(expected),
                "Complex operations should be applied correctly"
            );

            cleanup_test();
        }

        // ============================================================
        // **Property 4: Reducer batches multiple dispatches**
        // **Validates: Requirements 2.3**
        //
        // For any sequence of `dispatch()` calls within the same event phase,
        // the system SHALL mark the fiber dirty only ONCE, regardless of the
        // number of dispatches.
        // ============================================================

        #[test]
        fn prop_reducer_batches_multiple_dispatches(
            initial_state in any::<i32>(),
            num_dispatches in 2usize..50
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            let (_state, dispatch) = use_reducer(counter_reducer, initial_state);

            // Dispatch multiple actions
            for i in 0..num_dispatches {
                dispatch.dispatch(i as i32);
            }

            // Verify only one fiber is marked dirty (batched)
            let dirty_count = crate::scheduler::batch::with_state_batch(|batch| {
                batch.dirty_fiber_count()
            });

            prop_assert_eq!(
                dirty_count,
                1,
                "Multiple dispatches should result in only ONE dirty fiber, got {}",
                dirty_count
            );

            // Apply batch and verify only one fiber in dirty set
            let result = with_fiber_tree_mut(|tree| {
                tree.end_render();

                let dirty = crate::scheduler::batch::with_state_batch_mut(|batch| {
                    batch.end_batch(tree)
                });

                (dirty.len(), dirty.contains(&fiber_id))
            });

            let (dirty_len, contains_fiber) = result.unwrap();

            prop_assert_eq!(
                dirty_len,
                1,
                "Batch should return exactly one dirty fiber, got {}",
                dirty_len
            );

            prop_assert!(
                contains_fiber,
                "The dirty fiber should be our fiber"
            );

            cleanup_test();
        }

        #[test]
        fn prop_multiple_reducers_batch_independently(
            initial1 in any::<i32>(),
            initial2 in any::<i32>(),
            actions1 in prop::collection::vec(-10i32..10, 1..10),
            actions2 in prop::collection::vec(-10i32..10, 1..10)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // Create two reducers
            let (_state1, dispatch1) = use_reducer(counter_reducer, initial1);
            let (_state2, dispatch2) = use_reducer(counter_reducer, initial2);

            // Dispatch to both
            for action in &actions1 {
                dispatch1.dispatch(*action);
            }
            for action in &actions2 {
                dispatch2.dispatch(*action);
            }

            // Still only one fiber should be dirty (same component)
            let dirty_count = crate::scheduler::batch::with_state_batch(|batch| {
                batch.dirty_fiber_count()
            });

            prop_assert_eq!(
                dirty_count,
                1,
                "Multiple reducers in same fiber should still result in one dirty fiber"
            );

            let expected1 = actions1.iter().fold(initial1, |acc, a| acc + a);
            let expected2 = actions2.iter().fold(initial2, |acc, a| acc + a);

            // Apply and verify both states are correct
            let result = with_fiber_tree_mut(|tree| {
                tree.end_render();

                crate::scheduler::batch::with_state_batch_mut(|batch| {
                    batch.end_batch(tree);
                });

                let fiber = tree.get(fiber_id).unwrap();
                // First reducer: storage at 0, state at 1
                // Second reducer: storage at 2, state at 3
                (fiber.get_hook::<i32>(1), fiber.get_hook::<i32>(3))
            });

            let (final1, final2) = result.unwrap();

            prop_assert_eq!(final1, Some(expected1), "First reducer state should be correct");
            prop_assert_eq!(final2, Some(expected2), "Second reducer state should be correct");

            cleanup_test();
        }

        // ============================================================
        // **Property 5: Dispatch function stability**
        // **Validates: Requirements 2.4**
        //
        // For any component using `use_reducer`, the dispatch function
        // reference SHALL be pointer-equal across renders.
        // ============================================================

        #[test]
        fn prop_dispatch_function_stability(
            initial_state in any::<i32>(),
            num_renders in 2usize..10
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render
            let (_state1, dispatch1) = use_reducer(counter_reducer, initial_state);
            let dispatch1_fiber_id = dispatch1.fiber_id;
            let dispatch1_hook_index = dispatch1.hook_index;

            // Simulate multiple re-renders
            for render_num in 1..num_renders {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                // Get dispatch again
                let (_state, dispatch) = use_reducer(counter_reducer, 999999);

                // Property: Dispatch should have same fiber_id and hook_index (stable reference)
                prop_assert_eq!(
                    dispatch.fiber_id,
                    dispatch1_fiber_id,
                    "Render {}: dispatch fiber_id should be stable",
                    render_num
                );
                prop_assert_eq!(
                    dispatch.hook_index,
                    dispatch1_hook_index,
                    "Render {}: dispatch hook_index should be stable",
                    render_num
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_dispatch_works_after_rerender(
            initial_state in any::<i32>(),
            actions_before in prop::collection::vec(-10i32..10, 1..5),
            actions_after in prop::collection::vec(-10i32..10, 1..5)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render
            let (_state, dispatch) = use_reducer(counter_reducer, initial_state);

            // Dispatch some actions
            for action in &actions_before {
                dispatch.dispatch(*action);
            }

            // Apply batch
            with_fiber_tree_mut(|tree| {
                tree.end_render();
                crate::scheduler::batch::with_state_batch_mut(|batch| {
                    batch.end_batch(tree);
                });
            });

            // Calculate intermediate state
            let intermediate = actions_before.iter().fold(initial_state, |acc, a| acc + a);

            // Simulate re-render
            with_fiber_tree_mut(|tree| {
                tree.begin_render(fiber_id);
            });

            // Get new dispatch (should work with updated state)
            let (state_after_rerender, dispatch_after) = use_reducer(counter_reducer, 999999);

            // State should reflect previous dispatches
            prop_assert_eq!(
                state_after_rerender,
                intermediate,
                "State after re-render should reflect previous dispatches"
            );

            // Dispatch more actions using the new dispatch
            for action in &actions_after {
                dispatch_after.dispatch(*action);
            }

            let expected = actions_after.iter().fold(intermediate, |acc, a| acc + a);

            // Apply batch and verify final state
            let final_value = with_fiber_tree_mut(|tree| {
                tree.end_render();
                crate::scheduler::batch::with_state_batch_mut(|batch| {
                    batch.end_batch(tree);
                });

                let fiber = tree.get(fiber_id).unwrap();
                fiber.get_hook::<i32>(1)
            }).flatten();

            prop_assert_eq!(
                final_value,
                Some(expected),
                "Dispatch should work correctly after re-render"
            );

            cleanup_test();
        }
    }
}
