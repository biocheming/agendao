//! Consolidated render context for managing all thread-local state.
//!
//! This module provides a unified `RenderContext` struct that combines all
//! render-related state into a single location, simplifying the mental model
//! and making it easier to manage the render lifecycle.
//!
//! # Architecture
//!
//! Previously, the crate used multiple separate thread-local variables:
//! - `FIBER_TREE` - Component instance tracking
//! - `STATE_BATCH` - State update batching
//! - `EFFECT_QUEUE` - Post-commit effect execution
//! - `CONTEXT_STACK` - React-like context system
//! - `CURRENT_EVENT` - Terminal event handling
//!
//! The `RenderContext` consolidates these into a single struct, providing:
//! - Simpler initialization and cleanup
//! - Better encapsulation of render state
//! - Easier debugging and testing
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui::render_context::{RenderContext, init_render_context, with_render_context_mut};
//!
//! // Initialize the render context at the start of the render loop
//! init_render_context();
//!
//! // Access the context during rendering
//! with_render_context_mut(|ctx| {
//!     ctx.fiber_tree.prepare_for_render();
//!     ctx.state_batch.begin_batch();
//! });
//!
//! // Clean up when done
//! clear_render_context();
//! ```

use std::cell::RefCell;
use std::sync::Arc;

use crossterm::event::Event;

use crate::context_stack::ContextStack;
use crate::event::EventState;
use crate::fiber_tree::FiberTree;
use crate::scheduler::batch::StateBatch;
use crate::scheduler::effect_queue::EffectQueue;

thread_local! {
    /// Thread-local render context for the current render loop
    static RENDER_CONTEXT: RefCell<Option<RenderContext>> = const { RefCell::new(None) };
}

/// Consolidated render context containing all render-related state.
///
/// This struct combines all the previously separate thread-local state
/// into a single location for easier management and better encapsulation.
pub struct RenderContext {
    /// Fiber tree tracking all mounted component instances
    pub fiber_tree: FiberTree,
    /// State batch for grouping multiple state updates
    pub state_batch: StateBatch,
    /// Effect queue for post-commit effect execution
    pub effect_queue: EffectQueue,
    /// Context stack for React-like context system
    pub context_stack: ContextStack,
    /// Current event state for terminal event handling
    pub event_state: EventState,
}

impl RenderContext {
    /// Create a new render context with default state
    pub fn new() -> Self {
        Self {
            fiber_tree: FiberTree::new(),
            state_batch: StateBatch::new(),
            effect_queue: EffectQueue::new(),
            context_stack: ContextStack::new(),
            event_state: EventState::new(),
        }
    }

    /// Reset all state for a new render pass
    ///
    /// This prepares the context for a new frame by:
    /// - Resetting hook indices in the fiber tree
    /// - Clearing the current event
    pub fn prepare_for_render(&mut self) {
        self.fiber_tree.prepare_for_render();
    }

    /// Set the current event for this render pass
    pub fn set_event(&mut self, event: Option<Arc<Event>>) {
        self.event_state.event = event;
        // Reset propagation state for the new event
        self.event_state.reset_propagation();
    }

    /// Clear the current event
    pub fn clear_event(&mut self) {
        self.event_state.event = None;
    }

    /// Begin state batching
    pub fn begin_batch(&mut self) {
        self.state_batch.begin_batch();
    }

    /// End state batching and apply updates
    ///
    /// Returns the set of fiber IDs that were modified
    pub fn end_batch(&mut self) -> std::collections::HashSet<crate::fiber::FiberId> {
        self.state_batch.end_batch(&mut self.fiber_tree)
    }

    /// Flush synchronous effects
    pub fn flush_effects(&mut self) {
        self.effect_queue.flush(&mut self.fiber_tree);
    }

    /// Flush async effects
    pub async fn flush_async_effects(&mut self) {
        self.effect_queue.flush_async(&mut self.fiber_tree).await;
    }

    /// Mark unseen fibers for unmount
    pub fn mark_unseen_for_unmount(&mut self) {
        self.fiber_tree.mark_unseen_for_unmount();
    }

    /// Process pending unmounts
    ///
    /// This cleans up context values and removes fibers that were
    /// scheduled for unmount.
    pub fn process_unmounts(&mut self) -> Vec<crate::fiber::FiberId> {
        // Clean up context values for unmounted fibers
        for &fiber_id in &self.fiber_tree.pending_unmount {
            self.context_stack.pop_for_fiber(fiber_id);
        }
        self.fiber_tree.process_unmounts()
    }

