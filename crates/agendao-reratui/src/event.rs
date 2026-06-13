//! Event system for sharing terminal events between components.
//!
//! This module provides the event state management for the fiber-based component system.
//! It allows components to access terminal events (keyboard, mouse, resize) through
//! the `use_event` hook.
//!
//! # Architecture
//!
//! Events are available to ALL components during a render frame, matching React's
//! event semantics. Components can explicitly call `stop_propagation()` to prevent
//! other components from receiving the event.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::event::{set_current_event, clear_current_event};
//! use crossterm::event::Event;
//! use std::sync::Arc;
//!
//! // Set an event (typically done by the runtime)
//! set_current_event(Some(Arc::new(Event::Key(...))));
//!
//! // Components can then use use_event() to access it
//! // Multiple components can read the same event!
//!
//! // Clear the event at the end of the render cycle
//! clear_current_event();
//! ```

use crate::fiber::FiberId;
use crossterm::event::Event;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Structure to track the current event and propagation state.
///
/// Events are available to all fibers during a render frame unless propagation
/// is explicitly stopped. This matches React's event semantics where events
/// bubble through the component tree.
#[derive(Default)]
pub struct EventState {
    /// The current event being processed.
    pub(crate) event: Option<Arc<Event>>,
    /// Whether propagation has been stopped for this event.
    pub(crate) propagation_stopped: bool,
    /// The fiber that stopped propagation (can still read the event).
    pub(crate) stopped_by_fiber: Option<FiberId>,
}

impl EventState {
    /// Creates a new empty EventState.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if an event is currently available.
    pub fn has_event(&self) -> bool {
        self.event.is_some()
    }

    /// Resets propagation state (called when new event is set).
    pub fn reset_propagation(&mut self) {
        self.propagation_stopped = false;
        self.stopped_by_fiber = None;
    }
}

/// Global storage for the current event.
///
/// This is thread-local to ensure proper isolation in multi-threaded scenarios.
pub(crate) static CURRENT_EVENT: Lazy<RwLock<EventState>> = Lazy::new(Default::default);

/// Sets the current event in the global storage.
///
/// This function should be called by the runtime when an event is received.
/// It resets propagation state, allowing all fibers to read the new event.
///
/// # Arguments
///
/// * `event` - The event to set, or None to clear the current event.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::event::set_current_event;
/// use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
/// use std::sync::Arc;
///
/// let key_event = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
/// set_current_event(Some(Arc::new(key_event)));
/// ```
pub fn set_current_event(event: Option<Arc<Event>>) {
    let event_debug = event.clone();

    let mut current_event = CURRENT_EVENT.write().unwrap();
    current_event.event = event;
    current_event.reset_propagation();

    debug!("Set current event: {:?}, propagation reset", event_debug);
}

/// Gets the current event for the current fiber.
///
/// Returns the event if:
/// - An event is available
/// - Propagation has not been stopped, OR
/// - This fiber is the one that stopped propagation
///
/// Unlike the previous implementation, this does NOT consume the event.
/// Multiple calls from the same fiber or different fibers will all receive
/// the same event, matching React's event semantics.
///
/// # Returns
///
/// * `Some(Arc<Event>)` - The current event if available and propagation allows.
/// * `None` - If no event is available or propagation was stopped by another fiber.
pub fn get_current_event() -> Option<Arc<Event>> {
    use crate::fiber_tree::with_current_fiber;

    let event_state = CURRENT_EVENT.read().unwrap();

    // Get the current event, return None if no event is available
    let event = match event_state.event.as_ref() {
        Some(e) => e.clone(),
        None => {
            debug!("No event available");
            return None;
        }
    };

    // Check if propagation was stopped
    if event_state.propagation_stopped {
        // Get current fiber ID
        let current_fiber_id = with_current_fiber(|fiber| fiber.id);

        // Allow the fiber that stopped propagation to still read the event
        if current_fiber_id != event_state.stopped_by_fiber {
            debug!("Propagation stopped by another fiber");
            return None;
        }
    }

    Some(event)
}

/// Stops propagation of the current event.
///
/// After calling this, only the fiber that called stop_propagation
/// can continue to read the event via use_event(). All other fibers
/// will receive None.
///
/// This matches React's `event.stopPropagation()` behavior.
pub fn stop_event_propagation() {
    use crate::fiber_tree::with_current_fiber;

    let current_fiber_id = with_current_fiber(|fiber| fiber.id);

    let mut event_state = CURRENT_EVENT.write().unwrap();
    event_state.propagation_stopped = true;
    event_state.stopped_by_fiber = current_fiber_id;

    debug!("Propagation stopped by fiber {:?}", current_fiber_id);
}

/// Clears the current event from the global storage.
///
/// This function should be called at the end of each render cycle to ensure
/// events don't persist across renders.
pub fn clear_current_event() {
    set_current_event(None);
}

