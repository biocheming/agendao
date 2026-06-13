//! Effect Event hook for stable callbacks that always see current state.
//!
//! This module provides React's experimental `useEffectEvent` functionality.
//! It creates a stable callback reference that doesn't change between renders,
//! but always calls the latest version of the provided handler.
//!
//! # Use Cases
//!
//! - Event handlers in effects with empty dependency arrays
//! - Callbacks passed to memoized child components
//! - Avoiding unnecessary re-renders while maintaining fresh state access
//! - Breaking the dependency cycle in effects
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_state, use_effect_event, use_effect};
//!
//! #[component]
//! fn Logger() -> Element {
//!     let (count, set_count) = use_state(|| 0);
//!
//!     // This callback has a stable identity but always sees the latest count
//!     let log_count = use_effect_event(move |_: ()| {
//!         println!("Current count: {}", count);
//!     });
//!
//!     // Can be used in effects without adding count to dependencies
//!     use_effect(|| {
//!         // Set up some subscription that calls log_count
//!         log_count.call(());
//!         None
//!     }, None::<()>);
//!
//!     rsx! { <Text text={count.to_string()} /> }
//! }
//! ```

use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use crate::fiber::FiberId;
use crate::fiber_tree::with_current_fiber;

/// Type alias for effect event handler
type EffectEventHandler<IN, OUT> = Arc<RwLock<Box<dyn Fn(IN) -> OUT + Send + Sync>>>;

/// Internal storage for the effect event handler.
///
/// This stores the latest handler in an Arc<RwLock<...>> so it can be
/// updated each render while the wrapper function remains stable.
pub(crate) struct EffectEventStorage<IN, OUT> {
    /// The latest handler function, updated each render
    pub(crate) handler: EffectEventHandler<IN, OUT>,
}

impl<IN, OUT> Clone for EffectEventStorage<IN, OUT> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
        }
    }
}

/// A stable callback that always invokes the latest handler.
///
/// This struct is returned by `use_effect_event`. The callback identity
/// remains stable across renders (same Arc reference), but it always calls
/// the most recent version of the handler function.
///
/// # Thread Safety
///
/// `EffectEvent` is thread-safe and can be safely shared across async tasks.
/// It uses `Arc<RwLock<...>>` internally for concurrent access.
pub struct EffectEvent<IN, OUT> {
    pub(crate) fiber_id: FiberId,
    pub(crate) hook_index: usize,
    pub(crate) handler: EffectEventHandler<IN, OUT>,
    pub(crate) _marker: PhantomData<(IN, OUT)>,
}

impl<IN, OUT> std::fmt::Debug for EffectEvent<IN, OUT> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectEvent")
            .field("fiber_id", &self.fiber_id)
            .field("hook_index", &self.hook_index)
            .finish_non_exhaustive()
    }
}

impl<IN, OUT> Clone for EffectEvent<IN, OUT> {
    fn clone(&self) -> Self {
        Self {
            fiber_id: self.fiber_id,
            hook_index: self.hook_index,
            handler: self.handler.clone(),
            _marker: PhantomData,
        }
    }
}

impl<IN, OUT> EffectEvent<IN, OUT>
where
    IN: 'static,
    OUT: 'static,
{
    /// Call the effect event with the given input.
    ///
    /// This always invokes the latest handler that was provided during the
    /// most recent render, ensuring access to current props/state values.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let log_event = use_effect_event(move |msg: &str| {
    ///     println!("Event: {} (count: {})", msg, count);
    /// });
    ///
    /// // Later, in an effect or callback:
    /// log_event.call("button clicked");
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn call(&self, input: IN) -> OUT {
        let handler = self.handler.read().expect("EffectEvent lock poisoned");
        handler(input)
    }
}