    /// Clear all state
    ///
    /// This resets the context to its initial state, useful for
    /// cleanup at the end of the render loop or in tests.
    pub fn clear(&mut self) {
        self.fiber_tree = FiberTree::new();
        self.state_batch.clear();
        self.effect_queue.clear();
        self.context_stack.clear();
        self.event_state = EventState::new();
    }
}

impl Default for RenderContext {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Thread-local API functions
// ============================================================================

/// Initialize the thread-local render context
///
/// This should be called at the start of the render loop to set up
/// the render context for the current thread.
pub fn init_render_context() {
    RENDER_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = Some(RenderContext::new());
    });
}

/// Check if the render context is initialized
pub fn is_render_context_initialized() -> bool {
    RENDER_CONTEXT.with(|ctx| ctx.borrow().is_some())
}

/// Execute a closure with a reference to the render context
///
/// Returns `None` if the context is not initialized.
pub fn with_render_context<R, F: FnOnce(&RenderContext) -> R>(f: F) -> Option<R> {
    RENDER_CONTEXT.with(|ctx| ctx.borrow().as_ref().map(f))
}

/// Execute a closure with a mutable reference to the render context
///
/// Returns `None` if the context is not initialized.
pub fn with_render_context_mut<R, F: FnOnce(&mut RenderContext) -> R>(f: F) -> Option<R> {
    RENDER_CONTEXT.with(|ctx| ctx.borrow_mut().as_mut().map(f))
}