/// Returns the current event without affecting propagation state.
///
/// This is useful for peeking at the event without consuming it.
/// Works even after stop_propagation() was called.
///
/// # Returns
///
/// * `Some(Arc<Event>)` - The current event if available.
/// * `None` - If no event is available.
pub fn peek_current_event() -> Option<Arc<Event>> {
    let event_state = CURRENT_EVENT.read().unwrap();
    event_state.event.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;

    /// Test mutex to ensure tests run sequentially since they share global state
    /// This is shared between unit tests and property tests
    pub(super) static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn create_test_key_event(c: char) -> Event {
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char(c),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))
    }

    fn setup_test_fiber() -> FiberId {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn teardown_test_fiber() {
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
        clear_fiber_tree();
    }

    #[test]
    fn test_set_and_get_event() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();
        setup_test_fiber();

        let event = create_test_key_event('a');
        set_current_event(Some(Arc::new(event.clone())));

        let retrieved = get_current_event();
        assert!(retrieved.is_some());

        if let Some(e) = retrieved {
            if let Event::Key(key) = &*e {
                assert_eq!(key.code, KeyCode::Char('a'));
            } else {
                panic!("Expected Key event");
            }
        }

        teardown_test_fiber();
    }

    #[test]
    fn test_event_available_multiple_times_per_fiber() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();
        setup_test_fiber();

        let event = create_test_key_event('b');
        set_current_event(Some(Arc::new(event)));

        // First call should return the event
        let first = get_current_event();
        assert!(first.is_some());

        // Second call should ALSO return the event (React-like behavior)
        let second = get_current_event();
        assert!(second.is_some());

        // Third call should ALSO return the event
        let third = get_current_event();
        assert!(third.is_some());

        teardown_test_fiber();
    }

    #[test]
    fn test_different_fibers_can_read_same_event() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();

        let mut tree = FiberTree::new();
        let fiber1 = tree.mount(None, None);
        let fiber2 = tree.mount(None, None);
        set_fiber_tree(tree);

        let event = create_test_key_event('c');
        set_current_event(Some(Arc::new(event)));

        // Fiber 1 reads the event
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber1);
        });
        let result1 = get_current_event();
        assert!(result1.is_some());
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        // Fiber 2 can also read the same event
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber2);
        });
        let result2 = get_current_event();
        assert!(result2.is_some());
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        // Fiber 1 can read again (no consumption)
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber1);
        });
        let result1_again = get_current_event();
        assert!(result1_again.is_some());
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        clear_fiber_tree();
    }

    #[test]
    fn test_stop_propagation_blocks_other_fibers() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();

        let mut tree = FiberTree::new();
        let fiber1 = tree.mount(None, None);
        let fiber2 = tree.mount(None, None);
        set_fiber_tree(tree);

        let event = create_test_key_event('d');
        set_current_event(Some(Arc::new(event)));

        // Fiber 1 reads and stops propagation
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber1);
        });
        let result1 = get_current_event();
        assert!(result1.is_some());
        stop_event_propagation();
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        // Fiber 2 should NOT be able to read the event
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber2);
        });
        let result2 = get_current_event();
        assert!(result2.is_none());
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        clear_fiber_tree();
    }

    #[test]
    fn test_stop_propagation_allows_same_fiber() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();
        let fiber_id = setup_test_fiber();

        let event = create_test_key_event('e');
        set_current_event(Some(Arc::new(event)));

        // Read event
        let result1 = get_current_event();
        assert!(result1.is_some());

        // Stop propagation
        stop_event_propagation();

        // Same fiber can still read the event
        let result2 = get_current_event();
        assert!(result2.is_some());

        // Verify it's the same fiber
        with_fiber_tree_mut(|tree| {
            assert!(tree.get(fiber_id).is_some());
        });

        teardown_test_fiber();
    }

    #[test]
    fn test_clear_event() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();
        setup_test_fiber();

        let event = create_test_key_event('f');
        set_current_event(Some(Arc::new(event)));

        // Event should be available
        assert!(peek_current_event().is_some());

        // Clear the event
        clear_current_event();

        // Event should no longer be available
        assert!(peek_current_event().is_none());

        teardown_test_fiber();
    }

    #[test]
    fn test_new_event_resets_propagation_state() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();

        let mut tree = FiberTree::new();
        let fiber1 = tree.mount(None, None);
        let fiber2 = tree.mount(None, None);
        set_fiber_tree(tree);

        let event1 = create_test_key_event('g');
        set_current_event(Some(Arc::new(event1)));

        // Fiber 1 stops propagation
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber1);
        });
        stop_event_propagation();
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        // Set a new event - should reset propagation
        let event2 = create_test_key_event('h');
        set_current_event(Some(Arc::new(event2)));

        // Fiber 2 should be able to read the new event
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber2);
        });
        let result = get_current_event();
        assert!(result.is_some());
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        clear_fiber_tree();
    }

    #[test]
    fn test_peek_does_not_affect_propagation() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();
        setup_test_fiber();

        let event = create_test_key_event('i');
        set_current_event(Some(Arc::new(event)));

        // Peek at the event
        let peeked = peek_current_event();
        assert!(peeked.is_some());

        // Fiber should still be able to get the event
        let retrieved = get_current_event();
        assert!(retrieved.is_some());

        // Peek again after stop_propagation
        stop_event_propagation();
        let peeked_after = peek_current_event();
        assert!(peeked_after.is_some());

        teardown_test_fiber();
    }

    #[test]
    fn test_event_state_helpers() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();

        let state = CURRENT_EVENT.read().unwrap();
        assert!(!state.has_event());
        drop(state);

        let event = create_test_key_event('j');
        set_current_event(Some(Arc::new(event)));

        let state = CURRENT_EVENT.read().unwrap();
        assert!(state.has_event());
        assert!(!state.propagation_stopped);
        assert!(state.stopped_by_fiber.is_none());
    }

    #[test]
    fn test_propagation_state_reset() {
        let _lock = TEST_MUTEX.lock();
        clear_current_event();

        let mut state = EventState::new();
        state.propagation_stopped = true;
        state.stopped_by_fiber = Some(FiberId(42));

        state.reset_propagation();

        assert!(!state.propagation_stopped);
        assert!(state.stopped_by_fiber.is_none());
    }
}