/// React-style useEffectEvent for stable callbacks with fresh state access.
///
/// Returns a stable callback that doesn't change between renders, but always
/// calls the latest version of the provided handler. This is useful for:
///
/// - **Event handlers in effects**: Use in effects without adding dependencies
/// - **Memoized callbacks**: Pass to child components without causing re-renders
/// - **Breaking dependency cycles**: Access current state without effect re-runs
///
/// # How It Works
///
/// 1. On first render, creates storage with the handler wrapped in Arc<RwLock<...>>
/// 2. On subsequent renders, updates the handler reference inside the storage
/// 3. Returns a stable `EffectEvent` that always calls the current handler
///
/// # Differences from use_callback
///
/// | Feature | `use_effect_event` | `use_callback` |
/// |---------|----------------------|-------------------|
/// | Stability | Always stable | Stable when deps unchanged |
/// | State access | Always current | Captured at creation |
/// | Dependencies | None needed | Required for freshness |
/// | Use case | Effects, subscriptions | Memoization |
///
/// # Arguments
///
/// * `handler` - The handler function to wrap. This is updated each render
///   but the returned wrapper remains stable.
///
/// # Returns
///
/// An `EffectEvent<IN, OUT>` that provides a `call` method to invoke the handler.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::{use_state, use_effect_event, use_effect};
///
/// #[component]
/// fn ChatRoom(room_id: String) -> Element {
///     let (messages, set_messages) = use_state(|| vec![]);
///
///     // Stable callback that always sees current messages
///     let on_message = use_effect_event(move |new_msg: String| {
///         // This always has access to current messages
///         set_messages.update(|msgs| {
///             let mut new_msgs = msgs.clone();
///             new_msgs.push(new_msg);
///             new_msgs
///         });
///     });
///
///     // Effect only re-runs when room_id changes, not when messages change
///     use_effect(|| {
///         let subscription = subscribe_to_room(&room_id, move |msg| {
///             on_message.call(msg);
///         });
///
///         Some(Box::new(move || subscription.unsubscribe()))
///     }, Some(room_id.clone()));
///
///     rsx! { /* render messages */ }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_effect_event<IN, OUT, F>(handler: F) -> EffectEvent<IN, OUT>
where
    IN: 'static,
    OUT: 'static,
    F: Fn(IN) -> OUT + Send + Sync + 'static,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Check if storage already exists
        let existing_storage: Option<EffectEventStorage<IN, OUT>> = fiber.get_hook(hook_index);

        let storage = if let Some(storage) = existing_storage {
            // Storage exists, just update the handler
            {
                let mut guard = storage.handler.write().expect("EffectEvent lock poisoned");
                *guard = Box::new(handler);
            }
            storage
        } else {
            // First render: create new storage with the handler
            let new_storage = EffectEventStorage {
                handler: Arc::new(RwLock::new(
                    Box::new(handler) as Box<dyn Fn(IN) -> OUT + Send + Sync>
                )),
            };
            fiber.set_hook(hook_index, new_storage.clone());
            new_storage
        };

        EffectEvent {
            fiber_id: fiber.id,
            hook_index,
            handler: storage.handler,
            _marker: PhantomData,
        }
    })
    .expect("use_effect_event must be called within a component render context")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use std::sync::atomic::{AtomicI32, Ordering};

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
    fn test_use_effect_event_basic() {
        let _fiber_id = setup_test_fiber();

        let effect_event = use_effect_event(|x: i32| x * 2);
        assert_eq!(effect_event.call(5), 10);

        cleanup_test();
    }

    #[test]
    fn test_use_effect_event_with_unit_input() {
        let _fiber_id = setup_test_fiber();

        let counter = Arc::new(AtomicI32::new(0));
        let counter_clone = counter.clone();

        let effect_event = use_effect_event(move |_: ()| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        effect_event.call(());
        effect_event.call(());
        effect_event.call(());

        assert_eq!(counter.load(Ordering::SeqCst), 3);

        cleanup_test();
    }

    #[test]
    fn test_effect_event_stability_across_renders() {
        let fiber_id = setup_test_fiber();

        // First render
        let effect_event1 = use_effect_event(|x: i32| x + 1);
        let handler_ptr1 = Arc::as_ptr(&effect_event1.handler);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render with different handler
        let effect_event2 = use_effect_event(|x: i32| x + 100);
        let handler_ptr2 = Arc::as_ptr(&effect_event2.handler);

        // The Arc pointer should be the same (stable reference)
        assert_eq!(
            handler_ptr1, handler_ptr2,
            "Effect event should have stable Arc reference across renders"
        );

        // But the handler inside should be updated
        assert_eq!(effect_event2.call(5), 105); // Uses new handler

        cleanup_test();
    }

    #[test]
    fn test_effect_event_sees_current_state() {
        let fiber_id = setup_test_fiber();

        // Simulate state that changes between renders
        let state = Arc::new(AtomicI32::new(10));
        let state_clone = state.clone();

        // First render - handler captures state value 10
        let effect_event = use_effect_event(move |_: ()| state_clone.load(Ordering::SeqCst));

        assert_eq!(effect_event.call(()), 10);

        // Change state
        state.store(42, Ordering::SeqCst);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render - handler should see new state
        let state_clone2 = state.clone();
        let effect_event2 = use_effect_event(move |_: ()| state_clone2.load(Ordering::SeqCst));

        // Should see current state value
        assert_eq!(effect_event2.call(()), 42);

        // Even the old reference should see current state (same storage)
        assert_eq!(effect_event.call(()), 42);

        cleanup_test();
    }

    #[test]
    fn test_effect_event_clone_shares_handler() {
        let _fiber_id = setup_test_fiber();

        let counter = Arc::new(AtomicI32::new(0));
        let counter_clone = counter.clone();

        let effect_event = use_effect_event(move |_: ()| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let effect_event_clone = effect_event.clone();

        // Both should call the same handler
        effect_event.call(());
        effect_event_clone.call(());

        assert_eq!(counter.load(Ordering::SeqCst), 2);

        cleanup_test();
    }

    #[test]
    fn test_multiple_effect_events() {
        let _fiber_id = setup_test_fiber();

        let effect1 = use_effect_event(|x: i32| x * 2);
        let effect2 = use_effect_event(|x: i32| x + 10);
        let effect3 = use_effect_event(|s: &str| s.len());

        assert_eq!(effect1.call(5), 10);
        assert_eq!(effect2.call(5), 15);
        assert_eq!(effect3.call("hello"), 5);

        cleanup_test();
    }

    #[test]
    fn test_effect_event_with_return_value() {
        let _fiber_id = setup_test_fiber();

        let effect_event = use_effect_event(|input: (i32, i32)| {
            let (a, b) = input;
            format!("{} + {} = {}", a, b, a + b)
        });

        assert_eq!(effect_event.call((3, 4)), "3 + 4 = 7");

        cleanup_test();
    }

    #[test]
    fn test_effect_event_fiber_id_and_hook_index() {
        let fiber_id = setup_test_fiber();

        let effect1 = use_effect_event(|_: ()| {});
        let effect2 = use_effect_event(|_: ()| {});

        assert_eq!(effect1.fiber_id, fiber_id);
        assert_eq!(effect2.fiber_id, fiber_id);
        assert_eq!(effect1.hook_index, 0);
        assert_eq!(effect2.hook_index, 1);

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_effect_event must be called within a component render context")]
    fn test_use_effect_event_panics_outside_render() {
        clear_fiber_tree();

        // This should panic because there's no current fiber
        let _ = use_effect_event(|_: ()| {});
    }

    #[test]
    fn test_effect_event_handler_updated_each_render() {
        let fiber_id = setup_test_fiber();

        // Track which handler version is called
        let call_count = Arc::new(AtomicI32::new(0));

        // First render - handler returns 1
        let cc1 = call_count.clone();
        let effect_event = use_effect_event(move |_: ()| {
            cc1.fetch_add(1, Ordering::SeqCst);
            1
        });

        assert_eq!(effect_event.call(()), 1);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render - handler returns 2
        let cc2 = call_count.clone();
        let _effect_event2 = use_effect_event(move |_: ()| {
            cc2.fetch_add(1, Ordering::SeqCst);
            2
        });

        // Old reference should now call new handler
        assert_eq!(effect_event.call(()), 2);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);

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
    use std::sync::atomic::{AtomicI32, Ordering};

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
        // **Property 8: Effect event function stability**
        // **Validates: Requirements 4.1**
        //
        // For any function returned by `use_effect_event`, the function
        // reference SHALL be pointer-equal across renders.
        // ============================================================

        #[test]
        fn prop_effect_event_function_stability(
            num_renders in 2usize..20
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render - create effect event
            let effect_event1 = use_effect_event(|x: i32| x + 1);
            let handler_ptr1 = Arc::as_ptr(&effect_event1.handler);

            // Simulate multiple re-renders with different handlers
            for render_num in 1..num_renders {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                // Create effect event with a different handler each time
                let multiplier = render_num as i32;
                let effect_event = use_effect_event(move |x: i32| x * multiplier);
                let handler_ptr = Arc::as_ptr(&effect_event.handler);

                // Property: Arc pointer should be the same across all renders
                prop_assert_eq!(
                    handler_ptr,
                    handler_ptr1,
                    "Render {}: Effect event Arc pointer should be stable (expected {:?}, got {:?})",
                    render_num,
                    handler_ptr1,
                    handler_ptr
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_effect_event_stability_with_varying_handlers(
            initial_offset in any::<i32>(),
            offsets in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render
            let effect_event1 = use_effect_event(move |x: i32| x + initial_offset);
            let handler_ptr1 = Arc::as_ptr(&effect_event1.handler);
            let fiber_id1 = effect_event1.fiber_id;
            let hook_index1 = effect_event1.hook_index;

            // Simulate re-renders with different handler offsets
            for offset in &offsets {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                let offset_copy = *offset;
                let effect_event = use_effect_event(move |x: i32| x + offset_copy);

                // Property: All identifying fields should be stable
                prop_assert_eq!(
                    Arc::as_ptr(&effect_event.handler),
                    handler_ptr1,
                    "Handler Arc pointer should be stable"
                );
                prop_assert_eq!(
                    effect_event.fiber_id,
                    fiber_id1,
                    "Fiber ID should be stable"
                );
                prop_assert_eq!(
                    effect_event.hook_index,
                    hook_index1,
                    "Hook index should be stable"
                );
            }

            cleanup_test();
        }

        // ============================================================
        // **Property 9: Effect event sees current state**
        // **Validates: Requirements 4.2**
        //
        // For any function returned by `use_effect_event`, when called
        // after state updates, it SHALL execute with the current (updated)
        // state values.
        // ============================================================

        #[test]
        fn prop_effect_event_sees_current_state(
            initial_state in any::<i32>(),
            state_updates in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // Simulate state using AtomicI32
            let state = Arc::new(AtomicI32::new(initial_state));
            let state_clone = state.clone();

            // First render - handler reads from state
            let effect_event = use_effect_event(move |_: ()| {
                state_clone.load(Ordering::SeqCst)
            });

            // Property: Should see initial state
            prop_assert_eq!(
                effect_event.call(()),
                initial_state,
                "Should see initial state"
            );

            // Apply state updates and re-render
            for new_state in &state_updates {
                // Update state
                state.store(*new_state, Ordering::SeqCst);

                // Simulate re-render
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                // Re-create effect event with new handler that captures current state
                let state_clone2 = state.clone();
                let _effect_event2 = use_effect_event(move |_: ()| {
                    state_clone2.load(Ordering::SeqCst)
                });

                // Property: Old reference should see current state
                prop_assert_eq!(
                    effect_event.call(()),
                    *new_state,
                    "Old effect event reference should see current state {}",
                    new_state
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_effect_event_handler_always_current(
            num_renders in 2usize..15
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // First render - handler returns render number (1)
            let effect_event = use_effect_event(|_: ()| 1i32);

            // Property: First render should return 1
            prop_assert_eq!(effect_event.call(()), 1, "First render should return 1");

            // Simulate re-renders, each with a handler returning the render number
            for render_num in 2..=num_renders {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                let expected = render_num as i32;
                let _effect_event_new = use_effect_event(move |_: ()| expected);

                // Property: Old reference should call the NEW handler
                prop_assert_eq!(
                    effect_event.call(()),
                    expected,
                    "Render {}: Old effect event should call current handler (expected {}, got {})",
                    render_num,
                    expected,
                    effect_event.call(())
                );
            }

            cleanup_test();
        }

        // ============================================================
        // Additional property: Multiple effect events maintain independence
        // ============================================================

        #[test]
        fn prop_multiple_effect_events_independent(
            num_events in 2usize..5,
            num_renders in 1usize..5
        ) {
            let _lock = TEST_MUTEX.lock();
            cleanup_test();

            let fiber_id = setup_test_fiber();

            // Create multiple effect events on first render
            let mut effect_events = Vec::new();
            let mut handler_ptrs = Vec::new();

            for i in 0..num_events {
                let offset = i as i32;
                let effect_event = use_effect_event(move |x: i32| x + offset);
                handler_ptrs.push(Arc::as_ptr(&effect_event.handler));
                effect_events.push(effect_event);
            }

            // Verify initial behavior
            for (i, effect_event) in effect_events.iter().enumerate() {
                let expected = 10 + i as i32;
                prop_assert_eq!(
                    effect_event.call(10),
                    expected,
                    "Initial: Effect event {} should return {}",
                    i,
                    expected
                );
            }

            // Simulate re-renders
            for render_num in 1..=num_renders {
                with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                // Re-create effect events with different handlers
                for (i, handler_ptr) in handler_ptrs.iter().enumerate().take(num_events) {
                    let multiplier = (render_num + 1) as i32;
                    let offset = i as i32;
                    let effect_event = use_effect_event(move |x: i32| x * multiplier + offset);

                    // Property: Arc pointer should be stable
                    prop_assert_eq!(
                        Arc::as_ptr(&effect_event.handler),
                        *handler_ptr,
                        "Render {}: Effect event {} Arc pointer should be stable",
                        render_num,
                        i
                    );
                }

                // Property: Old references should call new handlers
                for (i, effect_event) in effect_events.iter().enumerate() {
                    let multiplier = (render_num + 1) as i32;
                    let offset = i as i32;
                    let expected = 10 * multiplier + offset;
                    prop_assert_eq!(
                        effect_event.call(10),
                        expected,
                        "Render {}: Effect event {} should return {} (got {})",
                        render_num,
                        i,
                        expected,
                        effect_event.call(10)
                    );
                }
            }

            cleanup_test();
        }
    }
}