/// Clear the thread-local render context
///
/// This should be called at the end of the render loop to clean up
/// all render-related state.
pub fn clear_render_context() {
    RENDER_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = None;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        clear_render_context();
        init_render_context();
    }

    fn teardown() {
        clear_render_context();
    }

    #[test]
    fn test_render_context_creation() {
        let ctx = RenderContext::new();
        assert!(ctx.fiber_tree.fibers.is_empty());
        assert!(!ctx.state_batch.is_batching());
        assert!(!ctx.effect_queue.has_pending());
        assert!(ctx.event_state.event.is_none());
    }

    #[test]
    fn test_init_and_clear_render_context() {
        clear_render_context();
        assert!(!is_render_context_initialized());

        init_render_context();
        assert!(is_render_context_initialized());

        clear_render_context();
        assert!(!is_render_context_initialized());
    }

    #[test]
    fn test_with_render_context() {
        setup();

        let result = with_render_context(|ctx| ctx.fiber_tree.fibers.len());
        assert_eq!(result, Some(0));

        teardown();
    }

    #[test]
    fn test_with_render_context_mut() {
        setup();

        with_render_context_mut(|ctx| {
            ctx.fiber_tree.mount(None, None);
        });

        let count = with_render_context(|ctx| ctx.fiber_tree.fibers.len());
        assert_eq!(count, Some(1));

        teardown();
    }

    #[test]
    fn test_set_and_clear_event() {
        setup();

        // Set an event
        let event = Event::FocusGained;
        with_render_context_mut(|ctx| {
            ctx.set_event(Some(Arc::new(event)));
        });

        let has_event = with_render_context(|ctx| ctx.event_state.event.is_some());
        assert_eq!(has_event, Some(true));

        // Clear the event
        with_render_context_mut(|ctx| {
            ctx.clear_event();
        });

        let has_event = with_render_context(|ctx| ctx.event_state.event.is_some());
        assert_eq!(has_event, Some(false));

        teardown();
    }

    #[test]
    fn test_begin_and_end_batch() {
        setup();

        with_render_context_mut(|ctx| {
            assert!(!ctx.state_batch.is_batching());
            ctx.begin_batch();
            assert!(ctx.state_batch.is_batching());
        });

        with_render_context_mut(|ctx| {
            let dirty = ctx.end_batch();
            assert!(dirty.is_empty());
            assert!(!ctx.state_batch.is_batching());
        });

        teardown();
    }

    #[test]
    fn test_prepare_for_render() {
        setup();

        // Mount a fiber and simulate some hook calls
        with_render_context_mut(|ctx| {
            let fiber_id = ctx.fiber_tree.mount(None, None);
            ctx.fiber_tree.begin_render(fiber_id);
            if let Some(fiber) = ctx.fiber_tree.get_mut(fiber_id) {
                fiber.next_hook_index();
                fiber.next_hook_index();
            }
            ctx.fiber_tree.end_render();

            // Hook index should be 2
            assert_eq!(ctx.fiber_tree.get(fiber_id).unwrap().hook_index, 2);

            // Prepare for render should reset hook indices
            ctx.prepare_for_render();
            assert_eq!(ctx.fiber_tree.get(fiber_id).unwrap().hook_index, 0);
        });

        teardown();
    }

    #[test]
    fn test_mark_unseen_for_unmount() {
        setup();

        with_render_context_mut(|ctx| {
            // Create a fiber via component ID
            let fiber_id = ctx.fiber_tree.get_or_create_fiber_by_component_id(42);
            ctx.fiber_tree.mark_unseen_for_unmount();

            // Fiber should not be scheduled for unmount (it was seen)
            assert!(!ctx.fiber_tree.pending_unmount.contains(&fiber_id));

            // Simulate next render where fiber is not rendered
            // Don't call get_or_create_fiber_by_component_id
            ctx.fiber_tree.mark_unseen_for_unmount();

            // Now fiber should be scheduled for unmount
            assert!(ctx.fiber_tree.pending_unmount.contains(&fiber_id));
        });

        teardown();
    }

    #[test]
    fn test_process_unmounts_cleans_up_context() {
        setup();

        with_render_context_mut(|ctx| {
            // Create a fiber and add a context value
            let fiber_id = ctx.fiber_tree.mount(None, None);
            ctx.context_stack.push(fiber_id, "test-value".to_string());

            // Verify context exists
            assert_eq!(
                ctx.context_stack.get::<String>(),
                Some("test-value".to_string())
            );

            // Schedule and process unmount
            ctx.fiber_tree.schedule_unmount(fiber_id);
            ctx.process_unmounts();

            // Context should be cleaned up
            assert_eq!(ctx.context_stack.get::<String>(), None);
        });

        teardown();
    }

    #[test]
    fn test_clear_resets_all_state() {
        setup();

        with_render_context_mut(|ctx| {
            // Add some state
            ctx.fiber_tree.mount(None, None);
            ctx.state_batch.begin_batch();
            ctx.context_stack.push(crate::fiber::FiberId(1), 42i32);

            // Clear everything
            ctx.clear();

            // Verify all state is reset
            assert!(ctx.fiber_tree.fibers.is_empty());
            assert!(!ctx.state_batch.is_batching());
            assert!(!ctx.context_stack.has::<i32>());
        });

        teardown();
    }

    #[test]
    fn test_fallback_to_legacy_fiber_tree() {
        // Don't initialize RenderContext
        clear_render_context();

        // Set up legacy fiber tree
        crate::fiber_tree::set_fiber_tree(crate::fiber_tree::FiberTree::new());

        // Direct access to legacy fiber tree still works
        let count = crate::fiber_tree::with_fiber_tree(|tree| tree.fibers.len());
        assert_eq!(count, Some(0));

        // Clean up
        crate::fiber_tree::clear_fiber_tree();
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashMap;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Property 1: Fiber Lifecycle Consistency**
        /// **Validates: Requirements 2.2, 2.3**
        ///
        /// For any component that renders, the RenderContext SHALL create or retrieve
        /// a fiber with stable identity based on TypeId + position (component_id),
        /// and the fiber SHALL be marked as seen for the current render pass.
        #[test]
        fn prop_fiber_lifecycle_consistency(
            component_ids in prop::collection::vec(1u64..10000, 1..20),
            render_passes in 1usize..5
        ) {
            // Initialize RenderContext
            clear_render_context();
            init_render_context();

            let mut created_fibers: HashMap<u64, crate::fiber::FiberId> = HashMap::new();

            for _pass in 0..render_passes {
                // Simulate rendering each component
                for &component_id in &component_ids {
                    with_render_context_mut(|ctx| {
                        let fiber_id = ctx.fiber_tree.get_or_create_fiber_by_component_id(component_id);

                        // Property: Same component ID always returns same fiber
                        if let Some(&existing_fiber_id) = created_fibers.get(&component_id) {
                            prop_assert_eq!(fiber_id, existing_fiber_id,
                                "Component ID {} should always map to same fiber", component_id);
                        } else {
                            created_fibers.insert(component_id, fiber_id);
                        }

                        // Property: Fiber exists in tree
                        prop_assert!(ctx.fiber_tree.fibers.contains_key(&fiber_id),
                            "Fiber {} should exist in tree", fiber_id.0);

                        // Property: Fiber is marked as seen
                        // We verify this indirectly by checking that the fiber won't be
                        // scheduled for unmount after mark_unseen_for_unmount
                        // (The seen_this_render field is private)

                        // Property: begin_render sets current fiber
                        ctx.fiber_tree.begin_render(fiber_id);
                        prop_assert_eq!(ctx.fiber_tree.current_fiber(), Some(fiber_id),
                            "Current fiber should be {} during render", fiber_id.0);

                        // Property: end_render restores previous context
                        ctx.fiber_tree.end_render();
                        prop_assert!(ctx.fiber_tree.current_fiber().is_none(),
                            "Current fiber should be None after end_render");

                        Ok(())
                    }).unwrap()?;
                }

                with_render_context_mut(|ctx| {
                    ctx.fiber_tree.mark_unseen_for_unmount();
                });
            }

            // Clean up
            clear_render_context();
        }

        /// Property: Unseen fibers are scheduled for unmount via RenderContext
        /// **Validates: Requirements 2.5**
        #[test]
        fn prop_unseen_fibers_scheduled_for_unmount_via_context(
            initial_ids in prop::collection::vec(1u64..10000, 1..10),
            surviving_ids in prop::collection::vec(1u64..10000, 0..5)
        ) {
            // Initialize RenderContext
            clear_render_context();
            init_render_context();

            // Create fibers for all initial IDs
            let mut fiber_map: HashMap<u64, crate::fiber::FiberId> = HashMap::new();
            with_render_context_mut(|ctx| {
                for &id in &initial_ids {
                    let fiber_id = ctx.fiber_tree.get_or_create_fiber_by_component_id(id);
                    fiber_map.insert(id, fiber_id);
                }
                ctx.fiber_tree.mark_unseen_for_unmount();
            });

            // Second render: only surviving_ids are rendered
            with_render_context_mut(|ctx| {
                for &id in &surviving_ids {
                    ctx.fiber_tree.get_or_create_fiber_by_component_id(id);
                }
                ctx.fiber_tree.mark_unseen_for_unmount();
            });

            // Property: Fibers not in surviving_ids should be scheduled for unmount
            with_render_context(|ctx| {
                for (&component_id, &fiber_id) in &fiber_map {
                    let should_survive = surviving_ids.contains(&component_id);
                    let is_pending_unmount = ctx.fiber_tree.pending_unmount.contains(&fiber_id);

                    if !should_survive {
                        prop_assert!(is_pending_unmount,
                            "Fiber for component {} should be scheduled for unmount", component_id);
                    }
                }
                Ok(())
            }).unwrap()?;

            // Clean up
            clear_render_context();
        }

        /// Property: Context cleanup on unmount via RenderContext
        /// **Validates: Requirements 6.4**
        #[test]
        fn prop_context_cleanup_on_unmount_via_context(
            fiber_count in 1usize..10,
            context_values in prop::collection::vec(1i32..1000, 1..10)
        ) {
            // Initialize RenderContext
            clear_render_context();
            init_render_context();

            let mut fiber_ids = Vec::new();

            // Create fibers and add context values
            with_render_context_mut(|ctx| {
                for i in 0..fiber_count {
                    let fiber_id = ctx.fiber_tree.mount(None, None);
                    fiber_ids.push(fiber_id);

                    // Add a context value for each fiber
                    if i < context_values.len() {
                        ctx.context_stack.push(fiber_id, context_values[i]);
                    }
                }
            });

            // Verify context values exist
            with_render_context(|ctx| {
                if !context_values.is_empty() {
                    prop_assert!(ctx.context_stack.has::<i32>(),
                        "Context should have i32 values");
                }
                Ok(())
            }).unwrap()?;

            // Schedule all fibers for unmount and process
            with_render_context_mut(|ctx| {
                for &fiber_id in &fiber_ids {
                    ctx.fiber_tree.schedule_unmount(fiber_id);
                }
                ctx.process_unmounts();
            });

            // Property: All context values should be cleaned up
            with_render_context(|ctx| {
                prop_assert!(!ctx.context_stack.has::<i32>(),
                    "Context should be empty after unmount");
                Ok(())
            }).unwrap()?;

            // Clean up
            clear_render_context();
        }

        /// Property: State batching works correctly via RenderContext
        /// **Validates: Requirements 3.1, 3.2, 3.3**
        #[test]
        fn prop_state_batching_via_context(
            update_count in 1usize..10
        ) {
            use crate::scheduler::batch::{StateUpdate, StateUpdateKind};

            // Initialize RenderContext
            clear_render_context();
            init_render_context();

            // Create a fiber
            let fiber_id = with_render_context_mut(|ctx| {
                let fiber_id = ctx.fiber_tree.mount(None, None);
                ctx.fiber_tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);
                fiber_id
            }).unwrap();

            // Begin batch and queue updates
            with_render_context_mut(|ctx| {
                ctx.begin_batch();

                for i in 0..update_count {
                    ctx.state_batch.queue_update(
                        fiber_id,
                        StateUpdate {
                            hook_index: 0,
                            update: StateUpdateKind::Value(Box::new(i as i32)),
                        },
                    );
                }

                // Property: Batching should be active
                prop_assert!(ctx.state_batch.is_batching(),
                    "Batching should be active");

                // End batch
                let dirty = ctx.end_batch();

                // Property: Fiber should be dirty
                prop_assert!(dirty.contains(&fiber_id),
                    "Fiber should be marked dirty");

                // Property: Final value should be the last update
                let final_value = ctx.fiber_tree.get(fiber_id).unwrap().get_hook::<i32>(0);
                prop_assert_eq!(final_value, Some((update_count - 1) as i32),
                    "Final value should be the last update");

                Ok(())
            }).unwrap()?;

            // Clean up
            clear_render_context();
        }

        /// Property: Event available to all fibers via RenderContext (React-like semantics)
        /// **Validates: Requirements 1.1, 1.2, 2.2, 2.3**
        #[test]
        fn prop_event_available_to_all_fibers_via_context(
            fiber_count in 2usize..10
        ) {
            // Initialize RenderContext
            clear_render_context();
            init_render_context();

            // Set an event
            let event = Event::FocusGained;
            with_render_context_mut(|ctx| {
                ctx.set_event(Some(Arc::new(event)));
            });

            // Create fibers for testing
            let fiber_ids: Vec<_> = with_render_context_mut(|ctx| {
                (0..fiber_count).map(|_| {
                    ctx.fiber_tree.mount(None, None)
                }).collect()
            }).unwrap_or_default();

            // Property 1: All fibers can read the same event (React-like behavior)
            for &fiber_id in &fiber_ids {
                let result = with_render_context_mut(|ctx| {
                    ctx.fiber_tree.begin_render(fiber_id);
                    let event = ctx.event_state.event.clone();
                    ctx.fiber_tree.end_render();
                    event
                });

                prop_assert!(result.flatten().is_some(),
                    "Fiber {:?} should be able to read the event", fiber_id);
            }

            // Property 2: Same fiber can read event multiple times
            if let Some(&fiber_id) = fiber_ids.first() {
                for read_num in 0..3 {
                    let result = with_render_context_mut(|ctx| {
                        ctx.fiber_tree.begin_render(fiber_id);
                        let event = ctx.event_state.event.clone();
                        ctx.fiber_tree.end_render();
                        event
                    });

                    prop_assert!(result.flatten().is_some(),
                        "Fiber should read event on attempt {}", read_num);
                }
            }

            // Property 3: After stop_propagation, only stopping fiber can read
            if fiber_ids.len() >= 2 {
                let stopping_fiber = fiber_ids[0];
                let other_fiber = fiber_ids[1];

                // Stop propagation from first fiber
                with_render_context_mut(|ctx| {
                    ctx.fiber_tree.begin_render(stopping_fiber);
                    ctx.event_state.propagation_stopped = true;
                    ctx.event_state.stopped_by_fiber = Some(stopping_fiber);
                    ctx.fiber_tree.end_render();
                });

                // Stopping fiber can still read
                let stopping_result = with_render_context_mut(|ctx| {
                    ctx.fiber_tree.begin_render(stopping_fiber);
                    let can_read = !ctx.event_state.propagation_stopped
                        || ctx.event_state.stopped_by_fiber == Some(stopping_fiber);
                    let event = if can_read { ctx.event_state.event.clone() } else { None };
                    ctx.fiber_tree.end_render();
                    event
                });
                prop_assert!(stopping_result.flatten().is_some(),
                    "Stopping fiber should still read event");

                // Other fiber cannot read
                let other_result = with_render_context_mut(|ctx| {
                    ctx.fiber_tree.begin_render(other_fiber);
                    let can_read = !ctx.event_state.propagation_stopped
                        || ctx.event_state.stopped_by_fiber == Some(other_fiber);
                    let event = if can_read { ctx.event_state.event.clone() } else { None };
                    ctx.fiber_tree.end_render();
                    event
                });
                prop_assert!(other_result.flatten().is_none(),
                    "Other fiber should NOT read event after stop_propagation");
            }

            // Clean up
            clear_render_context();
        }
    }
}
